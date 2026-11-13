//! Progressive damage: what has been torn off the aircraft, what each loss
//! does to the way it flies, and the pieces that tumble away afterwards.
//!
//! A break is a ragged plane through the body frame (`Tear`). The renderer
//! cuts any piece along it (see `tear_d` in plane.wgsl), so the stump left on
//! the aircraft and the part fluttering down are the same mesh seen from the
//! two sides of the same tear, with the same ragged edge.
use crate::airframe::{self, Piece, AILERON, GROUPS, G_ENGINE, G_FUSE, G_LWING, G_PROP, G_RWING, G_TAIL, SPAN};
use crate::crash::{Rng, GRIT, SPARK};
use crate::geo;
use crate::render::{DrawObj, Particle};
use crate::world::World;
use glam::{DQuat, DVec3, Mat4, Vec3};

/// How ragged a torn edge is (metres of wander either side of the break).
pub const JAG: f32 = 0.14;

/// A break through a piece. `mode` 1 keeps the side where n·L < d, -1 the
/// other side; ±2 the same with |L.x| in place of L.x (both halves at once).
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Tear {
    pub n: Vec3,
    pub d: f32,
    pub mode: f32,
}

impl Tear {
    pub const NONE: Tear = Tear { n: Vec3::ZERO, d: 0.0, mode: 0.0 };
    pub fn new(n: Vec3, d: f32, mode: f32) -> Self {
        Self { n, d, mode }
    }
    pub fn is_some(&self) -> bool {
        self.mode != 0.0
    }
    /// Does the break (without its ragged edge) keep this body-frame point?
    pub fn keeps(&self, p: Vec3) -> bool {
        if self.mode == 0.0 {
            return true;
        }
        let q = if self.mode.abs() > 1.5 { Vec3::new(p.x.abs(), p.y, p.z) } else { p };
        let v = q.dot(self.n) - self.d;
        (v < 0.0) == (self.mode > 0.0)
    }
    pub fn gpu(&self) -> [f32; 4] {
        [self.n.x, self.n.y, self.n.z, self.d]
    }
}

/// Everything that can be wrong with the airframe.
#[derive(Clone, Copy, Debug)]
pub struct Damage {
    /// span station each wing now ends at (SPAN = whole), left then right
    pub wing: [f32; 2],
    /// control surfaces: 1 sound, 0.5 hanging from one hinge, 0 gone
    pub aileron: [f32; 2],
    pub flap: [f32; 2],
    /// left and right halves of the elevator and of the stabiliser (1 or 0)
    pub elevator: [f32; 2],
    pub stab: [f32; 2],
    /// how much of the rudder's and the fin's height is left
    pub rudder: f32,
    pub fin: f32,
    /// 1 both blades, 0.5 one blade snapped, 0 gone
    pub prop: f32,
    /// engine health: below 1 it leaks oil and loses power, at 0 it has seized
    pub engine: f32,
    /// nose, left main, right main still attached
    pub gear: [bool; 3],
    /// oil film over the windshield
    pub oil: f32,
    /// a crack in the windshield: body-frame z and y of its centre, and its size
    pub crack: [f32; 3],
    /// scrapes and dents, by breakable group
    pub scuff: [f32; GROUPS.len()],
    /// how hard the airframe is shaking (camera, HUD)
    pub vibe: f32,
    /// smoke pouring from the engine, 0..1
    pub smoke: f32,
    /// black smoke rather than oil-white
    pub smoke_black: bool,
    pub fatigue: f32,
    pub flutter: f32,
    pub flap_stress: f32,
}

impl Default for Damage {
    fn default() -> Self {
        Self {
            wing: [SPAN; 2],
            aileron: [1.0; 2],
            flap: [1.0; 2],
            elevator: [1.0; 2],
            stab: [1.0; 2],
            rudder: 1.0,
            fin: 1.0,
            prop: 1.0,
            engine: 1.0,
            gear: [true; 3],
            oil: 0.0,
            crack: [0.0; 3],
            scuff: [0.0; GROUPS.len()],
            vibe: 0.0,
            smoke: 0.0,
            smoke_black: false,
            fatigue: 0.0,
            flutter: 0.0,
            flap_stress: 0.0,
        }
    }
}

fn side_of(name: &str) -> usize {
    if name.starts_with("left") { 0 } else { 1 }
}
fn sgn(side: usize) -> f32 {
    if side == 0 { -1.0 } else { 1.0 }
}
/// The break through one or both halves of a tail surface that lives at |x| > d.
fn halves(h: [f32; 2], d: f32) -> Tear {
    match (h[0] < 0.5, h[1] < 0.5) {
        (true, true) => Tear::new(Vec3::X, d, 2.0),
        (true, false) => Tear::new(Vec3::NEG_X, d, 1.0),
        (false, true) => Tear::new(Vec3::X, d, 1.0),
        _ => Tear::NONE,
    }
}

impl Damage {
    /// Pieces that are no longer there at all.
    pub fn hidden(&self, p: &Piece) -> bool {
        match p.name {
            "left aileron" | "right aileron" => self.aileron[side_of(p.name)] <= 0.0,
            "left flap" | "right flap" => self.flap[side_of(p.name)] <= 0.0,
            "rudder" => self.rudder <= 0.0,
            "spinner" | "blades" | "prop disc" => self.prop <= 0.0,
            "nose leg" | "nose fork" | "nose wheel" | "upper torque link" | "lower torque link" => !self.gear[0],
            "left main leg" | "left main wheel" => !self.gear[1],
            "right main leg" | "right main wheel" => !self.gear[2],
            // Internal structure only shows once the skin over it is torn.
            "left wing structure" => self.wing[0] >= SPAN,
            "right wing structure" => self.wing[1] >= SPAN,
            "tail structure" => self.stab[0] > 0.5 && self.stab[1] > 0.5 && self.fin >= 1.0,
            _ => false,
        }
    }

    /// The breaks that cut a piece as it is drawn on the aircraft.
    pub fn tears_for(&self, p: &Piece) -> [Tear; 2] {
        let mut t = [Tear::NONE; 2];
        let structure = p.name.ends_with("structure");
        // Spars poke out a little past the torn skin.
        let stick = if structure { 0.12 } else { 0.0 };
        match p.group {
            G_LWING | G_RWING => {
                let s = if p.group == G_RWING { 1 } else { 0 };
                if self.wing[s] < SPAN {
                    t[0] = Tear::new(Vec3::new(sgn(s), 0.0, 0.0), self.wing[s] + stick, 1.0);
                }
            }
            G_TAIL => match p.name {
                "elevator" => t[0] = halves(self.elevator, 0.12),
                "stabiliser" | "tail structure" => {
                    t[0] = halves(self.stab, 0.34 + stick);
                    if self.fin < 1.0 {
                        t[1] = Tear::new(Vec3::Y, 0.27 + 1.53 * self.fin + stick, 1.0);
                    }
                }
                "rudder" => {
                    if self.rudder < 1.0 {
                        t[0] = Tear::new(Vec3::Y, 0.22 + 1.57 * self.rudder, 1.0);
                    }
                }
                _ => {}
            },
            G_PROP => {
                if (p.name == "blades" || p.name == "prop disc") && self.prop > 0.0 && self.prop < 1.0 {
                    t[0] = Tear::new(Vec3::Y, airframe::PROP_HUB.y + 0.42, 1.0);
                }
            }
            _ => {}
        }
        t
    }

    /// Wing area outboard of span station `b` on one side (m²).
    pub fn wing_lost(b: f32) -> f32 {
        let chord = |s: f32| 1.65 - 0.5 * ((s - 2.55) / (SPAN - 2.55)).clamp(0.0, 1.0);
        let b = b.clamp(0.0, SPAN);
        let n = 12;
        (0..n).map(|k| chord(b + (SPAN - b) * (k as f32 + 0.5) / n as f32)).sum::<f32>() * (SPAN - b) / n as f32
    }
    /// How much of an aileron is still on a wing that ends at `b`.
    pub fn aileron_frac(b: f32) -> f32 {
        ((b - AILERON[0]) / (AILERON[1] - AILERON[0])).clamp(0.0, 1.0)
    }

    /// What is wrong, one line each, for the HUD.
    pub fn report(&self) -> Vec<String> {
        let mut r = Vec::new();
        for s in 0..2 {
            let lr = if s == 0 { "L" } else { "R" };
            if self.wing[s] < SPAN {
                r.push(format!("{lr} WING  -{:.1} m", SPAN - self.wing[s]));
            }
            if self.aileron[s] <= 0.0 {
                r.push(format!("{lr} AILERON GONE"));
            } else if self.aileron[s] < 1.0 {
                r.push(format!("{lr} AILERON HANGING"));
            }
            if self.flap[s] <= 0.0 {
                r.push(format!("{lr} FLAP GONE"));
            }
            if self.stab[s] < 0.5 {
                r.push(format!("{lr} STABILISER GONE"));
            } else if self.elevator[s] < 0.5 {
                r.push(format!("{lr} ELEVATOR GONE"));
            }
        }
        if self.fin < 1.0 {
            r.push(format!("FIN {:.0}%", self.fin * 100.0));
        }
        if self.rudder <= 0.0 {
            r.push("RUDDER GONE".into());
        } else if self.rudder < 1.0 {
            r.push(format!("RUDDER {:.0}%", self.rudder * 100.0));
        }
        if self.prop <= 0.0 {
            r.push("PROPELLER GONE".into());
        } else if self.prop < 1.0 {
            r.push("PROP BLADE BROKEN".into());
        }
        if self.engine <= 0.0 {
            r.push("ENGINE DEAD".into());
        } else if self.engine < 1.0 {
            r.push(format!("ENGINE {:.0}%  OIL LEAK", self.engine * 100.0));
        }
        for (g, name) in self.gear.iter().zip(["NOSE GEAR GONE", "L MAIN GEAR GONE", "R MAIN GEAR GONE"]) {
            if !g {
                r.push(name.into());
            }
        }
        if self.oil > 0.25 {
            r.push("OIL ON WINDSHIELD".into());
        }
        if self.crack[2] > 0.0 {
            r.push("WINDSHIELD CRACKED".into());
        }
        r
    }

    /// Fill in from a spec like `rwing=4.3,laileron=0,rudder=0.5,prop=0.5,engine=0.4,oil=0.6,crack=1,nosegear=0`
    /// (for testing from the command line).
    pub fn apply_spec(&mut self, spec: &str) {
        for item in spec.split(',') {
            let (k, v) = item.split_once('=').unwrap_or((item, "0"));
            let v: f32 = v.trim().parse().unwrap_or(0.0);
            match k.trim() {
                "lwing" => self.wing[0] = v,
                "rwing" => self.wing[1] = v,
                "laileron" => self.aileron[0] = v,
                "raileron" => self.aileron[1] = v,
                "lflap" => self.flap[0] = v,
                "rflap" => self.flap[1] = v,
                "lelevator" => self.elevator[0] = v,
                "relevator" => self.elevator[1] = v,
                "lstab" => self.stab[0] = v,
                "rstab" => self.stab[1] = v,
                "rudder" => self.rudder = v,
                "fin" => self.fin = v,
                "prop" => self.prop = v,
                "engine" => self.engine = v,
                "nosegear" => self.gear[0] = v > 0.5,
                "lgear" => self.gear[1] = v > 0.5,
                "rgear" => self.gear[2] = v > 0.5,
                "oil" => self.oil = v,
                "crack" => self.crack = [-1.45, 0.45, 0.22 * v],
                "scuff" => self.scuff = [v; GROUPS.len()],
                other => log::warn!("unknown damage item {other}"),
            }
        }
    }
}

/// Which pieces a shed request refers to.
#[derive(Clone, Copy, Debug)]
pub enum Sel {
    Group(usize),
    Names(&'static [&'static str]),
}

/// Something has just come off: the flight model asks the game to let it go.
#[derive(Clone, Copy, Debug)]
pub struct ShedReq {
    pub sel: Sel,
    /// the break, seen from the side that leaves (NONE: the whole piece goes)
    pub tear: Tear,
    /// extra velocity of the part, body frame
    pub kick: Vec3,
    pub spin: f32,
}

/// A part that has left the aircraft and is on its way down.
pub struct Shed {
    pub mesh: usize,
    /// the piece's pose at the moment it left
    local: Mat4,
    /// the point (after `local`) that `pos` tracks
    origin: Vec3,
    pos: DVec3,
    vel: DVec3,
    rot: DQuat,
    omega: DVec3,
    tear: Tear,
    cut: bool,
    radius: f64,
    span: f32,
    scuff: f32,
    seed: f32,
    age: f32,
    bounces: u8,
    resting: bool,
    sunk: bool,
}

#[derive(Default)]
pub struct Sheds {
    pub items: Vec<Shed>,
}

impl Sheds {
    /// Spawn whatever `req` tears off: every piece it names, cut along the tear.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(&mut self, req: &ShedReq, dmg: &Damage, pieces: &[Piece], piece_mesh: &[usize], piece_cut: &[bool], pose: &[Mat4], pos: DVec3, rot: DQuat, vel: DVec3, rng: &mut Rng, emit: &mut Vec<(DVec3, Particle)>) {
        let mut first = true;
        for (i, p) in pieces.iter().enumerate() {
            if p.glass || dmg.hidden(p) {
                continue;
            }
            let picked = match req.sel {
                Sel::Group(g) => p.group == g && !p.name.ends_with("structure"),
                Sel::Names(ns) => ns.contains(&p.name),
            };
            if !picked {
                continue;
            }
            // Where the leaving part is: the mean of its vertices, body frame.
            let (mut c, mut n, mut lo, mut hi) = (Vec3::ZERO, 0usize, Vec3::splat(1e9), Vec3::splat(-1e9));
            for v in &p.mesh.v {
                let q = Vec3::from(v.pos);
                if req.tear.keeps(q) {
                    c += q;
                    n += 1;
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
            if n < 3 {
                continue;
            }
            c /= n as f32;
            let ext = hi - lo;
            let origin = pose[i].transform_point3(c);
            let wpos = pos + rot * origin.as_dvec3();
            let kick = rot * (req.kick + rng.dir() * 1.5).as_dvec3();
            self.items.push(Shed {
                mesh: piece_mesh[i],
                local: pose[i],
                origin,
                pos: wpos,
                vel: vel * 0.97 + kick,
                rot,
                omega: rng.dir().as_dvec3() * (req.spin * rng.range(0.6, 1.3)) as f64,
                tear: req.tear,
                cut: piece_cut[i] || req.tear.is_some(),
                radius: (ext.max_element() * 0.5).clamp(0.1, 3.0) as f64,
                span: ext.max_element(),
                scuff: dmg.scuff[p.group].max(0.4),
                seed: rng.f(),
                age: 0.0,
                bounces: 0,
                resting: false,
                sunk: false,
            });
            if first {
                first = false;
                // A spray of rivets and shreds of skin where it let go.
                let up = wpos.normalize().as_vec3();
                for _ in 0..18 {
                    let d = rng.dir();
                    emit.push((wpos, Particle { vel: (vel.as_vec3() * 0.6 + d * rng.range(2.0, 9.0) + up * 2.0).into(), life: rng.range(0.4, 1.3), size0: 0.04, size1: 0.02, kind: SPARK, drag: 0.8, seed: rng.f(), alpha: 0.9, ..Default::default() }));
                }
                for _ in 0..6 {
                    let d = rng.dir();
                    emit.push((wpos, Particle { vel: (vel.as_vec3() * 0.5 + d * rng.range(1.0, 4.0)).into(), life: rng.range(1.5, 3.0), size0: 0.4, size1: 1.6, temp: 0.1, kind: GRIT, drag: 1.6, buoy: 0.3, seed: rng.f(), alpha: 0.3, ..Default::default() }));
                }
            }
        }
        if self.items.len() > 200 {
            self.items.drain(0..50);
        }
    }

    pub fn update(&mut self, dt: f32, world: &World) {
        let dtf = dt as f64;
        for s in self.items.iter_mut() {
            s.age += dt;
            if s.resting || s.sunk {
                continue;
            }
            let upw = s.pos.normalize();
            // Torn panels tumble: a lot of drag, more when they turn flat to the
            // airflow, and a sideways skitter as they do.
            let flat = 0.5 + 0.5 * (s.age as f64 * s.omega.length() * 0.7 + s.seed as f64 * 6.0).sin();
            let drag = (0.12 + 0.9 * flat) * (s.span as f64).sqrt() * 0.7;
            s.vel -= upw * 9.81 * dtf;
            s.vel *= (-dtf * drag).exp();
            let skitter = s.rot * DVec3::new((s.age as f64 * 3.1 + s.seed as f64 * 9.0).sin(), 0.0, (s.age as f64 * 2.3).cos()) * (flat * 2.5 * dtf);
            s.vel += skitter;
            s.pos += s.vel * dtf;
            let w = s.omega * dtf;
            if w.length_squared() > 1e-12 {
                s.rot = (DQuat::from_scaled_axis(w) * s.rot).normalize();
            }
            s.omega *= (-dtf * 0.12).exp();
            let (lat, lon, h) = geo::pos_to_lla(s.pos);
            let Some(g) = world.ground(lat, lon) else { continue };
            let mut floor = (g.h, g.water);
            if let Some(b) = world.building(lat, lon) {
                if h < b.top + 1.0 && (!b.bridge || h > b.base) {
                    floor = (b.top, false);
                }
            }
            let r = s.radius * 0.3;
            if h < floor.0 + r {
                if floor.1 {
                    s.sunk = true;
                    continue;
                }
                s.pos += upw * (floor.0 + r - h);
                let vn = s.vel.dot(upw);
                let tang = s.vel - upw * vn;
                if vn < -2.0 && s.bounces < 2 {
                    s.bounces += 1;
                    s.vel = tang * 0.5 - upw * vn * 0.2;
                    s.omega *= 0.5;
                } else if tang.length() > 1.0 && s.bounces < 6 {
                    s.bounces += 1;
                    s.vel = tang * 0.7;
                    s.omega *= 0.7;
                } else {
                    s.resting = true;
                    s.vel = DVec3::ZERO;
                    s.omega = DVec3::ZERO;
                }
            }
        }
        self.items.retain(|s| !s.sunk && s.age < 300.0);
    }

    pub fn draw(&self, cam: DVec3, params: [[f32; 4]; 6], objs: &mut Vec<DrawObj>) {
        for s in &self.items {
            let o = s.pos - cam;
            if o.length_squared() > 3000.0 * 3000.0 {
                continue;
            }
            let model = Mat4::from_translation(o.as_vec3()) * Mat4::from_quat(s.rot.as_quat()) * Mat4::from_translation(-s.origin) * s.local;
            objs.push(DrawObj {
                mesh: s.mesh,
                model,
                glow: [0.25, 0.0, 0.0, s.seed],
                params,
                cast: 0b11,
                cut: s.cut,
                tear: [s.tear.gpu(), [0.0; 4]],
                dmg: [s.tear.mode, 0.0, s.scuff, JAG],
                ..DrawObj::DEFAULT
            });
        }
    }
}

/// Which breakable group a hard point of the airframe belongs to (see `plane::HARD`).
pub fn hard_group(hi: usize) -> usize {
    match hi {
        0 | 6 => G_ENGINE,
        1 => G_LWING,
        2 => G_RWING,
        3 | 4 => G_TAIL,
        _ => G_FUSE,
    }
}
pub const GEAR_PIECES: [&[&str]; 3] = [&["nose leg", "nose fork", "nose wheel", "upper torque link", "lower torque link"], &["left main leg", "left main wheel"], &["right main leg", "right main wheel"]];
