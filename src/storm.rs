//! Thunderstorms up close: what the storm cells (weather.rs `storm_cells`) are
//! doing where the aircraft is (rain, cloud, up- and downdrafts), their
//! lightning, and the rain falling around the camera.
//!
//! Lightning: each mature cell flashes now and then, more often the bigger
//! and older it is. Most flashes stay in the cloud: a few pulses over a
//! fraction of a second, each lighting a different stretch of it from inside.
//! The rest come down to the ground: a stepped leader feels its way down
//! (faint), the return stroke blazes up the channel and its branches, and one
//! to four more strokes follow down the main channel tens of milliseconds
//! apart, each leaving a short afterglow. The channels are drawn by
//! precip.wgsl (`vs_bolt`); the light they throw on the clouds and the ground
//! goes to the shaders as up to `MAX_FLASHES` lit spans (common.wgsl
//! `flash_light`). The thunder follows at the speed of sound (audio.rs).
//!
//! The geometry of a cell here must match the shaders': the tower (common.wgsl
//! `storm_sd`), leaning downwind, and the rain shaft under its downwind half
//! (clouds.wgsl `storm_rain`).
use crate::geo;
use crate::weather::Weather;
use glam::DVec3;

/// Lit spans handed to the shaders at once (two vec4s each in Globals.flash).
pub const MAX_FLASHES: usize = 4;
/// Most lightning channel segments drawn in a frame.
pub const MAX_SEGS: usize = 4096;
/// Speed of sound, m/s.
const SOUND: f64 = 343.0;

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What a storm cell's air is doing at one point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Local {
    /// rain falling here, 0..1 (heavy under the core, light at the shaft's edge)
    pub rain: f64,
    /// inside the storm tower's cloud, 0..1
    pub cloud: f64,
    /// RMS turbulence the storm adds, m/s
    pub sigma: f64,
    /// vertical air motion, m/s (+ up): the updraft feeding the tower, the downdraft in the rain
    pub draft: f64,
    /// outflow along the ground from the downdraft (the gust front), planet frame m/s
    pub gust: DVec3,
}

/// A cell as seen from a point: (metres downwind of its centre, metres across
/// the wind, metres above the cloud base, tower radius m, maturity 0..1,
/// height of its tower above the base m), and the downwind and across axes.
struct At {
    dw0: f64,
    cx: f64,
    hb: f64,
    r: f64,
    age: f64,
    htop: f64,
    w: DVec3,
    x: DVec3,
}

/// The direction the wind aloft blows towards at `pos` (Globals.storm.xyz).
pub fn downwind(wx: &Weather, pos: DVec3) -> DVec3 {
    let w = wx.steady(pos, 2000.0);
    if w.length() > 0.1 { w.normalize() } else { geo::enu(pos).0 }
}

/// How high a cell grows, as a share of base..anvil (common.wgsl `storm_top`).
fn storm_top(age: f64) -> f64 {
    0.3 + 0.7 * smoothstep(0.0, 0.55, age)
}

fn cell_at(wx: &Weather, c: &[[f32; 4]; 2], pos: DVec3, w: DVec3) -> At {
    let un = pos.normalize();
    let x = un.cross(w).normalize_or_zero();
    let cen = DVec3::new(c[0][0] as f64, c[0][1] as f64, c[0][2] as f64);
    let dv = (un * 6371.0 + wx.cloud_off / 1000.0 - cen) * 1000.0;
    let alt = pos.length() - geo::R;
    let age = c[1][0] as f64;
    let at = c[1][1] as f64 - wx.base() as f64;
    At { dw0: dv.dot(w), cx: dv.dot(x), hb: alt - wx.base() as f64, r: c[0][3] as f64 * 1000.0, age, htop: at * storm_top(age), w, x }
}

/// How much rain a cell's shaft carries (clouds.wgsl `storm_rain`: mature, big enough).
fn shaft_amount(a: &At) -> f64 {
    smoothstep(0.45, 0.8, a.age) * smoothstep(1800.0, 3500.0, a.r)
}

/// The storm cells' rain, cloud and air at a point (planet frame) `agl` metres
/// above the ground, at game time `t`.
pub fn local(wx: &Weather, pos: DVec3, agl: f64, t: f64) -> Local {
    let mut o = Local::default();
    if wx.cells.is_empty() {
        return o;
    }
    let w = downwind(wx, pos);
    let up = pos.normalize();
    for (i, c) in wx.cells.iter().enumerate() {
        let a = cell_at(wx, c, pos, w);
        let r = a.r;
        if a.dw0.hypot(a.cx) > r * 3.2 || a.hb > a.htop + 2000.0 {
            continue;
        }
        let mature = smoothstep(0.5, 0.8, a.age);
        let size = smoothstep(2500.0, 8000.0, r);
        let amt = shaft_amount(&a);
        // The rain shaft hangs under the downwind half of the base.
        let rs = (a.dw0 - 0.45 * r).hypot(a.cx) / (0.8 * r);
        let heavy = 1.0 - smoothstep(0.35, 1.0, rs);
        let fringe = 1.0 - smoothstep(0.85, 1.45, rs);
        // The tower, leaning downwind with height.
        let hb = a.hb.max(0.0);
        let hc = (hb / a.htop.max(1.0)).min(1.2);
        let dw = a.dw0 - hb * (0.12 + 0.1 * mature);
        let rt = 0.82 * r * (1.0 - 0.28 * smoothstep(0.0, 0.9, hc));
        let rr = dw.hypot(a.cx) / rt;
        let inside = if a.hb >= 0.0 && a.hb < a.htop { 1.0 - smoothstep(0.85, 1.08, rr) } else { 0.0 };
        o.cloud = o.cloud.max(inside);
        let rain = if a.hb < 0.0 {
            amt * heavy.max(0.3 * fringe)
        } else {
            // In the cloud: rain under the freezing level, graupel and ice above.
            let core = smoothstep(0.3, 0.7, a.age) * (1.0 - smoothstep(0.4, 1.0, rr)) * (1.0 - 0.65 * smoothstep(0.35, 0.6, hc));
            core.max(amt * heavy * (1.0 - smoothstep(0.0, 1500.0, a.hb))) * (if a.hb < a.htop { 1.0 } else { 0.0 })
        };
        o.rain = o.rain.max(rain);

        // Drafts: the updraft rises through the core (inflow from under the
        // base upwind), strongest halfway up; the downdraft comes down in the
        // rain and spreads out over the ground as the gust front.
        let ru = (dw + 0.25 * r).hypot(a.cx) / (0.6 * r);
        let core = (-ru * ru).exp();
        let up_prof = if a.hb < 0.0 { 0.15 + 0.35 * smoothstep(-1500.0, 0.0, a.hb) } else { (1.0 - smoothstep(0.55, 1.05, hc)) * (0.75 + 0.5 * smoothstep(0.0, 0.5, hc)) };
        let shaft = (-(rs / 0.85).powi(2)).exp();
        let dn_prof = if a.hb < 0.0 { smoothstep(0.0, 300.0, agl) } else { 1.0 - smoothstep(0.0, 0.5, hc) };
        o.draft += a.age.min(1.0) * ((7.0 + 13.0 * size) * core * up_prof) - amt * (5.0 + 7.0 * size) * shaft * dn_prof;
        // Turbulence: severe in the cloud and where updraft meets downdraft,
        // moderate in the rain, light around the cell. It comes in bursts.
        let shear = core * shaft * 4.0;
        let near = 1.0 - smoothstep(1.0, 3.0, a.dw0.hypot(a.cx) / r);
        let s = a.age.min(1.0) * (1.2 * near + 4.5 * inside * (0.6 + 0.4 * size) + 2.8 * amt * heavy + 3.0 * shear.min(1.0));
        let k = i as f64 * 1.731;
        let burst = ((0.21 * t + k).sin() * 0.6 + (0.53 * t + 2.3 * k).sin() * 0.4 + (1.37 * t + 4.1 * k).sin() * 0.25) / 1.25;
        o.sigma = o.sigma.max(s * (0.6 + 0.65 * smoothstep(-0.2, 0.7, burst)));
        // Gust front: the downdraft fans out from under the shaft, low down.
        if a.hb < 0.0 && amt > 0.0 {
            let out = a.w * (a.dw0 - 0.45 * r) + a.x * a.cx;
            let d = out.length();
            if d > 1.0 {
                let ring = (d / (0.8 * r)) * (-(d / (1.3 * r)).powi(2)).exp() * 1.6;
                let low = 1.0 - smoothstep(150.0, 900.0, agl);
                let dir = (out - up * out.dot(up)) / d;
                o.gust += dir * (amt * (6.0 + 10.0 * size) * ring * low);
            }
        }
    }
    o.rain = o.rain.min(1.0);
    o
}

/// The storm cells (of those handed to the shaders) close enough to the camera
/// to darken the ground in view, as a bit mask (Globals.stormfx.x).
pub fn near_mask(wx: &Weather, cam: DVec3) -> u32 {
    let mut m = 0;
    for (i, c) in wx.cells.iter().enumerate().take(24) {
        let cen = DVec3::new(c[0][0] as f64, c[0][1] as f64, c[0][2] as f64) * 1000.0 - wx.cloud_off;
        let d = (cen.normalize() - cam.normalize()).length() * geo::R;
        if d < c[0][3] as f64 * 1000.0 * 6.5 + 40_000.0 {
            m |= 1 << i;
        }
    }
    m
}

/// How deep in a storm's shade a point is, 0..1: under the tower and its rain
/// nearly all the sky is dark cloud; under the anvil a thick grey lid. Must
/// match common.wgsl `storm_shade`.
pub fn shade(wx: &Weather, pos: DVec3) -> f64 {
    let w = downwind(wx, pos);
    let mut dark = 0.0f64;
    for c in &wx.cells {
        let a = cell_at(wx, c, pos, w);
        let r = a.r;
        let rc = (a.dw0 - 0.25 * r).hypot(a.cx) / r;
        let core = (1.0 - smoothstep(0.7, 1.7, rc)) * smoothstep(0.3, 0.75, a.age) * smoothstep(1500.0, 4000.0, r);
        let mature = smoothstep(0.5, 0.8, a.age);
        let an = (r * (1.0 + 4.5 * mature), r * (0.8 + 0.35 * mature), r * (1.0 + 0.6 * mature));
        let lc = an.2 + a.dw0.max(0.0) * 0.45;
        let rn = (a.dw0 / if a.dw0 > 0.0 { an.0 } else { an.1 }).hypot(a.cx / lc);
        let lid = (1.0 - smoothstep(0.5, 1.05, rn)) * 0.55 * mature;
        dark = dark.max(core.max(lid));
    }
    let alt = pos.length() - geo::R;
    dark * (1.0 - smoothstep(wx.base() as f64, wx.base() as f64 + 3000.0, alt))
}

/// Lightning that stays in the cloud, or comes down to the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Cloud,
    Ground,
}

/// A straight piece of a lightning channel.
#[derive(Clone, Copy, Debug)]
struct Seg {
    a: DVec3,
    b: DVec3,
    /// drawn width, m
    w: f32,
    /// 0 the main channel, 1 a branch, 2 a branch of a branch
    level: u8,
    /// when the leader reaches it, 0..1 of the leader's time
    t: f32,
    /// brightness along it: branches fade out towards their tips
    f: f32,
}

/// One flash: its channel and its strokes.
struct Flash {
    kind: Kind,
    /// seconds since the first return stroke (negative while the leader comes down)
    age: f64,
    leader: f64,
    /// (time after the first stroke, strength)
    strokes: Vec<(f64, f64)>,
    /// for each stroke, the stretch of channel it lights (planet frame)
    spans: Vec<[DVec3; 2]>,
    segs: Vec<Seg>,
    power: f64,
    end: f64,
}

/// A thunderclap on its way: how long until it arrives, the nearest and
/// farthest points of its channel (m), how strong, and where it comes from.
#[derive(Clone, Copy, Debug)]
pub struct Thunder {
    pub delay: f64,
    pub near: f64,
    pub far: f64,
    pub power: f64,
    pub from: DVec3,
}

/// A strike forced by the command line (`--strike`): game time of its first
/// return stroke, kind, distance (km) and compass bearing from the aircraft.
#[derive(Clone, Copy, Debug)]
pub struct Forced {
    pub at: f64,
    pub kind: Kind,
    pub km: f64,
    pub bearing: f64,
}

pub struct Lightning {
    flashes: Vec<Flash>,
    rng: u64,
    pub forced: Vec<Forced>,
    /// thunder set off since last taken (main.rs hands it to the sound)
    pub thunder: Vec<Thunder>,
}

/// Brightness of one stroke `d` seconds after it struck: the stroke itself,
/// then the glow of the current still flowing. The frame it strikes in shows
/// it at full strength (a camera's exposure would catch the whole flash).
fn stroke(d: f64, dt: f64) -> f64 {
    if d < 0.0 {
        return 0.0;
    }
    let e = (d - dt).max(0.0);
    (-e / 0.035).exp() + 0.22 * (-e / 0.16).exp()
}

impl Default for Lightning {
    fn default() -> Self {
        Self::new()
    }
}

impl Lightning {
    pub fn new() -> Self {
        Self { flashes: Vec::new(), rng: 0x9e37_79b9_7f4a_7c15, forced: Vec::new(), thunder: Vec::new() }
    }

    fn u(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.u()
    }
    fn unit(&mut self) -> DVec3 {
        loop {
            let v = DVec3::new(self.u(), self.u(), self.u()) * 2.0 - 1.0;
            let l = v.length_squared();
            if l > 1e-4 && l <= 1.0 {
                return v / l.sqrt();
            }
        }
    }

    /// A tortuous channel from a to b by midpoint displacement: `levels`
    /// halvings, each new point pushed sideways by up to `rough` of its piece's
    /// length on the coarse levels and much less on the fine ones, so a bolt is
    /// a few long, crooked strokes with medium kinks, not a fizzy squiggle at
    /// every scale (equal roughness per level made a self-similar fractal that
    /// read as electric fuzz). Branches (`fade`) dim towards their tips.
    fn channel(&mut self, a: DVec3, b: DVec3, levels: u32, rough: f64, w: f32, level: u8, t0: f32, t1: f32, fade: bool, out: &mut Vec<Seg>) -> Vec<DVec3> {
        const PROFILE: [f64; 8] = [1.6, 1.45, 1.35, 1.25, 0.9, 0.42, 0.18, 0.1];
        let mut pts = vec![a, b];
        let axis = (b - a).normalize_or_zero();
        for l in 0..levels {
            let mut next = Vec::with_capacity(pts.len() * 2);
            let k = rough * PROFILE[(l as usize).min(7)];
            for i in 0..pts.len() - 1 {
                let (p, q) = (pts[i], pts[i + 1]);
                let len = p.distance(q);
                let mut off = self.unit();
                off -= axis * off.dot(axis) * 0.7;
                // Kinks off-centre, so the pieces are of uneven length.
                let m = self.range(0.3, 0.7);
                next.push(p);
                next.push(p + (q - p) * m + off * len * k);
            }
            next.push(*pts.last().unwrap());
            pts = next;
        }
        let n = pts.len() - 1;
        for i in 0..n {
            let t = t0 + (t1 - t0) * (i as f32 / n as f32);
            let f = if fade { 1.0 - 0.85 * ((i as f32 + 0.5) / n as f32).powf(0.8) } else { 1.0 };
            out.push(Seg { a: pts[i], b: pts[i + 1], w, level, t, f });
        }
        pts
    }

    /// Side branches off a channel's points: forking down and out, shorter
    /// lower down, some forking again.
    fn branches(&mut self, pts: &[DVec3], up: DVec3, ground: f64, count: usize, level: u8, t0: f32, t1: f32, out: &mut Vec<Seg>) {
        let n = pts.len();
        if n < 4 {
            return;
        }
        for _ in 0..count {
            let k = ((self.u().powf(1.3) * 0.8 + 0.02) * (n - 2) as f64) as usize;
            let p = pts[k];
            let dir = (pts[(k + 1).min(n - 1)] - p).normalize_or_zero();
            let h = (p.length() - geo::R - ground).max(50.0);
            let len = h * self.range(0.25, 0.6) / (1.0 + level as f64);
            let side = self.unit();
            let mut d = (dir * 0.6 + (side - up * side.dot(up)).normalize_or_zero() * 0.8 - up * 0.35).normalize_or_zero();
            if level == 1 && self.u() < 0.3 {
                d = (d + up * 0.3).normalize_or_zero();
            }
            let tb = t0 + (t1 - t0) * (k as f32 / n as f32);
            let w = if level == 1 { 0.45 } else { 0.25 };
            let lv = if level == 1 { 5 } else { 4 };
            let bp = self.channel(p, p + d * len, lv, 0.2, w, level, tb, (tb + 0.5 * (t1 - tb)).min(t1), true, out);
            if level == 1 && self.u() < 0.35 {
                let nb = 1;
                self.branches(&bp, up, ground, nb, 2, tb, t1, out);
            }
        }
    }

    /// A cloud-to-ground flash from `top` (in the cloud) down to `foot` on the ground.
    fn ground_flash(&mut self, top: DVec3, foot: DVec3, ground: f64, power: f64, forced: bool) -> Flash {
        let up = foot.normalize();
        let mut segs = Vec::new();
        // The part in the cloud runs off sideways from where the leader started.
        let h = top.distance(foot);
        let pts = self.channel(top, foot, 7, 0.17, 1.0, 0, 0.0, 1.0, false, &mut segs);
        let nb = 2 + (self.u() * 3.5) as usize;
        self.branches(&pts, up, ground, nb, 1, 0.0, 1.0, &mut segs);
        let side = self.unit();
        let root = top + (side - up * side.dot(up)).normalize_or_zero() * h * 0.35 + up * h * 0.08;
        self.channel(top, root, 5, 0.22, 0.7, 1, 0.0, 0.0, true, &mut segs);
        // Strokes: the first and strongest, then up to four more down the same channel.
        let mut strokes = vec![(0.0, 1.0)];
        let n = if forced { 4 } else { 1 + (self.u().powf(0.8) * 4.0) as usize };
        let mut t = 0.0;
        for _ in 1..n {
            t += if forced { 0.075 } else { self.range(0.035, 0.12) };
            let a = if forced { 0.7 } else { self.range(0.35, 0.85) };
            strokes.push((t, a));
        }
        let spans = vec![[top, foot]; strokes.len()];
        Flash { kind: Kind::Ground, age: 0.0, leader: if forced { 0.03 } else { self.range(0.02, 0.035) }, end: t + 0.5, strokes, spans, segs, power }
    }

    /// An in-cloud flash around `c`: a few pulses, each lighting another
    /// stretch of the cloud, and a faint channel that shows only where the
    /// cloud is thin.
    fn cloud_flash(&mut self, c: DVec3, spread: f64, power: f64, forced: bool) -> Flash {
        let up = c.normalize();
        let flat = |v: DVec3| (v - up * v.dot(up) * 0.75).normalize_or_zero();
        let n = if forced { 5 } else { 3 + (self.u() * 6.0) as usize };
        let mut strokes = Vec::new();
        let mut spans = Vec::new();
        let mut t = 0.0;
        for k in 0..n {
            let a = if forced && k == 0 { 1.0 } else { self.range(0.25, 1.0) };
            strokes.push((t, a));
            let o = flat(self.unit()) * spread * self.range(0.0, 0.6);
            let d = flat(self.unit()) * spread * self.range(0.3, 0.9);
            spans.push([c + o - d, c + o + d]);
            t += self.range(0.04, 0.16);
        }
        let mut segs = Vec::new();
        let [a, b] = spans[0];
        let pts = self.channel(a, b, 6, 0.2, 0.7, 0, 0.0, 0.0, false, &mut segs);
        let nb = 2 + (self.u() * 4.0) as usize;
        for _ in 0..nb {
            let p = pts[(self.u() * (pts.len() - 1) as f64) as usize];
            let d = flat(self.unit()) * spread * self.range(0.2, 0.6);
            self.channel(p, p + d, 4, 0.22, 0.4, 1, 0.0, 0.0, true, &mut segs);
        }
        Flash { kind: Kind::Cloud, age: 0.0, leader: 0.0, end: t + 0.3, strokes, spans, segs, power }
    }

    /// Advance by `dt` game seconds at game time `time`: let the cells flash,
    /// set off forced strikes, retire finished flashes. `cam` is where the
    /// thunder is heard; `ground(p)` the height of the ground under p (m).
    pub fn step(&mut self, dt: f64, time: f64, wx: &Weather, plane: DVec3, cam: DVec3, ground: &dyn Fn(DVec3) -> f64) {
        if dt <= 0.0 {
            return;
        }
        for f in &mut self.flashes {
            f.age += dt;
        }
        self.flashes.retain(|f| f.age < f.end);
        let base = wx.base() as f64;
        // Forced strikes begin their leader just ahead of the stroke time.
        let mut i = 0;
        while i < self.forced.len() {
            let fs = self.forced[i];
            if time + 0.04 < fs.at {
                i += 1;
                continue;
            }
            self.forced.swap_remove(i);
            let (e, n, up) = geo::enu(plane);
            let b = fs.bearing.to_radians();
            let dir = e * b.sin() + n * b.cos();
            let a = fs.km * 1000.0 / geo::R;
            let un = (up * a.cos() + dir * a.sin()).normalize();
            let g = ground(un * geo::R).max(0.0);
            let mut f = match fs.kind {
                Kind::Ground => {
                    let side = self.unit();
                    let top = un * (geo::R + base + 3200.0) + (side - un * side.dot(un)) * 500.0;
                    self.ground_flash(top, un * (geo::R + g), g, 1.2, true)
                }
                Kind::Cloud => self.cloud_flash(un * (geo::R + base + 4500.0), 3500.0, 1.0, true),
            };
            f.age = time - fs.at;
            self.sound(&f, cam);
            self.flashes.push(f);
        }
        // Each cell flashes at random, more often the bigger and more mature it is.
        let w = downwind(wx, cam);
        for ci in 0..wx.cells.len() {
            let c = wx.cells[ci];
            let r = c[0][3] as f64 * 1000.0;
            let age = c[1][0] as f64;
            let strength = smoothstep(0.55, 0.95, age) * smoothstep(2500.0, 8000.0, r) * (0.5 + 0.5 * wx.storms.max(0.6) as f64);
            // A big mature cell: ~10 flashes a minute.
            let rate = strength * 10.0 / 60.0;
            if rate <= 0.0 || self.flashes.len() >= 12 || self.u() >= rate * dt {
                continue;
            }
            let cen = DVec3::new(c[0][0] as f64, c[0][1] as f64, c[0][2] as f64) * 1000.0 - wx.cloud_off;
            let un = cen.normalize();
            let x = un.cross(w).normalize_or_zero();
            let at = c[1][1] as f64 - base;
            let htop = at * storm_top(age);
            let power = self.range(0.6, 1.3) * (0.6 + 0.4 * strength);
            let f = if self.u() < 0.3 {
                // Down to the ground, mostly in and around the rain shaft;
                // now and then a stronger one out from under the anvil.
                let out = self.u() < 0.08;
                let (along, across) = if out { (self.range(1.4, 2.6) * r, self.range(-0.8, 0.8) * r) } else { (0.45 * r + self.range(-0.9, 0.9) * r * 0.8, self.range(-0.9, 0.9) * r * 0.75) };
                let foot_dir = (un * geo::R + w * along + x * across).normalize();
                let g = ground(foot_dir * geo::R).max(0.0);
                let hstart = if out { at - 2500.0 } else { self.range(2000.0, 4500.0).min(htop * 0.6) };
                let lean = hstart * 0.15;
                let top = (un * geo::R + w * (along * 0.4 + lean) + x * across * 0.4).normalize() * (geo::R + base + hstart);
                self.ground_flash(top, foot_dir * (geo::R + g), g, power * if out { 2.0 } else { 1.0 }, false)
            } else {
                // In the cloud: mostly in the middle of the tower, sometimes out in the anvil.
                let anvil = age > 0.7 && self.u() < 0.3;
                let (along, h) = if anvil { (self.range(1.0, 3.5) * r, at - self.range(800.0, 2500.0)) } else { (self.range(-0.4, 0.5) * r, htop * self.range(0.35, 0.8)) };
                let across = self.range(-0.5, 0.5) * r;
                let p = (un * geo::R + w * (along + h * 0.15) + x * across).normalize() * (geo::R + base + h);
                self.cloud_flash(p, (0.5 * r).clamp(1500.0, 4500.0), power * 0.8, false)
            };
            let mut f = f;
            f.age = -f.leader;
            self.sound(&f, cam);
            self.flashes.push(f);
        }
    }

    /// Queue the flash's thunder: it starts when the sound from the nearest
    /// bit of channel arrives and rolls on until that from the farthest has.
    fn sound(&mut self, f: &Flash, cam: DVec3) {
        let (mut near, mut far, mut at) = (f64::MAX, 0.0f64, DVec3::ZERO);
        for s in f.segs.iter().filter(|s| s.level == 0) {
            let d = s.a.distance(cam);
            if d < near {
                (near, at) = (d, s.a);
            }
            far = far.max(d);
        }
        if near == f64::MAX || near > 40_000.0 {
            return;
        }
        let power = f.power * if f.kind == Kind::Ground { 1.0 } else { 0.55 };
        self.thunder.push(Thunder { delay: near / SOUND + (-f.age).max(0.0), near, far, power, from: (at - cam).normalize_or_zero() });
    }

    /// Light given off by a flash now (relative to its peak), and which of
    /// its strokes is the latest.
    fn light(f: &Flash, dt: f64) -> (f64, usize) {
        let mut b = 0.0;
        let mut last = 0;
        for (k, &(ts, amp)) in f.strokes.iter().enumerate() {
            if f.age >= ts {
                last = k;
            }
            b += amp * stroke(f.age - ts, dt);
        }
        (b * f.power, last)
    }

    /// The flashes lighting the scene this frame, brightest (as seen from the
    /// camera) first: per flash, the two ends of the lit stretch (camera-relative)
    /// + brightness, and the kind. Returns how many.
    pub fn globals(&self, cam: DVec3, dt: f64, out: &mut [[f32; 4]; 2 * MAX_FLASHES]) -> usize {
        let mut lit: Vec<(f64, [[f32; 4]; 2])> = Vec::new();
        for f in &self.flashes {
            let (b, k) = Self::light(f, dt);
            if b < 0.004 {
                continue;
            }
            let [a, e] = f.spans[k.min(f.spans.len() - 1)];
            let d = ((a + e) * 0.5).distance(cam);
            let v = |p: DVec3, w: f64| [(p - cam).x as f32, (p - cam).y as f32, (p - cam).z as f32, w as f32];
            lit.push((b / (1.0 + (d / 20000.0).powi(2)), [v(a, b), v(e, if f.kind == Kind::Ground { 1.0 } else { 0.0 })]));
        }
        lit.sort_by(|x, y| y.0.total_cmp(&x.0));
        for (i, (_, l)) in lit.iter().take(MAX_FLASHES).enumerate() {
            out[2 * i] = l[0];
            out[2 * i + 1] = l[1];
        }
        lit.len().min(MAX_FLASHES)
    }

    /// The channels to draw this frame, camera-relative: per segment one end +
    /// width (m), the other end + brightness (precip.wgsl `vs_bolt`).
    pub fn segments(&self, cam: DVec3, dt: f64, out: &mut Vec<[f32; 8]>) {
        for f in &self.flashes {
            let mut main = 0.0;
            let mut side = 0.0;
            for (k, &(ts, amp)) in f.strokes.iter().enumerate() {
                let s = amp * stroke(f.age - ts, dt);
                main += s;
                // Only the first stroke lights the branches; later ones follow the main channel.
                side += s * if k == 0 { 1.0 } else { 0.08 };
            }
            // The stepped leader: a faint, branching feeler working its way down.
            let lead = if f.age < 0.0 && f.leader > 0.0 { Some(((f.age + f.leader) / f.leader) as f32) } else { None };
            let kind = if f.kind == Kind::Ground { 1.0 } else { 0.3 };
            for s in &f.segs {
                let b = match lead {
                    Some(p) if s.t <= p => 0.05 * (0.3 + 0.7 * ((s.t - p + 0.15) / 0.15).clamp(0.0, 1.0)) as f64,
                    Some(_) => 0.0,
                    None => if s.level == 0 { main } else { side * if s.level == 1 { 0.55 } else { 0.35 } },
                } * f.power.min(1.5)
                    * kind
                    * s.f as f64;
                if b < 0.002 || out.len() >= MAX_SEGS {
                    continue;
                }
                let (a, e) = (s.a - cam, s.b - cam);
                out.push([a.x as f32, a.y as f32, a.z as f32, s.w, e.x as f32, e.y as f32, e.z as f32, b as f32]);
            }
        }
    }

}

/// The rain falling around the camera: how far it has moved past the camera
/// (wrapped), for precip.wgsl's drops to keep their places from frame to frame.
#[derive(Default)]
pub struct RainField {
    /// metres along east, north, up (wrapped every `WRAP` m)
    pub off: DVec3,
    prev_cam: Option<DVec3>,
    /// the rain's velocity relative to the camera, planet frame, m/s
    pub rel: DVec3,
}

/// Every drop box size (precip.wgsl RAIN_BOX) divides this, metres.
const WRAP: f64 = 200.0;
/// How fast a typical raindrop falls, m/s.
const FALL: f64 = 7.5;

impl RainField {
    /// Advance by `dt` with the camera at `cam` and the wind there `wind` (planet frame).
    pub fn step(&mut self, dt: f64, cam: DVec3, wind: DVec3) {
        let v_cam = match self.prev_cam {
            Some(p) if dt > 0.0 => {
                let v = (cam - p) / dt;
                if v.length() < 500.0 { v } else { DVec3::ZERO }
            }
            _ => DVec3::ZERO,
        };
        self.prev_cam = Some(cam);
        if dt <= 0.0 {
            return;
        }
        let (e, n, u) = geo::enu(cam);
        let rel = wind - u * FALL - v_cam;
        // Smoothed a little: the chase camera's spring jitters frame to frame.
        self.rel += (rel - self.rel) * (dt * 20.0).min(1.0);
        let d = self.rel * dt;
        self.off = DVec3::new((self.off.x + d.dot(e)).rem_euclid(WRAP), (self.off.y + d.dot(n)).rem_euclid(WRAP), (self.off.z + d.dot(u)).rem_euclid(WRAP));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storm() -> (Weather, DVec3) {
        let mut wx = Weather::new(0.4);
        wx.storms = 0.0;
        (wx.wind, wx.wind_from) = (8.0, 270.0);
        let p = geo::lla_to_pos(0.65, -2.14, 600.0);
        wx.place_storm(p, 0.0, 0.0, 6.0);
        wx.cells = wx.storm_cells(p);
        (wx, p)
    }

    #[test]
    fn rain_under_the_shaft_not_in_clear_air() {
        let (wx, p) = storm();
        let (e, _, u) = geo::enu(p);
        let w = downwind(&wx, p);
        assert!(w.dot(e) > 0.9, "a westerly blows east");
        // Under the downwind half of the base: heavy rain and a downdraft.
        let under = (p + e * 2700.0).normalize() * (geo::R + 600.0);
        let l = local(&wx, under, 600.0, 0.0);
        assert!(l.rain > 0.8 && l.draft < -3.0 && l.sigma > 2.0, "{l:?}");
        // Out at the edge: light rain.
        let edge = (p + e * 7500.0).normalize() * (geo::R + 600.0);
        let l = local(&wx, edge, 600.0, 0.0);
        assert!(l.rain > 0.02 && l.rain < 0.5, "{l:?}");
        // Far off in clear air: nothing.
        let far = (p + e * 30000.0).normalize() * (geo::R + 600.0);
        let l = local(&wx, far, 600.0, 0.0);
        assert!(l.rain == 0.0 && l.sigma < 0.1, "{l:?}");
        // In the tower's heart: cloud, updraft.
        let core = (p - e * 1000.0).normalize() * (geo::R + wx.base() as f64 + 3000.0);
        let l = local(&wx, core, 4000.0, 0.0);
        assert!(l.cloud > 0.9 && l.draft > 5.0, "{l:?}");
        let _ = u;
    }

    #[test]
    fn lightning_flashes_and_thunders() {
        let (wx, p) = storm();
        let mut lt = Lightning::new();
        lt.forced.push(Forced { at: 1.0, kind: Kind::Ground, km: 3.0, bearing: 90.0 });
        let mut t = 0.0;
        let mut peak = 0.0f32;
        let mut segs = Vec::new();
        while t < 1.2 {
            lt.step(1.0 / 60.0, t, &wx, p, p, &|_| 0.0);
            let mut g = [[0.0; 4]; 2 * MAX_FLASHES];
            if lt.globals(p, 1.0 / 60.0, &mut g) > 0 {
                peak = peak.max(g[0][3]);
            }
            if (t - 1.0).abs() < 0.01 {
                lt.segments(p, 1.0 / 60.0, &mut segs);
            }
            t += 1.0 / 60.0;
        }
        assert!(peak > 0.8, "{peak}");
        assert!(segs.len() > 100, "{}", segs.len());
        let th = lt.thunder[0];
        assert!((th.delay - th.near / 343.0).abs() < 0.1 && th.near > 2500.0 && th.far > th.near, "{th:?}");
        // A storm left alone flashes now and then on its own.
        let mut lt = Lightning::new();
        let mut n = 0;
        for i in 0..36000 {
            lt.step(1.0 / 60.0, i as f64 / 60.0, &wx, p, p, &|_| 0.0);
            n += lt.thunder.drain(..).count();
        }
        assert!((20..=200).contains(&n), "{n} thunderclaps in 10 minutes");
    }
}
