//! Кадр: что делается за кадр, окна, сочетания клавиш.

use super::*;

impl App {
    pub(super) fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let started = crate::plat::Instant::now();
        self.frame += 1;
        // Картинки по размеру на экране — везде, кроме выгрузки для Foundry: там полные.
        self.textures.lod = !self.export.as_ref().is_some_and(|e| !e.picture);
        // Быстрый щелчок сдвигает мышь на 7–25 точек, при родных 6 он терялся.
        // На карте порог ниже: от него начинаются протяжки.
        let on_map = ctx.input(|i| i.pointer.latest_pos()).is_some_and(|p| self.canvas.contains(p));
        ctx.options_mut(|o| o.input_options.max_click_dist = if on_map { 12.0 } else { 28.0 });
        self.textures.poll(&ctx);
        self.library.poll(&self.textures);
        if !self.library.loading()
            && let Some(plan) = self.pending_plan.take()
        {
            self.build_from_file(&plan);
        }
        if !self.library.loading()
            && let Some(osm) = self.pending_osm.take()
        {
            self.import_osm_file(&osm);
        }
        self.place_step(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.control_step(&ctx);
        if self.library.loading() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        let now = ctx.input(|i| i.time);
        self.autosave(now, false);
        if self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(recovery::INTERVAL));
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty && !self.allow_close && self.test.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if self.export.is_none() {
            self.shortcuts(&ctx);
        }
        self.export_step(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.hide_after_ai(&ctx);

        let t = crate::plat::Instant::now();
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status(ui));
        // Как в привычных редакторах карт: слева инструменты и их настройки, справа библиотека.
        egui::Panel::left("tools").exact_size(64.0).resizable(false).frame(egui::Frame::new().fill(PANEL)).show(ui, |ui| self.tool_cells(ui));
        egui::Panel::left("options").default_size(250.0).size_range(200.0..=420.0).resizable(true).show(ui, |ui| self.options_bar(ui));
        egui::Panel::right("library").default_size(330.0).size_range(200.0..=700.0).resizable(true).show(ui, |ui| self.shelf(ui));
        self.prof.add("panels", t);
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG)).show(ui, |ui| self.canvas_ui(ui));
        // Пока идёт выгрузка, поверх холста ничего: окно, заходящее на холст,
        // впечатывалось краем в каждую плитку картинки.
        if self.export.is_none() {
            self.levels_overlay(&ctx);
            self.tool_flyout(&ctx);
            self.windows(&ctx);
        }

        let title = format!("{}{} — Картограф", if self.dirty { "• " } else { "" }, self.name());
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        self.screenshots(&ctx);
        self.ground_cache.trim(self.frame);
        self.prof.add("frame", started);
        self.prof.end_frame();
        if let Some(mut test) = self.test.take() {
            let more = test.tick(self, &ctx);
            if more {
                self.test = Some(test);
                ctx.request_repaint();
            } else {
                self.recovery.close();
                std::process::exit(test.finish());
            }
        }
    }

    pub(super) fn update_window(&mut self, ctx: &egui::Context) {
        self.update.poll();
        if !self.update.shown {
            return;
        }
        let mut open = true;
        egui::Window::new("Обновление").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Сейчас стоит {}.", env!("CARGO_PKG_VERSION")));
            if let Some(m) = &self.update.message {
                ui.label(m);
            }
            if self.update.installed {
                ui.label("Новая версия поставлена.");
                if ui.button("Перезапустить").clicked() {
                    self.update.restart = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else if let Some((got, total)) = self.update.progress {
                ui.add(egui::ProgressBar::new(got as f32 / total.max(1) as f32).show_percentage());
            } else if let Some(f) = self.update.found.clone() {
                ui.strong(format!("Вышла версия {}", f.version));
                if !f.notes.is_empty() {
                    ui.label(&f.notes);
                }
                ui.label(format!("{:.0} МБ", f.size as f64 / 1_048_576.0));
                if ui.add_enabled(!self.update.busy(), egui::Button::new("Обновить")).clicked() {
                    self.update.install(ctx);
                }
            }
        });
        self.update.shown = open;
    }

    pub(super) fn windows(&mut self, ctx: &egui::Context) {
        self.update_window(ctx);
        self.check_window(ctx);
        self.place_window(ctx);
        self.sketch_window(ctx);
        if let Some(e) = self.error.clone() {
            let mut open = true;
            egui::Window::new("Ошибка").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| ui.label(e));
            if !open {
                self.error = None;
            }
        }

        if !self.orphans.is_empty() {
            let mut take = None;
            let mut drop = None;
            egui::Window::new("Несохранённые карты").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label("Прошлый раз программа закрылась, не сохранив эти карты:");
                ui.add_space(6.0);
                for (i, o) in self.orphans.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let when = ago(recovery::now_unix().saturating_sub(o.meta.saved));
                        ui.label(format!("{} — {when}", o.meta.name));
                        if ui.button("Вернуть").clicked() {
                            take = Some(i);
                        }
                        if ui.button("Выбросить").clicked() {
                            drop = Some(i);
                        }
                    });
                }
            });
            if let Some(i) = take {
                let o = self.orphans.remove(i);
                self.restore(&o);
            } else if let Some(i) = drop {
                let o = self.orphans.remove(i);
                self.recovery.discard(&o);
            }
        }

        if let Some(maps) = self.open_list.take() {
            let mut open = true;
            let mut chosen = None;
            egui::Window::new("Открыть карту").open(&mut open).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                if maps.is_empty() {
                    ui.label("В данных Foundry пока нет сохранённых карт.");
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for p in &maps {
                        let name = p.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                        if ui.button(name).clicked() {
                            chosen = Some(p.clone());
                        }
                    }
                });
            });
            if let Some(p) = chosen {
                self.open_path(&p);
            } else if open {
                self.open_list = Some(maps);
            }
        }

        if let Some(mut dlg) = self.export_dialog.take() {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Выгрузка для Foundry").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.label("Пикселей на клетку:");
                ui.horizontal(|ui| {
                    for px in EXPORT_PX_CHOICES {
                        ui.selectable_value(&mut dlg.px, px, px.to_string());
                    }
                });
                // Мир для Boundless: огромная карта одной сценой, модуль подгружает её по частям.
                // Пишет только программа; район в нём не нужен.
                if cfg!(not(target_arch = "wasm32")) {
                    ui.checkbox(&mut dlg.boundless, "Мир для Boundless").on_hover_text("Огромная карта одной сценой для модуля Boundless: пирамида плиток, манифест со стенами, дверями и светом, дома с крышами. Снимается кусками; карта 1000×1000 — около часа.");
                }
                // Только район — отдельной сценой: у огромной карты целиком на клетку мало точек.
                let mut part = dlg.district.is_some();
                if ui.add_enabled(!dlg.boundless, egui::Checkbox::new(&mut part, "Только район")).on_hover_text("Прямоугольник карты отдельной сценой, в полном разрешении. Стены, крыши и предметы у края идут целиком.").changed() {
                    dlg.district = part.then(|| self.view_cells());
                }
                if let Some(a) = &mut dlg.district {
                    let (mw, mh) = (self.map.width, self.map.height);
                    ui.horizontal(|ui| {
                        ui.label("x");
                        ui.add(egui::DragValue::new(&mut a[0]).range(0..=mw.saturating_sub(1)));
                        ui.label("–");
                        ui.add(egui::DragValue::new(&mut a[2]).range(1..=mw));
                        ui.label("y");
                        ui.add(egui::DragValue::new(&mut a[1]).range(0..=mh.saturating_sub(1)));
                        ui.label("–");
                        ui.add(egui::DragValue::new(&mut a[3]).range(1..=mh));
                        if ui.button("По окну").on_hover_text("Район — то, что сейчас видно в окне.").clicked() {
                            *a = self.view_cells();
                        }
                    });
                }
                let (cw, ch) = match dlg.district {
                    Some(a) => (a[2].saturating_sub(a[0]).max(1), a[3].saturating_sub(a[1]).max(1)),
                    None => (self.map.width, self.map.height),
                };
                let fit = export::fit_px(cw, ch, dlg.px);
                let (w, h) = (cw * fit, ch * fit);
                if dlg.boundless {
                    let world = crate::boundless::World::new(self.map.width * dlg.px, self.map.height * dlg.px, dlg.px);
                    let cc = world.chunk_cells();
                    let chunks = self.map.width.div_ceil(cc) * self.map.height.div_ceil(cc);
                    ui.weak(format!("Мир {0}×{0}, плитка {1}, уровни z0–z{2}, кусков {chunks}.", world.side, world.tile, world.max_z));
                } else if fit < dlg.px {
                    ui.weak(format!("Картинка {w}×{h}: карта большая, больше {} точек Foundry не покажет — будет {fit} на клетку.", export::SCENE_MAX_SIDE + 1));
                } else {
                    ui.weak(format!("Картинка {w}×{h}."));
                }
                ui.add_space(4.0);
                let trees = self.map.props.iter().filter(|p| p.is_canopy()).count();
                if ui.checkbox(&mut self.settings.export_canopies, format!("Кроны деревьев тайлами над токенами ({trees} шт.)")).on_hover_text("В картинке на месте дерева пень, крона — отдельный тайл, который гаснет над токеном. Сама так уходит только встроенная крона; чужому дереву крону включают галочкой в панели объекта.").changed() {
                    self.save_settings();
                }
                ui.add_space(4.0);
                if cfg!(target_arch = "wasm32") {
                    ui.horizontal(|ui| {
                        ui.label("Название сцены:");
                        ui.text_edit_singleline(&mut dlg.name);
                    });
                    let last = crate::plat::last_scene();
                    if last.is_none() {
                        dlg.update = false;
                    }
                    ui.radio_value(&mut dlg.update, false, "Создать новую сцену");
                    ui.add_enabled_ui(last.is_some(), |ui| {
                        let label = last.as_ref().map_or_else(|| "Обновить последнюю сцену".to_owned(), |n| format!("Обновить последнюю сцену («{n}»)"));
                        ui.radio_value(&mut dlg.update, true, label).on_hover_text("Картинка, стены, свет и тайлы заменятся; токены, заметки и звуки останутся.");
                    });
                    ui.weak("Картинки лягут в данные Foundry: карта — в kartograf/<название латиницей>, предметы — в общую kartograf/assets.");
                    ui.add_space(6.0);
                    go = ui.add_enabled(!dlg.name.trim().is_empty(), egui::Button::new("На сцену")).clicked();
                } else {
                    ui.weak("Рядом лягут картинка, сцена для Foundry (.json), Universal VTT (.dd2vtt), стены и крыши; картинки предметов, крон и створок — по одной на ассет в общую папку assets рядом. Держать обе папки вместе. В Foundry: каталог сцен — «Из выгрузки», выбрать .json.");
                    ui.add_space(6.0);
                    go = ui.button("Выбрать место и выгрузить").clicked();
                }
            });
            if go && cfg!(target_arch = "wasm32") {
                // Своя папка на каждую выгрузку: прежние сцены не теряют картинок,
                // а обновлённая не берёт старую картинку из кэша браузера.
                let name: String = dlg.name.trim().chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect();
                let stamp = crate::plat::SystemTime::now().duration_since(crate::plat::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let dir = PathBuf::from("kartograf").join(export::ascii_name(&name)).join(stamp.to_string());
                match dlg.district {
                    Some(area) => {
                        if let Err(err) = self.start_district(dir, name.clone(), dlg.px, area, false) {
                            self.error = Some(err);
                        }
                    }
                    None => self.start_export(dir, name.clone(), dlg.px, false),
                }
                // Район — своя сцена: последнюю (всю карту) он не заменяет.
                if let Some(e) = &mut self.export {
                    e.update = dlg.update && dlg.district.is_none();
                }
                self.scene_name = Some(name);
            } else if go {
                let dir = export::foundry_data_dir().map(|d| d.join("kartograf"));
                if let Some(d) = &dir {
                    let _ = std::fs::create_dir_all(d);
                }
                let mut dialog = crate::plat::FileDialog::new().add_filter("Сцена Foundry", &["json"]).set_file_name(format!("{}.json", export::ascii_name(&self.name())));
                if let Some(d) = dir {
                    dialog = dialog.set_directory(d);
                }
                if let Some(file) = dialog.save_file() {
                    // Сцена — с именем карты, файлы — под выбранным именем латиницей.
                    let stem = file.file_stem().map(|s| export::ascii_name(&s.to_string_lossy()));
                    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
                    let name = self.name();
                    match dlg.district {
                        _ if dlg.boundless => {
                            if let Err(err) = self.start_boundless(dir, name, dlg.px, false) {
                                self.error = Some(err);
                            }
                        }
                        Some(area) => {
                            if let Err(err) = self.start_district(dir, name, dlg.px, area, false) {
                                self.error = Some(err);
                            }
                        }
                        None => self.start_export(dir, name, dlg.px, false),
                    }
                    if let (Some(e), Some(stem)) = (&mut self.export, stem) {
                        e.stem = stem;
                    }
                }
            } else if open {
                self.export_dialog = Some(dlg);
            }
        }

        if let Some(mut opt) = self.generator.take() {
            let mut open = true;
            let mut go = false;
            let dungeon = opt.kind.grows();
            let own_assets = self.library.assets.iter().any(|a| !a.is_builtin());
            let map_seed = self.map.seed.clone();
            egui::Window::new(opt.kind.label())
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(ctx.content_rect().center())
                .show(ctx, |ui| {
                egui::Grid::new("gen").num_columns(2).show(ui, |ui| {
                    ui.label("Ассеты");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut opt.library, false, "Встроенные");
                        ui.add_enabled_ui(own_assets, |ui| {
                            ui.selectable_value(&mut opt.library, true, "Из библиотеки").on_hover_text("Паки и папка из «Правка»; чего там нет — встроенные");
                        });
                        if !own_assets {
                            opt.library = false;
                            ui.weak("(паков и папки нет)");
                        }
                    });
                    ui.end_row();
                    if opt.kind == generate::Kind::Forest {
                        ui.label("Лес");
                        ui.horizontal(|ui| {
                            for s in crate::plan::Season::ALL {
                                ui.selectable_value(&mut opt.season, s, s.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Рельеф");
                        ui.horizontal(|ui| {
                            for k in crate::plan::Hills::ALL {
                                ui.selectable_value(&mut opt.hills, k, k.label());
                            }
                        });
                        ui.end_row();
                    } else if matches!(opt.kind, generate::Kind::Town | generate::Kind::Walls | generate::Kind::House | generate::Kind::Tavern | generate::Kind::Arena | generate::Kind::Church | generate::Kind::Jarl) {
                        ui.label("Время года");
                        ui.horizontal(|ui| {
                            for s in crate::plan::Season::ALL {
                                ui.selectable_value(&mut opt.season, s, s.name());
                            }
                        });
                        ui.end_row();
                    }
                    if opt.kind == generate::Kind::Town {
                        ui.label("Квартал");
                        ui.horizontal(|ui| {
                            for d in crate::plan::District::ALL {
                                ui.selectable_value(&mut opt.district, d, d.label());
                            }
                        });
                        ui.end_row();
                    }
                    if opt.kind == generate::Kind::Cave {
                        ui.label("Пещера");
                        ui.horizontal_wrapped(|ui| {
                            for c in crate::plan::CaveKind::ALL {
                                ui.selectable_value(&mut opt.cave, c, c.label());
                            }
                        });
                        ui.end_row();
                    }
                    if !dungeon {
                        ui.label("Густота");
                        ui.add(egui::Slider::new(&mut opt.density, 0.0..=1.0));
                        ui.end_row();
                        seed_rows(ui, &mut opt.seed, &map_seed);
                        return;
                    }
                    // Пол и стены — по виду подземелья, размеры комнат — свои у каждой.
                    // У киберпанка вид один, набор свой.
                    if opt.kind == generate::Kind::Dungeon {
                        ui.label("Вид");
                        ui.horizontal(|ui| {
                            for d in crate::plan::DungeonKind::ALL {
                                ui.selectable_value(&mut opt.dungeon, d, d.label());
                            }
                        });
                        ui.end_row();
                    }
                    ui.label("Комнат");
                    ui.add(egui::Slider::new(&mut opt.rooms, 3..=30));
                    ui.end_row();
                    ui.label("Кольца");
                    ui.add(egui::Slider::new(&mut opt.loops, 0.0..=1.0)).on_hover_text("Сколько обходных путей: дальние по ходу комнаты соединены напрямую, часть ходов тайные");
                    ui.end_row();
                    seed_rows(ui, &mut opt.seed, &map_seed);
                });
                if dungeon {
                    ui.checkbox(&mut opt.fill, "Двери и объекты");
                }
                {
                    ui.weak(format!("Заменит всю карту ({}×{} кл.), отменяется Ctrl+Z.", self.map.width, self.map.height));
                }
                ui.add_space(4.0);
                go = ui.button("Создать").clicked();
            });
            if go {
                if opt.seed.trim().is_empty() {
                    opt.seed = generate::new_seed();
                }
                if self.settings.gen_library != opt.library {
                    self.settings.gen_library = opt.library;
                    self.save_settings();
                }
                self.gen_season = opt.season;
                self.gen_hills = opt.hills;
                self.gen_cave = opt.cave;
                self.gen_district = opt.district;
                self.gen_dungeon = opt.dungeon;
                self.generate(&opt);
                // Карта построена — окно уходит; откроют снова — с новым сидом.
                open = false;
            }
            if open {
                self.generator = Some(opt);
            }
        }

        if let Some(mut nm) = self.new_map.take() {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Новая карта").open(&mut open).collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut nm.width).range(4..=map::MAX_SIDE).suffix(" кл."));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut nm.height).range(4..=map::MAX_SIDE).suffix(" кл."));
                });
                if self.dirty {
                    ui.weak("Несохранённые правки нынешней карты пропадут.");
                }
                go = ui.button("Создать").clicked();
            });
            if go {
                self.new_map(nm.width, nm.height);
            } else if open {
                self.new_map = Some(nm);
            }
        }

        if self.confirm_close {
            let mut choice = None;
            egui::Window::new("Закрыть карту").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(format!("Сохранить изменения в «{}»?", self.name()));
                ui.horizontal(|ui| {
                    if ui.button("Сохранить").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Не сохранять").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Отмена").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.save();
                    if !self.dirty {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    self.confirm_close = false;
                }
                Some(1) => {
                    self.allow_close = true;
                    self.confirm_close = false;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Some(_) => self.confirm_close = false,
                None => {}
            }
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let cmd = |key| ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key));
        if cmd(Key::Z) {
            self.undo();
        }
        if cmd(Key::Y) || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
            self.redo();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S)) {
            self.save_as();
        } else if cmd(Key::S) {
            self.save();
        }
        if cmd(Key::O) {
            self.open();
        }
        if cmd(Key::G) {
            self.bake_picked();
        }
        if cmd(Key::N) {
            self.new_map = Some(NewMap { width: self.map.width, height: self.map.height });
        }
        if typing {
            return;
        }
        // Сочетания Dungeondraft: X — «Выбор» и обратно к прежнему инструменту, G — сетка,
        // S — привязка, L — свет, R — линейка, Page Up / Page Down — этажи, цифры — слой.
        for tool in Tool::ALL {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, tool.key())) {
                if tool == Tool::Select && self.tool == Tool::Select {
                    self.set_tool(self.prev_tool);
                } else if tool == Tool::Pick && self.tool == Tool::Pick {
                    self.leave_pick(self.pick_from);
                } else {
                    self.set_tool(tool);
                }
            }
        }
        // V — прежняя клавиша «Выбора».
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::V)) {
            self.set_tool(Tool::Select);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::G)) {
            self.settings.grid = !self.settings.grid;
            self.save_settings();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::S)) {
            self.settings.snap = !self.settings.snap;
            self.save_settings();
        }
        // Свет считает `lighting` — его нет в сборке для браузера.
        #[cfg(not(target_arch = "wasm32"))]
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::L)) {
            self.light_view = !self.light_view;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
            self.ruler = !self.ruler;
            self.ruler_line = None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageUp)) && self.level + 1 < self.levels.len() {
            self.switch_level(self.level + 1);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageDown)) && self.level > 0 {
            self.switch_level(self.level - 1);
        }
        // Слой: 1–4 — свои слои Dungeondraft, 7 — над стенами, 9 — над крышами; 8 — на крыше.
        for (key, layer) in [(Key::Num1, 100), (Key::Num2, 200), (Key::Num3, 300), (Key::Num4, 400), (Key::Num7, map::ABOVE_WALLS), (Key::Num8, map::ON_ROOF), (Key::Num9, map::ABOVE_ROOFS)] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                self.set_layer(layer);
            }
        }
        // Стрелки двигают выбранный объект: на полклетки, с Shift — на сотую клетки.
        if let Some(Sel::Prop(i)) = self.sel {
            let mut d = [0.0f32; 2];
            for (key, v) in [(Key::ArrowLeft, [-1.0, 0.0]), (Key::ArrowRight, [1.0, 0.0]), (Key::ArrowUp, [0.0, -1.0]), (Key::ArrowDown, [0.0, 1.0])] {
                for m in [Modifiers::NONE, Modifiers::SHIFT] {
                    if ctx.input_mut(|i| i.consume_key(m, key)) {
                        let step = if m.shift { 0.01 } else { 0.5 };
                        d = [d[0] + v[0] * step, d[1] + v[1] * step];
                    }
                }
            }
            if d != [0.0, 0.0] {
                self.checkpoint();
                let p = &mut self.map.props[i].pos;
                *p = [p[0] + d[0], p[1] + d[1]];
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::OpenBracket)) {
            self.brush.radius = (self.brush.radius / 1.25).max(0.25);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::CloseBracket)) {
            self.brush.radius = (self.brush.radius * 1.25).min(10.0);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.finish_wall(false);
            self.finish_path();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if self.tool == Tool::Pick {
                self.leave_pick(self.pick_from);
            }
            self.draft.clear();
            self.path_draft.clear();
            self.sel = None;
            self.platform_put = None;
            self.wall_join = None;
            self.draft_bends.clear();
            self.ruler = false;
            self.ruler_line = None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)) {
            if !self.path_draft.is_empty() {
                self.path_draft.pop();
            } else if !self.draft.is_empty() {
                self.draft.pop();
            } else {
                self.delete_selected();
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            self.fitted = false;
        }
    }

    pub(super) fn set_door_kind(&mut self, kind: PortalKind) {
        if self.door_kind == kind {
            return;
        }
        self.door_kind = kind;
        // Окну — подоконник, двери — створка, если выбрано не то.
        let is_window = self.door_asset.as_deref().is_some_and(|k| k.contains("window") || k.to_lowercase().contains("sill"));
        if kind == PortalKind::Window && !is_window {
            self.door_asset = Some(format!("{}window_stone", library::BUILTIN));
        } else if kind.is_door() && (is_window || self.door_asset.is_none()) {
            self.door_asset = Some(library::DEFAULT_DOOR.into());
        }
        if kind == PortalKind::DoubleDoor && self.door_width < 2.0 {
            self.door_width = 2.0;
        }
    }
}
