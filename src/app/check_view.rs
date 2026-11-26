//! Окно «Проверка»: что в Foundry выйдет не так (`crate::check`) — список по разделам, метки на
//! карте, щелчок по строке ведёт к месту и выбирает предмет. Считается в фоне после каждой правки,
//! пока окно открыто. Тот же список отдаёт MCP `check_map`.

use super::*;
use crate::check;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Видимая часть картинок по ключу ассета: каждую картинку раскрывать один раз.
type Shapes = Arc<Mutex<HashMap<String, Option<([f32; 4], Vec<u16>)>>>>;

#[derive(Default)]
pub struct CheckState {
    pub open: bool,
    /// Правка карты и этаж, для которых посчитан список.
    done: Option<(u64, usize)>,
    pub issues: Vec<check::Issue>,
    hover: Option<usize>,
    picked: Option<usize>,
    #[cfg(not(target_arch = "wasm32"))]
    job: Option<(u64, usize, std::sync::mpsc::Receiver<Vec<check::Issue>>)>,
    shapes: Shapes,
}

/// Число в том виде, в каком его видел check.py: `json!` переводит f32 в f64 как есть
/// (16,55 → 16,549999237…), и пороги проверки стоят на этих значениях.
fn f(v: f32) -> f64 {
    v as f64
}

fn r2(v: f32) -> f64 {
    f((v * 100.0).round() / 100.0)
}

/// Видимая часть и маска картинок (`library::visible_shape`) — в предметы проверки.
fn fill_shapes(inp: &mut check::Input, keys: &[(String, [f32; 2])], sources: &library::Sources, shapes: &Shapes) {
    for (o, (key, size)) in inp.objs.iter_mut().zip(keys) {
        let known = shapes.lock().ok().and_then(|m| m.get(key).cloned());
        let shape = known.unwrap_or_else(|| {
            let s = library::visible_shape(sources, key);
            if let Ok(mut m) = shapes.lock() {
                m.insert(key.clone(), s.clone());
            }
            s
        });
        if let Some((v, mask)) = shape {
            o.vis = Some([r2(v[0] * size[0]), r2(v[1] * size[1]), r2(v[2] * size[0]), r2(v[3] * size[1])]);
            o.mask = Some(mask);
        }
    }
}

impl App {
    /// Что видит проверка на этом этаже — то же, что check.py брал из `describe`; рядом ключ
    /// картинки и размер каждого предмета (для видимой части).
    fn check_input(&mut self) -> (check::Input, Vec<(String, [f32; 2])>) {
        let pt = |q: [f32; 2]| [r2(q[0]), r2(q[1])];
        let room_walls = self.room_edges().iter().map(|&(p, q)| (pt(p), pt(q))).collect();
        let mut rails = Vec::new();
        for i in 0..self.map.platforms.len() {
            let pl = &self.map.platforms[i];
            if !pl.rail {
                continue;
            }
            let tidy = crate::platform::tidy(&pl.points);
            let free = self.platform_free(i);
            rails.extend(crate::platform::rails(&tidy, &free, &self.map.platforms[i].stairs).into_iter().map(|(l, _)| l.into_iter().map(pt).collect::<Vec<_>>()));
        }
        let m = &self.map;
        let raw = |q: [f32; 2]| [f(q[0]), f(q[1])];
        let mut houses: Vec<check::House> = m
            .roofs
            .iter()
            .enumerate()
            .map(|(i, r)| check::House { index: i, area: Some([f(r.a[0]), f(r.a[1]), f(r.b[0]), f(r.b[1])]), points: Vec::new(), flat: r.shape == map::RoofShape::Flat })
            .collect();
        let n = m.roofs.len();
        houses.extend(m.outline_roofs.iter().enumerate().filter_map(|(i, r)| {
            let p = r.pieces.first()?;
            Some(check::House { index: n + i, area: None, points: p.rings.first()?.iter().map(|&q| pt(q)).collect(), flat: p.form == map::RoofForm::Flat })
        }));
        let norm = |s: &str| s.replace('\\', "/").to_lowercase();
        // Полка предметов WoW и папка пола зданий WoW (там же переходы между этажами).
        let shelf = norm(&self.wow_dir().to_string_lossy()) + "/";
        let floors = norm(&crate::wow::location::tiles_dir(&self.data_dir).to_string_lossy()) + "/";
        let objs = m
            .props
            .iter()
            .enumerate()
            .map(|(i, p)| check::Obj {
                index: i,
                name: library::label(&p.asset),
                x: r2(p.pos[0]),
                y: r2(p.pos[1]),
                w: r2(p.size[0] * p.scale),
                h: r2(p.size[1] * p.scale),
                rot: f(p.rot),
                layer: p.layer() as i32,
                canopy: p.is_canopy(),
                vis: None,
                mask: None,
                game: norm(&p.asset).starts_with(&shelf) || norm(&p.asset).starts_with(&floors),
            })
            .collect();
        let keys = m.props.iter().map(|p| (p.asset.clone(), [p.size[0] * p.scale, p.size[1] * p.scale])).collect();
        let inp = check::Input {
            width: m.width as f64,
            height: m.height as f64,
            objs,
            houses,
            doors: m.portals.iter().enumerate().map(|(i, d)| check::Door { index: i, a: raw(d.a), b: raw(d.b), window: d.kind == PortalKind::Window, opening: d.kind == PortalKind::Opening }).collect(),
            labels: m.labels.iter().map(|l| check::Text { text: l.text.clone(), size: f(l.size), x: f(l.pos[0]), y: f(l.pos[1]) }).collect(),
            walls: m.walls.iter().map(|w| check::Line { points: w.points.iter().map(|&q| raw(q)).collect(), closed: w.closed }).collect(),
            room_walls,
            rails,
        };
        (inp, keys)
    }

    /// Проверка сразу, в этом кадре (MCP `check_map`).
    pub(super) fn check_now(&mut self) -> (check::Input, Vec<check::Issue>) {
        let (mut inp, keys) = self.check_input();
        fill_shapes(&mut inp, &keys, &self.textures.sources, &self.check.shapes);
        let out = check::run(&inp);
        (inp, out)
    }

    pub(crate) fn check_busy(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.check.job.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    fn check_take(&mut self, at: (u64, usize), issues: Vec<check::Issue>) {
        // Выбранная строка остаётся выбранной, если нарушение никуда не делось.
        let keep = |k: Option<usize>, old: &[check::Issue]| k.and_then(|k| old.get(k)).and_then(|o| issues.iter().position(|i| i.text == o.text));
        self.check.picked = keep(self.check.picked, &self.check.issues);
        self.check.hover = None;
        self.check.issues = issues;
        self.check.done = Some(at);
    }

    /// Пересчитать, если карта изменилась: в фоне (в браузере — сразу). Не посреди протяжки.
    fn check_refresh(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((rev, level, rx)) = &self.check.job {
            match rx.try_recv() {
                Ok(out) => {
                    let at = (*rev, *level);
                    self.check.job = None;
                    self.check_take(at, out);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.check.job = None;
                    self.check.done = Some((self.rev, self.level));
                    self.error = Some("Проверка карты упала — список не обновлён".into());
                    return;
                }
            }
        }
        let want = (self.rev, self.level);
        if self.check.done == Some(want) || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        let (mut inp, keys) = self.check_input();
        let (sources, shapes) = (self.textures.sources.clone(), self.check.shapes.clone());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                fill_shapes(&mut inp, &keys, &sources, &shapes);
                let _ = tx.send(check::run(&inp));
                ctx.request_repaint();
            });
            self.check.job = Some((want.0, want.1, rx));
        }
        #[cfg(target_arch = "wasm32")]
        {
            fill_shapes(&mut inp, &keys, &sources, &shapes);
            let out = check::run(&inp);
            self.check_take(want, out);
        }
    }

    /// К месту нарушения: оно посередине холста, предмет выбран.
    pub(crate) fn goto_issue(&mut self, i: usize) {
        let Some(issue) = self.check.issues.get(i) else { return };
        let (at, first) = ([issue.at[0] as f32, issue.at[1] as f32], issue.objects.first().copied());
        self.check.picked = Some(i);
        self.cam.scale = self.cam.scale.max(40.0);
        let c = self.canvas;
        if c.is_positive() {
            self.cam.pan = c.center() - c.min - vec2(at[0], at[1]) * self.cam.scale;
        }
        if let Some(k) = first.filter(|&k| k < self.map.props.len()) {
            self.set_tool(Tool::Select);
            self.sel = Some(Sel::Prop(k));
        }
    }

    pub(super) fn check_window(&mut self, ctx: &egui::Context) {
        if !self.check.open {
            return;
        }
        self.check_refresh(ctx);
        let mut open = true;
        let mut goto = None;
        let mut hover = None;
        let busy = self.check_busy();
        egui::Window::new("Проверка для Foundry").open(&mut open).default_width(420.0).default_pos(self.canvas.right_top() + vec2(-440.0, 12.0)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                }
                let n = self.check.issues.len();
                ui.strong(if self.check.done.is_none() { "Проверяю…".to_owned() } else if n == 0 { "Нарушений нет".to_owned() } else { format!("Нарушений: {n}") });
            });
            ui.label(egui::RichText::new("Щелчок по строке — к месту на карте. Список обновляется после каждой правки, пока окно открыто.").small().weak());
            ui.separator();
            egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                for kind in check::Kind::ALL {
                    let rows: Vec<usize> = (0..self.check.issues.len()).filter(|&i| self.check.issues[i].kind == kind).collect();
                    if rows.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(format!("{} — {}", kind.title(), rows.len())).id_salt(kind.tag()).default_open(true).show(ui, |ui| {
                        for i in rows {
                            let r = ui.selectable_label(self.check.picked == Some(i), &self.check.issues[i].text);
                            if r.hovered() {
                                hover = Some(i);
                            }
                            if r.clicked() {
                                goto = Some(i);
                            }
                        }
                    });
                }
            });
        });
        self.check.hover = hover;
        if let Some(i) = goto {
            self.goto_issue(i);
        }
        if !open {
            self.check.open = false;
            self.check.hover = None;
        }
    }

    /// Метки нарушений на карте; у наведённого и выбранного — обвод предметов.
    pub(super) fn draw_check_marks(&self, painter: &egui::Painter, canvas: Rect) {
        if !self.check.open {
            return;
        }
        let red = Color32::from_rgb(235, 80, 60);
        let gold = Color32::from_rgb(255, 210, 90);
        for (i, issue) in self.check.issues.iter().enumerate() {
            let hot = self.check.hover == Some(i) || self.check.picked == Some(i);
            let color = if hot { gold } else { red };
            if hot {
                for &k in &issue.objects {
                    if let Some(p) = self.map.props.get(k) {
                        let pts: Vec<Pos2> = p.corners().iter().map(|&q| self.cam.to_screen(canvas, q)).collect();
                        painter.add(Shape::closed_line(pts, Stroke::new(2.5, gold)));
                    }
                }
            }
            let p = self.cam.to_screen(canvas, [issue.at[0] as f32, issue.at[1] as f32]);
            let r = if hot { 11.0 } else { 7.5 };
            painter.circle(p, r, Color32::from_black_alpha(170), Stroke::new(2.0, color));
            painter.text(p, egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(r * 1.45), color);
        }
    }
}
