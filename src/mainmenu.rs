//! The main menu the game boots into: the "FABLE FLIGHT" title (logo.rs,
//! drawn by the renderer over this screen only) and three tiles side by
//! side: Aircraft, FLY (the largest, the globe behind it) and Settings.
//! Quit sits small in the bottom left corner. Behind it the renderer draws
//! space alone (the menu camera looks away from the planet, see `frame`).
use crate::menu::{self, Screen, ACCENT, DIM, TEXT};
use crate::padui::{self, Hint};
use crate::{aircraft, hangar_ui, t, tr, Game};
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke};

/// The picture behind the FLY tile, 4:3 at a little over twice the tile's
/// size: the game's own globe (sky.wgsl `globe_*`, live clouds) rendered
/// from 7600 km over the Caribbean (`--menu-view 24,-66,7600000 --time 8.35
/// --day 80`, exposure 0.3, a 3840x2160 frame with the menu hidden), cropped
/// so the planet rises off the right of the tile with the morning terminator
/// and the US's city lights along its left edge, the dark space to its left
/// left for the title. Swap the file to change it; nothing else depends on
/// its size beyond the aspect (it is cropped to fill the tile).
pub const GLOBE_PICTURE: &[u8] = include_bytes!("../assets/ui/globe.jpg");

/// What a tile does when pressed.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Tile {
    Aircraft,
    Fly,
    Settings,
}

impl Game {
    /// The main menu (see the module docs).
    pub(crate) fn main_menu(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let globe = self.picture(ctx, GLOBE_PICTURE);
        let spec = aircraft::nth(self.menu.aircraft);
        let mut press: Option<Tile> = None;
        let mut quit = false;
        egui::Area::new(egui::Id::unique("main_menu")).order(egui::Order::Background).fixed_pos(screen.min).show(ctx, |ui| {
            let p = ui.painter().clone();
            // Darken space towards the foot of the screen, for depth.
            gradient(&p, Rect::from_min_max(pos2(screen.left(), screen.center().y), screen.max), Color32::TRANSPARENT, Color32::from_rgba_premultiplied(4, 6, 10, 200));

            // The tiles: FLY in the middle and larger, the other two beside it.
            let gap = 18.0;
            let (fw, fh, sw, sh) = (600.0, 450.0, 300.0, 400.0);
            let total = fw + 2.0 * sw + 2.0 * gap;
            let top = 104.0;
            let k = ((screen.width() - 48.0) / total).min((screen.height() - top - 84.0) / fh).min(1.05).max(0.45);
            let (fw, fh, sw, sh, gap) = (fw * k, fh * k, sw * k, sh * k, gap * k);
            let cy = top + (screen.height() - top - 60.0) * 0.5;
            let x0 = screen.center().x - (fw + 2.0 * sw + 2.0 * gap) * 0.5;
            let air = Rect::from_min_size(pos2(x0, cy - sh * 0.5), vec2(sw, sh));
            let fly = Rect::from_min_size(pos2(air.right() + gap, cy - fh * 0.5), vec2(fw, fh));
            let set = Rect::from_min_size(pos2(fly.right() + gap, cy - sh * 0.5), vec2(sw, sh));

            // Tab and the d-pad go left to right: Aircraft, FLY, Settings.
            let r = ui.interact(air, ui.scope_id().with("tile_air"), Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t!("Aircraft", "Avions")));
            side_tile(&p, air, &r, k, t!("Aircraft", "Avions"), spec.name, |p, c, s, col| plane_icon(p, c, s, col));
            if r.clicked() {
                press = Some(Tile::Aircraft);
            }
            let r = ui.interact(fly, ui.scope_id().with("tile_fly"), Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t!("Fly", "Voler")));
            self.pad_home_focus(&r);
            fly_tile(&p, fly, &r, k, &globe);
            if r.clicked() {
                press = Some(Tile::Fly);
            }
            let r = ui.interact(set, ui.scope_id().with("tile_set"), Sense::click());
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t!("Settings", "Réglages")));
            side_tile(&p, set, &r, k, t!("Settings", "Réglages"), t!("Graphics · sound · controls", "Graphismes · son · commandes"), |p, c, s, col| gear_icon(p, c, s, col));
            if r.clicked() {
                press = Some(Tile::Settings);
            }

            // The foot: Quit at the left, the gamepad's buttons (or a word on the keys) in the middle.
            let foot_y = screen.bottom() - 34.0;
            let q = Rect::from_min_size(pos2(screen.left() + 24.0, foot_y - 16.0), vec2(96.0, 32.0));
            let qr = ui.interact(q, ui.scope_id().with("quit"), Sense::click());
            qr.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t!("Quit", "Quitter")));
            let lit = qr.hovered() || qr.has_focus();
            if lit {
                p.rect_filled(q, menu::R_WIDGET, Color32::from_white_alpha(12));
            }
            power_icon(&p, pos2(q.left() + 18.0, q.center().y), 14.0, if lit { ACCENT } else { DIM });
            p.text(pos2(q.left() + 34.0, q.center().y), Align2::LEFT_CENTER, t!("Quit", "Quitter"), FontId::proportional(14.5), if lit { Color32::WHITE } else { DIM });
            if qr.clicked() {
                quit = true;
            }
            if padui::active() {
                let mut f = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_center_size(pos2(screen.center().x, foot_y), vec2(360.0, 28.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
                padui::prompts(&mut f, &[("Dpad", tr("move")), ("A", tr("press"))], 20.0, 12.5, DIM);
            } else {
                p.text(pos2(screen.center().x, foot_y), Align2::CENTER_CENTER, t!("Enter to fly", "Entrée pour voler"), FontId::proportional(12.5), DIM.gamma_multiply(0.8));
            }
            p.text(pos2(screen.right() - 24.0, foot_y), Align2::RIGHT_CENTER, t!("Globe: Esri, Maxar, Earthstar Geographics · night lights NASA Black Marble · clouds Live Cloud Maps", "Globe : Esri, Maxar, Earthstar Geographics · lumières NASA Black Marble · nuages Live Cloud Maps"), FontId::proportional(11.0), DIM.gamma_multiply(0.7));
        });
        match press {
            Some(Tile::Fly) => self.menu.screen = Screen::Fly,
            Some(Tile::Aircraft) => self.open_aircraft(hangar_ui::Tab::Mine, Screen::Main),
            Some(Tile::Settings) => self.menu.screen = Screen::Settings,
            None => {}
        }
        if press.is_some() {
            self.pad_home = true;
        }
        if quit {
            self.quit = true;
        }
    }
}

/// The frame every tile shares: a dark glass fill, a hairline border, and
/// when hovered or picked out by the gamepad an accent border and a bar
/// along its foot.
fn tile_frame(p: &Painter, r: Rect, lit: bool, fill: bool) {
    if fill {
        p.rect_filled(r, menu::R_PANEL, if lit { Color32::from_rgba_premultiplied(24, 30, 39, 235) } else { Color32::from_rgba_premultiplied(12, 16, 22, 222) });
    }
    if lit {
        p.rect_stroke(r, menu::R_PANEL, Stroke::new(2.0, ACCENT), egui::StrokeKind::Inside);
        p.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - 4.0), r.max), 0.0, ACCENT);
    } else {
        p.rect_stroke(r, menu::R_PANEL, Stroke::new(1.0, Color32::from_white_alpha(26)), egui::StrokeKind::Inside);
    }
}

/// Aircraft and Settings: an icon, a title and a dim line under it.
fn side_tile(p: &Painter, r: Rect, resp: &egui::Response, k: f32, title: &str, sub: &str, icon: impl Fn(&Painter, Pos2, f32, Color32)) {
    let lit = resp.hovered() || resp.has_focus();
    tile_frame(p, r, lit, true);
    let c = pos2(r.center().x, r.top() + r.height() * 0.40);
    // a soft disc behind the icon
    p.circle_filled(c, 62.0 * k, Color32::from_white_alpha(if lit { 12 } else { 6 }));
    icon(p, c, 84.0 * k, if lit { ACCENT } else { Color32::from_rgb(214, 219, 227) });
    spaced(p, pos2(r.center().x, r.top() + r.height() * 0.70), Align2::CENTER_CENTER, &title.to_uppercase(), menu::bold(22.0 * k.max(0.7)), if lit { Color32::WHITE } else { TEXT }, 3.0 * k);
    p.text(pos2(r.center().x, r.top() + r.height() * 0.70 + 30.0 * k.max(0.7)), Align2::CENTER_CENTER, sub, FontId::proportional(13.0), DIM);
}

/// FLY: the globe picture filling it (easing in a little when hovered),
/// shades along the foot and the left for the words, and the word.
fn fly_tile(p: &Painter, r: Rect, resp: &egui::Response, k: f32, globe: &egui::TextureHandle) {
    let lit = resp.hovered() || resp.has_focus();
    let zoom = p.ctx().animate_bool_with_time(resp.id.with("zoom"), lit, 0.9);
    p.rect_filled(r, menu::R_PANEL, Color32::BLACK);
    // Fill the tile, cropping the picture to its aspect.
    let [iw, ih] = globe.size().map(|v| v as f32);
    let (ta, ia) = (r.width() / r.height(), iw / ih);
    let uv = if ia > ta {
        let f = ta / ia;
        Rect::from_min_max(pos2(0.5 - f * 0.5, 0.0), pos2(0.5 + f * 0.5, 1.0))
    } else {
        let f = ia / ta;
        Rect::from_min_max(pos2(0.0, 0.5 - f * 0.5), pos2(1.0, 0.5 + f * 0.5))
    };
    // A slow push in toward the planet's lit face on hover (eased).
    let z = 1.0 - 0.035 * (zoom * zoom * (3.0 - 2.0 * zoom));
    let at = uv.lerp_inside(vec2(0.72, 0.42));
    let uv = Rect::from_min_max(at + (uv.min - at) * z, at + (uv.max - at) * z);
    let tint = if lit { Color32::WHITE } else { Color32::from_gray(226) };
    p.image(globe.id(), r.shrink(1.0), uv, tint);
    // Shades for the words: across from the left, and up from the foot.
    let inner = r.shrink(1.0);
    hgradient(p, Rect::from_min_max(inner.min, pos2(inner.left() + inner.width() * 0.42, inner.bottom())), Color32::from_black_alpha(110), Color32::TRANSPARENT);
    gradient(p, Rect::from_min_max(pos2(inner.left(), inner.top() + inner.height() * 0.58), inner.max), Color32::TRANSPARENT, Color32::from_black_alpha(165));
    // and a hairline of light along the top, as on glass
    gradient(p, Rect::from_min_max(inner.min, pos2(inner.right(), inner.top() + 36.0 * k)), Color32::from_white_alpha(10), Color32::TRANSPARENT);
    tile_frame(p, r, lit, false);
    let x = r.left() + 30.0 * k;
    let base = r.bottom() - 34.0 * k;
    spaced(p, pos2(x - 2.0, base - 26.0 * k), Align2::LEFT_BOTTOM, t!("FLY", "VOLER"), menu::bold(60.0 * k.max(0.6)), if lit { ACCENT } else { Color32::WHITE }, 8.0 * k);
    p.text(pos2(x + 2.0, base - 6.0 * k), Align2::LEFT_CENTER, t!("Pick a place on the globe and take off", "Choisissez un lieu sur le globe et décollez"), FontId::proportional(15.0), Color32::from_rgb(214, 219, 227));
    // its key or button, bottom right
    let hint = Hint::of("Enter", "A");
    let hc = Color32::from_white_alpha(170);
    match hint {
        Hint::Text(h) => {
            p.text(pos2(r.right() - 26.0 * k, base - 6.0 * k), Align2::RIGHT_CENTER, h, FontId::proportional(13.0), hc);
        }
        Hint::Pad(spec) => {
            let h = 22.0;
            let w = padui::spec_width(p, spec, h);
            padui::paint_spec(p, r.right() - 26.0 * k - w, base - 6.0 * k, spec, h, hc);
        }
        Hint::None => {}
    }
}

/// Text with extra space between the letters.
fn spaced(p: &Painter, at: Pos2, align: Align2, text: &str, font: FontId, col: Color32, spacing: f32) {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat { font_id: font, color: col, extra_letter_spacing: spacing, ..Default::default() });
    let g = p.layout_job(job);
    // (the spacing trails the last letter too: leave it out of the centring)
    let size = g.size() - vec2(spacing, 0.0);
    let r = align.anchor_size(at, size);
    p.galley(r.min, g, col);
}

/// A horizontal gradient over a rectangle, `left` to `right`.
fn hgradient(p: &Painter, r: Rect, left: Color32, right: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), left);
    mesh.colored_vertex(r.right_top(), right);
    mesh.colored_vertex(r.right_bottom(), right);
    mesh.colored_vertex(r.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

/// A vertical gradient over a rectangle.
fn gradient(p: &Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

/// An airliner from above, nose up, `s` points across, in outline.
pub fn plane_icon(p: &Painter, c: Pos2, s: f32, col: Color32) {
    // Right half, nose to tail (x right, y down, in units of half the span).
    let half = [
        (0.0, -0.98),
        (0.07, -0.88),
        (0.09, -0.62),
        (0.09, -0.24),
        (0.98, 0.20),
        (0.98, 0.31),
        (0.09, 0.10),
        (0.07, 0.62),
        (0.40, 0.84),
        (0.40, 0.93),
        (0.03, 0.86),
        (0.0, 0.96),
    ];
    let u = s * 0.5;
    let mut pts: Vec<Pos2> = half.iter().map(|&(x, y)| c + vec2(x, y) * u).collect();
    pts.extend(half.iter().rev().skip(1).take(half.len() - 2).map(|&(x, y)| c + vec2(-x, y) * u));
    let st = Stroke::new((s * 0.03).clamp(1.6, 3.0), col);
    p.add(egui::Shape::closed_line(pts, st));
    // the two engines under the wings
    for sx in [-1.0, 1.0] {
        let e = c + vec2(sx * 0.42, 0.02) * u;
        p.line_segment([e - vec2(0.0, 0.12 * u), e + vec2(0.0, 0.06 * u)], Stroke::new((s * 0.05).clamp(2.0, 5.0), col));
    }
}

/// A cog: eight teeth round a ring, a hole in the middle.
pub fn gear_icon(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let (r_out, r_in, teeth) = (s * 0.46, s * 0.36, 8);
    let mut pts = Vec::new();
    for i in 0..teeth {
        let a0 = i as f32 / teeth as f32 * std::f32::consts::TAU;
        let step = std::f32::consts::TAU / teeth as f32;
        // root, rise, top across, fall
        for (f, r) in [(0.0, r_in), (0.18, r_in), (0.28, r_out), (0.62, r_out), (0.72, r_in)] {
            let a = a0 + f * step;
            pts.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    let st = Stroke::new((s * 0.03).clamp(1.6, 3.0), col);
    p.add(egui::Shape::closed_line(pts, st));
    p.circle_stroke(c, s * 0.15, st);
}

/// The power symbol: a ring open at the top, a bar through the gap.
fn power_icon(p: &Painter, c: Pos2, s: f32, col: Color32) {
    let r = s * 0.45;
    let st = Stroke::new(1.6, col);
    let pts: Vec<Pos2> = (0..=20).map(|i| {
        let a = -std::f32::consts::FRAC_PI_2 + 0.65 + i as f32 / 20.0 * (std::f32::consts::TAU - 1.3);
        c + vec2(a.cos(), a.sin()) * r
    }).collect();
    p.line(pts, st);
    p.line_segment([c - vec2(0.0, r * 1.15), c - vec2(0.0, r * 0.2)], st);
}
