//! Satellite/road moving map. Bounded background tile loading; no second scene render.
mod data;

use crate::{geo::{self, TileId}, sources::Sources};
use egui::{Color32, Pos2, Stroke, Vec2};
use std::{collections::HashMap, sync::{Arc, mpsc}, time::{Duration, Instant}};

struct Tile {
    texture: Option<egui::TextureHandle>,
    touched: u64,
    retry: Instant,
}

struct MapTile {
    texture: Option<egui::TextureHandle>,
    places: Vec<data::Place>,
    touched: u64,
    retry: Instant,
}

pub struct Minimap {
    pub visible: bool,
    pub landing_guidance: bool,
    pub heading_up: bool,
    pub zoom: u8,
    pub plan: bool,
    pub size: Vec2,
    resize_start: Vec2,
    maps: HashMap<TileId, MapTile>,
    map_pending: Option<(TileId, mpsc::Receiver<Option<data::Data>>)>,
    heading: f64,
    scroll: f32,
    tiles: HashMap<TileId, Tile>,
    pending: Option<(TileId, u32, mpsc::Receiver<Option<image::RgbaImage>>)>,
    generation: u32,
    frame: u64,
    runway_airports: Vec<u32>,
    runway_center: Option<glam::DVec3>,
}

fn rotate(v: Vec2, angle: f64) -> Vec2 {
    let (s, c) = (angle as f32).sin_cos();
    Vec2::new(c * v.x - s * v.y, s * v.x + c * v.y)
}

fn tile_offset(x: f64, center: f64, n: f64) -> f64 {
    (x - center + n * 0.5).rem_euclid(n) - n * 0.5
}

fn distance_label(meters: f64, lang: crate::i18n::Lang) -> String {
    let text = if meters >= 1000.0 { format!("{:.1} km", meters / 1000.0) } else { format!("{meters:.0} m") };
    if lang == crate::i18n::Lang::Fr { text.replace('.', ",") } else { text }
}

impl Minimap {
    fn draw_runways(&mut self, world: &crate::world::World, painter: &egui::Painter, rect: egui::Rect, lat: f64, lon: f64, cx: f64, cy: f64, n: f64, angle: f64) {
        // Below this zoom individual runways are too small to distinguish.
        if self.zoom < 9 { return; }
        let db = crate::airports::Db::get();
        let position = geo::lla_to_pos(lat, lon, 0.0);
        if self.runway_center.is_none_or(|center| (position - center).length() > 10_000.0) {
            self.runway_airports = db.near(lat.to_degrees(), lon.to_degrees(), 120_000.0).into_iter().map(|(id, _)| id).collect();
            self.runway_center = Some(position);
        }
        let project = |(la, lo): (f64, f64)| {
            let x = geo::lon_to_merc_x(lo.to_radians()).rem_euclid(1.0) * n;
            let y = geo::lat_to_merc_y(la.to_radians()) * n;
            rect.center() + rotate(Vec2::new((tile_offset(x, cx, n) * 256.0) as f32, ((y - cy) * 256.0) as f32), angle)
        };
        let color = Color32::from_rgb(95, 235, 255);
        let mut occupied = vec![
            egui::Rect::from_center_size(rect.center(), Vec2::splat(38.0)),
            egui::Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 43.0)),
            egui::Rect::from_min_max(rect.left_bottom() - Vec2::new(0.0, 40.0), rect.left_bottom() + Vec2::new(90.0, 0.0)),
        ];
        for &id in &self.runway_airports {
            let airport = &db.apts[id as usize];
            // Prefer the exact geometry used by the 3D airfield and approach guide.
            // Some database entries (including EBNM) have no endpoint coordinates.
            let loaded = world.airfield(id).filter(|field| !field.geo.runways.is_empty());
            let runways: Vec<_> = if let Some(field) = loaded {
                field.geo.runways.iter().map(|r| {
                    let endpoints = [r.a, r.b].map(|p| {
                        let (lat, lon) = field.geo.lla(p);
                        (lat.to_degrees(), lon.to_degrees())
                    });
                    (endpoints, &r.ident)
                }).collect()
            } else {
                airport.rwys.iter().filter_map(|r| {
                    let [Some(a), Some(b)] = r.pos else { return None; };
                    Some(([a, b], &r.ident))
                }).collect()
            };
            for ([a, b], ident) in runways {
                let ends = [project(a), project(b)];
                if !egui::Rect::from_two_pos(ends[0], ends[1]).expand(4.0).intersects(rect) { continue; }
                let length = ends[0].distance(ends[1]);
                if length < 2.0 { continue; }
                painter.line_segment(ends, Stroke::new(6.0, Color32::from_black_alpha(220)));
                painter.line_segment(ends, Stroke::new(3.0, color));
                if self.zoom < 11 || length < 18.0 { continue; }
                let direction = (ends[1] - ends[0]) / length;
                let side = Vec2::new(-direction.y, direction.x);
                for end in ends {
                    painter.line_segment([end - side * 4.0, end + side * 4.0], Stroke::new(2.0, color));
                }
                let text = format!("{}  {}/{}", airport.ident, ident[0], ident[1]);
                let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.0), color);
                let midpoint = ends[0] + (ends[1] - ends[0]) * 0.5;
                let Some(label) = [-1.0, 1.0].into_iter().map(|sign| {
                    egui::Rect::from_center_size(midpoint + Vec2::new(0.0, sign * (galley.size().y * 0.5 + 8.0)), galley.size() + Vec2::new(6.0, 4.0))
                }).find(|r| rect.shrink(3.0).contains_rect(*r) && !occupied.iter().any(|other| other.intersects(*r))) else { continue; };
                painter.rect_filled(label, 3.0, Color32::from_black_alpha(225));
                painter.galley(label.min + Vec2::new(3.0, 2.0), galley, color);
                occupied.push(label);
            }
        }
    }

    fn draw_map(&mut self, _ctx: &egui::Context, src: &Arc<Sources>, painter: &egui::Painter, rect: egui::Rect, cx: f64, cy: f64, n: f64, angle: f64) -> usize {
        let z = self.zoom.min(14);
        let factor = 2f64.powi((self.zoom - z) as i32);
        let (mx, my, count) = (cx / factor, cy / factor, (1u32 << z) as i64);
        let mut wanted = Vec::new();
        let mut visible = Vec::new();
        let mut drawn = 0;
        for dy in -2..=2 { for dx in -2..=2 {
            let (x, y) = (mx.floor() as i64 + dx, my.floor() as i64 + dy);
            if y < 0 || y >= count { continue; }
            let id = TileId::new(z, x.rem_euclid(count) as u32, y as u32);
            let offset = Vec2::new(tile_offset(x as f64, mx, count as f64) as f32, (y as f64 - my) as f32);
            let corners = [Vec2::ZERO, Vec2::X, Vec2::splat(1.0), Vec2::Y].map(|v| rect.center() + rotate((offset + v) * (256.0 * factor as f32), angle));
            if !egui::Rect::from_points(&corners).intersects(rect) { continue; }
            visible.push(id);
            wanted.push((dx * dx + dy * dy, id));
            if let Some(tile) = self.maps.get_mut(&id) {
                tile.touched = self.frame;
                if self.plan {
                    if let Some(texture) = &tile.texture {
                        let mut mesh = egui::Mesh::with_texture(texture.id());
                        for (pos, uv) in corners.into_iter().zip([[0.0,0.0],[1.0,0.0],[1.0,1.0],[0.0,1.0]]) {
                            mesh.vertices.push(egui::epaint::Vertex { pos, uv: Pos2::new(uv[0],uv[1]), color: Color32::WHITE });
                        }
                        mesh.indices.extend([0,1,2,0,2,3]);
                        painter.add(mesh);
                        drawn += 1;
                    }
                }
            }
        } }
        if self.map_pending.is_none() {
            wanted.sort_by_key(|(d, _)| *d);
            if let Some((_, id)) = wanted.into_iter().find(|(_, id)| self.maps.get(id).is_none_or(|t| t.texture.is_none() && Instant::now() >= t.retry)) {
                let (tx, rx) = mpsc::channel();
                let src = Arc::clone(src);
                self.map_pending = Some((id, rx));
                std::thread::spawn(move || { let _ = tx.send(data::load(&src, id)); });
            }
        }
        let mut labels: Vec<_> = visible.iter().filter_map(|id| self.maps.get(id)).flat_map(|tile| &tile.places)
            .filter(|p| self.zoom >= p.min_zoom).collect();
        labels.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.name.cmp(&b.name)).then(a.xy[0].total_cmp(&b.xy[0])));
        let north = rect.center() + rotate(Vec2::new(0.0, -rect.width().min(rect.height()) * 0.5 + 54.0), angle);
        let mut occupied = vec![egui::Rect::from_center_size(rect.center(), Vec2::splat(38.0)), egui::Rect::from_center_size(north, Vec2::splat(26.0)),
            egui::Rect::from_min_max(rect.left_bottom() + Vec2::new(0.0, -37.0), rect.left_bottom() + Vec2::new(90.0, 0.0))];
        occupied.push(egui::Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 43.0)));
        let mut names = std::collections::HashSet::new();
        for place in labels {
            let name = place.label(crate::i18n::lang());
            if names.contains(name) { continue; }
            let offset = Vec2::new((tile_offset(place.xy[0] * n, cx, n) * 256.0) as f32, ((place.xy[1] * n - cy) * 256.0) as f32);
            let point = rect.center() + rotate(offset, angle);
            let font = egui::FontId::proportional(if place.priority == 0 { 13.0 } else { 11.0 });
            let galley = painter.layout_no_wrap(name.to_owned(), font, Color32::WHITE);
            // Try above and below so a town underneath the aircraft can still be named.
            let Some(label_rect) = [-1.0, 1.0].into_iter().map(|side| egui::Rect::from_center_size(point + Vec2::new(0.0, side * (galley.size().y * 0.5 + 6.0)), galley.size() + Vec2::new(6.0, 2.0)))
                .find(|r| rect.shrink(3.0).contains_rect(*r) && !occupied.iter().any(|other| other.intersects(r.expand(3.0)))) else { continue };
            painter.circle_filled(point, 2.5, Color32::WHITE);
            painter.circle_stroke(point, 2.5, Stroke::new(1.0, Color32::from_gray(40)));
            painter.rect_filled(label_rect, 3.0, Color32::from_black_alpha(185));
            painter.galley(label_rect.min + Vec2::new(3.0, 1.0), galley, Color32::WHITE);
            occupied.push(label_rect);
            names.insert(name);
            if names.len() >= 8 { break; }
        }
        drawn
    }

    pub fn new(prefs: &crate::settings::Settings) -> Self {
        Self { visible: prefs.minimap, landing_guidance: prefs.landing_guidance, heading_up: prefs.minimap_heading_up, zoom: prefs.minimap_zoom,
            size: Vec2::new(prefs.minimap_size, prefs.minimap_height), resize_start: Vec2::ZERO, plan: prefs.minimap_plan, maps: HashMap::new(), map_pending: None, heading: 0.0, scroll: 0.0, tiles: HashMap::new(), pending: None, generation: 0, frame: 0, runway_airports: Vec::new(), runway_center: None }
    }

    fn poll(&mut self, ctx: &egui::Context, generation: u32) {
        if let Some((id, rx)) = &self.map_pending {
            match rx.try_recv() {
                Ok(result) => {
                    let (texture, places) = result.map(|d| {
                        let texture = ctx.load_texture(format!("minimap-plan-{id:?}"), egui::ColorImage::from_rgba_unmultiplied([256, 256], d.image.as_raw()), egui::TextureOptions::LINEAR);
                        (Some(texture), d.places)
                    }).unwrap_or_default();
                    self.maps.insert(*id, MapTile { texture, places, touched: self.frame, retry: Instant::now() + Duration::from_secs(30) });
                    self.map_pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => { self.map_pending = None; }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        while self.maps.len() > 32 {
            let oldest = *self.maps.iter().min_by_key(|(_, t)| t.touched).unwrap().0;
            self.maps.remove(&oldest);
        }
        if generation != self.generation {
            self.tiles.clear();
            self.generation = generation;
        }
        if let Some((id, gen_id, rx)) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    if *gen_id == generation {
                        let texture = result.map(|img| {
                            ctx.load_texture(format!("minimap-{id:?}-{generation}"),
                                egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()),
                                egui::TextureOptions::LINEAR)
                        });
                        self.tiles.insert(*id, Tile { texture, touched: self.frame, retry: Instant::now() + Duration::from_secs(30) });
                    }
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => { self.pending = None; }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        while self.tiles.len() > 64 {
            let oldest = *self.tiles.iter().min_by_key(|(_, t)| t.touched).unwrap().0;
            self.tiles.remove(&oldest);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, src: &Arc<Sources>, lat: f64, lon: f64, heading: Option<f64>, info_open: bool, guide: Option<&crate::approach::Guide>, world: &crate::world::World) {
        self.frame += 1;
        self.poll(ctx, src.google.generation());
        if let Some(h) = heading { self.heading = h; }
        let reserved_width = if info_open { 426.0 } else { 44.0 };
        let limit = Vec2::new(
            (ctx.content_rect().width() - reserved_width).clamp(120.0, 640.0),
            (ctx.content_rect().height() - 100.0).clamp(120.0, 640.0),
        );
        let size = self.size.clamp(Vec2::splat(240.0), Vec2::splat(640.0)).min(limit);
        egui::Area::new(egui::Id::unique("minimap")).anchor(egui::Align2::LEFT_BOTTOM, [14.0, -65.0]).show(ctx, |ui| {
            egui::Frame::new().fill(crate::menu::GLASS).corner_radius(8.0).inner_margin(8.0).show(ui, |ui| {
                ui.set_width(size.x);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                if response.hovered() {
                    let scroll = ui.input_mut(|i| {
                        let s = i.smooth_scroll_delta.y;
                        i.smooth_scroll_delta = Vec2::ZERO;
                        s
                    });
                    self.scroll += scroll;
                    let steps = (self.scroll / 40.0).trunc() as i32;
                    self.scroll -= steps as f32 * 40.0;
                    self.zoom = (self.zoom as i32 + steps).clamp(3, 17) as u8;
                } else {
                    self.scroll = 0.0;
                }
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, Color32::from_rgb(22, 32, 42));
                let angle = if self.heading_up { -self.heading } else { 0.0 };
                let n = (1u32 << self.zoom) as f64;
                let cx = geo::lon_to_merc_x(lon).rem_euclid(1.0) * n;
                let cy = geo::lat_to_merc_y(lat) * n;
                let supported = lat.abs() <= 85.0_f64.to_radians();
                let mut wanted = Vec::new();
                let mut drawn = 0;
                if supported && !self.plan {
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            let x = cx.floor() as i64 + dx;
                            let y = cy.floor() as i64 + dy;
                            if y < 0 || y >= n as i64 { continue; }
                            let id = TileId::new(self.zoom, x.rem_euclid(n as i64) as u32, y as u32);
                            let offset = Vec2::new(tile_offset(x as f64, cx, n) as f32, (y as f64 - cy) as f32) * 256.0;
                            let corners = [Vec2::ZERO, Vec2::new(256.0, 0.0), Vec2::splat(256.0), Vec2::new(0.0, 256.0)]
                                .map(|v| rect.center() + rotate(offset + v, angle));
                            if !egui::Rect::from_points(&corners).intersects(rect) { continue; }
                            // Coarse coverage first, then detail; reuse ancestors while zooming.
                            let coarse = id.ancestor(self.zoom.saturating_sub(2)).0;
                            wanted.push((0, coarse));
                            wanted.push((1, id));
                            for z in (0..=self.zoom).rev() {
                                let (a, u, v, scale) = id.ancestor(z);
                                if let Some(tile) = self.tiles.get_mut(&a) {
                                    tile.touched = self.frame;
                                    if let Some(texture) = &tile.texture {
                                        let mut mesh = egui::Mesh::with_texture(texture.id());
                                        let uv = [[u, v], [u + scale, v], [u + scale, v + scale], [u, v + scale]];
                                        for (pos, uv) in corners.into_iter().zip(uv) {
                                            mesh.vertices.push(egui::epaint::Vertex { pos, uv: Pos2::new(uv[0] as f32, uv[1] as f32), color: Color32::WHITE });
                                        }
                                        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                                        painter.add(mesh);
                                        drawn += 1;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                wanted.sort_by_key(|(priority, id)| (*priority, (id.x as i64 - (cx / 2f64.powi((self.zoom - id.z) as i32)) as i64).abs()));
                if !self.plan && self.pending.is_none() {
                    if let Some((_, id)) = wanted.into_iter().find(|(_, id)| self.tiles.get(id).is_none_or(|t| t.texture.is_none() && Instant::now() >= t.retry)) {
                        let (tx, rx) = mpsc::channel();
                        let src = Arc::clone(src);
                        self.pending = Some((id, self.generation, rx));
                        std::thread::spawn(move || {
                            let img = src.imagery(id).ok().map(|img| image::imageops::resize(&img, 256, 256, image::imageops::FilterType::Triangle));
                            let _ = tx.send(img);
                        });
                    }
                }
                if supported {
                    let map_drawn = self.draw_map(ctx, src, &painter, rect, cx, cy, n, angle);
                    if self.plan { drawn = map_drawn; }
                    self.draw_runways(world, &painter, rect, lat, lon, cx, cy, n, angle);
                    if let Some(guide) = guide { guide.draw_map(&painter, rect, cx, cy, n, angle); }
                }
                if !supported || drawn == 0 {
                    painter.text(rect.center() + Vec2::new(0.0, 32.0), egui::Align2::CENTER_CENTER,
                        if !supported { crate::t!("Map unavailable near poles", "Carte indisponible aux pôles") }
                        else if if self.plan { self.map_pending.is_some() } else { self.pending.is_some() } { crate::t!("Loading map…", "Chargement…") }
                        else { crate::t!("Imagery unavailable", "Images indisponibles") }, egui::FontId::proportional(12.0), Color32::WHITE);
                }
                let arrow = [Vec2::new(0.0, -13.0), Vec2::new(9.0, 10.0), Vec2::new(-9.0, 10.0)];
                let points: Vec<_> = arrow.into_iter().map(|v| rect.center() + rotate(v, self.heading + angle)).collect();
                painter.add(egui::Shape::convex_polygon(points, Color32::from_rgb(255, 210, 70), Stroke::new(2.0, Color32::BLACK)));
                let north = rect.center() + rotate(Vec2::new(0.0, -size.x.min(size.y) * 0.5 + 54.0), angle);
                painter.circle_filled(north, 10.0, Color32::from_black_alpha(190));
                painter.text(north, egui::Align2::CENTER_CENTER, "N", egui::FontId::proportional(13.0), Color32::WHITE);
                let meters = 2.0 * std::f64::consts::PI * geo::R * lat.cos() / (n * 256.0) * 60.0;
                let a = rect.left_bottom() + Vec2::new(10.0, -10.0);
                painter.line_segment([a, a + Vec2::new(60.0, 0.0)], Stroke::new(4.0, Color32::BLACK));
                painter.line_segment([a, a + Vec2::new(60.0, 0.0)], Stroke::new(2.0, Color32::WHITE));
                let label = distance_label(meters, crate::i18n::lang());
                let label_rect = egui::Rect::from_min_size(a + Vec2::new(-3.0, -23.0), Vec2::new(76.0, 20.0));
                painter.rect_filled(label_rect, 3.0, Color32::from_black_alpha(190));
                painter.text(label_rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), Color32::WHITE);
                // Controls float over the map instead of adding rows to its height.
                let toolbar_rect = egui::Rect::from_min_max(rect.min + Vec2::new(22.0, 6.0), Pos2::new(rect.right() - 5.0, rect.top() + 38.0));
                let mut toolbar = ui.new_child(egui::UiBuilder::new().max_rect(toolbar_rect));
                egui::Frame::new().fill(Color32::from_black_alpha(210)).corner_radius(5.0).inner_margin(3.0).show(&mut toolbar, |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.spacing_mut().button_padding = Vec2::new(5.0, 4.0);
                    ui.style_mut().override_font_id = Some(egui::FontId::proportional(12.0));
                    ui.horizontal(|ui| {
                        let button = egui::Button::selectable(self.heading_up, if self.heading_up { crate::t!("Heading", "Cap") } else { crate::t!("North", "Nord") });
                        if ui.add_sized([48.0, 26.0], button).on_hover_text(crate::t!("Toggle north up / heading up", "Basculer nord en haut / cap en haut")).clicked() { self.heading_up = !self.heading_up; }
                        let button = egui::Button::selectable(self.plan, if self.plan { crate::t!("Map", "Plan") } else { "Satellite" });
                        if ui.add_sized([62.0, 26.0], button).on_hover_text(crate::t!("Toggle satellite / map", "Basculer satellite / plan")).clicked() { self.plan = !self.plan; }
                        if ui.add_enabled(self.zoom > 3, egui::Button::new("-").min_size(Vec2::splat(26.0))).on_hover_text(crate::t!("Zoom out", "D\u{00e9}zoomer")).clicked() { self.zoom -= 1; }
                        if ui.add_enabled(self.zoom < 17, egui::Button::new("+").min_size(Vec2::splat(26.0))).on_hover_text(crate::t!("Zoom in", "Zoomer")).clicked() { self.zoom += 1; }
                        if ui.add_sized([26.0, 26.0], egui::Button::new("x")).on_hover_text(crate::t!("Hide map", "Masquer la carte")).clicked() { self.visible = false; }
                    });
                });
                // Pull the top corner opposite the anchored bottom corner.
                let corner = rect.right_top();
                let grip = egui::Rect::from_min_size(corner - Vec2::new(20.0, 0.0), Vec2::splat(20.0));
                let resize = ui.interact(grip, egui::Id::unique("minimap_resize"), egui::Sense::drag())
                    .on_hover_cursor(egui::CursorIcon::ResizeNeSw)
                    .on_hover_text(crate::t!("Drag to resize the map", "Glisser pour redimensionner la carte"));
                if resize.drag_started() { self.resize_start = size; }
                if resize.dragged() {
                    let d = resize.total_drag_delta().unwrap_or_default();
                    self.size = (self.resize_start + Vec2::new(d.x, -d.y)).clamp(Vec2::splat(240.0).min(limit), limit);
                }
                painter.rect_filled(grip, 3.0, Color32::from_black_alpha(210));
                for inset in [5.0, 10.0, 15.0] {
                    painter.line_segment([corner + Vec2::new(-3.0, inset), corner + Vec2::new(-inset, 3.0)], Stroke::new(1.5, Color32::WHITE));
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scale_uses_localized_decimal_separator() {
        assert_eq!(distance_label(1500.0, crate::i18n::Lang::Fr), "1,5 km");
        assert_eq!(distance_label(1500.0, crate::i18n::Lang::En), "1.5 km");
        assert_eq!(distance_label(250.0, crate::i18n::Lang::Fr), "250 m");
    }
    #[test]
    fn heading_up_keeps_aircraft_up_and_rotates_east_up() {
        let heading = std::f64::consts::FRAC_PI_2;
        assert!((rotate(Vec2::new(10.0, 0.0), -heading) - Vec2::new(0.0, -10.0)).length() < 1e-5);
        assert!((rotate(Vec2::new(0.0, -10.0), heading - heading) - Vec2::new(0.0, -10.0)).length() < 1e-5);
    }
    #[test]
    fn antimeridian_uses_nearest_copy() {
        assert!((tile_offset(0.0, 255.75, 256.0) - 0.25).abs() < 1e-8);
        assert!((tile_offset(255.0, 0.25, 256.0) + 1.25).abs() < 1e-8);
    }
}
