//! The aircraft's model: a four-seat, high-wing single with retractable gear,
//! built procedurally from analytic surfaces (a lofted superellipse fuselage,
//! NACA-section wings and tail, a twisted propeller) so that panel seams,
//! rivet rows, fasteners, lights and decals can be placed exactly on the skin.
//! The cockpit lives in `cabin.rs`; materials are drawn by `plane.wgsl`.
//!
//! Body axes: +X right, +Y up, -Z forward. Every piece keeps its vertices in
//! the body frame at rest, so the shader can paint the livery and wear from
//! the body-space position whatever the piece is doing.
use crate::render::PartVertex;
use glam::{Mat4, Quat, Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

// ---------------------------------------------------------------- materials

/// Material ids (vertex `mat.x`); keep in step with plane.wgsl.
pub mod mat {
    pub const PAINT: f32 = 0.0; // glossy paint, colour from the vertex
    pub const LIVERY: f32 = 1.0; // glossy exterior paint, colour from the paint scheme
    pub const ALU: f32 = 2.0; // bare aluminium
    pub const STEEL: f32 = 3.0; // machined / chromed steel
    pub const RUBBER: f32 = 4.0;
    pub const PLASTIC: f32 = 5.0; // textured interior plastic
    pub const FABRIC: f32 = 6.0; // upholstery, carpet, headliner
    pub const GLASS: f32 = 7.0;
    pub const LAMP: f32 = 8.0; // light lens; extra = which light
    pub const GAUGE: f32 = 9.0; // instrument face; extra = which instrument
    pub const BLACK: f32 = 10.0; // matte black finish
    pub const EXHAUST: f32 = 11.0;
    pub const SCREW: f32 = 12.0; // screw head, uv = local frame
    pub const PROP: f32 = 13.0;
    pub const DISPLAY: f32 = 14.0; // radio readout; extra = which display
    pub const SATIN: f32 = 15.0; // satin paint / primer
    pub const PROPDISC: f32 = 16.0; // the blur of a spinning propeller
}
/// Surface patterns (vertex `mat.y`); keep in step with plane.wgsl.
pub mod pat {
    pub const FUSE: f32 = 1.0; // fuselage skin: stations, stringers, doors, windows, wells
    pub const WING: f32 = 2.0;
    pub const CTRL: f32 = 3.0; // control surface with stiffening beads
    pub const COWL: f32 = 4.0;
    pub const TAIL: f32 = 5.0;
    pub const TREAD: f32 = 6.0;
    pub const LINING: f32 = 7.0; // cabin side walls and headliner (windows cut out)
    pub const SEAT: f32 = 8.0; // pleated upholstery
    pub const CARPET: f32 = 9.0;
    pub const PANEL: f32 = 10.0; // crinkle-finish instrument panel
    pub const PLACARD: f32 = 11.0; // small printed label
    pub const KNURL: f32 = 12.0;
}
/// Light ids (vertex `mat.w` on LAMP); keep in step with plane.wgsl.
pub mod lamp {
    pub const NAV_RED: f32 = 0.0;
    pub const NAV_GREEN: f32 = 1.0;
    pub const TAIL: f32 = 2.0;
    pub const STROBE: f32 = 3.0;
    pub const BEACON: f32 = 4.0;
    pub const LANDING: f32 = 5.0;
    pub const GEAR_DOWN: f32 = 6.0;
    pub const GEAR_UP: f32 = 7.0;
    pub const ANNUNC: f32 = 8.0;
}

#[derive(Clone, Copy)]
pub struct Mat {
    pub id: f32,
    pub pat: f32,
    pub col: Vec3,
    pub extra: f32,
}
pub const fn mt(id: f32, r: f32, g: f32, b: f32) -> Mat {
    Mat { id, pat: 0.0, col: Vec3::new(r, g, b), extra: 0.0 }
}
impl Mat {
    pub const fn pat(mut self, p: f32) -> Self {
        self.pat = p;
        self
    }
    pub const fn x(mut self, e: f32) -> Self {
        self.extra = e;
        self
    }
}

// Common finishes (linear colour).
pub const WHITE: Mat = mt(mat::LIVERY, 0.80, 0.80, 0.78);
pub const PAINT_WHITE: Mat = mt(mat::PAINT, 0.80, 0.80, 0.78);
pub const ALU: Mat = mt(mat::ALU, 0.62, 0.63, 0.65);
pub const STEEL: Mat = mt(mat::STEEL, 0.55, 0.56, 0.58);
pub const CHROME: Mat = mt(mat::STEEL, 0.85, 0.86, 0.88).x(1.0);
pub const RUBBER: Mat = mt(mat::RUBBER, 0.018, 0.018, 0.02);
pub const BLACK: Mat = mt(mat::BLACK, 0.025, 0.025, 0.027);
pub const PRIMER: Mat = mt(mat::SATIN, 0.20, 0.23, 0.19);
pub const GREY: Mat = mt(mat::SATIN, 0.18, 0.18, 0.18);
pub const SCREW_STEEL: Mat = mt(mat::SCREW, 0.55, 0.56, 0.58);
pub const SCREW_PAINT: Mat = mt(mat::SCREW, 0.78, 0.78, 0.76).x(1.0);
pub const SCREW_BLACK: Mat = mt(mat::SCREW, 0.03, 0.03, 0.03).x(2.0);

// ---------------------------------------------------------------- mesh building

#[derive(Default, Clone)]
pub struct Mesh {
    pub v: Vec<PartVertex>,
    pub i: Vec<u32>,
}

type SP = (Vec3, [f32; 2], f32);

impl Mesh {
    pub fn push(&mut self, p: Vec3, n: Vec3, uv: [f32; 2], m: &Mat, wear: f32) -> u32 {
        self.v.push(PartVertex { pos: p.into(), nrm: n.normalize_or_zero().into(), col: [m.col.x, m.col.y, m.col.z, 1.0], uv, mat: [m.id, m.pat, wear, m.extra] });
        (self.v.len() - 1) as u32
    }
    pub fn append(&mut self, o: &Mesh) {
        let b = self.v.len() as u32;
        self.v.extend_from_slice(&o.v);
        self.i.extend(o.i.iter().map(|i| i + b));
    }
    /// Move everything from vertex `from` on by `t` (rigid).
    pub fn transform(&mut self, from: usize, t: Mat4) {
        for v in &mut self.v[from..] {
            v.pos = t.transform_point3(v.pos.into()).into();
            v.nrm = t.transform_vector3(v.nrm.into()).normalize_or_zero().into();
        }
    }
    pub fn mirror_x(&self) -> Mesh {
        let mut m = self.clone();
        for v in &mut m.v {
            v.pos[0] = -v.pos[0];
            v.nrm[0] = -v.nrm[0];
        }
        m
    }

    /// A grid of precomputed points (rows x cols) with normals from the grid
    /// itself; `wrap` joins the last column back to the first.
    pub fn grid(&mut self, m: &Mat, rows: &[Vec<SP>], wrap: bool, flip: bool) {
        let nr = rows.len();
        let nc = rows[0].len();
        if nr < 2 || nc < 2 {
            return;
        }
        let base = self.v.len() as u32;
        let p = |i: usize, j: usize| rows[i][j].0;
        for i in 0..nr {
            for j in 0..nc {
                let (a, b) = (i.saturating_sub(1), (i + 1).min(nr - 1));
                let (c, d) = if wrap { ((j + nc - 2) % (nc - 1), (j + 1) % (nc - 1)) } else { (j.saturating_sub(1), (j + 1).min(nc - 1)) };
                let mut n = (p(b, j) - p(a, j)).cross(p(i, d) - p(i, c));
                // Degenerate (a pole or a collapsed row): borrow from the neighbours.
                let mut k = 1;
                while n.length_squared() < 1e-14 && k < 4 {
                    let i2 = if i + k < nr { i + k } else { i.saturating_sub(k) };
                    let (a2, b2) = (i2.saturating_sub(1), (i2 + 1).min(nr - 1));
                    n = (p(b2, j) - p(a2, j)).cross(p(i2, d) - p(i2, c));
                    if n.length_squared() < 1e-14 {
                        n = (p(b, j) - p(a, j)).cross(p(i2, d) - p(i2, c));
                    }
                    k += 1;
                }
                if flip {
                    n = -n;
                }
                let (pos, uv, w) = rows[i][j];
                self.push(pos, n, uv, m, w);
            }
        }
        for i in 0..nr as u32 - 1 {
            for j in 0..nc as u32 - 1 {
                let a = base + i * nc as u32 + j;
                let b = a + nc as u32;
                self.i.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
    }
    /// Parametric patch over [0,1]²: `f(u, v)` gives position, surface coordinates and wear.
    pub fn patch(&mut self, m: &Mat, nu: usize, nv: usize, flip: bool, f: impl Fn(f32, f32) -> SP) {
        let rows: Vec<Vec<SP>> = (0..=nu).map(|i| (0..=nv).map(|j| f(i as f32 / nu as f32, j as f32 / nv as f32)).collect()).collect();
        self.grid(m, &rows, false, flip);
    }
    pub fn quad(&mut self, m: &Mat, p: [Vec3; 4]) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let uvs = [[0.0, 0.0], [(p[1] - p[0]).length(), 0.0], [(p[1] - p[0]).length(), (p[2] - p[1]).length()], [0.0, (p[3] - p[0]).length()]];
        let b = self.v.len() as u32;
        for k in 0..4 {
            self.push(p[k], n, uvs[k], m, 0.0);
        }
        self.i.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    /// Flat fan from a centre point to a closed ring.
    pub fn fan(&mut self, m: &Mat, c: Vec3, ring: &[Vec3], n: Vec3) {
        let b = self.push(c, n, [0.0, 0.0], m, 0.0);
        for (k, p) in ring.iter().enumerate() {
            let d = *p - c;
            self.push(*p, n, [d.x + d.z, d.y], m, 0.0);
            if k > 0 {
                self.i.extend([b, b + k as u32, b + k as u32 + 1]);
            }
        }
        let k = ring.len() as u32;
        self.i.extend([b, b + k, b + 1]);
    }

    /// Box with rounded edges, `h` half extents, `r` edge radius.
    pub fn rbox(&mut self, m: &Mat, c: Vec3, h: Vec3, r: f32, rot: Quat) {
        let start = self.v.len();
        let r = r.min(h.min_element() * 0.999).max(0.0);
        let steps = |hh: f32| -> Vec<f32> {
            let mut v: Vec<f32> = Vec::new();
            if r > 0.0 {
                for i in 0..=3 {
                    v.push(-hh + r * (1.0 - (i as f32 / 3.0 * FRAC_PI_2).cos()));
                }
            } else {
                v.push(-hh);
            }
            let mut w: Vec<f32> = v.iter().rev().map(|x| -x).collect();
            if (w[0] - v[v.len() - 1]).abs() < 1e-5 {
                w.remove(0);
            }
            v.extend(w);
            v
        };
        for a in 0..3 {
            for s in [-1.0f32, 1.0] {
                let (b, cc) = ((a + 1) % 3, (a + 2) % 3);
                let (sb, sc) = (steps(h[b]), steps(h[cc]));
                let mut rows = Vec::new();
                for &x in &sb {
                    let mut row = Vec::new();
                    for &y in &sc {
                        let mut q = Vec3::ZERO;
                        q[a] = s * h[a];
                        q[b] = x;
                        q[cc] = y;
                        let inner = q.clamp(-(h - Vec3::splat(r)), h - Vec3::splat(r));
                        let d = q - inner;
                        let n = if d.length_squared() > 1e-12 { d.normalize() } else { let mut n = Vec3::ZERO; n[a] = s; n };
                        row.push((inner + n * r, [x, y], 0.0, n));
                    }
                    rows.push(row);
                }
                let base = self.v.len() as u32;
                let nc = sc.len() as u32;
                for row in &rows {
                    for (p, uv, w, n) in row {
                        self.push(*p, *n, *uv, m, *w);
                    }
                }
                for i in 0..sb.len() as u32 - 1 {
                    for j in 0..nc - 1 {
                        let a0 = base + i * nc + j;
                        self.i.extend([a0, a0 + nc, a0 + 1, a0 + 1, a0 + nc, a0 + nc + 1]);
                    }
                }
            }
        }
        self.transform(start, Mat4::from_rotation_translation(rot, c));
    }

    /// Surface of revolution. `prof` is (radius, height) going up the outside;
    /// `smooth` shares normals between profile segments.
    pub fn lathe(&mut self, m: &Mat, c: Vec3, axis: Vec3, prof: &[[f32; 2]], segs: usize, smooth: bool) {
        let ax = axis.normalize();
        let u = ax.any_orthonormal_vector();
        let w = ax.cross(u);
        let sn: Vec<Vec2> = prof.windows(2).map(|p| Vec2::new(p[1][1] - p[0][1], -(p[1][0] - p[0][0])).normalize_or_zero()).collect();
        // (radius, height, profile normal, arc length)
        let mut rings: Vec<(f32, f32, Vec2, f32)> = Vec::new();
        let mut arc = 0.0;
        for (k, p) in prof.iter().enumerate() {
            if k > 0 {
                arc += Vec2::new(p[0] - prof[k - 1][0], p[1] - prof[k - 1][1]).length();
            }
            if smooth {
                let n = (sn.get(k.wrapping_sub(1)).copied().unwrap_or(Vec2::ZERO) + sn.get(k).copied().unwrap_or(Vec2::ZERO)).normalize_or_zero();
                rings.push((p[0], p[1], n, arc));
            } else {
                if k > 0 {
                    rings.push((p[0], p[1], sn[k - 1], arc));
                }
                if k + 1 < prof.len() {
                    rings.push((p[0], p[1], sn[k], arc));
                }
            }
        }
        let base = self.v.len() as u32;
        let nc = segs as u32 + 1;
        for &(r, hgt, n, a) in &rings {
            for k in 0..=segs {
                let t = k as f32 / segs as f32 * TAU;
                let d = u * t.cos() + w * t.sin();
                self.push(c + ax * hgt + d * r, d * n.x + ax * n.y, [t * r.max(0.004), a], m, 0.0);
            }
        }
        let pairs: Vec<u32> = if smooth { (0..rings.len() as u32 - 1).collect() } else { (0..rings.len() as u32 / 2).map(|k| k * 2).collect() };
        for i in pairs {
            for j in 0..nc - 1 {
                let a = base + i * nc + j;
                self.i.extend([a, a + nc, a + 1, a + 1, a + nc, a + nc + 1]);
            }
        }
    }

    /// Tube along a polyline with radius `r(t)`, t in [0, 1] along its length.
    pub fn tube(&mut self, m: &Mat, pts: &[Vec3], r: &dyn Fn(f32) -> f32, segs: usize, caps: bool) {
        let n = pts.len();
        let tang: Vec<Vec3> = (0..n).map(|i| (pts[(i + 1).min(n - 1)] - pts[i.saturating_sub(1)]).normalize()).collect();
        let mut len = vec![0.0f32; n];
        for i in 1..n {
            len[i] = len[i - 1] + (pts[i] - pts[i - 1]).length();
        }
        let total = len[n - 1].max(1e-6);
        let mut nrm = tang[0].any_orthonormal_vector();
        let mut rows = Vec::new();
        for i in 0..n {
            if i > 0 {
                nrm = (Quat::from_rotation_arc(tang[i - 1], tang[i]) * nrm).normalize();
            }
            let b = tang[i].cross(nrm);
            let rad = r(len[i] / total);
            let row: Vec<SP> = (0..=segs).map(|k| {
                let a = k as f32 / segs as f32 * TAU;
                (pts[i] + (nrm * a.cos() + b * a.sin()) * rad, [a * rad, len[i]], 0.0)
            }).collect();
            rows.push(row);
        }
        self.grid(m, &rows, true, true);
        if caps {
            for (i, d) in [(0, -1.0f32), (n - 1, 1.0)] {
                let ring: Vec<Vec3> = rows[i][..segs].iter().map(|s| s.0).collect();
                self.fan(m, pts[i], &ring, tang[i] * d);
            }
        }
    }

    /// Flat outline with holes, extruded by `depth` behind the plane of `ax`, `ay`.
    pub fn slab(&mut self, m: &Mat, outline: &[Vec2], holes: &[Vec<Vec2>], o: Vec3, ax: Vec3, ay: Vec3, depth: f32) {
        let az = ax.cross(ay).normalize();
        let mut flat: Vec<f64> = Vec::new();
        let mut hi = Vec::new();
        let rings: Vec<&[Vec2]> = std::iter::once(outline).chain(holes.iter().map(|h| h.as_slice())).collect();
        for (k, ring) in rings.iter().enumerate() {
            if k > 0 {
                hi.push(flat.len() / 2);
            }
            for p in ring.iter() {
                flat.extend([p.x as f64, p.y as f64]);
            }
        }
        let tris = earcutr::earcut(&flat, &hi, 2).unwrap_or_default();
        let pts: Vec<Vec2> = flat.chunks(2).map(|c| Vec2::new(c[0] as f32, c[1] as f32)).collect();
        for (off, n) in [(0.0, az), (-depth, -az)] {
            if depth == 0.0 && off != 0.0 {
                break;
            }
            let b = self.v.len() as u32;
            for p in &pts {
                self.push(o + ax * p.x + ay * p.y + az * off, n, [p.x, p.y], m, 0.0);
            }
            self.i.extend(tris.iter().map(|&t| b + t as u32));
        }
        if depth > 0.0 {
            for ring in rings {
                for k in 0..ring.len() {
                    let (p, q) = (ring[k], ring[(k + 1) % ring.len()]);
                    let (a, b) = (o + ax * p.x + ay * p.y, o + ax * q.x + ay * q.y);
                    self.quad(m, [a, b, b - az * depth, a - az * depth]);
                }
            }
        }
    }

    /// Disc facing `n`, with uv the local frame scaled to [-1, 1].
    pub fn disc(&mut self, m: &Mat, c: Vec3, n: Vec3, up: Vec3, r: f32, segs: usize) {
        let n = n.normalize();
        let y = (up - n * up.dot(n)).normalize();
        let x = y.cross(n);
        let b = self.push(c, n, [0.0, 0.0], m, 0.0);
        for k in 0..=segs {
            let a = k as f32 / segs as f32 * TAU;
            self.push(c + (x * a.cos() + y * a.sin()) * r, n, [a.cos(), a.sin()], m, 0.0);
            if k > 0 {
                self.i.extend([b, b + k as u32, b + k as u32 + 1]);
            }
        }
    }
    /// Low domed head (screw, rivet, knob) of radius `r`, height `h`; uv is a
    /// local frame turned by `spin` so slots do not all line up.
    pub fn dome(&mut self, m: &Mat, c: Vec3, n: Vec3, r: f32, h: f32, segs: usize, spin: f32) {
        let n = n.normalize();
        let x0 = n.any_orthonormal_vector();
        let y0 = n.cross(x0);
        let (x, y) = (x0 * spin.cos() + y0 * spin.sin(), -x0 * spin.sin() + y0 * spin.cos());
        let rings = [1.0f32, 0.92, 0.72, 0.42, 0.0];
        let base = self.v.len() as u32;
        for &f in &rings {
            let z = h * (1.0 - f * f).max(0.0).sqrt();
            for k in 0..=segs {
                let a = k as f32 / segs as f32 * TAU;
                let d = x * a.cos() + y * a.sin();
                let nn = d * (f / (r * r)) + n * (z / (h * h)).max(if f == 0.0 { 1.0 } else { 0.0 });
                self.push(c + d * (r * f) + n * z, nn, [f * a.cos(), f * a.sin()], m, 0.0);
            }
        }
        let nc = segs as u32 + 1;
        for i in 0..rings.len() as u32 - 1 {
            for j in 0..nc - 1 {
                let a = base + i * nc + j;
                self.i.extend([a, a + nc, a + 1, a + 1, a + nc, a + nc + 1]);
            }
        }
    }
    /// A screw head sitting on a surface.
    pub fn screw(&mut self, m: &Mat, c: Vec3, n: Vec3, r: f32, seed: f32) {
        self.dome(m, c, n, r, r * 0.32, 8, hash(seed) * TAU);
    }
    /// Hex bolt head with a washer.
    pub fn bolt(&mut self, m: &Mat, c: Vec3, n: Vec3, r: f32) {
        let n = n.normalize();
        self.lathe(m, c, n, &[[r * 1.5, 0.0], [r * 1.5, r * 0.15], [0.0, r * 0.15]], 12, false);
        self.lathe(m, c + n * r * 0.15, n, &[[r, 0.0], [r, r * 0.65], [r * 0.85, r * 0.75], [0.0, r * 0.75]], 6, false);
    }
}

pub fn hash(x: f32) -> f32 {
    ((x * 127.1 + 311.7).sin() * 43758.547).fract().abs()
}
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------- airfoils

/// NACA four-digit section: camber line height and half thickness at chord fraction x.
pub fn naca(x: f32, t: f32, m: f32) -> (f32, f32) {
    let x = x.clamp(0.0, 1.0);
    let yt = 5.0 * t * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x * x * x - 0.1036 * x * x * x * x);
    let p = 0.4;
    let yc = if x < p { m / (p * p) * (2.0 * p * x - x * x) } else { m / ((1.0 - p) * (1.0 - p)) * (1.0 - 2.0 * p + 2.0 * p * x - x * x) };
    (yc, yt.max(0.0))
}

/// A lifting surface's section at span station s (metres along the span).
#[derive(Clone, Copy)]
pub struct Sect {
    pub le: Vec3,
    pub c: f32,
    pub cdir: Vec3,
    pub up: Vec3,
    /// thickness scale, for rounded tips
    pub tf: f32,
}
pub struct Foil {
    pub at: Box<dyn Fn(f32) -> Sect>,
    pub t: f32,
    pub camber: f32,
}
impl Foil {
    /// Point on the surface; `nose` >= 0 rounds the section off to start at
    /// that chord fraction (a control surface behind its hinge).
    pub fn pt(&self, s: f32, xc: f32, side: f32, nose: f32) -> Vec3 {
        let k = (self.at)(s);
        let (yc, mut yt) = naca(xc, self.t, self.camber);
        if nose >= 0.0 {
            yt *= 0.88 * ((xc - nose).max(0.0) / 0.07).min(1.0).sqrt();
        }
        k.le + k.cdir * (xc * k.c) + k.up * (k.c * (yc + side * yt * k.tf))
    }
    /// The skin between chord fractions x0(s) and x1(s), upper surface then lower.
    pub fn skin(&self, mesh: &mut Mesh, m: &Mat, s0: f32, s1: f32, ns: usize, x0: &dyn Fn(f32) -> f32, x1: &dyn Fn(f32) -> f32, nx: usize, nose: bool) {
        // Normals come from the grid's parameter directions: make sure they point out.
        let k = (self.at)((s0 + s1) * 0.5);
        let ds = (self.at)(s1).le - (self.at)(s0).le;
        let flip = ds.cross(-k.cdir).dot(k.up) < 0.0;
        let mut rows = Vec::new();
        for i in 0..=ns {
            let s = s0 + (s1 - s0) * i as f32 / ns as f32;
            let (a, b) = (x0(s), x1(s));
            let c = (self.at)(s).c;
            let mut row = Vec::new();
            for j in 0..=2 * nx {
                let (side, w) = if j <= nx { (1.0, 1.0 - j as f32 / nx as f32) } else { (-1.0, (j - nx) as f32 / nx as f32) };
                let xc = a + (b - a) * (1.0 - (w * FRAC_PI_2).cos());
                let wear = if nose { 0.2 * (1.0 - smoothstep(0.0, 0.08, xc - a)) } else { 1.0 - smoothstep(0.0, 0.1, xc) };
                row.push((self.pt(s, xc, side, if nose { a } else { -1.0 }), [s, side * xc * c], wear));
            }
            rows.push(row);
        }
        mesh.grid(m, &rows, false, flip);
    }
    /// The flat web closing an open skin at chord fraction x(s).
    pub fn web(&self, mesh: &mut Mesh, m: &Mat, s0: f32, s1: f32, ns: usize, x: &dyn Fn(f32) -> f32) {
        mesh.patch(m, ns, 2, false, |a, b| {
            let s = s0 + (s1 - s0) * a;
            let p = self.pt(s, x(s), 1.0, -1.0).lerp(self.pt(s, x(s), -1.0, -1.0), b);
            (p, [s, b], 0.0)
        });
    }
    /// Flat end cap of the section at s between chord fractions a and b.
    pub fn cap(&self, mesh: &mut Mesh, m: &Mat, s: f32, a: f32, b: f32, nose: bool) {
        let n = 16;
        let mut ring = Vec::new();
        for j in 0..=2 * n {
            let (side, w) = if j <= n { (1.0, 1.0 - j as f32 / n as f32) } else { (-1.0, (j - n) as f32 / n as f32) };
            let xc = a + (b - a) * (1.0 - (w * FRAC_PI_2).cos());
            ring.push(self.pt(s, xc, side, if nose { a } else { -1.0 }));
        }
        ring.pop();
        let k = (self.at)(s);
        let c = self.pt(s, (a + b) * 0.5, 0.0, -1.0);
        let span = k.cdir.cross(k.up).normalize();
        mesh.fan(m, c, &ring, span);
    }
}

// ---------------------------------------------------------------- fuselage

/// Fuselage cross-section keys: station z, half width, top, waist height,
/// bottom, squareness of the upper and the lower half.
const FUSE: [[f32; 7]; 20] = [
    [-3.17, 0.215, 0.175, -0.035, -0.245, 2.0, 2.0],
    [-3.14, 0.300, 0.215, -0.040, -0.330, 2.2, 2.2],
    [-3.08, 0.385, 0.245, -0.050, -0.440, 2.4, 2.4],
    [-2.98, 0.445, 0.268, -0.060, -0.540, 2.6, 2.7],
    [-2.75, 0.505, 0.292, -0.070, -0.610, 2.8, 3.0],
    [-2.40, 0.538, 0.310, -0.075, -0.650, 3.0, 3.2],
    [-2.05, 0.556, 0.322, -0.075, -0.680, 3.2, 3.4],
    [-1.72, 0.570, 0.335, -0.065, -0.705, 3.4, 3.6],
    [-1.35, 0.580, 0.500, -0.050, -0.720, 3.7, 3.6],
    [-0.97, 0.585, 0.655, -0.040, -0.730, 4.2, 3.6],
    [-0.60, 0.585, 0.640, -0.040, -0.730, 4.4, 3.6],
    [0.50, 0.585, 0.640, -0.040, -0.730, 4.4, 3.6],
    [0.85, 0.565, 0.625, -0.045, -0.725, 4.0, 3.5],
    [1.30, 0.520, 0.560, -0.050, -0.715, 3.4, 3.2],
    [1.80, 0.450, 0.480, -0.045, -0.620, 3.0, 2.9],
    [2.40, 0.360, 0.405, -0.020, -0.450, 2.6, 2.6],
    [3.20, 0.245, 0.335, 0.040, -0.220, 2.4, 2.4],
    [4.00, 0.135, 0.300, 0.100, -0.040, 2.2, 2.2],
    [4.45, 0.065, 0.280, 0.150, 0.060, 2.0, 2.0],
    [4.62, 0.020, 0.262, 0.170, 0.120, 2.0, 2.0],
];
pub const NOSE_Z: f32 = -3.17;
pub const TAIL_Z: f32 = 4.62;
pub const FIREWALL_Z: f32 = -2.05;

#[derive(Clone, Copy, Debug)]
pub struct Sec {
    pub w: f32,
    pub top: f32,
    pub mid: f32,
    pub bot: f32,
    pub nt: f32,
    pub nb: f32,
}

/// Monotone cubic through the keys (no overshoot between stations).
fn pchip(col: usize, z: f32) -> f32 {
    let k = &FUSE;
    let n = k.len();
    if z <= k[0][0] {
        return k[0][col];
    }
    if z >= k[n - 1][0] {
        return k[n - 1][col];
    }
    let i = (0..n - 1).find(|&i| z < k[i + 1][0]).unwrap_or(n - 2);
    let h = |i: usize| k[i + 1][0] - k[i][0];
    let d = |i: usize| (k[i + 1][col] - k[i][col]) / h(i);
    let slope = |i: usize| -> f32 {
        if i == 0 || i == n - 1 {
            return if i == 0 { d(0) } else { d(n - 2) };
        }
        let (a, b) = (d(i - 1), d(i));
        if a * b <= 0.0 {
            0.0
        } else {
            let (w1, w2) = (2.0 * h(i) + h(i - 1), h(i) + 2.0 * h(i - 1));
            (w1 + w2) / (w1 / a + w2 / b)
        }
    };
    let t = (z - k[i][0]) / h(i);
    let (y0, y1, m0, m1) = (k[i][col], k[i + 1][col], slope(i) * h(i), slope(i + 1) * h(i));
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * m0 + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * m1
}
pub fn sec(z: f32) -> Sec {
    Sec { w: pchip(1, z), top: pchip(2, z), mid: pchip(3, z), bot: pchip(4, z), nt: pchip(5, z), nb: pchip(6, z) }
}
/// Point on a section at angle th (0 = top, pi/2 = right side, pi = bottom).
pub fn sec_pt(s: &Sec, th: f32) -> Vec2 {
    let (sn, cs) = th.sin_cos();
    let n = if cs > 0.0 { s.nt } else { s.nb };
    let e = |v: f32| v.signum() * v.abs().powf(2.0 / n);
    let h = if cs > 0.0 { s.top - s.mid } else { s.mid - s.bot };
    Vec2::new(s.w * e(sn), s.mid + h * e(cs))
}
pub fn fuse_pt(z: f32, th: f32) -> Vec3 {
    let p = sec_pt(&sec(z), th);
    Vec3::new(p.x, p.y, z)
}
/// Angle on the section at z whose point is at height y on the right side.
pub fn fuse_th(z: f32, y: f32) -> f32 {
    let s = sec(z);
    let (mut a, mut b) = (0.0f32, PI);
    for _ in 0..30 {
        let m = (a + b) * 0.5;
        if sec_pt(&s, m).y > y { a = m } else { b = m }
    }
    (a + b) * 0.5
}
/// Height of the upper skin at station z, lateral position x.
pub fn fuse_top_y(z: f32, x: f32) -> f32 {
    let s = sec(z);
    let x = x.abs().min(s.w * 0.999);
    let (mut a, mut b) = (0.0, FRAC_PI_2);
    for _ in 0..30 {
        let m = (a + b) * 0.5;
        if sec_pt(&s, m).x < x { a = m } else { b = m }
    }
    sec_pt(&s, (a + b) * 0.5).y
}
/// Height of the lower skin at station z, lateral position x.
pub fn fuse_bottom_y(z: f32, x: f32) -> f32 {
    let s = sec(z);
    let x = x.abs().min(s.w * 0.999);
    let (mut a, mut b) = (FRAC_PI_2, PI);
    for _ in 0..30 {
        let m = (a + b) * 0.5;
        if sec_pt(&s, m).x > x { a = m } else { b = m }
    }
    sec_pt(&s, (a + b) * 0.5).y
}
/// Height of a well roof at (z, x): `top`, but never closer than 5 cm to the skin.
fn roof_y(z: f32, x: f32, top: f32) -> f32 {
    top.max(fuse_bottom_y(z, x) + 0.05)
}
/// Walls rising from an opening in the lower skin, outline in (z, x, roof
/// height of the edge that starts there).
fn well_walls(m: &mut Mesh, outline: &[(f32, f32, f32)]) {
    for k in 0..outline.len() {
        let (a, b) = (outline[k], outline[(k + 1) % outline.len()]);
        let n = 12;
        let rows: Vec<Vec<SP>> = (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let (z, x) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                let ys = fuse_bottom_y(z, x) - 0.004;
                let yt = roof_y(z, x, a.2);
                vec![(Vec3::new(x, ys, z), [z, ys], 0.0), (Vec3::new(x, yt, z), [z, yt], 0.0)]
            })
            .collect();
        m.grid(&PRIMER, &rows, false, false);
    }
}
/// A roof over a convex opening, outline in (z, x).
fn well_roof(m: &mut Mesh, outline: &[(f32, f32)], top: f32) {
    let mut ring = Vec::new();
    for k in 0..outline.len() {
        let (a, b) = (outline[k], outline[(k + 1) % outline.len()]);
        for i in 0..8 {
            let t = i as f32 / 8.0;
            let (z, x) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            ring.push(Vec3::new(x, roof_y(z, x, top), z));
        }
    }
    let c = ring.iter().copied().sum::<Vec3>() / ring.len() as f32;
    m.fan(&PRIMER, c, &ring, Vec3::NEG_Y);
}
/// x span of a convex outline in (z, x) at station z.
pub fn span_at(outline: &[[f32; 2]], z: f32) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for k in 0..outline.len() {
        let (a, b) = (outline[k], outline[(k + 1) % outline.len()]);
        if (a[0] - z) * (b[0] - z) <= 0.0 && a[0] != b[0] {
            let x = a[1] + (b[1] - a[1]) * (z - a[0]) / (b[0] - a[0]);
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo, hi)
}
/// A gear door filling a convex opening between stations z0 and z1, flush
/// with the skin, `gap` short of the edges: painted outside, primer inside.
fn gear_door(outline: &[[f32; 2]], z0: f32, z1: f32, gap: f32) -> Mesh {
    let mut m = Mesh::default();
    let at = |a: f32, c: f32, lift: f32| -> SP {
        let z = z0 + (z1 - z0) * a;
        let (lo, hi) = span_at(outline, z);
        let x = lo + gap + (hi - lo - 2.0 * gap) * c;
        (Vec3::new(x, fuse_bottom_y(z, x) + lift, z), [z, x], 0.0)
    };
    m.patch(&WHITE, 16, 8, true, |a, c| at(a, c, -0.002));
    m.patch(&PRIMER, 16, 8, false, |a, c| at(a, c, 0.004));
    // Thin edges all round, so the door has a thickness when it opens.
    let edge = |m: &mut Mesh, f: &dyn Fn(f32) -> (f32, f32)| {
        let rows: Vec<Vec<SP>> = (0..=16).map(|i| { let (a, c) = f(i as f32 / 16.0); vec![at(a, c, -0.002), at(a, c, 0.004)] }).collect();
        m.grid(&PRIMER, &rows, false, false);
    };
    edge(&mut m, &|t| (t, 0.0));
    edge(&mut m, &|t| (t, 1.0));
    edge(&mut m, &|t| (0.0, t));
    edge(&mut m, &|t| (1.0, t));
    m
}
/// Outward normal of the fuselage skin at (z, th).
pub fn fuse_n(z: f32, th: f32) -> Vec3 {
    let e = 1e-3;
    let du = fuse_pt(z + e, th) - fuse_pt(z - e, th);
    let dv = fuse_pt(z, th + e) - fuse_pt(z, th - e);
    dv.cross(du).normalize()
}
/// Point on the skin at station z, height y, on side sx (+1 right), lifted by `off`.
pub fn on_side(z: f32, y: f32, sx: f32, off: f32) -> (Vec3, Vec3) {
    let th = fuse_th(z, y);
    let (mut p, mut n) = (fuse_pt(z, th), fuse_n(z, th));
    p.x *= sx;
    n.x *= sx;
    (p + n * off, n)
}

// ---------------------------------------------------------------- cutouts

/// Exact signed distance to a polygon (negative inside).
pub fn sd_poly(p: Vec2, v: &[Vec2]) -> f32 {
    let mut d = (p - v[0]).length_squared();
    let mut s = 1.0f32;
    let n = v.len();
    let mut j = n - 1;
    for i in 0..n {
        let e = v[j] - v[i];
        let w = p - v[i];
        let b = w - e * (w.dot(e) / e.dot(e)).clamp(0.0, 1.0);
        d = d.min(b.length_squared());
        let c = [p.y >= v[i].y, p.y < v[j].y, e.x * w.y > e.y * w.x];
        if c.iter().all(|&x| x) || c.iter().all(|&x| !x) {
            s = -s;
        }
        j = i;
    }
    s * d.sqrt()
}
fn v2(a: [[f32; 2]; 4]) -> [Vec2; 4] {
    a.map(|p| Vec2::new(p[0], p[1]))
}
/// Signed distance to the nearest window opening (negative inside), body frame.
/// Keep in step with `window()` in plane.wgsl.
pub fn window(p: Vec3) -> f32 {
    let zy = Vec2::new(p.z, p.y);
    let ws = [Vec2::new(-1.72, 0.31), Vec2::new(-1.52, 0.07), Vec2::new(-1.03, 0.64), Vec2::new(-1.0, 0.80), Vec2::new(-1.75, 0.80)];
    let mut d = sd_poly(zy, &ws) + 0.01;
    if p.x.abs() > 0.3 && p.y < 0.6 {
        d = d.min(sd_poly(zy, &v2([[-1.36, 0.12], [-0.60, 0.12], [-0.60, 0.48], [-0.98, 0.48]])) - 0.04);
        d = d.min(sd_poly(zy, &v2([[-0.42, 0.12], [0.62, 0.12], [0.42, 0.48], [-0.42, 0.48]])) - 0.04);
    }
    if p.y > 0.35 {
        d = d.min(sd_poly(Vec2::new(p.z, p.x), &v2([[0.84, -0.30], [1.48, -0.17], [1.48, 0.17], [0.84, 0.30]])) - 0.04);
    }
    d
}
/// Nose gear bay in the belly, (z, x): it runs back under the footwell hump.
pub const NOSE_BAY: [f32; 3] = [-3.0, -1.915, 0.135];
/// Main gear bays, right side, (z, x): a trough the leg folds into, from the
/// slot at its root aft to the bay the wheel lies in.
pub const MAIN_TROUGH: [[f32; 2]; 4] = [[0.215, 0.37], [1.15, 0.183], [1.15, 0.427], [0.215, 0.55]];
pub const MAIN_BAY: [[f32; 2]; 5] = [[1.14, 0.03], [1.545, 0.03], [1.545, 0.43], [1.27, 0.505], [1.14, 0.44]];
/// Roof heights over the trough and the wheel bay, and where they meet.
pub const TROUGH_TOP: f32 = -0.47;
pub const BAY_TOP: f32 = -0.25;
pub const BAY_Z: f32 = 1.14;
/// The trough door starts behind the slot the leg passes through when down.
pub const TROUGH_DOOR_Z: f32 = 0.372;

/// Signed distance to the landing gear wells in the skin (5 mm inside their
/// walls). Keep in step with plane.wgsl.
pub fn well(p: Vec3) -> f32 {
    let mut d = 1.0f32;
    if p.y < -0.40 {
        let [z0, z1, hw] = NOSE_BAY;
        d = d.min(sd_poly(Vec2::new(p.z, p.x), &v2([[z0, -hw], [z1, -hw], [z1, hw], [z0, hw]])) + 0.005);
        let q = Vec2::new(p.z, p.x.abs());
        let bay = MAIN_BAY.map(|v| Vec2::new(v[0], v[1]));
        d = d.min(sd_poly(q, &v2(MAIN_TROUGH)).min(sd_poly(q, &bay)) + 0.005);
    }
    d
}
/// Does this surface pattern have the openings cut out of it?
pub fn cut_by(pattern: f32) -> bool {
    pattern == pat::FUSE || pattern == pat::LINING || pattern == pat::COWL
}
/// Only the skin has the gear wells cut in it (the cabin lining runs down to the floor above them).
pub fn well_cut_by(pattern: f32) -> bool {
    pattern == pat::FUSE || pattern == pat::COWL
}

// ---------------------------------------------------------------- model assembly

/// How a piece moves (see `Plane::pose`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Anim {
    Fixed,
    /// side: -1 left, +1 right
    Aileron(f32),
    Flap(f32),
    Elevator,
    Rudder,
    Prop,
    PropDisc,
    NoseLeg,
    NoseFork,
    NoseWheel,
    LinkUpper,
    LinkLower,
    NoseDoor(f32),
    /// a main gear door; the angle it opens to
    GearDoor(f32),
    MainLeg(f32),
    MainWheel(f32),
    Yoke,
    /// +1 the right pedal of a pair
    Pedal(f32),
    Throttle,
    GearHandle,
    FlapLever,
    TrimWheel,
}

pub struct Piece {
    pub name: &'static str,
    pub mesh: Mesh,
    /// which breakable group it belongs to (see `GROUPS`)
    pub group: usize,
    pub parent: Option<usize>,
    /// a point on the hinge axis, body frame at rest
    pub pivot: Vec3,
    pub axis: Vec3,
    pub anim: Anim,
    /// drawn in the transparent pass
    pub glass: bool,
    /// blocks ambient light in the occlusion bake
    pub occluder: bool,
}

/// The parts the aircraft breaks into: name, mass (kg).
pub const GROUPS: [(&str, f32); 7] = [("fuselage", 420.0), ("tail", 150.0), ("left wing", 170.0), ("right wing", 170.0), ("engine", 260.0), ("propeller", 30.0), ("gear", 60.0)];
pub const G_FUSE: usize = 0;
pub const G_TAIL: usize = 1;
pub const G_LWING: usize = 2;
pub const G_RWING: usize = 3;
pub const G_ENGINE: usize = 4;
pub const G_PROP: usize = 5;
pub const G_GEAR: usize = 6;

// Landing gear geometry (body frame, right side for the mains).
pub const NOSE_PIVOT: Vec3 = Vec3::new(0.0, -0.50, -2.95);
pub const NOSE_AXLE: Vec3 = Vec3::new(0.0, -1.335, -2.87);
pub const NOSE_R: f32 = 0.165;
/// how far the nose leg swings back to retract (rad)
pub const NOSE_FOLD: f32 = 1.487;
pub const MAIN_ROOT: Vec3 = Vec3::new(0.45, -0.60, 0.30);
pub const MAIN_AXLE: Vec3 = Vec3::new(1.25, -1.31, 0.30);
pub const MAIN_R: f32 = 0.19;
/// where a main wheel's centre ends up with the gear retracted
pub const MAIN_STOWED: Vec3 = Vec3::new(0.235, -0.46, 1.338);
pub const PROP_HUB: Vec3 = Vec3::new(0.0, -0.03, -3.30);
/// the pilot's eye
pub const EYE: Vec3 = Vec3::new(-0.27, 0.46, -0.52);

#[derive(Default)]
pub struct Builder {
    pub pieces: Vec<Piece>,
}
impl Builder {
    pub fn add(&mut self, name: &'static str, group: usize, mesh: Mesh, anim: Anim, pivot: Vec3, axis: Vec3, parent: Option<usize>) -> usize {
        self.pieces.push(Piece { name, mesh, group, parent, pivot, axis: axis.normalize_or_zero(), anim, glass: false, occluder: true });
        self.pieces.len() - 1
    }
    pub fn fixed(&mut self, name: &'static str, group: usize, mesh: Mesh) -> usize {
        self.add(name, group, mesh, Anim::Fixed, Vec3::ZERO, Vec3::X, None)
    }
    /// Add a right-side piece and its mirror image on the left. Returns (left, right).
    pub fn pair(&mut self, name: [&'static str; 2], group: [usize; 2], mesh: Mesh, anim: impl Fn(f32) -> Anim, pivot: Vec3, axis: Vec3, parent: [Option<usize>; 2]) -> (usize, usize) {
        let left = mesh.mirror_x();
        // Mirroring flips the sense of rotation: flip the axis back so equal
        // angles move both sides the same way.
        let l = self.add(name[0], group[0], left, anim(-1.0), pivot * Vec3::new(-1.0, 1.0, 1.0), -(axis * Vec3::new(-1.0, 1.0, 1.0)), parent[0]);
        let r = self.add(name[1], group[1], mesh, anim(1.0), pivot, axis, parent[1]);
        (l, r)
    }
}

pub struct Model {
    pub pieces: Vec<Piece>,
    /// every group's pieces merged at rest, for the wreckage
    pub groups: Vec<Mesh>,
}

pub fn build() -> Model {
    let mut b = Builder::default();
    fuselage(&mut b);
    wings(&mut b);
    tail(&mut b);
    engine(&mut b);
    gear(&mut b);
    crate::cabin::build(&mut b);
    bake_ao(&mut b.pieces);
    split_cut(&mut b.pieces);
    let mut groups = vec![Mesh::default(); GROUPS.len()];
    for p in &b.pieces {
        if !p.glass {
            groups[p.group].append(&p.mesh);
        }
    }
    Model { pieces: b.pieces, groups }
}

/// Static pieces that mix surfaces with openings cut in them and plain ones
/// are split, so the plain part can be drawn without discarding fragments
/// (which keeps early depth testing on for the bulk of the interior).
fn split_cut(pieces: &mut Vec<Piece>) {
    let n = pieces.len();
    for k in 0..n {
        let p = &mut pieces[k];
        if p.anim != Anim::Fixed || p.glass {
            continue;
        }
        let m = &p.mesh;
        let cut_tri = |t: &[u32]| cut_by(m.v[t[0] as usize].mat[1]);
        let n_cut = m.i.chunks(3).filter(|t| cut_tri(t)).count();
        if n_cut == 0 || n_cut * 3 == m.i.len() {
            continue;
        }
        let mut parts = [Mesh::default(), Mesh::default()];
        for (side, part) in parts.iter_mut().enumerate() {
            let mut remap = vec![u32::MAX; m.v.len()];
            for t in m.i.chunks(3).filter(|t| cut_tri(t) == (side == 0)) {
                for &i in t {
                    if remap[i as usize] == u32::MAX {
                        remap[i as usize] = part.v.len() as u32;
                        part.v.push(m.v[i as usize]);
                    }
                    part.i.push(remap[i as usize]);
                }
            }
        }
        let [cut, solid] = parts;
        let (name, group, occluder) = (p.name, p.group, p.occluder);
        p.mesh = cut;
        pieces.push(Piece { name, mesh: solid, group, parent: None, pivot: Vec3::ZERO, axis: Vec3::X, anim: Anim::Fixed, glass: false, occluder });
    }
}

// ---------------------------------------------------------------- fuselage

fn fuselage(b: &mut Builder) {
    // Stations: finer around the nose bowl and the tail cap.
    let mut zs = Vec::new();
    let mut z = NOSE_Z;
    while z < TAIL_Z - 0.005 {
        zs.push(z);
        z += if z < -3.0 { 0.012 } else if z > 4.4 { 0.02 } else { 0.035 };
    }
    zs.push(TAIL_Z);
    for split in [FIREWALL_Z, 1.0] {
        let k = zs.iter().position(|&z| z > split).unwrap();
        zs[k - 1] = split;
    }
    const NT: usize = 72; // around one side, top to bottom
    let rows: Vec<Vec<SP>> = zs
        .iter()
        .map(|&z| {
            let s = sec(z);
            let mut arc = 0.0;
            let mut prev = sec_pt(&s, 0.0);
            (0..=NT)
                .map(|k| {
                    let th = k as f32 / NT as f32 * PI;
                    let p = sec_pt(&s, th);
                    arc += (p - prev).length();
                    prev = p;
                    (Vec3::new(p.x, p.y, z), [z, arc], 0.0)
                })
                .collect()
        })
        .collect();
    let mut full = Mesh::default();
    full.grid(&WHITE, &rows, false, false);
    let nc = NT + 1;
    let row_of = |z: f32| zs.iter().position(|&q| (q - z).abs() < 1e-6).unwrap();
    let sub = |r0: usize, r1: usize, pattern: f32| -> Mesh {
        let mut m = Mesh::default();
        m.v = full.v[r0 * nc..(r1 + 1) * nc].to_vec();
        for v in &mut m.v {
            v.mat[1] = pattern;
        }
        for i in r0..r1 {
            for j in 0..nc - 1 {
                let a = ((i - r0) * nc + j) as u32;
                let c = a + nc as u32;
                m.i.extend([a, c, a + 1, a + 1, c, c + 1]);
            }
        }
        // The left half is the mirror image, its surface coordinate negative.
        let mut l = m.mirror_x();
        for v in &mut l.v {
            v.uv[1] = -v.uv[1];
        }
        m.append(&l);
        m
    };
    let (rf, rt) = (row_of(FIREWALL_Z), row_of(1.0));
    let mut cowl = sub(0, rf, pat::COWL);
    // The engine face behind the spinner.
    cowl.disc(&mt(mat::BLACK, 0.03, 0.03, 0.03), Vec3::new(0.0, -0.035, NOSE_Z + 0.03), Vec3::NEG_Z, Vec3::Y, 0.26, 24);
    let mut cabin = sub(rf, rt, pat::FUSE);
    let mut tailcone = sub(rt, zs.len() - 1, pat::FUSE);
    let s = sec(TAIL_Z);
    let ring: Vec<Vec3> = (0..48).map(|k| { let p = sec_pt(&s, k as f32 / 48.0 * TAU); Vec3::new(p.x, p.y, TAIL_Z) }).collect();
    tailcone.fan(&WHITE.pat(pat::FUSE), Vec3::new(0.0, s.mid, TAIL_Z + 0.004), &ring, Vec3::Z);

    // Gear wells: walls and a roof behind each opening in the skin.
    let [z0, z1, hw] = NOSE_BAY;
    const NOSE_TOP: f32 = -0.31;
    let nose = [(z0, -hw), (z1, -hw), (z1, hw), (z0, hw)];
    well_walls(&mut cowl, &nose.map(|(z, x)| (z, x, NOSE_TOP)));
    well_roof(&mut cowl, &nose, NOSE_TOP);
    let mut bays = Mesh::default();
    let (t, w) = (MAIN_TROUGH, MAIN_BAY);
    let (ti, to) = (span_at(&t, BAY_Z).0, span_at(&t, BAY_Z).1);
    // One outline round the trough and the bay, each edge with its roof height.
    well_walls(&mut bays, &[
        (t[0][0], t[0][1], TROUGH_TOP),
        (BAY_Z, ti, BAY_TOP),
        (w[0][0], w[0][1], BAY_TOP),
        (w[1][0], w[1][1], BAY_TOP),
        (w[2][0], w[2][1], BAY_TOP),
        (w[3][0], w[3][1], BAY_TOP),
        (w[4][0], w[4][1], BAY_TOP),
        (BAY_Z, to, TROUGH_TOP),
        (t[3][0], t[3][1], TROUGH_TOP),
    ]);
    well_roof(&mut bays, &[(t[0][0], t[0][1]), (BAY_Z, ti), (BAY_Z, to), (t[3][0], t[3][1])], TROUGH_TOP);
    well_roof(&mut bays, &w.map(|v| (v[0], v[1])), BAY_TOP);
    // The step up from the trough roof to the bay roof.
    let rows: Vec<Vec<SP>> = (0..=8)
        .map(|i| {
            let x = ti + (to - ti) * i as f32 / 8.0;
            let (a, b) = (roof_y(BAY_Z, x, TROUGH_TOP), roof_y(BAY_Z, x, BAY_TOP));
            vec![(Vec3::new(x, a, BAY_Z), [x, a], 0.0), (Vec3::new(x, b, BAY_Z), [x, b], 0.0)]
        })
        .collect();
    bays.grid(&PRIMER, &rows, false, false);
    cabin.append(&bays.mirror_x());
    cabin.append(&bays);

    // Cowling fasteners along the split line and around the nose bowl and firewall.
    for sx in [-1.0f32, 1.0] {
        let mut z = -2.92;
        while z < -2.1 {
            let (p, n) = on_side(z, -0.085, sx, 0.0005);
            cowl.screw(&SCREW_PAINT, p, n, 0.0065, z * 13.0 + sx);
            z += 0.105;
        }
        for k in 0..9 {
            let y = 0.24 - k as f32 * 0.105;
            for zz in [-2.975, FIREWALL_Z - 0.025] {
                let (p, n) = on_side(zz, y, sx, 0.0005);
                cowl.screw(&SCREW_PAINT, p, n, 0.006, y * 7.0 + zz + sx);
            }
        }
    }
    // Oil filler door on top of the cowl, two quarter-turn latches.
    for x in [-0.07f32, 0.07] {
        let z = -2.36;
        let th = (x / 0.55).asin().abs() * 0.7;
        let p = fuse_pt(z, th);
        cowl.screw(&SCREW_STEEL, Vec3::new(x, p.y + 0.0005, z), Vec3::Y, 0.008, x * 3.0);
    }
    // Rows of screws around the cabin door frames and along the wing root fairing.
    let door: [(f32, f32); 5] = [(-1.47, -0.42), (-0.47, -0.42), (-0.47, 0.59), (-0.99, 0.59), (-1.47, 0.03)];
    for sx in [-1.0f32, 1.0] {
        for k in 0..door.len() {
            let (a, c) = (door[k], door[(k + 1) % door.len()]);
            let len = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
            let (nz_, ny) = ((c.1 - a.1) / len, -(c.0 - a.0) / len);
            let n = (len / 0.085).round().max(1.0) as usize;
            for i in 0..n {
                let t = (i as f32 + 0.5) / n as f32;
                let (z, y) = (a.0 + (c.0 - a.0) * t - nz_ * 0.022, a.1 + (c.1 - a.1) * t - ny * 0.022);
                let (p, nn) = on_side(z, y, sx, 0.0004);
                if window(p) > 0.015 && y < 0.57 {
                    cabin.screw(&SCREW_PAINT, p, nn, 0.0045, y * 31.0 + z + sx);
                }
            }
        }
        let mut z = -0.95;
        while z < 0.7 {
            let (p, n) = on_side(z, 0.60, sx, 0.0004);
            cabin.screw(&SCREW_PAINT, p, n, 0.0045, z * 17.0 + sx);
            z += 0.08;
        }
    }
    // Inspection plates on the belly and tail cone.
    for &(z, x, r) in &[(2.10f32, 0.0f32, 0.07f32), (3.05, 0.0, 0.055), (-0.30, 0.0, 0.08)] {
        let s = sec(z);
        let p = Vec3::new(x, s.bot - 0.0012, z);
        inspection_plate(if z > 1.0 { &mut tailcone } else { &mut cabin }, p, Vec3::NEG_Y, Vec3::Z, r, 6);
    }
    for sx in [-1.0f32, 1.0] {
        let (p, n) = on_side(2.6, 0.05, sx, 0.0012);
        inspection_plate(&mut tailcone, p, n, Vec3::Y, 0.05, 6);
    }
    // Static port, left side.
    let (p, n) = on_side(0.95, 0.02, -1.0, 0.0);
    cabin.lathe(&ALU, p, n, &[[0.016, 0.0], [0.016, 0.0015], [0.004, 0.0015], [0.004, -0.001], [0.0, -0.001]], 16, false);
    // Door handles and hinges, the fuel-up step and grab handle.
    for sx in [-1.0f32, 1.0] {
        let (p, n) = on_side(-0.62, 0.20, sx, 0.004);
        let rot = Quat::from_rotation_arc(Vec3::X, n);
        cabin.rbox(&CHROME, p, Vec3::new(0.004, 0.012, 0.055), 0.004, rot);
        cabin.rbox(&BLACK, p - n * 0.003, Vec3::new(0.002, 0.02, 0.07), 0.002, rot);
        for y in [-0.30f32, -0.04] {
            let (p, n) = on_side(-1.49, y, sx, 0.006);
            cabin.rbox(&STEEL, p, Vec3::new(0.006, 0.035, 0.012), 0.004, Quat::from_rotation_arc(Vec3::X, n));
            cabin.bolt(&STEEL, p + n * 0.006, n, 0.0035);
        }
        let (p, n) = on_side(-1.88, -0.36, sx, 0.0);
        cowl.tube(&STEEL, &[p - n * 0.02, p + n * 0.06, p + n * 0.08 + Vec3::Y * 0.012], &|_| 0.011, 10, true);
        let (p, n) = on_side(-1.80, 0.12, sx, 0.0);
        let path: Vec<Vec3> = (0..=10).map(|k| { let a = k as f32 / 10.0 * PI; p + n * (0.035 * a.sin()) + Vec3::Y * (0.06 * a.cos()) }).collect();
        cowl.tube(&STEEL, &path, &|_| 0.008, 8, true);
    }

    // Antennas: two com whips on the cabin top, the transponder blade under
    // the belly, the ELT whip and the GPS puck on the tail cone.
    for &(z, x) in &[(1.68f32, 0.0f32), (2.25, 0.0)] {
        let s = sec(z);
        let p = Vec3::new(x, s.top, z);
        tailcone.rbox(&BLACK, p + Vec3::Y * 0.006, Vec3::new(0.02, 0.006, 0.035), 0.004, Quat::IDENTITY);
        let tip = p + Vec3::new(0.0, 0.55, 0.32);
        tailcone.tube(&BLACK, &[p + Vec3::Y * 0.01, tip], &|t| 0.006 - t * 0.0035, 6, true);
        tailcone.dome(&BLACK, tip, tip - p, 0.004, 0.004, 6, 0.0);
    }
    let s = sec(0.15);
    let p = Vec3::new(0.0, s.bot, 0.15);
    let blade = [Vec2::new(-0.03, 0.0), Vec2::new(0.035, 0.0), Vec2::new(0.02, -0.085), Vec2::new(0.0, -0.09)];
    cabin.slab(&BLACK, &blade, &[], p + Vec3::X * 0.003, Vec3::Z, Vec3::Y, 0.006);
    let s = sec(2.9);
    let p = Vec3::new(0.0, s.top, 2.9);
    tailcone.tube(&BLACK, &[p, p + Vec3::new(0.0, 0.38, 0.12)], &|t| 0.004 - t * 0.0025, 6, true);
    let s = sec(3.35);
    tailcone.lathe(&mt(mat::PLASTIC, 0.62, 0.62, 0.60), Vec3::new(0.0, s.top, 3.35), Vec3::Y, &[[0.045, 0.0], [0.045, 0.006], [0.038, 0.014], [0.02, 0.019], [0.0, 0.02]], 20, true);
    // Tail tie-down ring.
    let s = sec(4.25);
    let p = Vec3::new(0.0, s.bot - 0.005, 4.25);
    let ring: Vec<Vec3> = (0..=16).map(|k| { let a = k as f32 / 16.0 * TAU; p + Vec3::new(0.0, -0.03 - 0.03 * a.cos(), 0.03 * a.sin()) }).collect();
    tailcone.tube(&STEEL, &ring, &|_| 0.005, 6, false);

    b.fixed("cowling", G_ENGINE, cowl);
    b.fixed("cabin skin", G_FUSE, cabin);
    b.fixed("tail cone", G_TAIL, tailcone);
}

/// A round access plate held by screws.
fn inspection_plate(m: &mut Mesh, c: Vec3, n: Vec3, up: Vec3, r: f32, screws: usize) {
    m.disc(&WHITE, c, n, up, r, 28);
    let (x, y) = (up.cross(n).normalize(), up);
    for k in 0..screws {
        let a = k as f32 / screws as f32 * TAU + 0.3;
        m.screw(&SCREW_PAINT, c + (x * a.cos() + y * a.sin()) * (r - 0.011) + n * 0.0003, n, 0.0045, c.x * 9.0 + c.z * 3.0 + k as f32);
    }
}

// ---------------------------------------------------------------- wings

pub const SPAN: f32 = 5.5;
pub const FLAP: [f32; 2] = [0.62, 3.05];
pub const AILERON: [f32; 2] = [3.07, 5.13];

/// Right wing section at span station xs.
pub fn wing_sect(xs: f32) -> Sect {
    let (mut le, mut c) = if xs <= 2.55 { (-0.97, 1.65) } else { let t = (xs - 2.55) / (SPAN - 2.55); let c = 1.65 - 0.50 * t; (0.68 - c, c) };
    let mut tf = 1.0;
    let mut y = 0.70 + 0.0302 * xs;
    if xs > 5.3 {
        // Rounded, slightly drooped tip.
        let u = ((xs - 5.3) / (SPAN - 5.3)).min(1.0);
        let k = 1.0 - (1.0 - u * u).max(0.0).sqrt();
        le += c * 0.3 * k;
        c -= c * 0.3 * k;
        tf = (1.0 - u * u).max(0.0).sqrt();
    }
    if xs > 5.0 {
        y -= 0.05 * ((xs - 5.0) / 0.5).powi(2);
    }
    let inc = (1.5 - 3.0 * xs / SPAN).to_radians();
    let dih = 0.0302f32;
    Sect { le: Vec3::new(xs, y, le), c, cdir: Vec3::new(0.0, -inc.sin(), inc.cos()), up: Vec3::new(-dih, inc.cos(), inc.sin()).normalize(), tf }
}
pub fn wing_foil() -> Foil {
    Foil { at: Box::new(wing_sect), t: 0.12, camber: 0.02 }
}
/// Point and outward normal on a foil.
pub fn foil_frame(f: &Foil, s: f32, xc: f32, side: f32) -> (Vec3, Vec3) {
    let p = f.pt(s, xc, side, -1.0);
    let ds = f.pt(s + 0.002, xc, side, -1.0) - f.pt(s - 0.002, xc, side, -1.0);
    let dx = f.pt(s, xc + 0.002, side, -1.0) - f.pt(s, (xc - 0.002).max(0.0), side, -1.0);
    let n = (ds.cross(dx) * side).normalize();
    (p, n)
}

fn wings(b: &mut Builder) {
    let f = wing_foil();
    let skin = WHITE.pat(pat::WING);
    let ctrl = WHITE.pat(pat::CTRL);
    let mut w = Mesh::default();
    let one = |_: f32| 1.0;
    let zero = |_: f32| 0.0;
    f.skin(&mut w, &skin, 0.0, FLAP[0], 6, &zero, &one, 28, false);
    f.skin(&mut w, &skin, FLAP[0], FLAP[1], 24, &zero, &|_| 0.70, 28, false);
    f.skin(&mut w, &skin, FLAP[1], AILERON[1], 22, &zero, &|_| 0.75, 28, false);
    f.skin(&mut w, &skin, AILERON[1], SPAN, 18, &zero, &one, 28, false);
    f.web(&mut w, &PRIMER, FLAP[0], FLAP[1], 12, &|_| 0.70);
    f.web(&mut w, &PRIMER, FLAP[1], AILERON[1], 12, &|_| 0.75);
    f.cap(&mut w, &PRIMER, FLAP[0], 0.70, 1.0, false);
    f.cap(&mut w, &PRIMER, FLAP[1], 0.70, 0.75, false);
    f.cap(&mut w, &PRIMER, AILERON[1], 0.75, 1.0, false);

    // Fuel filler, sump drain, inspection plates, tie-down ring.
    let (p, n) = foil_frame(&f, 1.55, 0.30, 1.0);
    w.lathe(&ALU, p, n, &[[0.05, -0.002], [0.05, 0.002], [0.042, 0.004], [0.0, 0.004]], 28, false);
    w.lathe(&mt(mat::ALU, 0.5, 0.5, 0.52), p + n * 0.004, n, &[[0.036, 0.0], [0.036, 0.006], [0.03, 0.009], [0.0, 0.009]], 24, false);
    let tab = n.any_orthonormal_vector();
    w.rbox(&mt(mat::PAINT, 0.5, 0.04, 0.03), p + n * 0.016, Vec3::new(0.026, 0.007, 0.003), 0.0025, Quat::from_mat3(&glam::Mat3::from_cols(tab, n, tab.cross(n))));
    for &(s, xc) in &[(0.9f32, 0.40f32), (2.2, 0.42), (3.4, 0.40), (4.5, 0.38)] {
        let (p, n) = foil_frame(&f, s, xc, -1.0);
        inspection_plate(&mut w, p + n * 0.0012, n, Vec3::Z, 0.055, 6);
    }
    let (p, n) = foil_frame(&f, 0.85, 0.42, -1.0);
    w.tube(&STEEL, &[p, p + n * 0.025], &|_| 0.004, 8, true);
    let (p, _) = foil_frame(&f, 2.75, 0.30, -1.0);
    let ring: Vec<Vec3> = (0..=16).map(|k| { let a = k as f32 / 16.0 * TAU; p + Vec3::new(0.0, -0.03 + 0.03 * a.cos(), 0.03 * a.sin()) }).collect();
    w.tube(&STEEL, &ring, &|_| 0.005, 6, false);
    // Rows of screws along the leading-edge skin joint and the tip.
    for k in 0..36 {
        let s = 0.7 + k as f32 * 0.13;
        for side in [-1.0f32, 1.0] {
            let (p, n) = foil_frame(&f, s, 0.13, side);
            w.screw(&SCREW_PAINT, p + n * 0.0003, n, 0.0035, s * 7.0 + side);
        }
    }

    // Nav light and strobe on the tip, the strut and its fittings.
    let mut tip = Mesh::default();
    let (p, _) = foil_frame(&f, 5.47, 0.05, 0.0);
    tip.lathe(&mt(mat::LAMP, 0.9, 0.9, 0.9).x(lamp::STROBE), p + Vec3::new(0.0, 0.0, 0.05), Vec3::X, &[[0.016, 0.0], [0.016, 0.015], [0.012, 0.03], [0.0, 0.034]], 14, true);
    tip.lathe(&mt(mat::LAMP, 0.1, 0.9, 0.2).x(lamp::NAV_GREEN), p + Vec3::new(0.0, 0.0, -0.01), Vec3::new(0.6, 0.0, -0.8), &[[0.02, 0.0], [0.02, 0.012], [0.013, 0.028], [0.0, 0.032]], 16, true);
    w.append(&tip);

    let strut_w = foil_frame(&f, 2.70, 0.24, -1.0).0;
    let strut_f = Vec3::new(0.575, -0.40, -0.18);
    let d = (strut_w - strut_f).normalize();
    let cd = (Vec3::Z - d * d.z).normalize();
    let up = d.cross(cd).normalize();
    let len = (strut_w - strut_f).length();
    let strut = Foil {
        at: Box::new(move |s: f32| Sect { le: strut_f + d * s - cd * 0.04, c: 0.11, cdir: cd, up, tf: 1.0 }),
        t: 0.34,
        camber: 0.0,
    };
    let mut st = Mesh::default();
    strut.skin(&mut st, &WHITE, 0.03, len - 0.03, 24, &zero, &one, 14, false);
    for (p, n) in [(strut_f, -d), (strut_w, d)] {
        let c = p + n * 0.015;
        st.rbox(&WHITE, c - n * 0.02, Vec3::new(0.035, 0.035, 0.035) * Vec3::new(1.0, 1.0, 1.0), 0.015, Quat::from_rotation_arc(Vec3::Y, d));
        st.bolt(&STEEL, c + up * 0.03, up, 0.006);
        st.bolt(&STEEL, c - up * 0.03, -up, 0.006);
    }
    w.append(&st);

    // Landing lights in the left leading edge (built on the right, moved later).
    let mut ll = Mesh::default();
    for s0 in [1.25f32, 1.42] {
        let lens = mt(mat::LAMP, 0.95, 0.92, 0.85).x(lamp::LANDING);
        ll.patch(&lens, 8, 10, false, |a, bb| {
            let s = s0 + (a - 0.5) * 0.13;
            let side = if bb < 0.5 { 1.0 } else { -1.0 };
            let xc = (bb - 0.5).abs() * 2.0 * 0.025;
            let (p, n) = foil_frame(&f, s, xc.max(0.0005), side);
            (p + n * 0.0015, [(a - 0.5) * 2.0, (bb - 0.5) * 2.0], 0.0)
        });
    }
    // The left wing gets the landing lights, a pitot tube, the stall warning
    // vane and a fuel vent; the right one the green light.
    let mut left_extra = ll;
    let (p, n) = foil_frame(&f, 3.5, 0.2, -1.0);
    let bend = p + n * 0.09;
    left_extra.slab(&WHITE, &[Vec2::new(-0.03, 0.0), Vec2::new(0.03, 0.0), Vec2::new(0.012, -0.085), Vec2::new(-0.012, -0.085)], &[], p + Vec3::X * 0.003, Vec3::Z, Vec3::Y, 0.006);
    left_extra.tube(&STEEL, &[bend + Vec3::Z * 0.04, bend - Vec3::Z * 0.17], &|t| if t > 0.9 { 0.006 } else { 0.0075 }, 10, true);
    let (p, n) = foil_frame(&f, 1.72, 0.004, -1.0);
    left_extra.rbox(&ALU, p + n * 0.003, Vec3::new(0.012, 0.002, 0.008), 0.0015, Quat::IDENTITY);
    let (p, n) = foil_frame(&f, 2.4, 0.35, -1.0);
    left_extra.tube(&ALU, &[p, p + n * 0.05, p + n * 0.06 - Vec3::Z * 0.06], &|_| 0.005, 8, true);
    // Mirror the extras: built on the right, they belong to the left wing.
    let left_extra = left_extra.mirror_x();
    let mut lw = w.mirror_x();
    lw.append(&left_extra);
    // Left wing tip light is red.
    for v in &mut lw.v {
        if v.mat[0] == mat::LAMP && v.mat[3] == lamp::NAV_GREEN {
            v.mat[3] = lamp::NAV_RED;
            v.col = [0.95, 0.1, 0.05, v.col[3]];
        }
    }
    b.fixed("left wing", G_LWING, lw);
    b.fixed("right wing", G_RWING, w);
    let ws = wing_structure(&f);
    b.fixed("left wing structure", G_LWING, ws.mirror_x());
    b.fixed("right wing structure", G_RWING, ws);

    // Flaps (Fowler: they ride down and back on a hinge below the wing).
    let mut fl = Mesh::default();
    f.skin(&mut fl, &ctrl, FLAP[0] + 0.01, FLAP[1] - 0.01, 22, &|_| 0.70, &one, 14, true);
    f.cap(&mut fl, &ctrl, FLAP[0] + 0.01, 0.70, 1.0, true);
    f.cap(&mut fl, &ctrl, FLAP[1] - 0.01, 0.70, 1.0, true);
    let sm = (FLAP[0] + FLAP[1]) * 0.5;
    let k = wing_sect(sm);
    let axis = (f.pt(FLAP[1], 0.7, 0.0, -1.0) - f.pt(FLAP[0], 0.7, 0.0, -1.0)).normalize();
    let pivot = f.pt(sm, 0.66, -1.0, -1.0) - k.up * 0.09;
    // Flap tracks hanging under the wing.
    for s in [FLAP[0] + 0.35, FLAP[1] - 0.35] {
        let (p, n) = foil_frame(&f, s, 0.62, -1.0);
        let pts = [p, p + n * 0.05 + k.cdir * 0.04, p + n * 0.07 + k.cdir * 0.12];
        w_track(&mut fl, &pts);
        let _ = n;
    }
    b.pair(["left flap", "right flap"], [G_LWING, G_RWING], fl, Anim::Flap, pivot, axis, [None, None]);

    // Ailerons.
    let mut al = Mesh::default();
    f.skin(&mut al, &ctrl, AILERON[0], AILERON[1], 20, &|_| 0.755, &one, 14, true);
    f.cap(&mut al, &ctrl, AILERON[0], 0.755, 1.0, true);
    f.cap(&mut al, &ctrl, AILERON[1], 0.755, 1.0, true);
    let axis = (f.pt(AILERON[1], 0.78, 0.0, -1.0) - f.pt(AILERON[0], 0.78, 0.0, -1.0)).normalize();
    let pivot = f.pt((AILERON[0] + AILERON[1]) * 0.5, 0.78, 0.0, -1.0);
    // Hinge brackets.
    for s in [AILERON[0] + 0.3, AILERON[1] - 0.4] {
        let (p, n) = foil_frame(&f, s, 0.76, -1.0);
        al.rbox(&ALU, p + n * 0.012, Vec3::new(0.004, 0.012, 0.03), 0.003, Quat::IDENTITY);
    }
    b.pair(["left aileron", "right aileron"], [G_LWING, G_RWING], al, Anim::Aileron, pivot, axis, [None, None]);
}

fn w_track(m: &mut Mesh, pts: &[Vec3]) {
    m.tube(&GREY, pts, &|_| 0.007, 6, true);
}

/// Spars and ribs inside a lifting surface. Never seen while the skin is
/// whole; once a piece is torn off they are what the stump shows.
/// `spars`: chord fractions; `ribs`: (station, from, to chord fraction).
fn structure(f: &Foil, m: &mut Mesh, s0: f32, s1: f32, spars: &[f32], ribs: &[(f32, f32, f32)]) {
    let ns = ((s1 - s0) / 0.15).ceil().max(2.0) as usize;
    let inset = |p: Vec3, mid: Vec3| p + (mid - p).normalize_or_zero() * 0.004;
    for &xc in spars {
        // A vertical web with a cap strip along each edge.
        m.patch(&PRIMER, ns, 1, false, |a, bb| {
            let s = s0 + (s1 - s0) * a;
            let (t, b) = (f.pt(s, xc, 1.0, -1.0), f.pt(s, xc, -1.0, -1.0));
            let mid = (t + b) * 0.5;
            (inset(t, mid).lerp(inset(b, mid), bb), [s, bb], 0.0)
        });
        for side in [1.0f32, -1.0] {
            let pts: Vec<Vec3> = (0..=ns).map(|k| { let s = s0 + (s1 - s0) * k as f32 / ns as f32; let p = f.pt(s, xc, side, -1.0); inset(p, f.pt(s, xc, -side, -1.0)) }).collect();
            m.tube(&ALU, &pts, &|_| 0.011, 6, true);
        }
    }
    for &(s, a, bb) in ribs {
        let n = 14;
        let mut ring = Vec::new();
        for j in 0..=2 * n {
            let (side, w) = if j <= n { (1.0, 1.0 - j as f32 / n as f32) } else { (-1.0, (j - n) as f32 / n as f32) };
            let xc = a + (bb - a) * (1.0 - (w * FRAC_PI_2).cos());
            let p = f.pt(s, xc, side, -1.0);
            ring.push(inset(p, f.pt(s, xc, -side, -1.0)));
        }
        ring.pop();
        let k = (f.at)(s);
        let c = f.pt(s, (a + bb) * 0.5, 0.0, -1.0);
        m.fan(&PRIMER, c, &ring, k.cdir.cross(k.up).normalize());
    }
}

fn wing_structure(f: &Foil) -> Mesh {
    let mut m = Mesh::default();
    let mut ribs = Vec::new();
    let mut s = 0.62;
    while s < 5.3 {
        // Ribs stop at the control surface webs; the tip bay is closed.
        let to = if s < FLAP[1] { 0.68 } else if s < AILERON[1] { 0.73 } else { 0.97 };
        ribs.push((s, 0.03, to));
        s += 0.305;
    }
    structure(f, &mut m, 0.45, SPAN - 0.1, &[0.25, 0.64], &ribs);
    m
}

// ---------------------------------------------------------------- tail

fn tail(b: &mut Builder) {
    let stab = Foil {
        at: Box::new(|s: f32| {
            let a = s.abs();
            let mut le = 3.50 + a / 1.70 * 0.38;
            let te = 4.58 - a / 1.70 * 0.10;
            let mut c = te - le;
            let mut tf = 1.0;
            if a > 1.58 {
                let u = ((a - 1.58) / 0.12).min(1.0);
                let k = 1.0 - (1.0 - u * u).max(0.0).sqrt();
                le += c * 0.25 * k;
                c -= c * 0.25 * k;
                tf = (1.0 - u * u).max(0.0).sqrt();
            }
            Sect { le: Vec3::new(s, 0.11, le), c, cdir: Vec3::Z, up: Vec3::Y, tf }
        }),
        t: 0.10,
        camber: 0.0,
    };
    let hinge = |s: f32| {
        let k = (stab.at)(s);
        (4.16 - k.le.z) / k.c
    };
    let mut m = Mesh::default();
    stab.skin(&mut m, &WHITE.pat(pat::TAIL), -1.70, 1.70, 60, &|_| 0.0, &|s| hinge(s) - 0.004, 24, false);
    stab.web(&mut m, &PRIMER, -1.70, 1.70, 30, &|s| hinge(s) - 0.004);
    for s in [-1.0f32, 1.0] {
        let (p, n) = (stab.pt(s * 0.9, 0.3, -1.0, -1.0), Vec3::NEG_Y);
        inspection_plate(&mut m, p - Vec3::Y * 0.0012, n, Vec3::Z, 0.04, 4);
    }
    let mut el = Mesh::default();
    let ctrl = WHITE.pat(pat::CTRL);
    stab.skin(&mut el, &ctrl, -1.68, 1.68, 60, &|s| hinge(s) + 0.004, &|_| 1.0, 12, true);
    for s in [-1.68f32, 1.68] {
        stab.cap(&mut el, &ctrl, s, hinge(s) + 0.004, 1.0, true);
    }

    let fin = Foil {
        at: Box::new(|s: f32| {
            let u = s / 1.53;
            let mut le = 3.98 - 0.98 * (1.0 - u).max(0.0).powf(1.6);
            let te = 4.66 - 0.04 * u;
            let mut c = te - le;
            let mut tf = 1.0;
            if s > 1.43 {
                let v = ((s - 1.43) / 0.10).min(1.0);
                let k = 1.0 - (1.0 - v * v).max(0.0).sqrt();
                le += c * 0.15 * k;
                c -= c * 0.15 * k;
                tf = (1.0 - v * v).max(0.05).sqrt();
            }
            Sect { le: Vec3::new(0.0, 0.27 + s, le), c, cdir: Vec3::Z, up: Vec3::X, tf }
        }),
        t: 0.09,
        camber: 0.0,
    };
    let fh = |s: f32| {
        let k = (fin.at)(s);
        (4.30 + 0.12 * s / 1.53 - k.le.z) / k.c
    };
    fin.skin(&mut m, &WHITE.pat(pat::TAIL), 0.0, 1.53, 40, &|_| 0.0, &|s| fh(s) - 0.004, 24, false);
    fin.web(&mut m, &PRIMER, 0.0, 1.53, 20, &|s| fh(s) - 0.004);
    // Beacon on the fin tip, VOR whiskers, the white tail light.
    let top = fin.pt(1.50, 0.35, 0.0, -1.0);
    m.lathe(&BLACK, top + Vec3::Y * 0.01, Vec3::Y, &[[0.022, 0.0], [0.022, 0.012], [0.0, 0.012]], 16, false);
    m.lathe(&mt(mat::LAMP, 0.9, 0.06, 0.03).x(lamp::BEACON), top + Vec3::Y * 0.022, Vec3::Y, &[[0.018, 0.0], [0.018, 0.012], [0.013, 0.028], [0.0, 0.032]], 16, true);
    for sx in [-1.0f32, 1.0] {
        let p = fin.pt(1.2, 0.08, 0.0, -1.0);
        m.tube(&BLACK, &[p, p + Vec3::new(sx * 0.38, 0.08, -0.2)], &|t| 0.005 - 0.002 * t, 6, true);
    }
    m.lathe(&mt(mat::LAMP, 0.95, 0.95, 0.95).x(lamp::TAIL), Vec3::new(0.0, 0.19, TAIL_Z + 0.004), Vec3::Z, &[[0.012, 0.0], [0.012, 0.01], [0.0, 0.016]], 12, true);

    let mut ru = Mesh::default();
    fin.skin(&mut ru, &ctrl, -0.05, 1.52, 40, &|s| fh(s) + 0.004, &|_| 1.0, 12, true);
    fin.cap(&mut ru, &ctrl, -0.05, fh(-0.05) + 0.004, 1.0, true);
    // Rudder and elevator control horns with their cable bolts.
    let hp = Vec3::new(0.0, 0.18, 4.36);
    ru.rbox(&GREY, hp, Vec3::new(0.08, 0.012, 0.025), 0.004, Quat::IDENTITY);
    for sx in [-1.0f32, 1.0] {
        ru.bolt(&STEEL, hp + Vec3::new(sx * 0.075, 0.012, 0.0), Vec3::Y, 0.004);
    }
    let rax = Vec3::new(0.0, 1.53, 0.12).normalize();
    // Spars and ribs for the stump when either surface is broken off.
    let mut ts = Mesh::default();
    structure(&stab, &mut ts, -1.6, 1.6, &[0.22, 0.52], &[(-1.3, 0.03, 0.56), (-0.9, 0.03, 0.56), (-0.5, 0.03, 0.56), (0.5, 0.03, 0.56), (0.9, 0.03, 0.56), (1.3, 0.03, 0.56)]);
    structure(&fin, &mut ts, 0.05, 1.45, &[0.3, 0.6], &[(0.4, 0.03, 0.72), (0.8, 0.03, 0.72), (1.2, 0.03, 0.72)]);
    b.fixed("tail structure", G_TAIL, ts);
    b.fixed("stabiliser", G_TAIL, m);
    b.add("elevator", G_TAIL, el, Anim::Elevator, Vec3::new(0.0, 0.11, 4.16), Vec3::X, None);
    b.add("rudder", G_TAIL, ru, Anim::Rudder, Vec3::new(0.0, 0.27, 4.30), rax, None);
}

// ---------------------------------------------------------------- engine, propeller

fn engine(b: &mut Builder) {
    let mut m = Mesh::default();
    // Exhaust stack out of the lower right cowl, and the cowl flaps.
    let pts = [Vec3::new(0.18, -0.45, -2.35), Vec3::new(0.28, -0.60, -2.26), Vec3::new(0.32, -0.70, -2.17), Vec3::new(0.335, -0.75, -2.10)];
    m.tube(&mt(mat::EXHAUST, 0.3, 0.28, 0.26), &pts, &|t| 0.03 + 0.006 * smoothstep(0.85, 1.0, t), 16, false);
    m.tube(&mt(mat::BLACK, 0.01, 0.01, 0.01), &[pts[3] - (pts[3] - pts[2]).normalize() * 0.05, pts[3] + (pts[3] - pts[2]).normalize() * 0.001], &|_| 0.029, 16, true);
    for sx in [-1.0f32, 1.0] {
        let p = Vec3::new(0.17 * sx, sec(-2.12).bot + 0.015, -2.12);
        m.rbox(&WHITE, p, Vec3::new(0.13, 0.004, 0.055), 0.003, Quat::from_rotation_x(0.18));
    }
    b.fixed("exhaust", G_ENGINE, m);

    // Spinner and blades.
    let mut sp = Mesh::default();
    let prof: Vec<[f32; 2]> = (0..=16).map(|k| { let h = k as f32 / 16.0 * 0.42; [0.205 * (1.0 - (h / 0.42).powf(2.2)).max(0.0).sqrt(), h] }).collect();
    sp.lathe(&WHITE, Vec3::new(0.0, -0.035, NOSE_Z - 0.01), Vec3::NEG_Z, &prof, 40, true);
    sp.lathe(&GREY, Vec3::new(0.0, -0.035, NOSE_Z - 0.01), Vec3::NEG_Z, &[[0.21, -0.012], [0.21, 0.0], [0.0, 0.0]], 40, false);
    for k in 0..6 {
        let a = k as f32 / 6.0 * TAU + 0.5;
        let p = Vec3::new(0.0, -0.035, NOSE_Z - 0.03) + Vec3::new(a.cos(), a.sin(), 0.0) * 0.196;
        sp.screw(&SCREW_STEEL, p, Vec3::new(a.cos(), a.sin(), -0.15), 0.005, k as f32);
    }
    let blade_m = mt(mat::PROP, 0.5, 0.5, 0.5);
    let mut blades = Mesh::default();
    for k in 0..2 {
        let mut bl = Mesh::default();
        let blade = Foil {
            at: Box::new(|r: f32| {
                let beta = (1.55 / (TAU * r.max(0.1))).atan() + 0.035;
                let shank = smoothstep(0.24, 0.12, r);
                let mut c = 0.155 - 0.11 * ((r - 0.55) / 0.42).powi(2);
                c = c * (1.0 - shank) + 0.055 * shank;
                let mut tf = (0.18 - 0.11 * (r - 0.22) / 0.75) / 0.10;
                tf = tf * (1.0 - shank) + 9.0 * shank;
                if r > 0.93 {
                    let u = ((r - 0.93) / 0.035).min(1.0);
                    let q = (1.0 - u * u).max(0.0).sqrt();
                    c *= 0.4 + 0.6 * q;
                    tf *= q.max(0.05);
                }
                let t = Vec3::X;
                let cdir = -t * beta.cos() + Vec3::Z * beta.sin();
                let up = Vec3::NEG_Z * beta.cos() - t * beta.sin();
                Sect { le: PROP_HUB + Vec3::Y * r - cdir * (0.35 * c) - up * (0.03 * c), c, cdir, up, tf }
            }),
            t: 0.10,
            camber: 0.035,
        };
        blade.skin(&mut bl, &blade_m, 0.10, 0.965, 36, &|_| 0.0, &|_| 1.0, 14, false);
        bl.transform(0, Mat4::from_translation(PROP_HUB) * Mat4::from_rotation_z(k as f32 * PI) * Mat4::from_translation(-PROP_HUB));
        blades.append(&bl);
    }
    b.add("spinner", G_PROP, sp, Anim::Prop, PROP_HUB, Vec3::Z, None);
    let prop = b.add("blades", G_PROP, blades, Anim::Prop, PROP_HUB, Vec3::Z, None);
    let mut disc = Mesh::default();
    disc.disc(&mt(mat::PROPDISC, 0.1, 0.1, 0.1), PROP_HUB + Vec3::Z * 0.005, Vec3::Z, Vec3::Y, 0.975, 64);
    let d = b.add("prop disc", G_PROP, disc, Anim::PropDisc, PROP_HUB, Vec3::Z, None);
    b.pieces[d].glass = true;
    b.pieces[d].occluder = false;
    // Spinning blades are a blur, not something that shades the cowl.
    b.pieces[prop].occluder = false;
}

// ---------------------------------------------------------------- landing gear

/// Tyre and wheel around axle `axis` through `c`.
fn wheel(m: &mut Mesh, c: Vec3, axis: Vec3, r: f32, half_w: f32, rim: f32, brake: bool) {
    let tyre: Vec<[f32; 2]> = [(0.45, -0.80), (0.58, -0.97), (0.74, -1.0), (0.87, -0.92), (0.96, -0.72), (1.0, -0.40), (1.0, 0.40), (0.96, 0.72), (0.87, 0.92), (0.74, 1.0), (0.58, 0.97), (0.45, 0.80)]
        .iter()
        .map(|&(a, b)| [rim + (r - rim) * ((a - 0.45) / 0.55), b * half_w])
        .collect();
    m.lathe(&RUBBER.pat(pat::TREAD), c, axis, &tyre, 48, true);
    let hub = mt(mat::SATIN, 0.55, 0.56, 0.57);
    for s in [-1.0f32, 1.0] {
        let p: Vec<[f32; 2]> = [(rim + 0.008, 0.80), (rim + 0.008, 0.88), (rim - 0.004, 0.86), (rim * 0.8, 0.75), (rim * 0.62, 0.78), (rim * 0.40, 0.88), (rim * 0.22, 0.92), (0.0, 0.92)]
            .iter()
            .map(|&(a, b)| [a, b * half_w * s])
            .collect();
        let mut p = p;
        if s < 0.0 {
            p.reverse();
        }
        m.lathe(&hub, c, axis, &p, 32, false);
        for k in 0..6 {
            let a = k as f32 / 6.0 * TAU;
            let u = axis.any_orthonormal_vector();
            let w = axis.cross(u);
            let q = c + (u * a.cos() + w * a.sin()) * rim * 0.52 + axis * (half_w * 0.83 * s);
            m.bolt(&STEEL, q, axis * s, 0.0045);
        }
    }
    let u = axis.any_orthonormal_vector();
    let q = c + u * rim * 0.72 + axis * half_w * 0.8;
    m.tube(&STEEL, &[q, q + axis * 0.02], &|_| 0.003, 6, true);
    if brake {
        m.lathe(&mt(mat::STEEL, 0.4, 0.36, 0.33).x(2.0), c - axis * (half_w + 0.012), axis, &[[rim * 0.75, 0.0], [rim * 1.18, 0.0], [rim * 1.18, 0.006], [rim * 0.75, 0.006], [rim * 0.75, 0.0]], 40, false);
    }
}

fn gear(b: &mut Builder) {
    // Main legs: tapered spring-steel tubes swinging back into the belly.
    let mut leg = Mesh::default();
    let end = Vec3::new(1.152, -1.29, 0.30);
    leg.tube(&PAINT_WHITE, &[MAIN_ROOT, MAIN_ROOT.lerp(end, 0.5) + Vec3::new(0.0, 0.01, 0.0), end], &|t| 0.036 - 0.01 * t, 14, true);
    leg.rbox(&PAINT_WHITE, end + Vec3::new(-0.005, 0.0, 0.0), Vec3::new(0.012, 0.045, 0.035), 0.01, Quat::IDENTITY);
    leg.lathe(&GREY, MAIN_ROOT, (end - MAIN_ROOT).normalize(), &[[0.05, -0.02], [0.05, 0.05], [0.04, 0.07]], 16, false);
    // Brake caliper and its line.
    let cal = MAIN_AXLE + Vec3::new(-0.087, 0.075, 0.045);
    leg.rbox(&mt(mat::SATIN, 0.08, 0.08, 0.09), cal, Vec3::new(0.014, 0.03, 0.022), 0.006, Quat::from_rotation_x(-0.5));
    leg.bolt(&STEEL, cal + Vec3::new(-0.014, 0.012, 0.0), Vec3::NEG_X, 0.004);
    leg.bolt(&STEEL, cal + Vec3::new(-0.014, -0.012, 0.0), Vec3::NEG_X, 0.004);
    let line: Vec<Vec3> = (0..=8).map(|k| { let t = k as f32 / 8.0; MAIN_ROOT.lerp(end, 1.0 - t) + Vec3::new(0.0, 0.0, 0.04) + if k == 0 { Vec3::new(-0.02, 0.04, 0.0) } else { Vec3::ZERO } }).collect();
    leg.tube(&RUBBER, &line, &|_| 0.0045, 6, false);
    let mut wh = Mesh::default();
    wheel(&mut wh, MAIN_AXLE, Vec3::X, MAIN_R, 0.075, 0.08, true);
    let (ll, lr) = b.pair(["left main leg", "right main leg"], [G_GEAR, G_GEAR], leg, Anim::MainLeg, MAIN_ROOT, Vec3::Z, [None, None]);
    b.pair(["left main wheel", "right main wheel"], [G_GEAR, G_GEAR], wh, Anim::MainWheel, MAIN_AXLE, Vec3::X, [Some(ll), Some(lr)]);
    // Main gear doors, closed with the gear up and down: one over the trough,
    // hinged on its inboard edge, and one over the wheel bay, hinged near the keel.
    let gap = 0.004;
    let (z0, z1) = (TROUGH_DOOR_Z, BAY_Z - 0.003);
    let hinge = |z: f32| { let x = span_at(&MAIN_TROUGH, z).0 + gap; Vec3::new(x, fuse_bottom_y(z, x), z) };
    let (h0, h1) = (hinge(z0), hinge(z1));
    b.pair(["left trough door", "right trough door"], [G_GEAR, G_GEAR], gear_door(&MAIN_TROUGH, z0, z1, gap), |_| Anim::GearDoor(-1.75), h0, h1 - h0, [None, None]);
    let (z0, z1) = (BAY_Z + 0.003, MAIN_BAY[1][0] - gap);
    let x = MAIN_BAY[0][1] + gap;
    let (h0, h1) = (Vec3::new(x, fuse_bottom_y(z0, x), z0), Vec3::new(x, fuse_bottom_y(z1, x), z1));
    b.pair(["left bay door", "right bay door"], [G_GEAR, G_GEAR], gear_door(&MAIN_BAY, z0, z1, gap), |_| Anim::GearDoor(-1.50), h0, h1 - h0, [None, None]);

    // Nose leg: oleo strut that steers, with torque links and a shimmy damper.
    let (px, top) = (NOSE_PIVOT, NOSE_PIVOT + Vec3::Y * 0.04);
    let mut nl = Mesh::default();
    nl.tube(&PAINT_WHITE, &[top, Vec3::new(0.0, -0.97, px.z)], &|t| 0.044 - 0.004 * t, 18, true);
    nl.tube(&STEEL, &[px - Vec3::X * 0.1, px + Vec3::X * 0.1], &|_| 0.02, 12, true);
    nl.rbox(&PAINT_WHITE, px + Vec3::new(0.0, -0.02, 0.0), Vec3::new(0.06, 0.04, 0.032), 0.012, Quat::IDENTITY);
    nl.lathe(&GREY, Vec3::new(0.0, -0.66, px.z), Vec3::Y, &[[0.058, 0.0], [0.058, 0.035], [0.045, 0.045]], 20, false);
    // Shimmy damper alongside the strut, so it folds into the bay with it.
    let dmp = [Vec3::new(0.064, -0.62, px.z + 0.02), Vec3::new(0.064, -0.79, px.z + 0.02)];
    nl.tube(&mt(mat::SATIN, 0.1, 0.1, 0.11), &dmp, &|_| 0.016, 10, true);
    nl.tube(&CHROME, &[dmp[1], dmp[1] - Vec3::Y * 0.07], &|_| 0.006, 8, true);
    for y in [-0.64f32, -0.86] {
        nl.tube(&GREY, &[Vec3::new(0.04, y, px.z + 0.02), Vec3::new(0.064, y, px.z + 0.02)], &|_| 0.008, 8, true);
    }
    nl.rbox(&GREY, Vec3::new(0.0, -0.94, px.z + 0.044), Vec3::new(0.03, 0.012, 0.012), 0.004, Quat::IDENTITY);
    let leg = b.add("nose leg", G_GEAR, nl, Anim::NoseLeg, NOSE_PIVOT, Vec3::X, None);

    let mut fk = Mesh::default();
    fk.tube(&CHROME, &[Vec3::new(0.0, -0.72, px.z), Vec3::new(0.0, -1.09, px.z)], &|_| 0.031, 16, true);
    let crown = Vec3::new(0.0, -1.105, px.z + 0.01);
    fk.rbox(&PAINT_WHITE, crown, Vec3::new(0.105, 0.022, 0.05), 0.012, Quat::IDENTITY);
    for sx in [-1.0f32, 1.0] {
        let a = crown + Vec3::new(sx * 0.088, 0.0, 0.0);
        let c = NOSE_AXLE + Vec3::new(sx * 0.088, 0.0, 0.0);
        let mid = a.lerp(c, 0.5);
        let dir = (c - a).normalize();
        fk.rbox(&PAINT_WHITE, mid, Vec3::new(0.008, (c - a).length() * 0.5 + 0.02, 0.025), 0.006, Quat::from_rotation_arc(Vec3::Y, -dir));
        fk.bolt(&STEEL, c + Vec3::X * sx * 0.008, Vec3::X * sx, 0.007);
    }
    fk.rbox(&GREY, Vec3::new(0.0, -1.10, px.z + 0.044), Vec3::new(0.03, 0.012, 0.012), 0.004, Quat::IDENTITY);
    let fork = b.add("nose fork", G_GEAR, fk, Anim::NoseFork, NOSE_PIVOT, Vec3::Y, Some(leg));
    let mut nw = Mesh::default();
    wheel(&mut nw, NOSE_AXLE, Vec3::X, NOSE_R, 0.062, 0.065, false);
    b.add("nose wheel", G_GEAR, nw, Anim::NoseWheel, NOSE_AXLE, Vec3::X, Some(fork));
    // Torque links: two arms meeting at an apex behind the strut (facing up
    // into the bay once the leg has folded back).
    let (u, l) = (Vec3::new(0.0, -0.94, px.z + 0.05), Vec3::new(0.0, -1.10, px.z + 0.05));
    let apex = (u + l) * 0.5 + Vec3::Z * (0.095f32 * 0.095 - 0.08 * 0.08).sqrt();
    for (a, c, anim, parent) in [(u, apex, Anim::LinkUpper, leg), (l, apex, Anim::LinkLower, fork)] {
        let mut lk = Mesh::default();
        for sx in [-1.0f32, 1.0] {
            let off = Vec3::X * sx * 0.018;
            lk.rbox(&GREY, (a + c) * 0.5 + off, Vec3::new(0.004, 0.012, (c - a).length() * 0.5 + 0.008), 0.004, Quat::from_rotation_arc(Vec3::Z, (c - a).normalize()));
        }
        lk.tube(&STEEL, &[a - Vec3::X * 0.026, a + Vec3::X * 0.026], &|_| 0.006, 8, true);
        lk.tube(&STEEL, &[c - Vec3::X * 0.026, c + Vec3::X * 0.026], &|_| 0.006, 8, true);
        b.add(if anim == Anim::LinkUpper { "upper torque link" } else { "lower torque link" }, G_GEAR, lk, anim, a, Vec3::X, Some(parent));
    }

    // Nose gear doors, following the cowl bottom.
    let bottom_y = fuse_bottom_y;
    let mut dr = Mesh::default();
    let (z0, z1) = (NOSE_BAY[0] + 0.01, NOSE_BAY[1] - 0.01);
    dr.patch(&WHITE, 16, 6, true, |a, c| {
        let z = z0 + (z1 - z0) * a;
        let x = 0.002 + 0.128 * c;
        (Vec3::new(x, bottom_y(z, x) - 0.002, z), [z, x], 0.0)
    });
    dr.patch(&PRIMER, 16, 6, false, |a, c| {
        let z = z0 + (z1 - z0) * a;
        let x = 0.002 + 0.128 * c;
        (Vec3::new(x, bottom_y(z, x) + 0.004, z), [z, x], 0.0)
    });
    let h0 = Vec3::new(0.13, bottom_y(z0, 0.13), z0);
    let h1 = Vec3::new(0.13, bottom_y(z1, 0.13), z1);
    b.pair(["left nose door", "right nose door"], [G_ENGINE, G_ENGINE], dr, Anim::NoseDoor, h0, h1 - h0, [None, None]);
}

// ---------------------------------------------------------------- ambient occlusion

/// Bakes ambient occlusion into every vertex: the airframe (less its glass) is
/// voxelised and a fan of rays is marched from each vertex. Light reaching the
/// cabin only through its windows falls out of this naturally.
fn bake_ao(pieces: &mut [Piece]) {
    const VOX: f32 = 0.025;
    let lo = Vec3::new(-5.8, -1.7, -3.8);
    let dim = [(11.6 / VOX) as usize, (3.8 / VOX) as usize, (8.8 / VOX) as usize];
    let mut grid = vec![0u64; (dim[0] * dim[1] * dim[2]) / 64 + 1];
    let cell = |p: Vec3| -> Option<usize> {
        let q = (p - lo) / VOX;
        if q.min_element() < 0.0 {
            return None;
        }
        let (x, y, z) = (q.x as usize, q.y as usize, q.z as usize);
        if x >= dim[0] || y >= dim[1] || z >= dim[2] {
            return None;
        }
        Some((z * dim[1] + y) * dim[0] + x)
    };
    for p in pieces.iter().filter(|p| p.occluder && !p.glass && !p.name.ends_with("structure")) {
        let m = &p.mesh;
        for t in m.i.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.v[i as usize].pos));
            let cut = cut_by(m.v[t[0] as usize].mat[1]);
            let n = (((b - a).length().max((c - a).length()).max((c - b).length())) / (VOX * 0.5)).ceil().max(1.0) as usize;
            for i in 0..=n {
                for j in 0..=n - i {
                    let q = a + (b - a) * (i as f32 / n as f32) + (c - a) * (j as f32 / n as f32);
                    if cut && (window(q) < 0.0 || (well_cut_by(m.v[t[0] as usize].mat[1]) && well(q) < 0.0)) {
                        continue;
                    }
                    if let Some(k) = cell(q) {
                        grid[k / 64] |= 1 << (k % 64);
                    }
                }
            }
        }
    }
    // Cosine-weighted directions about +Z.
    const RAYS: usize = 28;
    let dirs: Vec<Vec3> = (0..RAYS)
        .map(|k| {
            let u = (k as f32 + 0.5) / RAYS as f32;
            let r = u.sqrt();
            let a = k as f32 * 2.399963;
            Vec3::new(r * a.cos(), r * a.sin(), (1.0 - u).sqrt())
        })
        .collect();
    let occluded = |o: Vec3, d: Vec3| -> bool {
        let mut t = VOX * 1.2;
        while t < 2.6 {
            match cell(o + d * t) {
                Some(k) if grid[k / 64] & (1 << (k % 64)) != 0 => return true,
                None => return false,
                _ => {}
            }
            t += VOX * 0.7 + t * 0.03;
        }
        false
    };
    let ao_of = |v: &PartVertex| -> f32 {
        let n = Vec3::from(v.nrm).normalize_or_zero();
        if n == Vec3::ZERO {
            return 1.0;
        }
        let q = Quat::from_rotation_arc(Vec3::Z, n);
        let o = Vec3::from(v.pos) + n * (VOX * 1.1);
        let hit = dirs.iter().filter(|d| occluded(o, q * **d)).count();
        1.0 - hit as f32 / RAYS as f32
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    for p in pieces.iter_mut() {
        if p.glass {
            continue;
        }
        if p.name.ends_with("structure") {
            // Boxed in by the skin: no light reaches it until that is torn open.
            for v in &mut p.mesh.v {
                v.col[3] = 0.6;
            }
            continue;
        }
        let verts = &mut p.mesh.v;
        let chunk = verts.len().div_ceil(threads).max(64);
        std::thread::scope(|s| {
            for part in verts.chunks_mut(chunk) {
                let ao_of = &ao_of;
                s.spawn(move || {
                    for v in part {
                        v.col[3] = ao_of(v);
                    }
                });
            }
        });
    }
}
