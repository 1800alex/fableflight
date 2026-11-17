//! The settings screen: one full-window page shared by the start screen and
//! the pause menu. A sidebar of sections on the left (painted icons, no
//! glyphs the bundled font might not have), the chosen section's cards on
//! the right in a column of at most `MAX_W` points.
//!
//! Entry point: [`show`] (or `Game::settings_screen`). It draws the screen
//! over everything and returns true when it is closed with its Back button;
//! Esc and the gamepad's B are handled by the caller's key handling (they
//! close it the same way). The section shown is `Game::settings_sec`.
//!
//! Every setting lives here except the ones that only make sense in flight
//! (failures, in the pause menu). The values themselves are saved where
//! they always were: settings.json (settings.rs, compared each frame by
//! `keep_settings`), graphics.json (graphics.rs), bindings (bind.rs) and the
//! Google key (gmaps.rs).
use crate::menu::{self, ACCENT, DIM, TEXT};
use crate::padui::{self, Hint};
use crate::{bind, gmaps, i18n, t, tiles3d, Game};
use egui::{pos2, vec2, Align, Align2, Color32, FontId, Layout, Painter, Pos2, Rect, RichText, Sense, Stroke, Ui};

/// The widest the content column gets, however wide the window.
pub const MAX_W: f32 = 900.0;

/// A section of the settings screen.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Section {
    General,
    Graphics,
    Sound,
    Camera,
    Controls,
    Map,
}

impl Section {
    pub const ALL: [Section; 6] = [Section::General, Section::Graphics, Section::Sound, Section::Camera, Section::Controls, Section::Map];

    /// The name the command line goes by (`--settings graphics`, `--pause-menu settings:graphics`).
    pub fn name(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Graphics => "graphics",
            Section::Sound => "sound",
            Section::Camera => "camera",
            Section::Controls => "controls",
            Section::Map => "map",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::General => t!("General", "Général"),
            Section::Graphics => t!("Graphics", "Graphismes"),
            Section::Sound => t!("Sound", "Son"),
            Section::Camera => t!("Camera & View", "Caméra et vue"),
            Section::Controls => t!("Controls", "Commandes"),
            Section::Map => t!("Map & Data", "Carte et données"),
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            Section::General => t!("Language, the HUD, the minimap and live traffic.", "Langue, HUD, mini-carte et trafic réel."),
            Section::Graphics => t!("How hard the renderer works. Changes apply at once.", "L'effort demandé au rendu. Les changements s'appliquent aussitôt."),
            Section::Sound => t!("Engine, wind, warnings and the world around you.", "Moteur, vent, alarmes et le monde autour de vous."),
            Section::Camera => t!("Where the camera starts and how wide the cockpit view is.", "Le point de vue de départ et l'ouverture de la vue cockpit."),
            Section::Controls => t!("Mouse, joysticks, pedals, gamepads and keys.", "Souris, joysticks, palonniers, manettes et touches."),
            Section::Map => t!("Map imagery, Google's 3D cities and real-world data.", "Imagerie, villes 3D de Google et données réelles."),
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        Some(match s.trim().to_lowercase().as_str() {
            "general" | "game" | "hud" | "language" => Section::General,
            "graphics" | "gfx" | "video" => Section::Graphics,
            "sound" | "audio" => Section::Sound,
            "camera" | "view" | "cam" => Section::Camera,
            "controls" | "bindings" | "input" | "keys" => Section::Controls,
            "map" | "data" | "imagery" | "google" => Section::Map,
            _ => return None,
        })
    }

    /// The section before or after this one, round and round.
    pub fn cycle(self, next: bool) -> Section {
        let n = Section::ALL.len();
        let i = Section::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Section::ALL[(i + if next { 1 } else { n - 1 }) % n]
    }

    /// A small line icon for the sidebar, `s` points across, centred on `c`.
    fn icon(self, p: &Painter, c: Pos2, s: f32, col: Color32) {
        let st = Stroke::new(1.6, col);
        let u = s / 2.0;
        match self {
            // three sliders
            Section::General => {
                for (i, k) in [(-1.0, 0.3), (0.0, -0.4), (1.0, 0.15)] {
                    let y = c.y + i * u * 0.65;
                    p.line_segment([pos2(c.x - u, y), pos2(c.x + u, y)], st);
                    p.circle_filled(pos2(c.x + k * u, y), u * 0.22, col);
                }
            }
            // a monitor on its stand
            Section::Graphics => {
                let r = Rect::from_center_size(c - vec2(0.0, u * 0.15), vec2(s, s * 0.66));
                p.rect_stroke(r, 1.5, st, egui::StrokeKind::Middle);
                p.line_segment([pos2(c.x, r.bottom()), pos2(c.x, r.bottom() + u * 0.3)], st);
                p.line_segment([pos2(c.x - u * 0.45, r.bottom() + u * 0.32), pos2(c.x + u * 0.45, r.bottom() + u * 0.32)], st);
            }
            // a speaker and two waves
            Section::Sound => {
                let x0 = c.x - u;
                let body = vec![pos2(x0, c.y - u * 0.3), pos2(x0 + u * 0.4, c.y - u * 0.3), pos2(x0 + u * 0.95, c.y - u * 0.8), pos2(x0 + u * 0.95, c.y + u * 0.8), pos2(x0 + u * 0.4, c.y + u * 0.3), pos2(x0, c.y + u * 0.3)];
                p.add(egui::Shape::closed_line(body, st));
                for r in [0.45, 0.85] {
                    let pts: Vec<Pos2> = (0..=8).map(|i| {
                        let a = (i as f32 / 8.0 - 0.5) * 1.6;
                        pos2(c.x + u * 0.05 + a.cos() * r * u, c.y + a.sin() * r * u)
                    }).collect();
                    p.line(pts, st);
                }
            }
            // a camera body and its lens
            Section::Camera => {
                let r = Rect::from_center_size(c + vec2(0.0, u * 0.1), vec2(s, s * 0.68));
                p.rect_stroke(r, 2.0, st, egui::StrokeKind::Middle);
                p.line_segment([pos2(c.x - u * 0.35, r.top() - u * 0.2), pos2(c.x + u * 0.35, r.top() - u * 0.2)], st);
                p.circle_stroke(r.center(), u * 0.38, st);
            }
            // a gamepad: its outline, a cross and two buttons
            Section::Controls => {
                let r = Rect::from_center_size(c, vec2(s * 1.05, s * 0.62));
                p.rect_stroke(r, u * 0.3, st, egui::StrokeKind::Middle);
                let d = c - vec2(u * 0.48, 0.0);
                p.line_segment([d - vec2(u * 0.22, 0.0), d + vec2(u * 0.22, 0.0)], st);
                p.line_segment([d - vec2(0.0, u * 0.22), d + vec2(0.0, u * 0.22)], st);
                p.circle_filled(c + vec2(u * 0.38, u * 0.1), u * 0.11, col);
                p.circle_filled(c + vec2(u * 0.6, -u * 0.12), u * 0.11, col);
            }
            // a globe: its rim, a meridian and the equator
            Section::Map => {
                p.circle_stroke(c, u * 0.95, st);
                p.line_segment([c - vec2(u * 0.95, 0.0), c + vec2(u * 0.95, 0.0)], st);
                let pts: Vec<Pos2> = (0..=16).map(|i| {
                    let a = (i as f32 / 16.0) * std::f32::consts::TAU;
                    c + vec2(a.cos() * u * 0.4, a.sin() * u * 0.95)
                }).collect();
                p.line(pts, st);
            }
        }
    }
}

// ---- building blocks for the sections (graphics.rs uses them too)

/// A titled group of settings: a faint card with a small accent title.
pub fn card(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(7))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(16)))
        .corner_radius(menu::R_PANEL)
        .inner_margin(egui::Margin { left: 20, right: 20, top: 14, bottom: 16 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title.to_uppercase()).size(12.0).strong().color(ACCENT).extra_letter_spacing(1.2));
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.x_range(), r.top() + 3.0, Stroke::new(1.0, Color32::from_white_alpha(14)));
            ui.add_space(10.0);
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
        });
    ui.add_space(14.0);
}

/// One `label | control` row: the label (and a short dim hint under it) in
/// a fixed left column, the control to its right.
pub fn row(ui: &mut Ui, label: &str, hint: &str, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let lw = (w * 0.34).clamp(150.0, 250.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(lw, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(lw - 12.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(3.0);
            ui.add(egui::Label::new(RichText::new(label).size(14.0).color(TEXT)).wrap());
            if !hint.is_empty() {
                ui.add(egui::Label::new(RichText::new(hint).size(11.5).color(DIM)).wrap());
            }
        });
        ui.vertical(|ui| {
            ui.set_width((w - lw - 8.0).max(100.0));
            add(ui);
        });
    });
    ui.add_space(6.0);
}

/// The first cell of a grid row, as wide as a `row`'s label column so
/// grids line up with the rows around them.
fn grid_label(ui: &mut Ui, text: &str, size: f32, card_w: f32) {
    let lw = (card_w * 0.34).clamp(150.0, 250.0) - 14.0;
    ui.horizontal(|ui| {
        ui.set_min_width(lw);
        ui.label(RichText::new(text).size(size).color(TEXT));
    });
}

/// A dim explanatory line across the card.
pub fn note(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(DIM)).wrap());
}

/// Draws the settings screen for `game` (see the module docs); true when
/// its Back button was pressed. `in_flight`: over the paused game (dimmed)
/// rather than the start screen.
pub fn show(ctx: &egui::Context, game: &mut Game, in_flight: bool) -> bool {
    game.settings_screen(ctx, in_flight)
}

impl Game {
    pub(crate) fn settings_screen(&mut self, ctx: &egui::Context, in_flight: bool) -> bool {
        let screen = ctx.content_rect();
        let mut closed = false;
        egui::Area::new(egui::Id::unique("settings_screen")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            // The backdrop takes every click; the game (or globe) behind is all but hidden.
            ui.interact(screen, ui.scope_id().with("backdrop"), Sense::click());
            let p = ui.painter().clone();
            p.rect_filled(screen, 0.0, if in_flight { Color32::from_rgba_premultiplied(4, 6, 9, 222) } else { Color32::from_rgb(8, 11, 15) });
            let side_w = if screen.width() < 1000.0 { 210.0 } else { 250.0 };
            let side = Rect::from_min_size(screen.min, vec2(side_w, screen.height()));
            p.rect_filled(side, 0.0, Color32::from_black_alpha(90));
            p.vline(side.right(), side.y_range(), Stroke::new(1.0, Color32::from_white_alpha(16)));

            // The sidebar: title, the sections, Back at the foot.
            let mut s = ui.new_child(egui::UiBuilder::new().max_rect(side.shrink2(vec2(16.0, 28.0))).layout(Layout::top_down(Align::Min)));
            s.add_space(4.0);
            s.label(RichText::new("FABLE FLIGHT").size(11.5).strong().color(ACCENT).extra_letter_spacing(1.5));
            s.label(RichText::new(t!("Settings", "Réglages")).size(30.0).strong().color(Color32::WHITE));
            s.add_space(22.0);
            s.spacing_mut().item_spacing.y = 4.0;
            for sec in Section::ALL {
                let r = self.settings_tab(&mut s, sec);
                if r.clicked() && self.settings_sec != sec {
                    self.settings_sec = sec;
                }
            }
            let foot = Rect::from_min_max(pos2(side.left() + 16.0, side.bottom() - 100.0), pos2(side.right() - 16.0, side.bottom() - 24.0));
            let mut f = ui.new_child(egui::UiBuilder::new().max_rect(foot).layout(Layout::bottom_up(Align::Min)));
            if padui::active() {
                padui::prompts(&mut f, &[("LB / RB", crate::tr("section")), ("B", crate::tr("back"))], 18.0, 12.0, DIM);
            } else {
                f.label(RichText::new(t!("Changes are saved as you make them.", "Les changements sont enregistrés aussitôt.")).size(11.5).color(DIM));
            }
            f.add_space(6.0);
            let back = if in_flight { t!("Back to the pause menu", "Retour au menu pause") } else { t!("Back", "Retour") };
            if menu::wide_button(&mut f, back, Hint::of("Esc", "B"), false, true).clicked() {
                closed = true;
            }

            // The content: the section's title and its cards, in a centred column.
            let area = Rect::from_min_max(pos2(side.right() + 1.0, screen.top()), screen.max);
            let mut c = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(Layout::top_down(Align::Min)));
            let col_w = (area.width() - 96.0).clamp(320.0, MAX_W);
            let left = ((area.width() - col_w) * 0.5).max(24.0);
            egui::ScrollArea::vertical().id_salt(("settings_scroll", self.settings_sec.name())).auto_shrink([false, false]).show(&mut c, |ui| {
                ui.horizontal_top(|ui| {
                    ui.add_space(left);
                    ui.vertical(|ui| {
                        ui.set_width(col_w);
                        ui.add_space(36.0);
                        let sec = self.settings_sec;
                        ui.label(RichText::new(sec.label()).size(26.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new(sec.blurb()).size(13.0).color(DIM));
                        ui.add_space(18.0);
                        ui.spacing_mut().slider_width = (col_w * 0.36).clamp(160.0, 300.0);
                        match sec {
                            Section::General => self.set_general(ui),
                            Section::Graphics => self.set_graphics(ui),
                            Section::Sound => self.set_sound(ui),
                            Section::Camera => self.set_camera(ui, in_flight),
                            Section::Controls => self.set_controls(ui),
                            Section::Map => self.set_map(ui),
                        }
                        ui.add_space(40.0);
                    });
                });
                padui::follow_focus(ui);
            });
            ui.expand_to_include_rect(screen);
        });
        closed
    }

    /// One sidebar tab: its icon and name, the chosen one marked in the accent colour.
    fn settings_tab(&mut self, ui: &mut Ui, sec: Section) -> egui::Response {
        let on = self.settings_sec == sec;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, sec.label()));
        if on {
            self.pad_home_focus(&resp);
        }
        let lit = resp.hovered() || resp.has_focus();
        let p = ui.painter();
        if on || lit {
            p.rect_filled(rect, menu::R_WIDGET, Color32::from_white_alpha(if on { 16 } else { 8 }));
        }
        if on {
            p.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, ACCENT);
        }
        let col = if on { Color32::WHITE } else if lit { TEXT } else { Color32::from_rgb(178, 185, 196) };
        sec.icon(p, pos2(rect.left() + 24.0, rect.center().y), 18.0, if on { ACCENT } else { col });
        p.text(pos2(rect.left() + 46.0, rect.center().y), Align2::LEFT_CENTER, sec.label(), FontId::proportional(15.5), col);
        resp
    }

    fn set_general(&mut self, ui: &mut Ui) {
        card(ui, t!("Language", "Langue"), |ui| {
            row(ui, t!("Interface language", "Langue de l'interface"), "", |ui| self.language_picker(ui));
        });
        card(ui, t!("HUD & minimap", "HUD et mini-carte"), |ui| {
            row(ui, "HUD", t!("H in flight", "H en vol"), |ui| {
                let t = if self.show_hud { t!("shown", "affiché") } else { t!("hidden", "masqué") };
                ui.checkbox(&mut self.show_hud, t);
            });
            row(ui, t!("Minimap", "Mini-carte"), t!("` in flight", "` en vol"), |ui| {
                ui.checkbox(&mut self.minimap.visible, t!("Show minimap", "Afficher la mini-carte"));
            });
            row(ui, t!("Minimap size", "Taille de la mini-carte"), t!("it fits the window and keeps this size", "elle s'adapte à la fenêtre et garde cette taille"), |ui| {
                if ui.add(egui::Slider::new(&mut self.minimap.size.x, 240.0..=640.0).step_by(10.0).suffix(" px")).changed() {
                    self.minimap.size.y = self.minimap.size.x;
                }
            });
            row(ui, t!("Minimap style", "Style de la mini-carte"), "", |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.minimap.plan, false, t!("Satellite", "Satellite"));
                    ui.selectable_value(&mut self.minimap.plan, true, t!("Plan", "Plan"));
                });
            });
            row(ui, t!("Minimap orientation", "Orientation de la mini-carte"), "", |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.minimap.heading_up, false, t!("North up", "Nord en haut"));
                    ui.selectable_value(&mut self.minimap.heading_up, true, t!("Heading up", "Cap en haut"));
                });
            });
            row(ui, t!("Landing guidance", "Aide à l’atterrissage"), "", |ui| {
                ui.checkbox(&mut self.minimap.landing_guidance, t!("Approach arrows and tunnel", "Flèches et tunnel d’approche"));
            });
        });
        card(ui, t!("Air traffic", "Trafic aérien"), |ui| {
            row(ui, t!("Live air traffic", "Trafic aérien réel"), t!("real aircraft around you", "les vrais avions autour de vous"), |ui| {
                let mut on = self.live.enabled();
                if ui.checkbox(&mut on, t!("real aircraft, from adsb.lol", "vrais avions, via adsb.lol")).changed() {
                    self.live.set_enabled(on);
                }
            });
        });
        let t = if padui::active() { t!("In flight, View switches the camera; Y + View opens the flight panel.", "En vol, View change de caméra ; Y + View ouvre le panneau de vol.") } else { t!("Shortcuts in flight: U sound, C camera, H HUD, ` minimap, F2 anti-aliasing, Tab the flight panel.", "Raccourcis en vol : U son, C caméra, H HUD, ` mini-carte, F2 anticrénelage, Tab le panneau de vol.") };
        note(ui, t);
    }

    fn set_graphics(&mut self, ui: &mut Ui) {
        let before = self.gfx;
        let size = self.window.inner_size();
        card(ui, t!("Preset", "Préréglage"), |ui| self.gfx.preset_ui(ui));
        card(ui, t!("Quality", "Qualité"), |ui| self.gfx.quality_ui(ui));
        card(ui, t!("Display", "Affichage"), |ui| self.gfx.display_ui(ui, (size.width, size.height), self.fps));
        let on = if self.src.google.photoreal() { t!("on", "activées") } else { t!("off", "désactivées") };
        note(ui, &t!("Google's photorealistic 3D cities ({}) have their own switch and detail level under Map & Data.", "Les villes 3D photoréalistes de Google ({}) ont leur propre réglage dans Carte et données.", on));
        if self.gfx != before {
            self.gfx.save();
        }
    }

    fn set_sound(&mut self, ui: &mut Ui) {
        card(ui, t!("Sound", "Son"), |ui| {
            row(ui, t!("Sound", "Son"), t!("U in flight", "U en vol"), |ui| {
                let t = if self.sound_on { t!("on", "activé") } else { t!("off", "coupé") };
                ui.checkbox(&mut self.sound_on, t);
            });
            row(ui, t!("Volume", "Volume"), "", |ui| {
                ui.add_enabled(self.sound_on, egui::Slider::new(&mut self.volume, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
        });
    }

    fn set_camera(&mut self, ui: &mut Ui, in_flight: bool) {
        card(ui, t!("Camera", "Caméra"), |ui| {
            let hint = if in_flight { t!("C switches it in flight", "C la change en vol") } else { t!("the view you start in; C switches it", "la vue au départ ; C la change") };
            row(ui, t!("Camera", "Caméra"), hint, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.cam_mode, 0, t!("Chase", "Extérieure"));
                    ui.selectable_value(&mut self.cam_mode, 1, "Cockpit");
                });
            });
            row(ui, t!("Cockpit field of view", "Champ de vision cockpit"), t!("the mouse wheel zooms for a moment", "la molette zoome un instant"), |ui| {
                ui.add(egui::Slider::new(&mut self.cockpit_fov, 35.0..=85.0).custom_formatter(|v, _| format!("{v:.0}° vertical")));
            });
        });
    }

    fn set_controls(&mut self, ui: &mut Ui) {
        self.binds_capture_banner(ui);
        card(ui, t!("Mouse", "Souris"), |ui| {
            row(ui, t!("Mouse look speed", "Vitesse du regard (souris)"), t!("dragging to look around", "glisser pour regarder autour"), |ui| {
                ui.add(egui::Slider::new(&mut self.look_sens, 0.25..=3.0).logarithmic(true).custom_formatter(|v, _| format!("{v:.2}×")));
            });
        });
        card(ui, t!("Connected devices", "Périphériques connectés"), |ui| self.binds_devices(ui));
        card(ui, t!("Axes", "Axes"), |ui| self.binds_axes(ui));
        card(ui, t!("Buttons (joystick, throttle, wheel)", "Boutons (joystick, manette des gaz, volant)"), |ui| self.binds_buttons(ui));
        card(ui, t!("Keyboard", "Clavier"), |ui| self.binds_keys(ui));
        if ui.add_enabled(self.binds.capture.is_none(), egui::Button::new(t!("Reset all bindings", "Tout réinitialiser"))).clicked() {
            self.binds = bind::Bindings::default();
            self.keys.clear();
        }
        ui.add_space(14.0);
        card(ui, t!("Key reference", "Aide-mémoire des touches"), |ui| {
            let t = if padui::active() { t!("In flight, Y + View shows the flight panel (instruments, places, weather) and Menu pauses.", "En vol, Y + View affiche le panneau de vol (instruments, destinations, météo) et Menu met en pause.") } else { t!("In flight, Tab shows the flight panel (instruments, places, weather) and Esc pauses.", "En vol, Tab affiche le panneau de vol (instruments, destinations, météo) et Échap met en pause.") };
            note(ui, t);
            ui.add_space(6.0);
            menu::controls_list(ui);
        });
    }

    fn set_map(&mut self, ui: &mut Ui) {
        let g = self.src.google.clone();
        let has_key = !self.gkey.trim().is_empty();
        let need_key = t!("Enter a Google Maps Platform API key first", "Saisissez d'abord une clé API Google Maps Platform");
        card(ui, t!("Map imagery", "Imagerie cartographique"), |ui| {
            row(ui, t!("Google API key", "Clé API Google"), t!("Map Tiles API, billed to you", "Map Tiles API, facturée à vous"), |ui| {
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.gkey).password(!self.gkey_show).hint_text(t!("Google Maps Platform key", "Clé Google Maps Platform")).desired_width((ui.available_width() - 60.0).min(380.0)));
                    if r.lost_focus() || (r.changed() && self.gkey.trim().is_empty()) {
                        g.set_key(&self.gkey);
                    }
                    if ui.small_button(if self.gkey_show { t!("hide", "masquer") } else { t!("show", "afficher") }).clicked() {
                        self.gkey_show = !self.gkey_show;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to(RichText::new(t!("How to get a key", "Obtenir une clé")).size(12.0), "https://developers.google.com/maps/documentation/tile/get-api-key");
                    ui.label(RichText::new("·").size(12.0).color(DIM));
                    ui.hyperlink_to(RichText::new(t!("Cloud console: credentials", "Console Cloud : identifiants")).size(12.0), "https://console.cloud.google.com/google/maps-apis/credentials");
                });
            });
            row(ui, t!("Satellite imagery", "Imagerie satellite"), "", |ui| {
                let mut on = g.enabled();
                if ui.add_enabled(has_key, egui::Checkbox::new(&mut on, t!("Google Maps satellite imagery", "Imagerie satellite Google Maps"))).on_disabled_hover_text(need_key).changed() {
                    g.set_key(&self.gkey);
                    g.set_enabled(on);
                }
                note(ui, if on { t!("Terrain and roofs take Google's tiles (Map Tiles API); what has loaded already is redone. Tiles not yet on disk are billed to your key.", "Le terrain et les toits utilisent les tuiles de Google (Map Tiles API) ; ce qui est déjà chargé est refait. Les tuiles absentes du disque sont facturées sur votre clé.") } else { t!("Off: the free default imagery. On: Google's Map Tiles API with your own key, billed per tile request.", "Désactivé : l'imagerie gratuite par défaut. Activé : la Map Tiles API de Google avec votre propre clé, facturée à la tuile.") });
            });
            let status = g.status.lock().clone();
            if !status.is_empty() {
                ui.label(RichText::new(status.chars().take(160).collect::<String>()).size(11.5).color(Color32::from_rgb(255, 150, 70)));
            }
        });
        card(ui, t!("Photorealistic 3D cities", "Villes 3D photoréalistes"), |ui| {
            let mut photo = g.photoreal();
            row(ui, t!("Google 3D tiles", "Tuiles 3D Google"), t!("beta", "bêta"), |ui| {
                if ui.add_enabled(has_key, egui::Checkbox::new(&mut photo, t!("Photorealistic 3D cities", "Villes 3D photoréalistes"))).on_disabled_hover_text(need_key).changed() {
                    g.set_key(&self.gkey);
                    g.set_photoreal(photo);
                }
            });
            let detail = g.detail();
            let zone_km = tiles3d::Params::of(detail).zone_r / 1000.0;
            row(ui, t!("3D detail", "Détail 3D"), "", |ui| {
                ui.add_enabled_ui(photo, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let mut d = detail;
                        for c in gmaps::Detail::ALL {
                            let name = match c {
                                gmaps::Detail::Low => t!("Low", "Faible"),
                                gmaps::Detail::Medium => t!("Medium", "Moyen"),
                                gmaps::Detail::High => t!("High", "Élevé"),
                                gmaps::Detail::Ultra => "Ultra",
                            };
                            ui.selectable_value(&mut d, c, name);
                        }
                        if d != detail {
                            g.set_detail(d);
                        }
                    });
                    note(ui, &t!("{} One billed session per flight at every level; the first minute over a city takes up to a third more.", "{} Une session facturée par vol à chaque niveau ; la première minute au-dessus d'une ville consomme jusqu'à un tiers de plus.", crate::detail_hint(detail)));
                });
            });
            note(ui, &if photo { t!("Google's photogrammetry meshes replace the ground, buildings and trees within about {zone_km:.0} km of the aircraft; at night they light up with our street lamps and windows. Each flight opens one billed 3D session (a root request, good for about 3 hours); the tiles under it are kept on disk only for the 4 hours Google allows.", "Les maillages photogrammétriques de Google remplacent le sol, les bâtiments et les arbres dans un rayon d'environ {zone_km:.0} km autour de l'avion ; la nuit, nos lampadaires et fenêtres les éclairent. Chaque vol ouvre une session 3D facturée (une requête racine, valable environ 3 heures) ; les tuiles sont gardées sur le disque seulement pendant les 4 heures autorisées par Google.", zone_km = zone_km) } else { t!("Off: our own 3D world everywhere. On: Google's Photorealistic 3D Tiles near the aircraft, one billed session per flight.", "Désactivé : notre propre monde 3D partout. Activé : les Photorealistic 3D Tiles de Google près de l'avion, une session facturée par vol.").to_string() });
        });
        card(ui, t!("Google usage", "Utilisation de Google"), |ui| {
            let u = g.usage.lock().clone();
            let req = |n: u64| i18n::plural(n, "request", "requests", "requête", "requêtes");
            let line = |ui: &mut Ui, k: &str, v: String| row(ui, k, "", |ui| {
                ui.label(RichText::new(v).size(12.5).color(TEXT));
            });
            line(ui, t!("This session", "Cette session"), format!("{} · {}", req(u.session_requests as u64), crate::bytes(u.session_bytes)));
            line(ui, t!("   imagery", "   imagerie"), t!("{} tiles downloaded · {} from disk", "{} tuiles téléchargées · {} depuis le disque", u.session_tiles, u.session_cached));
            if u.session_roots > 0 || u.total_roots > 0 {
                let ses = i18n::plural(u.session_roots as u64, "session", "sessions", "session", "sessions");
                line(ui, "   3D", t!("{} (billed) · {} tiles · {} from disk", "{} (facturées) · {} tuiles · {} depuis le disque", ses, u.session_3d, u.session_3d_cached));
            }
            if u.session_errors > 0 {
                line(ui, t!("   failed", "   échecs"), format!("{}", u.session_errors));
            }
            line(ui, &t!("Month {}", "Mois {}", u.month), format!("{} · {}", req(u.month_requests as u64), crate::bytes(u.month_bytes)));
            line(ui, t!("All time", "Depuis le début"), format!("{} · {}", req(u.total_requests as u64), crate::bytes(u.total_bytes)));
            if u.total_roots > 0 {
                line(ui, t!("3D sessions", "Sessions 3D"), t!("{} this month · {} all time", "{} ce mois-ci · {} au total", u.month_roots, u.total_roots));
            }
            note(ui, t!("Counts only calls that reach Google; tiles already on disk are free. Check the Cloud console for billing.", "Ne compte que les appels qui atteignent Google ; les tuiles déjà sur le disque sont gratuites. Consultez la console Cloud pour la facturation."));
        });
        card(ui, t!("Real-world data", "Données réelles"), |ui| {
            row(ui, t!("Real weather", "Météo réelle"), t!("also on the weather pages", "aussi dans les pages météo"), |ui| {
                ui.checkbox(&mut self.wx.live, t!("METAR, else Open-Meteo; radar", "METAR, sinon Open-Meteo ; radar"));
            });
            row(ui, t!("Real time", "Heure réelle"), t!("until a time is set by hand", "jusqu'à ce qu'on règle l'heure"), |ui| {
                ui.checkbox(&mut self.wx.real_time, t!("the sun where you are, now", "le soleil là où vous êtes, maintenant"));
            });
            row(ui, t!("Live air traffic", "Trafic aérien réel"), "", |ui| {
                let mut on = self.live.enabled();
                if ui.checkbox(&mut on, t!("real aircraft, from adsb.lol", "vrais avions, via adsb.lol")).changed() {
                    self.live.set_enabled(on);
                }
            });
        });
    }

    // ---- joysticks, pedals and keys (bind.rs)

    /// While waiting for an axis, button or key to bind: what to do, and Cancel.
    fn binds_capture_banner(&mut self, ui: &mut Ui) {
        use bind::{Capture, ACTIONS};
        let Some(c) = self.binds.capture.clone() else { return };
        let label = |id: &str| ACTIONS.iter().find(|a| a.id == id).map(|a| a.label()).unwrap_or("?");
        let what = match &c {
            Capture::Axis(a) => a.how().to_string(),
            Capture::Button(id) => t!("Press the button for: {}", "Appuyez sur le bouton pour : {}", label(id)),
            Capture::Key(id) => t!("Press the key for: {}  (Esc: cancel)", "Appuyez sur la touche pour : {}  (Échap : annuler)", label(id)),
        };
        egui::Frame::new().fill(ACCENT.gamma_multiply(0.22)).corner_radius(menu::R_PANEL).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(what).size(14.0).color(Color32::WHITE));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(t!("Cancel", "Annuler")).clicked() {
                        self.binds.capture = None;
                    }
                });
            });
        });
        ui.add_space(12.0);
    }

    fn binds_devices(&mut self, ui: &mut Ui) {
        let devices: Vec<(bind::Uuid, String, bool)> = self.gilrs.as_ref().map(|g| g.gamepads().map(|(_, p)| (p.uuid(), p.name().to_string(), self.binds.uses(&p.uuid()))).collect()).unwrap_or_default();
        if devices.is_empty() {
            note(ui, t!("No joystick, pedals or gamepad found.", "Aucun joystick, palonnier ni manette détecté."));
        }
        for (k, (uuid, name, bound)) in devices.into_iter().enumerate() {
            let role = if bound { t!("bound", "assigné") } else { t!("gamepad layout", "disposition manette") };
            row(ui, &name, role, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(("preset", k)).selected_text(t!("Apply a preset…", "Appliquer un préréglage…")).width(260.0).show_ui(ui, |ui| {
                        for p in bind::PRESETS {
                            if ui.selectable_label(false, p.label()).clicked() {
                                self.binds.apply_preset(p, uuid, &name, true);
                            }
                        }
                    });
                    if bound && ui.small_button(t!("Remove its bindings", "Retirer ses assignations")).clicked() {
                        self.binds.clear_device(&uuid);
                    }
                });
            });
        }
        note(ui, t!("A device with anything bound to it no longer works as a gamepad.", "Un périphérique dès qu'on lui assigne quelque chose ne sert plus de manette de jeu."));
    }

    fn binds_axes(&mut self, ui: &mut Ui) {
        use bind::{AxisAct, Capture};
        let dim = |s: String| RichText::new(s).size(12.0).color(DIM);
        let busy = self.binds.capture.is_some();
        note(ui, t!("Binding · value · dead zone (travel that does nothing)", "Assignation · valeur · zone morte (course sans effet)"));
        ui.add_space(4.0);
        let cw = ui.available_width();
        egui::Grid::new("bind_axes").num_columns(5).spacing([14.0, 6.0]).show(ui, |ui| {
            for a in AxisAct::ALL {
                grid_label(ui, a.label(), 14.0, cw);
                ui.label(dim(self.binds.describe_axis(a).unwrap_or_else(|| "—".into())));
                // (0 until the axis has moved since the game started)
                match self.binds.axis(a).or_else(|| self.binds.axes.contains_key(&a).then_some(0.0)) {
                    Some(v) => {
                        let f = if a.centred() { (v as f32 + 1.0) * 0.5 } else { v as f32 };
                        ui.add(egui::ProgressBar::new(f).desired_width(70.0).desired_height(8.0));
                    }
                    None => {
                        ui.label("");
                    }
                }
                // The dead zone: round the centre, or at the idle end of a throttle or brake.
                match self.binds.axes.get_mut(&a) {
                    Some(b) => {
                        ui.spacing_mut().slider_width = 70.0;
                        ui.add(egui::Slider::new(&mut b.dead, 0.0..=0.3).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)))
                            .on_hover_text(t!("Dead zone: how much travel does nothing", "Zone morte : la course qui ne fait rien"));
                    }
                    None => {
                        ui.label("");
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(t!("Assign", "Assigner")).small()).clicked() {
                        self.binds.start_capture(Capture::Axis(a));
                    }
                    if let Some(b) = self.binds.axes.get_mut(&a) {
                        // The sign is learnt when binding; this flips it.
                        let mut inv = b.sign < 0.0;
                        if ui.checkbox(&mut inv, t!("Invert", "Inverser")).changed() {
                            b.sign = -b.sign;
                        }
                    }
                    if self.binds.axes.contains_key(&a) && ui.small_button(t!("Clear", "Effacer")).clicked() {
                        self.binds.axes.remove(&a);
                    }
                });
                ui.end_row();
            }
        });
    }

    fn binds_buttons(&mut self, ui: &mut Ui) {
        use bind::{Capture, ACTIONS};
        let busy = self.binds.capture.is_some();
        let cw = ui.available_width();
        egui::Grid::new("bind_buttons").num_columns(3).min_col_width(150.0).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
            for a in ACTIONS {
                grid_label(ui, a.label(), 13.5, cw);
                ui.label(RichText::new(self.binds.describe_button(a.id).unwrap_or_else(|| "—".into())).size(12.0).color(DIM));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(t!("Assign", "Assigner")).small()).clicked() {
                        self.binds.start_capture(Capture::Button(a.id));
                    }
                    if self.binds.buttons.contains_key(a.id) && ui.small_button(t!("Clear", "Effacer")).clicked() {
                        self.binds.buttons.remove(a.id);
                    }
                });
                ui.end_row();
            }
        });
    }

    fn binds_keys(&mut self, ui: &mut Ui) {
        use bind::{Capture, ACTIONS};
        let busy = self.binds.capture.is_some();
        let cw = ui.available_width();
        egui::Grid::new("bind_keys").num_columns(3).min_col_width(150.0).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
            for a in ACTIONS.iter().filter(|a| a.keyboard) {
                grid_label(ui, a.label(), 13.5, cw);
                let moved = self.binds.keys.contains_key(a.id);
                let name = self.binds.key_of(a).map(bind::key_name).unwrap_or_else(|| t!("(none)", "(aucune)").into());
                ui.label(RichText::new(name).monospace().size(12.5).color(if moved { ACCENT } else { DIM }));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(t!("Change", "Changer")).small()).clicked() {
                        self.binds.start_capture(Capture::Key(a.id));
                        self.keys.clear();
                    }
                    if moved && ui.small_button(t!("Default", "Par défaut")).clicked() {
                        self.binds.reset_key(a.id);
                        self.keys.clear();
                    }
                });
                ui.end_row();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_parse_and_cycle() {
        for s in Section::ALL {
            assert_eq!(Section::parse(s.name()), Some(s));
            assert_eq!(s.cycle(true).cycle(false), s);
        }
        assert_eq!(Section::parse("bindings"), Some(Section::Controls));
        assert_eq!(Section::parse("Graphics"), Some(Section::Graphics));
        assert_eq!(Section::Map.cycle(true), Section::General);
        assert_eq!(Section::parse("nope"), None);
        // `--pause-menu controls` is the key list, not the settings section.
        assert_eq!(crate::menu::Page::parse("controls"), Some(crate::menu::Page::Controls));
        assert_eq!(crate::menu::Page::parse("graphics"), None);
    }
}
