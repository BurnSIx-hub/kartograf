//! Окно «Предметы из WoW»: клиент игры, поиск моделей по словам пути, вид сверху,
//! перенос на полку (папка `wow` в данных программы, на полке — «WoW/…»).

use super::App;
use crate::library::Kind;
use crate::wow;
use eframe::egui;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

type Opened = Result<(Arc<wow::Client>, Vec<String>), String>;
/// Вид сверху выбранной модели: путь и картинка с размером в клетках (или почему нет).
type Preview = (String, Result<(egui::TextureHandle, [f32; 2]), String>);

#[derive(Default)]
pub struct WowWindow {
    pub open: bool,
    pub(crate) query: String,
    pub(crate) client: Option<Arc<wow::Client>>,
    /// Все модели клиента (`.m2`), как записаны в списках архивов.
    pub(crate) models: Vec<String>,
    loading: Option<mpsc::Receiver<Opened>>,
    error: Option<String>,
    pub(crate) picked: Option<String>,
    pub(crate) preview: Option<Preview>,
    report: Vec<String>,
}

/// Больше стольких моделей за раз на полку не кладём: тысячи картинок — не полка, а свалка.
const MAX_BATCH: usize = 200;

fn open_client(dir: PathBuf) -> Opened {
    let client = wow::Client::open(&dir)?;
    let models = client.list(".m2").into_iter().map(str::to_owned).collect();
    Ok((Arc::new(client), models))
}

/// Модели, в пути которых есть все слова (без регистра).
pub fn search<'a>(models: &'a [String], query: &str) -> Vec<&'a String> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    models.iter().filter(|m| {
        let l = m.to_lowercase();
        words.iter().all(|w| l.contains(w.as_str()))
    }).collect()
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

    fn wow_poll(&mut self) {
        let Some(rx) = &self.wow.loading else { return };
        match rx.try_recv() {
            Ok(Ok((client, models))) => {
                self.settings.wow_client = Some(client.dir.clone());
                self.save_settings();
                self.wow.client = Some(client);
                self.wow.models = models;
                self.wow.loading = None;
            }
            Ok(Err(e)) => {
                self.wow.error = Some(e);
                self.wow.loading = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.wow.loading = None,
        }
    }

    /// Клиент сразу (для команд ИИ): открыть, если не открыт или задана другая папка.
    pub(crate) fn wow_client_now(&mut self, dir: Option<PathBuf>) -> Result<Arc<wow::Client>, String> {
        let want = dir.or_else(|| self.settings.wow_client.clone()).ok_or("не задана папка клиента WoW (client)")?;
        if let Some(c) = &self.wow.client
            && (c.dir == want || c.archives.first().is_some_and(|a| a.path.starts_with(&want)))
        {
            return Ok(c.clone());
        }
        let (client, models) = open_client(want)?;
        self.settings.wow_client = Some(client.dir.clone());
        self.save_settings();
        self.wow.client = Some(client.clone());
        self.wow.models = models;
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
        if self.wow.loading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = true;
        let mut choose = false;
        let mut put: Vec<String> = Vec::new();
        let found: Vec<String> = search(&self.wow.models, &self.wow.query).into_iter().cloned().collect();
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
            ui.add(egui::TextEdit::singleline(&mut self.wow.query).hint_text("слова пути: scarlet statue").desired_width(f32::INFINITY));
            let n = found.len();
            ui.label(if self.wow.query.trim().is_empty() { format!("Моделей в клиенте: {}", self.wow.models.len()) } else { format!("Найдено: {n}") });
            ui.columns(2, |cols| {
                let row_h = cols[0].text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::ScrollArea::vertical().id_salt("wow_list").max_height(360.0).show_rows(&mut cols[0], row_h, found.len(), |ui, range| {
                    for m in &found[range] {
                        let short = m.strip_prefix("World\\").or_else(|| m.strip_prefix("WORLD\\")).unwrap_or(m);
                        if ui.selectable_label(self.wow.picked.as_ref() == Some(m), short).clicked() {
                            self.wow.picked = Some(m.clone());
                        }
                    }
                });
                let ui = &mut cols[1];
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
                for line in &self.wow.report {
                    ui.label(line);
                }
            });
        });
        if choose && let Some(dir) = crate::plat::FileDialog::new().pick_folder() {
            self.wow.client = None;
            self.wow.models.clear();
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
        if !open {
            self.wow.open = false;
        }
    }

    /// `wow_models`: найти модели по словам пути.
    pub(super) fn tool_wow_models(&mut self, a: &Value) -> Result<Value, String> {
        let client = self.wow_client_now(a["client"].as_str().map(PathBuf::from))?;
        let limit = a["limit"].as_u64().unwrap_or(100).clamp(1, 2000) as usize;
        let found = search(&self.wow.models, a["words"].as_str().unwrap_or_default());
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
