//! Map data sources: satellite imagery, elevation, and OSM vector tiles, with
//! decoded in-memory caches, plus the worker pool that builds tiles off-thread.
use crate::fetch::Fetcher;
use crate::geo::{self, TileId};
use crate::mvt;
use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use glam::DVec3;
use parking_lot::{Condvar, Mutex};
use std::collections::HashMap;
use std::sync::Arc;

pub const DATA_ZOOM: u8 = 14; // deepest zoom of elevation + vector tiles
pub const GRID: usize = 64; // cells per tile side
pub const GV: usize = GRID + 1;
pub const MASK: usize = 256;

pub struct ElevTile {
    pub h: Vec<f32>,
    pub n: usize,
}
impl ElevTile {
    pub fn sample(&self, u: f64, v: f64) -> f32 {
        let n = self.n as f64;
        let x = (u * n - 0.5).clamp(0.0, n - 1.0);
        let y = (v * n - 0.5).clamp(0.0, n - 1.0);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.n - 1), (y0 + 1).min(self.n - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let g = |x: usize, y: usize| self.h[y * self.n + x];
        (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy) + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
    }
}

pub struct BuildingPoly {
    /// Exterior ring first, then holes; tile units.
    pub rings: Vec<Vec<[f32; 2]>>,
    pub height: f32,
    pub min_height: f32,
    /// Colour tagged in OpenStreetMap, when someone bothered.
    pub colour: Option<[u8; 3]>,
}

// Road classes, most to least important.
pub const RC_MOTORWAY: u8 = 0;
pub const RC_TRUNK: u8 = 1;
pub const RC_PRIMARY: u8 = 2;
pub const RC_SECONDARY: u8 = 3;
pub const RC_TERTIARY: u8 = 4;
pub const RC_MINOR: u8 = 5;
pub const RC_SERVICE: u8 = 6;
pub const RC_TRACK: u8 = 7;
pub const RC_PATH: u8 = 8;
pub const RC_RAIL: u8 = 9;

pub struct RoadLine {
    pub pts: Vec<[f32; 2]>,
    pub class: u8,
    /// 0 none, 1 bridge, 2 tunnel
    pub brunnel: u8,
    pub oneway: bool,
    pub ramp: bool,
    pub layer: i8,
    /// service roads: 1 = alley / driveway / parking aisle; paths: 1 = wide pedestrian way
    pub sub: u8,
}

// Land cover kinds.
pub const LC_WOOD: u8 = 1;
pub const LC_SCRUB: u8 = 2;
pub const LC_GRASS: u8 = 3;
pub const LC_FARM: u8 = 4;
pub const LC_WET: u8 = 5;
pub const LC_BARE: u8 = 6; // sand, ice
pub const LC_ROCK: u8 = 7;
// Land use kinds.
pub const LU_RESIDENTIAL: u8 = 1;
pub const LU_COMMERCIAL: u8 = 2;
pub const LU_INDUSTRIAL: u8 = 3;
pub const LU_OPEN: u8 = 4; // pitches, playgrounds: nothing grows or gets built here

#[derive(Default)]
pub struct VecTile {
    pub extent: f32,
    /// (class, rings): 1 ocean, 2 lake/pond, 3 river
    pub water: Vec<(u8, Vec<Vec<[f32; 2]>>)>,
    pub buildings: Vec<BuildingPoly>,
    pub roads: Vec<RoadLine>,
    pub landcover: Vec<(u8, Vec<Vec<[f32; 2]>>)>,
    pub landuse: Vec<(u8, Vec<Vec<[f32; 2]>>)>,
}

/// CSS-ish colour as tagged on OSM buildings: #rgb, #rrggbb or a common name.
fn parse_colour(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(h) = s.strip_prefix('#') {
        let v = u32::from_str_radix(h, 16).ok()?;
        return match h.len() {
            6 => Some([(v >> 16) as u8, (v >> 8) as u8, v as u8]),
            3 => Some([((v >> 8) & 15) as u8 * 17, ((v >> 4) & 15) as u8 * 17, (v & 15) as u8 * 17]),
            _ => None,
        };
    }
    Some(match s.as_str() {
        "white" => [240, 240, 236],
        "black" => [40, 40, 42],
        "grey" | "gray" => [140, 140, 140],
        "lightgrey" | "lightgray" | "silver" => [196, 196, 196],
        "darkgrey" | "darkgray" => [90, 90, 92],
        "red" => [170, 60, 50],
        "maroon" | "darkred" => [120, 45, 40],
        "brown" => [125, 85, 60],
        "orange" => [215, 135, 70],
        "yellow" => [225, 200, 110],
        "beige" | "cream" | "ivory" => [226, 214, 186],
        "tan" | "sand" => [200, 175, 135],
        "pink" => [225, 180, 180],
        "green" => [100, 140, 95],
        "blue" => [95, 130, 175],
        "lightblue" => [160, 195, 220],
        _ => return None,
    })
}

struct Lru<T> {
    map: HashMap<TileId, (Arc<T>, u64)>,
    tick: u64,
    cap: usize,
}
impl<T> Lru<T> {
    fn new(cap: usize) -> Self {
        Self { map: HashMap::new(), tick: 0, cap }
    }
    fn get(&mut self, id: TileId) -> Option<Arc<T>> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(&id).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }
    fn put(&mut self, id: TileId, v: Arc<T>) {
        if self.map.len() >= self.cap {
            if let Some(old) = self.map.iter().min_by_key(|(_, e)| e.1).map(|(k, _)| *k) {
                self.map.remove(&old);
            }
        }
        self.tick += 1;
        self.map.insert(id, (v, self.tick));
    }
}

pub struct Sources {
    pub fetch: Fetcher,
    elev: Mutex<Lru<ElevTile>>,
    vec: Mutex<Lru<VecTile>>,
    vec_url: Mutex<Option<String>>,
    pub imagery_url: String,
}

impl Sources {
    pub fn new(fetch: Fetcher, imagery_url: String) -> Self {
        Self { fetch, elev: Mutex::new(Lru::new(96)), vec: Mutex::new(Lru::new(96)), vec_url: Mutex::new(None), imagery_url }
    }

    pub fn imagery(&self, id: TileId) -> Result<image::RgbaImage> {
        let url = self.imagery_url.replace("{z}", &id.z.to_string()).replace("{x}", &id.x.to_string()).replace("{y}", &id.y.to_string());
        let data = self.fetch.get_cached(&format!("img/{}/{}/{}.jpg", id.z, id.x, id.y), &url)?;
        Ok(image::load_from_memory(&data)?.to_rgba8())
    }

    pub fn elev(&self, id: TileId) -> Result<Arc<ElevTile>> {
        if let Some(t) = self.elev.lock().get(id) {
            return Ok(t);
        }
        let url = format!("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{}/{}/{}.png", id.z, id.x, id.y);
        let data = self.fetch.get_cached(&format!("elev/{}/{}/{}.png", id.z, id.x, id.y), &url)?;
        let img = image::load_from_memory(&data)?.to_rgb8();
        let n = img.width() as usize;
        let h = img.pixels().map(|p| p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0 - 32768.0).collect();
        let t = Arc::new(ElevTile { h, n });
        self.elev.lock().put(id, t.clone());
        Ok(t)
    }

    fn vec_template(&self) -> Result<String> {
        if let Some(u) = self.vec_url.lock().clone() {
            return Ok(u);
        }
        // OpenFreeMap publishes the current planet build in a TileJSON document.
        let u = self
            .fetch
            .get("https://tiles.openfreemap.org/planet")
            .ok()
            .and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok())
            .and_then(|j| j["tiles"][0].as_str().map(String::from))
            .ok_or_else(|| anyhow!("no vector tile endpoint"))?;
        *self.vec_url.lock() = Some(u.clone());
        Ok(u)
    }

    pub fn vector(&self, id: TileId) -> Result<Arc<VecTile>> {
        if let Some(t) = self.vec.lock().get(id) {
            return Ok(t);
        }
        let key = format!("vec/{}/{}/{}.pbf", id.z, id.x, id.y);
        let data = match std::fs::read(self.fetch.root.join(&key)) {
            Ok(d) if !d.is_empty() => d,
            _ => {
                let url = self.vec_template()?.replace("{z}", &id.z.to_string()).replace("{x}", &id.x.to_string()).replace("{y}", &id.y.to_string());
                self.fetch.get_cached(&key, &url)?
            }
        };
        let mut layers = mvt::decode(&data, &["water", "building", "transportation", "landcover", "landuse"]);
        let mut vt = VecTile { extent: 4096.0, ..Default::default() };
        if let Some(l) = layers.remove("water") {
            vt.extent = l.extent;
            for f in l.features {
                if f.kind != 3 {
                    continue;
                }
                let class = match f.props.get("class").and_then(|v| v.str()) {
                    Some("ocean") => 1,
                    Some("river") => 3,
                    _ => 2,
                };
                vt.water.push((class, f.rings));
            }
        }
        if let Some(l) = layers.remove("building") {
            vt.extent = l.extent;
            for f in l.features {
                if f.kind != 3 || f.rings.is_empty() {
                    continue;
                }
                if f.props.get("hide_3d").is_some() {
                    continue;
                }
                let height = f.props.get("render_height").and_then(|v| v.num()).unwrap_or(6.0) as f32;
                let min_height = f.props.get("render_min_height").and_then(|v| v.num()).unwrap_or(0.0) as f32;
                let colour = f.props.get("colour").and_then(|v| v.str()).and_then(parse_colour);
                // Split multipolygons: a ring wound like the first one starts a new polygon.
                let sign = mvt::signed_area(&f.rings[0]).signum();
                let mut cur: Vec<Vec<[f32; 2]>> = Vec::new();
                for r in f.rings {
                    if r.len() < 3 {
                        continue;
                    }
                    if mvt::signed_area(&r).signum() == sign && !cur.is_empty() {
                        vt.buildings.push(BuildingPoly { rings: std::mem::take(&mut cur), height, min_height, colour });
                    }
                    cur.push(r);
                }
                if !cur.is_empty() {
                    vt.buildings.push(BuildingPoly { rings: cur, height, min_height, colour });
                }
            }
        }
        if let Some(l) = layers.remove("transportation") {
            let k = vt.extent / l.extent;
            for f in l.features {
                if f.kind != 2 {
                    continue;
                }
                let st = |k: &str| f.props.get(k).and_then(|v| v.str());
                let sub = st("subclass").unwrap_or("");
                let (class, sub) = match st("class").unwrap_or("") {
                    "motorway" => (RC_MOTORWAY, 0),
                    "trunk" => (RC_TRUNK, 0),
                    "primary" => (RC_PRIMARY, 0),
                    "secondary" => (RC_SECONDARY, 0),
                    "tertiary" => (RC_TERTIARY, 0),
                    "minor" | "raceway" => (RC_MINOR, 0),
                    "service" | "busway" => (RC_SERVICE, st("service").is_some() as u8),
                    "track" => (RC_TRACK, 0),
                    "path" => match sub {
                        "platform" | "corridor" | "steps" => continue,
                        "pedestrian" => (RC_PATH, 1),
                        _ => (RC_PATH, 0),
                    },
                    "rail" => (RC_RAIL, 0),
                    _ => continue,
                };
                let brunnel = match st("brunnel") {
                    Some("bridge") => 1,
                    Some("tunnel") => 2,
                    _ => 0,
                };
                let num = |k: &str| f.props.get(k).and_then(|v| v.num()).unwrap_or(0.0);
                let (oneway, ramp, layer) = (num("oneway") != 0.0, num("ramp") != 0.0, num("layer") as i8);
                let rev = num("oneway") < 0.0;
                for mut pts in f.rings {
                    if pts.len() < 2 {
                        continue;
                    }
                    for p in pts.iter_mut() {
                        *p = [p[0] * k, p[1] * k];
                    }
                    if rev {
                        pts.reverse();
                    }
                    vt.roads.push(RoadLine { pts, class, brunnel, oneway, ramp, layer, sub });
                }
            }
        }
        for (name, is_cover) in [("landcover", true), ("landuse", false)] {
            let Some(l) = layers.remove(name) else { continue };
            let k = vt.extent / l.extent;
            for f in l.features {
                if f.kind != 3 {
                    continue;
                }
                let class = f.props.get("class").and_then(|v| v.str()).unwrap_or("");
                let sub = f.props.get("subclass").and_then(|v| v.str()).unwrap_or("");
                let kind = if is_cover {
                    match (class, sub) {
                        ("wood", _) => LC_WOOD,
                        (_, "scrub") | (_, "heath") => LC_SCRUB,
                        ("grass", _) => LC_GRASS,
                        ("farmland", _) => LC_FARM,
                        ("wetland", _) => LC_WET,
                        ("rock", _) => LC_ROCK,
                        ("sand", _) | ("ice", _) => LC_BARE,
                        _ => continue,
                    }
                } else {
                    match class {
                        "residential" | "suburb" | "neighbourhood" | "quarter" => LU_RESIDENTIAL,
                        "commercial" | "retail" => LU_COMMERCIAL,
                        "industrial" | "railway" | "garages" => LU_INDUSTRIAL,
                        "pitch" | "playground" | "track" | "stadium" => LU_OPEN,
                        _ => continue,
                    }
                };
                let rings = f.rings.into_iter().map(|r| r.into_iter().map(|p| [p[0] * k, p[1] * k]).collect()).collect();
                if is_cover {
                    vt.landcover.push((kind, rings));
                } else {
                    vt.landuse.push((kind, rings));
                }
            }
        }
        let t = Arc::new(vt);
        self.vec.lock().put(id, t.clone());
        Ok(t)
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct TerrainVertex {
    pub pos: [f32; 3],
    pub depth: f32,
    pub nrm: [f32; 3],
    pub skirt: f32,
    pub uv: [f32; 2],
}

pub struct TileData {
    pub id: TileId,
    pub center: DVec3,
    pub radius: f64,
    pub verts: Vec<TerrainVertex>,
    /// Surface height (water surface where there is water) on the GV x GV grid.
    pub heights: Vec<f32>,
    pub water: Vec<bool>,
    pub imagery: Vec<Vec<u8>>, // mip chain, rgba
    pub img_size: u32,
    pub mask: Vec<Vec<u8>>, // mip chain, rgba (see ground_mask)
}

pub fn mip_chain(mut data: Vec<u8>, mut size: usize, ch: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while size > 1 {
        let half = size / 2;
        let mut next = vec![0u8; half * half * ch];
        for y in 0..half {
            for x in 0..half {
                for c in 0..ch {
                    let s = |dx: usize, dy: usize| data[((y * 2 + dy) * size + x * 2 + dx) * ch + c] as u32;
                    next[(y * half + x) * ch + c] = ((s(0, 0) + s(1, 0) + s(0, 1) + s(1, 1) + 2) / 4) as u8;
                }
            }
        }
        out.push(std::mem::replace(&mut data, next));
        size = half;
    }
    out.push(data);
    out
}

/// Rasterise the water polygons covering `id` into an RG mask
/// (R = coverage, G = water class * 80).
fn water_mask(vt: &VecTile, ox: f64, oy: f64, scale: f64) -> Vec<u8> {
    const SS: usize = MASK * 2;
    let mut cls = vec![0u8; SS * SS];
    let k = SS as f64 / scale / vt.extent as f64;
    let (ox, oy) = (ox * vt.extent as f64, oy * vt.extent as f64);
    // Lakes and rivers go on top of ocean so inland classes win where both exist.
    for pass in [1u8, 2, 3] {
        for (class, rings) in vt.water.iter().filter(|w| w.0 == pass) {
            let px: Vec<Vec<[f32; 2]>> =
                rings.iter().map(|r| r.iter().map(|p| [((p[0] as f64 - ox) * k) as f32, ((p[1] as f64 - oy) * k) as f32]).collect()).collect();
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for p in px.iter().flatten() {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
            if hi[0] < 0.0 || hi[1] < 0.0 || lo[0] > SS as f32 || lo[1] > SS as f32 {
                continue;
            }
            let mut layer = vec![0u8; SS * SS];
            mvt::fill_rings(&mut layer, SS, SS, &px, *class);
            for (d, s) in cls.iter_mut().zip(&layer) {
                if *s != 0 {
                    *d = *s;
                }
            }
        }
    }
    let mut out = vec![0u8; MASK * MASK * 2];
    for y in 0..MASK {
        for x in 0..MASK {
            let s = [cls[y * 2 * SS + x * 2], cls[y * 2 * SS + x * 2 + 1], cls[(y * 2 + 1) * SS + x * 2], cls[(y * 2 + 1) * SS + x * 2 + 1]];
            let n = s.iter().filter(|c| **c != 0).count();
            out[(y * MASK + x) * 2] = (n * 255 / 4) as u8;
            out[(y * MASK + x) * 2 + 1] = s.iter().copied().max().unwrap_or(0) * 80;
        }
    }
    out
}

// Ground classes in the B channel of the tile mask (low nibble; the high
// nibble is a per-polygon seed). Land cover kinds map onto themselves.
pub const GC_PAVED: u8 = 8;
pub const GC_PITCH: u8 = 9;

/// The full RGBA tile mask: water (R coverage, G class * 80) from `water_mask`,
/// B = ground class | seed << 4, A = direction of the polygon's longest side
/// (0..255 over 0..pi, from east toward north): crop rows and mowing stripes
/// run along it.
fn ground_mask(vt: &VecTile, ox: f64, oy: f64, scale: f64, tile_m: f64) -> Vec<u8> {
    let water = water_mask(vt, ox, oy, scale);
    let mut out = vec![0u8; MASK * MASK * 4];
    for i in 0..MASK * MASK {
        out[i * 4] = water[i * 2];
        out[i * 4 + 1] = water[i * 2 + 1];
    }
    let k = MASK as f64 / scale / vt.extent as f64;
    let (ox, oy) = (ox * vt.extent as f64, oy * vt.extent as f64);
    let to_px = |p: &[f32; 2]| [((p[0] as f64 - ox) * k) as f32, ((p[1] as f64 - oy) * k) as f32];
    let mut scratch = vec![0u8; MASK * MASK];
    let mut stamp = |out: &mut [u8], rings: &[Vec<[f32; 2]>], class: u8| {
        let px: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| r.iter().map(to_px).collect()).collect();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in px.iter().flatten() {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        if px.is_empty() || hi[0] < 0.0 || hi[1] < 0.0 || lo[0] > MASK as f32 || lo[1] > MASK as f32 {
            return;
        }
        // Longest side of the outer ring.
        let r0 = &px[0];
        let (mut best, mut ang) = (0f32, 0f32);
        for i in 0..r0.len() {
            let (a, b) = (r0[i], r0[(i + 1) % r0.len()]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx * dx + dy * dy;
            if l > best {
                best = l;
                ang = (-dy).atan2(dx).rem_euclid(std::f32::consts::PI);
            }
        }
        let first = rings[0][0];
        let seed = ((first[0].to_bits().wrapping_mul(0x9e37_79b9) ^ first[1].to_bits().wrapping_mul(0x85eb_ca6b)) >> 28) as u8;
        let a = ((ang / std::f32::consts::PI * 256.0) as u32).min(255) as u8;
        mvt::fill_rings(&mut scratch, MASK, MASK, &px, 1);
        let (x0, y0) = (lo[0].max(0.0) as usize, lo[1].max(0.0) as usize);
        let (x1, y1) = ((hi[0].ceil().max(0.0) as usize + 1).min(MASK), (hi[1].ceil().max(0.0) as usize + 1).min(MASK));
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * MASK + x;
                if scratch[i] != 0 {
                    scratch[i] = 0;
                    out[i * 4 + 2] = class | seed << 4;
                    out[i * 4 + 3] = a;
                }
            }
        }
    };
    // Later kinds win where polygons overlap (as for tree placement).
    for want in [LC_GRASS, LC_FARM, LC_WET, LC_SCRUB, LC_WOOD, LC_BARE, LC_ROCK] {
        for (_, rings) in vt.landcover.iter().filter(|l| l.0 == want) {
            stamp(&mut out, rings, want);
        }
    }
    for (_, rings) in vt.landuse.iter().filter(|l| l.0 == LU_OPEN) {
        stamp(&mut out, rings, GC_PITCH);
    }
    // Road surface: no grass, crops or rock relief under the road ribbons.
    let px_m = (tile_m / MASK as f64) as f32;
    for r in &vt.roads {
        if r.brunnel != 0 {
            continue;
        }
        let w_m = match r.class {
            RC_MOTORWAY => 13.0,
            RC_TRUNK => 11.0,
            RC_PRIMARY => 9.0,
            RC_SECONDARY => 8.0,
            RC_TERTIARY => 7.0,
            RC_MINOR => 6.0,
            RC_SERVICE => 4.0,
            RC_RAIL => 4.0,
            RC_TRACK => 3.0,
            _ => 2.0,
        };
        let hw = (w_m * 0.5 + 0.5) / px_m;
        for s in r.pts.windows(2) {
            let (a, b) = (to_px(&s[0]), to_px(&s[1]));
            let (x0, x1) = ((a[0].min(b[0]) - hw).floor().max(0.0) as i64, (a[0].max(b[0]) + hw).ceil().min(MASK as f32) as i64);
            let (y0, y1) = ((a[1].min(b[1]) - hw).floor().max(0.0) as i64, (a[1].max(b[1]) + hw).ceil().min(MASK as f32) as i64);
            let d = [b[0] - a[0], b[1] - a[1]];
            let dd = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = [x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]];
                    let t = ((p[0] * d[0] + p[1] * d[1]) / dd).clamp(0.0, 1.0);
                    let (ex, ey) = (p[0] - d[0] * t, p[1] - d[1] * t);
                    if ex * ex + ey * ey < hw * hw {
                        let i = (y as usize * MASK + x as usize) * 4;
                        out[i + 2] = GC_PAVED;
                    }
                }
            }
        }
    }
    out
}

pub fn build_tile(src: &Sources, id: TileId) -> Result<TileData> {
    let img = src.imagery(id)?;
    let img_size = img.width();
    if img_size != img.height() || !img_size.is_power_of_two() {
        return Err(anyhow!("unexpected imagery size"));
    }
    let (did, ox, oy, sc) = id.ancestor(DATA_ZOOM);
    let elev = src.elev(did).ok();
    let mask = match src.vector(did) {
        Ok(vt) => ground_mask(&vt, ox, oy, sc, id.width_m()),
        // No vector data: fall back to "below sea level is ocean".
        Err(_) => {
            let mut m = vec![0u8; MASK * MASK * 4];
            if let Some(e) = &elev {
                for y in 0..MASK {
                    for x in 0..MASK {
                        let (u, v) = ((x as f64 + 0.5) / MASK as f64, (y as f64 + 0.5) / MASK as f64);
                        if e.sample(ox + u * sc, oy + v * sc) < -1.0 {
                            m[(y * MASK + x) * 4] = 255;
                            m[(y * MASK + x) * 4 + 1] = 80;
                        }
                    }
                }
            }
            m
        }
    };

    let (clat, clon) = id.lat_lon(0.5, 0.5);
    let center = geo::lla_to_pos(clat, clon, 0.0);
    // Grid with a one-cell border so normals are continuous across tiles.
    let e = GV + 2;
    let mut pts = vec![DVec3::ZERO; e * e];
    let mut hs = vec![(0f32, 0f32, false); e * e]; // surface height, raw elevation, water
    for j in 0..e {
        for i in 0..e {
            let (u, v) = ((i as f64 - 1.0) / GRID as f64, (j as f64 - 1.0) / GRID as f64);
            let (uc, vc) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            let raw = elev.as_ref().map(|t| t.sample(ox + uc * sc, oy + vc * sc)).unwrap_or(0.0);
            let (mx, my) = (((uc * MASK as f64) as usize).min(MASK - 1), ((vc * MASK as f64) as usize).min(MASK - 1));
            let water = mask[(my * MASK + mx) * 4] > 127;
            let h = if water { raw.max(0.0) } else { raw };
            let (lat, lon) = id.lat_lon(u, v);
            pts[j * e + i] = geo::lla_to_pos(lat, lon, h as f64) - center;
            hs[j * e + i] = (h, raw, water);
        }
    }
    let mut verts = Vec::with_capacity(GV * GV + 4 * GV);
    let (mut heights, mut water) = (Vec::with_capacity(GV * GV), Vec::with_capacity(GV * GV));
    let mut radius = 0f64;
    for j in 1..=GV {
        for i in 1..=GV {
            let p = pts[j * e + i];
            let up = (p + center).normalize();
            let mut n = (pts[j * e + i + 1] - pts[j * e + i - 1]).cross(pts[(j - 1) * e + i] - pts[(j + 1) * e + i]);
            if n.length_squared() < 1e-9 || n.dot(up) <= 0.0 {
                n = up;
            }
            let n = n.normalize();
            let (h, raw, w) = hs[j * e + i];
            radius = radius.max(p.length());
            heights.push(h);
            water.push(w);
            verts.push(TerrainVertex {
                pos: p.as_vec3().into(),
                depth: if w { (-raw).max(0.0) } else { 0.0 },
                nrm: n.as_vec3().into(),
                skirt: 0.0,
                uv: [(i - 1) as f32 / GRID as f32, (j - 1) as f32 / GRID as f32],
            });
        }
    }
    // Skirts hide the cracks between neighbouring tiles of different detail.
    let drop = (id.width_m() * 0.04).max(4.0);
    let mut skirt = |i: usize, j: usize| {
        let mut v = verts[j * GV + i];
        let up = (DVec3::from(glam::Vec3::from(v.pos).as_dvec3()) + center).normalize();
        v.pos = (glam::Vec3::from(v.pos).as_dvec3() - up * drop).as_vec3().into();
        v.skirt = 1.0;
        verts.push(v);
    };
    for i in 0..GV {
        skirt(i, 0);
    }
    for i in 0..GV {
        skirt(i, GRID);
    }
    for j in 0..GV {
        skirt(0, j);
    }
    for j in 0..GV {
        skirt(GRID, j);
    }
    Ok(TileData {
        id,
        center,
        radius: radius + drop,
        verts,
        heights,
        water,
        imagery: mip_chain(img.into_raw(), img_size as usize, 4),
        img_size,
        mask: mip_chain(mask, MASK, 4),
    })
}

/// Index buffer shared by every terrain tile (grid + four skirts).
pub fn tile_indices() -> Vec<u16> {
    let mut ix = Vec::new();
    for j in 0..GRID {
        for i in 0..GRID {
            let (a, b, c, d) = (j * GV + i, j * GV + i + 1, (j + 1) * GV + i, (j + 1) * GV + i + 1);
            ix.extend([a, c, b, b, c, d].map(|v| v as u16));
        }
    }
    let base = GV * GV;
    // (edge vertex index fn, skirt strip start, flip winding)
    let edges: [(Box<dyn Fn(usize) -> usize>, usize, bool); 4] = [
        (Box::new(|i| i), base, false),
        (Box::new(|i| GRID * GV + i), base + GV, true),
        (Box::new(|j| j * GV), base + 2 * GV, true),
        (Box::new(|j| j * GV + GRID), base + 3 * GV, false),
    ];
    for (edge, s, flip) in edges.iter() {
        for k in 0..GRID {
            let (a, b, c, d) = (edge(k), edge(k + 1), s + k, s + k + 1);
            let t = if *flip { [a, c, b, b, c, d] } else { [a, b, c, b, d, c] };
            ix.extend(t.map(|v| v as u16));
        }
    }
    ix
}

// ---------------------------------------------------------------- worker pool

pub enum Job {
    Terrain(TileId),
    Buildings(TileId),
}
pub enum JobResult {
    Terrain(TileId, Result<TileData>),
    Buildings(TileId, Result<crate::buildings::BuildingData>),
}

pub struct Loader {
    queue: Arc<(Mutex<Vec<(f32, Job)>>, Condvar)>,
    pub rx: crossbeam_channel::Receiver<JobResult>,
    pub src: Arc<Sources>,
}

impl Loader {
    pub fn new(src: Arc<Sources>, threads: usize) -> Self {
        let queue = Arc::new((Mutex::new(Vec::<(f32, Job)>::new()), Condvar::new()));
        let (tx, rx) = crossbeam_channel::unbounded();
        for _ in 0..threads {
            let (queue, tx, src) = (queue.clone(), tx.clone(), src.clone());
            std::thread::spawn(move || loop {
                let job = {
                    let mut q = queue.0.lock();
                    while q.is_empty() {
                        queue.1.wait(&mut q);
                    }
                    let best = q.iter().enumerate().min_by(|a, b| a.1 .0.total_cmp(&b.1 .0)).map(|(i, _)| i).unwrap();
                    q.swap_remove(best).1
                };
                let res = match job {
                    Job::Terrain(id) => JobResult::Terrain(id, build_tile(&src, id)),
                    Job::Buildings(id) => JobResult::Buildings(id, crate::buildings::build(&src, id)),
                };
                if tx.send(res).is_err() {
                    break;
                }
            });
        }
        Self { queue, rx, src }
    }
    /// Lower priority value = sooner.
    pub fn submit(&self, prio: f32, job: Job) {
        self.queue.0.lock().push((prio, job));
        self.queue.1.notify_one();
    }
}
