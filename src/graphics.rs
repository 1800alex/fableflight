//! Graphics settings: how hard the renderer works, as presets (Very Low, Low,
//! Medium, High, Ultra) or tuned one by one. High is what the game always drew
//! before these settings existed; Ultra goes past it (supersampling, a finer
//! shadow map, finer cloud steps, terrain and scenery further out). Very Low is
//! for integrated graphics (a laptop without a GPU of its own): it also turns
//! off the costs that do not shrink with the resolution (the cloud shadow map,
//! MSAA, the water's reflections and sea bed, traffic and birds). Saved in
//! `<config>/fablegame/graphics.json`.
//!
//! Where the time goes (RTX 3060, 1080p, `FABLE_GPU_TIMES=1`, over San
//! Francisco with scattered cloud): High is about 23 ms of GPU work a frame,
//! most of it the deferred composite (water: its reflections and sea bed,
//! 9 ms) and the g-buffer (7 ms), then the clouds (2 ms), the sun's shadow
//! cascades (1.5 ms) and the cloud shadow map (1.3 ms, whatever the
//! resolution). Low is about 6 ms, Very Low about 3.3 ms (2.5 ms with the
//! clouds off). In the cockpit the panel's own shading is most of what is
//! left on Very Low (about 1 ms of its 4.3).
//!
//! Google's photorealistic 3D tiles are not part of this: they are switched on
//! and given their own detail level in the map imagery settings (gmaps.rs).
use crate::i18n::t;
use crate::menu::{self, DIM};
use crate::settings_ui::{note, row};
use egui::RichText;
use std::path::PathBuf;

/// A quality step. `Off` only applies to the shadows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Off,
    Low,
    Medium,
    High,
    Ultra,
}

impl Level {
    pub const PRESETS: [Level; 4] = [Level::Low, Level::Medium, Level::High, Level::Ultra];

    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "Off",
            Level::Low => "Low",
            Level::Medium => "Medium",
            Level::High => "High",
            Level::Ultra => "Ultra",
        }
    }

    /// Its name on the settings screen, in the current language.
    pub fn label(self) -> &'static str {
        crate::i18n::tr(&format!("graphics.level_{}", self.name().to_ascii_lowercase()))
    }

    pub fn parse(s: &str) -> Option<Level> {
        [Level::Off, Level::Low, Level::Medium, Level::High, Level::Ultra].into_iter().find(|l| l.name().eq_ignore_ascii_case(s.trim()))
    }

    /// Ultra as High, for the settings that stop at High.
    fn min_high(self) -> Level {
        if self == Level::Ultra { Level::High } else { self }
    }

    fn pick(self, low: f64, medium: f64, high: f64, ultra: f64) -> f64 {
        match self {
            Level::Off | Level::Low => low,
            Level::Medium => medium,
            Level::High => high,
            Level::Ultra => ultra,
        }
    }
}

/// A preset: every quality setting at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    /// for integrated graphics: Low, plus off whatever costs the same at any
    /// resolution
    VeryLow,
    Low,
    Medium,
    High,
    Ultra,
}

impl Preset {
    pub const ALL: [Preset; 5] = [Preset::VeryLow, Preset::Low, Preset::Medium, Preset::High, Preset::Ultra];

    pub fn name(self) -> &'static str {
        match self {
            Preset::VeryLow => "Very Low",
            Preset::Low => "Low",
            Preset::Medium => "Medium",
            Preset::High => "High",
            Preset::Ultra => "Ultra",
        }
    }

    pub fn label(self) -> &'static str {
        crate::i18n::tr(&format!("graphics.level_{}", self.name().replace(' ', "_").to_ascii_lowercase()))
    }

    /// "very low", "very-low", "verylow" and "very_low" all name Very Low.
    pub fn parse(s: &str) -> Option<Preset> {
        let k: String = s.trim().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        Preset::ALL.into_iter().find(|p| p.name().replace(' ', "").eq_ignore_ascii_case(&k))
    }

    /// The level each quality setting gets.
    fn level(self) -> Level {
        match self {
            Preset::VeryLow | Preset::Low => Level::Low,
            Preset::Medium => Level::Medium,
            Preset::High => Level::High,
            Preset::Ultra => Level::Ultra,
        }
    }
}

/// Set once at startup when the GPU shares the CPU's memory (graphics.rs
/// only uses it to suggest a preset).
static INTEGRATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_integrated(on: bool) {
    INTEGRATED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The preset a first start picks on an integrated GPU (`vendor`: PCI id):
/// Very Low on Intel's, Low on the others (AMD's APUs, Apple's chips).
pub fn integrated_preset(vendor: u32) -> Preset {
    if vendor == 0x8086 { Preset::VeryLow } else { Preset::Low }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Graphics {
    /// render resolution as a share of the window's (each side): below 1 is
    /// upscaled, above 1 supersampled
    pub scale: f32,
    pub shadows: Level,
    /// the volumetric clouds (Off: none drawn, though the weather still flies)
    pub clouds: Level,
    /// the clouds' shadows on the ground: a map redrawn a slice a frame, the
    /// same cost at any resolution
    pub cloud_shadows: bool,
    /// storm cell rendering detail (separate from ordinary clouds so heavy
    /// thunderstorm, supercell and squall-line skies can be tuned on their own)
    pub storms: Level,
    pub terrain: Level,
    /// how far out buildings and trees are loaded
    pub scenery: Level,
    /// how far out buildings keep their full geometry, dressing and window depth
    pub city: Level,
    /// how far out trees are drawn, and how near they turn into impostors
    pub trees: Level,
    /// moving and parked cars (Off, or Low to High; Ultra is High)
    pub cars: Level,
    /// birds in the air (birds.rs; Off, or Low to High; Ultra is High)
    pub birds: Level,
    /// ambient occlusion (GTAO)
    pub ao: bool,
    /// water: Low plain (a flat sea bed, simple foam, one reflected cloud
    /// sample), Medium all of it, High also the scenery reflected on screen
    /// (and in glass)
    pub water: Level,
    /// g-buffer MSAA samples (1, 2 or 4), applied on the next start
    pub msaa: u8,
    // Not part of the presets: preferences more than costs.
    /// temporal anti-aliasing (F2)
    pub taa: bool,
    pub vsync: bool,
    /// Optional frame limit (0 means no software limit).
    pub fps_cap: u16,
}

impl Default for Graphics {
    fn default() -> Self {
        Self::preset(Preset::High)
    }
}

/// (35% is for the weakest integrated graphics: blurry, but it flies)
pub const SCALE_MIN: f32 = 0.35;
pub const SCALE_MAX: f32 = 2.0;

impl Graphics {
    /// One of the presets (taa and vsync on).
    pub fn preset(p: Preset) -> Self {
        let mut g = Graphics { scale: 1.0, shadows: Level::High, clouds: Level::High, cloud_shadows: true, storms: Level::High, terrain: Level::High, scenery: Level::High, city: Level::High, trees: Level::High, cars: Level::High, birds: Level::High, ao: true, water: Level::High, msaa: 4, taa: true, vsync: true, fps_cap: 0 };
        g.apply(p);
        g
    }

    /// Sets everything a preset covers (not taa, vsync or the frame limit).
    pub fn apply(&mut self, p: Preset) {
        let l = p.level();
        let very_low = p == Preset::VeryLow;
        // each preset down roughly halves gpu work, resolution first, then shadows.
        self.scale = if very_low { 0.5 } else { l.pick(0.6, 0.75, 1.0, 1.5) as f32 };
        self.shadows = match l {
            Level::Off | Level::Low => Level::Off,
            Level::Medium => Level::Low,
            h => h,
        };
        self.clouds = l;
        self.cloud_shadows = !very_low;
        self.storms = l;
        self.terrain = l;
        self.scenery = l;
        self.city = l;
        self.trees = l;
        let capped = if l == Level::Ultra { Level::High } else { l };
        self.cars = if very_low { Level::Off } else { capped };
        self.birds = if very_low { Level::Off } else { capped };
        self.ao = matches!(l, Level::High | Level::Ultra);
        // (Low has always gone without the reflections on screen)
        self.water = match p {
            Preset::VeryLow => Level::Low,
            Preset::Low => Level::Medium,
            _ => Level::High,
        };
        self.msaa = match p {
            Preset::VeryLow => 1,
            Preset::Low => 2,
            _ => 4,
        };
    }

    /// The preset these settings are, if any.
    pub fn matches(&self) -> Option<Preset> {
        Preset::ALL.into_iter().find(|l| {
            let mut p = *self;
            p.apply(*l);
            p == *self
        })
    }

    /// Whether the volumetric clouds are drawn at all.
    pub fn clouds_on(&self) -> bool {
        self.clouds != Level::Off
    }

    /// The water's detail for the composite (Globals.lite.x, composite.wgsl
    /// `water`): 0 plain, 1 full, 2 full with screen-space reflections.
    pub fn water_detail(&self) -> f32 {
        match self.water {
            Level::Off | Level::Low => 0.0,
            Level::Medium => 1.0,
            Level::High | Level::Ultra => 2.0,
        }
    }

    /// Screen-space reflections (of the scenery in water and glass).
    pub fn ssr(&self) -> bool {
        self.water_detail() >= 2.0
    }

    /// Frames a redraw of the cloud shadow map is spread over (render.rs
    /// `CloudShadow`): the slow refresh integrated GPUs get, for Low clouds too.
    pub fn cloud_shadow_slices(&self) -> u32 {
        if matches!(self.clouds, Level::Off | Level::Low) { 64 } else { 16 }
    }

    /// Side of the sun's shadow map (texels; Off keeps a small unused one).
    pub fn shadow_res(&self) -> u32 {
        match self.shadows {
            Level::Off => 256,
            Level::Low => 1024,
            Level::Medium => 2048,
            Level::High => 4096,
            Level::Ultra => 6144,
        }
    }

    /// Length of the cloud raymarch's steps relative to High (clouds.wgsl).
    pub fn cloud_step(&self) -> f32 {
        self.clouds.pick(1.8, 1.35, 1.0, 0.7) as f32
    }

    /// Storm cell quality tier passed in Globals.detail.w (1 Low .. 4 Ultra;
    /// 0 in WGSL falls back to High).
    pub fn storm_tier(&self) -> f32 {
        match self.storms {
            Level::Off | Level::Low => 1.0,
            Level::Medium => 2.0,
            Level::High => 3.0,
            Level::Ultra => 4.0,
        }
    }

    /// Maximum nearby storm cells uploaded for rendering (nearest first; flight
    /// physics and weather radar still use all cells in `Weather::cells`).
    pub fn storm_max_cells(&self) -> usize {
        match self.storms {
            Level::Off | Level::Low => 8,
            Level::Medium => 12,
            Level::High | Level::Ultra => crate::weather::MAX_STORMS,
        }
    }

    /// Terrain tile split threshold (tile width / distance, world.rs `lod_k`):
    /// lower splits sooner, so finer tiles reach further.
    pub fn lod_k(&self) -> f64 {
        self.terrain.pick(0.95, 0.78, 0.62, 0.5)
    }

    /// Multiplies how far out building (and tree) tiles are loaded.
    pub fn reach(&self) -> f64 {
        self.scenery.pick(0.6, 0.8, 1.0, 1.3)
    }

    /// Terrain tiles kept loaded at most, before the least recently drawn go
    /// (High and Ultra as the game always kept). Each is about 1.6 MB of GPU
    /// memory (imagery, mask, lights, mesh), which an integrated GPU takes
    /// from the system's.
    pub fn tile_cap(&self) -> usize {
        self.terrain.pick(300.0, 450.0, 700.0, 700.0) as usize
    }

    /// Building tiles made at once (each holds 150-250 MB in a city until it
    /// is on the GPU: twelve at once took a 16 GB laptop out of memory).
    pub fn max_build_jobs(&self) -> usize {
        self.scenery.pick(3.0, 5.0, 10.0, 12.0) as usize
    }

    /// Building tiles kept loaded once out of reach, for turning back.
    pub fn spare_builds(&self) -> usize {
        self.scenery.pick(3.0, 6.0, 12.0, 16.0) as usize
    }

    /// Multiplies the distances buildings change level of detail at (their
    /// dissolve into far stand-ins, the dressing, the window depth close up).
    pub fn city_k(&self) -> f32 {
        self.city.pick(0.45, 0.7, 1.0, 1.4) as f32
    }

    /// How far out trees are drawn (m); the last fifth dissolves away.
    pub fn tree_far(&self) -> f32 {
        self.trees.pick(1400.0, 2200.0, 3200.0, 4500.0) as f32
    }

    /// Multiplies the distance trees turn from models into impostors at.
    /// Never above 1: the leaf cards and the impostor bake are made for it.
    pub fn tree_imp_k(&self) -> f32 {
        self.trees.pick(0.5, 0.75, 1.0, 1.0) as f32
    }

    /// Share of the cars on the roads (0..1, by each car's own rank) and how
    /// far off moving and parked cars are drawn (m).
    pub fn car_density(&self) -> (f32, f64, f64) {
        match self.cars {
            Level::Off => (0.0, 0.0, 0.0),
            Level::Low => (0.35, 900.0, 450.0),
            Level::Medium => (0.65, 1500.0, 750.0),
            Level::High | Level::Ultra => (1.0, 2200.0, 1100.0),
        }
    }

    /// Whether the settings have ever been saved (else the defaults may
    /// adapt to the machine, e.g. a lower resolution on an integrated GPU).
    pub fn saved() -> bool {
        Self::path().is_some_and(|p| p.exists())
    }

    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("fablegame").join("graphics.json"))
    }

    /// The saved settings, or High.
    pub fn load() -> Self {
        let j: serde_json::Value = Self::path().and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
        Self::from_json(&j)
    }

    fn from_json(j: &serde_json::Value) -> Self {
        let mut g = Self::default();
        g.fps_cap = j["fps_cap"].as_u64().map(|v| v as u16).filter(|v| [0, 30, 60, 90, 120, 144, 165, 240].contains(v)).unwrap_or(0);
        // a saved preset follows the code, so retuned presets reach old saves.
        if let Some(l) = j["preset"].as_str().and_then(Preset::parse) {
            g.apply(l);
            g.taa = j["taa"].as_bool().unwrap_or(g.taa);
            g.vsync = j["vsync"].as_bool().unwrap_or(g.vsync);
            return g;
        }
        let lvl = |k: &str, d: Level| j[k].as_str().and_then(Level::parse).filter(|l| *l != Level::Off).unwrap_or(d);
        if let Some(s) = j["scale"].as_f64() {
            g.scale = (s as f32).clamp(SCALE_MIN, SCALE_MAX);
        }
        g.shadows = j["shadows"].as_str().and_then(Level::parse).unwrap_or(g.shadows);
        g.clouds = j["clouds"].as_str().and_then(Level::parse).unwrap_or(g.clouds);
        g.cloud_shadows = j["cloud_shadows"].as_bool().unwrap_or(g.cloud_shadows);
        g.storms = lvl("storms", if g.clouds == Level::Off { Level::Low } else { g.clouds });
        g.terrain = lvl("terrain", g.terrain);
        g.scenery = lvl("scenery", g.scenery);
        g.city = lvl("city", g.city);
        g.trees = lvl("trees", g.trees);
        g.cars = j["cars"].as_str().and_then(Level::parse).map(|l| if l == Level::Ultra { Level::High } else { l }).unwrap_or(g.cars);
        g.birds = j["birds"].as_str().and_then(Level::parse).map(|l| if l == Level::Ultra { Level::High } else { l }).unwrap_or(g.birds);
        g.ao = j["ao"].as_bool().unwrap_or(g.ao);
        g.water = lvl("water", g.water).min_high();
        g.msaa = j["msaa"].as_u64().filter(|n| [1, 2, 4].contains(n)).map(|n| n as u8).unwrap_or(g.msaa);
        g.taa = j["taa"].as_bool().unwrap_or(g.taa);
        g.vsync = j["vsync"].as_bool().unwrap_or(g.vsync);
        g
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "preset": self.matches().map(Preset::name).unwrap_or("custom"),
            "scale": self.scale,
            "shadows": self.shadows.name(),
            "clouds": self.clouds.name(),
            "cloud_shadows": self.cloud_shadows,
            "storms": self.storms.name(),
            "terrain": self.terrain.name(),
            "scenery": self.scenery.name(),
            "city": self.city.name(),
            "trees": self.trees.name(),
            "cars": self.cars.name(),
            "birds": self.birds.name(),
            "ao": self.ao,
            "water": self.water.name(),
            "msaa": self.msaa,
            "taa": self.taa,
            "vsync": self.vsync,
            "fps_cap": self.fps_cap,
        })
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let j = self.to_json();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&j).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// The preset row and what the chosen preset does (the settings
    /// screen's Graphics section, settings_ui.rs).
    pub fn preset_ui(&mut self, ui: &mut egui::Ui) {
        row(ui, t!("graphics.preset"), t!("graphics.preset_hint"), |ui| {
            ui.horizontal(|ui| {
                let cur = self.matches();
                let mut pick = cur;
                menu::segments(ui, |ui| {
                    for p in Preset::ALL {
                        menu::seg(ui, &mut pick, Some(p), p.label());
                    }
                });
                if let (Some(p), true) = (pick, pick != cur) {
                    self.apply(p);
                }
                if cur.is_none() {
                    ui.label(RichText::new(t!("graphics.custom")).font(menu::label()).color(DIM));
                }
            });
        });
        note(ui, match self.matches() {
            Some(Preset::VeryLow) => t!("graphics.very_low_note"),
            Some(Preset::Low) => t!("graphics.low_note"),
            Some(Preset::Medium) => t!("graphics.medium_note"),
            Some(Preset::High) => t!("graphics.high_note"),
            Some(Preset::Ultra) => t!("graphics.ultra_note"),
            _ => t!("graphics.custom_note"),
        });
        if INTEGRATED.load(std::sync::atomic::Ordering::Relaxed) && matches!(self.matches(), Some(Preset::Medium | Preset::High | Preset::Ultra) | None) {
            note(ui, t!("graphics.integrated_note"));
        }
    }

    /// Shadows, clouds, terrain, scenery, cities, trees, traffic, AO, water,
    /// MSAA: what the presets set.
    pub fn quality_ui(&mut self, ui: &mut egui::Ui) {
        let levels = |ui: &mut egui::Ui, v: &mut Level, off: bool| {
            menu::segments(ui, |ui| {
                if off {
                    menu::seg(ui, v, Level::Off, Level::Off.label());
                }
                for l in Level::PRESETS {
                    menu::seg(ui, v, l, l.label());
                }
            });
        };
        let dim = |ui: &mut egui::Ui, s: String| {
            ui.label(RichText::new(s).font(menu::caption()).color(DIM));
        };
        row(ui, t!("graphics.shadows"), "", |ui| {
            levels(ui, &mut self.shadows, true);
            if self.shadows != Level::Off {
                let r = self.shadow_res();
                dim(ui, t!("graphics.shadow_map", r = r));
            }
        });
        row(ui, t!("graphics.clouds"), "", |ui| {
            levels(ui, &mut self.clouds, true);
            if self.clouds == Level::Off {
                dim(ui, t!("graphics.clouds_off").into());
            }
        });
        row(ui, t!("graphics.cloud_shadows"), t!("graphics.cloud_shadows_hint"), |ui| {
            ui.add_enabled_ui(self.clouds_on(), |ui| menu::switch(ui, &mut self.cloud_shadows));
            if self.clouds_on() && self.cloud_shadows {
                dim(ui, if self.cloud_shadow_slices() > 16 { t!("graphics.cloud_shadows_slow").into() } else { t!("graphics.cloud_shadows_16").into() });
            }
        });
        row(ui, t!("graphics.storm_cells"), t!("graphics.storm_cells_hint"), |ui| {
            levels(ui, &mut self.storms, false);
            dim(ui, match self.storms {
                Level::Off | Level::Low => t!("graphics.storms_low").into(),
                Level::Medium => t!("graphics.storms_medium").into(),
                Level::High => t!("graphics.storms_high").into(),
                Level::Ultra => t!("graphics.storms_ultra").into(),
            });
        });
        row(ui, t!("graphics.terrain_detail"), "", |ui| levels(ui, &mut self.terrain, false));
        row(ui, t!("graphics.scenery_distance"), "", |ui| {
            levels(ui, &mut self.scenery, false);
            dim(ui, t!("graphics.scenery_reach", 7.5 * self.reach()));
        });
        row(ui, t!("graphics.city_detail"), "", |ui| {
            levels(ui, &mut self.city, false);
            let k = self.city_k();
            dim(ui, t!("graphics.city_reach", 1.3 * k, 550.0 * k));
        });
        row(ui, t!("graphics.tree_detail"), "", |ui| {
            levels(ui, &mut self.trees, false);
            dim(ui, t!("graphics.tree_reach", self.tree_far() / 1000.0, 120.0 * self.tree_imp_k()));
        });
        row(ui, t!("graphics.traffic"), t!("graphics.traffic_hint"), |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Off, Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.cars, l, l.label());
                }
            });
            let (share, far, _) = self.car_density();
            dim(ui, if share > 0.0 { t!("graphics.traffic_share", share * 100.0, far / 1000.0) } else { t!("graphics.no_cars").into() });
        });
        row(ui, t!("graphics.birds"), t!("graphics.birds_hint"), |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Off, Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.birds, l, l.label());
                }
            });
            let (flocks, birds) = crate::birds::caps(self.birds);
            dim(ui, if flocks > 0 { t!("graphics.birds_caps", flocks, birds) } else { t!("graphics.no_birds").to_string() });
        });
        row(ui, t!("graphics.ambient_occlusion"), t!("graphics.ambient_occlusion_hint"), |ui| {
            menu::switch(ui, &mut self.ao);
        });
        row(ui, t!("graphics.water"), t!("graphics.water_hint"), |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.water, l, l.label());
                }
            });
            dim(ui, match self.water {
                Level::Off | Level::Low => t!("graphics.water_low").into(),
                Level::Medium => t!("graphics.water_medium").into(),
                Level::High | Level::Ultra => t!("graphics.water_high").into(),
            });
        });
        row(ui, t!("graphics.msaa"), t!("graphics.msaa_hint"), |ui| {
            menu::segments(ui, |ui| {
                for (n, name) in [(1u8, t!("graphics.level_off")), (2, "2×"), (4, "4×")] {
                    menu::seg(ui, &mut self.msaa, n, name);
                }
            });
            let now = crate::render::msaa();
            if self.msaa as u32 != now {
                dim(ui, t!("graphics.msaa_restart", if now == 1 { t!("graphics.level_off").to_string() } else { format!("{now}×") }));
            }
        });
    }

    /// Render resolution, anti-aliasing, vsync and the frame limit. `window`
    /// is the window's size in pixels (for the resolution readout), `fps`
    /// the frame rate now.
    pub fn display_ui(&mut self, ui: &mut egui::Ui, window: (u32, u32), fps: f64) {
        row(ui, t!("graphics.render_resolution"), t!("graphics.render_resolution_hint"), |ui| {
            let (w, h) = window;
            let (rw, rh) = ((w as f32 * self.scale).round() as u32, (h as f32 * self.scale).round() as u32);
            ui.add(egui::Slider::new(&mut self.scale, SCALE_MIN..=SCALE_MAX).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            let what = if self.scale > 1.001 { t!("graphics.supersampled") } else if self.scale < 0.999 { t!("graphics.upscaled") } else { t!("graphics.native") };
            ui.label(RichText::new(format!("{rw}×{rh}, {what}")).font(menu::caption()).color(DIM));
        });
        row(ui, t!("graphics.anti_aliasing"), t!("graphics.anti_aliasing_hint"), |ui| {
            menu::switch(ui, &mut self.taa);
        });
        row(ui, "VSync", if self.vsync { t!("graphics.vsync_on") } else { t!("graphics.vsync_off") }, |ui| {
            menu::switch(ui, &mut self.vsync);
        });
        row(ui, t!("graphics.frame_limit"), t!("graphics.frame_limit_hint"), |ui| {
            menu::segments(ui, |ui| {
                for cap in [0u16, 30, 60, 90, 120, 144, 165, 240] {
                    menu::seg(ui, &mut self.fps_cap, cap, &if cap == 0 { t!("graphics.unlimited").to_owned() } else { cap.to_string() });
                }
            });
        });
        note(ui, &t!("graphics.fps_now", fps = fps));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip() {
        for p in Preset::ALL {
            assert_eq!(Graphics::preset(p).matches(), Some(p));
            assert_eq!(Graphics::from_json(&Graphics::preset(p).to_json()), Graphics::preset(p));
            assert_eq!(Preset::parse(p.name()), Some(p));
        }
        assert_eq!(Preset::parse("very-low"), Some(Preset::VeryLow));
        assert_eq!(Preset::parse("VERY_LOW"), Some(Preset::VeryLow));
        assert_eq!(Preset::parse("off"), None);
        let mut g = Graphics::preset(Preset::High);
        g.taa = false;
        g.vsync = false;
        assert_eq!(g.matches(), Some(Preset::High), "taa and vsync are not part of a preset");
        g.scale = 1.2;
        assert_eq!(g.matches(), None);
    }

    #[test]
    fn custom_round_trip() {
        let mut g = Graphics::preset(Preset::Medium);
        g.clouds = Level::Off;
        g.cloud_shadows = false;
        g.water = Level::Low;
        g.msaa = 1;
        g.fps_cap = 30;
        assert_eq!(g.matches(), None);
        assert_eq!(Graphics::from_json(&g.to_json()), g);
        assert!(!g.clouds_on());
    }

    #[test]
    fn high_is_the_old_look() {
        let g = Graphics::default();
        assert_eq!((g.scale, g.shadow_res(), g.cloud_step(), g.lod_k(), g.reach(), g.ao), (1.0, 4096, 1.0, 0.62, 1.0, true));
        assert_eq!((g.storms, g.storm_tier(), g.storm_max_cells()), (Level::High, 3.0, crate::weather::MAX_STORMS));
        assert_eq!((g.city_k(), g.tree_far(), g.tree_imp_k(), g.car_density()), (1.0, 3200.0, 1.0, (1.0, 2200.0, 1100.0)));
        assert_eq!((g.clouds_on(), g.cloud_shadows, g.cloud_shadow_slices(), g.msaa, g.ssr(), g.water_detail()), (true, true, 16, 4, true, 2.0));
    }

    #[test]
    fn low_keeps_its_old_look() {
        // Low always had 2x MSAA and no screen-space reflections.
        let g = Graphics::preset(Preset::Low);
        assert_eq!((g.msaa, g.ssr(), g.water_detail(), g.cloud_shadows), (2, false, 1.0, true));
    }

    #[test]
    fn very_low_drops_the_fixed_costs() {
        let g = Graphics::preset(Preset::VeryLow);
        assert_eq!((g.scale, g.shadows, g.ao, g.msaa, g.cloud_shadows, g.water_detail()), (0.5, Level::Off, false, 1, false, 0.0));
        assert_eq!((g.cars, g.birds, g.car_density().0), (Level::Off, Level::Off, 0.0));
        assert!(g.clouds_on(), "Very Low keeps the clouds (Off is a choice of its own)");
        assert_eq!(integrated_preset(0x8086), Preset::VeryLow);
        assert_eq!(integrated_preset(0x1002), Preset::Low);
    }

    #[test]
    fn old_saves_load_as_before() {
        // A custom save from before Very Low, the water and MSAA settings.
        let j = serde_json::json!({"preset": "custom", "scale": 0.9, "shadows": "High", "clouds": "Ultra", "storms": "Medium", "ao": true, "fps_cap": 30});
        let g = Graphics::from_json(&j);
        assert_eq!((g.clouds, g.storms, g.cloud_shadows, g.water, g.msaa, g.fps_cap), (Level::Ultra, Level::Medium, true, Level::High, 4, 30));
        assert_eq!(Graphics::from_json(&serde_json::json!({"preset": "Low"})), Graphics::preset(Preset::Low));
    }

    #[test]
    fn storm_quality_independent_of_clouds() {
        let mut g = Graphics::preset(Preset::High);
        g.storms = Level::Medium;
        assert_eq!(g.matches(), None, "changing storm quality independently makes a custom profile");
        assert_eq!((g.clouds, g.storms, g.storm_tier(), g.storm_max_cells()), (Level::High, Level::Medium, 2.0, 12));
        g.storms = Level::Low;
        assert_eq!((g.storm_tier(), g.storm_max_cells()), (1.0, 8));
        g.storms = Level::Ultra;
        assert_eq!((g.storm_tier(), g.storm_max_cells()), (4.0, crate::weather::MAX_STORMS));
    }
}
