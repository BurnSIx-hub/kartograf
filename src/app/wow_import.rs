//! Окно «Предметы из WoW»: клиент игры, поиск по словам пути. Предметы (M2) — вид сверху и
//! перенос на полку (папка `wow` в данных программы, на полке — «WoW/…»); здания и подземелья
//! (WMO) — карта целиком (`wow::location`): пол снимком, стены, предметы на местах.

use super::App;
use crate::library::Kind;
use crate::wow;
use crate::wow::location;
use eframe::egui;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

/// Клиент, его модели и здания.
type Opened = Result<(Arc<wow::Client>, Vec<String>, Vec<String>), String>;
/// Вид сверху выбранной модели: путь и картинка с размером в клетках (или почему нет).
type Preview = (String, Result<(egui::TextureHandle, [f32; 2]), String>);

#[derive(Default)]
pub struct WowWindow {
    pub open: bool,
    /// Что в списке: предметы (false) или здания и подземелья (true).
    pub(crate) places: bool,
    pub(crate) query: String,
    pub(crate) client: Option<Arc<wow::Client>>,
    /// Все модели клиента (`.m2`) и здания (корни `.wmo`), как записаны в списках архивов.
    pub(crate) models: Vec<String>,
    pub(crate) locations: Vec<String>,
    loading: Option<mpsc::Receiver<Opened>>,
    error: Option<String>,
    pub(crate) picked: Option<String>,
    pub(crate) picked_place: Option<String>,
    pub(crate) preview: Option<Preview>,
    report: Vec<String>,
    /// Карта собирается в фоне.
    building: Option<mpsc::Receiver<Result<location::Built, String>>>,
    /// Текущая карта не сохранена — собрать поверх только после второго нажатия.
    confirm: bool,
}

/// Больше стольких моделей за раз на полку не кладём: тысячи картинок — не полка, а свалка.
const MAX_BATCH: usize = 200;

fn open_client(dir: PathBuf) -> Opened {
    let client = wow::Client::open(&dir)?;
    let models = client.list(".m2").into_iter().map(str::to_owned).collect();
    let locations = client.list(".wmo").into_iter().filter(|p| location::is_location(p)).map(str::to_owned).collect();
    Ok((Arc::new(client), models, locations))
}

/// Пути, в которых есть все слова (без регистра).
pub fn search<'a>(models: &'a [String], query: &str) -> Vec<&'a String> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    models
        .iter()
        .filter(|m| {
            let l = m.to_lowercase();
            words.iter().all(|w| l.contains(w.as_str()))
        })
        .collect()
}

fn location_options(a: &Value) -> location::Options {
    let d = location::Options::default();
    location::Options {
        ppc: a["ppc"].as_f64().map_or(d.ppc, |v| (v as f32).clamp(16.0, 256.0)),
        cut: a["cut"].as_f64().map_or(d.cut, |v| (v as f32).clamp(1.0, 12.0)),
        set: a["set"].as_str().map(str::to_owned).or_else(|| a["set"].as_u64().map(|n| n.to_string())),
        ..d
    }
}

impl App {
    fn wow_dir(&self) -> PathBuf {
        self.data_dir.join("wow")
    }

    /// Открыть клиент в фоне (списки архивов читаются секунду-другую).
    fn wow_start(&mut self, dir: PathBuf) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(open_client(dir));
        });
        self.wow.loading = Some(rx);
        self.wow.error = None;
    }

    fn wow_take(&mut self, client: Arc<wow::Client>, models: Vec<String>, locations: Vec<String>) {
        self.settings.wow_client = Some(client.dir.clone());
        self.save_settings();
        self.wow.client = Some(client);
        self.wow.models = models;
        self.wow.locations = locations;
    }

    fn wow_poll(&mut self) {
        if let Some(rx) = &self.wow.loading {
            match rx.try_recv() {
                Ok(Ok((client, models, locations))) => {
                    self.wow.loading = None;
                    self.wow_take(client, models, locations);
                }
                Ok(Err(e)) => {
                    self.wow.error = Some(e);
                    self.wow.loading = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.wow.loading = None,
            }
        }
        if let Some(rx) = &self.wow.building {
            match rx.try_recv() {
                Ok(result) => {
                    self.wow.building = None;
                    match result {
                        Ok(built) => {
                            self.wow.report = vec![built_line(&built)];
                            self.wow.report.extend(built.notes.iter().take(4).cloned());
                            self.wow_install(built);
                        }
                        Err(e) => self.wow.report = vec![format!("Не собралось: {e}")],
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.wow.building = None,
            }
        }
    }

    /// Собранная карта — вместо текущей; новые предметы — на полку.
    fn wow_install(&mut self, built: location::Built) {
        self.library.set_wow(&self.wow_dir());
        self.reset_doc(built.map, None);
        // Новая карта ещё не сохранена: при закрытии программа спросит.
        self.dirty = true;
    }

    /// Клиент сразу (для команд ИИ): открыть, если не открыт или задана другая папка.
    pub(crate) fn wow_client_now(&mut self, dir: Option<PathBuf>) -> Result<Arc<wow::Client>, String> {
        let want = dir.or_else(|| self.settings.wow_client.clone()).ok_or("не задана папка клиента WoW (client)")?;
        if let Some(c) = &self.wow.client
            && (c.dir == want || c.archives.first().is_some_and(|a| a.path.starts_with(&want)))
        {
            return Ok(c.clone());
        }
        let (client, models, locations) = open_client(want)?;
        self.wow_take(client.clone(), models, locations);
        Ok(client)
    }

    /// Нарисовать модели сверху и положить на полку. Ключи — для `add_object` и полки.
    pub(crate) fn wow_put_on_shelf(&mut self, client: &wow::Client, models: &[String]) -> (Vec<Value>, Vec<String>) {
        let dir = self.wow_dir();
        let opt = wow::render::Options::default();
        let (mut done, mut errors) = (Vec::new(), Vec::new());
        for m in models {
            match wow::import(client, m, &dir, &opt) {
                Ok(r) => {
                    let r2 = |v: f32| (v * 100.0).round() / 100.0;
                    done.push(json!({"model": m, "key": r.file.to_string_lossy(), "size": [r2(r.cells[0]), r2(r.cells[1])], "triangles": r.drawn, "missing_textures": r.missing, "skipped_parts": r.skipped}));
                }
                Err(e) => errors.push(format!("{m}: {e}")),
            }
        }
        self.library.set_wow(&dir);
        (done, errors)
    }

    /// Собрать карту здания сразу (команда ИИ, самопроверка).
    pub(crate) fn wow_build_now(&mut self, client: &wow::Client, path: &str, opt: &location::Options) -> Result<location::Built, String> {
        location::build(client, path, &location::tiles_dir(&self.data_dir), &self.wow_dir(), opt)
    }

    pub(super) fn wow_window(&mut self, ctx: &egui::Context) {
        self.wow_poll();
        if !self.wow.open {
            return;
        }
        if self.wow.client.is_none()
            && self.wow.loading.is_none()
            && self.wow.error.is_none()
            && let Some(dir) = self.settings.wow_client.clone()
        {
            self.wow_start(dir);
        }
        if self.wow.loading.is_some() || self.wow.building.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = true;
        let mut choose = false;
        let mut put: Vec<String> = Vec::new();
        let mut build: Option<String> = None;
        let places = self.wow.places;
        let found: Vec<String> = search(if places { &self.wow.locations } else { &self.wow.models }, &self.wow.query).into_iter().cloned().collect();
        egui::Window::new("Предметы из WoW").open(&mut open).default_size([760.0, 520.0]).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Клиент:");
                match &self.wow.client {
                    Some(c) => ui.label(format!("{} ({}, архивов {})", c.dir.display(), c.locale, c.archives.len())),
                    None if self.wow.loading.is_some() => ui.label("читаю архивы…"),
                    None => ui.label("не выбран"),
                };
                choose = ui.button("Папка игры…").on_hover_text("Папка World of Warcraft 1.x–3.x, где лежит Data").clicked();
            });
            if let Some(e) = &self.wow.error {
                ui.colored_label(egui::Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.wow.places, false, "Предметы");
                ui.selectable_value(&mut self.wow.places, true, "Здания и подземелья");
            });
            let hint = if places { "слова пути: scarlet cathedral" } else { "слова пути: scarlet statue" };
            ui.add(egui::TextEdit::singleline(&mut self.wow.query).hint_text(hint).desired_width(f32::INFINITY));
            let n = found.len();
            let total = if places { self.wow.locations.len() } else { self.wow.models.len() };
            ui.label(if self.wow.query.trim().is_empty() { format!("В клиенте: {total}") } else { format!("Найдено: {n}") });
            ui.columns(2, |cols| {
                let row_h = cols[0].text_style_height(&egui::TextStyle::Body) + 4.0;
                let picked = if places { self.wow.picked_place.clone() } else { self.wow.picked.clone() };
                let mut clicked = None;
                egui::ScrollArea::vertical().id_salt("wow_list").max_height(360.0).show_rows(&mut cols[0], row_h, found.len(), |ui, range| {
                    for m in &found[range] {
                        let short = m.strip_prefix("World\\").or_else(|| m.strip_prefix("WORLD\\")).unwrap_or(m);
                        if ui.selectable_label(picked.as_ref() == Some(m), short).clicked() {
                            clicked = Some(m.clone());
                        }
                    }
                });
                if let Some(m) = clicked {
                    if places {
                        self.wow.picked_place = Some(m);
                        self.wow.confirm = false;
                    } else {
                        self.wow.picked = Some(m);
                    }
                }
                let ui = &mut cols[1];
                if places {
                    if let Some(m) = self.wow.picked_place.clone() {
                        ui.label(m.rsplit('\\').next().unwrap_or(&m));
                        ui.label("Пол — снимком сверху, как в игре; стены — по срезу на уровне груди; предметы — на своих местах. Текущая карта заменится.");
                        if self.wow.building.is_some() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("собираю…");
                            });
                        } else if self.wow.confirm {
                            ui.colored_label(egui::Color32::LIGHT_YELLOW, "Текущая карта не сохранена.");
                            if ui.button("Собрать всё равно").clicked() {
                                build = Some(m);
                            }
                        } else if ui.button("Собрать карту").clicked() {
                            if self.dirty {
                                self.wow.confirm = true;
                            } else {
                                build = Some(m);
                            }
                        }
                    }
                } else {
                    if let (Some(m), Some(client)) = (self.wow.picked.clone(), self.wow.client.clone()) {
                        if self.wow.preview.as_ref().is_none_or(|(p, _)| *p != m) {
                            let opt = wow::render::Options { ppc: 96.0, supersample: 2, max_px: 360, ..Default::default() };
                            let shot = wow::load(&client, &m).and_then(|(model, tex, _)| wow::render::render(&model, &tex, &opt)).map(|s| {
                                let img = egui::ColorImage::from_rgba_unmultiplied([s.img.width() as usize, s.img.height() as usize], s.img.as_raw());
                                (ui.ctx().load_texture("wow_preview", img, egui::TextureOptions::LINEAR), s.cells)
                            });
                            self.wow.preview = Some((m.clone(), shot));
                        }
                        match &self.wow.preview {
                            Some((_, Ok((tex, cells)))) => {
                                let size = tex.size_vec2();
                                let k = (300.0f32 / size.x.max(size.y)).min(2.0);
                                egui::Frame::new().fill(egui::Color32::from_rgb(70, 74, 60)).show(ui, |ui| {
                                    ui.image((tex.id(), size * k));
                                });
                                ui.label(format!("{:.2} × {:.2} клетки", cells[0], cells[1]));
                            }
                            Some((_, Err(e))) => {
                                ui.colored_label(egui::Color32::LIGHT_RED, e);
                            }
                            None => {}
                        }
                        if ui.button("На полку").clicked() {
                            put = vec![m];
                        }
                    }
                    if n > 1 && ui.add_enabled(n <= MAX_BATCH, egui::Button::new(format!("Все найденные на полку ({n})"))).on_disabled_hover_text(format!("Не больше {MAX_BATCH} за раз: уточните поиск")).clicked() {
                        put = found.clone();
                    }
                }
                for line in &self.wow.report {
                    ui.label(line);
                }
            });
        });
        if choose && let Some(dir) = crate::plat::FileDialog::new().pick_folder() {
            self.wow.client = None;
            self.wow.models.clear();
            self.wow.locations.clear();
            self.wow_start(dir);
        }
        if !put.is_empty()
            && let Some(client) = self.wow.client.clone()
        {
            let (done, errors) = self.wow_put_on_shelf(&client, &put);
            self.wow.report = vec![format!("На полке: {}", done.len())];
            self.wow.report.extend(errors.iter().take(5).cloned());
            if let Some(key) = done.first().and_then(|d| d["key"].as_str()) {
                self.use_key(Kind::Object, key.to_owned(), false);
            }
        }
        if let (Some(path), Some(client)) = (build, self.wow.client.clone()) {
            self.wow.confirm = false;
            self.wow.report.clear();
            let (tiles, items) = (location::tiles_dir(&self.data_dir), self.wow_dir());
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(location::build(&client, &path, &tiles, &items, &location::Options::default()));
            });
            self.wow.building = Some(rx);
        }
        if !open {
            self.wow.open = false;
        }
    }

    /// `wow_models`: найти модели (или здания, `kind: wmo`) по словам пути.
    pub(super) fn tool_wow_models(&mut self, a: &Value) -> Result<Value, String> {
        let client = self.wow_client_now(a["client"].as_str().map(PathBuf::from))?;
        let limit = a["limit"].as_u64().unwrap_or(100).clamp(1, 2000) as usize;
        let list = if a["kind"].as_str() == Some("wmo") { &self.wow.locations } else { &self.wow.models };
        let found = search(list, a["words"].as_str().unwrap_or_default());
        Ok(json!({"client": client.dir, "locale": client.locale, "count": found.len(), "models": found.into_iter().take(limit).collect::<Vec<_>>()}))
    }

    /// `wow_import`: модели (по путям или словам) — сверху на полку; ключи для `add_object`.
    pub(super) fn tool_wow_import(&mut self, a: &Value) -> Result<Value, String> {
        let client = self.wow_client_now(a["client"].as_str().map(PathBuf::from))?;
        let mut models: Vec<String> = a["models"].as_array().map(|v| v.iter().filter_map(|m| m.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        if let Some(words) = a["words"].as_str() {
            models.extend(search(&self.wow.models, words).into_iter().cloned());
        }
        if models.is_empty() {
            return Err("нужны models: [пути] или words: \"слова пути\"".into());
        }
        let max = a["max"].as_u64().unwrap_or(50).clamp(1, MAX_BATCH as u64) as usize;
        if models.len() > max {
            return Err(format!("подходит {} моделей, больше max = {max}: уточните words или поднимите max (до {MAX_BATCH})", models.len()));
        }
        let (done, errors) = self.wow_put_on_shelf(&client, &models);
        Ok(json!({"imported": done, "errors": errors}))
    }

    /// `wow_location`: здание или подземелье WMO — картой вместо текущей.
    pub(super) fn tool_wow_location(&mut self, a: &Value) -> Result<Value, String> {
        let client = self.wow_client_now(a["client"].as_str().map(PathBuf::from))?;
        let path = match (a["wmo"].as_str(), a["words"].as_str()) {
            (Some(p), _) => p.to_owned(),
            (None, Some(w)) => {
                let mut found = search(&self.wow.locations, w);
                // Точное имя файла выигрывает: `goldshireinn` — не `GoldshireInn_closed`.
                let stem = |p: &str| p.rsplit('\\').next().unwrap_or(p).trim_end_matches(".wmo").trim_end_matches(".WMO").to_lowercase();
                if found.len() > 1 && found.iter().filter(|p| stem(p) == w.trim().to_lowercase()).count() == 1 {
                    found.retain(|p| stem(p) == w.trim().to_lowercase());
                }
                match found.len() {
                    0 => return Err(format!("нет здания со словами «{w}»")),
                    1 => found[0].clone(),
                    k => return Err(format!("подходит {k} зданий, уточните words или дайте wmo: {:?}", found.iter().take(10).collect::<Vec<_>>())),
                }
            }
            _ => return Err("нужен wmo: путь корня .wmo (из wow_models с kind: wmo) или words".into()),
        };
        let built = self.wow_build_now(&client, &path, &location_options(a))?;
        let reply = json!({
            "wmo": path, "width": built.map.width, "height": built.map.height, "levels": built.levels,
            "floor_tiles": built.tiles, "walls": built.walls, "objects": built.objects, "px_per_cell": built.ppc, "notes": built.notes,
            "doodad_sets": built.sets, "sets_used": built.sets_used,
        });
        self.wow_install(built);
        Ok(reply)
    }
}

fn built_line(b: &location::Built) -> String {
    format!("Собрано: {}×{} клеток, этажей {}, стен {}, предметов {}", b.map.width, b.map.height, b.levels, b.walls, b.objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_needs_every_word() {
        let models = vec!["World\\Dungeon\\ScarletMonastery\\Statues\\StatueHMPaladin.m2".to_owned(), "World\\Generic\\Human\\Pews\\ChurchPew01.m2".to_owned()];
        assert_eq!(search(&models, "scarlet paladin").len(), 1);
        assert_eq!(search(&models, "world").len(), 2);
        assert_eq!(search(&models, "scarlet pew").len(), 0);
        assert!(search(&models, "  ").is_empty());
    }
}
