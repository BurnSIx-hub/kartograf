//! Меню, инструменты и панель настроек над картой.

use super::*;

impl App {
    pub fn set_tool(&mut self, tool: Tool) {
        if tool != Tool::Wall {
            self.finish_wall(false);
        }
        if tool != Tool::Path {
            self.finish_path();
        }
        self.tool = tool;
        if let Some(k) = Group::ALL.iter().position(|g| g.has(tool)) {
            self.group_last[k] = tool;
        }
        // Инструмент сам открывает нужный раздел полки.
        match tool {
            Tool::Door => self.browse = Kind::Door,
            Tool::Roof => self.browse = Kind::Roof,
            Tool::Object | Tool::Scatter => self.browse = Kind::Object,
            Tool::Wall => self.browse = Kind::Wall,
            Tool::Dungeon | Tool::Floor | Tool::Path if self.browse != Kind::Floor => self.browse = Kind::Floor,
            _ => {}
        }
    }

    pub(super) fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Файл", |ui| {
                if item(ui, "Новая карта…", "Ctrl+N") {
                    self.new_map = Some(NewMap { width: self.map.width, height: self.map.height });
                }
                if item(ui, "Открыть…", "Ctrl+O") {
                    self.open();
                }
                if item(ui, "Сохранить", "Ctrl+S") {
                    self.save();
                }
                if item(ui, "Сохранить как…", "Ctrl+Shift+S") {
                    self.save_as();
                }
                ui.separator();
                if item(ui, "Выгрузить для Foundry…", "") {
                    self.export_dialog = Some(ExportDialog { px: EXPORT_PX, name: self.scene_name.clone().unwrap_or_else(|| self.name()), update: false });
                }
                ui.separator();
                if item(ui, "Папка с картинками…", "")
                    && let Some(dir) = crate::plat::FileDialog::new().pick_folder()
                {
                    self.set_textures_dir(dir);
                }
                if item(ui, "Подключить пак Dungeondraft…", "")
                    && let Some(files) = crate::plat::FileDialog::new().add_filter("Пак Dungeondraft", &["dungeondraft_pack"]).pick_files()
                {
                    for f in files {
                        self.add_pack(f);
                    }
                }
                ui.separator();
                if item(ui, "Проверить обновления", "") {
                    let ctx = ui.ctx().clone();
                    self.update.check(&ctx, false);
                }
            });
            ui.menu_button("Процедурно", |ui| {
                for kind in generate::Kind::ALL {
                    if item(ui, &format!("{}…", kind.label()), "") {
                        if kind == generate::Kind::Dungeon {
                            self.material = crate::procgen::dungeon_look(&mut self.map);
                        }
                        let mut o = Options { kind, seed: generate::new_seed(), library: self.settings.gen_library, season: self.gen_season, hills: self.gen_hills, cave: self.gen_cave, district: self.gen_district, dungeon: self.gen_dungeon, ..Default::default() };
                        // Подземелье и киберпанк растут от входа: комнат побольше, колец заметно.
                        if kind.grows() {
                            (o.rooms, o.loops) = (10, 0.4);
                        }
                        self.generator = Some(o);
                    }
                }
                ui.separator();
                if item(ui, "Построить из плана…", "")
                    && let Some(path) = crate::plat::FileDialog::new().add_filter("План карты", &["json"]).pick_file()
                {
                    self.build_from_file(&path);
                }
                // В модуле файл .osm не выбрать (окно браузера знает только паки), а
                // пункт молча ничего не делал; там есть «по координатам».
                #[cfg(not(target_arch = "wasm32"))]
                if item(ui, "Местность из OpenStreetMap…", "")
                    && let Some(path) = crate::plat::FileDialog::new().add_filter("Выгрузка OpenStreetMap", &["osm"]).pick_file()
                {
                    self.import_osm_file(&path);
                }
                if item(ui, "Местность по координатам…", "") {
                    self.place_form.get_or_insert(PlaceForm { text: String::new(), cells: [40, 30], trees: 0.0 });
                }
            });
            ui.menu_button("Биом", |ui| self.biome_ui(ui));
            ui.menu_button("Вид", |ui| {
                if item(ui, "Вся карта", "Home") {
                    self.fitted = false;
                }
                if ui.checkbox(&mut self.settings.snap, "Привязка к сетке   G").changed() {
                    self.save_settings();
                }
                ui.separator();
                ui.label("Крыши");
                let before = self.roof_view;
                ui.selectable_value(&mut self.roof_view, RoofView::Auto, "Видно");
                ui.selectable_value(&mut self.roof_view, RoofView::Ghost, "Сквозь");
                ui.selectable_value(&mut self.roof_view, RoofView::Hidden, "Скрыть").on_hover_text("Только в редакторе: в Foundry крыши выгружаются всегда.");
                if self.roof_view != before {
                    self.settings.roof_view = match self.roof_view {
                        RoofView::Auto => 0,
                        RoofView::Ghost => 1,
                        RoofView::Hidden => 2,
                    };
                    self.save_settings();
                }
                ui.separator();
                self.shadows_ui(ui);
            });
            ui.menu_button("Сочетания клавиш", |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for tool in Tool::ALL {
                    ui.label(format!("{} — {}", tool.key().name(), tool.label()));
                }
                ui.separator();
                for line in ["Колесо — масштаб", "Средняя кнопка или пробел с левой — сдвиг", "R — поворот", "Ctrl+G — объединить выбранное", "Ctrl+Shift с протяжкой — выгнуть стену", "[ и ] или Ctrl с колесом — размер кисти", "Alt с колесом — размер объекта"] {
                    ui.label(line);
                }
            });
            ui.menu_button("Правка", |ui| {
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Отменить").shortcut_text("Ctrl+Z")).clicked() {
                    self.undo();
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Вернуть").shortcut_text("Ctrl+Y")).clicked() {
                    self.redo();
                }
                if ui.add_enabled(self.sel.is_some(), egui::Button::new("Удалить выбранное").shortcut_text("Delete")).clicked() {
                    self.delete_selected();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("В Foundry").clicked() {
                    self.export_dialog = Some(ExportDialog { px: EXPORT_PX, name: self.scene_name.clone().unwrap_or_else(|| self.name()), update: false });
                }
                if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Вернуть")).clicked() {
                    self.redo();
                }
                if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Отменить")).clicked() {
                    self.undo();
                }
            });
        });
    }

    /// Биом для «Wild Canvas»: звук сцены, что летит от крон, погода. Процедурная
    /// карта ставит их сама, нарисованной руками — отсюда. Выбор звука леса
    /// подставляет его листву и погоду, их можно сменить следом.
    pub(super) fn biome_ui(&mut self, ui: &mut egui::Ui) {
        use crate::plan::{PARTICLES, PLACES, WEATHERS, place_defaults};
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.label("Звук места");
        let mut place = self.map.ambience.clone();
        for (key, label) in PLACES {
            ui.selectable_value(&mut place, key.to_owned(), label);
        }
        if place != self.map.ambience {
            self.checkpoint();
            let (particles, weather) = place_defaults(&place);
            self.map.ambience = place;
            self.map.particles = particles.to_owned();
            self.map.weather = weather.to_owned();
        }
        ui.separator();
        ui.label("От крон");
        let mut particles = self.map.particles.clone();
        for (key, label) in PARTICLES {
            ui.selectable_value(&mut particles, key.to_owned(), label);
        }
        if particles != self.map.particles {
            self.checkpoint();
            self.map.particles = particles;
        }
        ui.separator();
        ui.label("Погода");
        let mut weather = self.map.weather.clone();
        for (key, label) in WEATHERS {
            ui.selectable_value(&mut weather, key.to_owned(), label);
        }
        if weather != self.map.weather {
            self.checkpoint();
            self.map.weather = weather;
        }
        ui.separator();
        // Корабль на ходу: вода уходит мимо борта (области воды в «Wild Canvas»).
        ui.label("Течение воды");
        let (mut current, mut drift) = (self.map.current, self.map.drift);
        let r1 = ui.add(egui::Slider::new(&mut current, 0.0..=3.0).step_by(0.1).suffix(" кл/с"));
        let r2 = ui.add_enabled(current > 0.0, egui::Slider::new(&mut drift, 0.0..=355.0).step_by(5.0).suffix("°").text("куда"));
        if r1.drag_started() || r2.drag_started() || ((r1.changed() || r2.changed()) && !r1.dragged() && !r2.dragged()) {
            self.checkpoint();
        }
        (self.map.current, self.map.drift) = (current, drift);
    }

    pub(super) fn swatch(&mut self, ui: &mut egui::Ui, paint: &Paint, size: f32) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
        if ui.is_rect_visible(rect) {
            match paint {
                Paint::Color(c) => {
                    ui.painter().rect_filled(rect, 3.0, rgb(*c));
                }
                Paint::Texture(key) => self.thumb(ui.painter(), rect, key),
            }
        }
        resp
    }

    /// Миниатюра с сохранением пропорций; пока грузится — тёмная плашка.
    pub(super) fn thumb(&mut self, painter: &egui::Painter, rect: Rect, key: &str) {
        painter.rect_filled(rect, 3.0, Color32::from_gray(45));
        if let Some((tex, px)) = self.textures.get(key, true) {
            let aspect = px[0] as f32 / px[1].max(1) as f32;
            let size = if aspect >= 1.0 { vec2(rect.width(), rect.width() / aspect) } else { vec2(rect.height() * aspect, rect.height()) };
            painter.image(tex.id(), Rect::from_center_size(rect.center(), size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if self.textures.failed(key, true) {
            painter.rect_filled(rect.shrink(rect.width() * 0.3), 2.0, Color32::from_rgb(160, 40, 40));
        }
    }

    /// Подсказка к инструменту — в строке состояния.
    pub(super) fn hint(&self) -> &'static str {
        match self.tool {
            Tool::Select => "Щелчок выбирает объект, дверь или стену. Объект можно тащить, за углы — менять размер, за ручку сверху — поворачивать; Alt с колесом — размер; стрелки сдвигают. Delete удаляет, R поворачивает.",
            Tool::Dungeon => "Пол комнат: стены встают по краю сами. Ctrl+Shift с протяжкой выгибает стену дугой.",
            Tool::Floor => "Пол без стен: двор, трава, площадь.",
            Tool::Erase => "Стирает и комнаты, и местность.",
            Tool::Wall => "Щелчки ставят вершины. Щелчок по первой замыкает; двойной щелчок, Enter или правая кнопка — заканчивают. Ctrl+Shift с протяжкой выгибает отрезок дугой.",
            Tool::Door => "Щелчок по стене ставит проём. Картинку двери или окна выберите на полке; без картинки — простая плашка.",
            Tool::Roof => "Протяжка рамкой по стенам дома ставит крышу; протяжка за крышу двигает её. R — конёк поперёк, у односкатной — куда скат. Покрытие — на полке. С другими инструментами крыши полупрозрачны.",
            Tool::Text => "Щелчок по пустому месту ставит подпись, по подписи — выбирает её; протяжка двигает. Текст, размер и цвет — в строке над картой.",
            Tool::Shade => "Темнит пол: углы, ниши, под навесом. Ступени — как тени-узоры FA: повторный мазок не темнее ступени. Alt — стирать.",
            Tool::Relief => "Ярусы по 5 футов. Жёсткая кисть поднимает плато с обрывом — полоса скалы, в Foundry стена; мягкая — пологий склон уступами-гребнями. Нижние ярусы темнее. «Выровнять» тянет к ярусу, с которого начат мазок. Alt — наоборот.",
            Tool::Path => "Щелчки ставят точки — линия через них плавная; протяжка рисует от руки. Enter, двойной щелчок или правая кнопка — закончить. Материал — в строке над картой: земля — тропа, вода — ручей, ткань — ковёр.",
            Tool::Scatter => "Ведите кистью — объекты из набора ложатся вразнобой. Alt — стирать их. Ctrl с колесом — размер кисти. Объекты в набор — щелчком на полке.",
            Tool::Object => "Щелчок ставит объект, протяжка двигает уже стоящий. Alt — на время наоборот с привязкой, Alt с колесом — размер, R — поворот.",
        }
    }

    /// Левый столбец: семь ячеек на всю высоту. Ячейка — инструмент, что был в ней
    /// последним; стрелка или правая кнопка открывают её режимы.
    pub(super) fn tool_cells(&mut self, ui: &mut egui::Ui) {
        let h = (ui.available_height() / Group::ALL.len() as f32).floor();
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        for (k, group) in Group::ALL.into_iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
            let on = group.has(self.tool);
            let tool = if on { self.tool } else { self.group_last[k] };
            let modes = self.group_modes(group);
            let arrow = Rect::from_min_max(pos2(rect.right() - 16.0, rect.top()), rect.right_bottom());
            let fly = !modes.is_empty() && ui.interact(arrow, resp.id.with("arrow"), Sense::click()).clicked();
            let open = fly || (!modes.is_empty() && resp.secondary_clicked());
            if resp.clicked() {
                self.set_tool(tool);
            }
            if open {
                self.fly = if self.fly == Some(k) { None } else { Some(k) };
                self.fly_rect = rect;
                self.fly_frame = self.frame;
            }
            let (fill, ink) = if on {
                (ACCENT, ON_ACCENT)
            } else if resp.hovered() {
                (Color32::from_rgb(48, 50, 55), TEXT)
            } else {
                (PANEL, MUTED)
            };
            let p = ui.painter();
            p.rect_filled(rect, 0.0, fill);
            p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, LINE));
            let icon = Rect::from_center_size(pos2(rect.center().x - 3.0, rect.center().y - 8.0), vec2(24.0, 24.0));
            tool_icon(p, icon, group, ink);
            let label = if on { self.mode_label() } else { tool.label().to_owned() };
            p.text(pos2(rect.center().x - 3.0, rect.center().y + 14.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), ink);
            if !modes.is_empty() {
                let c = arrow.center();
                p.add(Shape::convex_polygon(vec![c + vec2(-3.0, -5.0), c + vec2(4.0, 0.0), c + vec2(-3.0, 5.0)], ink, Stroke::NONE));
            }
            resp.on_hover_text(format!("{}   {}", tool.label(), tool.key().name()));
        }
    }

    /// Подпись ячейки выбранного инструмента.
    pub(super) fn mode_label(&self) -> String {
        match self.tool {
            Tool::Door => self.door_kind.label().to_owned(),
            Tool::Roof => self.roof_style.shape.label().to_owned(),
            t => t.label().to_owned(),
        }
    }

    pub(super) fn group_modes(&self, group: Group) -> Vec<(String, Mode)> {
        let tool = |t: Tool| (format!("{}   {}", t.label(), t.key().name()), Mode::Tool(t));
        match group {
            Group::Brush => [Tool::Dungeon, Tool::Floor, Tool::Shade, Tool::Relief, Tool::Erase].map(tool).to_vec(),
            Group::Object => [Tool::Object, Tool::Scatter].map(tool).to_vec(),
            Group::Wall => std::iter::once(tool(Tool::Wall)).chain(PortalKind::ALL.map(|k| (k.label().to_owned(), Mode::Door(k)))).collect(),
            Group::Roof => map::RoofShape::ALL.map(|s| (s.label().to_owned(), Mode::Roof(s))).to_vec(),
            _ => Vec::new(),
        }
    }

    /// Всплывающий список режимов рядом с ячейкой.
    pub(super) fn tool_flyout(&mut self, ctx: &egui::Context) {
        let Some(k) = self.fly else { return };
        let group = Group::ALL[k];
        let modes = self.group_modes(group);
        let mut chosen = None;
        let at = egui::Area::new(egui::Id::new("tool-flyout")).order(egui::Order::Foreground).fixed_pos(self.fly_rect.right_top() + vec2(2.0, 0.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for (i, (label, mode)) in modes.iter().enumerate() {
                    let on = match *mode {
                        Mode::Tool(t) => self.tool == t,
                        Mode::Door(d) => self.tool == Tool::Door && self.door_kind == d,
                        Mode::Roof(s) => self.tool == Tool::Roof && self.roof_style.shape == s,
                    };
                    if ui.selectable_label(on, label).clicked() {
                        chosen = Some(i);
                    }
                }
            });
        });
        if let Some(i) = chosen {
            match modes[i].1 {
                Mode::Tool(t) => self.set_tool(t),
                Mode::Door(d) => {
                    self.set_door_kind(d);
                    self.set_tool(Tool::Door);
                }
                Mode::Roof(s) => {
                    self.roof_style.shape = s;
                    self.set_tool(Tool::Roof);
                }
            }
            self.fly = None;
        } else if self.fly_frame != self.frame && at.response.clicked_elsewhere() {
            self.fly = None;
        }
    }

    /// Этажи и этажность — плашкой в углу карты.
    pub(super) fn levels_overlay(&mut self, ctx: &egui::Context) {
        let at = self.canvas.right_top() + vec2(-8.0, 8.0);
        egui::Area::new(egui::Id::new("levels")).pivot(egui::Align2::RIGHT_TOP).fixed_pos(at).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                for k in (0..self.levels.len()).rev() {
                    if ui.selectable_label(self.level == k, self.level_name(k)).clicked() {
                        self.switch_level(k);
                    }
                }
                ui.menu_button("Этажи", |ui| {
                    if ui.button("Добавить выше").clicked() {
                        self.add_level(true);
                        ui.close();
                    }
                    if ui.button("Добавить ниже").clicked() {
                        self.add_level(false);
                        ui.close();
                    }
                    let mut name = self.map.name.clone();
                    let hint = format!("Этаж {}", self.level + 1);
                    let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(hint).desired_width(120.0));
                    if r.gained_focus() {
                        self.checkpoint();
                    }
                    self.map.name = name;
                    if self.levels.len() > 1 && ui.button("Удалить этот этаж").on_hover_text("Отменить нельзя").clicked() {
                        self.remove_level();
                        ui.close();
                    }
                });
            });
        });
    }

    pub(super) fn material_combo(&mut self, ui: &mut egui::Ui) {
        let materials = self.map.materials.clone();
        if let Some(m) = materials.get(self.material) {
            self.swatch(ui, &m.paint, 18.0);
        }
        let shown = materials.get(self.material).map_or_else(String::new, |m| m.name.clone());
        egui::ComboBox::from_id_salt("material").selected_text(shown).width(120.0).show_ui(ui, |ui| {
            for (i, m) in materials.iter().enumerate() {
                ui.horizontal(|ui| {
                    let resp = self.swatch(ui, &m.paint, 18.0);
                    let label = ui.selectable_label(self.material == i, &m.name);
                    if resp.clicked() || label.clicked() {
                        self.material = i;
                        self.recolor_path();
                    }
                });
            }
        });
    }

    /// Выбор пола для кисти и дороги.
    pub(super) fn material_picker(&mut self, ui: &mut egui::Ui) {
        self.material_combo(ui);
        let materials = self.map.materials.clone();
        if let Some(m) = materials.get(self.material).filter(|m| matches!(m.paint, Paint::Texture(_))) {
            ui.horizontal(|ui| {
                ui.label("Живой:").on_hover_text("Залитое этим материалом в выгрузке станет областью сцены, и модуль «Wild Canvas» её оживит: рябь и рыбки, течение лавы, пузыри болота.");
                let now = m.live;
                let shown = if now == map::Live::Auto { format!("{} ({})", now.label(), m.live().label()) } else { now.label().to_owned() };
                egui::ComboBox::from_id_salt("live").selected_text(shown).show_ui(ui, |ui| {
                    for c in map::Live::CHOICES {
                        if ui.selectable_label(now == c, c.label()).clicked() && now != c {
                            self.checkpoint();
                            self.map.materials[self.material].live = c;
                        }
                    }
                });
            });
            let mut vary = m.vary;
            if ui
                .checkbox(&mut vary, "Разнообразие")
                .on_hover_text("Плитка перемешивается кусками со сдвигом и поворотом на пол-оборота — повтор не бросается в глаза. Для травы, земли, воды, снега; доски и кладку ломает.")
                .changed()
            {
                self.checkpoint();
                self.map.materials[self.material].vary = vary;
            }
        }

    }

    /// Ползунки правят выбранную дорогу, а без неё — образец для новых.
    pub(super) fn path_panel(&mut self, ui: &mut egui::Ui) {
        let chosen = match self.sel {
            Some(Sel::Path(i)) => Some(i),
            _ => None,
        };
        let mut st = chosen.map_or_else(|| self.path_style.clone(), |i| self.map.paths[i].clone());
        let rs = [
            ui.add(egui::Slider::new(&mut st.width, 0.25..=12.0).logarithmic(true).text("ширина, кл.")),
            ui.add(egui::Slider::new(&mut st.soft, 0.0..=1.0).text("мягкость края")),
            ui.add(egui::Slider::new(&mut st.rough, 0.0..=1.0).text("неровность")),
        ];
        let along = ui.checkbox(&mut st.along, "Текстура вдоль линии").on_hover_text("Для ковра и мостовой. Без галочки текстура лежит по сетке, как пол, — для земли и воды.");
        match chosen {
            Some(i) => {
                if rs.iter().any(|r| r.drag_started()) || along.changed() || rs.iter().any(|r| r.changed() && !r.dragged()) {
                    self.checkpoint();
                }
                self.map.paths[i] = st;
            }
            None => {
                st.points.clear();
                self.path_style = st;
            }
        }
    }

    /// Строка над картой: настройки инструмента и выбранного. Высота постоянная:
    /// иначе карта прыгала бы под курсором; что не влезло — прокруткой вбок.
    /// Выбранный кусок крыши OSM: вид, поворот, покрытие (на полке). Импорт
    /// ошибается — нарисовал скатную вместо плоской, — правится тут.
    pub(super) fn outline_panel(&mut self, ui: &mut egui::Ui, i: usize, k: usize) {
        let mut st = self.map.outline_roofs[i].pieces[k].clone();
        ui.strong(if k == 0 { "Крыша дома из OSM" } else { "Часть крыши из OSM" });
        let was = st.form;
        ui.horizontal_wrapped(|ui| {
            for form in map::RoofForm::ALL {
                ui.selectable_value(&mut st.form, form, form.label());
            }
        });
        // Только при смене вида: выбор крыши сам её не меняет.
        if st.form != was {
            st.fit_form();
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(st.form.turns(), egui::Button::new("Повернуть   R")).clicked() {
                st.turn();
            }
            ui.label(format!("Покрытие: {}", library::label(&st.paint)));
        });
        ui.weak(if k == 0 { "Delete — убрать крышу дома с частями" } else { "Delete — убрать эту часть" });
        if st != self.map.outline_roofs[i].pieces[k] {
            self.checkpoint();
            self.map.outline_roofs[i].pieces[k] = st;
        }
    }

    pub(super) fn options_bar(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().id_salt("options").auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().slider_width = 120.0;
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                ui.add_space(4.0);
                ui.heading(self.tool.label());
                ui.separator();
                // Выбранная «Выбором» дорога или вода правится тут же: материал, «живой», ширина.
                let path_sel = self.tool == Tool::Select && self.editing_path();
                if let Some(Sel::Path(i)) = self.sel
                    && path_sel
                {
                    ui.strong("Дорога или вода");
                    self.material = self.map.paths[i].material.min(self.map.materials.len().saturating_sub(1));
                }
                if matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Path) || path_sel {
                    self.material_picker(ui);
                }
                match self.tool {
                    _ if path_sel => self.path_panel(ui),
                    Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade | Tool::Relief => {
                        if self.tool == Tool::Relief {
                            ui.horizontal_wrapped(|ui| {
                                for mode in ReliefMode::ALL {
                                    ui.selectable_value(&mut self.relief_mode, mode, mode.label());
                                }
                            });
                            if matches!(self.relief_mode, ReliefMode::Raise | ReliefMode::Lower) {
                                let ft = (self.relief_height * 5.0).round();
                                ui.add(egui::Slider::new(&mut self.relief_height, 1.0..=8.0).step_by(1.0).text(format!("ярусов ({ft} фт)")));
                            }
                            if let (Some(r), Some(h)) = (&self.map.relief, self.hover) {
                                let lv = r.level_at(self.cam.to_world(self.canvas, h));
                                ui.weak(format!("Под курсором: ярус {lv} ({} фт)", lv * 5));
                            }
                        }
                        if self.tool == Tool::Shade {
                            ui.horizontal(|ui| {
                                for level in [0.2, 0.4, 0.6, 0.8] {
                                    ui.selectable_value(&mut self.shade_level, level, format!("{}%", (level * 100.0) as u32));
                                }
                            });
                            ui.add(egui::Slider::new(&mut self.shade_level, 0.05..=1.0).text("темнота"));
                            ui.add(egui::Slider::new(&mut self.shade_soft, 0.0..=1.0).text("затухание"));
                        }
                        ui.horizontal_wrapped(|ui| {
                            for shape in Tip::ALL {
                                ui.selectable_value(&mut self.brush.shape, shape, shape.label());
                            }
                        });
                        if !matches!(self.brush.shape, Tip::Cells | Tip::Oval) {
                            ui.add(egui::Slider::new(&mut self.brush.radius, 0.25..=10.0).logarithmic(true).text("радиус, кл."));
                            if self.tool != Tool::Shade {
                                ui.add(egui::Slider::new(&mut self.brush.hardness, 0.0..=1.0).text("жёсткость"));
                            }
                            ui.add(egui::Slider::new(&mut self.brush.flow, 0.05..=1.0).text("сила"));
                        }
                    }
                    Tool::Door => {
                        ui.horizontal_wrapped(|ui| {
                            for kind in PortalKind::ALL {
                                if ui.selectable_label(self.door_kind == kind, kind.label()).clicked() {
                                    self.set_door_kind(kind);
                                }
                            }
                        });
                        ui.add(egui::Slider::new(&mut self.door_width, 1.0..=4.0).step_by(0.5).text("ширина, кл."));
                        if !matches!(self.door_kind, PortalKind::Opening | PortalKind::Secret) {
                            ui.horizontal(|ui| {
                                ui.label(if self.door_kind == PortalKind::Window { "Окно:" } else { "Створка:" });
                                match self.door_asset.clone() {
                                    Some(key) => {
                                        ui.label(library::label(&key));
                                    }
                                    None => {
                                        ui.weak("простая плашка");
                                    }
                                }
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Косяк:");
                            let b = library::BUILTIN;
                            let style = |f: &Option<String>| f.as_deref().map(|k| k.trim_end_matches(char::is_numeric).to_owned());
                            let now = style(&self.door_frame);
                            for (label, key) in [("нет", None), ("дерево", Some(format!("{b}frame_wood_1"))), ("камень", Some(format!("{b}frame_stone_1")))] {
                                if ui.selectable_label(now == style(&key), label).clicked() {
                                    self.door_frame = key;
                                }
                            }
                            if let Some(k) = &self.door_frame
                                && !k.starts_with(b)
                            {
                                ui.label(library::label(k));
                            }
                        });
                    }
                    Tool::Roof if matches!(self.sel, Some(Sel::Outline(..))) => {
                        if let Some(Sel::Outline(i, k)) = self.sel {
                            self.outline_panel(ui, i, k);
                        }
                    }
                    Tool::Roof => {
                        let chosen = match self.sel {
                            Some(Sel::Roof(i)) => Some(i),
                            _ => None,
                        };
                        let mut st = chosen.map_or_else(|| self.roof_style.clone(), |i| self.map.roofs[i].clone());
                        let before = st.clone();
                        ui.horizontal_wrapped(|ui| {
                            for shape in map::RoofShape::ALL {
                                ui.selectable_value(&mut st.shape, shape, shape.label());
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Повернуть   R").clicked() {
                                st.turn = (st.turn + 1) % 4;
                            }
                            ui.label(format!("Покрытие: {}", library::label(&st.paint)));
                        });
                        let over = ui.add(egui::Slider::new(&mut st.overhang, 0.0..=0.75).text("свес, кл."));
                        match chosen {
                            Some(i) => {
                                if st != before && (over.drag_started() || !over.dragged()) {
                                    self.checkpoint();
                                }
                                self.map.roofs[i] = st;
                            }
                            None => self.roof_style = st,
                        }
                    }
                    Tool::Text => {
                        let chosen = match self.sel {
                            Some(Sel::Label(i)) => Some(i),
                            _ => None,
                        };
                        let mut st = chosen.map_or_else(|| self.label_style.clone(), |i| self.map.labels[i].clone());
                        let text = ui.add(egui::TextEdit::singleline(&mut st.text).desired_width(180.0));
                        let rs = [
                            ui.add(egui::Slider::new(&mut st.size, 0.2..=4.0).logarithmic(true).text("высота, кл.")),
                            ui.add(egui::Slider::new(&mut st.rot, -180.0..=180.0).step_by(5.0).text("поворот")),
                        ];
                        let mut color = Color32::from_rgb(st.color[0], st.color[1], st.color[2]);
                        let picked = ui.horizontal(|ui| {
                            let r = ui.color_edit_button_srgba(&mut color);
                            ui.label("цвет");
                            r
                        });
                        st.color = [color.r(), color.g(), color.b()];
                        let gm = ui.checkbox(&mut st.gm, "только ведущему").on_hover_text("Видна здесь, но не попадает в выгрузку для игроков");
                        match chosen {
                            Some(i) => {
                                if text.gained_focus() || rs.iter().any(|r| r.drag_started() || (r.changed() && !r.dragged())) || picked.inner.changed() || gm.changed() {
                                    self.checkpoint();
                                }
                                self.map.labels[i] = st;
                            }
                            None => self.label_style = st,
                        }
                    }
                    Tool::Path => self.path_panel(ui),
                    Tool::Scatter => {
                        if self.scatter.set.is_empty() {
                            ui.weak("Набор пуст: щёлкните объекты на полке");
                        }
                        let mut drop = None;
                        for (k, key) in self.scatter.set.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.small_button("×").on_hover_text("Убрать из набора").clicked() {
                                    drop = Some(k);
                                }
                                ui.label(library::label(key));
                            });
                        }
                        if let Some(k) = drop {
                            self.scatter.set.remove(k);
                        }
                        ui.add(egui::Slider::new(&mut self.scatter.radius, 0.25..=10.0).logarithmic(true).text("радиус, кл."));
                        ui.add(egui::Slider::new(&mut self.scatter.density, 0.05..=4.0).logarithmic(true).text("плотность"));
                        ui.add(egui::Slider::new(&mut self.scatter.spacing, 0.0..=2.0).text("просвет"));
                        ui.add(egui::Slider::new(&mut self.scatter.scale[0], 0.2..=3.0).text("размер от"));
                        ui.add(egui::Slider::new(&mut self.scatter.scale[1], 0.2..=3.0).text("размер до"));
                        if self.scatter.scale[1] < self.scatter.scale[0] {
                            self.scatter.scale[1] = self.scatter.scale[0];
                        }
                        ui.checkbox(&mut self.scatter.spin, "Случайный поворот");
                    }
                    Tool::Object => {
                        ui.add(egui::Slider::new(&mut self.object_rot, 0.0..=345.0).step_by(15.0).text("поворот"));
                        ui.add(egui::Slider::new(&mut self.object_scale, SCALE_RANGE).logarithmic(true).text("размер"));
                    }
                    _ => {}
                }
                if matches!(self.tool, Tool::Object | Tool::Select) && ui.checkbox(&mut self.settings.snap, "Привязка к сетке   G").changed() {
                    self.save_settings();
                }
                self.picked.retain(|&k| k < self.map.props.len());
                if !self.picked.is_empty() {
                    ui.strong(format!("Выбрано объектов: {}", self.picked.len()));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.picked.len() >= 2, egui::Button::new("Объединить в один   Ctrl+G")).clicked() {
                            self.bake_picked();
                        }
                        if ui.small_button("снять выбор").clicked() {
                            self.picked.clear();
                        }
                    });
                }
                if let Some(Sel::Prop(i)) = self.sel {
                    ui.strong(format!("Объект: {}", library::label(&self.map.props[i].asset)));
                    let (mut rot, mut scale) = (self.map.props[i].rot, self.map.props[i].scale);
                    let r1 = ui.add(egui::Slider::new(&mut rot, 0.0..=359.0).text("поворот"));
                    let r2 = ui.add(egui::Slider::new(&mut scale, SCALE_RANGE).logarithmic(true).text("размер"));
                    if r1.drag_started() || r2.drag_started() || ((r1.changed() || r2.changed()) && !r1.dragged() && !r2.dragged()) {
                        self.checkpoint();
                    }
                    self.map.props[i].rot = rot;
                    self.map.props[i].scale = scale;
                    let mut casts = self.map.props[i].casts_shadow();
                    if ui.checkbox(&mut casts, "Отбрасывает тень").changed() {
                        self.checkpoint();
                        self.map.props[i].shadow = Some(casts);
                    }
                    // Что уйдёт в Foundry: свет и крона. «Авто» — по имени, как у встроенных.
                    let by_name = map::glow_by_name(&self.map.props[i].asset);
                    let mut glow = self.map.props[i].light;
                    let shown = match glow {
                        None => format!("авто: {}", by_name.map_or("нет", map::Glow::label)),
                        Some(g) => g.label().to_owned(),
                    };
                    ui.label("Свет:");
                    egui::ComboBox::from_id_salt("prop_light").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut glow, None, "авто по имени");
                        for g in map::Glow::CHOICES {
                            ui.selectable_value(&mut glow, Some(g), g.label());
                        }
                    });
                    if glow != self.map.props[i].light {
                        self.checkpoint();
                        self.map.props[i].light = glow;
                    }
                    let by_name = map::layer_by_name(&self.map.props[i].asset);
                    let mut layer = self.map.props[i].layer;
                    let shown = match layer {
                        None => format!("авто: {}", map::layer_label(by_name)),
                        Some(l) => map::layer_label(l),
                    };
                    ui.label("Слой:").on_hover_text("Кто кого накрывает, как в Dungeondraft. Внутри слоя выше поставленный позже.");
                    egui::ComboBox::from_id_salt("prop_layer").selected_text(shown).show_ui(ui, |ui| {
                        ui.selectable_value(&mut layer, None, "авто по имени");
                        for (l, name) in map::LAYERS {
                            ui.selectable_value(&mut layer, Some(l), name);
                        }
                    });
                    if layer != self.map.props[i].layer {
                        self.checkpoint();
                        self.map.props[i].layer = layer;
                    }
                    let mut canopy = self.map.props[i].is_canopy();
                    if ui.checkbox(&mut canopy, "Крона над токенами").on_hover_text("В Foundry уходит тайлом выше токенов, в картинке карты на его месте пень. Авто — по имени: деревья.").changed() {
                        self.checkpoint();
                        self.map.props[i].canopy = Some(canopy);
                    }
                }

                if self.tool == Tool::Wall || matches!(self.sel, Some(Sel::Wall(_) | Sel::Edge(..))) {
                    let own = match self.sel {
                        Some(Sel::Wall(i)) => Some(self.map.walls[i].style.is_some()),
                        Some(Sel::Edge(..)) => Some(false),
                        _ => None,
                    };
                    ui.horizontal(|ui| {
                        if own.is_some() {
                            ui.strong("Выбранная стена");
                        }
                        if own == Some(true) && ui.small_button("как у всех").on_hover_text("Убрать свой вид — стена станет как остальные").clicked()
                            && let Some(Sel::Wall(i)) = self.sel
                        {
                            self.checkpoint();
                            self.map.walls[i].style = None;
                        }
                    });
                    if let Some(Sel::Edge(key, whole)) = self.sel {
                            let mut w = whole;
                            ui.selectable_value(&mut w, false, "Участок");
                            ui.selectable_value(&mut w, true, "Весь контур");
                            self.sel = Some(Sel::Edge(key, w));
                        } else if self.sel.is_none() && ui.button("Ко всем стенам").on_hover_text("Перекрасить уже стоящие стены в этот вид").clicked() {
                            self.apply_wall_style_to_all();
                        }
                    let current = self.wall_style();
                    ui.horizontal(|ui| {
                        self.swatch(ui, &current.paint, 18.0);
                        ui.label(match &current.paint {
                            Paint::Color(_) => "цвет".to_owned(),
                            Paint::Texture(k) => library::label(k),
                        });
                        if matches!(current.paint, Paint::Texture(_)) && ui.small_button("цвет").clicked() {
                            self.use_wall_paint(map::WallStyle::default().paint);
                        }
                    });
                    let mut width = current.width;
                    let slider = ui.add(egui::Slider::new(&mut width, 0.1..=1.0).text("толщина"));
                    // Протяжка ползунка — одна правка в истории, а не по одной на кадр.
                    if slider.drag_started() || (slider.changed() && !slider.dragged()) {
                        self.checkpoint();
                    }
                    if width != current.width {
                        self.put_wall_style(map::WallStyle { width, ..current });
                    }

                }
            });
        });
    }

    /// Настройки теней карты, меню «Вид».
    pub(super) fn shadows_ui(&mut self, ui: &mut egui::Ui) {
        let mut sh = self.map.shadows.clone();
        let on = ui.checkbox(&mut sh.on, "Тени от стен и объектов");
        let mut rs = Vec::new();
        if sh.on {
            rs.push(ui.add(egui::Slider::new(&mut sh.angle, 0.0..=345.0).step_by(15.0).text("куда падают, °")));
            rs.push(ui.add(egui::Slider::new(&mut sh.dist, 0.0..=3.0).text("длина, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.blur, 0.1..=1.0).text("размытие, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.opacity, 0.05..=1.0).text("темнота")));
            rs.push(ui.add(egui::Slider::new(&mut sh.fade, 0.0..=1.0).text("затухание")));
            rs.push(ui.add(egui::Slider::new(&mut sh.ao, 0.0..=1.0).text("у подножия стен")));
        }
        if on.changed() || rs.iter().any(|r| r.drag_started() || (r.changed() && !r.dragged())) {
            self.checkpoint();
        }
        self.map.shadows = sh;

    }
}
