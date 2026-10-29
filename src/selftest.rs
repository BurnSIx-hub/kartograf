//! Самопроверка (`--selftest <папка>`): программа сама тянет мышью, щёлкает и жмёт клавиши
//! через настоящий путь ввода egui, сверяет карту и кладёт снимки в папку.
//! Итог — `report.txt` и код выхода: 0, если всё сошлось.

use crate::app::{App, Sel, TURN_LIFT_PX, Tool, grips, save_png};
use crate::generate::Options;
use crate::library::Kind;
use crate::map::{self, Layer, Map, Paint, PortalKind};
use crate::mask::{self, Brush, Shape as Tip};
use crate::recovery::Recovery;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, vec2};
use std::collections::VecDeque;
use std::path::PathBuf;

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
/// Пауза после одиночного щелчка: два щелчка подряд egui иначе сочтёт двойным.
const CLICK_GAP: f64 = 0.45;

pub struct SelfTest {
    dir: PathBuf,
    /// Площадь комнат до изгиба.
    area: f32,
    step: usize,
    /// Пачки событий, по одной на кадр.
    frames: VecDeque<(Modifiers, Vec<Event>)>,
    mods: Modifiers,
    log: Vec<String>,
    passed: u32,
    failed: u32,
    shot: Option<String>,
    resume_at: f64,
    /// Что запомнить до шага, чтобы сверить после.
    scale: f32,
    world: [f32; 2],
    pan: egui::Vec2,
    /// Куда поставить курсор перед снимком.
    hover_at: Option<Pos2>,
}

impl SelfTest {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            area: 0.0,
            step: 0,
            frames: VecDeque::new(),
            mods: Modifiers::NONE,
            log: Vec::new(),
            passed: 0,
            failed: 0,
            shot: None,
            resume_at: 0.5,
            scale: 0.0,
            world: [0.0, 0.0],
            pan: egui::Vec2::ZERO,
            hover_at: None,
        }
    }

    /// Подменяет ввод: живые мышь и клавиатура не должны мешать сценарию.
    pub fn feed(&mut self, raw: &mut egui::RawInput) {
        raw.events.retain(|e| matches!(e, Event::Screenshot { .. }));
        raw.focused = true;
        if let Some((mods, events)) = self.frames.pop_front() {
            self.mods = mods;
            raw.events.push(Event::ModifiersChanged(mods));
            raw.events.extend(events);
        } else {
            raw.events.push(Event::ModifiersChanged(self.mods));
        }
    }

    pub fn screenshot(&mut self, image: &egui::ColorImage) {
        if let Some(name) = self.shot.take() {
            let path = self.dir.join(format!("{name}.png"));
            match save_png(&path, image) {
                Ok(()) => self.log.push(format!("      снимок {name}.png")),
                Err(e) => self.check(&format!("снимок {name}"), false, &e),
            }
        }
    }

    /// Пишет отчёт и отдаёт код выхода.
    pub fn finish(&self) -> i32 {
        let mut text = self.log.join("\n");
        text.push_str(&format!("\n\nпройдено {}, не пройдено {}\n", self.passed, self.failed));
        let _ = std::fs::write(self.dir.join("report.txt"), text);
        (self.failed > 0) as i32
    }

    fn check(&mut self, what: &str, ok: bool, detail: &str) {
        if ok {
            self.passed += 1;
            self.log.push(format!("  ok  {what}"));
        } else {
            self.failed += 1;
            self.log.push(format!("FAIL  {what}: {detail}"));
        }
    }

    fn key(&mut self, key: Key, mods: Modifiers) {
        let ev = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: mods };
        self.frames.push_back((mods, vec![ev(true), ev(false)]));
        self.frames.push_back((Modifiers::NONE, vec![]));
    }

    fn button(pos: Pos2, button: PointerButton, pressed: bool) -> Event {
        Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }
    }

    fn click(&mut self, p: Pos2) {
        self.frames.push_back((Modifiers::NONE, vec![Event::PointerMoved(p)]));
        self.frames.push_back((Modifiers::NONE, vec![Self::button(p, PointerButton::Primary, true)]));
        self.frames.push_back((Modifiers::NONE, vec![Self::button(p, PointerButton::Primary, false)]));
        self.frames.push_back((Modifiers::NONE, vec![]));
    }

    fn drag(&mut self, from: Pos2, to: Pos2, button: PointerButton) {
        self.drag_with(from, to, button, Modifiers::NONE);
    }

    /// Протяжка с зажатыми клавишами.
    fn drag_with(&mut self, from: Pos2, to: Pos2, button: PointerButton, mods: Modifiers) {
        self.frames.push_back((mods, vec![Event::PointerMoved(from)]));
        self.frames.push_back((mods, vec![Self::button(from, button, true)]));
        for i in 1..=6 {
            let p = from + (to - from) * (i as f32 / 6.0);
            self.frames.push_back((mods, vec![Event::PointerMoved(p)]));
        }
        self.frames.push_back((mods, vec![Self::button(to, button, false)]));
        self.frames.push_back((Modifiers::NONE, vec![]));
    }

    fn wheel(&mut self, p: Pos2, notches: f32) {
        self.wheel_with(p, notches, Modifiers::NONE);
    }

    fn wheel_with(&mut self, p: Pos2, notches: f32, mods: Modifiers) {
        let turn = Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: vec2(0.0, notches), phase: egui::TouchPhase::Move, modifiers: mods };
        self.frames.push_back((Modifiers::NONE, vec![Event::PointerMoved(p)]));
        self.frames.push_back((mods, vec![turn]));
        self.frames.push_back((Modifiers::NONE, vec![]));
    }

    fn snap(&mut self, ctx: &egui::Context, name: &str) {
        self.shot = Some(name.to_owned());
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }

    /// Один шаг сценария за вызов. `false` — сценарий закончен.
    pub fn tick(&mut self, app: &mut App, ctx: &egui::Context) -> bool {
        if !self.frames.is_empty() || self.shot.is_some() {
            return true;
        }
        let now = ctx.input(|i| i.time);
        // Пока читаются паки и грузятся картинки, сценарий стоит.
        if now < self.resume_at || app.library.loading() || !app.textures.idle() {
            return true;
        }
        let at = |app: &App, w: [f32; 2]| app.cam.to_screen(app.canvas, w);
        let cell = |app: &App, x: f32, y: f32| at(app, [x + 0.5, y + 0.5]);
        // Контур стены: прямоугольник по узлам, последний щелчок — в первый узел.
        const NODES: [[f32; 2]; 5] = [[2.0, 2.0], [9.0, 2.0], [9.0, 7.0], [2.0, 7.0], [2.0, 2.0]];
        let step = self.step;
        self.step += 1;
        match step {
            0 => {
                let map_rect = app.cam.map_rect(app.canvas, app.map.width, app.map.height);
                self.check("карта видна целиком", app.canvas.contains_rect(map_rect), &format!("холст {:?}, карта {map_rect:?}", app.canvas));
                // Первая часть сценария — кистью «Клетки», как было до круглых кистей.
                app.brush.shape = Tip::Cells;
                self.key(Key::F, Modifiers::NONE);
                let (a, b) = (cell(app, 2.0, 2.0), cell(app, 8.0, 6.0));
                self.drag(a, b, PointerButton::Primary);
            }
            1 => {
                let n = app.map.floor_count();
                self.check("протяжка закрасила 7×5 клеток", n == 35 && app.map.floor_at(2, 2) == 1 && app.map.floor_at(8, 6) == 1, &format!("закрашено {n}"));
                self.check("правка отмечена", app.dirty, "карта не помечена изменённой");
                self.key(Key::W, Modifiers::NONE);
            }
            2..=6 => {
                let p = at(app, NODES[step - 2]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            7 => {
                let ok = app.map.walls.len() == 1 && app.map.walls[0].closed && app.map.walls[0].points.len() == 4 && app.draft.is_empty();
                self.check("щелчок по первой вершине замкнул стену из 4 вершин", ok, &format!("стены {:?}, черновик {:?}", app.map.walls, app.draft));
                self.key(Key::Z, CTRL);
            }
            8 => {
                self.check("Ctrl+Z убрал стену", app.map.walls.is_empty(), &format!("стен {}", app.map.walls.len()));
                self.key(Key::Y, CTRL);
            }
            9 => {
                self.check("Ctrl+Y вернул стену", app.map.walls.len() == 1, &format!("стен {}", app.map.walls.len()));
                let p = app.canvas.center();
                self.scale = app.cam.scale;
                self.world = app.cam.to_world(app.canvas, p);
                self.wheel(p, 2.0);
            }
            10 => {
                let after = app.cam.to_world(app.canvas, app.canvas.center());
                let still = (after[0] - self.world[0]).abs() < 0.01 && (after[1] - self.world[1]).abs() < 0.01;
                self.check("колесо приблизило карту", app.cam.scale > self.scale * 1.2, &format!("{} → {}", self.scale, app.cam.scale));
                self.check("точка под курсором осталась на месте", still, &format!("{:?} → {after:?}", self.world));
                self.pan = app.cam.pan;
                let c = app.canvas.center();
                self.drag(c, c + vec2(120.0, 60.0), PointerButton::Middle);
            }
            11 => {
                let moved = app.cam.pan - self.pan;
                self.check("средняя кнопка сдвинула карту", moved.x > 60.0 && moved.y > 30.0 && app.map.floor_count() == 35, &format!("сдвиг {moved:?}"));
                self.key(Key::Home, Modifiers::NONE);
            }
            12 => {
                let file = self.dir.join("проверка.karta");
                let before = app.map.clone();
                let saved = app.save_to(&file);
                self.check("карта сохранилась", saved && !app.dirty, &format!("{:?}", app.error));
                app.map = Map::new(5, 5);
                let opened = app.open_path(&file);
                self.check("сохранённая карта открылась такой же", opened && app.map == before, &format!("{:?}", app.error));
                self.key(Key::V, Modifiers::NONE);
                self.resume_at = now + 0.2;
            }
            13 => {
                self.check("клавиша V — инструмент выбора", app.tool == Tool::Select, &format!("{:?}", app.tool));
                let p = at(app, [5.5, 2.05]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            14 => {
                self.check("щелчок у стены выбрал её", app.sel == Some(Sel::Wall(0)), &format!("{:?}", app.sel));
                self.key(Key::Delete, Modifiers::NONE);
            }
            15 => {
                self.check("Delete удалил стену", app.map.walls.is_empty(), &format!("стен {}", app.map.walls.len()));
                self.key(Key::Z, CTRL);
            }
            16 => {
                self.check("Ctrl+Z вернул удалённую стену", app.map.walls.len() == 1, &format!("стен {}", app.map.walls.len()));
                // Открытая стена: две вершины и Enter.
                self.key(Key::W, Modifiers::NONE);
            }
            17 | 18 => {
                let p = at(app, [[12.0, 4.0], [20.0, 4.0]][step - 17]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            19 => {
                self.key(Key::Enter, Modifiers::NONE);
            }
            20 => {
                let ok = app.map.walls.len() == 2 && !app.map.walls[1].closed && app.map.walls[1].points == vec![[12.0, 4.0], [20.0, 4.0]];
                self.check("Enter закончил открытую стену", ok, &format!("{:?}", app.map.walls.last()));
                self.snap(ctx, "карта");
            }
            21 => {
                // Кисть подземелья: пол комнаты, стены встают сами.
                self.key(Key::D, Modifiers::NONE);
                let (a, b) = (cell(app, 12.0, 10.0), cell(app, 17.0, 14.0));
                self.drag(a, b, PointerButton::Primary);
            }
            22 => {
                let n = app.map.count(Layer::Rooms);
                let edges = app.map.room_edges();
                self.check("кисть подземелья: комната 6×5 и четыре стены вокруг", n == 30 && edges.len() == 4, &format!("клеток {n}, стен {}", edges.len()));
                let (a, b) = (cell(app, 15.0, 13.0), cell(app, 20.0, 16.0));
                self.drag(a, b, PointerButton::Primary);
            }
            23 => {
                let edges = app.map.room_edges();
                let total: f32 = edges.iter().map(|(a, b)| (b[0] - a[0]).abs() + (b[1] - a[1]).abs()).sum();
                self.check("пристроенная комната: стена между ними исчезла, обвод 32 клетки", total == 32.0 && app.map.count(Layer::Rooms) == 48, &format!("обвод {total}, клеток {}", app.map.count(Layer::Rooms)));

                // Свои текстуры: пол и стены из папки.
                let dir = self.dir.join("текстуры");
                let _ = std::fs::create_dir_all(&dir);
                let floor = dir.join("плитка.png");
                let wall = dir.join("кирпич.png");
                let tile = image::RgbaImage::from_fn(64, 64, |x, y| if (x / 16 + y / 16) % 2 == 0 { image::Rgba([150, 140, 120, 255]) } else { image::Rgba([110, 100, 86, 255]) });
                let brick = image::RgbaImage::from_fn(64, 32, |x, y| if y % 16 == 0 || (x + (y / 16) * 16) % 32 == 0 { image::Rgba([60, 50, 44, 255]) } else { image::Rgba([150, 70, 50, 255]) });
                let wrote = tile.save(&floor).is_ok() && brick.save(&wall).is_ok();
                app.set_textures_dir(dir.clone());
                // Две картинки папки, встроенные текстуры, объекты и стены.
                let builtin = crate::library::BUILTIN_TEXTURES.len() + crate::library::BUILTIN_OBJECTS.len() + crate::library::BUILTIN_WALLS.len();
                self.check("папка текстур прочиталась", wrote && app.library.assets.len() == 2 + builtin, &format!("ассетов {}", app.library.assets.len()));
                app.use_floor_texture(&floor.to_string_lossy());
                app.use_wall_paint(Paint::Texture(wall.to_string_lossy().into_owned()));
                let (a, b) = (cell(app, 3.0, 10.0), cell(app, 8.0, 16.0));
                self.drag(a, b, PointerButton::Primary);
            }
            24 => {
                // Своя текстура встаёт в палитру сразу за встроенными.
                let own = map::builtin_materials().len();
                let textured = (0..app.map.height as i32).flat_map(|y| (0..app.map.width as i32).map(move |x| (x, y))).filter(|&(x, y)| app.map.room_at(x, y) == own as u8 + 1).count();
                let is_tex = matches!(app.map.materials.get(own).map(|m| &m.paint), Some(Paint::Texture(k)) if !k.starts_with(crate::library::BUILTIN));
                self.check("пол своей текстурой: 6×7 клеток", is_tex && textured == 42, &format!("клеток {textured}, материалы {:?}", app.map.materials.len()));
                self.check("стены своей текстурой", matches!(app.map.wall_style.paint, Paint::Texture(_)), "стены не сменили вид");
                let key = app.library.assets[0].key.clone();
                let ok = app.textures.get(&key, false).is_some();
                self.check("текстура загрузилась в видеокарту", ok, "картинка не прочиталась");
                // Настройка запомнилась.
                let saved = std::fs::read_to_string(app.data_dir.join("settings.json")).unwrap_or_default();
                self.check("папка текстур записана в настройки", saved.contains("текстуры"), &saved);

                // Автосейв: копия есть, а после «падения» её находит следующий запуск.
                app.autosave(now, true);
                self.check("автосейв сделал копию", app.recovery.has_copy(), "копии нет");
                let before = app.map.clone();
                app.recovery.drop_lock();
                let next = Recovery::new(&app.data_dir);
                let found = next.orphans();
                self.check("следующий запуск нашёл несохранённую карту", found.len() == 1, &format!("{found:?}"));
                if let Some(o) = found.first() {
                    let restored = app.restore(o);
                    self.check("карта из копии та же", restored && app.map == before && app.dirty, &format!("{:?}", app.error));
                    self.check("возвращённая копия убрана", next.orphans().is_empty(), "копия осталась");
                }
            }
            25 => {
                // Генератор: то же зерно — та же карта; отмена возвращает прежнее.
                let before = app.map.rooms.clone();
                let opt = Options { rooms: 9, seed: "проверка".into(), ..Default::default() };
                app.material = map::builtin_materials().len();
                let n = app.generate(&opt);
                let first = app.map.rooms.clone();
                self.check("генератор расставил комнаты", n >= 5 && app.map.count(Layer::Rooms) > 60, &format!("комнат {n}"));
                app.undo();
                self.check("Ctrl+Z после генерации вернул прежнее", app.map.rooms == before, "комнаты не те");
                app.generate(&opt);
                self.check("то же зерно — то же подземелье", app.map.rooms == first, "карта другая");
                app.generator = Some(opt);
                self.snap(ctx, "подземелье");
            }
            26 => {
                // Пак Dungeondraft с объектом и дверью: тем же устройством, что настоящие.
                app.generator = None;
                app.undo();
                let png = |img: image::RgbaImage| {
                    let mut out = std::io::Cursor::new(Vec::new());
                    let _ = img.write_to(&mut out, image::ImageFormat::Png);
                    out.into_inner()
                };
                let barrel = image::RgbaImage::from_fn(256, 256, |x, y| {
                    let d = ((x as f32 - 128.0).powi(2) + (y as f32 - 128.0).powi(2)).sqrt();
                    if d < 100.0 { image::Rgba([120, 80, 40, 255]) } else if d < 112.0 { image::Rgba([50, 40, 30, 255]) } else { image::Rgba([0, 0, 0, 0]) }
                });
                let table = image::RgbaImage::from_fn(512, 256, |x, y| if (16..496).contains(&x) && (16..240).contains(&y) { image::Rgba([170, 130, 80, 255]) } else { image::Rgba([0, 0, 0, 0]) });
                let door = image::RgbaImage::from_fn(256, 64, |x, y| if y < 6 || y > 57 || x < 6 || x > 249 { image::Rgba([40, 30, 20, 255]) } else { image::Rgba([150, 100, 60, 255]) });
                let pack = self.dir.join("Проверка.dungeondraft_pack");
                crate::pck::write_test_pack(
                    &pack,
                    "TEST01",
                    &[
                        ("pack.json", b"{}".to_vec()),
                        ("textures/objects/Barrels/Barrel_Test_1x1.png", png(barrel)),
                        ("textures/objects/Tables/Table_Test_2x1.png", png(table)),
                        ("textures/portals/Doors/Door_Test_1x1.png", png(door)),
                    ],
                );
                app.add_pack(pack);
            }
            27 => {
                let objects = app.library.filter(Kind::Object, "").len();
                let doors = app.library.filter(Kind::Door, "").len();
                let builtin = crate::library::BUILTIN_OBJECTS.len();
                self.check("пак прочитан: 2 объекта и дверь", objects == 2 + builtin && doors == 1, &format!("объектов {objects}, дверей {doors}, паки {:?}", app.library.packs));
                let found = app.library.filter(Kind::Object, "barrel").len();
                self.check("поиск по имени нашёл бочку", found == 1, &format!("найдено {found}"));
                // Дверь из пака — в верхнюю стену комнаты 12..18 × 10..15.
                let door = app.library.assets[app.library.filter(Kind::Door, "")[0]].key.clone();
                app.door_asset = Some(door);
                self.key(Key::O, Modifiers::NONE);
                let p = at(app, [13.4, 10.1]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            28 => {
                let ok = app.map.portals.len() == 1 && app.map.portals[0].a == [13.0, 10.0] && app.map.portals[0].b == [14.0, 10.0] && app.map.portals[0].asset.is_some();
                self.check("дверь встала в стену ровно в клетку", ok, &format!("{:?}", app.map.portals));
                let cut = map::cut_portals(&app.map.room_edges(), &app.map.portals);
                let length = |s: &[map::Segment]| s.iter().map(|(a, b)| (b[0] - a[0]).abs() + (b[1] - a[1]).abs()).sum::<f32>();
                self.check("в стене проём под дверь", (length(&app.map.room_edges()) - length(&cut) - 1.0).abs() < 1e-4, "стена не прервалась");
                app.door_kind = PortalKind::Window;
                app.door_width = 2.0;
                let p = at(app, [17.0, 16.9]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            29 => {
                let w = app.map.portals.get(1).cloned();
                let ok = w.as_ref().is_some_and(|w| w.kind == PortalKind::Window && w.a == [16.0, 17.0] && w.b == [18.0, 17.0]);
                self.check("окно в две клетки встало в нижнюю стену", ok, &format!("{w:?}"));
                // Объекты: бочка по щелчку, привязка к полуклетке.
                let barrel = app.library.assets[app.library.filter(Kind::Object, "barrel")[0]].key.clone();
                app.object_asset = Some(barrel);
                self.key(Key::P, Modifiers::NONE);
                let p = at(app, [14.2, 12.3]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            30 => {
                let p = app.map.props.first().cloned();
                self.check("бочка встала по сетке, 1×1 по размеру картинки", p.as_ref().is_some_and(|p| p.pos == [14.0, 12.5] && p.size == [1.0, 1.0]), &format!("{p:?}"));
                let table = app.library.assets[app.library.filter(Kind::Object, "table")[0]].key.clone();
                app.object_asset = Some(table);
                let p = at(app, [15.6, 13.4]);
                self.click(p);
                self.resume_at = now + CLICK_GAP;
            }
            31 => {
                let t = app.map.props.get(1).cloned();
                self.check("стол 2×1", t.as_ref().is_some_and(|t| t.size == [2.0, 1.0]), &format!("{t:?}"));
                self.key(Key::R, Modifiers::NONE);
            }
            32 => {
                self.check("R повернул выбранный стол на 90°", app.map.props[1].rot == 90.0, &format!("{}", app.map.props[1].rot));
                // Выбор и перетаскивание бочки на две клетки вправо.
                self.key(Key::V, Modifiers::NONE);
                let (a, b) = (at(app, [14.0, 12.5]), at(app, [16.0, 11.5]));
                self.drag(a, b, PointerButton::Primary);
            }
            33 => {
                let pos = app.map.props[0].pos;
                self.check("бочку перетащили", pos == [16.0, 11.5] && app.sel == Some(Sel::Prop(0)), &format!("{pos:?}, {:?}", app.sel));
                self.key(Key::Z, CTRL);
            }
            34 => {
                self.check("Ctrl+Z вернул бочку на место", app.map.props[0].pos == [14.0, 12.5], &format!("{:?}", app.map.props[0].pos));
                let file = self.dir.join("с дверями.karta");
                let before = app.map.clone();
                let ok = app.save_to(&file) && app.open_path(&file) && app.map == before;
                self.check("двери и объекты сохраняются в карту", ok, &format!("{:?}", app.error));
            }
            35 => {
                // Крупнее, чтобы на снимке были видны двери. После открытия карта уже вписана.
                let p = at(app, [15.0, 13.0]);
                app.cam.zoom_at(app.canvas, p, 2.0);
                self.resume_at = now + 0.3;
            }
            36 => {
                self.snap(ctx, "двери и объекты");
            }
            37 => {
                // Кисти: круг, спрей, мягкий край, ластик кругом.
                self.key(Key::Home, Modifiers::NONE);
                app.set_tool(Tool::Dungeon);
                app.material = 0;
                app.brush = Brush { shape: Tip::Circle, radius: 2.0, hardness: 1.0, flow: 1.0 };
                self.scale = app.map.rooms.area();
            }
            38 => {
                let (a, b) = (at(app, [23.0, 4.0]), at(app, [27.0, 4.0]));
                self.drag(a, b, PointerButton::Primary);
            }
            39 => {
                let grown = app.map.rooms.area() - self.scale;
                let capsule = 4.0 * 4.0 + std::f32::consts::PI * 4.0;
                self.check("круглая кисть: мазок-капсула нужной площади", (grown - capsule).abs() < 3.0, &format!("прибавилось {grown:.1} кл., ждали {capsule:.1}"));
                let round = app.map.room_edges().iter().any(|(a, b)| a[0] != b[0] && a[1] != b[1]);
                self.check("стена вокруг круглого мазка скруглена", round, "все стены прямые");
                // Спрей мягкой кистью травой по полу двора.
                app.set_tool(Tool::Floor);
                app.material = 3;
                app.brush = Brush { shape: Tip::Spray, radius: 2.0, hardness: 0.3, flow: 0.6 };
                let (a, b) = (at(app, [4.0, 4.0]), at(app, [8.0, 5.0]));
                self.drag(a, b, PointerButton::Primary);
            }
            40 => {
                let grass = app.map.terrain.iter().find(|c| c.material == 3);
                let soft = grass.is_some_and(|c| c.mask.data.iter().any(|&v| v > 20 && v < 230));
                self.check("спрей лёг травой с мягкими краями", soft, &format!("слоёв местности {}", app.map.terrain.len()));
                self.check("мазок — одна правка: Ctrl+Z его снимет", { app.undo(); let gone = !app.map.terrain.iter().any(|c| c.material == 3); app.redo(); gone }, "отмена не сняла спрей");
                // Ластик кругом поперёк капсулы.
                app.set_tool(Tool::Erase);
                app.brush = Brush { shape: Tip::Circle, radius: 0.75, hardness: 1.0, flow: 1.0 };
                self.scale = app.map.rooms.area();
                let (a, b) = (at(app, [25.0, 0.5]), at(app, [25.0, 7.5]));
                self.drag(a, b, PointerButton::Primary);
            }
            41 => {
                let left = app.map.rooms.area();
                self.check("ластик прорезал капсулу", left < self.scale - 4.0, &format!("{:.1} → {left:.1}", self.scale));
                let file = self.dir.join("кисти.karta");
                let before = app.map.clone();
                let ok = app.save_to(&file) && app.open_path(&file) && app.map == before;
                let size = std::fs::metadata(&file).map_or(0, |m| m.len());
                self.check("карта с масками сохраняется и открывается", ok && size < 200_000, &format!("{size} байт, {:?}", app.error));
                app.set_tool(Tool::Floor);
                app.brush = Brush { shape: Tip::Circle, radius: 2.0, hardness: 0.3, flow: 1.0 };
                self.hover_at = Some(at(app, [16.0, 3.0]));
            }
            42 => {
                if let Some(p) = self.hover_at.take() {
                    self.frames.push_back((Modifiers::NONE, vec![Event::PointerMoved(p)]));
                    self.frames.push_back((Modifiers::NONE, vec![]));
                    // Этот же шаг ещё раз — уже со снимком.
                    self.step -= 1;
                    return true;
                }
                self.snap(ctx, "кисти");
            }
            43 => {
                // Свободная фигура, как рисуют от руки: дуга, крюк, перемычка поперёк
                // и квадратная пристройка. Во внутренних углах раньше торчали шипы.
                app.new_map(30, 20);
                app.set_tool(Tool::Dungeon);
                app.material = 0;
                // Дуга, как ведёт рука, перемычка поперёк неё и квадратная пристройка.
                let arc: Vec<[f32; 2]> = (0..=60).map(|k| {
                    let a = (150.0 + k as f32 * 4.5f32).to_radians();
                    [14.0 + 5.5 * a.cos(), 10.0 + 5.0 * a.sin()]
                }).collect();
                let strokes: [(Tip, f32, Vec<[f32; 2]>); 3] = [
                    (Tip::Circle, 1.5, arc),
                    (Tip::Circle, 1.0, vec![[9.0, 8.0], [15.5, 9.5]]),
                    (Tip::Square, 1.5, vec![[22.5, 14.0], [25.0, 14.0]]),
                ];
                for (shape, radius, path) in strokes {
                    app.brush = Brush { shape, radius, hardness: 0.8, flow: 1.0 };
                    app.begin_stroke(path[0]);
                    for &w in &path[1..] {
                        app.stroke_to(w);
                    }
                    app.end_stroke();
                }
                // Излом стены там, где она не идёт по сетке: плавный изгиб — мелкие.
                let mut worst = 0.0f32;
                for line in mask::join(app.map.room_edges()) {
                    for k in 1..line.len().saturating_sub(1) {
                        let (a, b, c) = (line[k - 1], line[k], line[k + 1]);
                        let grid = |p: [f32; 2], q: [f32; 2]| p[0] == q[0] || p[1] == q[1];
                        if grid(a, b) && grid(b, c) {
                            continue;
                        }
                        let (d1, d2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
                        let cos = (d1[0] * d2[0] + d1[1] * d2[1]) / ((d1[0].hypot(d1[1]) * d2[0].hypot(d2[1])).max(1e-9));
                        worst = worst.max(cos.clamp(-1.0, 1.0).acos().to_degrees());
                    }
                }
                self.check("у свободной фигуры стены без шипов", worst < 50.0, &format!("самый резкий излом {worst:.0}°"));
                let square = app.map.room_edges().iter().filter(|(a, b)| (a[0] == b[0] || a[1] == b[1]) && ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()) > 2.0).count();
                self.check("квадратная кисть оставила прямые стены", square >= 4, &format!("длинных прямых {square}"));
                let c = app.canvas;
                app.cam.fit(c, 30, 20);
                app.cam.zoom_at(c, app.cam.to_screen(c, [17.0, 10.0]), 1.4);
            }
            44 => self.snap(ctx, "от руки"),
            45 => {
                // Изгиб: верхняя стена комнаты наружу, нарисованная стена вниз.
                app.new_map(20, 12);
                app.set_tool(Tool::Dungeon);
                app.brush.shape = Tip::Cells;
                app.material = 0;
                app.paint((2, 2), (8, 6));
                app.map.walls.push(map::Wall { points: vec![[12.0, 3.0], [18.0, 3.0]], closed: false, bends: Vec::new() });
                let c = app.canvas;
                app.cam.fit(c, 20, 12);
                self.area = app.map.rooms.area();
                let bend = Modifiers { ctrl: true, shift: true, command: true, ..Modifiers::NONE };
                let (a, b) = (at(app, [5.0, 2.0]), at(app, [5.0, 0.5]));
                self.drag_with(a, b, PointerButton::Primary, bend);
            }
            46 => {
                let grown = app.map.rooms.area() - self.area;
                // Хорда 7, прогиб 1,5: сегмент круга около 7,4 клетки.
                self.check("Ctrl+Shift выгнул стену комнаты наружу", (grown - 7.4).abs() < 1.0 && app.map.room_at(5, 1) == 1, &format!("приросло {grown:.2}, пол {}", app.map.room_at(5, 1)));
                let bend = Modifiers { ctrl: true, shift: true, command: true, ..Modifiers::NONE };
                let (a, b) = (at(app, [15.0, 3.0]), at(app, [15.0, 5.0]));
                self.drag_with(a, b, PointerButton::Primary, bend);
            }
            47 => {
                let h = app.map.walls[0].bend(0);
                self.check("нарисованная стена выгнулась вниз на 2 клетки", h == -2.0, &format!("прогиб {h}"));
                // Ручки: бочка вдвое больше за угол, потом поворот за верхнюю ручку.
                let barrel = app.library.assets[app.library.filter(Kind::Object, "barrel")[0]].key.clone();
                app.object_asset = Some(barrel);
                app.object_rot = 0.0;
                app.object_scale = 1.0;
                app.place_prop([5.0, 9.5], false);
                app.set_tool(Tool::Select);
                let prop = app.map.props[0].clone();
                let c = prop.corners()[2];
                let far = [prop.pos[0] + (c[0] - prop.pos[0]) * 2.0, prop.pos[1] + (c[1] - prop.pos[1]) * 2.0];
                let (a, b) = (at(app, c), at(app, far));
                self.drag(a, b, PointerButton::Primary);
            }
            48 => {
                let p = app.map.props[0].clone();
                self.check("угловая ручка увеличила бочку вдвое", p.scale == 2.0 && p.pos == [5.0, 9.5], &format!("размер {}, место {:?}", p.scale, p.pos));
                let (_, turn) = grips(&p, TURN_LIFT_PX / app.cam.scale);
                let (a, b) = (at(app, turn), at(app, [p.pos[0] + 3.0, p.pos[1]]));
                self.drag(a, b, PointerButton::Primary);
            }
            49 => {
                let r = app.map.props[0].rot;
                self.check("верхняя ручка повернула бочку на 90°", r == 90.0, &format!("поворот {r}"));
                self.snap(ctx, "изгиб и ручки");
            }
            50 => {
                // Ctrl с колесом меняет кисть, а не масштаб карты.
                app.set_tool(Tool::Dungeon);
                app.brush = Brush { shape: Tip::Circle, radius: 2.0, hardness: 0.8, flow: 1.0 };
                self.area = app.cam.scale;
                let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
                let p = at(app, [10.0, 6.0]);
                self.wheel_with(p, 2.0, ctrl);
            }
            51 => {
                let (r, same) = (app.brush.radius, app.cam.scale == self.area);
                self.check("Ctrl с колесом увеличил кисть, карта на месте", (r - 3.125).abs() < 1e-3 && same, &format!("радиус {r}, масштаб {} → {}", self.area, app.cam.scale));
                // Alt с колесом — размер выбранной бочки (сейчас 2).
                app.set_tool(Tool::Select);
                app.sel = Some(Sel::Prop(0));
                let alt = Modifiers { alt: true, ..Modifiers::NONE };
                let p = at(app, [10.0, 6.0]);
                self.wheel_with(p, -2.0, alt);
            }
            52 => {
                let s = app.map.props[0].scale;
                self.check("Alt с колесом уменьшил бочку, карта на месте", (s - 2.0 / 1.21).abs() < 1e-3 && app.cam.scale == self.area, &format!("размер {s}"));
            }
            _ => return false,
        }
        true
    }
}
