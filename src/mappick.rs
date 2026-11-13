//! Место на карте для «Местности по координатам»: снимок Esri с подписями и
//! рамка будущей карты посередине. Карту тащат мышью, колесом приближают, рамку
//! меняют за углы. Середина рамки — координаты, её стороны — клетки. Поиск по
//! названию — Nominatim (OpenStreetMap), один запрос на нажатие.
//!
//! Плитки качаются фоном: в программе — потоками, в модуле — `fetch` браузера
//! (`kartografGet` в `foundry/scripts/main.js`).

use crate::place;
use crate::procgen::osm::CELL_M;
use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};

const LABELS_URL: &str = "https://server.arcgisonline.com/ArcGIS/rest/services/Reference/World_Boundaries_and_Places/MapServer/tile";
const ZOOM_MIN: u32 = 2;
/// Крупнее у Esri есть не везде, а рамка в 8 клеток видна и тут.
const ZOOM_MAX: u32 = 19;
/// Сколько плиток держать картинками: 256×256, ≈0,25 МБ каждая.
const KEEP: usize = 400;
const MAP: Vec2 = Vec2 { x: 720.0, y: 480.0 };
/// Ручка угла рамки: за неё берут на таком расстоянии, точек.
const GRIP: f32 = 12.0;
const FRAME: Color32 = Color32::from_rgb(255, 200, 60);

/// Что выбрали: середина рамки и её стороны в клетках.
pub struct Pick {
    pub lat: f64,
    pub lon: f64,
    pub cells: [u32; 2],
}

enum Tile {
    /// Картинка и кадр, когда её рисовали последний раз.
    Ready(TextureHandle, u64),
    /// Не скачалась или заглушка Esri «нет снимка»: рисуется плитка мельче.
    Missing,
}

pub struct MapPick {
    lat: f64,
    lon: f64,
    zoom: u32,
    cells: [u32; 2],
    tiles: HashMap<String, Tile>,
    pending: HashSet<String>,
    fetch: Fetcher,
    frame: u64,
    /// Тянут угол рамки, а не карту.
    resizing: bool,
    wheel: f32,
    query: String,
    searching: Option<String>,
    note: Option<String>,
}

/// Широта и долгота точки мира `(px, py)` на масштабе `z` (обратное к [`place::tile_px`]).
fn lat_lon(px: f64, py: f64, z: u32) -> (f64, f64) {
    let n = 256.0 * 2f64.powi(z as i32);
    let lon = (px / n * 360.0 - 180.0 + 540.0).rem_euclid(360.0) - 180.0;
    let lat = (std::f64::consts::PI * (1.0 - 2.0 * py / n)).sinh().atan().to_degrees();
    (lat.clamp(-85.0, 85.0), lon)
}

/// Масштаб, на котором `metres` занимают `share` от меньшей стороны карты.
fn zoom_for(lat: f64, metres: f64, share: f64) -> u32 {
    let mpp = metres.max(1.0) / (share * MAP.x.min(MAP.y) as f64);
    let z = (place::mpp_at(lat, 0) / mpp).log2().floor();
    (z.max(0.0) as u32).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Адрес поиска Nominatim: всё, кроме букв и цифр ASCII, — `%XX`.
fn search_url(query: &str) -> String {
    let q: String = query.trim().bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("https://nominatim.openstreetmap.org/search?q={q}&format=jsonv2&limit=1")
}

impl MapPick {
    /// С места `at` (что уже вписано в окно), иначе — весь мир.
    pub fn new(at: Option<(f64, f64)>, cells: [u32; 2]) -> Self {
        let (lat, lon, zoom) = match at {
            Some((lat, lon)) => (lat, lon, zoom_for(lat, cells[0].max(cells[1]) as f64 * CELL_M as f64, 0.45)),
            None => (30.0, 10.0, ZOOM_MIN),
        };
        Self { lat, lon, zoom, cells, tiles: HashMap::new(), pending: HashSet::new(), fetch: Fetcher::default(), frame: 0, resizing: false, wheel: 0.0, query: String::new(), searching: None, note: None }
    }

    /// Окно; `Some` — закрыли: с выбором («Готово») или без.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<Pick>> {
        self.frame += 1;
        self.receive(ctx);
        let mut done = None;
        let mut open = true;
        egui::Window::new("Место на карте").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Город, улица, место").desired_width(320.0));
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let can = self.searching.is_none() && !self.query.trim().is_empty();
                let clicked = ui.add_enabled(can, egui::Button::new("Найти")).clicked();
                if can && (clicked || enter) {
                    let url = search_url(&self.query);
                    self.note = None;
                    self.fetch.get(ctx, &url);
                    self.searching = Some(url);
                }
                if self.searching.is_some() {
                    ui.spinner();
                }
                if let Some(n) = &self.note {
                    ui.weak(n);
                }
            });
            self.map(ui);
            let m = |c: u32| c as f32 * CELL_M;
            ui.horizontal(|ui| {
                ui.strong(format!("{} × {} клеток", self.cells[0], self.cells[1]));
                ui.label(format!("{:.0} × {:.0} м", m(self.cells[0]), m(self.cells[1])));
                ui.weak(format!("{:.5}, {:.5}", self.lat, self.lon));
            });
            ui.weak("Карту — мышью, колесо — ближе и дальше, углы рамки — её размер.");
            ui.horizontal(|ui| {
                if ui.button("Готово").clicked() {
                    done = Some(Some(Pick { lat: self.lat, lon: self.lon, cells: self.cells }));
                }
                if ui.button("Отмена").clicked() {
                    done = Some(None);
                }
                ui.weak("Снимок и подписи © Esri, поиск © OpenStreetMap");
            });
        });
        if !open {
            done = Some(None);
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        done
    }

    fn map(&mut self, ui: &mut egui::Ui) {
        let (rect, resp) = ui.allocate_exact_size(MAP, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(28));
        let z = self.zoom;
        let (cx, cy) = place::tile_px(self.lat, self.lon, z);
        let mid = rect.center();
        let n = 1i64 << z;
        let (hw, hh) = (rect.width() as f64 / 2.0, rect.height() as f64 / 2.0);
        let tiles = |a: f64| (a / 256.0).floor() as i64;
        for ty in tiles(cy - hh)..=tiles(cy + hh) {
            if !(0..n).contains(&ty) {
                continue;
            }
            for tx in tiles(cx - hw)..=tiles(cx + hw) {
                let at = Pos2::new(mid.x + (tx as f64 * 256.0 - cx) as f32, mid.y + (ty as f64 * 256.0 - cy) as f32);
                let r = Rect::from_min_size(at, Vec2::splat(256.0));
                let wx = tx.rem_euclid(n);
                self.draw_tile(ui.ctx(), &painter, place::TILE_URL, z, wx, ty, r, true);
                self.draw_tile(ui.ctx(), &painter, LABELS_URL, z, wx, ty, r, false);
            }
        }

        // Рамка будущей карты посередине, снаружи притемнено.
        let mpp = place::mpp_at(self.lat, z) as f32;
        let half = Vec2::new(self.cells[0] as f32 * CELL_M / mpp / 2.0, self.cells[1] as f32 * CELL_M / mpp / 2.0);
        let frame = Rect::from_center_size(mid, half * 2.0);
        let shade = Color32::from_black_alpha(110);
        for part in [
            Rect::from_min_max(rect.min, Pos2::new(rect.max.x, frame.min.y)),
            Rect::from_min_max(Pos2::new(rect.min.x, frame.max.y), rect.max),
            Rect::from_min_max(Pos2::new(rect.min.x, frame.min.y), Pos2::new(frame.min.x, frame.max.y)),
            Rect::from_min_max(Pos2::new(frame.max.x, frame.min.y), Pos2::new(rect.max.x, frame.max.y)),
        ] {
            let part = part.intersect(rect);
            if part.is_positive() {
                painter.rect_filled(part, 0.0, shade);
            }
        }
        painter.rect_stroke(frame, 0.0, Stroke::new(2.0, FRAME), egui::StrokeKind::Outside);
        let corners = [frame.left_top(), frame.right_top(), frame.left_bottom(), frame.right_bottom()];
        for c in corners {
            painter.rect_filled(Rect::from_center_size(c, Vec2::splat(9.0)), 2.0, FRAME);
        }
        // Рамка мельче ручек — крестик, чтобы было видно, где середина.
        if half.x.min(half.y) < 6.0 {
            painter.line_segment([mid - Vec2::new(10.0, 0.0), mid + Vec2::new(10.0, 0.0)], Stroke::new(2.0, FRAME));
            painter.line_segment([mid - Vec2::new(0.0, 10.0), mid + Vec2::new(0.0, 10.0)], Stroke::new(2.0, FRAME));
        }

        let near_corner = |p: Pos2| corners.iter().any(|c| c.distance(p) <= GRIP);
        // Угол или карта — по точке нажатия: тяга начинается, когда курсор уже
        // отошёл, и от угла он к тому времени бывает дальше ручки.
        if resp.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
            self.resizing = origin.is_some_and(near_corner);
        }
        if resp.dragged() {
            match resp.interact_pointer_pos() {
                // Угол — рамка вокруг середины: середина на месте, стороны до курсора.
                Some(p) if self.resizing => {
                    let d = (p - mid).abs();
                    for (k, half) in [d.x, d.y].into_iter().enumerate() {
                        self.cells[k] = ((2.0 * half * mpp / CELL_M).round() as u32).clamp(8, 256);
                    }
                }
                _ => {
                    let d = resp.drag_delta();
                    (self.lat, self.lon) = lat_lon(cx - d.x as f64, cy - d.y as f64, z);
                }
            }
        }
        if resp.drag_stopped() {
            self.resizing = false;
        }
        if let Some(p) = resp.hover_pos() {
            let icon = if self.resizing || near_corner(p) {
                if (p.x < mid.x) == (p.y < mid.y) { CursorIcon::ResizeNwSe } else { CursorIcon::ResizeNeSw }
            } else if resp.dragged() {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            };
            ui.ctx().set_cursor_icon(icon);
            // Колесо — масштаб вокруг курсора: точка под ним остаётся на месте.
            self.wheel += ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, .. } => delta.y / 50.0,
                        egui::Event::MouseWheel { delta, .. } => delta.y,
                        _ => 0.0,
                    })
                    .sum::<f32>()
            });
            while self.wheel.abs() >= 1.0 {
                let step = self.wheel.signum();
                self.wheel -= step;
                let z2 = if step > 0.0 { (self.zoom + 1).min(ZOOM_MAX) } else { self.zoom.saturating_sub(1).max(ZOOM_MIN) };
                if z2 == self.zoom {
                    continue;
                }
                let (cx, cy) = place::tile_px(self.lat, self.lon, self.zoom);
                let off = p - mid;
                let k = 2f64.powi(z2 as i32 - self.zoom as i32);
                let (wx, wy) = ((cx + off.x as f64) * k, (cy + off.y as f64) * k);
                (self.lat, self.lon) = lat_lon(wx - off.x as f64, wy - off.y as f64, z2);
                self.zoom = z2;
            }
        } else {
            self.wheel = 0.0;
        }
    }

    /// Плитка слоя в `r`: готовая — сама, нет — у снимка мельче (растянутая),
    /// и спросить её.
    #[allow(clippy::too_many_arguments)]
    fn draw_tile(&mut self, ctx: &egui::Context, painter: &egui::Painter, layer: &str, z: u32, tx: i64, ty: i64, r: Rect, fallback: bool) {
        let url = format!("{layer}/{z}/{ty}/{tx}");
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        match self.tiles.get_mut(&url) {
            Some(Tile::Ready(tex, used)) => {
                *used = self.frame;
                painter.image(tex.id(), r, full, Color32::WHITE);
                return;
            }
            Some(Tile::Missing) => {}
            None => {
                if self.pending.insert(url.clone()) {
                    self.fetch.get(ctx, &url);
                }
            }
        }
        if !fallback {
            return;
        }
        for d in 1..=z.min(6) {
            let (px, py) = (tx >> d, ty >> d);
            if let Some(Tile::Ready(tex, used)) = self.tiles.get_mut(&format!("{layer}/{}/{py}/{px}", z - d)) {
                *used = self.frame;
                let size = 1.0 / (1u32 << d) as f32;
                let uv = Rect::from_min_size(Pos2::new((tx - (px << d)) as f32 * size, (ty - (py << d)) as f32 * size), Vec2::splat(size));
                painter.image(tex.id(), r, uv, Color32::WHITE);
                return;
            }
        }
    }

    /// Что скачалось: плитки — в картинки, поиск — на место.
    fn receive(&mut self, ctx: &egui::Context) {
        for (url, got) in self.fetch.take() {
            self.pending.remove(&url);
            if self.searching.as_deref() == Some(url.as_str()) {
                self.searching = None;
                self.found(got);
                continue;
            }
            let tile = got.ok().and_then(|b| image::load_from_memory(&b).ok()).and_then(|img| {
                if url.starts_with(place::TILE_URL) && place::blank(&img.to_rgb8()) {
                    return None;
                }
                let rgba = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                Some(Tile::Ready(ctx.load_texture(&url, ci, egui::TextureOptions::LINEAR), self.frame))
            });
            self.tiles.insert(url, tile.unwrap_or(Tile::Missing));
        }
        // Старые картинки — вон, давно не рисованные первыми.
        if self.tiles.len() > KEEP {
            let mut used: Vec<(u64, String)> = self.tiles.iter().filter_map(|(k, t)| if let Tile::Ready(_, u) = t { Some((*u, k.clone())) } else { None }).collect();
            used.sort();
            for (_, k) in used.into_iter().take(self.tiles.len() - KEEP / 2) {
                self.tiles.remove(&k);
            }
        }
    }

    /// Ответ поиска: на место и масштаб по его рамке.
    fn found(&mut self, got: Result<Vec<u8>, String>) {
        let first = got.ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|j| j.as_array().and_then(|a| a.first().cloned()));
        let num = |v: &serde_json::Value| v.as_str().and_then(|s| s.parse::<f64>().ok());
        let Some(hit) = first else {
            self.note = Some("Не нашлось".into());
            return;
        };
        let (Some(lat), Some(lon)) = (num(&hit["lat"]), num(&hit["lon"])) else {
            self.note = Some("Не нашлось".into());
            return;
        };
        // boundingbox: юг, север, запад, восток.
        let b: Vec<f64> = hit["boundingbox"].as_array().map(|a| a.iter().filter_map(num).collect()).unwrap_or_default();
        let span = if b.len() == 4 { ((b[1] - b[0]) * 110_574.0).max((b[3] - b[2]) * 111_320.0 * lat.to_radians().cos()) } else { 200.0 };
        (self.lat, self.lon) = (lat, lon);
        self.zoom = zoom_for(lat, span, 0.8).min(18);
        // Целиком название раздвигало окно шире карты.
        self.note = hit["display_name"].as_str().map(|s| {
            let short: String = s.chars().take(40).collect();
            if short.len() < s.len() { format!("{short}…") } else { short }
        });
    }
}

/// Скачивание по адресу фоном. В программе — потоки: последнее спрошенное
/// идёт первым (видимые сейчас плитки); в модуле — `fetch` браузера.
#[derive(Default)]
struct Fetcher {
    #[cfg(not(target_arch = "wasm32"))]
    native: Option<native::Pool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Fetcher {
    fn get(&mut self, ctx: &egui::Context, url: &str) {
        self.native.get_or_insert_with(|| native::Pool::new(ctx.clone())).push(url.to_owned());
    }

    fn take(&mut self) -> Vec<(String, Result<Vec<u8>, String>)> {
        self.native.as_ref().map(|p| p.take()).unwrap_or_default()
    }
}

#[cfg(target_arch = "wasm32")]
impl Fetcher {
    fn get(&mut self, _ctx: &egui::Context, url: &str) {
        crate::plat::net_get_async(url);
    }

    fn take(&mut self) -> Vec<(String, Result<Vec<u8>, String>)> {
        crate::plat::net_got()
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    const WORKERS: usize = 6;

    type Done = (String, Result<Vec<u8>, String>);

    struct Shared {
        jobs: Mutex<Vec<String>>,
        wake: Condvar,
        closed: AtomicBool,
    }

    pub struct Pool {
        shared: Arc<Shared>,
        rx: Receiver<Done>,
    }

    impl Pool {
        pub fn new(ctx: eframe::egui::Context) -> Self {
            let shared = Arc::new(Shared { jobs: Mutex::new(Vec::new()), wake: Condvar::new(), closed: AtomicBool::new(false) });
            let (tx, rx): (Sender<Done>, Receiver<Done>) = channel();
            for _ in 0..WORKERS {
                let (shared, tx, ctx) = (shared.clone(), tx.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let agent = crate::place::net::agent();
                    loop {
                        let job = {
                            let mut jobs = shared.jobs.lock().unwrap_or_else(|e| e.into_inner());
                            while jobs.is_empty() && !shared.closed.load(Ordering::Relaxed) {
                                jobs = shared.wake.wait_timeout(jobs, Duration::from_secs(1)).unwrap_or_else(|e| e.into_inner()).0;
                            }
                            if shared.closed.load(Ordering::Relaxed) {
                                return;
                            }
                            jobs.pop()
                        };
                        let Some(url) = job else { continue };
                        let got = crate::place::net::get(&agent, &url);
                        if tx.send((url, got)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                });
            }
            Self { shared, rx }
        }

        pub fn push(&self, url: String) {
            self.shared.jobs.lock().unwrap_or_else(|e| e.into_inner()).push(url);
            self.shared.wake.notify_one();
        }

        pub fn take(&self) -> Vec<Done> {
            self.rx.try_iter().collect()
        }
    }

    impl Drop for Pool {
        fn drop(&mut self) {
            self.shared.closed.store(true, Ordering::Relaxed);
            self.shared.wake.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_pixels_and_coordinates_round_trip() {
        for (lat, lon) in [(39.4735, -0.37893), (-33.86, 151.21), (64.1, -21.9), (0.0, 179.9)] {
            for z in [3, 12, 19] {
                let (px, py) = place::tile_px(lat, lon, z);
                let (a, b) = lat_lon(px, py, z);
                assert!((a - lat).abs() < 1e-6 && (b - lon).abs() < 1e-6, "{lat},{lon} z{z}: {a},{b}");
            }
        }
    }

    /// Рамка 40×30 клеток (61 м) у рынка — на масштабе, где она видна целиком
    /// и не точкой; целый город (10 км) — далеко.
    #[test]
    fn zoom_fits_the_frame() {
        let z = zoom_for(39.47, 61.0, 0.45);
        let px = 61.0 / place::mpp_at(39.47, z);
        assert!((80.0..=480.0).contains(&px), "z{z}: рамка {px:.0} точек");
        assert!(zoom_for(39.47, 10_000.0, 0.8) <= 12);
    }

    /// Потоки программы качают плитку и поиск (сеть — потому и `ignore`).
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn pool_downloads_in_background() {
        let mut f = Fetcher::default();
        let ctx = egui::Context::default();
        let tile = format!("{}/18/100000/130000", place::TILE_URL);
        let find = search_url("Plaza Mayor Madrid");
        f.get(&ctx, &tile);
        f.get(&ctx, &find);
        let mut got = Vec::new();
        for _ in 0..300 {
            got.extend(f.take());
            if got.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let body = |u: &str| got.iter().find(|(k, _)| k == u).and_then(|(_, b)| b.as_ref().ok()).cloned().unwrap_or_default();
        assert!(image::load_from_memory(&body(&tile)).is_ok(), "плитка не пришла");
        let mut pick = MapPick::new(None, [40, 30]);
        pick.found(Ok(body(&find)));
        assert!((pick.lat - 40.415).abs() < 0.01 && (pick.lon + 3.707).abs() < 0.01, "поиск: {} {}", pick.lat, pick.lon);
    }

    #[test]
    fn search_url_escapes_everything_but_ascii_letters() {
        assert_eq!(search_url(" Mercat Central, València "), "https://nominatim.openstreetmap.org/search?q=Mercat%20Central%2C%20Val%C3%A8ncia&format=jsonv2&limit=1");
    }
}
