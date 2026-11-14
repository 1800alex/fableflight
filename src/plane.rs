//! The aircraft's systems and a six-degree-of-freedom flight model for a
//! Cessna 172-class four-seater (180 hp, about 1100 kg, 16.2 m² of
//! strut-braced wing; this one has retractable gear, worth a few knots):
//! a fixed-pitch propeller whose revs and thrust follow the airspeed, the
//! engine losing power with altitude in a standard atmosphere, the
//! airspeed indicator reading indicated (not true) airspeed,
//! propeller torque and slipstream, adverse yaw, stall buffet and wing drop,
//! ground effect, and the turbulence the weather stirs up.
//! The model itself is built in `airframe.rs`; `pose` animates its pieces;
//! `damage.rs` describes what has been knocked off it. Trees, rooftops, the
//! ground, over-speed and over-G all take parts off the airframe here, and
//! the aircraft flies on (as best it can) with what is left.
use crate::airframe::{self, Anim, Piece, SPAN, G_ENGINE, G_LWING, G_RWING, G_TAIL};
use crate::crash::Rng;
use crate::damage::{self, Damage, Sel, ShedReq, Tear};
use crate::weather::Air;
use crate::geo;
use crate::world::{TreeRef, World};
use glam::{DMat3, DQuat, DVec3, Mat4, Quat, Vec3};

#[derive(Default, Clone, Copy)]
pub struct Controls {
    pub pitch: f64, // +1 = nose up
    pub roll: f64,  // +1 = roll right
    pub yaw: f64,   // +1 = nose right
    pub throttle: f64,
    pub brake: bool,
    pub boost: bool,
}

pub struct Impact {
    pub pos: DVec3,
    pub vel: DVec3,
    pub water: bool,
    pub ground_h: f64,
    /// the building that was hit, if any (tile, footprint index)
    pub building: Option<(geo::TileId, usize)>,
}

pub struct Plane {
    pub pos: DVec3,
    pub vel: DVec3,
    /// the air's own motion where the aircraft is (planet frame); the wing only feels `vel - wind`
    pub wind: DVec3,
    /// how rough the air is here (set by the caller from the weather each frame)
    pub air: Air,
    /// the turbulent part of the air's motion (planet frame), on top of `wind`
    pub gust: DVec3,
    /// rotational gusts (body frame, rad/s): the air rolling, pitching and yawing across the airframe
    pub gust_rot: DVec3,
    /// turbulence filter states: translational in the flight-path frame (along, across, up), and rotational
    gust_a: DVec3,
    gust_b: DVec3,
    rot_a: DVec3,
    rot_b: DVec3,
    /// the pilot's head swaying on its neck as the airframe is shaken (body frame, m)
    pub head: DVec3,
    /// specific force (what an accelerometer reads: everything but gravity), body frame, m/s²
    pub sf: DVec3,
    head_v: DVec3,
    /// shaft power being made, as a fraction of rated
    pub power: f64,
    /// how far open the throttle effectively is (the engine takes a moment to answer it)
    charge: f64,
    /// which wing a stall would rather drop (slowly wandering, -1..1)
    drop: f64,
    pub rot: DQuat,
    pub omega: DVec3,
    pub throttle: f64,
    pub prop: f64,
    pub on_ground: bool,
    pub stalled: bool,
    pub alpha: f64,
    pub g_load: f64,
    /// smoothed control inputs: pitch, yaw, roll
    pub stick: DVec3,
    /// flap extension, 0 up .. 1 full (30 degrees), and the selected notch 0..3
    pub flaps: f64,
    pub flap_set: u8,
    /// gear position, 0 stowed .. 1 down and locked, and where the handle is
    pub gear: f64,
    pub gear_down: bool,
    /// nose, left main, right main: how far each oleo / spring is compressed (m),
    /// how far each wheel has turned (rad) and how fast it is turning (rad/s)
    pub compress: [f64; 3],
    pub wheel_rot: [f64; 3],
    pub wheel_spin: [f64; 3],
    /// propeller speed, rad/s
    pub prop_rate: f64,
    /// the angle of attack the auto-trim is holding (drives the trim wheel)
    pub trim: f64,
    /// trees struck during the last step, for the caller to knock down
    pub tree_hits: Vec<TreeRef>,
    /// recent brushes with foliage
    pub brushes: f64,
    /// what is broken, and what was broken at the start of the step (so the
    /// caller can still find the pieces that have just gone)
    pub dmg: Damage,
    pub dmg_prev: Damage,
    /// parts that came off during the last step, for the caller to let go of
    pub shed: Vec<ShedReq>,
    /// one line per thing that just broke, for the HUD and the sound
    pub notes: Vec<String>,
    /// where metal is grinding along the ground: position, velocity of the sparks
    pub sparks: Vec<(DVec3, DVec3)>,
    /// seconds flown
    pub t: f64,
    near_trees: Vec<TreeRef>,
    scan: u32,
    rng: Rng,
    /// held where it is (waiting on the ground for the terrain under it to load)
    pub hold: bool,
    /// where it is splashing through water: position, velocity of the spray
    pub splashes: Vec<(DVec3, DVec3)>,
    /// where something is ploughing or rolling through the ground: position,
    /// velocity the dirt is thrown at, how much (wheel ~1, airframe more)
    pub dirt: Vec<(DVec3, DVec3, f32)>,
    /// resting or sliding on the airframe itself rather than the wheels
    pub on_hull: bool,
    /// contact points (reused each step)
    pts: Vec<(DVec3, Part)>,
    /// moments since the left wing, right wing, tail and nose last broke
    /// something (so one blow is one break, not one per step)
    cool: [f64; 4],
    /// torque the ground puts on each hanging wing this step
    fold_tq: [f64; 2],
    /// how far a ditched aircraft has settled into the water
    sink: f64,
    /// what made it go up in flames, once it has
    pub cause: &'static str,
}

/// What a contact point belongs to (see `Plane::contacts`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    /// spinner and cowling
    Nose,
    /// the propeller's tips
    Prop,
    Belly,
    /// the cabin's sides
    Side,
    /// over the cabin and behind the wing
    Roof,
    /// the tail cone
    Tail,
    Fin,
    Stab(usize),
    /// a wing (side) at a span station
    Wing(usize, f32),
    /// a wing that is hanging from its stump
    Fold(usize),
}

impl Part {
    fn group(self) -> usize {
        match self {
            Part::Nose | Part::Prop => G_ENGINE,
            Part::Wing(0, _) | Part::Fold(0) => G_LWING,
            Part::Wing(..) | Part::Fold(_) => G_RWING,
            Part::Tail | Part::Fin | Part::Stab(_) => G_TAIL,
            _ => airframe::G_FUSE,
        }
    }
}

/// Two aboard and most of the fuel (maximum takeoff weight is 1157 kg).
const MASS: f64 = 1100.0;
/// Rated power at sea level: 180 hp at 2700 rpm.
const POWER: f64 = 134_000.0;
const W_RATED: f64 = 2700.0 * std::f64::consts::TAU / 60.0;
/// Fixed-pitch propeller, 76 in: diameter (m), thrust and power coefficients
/// at rest, and the advance ratios J = V / nD at which the thrust and then
/// the power it takes run out (past the second it windmills, driving the engine).
const PROP_D: f64 = 1.93;
const CT0: f64 = 0.096;
const CP0: f64 = 0.059;
const J_T: f64 = 1.05;
const J_P: f64 = 1.5;
/// Propeller, crankshaft and flywheel (kg m²).
const PROP_I: f64 = 3.0;
/// About the static thrust at full power (the scale for torque and slipstream effects).
const STATIC_THRUST: f64 = 2500.0;
/// Sea-level air density (kg/m³).
pub const RHO0: f64 = 1.225;
/// Wing span (m).
const B: f64 = 11.0;
const WING_AREA: f64 = 16.2;
/// Lift: at zero angle of attack, and the slope per radian (aspect ratio 7.5).
const CL0: f64 = 0.25;
const CL_A: f64 = 5.0;
/// Drag: the clean airframe with its struts, steps and cooling (gear up), the
/// induced-drag factor, and what the lowered gear adds.
const CD0: f64 = 0.034;
const K_IND: f64 = 0.055;
const CD_GEAR: f64 = 0.009;
/// Pitch, yaw, roll (kg m², published figures for a 172).
const INERTIA: DVec3 = DVec3::new(1825.0, 2667.0, 1285.0);
/// Tyre contact points: nose, left main, right main (see airframe.rs).
const GEAR: [DVec3; 3] = [DVec3::new(0.0, -1.50, -2.87), DVec3::new(-1.25, -1.50, 0.30), DVec3::new(1.25, -1.50, 0.30)];
const WHEEL_R: [f64; 3] = [airframe::NOSE_R as f64, airframe::MAIN_R as f64, airframe::MAIN_R as f64];
/// Points that must not touch anything: spinner, wing tips, tail, fin, belly, propeller tip.
pub const HARD: [DVec3; 7] = [
    DVec3::new(0.0, -0.03, -3.62),
    DVec3::new(-5.5, 0.82, -0.2),
    DVec3::new(5.5, 0.82, -0.2),
    DVec3::new(0.0, 0.19, 4.62),
    DVec3::new(0.0, 1.83, 4.1),
    DVec3::new(0.0, -0.73, 0.0),
    DVec3::new(0.0, -1.0, -3.30),
];
const FLAP_RATE: f64 = 1.0 / 7.0;
const GEAR_RATE: f64 = 1.0 / 6.0;
/// Never-exceed speed (m/s indicated, 163 kt); beyond it the control surfaces start to flutter.
pub const VNE: f64 = 83.9;
const FLUTTER_V: f64 = 95.0;
/// Flaps must not be out above these (indicated): 110 kt for the first notch, 85 kt beyond.
pub const VFE_10: f64 = 56.6;
pub const VFE: f64 = 43.7;

impl Plane {
    pub fn spawn(lat: f64, lon: f64, alt: f64, heading: f64, speed: f64) -> Self {
        let pos = geo::lla_to_pos(lat, lon, alt);
        let (e, n, u) = geo::enu(pos);
        let f = n * heading.cos() + e * heading.sin();
        // Nose up by the angle of attack that carries the weight at this speed
        // and height, and trimmed there, so it starts out in level flight.
        let rho = density(alt);
        let trim = Self::alpha_level(speed, rho, 0.0);
        // Power for level flight too, and the propeller already turning at the
        // revs that power makes, or it starts out climbing or sinking.
        let throttle = if speed > 20.0 { Self::level_throttle(speed, rho, 0.0, 0.0) } else { 0.0 };
        let prop_rate = Self::prop_equilibrium(throttle, speed, rho);
        let rot = DQuat::from_mat3(&DMat3::from_cols(f.cross(u), u, -f)) * DQuat::from_rotation_x(trim);
        Self {
            pos,
            vel: f * speed,
            wind: DVec3::ZERO,
            air: Air::default(),
            gust: DVec3::ZERO,
            gust_rot: DVec3::ZERO,
            gust_a: DVec3::ZERO,
            gust_b: DVec3::ZERO,
            rot_a: DVec3::ZERO,
            rot_b: DVec3::ZERO,
            head: DVec3::ZERO,
            sf: DVec3::new(0.0, 9.81, 0.0),
            head_v: DVec3::ZERO,
            power: 0.7,
            charge: throttle,
            drop: 0.0,
            rot,
            omega: DVec3::ZERO,
            throttle,
            prop: 0.0,
            on_ground: false,
            stalled: false,
            alpha: 0.0,
            g_load: 1.0,
            stick: DVec3::ZERO,
            flaps: 0.0,
            flap_set: 0,
            // Airborne starts are cleaned up: gear stowed.
            gear: 0.0,
            gear_down: false,
            compress: [0.0; 3],
            wheel_rot: [0.0; 3],
            wheel_spin: [0.0; 3],
            prop_rate,
            trim,
            tree_hits: Vec::new(),
            brushes: 0.0,
            dmg: Damage::default(),
            dmg_prev: Damage::default(),
            shed: Vec::new(),
            notes: Vec::new(),
            sparks: Vec::new(),
            dirt: Vec::new(),
            t: 0.0,
            near_trees: Vec::new(),
            scan: 0,
            rng: Rng(pos.x.to_bits() | 1),
            hold: false,
            splashes: Vec::new(),
            on_hull: false,
            pts: Vec::new(),
            cool: [0.0; 4],
            fold_tq: [0.0; 2],
            sink: 0.0,
            cause: "",
        }
    }

    /// Standing on its wheels, engine idling, brakes off: ready to taxi.
    /// `h` is the ground height under it. It is held still until released.
    pub fn spawn_ground(lat: f64, lon: f64, h: f64, heading: f64, pitch: f64) -> Self {
        let mut p = Self::spawn(lat, lon, h - GEAR[0].y, heading, 0.0);
        let (e, n, u) = geo::enu(p.pos);
        let f = n * heading.cos() + e * heading.sin();
        p.rot = DQuat::from_mat3(&DMat3::from_cols(f.cross(u), u, -f)) * DQuat::from_rotation_x(pitch);
        p.vel = DVec3::ZERO;
        p.gear = 1.0;
        p.gear_down = true;
        p.throttle = 0.0;
        p.charge = 0.0;
        p.power = 0.0;
        p.prop_rate = 65.0;
        p.trim = 0.08;
        p.on_ground = true;
        p.hold = true;
        p
    }
    pub fn forward(&self) -> DVec3 {
        self.rot * DVec3::NEG_Z
    }
    pub fn up(&self) -> DVec3 {
        self.rot * DVec3::Y
    }
    pub fn right(&self) -> DVec3 {
        self.rot * DVec3::X
    }

    /// Nose wheel steering angle (rad, + = right), only with the gear down.
    pub fn steer_angle(&self) -> f64 {
        self.stick.y * 0.45 * self.gear
    }
    /// Angle of attack the wing stalls at (flaps make it stall a little earlier).
    pub fn stall_alpha(&self) -> f64 {
        0.27 - 0.04 * self.flaps
    }
    pub fn flap_deg(&self) -> f64 {
        self.flaps * 30.0
    }
    pub fn toggle_gear(&mut self) {
        // The squat switch blocks retraction on the ground.
        if !(self.gear_down && self.on_ground) {
            self.gear_down = !self.gear_down;
        }
    }
    pub fn set_flaps(&mut self, delta: i32) {
        self.flap_set = (self.flap_set as i32 + delta).clamp(0, 3) as u8;
    }
    /// The highest speed (indicated) the flaps may be out at as they are now.
    pub fn vfe(&self) -> f64 {
        if self.flaps <= 0.34 { VFE_10 } else { VFE }
    }
    /// Indicated airspeed (m/s): what the pitot tube feels, true airspeed scaled
    /// by the square root of the density. It is what the wing flies by: the
    /// aircraft stalls, and its limits are set, at the same indicated speed at any height.
    pub fn ias(&self) -> f64 {
        let (_, _, h) = geo::pos_to_lla(self.pos);
        self.airspeed().length() * (density(h) / RHO0).sqrt()
    }
    /// Start out with the flaps at `notch` and the gear as given, re-trimmed
    /// for the airspeed it has (to fly level, or on a glide of `descent` m/s),
    /// the power set to match.
    pub fn configure(&mut self, notch: u8, gear_down: bool, descent: f64) {
        self.flap_set = notch.min(3);
        self.flaps = self.flap_set as f64 / 3.0;
        self.gear_down = gear_down;
        self.gear = if gear_down { 1.0 } else { 0.0 };
        let (_, _, h) = geo::pos_to_lla(self.pos);
        let rho = density(h);
        let v = self.airspeed().length();
        let flap_cl = 0.55 * self.flaps;
        let a_old = self.trim;
        self.trim = Self::alpha_level(v, rho, flap_cl);
        // Pitch the nose to the new angle of attack and the flight path down the glide.
        let up = self.pos.normalize();
        let fp = (self.airspeed() - up * descent).normalize();
        self.vel = self.wind + fp * v;
        let right = self.right();
        self.rot = DQuat::from_axis_angle(right, self.trim - a_old - (descent / v).asin()) * self.rot;
        self.throttle = Self::throttle_for(v, rho, self.flaps, self.gear, descent);
        self.charge = self.throttle;
        self.prop_rate = Self::prop_equilibrium(self.throttle, v, rho);
    }

    /// Where every piece of the model is, relative to the body, this instant.
    pub fn pose(&self, pieces: &[Piece]) -> Vec<Mat4> {
        let s = self.stick.as_vec3();
        let (pitch, yaw, roll) = (s.x, s.y, s.z);
        let hinge = |p: &Piece, a: f32| Mat4::from_translation(p.pivot) * Mat4::from_axis_angle(p.axis, a) * Mat4::from_translation(-p.pivot);
        let gear = self.gear as f32;
        // The doors open first, the legs swing, then the main doors close
        // again behind them (the nose doors stay open with the gear down).
        let legs = smooth01(0.15, 0.85, gear);
        let doors = smooth01(0.0, 0.12, gear);
        let main_doors = doors * (1.0 - smooth01(0.88, 1.0, gear));
        let comp = self.compress.map(|c| c as f32);
        let t = self.t as f32;
        // Past the never-exceed speed the control surfaces buzz on their hinges.
        let fl = smooth01((VNE + 5.0) as f32, (VNE + 30.0) as f32, self.ias() as f32) * 0.12;
        let d = &self.dmg;
        // Torque link angles for a given oleo compression.
        let link = |c: f32, upper: bool| -> f32 {
            let ang = |d: f32| {
                let half = d * 0.5;
                let h = (0.095f32 * 0.095 - half * half).max(0.0).sqrt();
                if upper { (-h).atan2(-half) } else { (-h).atan2(half) }
            };
            ang(0.16 - c) - ang(0.16)
        };
        let mut out: Vec<Mat4> = Vec::with_capacity(pieces.len());
        for p in pieces {
            let local = match p.anim {
                Anim::Fixed => Mat4::IDENTITY,
                // About a spanwise (+X) hinge a positive angle puts the trailing edge down.
                Anim::Aileron(side) => {
                    let k = if side < 0.0 { 0 } else { 1 };
                    let a = if d.aileron[k] > 0.0 && d.aileron[k] < 1.0 {
                        // Hanging from one hinge: drooped, flapping in the slipstream.
                        0.55 + 0.14 * (t * 27.0 + side).sin()
                    } else {
                        -side * 0.35 * roll + fl * (t * 41.0 + side).sin()
                    };
                    hinge(p, a)
                }
                Anim::Flap(_) => hinge(p, self.flaps as f32 * 30f32.to_radians()),
                Anim::Elevator => hinge(p, -(0.40 * pitch + 0.5 * (self.trim as f32 - 0.04)) + fl * 0.7 * (t * 37.0).sin()),
                Anim::Rudder => hinge(p, 0.42 * yaw + fl * (t * 43.0).sin()),
                Anim::Prop | Anim::PropDisc => hinge(p, -(self.prop % std::f64::consts::TAU) as f32),
                Anim::NoseLeg => {
                    let q = Quat::from_rotation_x(-(1.0 - legs) * airframe::NOSE_FOLD) * Quat::from_rotation_y(-self.steer_angle() as f32 * smooth01(0.9, 1.0, gear));
                    Mat4::from_translation(p.pivot) * Mat4::from_quat(q) * Mat4::from_translation(-p.pivot)
                }
                Anim::NoseFork => Mat4::from_translation(Vec3::Y * comp[0]),
                Anim::NoseWheel => hinge(p, -self.wheel_rot[0] as f32),
                // The links sit behind the strut: mirrored fore and aft, the angles change sign.
                Anim::LinkUpper => hinge(p, -link(comp[0], true)),
                Anim::LinkLower => hinge(p, -link(comp[0], false)),
                Anim::NoseDoor(_) => hinge(p, 1.45 * doors),
                Anim::GearDoor(open) => hinge(p, open * main_doors),
                Anim::MainLeg(side) => {
                    let m = Vec3::new(side, 1.0, 1.0);
                    let (root, axle, stowed) = (airframe::MAIN_ROOT * m, airframe::MAIN_AXLE * m, airframe::MAIN_STOWED * m);
                    let up = Quat::from_rotation_arc((axle - root).normalize(), (stowed - root).normalize());
                    let q = Quat::IDENTITY.slerp(up, 1.0 - legs) * Quat::from_rotation_z(side * comp[if side < 0.0 { 1 } else { 2 }] / 1.07);
                    Mat4::from_translation(root) * Mat4::from_quat(q) * Mat4::from_translation(-root)
                }
                Anim::MainWheel(side) => hinge(p, -self.wheel_rot[if side < 0.0 { 1 } else { 2 }] as f32),
                Anim::Yoke => Mat4::from_translation(Vec3::Z * pitch * 0.06) * hinge(p, -roll * 0.6),
                Anim::Pedal(side) => hinge(p, side * yaw * 0.2),
                Anim::Throttle => Mat4::from_translation(Vec3::Z * (1.0 - self.throttle as f32) * 0.06),
                Anim::GearHandle => hinge(p, if self.gear_down { 0.45 } else { -0.45 }),
                Anim::FlapLever => hinge(p, -0.35 + 0.7 * self.flap_set as f32 / 3.0),
                Anim::TrimWheel => hinge(p, self.trim as f32 * 9.0),
            };
            out.push(match p.parent {
                Some(i) => out[i] * local,
                None => local,
            });
        }
        out
    }

    // ---------------------------------------------------------------- breaking things

    fn note(&mut self, s: impl Into<String>) {
        self.notes.push(s.into());
    }
    fn scuff(&mut self, group: usize, amount: f32) {
        self.dmg.scuff[group] = (self.dmg.scuff[group] + amount).min(1.0);
    }
    fn kick(&mut self, w: DVec3) {
        self.omega += w;
    }
    fn wing_group(side: usize) -> usize {
        if side == 0 { G_LWING } else { G_RWING }
    }
    fn lr(side: usize) -> &'static str {
        if side == 0 { "LEFT" } else { "RIGHT" }
    }

    /// The wing on `side` (0 left) breaks off outboard of span `station`.
    fn break_wing(&mut self, side: usize, station: f32, why: &str) {
        self.snap_wing(side, station, 0.5, 0.0, true, why);
    }
    /// The wing on `side` breaks near span `station`, and how is left to the
    /// break: `rough` 0 is a clean shear (an edge struck hard), 1 skin torn
    /// apart, slanting across the chord and through the thickness; `fold` is
    /// the chance it buckles and hangs on by its skin rather than letting go
    /// (bending `up` or down). Breaks further in leave the bits between as
    /// pieces of their own.
    fn snap_wing(&mut self, side: usize, station: f32, rough: f32, fold: f32, up: bool, why: &str) {
        // A wing already hanging lets go before it breaks again.
        if self.dmg.fold[side].on {
            self.release_fold(side, "");
        }
        let station = station.max(0.3);
        if station >= self.dmg.wing[side] - 0.05 {
            return;
        }
        let sgn = if side == 0 { -1.0 } else { 1.0 };
        let r = &mut self.rng;
        let tz = r.range(-1.0, 1.0) * (0.12 + 0.6 * rough);
        let ty = r.range(-1.0, 1.0) * (0.04 + 0.22 * rough);
        let jag = (0.04 + 0.26 * rough) * r.range(0.6, 1.25);
        let folds = r.f() < fold;
        let hold = r.range(500.0, 2600.0);
        let prev = self.dmg.stump(side);
        let tear = damage::wing_break(side, station, tz, ty, jag, &prev);
        let tip = self.dmg.wing[side];
        let lost = tip - station;
        let tank = station < airframe::TANK[1] && tip >= airframe::TANK[1];
        self.dmg.wing[side] = station;
        self.dmg.wing_tear[side] = tear;
        self.scuff(Self::wing_group(side), 0.5);
        // On the ground the blow itself does the shoving.
        let (sg, lo) = (sgn as f64 * if self.on_hull || self.on_ground { 0.3 } else { 1.0 }, lost as f64);
        if folds && lost > 0.7 {
            self.dmg.fold[side] = damage::fold_at(side, station, tear, prev, tip, up, hold);
            // Still attached: less of a shove than losing it.
            self.kick(DVec3::new(0.15, -sg * 0.3 * lo, -sg * (0.4 + 0.4 * lo)));
            self.note(format!("{} WING BUCKLED ({why})", Self::lr(side)));
        } else {
            // The part that leaves: the far side of the break, with whatever flap
            // or aileron was riding on it, as one piece.
            let mut req = ShedReq::new(Sel::GroupAll(Self::wing_group(side)), tear.flip(), Vec3::new(sgn * 2.5, 1.5, 2.0), 5.0);
            req.tear2 = prev;
            req.own2 = true;
            req.rigid = true;
            self.shed.push(req);
            if station < airframe::AILERON[0] + 0.3 {
                self.dmg.aileron[side] = 0.0;
            }
            // Shoved: it rolls toward the stump and yaws into the drag of the break.
            self.kick(DVec3::new(0.3, -sg * 0.6 * lo, -sg * (0.8 + 0.9 * lo)));
            self.note(format!("{} WING TORN OFF ({why})", Self::lr(side)));
        }
        if tank {
            self.note(format!("{} FUEL TANK RUPTURED", Self::lr(side)));
        }
    }
    /// A buckled wing lets go of the stump.
    fn release_fold(&mut self, side: usize, why: &str) {
        let f = self.dmg.fold[side];
        if !f.on {
            return;
        }
        self.dmg.fold[side].on = false;
        let cm = f.mat().transform_point3(damage::wing_pt(side, (f.root + f.tip) * 0.5, 0.4, 0.0));
        let swing = f.axis.cross(cm - f.pivot) * f.rate;
        let mut req = ShedReq::new(Sel::GroupAll(Self::wing_group(side)), f.tear.flip(), swing + Vec3::new(0.0, 0.5, 1.5), 2.0 + f.rate.abs());
        req.tear2 = f.outer;
        req.own2 = true;
        req.rigid = true;
        req.pre = f.mat();
        self.shed.push(req);
        if f.root < airframe::AILERON[0] + 0.3 {
            self.dmg.aileron[side] = 0.0;
        }
        if !why.is_empty() {
            self.note(format!("{} WING TORN AWAY ({why})", Self::lr(side)));
        }
    }
    /// Let go of anything still hanging (the aircraft is about to go up in flames).
    fn release_folds(&mut self) {
        for s in 0..2 {
            self.release_fold(s, "");
        }
    }
    /// The tail breaks off through the tail cone, somewhere about `z`.
    fn snap_tail(&mut self, z: f32, why: &str) {
        if self.dmg.tail.is_some() {
            return;
        }
        let r = &mut self.rng;
        let tear = damage::tail_break(z, r.range(-0.35, 0.35), r.range(-0.45, 0.45), r.range(0.06, 0.24));
        let mut req = ShedReq::new(Sel::Group(G_TAIL), tear.flip(), Vec3::new(r.range(-1.0, 1.0), 1.5, 3.0), 4.0);
        req.rigid = true;
        self.shed.push(req);
        self.dmg.tail = tear;
        self.dmg.stab = [0.0; 2];
        self.dmg.elevator = [0.0; 2];
        self.dmg.fin = 0.0;
        self.dmg.rudder = 0.0;
        self.dmg.hull = (self.dmg.hull + 0.2).min(1.0);
        self.kick(DVec3::new(0.8, 0.0, 0.0));
        self.note(format!("TAIL BROKEN OFF ({why})"));
    }
    /// Something hits hard enough to explode: anything still hanging off goes too.
    fn impact(&mut self, imp: Impact, cause: &'static str) -> Option<Impact> {
        self.cause = cause;
        self.release_folds();
        Some(imp)
    }
    fn lose_aileron(&mut self, side: usize, hanging: bool, why: &str) {
        if self.dmg.aileron[side] <= 0.0 || (hanging && self.dmg.aileron[side] < 1.0) {
            return;
        }
        if hanging {
            self.dmg.aileron[side] = 0.5;
            self.note(format!("{} AILERON HINGE BROKEN ({why})", Self::lr(side)));
            return;
        }
        self.dmg.aileron[side] = 0.0;
        let name: &'static [&'static str] = if side == 0 { &["left aileron"] } else { &["right aileron"] };
        self.shed.push(ShedReq::new(Sel::Names(name), Tear::NONE, Vec3::new(0.0, 1.0, 3.0), 9.0));
        self.note(format!("{} AILERON TORN OFF ({why})", Self::lr(side)));
    }
    fn lose_flap(&mut self, side: usize, why: &str) {
        if self.dmg.flap[side] <= 0.0 {
            return;
        }
        self.dmg.flap[side] = 0.0;
        let name: &'static [&'static str] = if side == 0 { &["left flap"] } else { &["right flap"] };
        self.shed.push(ShedReq::new(Sel::Names(name), Tear::NONE, Vec3::new(0.0, -1.0, 4.0), 8.0));
        self.kick(DVec3::new(0.0, 0.0, if side == 0 { -0.4 } else { 0.4 }));
        self.note(format!("{} FLAP TORN OFF ({why})", Self::lr(side)));
    }
    fn lose_elevator(&mut self, side: usize, with_stab: bool, why: &str) {
        if with_stab && self.dmg.stab[side] > 0.5 {
            self.dmg.stab[side] = 0.0;
            self.dmg.elevator[side] = 0.0;
            let sgn = if side == 0 { -1.0 } else { 1.0 };
            self.shed.push(ShedReq::new(Sel::Names(&["stabiliser", "elevator"]), Tear::new(Vec3::new(sgn, 0.0, 0.0), 0.34, -1.0), Vec3::new(sgn * 2.0, 0.5, 2.0), 6.0));
            self.kick(DVec3::new(-0.5, 0.0, sgn as f64 * 0.3));
            self.note(format!("{} STABILISER TORN OFF ({why})", Self::lr(side)));
        } else if self.dmg.elevator[side] > 0.5 {
            self.dmg.elevator[side] = 0.0;
            let sgn = if side == 0 { -1.0 } else { 1.0 };
            self.shed.push(ShedReq::new(Sel::Names(&["elevator"]), Tear::new(Vec3::new(sgn, 0.0, 0.0), 0.12, -1.0), Vec3::new(sgn, 1.0, 3.0), 9.0));
            self.note(format!("{} ELEVATOR TORN OFF ({why})", Self::lr(side)));
        }
    }
    /// The top of the rudder (and perhaps the fin with it) goes.
    fn lose_tail_top(&mut self, rudder: f32, fin: f32, why: &str) {
        let rudder = rudder.min(self.dmg.rudder);
        let fin = fin.min(self.dmg.fin);
        if rudder < self.dmg.rudder - 0.01 {
            if rudder <= 0.0 {
                self.shed.push(ShedReq::new(Sel::Names(&["rudder"]), Tear::NONE, Vec3::new(1.0, 1.0, 3.0), 7.0));
                self.note(format!("RUDDER TORN OFF ({why})"));
            } else {
                self.shed.push(ShedReq::new(Sel::Names(&["rudder"]), Tear::new(Vec3::Y, 0.22 + 1.57 * rudder, -1.0), Vec3::new(1.0, 1.5, 3.0), 8.0));
                self.note(format!("RUDDER DAMAGED ({why})"));
            }
            self.dmg.rudder = rudder;
        }
        if fin < self.dmg.fin - 0.01 {
            self.shed.push(ShedReq::new(Sel::Names(&["stabiliser"]), Tear::new(Vec3::Y, 0.27 + 1.53 * fin, -1.0), Vec3::new(1.0, 2.0, 2.0), 6.0));
            self.dmg.fin = fin;
            self.note(format!("FIN BROKEN ({why})"));
        }
    }
    /// The propeller hits something solid.
    fn prop_strike(&mut self, why: &str) {
        if self.dmg.prop <= 0.0 {
            return;
        }
        let spinning = self.prop_rate > 40.0;
        if self.dmg.prop > 0.75 {
            self.dmg.prop = 0.5;
            self.shed.push(ShedReq::new(Sel::Names(&["blades"]), Tear::new(Vec3::Y, airframe::PROP_HUB.y + 0.42, -1.0), Vec3::new(2.0, 4.0, -3.0), 30.0));
            self.note(format!("PROPELLER BLADE BROKEN ({why})"));
        }
        if spinning && self.dmg.engine > 0.0 {
            // A sudden stop wrecks the crankshaft.
            self.dmg.engine = 0.0;
            self.dmg.smoke_black = true;
            self.note("ENGINE SEIZED");
        }
        self.scuff(G_ENGINE, 0.6);
    }
    fn engine_hit(&mut self, amount: f32, why: &str) {
        if self.dmg.engine <= 0.0 {
            return;
        }
        let was = self.dmg.engine;
        self.dmg.engine = (self.dmg.engine - amount).max(0.0);
        if was >= 1.0 {
            self.note(format!("ENGINE DAMAGED, LOSING OIL ({why})"));
        }
    }
    fn crack_windshield(&mut self, why: &str) {
        if self.dmg.crack[2] > 0.0 {
            return;
        }
        self.dmg.crack = [self.rng.range(-1.55, -1.25), self.rng.range(0.3, 0.6), self.rng.range(0.14, 0.3)];
        self.note(format!("WINDSHIELD CRACKED ({why})"));
    }
    fn lose_gear(&mut self, gi: usize, why: &str) {
        if !self.dmg.gear[gi] {
            return;
        }
        self.dmg.gear[gi] = false;
        self.shed.push(ShedReq::new(Sel::Names(damage::GEAR_PIECES[gi]), Tear::NONE, Vec3::new(0.0, -1.0, 2.0), 6.0));
        self.note(format!("{} TORN OFF ({why})", ["NOSE GEAR", "LEFT MAIN GEAR", "RIGHT MAIN GEAR"][gi]));
    }
    /// Above the never-exceed speed something lets go: whatever is next.
    fn flutter_loss(&mut self) {
        let r = self.rng.f();
        let s = (r * 2.0) as usize;
        if self.dmg.aileron[s] > 0.0 {
            self.lose_aileron(s, r > 0.6, "FLUTTER");
        } else if self.dmg.aileron[1 - s] > 0.0 {
            self.lose_aileron(1 - s, false, "FLUTTER");
        } else if self.dmg.rudder > 0.5 {
            self.lose_tail_top(0.45, 1.0, "FLUTTER");
        } else if self.dmg.elevator[s] > 0.5 {
            self.lose_elevator(s, false, "FLUTTER");
        } else if self.dmg.elevator[1 - s] > 0.5 {
            self.lose_elevator(1 - s, false, "FLUTTER");
        } else {
            self.break_wing(s, 4.3, "FLUTTER");
        }
    }

    /// Velocity through the air (planet frame).
    pub fn airspeed(&self) -> DVec3 {
        self.vel - self.wind - self.gust
    }

    /// The torque the engine's cylinders make (N m, before friction) with the
    /// throttle `open` (0 at the idle stop .. 1). A normally aspirated engine
    /// loses power with the air (Gagg-Ferrar); `boost` is the game's
    /// over-boost, nothing a real 172 has.
    fn engine_torque(open: f64, rho: f64, boost: bool, health: f64) -> f64 {
        let sigma = rho / RHO0;
        let alt = ((sigma - 0.12) / 0.88).max(0.0);
        (POWER / W_RATED + Self::friction(W_RATED)) * alt * (0.11 + 0.89 * open.clamp(0.0, 1.0)) * health * if boost { 1.35 } else { 1.0 }
    }
    /// Friction and pumping losses (N m) at shaft speed `w` (rad/s).
    fn friction(w: f64) -> f64 {
        15.0 + 35.0 * (w / W_RATED).powi(2)
    }
    /// The fixed-pitch propeller turning at `w` rad/s with the air coming at
    /// it at `v`: its thrust (N) and the torque it takes (N m; negative when
    /// the air drives it round). `k` is what is left of the blades.
    /// Ct = Ct0 (1 - (J/Jt)²), Cp = Cp0 (1 - (J/Jp)²), written in n² so a
    /// stopped propeller needs no special case.
    pub fn prop_load(w: f64, v: f64, rho: f64, k: f64) -> (f64, f64) {
        let n = w / std::f64::consts::TAU;
        let vd = v.max(0.0) / PROP_D;
        let t = CT0 * rho * PROP_D.powi(4) * (n * n - (vd / J_T).powi(2));
        let q = CP0 * rho * PROP_D.powi(5) / std::f64::consts::TAU * (n * n - (vd / J_P).powi(2));
        // Windmilling, or stopped, it drags no worse than a flat plate the size of its blades.
        let t = t.max(-0.28 * 0.5 * rho * v * v);
        (t * k, q * k)
    }
    /// The revs (rad/s) an undamaged engine and propeller settle at.
    pub fn prop_equilibrium(throttle: f64, v: f64, rho: f64) -> f64 {
        let (mut lo, mut hi) = (0.0, 2.0 * W_RATED);
        for _ in 0..50 {
            let w = 0.5 * (lo + hi);
            if Self::engine_torque(throttle, rho, false, 1.0) - Self::friction(w) > Self::prop_load(w, v, rho, 1.0).1 { lo = w } else { hi = w }
        }
        0.5 * (lo + hi)
    }
    /// Thrust (N) once the revs have settled.
    pub fn thrust(throttle: f64, v: f64, rho: f64) -> f64 {
        Self::prop_load(Self::prop_equilibrium(throttle, v, rho), v, rho, 1.0).0
    }
    /// Angle of attack (rad) that carries the weight at true airspeed `v`.
    fn alpha_level(v: f64, rho: f64, flap_cl: f64) -> f64 {
        let q = 0.5 * rho * v * v;
        ((MASS * 9.81 / (q * WING_AREA).max(1.0) - CL0 - flap_cl) / CL_A).clamp(-0.05, 0.24)
    }
    /// Drag (N) in steady flight at true airspeed `v`.
    fn drag_level(v: f64, rho: f64, flaps: f64, gear: f64) -> f64 {
        let qs = 0.5 * rho * v * v * WING_AREA;
        let cl = MASS * 9.81 / qs.max(1.0);
        qs * (CD0 + K_IND * cl * cl + 0.02 * flaps + 0.06 * flaps * flaps + CD_GEAR * gear)
    }
    /// Throttle to fly level at `v`.
    fn level_throttle(v: f64, rho: f64, flaps: f64, gear: f64) -> f64 {
        Self::throttle_for(v, rho, flaps, gear, 0.0)
    }
    /// Throttle to hold `v` while coming down at `descent` m/s.
    fn throttle_for(v: f64, rho: f64, flaps: f64, gear: f64, descent: f64) -> f64 {
        let need = Self::drag_level(v, rho, flaps, gear) - MASS * 9.81 * (descent / v.max(1.0));
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..30 {
            let t = 0.5 * (lo + hi);
            if Self::thrust(t, v, rho) < need { lo = t } else { hi = t }
        }
        0.5 * (lo + hi)
    }

    /// Gaussian noise, unit variance (sum of three uniforms: close enough, and cheap).
    fn gauss(&mut self) -> f64 {
        (self.rng.range(-1.0, 1.0) + self.rng.range(-1.0, 1.0) + self.rng.range(-1.0, 1.0)) as f64
    }

    /// Turbulence: white noise through two first-order lags in series (a
    /// smoothed Dryden spectrum), the length scales turned into time by the
    /// airspeed. `a` is the coarse state, `b` the smoothed one; the second lag
    /// takes a fifth of the variance, which the output scale gives back.
    fn turbulence(&mut self, dt: f64, v: f64, upw: DVec3) {
        let air = self.air;
        let vt = v.max(12.0);
        let len = DVec3::new(air.len_h, air.len_h, air.len_w);
        let sig = DVec3::new(air.sigma_h, air.sigma_h, air.sigma_w);
        let n = DVec3::new(self.gauss(), self.gauss(), self.gauss());
        let e1 = (-vt * dt / len).exp();
        let e2 = (-vt * dt * 4.0 / len).exp();
        let one = DVec3::ONE;
        self.gust_a = self.gust_a * e1 + sig * (one - e1 * e1).max(DVec3::ZERO).sqrt() * n;
        self.gust_b = self.gust_b * e2 + self.gust_a * (one - e2);
        let g = self.gust_b * 1.118;
        // Along and across the flight path, and up.
        let air_v = self.vel - self.wind;
        let mut along = air_v - upw * air_v.dot(upw);
        if along.length_squared() < 1.0 {
            along = self.forward() - upw * self.forward().dot(upw);
        }
        let along = along.normalize_or_zero();
        let across = along.cross(upw);
        self.gust = along * g.x + across * g.y + upw * g.z;
        // Rotational gusts: the vertical gust differs from one wing tip to the
        // other (roll), from the nose to the tail (pitch), and the side gust
        // from the nose to the fin (yaw). Eddies about the size of the airframe.
        let lr = B * 2.5;
        let sr = DVec3::new(0.012 * air.sigma_w, 0.020 * air.sigma_h, 0.03 * air.sigma_w);
        let n = DVec3::new(self.gauss(), self.gauss(), self.gauss());
        let e1 = (-vt * dt / lr).exp();
        let e2 = (-vt * dt * 4.0 / lr).exp();
        self.rot_a = self.rot_a * e1 + sr * (1.0 - e1 * e1).max(0.0).sqrt() * n;
        self.rot_b = self.rot_b * e2 + self.rot_a * (1.0 - e2);
        self.gust_rot = self.rot_b * 1.118 * smooth(5.0, 25.0, v);
    }

    /// Advance one physics step. Returns an impact if the aircraft was destroyed.
    /// Points on the airframe that meet the ground (body frame), as it is now:
    /// what has broken off has no points left, a hanging wing has its own.
    fn contacts(&self, out: &mut Vec<(DVec3, Part)>) {
        let d = &self.dmg;
        let v = DVec3::new;
        if d.prop > 0.0 {
            out.push((HARD[0], Part::Prop));
            out.push((HARD[6], Part::Prop));
        }
        out.extend([
            (v(0.0, -0.035, -3.17), Part::Nose),
            (v(0.0, -0.60, -2.6), Part::Nose),
            (v(0.0, 0.31, -2.45), Part::Nose),
            (v(-0.5, -0.1, -2.6), Part::Nose),
            (v(0.5, -0.1, -2.6), Part::Nose),
            (v(0.0, -0.72, -1.5), Part::Belly),
            (HARD[5], Part::Belly),
            (v(0.0, -0.70, 0.9), Part::Belly),
            (v(-0.58, 0.05, -1.2), Part::Side),
            (v(0.58, 0.05, -1.2), Part::Side),
            (v(-0.58, 0.05, 0.4), Part::Side),
            (v(0.58, 0.05, 0.4), Part::Side),
            (v(0.0, 0.52, -1.35), Part::Roof),
            (v(0.0, 0.50, 1.6), Part::Roof),
        ]);
        if d.tail.is_some() {
            // The stump of the tail cone, a little ahead of the break.
            let z = (d.tail.d / d.tail.n.z.max(0.5)) as f64 - 0.15;
            let s = airframe::sec(z as f32);
            out.push((v(0.0, s.bot as f64, z), Part::Belly));
            out.push((v(0.0, s.top as f64, z), Part::Roof));
        } else {
            out.extend([(v(0.0, -0.45, 2.4), Part::Tail), (v(0.0, -0.12, 3.6), Part::Tail), (HARD[3], Part::Tail), (v(0.0, 0.34, 3.2), Part::Tail)]);
            if d.fin > 0.05 {
                out.push((v(0.0, 0.27 + 1.53 * d.fin as f64, 4.1 + 0.25 * (1.0 - d.fin as f64)), Part::Fin));
                out.push((v(0.0, 0.27 + 0.8 * d.fin as f64, 4.2), Part::Fin));
            }
            for s in 0..2 {
                if d.stab[s] > 0.5 {
                    out.push((v(if s == 0 { -1.65 } else { 1.65 }, 0.11, 4.15), Part::Stab(s)));
                }
            }
        }
        for s in 0..2 {
            let b = d.wing[s];
            let mut at = |st: f32, tip: bool| {
                for (xc, skin) in [(0.35, 1.0), (0.35, -1.0), (0.0, 0.0)] {
                    out.push((damage::wing_pt(s, st, xc, skin).as_dvec3(), Part::Wing(s, st)));
                }
                if tip {
                    out.push((damage::wing_pt(s, st, 0.92, 0.0).as_dvec3(), Part::Wing(s, st)));
                }
            };
            for st in [0.45f32, 1.5, 2.6, 3.7, 4.7] {
                if st < b - 0.3 {
                    at(st, false);
                }
            }
            if b > 0.4 {
                at(b - 0.08, true);
            }
            let f = &d.fold[s];
            if f.on {
                let m = f.mat();
                for (st, skin) in [(f.tip - 0.08, 1.0), (f.tip - 0.08, -1.0), ((f.root + f.tip) * 0.5, -1.0), ((f.root + f.tip) * 0.5, 1.0)] {
                    out.push((m.transform_point3(damage::wing_pt(s, st, 0.4, skin)).as_dvec3(), Part::Fold(s)));
                }
            }
        }
    }

    /// Wrecked: it will not fly again (a wing or the tail gone, the cabin
    /// caved in, or lying on its back).
    pub fn wrecked(&self) -> bool {
        let d = &self.dmg;
        d.wing[0] < 3.2 || d.wing[1] < 3.2 || d.fold.iter().any(|f| f.on) || d.tail.is_some() || d.hull > 0.6 || self.sink > 0.3 || (self.on_hull && self.up().dot(self.pos.normalize()) < 0.2)
    }

    pub fn step(&mut self, dt: f64, c: &Controls, world: &World) -> Option<Impact> {
        self.t += dt;
        if self.hold {
            self.vel = self.wind * 0.0;
            self.omega = DVec3::ZERO;
            self.prop_rate += ((60.0 + 185.0 * self.throttle) * 0.0 + 65.0 - self.prop_rate) * (dt * 2.0).min(1.0);
            self.prop += dt * self.prop_rate;
            return None;
        }
        self.dmg_prev = self.dmg;
        self.dmg.vibe = 0.0;
        for c in self.cool.iter_mut() {
            *c -= dt;
        }
        self.fold_tq = [0.0; 2];
        self.throttle = (self.throttle + c.throttle * dt * 0.6).clamp(0.0, 1.0);
        // Control surfaces do not snap.
        let target = DVec3::new(c.pitch, c.yaw, c.roll);
        self.stick += (target - self.stick) * (dt * 7.0).min(1.0);
        // Flaps run on an electric motor, the gear on a hydraulic pump; the
        // squat switch keeps the gear down while there is weight on the wheels.
        let flap_target = self.flap_set as f64 / 3.0;
        self.flaps += (flap_target - self.flaps).clamp(-FLAP_RATE * dt, FLAP_RATE * dt);
        if !self.gear_down && self.on_ground {
            self.gear_down = true;
        }
        self.gear = (self.gear + if self.gear_down { GEAR_RATE * dt } else { -GEAR_RATE * dt }).clamp(0.0, 1.0);

        let (lat, lon, h) = geo::pos_to_lla(self.pos);
        let upw = self.pos.normalize();
        let rho = density(h);
        // Limits are set in indicated airspeed (the dynamic pressure the airframe feels).
        let eas_k = (rho / RHO0).sqrt();
        let ground = world.ground(lat, lon);
        // The turbulent air the aircraft is flying through this instant.
        let v_air = (self.vel - self.wind).length();
        self.turbulence(dt, v_air, upw);
        let vb = self.rot.inverse() * self.airspeed();
        let v = vb.length();
        let mut force_b = DVec3::ZERO;
        let mut wdot = DVec3::ZERO;
        let q = 0.5 * rho * v * v;

        // What is left to fly with.
        let dm = self.dmg;
        let lost = [Damage::wing_lost(dm.wing[0]) as f64, Damage::wing_lost(dm.wing[1]) as f64];
        let area = (WING_AREA - lost[0] - lost[1]).max(4.0);
        let ail = [dm.aileron[0] * Damage::aileron_frac(dm.wing[0]), dm.aileron[1] * Damage::aileron_frac(dm.wing[1])];
        // A hanging aileron does nothing for you.
        let roll_auth = ail.iter().map(|&a| if a < 0.99 && a > 0.0 { 0.0 } else { a as f64 }).sum::<f64>() * 0.5;
        let flap_eff = (dm.flap[0] + dm.flap[1]) as f64 * 0.5;
        let el = (dm.elevator[0] * dm.stab[0] + dm.elevator[1] * dm.stab[1]) as f64 * 0.5;
        let st = (dm.stab[0] + dm.stab[1]) as f64 * 0.5;
        let fin = (dm.fin * 0.6 + dm.rudder * 0.4) as f64;
        let rud = dm.rudder as f64;

        if v > 0.5 {
            let vh = vb / v;
            let alpha = (-vb.y).atan2(-vb.z);
            let beta = (vb.x / v).clamp(-1.0, 1.0).asin();
            self.alpha = alpha;
            // Flaps add lift (and stall a little earlier); flaps and gear add drag.
            let flap_cl = 0.55 * self.flaps * flap_eff;
            let stall_a = self.stall_alpha();
            let stall = smooth(stall_a, stall_a + 0.13, alpha.abs());
            self.stalled = stall > 0.5 && v > 8.0;
            // Ground effect: within a span of the ground the wing tip vortices are
            // squashed, so there is less induced drag and a little more lift (the
            // float in the flare).
            let ge = match &ground {
                Some(g) if !self.on_ground || v > 5.0 => {
                    let r = 16.0 * ((h - g.h - 0.5).max(0.2) / B);
                    r * r / (1.0 + r * r)
                }
                _ => 1.0,
            };
            let cl = ((CL0 + flap_cl + CL_A * alpha) * (1.0 - stall) + (2.0 * alpha).sin() * 0.95 * stall) * (1.0 + 0.12 * (1.0 - ge));
            // Ragged stumps and missing fairings all drag.
            let rag = ((SPAN - dm.wing[0]) + (SPAN - dm.wing[1])) as f64;
            // A strut-braced high wing: Cd0 ~0.034 cleaned up, induced drag for an
            // aspect ratio of 7.5; the big slotted flaps are mostly drag past 20 degrees.
            let cd = CD0 + K_IND * cl * cl * ge + stall * 1.2 * alpha.sin().powi(2) + 0.6 * beta * beta + 0.02 * self.flaps + 0.06 * self.flaps * self.flaps + CD_GEAR * self.gear + 0.02 * rag + 0.03 * (1.0 - st) + 0.01 * (1.0 - rud);
            // Buffet as the airflow starts to break away over the wing, before it stalls.
            self.dmg.vibe += (smooth(stall_a - 0.07, stall_a + 0.03, alpha.abs()) * (1.0 - 0.5 * stall) * 0.9 * (q / 1500.0).min(1.0)) as f32;
            let lift_dir = DVec3::X.cross(vh).normalize_or_zero();
            force_b += lift_dir * (q * area * cl) - vh * (q * area * cd) + DVec3::X * (-0.9 * beta * q * WING_AREA);

            // Direct angular accelerations, scaled by dynamic pressure; the
            // damping of a rotation grows only with airspeed (it comes from the
            // angle the rotation adds at the wing tip or tail, ω·arm / V), so a
            // slow aircraft is sluggish and a fast one crisp.
            let a = (q / 2200.0).clamp(0.0, 2.6);
            let ad = (a * 68.0 / v.max(20.0)).clamp(0.3, 2.6);
            // Relative to the air, which is itself turning in the eddies.
            let om = self.omega - self.gust_rot;
            // Auto-trim: while the pilot is flying the stick it winds the trim to
            // the angle that carries the weight at this speed; let go and it stays
            // put, so the aircraft holds the airspeed it was trimmed for, as a real
            // one does: power then makes it climb or descend, and a bump sets it
            // porpoising gently back to that speed. Without an elevator the trim
            // stays where it was.
            let level = self.up().dot(upw).max(0.0);
            let alpha_1g = ((MASS * 9.81 * level / (q * area).max(1.0) - CL0 - flap_cl) / CL_A).clamp(-0.05, 0.24);
            if el > 0.05 && self.stick.x.abs() > 0.06 {
                self.trim += (alpha_1g - self.trim) * (dt * 0.5).min(1.0);
            }
            let alpha_cmd = self.trim + self.stick.x * 0.26 * el;
            // Pitch stiffness comes from the tailplane; without one the nose wanders off.
            let stiff = 7.0 * st - 2.0 * (1.0 - st);
            wdot.x = a * (stiff * (alpha_cmd - alpha) + self.stick.x * 0.6 * el) - 3.2 * ad * (0.3 + 0.7 * st) * om.x;
            // Roll damping fades in a stall, and past it turns round: the wing going
            // down meets the air at a still higher angle and loses still more lift.
            let roll_damp = (1.0 - 1.5 * stall).max(-0.35);
            wdot.z = a * (-4.0 * self.stick.z * roll_auth - 0.8 * beta) - 4.0 * ad * roll_damp * om.z;
            wdot.y = a * (-1.3 * self.stick.y * rud - 4.5 * beta * fin) - 2.4 * ad * (0.25 + 0.75 * fin) * om.y;
            // Dihedral: a banked aircraft slips toward the low wing, which then
            // meets the air at a higher angle and lifts it back, slowly (but not upside down).
            wdot.z -= a * 0.3 * self.right().dot(upw) * self.up().dot(upw).max(0.0);
            // Adverse yaw: the aileron that goes down drags its wing back.
            wdot.y += a * 0.35 * self.stick.z * roll_auth * cl.clamp(0.0, 1.6);
            // A stalled wing drops the nose, and one wing goes first: which one
            // wanders, and a yawing aircraft drops the wing that is going back (a spin).
            wdot.x -= stall * 0.9 * alpha.signum();
            self.drop = (self.drop + (self.gauss() * 1.2 - self.drop * 0.4) * dt).clamp(-1.0, 1.0);
            wdot.z += stall * a.max(0.25) * (1.8 * self.drop + 2.5 * om.y);
            // A wing that ends short of the tip lifts less than its opposite: it
            // drops, and the drag of its ragged end pulls the nose round.
            let arm = |b: f32| ((b + SPAN) * 0.5) as f64;
            let roll = (lost[0] * arm(dm.wing[0]) - lost[1] * arm(dm.wing[1])) * q * (cl.max(0.05) + 0.1) * 0.6 / INERTIA.z;
            wdot.z += roll;
            wdot.y -= ((SPAN - dm.wing[1]) - (SPAN - dm.wing[0])) as f64 * q * 0.34 / INERTIA.y;
            // One flap gone with the other out: the lift is lopsided (retract them!).
            wdot.z += (dm.flap[1] - dm.flap[0]) as f64 * self.flaps * q * WING_AREA * 0.275 * 1.8 * 0.35 / INERTIA.z;
            // A hanging aileron droops (lifting that wing) and flaps about.
            for s in 0..2 {
                if dm.aileron[s] > 0.0 && dm.aileron[s] < 1.0 {
                    let sgn = if s == 0 { -1.0 } else { 1.0 };
                    wdot.z += sgn * a * (0.45 + 0.35 * (self.t * 23.0 + s as f64).sin());
                    self.dmg.vibe += 0.12 * a as f32;
                }
            }
        }
        // Engine and propeller. A damaged engine leaks its oil away and fails;
        // a snapped blade shakes it to pieces; with no blades it just windmills.
        if self.dmg.engine < 1.0 && self.dmg.engine > 0.0 {
            self.dmg.engine = (self.dmg.engine - dt as f32 * (0.002 + 0.01 * (1.0 - self.dmg.engine))).max(0.0);
            if self.dmg.engine <= 0.0 {
                self.note("ENGINE FAILED");
            }
        }
        let prop_k = if dm.prop > 0.75 { 1.0 } else if dm.prop > 0.25 { 0.45 } else { 0.0 };
        let eng = (self.dmg.engine as f64).powf(0.6);
        // The engine takes a moment to answer the throttle.
        self.charge += (self.throttle - self.charge) * (dt * 4.0).min(1.0);
        // The fixed-pitch propeller: the engine's torque against what the
        // blades take at these revs and this airspeed. Faster through the air,
        // the blades meet it at a smaller angle and take less, so the revs
        // rise (and over-rev in a dive); slow, the propeller holds the engine
        // well below its rated revs and power. Without power it windmills, and
        // only slowing nearly to the stall stops it.
        let w = self.prop_rate.max(0.0);
        let v_ax = (-vb.z).max(0.0);
        let (mut thrust, q_prop) = Self::prop_load(w, v_ax, rho, prop_k);
        let q_eng = Self::engine_torque(self.charge, rho, c.boost, eng);
        let drive = q_eng - q_prop;
        if prop_k <= 0.0 {
            // No blades left: nothing to turn.
            self.prop_rate += (0.0 - self.prop_rate) * (dt * 2.0).min(1.0);
        } else if w > 1.0 || drive > 40.0 {
            self.prop_rate = (w + (drive - Self::friction(w)) / PROP_I * dt).max(0.0);
        } else {
            // Stopped: the compression holds it until the air pushes harder.
            self.prop_rate = 0.0;
        }
        self.power = ((q_eng - Self::friction(w)) * w / POWER).max(0.0);
        if dm.prop <= 0.25 {
            thrust = 0.0;
        }
        force_b += DVec3::NEG_Z * thrust;
        // The propeller turns clockwise seen from the cockpit: its torque rolls the
        // aircraft left and its spiralling slipstream (and P-factor, climbing
        // slowly) yaws it left. The fin and ailerons are rigged for the cruise, so
        // it is full power at low speed that needs right rudder.
        if v > 2.0 {
            let slow = (1.0 - v / 60.0).max(0.0);
            let tk = thrust.max(0.0) / STATIC_THRUST;
            wdot.y += tk * slow * (0.22 + 0.25 * self.alpha.max(0.0));
            wdot.z += tk * slow * 0.12 * if self.on_ground { 0.3 } else { 1.0 };
        }
        self.prop += dt * self.prop_rate;
        if dm.prop > 0.25 && dm.prop < 0.75 {
            let spin = (self.prop_rate / 250.0) as f32;
            self.dmg.vibe += spin * 1.4;
            if self.dmg.engine > 0.0 {
                self.dmg.engine = (self.dmg.engine - dt as f32 * 0.06 * spin).max(0.0);
            }
        }
        if self.dmg.engine < 1.0 && self.dmg.engine > 0.0 {
            self.dmg.oil = (self.dmg.oil + dt as f32 * 0.04 * (1.0 - self.dmg.engine) * (self.prop_rate / 250.0) as f32).min(1.0);
        }
        self.dmg.smoke = if self.dmg.engine < 1.0 { (1.0 - self.dmg.engine) * (0.3 + 0.7 * (self.prop_rate / 250.0) as f32) * if self.dmg.engine <= 0.0 { 0.5 } else { 1.0 } } else { 0.0 };

        // Ground contact.
        let mut force_w = self.rot * force_b - upw * (MASS * 9.81);
        let mut torque_b = DVec3::ZERO;
        self.on_ground = false;
        // Nothing is solid for the first moments after a spawn: the terrain
        // under the aircraft is still a coarse guess until its tile arrives.
        let close = self.t > 1.5 && (ground.as_ref().map(|g| h - g.h < 60.0).unwrap_or(false) || world.building(lat, lon).is_some());
        self.brushes = (self.brushes - dt * 0.5).max(0.0);
        let mut skidding = false;
        if close {
            if let Some(imp) = self.trees(world) {
                return self.impact(imp, "tree");
            }
            let hit = |p: DVec3, world: &World| -> Option<(f64, bool, Option<(geo::TileId, usize)>)> {
                let (la, lo, ph) = geo::pos_to_lla(p);
                let g = world.ground(la, lo)?;
                if let Some(b) = world.building(la, lo) {
                    if ph < b.top && ph > b.base - if b.bridge { 0.0 } else { 3.0 } {
                        return Some((b.top - ph, false, Some((b.tile, b.idx))));
                    }
                } else if ph < g.h {
                    // Under the 3D tiles the ground is the mesh, buildings and all:
                    // into one of them, it is that building that was hit.
                    if let Some(b) = world.photo_building(la, lo, g.h) {
                        if ph > b.base - 3.0 {
                            return Some((g.h - ph, false, Some((b.tile, b.idx))));
                        }
                    }
                }
                Some((g.h - ph, g.water, None))
            };
            // Every point of the airframe that can touch something, and what it
            // takes to break it: gentle enough, things only scrape; harder, the
            // part that hit comes off (a wing, the tail, the gear); only a hit
            // hard enough to burst the cabin or the tanks ends in fire.
            let mut pts = std::mem::take(&mut self.pts);
            pts.clear();
            self.contacts(&mut pts);
            let mut walled = false;
            for &(r, part) in &pts {
                let p = self.pos + self.rot * r;
                let Some((mut pen, water, building)) = hit(p, world) else { continue };
                if water {
                    // A ditched aircraft settles lower in the water as it floods.
                    pen -= self.sink;
                }
                if pen <= 0.0 {
                    continue;
                }
                let mut pv = self.vel + self.rot * self.omega.cross(r);
                if let Part::Fold(s) = part {
                    let f = &self.dmg.fold[s];
                    pv += self.rot * (f.axis.cross(r.as_vec3() - f.pivot) * f.rate).as_dvec3();
                }
                let vn = pv.dot(upw);
                let closing = (-vn).max(0.0);
                let sp = pv.length();
                let crash = Impact { pos: p, vel: self.vel, water, ground_h: geo::pos_to_lla(p).2 + if building.is_some() { 0.0 } else { pen }, building };
                let up_right = self.up().dot(upw) > 0.0;
                if building.is_some() && pen > 1.0 {
                    // Into the side of a building: all of the speed goes into it.
                    match part {
                        Part::Wing(s, st) => {
                            // A wing's root goes in with the cabin right behind it.
                            if st < 1.8 && sp > 28.0 {
                                return self.impact(crash, "wing root into building");
                            }
                            if self.cool[s] <= 0.0 {
                                let into = self.rng.range(0.1, 0.6) + (sp as f32 / 30.0).min(2.0) * self.rng.f();
                                let (rough, up) = (self.rng.range(0.0, 0.5), self.rng.f() < 0.5);
                                self.snap_wing(s, st - into, rough, if sp < 25.0 { 0.35 } else { 0.1 }, up, "BUILDING");
                                self.cool[s] = 0.25;
                                self.vel *= 0.85;
                            }
                        }
                        Part::Fold(s) => self.release_fold(s, "BUILDING"),
                        Part::Fin => self.lose_tail_top(0.0, 0.3, "BUILDING"),
                        Part::Stab(s) => self.lose_elevator(s, true, "BUILDING"),
                        Part::Tail if sp > 12.0 => {
                            let z = self.rng.range(1.5, 2.6);
                            self.snap_tail(z, "BUILDING");
                        }
                        _ if sp > 18.0 => return self.impact(crash, "building"),
                        _ if !walled => {
                            // Taxiing into a wall: crumpled, stopped and pushed back off it.
                            walled = true;
                            if matches!(part, Part::Prop | Part::Nose) {
                                self.prop_strike("BUILDING");
                                if sp > 8.0 {
                                    self.engine_hit(0.35, "BUILDING");
                                    self.crack_windshield("BUILDING");
                                }
                            }
                            self.dmg.hull = (self.dmg.hull + sp as f32 * 0.03).min(1.0);
                            self.scuff(part.group(), 0.6);
                            let back = (self.vel - upw * self.vel.dot(upw)).normalize_or_zero();
                            self.pos -= back * 0.25;
                            self.vel = -self.vel * 0.15;
                            self.omega *= 0.4;
                            self.dmg.vibe += 2.0;
                        }
                        _ => {}
                    }
                    continue;
                }
                // Ground, a roof or the water. First what this hit breaks.
                let nose_on = matches!(part, Part::Nose | Part::Prop);
                let body = matches!(part, Part::Belly | Part::Side | Part::Roof);
                if water && (nose_on || body) && (closing > 9.0 || (sp > 42.0 && closing > 4.0)) {
                    return self.impact(crash, "water");
                }
                match part {
                    Part::Prop | Part::Nose => {
                        if self.dmg.prop > 0.25 && (part == Part::Prop || self.prop_rate > 30.0) && (self.prop_rate > 5.0 || sp > 3.0) {
                            self.prop_strike(if water { "WATER" } else { "GROUND" });
                        }
                        // Nose first into the ground: the engine is driven back into the cabin.
                        if closing > 15.0 || (sp > 50.0 && closing > 7.5) {
                            return self.impact(crash, "water");
                        }
                        if closing > 6.0 && self.cool[3] <= 0.0 {
                            self.cool[3] = 0.3;
                            self.engine_hit(0.15 + (closing as f32 - 6.0) * 0.05, "NOSE IMPACT");
                            self.crack_windshield("NOSE IMPACT");
                            self.dmg.hull = (self.dmg.hull + (closing as f32 - 6.0) * 0.04).min(1.0);
                            if closing > 8.5 {
                                self.lose_gear(0, "NOSE IMPACT");
                            }
                        }
                    }
                    Part::Belly | Part::Side | Part::Roof => {
                        // The cabin can take a hard landing on its belly; on its side or
                        // its roof it caves in sooner. Fast and steep, nothing survives.
                        let lim = if part == Part::Belly { 15.0 } else { 12.0 };
                        if closing > lim || (sp > 55.0 && closing > lim * 0.55) {
                            return self.impact(crash, "cabin");
                        }
                        if closing > 4.5 && self.cool[3] <= 0.0 {
                            self.cool[3] = 0.3;
                            self.dmg.hull = (self.dmg.hull + (closing as f32 - 4.5) * 0.05).min(1.0);
                            self.dmg.vibe += 3.0;
                            if closing > 6.5 || part != Part::Belly {
                                self.crack_windshield(if part == Part::Roof { "ROLLED OVER" } else { "HARD IMPACT" });
                            }
                            if part == Part::Belly && closing > 7.0 {
                                for gi in 0..3 {
                                    if self.gear > 0.5 {
                                        self.lose_gear(gi, "HARD IMPACT");
                                    }
                                }
                            }
                        }
                    }
                    Part::Tail => {
                        // A tail slammed down (or dug in sliding fast) snaps off the cone.
                        if closing > 7.5 || (closing > 4.0 && sp > 25.0) {
                            let z = self.rng.range(1.4, 2.7);
                            self.snap_tail(z, "GROUND");
                            continue;
                        }
                    }
                    Part::Fin => {
                        // Scraped along the ground upside down, the fin wears away.
                        if self.cool[2] <= 0.0 && (closing > 2.5 || (sp > 7.0 && pen > 0.04)) {
                            self.cool[2] = 0.35;
                            let fin = (self.dmg.fin - self.rng.range(0.2, 0.5)).max(0.0);
                            self.lose_tail_top((self.dmg.rudder - 0.4).max(0.0).min(fin + 0.1), fin, "GROUND");
                            continue;
                        }
                    }
                    Part::Stab(s) => {
                        if closing > 3.0 || (sp > 12.0 && pen > 0.06) {
                            self.lose_elevator(s, true, "GROUND");
                            continue;
                        }
                    }
                    Part::Wing(s, st) => {
                        // A wing struck into the ground breaks in from where it hit,
                        // the further the harder; scraped along fast it tears too.
                        let hard = closing as f32 / 4.0 + sp as f32 / 45.0;
                        if self.cool[s] <= 0.0 && (closing > 3.5 || (sp > 14.0 && pen > 0.1) || (closing > 2.0 && sp > 25.0)) {
                            // (One blow takes a couple of metres at most: breaking the
                            // outer part soaks up what would have hit further in.)
                            let into = self.rng.range(0.05, 0.5) + (hard - 1.0).clamp(0.0, 3.0) * self.rng.range(0.2, 0.6);
                            let station = (st - into).max(0.3);
                            // Through the tank fast: the fuel goes up.
                            if station < airframe::TANK[1] && sp > 32.0 && closing > 8.0 && !water {
                                return self.impact(crash, "wing tank");
                            }
                            let rough = self.rng.range(0.35, 1.0);
                            let fold = if water { 0.2 } else if sp < 30.0 { 0.5 } else { 0.25 };
                            self.snap_wing(s, station, rough, fold, up_right, if water { "WATER" } else { "GROUND" });
                            self.cool[s] = 0.3;
                            self.vel *= 0.94;
                            continue;
                        }
                    }
                    Part::Fold(_) => {}
                }
                // Then it skids: a spring out of the surface, friction along it.
                // Water gives less and drags far more.
                self.pos += upw * if water { pen * 0.3 } else { pen };
                let (k, c, mu) = if water { (25_000.0, 6_000.0, 0.9) } else { (60_000.0, 9_000.0, 0.45) };
                let nf = (pen * k - vn * c).clamp(0.0, 120_000.0);
                let tang = pv - upw * vn;
                let tl = tang.length();
                let f = upw * nf - tang.normalize_or_zero() * (mu * nf).min(tl * 3000.0);
                force_w += f;
                let fb_ = self.rot.inverse() * f;
                torque_b += r.cross(fb_);
                if let Part::Fold(s) = part {
                    let fd = &self.dmg.fold[s];
                    self.fold_tq[s] += (r.as_vec3() - fd.pivot).as_dvec3().cross(fb_).dot(fd.axis.as_dvec3());
                }
                skidding = true;
                if water {
                    self.sink = (self.sink + dt * 0.03).min(1.6);
                    if tl > 4.0 {
                        self.splashes.push((p, tang * 0.4 + upw * (tl * 0.15)));
                    }
                    continue;
                }
                self.scuff(part.group(), (dt * tl * 0.04) as f32);
                self.dmg.vibe += (tl * 0.03) as f32;
                if tl > 3.0 {
                    self.sparks.push((p, tang * 0.5));
                }
                if tl > 1.5 && building.is_none() {
                    self.dirt.push((p, tang * 0.45 + upw * (2.0 + tl * 0.12), 2.5));
                }
            }
            self.pts = pts;
            let mut touching = [false; 3];
            for (gi, r) in GEAR.into_iter().enumerate() {
                // Only gear that is down and locked carries the aircraft.
                if self.gear < 0.999 {
                    break;
                }
                if !self.dmg.gear[gi] {
                    continue;
                }
                let p = self.pos + self.rot * r;
                if let Some((pen, water, building)) = hit(p, world) {
                    if pen <= 0.0 {
                        continue;
                    }
                    let pv = self.vel + self.rot * self.omega.cross(r);
                    let vn = pv.dot(upw);
                    // Wheels do nothing in water or against a wall, and a leg driven
                    // in far too hard just goes: the airframe takes the rest.
                    if water || (building.is_some() && pen > 1.0) || pen > 1.2 {
                        if !water {
                            self.lose_gear(gi, if building.is_some() { "BUILDING" } else { "HARD LANDING" });
                        }
                        continue;
                    }
                    if vn < -11.0 {
                        self.lose_gear(gi, "HARD LANDING");
                        continue;
                    }
                    // Too hard a landing and the leg folds; the airframe comes down on its belly.
                    if vn < if gi == 0 { -4.5 } else { -6.5 } {
                        self.lose_gear(gi, "HARD LANDING");
                        self.kick(DVec3::new(if gi == 0 { -0.6 } else { 0.4 }, 0.0, if gi == 1 { 0.5 } else if gi == 2 { -0.5 } else { 0.0 }));
                        self.crack_windshield("HARD LANDING");
                        continue;
                    }
                    self.on_ground = true;
                    touching[gi] = true;
                    self.compress[gi] = pen.min(if gi == 0 { 0.12 } else { 0.10 });
                    let nf = (pen * 70_000.0 - vn * 7_000.0).clamp(0.0, 120_000.0);
                    let mut fwd = (self.forward() - upw * self.forward().dot(upw)).normalize_or_zero();
                    if gi == 0 {
                        // The nose wheel steers with the rudder pedals.
                        fwd = DQuat::from_axis_angle(upw, -self.steer_angle()) * fwd;
                    }
                    let side = fwd.cross(upw);
                    let (vl, vs) = (pv.dot(fwd), pv.dot(side));
                    let f_side = (-vs * 5000.0).clamp(-0.9 * nf, 0.9 * nf);
                    let mu = if c.brake { 0.55 } else { 0.025 };
                    let f_long = (-vl * if c.brake { 2500.0 } else { 80.0 }).clamp(-mu * nf, mu * nf);
                    let f = upw * nf + side * f_side + fwd * f_long;
                    force_w += f;
                    torque_b += r.cross(self.rot.inverse() * f);
                    self.wheel_spin[gi] = vl / WHEEL_R[gi];
                    if pv.length() > 3.0 && building.is_none() {
                        self.dirt.push((p, (pv - upw * vn) * 0.3 + upw * 1.5, 1.0));
                    }
                }
            }
            for gi in 0..3 {
                if !touching[gi] {
                    self.compress[gi] *= (1.0 - dt * 12.0).max(0.0);
                }
            }
        } else {
            self.compress = [0.0; 3];
        }
        // Wheels spin down in the air; brakes stop the mains at once.
        for gi in 0..3 {
            if !self.on_ground {
                self.wheel_spin[gi] *= (1.0 - dt * if c.brake && gi > 0 { 8.0 } else { 0.25 }).max(0.0);
            }
            self.wheel_rot[gi] = (self.wheel_rot[gi] + self.wheel_spin[gi] * dt) % std::f64::consts::TAU;
        }

        self.on_hull = skidding;
        let acc = force_w / MASS;
        self.swing_folds(dt, acc, upw, vb, rho);
        self.g_load = (acc + upw * 9.81).dot(self.up()) / 9.81;
        // The pilot's head on its neck: a stiff, well-damped spring shoved about
        // by every change in the load, about 3 cm per g.
        self.sf = self.rot.inverse() * (acc + upw * 9.81);
        let fb = self.sf - DVec3::Y * 9.81;
        let (w0, zeta) = (11.0, 0.55);
        self.head_v += (-self.head * w0 * w0 - self.head_v * 2.0 * zeta * w0 - fb * 0.35) * dt;
        if self.on_ground {
            // Rolling over the pavement's joints and patches: a few millimetres
            // of shake by taxi speed, more on the takeoff roll.
            let s = smooth(0.5, 35.0, self.vel.length()) * 0.375 * dt.sqrt();
            let mut k = || self.rng.range(-1.0, 1.0) as f64;
            self.head_v += DVec3::new(k() * 0.35, k(), k() * 0.2) * s;
        }
        self.head = (self.head + self.head_v * dt).clamp_length_max(0.09);
        // Rough air rattles the cabin.
        self.dmg.vibe += ((self.air.sigma_w - 0.6).max(0.0) * 0.07 * smooth(10.0, 40.0, v)) as f32;
        self.vel += acc * dt;
        self.omega += (wdot + torque_b / INERTIA) * dt;
        self.omega = self.omega.clamp_length_max(6.0);
        self.pos += self.vel * dt;
        self.rot = (self.rot * DQuat::from_scaled_axis(self.omega * dt)).normalize();

        // Airframe limits: pull too hard and a wing goes; too fast and the
        // control surfaces flutter themselves off; flaps out too fast tear away.
        if !self.on_ground && !skidding {
            let g = self.g_load.abs() as f32;
            // Limit load is 4.4 g; hold more than that and the spar fatigues, 6.6 g snaps it outright.
            self.dmg.fatigue = (self.dmg.fatigue + ((g - 4.6).max(0.0) * 1.5 - 0.3) * dt as f32).max(-1.5);
            if self.dmg.fatigue < 0.0 {
                self.dmg.fatigue = (self.dmg.fatigue + dt as f32).min(0.0);
            }
            if (g > 6.6 && self.dmg.fatigue >= 0.0) || self.dmg.fatigue > 2.2 {
                let s = (self.rng.f() * 2.0) as usize;
                self.break_wing(s, 3.5, "OVER G");
                // The load comes off the rest of the structure for a moment.
                self.dmg.fatigue = -1.5;
            }
        }
        let ve = v * eas_k;
        let over = (ve - FLUTTER_V).max(0.0) as f32;
        self.dmg.flutter = (self.dmg.flutter + (over * 0.035 - 0.25) * dt as f32).max(0.0);
        self.dmg.vibe += smooth(VNE + 5.0, VNE + 30.0, ve) as f32 * 0.6;
        if self.dmg.flutter > 1.0 {
            self.flutter_loss();
            self.dmg.flutter = 0.35;
        }
        let vfe = self.vfe();
        if self.flaps > 0.25 && ve > vfe {
            self.dmg.flap_stress += (dt * (ve - vfe) * 0.06 * self.flaps) as f32;
            if self.dmg.flap_stress > 1.0 {
                let s = (self.rng.f() * 2.0) as usize;
                let s = if self.dmg.flap[s] > 0.0 { s } else { 1 - s };
                self.lose_flap(s, "OVERSPEED");
                self.dmg.flap_stress = 0.3;
            }
        } else {
            self.dmg.flap_stress = (self.dmg.flap_stress - dt as f32 * 0.3).max(0.0);
        }
        None
    }

    /// A buckled wing swings on its torn skin: its weight and the aircraft's
    /// own acceleration, the air on it as on a flat plate, the ground under
    /// it, and what stiffness the crumpled skin has left. Load it past what
    /// the skin holds (as the slipstream does at once in flight) and it goes.
    fn swing_folds(&mut self, dt: f64, acc: DVec3, upw: DVec3, vb: DVec3, rho: f64) {
        for s in 0..2 {
            let mut f = self.dmg.fold[s];
            if !f.on {
                continue;
            }
            let len = (f.tip - f.root).max(0.3) as f64;
            let m = 24.0 * len;
            let inertia = m * len * len / 3.0 + 2.0;
            let mat = f.mat();
            let ax = f.axis.as_dvec3();
            let cm = mat.transform_point3(damage::wing_pt(s, (f.root + f.tip) * 0.5, 0.4, 0.0)).as_dvec3();
            let arm = cm - f.pivot.as_dvec3();
            // Weight, and the shove of the aircraft's own acceleration, as the part feels them.
            let felt = self.rot.inverse() * (acc + upw * 9.81);
            let t_g = arm.cross(-felt * m).dot(ax);
            // The air on it: a plate across its swung upper skin, the air meeting
            // it at the aircraft's speed plus its own swinging.
            let np = mat.transform_vector3(Vec3::Y).as_dvec3();
            let area = (Damage::wing_lost(f.root) - Damage::wing_lost(f.tip)) as f64;
            let rel = -(vb + self.omega.cross(cm)) - ax.cross(arm) * f.rate as f64;
            let vr = rel.length();
            let mut t_a = 0.0;
            if vr > 0.5 {
                let a_n = (rel.dot(np) / vr).clamp(-1.0, 1.0).asin();
                let cn = (CL_A * a_n + CL0 * (1.0 - a_n.abs() * 2.0).max(0.0)).clamp(-1.2, 1.2);
                let fa = np * (0.5 * rho * vr * vr * area * cn);
                t_a = arm.cross(fa).dot(ax);
                // The aircraft feels it through the hinge.
                self.vel += self.rot * fa * (dt / MASS);
                self.omega += cm.cross(fa) / INERTIA * dt;
            }
            // The crumpled skin still springs back a little, and damps the swing.
            let w = f.rate as f64;
            let t_s = -350.0 * f.ang as f64 - 220.0 * w - 0.5 * rho * 1.2 * 1.4 * w * w.abs() * len.powi(4) / 4.0;
            let t_c = self.fold_tq[s];
            f.rate += ((t_g + t_a + t_s + t_c) / inertia * dt) as f32;
            f.ang += f.rate * dt as f32;
            // Folded right over (or down against the strut), it stops hard.
            let (lo, hi) = (-1.75, 2.7);
            if f.ang > hi || f.ang < lo {
                f.ang = f.ang.clamp(lo, hi);
                f.stress += (f.rate.abs() * 0.04).min(0.3);
                f.rate *= -0.2;
            }
            // How hard the skin is being worked: steady load past what it holds,
            // and every big swing.
            let load = (t_g + t_a + t_c).abs() as f32;
            // Crumpled skin in a fast airflow flutters itself apart in moments.
            let flog = 0.5 * ((0.5 * rho * vb.length_squared()) / 600.0).powf(1.5);
            // (Tearing takes a moment, however hard it is pulled: at least a fifth of a second.)
            let over = (((load - f.hold).max(0.0) / f.hold) * 4.0).min(5.0);
            f.stress += (over + (f.rate.abs() - 4.0).max(0.0) * 0.1 + 0.01 + flog as f32) * dt as f32;
            self.dmg.vibe += (f.rate.abs() * 0.05).min(1.0);
            self.dmg.fold[s] = f;
            if f.stress >= 1.0 {
                self.release_fold(s, if self.on_hull || self.on_ground { "GROUND" } else { "AIRFLOW" });
            }
        }
    }

    /// Trees: refresh the list of nearby ones a few times a second, test every
    /// step. Foliage scrapes and tears things off; a trunk through the nose
    /// or belly is the end.
    fn trees(&mut self, world: &World) -> Option<Impact> {
        if self.scan == 0 {
            self.near_trees.clear();
            world.trees_near(self.pos, 30.0 + self.vel.length() * 0.2, &mut self.near_trees);
        }
        self.scan = (self.scan + 1) % 12;
        let v = self.vel.length();
        let mut k = 0;
        while k < self.near_trees.len() {
            let t = &self.near_trees[k];
            let tu = t.pos.normalize();
            let (cc, ch) = ((t.crown_base + t.height) * 0.5, ((t.height - t.crown_base) * 0.5).max(0.5));
            // 0 = miss, 1 = through the foliage, 2 = into the trunk or the heart of the crown.
            let mut worst = 0;
            let mut at = 5;
            for (hi, r) in HARD.into_iter().enumerate() {
                let d = self.pos + self.rot * r - t.pos;
                let hgt = d.dot(tu);
                if hgt < -1.0 || hgt > t.height + 1.0 {
                    continue;
                }
                let rad = (d - tu * hgt).length();
                let e = ((rad / t.crown_r.max(0.5)).powi(2) + ((hgt - cc) / ch).powi(2)).sqrt();
                let level = if (rad < t.trunk_r + 0.35 && hgt < cc) || e < 0.45 { 2 } else if e < 1.0 { 1 } else { 0 };
                if level > worst {
                    worst = level;
                    at = hi;
                }
            }
            if worst == 0 {
                k += 1;
                continue;
            }
            let t = self.near_trees.swap_remove(k);
            let base_h = geo::pos_to_lla(t.pos).2;
            self.tree_hits.push(t);
            self.brushes += 1.0;
            let side = if at == 1 { 0 } else { 1 };
            let sgn = if side == 0 { -1.0 } else { 1.0 };
            let r = self.rng.f();
            if worst == 2 {
                match at {
                    1 | 2 => {
                        // A trunk through the outer wing shears it off.
                        self.break_wing(side, if v > 45.0 { 3.5 } else { 4.3 }, "TREE");
                        self.vel *= 0.85;
                        self.kick(DVec3::new(0.2, -sgn * 0.7, -sgn * 1.5));
                    }
                    3 | 4 => {
                        self.lose_tail_top(0.4, 0.45, "TREE");
                        self.lose_elevator((r * 2.0) as usize, true, "TREE");
                        self.vel *= 0.88;
                        self.kick(DVec3::new(-0.8, 0.6, 0.4));
                    }
                    // Rolling slowly into a trunk crumples the nose and stops it dead.
                    _ if v < 18.0 => {
                        self.prop_strike("TREE");
                        self.crack_windshield("TREE");
                        self.dmg.hull = (self.dmg.hull + 0.2).min(1.0);
                        self.vel *= 0.2;
                        self.omega *= 0.3;
                    }
                    _ => return Some(Impact { pos: self.pos, vel: self.vel, water: false, ground_h: base_h + 0.2, building: None }),
                }
                continue;
            }
            // Clipped the canopy: it costs speed, throws the aircraft about, and
            // whatever went through the branches comes out worse for it.
            self.vel *= 0.95;
            let kick = (self.scan as f64 * 2.4).sin();
            self.kick(DVec3::new(0.25 * kick, 0.5 * kick, 0.9 * (self.scan as f64 * 1.7).cos()));
            let hard = (v / 45.0) as f32;
            match at {
                1 | 2 => {
                    self.scuff(Self::wing_group(side), 0.35);
                    if r < 0.12 * hard {
                        self.break_wing(side, 5.0, "TREE");
                    } else if r < 0.4 * hard {
                        self.lose_aileron(side, r > 0.2 * hard, "TREE");
                    }
                }
                0 | 6 => {
                    self.scuff(G_ENGINE, 0.4);
                    if r < 0.3 * hard {
                        self.prop_strike("TREE");
                    } else if r < 0.6 * hard {
                        self.engine_hit(0.15, "TREE");
                    }
                    if self.rng.f() < 0.45 * hard {
                        self.crack_windshield("TREE");
                    }
                }
                3 | 4 => {
                    self.scuff(G_TAIL, 0.35);
                    if r < 0.3 * hard {
                        self.lose_tail_top(0.5, 1.0, "TREE");
                    } else if r < 0.5 * hard {
                        self.lose_elevator((self.scan % 2) as usize, false, "TREE");
                    }
                }
                _ => {
                    self.scuff(damage::hard_group(at), 0.3);
                    if self.gear > 0.5 && r < 0.3 * hard {
                        self.lose_gear(1 + (self.scan % 2) as usize, "TREE");
                    }
                }
            }
        }
        None
    }
}

/// Air density (kg/m³) at height `h` (m) in the standard atmosphere: the
/// troposphere's lapse rate up to 11 km, isothermal above.
pub fn density(h: f64) -> f64 {
    let h = h.max(-500.0);
    let t = 1.0 - 2.25577e-5 * h.min(11_000.0);
    RHO0 * t.powf(4.2559) * (-(h - 11_000.0).max(0.0) / 6341.6).exp()
}

/// Static pressure as a fraction of sea level's, in the standard atmosphere.
pub fn pressure_ratio(h: f64) -> f64 {
    let h = h.max(-500.0);
    let t = 1.0 - 2.25577e-5 * h.min(11_000.0);
    t.powf(5.2559) * (-(h - 11_000.0).max(0.0) / 6341.6).exp()
}

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smooth01(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn empty_world() -> World {
        let dir = std::env::temp_dir().join("fablegame-test-cache");
        let src = Arc::new(crate::sources::Sources::new(crate::fetch::Fetcher::new(dir, true), String::new()));
        World::new(crate::sources::Loader::new(src, 0), 14)
    }

    /// Fly for `secs` with a pitch law; returns (airspeed, vertical speed) averaged over the last 20 s.
    fn fly(p: &mut Plane, w: &World, secs: f64, throttle: f64, mut pitch: impl FnMut(&Plane) -> f64) -> (f64, f64) {
        let dt = 1.0 / 180.0;
        p.throttle = throttle;
        let (mut sv, mut svs, mut n) = (0.0, 0.0, 0.0);
        let steps = (secs / dt) as usize;
        for i in 0..steps {
            let roll_err = -p.right().dot(p.pos.normalize());
            let c = Controls { pitch: pitch(p).clamp(-1.0, 1.0), roll: (-roll_err * 3.0 + p.omega.z * 0.5).clamp(-1.0, 1.0), ..Default::default() };
            assert!(p.step(dt, &c, w).is_none());
            if (steps - i) as f64 * dt < 20.0 {
                sv += p.airspeed().length();
                svs += p.vel.dot(p.pos.normalize());
                n += 1.0;
            }
        }
        (sv / n, svs / n)
    }

    const KT: f64 = 1.94384;
    fn rpm(p: &Plane) -> f64 {
        p.prop_rate * 60.0 / std::f64::consts::TAU
    }
    /// An altitude hold for the test pilot (gentle, or it porpoises).
    fn hold_alt(alt0: f64) -> impl FnMut(&Plane) -> f64 {
        move |p| {
            let (_, _, h) = geo::pos_to_lla(p.pos);
            let vs = p.vel.dot(p.pos.normalize());
            (alt0 - h) * 0.002 - vs * 0.02
        }
    }

    #[test]
    fn performance_is_a_cessna_172() {
        let w = empty_world();
        // Flat out and level near sea level: about 125 kt with the gear up, a
        // few knots less with it down (a fixed-gear 172 does 126 kt).
        let mut p = Plane::spawn(0.6, -2.1, 300.0, 0.0, 55.0);
        let (v, _) = fly(&mut p, &w, 240.0, 1.0, hold_alt(300.0));
        println!("max level speed {:.1} m/s = {:.0} kt at {:.0} rpm", v, v * KT, rpm(&p));
        assert!(v * KT > 118.0 && v * KT < 132.0, "{}", v * KT);
        assert!(rpm(&p) > 2550.0 && rpm(&p) < 2800.0, "{}", rpm(&p));
        let mut p = Plane::spawn(0.6, -2.1, 300.0, 0.0, 55.0);
        p.configure(0, true, 0.0);
        let (vg, _) = fly(&mut p, &w, 240.0, 1.0, hold_alt(300.0));
        println!("gear down: {:.0} kt", vg * KT);
        assert!(vg < v - 1.0 && vg * KT > 112.0, "{}", vg * KT);

        // 75% power at 8000 ft: about 120 kt true.
        let mut p = Plane::spawn(0.6, -2.1, 2438.0, 0.0, 58.0);
        let mut thr = 0.8;
        for _ in 0..40 {
            fly(&mut p, &w, 5.0, thr, hold_alt(2438.0));
            thr = (thr + (0.75 - p.power) * 0.5).clamp(0.0, 1.0);
        }
        let (v, _) = fly(&mut p, &w, 120.0, thr, hold_alt(2438.0));
        println!("75% at 8000 ft: {:.0} kt true, {:.0} kt indicated, {:.0} rpm, power {:.2}", v * KT, p.ias() * KT, rpm(&p), p.power);
        assert!(v * KT > 110.0 && v * KT < 128.0, "{}", v * KT);

        // Full power at the best-rate speed (74 kt): 700-900 fpm.
        let mut p = Plane::spawn(0.6, -2.1, 300.0, 0.0, 40.0);
        let (v, vs) = fly(&mut p, &w, 90.0, 1.0, |p| (p.airspeed().length() - 38.0) * 0.04);
        println!("climb at {:.0} kt: {:.0} fpm, {:.0} rpm", v * KT, vs * 196.85, rpm(&p));
        assert!(vs * 196.85 > 650.0 && vs * 196.85 < 950.0, "{}", vs * 196.85);

        // Engine at idle at the best glide speed (68 kt): about 9 to 1.
        let mut p = Plane::spawn(0.6, -2.1, 1500.0, 0.0, 35.0);
        let (v, vs) = fly(&mut p, &w, 90.0, 0.0, |p| (p.airspeed().length() - 35.0) * 0.04);
        let glide = (v * v - vs * vs).sqrt() / -vs;
        println!("idle glide at {:.0} kt: {:.0} fpm, {:.1}:1, {:.0} rpm", v * KT, vs * 196.85, glide, rpm(&p));
        assert!(glide > 7.5 && glide < 11.0, "{glide}");
        assert!(rpm(&p) > 700.0 && rpm(&p) < 1600.0, "{}", rpm(&p));

        // Stall: about 50 kt clean, 42 kt with full flaps.
        let vs_at = |flap_cl: f64, a: f64| (2.0 * MASS * 9.81 / (RHO0 * WING_AREA * (CL0 + flap_cl + CL_A * a))).sqrt() * KT;
        let (clean, full) = (vs_at(0.0, 0.27), vs_at(0.55, 0.23));
        println!("stall {clean:.0} kt clean, {full:.0} kt full flaps");
        assert!(clean > 46.0 && clean < 54.0 && full > 39.0 && full < 46.0);

        // On the ground: about 2300 rpm flat out, 600-800 at idle, static thrust a
        // quarter of the weight.
        let ws = Plane::prop_equilibrium(1.0, 0.0, RHO0) * 60.0 / std::f64::consts::TAU;
        let wi = Plane::prop_equilibrium(0.0, 0.0, RHO0) * 60.0 / std::f64::consts::TAU;
        let t0 = Plane::thrust(1.0, 0.0, RHO0);
        println!("static {ws:.0} rpm, idle {wi:.0} rpm, thrust {t0:.0} N = {:.2} W", t0 / (MASS * 9.81));
        assert!(ws > 2200.0 && ws < 2450.0 && wi > 550.0 && wi < 850.0, "{ws} {wi}");
        assert!(t0 / (MASS * 9.81) > 0.18 && t0 / (MASS * 9.81) < 0.28, "{t0}");
        // Diving at Vne at full throttle the fixed-pitch propeller over-revs.
        let wd = Plane::prop_equilibrium(1.0, VNE, RHO0) * 60.0 / std::f64::consts::TAU;
        println!("full throttle at Vne: {wd:.0} rpm");
        assert!(wd > 2850.0, "{wd}");
    }

    #[test]
    fn approach_is_trimmed_down_the_glide() {
        let w = empty_world();
        // As the game sets up a final approach: 70 kt, flaps 20, a 3 degree glide.
        let mut p = Plane::spawn(0.6, -2.1, 400.0, 0.0, 36.0);
        let sink = 36.0 * 3f64.to_radians().sin();
        p.configure(2, true, sink);
        let thr = p.throttle;
        let (v, vs) = fly(&mut p, &w, 30.0, thr, |_| 0.0);
        println!("approach: throttle {thr:.2}, {:.0} kt, {:.0} fpm, {:.0} rpm", v * KT, vs * 196.85, rpm(&p));
        assert!((v - 36.0).abs() < 3.0, "{v}");
        assert!((vs + sink).abs() < 1.0, "{vs}");
    }

    #[test]
    fn turbulence_bumps_the_aircraft() {
        let w = empty_world();
        let mut spread = Vec::new();
        for sigma in [0.0, 1.5, 4.0] {
            let mut p = Plane::spawn(0.6, -2.1, 1000.0, 0.0, 55.0);
            p.air = Air { sigma_w: sigma, sigma_h: sigma, len_w: 300.0, len_h: 300.0, draft: 0.0 };
            let dt = 1.0 / 180.0;
            let (mut s, mut s2, mut n, mut bank) = (0.0, 0.0, 0.0, 0.0f64);
            for i in 0..(60.0 / dt) as usize {
                // A pilot holding the wings roughly level.
                let roll_err = -p.right().dot(p.pos.normalize());
                let c = Controls { roll: (-roll_err * 3.0 + p.omega.z * 0.5).clamp(-1.0, 1.0), ..Default::default() };
                p.step(dt, &c, &w);
                if i > 1800 {
                    s += p.g_load;
                    s2 += p.g_load * p.g_load;
                    n += 1.0;
                    bank = bank.max(roll_err.abs().asin().to_degrees());
                }
            }
            let sd = (s2 / n - (s / n).powi(2)).max(0.0).sqrt();
            println!("sigma {sigma}: g rms {sd:.3}, worst bank {bank:.1} deg");
            spread.push(sd);
        }
        assert!(spread[0] < 0.02 && spread[1] > 0.05 && spread[2] > spread[1] * 1.8, "{spread:?}");
    }

    fn flat_world() -> World {
        let mut w = empty_world();
        w.flat = Some(0.0);
        w
    }
    /// Run until it explodes or `secs` pass; every note it made along the way.
    fn run(p: &mut Plane, w: &World, secs: f64, c: Controls) -> (bool, Vec<String>) {
        run_pushed(p, w, secs, c, 0.0)
    }
    /// The same, with something rolling it over (rad/s²) for the first 0.8 s: a gust under one wing.
    fn run_pushed(p: &mut Plane, w: &World, secs: f64, c: Controls, push: f64) -> (bool, Vec<String>) {
        let dt = 1.0 / 180.0;
        let mut notes = Vec::new();
        for k in 0..(secs / dt) as usize {
            if (k as f64) * dt < 0.8 {
                p.omega.z += push * dt;
            }
            let imp = p.step(dt, &c, w);
            notes.extend(p.notes.drain(..));
            p.shed.clear();
            if imp.is_some() {
                return (true, notes);
            }
        }
        (false, notes)
    }
    /// Rolling down the runway at `kt`, banked over on one main wheel and
    /// still rolling `roll` rad/s (a gust under a wing, a botched crosswind landing).
    fn rolling(kt: f64, roll: f64, seed: u64) -> Plane {
        let mut p = Plane::spawn_ground(37.8, -122.46, 0.0, 0.5, 0.0);
        p.hold = false;
        p.t = 2.0;
        p.rng = Rng(seed);
        p.throttle = 0.0;
        p.charge = 0.0;
        let bank = 0.12 * roll.signum();
        // Pivot about the downhill main wheel so it stays on the ground.
        let wheel = GEAR[if roll > 0.0 { 1 } else { 2 }];
        let before = p.pos + p.rot * wheel;
        p.rot = p.rot * DQuat::from_rotation_z(-bank);
        p.pos += before - (p.pos + p.rot * wheel);
        p.vel = p.forward() * (kt / KT);
        p.omega = DVec3::new(0.0, 0.0, -roll);
        p
    }

    #[test]
    fn tipping_over_at_40_kt_breaks_a_wing_without_fire() {
        let w = flat_world();
        let mut fires = 0;
        for seed in 1..=12u64 {
            let mut p = rolling(40.0, if seed % 2 == 0 { 2.2 } else { -2.2 }, seed * 7919);
            let (boom, notes) = run_pushed(&mut p, &w, 12.0, Controls::default(), if seed % 2 == 0 { -9.0 } else { 9.0 });
            println!("seed {seed}: boom {boom} {}, wings {:?}, folds {:?}, speed {:.1}, wrecked {}: {notes:?}", p.cause, p.dmg.wing, p.dmg.fold.map(|f| f.on), p.vel.length(), p.wrecked());
            fires += boom as usize;
            assert!(p.dmg.wing.iter().any(|&b| b < SPAN) || p.dmg.fold.iter().any(|f| f.on) || !notes.is_empty(), "something breaks");
        }
        assert_eq!(fires, 0, "a slow tip-over never burns");
    }

    #[test]
    fn a_cartwheel_at_70_kt_tears_things_off() {
        let w = flat_world();
        let mut fires = 0;
        for seed in 1..=8u64 {
            let mut p = rolling(70.0, 3.5, seed * 104_729);
            p.vel += p.pos.normalize() * -3.0;
            let (boom, notes) = run(&mut p, &w, 14.0, Controls { roll: 1.0, ..Default::default() });
            println!("seed {seed}: boom {boom} {}, wings {:?}, tail {}, hull {:.2}: {notes:?}", p.cause, p.dmg.wing, p.dmg.tail.is_some(), p.dmg.hull);
            fires += boom as usize;
            assert!(boom || !notes.is_empty());
        }
        assert!(fires < 8, "not every cartwheel burns");
    }

    #[test]
    fn belly_landing_slides_to_a_stop() {
        let w = flat_world();
        let mut p = Plane::spawn(37.8, -122.46, 2.0, 0.5, 30.0);
        p.t = 2.0;
        p.throttle = 0.0;
        p.charge = 0.0;
        p.vel += p.pos.normalize() * -2.0;
        let (boom, notes) = run(&mut p, &w, 20.0, Controls::default());
        println!("belly: boom {boom}, speed {:.1}, prop {}, {notes:?}", p.vel.length(), p.dmg.prop);
        assert!(!boom);
        assert!(p.vel.length() < 2.0);
        assert!(p.dmg.wing == [SPAN; 2]);
    }

    #[test]
    fn diving_into_the_ground_explodes() {
        let w = flat_world();
        let mut p = Plane::spawn(37.8, -122.46, 40.0, 0.5, 60.0);
        p.t = 2.0;
        p.rot = p.rot * DQuat::from_rotation_x(-0.7);
        p.vel = p.forward() * 60.0;
        let (boom, _) = run(&mut p, &w, 5.0, Controls { pitch: -1.0, ..Default::default() });
        assert!(boom);
    }

    #[test]
    fn a_buckled_wing_rips_off_in_the_slipstream() {
        let w = empty_world();
        let mut p = Plane::spawn(37.8, -122.46, 1500.0, 0.5, 50.0);
        p.dmg.apply_spec("rfold=3.0");
        let dt = 1.0 / 180.0;
        let mut gone = None;
        for k in 0..360 {
            p.step(dt, &Controls::default(), &w);
            if !p.dmg.fold[1].on {
                gone = Some(k as f64 * dt);
                break;
            }
        }
        println!("fold let go after {gone:?} s");
        assert!(gone.is_some_and(|t| t < 1.5));
        assert!(!p.shed.is_empty());
    }

    #[test]
    fn breaks_stay_inside_each_other() {
        let mut rng = Rng(99);
        for s in 0..2 {
            let mut prev = Tear::NONE;
            let mut st = SPAN;
            while st > 1.0 {
                st -= rng.range(0.4, 1.4);
                let t = damage::wing_break(s, st.max(0.5), rng.range(-0.8, 0.8), rng.range(-0.3, 0.3), 0.2, &prev);
                // The new break's mid-chord point is kept by the old stump, the old tip is not kept by the new.
                let mid = damage::wing_pt(s, st.max(0.5), 0.45, 0.0);
                assert!(prev.keeps(mid));
                assert!(!t.keeps(damage::wing_pt(s, st.max(0.5) + 0.3, 0.45, 0.0)));
                prev = t;
            }
        }
    }

}
