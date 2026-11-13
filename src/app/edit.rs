//! Правки выбранного: удаление, крыши, двери, предметы, материалы, вид стен.

use super::*;

impl App {
    pub fn delete_selected(&mut self) {
        if !self.picked.is_empty() {
            self.checkpoint();
            let mut idx = std::mem::take(&mut self.picked);
            idx.sort_unstable();
            for i in idx.into_iter().rev() {
                self.map.props.remove(i);
            }
            self.sel = None;
            return;
        }
        let Some(s) = self.sel.take() else { return };
        self.checkpoint();
        match s {
            Sel::Wall(i) => {
                self.map.walls.remove(i);
            }
            Sel::Portal(i) => {
                self.map.portals.remove(i);
            }
            Sel::Prop(i) => {
                self.map.props.remove(i);
            }
            Sel::Path(i) => {
                self.map.paths.remove(i);
            }
            Sel::Label(i) => {
                self.map.labels.remove(i);
            }
            Sel::Roof(i) => {
                self.map.roofs.remove(i);
            }
            // Весь дом — вся крыша с частями; часть — только она.
            Sel::Outline(i, 0) => {
                self.map.outline_roofs.remove(i);
            }
            Sel::Outline(i, k) => {
                self.map.outline_roofs[i].pieces.remove(k);
            }
            // Стена комнаты стирается вместе с комнатой.
            Sel::Edge(..) => {}
        }
    }

    /// Насколько видны крыши сейчас: 1 — целиком, 0 — нет.
    pub fn roof_alpha(&self) -> f32 {
        match self.roof_view {
            RoofView::Hidden => 0.0,
            RoofView::Ghost => 0.35,
            RoofView::Auto if matches!(self.tool, Tool::Select | Tool::Roof) => 1.0,
            RoofView::Auto => 0.35,
        }
    }

    /// Крыша под точкой, если её сейчас можно взять.
    pub(super) fn roof_at(&self, w: [f32; 2]) -> Option<usize> {
        if self.roof_alpha() < 1.0 && self.tool != Tool::Roof {
            return None;
        }
        self.map.roofs.iter().rposition(|r| r.contains(w))
    }

    /// Крыша OSM и её кусок под точкой, если её сейчас можно взять: верхняя
    /// крыша, а в ней верхняя часть.
    pub(super) fn outline_at(&self, w: [f32; 2]) -> Option<Sel> {
        if self.roof_alpha() < 1.0 && self.tool != Tool::Roof {
            return None;
        }
        let roofs = &self.map.outline_roofs;
        roofs.iter().enumerate().rev().find_map(|(i, r)| r.pieces.iter().rposition(|p| p.contains(w)).map(|k| Sel::Outline(i, k)))
    }

    /// Крыша над прямоугольником клеток `a`–`b` (включительно), по образцу.
    pub fn place_roof(&mut self, a: (i32, i32), b: (i32, i32)) -> usize {
        let lo = [a.0.min(b.0) as f32, a.1.min(b.1) as f32];
        let hi = [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0];
        let roof = map::Roof { a: lo, b: hi, ..self.roof_style.clone() };
        self.checkpoint();
        self.map.roofs.push(roof);
        let i = self.map.roofs.len() - 1;
        self.sel = Some(Sel::Roof(i));
        i
    }

    /// Поставить начатую дорогу. Из одной точки дороги не бывает.
    pub fn finish_path(&mut self) {
        let points = std::mem::take(&mut self.path_draft);
        if points.len() >= 2 {
            self.checkpoint();
            let material = self.material.min(self.map.materials.len().saturating_sub(1));
            self.map.paths.push(map::Path { points, material, ..self.path_style.clone() });
            self.sel = Some(Sel::Path(self.map.paths.len() - 1));
        }
    }

    /// Все стены карты: вокруг комнат и нарисованные вручную (без проёмов).
    pub fn all_wall_segments(&mut self) -> Vec<Segment> {
        let mut segs = self.room_edges().to_vec();
        for w in &self.map.walls {
            segs.extend(w.segments());
        }
        segs
    }

    /// Дверь или окно в ближайшую к точке стену. `None` — стены рядом нет.
    pub fn place_portal(&mut self, p: [f32; 2]) -> Option<usize> {
        let (a, b) = self
            .all_wall_segments()
            .into_iter()
            .map(|s| (s, map::segment_distance(p, s.0, s.1)))
            .filter(|(_, d)| *d <= DOOR_REACH)
            .min_by(|x, y| x.1.total_cmp(&y.1))?
            .0;
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let w = self.door_width.min(len);
        let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len;
        // Проём встаёт на узлы сетки: дверь в клетку — ровно в клетку.
        let start = (t - w / 2.0).round().clamp(0.0, len - w);
        let at = |t: f32| [a[0] + dx * t / len, a[1] + dy * t / len];
        // Концы всегда слева направо и сверху вниз — как бы ни шла стена.
        let (mut a, mut b) = (at(start), at(start + w));
        if (b[0], b[1]) < (a[0], a[1]) {
            std::mem::swap(&mut a, &mut b);
        }
        let asset = if matches!(self.door_kind, PortalKind::Opening | PortalKind::Secret) { None } else { self.door_asset.clone() };
        let frame = if self.door_kind.hidden() { None } else { self.door_frame.as_deref().map(|f| library::frame_for_width(f, w)) };
        let portal = Portal { a, b, kind: self.door_kind, asset, frame };
        // На том же месте уже есть проём — новый его заменяет.
        let clash = self.map.portals.iter().position(|q| map::segment_distance(q.center(), portal.a, portal.b) < 0.01);
        self.checkpoint();
        let i = match clash {
            Some(i) => {
                self.map.portals[i] = portal;
                i
            }
            None => {
                self.map.portals.push(portal);
                self.map.portals.len() - 1
            }
        };
        self.sel = Some(Sel::Portal(i));
        Some(i)
    }

    /// Размер ассета в клетках: по самой картинке, а пока она не загрузилась — по имени.
    pub(super) fn asset_size(&mut self, key: &str) -> [f32; 2] {
        if let Some(size) = library::builtin_size(key) {
            return size;
        }
        if let Some((_, px)) = self.textures.get(key, false) {
            // Картинка вдвое крупнее обычной (512 точек на клетку, как объединённые)
            // узнаётся по размеру в имени: `…_2.5x1.25`.
            if let Some(s) = library::size_from_name(&library::label(key))
                && s[0] > 0.0
                && (px[0] as f32 / s[0] - 2.0 * PX_PER_CELL).abs() < 3.0
            {
                return s;
            }
            return [px[0] as f32 / PX_PER_CELL, px[1] as f32 / PX_PER_CELL];
        }
        library::size_from_name(&library::label(key)).unwrap_or([1.0, 1.0])
    }

    /// Точка объекта с привязкой к сетке или без: `alt` меняет настройку на время.
    pub fn snap_pos(&self, p: [f32; 2], alt: bool) -> [f32; 2] {
        if self.settings.snap != alt { [snap_half(p[0]), snap_half(p[1])] } else { p }
    }

    pub fn place_prop(&mut self, p: [f32; 2], alt: bool) -> Option<usize> {
        let asset = self.object_asset.clone()?;
        let size = self.asset_size(&asset);
        let pos = self.snap_pos(p, alt);
        self.checkpoint();
        self.map.props.push(Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None });
        let i = self.map.props.len() - 1;
        self.sel = Some(Sel::Prop(i));
        Some(i)
    }

    /// Что лежит под точкой: сначала объекты (верхний — последний), потом проёмы, потом стены.
    pub fn pick(&self, w: [f32; 2]) -> Option<Sel> {
        if let Some(i) = self.map.labels.iter().rposition(|l| l.contains(w)) {
            return Some(Sel::Label(i));
        }
        // Видимая крыша закрывает всё, что под ней.
        if let Some(i) = self.roof_at(w) {
            return Some(Sel::Roof(i));
        }
        if let Some(s) = self.outline_at(w) {
            return Some(s);
        }
        // Щелчок берёт видимый сверху: последний в порядке рисования.
        if let Some(&i) = self.prop_order().iter().rev().find(|&&i| self.map.props[i].contains(w)) {
            return Some(Sel::Prop(i));
        }
        let near = |d: f32| d <= PICK;
        if let Some((i, _)) = self.map.portals.iter().enumerate().map(|(i, p)| (i, map::segment_distance(w, p.a, p.b))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)) {
            return Some(Sel::Portal(i));
        }
        if let Some(s) = self.map.walls.iter().enumerate().map(|(i, wall)| (i, wall.distance(w))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| Sel::Wall(i)) {
            return Some(s);
        }
        if let Some(s) = self.edges.iter().map(|s| (s, map::segment_distance(w, s.0, s.1))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(s, _)| Sel::Edge(map::edge_key(s), false)) {
            return Some(s);
        }
        // Дорога — по всей ширине; верхняя — последняя.
        self.map.paths.iter().rposition(|p| p.distance(w) <= p.width / 2.0).map(Sel::Path)
    }

    /// Один отпечаток разброса в точке `c`: насыпать объекты из набора или, с Alt,
    /// убрать их из круга. Правку в историю кладёт начало мазка.
    pub fn scatter_dab(&mut self, c: [f32; 2], erase: bool) {
        let r = self.scatter.radius;
        let inside = |p: [f32; 2]| (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r;
        if erase {
            let set = self.scatter.set.clone();
            self.map.props.retain(|p| !(inside(p.pos) && (set.is_empty() || set.contains(&p.asset))));
            self.sel = None;
            self.picked.clear();
            return;
        }
        if self.scatter.set.is_empty() {
            self.browse = Kind::Object;
            return;
        }
        let area = std::f32::consts::PI * r * r;
        let tries = ((self.scatter.density * area * 0.5).round() as usize).max(1);
        let reach = |p: &Prop| p.size[0].max(p.size[1]) * p.scale / 2.0;
        for _ in 0..tries {
            let (a, d) = (self.scatter.rand() * std::f32::consts::TAU, self.scatter.rand().sqrt() * r);
            let pos = [c[0] + a.cos() * d, c[1] + a.sin() * d];
            if pos[0] < 0.0 || pos[1] < 0.0 || pos[0] > self.map.width as f32 || pos[1] > self.map.height as f32 {
                continue;
            }
            let k = ((self.scatter.rand() * self.scatter.set.len() as f32) as usize).min(self.scatter.set.len() - 1);
            let asset = self.scatter.set[k].clone();
            let size = self.asset_size(&asset);
            let t = self.scatter.rand();
            let scale = self.scatter.scale[0] + (self.scatter.scale[1] - self.scatter.scale[0]) * t;
            let rot = if self.scatter.spin { (self.scatter.rand() * 360.0).floor() } else { self.object_rot };
            let prop = Prop { asset, pos, size, rot, scale, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
            let me = reach(&prop) * self.scatter.spacing;
            // Просвет: не ближе суммы радиусов (с долей) к соседям того же набора.
            let crowded = self.map.props.iter().any(|q| {
                self.scatter.set.contains(&q.asset) && ((q.pos[0] - pos[0]).powi(2) + (q.pos[1] - pos[1]).powi(2)).sqrt() < me + reach(q) * self.scatter.spacing
            });
            if !crowded {
                self.map.props.push(prop);
            }
        }
    }

    /// Объединить выбранные объекты в один: картинка 512 точек на клетку с
    /// прозрачным фоном, как их видно на карте, в папку «Запечённые»; на карте
    /// группа заменяется этим объектом там же, где была самая нижняя из группы.
    pub fn bake_picked(&mut self) {
        let mut idx: Vec<usize> = self.picked.iter().copied().filter(|&i| i < self.map.props.len()).collect();
        idx.sort_unstable();
        idx.dedup();
        if idx.len() < 2 {
            return;
        }
        match self.bake(&idx) {
            Ok(n) => self.notice = Some(format!("Объединено в один объект: {n} шт.")),
            Err(e) => self.error = Some(format!("Не объединилось: {e}")),
        }
    }

    pub(super) fn bake(&mut self, idx: &[usize]) -> Result<usize, String> {
        const PPC: f32 = 2.0 * PX_PER_CELL;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for &i in idx {
            for c in self.map.props[i].corners() {
                lo = [lo[0].min(c[0]), lo[1].min(c[1])];
                hi = [hi[0].max(c[0]), hi[1].max(c[1])];
            }
        }
        let (w, h) = (((hi[0] - lo[0]) * PPC).ceil() as u32, ((hi[1] - lo[1]) * PPC).ceil() as u32);
        if w == 0 || h == 0 || w > 16384 || h > 16384 {
            return Err(format!("размер {w}×{h} точек"));
        }
        let mut out = image::RgbaImage::new(w, h);
        for &i in idx {
            let p = self.map.props[i].clone();
            let bytes = self.textures.bytes(&p.asset)?;
            let img = image::load_from_memory(&bytes).map_err(|e| format!("{}: {e}", library::label(&p.asset)))?.to_rgba8();
            bake_prop(&mut out, &img, &p, lo, PPC);
        }
        let dir = self.data_dir.join("baked");
        let (cw, ch) = (w as f32 / PPC, h as f32 / PPC);
        let num = |v: f32| format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_owned();
        let taken = crate::plat::fs::list(&dir);
        let n = (1..).find(|n| !taken.iter().any(|p| p.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&format!("Объединённый {n}_"))))).unwrap_or(1);
        let file = dir.join(format!("Объединённый {n}_{}x{}.png", num(cw), num(ch)));
        let mut png = Vec::new();
        out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| format!("{}: {e}", file.display()))?;
        crate::plat::fs::write(&file, &png).map_err(|e| format!("{}: {e}", file.display()))?;
        self.library.set_baked(&dir);
        self.checkpoint();
        let at = idx[0];
        for &i in idx.iter().rev() {
            self.map.props.remove(i);
        }
        let key = file.to_string_lossy().into_owned();
        let prop = Prop { asset: key, pos: [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0], size: [cw, ch], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
        self.map.props.insert(at, prop);
        self.picked.clear();
        self.sel = Some(Sel::Prop(at));
        Ok(idx.len())
    }

    /// Поворот на четверть: выбранного объекта или того, что сейчас ставится.
    pub fn rotate(&mut self) {
        if let Some(Sel::Roof(i)) = self.sel {
            self.checkpoint();
            let r = &mut self.map.roofs[i];
            r.turn = (r.turn + 1) % 4;
            return;
        }
        if let Some(Sel::Outline(i, k)) = self.sel {
            if self.map.outline_roofs[i].pieces[k].form.turns() {
                self.checkpoint();
                self.map.outline_roofs[i].pieces[k].turn();
            }
            return;
        }
        if self.tool == Tool::Roof {
            self.roof_style.turn = (self.roof_style.turn + 1) % 4;
            return;
        }
        if let Some(Sel::Prop(i)) = self.sel {
            self.checkpoint();
            let p = &mut self.map.props[i];
            p.rot = (p.rot + 90.0) % 360.0;
        } else {
            self.object_rot = (self.object_rot + 90.0) % 360.0;
        }
    }

    /// Картинка из библиотеки становится материалом пола и выбирается.
    pub fn use_floor_texture(&mut self, key: &str) {
        let m = Material { name: library::label(key), paint: Paint::Texture(key.to_owned()), vary: false, live: map::Live::Auto };
        if !self.map.materials.iter().any(|x| x.paint == m.paint) {
            self.checkpoint();
        }
        match self.map.material_index(m) {
            Some(i) => self.material = i,
            None => self.error = Some("В карте уже 255 материалов — больше не помещается.".into()),
        }
        // Выбрана дорога или вода — её и перекрасить, инструмент не трогать.
        if !matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Path) && !self.editing_path() {
            self.tool = Tool::Dungeon;
        }
        self.recolor_path();
    }

    /// Выбрана дорога (вода, тропа) инструментом «Выбор» или «Дорога»: панель правит её.
    pub(super) fn editing_path(&self) -> bool {
        matches!(self.sel, Some(Sel::Path(_))) && matches!(self.tool, Tool::Select | Tool::Path)
    }

    /// Выбранная дорога перекрашивается в выбранный материал.
    pub(super) fn recolor_path(&mut self) {
        if let Some(Sel::Path(i)) = self.sel
            && self.editing_path()
            && self.map.paths[i].material != self.material
        {
            self.checkpoint();
            self.map.paths[i].material = self.material;
        }
    }

    pub fn use_wall_paint(&mut self, paint: Paint) {
        let mut st = self.wall_style();
        st.paint = paint;
        self.set_wall_style(st);
    }

    /// Вид, который правят панель и библиотека: у выбранной стены — её
    /// (свой появится с первой правкой), без выбора — новых стен.
    pub fn wall_style(&self) -> map::WallStyle {
        match self.sel {
            Some(Sel::Wall(i)) => self.map.walls[i].style.clone().unwrap_or_else(|| self.map.wall_style.clone()),
            Some(Sel::Edge(key, _)) => self.map.edge_look(key),
            _ => self.map.wall_style.clone(),
        }
    }

    pub fn set_wall_style(&mut self, st: map::WallStyle) {
        if st != self.wall_style() || matches!(self.sel, Some(Sel::Wall(i)) if self.map.walls[i].style.is_none()) {
            self.checkpoint();
            self.put_wall_style(st);
        }
    }

    /// Без записи в историю — для протяжки ползунка.
    pub(super) fn put_wall_style(&mut self, st: map::WallStyle) {
        match self.sel {
            Some(Sel::Wall(i)) => self.map.walls[i].style = Some(st),
            Some(Sel::Edge(key, whole)) => {
                let run = self.edge_run(key, whole);
                self.map.set_edge_look(&run, &st);
            }
            _ => {
                // Стоящие стены остаются какими были.
                let old = self.map.wall_style.clone();
                for w in self.map.walls.iter_mut().filter(|w| w.style.is_none()) {
                    w.style = Some(old.clone());
                }
                self.map.wall_style = st;
            }
        }
    }
}
