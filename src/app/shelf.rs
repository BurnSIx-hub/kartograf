//! Библиотека: полка, шторка вариантов, поиск, строка состояния.

use super::*;

impl App {
    /// Полка ассетов: слева разделы, справа поиск и большие миниатюры.
    pub(super) fn shelf(&mut self, ui: &mut egui::Ui) {
        ui.set_max_width(ui.available_width());
        ui.add_space(4.0);
        ui.heading("Библиотека");
        ui.horizontal_wrapped(|ui| {
            for k in Kind::ALL {
                ui.selectable_value(&mut self.browse, k, k.label());
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.used_only, false, "Все");
            ui.selectable_value(&mut self.used_only, true, "На карте").on_hover_text("Только то, что уже стоит на этой карте");
        });
        ui.separator();
        self.browser(ui);
    }

    /// Шторка вариантов над миниатюрой семьи — как у эмодзи с оттенками.
    pub(super) fn variant_drawer(&mut self, ctx: &egui::Context) -> Option<String> {
        let (group, rect) = self.drawer.clone()?;
        const SIZE: f32 = 96.0;
        const IN_ROW: usize = 6;
        let mut picked = None;
        // Библиотека справа: варианты раскрываются влево от миниатюры, на карту.
        let area = egui::Area::new(egui::Id::new("variants")).order(egui::Order::Foreground).pivot(egui::Align2::RIGHT_TOP).fixed_pos(rect.left_top() - vec2(6.0, 0.0)).constrain(true).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                for row in group.chunks(IN_ROW) {
                    ui.horizontal(|ui| {
                        for &i in row {
                            let (key, name) = (self.library.assets[i].key.clone(), self.library.assets[i].name.clone());
                            let (r, resp) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::click());
                            self.thumb(ui.painter(), r, &key);
                            if resp.hovered() {
                                ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
                            }
                            if resp.on_hover_text(name).clicked() {
                                picked = Some(key);
                            }
                        }
                    });
                }
            });
        });
        self.drawer_area = area.response.rect;
        if let Some(key) = &picked {
            let family = self.library.assets[group[0]].family.clone();
            self.family_pick.insert(family, key.clone());
            self.drawer = None;
        } else if self.drawer_frame != self.frame && area.response.clicked_elsewhere() {
            self.drawer = None;
        }
        picked
    }

    pub(super) fn browser(&mut self, ui: &mut egui::Ui) {
        // Поиск на всю ширину своей строкой: рядом с кнопками он раздувал панель.
        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Поиск").desired_width(ui.available_width()));
        ui.horizontal(|ui| {
            let found = self.filtered().len();
            ui.weak(format!("Найдено: {found}"));
            ui.menu_button("Источники", |ui| {
                if ui.button("Папка с картинками…").clicked()
                    && let Some(dir) = crate::plat::FileDialog::new().pick_folder()
                {
                    self.set_textures_dir(dir);
                    ui.close();
                }
                if ui.button("Подключить пак Dungeondraft…").clicked()
                    && let Some(files) = crate::plat::FileDialog::new().add_filter("Пак Dungeondraft", &["dungeondraft_pack"]).pick_files()
                {
                    for f in files {
                        self.add_pack(f);
                    }
                    ui.close();
                }
                for (file, status) in &self.library.packs {
                    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    match status {
                        Ok(0) if self.library.loading() => ui.weak(format!("{name}: читается…")),
                        Ok(n) => ui.weak(format!("{name}: {n}")),
                        Err(e) => ui.colored_label(Color32::from_rgb(230, 110, 90), e),
                    };
                }
            });
        });
        if self.library.assets.is_empty() && !self.library.loading() {
            ui.weak("Пусто. Подключите папку с картинками или пак Dungeondraft: кнопка «Источники».");
        }

        self.filtered();
        let groups = self.groups.clone();
        let cols = ((ui.available_width() + 4.0) / (THUMB + 4.0)).floor().max(1.0) as usize;
        let rows = groups.len().div_ceil(cols);
        let current = match self.browse {
            Kind::Floor => match self.map.materials.get(self.material).map(|m| &m.paint) {
                Some(Paint::Texture(k)) => Some(k.clone()),
                _ => None,
            },
            Kind::Wall => match &self.map.wall_style.paint {
                Paint::Texture(k) => Some(k.clone()),
                _ => None,
            },
            Kind::Door => self.door_asset.clone(),
            Kind::Object => self.object_asset.clone(),
            Kind::Roof => Some(match self.sel {
                Some(Sel::Roof(i)) => self.map.roofs[i].paint.clone(),
                Some(Sel::Outline(i, k)) => self.map.outline_roofs[i].pieces[k].paint.clone(),
                _ => self.roof_style.paint.clone(),
            }),
        };
        let mut chosen: Option<(String, bool)> = None;
        let mut open_family = None;
        egui::ScrollArea::vertical().id_salt("assets").max_height(ui.available_height()).auto_shrink([false, false]).show_rows(ui, THUMB + 4.0, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for group in groups.iter().skip(row * cols).take(cols) {
                        // Видна та из семьи, что выбрана или бралась последней.
                        let family = self.library.assets[group[0]].family.clone();
                        let has = |k: &str| group.iter().copied().find(|&i| self.library.assets[i].key == k);
                        let i = current.as_deref().and_then(has).or_else(|| self.family_pick.get(&family).and_then(|k| has(k))).unwrap_or(group[0]);
                        let asset = &self.library.assets[i];
                        let (key, tip) = (asset.key.clone(), format!("{}\n{}", asset.name, asset.folder));
                        let (rect, resp) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click());
                        if row == 0 && Some(group) == groups.first() {
                            self.first_thumb = rect;
                        }
                        self.thumb(ui.painter(), rect, &key);
                        if group.iter().any(|&g| current.as_deref() == Some(self.library.assets[g].key.as_str())) {
                            ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
                        }
                        if group.len() > 1 {
                            // Уголок с числом вариантов.
                            let c = rect.right_bottom();
                            ui.painter().add(Shape::convex_polygon(vec![c, c - vec2(0.0, 26.0), c - vec2(26.0, 0.0)], ACCENT, Stroke::NONE));
                            ui.painter().text(c - vec2(7.0, 8.0), egui::Align2::CENTER_CENTER, group.len().to_string(), egui::FontId::proportional(11.0), ON_ACCENT);
                        }
                        let resp = resp.on_hover_text(tip);
                        if resp.clicked() {
                            if group.len() > 1 {
                                open_family = Some((group.clone(), rect));
                            } else {
                                chosen = Some((key.clone(), false));
                            }
                        }
                        if matches!(self.browse, Kind::Floor | Kind::Wall) {
                            resp.context_menu(|ui| {
                                if ui.button("Полом").clicked() {
                                    chosen = Some((key.clone(), true));
                                    ui.close();
                                }
                                if ui.button("Стенами").clicked() {
                                    chosen = Some((key.clone(), false));
                                    self.browse = Kind::Wall;
                                    ui.close();
                                }
                            });
                        }
                    }
                });
            }
        });
        if let Some((group, rect)) = open_family {
            let same = self.drawer.as_ref().is_some_and(|d| d.0 == group);
            self.drawer = if same { None } else { Some((group, rect)) };
            self.drawer_frame = self.frame;
        }
        if let Some(key) = self.variant_drawer(ui.ctx()) {
            chosen = Some((key, false));
        }
        if let Some((key, as_floor)) = chosen {
            match self.browse {
                _ if as_floor => self.use_floor_texture(&key),
                Kind::Floor => self.use_floor_texture(&key),
                Kind::Wall => {
                    // Встроенная полоса нарисована под свою толщину — её и ставим.
                    let mut st = self.wall_style();
                    if let Some(width) = library::builtin_wall_width(&key) {
                        st.width = width;
                    } else if library::is_cliff(&key)
                        && let Some(px) = library::dimensions(&self.textures.sources, &key)
                    {
                        st.width = library::cliff_width(px);
                    }
                    st.paint = Paint::Texture(key);
                    self.set_wall_style(st);
                }
                Kind::Door => {
                    // В паках FA косяки, окна и створки лежат вместе: что это — по имени.
                    let lower = key.to_lowercase();
                    let name = library::label(&key).to_lowercase();
                    if lower.contains("frame") || name.contains("косяк") {
                        self.door_frame = Some(key);
                    } else {
                        let window = ["window", "arrowslit", "sill", "окно"].iter().any(|w| lower.contains(w) || name.contains(w));
                        self.door_kind = if window {
                            PortalKind::Window
                        } else if lower.contains("double") {
                            PortalKind::DoubleDoor
                        } else if self.door_kind == PortalKind::DoubleDoor {
                            PortalKind::DoubleDoor
                        } else {
                            PortalKind::Door
                        };
                        // Ширина — из имени у FA (Door_Large_…_2x1); встроенные не меняют её.
                        if !key.starts_with(library::BUILTIN)
                            && let Some(size) = library::size_from_name(&library::label(&key))
                        {
                            self.door_width = size[0].clamp(1.0, 4.0);
                        }
                        self.door_asset = Some(key);
                    }
                    self.set_tool(Tool::Door);
                }
                Kind::Roof => {
                    // Покрытие — выбранной крыше и образцу новых.
                    if let Some(Sel::Roof(i)) = self.sel
                        && self.map.roofs[i].paint != key
                    {
                        self.checkpoint();
                        self.map.roofs[i].paint = key.clone();
                    }
                    // Крыше OSM — только ей: образец новых прямоугольных не трогаем.
                    if let Some(Sel::Outline(i, k)) = self.sel {
                        if self.map.outline_roofs[i].pieces[k].paint != key {
                            self.checkpoint();
                            self.map.outline_roofs[i].pieces[k].paint = key;
                        }
                        if self.tool != Tool::Roof {
                            self.set_tool(Tool::Roof);
                        }
                        return;
                    }
                    self.roof_style.paint = key;
                    if self.tool != Tool::Roof {
                        self.set_tool(Tool::Roof);
                    }
                }
                Kind::Object if self.tool == Tool::Scatter => {
                    // В разбросе щелчок кладёт объект в набор или убирает из него.
                    match self.scatter.set.iter().position(|k| *k == key) {
                        Some(at) => {
                            self.scatter.set.remove(at);
                        }
                        None => self.scatter.set.push(key),
                    }
                }
                Kind::Object => {
                    self.object_asset = Some(key);
                    self.set_tool(Tool::Object);
                }
            }
        }
    }

    pub(super) fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let cell = self.hover.filter(|p| self.canvas.contains(*p)).map(|p| self.cam.cell(self.canvas, p));
            match cell {
                Some((x, y)) if self.map.contains(x, y) => ui.label(format!("Клетка {}, {}", x + 1, y + 1)),
                _ => ui.label("—"),
            };
            ui.separator();
            ui.label(format!("Карта {}×{}", self.map.width, self.map.height));
            ui.separator();
            ui.label(format!("Объектов {}, дверей {}, стен {}", self.map.props.len(), self.map.portals.len(), self.map.walls.len()));
            ui.separator();
            ui.label(format!("Масштаб {:.0}%", self.cam.scale / 64.0 * 100.0));
            if self.recovery.has_copy() {
                ui.separator();
                ui.weak("копия на случай сбоя сделана");
            }
            if let Some(e) = &self.export {
                ui.separator();
                ui.strong(format!("Выгрузка: плитка {} из {}…", (e.next + 1).min(e.tiles.len()), e.tiles.len()));
            } else if let Some(n) = &self.notice {
                ui.separator();
                ui.label(n);
            }
            ui.separator();
            let hint = self.hint();
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate()).on_hover_text(hint);
        });
    }
}
