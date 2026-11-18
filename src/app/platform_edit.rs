//! Помосты в окне: выбор щелчком, перенос протяжкой, лестница и решётка щелчком у края,
//! панель выбранного помоста и его метки на карте.
//!
//! Всё — в инструменте «Крыша» с галочкой «Помост вместо крыши»: протяжка по пустому
//! месту ставит новый помост (прямоугольник или овал в рамке), щелчок по помосту выбирает
//! его, протяжка за выбранный — двигает целыми клетками, за его лестницу или решётку —
//! ведёт их вдоль края.

use super::*;
use crate::platform::{self, Grate, Stair};

/// Что поставит следующий щелчок у края выбранного помоста.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlatformPut {
    Stair,
    Grate,
}

/// Что тянут мышью.
#[derive(Clone, Debug)]
pub enum PlatformGrab {
    /// Весь помост: откуда начали и каким он был.
    Whole(usize, [f32; 2], crate::platform::Platform),
    Stair(usize, usize),
    Grate(usize, usize),
}

impl App {
    /// Помост под точкой — верхний (последний), с его лестницами.
    pub fn platform_at(&self, w: [f32; 2]) -> Option<usize> {
        self.map.platforms.iter().rposition(|pl| {
            let p = platform::tidy(&pl.points);
            p.len() >= 3 && (platform::contains(&p, w) || pl.stairs.iter().filter_map(|s| platform::stair_geo(&p, s)).any(|g| platform::contains(&g.outline(), w)))
        })
    }

    /// Лестница или решётка выбранного помоста под точкой.
    pub(super) fn platform_part_at(&self, i: usize, w: [f32; 2]) -> Option<PlatformGrab> {
        let pl = self.map.platforms.get(i)?;
        let p = platform::tidy(&pl.points);
        if let Some(k) = pl.stairs.iter().position(|s| platform::stair_geo(&p, s).is_some_and(|g| platform::contains(&g.outline(), w))) {
            return Some(PlatformGrab::Stair(i, k));
        }
        pl.grates
            .iter()
            .position(|g| {
                let strip = platform::grate_strip(&p, g, 0.25);
                strip.windows(2).any(|s| platform::contains(&[s[0].0, s[1].0, s[1].1, s[0].1], w))
            })
            .map(|k| PlatformGrab::Grate(i, k))
    }

    pub(super) fn selected_platform(&self) -> Option<usize> {
        match self.sel {
            Some(Sel::Platform(i) | Sel::PlatformEdge(i, _)) if i < self.map.platforms.len() => Some(i),
            _ => None,
        }
    }

    /// Край помоста под точкой: щелчок по боковой стенке (от кромки наружу на её высоту
    /// и чуть внутрь). Открытый край без стенки берётся только у выбранного помоста —
    /// по линии контура. Верхний помост — первым.
    pub fn platform_edge_at(&self, w: [f32; 2]) -> Option<(usize, usize)> {
        let chosen = self.selected_platform();
        for i in (0..self.map.platforms.len()).rev() {
            let pl = &self.map.platforms[i];
            let p = platform::tidy(&pl.points);
            if p.len() < 3 {
                continue;
            }
            let (nrm, free, h, n) = (platform::normals(&p), self.platform_free(i), pl.height.clamp(0.02, 1.5), p.len());
            let mut best: Option<(usize, f32)> = None;
            for k in 0..n {
                let (a, b) = (p[k], p[(k + 1) % n]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-8 {
                    continue;
                }
                let t = ((w[0] - a[0]) * dx + (w[1] - a[1]) * dy) / len2;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                // Насколько точка снаружи ребра: плюс — по наружной нормали.
                let d = (w[0] - a[0]) * nrm[k][0] + (w[1] - a[1]) * nrm[k][1];
                let score = if free[k] && (-0.15..=h + 0.1).contains(&d) {
                    (d - h / 2.0).abs()
                } else if pl.open.contains(&k) && chosen == Some(i) && d.abs() <= 0.2 {
                    d.abs()
                } else {
                    continue;
                };
                if best.is_none_or(|(_, s)| score < s) {
                    best = Some((k, score));
                }
            }
            if let Some((k, _)) = best {
                return Some((i, k));
            }
        }
        None
    }

    /// Помост или его край под точкой: стенка — край, площадка — весь помост.
    pub fn platform_hit(&self, w: [f32; 2]) -> Option<Sel> {
        self.platform_edge_at(w).map(|(i, k)| Sel::PlatformEdge(i, k)).or_else(|| self.platform_at(w).map(Sel::Platform))
    }

    /// Мышь в режиме помоста.
    pub(super) fn platform_input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, pointer: Option<Pos2>) {
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        {
            let w = self.cam.to_world(rect, o);
            let chosen = self.selected_platform();
            let part = chosen.and_then(|i| self.platform_part_at(i, w));
            self.platform_grab = match (part, chosen) {
                (Some(g), _) => Some(g),
                (None, Some(i)) if self.platform_at(w) == Some(i) => Some(PlatformGrab::Whole(i, w, self.map.platforms[i].clone())),
                _ => None,
            };
            if self.platform_grab.is_some() {
                self.checkpoint();
            } else {
                self.roof_from = Some(self.cam.cell(rect, o));
                self.roof_from_w = (!self.settings.snap).then_some(w);
            }
        }
        if let Some(p) = pointer
            && resp.dragged_by(PointerButton::Primary)
        {
            self.platform_drag(self.cam.to_world(rect, p));
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.platform_grab = None;
            let free = self.roof_from_w.take();
            if let (Some(a), Some(p)) = (self.roof_from.take(), pointer) {
                // Без привязки — рамка где протянули, не по клеткам.
                let i = match free {
                    Some(w0) => self.place_platform_rect(w0, self.cam.to_world(rect, p)),
                    None => self.place_platform(a, self.cam.cell(rect, p)),
                };
                self.sel = Some(Sel::Platform(i));
            }
        } else if resp.clicked()
            && let Some(p) = pointer
        {
            let w = self.cam.to_world(rect, p);
            if !self.platform_put_click(w) {
                self.sel = self.platform_hit(w);
            }
        }
    }

    /// Протяжка того, что взято у помоста: весь он, лестница или решётка.
    pub(super) fn platform_drag(&mut self, w: [f32; 2]) {
        let Some(grab) = self.platform_grab.clone() else { return };
        match grab {
            PlatformGrab::Whole(i, w0, before) => {
                // С привязкой — целыми клетками: прямоугольный помост остаётся по сетке.
                let d = [w[0] - w0[0], w[1] - w0[1]];
                let d = if self.settings.snap { [d[0].round(), d[1].round()] } else { d };
                let mut pl = before;
                pl.shift(d);
                self.map.platforms[i] = pl;
            }
            PlatformGrab::Stair(i, k) => self.map.platforms[i].stairs[k].at = w,
            PlatformGrab::Grate(i, k) => self.map.platforms[i].grates[k].at = w,
        }
    }

    /// Щелчок с «+ лестница» или «+ решётка»: ставит у края выбранного помоста — не
    /// дальше полутора клеток от контура. `true` — поставил; дальше — просто выбор.
    pub(super) fn platform_put_click(&mut self, w: [f32; 2]) -> bool {
        let near = |i: usize| {
            let p = platform::tidy(&self.map.platforms[i].points);
            platform::stair_frame(&p, w).is_some_and(|(q, _, _)| (q[0] - w[0]).hypot(q[1] - w[1]) <= 1.5)
        };
        match (self.platform_put.take(), self.selected_platform()) {
            (Some(put), Some(i)) if near(i) => {
                self.platform_add(i, put, w);
                true
            }
            _ => false,
        }
    }

    /// Лестница или решётка у края помоста `i` — у ближайшей к `w` точки контура.
    pub fn platform_add(&mut self, i: usize, put: PlatformPut, w: [f32; 2]) {
        self.checkpoint();
        let pl = &mut self.map.platforms[i];
        match put {
            PlatformPut::Stair => pl.stairs.push(Stair { at: w, width: 2.0, steps: 3 }),
            PlatformPut::Grate => pl.grates.push(Grate { at: w, length: 3.0 }),
        }
    }

    /// Метки поверх карты: контур выбранного помоста, его лестницы и решётки; призрак
    /// того, что поставит щелчок.
    pub(super) fn draw_platform_marks(&self, painter: &egui::Painter, canvas: Rect) {
        let cam = self.cam;
        let line = |pts: &[[f32; 2]], color: Color32, w: f32| {
            let q: Vec<Pos2> = pts.iter().map(|&p| cam.to_screen(canvas, p)).collect();
            painter.add(Shape::closed_line(q, Stroke::new(w, color)));
        };
        let blue = Color32::from_rgb(120, 180, 255);
        let Some(i) = self.selected_platform() else { return };
        let pl = &self.map.platforms[i];
        let p = platform::tidy(&pl.points);
        if p.len() < 3 {
            return;
        }
        line(&p, blue, 2.0);
        // Края без стенки — пунктиром, выбранный край — толсто.
        let n = p.len();
        let orange = Color32::from_rgb(255, 170, 60);
        for &k in pl.open.iter().filter(|&&k| k < n) {
            let ends = [cam.to_screen(canvas, p[k]), cam.to_screen(canvas, p[(k + 1) % n])];
            painter.extend(Shape::dashed_line(&ends, Stroke::new(2.0, orange), 6.0, 4.0));
        }
        if let Some(Sel::PlatformEdge(_, k)) = self.sel
            && k < n
        {
            painter.line_segment([cam.to_screen(canvas, p[k]), cam.to_screen(canvas, p[(k + 1) % n])], Stroke::new(4.0, orange));
        }
        for s in &pl.stairs {
            if let Some(g) = platform::stair_geo(&p, s) {
                line(&g.outline(), blue, 1.5);
            }
        }
        for g in &pl.grates {
            let strip = platform::grate_strip(&p, g, 0.25);
            let ring: Vec<[f32; 2]> = strip.iter().map(|s| s.0).chain(strip.iter().rev().map(|s| s.1)).collect();
            if ring.len() >= 3 {
                line(&ring, blue, 1.5);
            }
        }
        // Призрак: лестница или решётка у края под мышью.
        if let (Some(put), Some(h)) = (self.platform_put, self.hover) {
            let w = cam.to_world(canvas, h);
            let orange = Color32::from_rgb(255, 200, 90);
            match put {
                PlatformPut::Stair => {
                    if let Some(g) = platform::stair_geo(&p, &Stair { at: w, width: 2.0, steps: 3 }) {
                        line(&g.outline(), orange, 2.0);
                    }
                }
                PlatformPut::Grate => {
                    let strip = platform::grate_strip(&p, &Grate { at: w, length: 3.0 }, 0.25);
                    let ring: Vec<[f32; 2]> = strip.iter().map(|s| s.0).chain(strip.iter().rev().map(|s| s.1)).collect();
                    if ring.len() >= 3 {
                        line(&ring, orange, 2.0);
                    }
                }
            }
        }
    }

    /// Панель режима помоста: форма новой рамки; у выбранного — высота, цвета, перила,
    /// огоньки, лестницы и решётки; без выбора — список, чтобы выбрать.
    pub(super) fn platform_panel(&mut self, ui: &mut egui::Ui) {
        if self.tool == Tool::Roof {
            ui.horizontal(|ui| {
                ui.label("Рамка ставит:");
                ui.selectable_value(&mut self.platform_oval, false, "прямоугольник");
                ui.selectable_value(&mut self.platform_oval, true, "овал");
            });
        }
        let Some(i) = self.selected_platform() else {
            if self.map.platforms.is_empty() {
                ui.label("Помостов нет: протяните рамку по полу.");
            } else {
                ui.label("Щёлкните помост, чтобы выбрать его.");
                ui.horizontal_wrapped(|ui| {
                    for k in 0..self.map.platforms.len() {
                        if ui.selectable_label(false, format!("Помост {}", k + 1)).clicked() {
                            self.sel = Some(Sel::Platform(k));
                        }
                    }
                });
            }
            return;
        };
        let held = ui.input(|inp| inp.pointer.any_down());
        let before = self.map.platforms[i].clone();
        let mut pl = before.clone();
        let mut drop = false;
        ui.horizontal(|ui| {
            ui.strong(format!("Помост {}", i + 1));
            if ui.small_button("удалить").clicked() {
                drop = true;
            }
        });
        // Край помоста: стенку можно убрать у одного края, остальные остаются.
        if let Some(Sel::PlatformEdge(_, k)) = self.sel {
            let mut walled = !pl.open.contains(&k);
            ui.horizontal(|ui| {
                ui.label(format!("Край {}:", k + 1));
                ui.checkbox(&mut walled, "стенка")
                    .on_hover_text("Без стенки помост у этого края сходит в пол той же высоты: нет ни стенки, ни кромки, ни тени, ни перил. Delete — убрать стенку");
            });
            pl.set_open(k, !walled);
        } else {
            ui.weak("Щелчок по стенке помоста выбирает один край.");
        }
        if !pl.open.is_empty() {
            ui.horizontal(|ui| {
                ui.label(format!("Краёв без стенки: {}", pl.open.len()));
                if ui.small_button("вернуть все").clicked() {
                    pl.open.clear();
                }
            });
        }
        ui.add(egui::Slider::new(&mut pl.height, 0.05..=1.0).text("высота стенки, кл."));
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut pl.color);
            ui.add(egui::Slider::new(&mut pl.strength, 0.0..=1.0).text("сила цвета"));
        });
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut pl.side);
            ui.label("стенка и ступени");
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut pl.rail, "перила");
            let mut lit = pl.lights.is_some();
            if ui.checkbox(&mut lit, "огоньки").changed() {
                pl.lights = lit.then(platform::cyan);
            }
            if let Some(c) = pl.lights.as_mut() {
                ui.color_edit_button_srgb(c);
            }
        });
        ui.separator();
        let put = self.platform_put;
        ui.horizontal(|ui| {
            ui.strong(format!("Лестницы: {}", pl.stairs.len()));
            if ui.selectable_label(put == Some(PlatformPut::Stair), "+ лестница").clicked() {
                self.platform_put = if put == Some(PlatformPut::Stair) { None } else { Some(PlatformPut::Stair) };
            }
        });
        let mut gone = None;
        for (k, s) in pl.stairs.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = 64.0;
                ui.label(format!("{}", k + 1));
                ui.add(egui::Slider::new(&mut s.width, 0.6..=8.0).step_by(0.1).text("шир."));
                ui.add(egui::Slider::new(&mut s.steps, 1..=10).text("ступ."));
                if ui.small_button("×").on_hover_text("Убрать лестницу").clicked() {
                    gone = Some(k);
                }
            });
        }
        if let Some(k) = gone {
            pl.stairs.remove(k);
        }
        ui.horizontal(|ui| {
            ui.strong(format!("Решётки: {}", pl.grates.len()));
            if ui.selectable_label(put == Some(PlatformPut::Grate), "+ решётка").clicked() {
                self.platform_put = if put == Some(PlatformPut::Grate) { None } else { Some(PlatformPut::Grate) };
            }
        });
        let mut gone = None;
        for (k, g) in pl.grates.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{}", k + 1));
                ui.add(egui::Slider::new(&mut g.length, 0.5..=10.0).step_by(0.1).text("длина"));
                if ui.small_button("×").on_hover_text("Убрать решётку").clicked() {
                    gone = Some(k);
                }
            });
        }
        if let Some(k) = gone {
            pl.grates.remove(k);
        }
        if let Some(put) = self.platform_put {
            ui.label(match put {
                PlatformPut::Stair => "Щёлкните у края помоста — там встанет лестница.",
                PlatformPut::Grate => "Щёлкните у края помоста — там ляжет решётка.",
            });
        } else if !pl.stairs.is_empty() || !pl.grates.is_empty() {
            ui.weak("Лестницу и решётку можно вести вдоль края протяжкой.");
        }
        if pl != before {
            // Один шаг отмены на одно движение ползунка.
            if !self.glow_hold {
                self.checkpoint();
            }
            self.glow_hold = held;
            self.map.platforms[i] = pl;
        } else if !held {
            self.glow_hold = false;
        }
        if drop {
            self.checkpoint();
            self.map.platforms.remove(i);
            self.sel = None;
            self.platform_put = None;
        }
    }
}
