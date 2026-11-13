//! The aircraft's systems and a six-degree-of-freedom flight model for a
//! 200 hp retractable single (about a Mooney M20J: 1300 kg, 16 m² of wing):
//! a constant-speed propeller whose thrust falls off with speed and altitude,
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
    head_v: DVec3,
    /// engine power actually being made, 0..1 of rated (lags the throttle)
    pub power: f64,
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
}

const MASS: f64 = 1300.0;
/// Rated power at sea level (200 hp) and the propeller's static thrust at full power.
const POWER: f64 = 149_000.0;
const STATIC_THRUST: f64 = 3200.0;
/// Propeller efficiency in the cruise.
const PROP_ETA: f64 = 0.82;
/// Wing span (m).
const B: f64 = 11.0;
const WING_AREA: f64 = 16.0;
const INERTIA: DVec3 = DVec3::new(1900.0, 2800.0, 1400.0);
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
/// Never-exceed speed (m/s); beyond it the control surfaces start to flutter.
pub const VNE: f64 = 95.0;
const FLUTTER_V: f64 = 108.0;
/// Flaps must not be out above this.
pub const VFE: f64 = 60.0;

impl Plane {
    pub fn spawn(lat: f64, lon: f64, alt: f64, heading: f64, speed: f64) -> Self {
        let pos = geo::lla_to_pos(lat, lon, alt);
        let (e, n, u) = geo::enu(pos);
        let f = n * heading.cos() + e * heading.sin();
        // Nose up by the angle of attack that carries the weight at this speed
        // and height, and trimmed there, so it starts out in level flight.
        let q = 0.5 * 1.225 * (-alt.max(0.0) / 8500.0).exp() * speed * speed;
        let trim = ((MASS * 9.81 / (q * WING_AREA).max(1.0) - 0.25) / 5.0).clamp(-0.05, 0.24);
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
            head_v: DVec3::ZERO,
            power: 0.72,
            drop: 0.0,
            rot,
            omega: DVec3::ZERO,
            // About cruise power for the spawn speed.
            throttle: 0.72,
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
            prop_rate: 0.0,
            trim,
            tree_hits: Vec::new(),
            brushes: 0.0,
            dmg: Damage::default(),
            dmg_prev: Damage::default(),
            shed: Vec::new(),
            notes: Vec::new(),
            sparks: Vec::new(),
            t: 0.0,
            near_trees: Vec::new(),
            scan: 0,
            rng: Rng(pos.x.to_bits() | 1),
        }
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
        let fl = smooth01(100.0, 125.0, self.vel.length() as f32) * 0.12;
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
        if station >= self.dmg.wing[side] - 0.05 {
            return;
        }
        let sgn = if side == 0 { -1.0 } else { 1.0 };
        let lost = self.dmg.wing[side] - station;
        self.dmg.wing[side] = station;
        // The part that leaves: the far side of the break, with whatever flap
        // or aileron was riding on it.
        self.shed.push(ShedReq { sel: Sel::Group(Self::wing_group(side)), tear: Tear::new(Vec3::new(sgn, 0.0, 0.0), station, -1.0), kick: Vec3::new(sgn * 2.5, 1.5, 2.0), spin: 5.0 });
        if station < airframe::AILERON[0] + 0.3 {
            self.dmg.aileron[side] = 0.0;
        }
        self.scuff(Self::wing_group(side), 0.5);
        // Shoved: it rolls toward the stump and yaws into the drag of the break.
        let (sg, lo) = (sgn as f64, lost as f64);
        self.kick(DVec3::new(0.3, -sg * 0.6 * lo, -sg * (0.8 + 0.9 * lo)));
        self.note(format!("{} WING TORN OFF ({why})", Self::lr(side)));
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
        self.shed.push(ShedReq { sel: Sel::Names(name), tear: Tear::NONE, kick: Vec3::new(0.0, 1.0, 3.0), spin: 9.0 });
        self.note(format!("{} AILERON TORN OFF ({why})", Self::lr(side)));
    }
    fn lose_flap(&mut self, side: usize, why: &str) {
        if self.dmg.flap[side] <= 0.0 {
            return;
        }
        self.dmg.flap[side] = 0.0;
        let name: &'static [&'static str] = if side == 0 { &["left flap"] } else { &["right flap"] };
        self.shed.push(ShedReq { sel: Sel::Names(name), tear: Tear::NONE, kick: Vec3::new(0.0, -1.0, 4.0), spin: 8.0 });
        self.kick(DVec3::new(0.0, 0.0, if side == 0 { -0.4 } else { 0.4 }));
        self.note(format!("{} FLAP TORN OFF ({why})", Self::lr(side)));
    }
    fn lose_elevator(&mut self, side: usize, with_stab: bool, why: &str) {
        if with_stab && self.dmg.stab[side] > 0.5 {
            self.dmg.stab[side] = 0.0;
            self.dmg.elevator[side] = 0.0;
            let sgn = if side == 0 { -1.0 } else { 1.0 };
            self.shed.push(ShedReq { sel: Sel::Names(&["stabiliser", "elevator"]), tear: Tear::new(Vec3::new(sgn, 0.0, 0.0), 0.34, -1.0), kick: Vec3::new(sgn * 2.0, 0.5, 2.0), spin: 6.0 });
            self.kick(DVec3::new(-0.5, 0.0, sgn as f64 * 0.3));
            self.note(format!("{} STABILISER TORN OFF ({why})", Self::lr(side)));
        } else if self.dmg.elevator[side] > 0.5 {
            self.dmg.elevator[side] = 0.0;
            let sgn = if side == 0 { -1.0 } else { 1.0 };
            self.shed.push(ShedReq { sel: Sel::Names(&["elevator"]), tear: Tear::new(Vec3::new(sgn, 0.0, 0.0), 0.12, -1.0), kick: Vec3::new(sgn, 1.0, 3.0), spin: 9.0 });
            self.note(format!("{} ELEVATOR TORN OFF ({why})", Self::lr(side)));
        }
    }
    /// The top of the rudder (and perhaps the fin with it) goes.
    fn lose_tail_top(&mut self, rudder: f32, fin: f32, why: &str) {
        let rudder = rudder.min(self.dmg.rudder);
        let fin = fin.min(self.dmg.fin);
        if rudder < self.dmg.rudder - 0.01 {
            if rudder <= 0.0 {
                self.shed.push(ShedReq { sel: Sel::Names(&["rudder"]), tear: Tear::NONE, kick: Vec3::new(1.0, 1.0, 3.0), spin: 7.0 });
                self.note(format!("RUDDER TORN OFF ({why})"));
            } else {
                self.shed.push(ShedReq { sel: Sel::Names(&["rudder"]), tear: Tear::new(Vec3::Y, 0.22 + 1.57 * rudder, -1.0), kick: Vec3::new(1.0, 1.5, 3.0), spin: 8.0 });
                self.note(format!("RUDDER DAMAGED ({why})"));
            }
            self.dmg.rudder = rudder;
        }
        if fin < self.dmg.fin - 0.01 {
            self.shed.push(ShedReq { sel: Sel::Names(&["stabiliser"]), tear: Tear::new(Vec3::Y, 0.27 + 1.53 * fin, -1.0), kick: Vec3::new(1.0, 2.0, 2.0), spin: 6.0 });
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
            self.shed.push(ShedReq { sel: Sel::Names(&["blades"]), tear: Tear::new(Vec3::Y, airframe::PROP_HUB.y + 0.42, -1.0), kick: Vec3::new(2.0, 4.0, -3.0), spin: 30.0 });
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
        self.shed.push(ShedReq { sel: Sel::Names(damage::GEAR_PIECES[gi]), tear: Tear::NONE, kick: Vec3::new(0.0, -1.0, 2.0), spin: 6.0 });
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

    /// Thrust (N) at a power fraction, airspeed and air density. The propeller
    /// turns the engine's power into thrust at ~82% efficiency in the cruise;
    /// slow, it cannot use all of it and the thrust levels off at its static value.
    pub fn thrust(power: f64, v: f64, rho: f64, boost: bool) -> f64 {
        // A normally aspirated engine loses power with the air (Gagg-Ferrar).
        let sigma = rho / 1.225;
        let p = POWER * ((sigma - 0.12) / 0.88).max(0.0) * power * if boost { 1.4 } else { 1.0 };
        let t_static = STATIC_THRUST * sigma * power * if boost { 1.3 } else { 1.0 };
        let t_power = PROP_ETA * p / v.max(1.0);
        if t_static <= 0.0 || t_power <= 0.0 {
            return 0.0;
        }
        // A smooth handover between the two limits.
        (t_static.powi(-4) + t_power.powi(-4)).powf(-0.25)
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
    pub fn step(&mut self, dt: f64, c: &Controls, world: &World) -> Option<Impact> {
        self.t += dt;
        self.dmg_prev = self.dmg;
        self.dmg.vibe = 0.0;
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
        let rho = 1.225 * (-h.max(0.0) / 8500.0).exp();
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
            let cl = ((0.25 + flap_cl + 5.0 * alpha) * (1.0 - stall) + (2.0 * alpha).sin() * 0.95 * stall) * (1.0 + 0.12 * (1.0 - ge));
            // Ragged stumps and missing fairings all drag.
            let rag = ((SPAN - dm.wing[0]) + (SPAN - dm.wing[1])) as f64;
            // A clean retractable: Cd0 ~0.024, induced drag for an aspect ratio of 7.6.
            let cd = 0.024 + 0.055 * cl * cl * ge + stall * 1.2 * alpha.sin().powi(2) + 0.6 * beta * beta + 0.02 * self.flaps + 0.045 * self.flaps * self.flaps + 0.016 * self.gear + 0.02 * rag + 0.03 * (1.0 - st) + 0.01 * (1.0 - rud);
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
            let alpha_1g = ((MASS * 9.81 * level / (q * area).max(1.0) - 0.25 - flap_cl) / 5.0).clamp(-0.05, 0.24);
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
        let prop_k = if dm.prop > 0.75 { 1.0 } else if dm.prop > 0.25 { 0.3 } else { 0.0 };
        let eng = (self.dmg.engine as f64).powf(0.6) * prop_k;
        // The engine takes a moment to answer the throttle.
        self.power += (self.throttle * eng - self.power) * (dt * 2.0).min(1.0);
        let thrust = Self::thrust(self.power, v, rho, c.boost);
        force_b += DVec3::NEG_Z * thrust;
        // The propeller turns clockwise seen from the cockpit: its torque rolls the
        // aircraft left and its spiralling slipstream (and P-factor, climbing
        // slowly) yaws it left. The fin and ailerons are rigged for the cruise, so
        // it is full power at low speed that needs right rudder.
        if v > 2.0 {
            let slow = (1.0 - v / 70.0).max(0.0);
            wdot.y += thrust / STATIC_THRUST * slow * (0.22 + 0.25 * self.alpha.max(0.0));
            wdot.z += thrust / STATIC_THRUST * slow * 0.12 * if self.on_ground { 0.3 } else { 1.0 };
        }
        // An unbalanced propeller cannot be run up without shaking the engine off its mounts.
        let rate = if dm.prop <= 0.25 {
            0.0
        } else if eng > 0.0 {
            (65.0 + 185.0 * self.throttle * if c.boost { 1.15 } else { 1.0 }) * (0.4 + 0.6 * eng) * if dm.prop < 0.75 { 0.45 } else { 1.0 }
        } else {
            (v * 0.45).min(45.0)
        };
        self.prop_rate += (rate - self.prop_rate) * (dt * 2.0).min(1.0);
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
                return Some(imp);
            }
            let hit = |p: DVec3, world: &World| -> Option<(f64, bool, Option<(geo::TileId, usize)>)> {
                let (la, lo, ph) = geo::pos_to_lla(p);
                let g = world.ground(la, lo)?;
                if let Some(b) = world.building(la, lo) {
                    if ph < b.top && ph > b.base - if b.bridge { 0.0 } else { 3.0 } {
                        return Some((b.top - ph, false, Some((b.tile, b.idx))));
                    }
                }
                Some((g.h - ph, g.water, None))
            };
            for (hi, r) in HARD.into_iter().enumerate() {
                let p = self.pos + self.rot * r;
                let Some((pen, water, building)) = hit(p, world) else { continue };
                if pen <= 0.0 {
                    continue;
                }
                let pv = self.vel + self.rot * self.omega.cross(r);
                let vn = pv.dot(upw);
                let sp = pv.length();
                let crash = Impact { pos: p, vel: self.vel, water, ground_h: geo::pos_to_lla(p).2 + if building.is_some() { 0.0 } else { pen }, building };
                let side = if hi == 1 { 0 } else { 1 };
                let sgn = if side == 0 { -1.0 } else { 1.0 };
                if water {
                    return Some(crash);
                }
                if building.is_some() {
                    // A wing tip or the tail clipping a roof comes off; anything else is the end.
                    match hi {
                        1 | 2 if vn.abs() < 7.0 => {
                            self.break_wing(side, if sp > 45.0 { 3.5 } else { 4.3 }, "BUILDING");
                            self.vel *= 0.8;
                            self.kick(DVec3::new(0.0, -sgn * 0.8, -sgn * 1.2));
                            self.pos += upw * pen.min(0.5);
                            continue;
                        }
                        3 | 4 if vn.abs() < 7.0 => {
                            self.lose_tail_top(0.3, 0.5, "BUILDING");
                            self.lose_elevator((self.scan % 2) as usize, true, "BUILDING");
                            self.vel *= 0.85;
                            self.pos += upw * pen.min(0.5);
                            continue;
                        }
                        _ => return Some(crash),
                    }
                }
                // The ground. Gently enough, a dragged wing tip only loses its end,
                // the tail skids, the belly slides and the propeller strikes.
                let skid = match hi {
                    1 | 2 => {
                        if vn > -5.0 && sp < 70.0 {
                            self.break_wing(side, if sp > 35.0 { 4.3 } else { 5.0 }, "GROUND");
                            self.kick(DVec3::new(0.0, -sgn * 0.5, -sgn * 1.0));
                            self.vel *= 0.92;
                            self.pos += upw * pen;
                            continue;
                        }
                        false
                    }
                    4 => false,
                    0 | 6 => {
                        if vn > -3.5 {
                            if self.dmg.prop > 0.25 {
                                self.prop_strike("GROUND");
                            }
                            true
                        } else {
                            false
                        }
                    }
                    _ => vn > -4.0,
                };
                if !skid {
                    return Some(crash);
                }
                self.pos += upw * pen;
                let nf = (pen * 60_000.0 - vn * 9_000.0).clamp(0.0, 120_000.0);
                let tang = pv - upw * vn;
                let tl = tang.length();
                let f = upw * nf - tang.normalize_or_zero() * (0.45 * nf).min(tl * 3000.0);
                force_w += f;
                torque_b += r.cross(self.rot.inverse() * f);
                skidding = true;
                self.scuff(damage::hard_group(hi), (dt * tl * 0.04) as f32);
                self.dmg.vibe += (tl * 0.03) as f32;
                if tl > 3.0 {
                    self.sparks.push((p, tang * 0.5));
                }
            }
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
                    if water || building.is_some() || vn < -11.0 || pen > 1.2 {
                        return Some(Impact { pos: p, vel: self.vel, water, ground_h: geo::pos_to_lla(p).2 + if building.is_some() { 0.0 } else { pen }, building });
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

        let acc = force_w / MASS;
        self.g_load = (acc + upw * 9.81).dot(self.up()) / 9.81;
        // The pilot's head on its neck: a stiff, well-damped spring shoved about
        // by every change in the load, about 3 cm per g.
        let fb = self.rot.inverse() * (acc + upw * 9.81) - DVec3::Y * 9.81;
        let (w0, zeta) = (11.0, 0.55);
        self.head_v += (-self.head * w0 * w0 - self.head_v * 2.0 * zeta * w0 - fb * 0.35) * dt;
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
        let over = (v - FLUTTER_V).max(0.0) as f32;
        self.dmg.flutter = (self.dmg.flutter + (over * 0.035 - 0.25) * dt as f32).max(0.0);
        self.dmg.vibe += smooth(VNE + 5.0, VNE + 30.0, v) as f32 * 0.6;
        if self.dmg.flutter > 1.0 {
            self.flutter_loss();
            self.dmg.flutter = 0.35;
        }
        if self.flaps > 0.25 && v > VFE {
            self.dmg.flap_stress += (dt * (v - VFE) * 0.06 * self.flaps) as f32;
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

    #[test]
    fn performance_is_a_200hp_single() {
        let w = empty_world();
        // Flat out and level near sea level: about 150 kt, not a fighter.
        let mut p = Plane::spawn(0.6, -2.1, 300.0, 0.0, 68.0);
        let alt0 = 300.0;
        let (v, _) = fly(&mut p, &w, 240.0, 1.0, |p| {
            let (_, _, h) = geo::pos_to_lla(p.pos);
            let vs = p.vel.dot(p.pos.normalize());
            (alt0 - h) * 0.002 - vs * 0.02
        });
        println!("max level speed {:.1} m/s = {:.0} kt", v, v * 1.94384);
        assert!(v > 70.0 && v < 84.0, "{v}");

        // Full power at the best-rate speed (~90 kt): about 1000 fpm.
        let mut p = Plane::spawn(0.6, -2.1, 300.0, 0.0, 50.0);
        let (v, vs) = fly(&mut p, &w, 90.0, 1.0, |p| (p.airspeed().length() - 46.0) * 0.04);
        println!("climb at {:.1} m/s: {:.0} fpm", v, vs * 196.85);
        assert!(vs * 196.85 > 800.0 && vs * 196.85 < 1400.0, "{vs}");

        // Static thrust is a quarter of the weight, so the takeoff roll is a few hundred metres.
        let t0 = Plane::thrust(1.0, 0.0, 1.225, false);
        assert!(t0 / (MASS * 9.81) < 0.3, "{t0}");
        assert!(Plane::thrust(1.0, 0.0, 1.225, true) < MASS * 9.81 * 0.4);
    }

    #[test]
    fn turbulence_bumps_the_aircraft() {
        let w = empty_world();
        let mut spread = Vec::new();
        for sigma in [0.0, 1.5, 4.0] {
            let mut p = Plane::spawn(0.6, -2.1, 1000.0, 0.0, 65.0);
            p.air = Air { sigma_w: sigma, sigma_h: sigma, len_w: 300.0, len_h: 300.0, draft: 0.0 };
            let dt = 1.0 / 180.0;
            p.throttle = 0.72;
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
}
