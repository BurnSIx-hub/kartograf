//! Постройка: генераторы, план, местность из OSM и по координатам.

use super::*;

impl App {
    /// Новая карта вместо нынешней, отменяется как обычная правка.
    pub fn generate(&mut self, opt: &Options) -> usize {
        let plan = Plan::random(opt, [self.map.width, self.map.height]);
        self.build_plan(&plan, false).map_or(0, |r| r.count)
    }

    /// Карта по плану: заменяет текущий этаж, отменяется Ctrl+Z. План другого
    /// размера пересоздаёт карту. `fresh_look` — вид подземелья по умолчанию, как
    /// при открытии окна генератора (из окна вид уже выбран). Ошибка плана — в окно
    /// ошибки, что не удалось из задуманного — в строку состояния.
    pub fn build_plan(&mut self, plan: &Plan, fresh_look: bool) -> Option<procgen::Report> {
        let size = plan.map_size([self.map.width, self.map.height]);
        if let Err(e) = plan.validate(size) {
            self.error = Some(format!("План не годится:\n{e}"));
            return None;
        }
        if size != [self.map.width, self.map.height] && self.levels.len() > 1 {
            self.error = Some(format!("План рассчитан на карту {}×{}, а у этой карты несколько этажей размером {}×{}", size[0], size[1], self.map.width, self.map.height));
            return None;
        }
        self.checkpoint();
        self.sel = None;
        if size != [self.map.width, self.map.height] {
            let mut fresh = Map::new(size[0], size[1]);
            fresh.materials = self.map.materials.clone();
            fresh.wall_style = self.map.wall_style.clone();
            self.map = fresh;
            self.fitted = false;
        }
        if fresh_look && plan.kind == generate::Kind::Dungeon {
            self.material = procgen::dungeon_look(&mut self.map);
        }
        // Встроенный набор (киберпанк) — только своей вкладке: в обычные карты его
        // ассеты не попадают. Киберпанку свой набор есть всегда, а паки — лишь
        // «Из библиотеки» (железные двери и косяки FA).
        let cyber = plan.kind == generate::Kind::Cyber;
        let library_mode = plan.assets == crate::plan::Assets::Library;
        let palette = self.generator_palette(library_mode, cyber);
        match procgen::build(&mut self.map, plan, self.material, palette) {
            Ok(r) => {
                // Сид пустого плана случайный и наружу не выходит — тогда карта без сида.
                self.map.seed = plan.seed.clone();
                // Время года — у всего, что под открытым небом, кроме пещеры и подземелья.
                let outdoor = matches!(plan.kind, generate::Kind::Forest | generate::Kind::Town | generate::Kind::Walls | generate::Kind::House | generate::Kind::Tavern | generate::Kind::Arena | generate::Kind::Church | generate::Kind::Jarl);
                self.map.particles = match plan.kind {
                    _ if outdoor => plan.options.season.particles(),
                    generate::Kind::Cave => plan.options.cave.particles(),
                    _ => "",
                }
                .to_owned();
                self.map.weather = if outdoor { plan.options.season.weather().to_owned() } else { String::new() };
                self.map.ambience = plan.kind.ambience(&plan.options).to_owned();
                self.map.indoor = plan.kind.indoor().to_owned();
                self.notice = (!r.warnings.is_empty()).then(|| format!("Построено, но: {}", r.warnings.join("; ")));
                self.edges_dirty = true;
                Some(r)
            }
            Err(e) => {
                self.undo();
                self.error = Some(format!("План не построился:\n{e}"));
                None
            }
        }
    }

    /// Подбор ассетов для генератора: свои из библиотеки (`library_mode`) и набор
    /// киберпанка (`cyber`), иначе встроенные.
    pub(super) fn generator_palette(&self, library_mode: bool, cyber: bool) -> Palette {
        if !library_mode && !cyber {
            return Palette::builtin();
        }
        let assets: Vec<library::Asset> = self
            .library
            .assets
            .iter()
            .filter(|a| match library::set_of(&a.key) {
                Some(set) => cyber && set == "cyberpunk",
                None => library_mode,
            })
            .cloned()
            .collect();
        let (sources, seen, dense, strung) = (self.textures.sources.clone(), self.textures.sources.clone(), self.textures.sources.clone(), self.textures.sources.clone());
        Palette::from_library(&assets, Box::new(move |key| library::dimensions(&sources, key))).with_visible(Box::new(move |key| library::visible_box(&seen, key)))
            .with_ends(Box::new(move |key| library::strand_ends(&strung, key)))
            .with_opacity(Box::new(move |key| library::opacity(&dense, key)))
    }

    /// «Построить из плана…»: JSON-файл плана (см. `plans/README.md`).
    pub fn build_from_file(&mut self, path: &Path) -> bool {
        let plan = match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Plan::from_json(&t)) {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                return false;
            }
        };
        let Some(r) = self.build_plan(&plan, true) else { return false };
        if self.notice.is_none() {
            let name = if plan.name.is_empty() { path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { plan.name.clone() };
            self.notice = Some(format!("Построено по плану «{name}»: {} {}", r.count, match plan.kind {
                generate::Kind::Dungeon | generate::Kind::Cyber | generate::Kind::House | generate::Kind::Tavern => "комнат",
                generate::Kind::Town => "домов",
                generate::Kind::Forest => "деревьев",
                generate::Kind::Cave => "клеток",
                generate::Kind::Arena => "рядов",
                generate::Kind::Church => "частей",
                generate::Kind::Walls => "построек",
                generate::Kind::Jarl => "комнат",
            }));
        }
        true
    }

    /// «Местность из OpenStreetMap…»: выгрузка `.osm` заменяет карту целиком,
    /// неполные в ней природные контуры докачиваются из API OSM (`osm_relations`),
    /// размер — по участку. Ассеты — как у генератора: из библиотеки, если там
    /// отмечено «из библиотеки». Отменяется Ctrl+Z.
    pub fn import_osm_file(&mut self, path: &Path) -> bool {
        if self.levels.len() > 1 {
            self.error = Some("Местность ставится на карту с одним этажом, а у этой их несколько".into());
            return false;
        }
        let palette = self.generator_palette(self.settings.gen_library, false);
        // Подсказка со снимка (`tools/osm/cover.py`) — рядом, `<имя>.cover.png`.
        let cover_path = path.with_extension("cover.png");
        let cover = std::fs::read(&cover_path).ok().map(|b| procgen::osm::Cover::from_image(&b));
        // Кроны из анализа снимка (`tools/osm/trees.py`) — `<имя>.crowns.json`.
        let crowns_path = path.with_extension("crowns.json");
        let cover = cover.map(|c| c.and_then(|c| match std::fs::read_to_string(&crowns_path) {
            Ok(t) => c.with_crowns(&t).map_err(|e| format!("{}: {e}", crowns_path.display())),
            Err(_) => Ok(c),
        }));
        let cover = match cover {
            Some(Err(e)) => {
                self.error = Some(if e.contains("кроны") { e } else { format!("{}: {e}", cover_path.display()) });
                return false;
            }
            Some(Ok(c)) => Some(c),
            None => None,
        };
        let built = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| {
            let extra = osm_relations(&procgen::osm::incomplete(&t));
            procgen::import_osm(&t, &extra, cover.as_ref(), procgen::osm::Options::default(), palette)
        });
        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.take_terrain(built.map_err(|e| format!("{}: {e}", path.display())), &name)
    }

    /// Положить собранную местность на карту (с отменой) и сказать, что вышло.
    pub(super) fn take_terrain(&mut self, built: Result<(Map, procgen::Report), String>, name: &str) -> bool {
        let (map, r) = match built {
            Ok(x) => x,
            Err(e) => {
                self.error = Some(e);
                return false;
            }
        };
        self.checkpoint();
        self.sel = None;
        self.map = map;
        self.fitted = false;
        self.edges_dirty = true;
        self.notice = Some(if r.warnings.is_empty() { format!("Местность «{name}»: домов {}", r.count) } else { format!("Местность «{name}»: домов {}; {}", r.count, r.warnings.join("; ")) });
        true
    }

    /// Местность по координатам: запустить сборку в фоне, принять её ход и, когда
    /// готово и паки прочитаны, положить на карту.
    pub(super) fn place_step(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((lat, lon, cells)) = self.place.start.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            let cache = self.data_dir.join("местность");
            let ctx = ctx.clone();
            self.place.rx = Some(rx);
            self.place.error = None;
            self.place.step = String::new();
            std::thread::spawn(move || {
                let (steps, repaint) = (tx.clone(), ctx.clone());
                let done = crate::place::build(lat, lon, cells, &cache, &move |s| {
                    let _ = steps.send(crate::place::Msg::Step(s.to_owned()));
                    repaint.request_repaint();
                });
                let _ = tx.send(crate::place::Msg::Done(Box::new(done)));
                ctx.request_repaint();
            });
        }
        // В модуле потоков нет: сначала фоном качается всё, что сборке заведомо
        // нужно, потом она идёт одним куском — кадром позже, чтобы окно успело
        // сказать «Разбор снимка…».
        #[cfg(target_arch = "wasm32")]
        {
            if let Some((lat, lon, cells)) = self.place.analyse.take() {
                let cache = self.data_dir.join("местность");
                match crate::place::build(lat, lon, cells, &cache, &|_| {}) {
                    Ok(place) => self.place.ready = Some(place),
                    Err(e) => self.place.error = Some(e),
                }
            }
            if let Some((lat, lon, cells)) = self.place.start.take() {
                crate::plat::net_prefetch(&crate::place::urls(lat, lon, cells));
                self.place.error = None;
                self.place.fetch = Some((lat, lon, cells));
            }
            if let Some(job) = self.place.fetch {
                let (done, total) = crate::plat::net_state();
                if done >= total {
                    self.place.fetch = None;
                    self.place.analyse = Some(job);
                    self.place.step = "analysis".into();
                    ctx.request_repaint();
                } else {
                    self.place.step = format!("net:{done}:{total}");
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        if let Some(rx) = &self.place.rx {
            let mut finished = false;
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    crate::place::Msg::Step(s) => self.place.step = s,
                    crate::place::Msg::Done(done) => {
                        finished = true;
                        match *done {
                            Ok(place) => self.place.ready = Some(place),
                            Err(e) => self.place.error = Some(e),
                        }
                    }
                }
            }
            if finished {
                self.place.rx = None;
            }
        }
        if !self.library.loading()
            && let Some(place) = self.place.ready.take()
        {
            let palette = self.generator_palette(self.settings.gen_library, false);
            let opt = procgen::osm::Options { trees: self.place.trees, ..Default::default() };
            let built = procgen::import_osm(&place.xml, &place.extra, Some(&place.cover), opt, palette);
            let name = place.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if self.take_terrain(built, &name) {
                self.place_form = None;
                self.notice = Some(format!("{} — {}", self.notice.clone().unwrap_or_default(), place.summary));
            }
        }
        if let Some(e) = self.place.error.take().filter(|_| self.place_form.is_none()) {
            self.error = Some(e);
        }
    }

    pub(super) fn place_window(&mut self, ctx: &egui::Context) {
        // Окно карты: «Готово» вписывает середину рамки и её размер в форму.
        if let Some(pick) = &mut self.map_pick
            && let Some(done) = pick.show(ctx)
        {
            if let (Some(p), Some(form)) = (done, &mut self.place_form) {
                form.text = format!("{:.5}, {:.5}", p.lat, p.lon);
                form.cells = p.cells;
            }
            self.map_pick = None;
        }
        let Some(form) = &mut self.place_form else {
            self.map_pick = None;
            return;
        };
        let mut open = true;
        let mut go = None;
        let mut pick = false;
        let busy = self.place.busy();
        // Ассеты — как у генераторов: тот же выбор, тот же флажок в настройках.
        let own_assets = self.library.assets.iter().any(|a| !a.is_builtin());
        let mut library = self.settings.gen_library && own_assets;
        egui::Window::new("Местность по координатам").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label("Координаты или ссылка на карту:");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.text).desired_width(360.0).hint_text("59.46956, 25.65897"));
                pick = ui.button("На карте…").on_hover_text("Найти место и обвести рамкой на снимке").clicked();
            });
            let found = crate::place::parse_place(&form.text);
            ui.horizontal(|ui| {
                ui.label("Размер, клеток:");
                ui.add(egui::DragValue::new(&mut form.cells[0]).range(8..=256));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut form.cells[1]).range(8..=256));
            });
            ui.horizontal(|ui| {
                ui.label("Деревья:");
                ui.add(egui::Slider::new(&mut form.trees, 0.0..=1.0).custom_formatter(|v, _| if v <= 0.0 { "по снимку".to_owned() } else { format!("+{:.0} %", v * 100.0) }));
            });
            ui.horizontal(|ui| {
                ui.label("Ассеты:");
                ui.selectable_value(&mut library, false, "Встроенные");
                ui.add_enabled_ui(own_assets, |ui| {
                    ui.selectable_value(&mut library, true, "Из библиотеки").on_hover_text("Паки и папка из «Правка»; чего там нет — встроенные");
                });
                if !own_assets {
                    ui.weak("(паков и папки нет)");
                }
            });
            if let Some((lat, lon)) = found {
                ui.label(format!("{lat:.5}, {lon:.5}"));
            }
            if ui.add_enabled(found.is_some() && !busy, egui::Button::new("Собрать")).clicked() {
                go = found.map(|(lat, lon)| (lat, lon, form.cells, form.trees));
            }
            if busy {
                let step = match self.place.step.as_str() {
                    "osm" => "Выгрузка OpenStreetMap…".to_owned(),
                    "photo" => "Снимок…".to_owned(),
                    "layers" => "Карты лесов и болот…".to_owned(),
                    "analysis" => "Разбор снимка…".to_owned(),
                    // В модуле: фоном качается выгрузка, плитки снимка и слои.
                    s if s.starts_with("net:") => {
                        let n: Vec<&str> = s.split(':').collect();
                        format!("Скачиваю: {} из {}", n.get(1).unwrap_or(&"0"), n.get(2).unwrap_or(&"0"))
                    }
                    _ => "Готовлю…".to_owned(),
                };
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(step);
                });
            }
            if let Some(e) = &self.place.error {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 70), e);
            }
        });
        if pick {
            self.map_pick = Some(crate::mappick::MapPick::new(crate::place::parse_place(&form.text), form.cells));
        }
        if own_assets && library != self.settings.gen_library {
            self.settings.gen_library = library;
            self.save_settings();
        }
        if let Some((lat, lon, cells, trees)) = go {
            self.place.error = None;
            self.place.trees = trees;
            self.place.start = Some((lat, lon, cells));
        }
        if !open {
            self.place_form = None;
            self.map_pick = None;
        }
    }

    // ----- файлы -----
}
