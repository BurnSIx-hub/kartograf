//! Картинки только для редактора: образец под обводку (поверх или под картой) и
//! эскиз в окошке сбоку, как референс в программах для рисования. В выгрузку,
//! на снимки и в картинку карты они не идут.

use super::*;

/// Картинка образца или эскиза; грузится один раз на файл, большая — в фоне.
pub(super) enum RefPic {
    Loading(std::sync::mpsc::Receiver<Result<egui::ColorImage, String>>),
    Ready(egui::TextureHandle, [usize; 2]),
    Failed(String),
}

/// Длинная сторона текстуры: больше видеокарта может не взять. Карта из книги
/// бывает 15 000 точек — для обводки хватает и уменьшенной.
const MAX_SIDE: u32 = 8192;

fn decode(path: &str) -> Result<egui::ColorImage, String> {
    let bytes = crate::plat::fs::read(Path::new(path)).map_err(|e| e.to_string())?;
    let mut img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = img.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}

/// Картинки, которые берутся образцом и эскизом.
const PICTURES: [&str; 5] = ["png", "jpg", "jpeg", "webp", "tif"];

impl App {
    /// Текстура картинки и её размер; пока грузится или не прочиталась — `None`.
    pub fn ref_pic(&mut self, ctx: &egui::Context, path: &str) -> Option<(egui::TextureId, [usize; 2])> {
        let slot = self.ref_pics.entry(path.to_owned()).or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let p = path.to_owned();
            #[cfg(not(target_arch = "wasm32"))]
            {
                let c = ctx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(decode(&p));
                    c.request_repaint();
                });
            }
            #[cfg(target_arch = "wasm32")]
            let _ = tx.send(decode(&p));
            RefPic::Loading(rx)
        });
        if let RefPic::Loading(rx) = slot {
            match rx.try_recv() {
                Ok(Ok(ci)) => {
                    let size = ci.size;
                    *slot = RefPic::Ready(ctx.load_texture(format!("ref-{path}"), ci, egui::TextureOptions::LINEAR), size);
                }
                Ok(Err(e)) => *slot = RefPic::Failed(e),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => *slot = RefPic::Failed("картинка не прочиталась".into()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        match slot {
            RefPic::Ready(t, s) => Some((t.id(), *s)),
            _ => None,
        }
    }

    fn ref_pic_state(&self, path: &str) -> Option<String> {
        match self.ref_pics.get(path) {
            Some(RefPic::Loading(_)) => Some("читается…".into()),
            Some(RefPic::Failed(e)) => Some(e.clone()),
            _ => None,
        }
    }

    /// Образец на холсте: поверх нарисованного (`top`) или под ним.
    pub(super) fn draw_trace(&mut self, painter: &egui::Painter, canvas: Rect, top: bool) {
        // Только у текущего этажа в окне: ни в выгрузку, ни на снимок, ни под верхним этажом.
        if self.underlay != 0 || self.export.is_some() {
            return;
        }
        let Some(t) = self.map.trace.clone().filter(|t| t.shown && t.on_top == top) else { return };
        let Some((tex, size)) = self.ref_pic(painter.ctx(), &t.path) else { return };
        let h = t.width * size[1] as f32 / size[0].max(1) as f32;
        let r = Rect::from_min_max(self.cam.to_screen(canvas, t.pos), self.cam.to_screen(canvas, [t.pos[0] + t.width, t.pos[1] + h]));
        let tint = Color32::from_white_alpha((t.opacity.clamp(0.0, 1.0) * 255.0).round() as u8);
        painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }

    /// Меню «Образец»: картинка под обводку и эскиз в окошке.
    pub(super) fn reference_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Образец — открыть картинку…").on_hover_text("Картинка поверх карты: обводите её кистями, она сама не меняется и в выгрузку не идёт").clicked() {
            if let Some(file) = crate::plat::FileDialog::new().add_filter("Картинка", &PICTURES).pick_file() {
                self.checkpoint();
                self.map.trace = Some(map::Trace::new(file.to_string_lossy().into_owned(), self.map.width));
            }
            ui.close();
        }
        if let Some(mut t) = self.map.trace.clone() {
            ui.label(library::label(&t.path));
            if let Some(state) = self.ref_pic_state(&t.path) {
                ui.weak(state);
            }
            ui.checkbox(&mut t.shown, "Показывать");
            ui.checkbox(&mut t.on_top, "Поверх карты").on_hover_text("Без галочки образец под картой: виден только там, где ещё ничего не нарисовано");
            ui.add(egui::Slider::new(&mut t.opacity, 0.05..=1.0).text("непрозрачность"));
            ui.add(egui::Slider::new(&mut t.width, 1.0..=(self.map.width as f32 * 4.0).max(10.0)).logarithmic(true).text("ширина, кл."));
            ui.horizontal(|ui| {
                ui.label("сдвиг, кл.:");
                ui.add(egui::DragValue::new(&mut t.pos[0]).speed(0.05).prefix("x "));
                ui.add(egui::DragValue::new(&mut t.pos[1]).speed(0.05).prefix("y "));
            });
            if ui.button("По ширине карты").clicked() {
                t.pos = [0.0, 0.0];
                t.width = self.map.width as f32;
            }
            let drop = ui.button("Убрать образец").clicked();
            if drop {
                self.checkpoint();
                self.map.trace = None;
            } else if Some(&t) != self.map.trace.as_ref() {
                self.map.trace = Some(t);
                self.touch();
            }
        }
        ui.separator();
        if ui.button("Эскиз — открыть картинку…").on_hover_text("Картинка в окошке сбоку, для сверки: колесо — ближе, протяжка — сдвиг").clicked() {
            if let Some(file) = crate::plat::FileDialog::new().add_filter("Картинка", &PICTURES).pick_file() {
                self.checkpoint();
                self.map.sketch = file.to_string_lossy().into_owned();
                self.sketch_open = true;
                self.sketch_view = (1.0, vec2(0.0, 0.0));
            }
            ui.close();
        }
        if !self.map.sketch.is_empty() {
            ui.checkbox(&mut self.sketch_open, "Окошко эскиза");
            if ui.button("Убрать эскиз").clicked() {
                self.checkpoint();
                self.map.sketch.clear();
                self.sketch_open = false;
            }
        }
    }

    /// Окошко эскиза: картинка целиком по размеру окна; колесо — ближе к курсору,
    /// протяжка — сдвиг, двойной щелчок — снова целиком.
    pub(super) fn sketch_window(&mut self, ctx: &egui::Context) {
        if self.map.sketch.is_empty() || !self.sketch_open {
            return;
        }
        let path = self.map.sketch.clone();
        let mut open = true;
        // Сначала — у правого края карты, слева от библиотеки: панель инструментов не закрывает.
        let at = ctx.content_rect().right_top() + vec2(-700.0, 70.0);
        egui::Window::new("Эскиз").id(egui::Id::new("sketch")).open(&mut open).default_pos(at).default_size([320.0, 260.0]).resizable(true).show(ctx, |ui| {
            let Some((tex, size)) = self.ref_pic(ctx, &path) else {
                ui.weak(self.ref_pic_state(&path).unwrap_or_default());
                return;
            };
            let (rect, resp) = ui.allocate_exact_size(ui.available_size().max(vec2(80.0, 60.0)), Sense::click_and_drag());
            let (mut zoom, mut pan) = self.sketch_view;
            let fit = (rect.width() / size[0] as f32).min(rect.height() / size[1] as f32);
            if resp.double_clicked() {
                (zoom, pan) = (1.0, vec2(0.0, 0.0));
            }
            if resp.dragged() {
                pan += resp.drag_delta();
            }
            if resp.hovered() {
                let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                if wheel != 0.0 {
                    let k = (wheel * 0.004).exp();
                    let new = (zoom * k).clamp(0.2, 40.0);
                    // Точка под курсором остаётся на месте.
                    if let Some(p) = resp.hover_pos() {
                        let c = rect.center() + pan;
                        pan += (p - c) * (1.0 - new / zoom);
                    }
                    zoom = new;
                }
            }
            self.sketch_view = (zoom, pan);
            let s = fit * zoom;
            let img = Rect::from_center_size(rect.center() + pan, vec2(size[0] as f32 * s, size[1] as f32 * s));
            ui.painter().with_clip_rect(rect).image(tex, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        });
        if !open {
            self.sketch_open = false;
        }
    }
}
