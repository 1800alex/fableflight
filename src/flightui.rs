//! The interface over the flying itself (the chase view's; the cockpit has
//! its own panel and hides this unless asked): the HUD at the edges of the
//! view (see `hud` below), notices at the top centre that fade (an airport
//! as you arrive, the game's status messages), alerts at the bottom centre,
//! and a systems card at the top left for the aircraft that have something
//! to show there: a rocket's flight figures, a fighter's weapons.
use crate::menu::{self, Icon, Page, DIM, TEXT};
use crate::{padui, t, tr, Game};
use egui::{pos2, vec2, Align2, Color32, Rect, RichText, Sense, Stroke};

// ---- the flight panel's pieces (its pages are in main.rs, radio_page and weather.rs)
//
// Every page is made of these, so they read alike: a section's tag and a
// hairline; rows of a word and a value (words in Barlow at the left, values
// in the instrument face at the right); tiles for the few numbers that
// matter most; lists of things to pick; chips.

/// The panel's width (its content's).
pub const PANEL_W: f32 = 344.0;

/// A section of a page: its tag, a hairline under it.
pub fn pane_section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.label(tag_text(title));
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top() + 3.0, Stroke::new(1.0, menu::LINE));
    ui.add_space(8.0);
}

/// One row: a word at the left, its value at the right.
pub fn kv(ui: &mut egui::Ui, key: &str, value: &str, col: Color32) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.label(RichText::new(key).font(menu::caption()).color(SOFT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(value).font(num_s()).color(col)).truncate());
        });
    });
}

/// The numbers that matter most, as tiles two to a row: (tag, value, unit, colour).
pub fn tiles(ui: &mut egui::Ui, items: &[(&str, String, &str, Color32)]) {
    let gap = 8.0;
    let w = (ui.available_width() - gap) * 0.5;
    for pair in items.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (k, v, unit, col) in pair {
                let (r, _) = ui.allocate_exact_size(vec2(w, 58.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(r, R_IN, Color32::from_white_alpha(7));
                paint_tag(p, r.left_top() + vec2(10.0, 15.0), Align2::LEFT_CENTER, k);
                let g = p.layout_no_wrap(v.clone(), num_l(), *col);
                let base = r.left_bottom() + vec2(10.0, -19.0);
                let vw = g.size().x;
                p.galley(pos2(base.x, base.y - g.size().y * 0.5), g, *col);
                p.text(pos2(base.x + vw + 5.0, base.y + 3.0), Align2::LEFT_CENTER, *unit, num_s(), SOFT);
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
}

/// A small button of a page: a word, or a code in the instrument face; the
/// chosen one in the accent.
pub fn chip(ui: &mut egui::Ui, text: &str, mono: bool, on: bool) -> egui::Response {
    let font = if mono { num_s() } else { menu::medium(13.5) };
    let g = ui.painter().layout_no_wrap(text.to_string(), font, TEXT);
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0, 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, text));
    let lit = menu::ease(ui.ctx(), resp.id, resp.hovered() || resp.has_focus());
    let p = ui.painter();
    if on {
        p.rect_filled(r, R_IN, SKY);
    } else {
        p.rect_filled(r, R_IN, white(0.06 + 0.08 * lit));
        p.rect_stroke(r, R_IN, Stroke::new(1.0, white(0.22 + 0.2 * lit)), egui::StrokeKind::Inside);
    }
    p.galley(r.center() - g.size() * 0.5, g, Color32::WHITE);
    resp
}

/// One entry of a list: a code and a name over a line of detail at the left,
/// whatever `right` adds (chips, a value) at the right. The right part is laid
/// out first, so the name is cut short to the room left, never under it.
pub fn list_row(ui: &mut egui::Ui, code: &str, name: &str, detail: &str, right: impl FnOnce(&mut egui::Ui)) {
    let w = ui.available_width();
    ui.allocate_ui_with_layout(vec2(w, 42.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.spacing_mut().item_spacing.x = 4.0;
        right(ui);
        ui.add_space(8.0);
        let room = ui.available_width();
        ui.allocate_ui_with_layout(vec2(room, 42.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(room);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new(code).font(num_m()).color(Color32::WHITE));
                ui.add(egui::Label::new(RichText::new(name).font(menu::caption()).color(TEXT)).truncate());
            });
            if !detail.is_empty() {
                ui.add(egui::Label::new(RichText::new(detail).font(num_s()).color(SOFT)).truncate());
            }
        });
    });
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, Color32::from_white_alpha(10)));
    ui.add_space(4.0);
}

/// A button as wide as `w` in the panel's chip style.
pub fn pane_button(ui: &mut egui::Ui, text: &str, w: f32) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(w, 34.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let lit = menu::ease(ui.ctx(), resp.id, resp.hovered() || resp.has_focus());
    let p = ui.painter();
    p.rect_filled(r, R_IN, white(0.08 + 0.1 * lit));
    p.rect_stroke(r, R_IN, Stroke::new(1.0, white(0.28 + 0.2 * lit)), egui::StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, menu::medium(14.5), Color32::WHITE);
    resp
}

/// What the toolbar asked for this frame.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Bar {
    Page(Page),
    Map,
    Camera,
}

/// How long an airport notice shows (s), the last of it fading.
const NOTE_SHOW: f64 = 7.0;
const NOTE_FADE: f64 = 1.0;

impl Game {
    /// Carries out what the toolbar asked for.
    pub(crate) fn toolbar_act(&mut self, b: Bar) {
        match b {
            Bar::Page(p) => {
                if self.info_open && self.info_page == p {
                    self.info_open = false;
                } else {
                    self.info_open = true;
                    self.info_page = p;
                }
            }
            Bar::Map => self.minimap.visible = !self.minimap.visible,
            Bar::Camera => self.next_view(),
        }
    }

    /// The notices at the top centre, and the guidance line over the
    /// instruments. `airport`: `airport_hud`'s lines (name, runway and wind,
    /// then the live line, coloured 1 good, 2 caution, 3 warning).
    pub(crate) fn notices(&mut self, ctx: &egui::Context, airport: Option<&[(String, u8)]>) {
        // A new airport (or a new runway in use) brings its notice back.
        let key = airport.map(|l| l.iter().take(2).map(|x| x.0.as_str()).collect::<Vec<_>>().join("|")).unwrap_or_default();
        if key != self.airport_note.0 {
            self.airport_note = (key.clone(), if key.is_empty() { -100.0 } else { self.real });
        }
        let fade = |since: f64, show: f64| -> f32 { (1.0 - ((self.real - since - (show - NOTE_FADE)) / NOTE_FADE).max(0.0)).clamp(0.0, 1.0) as f32 };
        let mut notes: Vec<(f32, Vec<(String, egui::FontId, Color32)>)> = Vec::new();
        if let (Some(lines), a) = (airport, fade(self.airport_note.1, NOTE_SHOW)) {
            if a > 0.0 && !lines.is_empty() {
                let mut v = vec![(lines[0].0.clone(), menu::item_title(), Color32::WHITE)];
                if let Some(l) = lines.get(1) {
                    v.push((l.0.clone(), menu::caption(), TEXT));
                }
                notes.push((a, v));
            }
        }
        // (a status that only repeats the airport the notice above already names is left out)
        let airport_code = airport.and_then(|l| l.first()).and_then(|l| l.0.split_whitespace().next()).unwrap_or("\u{0}");
        let repeats = !notes.is_empty() && self.status.starts_with(airport_code);
        if self.real < self.status_until && !self.status.is_empty() && !repeats {
            let a = ((self.status_until - self.real) / NOTE_FADE).clamp(0.0, 1.0) as f32;
            notes.push((a, vec![(self.status.chars().take(90).collect(), menu::label(), Color32::WHITE)]));
        }
        if !notes.is_empty() {
            // under the heading strip
            let y = HEADING_H + 34.0;
            egui::Area::new(egui::Id::unique("notices")).anchor(Align2::CENTER_TOP, [0.0, y]).interactable(false).show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.vertical_centered(|ui| {
                    for (a, lines) in &notes {
                        // (a scope, so each notice takes its own room in the stack)
                        ui.scope(|ui| {
                            ui.multiply_opacity(*a);
                            frosted(ui, glass().inner_margin(egui::Margin::symmetric(18, 10)), |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                for (text, font, col) in lines {
                                    ui.label(RichText::new(text).font(font.clone()).color(*col));
                                }
                            });
                        });
                    }
                });
            });
            ctx.request_repaint();
        }
    }

    /// The systems card at the top left: a rocket's flight figures, or a
    /// fighter's weapons. Nothing (None) for every other aircraft.
    pub(crate) fn systems_card(&self, ctx: &egui::Context, vs: f64, base_up: f64) -> Option<Rect> {
        let p = &self.plane;
        let rocket = p.engines().is_some();
        if !rocket && !self.weapons.has_weapons() {
            return None;
        }
        let amber = Color32::from_rgb(255, 196, 80);
        let row = |ui: &mut egui::Ui, k: &str, v: String, col: Color32| {
            ui.label(RichText::new(k).font(menu::caption()).color(DIM));
            ui.label(RichText::new(v).font(num_s()).color(col));
            ui.end_row();
        };
        let r = egui::Area::new(egui::Id::unique("systems")).anchor(Align2::LEFT_TOP, [14.0, 12.0]).interactable(false).show(ctx, |ui| {
            frosted(ui, glass(), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                if let Some((name, _, _)) = p.engines() {
                    ui.label(tag_text(&format!("{}  {}", p.stage_name().unwrap_or(""), name)));
                    egui::Grid::new("rocket").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                        let apo = p.apogee().map_or(t!("escape", "libération").to_string(), |a| format!("{:.0} km", a.max(0.0) / 1000.0));
                        let high = if base_up < 3000.0 { format!("{:.0} m", base_up.max(0.0)) } else { format!("{:.1} km", base_up / 1000.0) };
                        row(ui, t!("Throttle", "Poussée"), format!("{:.0}%  ·  {:.0} MN", p.throttle * 100.0, p.thrust_now() / 1.0e6), TEXT);
                        row(ui, t!("Climb, speed", "Montée, vitesse"), format!("{:+.0} m/s  ·  {:.0} m/s", vs, p.vel.length()), TEXT);
                        row(ui, t!("Height", "Hauteur"), high, TEXT);
                        row(ui, t!("Apogee", "Apogée"), apo, TEXT);
                        row(ui, t!("Downrange", "Distance"), format!("{:.0} km", p.downrange() / 1000.0), TEXT);
                        if let Some(t) = p.burnout() {
                            row(ui, t!("Burnout", "Fin de combustion"), format!("{}:{:02}", t as u32 / 60, t as u32 % 60), TEXT);
                        }
                        row(ui, t!("Mass, load", "Masse, facteur"), format!("{:.0} t  ·  {:.1} g", p.mass() / 1000.0, p.g_load), TEXT);
                    });
                }
                if self.weapons.has_weapons() {
                    let armed = self.weapons.armed;
                    let power = p.sys.powered();
                    ui.label(tag_text(t!("Weapons", "Armement")));
                    egui::Grid::new("weapons").num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                        let arm_col = if !armed { DIM } else if power { menu::OK } else { amber };
                        row(ui, t!("Master arm", "Armement général"), if armed { "ON".into() } else { "OFF".into() }, arm_col);
                        let (mut left, mut right) = (0u8, 0u8);
                        for (i, st) in self.weapons.stations.iter().enumerate() {
                            if self.weapons.loaded(i) {
                                if st.spec.side == 0 { left += 1 } else { right += 1 }
                            }
                        }
                        row(ui, "AIM-9", format!("L {left}  ·  R {right}"), TEXT);
                        let (line, col) = if !armed {
                            (t!("safe", "sécurisé").to_string(), DIM)
                        } else if !power {
                            (t!("no power", "pas d'alimentation").to_string(), amber)
                        } else if let Some(v) = &self.weapons.target_view {
                            if self.weapons.lock >= 1.0 {
                                (t!("lock  {:.1} km", "verrouillé  {:.1} km", v.range / 1000.0), menu::OK)
                            } else {
                                (t!("track  {:.1} km", "poursuite  {:.1} km", v.range / 1000.0), amber)
                            }
                        } else {
                            (t!("search", "recherche").to_string(), DIM)
                        };
                        row(ui, t!("Seeker", "Autodirecteur"), line, col);
                    });
                }
            });
        });
        Some(r.response.rect)
    }
}

// ---- the HUD over the flying, in the chase view (the cockpit has its own panel)
//
// Everything sits at the edges and leaves the middle of the view to the world:
//
//   heading strip ............ hangs from the top edge; the track, "heading"
//                              and the aircraft in small capitals under it
//   conditions card .......... top right: the time of day, the sky, the
//                              temperature, the wind, the clock
//   speed tape ............... on the left edge: the limit bands, a rolling
//                              readout, the throttle; the ground speed over it
//   altitude tape ............ on the right edge: a rolling readout, the climb
//                              over it, and a column of buttons for the flight
//                              panel's pages, the map, the camera and pause
//   controls ................. bottom right: the engines' dials, and trim,
//                              spoilers, flaps and gear in slots to drag
//   flight-path marker ....... where the aircraft is actually going
//   alerts ................... bottom centre: warnings, damage, the approach line
//
// One material, frosted glass: the world behind blurred (the renderer's
// `frost` texture), a light veil over it and a bright hairline edge, with
// round corners. One face, Barlow, for words and numbers alike (the rolling
// readouts set their digits in fixed cells so they do not jitter); labels in
// small spaced capitals.

/// Height of the heading strip at the top centre.
pub const HEADING_H: f32 = 46.0;
/// The edge tapes: height, the speed tape's width, the altitude tape's and its button column's.
const TAPE_H: f32 = 352.0;
const SPEED_W: f32 = 124.0;
const ALT_W: f32 = 104.0;
const BTN_W: f32 = 66.0;
/// Corner radii: panels, and the pieces inside them.
pub(crate) const R_HUD: u8 = 14;
pub(crate) const R_IN: u8 = 7;
/// Inset of everything inside a panel.
const PAD: f32 = 12.0;
/// What the flight panel leaves free at its right for the altitude tape.
pub const RIGHT_EDGE: f32 = ALT_W + BTN_W + 14.0;
/// Height of the radio instruments at the bottom centre, when shown.
pub const NAV_H: f32 = 156.0;

const AMBER: Color32 = Color32::from_rgb(255, 196, 80);
/// The active button's light blue.
pub const SKY: Color32 = Color32::from_rgb(56, 182, 240);
/// Soft white: the HUD's secondary text, readable over bright sky and dark ground alike.
pub const SOFT: Color32 = Color32::from_rgba_premultiplied(184, 184, 184, 184);
/// The readouts' dark boxes.
const BOX: Color32 = Color32::from_rgba_premultiplied(10, 12, 16, 225);
/// White at a share, for the dim parts.
fn white(a: f32) -> Color32 {
    Color32::from_white_alpha((255.0 * a) as u8)
}

/// The HUD's faces: a readout's digits, a scale's figures (and their minor
/// ones), small figures, and the small spaced capitals of the labels.
fn num_l() -> egui::FontId {
    menu::semibold(24.0)
}
fn num_m() -> egui::FontId {
    menu::medium(17.0)
}
fn num_minor() -> egui::FontId {
    egui::FontId::proportional(12.5)
}
fn num_s() -> egui::FontId {
    menu::medium(13.5)
}
fn tag() -> egui::FontId {
    menu::medium(11.5)
}
fn tag_text(s: &str) -> RichText {
    RichText::new(s.to_uppercase()).font(tag()).color(white(0.62)).extra_letter_spacing(1.1)
}
/// Small spaced capitals painted at a point.
pub(crate) fn paint_tag(p: &egui::Painter, at: egui::Pos2, align: Align2, s: &str) {
    paint_tag_col(p, at, align, s, white(0.62));
}
fn paint_tag_col(p: &egui::Painter, at: egui::Pos2, align: Align2, s: &str, col: Color32) {
    let mut job = egui::text::LayoutJob::default();
    job.append(&s.to_uppercase(), 0.0, egui::TextFormat { font_id: tag(), color: col, extra_letter_spacing: 1.1, ..Default::default() });
    let g = p.layout_job(job);
    let size = g.size() - vec2(1.1, 0.0);
    p.galley(align.anchor_size(at, size).min, g, col);
}

/// What the HUD shows, measured once a frame by the caller.
pub struct HudIn {
    pub hdg: f64,
    /// indicated airspeed (m/s), height above the sea and the ground (m), climb (m/s)
    pub ias: f64,
    pub alt: f64,
    pub agl: f64,
    pub vs: f64,
    /// the local time of day (hours)
    pub local: f64,
}

// ---- the frosted glass

/// Where the blurred frame is, for the panels drawn this frame (set by the game).
pub fn set_frost(ctx: &egui::Context, id: Option<egui::TextureId>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("flightui frost"), id));
}

/// The outline of a rounded rectangle, corner by corner.
pub(crate) fn rounded(r: Rect, cr: egui::CornerRadius) -> Vec<egui::Pos2> {
    let mut pts = Vec::new();
    let corners = [
        (pos2(r.right() - cr.ne as f32, r.top() + cr.ne as f32), cr.ne as f32, -90.0f32),
        (pos2(r.right() - cr.se as f32, r.bottom() - cr.se as f32), cr.se as f32, 0.0),
        (pos2(r.left() + cr.sw as f32, r.bottom() - cr.sw as f32), cr.sw as f32, 90.0),
        (pos2(r.left() + cr.nw as f32, r.top() + cr.nw as f32), cr.nw as f32, 180.0),
    ];
    for (c, rad, a0) in corners {
        if rad <= 0.5 {
            pts.push(c);
            continue;
        }
        for k in 0..=8 {
            let a = (a0 + 90.0 * k as f32 / 8.0).to_radians();
            pts.push(c + vec2(a.cos(), a.sin()) * rad);
        }
    }
    pts
}

/// Frosted glass in `r`: the world behind it blurred, a light veil, a bright edge.
fn frost_shapes(ctx: &egui::Context, r: Rect, cr: egui::CornerRadius) -> Vec<egui::Shape> {
    let mut out = Vec::new();
    let screen = ctx.content_rect();
    let pts = rounded(r, cr);
    if let Some(id) = ctx.data(|d| d.get_temp::<Option<egui::TextureId>>(egui::Id::unique("flightui frost"))).flatten() {
        let mut mesh = egui::Mesh::with_texture(id);
        let uv = |p: egui::Pos2| pos2((p.x - screen.left()) / screen.width(), (p.y - screen.top()) / screen.height());
        let c = r.center();
        mesh.vertices.push(egui::epaint::Vertex { pos: c, uv: uv(c), color: Color32::WHITE });
        for p in &pts {
            mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: uv(*p), color: Color32::WHITE });
        }
        let n = pts.len() as u32;
        for k in 0..n {
            mesh.add_triangle(0, 1 + k, 1 + (k + 1) % n);
        }
        out.push(egui::Shape::mesh(mesh));
    }
    // the veil: lifts the blur a touch and quiets it, then the edge
    out.push(egui::Shape::convex_polygon(pts.clone(), Color32::from_rgba_premultiplied(10, 14, 22, 82), Stroke::NONE));
    out.push(egui::Shape::closed_line(pts, Stroke::new(1.0, white(0.28))));
    out
}

/// Paints frosted glass in `r` with the painter.
fn frost_rect(p: &egui::Painter, r: Rect, cr: egui::CornerRadius) {
    for s in frost_shapes(p.ctx(), r, cr) {
        p.add(s);
    }
}

/// A frame for the HUD's panels, its glass painted under it by `frosted`.
pub fn glass() -> egui::Frame {
    egui::Frame::new().corner_radius(R_HUD).inner_margin(PAD)
}

/// Shows `frame` with frosted glass behind its contents.
pub fn frosted<R>(ui: &mut egui::Ui, frame: egui::Frame, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    let slot = ui.painter().add(egui::Shape::Noop);
    let cr = frame.corner_radius;
    let resp = frame.show(ui, add);
    ui.painter().set(slot, egui::Shape::Vec(frost_shapes(ui.ctx(), resp.response.rect, cr)));
    resp
}

/// A rolling readout: a dark box with the leading figures steady in it, and
/// beside them a taller drum (it stands out of the box above and below) on
/// which the last figure, or the last two stepping by `roll_step` when that
/// is 10 or more, turn with the value: the figure read in the middle, the next
/// and the last ones half seen above and below. `lead`: figures to keep (the
/// leading zeros dim); 0 shows only the figures the number has. Returns the
/// box with the drum.
fn drum(p: &egui::Painter, at: egui::Pos2, align: Align2, value: f32, lead: usize, roll_step: i32, col: Color32) -> Rect {
    let big = num_l();
    let small = menu::semibold(17.0);
    let cw = p.layout_no_wrap("0".into(), big.clone(), col).size().x;
    let sw = p.layout_no_wrap("0".into(), small.clone(), col).size().x;
    let v = value.max(0.0);
    let roll_digits = if roll_step >= 10 { 2 } else { 1 };
    let unit = 10f32.powi(roll_digits as i32);
    let top_part = (v / unit).floor();
    let k = (v - top_part * unit) / roll_step as f32;
    let digits = format!("{}", top_part as i64);
    let n_lead = if lead > digits.len() { lead } else if top_part == 0.0 && lead == 0 { 0 } else { digits.len() };
    let shown = if n_lead == 0 { String::new() } else { format!("{:0w$}", top_part as i64, w = n_lead) };
    let pad = n_lead.saturating_sub(digits.len());
    // sizes: the box holds the steady figures; the drum is a figure wider and taller
    let drum_w = sw * roll_digits as f32 + 10.0;
    let box_w = 12.0 + cw * shown.len().max(1) as f32 + drum_w + 4.0;
    let r = align.anchor_size(at, vec2(box_w, 40.0));
    p.rect_filled(r, R_IN, BOX);
    let dr = Rect::from_min_max(pos2(r.right() - drum_w - 4.0, r.top() - 8.0), pos2(r.right() - 4.0, r.bottom() + 8.0));
    p.rect_filled(dr, R_IN, Color32::from_rgba_premultiplied(18, 21, 27, 240));
    let clip = p.with_clip_rect(dr.shrink(1.0));
    let cell_h = 21.0;
    for d in -2..=2 {
        let s = k.floor() + d as f32;
        let n = ((s * roll_step as f32) as i32).rem_euclid(unit as i32);
        let y = r.center().y - (s - k) * cell_h;
        let a = 1.0 - ((y - r.center().y).abs() / cell_h).min(1.0) * 0.7;
        clip.text(pos2(dr.center().x, y), Align2::CENTER_CENTER, format!("{:0w$}", n, w = roll_digits), small.clone(), col.gamma_multiply(a));
    }
    let mut x = dr.left() - 4.0 - cw * shown.len() as f32;
    for (i, ch) in shown.chars().enumerate() {
        let c = if i < pad { col.gamma_multiply(0.32) } else { col };
        p.text(pos2(x + cw * 0.5, r.center().y), Align2::CENTER_CENTER, ch.to_string(), big.clone(), c);
        x += cw;
    }
    r.union(dr)
}

/// "12 480": thousands set apart with a thin space.
fn grouped(v: f32) -> String {
    let n = v.round() as i64;
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{2009}');
        }
        out.push(ch);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// The time of day as people say it.
fn period(h: f64) -> &'static str {
    match h.rem_euclid(24.0) {
        h if (5.0..11.0).contains(&h) => t!("Morning", "Matin"),
        h if (11.0..17.0).contains(&h) => t!("Afternoon", "Après-midi"),
        h if (17.0..21.0).contains(&h) => t!("Evening", "Soirée"),
        _ => t!("Night", "Nuit"),
    }
}

/// The conditions card's line icons, one stroke weight.
#[derive(Copy, Clone)]
enum Glyph {
    Sun,
    Moon,
    Cloud,
    Menu,
    Thermometer,
    WindFrom(f32),
    Sock,
    Clock,
}

fn glyph(p: &egui::Painter, g: Glyph, c: egui::Pos2, s: f32, col: Color32) {
    let st = Stroke::new(if s > 20.0 { 1.8 } else { 1.4 }, col);
    let u = s * 0.5;
    let arc = |c: egui::Pos2, r: f32, a0: f32, a1: f32| -> Vec<egui::Pos2> { (0..=20).map(|i| { let a = a0 + (a1 - a0) * i as f32 / 20.0; c + vec2(a.cos(), a.sin()) * r }).collect() };
    match g {
        Glyph::Sun => {
            p.circle_stroke(c, u * 0.4, st);
            for k in 0..8 {
                let a = k as f32 / 8.0 * std::f32::consts::TAU;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * u * 0.62, c + d * u * 0.92], st);
            }
        }
        Glyph::Moon => {
            let mut v = arc(c, u * 0.8, 0.9, 5.4);
            v.extend(arc(c + vec2(0.35 * u, -0.2 * u), u * 0.62, 4.7, 1.6).into_iter());
            p.add(egui::Shape::closed_line(v, st));
        }
        Glyph::Cloud => menu::icon(p, Icon::Weather, c, s, col),
        Glyph::Menu => {
            for k in [-1.0, 0.0, 1.0] {
                p.line_segment([c + vec2(-u * 0.75, k * u * 0.55), c + vec2(u * 0.75, k * u * 0.55)], st);
            }
        }
        Glyph::Thermometer => {
            p.circle_stroke(c + vec2(0.0, u * 0.55), u * 0.3, st);
            p.line_segment([c + vec2(-u * 0.16, u * 0.3), c + vec2(-u * 0.16, -u * 0.8)], st);
            p.line_segment([c + vec2(u * 0.16, u * 0.3), c + vec2(u * 0.16, -u * 0.8)], st);
            p.add(egui::Shape::line(arc(c + vec2(0.0, -u * 0.8), u * 0.16, std::f32::consts::PI, std::f32::consts::TAU), st));
        }
        Glyph::WindFrom(deg) => {
            p.circle_stroke(c, u * 0.85, st);
            // the arrow points where the wind goes
            let a = (deg + 180.0).to_radians();
            let d = vec2(a.sin(), -a.cos());
            p.line_segment([c - d * u * 0.4, c + d * u * 0.45], st);
            let side = vec2(-d.y, d.x);
            p.line_segment([c + d * u * 0.45, c + d * u * 0.1 + side * u * 0.3], st);
            p.line_segment([c + d * u * 0.45, c + d * u * 0.1 - side * u * 0.3], st);
        }
        Glyph::Sock => {
            p.line_segment([c + vec2(-u * 0.9, -u * 0.8), c + vec2(-u * 0.9, u * 0.9)], st);
            let pts = vec![c + vec2(-u * 0.9, -u * 0.7), c + vec2(u * 0.9, -u * 0.35), c + vec2(u * 0.9, u * 0.05), c + vec2(-u * 0.9, u * 0.15)];
            p.add(egui::Shape::closed_line(pts, st));
            for x in [-0.3, 0.3] {
                p.line_segment([c + vec2(x * u, -u * 0.55 + x * 0.2 * u), c + vec2(x * u, u * 0.1)], st);
            }
        }
        Glyph::Clock => {
            p.circle_stroke(c, u * 0.85, st);
            p.line_segment([c, c + vec2(0.0, -u * 0.5)], st);
            p.line_segment([c, c + vec2(u * 0.38, u * 0.1)], st);
        }
    }
}

impl Game {
    /// The whole HUD (see above); the buttons' presses go into `bar`.
    pub(crate) fn hud(&mut self, ctx: &egui::Context, h: &HudIn, bar: &mut Option<Bar>) {
        let screen = ctx.content_rect();
        // the tapes' centre line, a little above the middle: the map goes under the speed tape
        let cy = screen.center().y - 36.0;
        self.heading_strip(ctx, h.hdg as f32);
        if !self.info_open {
            self.conditions_card(ctx, h);
        }
        self.speed_tape(ctx, h, cy);
        self.altitude_tape(ctx, h, cy, bar);
        self.path_marker(ctx);
        self.minimap.top = cy + TAPE_H * 0.5 + 12.0;
        self.minimap.bottom = 14.0;
    }

    fn heading_strip(&self, ctx: &egui::Context, hdg: f32) {
        let screen = ctx.content_rect();
        let w = 540.0;
        let at = pos2(screen.center().x - w * 0.5, screen.top());
        egui::Area::new(egui::Id::unique("hud_heading")).fixed_pos(at).interactable(false).order(egui::Order::Background).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(w, HEADING_H + 26.0), Sense::hover());
            let strip = Rect::from_min_size(r.min, vec2(w, HEADING_H));
            let p = ui.painter();
            // hanging from the top edge: only its lower corners are round
            frost_rect(p, strip, egui::CornerRadius { nw: 0, ne: 0, sw: R_HUD, se: R_HUD });
            let clip = p.with_clip_rect(strip.shrink(2.0));
            const PPD: f32 = 8.4;
            let lo = ((hdg - 40.0) / 10.0).floor() as i32 * 10;
            let y = strip.center().y + 1.0;
            for d in (lo..=lo + 80).step_by(10) {
                let x = strip.center().x + (d as f32 - hdg) * PPD;
                let dd = d.rem_euclid(360);
                let edge = ((x - strip.left()).min(strip.right() - x) / 70.0).clamp(0.0, 1.0);
                let near = 1.0 - ((x - strip.center().x).abs() / 42.0).min(1.0);
                // the one under the index grows and brightens as it comes in
                let font = egui::FontId::new(17.0 + 5.0 * near, egui::FontFamily::Name(if near > 0.5 { menu::SEMIBOLD } else { menu::MEDIUM }.into()));
                let label = match dd { 0 => "N".to_string(), 90 => "E".to_string(), 180 => "S".to_string(), 270 => t!("W", "O").to_string(), _ => format!("{dd:03}") };
                clip.text(pos2(x, y), Align2::CENTER_CENTER, label, font, white((0.7 + 0.3 * near) * edge));
            }
            // the index: a short bar under the middle
            p.rect_filled(Rect::from_center_size(pos2(strip.center().x, strip.bottom() - 4.0), vec2(34.0, 2.5)), 1, Color32::WHITE);
            // and under the strip, its small capitals: track, the word, the aircraft
            let ty = strip.bottom() + 12.0;
            let pl = &self.plane;
            let (e, n, _) = crate::geo::enu(pl.pos);
            let trk = pl.vel.dot(e).atan2(pl.vel.dot(n)).to_degrees().rem_euclid(360.0);
            if pl.vel.length() > 3.0 {
                paint_tag(p, pos2(strip.left() + 16.0, ty), Align2::LEFT_CENTER, &t!("Track {:03.0}°", "Route {:03.0}°", trk));
            }
            paint_tag(p, pos2(strip.center().x, ty), Align2::CENTER_CENTER, t!("Heading", "Cap"));
            paint_tag(p, pos2(strip.right() - 16.0, ty), Align2::RIGHT_CENTER, self.spec.name);
        });
    }

    fn conditions_card(&self, ctx: &egui::Context, h: &HudIn) {
        let wx = &self.wx;
        let wind_kt = wx.wind * 1.94384;
        let night = !(6.0..19.5).contains(&h.local.rem_euclid(24.0));
        let cloudy = wx.clouds > 0.6 || wx.rain > 0.1 || wx.storms > 0.3;
        let sky = if wx.storms > 0.3 {
            t!("Storms", "Orages")
        } else if wx.rain > 0.1 {
            t!("Rain", "Pluie")
        } else if wx.clouds < 0.15 {
            t!("Clear", "Dégagé")
        } else if wx.clouds < 0.45 {
            t!("Some cloud", "Quelques nuages")
        } else if wx.clouds < 0.8 {
            t!("Cloudy", "Nuageux")
        } else {
            t!("Overcast", "Couvert")
        };
        let breeze = match wind_kt {
            w if w < 3.0 => t!("no wind", "sans vent"),
            w if w < 11.0 => t!("a light breeze", "brise légère"),
            w if w < 21.0 => t!("a moderate breeze", "brise modérée"),
            w if w < 34.0 => t!("a strong wind", "vent fort"),
            _ => t!("a gale", "tempête"),
        };
        let sub = t!("{} with {}", "{}, {}", sky, breeze);
        let oat = 15.0 - 0.0065 * h.alt.clamp(-500.0, 11_000.0);
        let w = 284.0;
        egui::Area::new(egui::Id::unique("hud_conditions")).anchor(Align2::RIGHT_TOP, [-14.0, 12.0]).interactable(false).order(egui::Order::Background).show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(vec2(w, 126.0), Sense::hover());
            let p = ui.painter();
            frost_rect(p, r, R_HUD.into());
            // the head: how to open the menu
            let hy = r.top() + 19.0;
            glyph(p, Glyph::Menu, pos2(r.left() + 22.0, hy), 14.0, white(0.75));
            let hint = if padui::active() { tr("Y + View: flight panel") } else { t!("Tab: flight panel", "Tab : panneau de vol") };
            paint_tag(p, pos2(r.left() + 38.0, hy), Align2::LEFT_CENTER, hint);
            p.hline(r.x_range(), r.top() + 38.0, Stroke::new(1.0, white(0.16)));
            // the body: the sky, the time of day, the temperature
            let by = r.top() + 70.0;
            let g = if cloudy { Glyph::Cloud } else if night { Glyph::Moon } else { Glyph::Sun };
            glyph(p, g, pos2(r.left() + 32.0, by), 32.0, Color32::WHITE);
            p.text(pos2(r.left() + 62.0, by - 9.0), Align2::LEFT_CENTER, period(h.local), menu::medium(19.0), Color32::WHITE);
            p.text(pos2(r.left() + 62.0, by + 11.0), Align2::LEFT_CENTER, sub, menu::caption(), white(0.68));
            glyph(p, Glyph::Thermometer, pos2(r.right() - 20.0, by - 9.0), 16.0, white(0.8));
            p.text(pos2(r.right() - 32.0, by - 9.0), Align2::RIGHT_CENTER, format!("{oat:.0}°C"), num_s(), Color32::WHITE);
            // the foot: wind direction, its strength, the clock
            let fy = r.bottom() - 18.0;
            let mut x = r.left() + 22.0;
            glyph(p, Glyph::WindFrom(wx.wind_from as f32), pos2(x, fy), 16.0, white(0.8));
            x += 14.0;
            p.text(pos2(x, fy), Align2::LEFT_CENTER, format!("{:03.0}°", wx.wind_from), num_s(), white(0.85));
            x += 58.0;
            glyph(p, Glyph::Sock, pos2(x, fy), 15.0, white(0.8));
            p.text(pos2(x + 14.0, fy), Align2::LEFT_CENTER, t!("{:.0} knots", "{:.0} nœuds", wind_kt), num_s(), white(0.85));
            p.text(pos2(r.right() - 16.0, fy), Align2::RIGHT_CENTER, menu::hhmm(h.local), num_s(), white(0.85));
            glyph(p, Glyph::Clock, pos2(r.right() - 62.0, fy), 16.0, white(0.8));
        });
    }

    fn speed_tape(&self, ctx: &egui::Context, h: &HudIn, cy: f32) {
        let screen = ctx.content_rect();
        let p = &self.plane;
        let kt = |ms: f64| (ms * 1.943_84) as f32;
        // The airspeed jitters a little frame to frame and the drum turns a
        // figure per knot: the tape and readout follow it through a short
        // low-pass (about a quarter of a second) so they glide.
        let ias = {
            let raw = kt(h.ias).max(0.0);
            let id = egui::Id::unique("hud ias smooth");
            let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
            let prev = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(raw);
            let v = if (raw - prev).abs() > 40.0 { raw } else { prev + (raw - prev) * (1.0 - (-dt / 0.22).exp()) };
            ctx.data_mut(|d| d.insert_temp(id, v));
            v
        };
        let panel = Rect::from_min_size(pos2(screen.left(), cy - TAPE_H * 0.5), vec2(SPEED_W, TAPE_H));
        egui::Area::new(egui::Id::unique("hud_speed")).fixed_pos(panel.min - vec2(0.0, 24.0)).interactable(false).order(egui::Order::Background).show(ctx, |ui| {
            ui.allocate_exact_size(vec2(SPEED_W + 20.0, TAPE_H + 24.0), Sense::hover());
            let pt = ui.painter();
            // over it: the ground speed
            paint_tag(pt, pos2(panel.left() + 14.0, panel.top() - 12.0), Align2::LEFT_CENTER, &format!("G/S  {:.0}", kt(p.vel.length())));
            frost_rect(pt, panel, egui::CornerRadius { nw: 0, sw: 0, ne: R_HUD, se: R_HUD });
            // the scale lives inside a padded window; figures fade out towards its ends
            let tape = Rect::from_min_max(pos2(panel.left(), panel.top() + 8.0), pos2(panel.right() - 46.0, panel.bottom() - 8.0));
            let clip = pt.with_clip_rect(tape);
            let c = pos2(tape.center().x, cy);
            const PPK: f32 = 6.0;
            let y_of = |v: f32| c.y - (v - ias) * PPK;
            let fade = |y: f32| ((y - tape.top()).min(tape.bottom() - y) / 36.0).clamp(0.0, 1.0);
            // The limits, a strip along the screen's edge (the aircraft's own: stall, flaps, caution, never-exceed).
            if p.spec.rocket().is_none() {
                let (vs_now, vs_clean, vs_full) = (kt(p.stall_ias_at(p.flaps)), kt(p.stall_ias_at(0.0)), kt(p.stall_ias_at(1.0)));
                let vne = kt(p.spec.vne);
                let band = |a: f32, b: f32, x: f32, w: f32, col: Color32| {
                    let (ya, yb) = (y_of(a.max(0.0)), y_of(b));
                    if ya > tape.top() && yb < tape.bottom() {
                        clip.rect_filled(Rect::from_min_max(pos2(x, yb.max(tape.top())), pos2(x + w, ya.min(tape.bottom()))), 0.0, col);
                    }
                };
                band(vs_clean, vne * 0.9, tape.left(), 5.0, Color32::from_rgb(64, 200, 96));
                band(vne * 0.9, vne, tape.left(), 5.0, AMBER);
                band(vne, vne + 500.0, tape.left(), 5.0, menu::BAD);
                if p.spec.flaps.len() > 1 {
                    if let Some(vfe) = p.spec.vfe.iter().copied().reduce(f64::max) {
                        band(vs_full, kt(vfe), tape.left() + 5.0, 3.0, white(0.9));
                    }
                }
                // below the stall: red hatching
                let (ya, yb) = (y_of(0.0).min(tape.bottom()), y_of(vs_now).max(tape.top()));
                if ya > yb {
                    let hp = clip.with_clip_rect(Rect::from_min_max(pos2(tape.left(), yb), pos2(tape.left() + 7.0, ya)));
                    let mut y = yb - 8.0;
                    while y < ya + 8.0 {
                        hp.line_segment([pos2(tape.left(), y + 7.0), pos2(tape.left() + 7.0, y)], Stroke::new(2.5, menu::BAD));
                        y += 6.0;
                    }
                }
            }
            // the scale: tens large and white, fives small and dim; nothing under the readout
            let lo = ((ias - 40.0) / 5.0).floor() as i32 * 5;
            for v in (lo.max(0)..=lo + 80).step_by(5) {
                let y = y_of(v as f32);
                let a = fade(y);
                if a <= 0.0 || (y - c.y).abs() < 30.0 {
                    continue;
                }
                let big = v % 10 == 0;
                clip.line_segment([pos2(tape.left() + 10.0, y), pos2(tape.left() + if big { 18.0 } else { 14.0 }, y)], Stroke::new(1.3, white(if big { 0.85 } else { 0.5 } * a)));
                if big {
                    clip.text(pos2(tape.left() + 26.0, y), Align2::LEFT_CENTER, format!("{v}"), num_m(), white(a));
                } else {
                    clip.text(pos2(tape.left() + 26.0, y), Align2::LEFT_CENTER, format!("{v}"), num_minor(), white(0.5 * a));
                }
            }
            // the rolling readout, its pointer at the screen's edge
            let warn = p.stalled || p.overspeed() > 0.0 || (p.flaps > 0.25 && p.ias() > p.vfe());
            let col = if p.stalled { menu::BAD } else if warn { AMBER } else { Color32::WHITE };
            let rr = drum(pt, pos2(tape.left() + 12.0, c.y), Align2::LEFT_CENTER, ias, 0, 1, col);
            pt.add(egui::Shape::convex_polygon(vec![pos2(tape.left() + 5.0, c.y), pos2(tape.left() + 12.5, c.y - 7.0), pos2(tape.left() + 12.5, c.y + 7.0)], BOX, Stroke::NONE));
            if warn {
                pt.rect_stroke(rr, R_IN, Stroke::new(1.5, col), egui::StrokeKind::Outside);
            }
            // the throttle: its own column at the right, a bar filling up, its share under it
            let col_r = Rect::from_min_max(pos2(panel.right() - 40.0, panel.top() + 12.0), pos2(panel.right() - 12.0, panel.bottom() - 12.0));
            paint_tag(pt, pos2(col_r.center().x, col_r.top() + 6.0), Align2::CENTER_CENTER, t!("Thr", "Gaz"));
            let track = Rect::from_min_max(pos2(col_r.center().x - 3.0, col_r.top() + 20.0), pos2(col_r.center().x + 3.0, col_r.bottom() - 26.0));
            pt.rect_filled(track, 3, white(0.14));
            let thr = (p.throttle as f32).clamp(0.0, 1.0);
            pt.rect_filled(Rect::from_min_max(pos2(track.left(), track.bottom() - track.height() * thr), track.max), 3, if p.running { SKY } else { menu::BAD });
            pt.text(pos2(col_r.center().x, col_r.bottom() - 8.0), Align2::CENTER_CENTER, format!("{:.0}%", thr * 100.0), num_s(), Color32::WHITE);
        });
    }

    fn altitude_tape(&self, ctx: &egui::Context, h: &HudIn, cy: f32, bar: &mut Option<Bar>) {
        let screen = ctx.content_rect();
        let ft = |m: f64| (m * 3.280_84) as f32;
        let alt = ft(h.alt);
        let vs_fpm = (h.vs * 196.85) as f32;
        let agl = ft(h.agl.max(0.0));
        let w = ALT_W + BTN_W;
        let panel = Rect::from_min_size(pos2(screen.right() - w, cy - TAPE_H * 0.5), vec2(w, TAPE_H));
        egui::Area::new(egui::Id::unique("hud_alt")).fixed_pos(panel.min - vec2(0.0, 24.0)).order(egui::Order::Background).show(ctx, |ui| {
            ui.allocate_exact_size(vec2(w, TAPE_H + 48.0), Sense::hover());
            let pt = ui.painter().clone();
            // over it: the climb; under it, near the ground, the height above it
            let vs_col = if vs_fpm < -1500.0 && agl < 2500.0 { menu::BAD } else { Color32::WHITE };
            let vy = panel.top() - 12.0;
            paint_tag(&pt, pos2(panel.right() - 14.0, vy), Align2::RIGHT_CENTER, "V/S");
            pt.text(pos2(panel.right() - 48.0, vy), Align2::RIGHT_CENTER, format!("{:+.0}", (vs_fpm / 10.0).round() * 10.0), num_s(), vs_col);
            if agl < 2500.0 && !self.plane.on_ground {
                let col = if agl < 300.0 { AMBER } else { white(0.62) };
                paint_tag_col(&pt, pos2(panel.right() - 14.0, panel.bottom() + 12.0), Align2::RIGHT_CENTER, &format!("{:.0} ft AGL", agl), col);
            }
            frost_rect(&pt, panel, egui::CornerRadius { nw: R_HUD, sw: R_HUD, ne: 0, se: 0 });
            let col_x = panel.left() + BTN_W;
            pt.vline(col_x, panel.top() + 12.0..=panel.bottom() - 12.0, Stroke::new(1.0, white(0.14)));
            let tape = Rect::from_min_max(pos2(col_x + 1.0, panel.top() + 8.0), pos2(panel.right(), panel.bottom() - 8.0));
            let c = pos2(tape.center().x, cy);
            let clip = pt.with_clip_rect(tape);
            const PPF: f32 = 0.6;
            let y_of = |v: f32| c.y - (v - alt) * PPF;
            let fade = |y: f32| ((y - tape.top()).min(tape.bottom() - y) / 36.0).clamp(0.0, 1.0);
            let lo = ((alt - 340.0) / 50.0).floor() as i32 * 50;
            for v in (lo..=lo + 700).step_by(50) {
                let y = y_of(v as f32);
                let a = fade(y);
                if a <= 0.0 || (v < 0 && alt >= -50.0) {
                    continue;
                }
                let big = v % 100 == 0;
                clip.line_segment([pos2(tape.right() - 4.0, y), pos2(tape.right() - if big { 14.0 } else { 9.0 }, y)], Stroke::new(1.3, white(if big { 0.85 } else { 0.5 } * a)));
                if (y - c.y).abs() < 30.0 {
                    continue;
                }
                if big {
                    clip.text(pos2(tape.right() - 20.0, y), Align2::RIGHT_CENTER, grouped(v as f32), num_m(), white(a));
                } else {
                    clip.text(pos2(tape.right() - 20.0, y), Align2::RIGHT_CENTER, grouped(v as f32), num_minor(), white(0.5 * a));
                }
            }
            // the rolling readout, inside the tape's column, its pointer at the scale
            let rr = drum(&pt, pos2(tape.right() - 18.0, c.y), Align2::RIGHT_CENTER, alt, 3, 20, Color32::WHITE);
            pt.add(egui::Shape::convex_polygon(vec![pos2(tape.right() - 10.0, c.y), pos2(rr.right() - 0.5, c.y - 7.0), pos2(rr.right() - 0.5, c.y + 7.0)], BOX, Stroke::NONE));
            // The buttons: the flight panel's pages, the map, the camera, the pause menu.
            let items: [(Icon, &str, &str, Bar, bool); 6] = [
                (Icon::Flight, t!("Flight", "Vol"), t!("Flight", "Vol"), Bar::Page(Page::Flight), self.info_open && self.info_page == Page::Flight),
                (Icon::GoTo, t!("Go to", "Aller"), t!("Go to", "Aller à"), Bar::Page(Page::Places), self.info_open && self.info_page == Page::Places),
                (Icon::Radio, t!("Radio", "Radio"), t!("Radio", "Radio"), Bar::Page(Page::Radio), self.info_open && self.info_page == Page::Radio),
                (Icon::Weather, t!("Weather", "Météo"), t!("Weather", "Météo"), Bar::Page(Page::Weather), self.info_open && self.info_page == Page::Weather),
                (Icon::Map, t!("Map", "Carte"), t!("Map", "Carte"), Bar::Map, self.minimap.visible),
                (Icon::Camera, t!("Camera", "Caméra"), t!("Camera (C)", "Caméra (C)"), Bar::Camera, false),
            ];
            let bs = BTN_W - 18.0;
            let gap = (TAPE_H - 20.0 - bs * items.len() as f32) / (items.len() - 1) as f32;
            for (k, (ic, label, tip, act, on)) in items.into_iter().enumerate() {
                let br = Rect::from_min_size(pos2(panel.left() + 9.0, panel.top() + 10.0 + k as f32 * (bs + gap)), vec2(bs, bs));
                let resp = ui.interact(br, ui.scope_id().with(("hud_btn", k)), Sense::click()).on_hover_text(tip);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tip));
                let lit = menu::ease(ctx, resp.id, resp.hovered() || resp.has_focus());
                if on {
                    pt.rect_filled(br, R_IN, SKY);
                } else {
                    pt.rect_filled(br, R_IN, white(0.05 + 0.08 * lit));
                    pt.rect_stroke(br, R_IN, Stroke::new(1.0, white(0.28 + 0.2 * lit)), egui::StrokeKind::Inside);
                }
                menu::icon(&pt, ic, pos2(br.center().x, br.top() + bs * 0.4), 17.0, Color32::WHITE);
                let mut job = egui::text::LayoutJob::default();
                job.append(&label.to_uppercase(), 0.0, egui::TextFormat { font_id: menu::medium(8.5), color: white(if on { 1.0 } else { 0.72 }), extra_letter_spacing: 0.6, ..Default::default() });
                let g = pt.layout_job(job);
                pt.galley(pos2(br.center().x - (g.size().x - 0.6) * 0.5, br.bottom() - 5.0 - g.size().y), g, Color32::WHITE);
                if resp.clicked() {
                    *bar = Some(act);
                }
            }
        });
    }

    /// The flight-path marker: where the aircraft is going, not where its nose points.
    fn path_marker(&self, ctx: &egui::Context) {
        let Some((vp, cam)) = self.live_view else { return };
        let p = &self.plane;
        if p.vel.length() < 15.0 || p.on_ground {
            return;
        }
        let target = p.pos + p.vel.normalize() * 2000.0;
        let c = vp * (target - cam).as_vec3().extend(1.0);
        if c.w <= 0.0 {
            return;
        }
        let screen = ctx.content_rect();
        let at = pos2(screen.left() + (c.x / c.w * 0.5 + 0.5) * screen.width(), screen.top() + (0.5 - c.y / c.w * 0.5) * screen.height());
        if !screen.shrink(40.0).contains(at) {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::background());
        for (w, col) in [(3.0, Color32::from_black_alpha(90)), (1.5, Color32::from_white_alpha(235))] {
            let st = Stroke::new(w, col);
            painter.line_segment([at + vec2(-9.0, 0.0), at + vec2(9.0, 0.0)], st);
            painter.line_segment([at + vec2(0.0, -9.0), at + vec2(0.0, 9.0)], st);
        }
    }

    /// Bottom centre, stacked upwards: the approach line, the alerts, and what
    /// has broken. `lift`: room kept under it (the radio instruments).
    pub(crate) fn alerts(&self, ctx: &egui::Context, warn: &[(&str, Color32)], report: &[String], guidance: Option<&(String, u8)>, lift: f32) {
        if warn.is_empty() && report.is_empty() && guidance.is_none() {
            return;
        }
        egui::Area::new(egui::Id::unique("hud_alerts")).anchor(Align2::CENTER_BOTTOM, [0.0, -16.0 - lift]).interactable(false).show(ctx, |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                if let Some((text, col)) = guidance {
                    let col = match col { 1 => Color32::from_rgb(64, 200, 96), 2 => AMBER, 3 => menu::BAD, _ => Color32::WHITE };
                    frosted(ui, glass().inner_margin(egui::Margin::symmetric(16, 8)), |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, col);
                            ui.label(RichText::new(text).font(num_s()).color(Color32::WHITE));
                        });
                    });
                }
                if !warn.is_empty() {
                    ui.horizontal(|ui| {
                        for (w, col) in warn {
                            // a lit plate, glowing in its colour
                            egui::Frame::new().fill(col.gamma_multiply(0.92)).corner_radius(R_HUD).inner_margin(egui::Margin::symmetric(16, 7)).shadow(egui::Shadow { offset: [0, 0], blur: 22, spread: 2, color: col.gamma_multiply(0.55) }).show(ui, |ui| {
                                ui.label(RichText::new(w.to_uppercase()).font(menu::semibold(14.0)).color(Color32::WHITE).extra_letter_spacing(1.0));
                            });
                        }
                    });
                }
                if !report.is_empty() {
                    frosted(ui, glass(), |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(tag_text(t!("Damage", "Dégâts")).color(AMBER));
                        for l in report {
                            ui.label(RichText::new(l.as_str()).font(num_s()).color(Color32::WHITE));
                        }
                    });
                }
            });
        });
    }
}
