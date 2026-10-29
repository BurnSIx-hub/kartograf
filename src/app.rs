//! Окно редактора: меню, панель инструментов с библиотекой, холст и строка состояния.

use crate::generate::{self, Options};
use crate::library::{self, Kind, Library, PX_PER_CELL, Textures};
use crate::map::{self, Coat, Layer, Map, Material, Paint, Portal, PortalKind, Prop, Segment, Wall};
use crate::mask::{self, Brush, Mask, RES, Shape as Tip};
use crate::recovery::{self, Orphan, Recovery};
use crate::selftest::SelfTest;
use crate::view::{Camera, WHEEL_STEP};
use eframe::egui::{self, Color32, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Shape, Stroke, ViewportCommand, pos2, vec2};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const HISTORY: usize = 200;
/// Насколько близко к стене или двери (в клетках) надо щёлкнуть, чтобы её выбрать.
const PICK: f32 = 0.35;
/// Насколько близко к стене надо щёлкнуть, чтобы поставить в неё дверь.
const DOOR_REACH: f32 = 0.6;
/// Меньше этого (в клетках) плитка пола не бывает, даже у крошечной картинки.
const MIN_TILE: f32 = 1.0;
const THUMB: f32 = 52.0;
const BROWSER_HEIGHT: f32 = 340.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Dungeon,
    Floor,
    Erase,
    Wall,
    Door,
    Object,
}

impl Tool {
    const ALL: [Tool; 7] = [Tool::Select, Tool::Dungeon, Tool::Floor, Tool::Erase, Tool::Wall, Tool::Door, Tool::Object];

    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Выбор",
            Tool::Dungeon => "Подземелье",
            Tool::Floor => "Местность",
            Tool::Erase => "Стереть",
            Tool::Wall => "Стена",
            Tool::Door => "Дверь и окно",
            Tool::Object => "Объект",
        }
    }

    pub fn key(self) -> Key {
        match self {
            Tool::Select => Key::V,
            Tool::Dungeon => Key::D,
            Tool::Floor => Key::F,
            Tool::Erase => Key::E,
            Tool::Wall => Key::W,
            Tool::Door => Key::O,
            Tool::Object => Key::P,
        }
    }
}

/// Что выбрано на карте.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sel {
    Wall(usize),
    Portal(usize),
    Prop(usize),
}

pub struct Args {
    pub file: Option<PathBuf>,
    pub selftest: Option<PathBuf>,
    pub shot: Option<PathBuf>,
    pub size: Option<[f32; 2]>,
    pub data: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub textures_dir: Option<PathBuf>,
    #[serde(default)]
    pub packs: Vec<PathBuf>,
    /// Объекты встают по сетке; Alt на время меняет на обратное.
    #[serde(default = "yes")]
    pub snap: bool,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { textures_dir: None, packs: Vec::new(), snap: true }
    }
}

/// Какую стену выгибают Ctrl+Shift с протяжкой.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Bend {
    /// Прямой участок стены комнаты: дуга ляжет в маску, когда отпустят.
    Room { a: [f32; 2], b: [f32; 2], h: f32 },
    /// Отрезок `seg` нарисованной стены `wall`: гнётся сразу.
    Wall { wall: usize, seg: usize },
}

/// За что взяли выбранный объект.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Grip {
    /// Тащат целиком: смещение точки захвата от центра.
    Move([f32; 2]),
    /// Угол: размер растёт с расстоянием до центра. Размер и расстояние в начале.
    Scale(f32, f32),
    /// Ручка над объектом: поворот вслед за курсором.
    Turn,
}

/// Окно «Новая карта».
struct NewMap {
    width: u32,
    height: u32,
}

pub struct App {
    pub map: Map,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Растёт с каждой правкой: по нему автосейв понимает, что копия устарела.
    pub rev: u64,
    pub tool: Tool,
    /// Номер материала в палитре карты.
    pub material: usize,
    pub cam: Camera,
    fitted: bool,
    undo: Vec<Map>,
    redo: Vec<Map>,
    /// Клетка, с которой начали тянуть заливку.
    drag_from: Option<(i32, i32)>,
    pub brush: Brush,
    /// Где был последний отпечаток идущего мазка.
    stroke_last: Option<[f32; 2]>,
    /// Где сейчас курсор в мазке: отпечатки идут с шагом и могут не дойти до него.
    stroke_tip: [f32; 2],
    /// Сила, уже положенная этим мазком в каждую точку маски.
    stroke_max: std::collections::HashMap<usize, f32>,
    /// Счётчик отпечатков: спрей рассыпается каждый раз по-новому.
    dabs: u32,
    edges: Vec<Segment>,
    edges_dirty: bool,
    /// Объект, который тащат, и за что взяли.
    moving: Option<(usize, Grip)>,
    pub bending: Option<Bend>,
    /// Alt с колесом уже меняет размер объекта: правка в истории сделана.
    alt_wheel: bool,
    /// Стена, которую сейчас ставят по точкам.
    pub draft: Vec<[f32; 2]>,
    pub sel: Option<Sel>,
    pub canvas: Rect,
    pub hover: Option<Pos2>,
    pub error: Option<String>,
    pub test: Option<SelfTest>,
    shot: Option<PathBuf>,
    shot_asked: bool,
    frame: u64,
    title: String,

    pub data_dir: PathBuf,
    pub settings: Settings,
    pub library: Library,
    pub textures: Textures,
    pub browse: Kind,
    pub query: String,
    filtered: Vec<usize>,
    filter_key: (Kind, String, usize),
    pub door_asset: Option<String>,
    pub door_kind: PortalKind,
    pub door_width: f32,
    pub object_asset: Option<String>,
    pub object_rot: f32,
    pub object_scale: f32,

    pub recovery: Recovery,
    pub orphans: Vec<Orphan>,
    pub generator: Option<Options>,
    new_map: Option<NewMap>,
    confirm_close: bool,
    allow_close: bool,
}

pub fn default_data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kartograf")
}

fn snap_half(v: f32) -> f32 {
    (v * 2.0).round() / 2.0
}

/// Ручки выбранного объекта в мире: четыре угла и поворотная над верхней кромкой.
/// Поворотная отстоит на `lift` (в клетках — чтобы на экране было постоянно).
pub fn grips(prop: &Prop, lift: f32) -> ([[f32; 2]; 4], [f32; 2]) {
    let c = prop.corners();
    let top = [(c[0][0] + c[1][0]) / 2.0, (c[0][1] + c[1][1]) / 2.0];
    let (dx, dy) = (top[0] - prop.pos[0], top[1] - prop.pos[1]);
    let l = (dx * dx + dy * dy).sqrt().max(1e-6);
    (c, [top[0] + dx / l * lift, top[1] + dy / l * lift])
}

const GRIP_PX: f32 = 7.0;
pub const TURN_LIFT_PX: f32 = 26.0;
const SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.1..=8.0;

impl App {
    pub fn new(cc: &eframe::CreationContext, args: Args) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let data_dir = args.data.or_else(|| args.selftest.as_ref().map(|d| d.join("data"))).unwrap_or_else(default_data_dir);
        let _ = std::fs::create_dir_all(&data_dir);
        let settings: Settings = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&b)).ok())
            .unwrap_or_default();
        let recovery = Recovery::new(&data_dir);
        let orphans = if args.selftest.is_some() { Vec::new() } else { recovery.orphans() };
        let mut library = Library::default();
        if let Some(dir) = &settings.textures_dir {
            library.set_folder(dir);
        }
        for p in &settings.packs {
            library.add_pack(p.clone());
        }
        let mut app = Self {
            map: Map::new(30, 20),
            path: None,
            dirty: false,
            rev: 0,
            tool: Tool::Dungeon,
            material: 0,
            cam: Camera::default(),
            fitted: false,
            undo: Vec::new(),
            redo: Vec::new(),
            drag_from: None,
            brush: Brush::default(),
            stroke_last: None,
            stroke_tip: [0.0, 0.0],
            stroke_max: Default::default(),
            dabs: 0,
            edges: Vec::new(),
            edges_dirty: true,
            moving: None,
            bending: None,
            alt_wheel: false,
            draft: Vec::new(),
            sel: None,
            canvas: Rect::NOTHING,
            hover: None,
            error: None,
            test: args.selftest.map(SelfTest::new),
            shot: args.shot,
            shot_asked: false,
            frame: 0,
            title: String::new(),
            data_dir,
            settings,
            library,
            textures: Textures::new(&cc.egui_ctx),
            browse: Kind::Floor,
            query: String::new(),
            filtered: Vec::new(),
            filter_key: (Kind::Floor, String::from("\u{0}"), 0),
            door_asset: None,
            door_kind: PortalKind::Door,
            door_width: 1.0,
            object_asset: None,
            object_rot: 0.0,
            object_scale: 1.0,
            recovery,
            orphans,
            generator: None,
            new_map: None,
            confirm_close: false,
            allow_close: false,
        };
        if let Some(file) = args.file {
            app.open_path(&file);
        }
        app
    }

    // ----- правки и история -----

    /// Запомнить карту перед правкой.
    pub fn checkpoint(&mut self) {
        self.undo.push(self.map.clone());
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.touch();
    }

    fn touch(&mut self) {
        self.dirty = true;
        self.rev += 1;
        self.edges_dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.map, prev));
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.map, next));
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.sel = self.sel.filter(|s| self.sel_exists(*s));
        self.material = self.material.min(self.map.materials.len().saturating_sub(1));
        self.draft.clear();
        self.moving = None;
        self.touch();
    }

    fn sel_exists(&self, s: Sel) -> bool {
        match s {
            Sel::Wall(i) => i < self.map.walls.len(),
            Sel::Portal(i) => i < self.map.portals.len(),
            Sel::Prop(i) => i < self.map.props.len(),
        }
    }

    /// Кисть «Клетки»: целые клетки прямоугольника.
    pub fn paint(&mut self, a: (i32, i32), b: (i32, i32)) {
        let mut probe = self.map.clone();
        let changed = match self.tool {
            Tool::Erase => probe.paint_rect(Layer::Floor, a, b, None),
            Tool::Dungeon => probe.paint_rect(Layer::Rooms, a, b, Some(self.material)),
            _ => probe.paint_rect(Layer::Floor, a, b, Some(self.material)),
        };
        if changed {
            self.checkpoint();
            self.map = probe;
        }
    }

    /// Один отпечаток кисти в точке карты. В пределах мазка точка получает самую
    /// большую силу из легших на неё отпечатков, а не их сумму: иначе мягкие края
    /// соседних отпечатков складываются, середина края выпирает, а углы квадратной
    /// кисти проседают. Докладывается только прибавка: слои смешиваются через
    /// множитель (1 − сила), и прибавка `(new − old) / (1 − old)` даёт ровно `new`.
    fn dab(&mut self, w: [f32; 2]) {
        let mut weights = mask::dab(self.map.rooms.w, self.map.rooms.h, &self.brush, w, self.dabs);
        self.dabs = self.dabs.wrapping_add(1);
        weights.retain_mut(|(i, v)| {
            let old = self.stroke_max.get(i).copied().unwrap_or(0.0);
            if *v <= old {
                return false;
            }
            self.stroke_max.insert(*i, *v);
            *v = if old >= 1.0 { 0.0 } else { (*v - old) / (1.0 - old) };
            *v > 0.0
        });
        match self.tool {
            Tool::Erase => self.map.erase(&weights),
            Tool::Dungeon => self.map.paint(Layer::Rooms, self.material, &weights),
            _ => self.map.paint(Layer::Floor, self.material, &weights),
        }
        self.edges_dirty = true;
    }

    /// Начало мазка: одна правка в истории на весь мазок.
    pub fn begin_stroke(&mut self, w: [f32; 2]) {
        self.checkpoint();
        self.stroke_last = Some(w);
        self.stroke_tip = w;
        self.stroke_max.clear();
        self.dab(w);
    }

    /// Ведение мазка: отпечатки через равные промежутки, чтобы след был сплошным
    /// при любой скорости мыши.
    pub fn stroke_to(&mut self, w: [f32; 2]) {
        let Some(last) = self.stroke_last else { return };
        self.stroke_tip = w;
        let gap = (self.brush.radius * if self.brush.shape == Tip::Spray { 0.8 } else { 0.3 }).max(0.08);
        let (dx, dy) = (w[0] - last[0], w[1] - last[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let n = (d / gap).floor() as usize;
        for k in 1..=n {
            let t = k as f32 * gap / d;
            self.dab([last[0] + dx * t, last[1] + dy * t]);
        }
        if n > 0 {
            let t = n as f32 * gap / d;
            self.stroke_last = Some([last[0] + dx * t, last[1] + dy * t]);
        }
    }

    pub fn end_stroke(&mut self) {
        if let Some(last) = self.stroke_last {
            // Последний отпечаток — под курсором, иначе мазок короче, чем вели.
            // Спрей не добиваем: лишнее пятно в конце заметно.
            let tip = self.stroke_tip;
            if self.brush.shape != Tip::Spray && (tip[0] - last[0]).abs() + (tip[1] - last[1]).abs() > 1e-3 {
                self.dab(tip);
            }
            self.stroke_last = None;
            self.map.prune();
            self.edges_dirty = true;
        }
    }

    /// Стены вокруг комнат; пересчитываются, только когда комнаты менялись.
    pub fn room_edges(&mut self) -> &[Segment] {
        if self.edges_dirty {
            self.edges = self.map.room_edges();
            self.edges_dirty = false;
        }
        &self.edges
    }

    /// Поставить начатую стену на карту. Из одной точки стены не бывает.
    pub fn finish_wall(&mut self, closed: bool) {
        let points = std::mem::take(&mut self.draft);
        if points.len() >= 2 {
            self.checkpoint();
            self.map.walls.push(Wall { closed: closed && points.len() >= 3, points, bends: Vec::new() });
            self.sel = Some(Sel::Wall(self.map.walls.len() - 1));
        }
    }

    pub fn delete_selected(&mut self) {
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
        let portal = Portal { a, b, kind: self.door_kind, asset: self.door_asset.clone() };
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
    fn asset_size(&mut self, key: &str) -> [f32; 2] {
        if let Some(size) = library::builtin_size(key) {
            return size;
        }
        if let Some((_, px)) = self.textures.get(key, false) {
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
        self.map.props.push(Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale });
        let i = self.map.props.len() - 1;
        self.sel = Some(Sel::Prop(i));
        Some(i)
    }

    /// Что лежит под точкой: сначала объекты (верхний — последний), потом проёмы, потом стены.
    pub fn pick(&self, w: [f32; 2]) -> Option<Sel> {
        if let Some(i) = self.map.props.iter().rposition(|p| p.contains(w)) {
            return Some(Sel::Prop(i));
        }
        let near = |d: f32| d <= PICK;
        if let Some((i, _)) = self.map.portals.iter().enumerate().map(|(i, p)| (i, map::segment_distance(w, p.a, p.b))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)) {
            return Some(Sel::Portal(i));
        }
        self.map.walls.iter().enumerate().map(|(i, wall)| (i, wall.distance(w))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| Sel::Wall(i))
    }

    /// Поворот на четверть: выбранного объекта или того, что сейчас ставится.
    pub fn rotate(&mut self) {
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
        let m = Material { name: library::label(key), paint: Paint::Texture(key.to_owned()), vary: false };
        if !self.map.materials.iter().any(|x| x.paint == m.paint) {
            self.checkpoint();
        }
        match self.map.material_index(m) {
            Some(i) => self.material = i,
            None => self.error = Some("В карте уже 255 материалов — больше не помещается.".into()),
        }
        if !matches!(self.tool, Tool::Dungeon | Tool::Floor) {
            self.tool = Tool::Dungeon;
        }
    }

    pub fn use_wall_paint(&mut self, paint: Paint) {
        if self.map.wall_style.paint != paint {
            self.checkpoint();
            self.map.wall_style.paint = paint;
        }
    }

    /// Случайное подземелье вместо нынешних комнат; местность и стены не трогаются.
    pub fn generate(&mut self, opt: &Options) -> usize {
        let (cells, rooms) = generate::dungeon(self.map.width, self.map.height, opt);
        self.checkpoint();
        self.map.rooms = Mask::for_cells(self.map.width, self.map.height);
        self.map.room_floor.clear();
        let (w, h, cw) = (self.map.rooms.w, self.map.rooms.h, self.map.width as usize);
        let weights: Vec<(usize, f32)> = cells
            .iter()
            .enumerate()
            .filter(|(_, on)| **on)
            .flat_map(|(k, _)| {
                let c = ((k % cw) as i32, (k / cw) as i32);
                mask::cells(w, h, c, c)
            })
            .collect();
        self.map.paint(Layer::Rooms, self.material, &weights);
        self.edges_dirty = true;
        rooms.len()
    }

    // ----- файлы -----

    fn reset_doc(&mut self, map: Map, path: Option<PathBuf>) {
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
        match std::fs::write(path, self.map.to_json()) {
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
        match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Map::from_json(&t)) {
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

    fn save(&mut self) {
        match self.path.clone() {
            Some(p) => {
                self.save_to(&p);
            }
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) {
        let dialog = rfd::FileDialog::new().add_filter("Карта", &[map::EXT]).set_file_name(format!("{}.{}", self.name(), map::EXT));
        if let Some(path) = dialog.save_file() {
            self.save_to(&path);
        }
    }

    fn open(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Карта", &[map::EXT]).pick_file() {
            self.open_path(&path);
        }
    }

    fn save_settings(&self) {
        let _ = std::fs::write(self.data_dir.join("settings.json"), serde_json::to_vec_pretty(&self.settings).unwrap_or_default());
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

    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Новая карта".into())
    }

    /// Копия на случай сбоя, если пора.
    pub fn autosave(&mut self, now: f64, force: bool) {
        if force || self.recovery.due(now, self.dirty, self.rev) {
            let meta = recovery::Meta { name: self.name(), path: self.path.clone(), saved: recovery::now_unix() };
            self.recovery.snapshot(now, self.rev, &self.map, &meta);
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
        let key = (self.browse, self.query.clone(), self.library.assets.len());
        if key != self.filter_key {
            self.filtered = self.library.filter(self.browse, &self.query);
            self.filter_key = key;
        }
        &self.filtered
    }

    // ----- кадр -----

    fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
        self.textures.poll(&ctx);
        self.library.poll(&self.textures);
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
        self.shortcuts(&ctx);

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status(ui));
        egui::Panel::left("tools").default_size(270.0).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("tools-scroll").show(ui, |ui| self.tools(ui));
        });
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(Color32::from_gray(24))).show(ui, |ui| self.canvas_ui(ui));
        self.windows(&ctx);

        let title = format!("{}{} — Картограф", if self.dirty { "• " } else { "" }, self.name());
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        self.screenshots(&ctx);
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

    fn windows(&mut self, ctx: &egui::Context) {
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

        if let Some(mut opt) = self.generator.take() {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Случайное подземелье").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                egui::Grid::new("gen").num_columns(2).show(ui, |ui| {
                    ui.label("Комнат");
                    ui.add(egui::Slider::new(&mut opt.rooms, 2..=40));
                    ui.end_row();
                    ui.label("Комната от");
                    ui.add(egui::Slider::new(&mut opt.min_size, 2..=20).suffix(" кл."));
                    ui.end_row();
                    ui.label("до");
                    ui.add(egui::Slider::new(&mut opt.max_size, opt.min_size..=30).suffix(" кл."));
                    ui.end_row();
                    ui.label("Кольца");
                    ui.add(egui::Slider::new(&mut opt.loops, 0.0..=1.0));
                    ui.end_row();
                    ui.label("Зерно");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut opt.seed).desired_width(110.0));
                        if ui.button("🎲").on_hover_text("Новое зерно").clicked() {
                            opt.seed = generate::new_seed();
                        }
                    });
                    ui.end_row();
                });
                ui.weak("Одно и то же зерно даёт ту же карту. Пол берётся из выбранного материала.");
                ui.add_space(4.0);
                go = ui.button("Создать").clicked();
            });
            if go {
                if opt.seed.trim().is_empty() {
                    opt.seed = generate::new_seed();
                }
                self.generate(&opt);
            }
            if open {
                self.generator = Some(opt);
            }
        }

        if let Some(mut nm) = self.new_map.take() {
            let mut open = true;
            let mut go = false;
            egui::Window::new("Новая карта").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut nm.width).range(4..=400).suffix(" кл."));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut nm.height).range(4..=400).suffix(" кл."));
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

    fn shortcuts(&mut self, ctx: &egui::Context) {
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
        if cmd(Key::N) {
            self.new_map = Some(NewMap { width: self.map.width, height: self.map.height });
        }
        if typing {
            return;
        }
        for tool in Tool::ALL {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, tool.key())) {
                self.set_tool(tool);
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::R)) {
            self.rotate();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::G)) {
            self.settings.snap = !self.settings.snap;
            self.save_settings();
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
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.draft.clear();
            self.sel = None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)) {
            if !self.draft.is_empty() {
                self.draft.pop();
            } else {
                self.delete_selected();
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            self.fitted = false;
        }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if tool != Tool::Wall {
            self.finish_wall(false);
        }
        self.tool = tool;
        // Инструмент сам открывает нужную вкладку библиотеки.
        match tool {
            Tool::Door => self.browse = Kind::Door,
            Tool::Object => self.browse = Kind::Object,
            Tool::Dungeon | Tool::Floor if matches!(self.browse, Kind::Door | Kind::Object) => self.browse = Kind::Floor,
            _ => {}
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
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
                if item(ui, "Папка с картинками…", "")
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    self.set_textures_dir(dir);
                }
                if item(ui, "Подключить пак Dungeondraft…", "")
                    && let Some(files) = rfd::FileDialog::new().add_filter("Пак Dungeondraft", &["dungeondraft_pack"]).pick_files()
                {
                    for f in files {
                        self.add_pack(f);
                    }
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
            ui.menu_button("Создать", |ui| {
                if item(ui, "Случайное подземелье…", "") {
                    self.generator = Some(Options { seed: generate::new_seed(), ..Default::default() });
                }
            });
            ui.menu_button("Вид", |ui| {
                if item(ui, "Вся карта", "Home") {
                    self.fitted = false;
                }
            });
        });
    }

    fn swatch(&mut self, ui: &mut egui::Ui, paint: &Paint, size: f32) -> egui::Response {
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
    fn thumb(&mut self, painter: &egui::Painter, rect: Rect, key: &str) {
        painter.rect_filled(rect, 3.0, Color32::from_gray(45));
        if let Some((tex, px)) = self.textures.get(key, true) {
            let aspect = px[0] as f32 / px[1].max(1) as f32;
            let size = if aspect >= 1.0 { vec2(rect.width(), rect.width() / aspect) } else { vec2(rect.height() * aspect, rect.height()) };
            painter.image(tex.id(), Rect::from_center_size(rect.center(), size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else if self.textures.failed(key, true) {
            painter.rect_filled(rect.shrink(rect.width() * 0.3), 2.0, Color32::from_rgb(160, 40, 40));
        }
    }

    fn tools(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.strong("Инструменты");
        for tool in Tool::ALL {
            let text = format!("{}   {}", tool.label(), tool.key().name());
            if ui.selectable_label(self.tool == tool, text).clicked() {
                self.set_tool(tool);
            }
        }
        ui.add_space(4.0);
        ui.weak(match self.tool {
            Tool::Select => "Щелчок выбирает объект, дверь или стену. Объект можно тащить, за углы — менять размер, за ручку сверху — поворачивать; Alt с колесом — размер; стрелки сдвигают. Delete удаляет, R поворачивает.",
            Tool::Dungeon => "Пол комнат: стены встают по краю сами. Ctrl+Shift с протяжкой выгибает стену дугой.",
            Tool::Floor => "Пол без стен: двор, трава, площадь.",
            Tool::Erase => "Стирает и комнаты, и местность.",
            Tool::Wall => "Щелчки ставят вершины. Щелчок по первой замыкает; двойной щелчок, Enter или правая кнопка — заканчивают. Ctrl+Shift с протяжкой выгибает отрезок дугой.",
            Tool::Door => "Щелчок по стене ставит проём. Картинку двери или окна выберите во вкладке «Двери»; без картинки — простая плашка.",
            Tool::Object => "Щелчок ставит объект, протяжка двигает уже стоящий. Alt — на время наоборот с привязкой, Alt с колесом — размер, R — поворот.",
        });

        match self.tool {
            Tool::Dungeon | Tool::Floor | Tool::Erase => {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for shape in Tip::ALL {
                        ui.selectable_value(&mut self.brush.shape, shape, shape.label());
                    }
                });
                if self.brush.shape == Tip::Cells {
                    ui.weak("Щелчок — клетка, протяжка — прямоугольник клеток.");
                } else {
                    ui.add(egui::Slider::new(&mut self.brush.radius, 0.25..=10.0).logarithmic(true).text("радиус, кл."));
                    ui.add(egui::Slider::new(&mut self.brush.hardness, 0.0..=1.0).text("жёсткость"));
                    ui.add(egui::Slider::new(&mut self.brush.flow, 0.05..=1.0).text("сила"));
                    ui.weak("[ и ] или Ctrl с колесом — меньше и больше. Мягкий край и неполная сила смешивают материалы.");
                }
            }
            Tool::Door => {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.door_kind, PortalKind::Door, "Дверь");
                    ui.selectable_value(&mut self.door_kind, PortalKind::Window, "Окно");
                });
                ui.add(egui::Slider::new(&mut self.door_width, 1.0..=4.0).step_by(0.5).text("ширина, кл."));
                ui.horizontal(|ui| {
                    match self.door_asset.clone() {
                        Some(key) => {
                            ui.label(library::label(&key));
                            if ui.small_button("без картинки").clicked() {
                                self.door_asset = None;
                            }
                        }
                        None => {
                            ui.weak("без картинки");
                        }
                    }
                });
            }
            Tool::Object => {
                ui.add_space(6.0);
                ui.add(egui::Slider::new(&mut self.object_rot, 0.0..=345.0).step_by(15.0).text("поворот"));
                ui.add(egui::Slider::new(&mut self.object_scale, SCALE_RANGE).logarithmic(true).text("размер"));
            }
            _ => {}
        }
        if matches!(self.tool, Tool::Object | Tool::Select) && ui.checkbox(&mut self.settings.snap, "Привязка к сетке   G").changed() {
            self.save_settings();
        }
        if let Some(Sel::Prop(i)) = self.sel {
            ui.add_space(6.0);
            ui.strong(format!("Объект: {}", library::label(&self.map.props[i].asset)));
            let (mut rot, mut scale) = (self.map.props[i].rot, self.map.props[i].scale);
            let r1 = ui.add(egui::Slider::new(&mut rot, 0.0..=359.0).text("поворот"));
            let r2 = ui.add(egui::Slider::new(&mut scale, SCALE_RANGE).logarithmic(true).text("размер"));
            if r1.drag_started() || r2.drag_started() || ((r1.changed() || r2.changed()) && !r1.dragged() && !r2.dragged()) {
                self.checkpoint();
            }
            self.map.props[i].rot = rot;
            self.map.props[i].scale = scale;
        }

        ui.add_space(10.0);
        ui.strong("Пол");
        let materials = self.map.materials.clone();
        for (i, m) in materials.iter().enumerate() {
            ui.horizontal(|ui| {
                let resp = self.swatch(ui, &m.paint, 18.0);
                let label = ui.selectable_label(self.material == i, &m.name);
                if resp.clicked() || label.clicked() {
                    self.material = i;
                    if !matches!(self.tool, Tool::Dungeon | Tool::Floor) {
                        self.set_tool(Tool::Dungeon);
                    }
                }
            });
        }
        if let Some(m) = materials.get(self.material).filter(|m| matches!(m.paint, Paint::Texture(_))) {
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

        ui.add_space(10.0);
        ui.strong("Стены");
        ui.horizontal(|ui| {
            let paint = self.map.wall_style.paint.clone();
            self.swatch(ui, &paint, 18.0);
            ui.label(match &paint {
                Paint::Color(_) => "цвет".to_owned(),
                Paint::Texture(k) => library::label(k),
            });
            if matches!(paint, Paint::Texture(_)) && ui.small_button("цвет").clicked() {
                self.use_wall_paint(map::WallStyle::default().paint);
            }
        });
        let mut width = self.map.wall_style.width;
        let slider = ui.add(egui::Slider::new(&mut width, 0.1..=1.0).text("толщина"));
        // Протяжка ползунка — одна правка в истории, а не по одной на кадр.
        if slider.drag_started() || (slider.changed() && !slider.dragged()) {
            self.checkpoint();
        }
        self.map.wall_style.width = width;

        ui.add_space(10.0);
        self.browser(ui);
        ui.add_space(10.0);
        ui.weak("Колесо — масштаб, средняя кнопка или пробел с левой — сдвиг.");
    }

    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Библиотека");
            if ui.small_button("Папка…").on_hover_text("Папка с картинками").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                self.set_textures_dir(dir);
            }
            if ui.small_button("Пак DD…").on_hover_text("Подключить пак Dungeondraft").clicked()
                && let Some(files) = rfd::FileDialog::new().add_filter("Пак Dungeondraft", &["dungeondraft_pack"]).pick_files()
            {
                for f in files {
                    self.add_pack(f);
                }
            }
        });
        for (file, status) in &self.library.packs {
            let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            match status {
                Ok(0) if self.library.loading() => ui.weak(format!("{name}: читается…")),
                Ok(n) => ui.weak(format!("{name}: {n}")),
                Err(e) => ui.colored_label(Color32::from_rgb(230, 110, 90), e),
            };
        }
        if self.library.assets.is_empty() && !self.library.loading() {
            ui.weak("Пусто. Подключите папку с картинками или пак Dungeondraft.");
        }
        ui.horizontal(|ui| {
            for k in Kind::ALL {
                ui.selectable_value(&mut self.browse, k, k.label());
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Поиск: слова из имени или папки").desired_width(f32::INFINITY));

        let list = self.filtered().to_vec();
        ui.weak(format!("Найдено: {}", list.len()));
        let cols = ((ui.available_width() + 4.0) / (THUMB + 4.0)).floor().max(1.0) as usize;
        let rows = list.len().div_ceil(cols);
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
        };
        let mut chosen: Option<(String, bool)> = None;
        egui::ScrollArea::vertical().id_salt("assets").max_height(BROWSER_HEIGHT).auto_shrink([false, true]).show_rows(ui, THUMB + 4.0, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for &i in list.iter().skip(row * cols).take(cols) {
                        let asset = &self.library.assets[i];
                        let (key, tip) = (asset.key.clone(), format!("{}\n{}", asset.name, asset.folder));
                        let (rect, resp) = ui.allocate_exact_size(vec2(THUMB, THUMB), Sense::click());
                        self.thumb(ui.painter(), rect, &key);
                        if current.as_deref() == Some(key.as_str()) {
                            ui.painter().rect_stroke(rect, 3.0, Stroke::new(2.0, Color32::from_rgb(120, 180, 255)), egui::StrokeKind::Outside);
                        }
                        let resp = resp.on_hover_text(tip);
                        if resp.clicked() {
                            chosen = Some((key.clone(), false));
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
        if let Some((key, as_floor)) = chosen {
            match self.browse {
                _ if as_floor => self.use_floor_texture(&key),
                Kind::Floor => self.use_floor_texture(&key),
                Kind::Wall => {
                    // Встроенная полоса нарисована под свою толщину — её и ставим.
                    if let Some(width) = library::builtin_wall_width(&key) {
                        if self.map.wall_style.width != width {
                            self.checkpoint();
                            self.map.wall_style.width = width;
                        }
                    }
                    self.use_wall_paint(Paint::Texture(key));
                }
                Kind::Door => {
                    // Окна в паках FA лежат среди проёмов: вид проёма — по имени.
                    let lower = key.to_lowercase();
                    self.door_kind = if ["window", "arrowslit", "окно"].iter().any(|w| lower.contains(w)) { PortalKind::Window } else { PortalKind::Door };
                    if let Some(size) = library::size_from_name(&library::label(&key)) {
                        self.door_width = size[0].clamp(1.0, 4.0);
                    }
                    self.door_asset = Some(key);
                    self.set_tool(Tool::Door);
                }
                Kind::Object => {
                    self.object_asset = Some(key);
                    self.set_tool(Tool::Object);
                }
            }
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
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
        });
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas = rect;
        if !self.fitted && rect.width() > 50.0 {
            self.cam.fit(rect, self.map.width, self.map.height);
            self.fitted = true;
        }
        self.hover = resp.hover_pos().or(self.hover.filter(|_| resp.dragged()));

        // Колесо — масштаб вокруг курсора.
        if let Some(p) = resp.hover_pos() {
            let notches = ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit, delta, .. } => match unit {
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            _ => delta.y,
                        },
                        egui::Event::Zoom(f) if *f > 0.0 => f.ln() / WHEEL_STEP.ln(),
                        _ => 0.0,
                    })
                    .sum::<f32>()
            });
            // Ctrl с колесом у кисти — её размер, а не масштаб карты.
            let brush = matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Erase) && self.brush.shape != Tip::Cells;
            let (ctrl, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.alt));
            let prop = matches!(self.tool, Tool::Select | Tool::Object);
            if !alt {
                self.alt_wheel = false;
            }
            if notches != 0.0 && brush && ctrl {
                self.brush.radius = (self.brush.radius * 1.25f32.powf(notches)).clamp(0.25, 10.0);
            } else if notches != 0.0 && prop && alt {
                // Alt с колесом — размер выбранного объекта, а без выбора — того, что ставится.
                let k = 1.1f32.powf(notches);
                let range = |v: f32| v.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
                if let Some(Sel::Prop(i)) = self.sel {
                    // Одна правка в истории на всю прокрутку, пока держат Alt.
                    if !self.alt_wheel {
                        self.checkpoint();
                        self.alt_wheel = true;
                    }
                    self.map.props[i].scale = range(self.map.props[i].scale * k);
                } else {
                    self.object_scale = range(self.object_scale * k);
                }
            } else if notches != 0.0 {
                self.cam.zoom_at(rect, p, WHEEL_STEP.powf(notches));
            }
        }

        // Сдвиг: средняя кнопка или пробел с левой.
        let space = ui.input(|i| i.key_down(Key::Space));
        let panning = resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary));
        if panning {
            self.cam.pan += resp.drag_delta();
        } else {
            self.handle_tool(ui, &resp);
        }

        self.draw(&painter, rect);
    }

    /// Какую стену выгнет протяжка из точки: нарисованные — по отрезку между
    /// вершинами, стены комнат — по прямому участку целиком.
    pub fn bend_target(&mut self, w: [f32; 2]) -> Option<Bend> {
        let mut best: Option<(f32, Bend)> = None;
        for (i, wall) in self.map.walls.iter().enumerate() {
            for (k, (a, b)) in wall.chords().enumerate() {
                let pts = map::arc_points(a, b, wall.bend(k));
                let d = pts.windows(2).map(|p| map::segment_distance(w, p[0], p[1])).fold(f32::INFINITY, f32::min);
                if d <= PICK && best.is_none_or(|(x, _)| d < x) {
                    best = Some((d, Bend::Wall { wall: i, seg: k }));
                }
            }
        }
        if best.is_some() {
            return best.map(|(_, b)| b);
        }
        // Прямой участок вокруг ближайшего отрезка: соседи в том же направлении.
        for line in mask::join(self.room_edges().to_vec()) {
            let Some((k, d)) = line.windows(2).enumerate().map(|(k, p)| (k, map::segment_distance(w, p[0], p[1]))).min_by(|x, y| x.1.total_cmp(&y.1)) else { continue };
            if d > PICK || best.is_some_and(|(x, _)| d >= x) {
                continue;
            }
            let dir = |p: [f32; 2], q: [f32; 2]| {
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                [dx / l, dy / l]
            };
            let d0 = dir(line[k], line[k + 1]);
            let same = |p: [f32; 2], q: [f32; 2]| {
                let e = dir(p, q);
                d0[0] * e[0] + d0[1] * e[1] > 0.9995
            };
            let (mut s, mut e) = (k, k + 1);
            while s > 0 && same(line[s - 1], line[s]) {
                s -= 1;
            }
            while e + 1 < line.len() && same(line[e], line[e + 1]) {
                e += 1;
            }
            best = Some((d, Bend::Room { a: line[s], b: line[e], h: 0.0 }));
        }
        best.map(|(_, b)| b)
    }

    /// Концы выгибаемого участка.
    fn bend_chord(&self, bend: Bend) -> ([f32; 2], [f32; 2]) {
        match bend {
            Bend::Room { a, b, .. } => (a, b),
            Bend::Wall { wall, seg } => self.map.walls[wall].chords().nth(seg).unwrap_or(([0.0; 2], [0.0; 2])),
        }
    }

    /// Прогиб под курсором: расстояние от хорды, плюс — влево по ходу.
    /// С привязкой — шагом в четверть клетки.
    fn bend_at(&self, bend: Bend, w: [f32; 2], snap: bool) -> f32 {
        let (a, b) = self.bend_chord(bend);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let h = ((w[0] - mid[0]) * dy - (w[1] - mid[1]) * dx) / len;
        let h = if snap { (h * 4.0).round() / 4.0 } else { h };
        let m = map::max_bend(a, b);
        h.clamp(-m, m)
    }

    /// Отпустили: дуга стены комнаты ложится в маску тем полом, что у стены внутри.
    pub fn finish_bend(&mut self) {
        let Some(Bend::Room { a, b, h }) = self.bending.take() else { return };
        if h.abs() < 0.05 {
            return;
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let material = [0.3f32, -0.3]
            .iter()
            .map(|k| [mid[0] + dy / len * k, mid[1] - dx / len * k])
            .map(|p| self.map.room_at(p[0].floor() as i32, p[1].floor() as i32))
            .find(|&m| m > 0)
            .map_or(self.material, |m| m as usize - 1);
        self.checkpoint();
        self.map.bend_room(a, b, h, material);
        self.edges_dirty = true;
    }

    /// Ctrl+Shift с протяжкой гнёт стену дугой. `true` — ввод занят изгибом.
    fn handle_bend(&mut self, ui: &egui::Ui, resp: &egui::Response, pointer: Option<Pos2>) -> bool {
        let rect = self.canvas;
        let (bend_keys, alt) = ui.input(|i| (i.modifiers.command && i.modifiers.shift, i.modifiers.alt));
        if self.bending.is_none() {
            if !bend_keys {
                return false;
            }
            if resp.drag_started_by(PointerButton::Primary)
                && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
            {
                self.bending = self.bend_target(self.cam.to_world(rect, o));
                if matches!(self.bending, Some(Bend::Wall { .. })) {
                    self.checkpoint();
                }
            }
            return true;
        }
        let Some(bend) = self.bending else { return true };
        if let Some(p) = pointer
            && resp.dragged_by(PointerButton::Primary)
        {
            let h = self.bend_at(bend, self.cam.to_world(rect, p), self.settings.snap != alt);
            match bend {
                Bend::Room { a, b, .. } => self.bending = Some(Bend::Room { a, b, h }),
                Bend::Wall { wall, seg } => self.map.walls[wall].set_bend(seg, h),
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.finish_bend();
            self.bending = None;
        }
        true
    }

    fn handle_tool(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let rect = self.canvas;
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let alt = ui.input(|i| i.modifiers.alt);
        if self.handle_bend(ui, resp, pointer) {
            return;
        }
        match self.tool {
            Tool::Floor | Tool::Erase | Tool::Dungeon if self.brush.shape != Tip::Cells => {
                // Мазок начинается сразу по нажатию, а не после порога протяжки.
                let (pressed, down, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
                if pressed
                    && resp.contains_pointer()
                    && let Some(p) = at
                {
                    self.begin_stroke(self.cam.to_world(rect, p));
                } else if self.stroke_last.is_some() {
                    match (down, at) {
                        (true, Some(p)) => self.stroke_to(self.cam.to_world(rect, p)),
                        _ => self.end_stroke(),
                    }
                }
            }
            Tool::Floor | Tool::Erase | Tool::Dungeon => {
                if resp.drag_started_by(PointerButton::Primary) {
                    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
                    self.drag_from = origin.map(|p| self.cam.cell(rect, p));
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    if let (Some(a), Some(p)) = (self.drag_from.take(), pointer) {
                        let b = self.cam.cell(rect, p);
                        self.paint(a, b);
                    }
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let c = self.cam.cell(rect, p);
                    self.paint(c, c);
                }
            }
            Tool::Wall => {
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    let node = self.cam.node(rect, p);
                    if self.draft.len() >= 3 && self.draft[0] == node {
                        self.finish_wall(true);
                    } else if self.draft.last() != Some(&node) {
                        self.draft.push(node);
                    }
                }
                if resp.double_clicked() || resp.secondary_clicked() {
                    self.finish_wall(false);
                }
            }
            Tool::Door => {
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    self.place_portal(self.cam.to_world(rect, p));
                }
            }
            Tool::Object => {
                self.drag_prop(ui, resp, pointer, alt);
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    if self.object_asset.is_none() {
                        self.browse = Kind::Object;
                    }
                    self.place_prop(self.cam.to_world(rect, p), alt);
                }
            }
            Tool::Select => {
                self.drag_prop(ui, resp, pointer, alt);
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    self.sel = self.pick(self.cam.to_world(rect, p));
                }
            }
        }
    }

    /// Ручка выбранного объекта под точкой экрана.
    fn grip_at(&self, screen: Pos2) -> Option<(usize, Grip)> {
        let Some(Sel::Prop(i)) = self.sel else { return None };
        let prop = &self.map.props[i];
        let (corners, turn) = grips(prop, TURN_LIFT_PX / self.cam.scale);
        let near = |w: [f32; 2]| self.cam.to_screen(self.canvas, w).distance(screen) <= GRIP_PX + 3.0;
        if near(turn) {
            return Some((i, Grip::Turn));
        }
        if corners.iter().any(|&c| near(c)) {
            let w = self.cam.to_world(self.canvas, screen);
            let d = ((w[0] - prop.pos[0]).powi(2) + (w[1] - prop.pos[1]).powi(2)).sqrt().max(1e-3);
            return Some((i, Grip::Scale(prop.scale, d)));
        }
        None
    }

    /// Протяжка объекта: за ручку — размер или поворот, за тело — сдвиг.
    fn drag_prop(&mut self, ui: &egui::Ui, resp: &egui::Response, pointer: Option<Pos2>, alt: bool) {
        let rect = self.canvas;
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        {
            let w = self.cam.to_world(rect, o);
            let grip = self.grip_at(o).or_else(|| match self.pick(w) {
                Some(Sel::Prop(i)) => {
                    let pos = self.map.props[i].pos;
                    Some((i, Grip::Move([w[0] - pos[0], w[1] - pos[1]])))
                }
                _ => None,
            });
            if let Some((i, g)) = grip {
                self.checkpoint();
                self.moving = Some((i, g));
                self.sel = Some(Sel::Prop(i));
            }
        }
        if let (Some((i, grip)), Some(p)) = (self.moving, pointer)
            && resp.dragged_by(PointerButton::Primary)
        {
            let w = self.cam.to_world(rect, p);
            let snap = self.settings.snap != alt;
            let prop = &mut self.map.props[i];
            let (dx, dy) = (w[0] - prop.pos[0], w[1] - prop.pos[1]);
            match grip {
                Grip::Move(grab) => {
                    let pos = [w[0] - grab[0], w[1] - grab[1]];
                    prop.pos = if snap { [snap_half(pos[0]), snap_half(pos[1])] } else { pos };
                }
                Grip::Scale(s0, d0) => {
                    let s = s0 * (dx * dx + dy * dy).sqrt() / d0;
                    // С привязкой — шагом в четверть.
                    let s = if snap { (s * 4.0).round() / 4.0 } else { s };
                    prop.scale = s.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
                }
                Grip::Turn => {
                    // При нулевом повороте ручка смотрит вверх.
                    let a = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
                    prop.rot = if snap { ((a / 15.0).round() * 15.0) % 360.0 } else { a };
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.moving = None;
        }
    }

    fn draw(&mut self, painter: &egui::Painter, canvas: Rect) {
        let cam = self.cam;
        let s = cam.scale;
        let (mw, mh) = (self.map.width as i32, self.map.height as i32);
        let map_rect = cam.map_rect(canvas, self.map.width, self.map.height);
        painter.rect_filled(map_rect, 0.0, Color32::from_gray(38));

        // Рисуется только видимая часть: на большой карте при сильном приближении
        // остальное всё равно за краем окна.
        let (tl, br) = (cam.to_world(canvas, canvas.min), cam.to_world(canvas, canvas.max));
        let view = Rect::from_min_max(pos2(tl[0], tl[1]), pos2(br[0], br[1]));
        let (x0, x1) = ((tl[0].floor() as i32).max(0), (br[0].ceil() as i32).min(mw));
        let (y0, y1) = ((tl[1].floor() as i32).max(0), (br[1].ceil() as i32).min(mh));

        // Пол: местность, затем пол комнат, обрезанный по самим комнатам.
        let terrain = std::mem::take(&mut self.map.terrain);
        let room_floor = std::mem::take(&mut self.map.room_floor);
        for (k, coat) in terrain.iter().enumerate() {
            self.draw_coat(painter, canvas, coat, &terrain[k + 1..], None, view);
        }
        let rooms = std::mem::replace(&mut self.map.rooms, Mask { w: 0, h: 0, data: Vec::new() });
        for (k, coat) in room_floor.iter().enumerate() {
            self.draw_coat(painter, canvas, coat, &room_floor[k + 1..], Some(&rooms), view);
        }
        self.map.terrain = terrain;
        self.map.room_floor = room_floor;
        self.map.rooms = rooms;

        // Протянутая заливка — рамкой, пока кнопку не отпустили.
        if let (Some(a), Some(p)) = (self.drag_from, self.hover) {
            let b = cam.cell(canvas, p);
            let r = Rect::from_min_max(
                cam.to_screen(canvas, [a.0.min(b.0) as f32, a.1.min(b.1) as f32]),
                cam.to_screen(canvas, [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0]),
            )
            .intersect(map_rect);
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(120, 180, 255)), egui::StrokeKind::Inside);
        }

        // Сетка. На мелком масштабе она сливается в сплошной фон — тогда её нет.
        if s >= 10.0 {
            let grid = Stroke::new(1.0, Color32::from_black_alpha(60));
            for x in x0..=x1 {
                painter.line_segment([cam.to_screen(canvas, [x as f32, y0 as f32]), cam.to_screen(canvas, [x as f32, y1 as f32])], grid);
            }
            for y in y0..=y1 {
                painter.line_segment([cam.to_screen(canvas, [x0 as f32, y as f32]), cam.to_screen(canvas, [x1 as f32, y as f32])], grid);
            }
        }
        painter.rect_stroke(map_rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Outside);

        // Объекты — под стенами, как мебель у стены.
        let props = self.map.props.clone();
        for (i, prop) in props.iter().enumerate() {
            let c = prop.corners();
            let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1])));
            if !bounds.intersects(view) {
                continue;
            }
            self.draw_prop(painter, canvas, prop, Color32::WHITE);
            if self.sel == Some(Sel::Prop(i)) {
                let q: Vec<Pos2> = c.iter().map(|&p| cam.to_screen(canvas, p)).collect();
                painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
            }
        }

        // Стены комнат и нарисованные вручную — за вычетом проёмов.
        let style = self.map.wall_style.clone();
        let edges = map::cut_portals(&self.room_edges().to_vec(), &self.map.portals);
        self.draw_walls(painter, canvas, &style, &edges);
        let walls = self.map.walls.clone();
        for (i, wall) in walls.iter().enumerate() {
            let segs: Vec<Segment> = wall.segments().collect();
            if self.sel == Some(Sel::Wall(i)) {
                for (a, b) in &segs {
                    painter.line_segment([cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], Stroke::new(style.width * s + 6.0, Color32::from_rgb(120, 180, 255)));
                }
            }
            self.draw_walls(painter, canvas, &style, &map::cut_portals(&segs, &self.map.portals));
        }

        let portals = self.map.portals.clone();
        for (i, portal) in portals.iter().enumerate() {
            self.draw_portal(painter, canvas, portal, style.width);
            if self.sel == Some(Sel::Portal(i)) {
                painter.line_segment([cam.to_screen(canvas, portal.a), cam.to_screen(canvas, portal.b)], Stroke::new(3.0, Color32::from_rgb(120, 180, 255)));
            }
        }

        // Изгиб: какую стену возьмёт протяжка и куда ляжет дуга стены комнаты.
        let blue = Color32::from_rgb(120, 180, 255);
        match self.bending {
            Some(Bend::Room { a, b, h }) => {
                let q: Vec<Pos2> = map::arc_points(a, b, h).iter().map(|&p| cam.to_screen(canvas, p)).collect();
                painter.add(Shape::line(q, Stroke::new(3.0, blue)));
            }
            Some(_) => {}
            None => {
                let keys = painter.ctx().input(|i| i.modifiers.command && i.modifiers.shift);
                if keys && let Some(h) = self.hover.filter(|p| canvas.contains(*p)) {
                    let target = self.bend_target(cam.to_world(canvas, h));
                    if let Some(t) = target {
                        let (a, b) = self.bend_chord(t);
                        let k = match t {
                            Bend::Wall { wall, seg } => self.map.walls[wall].bend(seg),
                            Bend::Room { .. } => 0.0,
                        };
                        let q: Vec<Pos2> = map::arc_points(a, b, k).iter().map(|&p| cam.to_screen(canvas, p)).collect();
                        painter.add(Shape::line(q, Stroke::new(3.0, blue)));
                    }
                }
            }
        }

        // Ручки выбранного объекта — поверх стен и всего остального.
        if let Some(Sel::Prop(i)) = self.sel
            && matches!(self.tool, Tool::Select | Tool::Object)
        {
            let (corners, turn) = grips(&self.map.props[i], TURN_LIFT_PX / s);
            let q: Vec<Pos2> = corners.iter().map(|&p| cam.to_screen(canvas, p)).collect();
            painter.add(Shape::closed_line(q.clone(), Stroke::new(1.5, blue)));
            let t = cam.to_screen(canvas, turn);
            painter.line_segment([q[0].lerp(q[1], 0.5), t], Stroke::new(1.5, blue));
            painter.circle(t, GRIP_PX * 0.8, Color32::from_gray(30), Stroke::new(2.0, blue));
            for c in q {
                painter.rect(Rect::from_center_size(c, vec2(GRIP_PX * 1.6, GRIP_PX * 1.6)), 2.0, Color32::from_gray(30), Stroke::new(2.0, blue), egui::StrokeKind::Middle);
            }
        }

        // Призрак того, что поставит щелчок.
        let hover = self.hover.filter(|p| canvas.contains(*p));
        match (self.tool, hover) {
            (Tool::Object, Some(h)) => {
                if let Some(asset) = self.object_asset.clone() {
                    let w = cam.to_world(canvas, h);
                    let alt = painter.ctx().input(|i| i.modifiers.alt);
                    let pos = self.snap_pos(w, alt);
                    let size = self.asset_size(&asset);
                    let ghost = Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale };
                    self.draw_prop(painter, canvas, &ghost, Color32::from_white_alpha(150));
                }
            }
            (Tool::Dungeon | Tool::Floor | Tool::Erase, Some(h)) if self.brush.shape != Tip::Cells => {
                // Контур кисти: куда ляжет мазок.
                let r = self.brush.radius * s;
                let color = if self.tool == Tool::Erase { Color32::from_rgb(255, 130, 110) } else { Color32::from_rgb(120, 180, 255) };
                let stroke = Stroke::new(1.5, color);
                match self.brush.shape {
                    Tip::Square => {
                        painter.rect_stroke(Rect::from_center_size(h, vec2(2.0 * r, 2.0 * r)), 0.0, stroke, egui::StrokeKind::Middle);
                    }
                    _ => {
                        painter.circle_stroke(h, r, stroke);
                    }
                }
                if self.brush.hardness < 0.95 {
                    let inner = (r * self.brush.hardness).max(2.0);
                    painter.circle_stroke(h, inner, Stroke::new(1.0, color.gamma_multiply(0.5)));
                }
            }
            (Tool::Wall, Some(h)) => {
                painter.circle_stroke(cam.to_screen(canvas, cam.node(canvas, h)), 6.0, Stroke::new(1.5, Color32::from_rgb(255, 200, 90)));
            }
            _ => {}
        }

        // Начатая стена и отрезок до курсора.
        let outer = (style.width * s).max(3.0);
        if !self.draft.is_empty() {
            let mut points: Vec<Pos2> = self.draft.iter().map(|&p| cam.to_screen(canvas, p)).collect();
            if let Some(h) = hover {
                points.push(cam.to_screen(canvas, cam.node(canvas, h)));
            }
            painter.add(Shape::line(points.clone(), Stroke::new(outer * 0.6, Color32::from_rgb(255, 200, 90))));
            for p in &points[..self.draft.len()] {
                painter.circle_filled(*p, 4.0, Color32::from_rgb(255, 200, 90));
            }
        }
    }

    /// Картинка, натянутая на четыре угла (в клетках).
    fn quad(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, corners: [[f32; 2]; 4], uv: [Pos2; 4], tint: Color32) -> bool {
        let Some((tex, _)) = self.textures.get(key, false) else { return false };
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (p, uv) in corners.iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex { pos: self.cam.to_screen(canvas, *p), uv, color: tint });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        true
    }

    fn draw_prop(&mut self, painter: &egui::Painter, canvas: Rect, prop: &Prop, tint: Color32) {
        let c = prop.corners();
        let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        if !self.quad(painter, canvas, &prop.asset, c, full, tint) {
            // Картинка ещё грузится или не нашлась.
            let q: Vec<Pos2> = c.iter().map(|&p| self.cam.to_screen(canvas, p)).collect();
            let color = if self.textures.failed(&prop.asset, false) { Color32::from_rgb(200, 60, 60) } else { Color32::from_gray(140) };
            painter.add(Shape::convex_polygon(q, Color32::from_black_alpha(60), Stroke::new(1.0, color)));
        }
    }

    /// Дверь — картинкой вдоль проёма или простой плашкой; окно — светлой полосой.
    fn draw_portal(&mut self, painter: &egui::Painter, canvas: Rect, p: &Portal, wall_width: f32) {
        let (dx, dy) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let rect_around = |half: f32| {
            let (nx, ny) = (-uy * half, ux * half);
            [[p.a[0] + nx, p.a[1] + ny], [p.b[0] + nx, p.b[1] + ny], [p.b[0] - nx, p.b[1] - ny], [p.a[0] - nx, p.a[1] - ny]]
        };
        let edge = Stroke::new(1.5, Color32::from_rgb(20, 16, 13));
        let to_screen = |c: [[f32; 2]; 4], cam: &Camera| c.iter().map(|&w| cam.to_screen(canvas, w)).collect::<Vec<_>>();
        match (p.kind, &p.asset) {
            (_, Some(key)) => {
                // Толщина — по пропорциям картинки: дверь 3×1 в проёме в клетку — треть клетки.
                let half = match self.textures.get(key, false) {
                    Some((_, px)) => len * px[1] as f32 / px[0].max(1) as f32 / 2.0,
                    None => wall_width / 2.0,
                };
                let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
                if !self.quad(painter, canvas, key, rect_around(half), full, Color32::WHITE) {
                    painter.add(Shape::convex_polygon(to_screen(rect_around(half), &self.cam), Color32::from_black_alpha(60), Stroke::new(1.0, Color32::from_gray(140))));
                }
            }
            (PortalKind::Door, None) => {
                painter.add(Shape::convex_polygon(to_screen(rect_around(wall_width * 0.35), &self.cam), Color32::from_rgb(132, 92, 54), edge));
            }
            (PortalKind::Window, None) => {
                painter.add(Shape::convex_polygon(to_screen(rect_around(wall_width * 0.3), &self.cam), Color32::from_rgb(150, 196, 220), edge));
            }
        }
    }

    /// Слой материала: сетка четырёхугольников по точкам маски, прозрачность вершины —
    /// сила слоя в ней. Края мягкой кисти так и получаются мягкими. `clip` — маска,
    /// за которую слой не выходит (пол комнат — за саму комнату). `above` — слои,
    /// которые лягут поверх: сила в маске — доля слоя в точке, а рисуются слои
    /// друг на друга, поэтому прозрачность пересчитывается (см. [`map::coat_alpha`]).
    fn draw_coat(&mut self, painter: &egui::Painter, canvas: Rect, coat: &Coat, above: &[Coat], clip: Option<&Mask>, view: Rect) {
        let Some(material) = self.map.materials.get(coat.material as usize) else { return };
        let m = &coat.mask;
        let res = RES as f32;
        // Мельче четырёх точек экрана четырёхугольник не дробится: дальше глаз не различит,
        // а на большой карте издалека это сотни тысяч лишних вершин.
        let px = self.cam.scale / res;
        let step = [1usize, 2, 4, 8].into_iter().find(|&s| px * s as f32 >= 4.0).unwrap_or(8);
        let clampi = |v: f32, hi: usize| (v.floor() as isize).clamp(-1, hi as isize);
        let (i0, i1) = (clampi(view.min.x * res - 1.0, m.w), clampi(view.max.x * res + 1.0, m.w));
        let (j0, j1) = (clampi(view.min.y * res - 1.0, m.h), clampi(view.max.y * res + 1.0, m.h));
        // Начало шага кратно шагу: при сдвиге карты сетка не «плывёт».
        let (i0, j0) = (i0 - i0.rem_euclid(step as isize), j0 - j0.rem_euclid(step as isize));
        let (tex, tile, tint): (Option<egui::TextureId>, [f32; 2], [u8; 3]) = match &material.paint {
            Paint::Color(c) => (None, [1.0, 1.0], *c),
            Paint::Texture(key) => {
                let key = if material.vary { format!("{}{key}", library::VARY) } else { key.clone() };
                let failed = self.textures.failed(&key, false);
                match self.textures.get(&key, false) {
                    // Плитка — по размеру картинки, 256 точек на клетку, как в Dungeondraft.
                    Some((t, px)) => (Some(t.id()), [(px[0] as f32 / PX_PER_CELL).max(MIN_TILE), (px[1] as f32 / PX_PER_CELL).max(MIN_TILE)], [255, 255, 255]),
                    None if failed => (None, [1.0, 1.0], [160, 40, 40]),
                    None => (None, [1.0, 1.0], [60, 60, 60]),
                }
            }
        };
        let share = |mask: &Mask, i: isize, j: isize| {
            let v = mask.get(i, j);
            clip.map_or(v, |c| v.min(c.get(i, j)))
        };
        let value = |i: isize, j: isize| {
            let v = share(m, i, j);
            if v == 0 {
                return 0;
            }
            map::coat_alpha(v, above.iter().map(|c| share(&c.mask, i, j) as u32).sum())
        };
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        let cam = self.cam;
        // Вершина в точке маски с дробными номерами (fx, fy) и силой `alpha`.
        let vertex = |fx: f32, fy: f32, alpha: u8| {
            let w = [(fx + 0.5) / res, (fy + 0.5) / res];
            let color = match tex {
                Some(_) => Color32::from_white_alpha(alpha),
                None => Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], alpha),
            };
            // Цветной слой берёт белую точку атласа egui, текстурный — плитку.
            let uv = if tex.is_some() { pos2(w[0] / tile[0], w[1] / tile[1]) } else { egui::epaint::WHITE_UV };
            egui::epaint::Vertex { pos: cam.to_screen(canvas, w), uv, color }
        };
        // Вблизи четырёхугольник у края дробится мельче, сила между точками маски —
        // по кривой Катмулла — Рома через соседние точки. По прямой между четырьмя
        // углами край ложится ступеньками в шаг маски, по кривой — плавной линией.
        let detail = if step == 1 { ((px / 6.0).ceil() as usize).clamp(1, 8) } else { 1 };
        let weights: Vec<[f32; 4]> = (0..=detail)
            .map(|k| {
                let t = k as f32 / detail as f32;
                let (t2, t3) = (t * t, t * t * t);
                [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
            })
            .collect();
        let s = step as isize;
        let mut j = j0;
        while j < j1 {
            let mut i = i0;
            while i < i1 {
                let corners = [(i, j), (i + s, j), (i + s, j + s), (i, j + s)];
                let a = corners.map(|(x, y)| value(x, y));
                if a != [0; 4] {
                    let mut near = [[0f32; 4]; 4];
                    if detail > 1 {
                        for (dy, row) in near.iter_mut().enumerate() {
                            for (dx, v) in row.iter_mut().enumerate() {
                                *v = value(i + dx as isize - 1, j + dy as isize - 1) as f32;
                            }
                        }
                    }
                    // Дробится не только сам край, но и соседи: кривая берёт точки на шаг
                    // дальше, и стык мелкого с крупным иначе дал бы щель.
                    let flat = near.iter().flatten().all(|&v| v == near[0][0]);
                    let base = mesh.vertices.len() as u32;
                    if detail > 1 && !flat {
                        let n = detail;
                        for (ky, wy) in weights.iter().enumerate() {
                            for (kx, wx) in weights.iter().enumerate() {
                                let sum: f32 = (0..4).map(|r| wy[r] * (0..4).map(|c| wx[c] * near[r][c]).sum::<f32>()).sum();
                                let alpha = sum.round().clamp(0.0, 255.0) as u8;
                                mesh.vertices.push(vertex(i as f32 + kx as f32 / n as f32, j as f32 + ky as f32 / n as f32, alpha));
                            }
                        }
                        let row = (n + 1) as u32;
                        for ky in 0..n as u32 {
                            for kx in 0..n as u32 {
                                let q = base + ky * row + kx;
                                mesh.add_triangle(q, q + 1, q + row + 1);
                                mesh.add_triangle(q, q + row + 1, q + row);
                            }
                        }
                    } else {
                        for ((x, y), alpha) in corners.into_iter().zip(a) {
                            mesh.vertices.push(vertex(x as f32, y as f32, alpha));
                        }
                        if a == [255; 4] && detail > 1 {
                            // Сплошной кусок рядом с дроблёным чуть шире — на полпикселя,
                            // чтобы на стыке не проглядывал фон.
                            let k = 0.5 / (px * s as f32);
                            let c = (mesh.vertices[base as usize].pos + mesh.vertices[base as usize + 2].pos.to_vec2()) / 2.0;
                            for v in &mut mesh.vertices[base as usize..] {
                                v.pos = c + (v.pos - c) * (1.0 + 2.0 * k);
                            }
                        }
                        mesh.add_triangle(base, base + 1, base + 2);
                        mesh.add_triangle(base, base + 2, base + 3);
                    }
                }
                i += s;
            }
            j += s;
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Стены лентой: отрезки склеиваются в ломаные, у каждой вершины лента
    /// поворачивает по биссектрисе — без нахлёстов и швов. Обводка — по двум краям
    /// ленты, текстура идёт вдоль без разрывов; на замкнутой стене её шаг подогнан,
    /// чтобы целое число повторов сошлось на стыке.
    fn draw_walls(&mut self, painter: &egui::Painter, canvas: Rect, style: &map::WallStyle, segs: &[Segment]) {
        let cam = self.cam;
        let hw = style.width / 2.0;
        let edge = Stroke::new(1.5, Color32::from_rgb(20, 16, 13));
        let (tex, aspect, fill) = match &style.paint {
            Paint::Color(c) => (None, 4.0, rgb(*c)),
            Paint::Texture(key) => match self.textures.get(key, false) {
                Some((t, px)) => (Some(t.id()), px[0] as f32 / px[1].max(1) as f32, Color32::WHITE),
                None => (None, 4.0, Color32::from_gray(70)),
            },
        };
        if let (Some(id), Paint::Texture(key)) = (tex, &style.paint) {
            if let Some(n) = library::builtin_wall_stamps(key) {
                self.draw_stamped(painter, canvas, style.width, id, n, segs);
                return;
            }
        }
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        // Заливка ляжет под обводку: место под неё занято заранее.
        let slot = painter.add(Shape::Noop);
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let rows = ribbon(&line, closed, hw);
            if rows.len() < 2 {
                continue;
            }
            let total = rows[rows.len() - 1].2;
            let mut repeat = style.width * aspect;
            if closed {
                repeat = total / (total / repeat).round().max(1.0);
            }
            // У каждого участка свои вершины. На скосе угла внешняя и внутренняя точки
            // ряда не напротив друг друга: с общей координатой по длине участок —
            // трапеция из двух треугольников, и текстура в ней ломалась ёлочкой.
            // Координата — проекция точки на направление участка: внутри участка
            // рисунок лежит ровно, а на углу срезается по скосу, как в Dungeondraft.
            for k in 0..rows.len() - 1 {
                let ((l0, r0, d0), (l1, r1, _)) = (rows[k], rows[k + 1]);
                let mid = |l: [f32; 2], r: [f32; 2]| [(l[0] + r[0]) / 2.0, (l[1] + r[1]) / 2.0];
                let (c0, c1) = (mid(l0, r0), mid(l1, r1));
                let (dx, dy) = (c1[0] - c0[0], c1[1] - c0[1]);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let t = [dx / len, dy / len];
                let along = |p: [f32; 2]| d0 + (p[0] - c0[0]) * t[0] + (p[1] - c0[1]) * t[1];
                let base = mesh.vertices.len() as u32;
                for (p, v) in [(l0, 0.0), (r0, 1.0), (l1, 0.0), (r1, 1.0)] {
                    let uv = if tex.is_some() { pos2(along(p) / repeat, v) } else { egui::epaint::WHITE_UV };
                    mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color: fill });
                }
                mesh.add_triangle(base, base + 1, base + 3);
                mesh.add_triangle(base, base + 3, base + 2);
            }
            // У текстурной стены обводка нарисована в самой полосе (как в паках FA),
            // и у рваной кромки прямая линия поверх только мешает.
            if tex.is_some() {
                continue;
            }
            let left: Vec<Pos2> = rows.iter().map(|(l, _, _)| cam.to_screen(canvas, *l)).collect();
            let right: Vec<Pos2> = rows.iter().map(|(_, r, _)| cam.to_screen(canvas, *r)).collect();
            if closed {
                painter.add(Shape::closed_line(left[..left.len() - 1].to_vec(), edge));
                painter.add(Shape::closed_line(right[..right.len() - 1].to_vec(), edge));
            } else {
                // Открытая — одним контуром: левый край, торец, правый край назад.
                let mut outline = left;
                outline.extend(right.into_iter().rev());
                painter.add(Shape::closed_line(outline, edge));
            }
        }
        if !mesh.is_empty() {
            painter.set(slot, Shape::mesh(mesh));
        }
    }

    /// Стена из отдельных штук — брёвен частокола. Полоса из кругов на изгибе
    /// сжимается и режет брёвна, поэтому каждое бревно — свой квадрат: шаг вдоль
    /// стены ровный, в каждом остром углу стоит бревно. Под ними тёмная лента,
    /// чтобы между кругами не просвечивал пол.
    fn draw_stamped(&mut self, painter: &egui::Painter, canvas: Rect, width: f32, id: egui::TextureId, n: u32, segs: &[Segment]) {
        let cam = self.cam;
        let mut under = egui::Mesh::default();
        let mut logs = egui::Mesh::with_texture(id);
        let bark = Color32::from_rgb(30, 21, 14);
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let rows = ribbon(&line, closed, width * 0.28);
            for k in 0..rows.len().saturating_sub(1) {
                let base = under.vertices.len() as u32;
                for p in [rows[k].0, rows[k].1, rows[k + 1].0, rows[k + 1].1] {
                    under.colored_vertex(cam.to_screen(canvas, p), bark);
                }
                under.add_triangle(base, base + 1, base + 3);
                under.add_triangle(base, base + 3, base + 2);
            }
            let r = width / 2.0;
            for c in stamp_points(&line, closed, width * 0.9) {
                // Поворот и вид бревна — от места: при правке соседних стен не прыгают.
                let h = ((c[0] * 73.13).round() as i64).wrapping_mul(31).wrapping_add((c[1] * 91.7).round() as i64).wrapping_mul(2654435761) as u64;
                let a = (h % 6283) as f32 / 1000.0;
                let v = ((h >> 16) % n as u64) as f32;
                let (ax, ay) = ([a.cos() * r, a.sin() * r], [-a.sin() * r, a.cos() * r]);
                let base = logs.vertices.len() as u32;
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let p = [c[0] + ax[0] * sx + ay[0] * sy, c[1] + ax[1] * sx + ay[1] * sy];
                    let uv = pos2((v + (sx + 1.0) / 2.0) / n as f32, (sy + 1.0) / 2.0);
                    logs.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color: Color32::WHITE });
                }
                logs.add_triangle(base, base + 1, base + 3);
                logs.add_triangle(base, base + 3, base + 2);
            }
        }
        painter.add(Shape::mesh(under));
        painter.add(Shape::mesh(logs));
    }

    /// `--shot файл.png`: снять окно, когда оно устоялось, и выйти.
    fn screenshots(&mut self, ctx: &egui::Context) {
        let images: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        if let Some(test) = &mut self.test {
            for image in &images {
                test.screenshot(image);
            }
            return;
        }
        let Some(path) = &self.shot else { return };
        if let Some(image) = images.first() {
            if let Err(e) = save_png(path, image) {
                eprintln!("снимок не сохранён: {e}");
            }
            self.recovery.close();
            std::process::exit(0);
        } else if !self.shot_asked && self.frame >= 10 && !self.library.loading() && self.textures.idle() {
            self.shot_asked = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame_ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Сюда доходят, только закрыв окно по-хорошему: карта сохранена или от правок
        // отказались. Копия на случай сбоя больше не нужна.
        self.recovery.close();
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(test) = &mut self.test {
            test.feed(raw);
        }
    }
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn ago(secs: u64) -> String {
    match secs {
        0..60 => "только что".into(),
        60..3600 => format!("{} мин назад", secs / 60),
        3600..86400 => format!("{} ч назад", secs / 3600),
        _ => format!("{} дн назад", secs / 86400),
    }
}

pub fn save_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = image.size;
    let raw: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    image::save_buffer(path, &raw, w as u32, h as u32, image::ColorType::Rgba8).map_err(|e| e.to_string())
}

fn item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let mut b = egui::Button::new(label);
    if !shortcut.is_empty() {
        b = b.shortcut_text(shortcut);
    }
    let clicked = ui.add(b).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Левый и правый край ленты вдоль ломаной и путь от её начала. В вершине край
/// сдвигается по биссектрисе на столько, чтобы толщина везде была `hw` (на остром
/// изломе — не дальше трёх толщин). Открытые концы вытянуты на `hw`, чтобы стены
/// сходились без щелей; у замкнутой последняя строка повторяет первую.
/// Где встанут брёвна: острые углы (поворот больше 55° на коротком отрезке —
/// срезанный угол контура тоже угол) получают бревно, между ними шаг
/// подгоняется, чтобы брёвна легли ровно от угла до угла.
fn stamp_points(line: &[[f32; 2]], closed: bool, step: f32) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last().is_none_or(|q: &[f32; 2]| (q[0] - p[0]).abs() > 1e-5 || (q[1] - p[1]).abs() > 1e-5) {
            pts.push(p);
        }
    }
    if closed && pts.len() > 1 {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return pts;
    }
    let len = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let ang = |a: [f32; 2], b: [f32; 2]| (b[1] - a[1]).atan2(b[0] - a[0]);
    // Поворот в каждой вершине и путь до неё.
    let mut turn = vec![0.0f32; n];
    let mut at = vec![0.0f32; n];
    for i in 0..n {
        if i > 0 {
            at[i] = at[i - 1] + len(pts[i - 1], pts[i]);
        }
        if closed || (i > 0 && i + 1 < n) {
            let (p, q, r) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let mut d = ang(q, r) - ang(p, q);
            while d > std::f32::consts::PI {
                d -= std::f32::consts::TAU;
            }
            while d < -std::f32::consts::PI {
                d += std::f32::consts::TAU;
            }
            turn[i] = d;
        }
    }
    let total = at[n - 1] + if closed { len(pts[n - 1], pts[0]) } else { 0.0 };
    let gap = |i: usize, j: usize| {
        let d = (at[i] - at[j]).abs();
        if closed { d.min(total - d) } else { d }
    };
    let win = step * 0.45;
    let mut corners: Vec<usize> = (0..n)
        .filter(|&i| {
            let near: Vec<usize> = (0..n).filter(|&j| gap(i, j) <= win).collect();
            let sum: f32 = near.iter().map(|&j| turn[j]).sum();
            sum.abs() > 55f32.to_radians() && near.iter().all(|&j| turn[j].abs() < turn[i].abs() || (turn[j].abs() == turn[i].abs() && j >= i))
        })
        .collect();
    if !closed {
        corners.insert(0, 0);
        corners.push(n - 1);
    }
    let point_at = |d: f32| -> [f32; 2] {
        let d = if closed { d.rem_euclid(total) } else { d.clamp(0.0, total) };
        let k = at.partition_point(|&x| x <= d).saturating_sub(1);
        let (a, b) = (pts[k], pts[(k + 1) % n]);
        let l = len(a, b).max(1e-6);
        let t = ((d - at[k]) / l).clamp(0.0, 1.0);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    };
    let mut out = Vec::new();
    if corners.is_empty() {
        let m = (total / step).round().max(3.0) as usize;
        return (0..m).map(|j| point_at(total * j as f32 / m as f32)).collect();
    }
    let runs = if closed { corners.len() } else { corners.len() - 1 };
    for k in 0..runs {
        let (s, e) = (at[corners[k]], at[corners[(k + 1) % corners.len()]]);
        let l = if e > s { e - s } else { e + total - s };
        let m = (l / step).round().max(1.0) as usize;
        for j in 0..m {
            out.push(point_at(s + l * j as f32 / m as f32));
        }
    }
    if !closed {
        out.push(pts[n - 1]);
    }
    out
}

fn ribbon(line: &[[f32; 2]], closed: bool, hw: f32) -> Vec<([f32; 2], [f32; 2], f32)> {
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(line.len());
    for &p in line {
        if pts.last().is_none_or(|q: &[f32; 2]| (q[0] - p[0]).abs() > 1e-5 || (q[1] - p[1]).abs() > 1e-5) {
            pts.push(p);
        }
    }
    if closed && pts.len() > 1 {
        pts.pop();
    }
    let n = pts.len();
    if n < 2 {
        return Vec::new();
    }
    let dir = |a: [f32; 2], b: [f32; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        [dx / l, dy / l]
    };
    if !closed {
        let d0 = dir(pts[1], pts[0]);
        pts[0] = [pts[0][0] + d0[0] * hw, pts[0][1] + d0[1] * hw];
        let d1 = dir(pts[n - 2], pts[n - 1]);
        pts[n - 1] = [pts[n - 1][0] + d1[0] * hw, pts[n - 1][1] + d1[1] * hw];
    }
    let mut rows = Vec::with_capacity(n + 1);
    let mut dist = 0.0;
    let count = if closed { n + 1 } else { n };
    for k in 0..count {
        let i = k % n;
        let p = pts[i];
        let prev = if closed || i > 0 { Some(dir(pts[(i + n - 1) % n], p)) } else { None };
        let next = if closed || i + 1 < n { Some(dir(p, pts[(i + 1) % n])) } else { None };
        let normal = |d: [f32; 2]| [-d[1], d[0]];
        let off = match (prev, next) {
            (Some(a), Some(b)) => {
                let (na, nb) = (normal(a), normal(b));
                let m = [na[0] + nb[0], na[1] + nb[1]];
                let ml = (m[0] * m[0] + m[1] * m[1]).sqrt();
                if ml < 1e-4 {
                    [na[0] * hw, na[1] * hw]
                } else {
                    let m = [m[0] / ml, m[1] / ml];
                    let s = hw / (m[0] * na[0] + m[1] * na[1]).max(1.0 / 3.0);
                    [m[0] * s, m[1] * s]
                }
            }
            (Some(d), None) | (None, Some(d)) => {
                let nd = normal(d);
                [nd[0] * hw, nd[1] * hw]
            }
            (None, None) => [0.0, 0.0],
        };
        if k > 0 {
            let q = pts[(k - 1) % n];
            dist += ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        }
        rows.push(([p[0] + off[0], p[1] + off[1]], [p[0] - off[0], p[1] - off[1]], dist));
    }
    rows
}
