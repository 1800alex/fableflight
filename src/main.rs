mod airframe;
mod audio;
mod buildings;
mod cabin;
mod cars;
mod crash;
mod damage;
mod destruct;
mod fetch;
mod fluid;
mod geo;
mod impostor;
mod materials;
mod menu;
mod weather;
mod mvt;
mod plane;
mod render;
mod roads;
mod sources;
mod trees;
mod world;

#[derive(Copy, Clone, PartialEq)]
enum Mode {
    Menu,
    Fly,
}

/// What was chosen in the in-flight menu, carried out once the UI is drawn.
#[derive(Copy, Clone)]
enum PauseAct {
    Resume,
    Respawn,
    GoTo,
    FlyTo(f64, f64, &'static str),
    Globe,
    Quit,
}

use crash::Crash;
use damage::Tear;
use glam::{DMat4, DQuat, DVec2, DVec3, Mat4, Vec2, Vec3};
use plane::{Controls, Plane};
use render::{DrawObj, Frame, Globals, PSim, Renderer};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const SUN_E: f32 = 10.0;
/// The aircraft's own shadow map: half width (m), how far back toward the sun
/// it starts, and how deep it reaches.
const PSHADOW_R: f32 = 6.2;
const PSHADOW_BACK: f32 = 40.0;
const PSHADOW_DEPTH: f32 = 1600.0;
/// Moonlight relative to sunlight. Far brighter than the real ratio, so a
/// moonlit night reads like a film's night scene instead of a black screen.
const MOON_E: f32 = SUN_E * 0.0035;
const ATTRIBUTION: &str = "Imagery © Esri, Maxar, Earthstar Geographics  ·  Elevation: Mapzen / AWS Terrain Tiles  ·  Buildings, roads, land cover & water © OpenStreetMap contributors, OpenFreeMap";

struct Args {
    lat: f64,
    lon: f64,
    alt: Option<f64>,
    /// starting height above the ground, instead of `alt` above sea level
    agl: Option<f64>,
    heading: f64,
    place: Option<String>,
    max_zoom: u8,
    hour: f64,
    clouds: f32,
    /// cumulonimbus storm cell amount 0..1
    storms: Option<f32>,
    /// wind in knots and the direction it blows from, haze, day of year; open the F3 panel
    wind: Option<(f64, f64)>,
    haze: Option<f32>,
    day: Option<f64>,
    weather_panel: bool,
    vsync: bool,
    taa: bool,
    offline: bool,
    shots: Vec<(f64, PathBuf, Option<(f64, f64, f64)>)>,
    /// record frames + audio to a directory: (dir, start, seconds), fixed 30 fps steps
    record: Option<(PathBuf, f64, f64)>,
    crash_at: Option<f64>,
    size: (u32, u32),
    imagery: String,
    cam: Option<(f64, f64, f64)>,
    cockpit: bool,
    mute: bool,
    /// start on the globe menu (default unless a place or scripted run is given)
    menu: Option<bool>,
    menu_view: Option<(f64, f64, f64)>,
    menu_go: Option<f64>,
    /// start already damaged (see `Damage::apply_spec`), for trying the damage model out
    damage: Option<String>,
    /// open the in-flight menu on this page after this many seconds, for scripted screenshots
    pause_menu: Option<(menu::Page, f64)>,
}

fn parse_args() -> Args {
    let mut a = Args {
        lat: 37.8085,
        lon: -122.4650,
        alt: None,
        agl: None,
        heading: 95.0,
        place: None,
        max_zoom: 17,
        hour: 16.6,
        clouds: 0.5,
        storms: None,
        wind: None,
        haze: None,
        day: None,
        weather_panel: false,
        vsync: true,
        taa: true,
        offline: false,
        shots: Vec::new(),
        record: None,
        crash_at: None,
        size: (1600, 900),
        imagery: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}".into(),
        cam: None,
        cockpit: false,
        mute: false,
        menu: None,
        menu_view: None,
        menu_go: None,
        damage: None,
        pause_menu: None,
    };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let num = |v: &Vec<String>, i: usize| v.get(i + 1).and_then(|s| s.parse::<f64>().ok());
    while i < v.len() {
        let mut used = 2;
        match v[i].as_str() {
            "--lat" => {
                a.lat = num(&v, i).unwrap_or(a.lat);
                a.menu.get_or_insert(false);
            }
            "--lon" => {
                a.lon = num(&v, i).unwrap_or(a.lon);
                a.menu.get_or_insert(false);
            }
            "--menu-go" => a.menu_go = num(&v, i),
            "--menu-view" => {
                // lat,lon,altitude of the menu camera, for scripted screenshots
                let p: Vec<f64> = v.get(i + 1).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                if p.len() == 3 {
                    a.menu_view = Some((p[0], p[1], p[2]));
                }
            }
            "--menu" => {
                a.menu = Some(true);
                used = 1;
            }
            "--no-menu" => {
                a.menu = Some(false);
                used = 1;
            }
            "--cockpit" => {
                a.cockpit = true;
                used = 1;
            }
            "--alt" => a.alt = num(&v, i),
            "--agl" => a.agl = num(&v, i),
            "--heading" => a.heading = num(&v, i).unwrap_or(a.heading),
            "--place" => {
                a.place = v.get(i + 1).cloned();
                a.menu.get_or_insert(false);
            }
            "--max-zoom" => a.max_zoom = num(&v, i).unwrap_or(17.0).clamp(8.0, 19.0) as u8,
            "--time" => a.hour = num(&v, i).unwrap_or(a.hour),
            "--clouds" => a.clouds = num(&v, i).unwrap_or(0.5) as f32,
            "--storms" => a.storms = num(&v, i).map(|x| x as f32),
            "--wind" => {
                // knots,from-degrees
                let p: Vec<f64> = v.get(i + 1).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                if p.len() == 2 {
                    a.wind = Some((p[0], p[1]));
                }
            }
            "--haze" => a.haze = num(&v, i).map(|h| h as f32),
            "--day" => a.day = num(&v, i),
            "--weather" => {
                a.weather_panel = true;
                used = 1;
            }
            "--imagery" => a.imagery = v.get(i + 1).cloned().unwrap_or(a.imagery.clone()),
            "--crash-at" => a.crash_at = num(&v, i),
            "--damage" => a.damage = v.get(i + 1).cloned(),
            "--pause-menu" => {
                // page[@seconds]: open the Tab menu on that page, for scripted screenshots
                let f: Vec<&str> = v.get(i + 1).map(|s| s.split('@').collect()).unwrap_or_default();
                if let Some(page) = f.first().and_then(|p| menu::Page::parse(p)) {
                    a.pause_menu = Some((page, f.get(1).and_then(|t| t.parse().ok()).unwrap_or(0.0)));
                }
            }
            "--width" => a.size.0 = num(&v, i).unwrap_or(1600.0) as u32,
            "--height" => a.size.1 = num(&v, i).unwrap_or(900.0) as u32,
            "--cam" => {
                // yaw,pitch,distance of the chase camera, for scripted screenshots
                let p: Vec<f64> = v.get(i + 1).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                if p.len() == 3 {
                    a.cam = Some((p[0], p[1], p[2]));
                }
            }
            "--shot" => {
                // path@seconds[@yaw,pitch,distance]
                let f: Vec<&str> = v.get(i + 1).map(|s| s.split('@').collect()).unwrap_or_default();
                if f.len() >= 2 {
                    let c: Vec<f64> = f.get(2).map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                    a.shots.push((f[1].parse().unwrap_or(5.0), PathBuf::from(f[0]), if c.len() == 3 { Some((c[0], c[1], c[2])) } else { None }));
                }
                a.menu.get_or_insert(false);
            }
            "--record" => {
                // dir@start@seconds: every frame steps 1/30 s; PNG frames + audio.wav
                let f: Vec<&str> = v.get(i + 1).map(|s| s.split('@').collect()).unwrap_or_default();
                if f.len() == 3 {
                    a.record = Some((PathBuf::from(f[0]), f[1].parse().unwrap_or(0.0), f[2].parse().unwrap_or(10.0)));
                }
                a.menu.get_or_insert(false);
            }
            "--no-vsync" => {
                a.vsync = false;
                used = 1;
            }
            "--no-taa" => {
                a.taa = false;
                used = 1;
            }
            "--mute" => {
                a.mute = true;
                used = 1;
            }
            "--offline" => {
                a.offline = true;
                used = 1;
            }
            "--help" | "-h" => {
                println!("fablegame [--menu | --no-menu] [--place \"city or address\"] [--lat D --lon D] [--alt M | --agl M] [--heading DEG] [--time HOURS] [--day 0..364] [--clouds 0..1] [--storms 0..1] [--wind KT,FROM_DEG] [--haze 0.2..40] [--weather] [--pause-menu flight|weather|settings|controls[@SECONDS]] [--max-zoom 8..19] [--cockpit] [--no-vsync] [--no-taa] [--mute] [--offline] [--imagery URL_TEMPLATE] [--damage rwing=4.3,laileron=0,rudder=0.5,prop=0.5,engine=0.4,oil=0.6,crack=1,nosegear=0]");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument {other}");
                used = 1;
            }
        }
        i += used;
    }
    a.shots.sort_by(|x, y| x.0.total_cmp(&y.0));
    a
}

fn url_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Look a place name up with OpenStreetMap's Nominatim. Returns lat, lon (degrees) and a label.
fn geocode(f: &fetch::Fetcher, q: &str) -> Option<(f64, f64, String)> {
    let url = format!("https://nominatim.openstreetmap.org/search?format=json&limit=1&q={}", url_encode(q));
    let j: serde_json::Value = serde_json::from_slice(&f.get(&url).ok()?).ok()?;
    let r = j.get(0)?;
    Some((r["lat"].as_str()?.parse().ok()?, r["lon"].as_str()?.parse().ok()?, r["display_name"].as_str().unwrap_or(q).to_string()))
}

fn ground_elev(src: &sources::Sources, lat: f64, lon: f64) -> f64 {
    let id = geo::TileId::at(lat.to_radians(), lon.to_radians(), 11);
    let (u, v) = id.frac(lat.to_radians(), lon.to_radians());
    src.elev(id).map(|e| e.sample(u, v) as f64).unwrap_or(0.0).max(0.0)
}

struct Game {
    window: Arc<Window>,
    r: Renderer,
    world: world::World,
    src: Arc<sources::Sources>,
    model: airframe::Model,
    /// renderer mesh of each model piece, of each breakable group, of the two shard shapes
    piece_mesh: Vec<usize>,
    group_mesh: Vec<usize>,
    /// does the piece have openings cut in it
    piece_cut: Vec<bool>,
    plane: Plane,
    crashed: bool,
    crash: Option<Crash>,
    crash_real: f64,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    keys: HashSet<KeyCode>,
    drag: bool,
    last_cursor: Option<(f64, f64)>,
    cam_yaw: f64,
    cam_pitch: f64,
    cam_dist: f64,
    cam_mode: u8,
    cam_pos: DVec3,
    cam_dir: DVec3,
    cam_look: DVec3,
    orbit: f64,
    /// how far the crash camera rises to see past buildings in the way (a
    /// fraction of its distance), and when that was last checked
    cam_rise: f64,
    cam_clear_t: f64,
    /// the crash camera's bearing round what it watches
    cam_ang: Option<f64>,
    cam_hold: f64,
    utc: f64,
    doy: f64,
    /// clouds, haze, wind, clock speed (the F3 panel)
    wx: weather::Weather,
    time: f64,
    real: f64,
    last: Instant,
    acc: f64,
    wave_off: DVec2,
    prev_cam: DVec3,
    prev_vp: [[f32; 4]; 4],
    /// temporal anti-aliasing (F2)
    taa: bool,
    /// where the aircraft was drawn last frame, relative to that frame's camera
    prev_plane: Mat4,
    /// centre and half size of the aircraft in its body frame
    plane_box: [[f32; 4]; 2],
    frame: u64,
    fps: f64,
    paused: bool,
    show_help: bool,
    show_hud: bool,
    goto_open: bool,
    goto_text: String,
    goto_rx: Option<std::sync::mpsc::Receiver<Option<(f64, f64, f64, String)>>>,
    status: String,
    status_until: f64,
    shots: Vec<(f64, PathBuf, Option<(f64, f64, f64)>)>,
    crash_at: Option<f64>,
    gilrs: Option<gilrs::Gilrs>,
    pad: Controls,
    rng: crash::Rng,
    audio: Option<audio::Audio>,
    record: Option<(PathBuf, f64, f64)>,
    rec_audio: Vec<[f32; 2]>,
    rec_n: u32,
    rec_t0: Instant,
    /// sound on (U), and its volume (0..1)
    sound_on: bool,
    volume: f32,
    /// the in-flight menu (Tab): open, the page it shows, and a scripted opening
    pause_open: bool,
    pause_page: menu::Page,
    pause_at: Option<(menu::Page, f64)>,
    /// mouse-look speed (1 = default)
    look_sens: f64,
    /// the cockpit warnings sounding, and when each latched one last tripped
    /// (pull up, too low, sink rate, gear) so they do not flicker
    warn: audio::Warn,
    warn_t: [f64; 4],
    /// for the sound: slow-following load factor (jolts stand out from it),
    /// when the last jolt was, last frame's ground contact, vertical speed,
    /// gear and flaps, and metal scraping along the ground this frame
    g_slow: f64,
    last_bump: f64,
    was_ground: bool,
    prev_vs: f64,
    prev_gear: f64,
    prev_flaps: f64,
    quit: bool,
    mode: Mode,
    menu: menu::Menu,
    menu_go: Option<f64>,
    heading: f64,
    destr: destruct::Destruction,
    /// parts that have come off the aircraft and are falling
    sheds: damage::Sheds,
    /// origin of the particle system's local frame, and how far it moved this frame
    fx_anchor: DVec3,
    fx_shift: DVec3,
    smoke_acc: f32,
    /// things that just broke, to flash on the HUD: text, when
    flash: Vec<(String, f64)>,
    damage_spec: Option<String>,
    /// group wreck meshes have been cut down and need restoring on respawn
    wreck_dirty: bool,
    start_rx: Option<std::sync::mpsc::Receiver<(f64, f64, f64)>>,
    name_rx: Option<std::sync::mpsc::Receiver<String>>,
    last_click: f64,
    n_cars: usize,
    /// how much city is around (0..1), for the orange skyglow at night
    city_glow: f32,
}

impl Game {
    fn new(window: Arc<Window>, args: &Args) -> anyhow::Result<Self> {
        let r = pollster::block_on(Renderer::new(window.clone(), args.vsync))?;
        let cache = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("fablegame");
        let mut r = r;
        // Cached scans load offline too; missing ones keep the procedural layer.
        r.load_ground_photos(fetch::Fetcher::new(cache.clone(), args.offline));
        let src = Arc::new(sources::Sources::new(fetch::Fetcher::new(cache, args.offline), args.imagery.clone()));
        let (mut lat, mut lon, mut status) = (args.lat, args.lon, String::new());
        if let Some(p) = &args.place {
            match geocode(&src.fetch, p) {
                Some((la, lo, name)) => {
                    (lat, lon) = (la, lo);
                    status = name;
                }
                None => status = format!("Could not find \"{p}\""),
            }
        }
        let alt = args.alt.unwrap_or_else(|| ground_elev(&src, lat, lon) + args.agl.unwrap_or(450.0));
        let mut plane = Plane::spawn(lat.to_radians(), lon.to_radians(), alt, args.heading.to_radians(), 68.0);
        if let Some(spec) = &args.damage {
            plane.dmg.apply_spec(spec);
        }
        let mut world = world::World::new(sources::Loader::new(src.clone(), 14), args.max_zoom);
        let mut r = r;
        let t0 = Instant::now();
        let model = airframe::build();
        log::info!("aircraft model: {} pieces, {} vertices, built in {:.0} ms", model.pieces.len(), model.pieces.iter().map(|p| p.mesh.v.len()).sum::<usize>(), t0.elapsed().as_secs_f64() * 1000.0);
        let piece_mesh: Vec<usize> = model.pieces.iter().map(|p| r.add_mesh(&p.mesh.v, &p.mesh.i)).collect();
        let piece_cut: Vec<bool> = model.pieces.iter().map(|p| p.mesh.v.iter().any(|v| airframe::cut_by(v.mat[1]))).collect();
        let mut group_mesh: Vec<usize> = model.groups.iter().map(|m| r.add_mesh(&m.v, &m.i)).collect();
        for m in crash::shards() {
            group_mesh.push(r.add_mesh(&m.v, &m.i));
        }
        let tm = trees::build_meshes();
        r.set_tree_meshes(&tm.meshes);
        world.tree_dims = tm.dims;
        let mut solids = cars::build_models();
        solids.push(cars::cube());
        solids.extend((0..3).map(|k| cars::rock(k * 17 + 3)));
        solids.extend(destruct::slab_meshes());
        solids.extend(destruct::rebar_meshes());
        r.set_inst_meshes(&solids);
        let mut menu = menu::Menu::new(lat, lon, args.hour);
        if let Some((la, lo, al)) = args.menu_view {
            menu.look_at(la, lo, al);
            menu.alt = al;
        }
        let mut wx = weather::Weather::new(args.clouds);
        if let Some(s) = args.storms {
            wx.storms = s.clamp(0.0, 1.0);
        }
        if let Some((kt, from)) = args.wind {
            (wx.wind, wx.wind_from) = (kt / 1.94384, from);
        }
        if let Some(h) = args.haze {
            wx.haze = h.clamp(0.2, 40.0);
        }
        wx.open = args.weather_panel;
        let egui_ctx = egui::Context::default();
        load_fonts(&egui_ctx);
        menu::apply_theme(&egui_ctx);
        let egui_state = egui_winit::State::new(egui_ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(4096));
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        let doy = (secs / 86400.0) % 365.25;
        let cam_pos = plane.pos - plane.forward() * 16.0;
        // What the anti-aliasing takes to be flying along with the camera:
        // everything within the aircraft's bounds, and a little around them
        // for the gear, flaps and control surfaces as they move.
        let plane_box = {
            let (lo, hi) = model.pieces.iter().flat_map(|p| p.mesh.v.iter()).fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| (lo.min(v.pos.into()), hi.max(v.pos.into())));
            [((lo + hi) * 0.5).extend(0.0).to_array(), ((hi - lo) * 0.5 + 0.25).extend(0.0).to_array()]
        };
        let (cy, cp, cd) = args.cam.unwrap_or((0.0, 0.2, 16.0));
        Ok(Self {
            window, r, world, src, model, piece_mesh, group_mesh, piece_cut, crashed: false, crash: None, crash_real: 0.0, egui_ctx, egui_state, keys: HashSet::new(), drag: false, last_cursor: None,
            cam_yaw: cy.to_radians(), cam_pitch: cp, cam_dist: cd, cam_mode: if args.cockpit { 1 } else { 0 }, cam_pos, cam_dir: -plane.forward(), cam_look: plane.pos, orbit: 0.0, cam_rise: 0.0, cam_clear_t: 0.0, cam_ang: None, cam_hold: 0.0,
            utc: (args.hour - lon / 15.0).rem_euclid(24.0), doy: args.day.unwrap_or(doy), wx, time: 0.0, real: 0.0, last: Instant::now(), acc: 0.0, wave_off: DVec2::ZERO, prev_cam: cam_pos, prev_vp: [[0.0; 4]; 4], taa: args.taa, prev_plane: Mat4::IDENTITY, plane_box,
            frame: 0, fps: 60.0, paused: false, show_help: false, show_hud: true, goto_open: false, goto_text: String::new(), goto_rx: None,
            status, status_until: 8.0, shots: args.shots.clone(), crash_at: args.crash_at, gilrs: gilrs::Gilrs::new().ok(), pad: Controls::default(), rng: crash::Rng(secs.to_bits() | 1),
            audio: if args.record.is_some() { Some(audio::Audio::offline()) } else if args.mute { None } else { audio::Audio::new() },
            record: args.record.clone(), rec_audio: Vec::new(), rec_n: 0, rec_t0: Instant::now(), sound_on: true, volume: 1.0, pause_open: false, pause_page: menu::Page::Flight, pause_at: args.pause_menu, look_sens: 1.0, warn: audio::Warn::default(), warn_t: [-10.0; 4],
            g_slow: 1.0, last_bump: 0.0, was_ground: false, prev_vs: 0.0, prev_gear: plane.gear, prev_flaps: plane.flaps, quit: false,
            mode: if args.menu.unwrap_or(true) { Mode::Menu } else { Mode::Fly }, menu, menu_go: args.menu_go, heading: args.heading.to_radians(), destr: destruct::Destruction::new(secs.to_bits()),
            sheds: damage::Sheds::default(), fx_anchor: plane.pos, fx_shift: DVec3::ZERO, smoke_acc: 0.0, flash: Vec::new(), damage_spec: args.damage.clone(), wreck_dirty: false,
            start_rx: None, name_rx: None, last_click: -10.0, n_cars: 0, city_glow: 0.0, plane,
        })
    }

    /// Readings for the cockpit instruments and the state of the lights (see plane.wgsl).
    fn instruments(&self, agl: f64) -> [[f32; 4]; 6] {
        let p = &self.plane;
        let (_, lon, h) = geo::pos_to_lla(p.pos);
        let (e, n, u) = geo::enu(p.pos);
        let f = p.forward();
        let hdg = f.dot(e).atan2(f.dot(n)).to_degrees().rem_euclid(360.0);
        let pitch = f.dot(u).clamp(-1.0, 1.0).asin().to_degrees();
        let bank = (-p.right().dot(u)).atan2(p.up().dot(u)).to_degrees();
        let turn = -(p.rot * p.omega).dot(u).to_degrees();
        let air = p.airspeed();
        let v = air.length().max(1.0);
        let slip = (p.rot.inverse() * air).x / v * -6.0 + (p.rot * p.omega).dot(u) * 0.0;
        let thr = p.throttle;
        let rho = (-h.max(0.0) / 8500.0).exp();
        let local = (self.utc + lon.to_degrees() / 15.0).rem_euclid(24.0) * 3600.0;
        let t = self.time;
        let strobe = { let s = t.rem_euclid(1.3); if s < 0.045 || (s > 0.16 && s < 0.2) { 1.0 } else { 0.0 } };
        let beacon = (1.0 - ((t * 1.1).rem_euclid(1.0) - 0.15).abs() * 7.0).max(0.0);
        let _ = agl;
        [
            [(air.length() * 1.94384) as f32, (h * 3.28084) as f32, (p.vel.dot(u) * 196.85) as f32, hdg as f32],
            [pitch as f32, bank as f32, turn as f32, slip.clamp(-1.0, 1.0) as f32],
            [(p.prop_rate * 60.0 / std::f64::consts::TAU) as f32, (8.0 + 21.0 * thr * rho + 4.0 * (1.0 - thr)) as f32, (21.0 - t / 900.0).max(1.0) as f32, (19.5 - t / 950.0).max(1.0) as f32],
            [(175.0 + 25.0 * thr + 90.0 * (1.0 - p.dmg.engine as f64)) as f32, ((68.0 + 14.0 * thr) * (p.dmg.engine as f64).sqrt()) as f32, (0.45 + 0.3 * thr) as f32, (3.0 + 2.0 * (t * 0.7).sin()) as f32],
            [5.0, local as f32, p.gear as f32, p.flap_deg() as f32],
            [1.0, strobe, beacon as f32, if p.gear_down { 1.0 } else { 0.0 }],
        ]
    }

    fn sun_dir(&self) -> DVec3 {
        let decl = (-23.44f64).to_radians() * (std::f64::consts::TAU * (self.doy + 10.0) / 365.25).cos();
        let lon = (-15.0 * (self.utc - 12.0)).to_radians();
        DVec3::new(decl.cos() * lon.cos(), decl.cos() * lon.sin(), decl.sin())
    }

    /// Direction to the moon: the sun's ecliptic longitude advanced by the
    /// moon's elongation, carried into the same Earth-fixed frame as the sun.
    fn moon_dir(&self) -> (DVec3, f64) {
        use std::f64::consts::{FRAC_PI_2, TAU};
        let eps = 23.44f64.to_radians();
        let radec = |lam: f64| ((eps.cos() * lam.sin()).atan2(lam.cos()), (eps.sin() * lam.sin()).asin());
        let lam_s = TAU * (self.doy + 10.0) / 365.25 - FRAC_PI_2;
        let elong = TAU * self.wx.moon_age / 29.530588;
        let (ra_s, _) = radec(lam_s);
        let (ra_m, dec_m) = radec(lam_s + elong);
        let lon = (-15.0 * (self.utc - 12.0)).to_radians() + ra_m - ra_s;
        let lit = 0.5 * (1.0 - elong.cos());
        (DVec3::new(dec_m.cos() * lon.cos(), dec_m.cos() * lon.sin(), dec_m.sin()), lit)
    }

    fn respawn(&mut self, lat: f64, lon: f64, alt: f64, heading: f64) {
        self.plane = Plane::spawn(lat, lon, alt, heading, 68.0);
        // 68 m/s through the air, not over the ground.
        self.plane.wind = self.wx.steady(self.plane.pos, 300.0);
        self.plane.vel += self.plane.wind;
        if let Some(spec) = &self.damage_spec {
            self.plane.dmg.apply_spec(spec);
        }
        self.restore_wreck_meshes();
        self.flash.clear();
        self.crashed = false;
        self.cam_rise = 0.0;
        self.cam_ang = None;
        self.acc = 0.0;
        self.cam_pos = self.plane.pos - self.plane.forward() * 16.0;
        self.cam_dir = -self.plane.forward();
        self.prev_cam = self.cam_pos;
        self.warn_t = [-10.0; 4];
        (self.g_slow, self.was_ground, self.prev_vs, self.prev_gear, self.prev_flaps) = (1.0, false, 0.0, self.plane.gear, self.plane.flaps);
    }

    fn controls(&mut self) -> Controls {
        let k = |c: KeyCode| self.keys.contains(&c);
        let ax = |pos: bool, neg: bool| (pos as i32 - neg as i32) as f64;
        let mut c = Controls {
            pitch: ax(k(KeyCode::KeyS) || k(KeyCode::ArrowDown), k(KeyCode::KeyW) || k(KeyCode::ArrowUp)),
            roll: ax(k(KeyCode::KeyD) || k(KeyCode::ArrowRight), k(KeyCode::KeyA) || k(KeyCode::ArrowLeft)),
            yaw: ax(k(KeyCode::KeyE), k(KeyCode::KeyQ)),
            throttle: ax(k(KeyCode::ShiftLeft) || k(KeyCode::ShiftRight), k(KeyCode::ControlLeft) || k(KeyCode::ControlRight)),
            brake: k(KeyCode::KeyB),
            boost: k(KeyCode::Space),
        };
        // Gamepad: d-pad down / up moves the flaps a notch, the top face button works the gear.
        let (mut flaps, mut gear, mut start) = (0, false, false);
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                if let gilrs::EventType::ButtonPressed(b, _) = ev.event {
                    match b {
                        gilrs::Button::DPadDown => flaps += 1,
                        gilrs::Button::DPadUp => flaps -= 1,
                        gilrs::Button::North => gear = !gear,
                        gilrs::Button::Start => start = true,
                        _ => {}
                    }
                }
            }
            if let Some((_, pad)) = g.gamepads().next() {
                use gilrs::{Axis, Button};
                let dz = |v: f32| if v.abs() < 0.12 { 0.0 } else { v as f64 };
                self.pad = Controls {
                    pitch: -dz(pad.value(Axis::LeftStickY)),
                    roll: dz(pad.value(Axis::LeftStickX)),
                    yaw: dz(pad.value(Axis::RightStickX)),
                    throttle: pad.button_data(Button::RightTrigger2).map(|b| b.value() as f64).unwrap_or(0.0) - pad.button_data(Button::LeftTrigger2).map(|b| b.value() as f64).unwrap_or(0.0),
                    brake: pad.is_pressed(Button::South),
                    boost: pad.is_pressed(Button::East),
                };
            }
            c.pitch = (c.pitch + self.pad.pitch).clamp(-1.0, 1.0);
            c.roll = (c.roll + self.pad.roll).clamp(-1.0, 1.0);
            c.yaw = (c.yaw + self.pad.yaw).clamp(-1.0, 1.0);
            c.throttle = (c.throttle + self.pad.throttle).clamp(-1.0, 1.0);
            c.brake |= self.pad.brake;
            c.boost |= self.pad.boost;
        }
        if start {
            match self.mode {
                Mode::Fly => self.set_pause_menu(!self.pause_open),
                Mode::Menu => self.begin(),
            }
        }
        if !self.crashed && self.mode == Mode::Fly && !self.pause_open {
            if flaps != 0 {
                self.plane.set_flaps(flaps);
            }
            if gear {
                self.plane.toggle_gear();
            }
        }
        if let Some(t) = self.crash_at {
            if self.real > t && !self.crashed {
                c.pitch = -1.0;
                c.throttle = 1.0;
            }
        }
        c
    }

    fn key(&mut self, code: KeyCode) {
        if self.mode == Mode::Menu {
            match code {
                KeyCode::Escape if self.menu.show_controls => self.menu.show_controls = false,
                KeyCode::Escape => self.quit = true,
                KeyCode::Enter | KeyCode::NumpadEnter => self.begin(),
                KeyCode::F12 => self.shots.insert(0, (0.0, PathBuf::from(format!("screenshot-{}.png", self.frame)), None)),
                _ => {}
            }
            return;
        }
        if self.pause_open {
            // The menu has the keyboard (and the game is paused) until it closes.
            match code {
                KeyCode::Tab | KeyCode::Escape => self.set_pause_menu(false),
                KeyCode::F12 => self.shots.insert(0, (0.0, PathBuf::from(format!("screenshot-{}.png", self.frame)), None)),
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Tab => self.set_pause_menu(true),
            // Esc closes what is open first; with nothing open it goes back to the globe.
            KeyCode::Escape if self.show_help => self.show_help = false,
            KeyCode::Escape if self.wx.open => self.wx.open = false,
            KeyCode::Escape => self.to_globe(),
            KeyCode::KeyR => self.respawn_here(),
            KeyCode::KeyC => self.cam_mode = (self.cam_mode + 1) % 2,
            KeyCode::KeyF if !self.crashed => self.plane.set_flaps(1),
            KeyCode::KeyV if !self.crashed => self.plane.set_flaps(-1),
            KeyCode::KeyL if !self.crashed => self.plane.toggle_gear(),
            KeyCode::KeyG => {
                self.goto_open = true;
                self.goto_text.clear();
            }
            KeyCode::KeyT => self.utc = (self.utc + 0.5).rem_euclid(24.0),
            KeyCode::KeyY => self.utc = (self.utc - 0.5).rem_euclid(24.0),
            KeyCode::KeyN => self.wx.clouds = (self.wx.clouds + 0.1).min(1.0),
            KeyCode::KeyM => self.wx.clouds = (self.wx.clouds - 0.1).max(0.0),
            KeyCode::F3 => self.wx.open = !self.wx.open,
            KeyCode::KeyP => self.paused = !self.paused,
            KeyCode::KeyH => self.show_hud = !self.show_hud,
            KeyCode::KeyU => {
                self.sound_on = !self.sound_on;
                self.status = if self.sound_on { "Sound on" } else { "Sound off" }.into();
                self.status_until = self.real + 2.0;
            }
            KeyCode::F1 => self.show_help = !self.show_help,
            KeyCode::F2 => self.taa = !self.taa,
            KeyCode::F12 => {
                let p = PathBuf::from(format!("screenshot-{}.png", self.frame));
                self.shots.insert(0, (0.0, p, None));
            }
            _ => {}
        }
    }

    /// Open or close the in-flight menu. The game is paused while it is open.
    fn set_pause_menu(&mut self, open: bool) {
        if open == self.pause_open || self.mode != Mode::Fly {
            return;
        }
        self.pause_open = open;
        self.drag = false;
        self.show_help = false;
        // A button or slider left focused would keep the keys from the aircraft.
        self.egui_ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    /// Back to the globe, looking at where we were.
    fn to_globe(&mut self) {
        self.set_pause_menu(false);
        let (lat, lon, _) = geo::pos_to_lla(self.plane.pos);
        self.menu.look_at(lat.to_degrees(), lon.to_degrees(), 60_000.0);
        self.menu.pick = Some((lat.to_degrees(), lon.to_degrees()));
        self.menu.hour = (self.utc + lon.to_degrees() / 15.0).rem_euclid(24.0);
        self.menu.starting = false;
        self.mode = Mode::Menu;
    }

    /// A fresh aircraft in the air over where we are (or where we crashed), on the same heading.
    fn respawn_here(&mut self) {
        let (lat, lon, h) = geo::pos_to_lla(if self.crashed { self.crash.as_ref().map(|c| c.site).unwrap_or(self.plane.pos) } else { self.plane.pos });
        let g = self.world.ground(lat, lon).map(|g| g.h).unwrap_or(h);
        let hd = self.plane.forward().dot(geo::enu(self.plane.pos).0).atan2(self.plane.forward().dot(geo::enu(self.plane.pos).1));
        self.respawn(lat, lon, g + 420.0, hd);
    }

    /// Fly from a known spot: its ground height is looked up in the background,
    /// then the aircraft appears above it (see `goto_rx`).
    fn fly_to(&mut self, lat: f64, lon: f64, name: &str) {
        self.status = format!("Flying to {name}…");
        self.status_until = self.real + 30.0;
        let (tx, rx) = std::sync::mpsc::channel();
        let src = self.src.clone();
        let name = name.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(Some((lat, lon, ground_elev(&src, lat, lon), name)));
        });
        self.goto_rx = Some(rx);
    }

    /// Input for the UI, minus Tab: that opens and closes the menu here, not
    /// egui's keyboard focus (which would then keep keys from the aircraft).
    fn egui_input(&mut self) -> egui::RawInput {
        let mut raw = self.egui_state.take_egui_input(&self.window);
        if !self.egui_ctx.text_edit_focused() {
            raw.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Tab, .. }));
        }
        raw
    }

    /// Menu: choose a spot on the globe and look its name up in the background.
    fn pick(&mut self, lat: f64, lon: f64) {
        self.menu.pick = Some((lat, lon));
        self.menu.pick_name = match menu::Menu::near_city(lat, lon) {
            Some((name, d)) if d < 40_000.0 => name.to_string(),
            Some((name, d)) if d < 400_000.0 => format!("{:.0} km from {name}", d / 1000.0),
            _ => String::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let src = self.src.clone();
        std::thread::spawn(move || {
            let url = format!("https://nominatim.openstreetmap.org/reverse?format=json&zoom=12&lat={lat:.5}&lon={lon:.5}");
            let name = src.fetch.get(&url).ok().and_then(|d| serde_json::from_slice::<serde_json::Value>(&d).ok()).and_then(|j| j["display_name"].as_str().map(String::from));
            if let Some(n) = name {
                let _ = tx.send(n);
            }
        });
        self.name_rx = Some(rx);
    }

    /// Menu: take off at the chosen spot (once its ground height is known).
    fn begin(&mut self) {
        let Some((lat, lon)) = self.menu.pick else { return };
        if self.menu.starting {
            return;
        }
        self.menu.starting = true;
        let (tx, rx) = std::sync::mpsc::channel();
        let src = self.src.clone();
        std::thread::spawn(move || {
            let _ = tx.send((lat, lon, ground_elev(&src, lat, lon)));
        });
        self.start_rx = Some(rx);
    }

    /// Particles live in a frame anchored near the action; move it if the action has moved on.
    fn anchor_fx(&mut self, at: DVec3) {
        let stale = self.crash.as_ref().map(|c| c.time > 90.0).unwrap_or(true);
        if stale && (self.fx_anchor - at).length() > 800.0 {
            // Live particles are carried over into the new frame.
            self.fx_shift += self.fx_anchor - at;
            self.crash = None;
            self.fx_anchor = at;
        }
    }

    /// Knock a building down, with any parts standing on it.
    /// `hit`: the aircraft itself went in (the wall shows its outline).
    fn bring_down(&mut self, tile: geo::TileId, idx: usize, at: DVec3, vel: DVec3, hit: Option<destruct::Hitter>, femit: &mut Vec<(DVec3, render::Particle)>) {
        let above = self.world.structure_of(tile, idx);
        let mut parts = Vec::new();
        for (t, i) in std::iter::once((tile, idx)).chain(above) {
            if let Some(w) = self.world.destroy_building(&self.r.queue, t, i) {
                parts.push(w);
            }
        }
        if !parts.is_empty() {
            self.destr.collapse(&parts, at, vel, hit, femit);
        }
    }

    /// The wreck is made of what was left of the aircraft: every group's mesh
    /// is rebuilt without the pieces that had already gone, and cut along its
    /// breaks. Returns the break each group keeps showing.
    fn wreck_meshes(&mut self) -> Vec<[Tear; 2]> {
        let dm = self.plane.dmg;
        let mut out = vec![[Tear::NONE; 2]; airframe::GROUPS.len()];
        for g in 0..airframe::GROUPS.len() {
            let pieces: Vec<&airframe::Piece> = self.model.pieces.iter().filter(|p| p.group == g && !p.glass).collect();
            let changed = pieces.iter().any(|p| dm.hidden(p) || dm.tears_for(p).iter().any(|t| t.is_some()));
            if !changed {
                continue;
            }
            self.wreck_dirty = true;
            let mut m = airframe::Mesh::default();
            for p in &pieces {
                if dm.hidden(p) {
                    continue;
                }
                let tears = dm.tears_for(p);
                let primary = matches!(p.name, "left wing" | "right wing" | "stabiliser" | "blades");
                if primary {
                    out[g] = tears;
                }
                let base = m.v.len() as u32;
                m.v.extend_from_slice(&p.mesh.v);
                for t in p.mesh.i.chunks(3) {
                    let gone = t.iter().all(|&i| { let q = Vec3::from(p.mesh.v[i as usize].pos); tears.iter().any(|tr| !tr.keeps(q)) });
                    if !gone {
                        m.i.extend(t.iter().map(|i| i + base));
                    }
                }
            }
            if m.i.is_empty() {
                m = self.model.groups[g].clone();
            }
            self.r.replace_mesh(self.group_mesh[g], &m.v, &m.i);
        }
        out
    }
    fn restore_wreck_meshes(&mut self) {
        if self.wreck_dirty {
            for g in 0..airframe::GROUPS.len() {
                let m = &self.model.groups[g];
                self.r.replace_mesh(self.group_mesh[g], &m.v, &m.i);
            }
            self.wreck_dirty = false;
        }
    }

    /// The cockpit warnings: stall, terrain ahead, nose dive, too low, sink
    /// rate, overspeed and gear up.
    fn warnings(&mut self) -> audio::Warn {
        let p = &self.plane;
        let mut w = audio::Warn::default();
        if self.crashed || self.mode == Mode::Menu || p.on_ground {
            return w;
        }
        let up = p.pos.normalize();
        let (la, lo, h) = geo::pos_to_lla(p.pos);
        let Some(g) = self.world.ground(la, lo) else { return w };
        let agl = h - g.h;
        if agl < 3.0 {
            return w;
        }
        let vs = p.vel.dot(up);
        let air = p.airspeed().length();
        let pitch = p.forward().dot(up).clamp(-1.0, 1.0).asin().to_degrees();
        if air > 8.0 {
            let sa = p.stall_alpha();
            w.stall = smoothstep(sa - 0.07, sa - 0.01, p.alpha.abs()) as f32;
        }
        if air > plane::VNE {
            w.overspeed = ((air - plane::VNE) / 13.0).clamp(0.1, 1.0) as f32;
        }
        // Going down nose first, steeply and fast.
        if vs < -10.0 {
            w.dive = smoothstep(-20.0, -55.0, pitch) as f32;
            if pitch < -22.0 || vs < -40.0 {
                w.dive = w.dive.max(0.2);
            }
        }
        // What is along the flight path in the next few seconds: ground, and
        // any building standing on it.
        let (mut tti, mut tower) = (f64::INFINITY, false);
        if p.vel.length() > 15.0 && agl < 1500.0 {
            for t in [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0] {
                let (la, lo, h) = geo::pos_to_lla(p.pos + p.vel * t);
                let Some(g) = self.world.ground(la, lo) else { continue };
                let b = if h - g.h < 400.0 { self.world.building(la, lo).filter(|b| !b.bridge && h < b.top + 3.0 && h > b.base - 3.0) } else { None };
                if b.is_some() || h < g.h + 3.0 {
                    (tti, tower) = (t, b.is_some());
                    break;
                }
            }
        }
        // A gentle descent with the gear down is a landing, not a warning
        // (unless it is into a building).
        let landing = p.gear >= 1.0 && vs > -5.0 && !tower;
        let closing = if vs < -1.0 { agl / -vs } else { f64::INFINITY };
        let now = self.real;
        let trip = [
            !landing && (tti <= 5.0 || closing < 5.0),
            !landing && (tti <= 10.0 || (agl < 46.0 && vs < -1.5 && p.gear < 1.0)),
            !landing && agl < 600.0 && vs < -12.0,
            p.gear < 1.0 && agl < 150.0 && p.throttle < 0.35,
        ];
        for (t, on) in self.warn_t.iter_mut().zip(trip) {
            if on {
                *t = now;
            }
        }
        let held = |i: usize| now - self.warn_t[i] < 0.6;
        (w.pull_up, w.too_low, w.sink, w.gear) = (held(0), held(1), held(2), held(3));
        w
    }

    /// Tell the sound what the aircraft and the camera are doing.
    fn update_audio(&mut self, cam: DVec3, fire: f32, in_menu: bool, c: &Controls, dt: f64, scrape: f32) {
        self.warn = if dt > 0.0 { self.warnings() } else { audio::Warn::default() };
        let live = !self.crashed && !in_menu && dt > 0.0;
        let p = &self.plane;
        let up = p.pos.normalize();
        let vs = p.vel.dot(up);
        let air = p.airspeed().length();
        // Jolts: quick changes in load stand out from a slow-following copy.
        let dg = p.g_load - self.g_slow;
        self.g_slow += dg * (dt * 8.0).min(1.0);
        let jolt = if live && !p.on_ground { dg.abs() } else { 0.0 };
        let mut events = Vec::new();
        if live {
            if jolt > 0.35 && self.real - self.last_bump > 0.15 {
                self.last_bump = self.real;
                events.push(audio::Event::Bump(((jolt - 0.2) * 1.5) as f32));
            }
            if p.on_ground && !self.was_ground {
                events.push(audio::Event::Touch((-self.prev_vs).max(0.5) as f32));
            }
            if (p.gear - self.prev_gear).abs() < 0.1 {
                if p.gear >= 1.0 && self.prev_gear < 1.0 {
                    events.push(audio::Event::Clunk(1.0));
                } else if p.gear <= 0.0 && self.prev_gear > 0.0 {
                    events.push(audio::Event::Clunk(0.8));
                } else if (self.prev_gear >= 1.0 || self.prev_gear <= 0.0) && p.gear > 0.0 && p.gear < 1.0 {
                    events.push(audio::Event::Clunk(0.4));
                }
            }
        }
        let gear_moving = live && p.gear > 0.0 && p.gear < 1.0 && p.gear != self.prev_gear;
        let flaps_moving = live && p.flaps != self.prev_flaps;
        (self.was_ground, self.prev_vs, self.prev_gear, self.prev_flaps) = (p.on_ground, vs, p.gear, p.flaps);
        let sa = p.stall_alpha();
        let ground_speed = (p.vel - up * vs).length();
        let params = audio::Params {
            live: if live { 1.0 } else { 0.0 },
            master: if self.sound_on { self.volume } else { 0.0 },
            rpm: (p.prop_rate * 60.0 / std::f64::consts::TAU) as f32,
            power: p.power as f32,
            throttle: p.throttle as f32,
            health: p.dmg.engine,
            prop: p.dmg.prop,
            airspeed: air as f32,
            g_load: p.g_load as f32,
            rough: p.air.sigma_w as f32,
            jolt: jolt as f32,
            buffet: if p.on_ground { 0.0 } else { (smoothstep(sa - 0.07, sa + 0.03, p.alpha.abs()) * smoothstep(10.0, 35.0, air)) as f32 },
            flutter: p.dmg.flutter,
            gear: p.gear as f32,
            gear_moving: if gear_moving { 1.0 } else { 0.0 },
            flaps: p.flaps as f32,
            flaps_moving: if flaps_moving { 1.0 } else { 0.0 },
            rolling: if p.on_ground { ground_speed as f32 } else { 0.0 },
            braking: if c.brake && p.on_ground { 1.0 } else { 0.0 },
            scrape,
            foliage: p.brushes.min(1.0) as f32,
            cracked: if p.dmg.crack[2] > 0.0 { 1.0 } else { 0.0 },
            cabin: if self.cam_mode == 1 && !self.crashed && !in_menu { 1.0 } else { 0.0 },
            dist: (cam - p.pos).length() as f32,
            fire,
            warn: self.warn,
        };
        if let Some(a) = &self.audio {
            for e in events {
                a.event(e);
            }
            a.set(params);
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let mut dt_real = (now - self.last).as_secs_f64().min(0.05);
        if self.record.is_some() {
            // Frozen for a few wall-clock seconds so the city streams in first.
            dt_real = if self.rec_t0.elapsed().as_secs_f64() < 8.0 { 0.0 } else { 1.0 / REC_FPS };
        }
        self.last = now;
        self.real += dt_real;
        self.frame += 1;
        if self.record.is_none() {
            self.fps += (1.0 / dt_real.max(1e-4) - self.fps) * 0.05;
        }

        // Results of background lookups.
        if let Some(rx) = &self.goto_rx {
            if let Ok(res) = rx.try_recv() {
                self.goto_rx = None;
                match res {
                    Some((lat, lon, elev, name)) if self.mode == Mode::Menu => {
                        self.menu.look_at(lat, lon, 45_000.0);
                        self.menu.pick = Some((lat, lon));
                        self.menu.pick_name = name;
                        let _ = elev;
                    }
                    Some((lat, lon, elev, name)) => {
                        let local = (self.utc + geo::pos_to_lla(self.plane.pos).1.to_degrees() / 15.0).rem_euclid(24.0);
                        self.utc = (local - lon / 15.0).rem_euclid(24.0);
                        self.respawn(lat.to_radians(), lon.to_radians(), elev + 450.0, 0.0);
                        self.status = name;
                    }
                    None => {
                        self.status = "Place not found".into();
                        self.menu.pick_name = "Place not found".into();
                    }
                }
                self.status_until = self.real + 8.0;
            }
        }
        if let Some(rx) = &self.name_rx {
            if let Ok(name) = rx.try_recv() {
                self.menu.pick_name = name;
                self.name_rx = None;
            }
        }
        if let Some(rx) = &self.start_rx {
            if let Ok((lat, lon, elev)) = rx.try_recv() {
                self.start_rx = None;
                self.utc = (self.menu.hour - lon / 15.0).rem_euclid(24.0);
                self.respawn(lat.to_radians(), lon.to_radians(), elev + 450.0, self.heading);
                self.status = self.menu.pick_name.clone();
                self.status_until = self.real + 10.0;
                self.mode = Mode::Fly;
                self.menu.starting = false;
            }
        }
        if let Some(t) = self.menu_go {
            if self.real > t && self.mode == Mode::Menu && !self.menu.starting {
                self.pick(self.menu.lat, self.menu.lon);
                self.begin();
                self.menu_go = None;
            }
        }
        let in_menu = self.mode == Mode::Menu;

        if let Some((page, t)) = self.pause_at {
            if self.real >= t && self.mode == Mode::Fly {
                self.pause_page = page;
                self.set_pause_menu(true);
                self.pause_at = None;
            }
        }
        let scale = if self.paused || in_menu || self.pause_open { 0.0 } else { 1.0 };
        let dt = dt_real * scale;
        self.time += dt;
        if !in_menu {
            // The clock runs at the panel's rate; past midnight the date turns over.
            let t = self.utc + dt * self.wx.time_rate / 3600.0;
            self.doy = (self.doy + (t / 24.0).floor()).rem_euclid(365.0);
            self.utc = t.rem_euclid(24.0);
            // The air the aircraft flies through, gusts and all.
            let (la, lo, h) = geo::pos_to_lla(self.plane.pos);
            let g = self.world.ground(la, lo);
            let agl = h - g.as_ref().map(|g| g.h).unwrap_or(0.0);
            let water = g.map(|g| g.water).unwrap_or(false);
            self.wx.sun = self.sun_dir().dot(self.plane.pos.normalize());
            // Turbulence, thermals and storm drafts ride on top of the gusting wind.
            let air = self.wx.air(self.plane.pos, agl, self.time, water);
            self.plane.air = air;
            self.plane.wind = self.wx.at(self.plane.pos, agl, self.time) + self.plane.pos.normalize() * air.draft;
            self.wx.advance(dt, self.plane.pos);
        }

        // Physics at a fixed step.
        let c = self.controls();
        let mut emit = Vec::new();
        let mut femit: Vec<(DVec3, render::Particle)> = Vec::new();
        let (mut scrape_n, mut steps) = (0usize, 0usize);
        if !self.crashed && !in_menu {
            self.acc += dt;
            let h = 1.0 / 180.0;
            while self.acc >= h {
                self.acc -= h;
                let imp = self.plane.step(h, &c, &self.world);
                steps += 1;
                scrape_n += self.plane.sparks.len();
                // Anything the aircraft ploughed through goes over in its direction of travel.
                if !self.plane.tree_hits.is_empty() {
                    let hits: Vec<_> = self.plane.tree_hits.drain(..).collect();
                    self.anchor_fx(hits[0].pos);
                    for t in hits {
                        self.destr.knock(&mut self.world, &self.r.queue, &t, self.plane.vel, 1.3, 0.0, 0.0);
                    }
                    if let Some(a) = &self.audio {
                        a.event(audio::Event::Crunch((self.plane.vel.length() / 50.0).clamp(0.4, 1.6) as f32));
                    }
                }
                // Parts that came off: let go of them where the aircraft is now.
                if !self.plane.shed.is_empty() {
                    self.anchor_fx(self.plane.pos);
                    let pose = self.plane.pose(&self.model.pieces);
                    let reqs: Vec<_> = self.plane.shed.drain(..).collect();
                    for req in &reqs {
                        self.sheds.spawn(req, &self.plane.dmg_prev, &self.model.pieces, &self.piece_mesh, &self.piece_cut, &pose, self.plane.pos, self.plane.rot, self.plane.vel, &mut self.rng, &mut femit);
                    }
                }
                for n in self.plane.notes.drain(..) {
                    self.flash.push((n, self.real));
                    if let Some(a) = &self.audio {
                        a.event(audio::Event::Snap(1.0));
                    }
                }
                if !self.plane.sparks.is_empty() {
                    self.anchor_fx(self.plane.pos);
                    let up = self.plane.pos.normalize().as_vec3();
                    for (p, v) in self.plane.sparks.drain(..) {
                        if self.rng.f() < 0.3 {
                            let r = &mut self.rng;
                            femit.push((p, render::Particle { vel: (v.as_vec3() + r.dir() * 3.0 + up * 1.5).into(), life: r.range(0.3, 0.9), size0: 0.05, size1: 0.02, kind: crash::SPARK, drag: 0.6, seed: r.f(), alpha: 0.9, ..Default::default() }));
                        }
                    }
                }
                if let Some(imp) = imp {
                    self.r.clear_particles();
                    let tears = self.wreck_meshes();
                    let mut crash = Crash::new(&imp, &self.plane, &self.group_mesh, &tears, self.rng.0 ^ self.frame, self.wx.steady(imp.pos, 40.0));
                    self.r.fluid.latest = None;
                    let (site, power) = (crash.site, crash.power);
                    self.fx_anchor = site;
                    let mut gone = 0.0;
                    if let Some((tile, idx)) = imp.building {
                        // The building that was hit comes down, and any shed or small house right beside it.
                        let (la, lo, alt) = geo::pos_to_lla(site);
                        if let Some(g) = self.world.ground(la, lo) {
                            crash.set_lift(alt - g.h);
                        }
                        let d = &self.plane.dmg;
                        let hit = destruct::Hitter { right: self.plane.right(), up: self.plane.up(), span: d.wing, stab: d.stab, fin: d.fin };
                        self.bring_down(tile, idx, imp.pos, imp.vel, Some(hit), &mut femit);
                        for (t, i) in self.world.buildings_near(site, 9.0 * power as f64, 160.0) {
                            self.bring_down(t, i, imp.pos, imp.vel * 0.3, None, &mut femit);
                        }
                    } else if !imp.water {
                        // Open ground: a bite is taken out of it.
                        let r = 3.0 + 5.0 * power;
                        self.world.add_crater(site, r);
                        self.destr.eject(site, r, imp.vel, &mut femit);
                        gone = r as f64;
                        for (t, i) in self.world.buildings_near(site, r as f64 + 4.0, 400.0) {
                            self.bring_down(t, i, imp.pos, imp.vel * 0.3, None, &mut femit);
                        }
                    }
                    if !imp.water {
                        self.destr.blast_trees(&mut self.world, &self.r.queue, site, 26.0 + 30.0 * power as f64, gone, &mut femit);
                    }
                    self.crash = Some(crash);
                    self.crashed = true;
                    self.crash_real = self.real;
                    self.orbit = 0.0;
                    self.cam_ang = None;
                    self.cam_clear_t = 0.0;
                    self.cam_rise = 0.0;
                    self.acc = 0.0;
                    break;
                }
            }
        }
        if dt > 0.0 {
            self.destr.update(dt as f32, &self.world, &mut femit);
            // Buildings next door that heavy falling pieces went through.
            let hits: Vec<_> = self.destr.crush.drain(..).collect();
            for (t, i, p, v) in hits {
                self.bring_down(t, i, p, v * 0.4, None, &mut femit);
                self.destr.booms.push(0.5);
            }
            self.sheds.update(dt as f32, &self.world);
        }
        let booms: Vec<f32> = self.destr.booms.drain(..).collect();
        if !booms.is_empty() {
            let near = self.crash.as_ref().map(|cr| 1.0 / (1.0 + ((self.cam_pos - cr.site).length() as f32 / 400.0).powi(2))).unwrap_or(0.5);
            for b in booms {
                if let Some(a) = &self.audio {
                    a.boom(b * (0.4 + 0.6 * near));
                }
            }
        }
        // Smoke from a damaged engine: oil-white while it still runs, black once it has seized.
        let sm = self.plane.dmg.smoke;
        if sm > 0.0 && dt > 0.0 && !self.crashed && !in_menu {
            self.anchor_fx(self.plane.pos);
            self.smoke_acc += sm * 22.0 * dt as f32;
            let exhaust = self.plane.pos + self.plane.rot * DVec3::new(0.335, -0.75, -2.10);
            let up = self.plane.pos.normalize().as_vec3();
            let fwd = self.plane.forward().as_vec3();
            let pv = self.plane.vel.as_vec3();
            let black = self.plane.dmg.smoke_black;
            while self.smoke_acc >= 1.0 {
                self.smoke_acc -= 1.0;
                let r = &mut self.rng;
                femit.push((exhaust, render::Particle { vel: (pv * 0.3 - fwd * 2.0 + r.dir() * 1.5 + up * 0.5).into(), life: r.range(2.0, 4.5), size0: r.range(0.25, 0.5), size1: r.range(2.5, 5.0), temp: 0.05, kind: if black { crash::GAS } else { crash::MIST }, drag: 1.2, buoy: 0.8, seed: r.f(), alpha: r.range(0.25, 0.45) * sm.clamp(0.4, 1.0), ..Default::default() }));
            }
        }
        let mut fire = 0.0f32;
        if let Some(cr) = self.crash.as_mut() {
            if dt > 0.0 {
                // Smoke leaving the top of the fire simulation carries on as particles.
                cr.top_out = match (&cr.domain, self.r.fluid.latest) {
                    (Some(dm), Some(st)) if st.out > 1e-3 => {
                        // Just outside the face it leaves through.
                        let f = st.out_c;
                        let edge = |v: f32| if v < 0.08 { -0.01 } else if v > 0.92 { 1.01 } else { v };
                        let fr = Vec3::new(edge(f.x), edge(f.y), if f.z > 0.92 { 1.01 } else { f.z });
                        let at = dm.at_frac(fr) - cr.site;
                        Some((at.as_vec3(), st.out * 1.8 / 110.0, 5.0))
                    }
                    _ => None,
                };
                cr.update(dt, &self.world, &mut emit);
            }
            if let Some(a) = &self.audio {
                let near = 1.0 / (1.0 + ((self.cam_pos - cr.site).length() as f32 / 250.0).powi(2));
                for p in &cr.booms {
                    a.boom(p * (0.35 + 0.65 * near));
                }
                let f = if cr.water { 0.0 } else { (cr.light().min(12.0) / 12.0) * near };
                fire = (f + self.destr.rumble.min(1.0) * near) * scale as f32;
            }
            cr.booms.clear();
        }
        for (pos, mut p) in femit {
            p.pos = (pos - self.fx_anchor).as_vec3().into();
            emit.push(p);
        }
        // Smoke and dust that would start inside the fire simulation's grid go
        // into the grid instead, where they can mix and flow with the rest.
        if let Some(cr) = self.crash.as_mut().filter(|_| dt > 0.0) {
            if let Some(dm) = &cr.domain {
                let off = (self.fx_anchor - cr.site).as_vec3();
                let mut conv = Vec::new();
                emit.retain(|p| {
                    let k = p.kind as i32;
                    let pos = Vec3::from(p.pos) + off;
                    if matches!(k, 0 | 2 | 7) && dm.contains(pos, 2.0) {
                        conv.push((pos, *p));
                        false
                    } else {
                        true
                    }
                });
                let keep = 48usize;
                let scale = (conv.len() as f32 / keep as f32).max(1.0);
                let step = scale.ceil() as usize;
                for (pos, p) in conv.iter().step_by(step.max(1)) {
                    let r = (p.size1 * 0.55).clamp(1.5, 7.0);
                    let amt = p.alpha * 1.2 / r * scale / dt as f32;
                    let k = p.kind as i32;
                    cr.fluid.push(fluid::FluidSrc {
                        pos: *pos,
                        radius: r,
                        vel: Vec3::from(p.vel),
                        radial: 1.0,
                        soot: if k == 0 { amt * 0.5 } else { 0.0 },
                        dust: if k == 0 { 0.0 } else { amt },
                        temp: if k == 0 { p.temp.min(1.2) } else { 0.0 },
                        vel_k: 2.5,
                        temp_k: 4.0,
                        ..Default::default()
                    });
                }
            }
        }
        if !emit.is_empty() {
            self.r.emit(&emit);
        }

        // ---- camera
        let size = self.window.inner_size();
        let (w, h) = (size.width.max(1) as f32, size.height.max(1) as f32);
        let fov = if self.cam_mode == 1 && !self.crashed && !in_menu { 72f32 } else { 60f32 }.to_radians();
        let (cam, dir, upv);
        if in_menu {
            self.menu.update(dt_real);
            (cam, dir, upv) = self.menu.camera();
            // Keep the side of the planet you are looking at in daylight.
            self.utc = (12.6 - self.menu.lon / 15.0).rem_euclid(24.0);
            self.cam_pos = cam;
        } else if self.crashed {
            let cr = self.crash.as_ref().unwrap();
            // After an impact up the side of a building, the wreck ends up in the street below.
            let street = cr.site - cr.up * cr.lift;
            let mut target = cr.debris.first().filter(|d| !d.sunk).map(|d| d.pos).unwrap_or(street) * 0.5 + street * 0.5 + cr.up * 6.0;
            let mut dist = 70.0 + 34.0 * cr.power as f64 + self.cam_dist;
            let (mut centre, mut rise) = (street, 18.0 + dist * 0.24);
            // A tower coming down: back off and watch all of it from the side.
            if let Some((mid, hgt)) = self.destr.focus().filter(|f| (f.0 - cr.site).length() < 800.0) {
                target = target * 0.25 + mid * 0.75;
                dist = dist.max(hgt * 1.2 + 60.0 + self.cam_dist);
                centre = mid;
                rise = dist * 0.1;
            }
            self.cam_look += (target - self.cam_look) * (dt_real * 3.0).min(1.0);
            self.orbit += dt_real * 0.1;
            let (e, n, u) = geo::enu(cr.site);
            let back = (self.cam_pos - centre).normalize_or_zero();
            let mut a = self.cam_ang.unwrap_or_else(|| back.dot(n).atan2(back.dot(e))) + dt_real * 0.12;
            let horiz = |a: f64| e * a.cos() + n * a.sin();
            // Keep a clear view: swing round, or rise a little, to see past
            // whatever building stands in the way.
            if self.real - self.cam_clear_t > 0.25 {
                self.cam_clear_t = self.real;
                let look = centre + u * 6.0;
                let clear = |from: DVec3| {
                    !(0..18).any(|k| {
                        let p = from + (look - from) * (k as f64 / 20.0);
                        let (la, lo, h) = geo::pos_to_lla(p);
                        self.world.building(la, lo).is_some_and(|b| h < b.top + 3.0 && h > b.base - 2.0)
                    })
                };
                // Only move when the view is blocked, and then stay a while.
                if !clear(centre + horiz(a) * dist + u * (rise + dist * self.cam_rise)) && self.real > self.cam_hold {
                    let mut best = (0.0, 0.4);
                    'search: for off in [0.0, 0.35, -0.35, 0.7, -0.7, 1.05, -1.05, 1.4, -1.4, 1.75, -1.75, 2.1, -2.1] {
                        for k in [0.0, 0.15, 0.3] {
                            if clear(centre + horiz(a + off) * dist + u * (rise + dist * k)) {
                                best = (off, k);
                                break 'search;
                            }
                        }
                    }
                    a += best.0;
                    self.cam_rise = best.1;
                    self.cam_hold = self.real + 1.5;
                }
            }
            self.cam_ang = Some(a);
            let want = centre + horiz(a) * dist + u * (rise + dist * self.cam_rise);
            self.cam_pos += (want - self.cam_pos) * (dt_real * 1.2).min(1.0);
            cam = self.cam_pos;
            dir = (self.cam_look - cam).normalize();
            upv = u;
        } else if self.cam_mode == 1 {
            let vibe = self.plane.dmg.vibe.min(2.0) as f64;
            cam = self.plane.pos + self.plane.rot * (airframe::EYE.as_dvec3() + self.plane.head) + (self.plane.right() * self.rng.range(-1.0, 1.0) as f64 + self.plane.up() * self.rng.range(-1.0, 1.0) as f64) * (vibe * 0.006);
            let q = DQuat::from_axis_angle(self.plane.up(), -self.cam_yaw + self.rng.range(-1.0, 1.0) as f64 * vibe * 0.004) * DQuat::from_axis_angle(self.plane.right(), -self.cam_pitch + self.rng.range(-1.0, 1.0) as f64 * vibe * 0.004);
            dir = q * self.plane.forward();
            upv = self.plane.up();
            self.cam_pos = cam;
        } else {
            let u = self.plane.pos.normalize();
            let back = -self.plane.forward();
            self.cam_dir = (self.cam_dir + (back - self.cam_dir) * (dt_real * 3.5).min(1.0)).normalize();
            let flat = (self.cam_dir - u * self.cam_dir.dot(u)).normalize_or_zero();
            let tilt = self.cam_dir.dot(u).asin() * 0.6 + self.cam_pitch;
            let d = DQuat::from_axis_angle(u, self.cam_yaw) * (flat * tilt.cos() + u * tilt.sin());
            let vibe = self.plane.dmg.vibe.min(2.0) as f64;
            cam = self.plane.pos + d * self.cam_dist + u * 1.5 + (self.plane.right() * self.rng.range(-1.0, 1.0) as f64 + u * self.rng.range(-1.0, 1.0) as f64) * (vibe * 0.03);
            dir = (self.plane.pos + u * 1.2 + self.plane.forward() * 6.0 - cam).normalize();
            upv = (u + self.plane.up() * 0.25).normalize();
            self.cam_pos = cam;
            self.cam_look = self.plane.pos;
        }
        // Never let the camera sink into the ground.
        let (clat, clon, calt) = geo::pos_to_lla(cam);
        let ground = self.world.ground(clat, clon).map(|g| g.h);
        let cam = match ground {
            Some(g) if calt < g + 1.5 => cam + cam.normalize() * (g + 1.5 - calt),
            _ => cam,
        };
        let calt = geo::pos_to_lla(cam).2;
        let agl = calt - ground.unwrap_or(0.0);
        let scrape = if steps > 0 { (scrape_n as f32 / steps as f32 * 0.5).min(1.0) } else { 0.0 };
        self.update_audio(cam, fire, in_menu, &c, dt, scrape);

        let near = if in_menu { (calt * 0.02).clamp(0.4, 20_000.0) as f32 } else if self.cam_mode == 1 && !self.crashed { 0.03f32 } else { 0.25f32 };
        let proj = perspective_rev(fov, w / h, near);
        let view = look_to(Vec3::ZERO, dir.as_vec3(), upv.as_vec3());
        let vp = proj * view;
        // Temporal anti-aliasing: each frame is drawn a different fraction of
        // a pixel off centre (a Halton sequence), and the renderer blends the
        // frames back together. Everything else works from the centred `vp`.
        let jitter = if self.taa {
            let halton = |mut i: u64, b: u64| { let (mut f, mut r) = (1.0f32, 0.0f32); while i > 0 { f /= b as f32; r += f * (i % b) as f32; i /= b; } r };
            let k = self.frame % 8 + 1;
            Vec2::new(halton(k, 2) - 0.5, halton(k, 3) - 0.5) * 2.0 / Vec2::new(w, h)
        } else {
            Vec2::ZERO
        };
        let vp_jit = Mat4::from_translation(jitter.extend(0.0)) * vp;
        self.world.update(&self.r, cam, &world::Frustum::new(DMat4::from_cols_array(&vp.to_cols_array().map(|v| v as f64))));
        let focus_abs = if in_menu { cam } else if self.crashed { self.crash.as_ref().unwrap().site } else { self.plane.pos };
        if dt > 0.0 {
            self.world.update_traffic(dt as f32, focus_abs);
        }

        // ---- frame constants
        let (east, north, up) = geo::enu(cam);
        let delta = cam - self.prev_cam;
        self.prev_cam = cam;
        if delta.length() < 5000.0 {
            self.wave_off += DVec2::new(delta.dot(east), delta.dot(north));
            self.wave_off = DVec2::new(self.wave_off.x.rem_euclid(65536.0), self.wave_off.y.rem_euclid(65536.0));
        }
        let sun = self.sun_dir();
        let (moon, moon_lit) = self.moon_dir();
        // Detail building tiles in range stand in for "how much city is around".
        let city = if in_menu { 0.0 } else { (self.world.stats.2 as f32 / 45.0).min(1.0) };
        self.city_glow += (city - self.city_glow) * (dt_real as f32 * 0.5).min(1.0);
        let mu = up.dot(sun) as f32;
        let day = ((mu + 0.2) / 0.4).clamp(0.0, 1.0);
        let day = day * day * (3.0 - 2.0 * day);
        // From the cockpit the eye adapts a little to the cabin.
        // Exposure moves in stops through twilight (a straight blend opens up
        // far too early, washing the city out while the sky still glows).
        // Night stays a night photograph: dark walls, bright windows.
        let exposure = (6.0f32.ln() + (0.5f32.ln() - 6.0f32.ln()) * day).exp() * if self.cam_mode == 1 && !self.crashed && !in_menu { 1.45 } else { 1.0 };
        let v4 = |v: DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
        let cam_right = dir.cross(upv).normalize();
        let cam_up = cam_right.cross(dir);
        // Shadow cascades: three nested boxes around the aircraft, each snapped
        // to its own texel grid so shadows do not shimmer as it moves.
        let shadow_on = mu > 0.04 && !in_menu;
        const CASC: [f32; 3] = [260.0, 1500.0, 8500.0];
        let mut casc_vp = [[[0f32; 4]; 4]; 4];
        {
            let right = { let r = DVec3::Z.cross(sun); if r.length_squared() < 1e-6 { DVec3::X } else { r.normalize() } };
            let upl = sun.cross(right);
            for (i, r) in CASC.iter().enumerate() {
                let texel = (2.0 * r / 4096.0) as f64;
                let snap = |v: f64| (v / texel).floor() * texel - v;
                let centre = focus_abs - cam + right * snap(focus_abs.dot(right)) + upl * snap(focus_abs.dot(upl));
                let d = r + 3500.0;
                let lview = look_to(centre.as_vec3() + sun.as_vec3() * d, -sun.as_vec3(), upl.as_vec3());
                casc_vp[i] = (ortho(-r, *r, -r, *r, 0.0, 2.0 * d) * lview).to_cols_array_2d();
            }
            // The aircraft's own shadow map: a few metres across, deep enough
            // to reach the ground below it.
            let centre = (self.plane.pos - cam).as_vec3();
            let lview = look_to(centre + sun.as_vec3() * PSHADOW_BACK, -sun.as_vec3(), upl.as_vec3());
            casc_vp[3] = (ortho(-PSHADOW_R, PSHADOW_R, -PSHADOW_R, PSHADOW_R, 0.0, PSHADOW_DEPTH) * lview).to_cols_array_2d();
        }
        let mut shadow_vp = [[[0f32; 4]; 4]; 3];
        shadow_vp.copy_from_slice(&casc_vp[..3]);
        let mut g = Globals {
            view_proj: vp_jit.to_cols_array_2d(),
            inv_view_proj: vp_jit.inverse().to_cols_array_2d(),
            shadow_vp,
            planet: v4(-cam, geo::R),
            sun_dir: v4(sun, SUN_E as f64),
            east: v4(east, self.wave_off.x),
            north: v4(north, self.wave_off.y),
            up: v4(up, calt),
            screen: [w, h, 1.0 / w, 1.0 / h],
            time: [self.time as f32, dt as f32, exposure, self.frame as f32],
            cloud: [if in_menu { 0.0 } else { self.wx.clouds }, self.wx.cloud_base, self.wx.cloud_depth, self.wx.cloud_ceiling()],
            light_pos: [0.0; 4],
            light_col: [0.0; 4],
            vol_o: [0.0; 4],
            vol_u: [0.0; 4],
            misc: [near, self.city_glow, 1.0, 2.0 * (fov * 0.5).tan() / h],
            csm: [CASC[0] * 2.0 / 4096.0, CASC[1] * 2.0 / 4096.0, CASC[2] * 2.0 / 4096.0, if shadow_on { 1.0 } else { 0.0 }],
            scorch: [[0.0; 4]; 8],
            crater: [[0.0; 4]; 8],
            prev_vp: self.prev_vp,
            // A jump (teleport, camera switch) invalidates the cloud history.
            cam_delta: v4(delta, if delta.length() < 500.0 { 1.0 } else { 0.0 }),
            moon: v4(moon, (MOON_E as f64) * moon_lit.powf(1.5)),
            vol_e: [0.0; 4],
            vol_n: [0.0; 4],
            pshadow_vp: casc_vp[3],
            pshadow: [PSHADOW_R * 2.0 / render::PLANE_SHADOW_RES as f32, 0.0012 / PSHADOW_DEPTH, render::PLANE_SHADOW_RES as f32 / 4096.0, if shadow_on && !self.crashed { 1.0 } else { 0.0 }],
            taa: [jitter.x * 0.5, -jitter.y * 0.5, if self.taa { 1.0 } else { 0.0 }, 0.0],
            plane_inv: [[0.0; 4]; 4],
            plane_prev: [[0.0; 4]; 4],
            plane_box: self.plane_box,
            weather: { let o = self.wx.cloud_off; [o.x as f32, o.y as f32, o.z as f32, if in_menu { 1.0 } else { self.wx.haze }] },
            storm: { let w = self.wx.steady(cam, 2000.0); let d = if w.length() > 0.1 { w.normalize() } else { east }; v4(d, 0.0) },
            storm_cells: [[0.0; 4]; 2 * weather::MAX_STORMS],
            det: [[0.0; 4]; 16],
        };
        if !in_menu {
            let cells = self.wx.storm_cells(cam);
            g.storm[3] = cells.len() as f32;
            for (i, c) in cells.iter().enumerate() {
                g.storm_cells[2 * i] = c[0];
                g.storm_cells[2 * i + 1] = c[1];
            }
            // The flight model flies through the same cells.
            self.wx.cells = cells;
        }
        if !self.crashed && !in_menu {
            // The aircraft keeps pace with the camera, so its pixels are found
            // in last frame's image through where it was then, not through
            // where the ground under them was.
            let base = Mat4::from_rotation_translation(self.plane.rot.as_quat(), (self.plane.pos - cam).as_vec3());
            g.taa[3] = 1.0;
            g.plane_inv = base.inverse().to_cols_array_2d();
            g.plane_prev = (Mat4::from_cols_array_2d(&self.prev_vp) * self.prev_plane).to_cols_array_2d();
            self.prev_plane = base;
        }
        self.prev_vp = vp.to_cols_array_2d();
        for (i, (p, r)) in self.world.craters.iter().take(8).enumerate() {
            g.crater[i] = v4(*p - cam, *r as f64);
        }
        let mut psim = PSim { anchor: v4(self.fx_anchor - cam, 0.0), upv: v4(self.fx_anchor.normalize(), dt), cam_r: v4(cam_right, self.time), cam_u: v4(cam_up, 0.0), wind: v4(self.wx.steady(self.fx_anchor, 10.0), 0.0), shift: v4(self.fx_shift, 0.0), ..Default::default() };
        if dt > 0.0 {
            // The simulation runs this frame and applies the shift.
            self.fx_shift = DVec3::ZERO;
        }
        let mut objs = Vec::new();
        if let Some(cr) = &self.crash {
            // The ground under an impact up the side of a building is further down.
            psim.anchor = v4(cr.site - cam, -cr.lift);
            psim.upv = v4(cr.up, dt);
            g.light_pos = v4(cr.site + cr.up * 8.0 - cam, 12.0 + 8.0 * cr.power as f64);
            let l = cr.light();
            g.light_col = [1.0 * l, 0.5 * l, 0.2 * l, 0.0];
            let grow = (cr.time as f32 / 1.2).min(1.0);
            for (i, (p, rad)) in cr.scorch.iter().take(8).enumerate() {
                g.scorch[i] = v4(cr.site + p.as_dvec3() - cam, (*rad * grow) as f64);
            }
            if self.crashed || cr.time < 240.0 {
                for (i, d) in cr.debris.iter().enumerate() {
                    if d.sunk {
                        continue;
                    }
                    let m = Mat4::from_scale_rotation_translation(Vec3::splat(d.scale), d.rot.as_quat(), (d.pos - cam).as_vec3()) * Mat4::from_translation(-d.origin);
                    objs.push(DrawObj { mesh: d.mesh, model: m, tint: [1.0, 1.0, 1.0, d.char], glow: [if d.burn > 0.0 { 1.0 } else { 0.25 }, 0.0, 0.0, i as f32 * 0.37], cast: 0b11, cut: true, tear: [d.tear[0].gpu(), d.tear[1].gpu()], dmg: [d.tear[0].mode, d.tear[1].mode, 0.6, damage::JAG], ..DrawObj::DEFAULT });
                }
            }
        }
        // The fire simulation: this frame's sources, and the light it gives off.
        let mut fluid = None;
        if let Some(cr) = self.crash.as_mut() {
            let wind = self.wx.steady(cr.site, 10.0);
            // Once the fire is out and its smoke has gone, the grid rests.
            let spent = cr.fluid.is_empty() && cr.time > 8.0 && self.r.fluid.latest.is_some_and(|st| st.soot < 2.0 && st.emit < 1.0);
            if let Some(dm) = cr.domain.as_mut().filter(|_| !spent) {
                fluid = Some(dm.frame(dt, cam, wind, sun, &cr.fluid, &self.world));
                if let Some(st) = self.r.fluid.latest {
                    // A ball about as big as the burning gas, as bright as flame.
                    let (t, p, _) = cr.last_blast;
                    let flash = p * 6.0 * (-(t as f32) / 0.25).exp();
                    let rad = (0.75 * st.emit.max(0.0).cbrt()).clamp(3.0, 25.0);
                    let l = flash + 2.5 * (st.emit / 300.0).min(1.0);
                    g.light_pos = v4(dm.at_frac(st.emit_c) - cam, rad as f64);
                    g.light_col = [1.0 * l, 0.48 * l, 0.17 * l, 0.0];
                }
            }
        }
        let params = self.instruments(agl);
        if !self.crashed && !in_menu {
            let base = Mat4::from_rotation_translation(self.plane.rot.as_quat(), (self.plane.pos - cam).as_vec3());
            let pose = self.plane.pose(&self.model.pieces);
            // Blades while they can be followed by eye, a blur once they cannot.
            let blur = ((self.plane.prop_rate - 30.0) / 25.0).clamp(0.0, 1.0) as f32;
            let dm = self.plane.dmg;
            for (i, p) in self.model.pieces.iter().enumerate() {
                let disc = p.anim == airframe::Anim::PropDisc;
                if (p.name == "blades" && blur >= 1.0) || (disc && blur <= 0.0) || dm.hidden(p) {
                    continue;
                }
                let tears = dm.tears_for(p);
                let torn = tears[0].is_some() || tears[1].is_some();
                objs.push(DrawObj {
                    mesh: self.piece_mesh[i],
                    model: base * pose[i],
                    tint: [blur, 0.0, 0.0, 0.0],
                    params,
                    cast: 1 << render::PLANE_LAYER,
                    glass: p.glass,
                    cut: self.piece_cut[i] || torn,
                    tear: [tears[0].gpu(), tears[1].gpu()],
                    dmg: [tears[0].mode, tears[1].mode, dm.scuff[p.group], damage::JAG],
                    dmg2: [dm.oil, dm.crack[0], dm.crack[1], dm.crack[2]],
                    ..DrawObj::DEFAULT
                });
            }
        }
        if !in_menu {
            self.sheds.draw(cam, params, &mut objs);
        }
        let mut insts = Vec::new();
        let mut fall = Vec::new();
        if !in_menu {
            let local = (self.utc + clon.to_degrees() / 15.0).rem_euclid(24.0);
            self.world.car_instances(cam, cars::activity(local as f32), &mut insts);
            self.n_cars = insts.len();
            self.destr.instances(cam, &mut insts, &mut fall);
        }

        let ui = if in_menu { self.menu_ui(vp, cam, w, h) } else { self.ui(agl) };
        // The smoke volume: a grid around the effects anchor, on its local east/north/up axes.
        {
            let site = cam + DVec3::new(psim.anchor[0] as f64, psim.anchor[1] as f64, psim.anchor[2] as f64);
            let (ve, vn, vu) = geo::enu(site);
            let (cs, zmin) = (render::VOL_CELL as f64, render::VOL_ZMIN as f64);
            let lift = self.crash.as_ref().map(|c| c.lift).unwrap_or(0.0);
            let corner = site - ve * (cs * render::VOL_N[0] as f64 * 0.5) - vn * (cs * render::VOL_N[1] as f64 * 0.5) + vu * (zmin - lift);
            g.vol_o = v4(corner - cam, cs);
            g.vol_u = v4(vu, 0.0);
            g.vol_e = v4(ve, 0.0);
            g.vol_n = v4(vn, 0.0);
            psim.ve = v4(ve, cs);
            psim.vn = v4(vn, zmin - lift);
        }
        let (tiles, details, near) = self.world.draw_lists(cam, focus_abs, CASC);
        let mut shot = match self.shots.first() {
            Some((t, _, _)) if self.real >= *t => Some(self.shots.remove(0).1),
            _ => None,
        };
        let mut rec_done = false;
        if let Some((dir, start, secs)) = &self.record {
            // The synth runs every frame so it is settled when recording starts.
            let n = (audio::Audio::RATE as f64 / REC_FPS).round() as usize;
            let samples = self.audio.as_ref().map(|a| a.take(n)).unwrap_or_default();
            if self.real >= *start {
                if shot.is_none() {
                    let _ = std::fs::create_dir_all(dir);
                    shot = Some(dir.join(format!("f{:05}.png", self.rec_n)));
                    self.rec_n += 1;
                }
                self.rec_audio.extend(samples);
                if self.real >= start + secs {
                    write_wav(&dir.join("audio.wav"), &self.rec_audio, audio::Audio::RATE);
                    rec_done = true;
                }
            }
        }
        // Scripted shots can pre-position the chase camera for the next capture.
        if let Some((_, _, Some((y, p, d)))) = self.shots.first() {
            (self.cam_yaw, self.cam_pitch, self.cam_dist) = (y.to_radians(), *p, *d);
        }
        let had_shots = shot.is_some() && self.shots.is_empty();
        self.r.render(Frame { g, tiles, details, objs, insts, fall, near, casc_vp, psim, sim_steps: dt > 0.0, fluid, egui: Some(ui), shot });
        if (had_shots && std::env::args().any(|a| a == "--shot") && self.record.is_none()) || rec_done {
            self.quit = true;
        }
    }

    /// The start screen drawn over the globe: labels, the chosen spot, and the panel.
    fn menu_ui(&mut self, vp: Mat4, cam: DVec3, w: f32, h: f32) -> (Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32) {
        let raw = self.egui_input();
        let mut go: Option<String> = None;
        let mut start = false;
        let mut jump: Option<(f64, f64, &'static str)> = None;
        let alt = self.menu.alt;
        let out = self.egui_ctx.clone().run_ui(raw, |root| {
            let ctx = &root.ctx().clone();
            use egui::{Align2, Color32, FontId, RichText};
            let ppp = ctx.pixels_per_point();
            // Screen position of a place, if it is on the near side of the planet.
            let project = |lat: f64, lon: f64| -> Option<egui::Pos2> {
                let p = geo::lla_to_pos(lat.to_radians(), lon.to_radians(), 0.0);
                if p.normalize().dot((cam - p).normalize()) < 0.05 {
                    return None;
                }
                let c = vp * (p - cam).as_vec3().extend(1.0);
                if c.w <= 0.0 {
                    return None;
                }
                let (x, y) = ((c.x / c.w * 0.5 + 0.5) * w, (0.5 - c.y / c.w * 0.5) * h);
                (x > -50.0 && y > -50.0 && x < w + 50.0 && y < h + 50.0).then(|| egui::pos2(x / ppp, y / ppp))
            };
            let painter = ctx.layer_painter(egui::LayerId::background());
            let label = |p: egui::Pos2, text: &str, size: f32, col: Color32| {
                painter.text(p + egui::vec2(1.0, 1.0), Align2::LEFT_CENTER, text, FontId::proportional(size), Color32::from_black_alpha(200));
                painter.text(p, Align2::LEFT_CENTER, text, FontId::proportional(size), col);
            };
            if alt > 25_000.0 {
                for (name, lat, lon, tier) in menu::CITIES {
                    if *tier > 1 && alt > 7.0e6 {
                        continue;
                    }
                    if let Some(p) = project(*lat, *lon) {
                        painter.circle_filled(p, 2.5, Color32::from_rgba_unmultiplied(255, 255, 255, 220));
                        label(p + egui::vec2(7.0, 0.0), name, if *tier == 1 { 14.0 } else { 12.5 }, Color32::from_rgba_unmultiplied(255, 255, 255, 225));
                    }
                }
            }
            if let Some((lat, lon)) = self.menu.pick {
                if let Some(p) = project(lat, lon) {
                    let accent = Color32::from_rgb(255, 196, 64);
                    let pulse = 11.0 + 3.0 * (self.real * 3.0).sin() as f32;
                    painter.circle_stroke(p, pulse, egui::Stroke::new(2.0, accent));
                    painter.circle_filled(p, 3.5, accent);
                    label(p + egui::vec2(18.0, 0.0), "Start here", 15.0, accent);
                }
            }
            use menu::{ACCENT, DIM, TEXT};
            const W: f32 = 330.0;
            egui::Area::new(egui::Id::unique("menu")).anchor(Align2::LEFT_TOP, [22.0, 22.0]).show(ctx, |ui| {
                menu::panel().show(ui, |ui| {
                    ui.set_width(W);
                    ui.label(RichText::new("FABLE FLIGHT").size(32.0).strong().color(Color32::WHITE));
                    ui.label(RichText::new("Fly anywhere on Earth").size(14.0).color(ACCENT));
                    ui.add_space(6.0);
                    ui.label(RichText::new("Drag to turn the globe, scroll to zoom, click where you want to start.").size(13.0).color(DIM));
                    ui.add_space(8.0);

                    menu::heading(ui, "WHERE");
                    ui.horizontal(|ui| {
                        let resp = ui.add(egui::TextEdit::singleline(&mut self.menu.search).desired_width(W - 56.0).hint_text("Search a city, landmark or address").margin(egui::vec2(8.0, 6.0)));
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (ui.add(egui::Button::new(RichText::new("Go").size(14.0)).min_size(egui::vec2(48.0, 28.0))).clicked() || enter) && !self.menu.search.trim().is_empty() {
                            go = Some(self.menu.search.trim().to_string());
                        }
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                        for (name, lat, lon) in menu::QUICK {
                            let sel = self.menu.pick == Some((*lat, *lon));
                            if ui.add(egui::Button::new(RichText::new(*name).size(12.5)).selected(sel)).clicked() {
                                jump = Some((*lat, *lon, name));
                            }
                        }
                    });
                    ui.add_space(8.0);

                    menu::heading(ui, "CONDITIONS");
                    egui::Grid::new("start_wx").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.spacing_mut().slider_width = W - 150.0;
                        ui.label(RichText::new("Time of day").color(TEXT));
                        ui.add(egui::Slider::new(&mut self.menu.hour, 5.0..=21.0).custom_formatter(|v, _| menu::hhmm(v)));
                        ui.end_row();
                        ui.label(RichText::new("Clouds").color(TEXT));
                        ui.add(egui::Slider::new(&mut self.wx.clouds, 0.0..=1.0).custom_formatter(|v, _| match v {
                            v if v < 0.05 => "clear".into(),
                            v => format!("{:.0}%", v * 100.0),
                        }));
                        ui.end_row();
                        ui.label(RichText::new("Wind").color(TEXT));
                        let mut kt = self.wx.wind * 1.94384;
                        if ui.add(egui::Slider::new(&mut kt, 0.0..=40.0).fixed_decimals(0).suffix(" kt")).changed() {
                            self.wx.wind = kt / 1.94384;
                        }
                        ui.end_row();
                    });
                    ui.add_space(8.0);

                    menu::heading(ui, "START POINT");
                    egui::Frame::new().fill(Color32::from_white_alpha(8)).corner_radius(8.0).inner_margin(10.0).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        match self.menu.pick {
                            Some((lat, lon)) => {
                                let name = if self.menu.pick_name.is_empty() { "Unnamed spot".to_string() } else { self.menu.pick_name.chars().take(90).collect() };
                                ui.label(RichText::new(name).size(14.0).color(Color32::WHITE));
                                ui.label(RichText::new(format!("{}   ·   {} local", menu::latlon(lat, lon), menu::hhmm(self.menu.hour))).size(12.0).color(DIM));
                            }
                            None => {
                                ui.label(RichText::new("Click the globe or pick a place above.").size(13.5).color(DIM));
                            }
                        }
                    });
                    ui.add_space(10.0);
                    let text = if self.menu.starting { "Preparing…" } else { "TAKE OFF" };
                    let ready = self.menu.pick.is_some() && !self.menu.starting;
                    if menu::wide_button(ui, text, if ready { "Enter" } else { "" }, true, ready) {
                        start = true;
                    }
                    ui.add_space(6.0);
                    ui.columns(2, |c| {
                        if menu::wide_button(&mut c[0], if self.menu.show_controls { "Hide controls" } else { "Controls" }, "", false, true) {
                            self.menu.show_controls = !self.menu.show_controls;
                        }
                        if menu::wide_button(&mut c[1], "Quit", "Esc", false, true) {
                            self.quit = true;
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new("Double-click the globe to start there right away.").size(11.5).color(DIM));
                });
            });
            if self.menu.show_controls {
                egui::Area::new(egui::Id::unique("menu_keys")).anchor(Align2::LEFT_TOP, [22.0 + W + 40.0 + 18.0, 22.0]).show(ctx, |ui| {
                    menu::panel().show(ui, |ui| {
                        ui.set_width(560.0);
                        ui.label(RichText::new("Controls").size(22.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new("In flight, Tab opens the menu: pause, weather, settings and this list.").size(12.5).color(DIM));
                        ui.add_space(6.0);
                        menu::controls_list(ui);
                    });
                });
            }
            egui::Area::new(egui::Id::unique("attr")).anchor(Align2::RIGHT_BOTTOM, [-10.0, -6.0]).show(ctx, |ui| {
                ui.label(RichText::new(ATTRIBUTION).size(10.0).color(Color32::from_rgba_unmultiplied(255, 255, 255, 110)));
            });
        });
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        if let Some((lat, lon, name)) = jump {
            self.menu.look_at(lat, lon, 30_000.0);
            self.menu.pick = Some((lat, lon));
            self.menu.pick_name = name.to_string();
        }
        if let Some(q) = go {
            self.menu.pick_name = format!("Searching for {q}…");
            let (tx, rx) = std::sync::mpsc::channel();
            let src = self.src.clone();
            std::thread::spawn(move || {
                let _ = tx.send(geocode(&src.fetch, &q).map(|(la, lo, name)| (la, lo, 0.0, name)));
            });
            self.goto_rx = Some(rx);
        }
        if start {
            self.begin();
        }
        let jobs = self.egui_ctx.tessellate(out.shapes, out.pixels_per_point);
        (jobs, out.textures_delta, out.pixels_per_point)
    }

    /// The in-flight menu (Tab), drawn over the dimmed, paused game.
    #[allow(clippy::too_many_arguments)]
    fn pause_menu(&mut self, ctx: &egui::Context, act: &mut Option<PauseAct>, lat: f64, lon: f64, local: f64, speed: f64, alt: f64, agl: f64, hdg: f64) {
        use egui::{Align2, Color32, RichText};
        use menu::{Page, ACCENT, DIM, TEXT};
        // The backdrop dims the scene and keeps the mouse from the camera behind it.
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("pause_dim")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
            ui.allocate_response(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(130));
        });
        const W: f32 = 700.0;
        const BODY: f32 = 470.0;
        egui::Area::new(egui::Id::unique("pause")).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            menu::panel().show(ui, |ui| {
                ui.set_width(W);
                // Header: what this is and where we are.
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("PAUSED").size(28.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new("FABLE FLIGHT").size(11.5).strong().color(ACCENT));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                ui.label(RichText::new(menu::latlon(lat, lon)).size(13.0).color(TEXT));
                                ui.label(RichText::new(format!("{} local", menu::hhmm(local))).size(12.0).color(DIM));
                            });
                        });
                    });
                });
                ui.add_space(10.0);
                // Page tabs.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for p in Page::ALL {
                        let b = egui::Button::selectable(self.pause_page == p, RichText::new(p.name()).size(15.0)).min_size(egui::vec2(0.0, 30.0));
                        if ui.add(b).clicked() {
                            self.pause_page = p;
                        }
                    }
                });
                ui.separator();
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(BODY).auto_shrink([false, false]).min_scrolled_height(BODY).show(ui, |ui| {
                    ui.set_min_height(BODY);
                    match self.pause_page {
                        Page::Flight => {
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(240.0);
                                    menu::heading(ui, "ACTIONS");
                                    ui.add_space(2.0);
                                    if menu::wide_button(ui, "Respawn in the air", "R", false, true) {
                                        *act = Some(PauseAct::Respawn);
                                    }
                                    if menu::wide_button(ui, "Go to a place…", "G", false, true) {
                                        *act = Some(PauseAct::GoTo);
                                    }
                                    if menu::wide_button(ui, "Back to the globe", "", false, true) {
                                        *act = Some(PauseAct::Globe);
                                    }
                                    if menu::wide_button(ui, "Quit to desktop", "", false, true) {
                                        *act = Some(PauseAct::Quit);
                                    }
                                    ui.add_space(10.0);
                                    menu::heading(ui, "FLY TO");
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);
                                        for (name, la, lo) in menu::QUICK {
                                            if ui.button(RichText::new(*name).size(12.5)).clicked() {
                                                *act = Some(PauseAct::FlyTo(*la, *lo, name));
                                            }
                                        }
                                    });
                                });
                                ui.add_space(18.0);
                                ui.vertical(|ui| {
                                    let p = &self.plane;
                                    menu::heading(ui, "AIRCRAFT");
                                    egui::Grid::new("pause_ac").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                                        let mut row = |k: &str, v: String| {
                                            ui.label(RichText::new(k).color(DIM));
                                            ui.label(RichText::new(v).monospace().size(13.5).color(TEXT));
                                            ui.end_row();
                                        };
                                        row("Airspeed", format!("{:.0} kt", speed * 1.94384));
                                        row("Altitude", format!("{:.0} ft  ({:.0} ft above ground)", alt * 3.28084, agl.max(0.0) * 3.28084));
                                        row("Heading", format!("{hdg:03.0}°"));
                                        row("Throttle", format!("{:.0}%", p.throttle * 100.0));
                                        row("Flaps", format!("{:.0}°", p.flap_deg()));
                                        row("Gear", if p.gear >= 1.0 { "down" } else if p.gear <= 0.0 { "up" } else { "moving" }.into());
                                    });
                                    ui.add_space(10.0);
                                    menu::heading(ui, "CONDITION");
                                    let report = p.dmg.report();
                                    if self.crashed {
                                        ui.label(RichText::new("Crashed. Respawn to fly again.").color(Color32::from_rgb(255, 90, 60)));
                                    } else if report.is_empty() {
                                        ui.label(RichText::new("No damage").color(Color32::from_rgb(140, 220, 140)));
                                    } else {
                                        for l in &report {
                                            ui.label(RichText::new(l).color(Color32::from_rgb(255, 180, 50)));
                                        }
                                    }
                                    ui.add_space(10.0);
                                    menu::heading(ui, "WORLD");
                                    let st = self.world.stats;
                                    ui.label(RichText::new(format!("{} tiles loaded, {} loading, {} cars", st.0, st.1, self.n_cars)).size(12.5).color(DIM));
                                    ui.label(RichText::new(format!("{:.0} fps", self.fps)).size(12.5).color(DIM));
                                });
                            });
                        }
                        Page::World => {
                            ui.label(RichText::new("Changes show behind this menu as you make them.").size(12.5).color(DIM));
                            self.wx.controls(ui, &mut self.utc, &mut self.doy, lon, true);
                        }
                        Page::Settings => {
                            egui::Grid::new("pause_set").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                                ui.spacing_mut().slider_width = 220.0;
                                menu::heading(ui, "SOUND");
                                ui.end_row();
                                ui.label("Sound");
                                let t = if self.sound_on { "on" } else { "off" };
                                ui.checkbox(&mut self.sound_on, t);
                                ui.end_row();
                                ui.label("Volume");
                                ui.add_enabled(self.sound_on, egui::Slider::new(&mut self.volume, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                                ui.end_row();
                                menu::heading(ui, "VIEW");
                                ui.end_row();
                                ui.label("Camera");
                                ui.horizontal(|ui| {
                                    ui.selectable_value(&mut self.cam_mode, 0, "Chase");
                                    ui.selectable_value(&mut self.cam_mode, 1, "Cockpit");
                                });
                                ui.end_row();
                                ui.label("Mouse look speed");
                                ui.add(egui::Slider::new(&mut self.look_sens, 0.25..=3.0).logarithmic(true).custom_formatter(|v, _| format!("{v:.2}×")));
                                ui.end_row();
                                ui.label("HUD");
                                let t = if self.show_hud { "shown" } else { "hidden" };
                                ui.checkbox(&mut self.show_hud, t);
                                ui.end_row();
                                menu::heading(ui, "GRAPHICS");
                                ui.end_row();
                                ui.label("Anti-aliasing");
                                ui.checkbox(&mut self.taa, "temporal (TAA)");
                                ui.end_row();
                            });
                            ui.add_space(10.0);
                            ui.label(RichText::new("Shortcuts in flight: U sound, C camera, H HUD, F2 anti-aliasing.").size(12.0).color(DIM));
                        }
                        Page::Controls => menu::controls_list(ui),
                    }
                });
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.scope(|ui| {
                        ui.set_width(220.0);
                        if menu::wide_button(ui, "RESUME", "Tab", true, true) {
                            *act = Some(PauseAct::Resume);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Tab or Esc to resume  ·  Start on a gamepad").size(12.0).color(DIM));
                    });
                });
            });
        });
    }

    fn ui(&mut self, agl: f64) -> (Vec<egui::ClippedPrimitive>, egui::TexturesDelta, f32) {
        let raw = self.egui_input();
        let (lat, lon, alt) = geo::pos_to_lla(self.plane.pos);
        let (e, n, u) = geo::enu(self.plane.pos);
        let f = self.plane.forward();
        let hdg = f.dot(e).atan2(f.dot(n)).to_degrees().rem_euclid(360.0);
        // Airspeed: what the wing feels, and what the overspeed warnings are about.
        let speed = self.plane.airspeed().length();
        let gs = self.plane.vel.length();
        let vs = self.plane.vel.dot(u);
        let wind_kt = self.wx.wind * 1.94384;
        let local = (self.utc + lon.to_degrees() / 15.0).rem_euclid(24.0);
        let mut go: Option<String> = None;
        let mut close_goto = false;
        let mut act = None;
        let stats = self.world.stats;
        self.flash.retain(|(_, t)| self.real - t < 4.0);
        let out = self.egui_ctx.clone().run_ui(raw, |root| {
            let ctx = &root.ctx().clone();
            use egui::{Align2, Color32, FontId, RichText};
            let white = Color32::from_rgba_unmultiplied(255, 255, 255, 235);
            let dim = Color32::from_rgba_unmultiplied(255, 255, 255, 150);
            let mono = |s: f32| FontId::monospace(s);
            let shadow = |ui: &mut egui::Ui, text: String, size: f32, col: Color32| {
                ui.label(RichText::new(text).font(mono(size)).color(col));
            };
            let panel = egui::Frame::new().fill(Color32::from_rgba_unmultiplied(8, 12, 18, 120)).corner_radius(6.0).inner_margin(10.0);
            if self.pause_open {
                self.pause_menu(ctx, &mut act, lat.to_degrees(), lon.to_degrees(), local, speed, alt, agl, hdg);
                return;
            }
            if self.show_hud {
                egui::Area::new(egui::Id::unique("hud")).anchor(Align2::LEFT_TOP, [14.0, 14.0]).show(ctx, |ui| {
                    panel.show(ui, |ui| {
                        shadow(ui, format!("SPD {:>4.0} kt", speed * 1.94384), 20.0, white);
                        shadow(ui, format!("ALT {:>5.0} ft", alt * 3.28084), 20.0, white);
                        shadow(ui, format!("AGL {:>5.0} ft", agl.max(0.0) * 3.28084), 14.0, dim);
                        shadow(ui, format!("HDG {:>4.0}°   VS {:>+6.0} fpm", hdg, vs * 196.85), 14.0, dim);
                        if wind_kt >= 0.5 {
                            shadow(ui, format!("GS {:>4.0} kt   WIND {:03.0}°/{:.0}", gs * 1.94384, self.wx.wind_from, wind_kt), 14.0, dim);
                        }
                        shadow(ui, format!("THR {:>3.0}%   G {:>4.1}", self.plane.throttle * 100.0, self.plane.g_load), 14.0, dim);
                        let p = &self.plane;
                        let gear = if p.gear >= 1.0 { "DOWN".to_string() } else if p.gear <= 0.0 { "UP".to_string() } else { "transit".to_string() };
                        shadow(ui, format!("FLAPS {:>2.0}°   GEAR {}", p.flap_deg(), gear), 14.0, if p.gear > 0.0 && p.gear < 1.0 { Color32::from_rgb(255, 190, 60) } else { dim });
                        if p.gear < 1.0 && agl < 150.0 && p.throttle < 0.35 && !self.crashed {
                            shadow(ui, "GEAR UP  (L)".into(), 20.0, Color32::from_rgb(255, 90, 60));
                        }
                        if self.plane.stalled && !self.crashed {
                            shadow(ui, "STALL".into(), 20.0, Color32::from_rgb(255, 90, 60));
                        }
                    });
                });
                egui::Area::new(egui::Id::unique("info")).anchor(Align2::LEFT_BOTTOM, [14.0, -14.0]).show(ctx, |ui| {
                    panel.show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        if self.real < self.status_until && !self.status.is_empty() {
                            shadow(ui, self.status.chars().take(70).collect(), 13.0, white);
                        }
                        shadow(ui, format!("{:.4}°{}  {:.4}°{}   {:02}:{:02} local", lat.to_degrees().abs(), if lat >= 0.0 { 'N' } else { 'S' }, lon.to_degrees().abs(), if lon >= 0.0 { 'E' } else { 'W' }, local as u32, (local.fract() * 60.0) as u32), 13.0, dim);
                        shadow(ui, format!("{:.0} fps   {} tiles   {} loading   {} cars   Tab menu", self.fps, stats.0, stats.1, self.n_cars), 12.0, dim);
                    });
                });
                egui::Area::new(egui::Id::unique("attr")).anchor(Align2::RIGHT_BOTTOM, [-10.0, -6.0]).show(ctx, |ui| {
                    ui.label(RichText::new(ATTRIBUTION).size(10.0).color(Color32::from_rgba_unmultiplied(255, 255, 255, 110)));
                });
            }
            // Damage: warnings while the airframe is being overstressed, what has
            // already broken, and a flash for each new loss.
            let p = &self.plane;
            let mut warn: Vec<(&str, Color32)> = Vec::new();
            let red = Color32::from_rgb(255, 80, 50);
            let amber = Color32::from_rgb(255, 180, 50);
            if !self.crashed && self.show_hud {
                let w = &self.warn;
                if w.pull_up {
                    warn.push(("PULL UP", red));
                } else if w.dive > 0.0 {
                    warn.push(("NOSE DIVE", red));
                } else if w.too_low {
                    warn.push(("TOO LOW  TERRAIN", amber));
                } else if w.sink {
                    warn.push(("SINK RATE", amber));
                }
                if speed > plane::VNE + 13.0 {
                    warn.push(("OVERSPEED  AIRFRAME BREAKING UP", red));
                } else if speed > plane::VNE {
                    warn.push(("OVERSPEED", amber));
                }
                if p.flaps > 0.25 && speed > plane::VFE {
                    warn.push(("FLAPS OVERSPEED", if p.dmg.flap_stress > 0.5 { red } else { amber }));
                }
                if p.g_load.abs() > 5.0 {
                    warn.push(("OVER G", if p.dmg.fatigue > 1.2 { red } else { amber }));
                }
            }
            let report = if self.crashed { Vec::new() } else { p.dmg.report() };
            if self.show_hud && (!warn.is_empty() || !report.is_empty()) {
                egui::Area::new(egui::Id::unique("damage")).anchor(Align2::RIGHT_TOP, [-14.0, 14.0]).show(ctx, |ui| {
                    panel.show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        for (w, col) in &warn {
                            shadow(ui, (*w).into(), 18.0, *col);
                        }
                        if !report.is_empty() {
                            shadow(ui, "DAMAGE".into(), 12.0, dim);
                            for l in &report {
                                shadow(ui, l.clone(), 14.0, amber);
                            }
                        }
                    });
                });
            }
            if !self.flash.is_empty() && !self.crashed {
                egui::Area::new(egui::Id::unique("flash")).anchor(Align2::CENTER_TOP, [0.0, 70.0]).show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        for (s, t0) in &self.flash {
                            let a = (1.0 - ((self.real - t0 - 2.5) / 1.5).max(0.0)).clamp(0.0, 1.0);
                            shadow(ui, s.clone(), 24.0, Color32::from_rgba_unmultiplied(255, 70, 40, (a * 255.0) as u8));
                        }
                    });
                });
            }
            if self.crashed && self.real - self.crash_real > 3.5 {
                egui::Area::new(egui::Id::unique("crash")).anchor(Align2::CENTER_BOTTOM, [0.0, -70.0]).show(ctx, |ui| {
                    panel.show(ui, |ui| shadow(ui, "R  respawn      G  go somewhere else      Tab  menu      Esc  globe".into(), 18.0, white));
                });
            }
            if self.show_help {
                egui::Area::new(egui::Id::unique("help")).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    menu::panel().show(ui, |ui| {
                        ui.set_width(600.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Controls").size(22.0).strong().color(Color32::WHITE));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new("F1 / Esc  close").size(12.0).color(menu::DIM));
                            });
                        });
                        ui.add_space(4.0);
                        menu::controls_list(ui);
                    });
                });
            }
            if self.wx.open {
                self.wx.panel(ctx, &mut self.utc, &mut self.doy, lon.to_degrees());
            }
            if self.goto_open {
                egui::Window::new("Go to").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, -80.0]).show(ctx, |ui| {
                    ui.label("City, landmark or street address:");
                    let resp = ui.add(egui::TextEdit::singleline(&mut self.goto_text).desired_width(360.0));
                    resp.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !self.goto_text.trim().is_empty() {
                        go = Some(self.goto_text.trim().to_string());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close_goto = true;
                    }
                });
            }
        });
        self.egui_state.handle_platform_output(&self.window, out.platform_output);
        if let Some(q) = go {
            self.goto_open = false;
            self.status = format!("Searching for {q}…");
            self.status_until = self.real + 30.0;
            let (tx, rx) = std::sync::mpsc::channel();
            let src = self.src.clone();
            std::thread::spawn(move || {
                let _ = tx.send(geocode(&src.fetch, &q).map(|(la, lo, name)| (la, lo, ground_elev(&src, la, lo), name)));
            });
            self.goto_rx = Some(rx);
        }
        if close_goto {
            self.goto_open = false;
        }
        match act {
            Some(PauseAct::Resume) => self.set_pause_menu(false),
            Some(PauseAct::Respawn) => {
                self.set_pause_menu(false);
                self.respawn_here();
            }
            Some(PauseAct::GoTo) => {
                self.set_pause_menu(false);
                self.goto_open = true;
                self.goto_text.clear();
            }
            Some(PauseAct::FlyTo(lat, lon, name)) => {
                self.set_pause_menu(false);
                self.fly_to(lat, lon, name);
            }
            Some(PauseAct::Globe) => self.to_globe(),
            Some(PauseAct::Quit) => self.quit = true,
            None => {}
        }
        let jobs = self.egui_ctx.tessellate(out.shapes, out.pixels_per_point);
        (jobs, out.textures_delta, out.pixels_per_point)
    }
}

struct App {
    args: Args,
    game: Option<Game>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.game.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Fable Flight").with_inner_size(winit::dpi::PhysicalSize::new(self.args.size.0, self.args.size.1));
        let window = Arc::new(el.create_window(attrs).expect("window"));
        match Game::new(window, &self.args) {
            Ok(g) => self.game = Some(g),
            Err(e) => {
                eprintln!("failed to start: {e:#}");
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(g) = self.game.as_mut() else { return };
        let consumed = g.egui_state.on_window_event(&g.window, &event).consumed;
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(s) => g.r.resize(s.width, s.height),
            WindowEvent::RedrawRequested => {
                g.frame();
                if g.quit {
                    // Worker threads may be mid-download, so the process just
                    // exits; but the GPU device is torn down first: exiting
                    // under the driver's shader compiler threads can crash it.
                    self.game = None;
                    std::process::exit(0);
                }
            }
            WindowEvent::Focused(false) => g.keys.clear(),
            WindowEvent::KeyboardInput { event: k, .. } => {
                if let PhysicalKey::Code(code) = k.physical_key {
                    if k.state == ElementState::Pressed {
                        // egui always claims Tab (and, while one of its buttons has
                        // focus, every key), so the menu keys are let through here.
                        let typing = g.goto_open || g.egui_ctx.text_edit_focused();
                        let menu_key = code == KeyCode::Tab || (g.pause_open && code == KeyCode::Escape);
                        if menu_key && !typing {
                            if !k.repeat {
                                g.key(code);
                            }
                        } else if !(consumed || g.goto_open) {
                            if !k.repeat {
                                g.key(code);
                            }
                            g.keys.insert(code);
                        }
                    } else {
                        g.keys.remove(&code);
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } if g.mode == Mode::Menu => {
                let size = g.window.inner_size();
                let (w, h) = (size.width.max(1) as f64, size.height.max(1) as f64);
                if state == ElementState::Pressed {
                    if !consumed {
                        g.menu.press = g.last_cursor;
                        g.menu.dragged = 0.0;
                    }
                } else if let Some((x, y)) = g.menu.press.take() {
                    // A click (not a drag) chooses the spot; a second click on it starts.
                    if g.menu.dragged < 6.0 {
                        if let Some((lat, lon)) = g.menu.pick_at(x, y, w, h, 60f64.to_radians()) {
                            let double = g.real - g.last_click < 0.45;
                            g.last_click = g.real;
                            if double && g.menu.pick.is_some() {
                                g.begin();
                            } else {
                                g.pick(lat, lon);
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left | MouseButton::Right, .. } if !consumed => {
                g.drag = state == ElementState::Pressed;
                g.last_cursor = None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if g.mode == Mode::Menu {
                    if let (Some(_), Some((x, y))) = (g.menu.press, g.last_cursor) {
                        let h = g.window.inner_size().height.max(1) as f64;
                        g.menu.drag(position.x - x, position.y - y, h, 60f64.to_radians());
                    }
                } else if g.drag && !g.pause_open {
                    if let Some((x, y)) = g.last_cursor {
                        g.cam_yaw -= (position.x - x) * 0.005 * g.look_sens;
                        g.cam_pitch = (g.cam_pitch + (position.y - y) * 0.004 * g.look_sens).clamp(-1.2, 1.4);
                    }
                }
                g.last_cursor = Some((position.x, position.y));
            }
            WindowEvent::MouseWheel { delta, .. } if !consumed => {
                let d = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                if g.mode == Mode::Menu {
                    g.menu.zoom(d);
                } else {
                    g.cam_dist = (g.cam_dist * (1.0 - d * 0.1)).clamp(6.0, 400.0);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(g) = &self.game {
            g.window.request_redraw();
        }
    }
}

/// The UI library ships no fonts of its own here, so borrow the system's.
fn load_fonts(ctx: &egui::Context) {
    let find = |kind: &str, fallbacks: &[&str]| -> Option<Vec<u8>> {
        let fc = std::process::Command::new("fc-match").args(["-f", "%{file}", kind]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok());
        fc.iter().map(|s| s.trim().to_string()).chain(fallbacks.iter().map(|s| s.to_string())).filter(|p| p.ends_with(".ttf") || p.ends_with(".otf")).find_map(|p| std::fs::read(p).ok())
    };
    let mut fonts = egui::FontDefinitions::empty();
    for (name, family, kind, fb) in [
        ("mono", egui::FontFamily::Monospace, "monospace", ["/usr/share/fonts/google-noto/NotoSansMono-Regular.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/usr/share/fonts/dejavu-sans-mono-fonts/DejaVuSansMono.ttf"]),
        ("sans", egui::FontFamily::Proportional, "sans", ["/usr/share/fonts/google-noto/NotoSans-Regular.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf"]),
    ] {
        if let Some(data) = find(kind, &fb) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(data)));
            fonts.families.entry(family).or_default().push(name.into());
        } else {
            log::warn!("no {kind} font found; HUD text will be missing");
        }
    }
    ctx.set_fonts(fonts);
}

/// Right-handed view matrix looking along `dir`.
fn look_to(eye: Vec3, dir: Vec3, up: Vec3) -> Mat4 {
    let f = dir.normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols(s.x_axis4(u, f, 0), s.x_axis4(u, f, 1), s.x_axis4(u, f, 2), glam::Vec4::new(-s.dot(eye), -u.dot(eye), f.dot(eye), 1.0))
}
trait Axis4 {
    fn x_axis4(self, u: Vec3, f: Vec3, i: usize) -> glam::Vec4;
}
impl Axis4 for Vec3 {
    fn x_axis4(self, u: Vec3, f: Vec3, i: usize) -> glam::Vec4 {
        glam::Vec4::new(self[i], u[i], -f[i], 0.0)
    }
}
/// Infinite far plane, reversed depth (1 at the near plane, 0 at infinity).
fn perspective_rev(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    let f = 1.0 / (fov_y * 0.5).tan();
    Mat4::from_cols_array(&[f / aspect, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, near, 0.0])
}
fn ortho(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Mat4 {
    Mat4::from_cols_array(&[2.0 / (r - l), 0.0, 0.0, 0.0, 0.0, 2.0 / (t - b), 0.0, 0.0, 0.0, 0.0, -1.0 / (f - n), 0.0, -(r + l) / (r - l), -(t + b) / (t - b), -n / (f - n), 1.0])
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,fablegame=info")).init();
    let el = EventLoop::new()?;
    el.set_control_flow(ControlFlow::Poll);
    el.run_app(&mut App { args: parse_args(), game: None })?;
    Ok(())
}


const REC_FPS: f64 = 30.0;

/// 16-bit stereo WAV.
fn write_wav(path: &std::path::Path, x: &[[f32; 2]], rate: u32) {
    let data = (x.len() * 4) as u32;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), 0x0002_0001u32.to_le_bytes(), rate.to_le_bytes(), (rate * 4).to_le_bytes(), 0x0010_0004u32.to_le_bytes()] {
        w.extend_from_slice(&v);
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in x {
        for c in s {
            w.extend_from_slice(&((c.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    if let Err(e) = std::fs::write(path, w) {
        log::error!("writing {}: {e}", path.display());
    }
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
