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
        row(ui, "Preset", "sets everything below at once", |ui| {
            ui.horizontal(|ui| {
                let cur = self.matches();
                let mut pick = cur;
                menu::segments(ui, |ui| {
                    for p in Preset::ALL {
                        menu::seg(ui, &mut pick, Some(p), p.name());
                    }
                });
                if let (Some(p), true) = (pick, pick != cur) {
                    self.apply(p);
                }
                if cur.is_none() {
                    ui.label(RichText::new("Custom").font(menu::label()).color(DIM));
                }
            });
        });
        note(ui, match self.matches() {
            Some(Preset::VeryLow) => "Very Low: for integrated graphics and older laptops. 50% resolution, no MSAA, no shadows or cloud shadows, plain water, nearer scenery, no traffic or birds. A 30 fps frame limit below also saves battery.",
            Some(Preset::Low) => "Low: 60% resolution, no shadows, quicker clouds, nearer scenery, simpler cities and trees, light traffic, no ambient occlusion or reflections.",
            Some(Preset::Medium) => "Medium: 75% resolution, a coarse shadow map, no ambient occlusion and a lighter version of everything else.",
            Some(Preset::High) => "High: the full look at the window's own resolution (what the game has always drawn).",
            Some(Preset::Ultra) => "Ultra: past High. Supersampled at 150%, a 6K shadow map, finer clouds, terrain and scenery further out. Wants a strong GPU.",
            _ => "Custom: tuned one by one below.",
        });
        if INTEGRATED.load(std::sync::atomic::Ordering::Relaxed) && matches!(self.matches(), Some(Preset::Medium | Preset::High | Preset::Ultra) | None) {
            note(ui, "This computer's graphics share its memory (integrated graphics): Very Low or Low should run far more smoothly.");
        }
    }

    /// Shadows, clouds, terrain, scenery, cities, trees, traffic, AO, water,
    /// MSAA: what the presets set.
    pub fn quality_ui(&mut self, ui: &mut egui::Ui) {
        let levels = |ui: &mut egui::Ui, v: &mut Level, off: bool| {
            menu::segments(ui, |ui| {
                if off {
                    menu::seg(ui, v, Level::Off, "Off");
                }
                for l in Level::PRESETS {
                    menu::seg(ui, v, l, l.name());
                }
            });
        };
        let dim = |ui: &mut egui::Ui, s: String| {
            ui.label(RichText::new(s).font(menu::caption()).color(DIM));
        };
        row(ui, "Shadows", "", |ui| {
            levels(ui, &mut self.shadows, true);
            if self.shadows != Level::Off {
                let r = self.shadow_res();
                dim(ui, format!("{r}×{r} map"));
            }
        });
        row(ui, "Clouds", "", |ui| {
            levels(ui, &mut self.clouds, true);
            if self.clouds == Level::Off {
                dim(ui, "no clouds drawn (rain, wind and turbulence stay)".into());
            }
        });
        row(ui, "Cloud shadows", "the clouds' shade on the ground", |ui| {
            ui.add_enabled_ui(self.clouds_on(), |ui| menu::switch(ui, &mut self.cloud_shadows));
            if self.clouds_on() && self.cloud_shadows {
                dim(ui, if self.cloud_shadow_slices() > 16 { "refreshed slowly (cheaper)".into() } else { "refreshed every 16 frames".into() });
            }
        });
        row(ui, "Storm cells", "thunderstorms, supercells, squall lines", |ui| {
            levels(ui, &mut self.storms, false);
            dim(ui, match self.storms {
                Level::Off | Level::Low => "lighter lighting and detail, up to 8 nearest cells".into(),
                Level::Medium => "faster march and lighting, up to 12 nearest cells".into(),
                Level::High => "full billows, lighting and up to 16 cells".into(),
                Level::Ultra => "finer steps and full storm detail".into(),
            });
        });
        row(ui, "Terrain detail", "", |ui| levels(ui, &mut self.terrain, false));
        row(ui, "Scenery distance", "", |ui| {
            levels(ui, &mut self.scenery, false);
            dim(ui, format!("buildings and trees out to {:.1} km", 7.5 * self.reach()));
        });
        row(ui, "City detail", "", |ui| {
            levels(ui, &mut self.city, false);
            let k = self.city_k();
            dim(ui, format!("full buildings out to {:.1} km (towers further), lit window depth to {:.0} m", 1.3 * k, 550.0 * k));
        });
        row(ui, "Tree detail", "", |ui| {
            levels(ui, &mut self.trees, false);
            dim(ui, format!("trees out to {:.1} km, full models to {:.0} m", self.tree_far() / 1000.0, 120.0 * self.tree_imp_k()));
        });
        row(ui, "Traffic", "cars on the roads", |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Off, Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.cars, l, l.name());
                }
            });
            let (share, far, _) = self.car_density();
            dim(ui, if share > 0.0 { format!("{:.0}% of the cars, out to {:.1} km", share * 100.0, far / 1000.0) } else { "no cars on the roads".into() });
        });
        row(ui, crate::i18n::t!("Birds", "Oiseaux"), crate::i18n::t!("gulls, crows, starlings, buzzards and geese, by place, season and hour", "goélands, corneilles, étourneaux, buses et oies, selon le lieu, la saison et l'heure"), |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Off, Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.birds, l, l.name());
                }
            });
            let (flocks, birds) = crate::birds::caps(self.birds);
            dim(ui, if flocks > 0 { crate::i18n::t!("up to {} flocks, {} birds", "jusqu'à {} volées, {} oiseaux", flocks, birds) } else { crate::i18n::t!("no birds", "aucun oiseau").to_string() });
        });
        row(ui, "Ambient occlusion", "contact shadows (GTAO)", |ui| {
            menu::switch(ui, &mut self.ao);
        });
        row(ui, "Water", "waves, sea bed, foam and reflections", |ui| {
            menu::segments(ui, |ui| {
                for l in [Level::Low, Level::Medium, Level::High] {
                    menu::seg(ui, &mut self.water, l, l.name());
                }
            });
            dim(ui, match self.water {
                Level::Off | Level::Low => "plain: waves, sky and softer cloud reflections, a flat sea bed, simple foam".into(),
                Level::Medium => "full detail, sky and clouds reflected".into(),
                Level::High | Level::Ultra => "full detail, buildings and shores reflected too".into(),
            });
        });
        row(ui, "MSAA", "smooths edges; Off leaves it to TAA", |ui| {
            menu::segments(ui, |ui| {
                for (n, name) in [(1u8, "Off"), (2, "2×"), (4, "4×")] {
                    menu::seg(ui, &mut self.msaa, n, name);
                }
            });
            let now = crate::render::msaa();
            if self.msaa as u32 != now {
                dim(ui, format!("now {}; a change applies after a restart", if now == 1 { "off".to_string() } else { format!("{now}×") }));
            }
        });
    }

    /// Render resolution, anti-aliasing, vsync and the frame limit. `window`
    /// is the window's size in pixels (for the resolution readout), `fps`
    /// the frame rate now.
    pub fn display_ui(&mut self, ui: &mut egui::Ui, window: (u32, u32), fps: f64) {
        row(ui, "Render resolution", "below 100% upscales, above supersamples", |ui| {
            let (w, h) = window;
            let (rw, rh) = ((w as f32 * self.scale).round() as u32, (h as f32 * self.scale).round() as u32);
            ui.add(egui::Slider::new(&mut self.scale, SCALE_MIN..=SCALE_MAX).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            let what = if self.scale > 1.001 { "supersampled" } else if self.scale < 0.999 { "upscaled" } else { "native" };
            ui.label(RichText::new(format!("{rw}×{rh}, {what}")).font(menu::caption()).color(DIM));
        });
        row(ui, "Anti-aliasing", "temporal (TAA) · F2 in flight", |ui| {
            menu::switch(ui, &mut self.taa);
        });
        row(ui, "VSync", if self.vsync { "no tearing" } else { "uncapped, may tear" }, |ui| {
            menu::switch(ui, &mut self.vsync);
        });
        row(ui, "Frame limit", "frames per second", |ui| {
            menu::segments(ui, |ui| {
                for cap in [0u16, 30, 60, 90, 120, 144, 165, 240] {
                    menu::seg(ui, &mut self.fps_cap, cap, &if cap == 0 { "Unlimited".to_owned() } else { cap.to_string() });
                }
            });
        });
        note(ui, &format!("Now {fps:.0} fps. Changes apply at once; scenery and terrain fill in over a few seconds."));
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
