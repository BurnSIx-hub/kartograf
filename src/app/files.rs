//! Файлы: новая карта, сохранение, открытие, настройки, автосейв.

use super::*;

impl App {
    pub(super) fn reset_doc(&mut self, mut map: Map, path: Option<PathBuf>) {
        self.scene_name = None;
        let rest = std::mem::take(&mut map.levels);
        self.levels = std::iter::once(map.clone()).chain(rest).collect();
        self.level = 0;
        self.below.clear();
        self.map = map;
        self.path = path;
        self.dirty = false;
        self.rev += 1;
        self.undo.clear();
        self.redo.clear();
        self.draft.clear();
        self.sel = None;
        self.drag_from = None;
        self.moving = None;
        self.stroke_last = None;
        self.edges_dirty = true;
        self.fitted = false;
        self.material = 0;
        self.recovery.clear();
    }

    pub fn new_map(&mut self, width: u32, height: u32) {
        self.reset_doc(Map::new(width.clamp(4, 400), height.clamp(4, 400)), None);
    }

    pub fn save_to(&mut self, path: &Path) -> bool {
        match crate::plat::fs::write(path, self.project().to_json().as_bytes()) {
            Ok(()) => {
                self.path = Some(path.to_owned());
                self.dirty = false;
                self.recovery.clear();
                true
            }
            Err(e) => {
                self.error = Some(format!("Не сохранилось: {e}"));
                false
            }
        }
    }

    pub fn open_path(&mut self, path: &Path) -> bool {
        let text = crate::plat::fs::read(path).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string()));
        match text.and_then(|t| Map::from_json(&t)) {
            Ok(map) => {
                self.reset_doc(map, Some(path.to_owned()));
                true
            }
            Err(e) => {
                self.error = Some(format!("{}: {e}", path.display()));
                false
            }
        }
    }

    pub(super) fn save(&mut self) {
        match self.path.clone() {
            Some(p) => {
                self.save_to(&p);
            }
            None => self.save_as(),
        }
    }

    pub(super) fn save_as(&mut self) {
        // В модуле окна сохранения нет: карта ложится в данные Foundry под своим именем.
        #[cfg(target_arch = "wasm32")]
        {
            let path = self.free_map_path();
            if self.save_to(&path) {
                self.notice = Some(format!("Сохранено в данные Foundry: {}", path.to_string_lossy().trim_start_matches('/')));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dialog = crate::plat::FileDialog::new().add_filter("Карта", &[map::EXT]).set_file_name(format!("{}.{}", self.name(), map::EXT));
            if let Some(path) = dialog.save_file() {
                self.save_to(&path);
            }
        }
    }

    /// Папка карт модуля в данных Foundry.
    pub(super) fn maps_dir(&self) -> PathBuf {
        self.data_dir.join("maps")
    }

    /// Файл для карты в данных Foundry: по имени, без знаков, которые Foundry не
    /// примет в имени файла; чужая карта с тем же именем не затирается — номер.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(super) fn free_map_path(&self) -> PathBuf {
        let clean: String = self.name().chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { '_' }).collect();
        let clean = clean.trim().to_owned();
        let base = if clean.is_empty() { "Карта".to_owned() } else { clean };
        let dir = self.maps_dir();
        (1..)
            .map(|n| dir.join(if n == 1 { format!("{base}.{}", map::EXT) } else { format!("{base} {n}.{}", map::EXT) }))
            .find(|p| self.path.as_deref() == Some(p.as_path()) || !crate::plat::fs::exists(p))
            .unwrap_or_else(|| dir.join(format!("{base}.{}", map::EXT)))
    }

    pub(super) fn open(&mut self) {
        // В модуле — окно со списком карт из данных Foundry.
        if cfg!(target_arch = "wasm32") {
            let mut maps: Vec<PathBuf> = crate::plat::fs::list(&self.maps_dir()).into_iter().filter(|p| p.extension().is_some_and(|e| e == map::EXT)).collect();
            maps.sort();
            self.open_list = Some(maps);
            return;
        }
        if let Some(path) = crate::plat::FileDialog::new().add_filter("Карта", &[map::EXT]).pick_file() {
            self.open_path(&path);
        }
    }

    pub(super) fn save_settings(&self) {
        let _ = crate::plat::fs::write(&self.data_dir.join("settings.json"), &serde_json::to_vec_pretty(&self.settings).unwrap_or_default());
    }

    pub fn set_textures_dir(&mut self, dir: PathBuf) {
        self.library.set_folder(&dir);
        self.settings.textures_dir = Some(dir);
        self.save_settings();
    }

    pub fn add_pack(&mut self, file: PathBuf) {
        if !self.settings.packs.contains(&file) {
            self.settings.packs.push(file.clone());
            self.save_settings();
        }
        self.library.add_pack(file);
    }

    pub(super) fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Новая карта".into())
    }

    /// Копия на случай сбоя, если пора.
    pub fn autosave(&mut self, now: f64, force: bool) {
        if force || self.recovery.due(now, self.dirty, self.rev) {
            let meta = recovery::Meta { name: self.name(), path: self.path.clone(), saved: recovery::now_unix() };
            self.recovery.snapshot(now, self.rev, &self.project(), &meta);
        }
    }

    pub fn restore(&mut self, orphan: &Orphan) -> bool {
        match self.recovery.load(orphan) {
            Ok(map) => {
                self.reset_doc(map, orphan.meta.path.clone());
                self.dirty = true;
                self.recovery.discard(orphan);
                true
            }
            Err(e) => {
                self.error = Some(format!("Копию не удалось открыть: {e}"));
                false
            }
        }
    }

    /// Ассеты текущей вкладки под строку поиска; пересчёт только когда что-то поменялось.
    pub fn filtered(&mut self) -> &[usize] {
        // С фильтром «на карте» пересчёт и при правках карты: их видно по числу вещей.
        let m = &self.map;
        let used = if self.used_only { 1 + m.props.len() + m.portals.len() + m.roofs.len() + m.materials.len() + m.wall_looks.len() + m.walls.len() } else { 0 };
        let key = (self.browse, self.query.clone(), self.library.assets.len(), used);
        if key != self.filter_key {
            self.filtered = self.library.filter(self.browse, &self.query);
            if self.used_only {
                let on_map = self.map.used_assets();
                let assets = &self.library.assets;
                self.filtered.retain(|&i| on_map.contains(&assets[i].key));
            }
            self.groups = self.library.group(&self.filtered);
            self.drawer = None;
            self.filter_key = key;
        }
        &self.filtered
    }

    // ----- кадр -----
}
