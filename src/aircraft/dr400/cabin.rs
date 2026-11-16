//! The cockpit under the bubble: the lining below the sills and the floor
//! on the wing's centre section, the panel and its gauges (the Cessna's
//! instruments, which plane.wgsl draws), the seats, a stick each, the
//! pedals, the throttle and flap lever between the seats, and the canopy:
//! a fixed windscreen and the bubble that slides forward over it.
use super::*;
use crate::aircraft::c172::cabin::gauge;
use glam::{Quat, Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI};

pub const PANEL_Z: f32 = -1.08;
pub const FLOOR_Y: f32 = -0.52;
/// The canopy's apex, and where the windscreen meets the sliding bubble.
pub const CANOPY_TOP: f32 = 0.60;
pub const SCREEN_Z: f32 = -0.80;

const PANEL_COL: Mat = mt(mat::PLASTIC, 0.035, 0.036, 0.038);
const TRIM: Mat = mt(mat::PLASTIC, 0.26, 0.25, 0.235);
const UPHOLSTERY: Mat = mt(mat::FABRIC, 0.11, 0.07, 0.05);
const KNOB: Mat = mt(mat::PLASTIC, 0.02, 0.02, 0.022);

/// Instruments: id, x, y, radius, on the panel.
const GAUGES: [(f32, f32, f32, f32); 15] = [
    (gauge::ASI, -0.44, 0.035, 0.040),
    (gauge::AI, -0.33, 0.035, 0.040),
    (gauge::ALT, -0.22, 0.035, 0.040),
    (gauge::TC, -0.44, -0.075, 0.040),
    (gauge::HI, -0.33, -0.075, 0.040),
    (gauge::VSI, -0.22, -0.075, 0.040),
    (gauge::TACH, -0.10, 0.035, 0.040),
    (gauge::CDI, -0.10, -0.075, 0.040),
    (gauge::FUEL_L, 0.10, 0.045, 0.024),
    (gauge::FUEL_R, 0.16, 0.045, 0.024),
    (gauge::OIL_T, 0.10, -0.015, 0.024),
    (gauge::OIL_P, 0.16, -0.015, 0.024),
    (gauge::AMPS, 0.22, -0.015, 0.024),
    (gauge::SUCTION, 0.22, 0.045, 0.024),
    (gauge::CLOCK, 0.43, 0.035, 0.028),
];
const STACK: [f32; 4] = [0.27, 0.40, -0.21, 0.08];

pub fn build(b: &mut Builder) {
    let mut m = Mesh::default();
    lining(&mut m);
    panel(&mut m);
    seats(&mut m);
    b.fixed("cabin", G_FUSE, m);
    controls(b);
    canopy(b);
}

// ---------------------------------------------------------------- shell

/// The sill's point (right side) at station z: where the skin stops under the canopy.
pub fn sill(z: f32) -> Vec2 {
    sec_pt(&sec(z), 0.62)
}
/// The canopy's height along the centre line at station z.
pub fn canopy_top(z: f32) -> f32 {
    let (z0, z1) = (CANOPY[0] - 0.02, CANOPY[1] + 0.02);
    let base = sec(z).top;
    let up = smoothstep(z0, -0.40, z).powf(0.75) * (1.0 - smoothstep(0.25, z1, z).powf(1.4));
    base + (CANOPY_TOP - base) * up
}
/// A point on the canopy at station z, u from the right sill (0) over the top (1) to the left sill (2).
pub fn canopy_pt(z: f32, u: f32) -> Vec3 {
    let s = sill(z);
    let top = canopy_top(z);
    let th = (u - 1.0) * FRAC_PI_2;
    let (sn, cs) = th.sin_cos();
    let e = |v: f32, n: f32| v.signum() * v.abs().powf(2.0 / n);
    // A bubble: a little fuller than the sill line at shoulder height.
    let bulge = 1.0 + 0.04 * (u * PI).sin();
    Vec3::new(-s.x * bulge * e(sn, 2.4), s.y + (top - s.y) * e(cs, 2.4), z)
}

/// Half width inside the walls at station z, height y (below the sill).
fn inner_x(z: f32, y: f32) -> f32 {
    let s = sec(z);
    let (mut a, mut c) = (0.62f32, PI);
    for _ in 0..30 {
        let m = (a + c) * 0.5;
        if sec_pt(&s, m).y > y { a = m } else { c = m }
    }
    sec_pt(&s, (a + c) * 0.5).x - 0.02
}

fn lining(m: &mut Mesh) {
    // The side walls, from the sill down to the floor.
    let (z0, z1) = (PANEL_Z - 0.08, CANOPY[1] + 0.05);
    let nz = 60;
    let mut side = Mesh::default();
    let rows: Vec<Vec<SP>> = (0..=nz)
        .map(|i| {
            let z = z0 + (z1 - z0) * i as f32 / nz as f32;
            let top = sill(z).y - 0.01;
            (0..=12)
                .map(|k| {
                    let y = top + (FLOOR_Y - 0.01 - top) * k as f32 / 12.0;
                    (Vec3::new(inner_x(z, y), y, z), [z, y], 0.0)
                })
                .collect()
        })
        .collect();
    side.grid(&TRIM.pat(pat::LINING), &rows, false, true);
    // The sill's top: a padded rail from the wall out to the skin's edge.
    let rail: Vec<Vec<SP>> = (0..=nz)
        .map(|i| {
            let z = z0 + (z1 - z0) * i as f32 / nz as f32;
            let s = sill(z);
            let a = Vec3::new(inner_x(z, s.y - 0.01), s.y - 0.01, z);
            vec![(a, [z, 0.0], 0.0), (Vec3::new(s.x, s.y, z), [z, 1.0], 0.0)]
        })
        .collect();
    side.grid(&mt(mat::FABRIC, 0.05, 0.05, 0.055), &rail, false, true);
    let l = side.mirror_x();
    m.append(&side);
    m.append(&l);
    // The floor on the centre section, the bulkhead behind the baggage shelf.
    let carpet = mt(mat::FABRIC, 0.06, 0.06, 0.065).pat(pat::CARPET);
    m.patch(&carpet, 24, 4, false, |a, c| {
        let z = z0 + (z1 - z0) * a;
        let w = inner_x(z, FLOOR_Y);
        (Vec3::new((c * 2.0 - 1.0) * w, FLOOR_Y, z), [z, c], 0.0)
    });
    let s = sec(z1);
    let ring: Vec<Vec3> = (0..=32).map(|k| { let t = 0.62 + (PI - 0.62) * k as f32 / 32.0; let p = sec_pt(&s, t); Vec3::new(p.x - 0.02, p.y, z1) }).collect();
    let mut ring2 = ring.clone();
    ring2.extend(ring.iter().rev().map(|p| Vec3::new(-p.x, p.y, p.z)));
    m.fan(&TRIM.pat(pat::LINING), Vec3::new(0.0, (s.mid + FLOOR_Y) * 0.5, z1), &ring2, Vec3::NEG_Z);
    // The firewall-side footwell closing the front, under the panel.
    let s = sec(PANEL_Z - 0.08);
    let ring: Vec<Vec3> = (0..=32).map(|k| { let t = 0.62 + (PI - 0.62) * k as f32 / 32.0; let p = sec_pt(&s, t); Vec3::new(p.x - 0.02, p.y, PANEL_Z - 0.08) }).collect();
    let mut ring2 = ring.clone();
    ring2.extend(ring.iter().rev().map(|p| Vec3::new(-p.x, p.y, p.z)));
    m.fan(&mt(mat::SATIN, 0.08, 0.08, 0.085), Vec3::new(0.0, s.mid, PANEL_Z - 0.08), &ring2, Vec3::Z);
}

// ---------------------------------------------------------------- panel

fn panel(m: &mut Mesh) {
    let z = PANEL_Z;
    let w = 0.50;
    let outline: Vec<Vec2> = [(-w, -0.25), (w, -0.25), (w, 0.10), (0.25, 0.125), (0.0, 0.13), (-0.25, 0.125), (-w, 0.10)].iter().map(|&(x, y)| Vec2::new(x, y)).collect();
    let mut holes: Vec<Vec<Vec2>> = GAUGES.iter().map(|g| (0..32).map(|k| { let a = -(k as f32) / 32.0 * std::f32::consts::TAU; Vec2::new(g.1, g.2) + Vec2::new(a.cos(), a.sin()) * (g.3 + 0.001) }).collect()).collect();
    holes.push(vec![Vec2::new(STACK[0], STACK[2]), Vec2::new(STACK[0], STACK[3]), Vec2::new(STACK[1], STACK[3]), Vec2::new(STACK[1], STACK[2])]);
    m.slab(&PANEL_COL.pat(pat::PANEL), &outline, &holes, Vec3::new(0.0, 0.0, z), Vec3::X, Vec3::Y, 0.012);
    // The glareshield: from the panel's lip forward to the windscreen's foot.
    let gs = mt(mat::FABRIC, 0.03, 0.03, 0.032);
    m.patch(&gs, 20, 8, false, |a, c| {
        let x = (a - 0.5) * 2.0 * w;
        let zz = z + 0.02 - c * (z - CANOPY[0] + 0.05);
        let top = sill(zz).y.max(sec(zz).top - 0.05 * (1.0 - smoothstep(CANOPY[0] - 0.02, CANOPY[0] + 0.1, zz)));
        let y = 0.13 + (top - 0.13) * c;
        let xc = x.signum() * x.abs().min(sill(zz).x - 0.02);
        (Vec3::new(xc, y, zz), [x, c], 0.0)
    });
    // Instruments: the recessed face, its bezel and four screws.
    for (k, &(id, x, y, r)) in GAUGES.iter().enumerate() {
        let c = Vec3::new(x, y, z - 0.010);
        m.disc(&mt(mat::GAUGE, 0.02, 0.02, 0.02).x(id), c, Vec3::Z, Vec3::Y, r, 40);
        m.lathe(&BLACK, c, Vec3::Z, &[[r, 0.0], [r, 0.0095], [r + 0.0035, 0.0125], [r + 0.0065, 0.0112], [r + 0.0068, 0.0098]], 40, false);
        let q = r * 1.06;
        for (dx, dy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            m.screw(&SCREW_BLACK, Vec3::new(x + dx * q, y + dy * q, z + 0.0005), Vec3::Z, 0.0025, k as f32 * 4.0 + dx + dy * 2.0);
        }
    }
    // Radios: a nav-com and a transponder.
    for (k, &(y1, y0, disp)) in [(0.08f32, -0.01f32, 0.0f32), (-0.02, -0.08, 2.0)].iter().enumerate() {
        let (cx, cy) = ((STACK[0] + STACK[1]) * 0.5, (y0 + y1) * 0.5);
        let (hx, hy) = ((STACK[1] - STACK[0]) * 0.5 - 0.001, (y1 - y0) * 0.5 - 0.0012);
        m.rbox(&mt(mat::PLASTIC, 0.045, 0.046, 0.05), Vec3::new(cx, cy, z - 0.003), Vec3::new(hx, hy, 0.008), 0.0025, Quat::IDENTITY);
        m.rbox(&mt(mat::DISPLAY, 0.0, 0.0, 0.0).x(disp), Vec3::new(cx - 0.01, cy + 0.006, z + 0.0055), Vec3::new(0.045, 0.009, 0.0005), 0.0003, Quat::IDENTITY);
        m.lathe(&KNOB.pat(pat::KNURL), Vec3::new(STACK[1] - 0.015, cy - hy * 0.3, z + 0.005), Vec3::Z, &[[0.010, 0.0], [0.010, 0.009], [0.0, 0.010]], 18, false);
        m.screw(&SCREW_BLACK, Vec3::new(cx - hx + 0.004, cy, z + 0.005), Vec3::Z, 0.0018, k as f32);
    }
    // Rocker switches and the magneto key along the bottom left, placards.
    let f = z + 0.0005;
    m.lathe(&CHROME, Vec3::new(-0.44, -0.17, f), Vec3::Z, &[[0.016, 0.0], [0.016, 0.004], [0.012, 0.007], [0.0, 0.007]], 24, false);
    m.rbox(&BLACK, Vec3::new(-0.44, -0.17, f + 0.02), Vec3::new(0.004, 0.012, 0.009), 0.003, Quat::IDENTITY);
    for i in 0..7 {
        let x = -0.37 + i as f32 * 0.03;
        let col = if i < 2 { mt(mat::PLASTIC, 0.35, 0.02, 0.02) } else { mt(mat::PLASTIC, 0.03, 0.03, 0.03) };
        m.rbox(&BLACK, Vec3::new(x, -0.19, f + 0.002), Vec3::new(0.008, 0.013, 0.002), 0.0015, Quat::IDENTITY);
        m.rbox(&col, Vec3::new(x, -0.19, f + 0.006), Vec3::new(0.0062, 0.011, 0.004), 0.002, Quat::from_rotation_x(if i % 2 == 0 { 0.18 } else { -0.18 }));
        m.rbox(&PAINT_WHITE.pat(pat::PLACARD), Vec3::new(x, -0.214, f), Vec3::new(0.011, 0.0035, 0.0004), 0.0002, Quat::IDENTITY);
    }
    // Mixture and carb heat pulls, the fuel selector and the circuit breakers.
    for (x, col) in [(0.02f32, mt(mat::PLASTIC, 0.35, 0.02, 0.02)), (0.07, mt(mat::PLASTIC, 0.04, 0.04, 0.04))] {
        m.tube(&CHROME, &[Vec3::new(x, -0.17, z - 0.02), Vec3::new(x, -0.17, z + 0.02)], &|_| 0.0045, 10, false);
        m.lathe(&col.pat(pat::KNURL), Vec3::new(x, -0.17, z + 0.02), Vec3::Z, &[[0.0045, 0.0], [0.013, 0.004], [0.014, 0.016], [0.0, 0.018]], 16, true);
    }
    m.disc(&mt(mat::GAUGE, 0.02, 0.02, 0.02).x(gauge::FUEL_SEL), Vec3::new(0.16, -0.16, f + 0.001), Vec3::Z, Vec3::Y, 0.045, 32);
    m.rbox(&mt(mat::PLASTIC, 0.35, 0.03, 0.02), Vec3::new(0.16, -0.16, f + 0.01), Vec3::new(0.008, 0.035, 0.008), 0.004, Quat::IDENTITY);
    for i in 0..8 {
        let p = Vec3::new(0.25 + i as f32 * 0.028, -0.20, f);
        m.lathe(&BLACK, p, Vec3::Z, &[[0.0065, 0.0], [0.0065, 0.002], [0.0048, 0.003], [0.0048, 0.009], [0.0, 0.0105]], 12, false);
    }
    // The compass on top of the glareshield.
    let c = Vec3::new(0.0, 0.16, z - 0.10);
    m.rbox(&BLACK, c, Vec3::new(0.036, 0.03, 0.032), 0.012, Quat::IDENTITY);
    m.rbox(&mt(mat::GAUGE, 0.02, 0.02, 0.02).x(gauge::COMPASS), c + Vec3::Z * 0.031, Vec3::new(0.026, 0.017, 0.0006), 0.0003, Quat::IDENTITY);
}

// ---------------------------------------------------------------- seats

fn seats(m: &mut Mesh) {
    let up = UPHOLSTERY.pat(pat::SEAT);
    let cushion = FLOOR_Y + 0.10;
    for xs in [-0.27f32, 0.27] {
        m.rbox(&up, Vec3::new(xs, cushion, -0.48), Vec3::new(0.20, 0.05, 0.24), 0.035, Quat::from_rotation_x(-0.10));
        m.rbox(&up, Vec3::new(xs, cushion + 0.33, -0.16), Vec3::new(0.20, 0.30, 0.05), 0.04, Quat::from_rotation_x(0.30));
        m.rbox(&ALU, Vec3::new(xs, FLOOR_Y + 0.025, -0.45), Vec3::new(0.16, 0.025, 0.20), 0.01, Quat::IDENTITY);
    }
    // The rear bench and the baggage shelf behind it.
    m.rbox(&up, Vec3::new(0.0, cushion, 0.40), Vec3::new(0.46, 0.05, 0.22), 0.04, Quat::IDENTITY);
    m.rbox(&up, Vec3::new(0.0, cushion + 0.30, 0.70), Vec3::new(0.46, 0.28, 0.05), 0.045, Quat::from_rotation_x(0.25));
    m.rbox(&TRIM, Vec3::new(0.0, FLOOR_Y + 0.04, 0.40), Vec3::new(0.47, 0.04, 0.2), 0.02, Quat::IDENTITY);
    // The tunnel between the front seats: throttle quadrant and flap lever on it.
    m.rbox(&PANEL_COL.pat(pat::PANEL), Vec3::new(0.0, FLOOR_Y + 0.08, -0.62), Vec3::new(0.06, 0.08, 0.40), 0.02, Quat::IDENTITY);
}

// ---------------------------------------------------------------- controls

fn controls(b: &mut Builder) {
    // A stick each, pivoting on the floor.
    for xs in [-0.27f32, 0.27] {
        let mut s = Mesh::default();
        let pv = Vec3::new(xs, FLOOR_Y + 0.02, -0.80);
        let grip = pv + Vec3::new(0.0, 0.50, 0.04);
        s.lathe(&mt(mat::RUBBER, 0.03, 0.03, 0.03), pv, Vec3::Y, &[[0.06, 0.0], [0.05, 0.05], [0.02, 0.09], [0.0, 0.10]], 16, true);
        s.tube(&BLACK, &[pv, grip], &|_| 0.011, 12, true);
        s.tube(&KNOB.pat(pat::KNURL), &[grip - Vec3::Y * 0.11, grip], &|t| 0.016 + 0.004 * (t * PI).sin(), 14, true);
        s.dome(&mt(mat::PLASTIC, 0.35, 0.03, 0.03), grip + Vec3::Y * 0.005, Vec3::Y, 0.008, 0.005, 10, 0.0);
        b.add("stick", G_FUSE, s, Anim::turn_slide(Drive::new(Input::Roll, -0.3), Drive::new(Input::Pitch, 0.0), Vec3::Z), pv, Vec3::Z, None);
        let mut s2 = Mesh::default();
        s2.disc(&BLACK, pv + Vec3::Y * 0.001, Vec3::Y, Vec3::Z, 0.07, 20);
        b.fixed("stick boot", G_FUSE, s2);
        // Rudder pedals.
        for dx in [-0.085f32, 0.085] {
            let mut p = Mesh::default();
            let pv = Vec3::new(xs + dx, FLOOR_Y + 0.25, PANEL_Z - 0.30);
            let pc = Vec3::new(xs + dx, FLOOR_Y + 0.07, PANEL_Z - 0.22);
            p.tube(&GREY, &[pv, pc], &|_| 0.009, 8, true);
            let tilt = Quat::from_rotation_x(-0.45);
            p.rbox(&ALU, pc + Vec3::new(0.0, 0.0, 0.008), Vec3::new(0.034, 0.06, 0.005), 0.004, tilt);
            b.add("pedal", G_FUSE, p, Anim::hinge(Drive::new(Input::Yaw, dx.signum() * 0.2)), pv, Vec3::X, None);
        }
    }
    // The throttle on the tunnel (pushed forward for power).
    let mut t = Mesh::default();
    let tp = Vec3::new(-0.03, FLOOR_Y + 0.16, -0.90);
    t.tube(&CHROME, &[tp, tp + Vec3::new(0.0, 0.10, 0.0)], &|_| 0.005, 10, false);
    t.lathe(&KNOB, tp + Vec3::Y * 0.10, Vec3::Y, &[[0.005, 0.0], [0.016, 0.005], [0.017, 0.02], [0.0, 0.026]], 18, true);
    b.add("throttle", G_FUSE, t, Anim::hinge(Drive::new(Input::Throttle, -0.6).offset(0.3)), tp, Vec3::X, None);
    // The flap lever on the tunnel, a handbrake-style handle.
    let mut fl = Mesh::default();
    let fp = Vec3::new(0.0, FLOOR_Y + 0.16, -0.45);
    fl.tube(&CHROME, &[fp, fp + Vec3::new(0.0, 0.03, -0.20)], &|_| 0.008, 10, true);
    fl.tube(&BLACK, &[fp + Vec3::new(0.0, 0.025, -0.16), fp + Vec3::new(0.0, 0.035, -0.23)], &|_| 0.013, 12, true);
    b.add("flap lever", G_FUSE, fl, Anim::hinge(Drive::new(Input::FlapLever, 0.8)), fp, Vec3::X, None);
}

// ---------------------------------------------------------------- canopy

fn canopy(b: &mut Builder) {
    let (z0, z1) = (CANOPY[0] - 0.02, CANOPY[1] + 0.02);
    let nz = 90;
    let nu = 48;
    let rows: Vec<Vec<SP>> = (0..=nz)
        .map(|i| {
            let z = z0 + (z1 - z0) * i as f32 / nz as f32;
            (0..=nu).map(|k| { let p = canopy_pt(z, k as f32 / nu as f32 * 2.0); (p, [z, p.x], 0.0) }).collect()
        })
        .collect();
    let mut g = Mesh::default();
    g.grid(&mt(mat::GLASS, 0.0, 0.0, 0.0), &rows, false, false);
    let k = b.fixed("canopy", G_FUSE, g);
    b.pieces[k].glass = true;
    b.pieces[k].occluder = false;
    // Its frame: the arch where the windscreen ends, the rear bow, the rails on the sills.
    let mut f = Mesh::default();
    let frame = mt(mat::PAINT, 0.80, 0.80, 0.78);
    for (z, r) in [(SCREEN_Z, 0.012f32), (CANOPY[1] - 0.05, 0.016)] {
        let arc: Vec<Vec3> = (0..=40).map(|k| canopy_pt(z, k as f32 / 20.0) + Vec3::Y * 0.004).collect();
        f.tube(&frame, &arc, &|_| r, 8, true);
    }
    for sx in [-1.0f32, 1.0] {
        let rail: Vec<Vec3> = (0..=30).map(|k| { let z = z0 + (z1 - z0) * k as f32 / 30.0; let s = sill(z); Vec3::new(sx * (s.x + 0.004), s.y + 0.006, z) }).collect();
        f.tube(&CHROME, &rail, &|_| 0.008, 8, true);
    }
    // The handle inside the arch.
    let h = canopy_pt(SCREEN_Z + 0.03, 1.0) - Vec3::Y * 0.03;
    f.tube(&BLACK, &[h - Vec3::X * 0.06, h - Vec3::X * 0.06 - Vec3::Y * 0.02, h + Vec3::X * 0.06 - Vec3::Y * 0.02, h + Vec3::X * 0.06], &|_| 0.007, 8, true);
    b.fixed("canopy frame", G_FUSE, f);
}

// ---------------------------------------------------------------- what the eye sees

/// Whether a look from `eye` along `d` (body frame) ends on something in the
/// cockpit (true) or goes out through the canopy (false).
pub fn sees_inside(eye: Vec3, d: Vec3) -> bool {
    let mut t = 0.02;
    while t < 3.0 {
        let p = eye + d * t;
        if p.y < FLOOR_Y + 0.01 {
            return true;
        }
        if p.z < PANEL_Z + 0.01 && p.y < 0.13 + 0.6 * (PANEL_Z - p.z).max(0.0) {
            return true;
        }
        if p.z < CANOPY[0] || p.z > CANOPY[1] {
            return p.y < sec(p.z).top;
        }
        let s = sill(p.z);
        if p.y < s.y {
            if p.x.abs() > inner_x(p.z, p.y) {
                return true;
            }
        } else {
            // Out through the glass.
            let top = canopy_top(p.z);
            let u = ((p.y - s.y) / (top - s.y).max(1e-3)).clamp(0.0, 1.0);
            if p.y > top || p.x.abs() > s.x * (1.0 - u * u).max(0.0).sqrt() {
                return false;
            }
        }
        t += 0.015 + t * 0.01;
    }
    true
}
