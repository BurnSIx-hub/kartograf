//! Окно редактора: меню, панель инструментов с библиотекой, холст и строка состояния.

use crate::export;
use crate::generate::{self, Options};
use crate::library::{self, Kind, Library, PX_PER_CELL, Textures};
use crate::map::{self, Coat, Layer, Map, Material, Paint, Portal, PortalKind, Prop, Segment, Wall};
use crate::mask::{self, Brush, Mask, RES, Shape as Tip};
use crate::recovery::{self, Orphan, Recovery};
use crate::roofs;
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
    Scatter,
    Path,
    Shade,
    Text,
    Roof,
}

impl Tool {
    const ALL: [Tool; 12] = [Tool::Select, Tool::Dungeon, Tool::Floor, Tool::Path, Tool::Shade, Tool::Erase, Tool::Wall, Tool::Door, Tool::Object, Tool::Scatter, Tool::Roof, Tool::Text];

    fn label(self) -> &'static str {
        match self {
            Tool::Select => "Выбор",
            Tool::Dungeon => "Подземелье",
            Tool::Floor => "Местность",
            Tool::Erase => "Стереть",
            Tool::Wall => "Стена",
            Tool::Door => "Дверь и окно",
            Tool::Object => "Объект",
            Tool::Scatter => "Разброс",
            Tool::Path => "Дорога",
            Tool::Shade => "Тень",
            Tool::Text => "Подпись",
            Tool::Roof => "Крыша",
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
            Tool::Scatter => Key::S,
            Tool::Path => Key::L,
            Tool::Shade => Key::H,
            Tool::Text => Key::T,
            Tool::Roof => Key::K,
        }
    }
}

/// Разброс: кисть, которая сыплет объекты из набора вразнобой (как Scatter
/// в Dungeondraft) — случайный объект, поворот и размер, не ближе заданного шага.
pub struct Scatter {
    pub set: Vec<String>,
    /// Радиус кисти в клетках.
    pub radius: f32,
    /// Сколько объектов на клетку площади кисти за один отпечаток.
    pub density: f32,
    /// Не ближе этой доли от суммы радиусов двух объектов.
    pub spacing: f32,
    pub scale: [f32; 2],
    /// Случайный поворот; без него — поворот из инструмента «Объект».
    pub spin: bool,
    last: Option<[f32; 2]>,
    rng: u64,
}

impl Default for Scatter {
    fn default() -> Self {
        Self { set: Vec::new(), radius: 1.5, density: 0.6, spacing: 0.8, scale: [0.8, 1.2], spin: true, last: None, rng: 0x9E37_79B9_7F4A_7C15 }
    }
}

impl Scatter {
    /// Случайное число 0..1 (xorshift): своё зерно, без внешних библиотек.
    pub fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Что выбрано на карте.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sel {
    Wall(usize),
    Portal(usize),
    Prop(usize),
    Path(usize),
    Label(usize),
    Roof(usize),
}

/// Как видны крыши в редакторе.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofView {
    /// Видны; с инструментами для внутренностей дома — полупрозрачны.
    Auto,
    Ghost,
    Hidden,
}

pub struct Args {
    pub file: Option<PathBuf>,
    pub selftest: Option<PathBuf>,
    pub shot: Option<PathBuf>,
    pub size: Option<[f32; 2]>,
    pub data: Option<PathBuf>,
    /// `--export папка`: выгрузить карту для Foundry и выйти.
    pub export: Option<PathBuf>,
    /// `--px N`: пикселей на клетку в выгрузке.
    pub px: Option<u32>,
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

/// Выгрузка для VTT идёт по кадрам: камера встаёт на плитку карты, окно снимает
/// себя, плитка переносится в большую картинку. Так картинка выходит тем же
/// отрисовщиком, что на экране, а не второй копией его кода.
struct Export {
    dir: PathBuf,
    name: String,
    px: u32,
    /// Размер плитки в пикселях экрана.
    tile: [u32; 2],
    tiles: Vec<[u32; 2]>,
    next: usize,
    image: image::RgbImage,
    /// Снятые этажи (кроме того, что снимается сейчас) и какой снимается.
    done: Vec<image::RgbImage>,
    doing: usize,
    /// С какого этажа начали — на него вернуться.
    start_level: usize,
    /// Камера и выбор до выгрузки — вернуть после.
    cam: Camera,
    sel: Option<Sel>,
    /// Кадров после сдвига камеры: дать догрузиться картинкам.
    settle: u32,
    asked: bool,
    /// Выгрузка из командной строки: по окончании выйти.
    quit: bool,
}

/// Окно выгрузки: сколько пикселей на клетку.
struct ExportDialog {
    px: u32,
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
    /// История: этаж и карта этого этажа до правки.
    undo: Vec<(usize, Map)>,
    redo: Vec<(usize, Map)>,
    /// Все этажи снизу вверх; на месте текущего — устаревшая копия, живой — `map`.
    pub levels: Vec<Map>,
    pub level: usize,
    /// Стены этажа ниже — тонкими линиями, чтобы попасть в них наверху.
    below: Vec<Segment>,
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
    pub scatter: Scatter,
    /// Начатая дорога: точки в клетках; `path_free` — рисуется протяжкой.
    pub path_draft: Vec<[f32; 2]>,
    path_free: bool,
    /// Ширина и край новых дорог.
    pub path_style: map::Path,
    /// Кисть «Тень»: до какой темноты, и стирает ли (Alt в начале мазка).
    pub shade_level: f32,
    shade_erase: bool,
    /// Образец новой подписи; тащат подпись — откуда взяли.
    pub label_style: map::Label,
    label_grab: Option<(usize, [f32; 2])>,
    /// Образец новой крыши, вид крыш, протяжка новой и перенос стоящей.
    pub roof_style: map::Roof,
    pub roof_view: RoofView,
    roof_from: Option<(i32, i32)>,
    roof_grab: Option<(usize, [f32; 2], [f32; 2], [f32; 2])>,
    /// Alt с колесом уже меняет размер объекта: правка в истории сделана.
    alt_wheel: bool,
    /// Стена, которую сейчас ставят по точкам.
    pub draft: Vec<[f32; 2]>,
    pub sel: Option<Sel>,
    /// Несколько объектов разом (Shift, рамка): двигаются вместе, объединяются в один.
    pub picked: Vec<usize>,
    /// Рамка выбора: откуда тянут, в клетках.
    band_from: Option<[f32; 2]>,
    /// Группа, которую тащат: где взяли и где каждый был.
    moving_group: Option<([f32; 2], Vec<(usize, [f32; 2])>)>,
    pub canvas: Rect,
    /// Пикселей экрана в точке egui — для выгрузки по плиткам.
    canvas_ppp: f32,
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
    /// Косяк для новых проёмов; встроенный подгоняется под ширину.
    pub door_frame: Option<String>,
    pub object_asset: Option<String>,
    pub object_rot: f32,
    pub object_scale: f32,

    pub recovery: Recovery,
    pub orphans: Vec<Orphan>,
    pub generator: Option<Options>,
    new_map: Option<NewMap>,
    export: Option<Export>,
    export_dialog: Option<ExportDialog>,
    /// Выгрузка из командной строки: куда и с каким разрешением.
    export_args: Option<(PathBuf, u32)>,
    /// Сообщение в строке состояния: что сделано.
    pub notice: Option<String>,
    confirm_close: bool,
    allow_close: bool,
}

pub fn default_data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kartograf")
}

/// Пикселей на клетку в выгрузке по умолчанию: столько у Foundry по умолчанию
/// не бывает (там 100), но карта с мелкими предметами при 100 мылится.
const EXPORT_PX: u32 = 140;
const EXPORT_PX_CHOICES: [u32; 4] = [100, 140, 200, 256];

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
        library.set_baked(&data_dir.join("baked"));
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
            levels: vec![Map::new(1, 1)],
            level: 0,
            below: Vec::new(),
            drag_from: None,
            brush: Brush::default(),
            stroke_last: None,
            stroke_tip: [0.0, 0.0],
            stroke_max: Default::default(),
            dabs: 0,
            edges: Vec::new(),
            edges_dirty: true,
            moving: None,
            picked: Vec::new(),
            band_from: None,
            moving_group: None,
            canvas_ppp: 1.0,
            bending: None,
            scatter: Scatter::default(),
            path_draft: Vec::new(),
            path_free: false,
            path_style: map::Path::template(0),
            shade_level: 0.4,
            shade_erase: false,
            label_style: map::Label::new("Комната".into(), [0.0, 0.0]),
            label_grab: None,
            roof_style: map::Roof::new([0.0, 0.0], [1.0, 1.0], map::RoofShape::Gable, library::DEFAULT_ROOF.into()),
            roof_view: RoofView::Auto,
            roof_from: None,
            roof_grab: None,
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
            door_asset: Some(library::DEFAULT_DOOR.into()),
            door_kind: PortalKind::Door,
            door_width: 1.0,
            door_frame: Some(format!("{}frame_wood_1", library::BUILTIN)),
            object_asset: None,
            object_rot: 0.0,
            object_scale: 1.0,
            recovery,
            orphans,
            generator: None,
            new_map: None,
            export: None,
            export_dialog: None,
            export_args: args.export.map(|d| (d, args.px.unwrap_or(EXPORT_PX))),
            notice: None,
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
        self.undo.push((self.level, self.map.clone()));
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
        if let Some((level, prev)) = self.undo.pop() {
            // Правка была на другом этаже — сначала туда.
            self.switch_level(level);
            self.redo.push((level, std::mem::replace(&mut self.map, prev)));
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if let Some((level, next)) = self.redo.pop() {
            self.switch_level(level);
            self.undo.push((level, std::mem::replace(&mut self.map, next)));
            self.after_history();
        }
    }

    /// Перейти на этаж `k`: живая карта уходит на своё место в списке.
    pub fn switch_level(&mut self, k: usize) {
        if k == self.level || k >= self.levels.len() {
            return;
        }
        std::mem::swap(&mut self.map, &mut self.levels[self.level]);
        std::mem::swap(&mut self.map, &mut self.levels[k]);
        self.level = k;
        self.sel = None;
        self.picked.clear();
        self.draft.clear();
        self.path_draft.clear();
        self.moving = None;
        self.edges_dirty = true;
        self.material = self.material.min(self.map.materials.len().saturating_sub(1));
        self.refresh_below();
    }

    /// Карта для файла: первый этаж, остальные — в его `levels`.
    pub fn project(&self) -> Map {
        let mut all: Vec<Map> = self.levels.iter().enumerate().map(|(k, m)| if k == self.level { self.map.clone() } else { m.clone() }).collect();
        let mut root = all.remove(0);
        root.levels = all;
        root
    }

    /// Новый этаж выше текущего (`above`) или ниже; история правок сбрасывается.
    pub fn add_level(&mut self, above: bool) {
        let fresh = self.map.new_level();
        let at = if above { self.level + 1 } else { self.level };
        self.levels[self.level] = self.map.clone();
        self.levels.insert(at, fresh);
        self.level = if above { self.level } else { self.level + 1 };
        self.undo.clear();
        self.redo.clear();
        self.switch_level(at);
        self.touch();
    }

    pub fn remove_level(&mut self) {
        if self.levels.len() < 2 {
            return;
        }
        self.levels.remove(self.level);
        self.level = self.level.min(self.levels.len() - 1);
        self.map = self.levels[self.level].clone();
        self.undo.clear();
        self.redo.clear();
        self.sel = None;
        self.edges_dirty = true;
        self.refresh_below();
        self.touch();
    }

    fn refresh_below(&mut self) {
        let k = self.level;
        self.below = if k > 0 { self.levels[k - 1].room_edges().into_iter().chain(self.levels[k - 1].walls.iter().flat_map(|w| w.segments().collect::<Vec<_>>())).collect() } else { Vec::new() };
    }

    pub fn level_name(&self, k: usize) -> String {
        let m = if k == self.level { &self.map } else { &self.levels[k] };
        if m.name.trim().is_empty() { format!("Этаж {}", k + 1) } else { m.name.clone() }
    }

    fn after_history(&mut self) {
        self.sel = self.sel.filter(|s| self.sel_exists(*s));
        self.material = self.material.min(self.map.materials.len().saturating_sub(1));
        self.draft.clear();
        self.path_draft.clear();
        self.moving = None;
        self.touch();
    }

    fn sel_exists(&self, s: Sel) -> bool {
        match s {
            Sel::Wall(i) => i < self.map.walls.len(),
            Sel::Portal(i) => i < self.map.portals.len(),
            Sel::Prop(i) => i < self.map.props.len(),
            Sel::Path(i) => i < self.map.paths.len(),
            Sel::Label(i) => i < self.map.labels.len(),
            Sel::Roof(i) => i < self.map.roofs.len(),
        }
    }

    /// Кисти «Клетки» и «Овал»: прямоугольник клеток или овал в нём.
    pub fn paint(&mut self, a: (i32, i32), b: (i32, i32)) {
        let mut probe = self.map.clone();
        if self.tool == Tool::Shade {
            let (w, h) = (probe.rooms.w, probe.rooms.h);
            let weights = if self.brush.shape == Tip::Oval { mask::oval(w, h, a, b) } else { mask::cells(w, h, a, b) };
            if self.shade_erase {
                probe.shade_fade(&weights);
            } else {
                probe.shade_paint(&weights, self.shade_level);
            }
            probe.prune();
            if probe != self.map {
                self.checkpoint();
                self.map = probe;
            }
            return;
        }
        let (layer, material) = match self.tool {
            Tool::Erase => (Layer::Floor, None),
            Tool::Dungeon => (Layer::Rooms, Some(self.material)),
            _ => (Layer::Floor, Some(self.material)),
        };
        let changed = if self.brush.shape == Tip::Oval {
            probe.paint_oval(layer, a, b, material)
        } else {
            probe.paint_rect(layer, a, b, material)
        };
        if changed {
            self.checkpoint();
            self.map = probe;
        }
    }

    /// Второй угол рамки; с Shift у овала — квадрат, чтобы вышел круг.
    fn drag_corner(&self, a: (i32, i32), b: (i32, i32), shift: bool) -> (i32, i32) {
        if !(shift && self.brush.shape == Tip::Oval) {
            return b;
        }
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        (a.0 + n * if b.0 < a.0 { -1 } else { 1 }, a.1 + n * if b.1 < a.1 { -1 } else { 1 })
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
            Tool::Shade if self.shade_erase => self.map.shade_fade(&weights),
            Tool::Shade => {
                // Темнеет до уровня по полной силе отпечатка, а не по прибавке.
                let raw: Vec<(usize, f32)> = weights.iter().map(|&(i, _)| (i, self.stroke_max.get(&i).copied().unwrap_or(0.0))).collect();
                self.map.shade_paint(&raw, self.shade_level);
            }
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
            self.map.walls.push(Wall { closed: closed && points.len() >= 3, points, bends: Vec::new(), style: None });
            self.sel = Some(Sel::Wall(self.map.walls.len() - 1));
        }
    }

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
    fn roof_at(&self, w: [f32; 2]) -> Option<usize> {
        if self.roof_alpha() < 1.0 && self.tool != Tool::Roof {
            return None;
        }
        self.map.roofs.iter().rposition(|r| r.contains(w))
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
        let asset = if self.door_kind == PortalKind::Opening { None } else { self.door_asset.clone() };
        let frame = self.door_frame.as_deref().map(|f| library::frame_for_width(f, w));
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
    fn asset_size(&mut self, key: &str) -> [f32; 2] {
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
        self.map.props.push(Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale, shadow: None });
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
        if let Some(i) = self.map.props.iter().rposition(|p| p.contains(w)) {
            return Some(Sel::Prop(i));
        }
        let near = |d: f32| d <= PICK;
        if let Some((i, _)) = self.map.portals.iter().enumerate().map(|(i, p)| (i, map::segment_distance(w, p.a, p.b))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)) {
            return Some(Sel::Portal(i));
        }
        if let Some(s) = self.map.walls.iter().enumerate().map(|(i, wall)| (i, wall.distance(w))).filter(|(_, d)| near(*d)).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| Sel::Wall(i)) {
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
            let prop = Prop { asset, pos, size, rot, scale, shadow: None };
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

    fn bake(&mut self, idx: &[usize]) -> Result<usize, String> {
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
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let (cw, ch) = (w as f32 / PPC, h as f32 / PPC);
        let num = |v: f32| format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_owned();
        let n = (1..).find(|n| !std::fs::read_dir(&dir).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with(&format!("Объединённый {n}_")))).unwrap_or(false)).unwrap_or(1);
        let file = dir.join(format!("Объединённый {n}_{}x{}.png", num(cw), num(ch)));
        out.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        self.library.set_baked(&dir);
        self.checkpoint();
        let at = idx[0];
        for &i in idx.iter().rev() {
            self.map.props.remove(i);
        }
        let key = file.to_string_lossy().into_owned();
        let prop = Prop { asset: key, pos: [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0], size: [cw, ch], rot: 0.0, scale: 1.0, shadow: None };
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
        if !matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Path) {
            self.tool = Tool::Dungeon;
        }
        self.recolor_path();
    }

    /// Выбранная дорога перекрашивается в выбранный материал.
    fn recolor_path(&mut self) {
        if let Some(Sel::Path(i)) = self.sel
            && self.tool == Tool::Path
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
    /// (свой появится с первой правкой), без выбора — всей карты.
    pub fn wall_style(&self) -> map::WallStyle {
        match self.sel {
            Some(Sel::Wall(i)) => self.map.walls[i].style.clone().unwrap_or_else(|| self.map.wall_style.clone()),
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
    fn put_wall_style(&mut self, st: map::WallStyle) {
        match self.sel {
            Some(Sel::Wall(i)) => self.map.walls[i].style = Some(st),
            _ => self.map.wall_style = st,
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

    fn reset_doc(&mut self, mut map: Map, path: Option<PathBuf>) {
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
        match std::fs::write(path, self.project().to_json()) {
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
        if self.export.is_none() {
            self.shortcuts(&ctx);
        }
        self.export_step(&ctx);

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
                let (w, h) = (self.map.width * dlg.px, self.map.height * dlg.px);
                ui.weak(format!("Картинка {w}×{h}."));
                ui.add_space(4.0);
                ui.weak("Рядом лягут три файла: картинка, сцена для Foundry (.json) и Universal VTT (.dd2vtt). В Foundry: создать сцену, правой кнопкой — «Import Data», выбрать .json.");
                ui.add_space(6.0);
                go = ui.button("Выбрать место и выгрузить").clicked();
            });
            if go {
                let dir = export::foundry_data_dir().map(|d| d.join("kartograf"));
                if let Some(d) = &dir {
                    let _ = std::fs::create_dir_all(d);
                }
                let mut dialog = rfd::FileDialog::new().add_filter("Сцена Foundry", &["json"]).set_file_name(format!("{}.json", self.name()));
                if let Some(d) = dir {
                    dialog = dialog.set_directory(d);
                }
                if let Some(file) = dialog.save_file() {
                    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.name());
                    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
                    self.start_export(dir, name, dlg.px, false);
                }
            } else if open {
                self.export_dialog = Some(dlg);
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
        if cmd(Key::G) {
            self.bake_picked();
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
            self.finish_path();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.draft.clear();
            self.path_draft.clear();
            self.sel = None;
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

    pub fn set_tool(&mut self, tool: Tool) {
        if tool != Tool::Wall {
            self.finish_wall(false);
        }
        if tool != Tool::Path {
            self.finish_path();
        }
        self.tool = tool;
        // Инструмент сам открывает нужную вкладку библиотеки.
        match tool {
            Tool::Door => self.browse = Kind::Door,
            Tool::Roof => self.browse = Kind::Roof,
            Tool::Object => self.browse = Kind::Object,
            Tool::Dungeon | Tool::Floor | Tool::Path if matches!(self.browse, Kind::Door | Kind::Object) => self.browse = Kind::Floor,
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
                if item(ui, "Выгрузить для Foundry…", "") {
                    self.export_dialog = Some(ExportDialog { px: EXPORT_PX });
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

    /// Этажи сверху вниз, как в доме; текущий можно переименовать.
    fn levels_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Этажи");
            if ui.small_button("+ выше").clicked() {
                self.add_level(true);
            }
            if ui.small_button("+ ниже").clicked() {
                self.add_level(false);
            }
        });
        if self.levels.len() < 2 {
            ui.weak("Один этаж. Новый — того же размера; в Foundry этажи станут уровнями сцены.");
            return;
        }
        for k in (0..self.levels.len()).rev() {
            if ui.selectable_label(self.level == k, self.level_name(k)).clicked() {
                self.switch_level(k);
            }
        }
        ui.horizontal(|ui| {
            let mut name = self.map.name.clone();
            let hint = format!("Этаж {}", self.level + 1);
            let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(hint).desired_width(120.0));
            if r.gained_focus() {
                self.checkpoint();
            }
            self.map.name = name;
            if ui.small_button("удалить").on_hover_text("Удалить этот этаж; отменить нельзя").clicked() {
                self.remove_level();
            }
        });
    }

    fn tools(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        self.levels_ui(ui);
        ui.add_space(8.0);
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
            Tool::Roof => "Протяжка рамкой по стенам дома ставит крышу; протяжка за крышу двигает её. R — конёк поперёк, у односкатной — куда скат. Покрытие — во вкладке «Крыши». С другими инструментами крыши полупрозрачны.",
            Tool::Text => "Щелчок по пустому месту ставит подпись, по подписи — выбирает её; протяжка двигает. Текст, размер и цвет — ниже.",
            Tool::Shade => "Темнит пол: углы, ниши, под навесом. Ступени — как тени-узоры FA: повторный мазок не темнее ступени. Alt — стирать.",
            Tool::Path => "Щелчки ставят точки — линия через них плавная; протяжка рисует от руки. Enter, двойной щелчок или правая кнопка — закончить. Материал — из «Пол»: земля — тропа, вода — ручей, ткань — ковёр.",
            Tool::Scatter => "Ведите кистью — объекты из набора ложатся вразнобой. Alt — стирать их. Ctrl с колесом — размер кисти. Объекты в набор — щелчком в библиотеке.",
            Tool::Object => "Щелчок ставит объект, протяжка двигает уже стоящий. Alt — на время наоборот с привязкой, Alt с колесом — размер, R — поворот.",
        });

        match self.tool {
            Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade => {
                ui.add_space(6.0);
                if self.tool == Tool::Shade {
                    ui.horizontal(|ui| {
                        for level in [0.2, 0.4, 0.6, 0.8] {
                            ui.selectable_value(&mut self.shade_level, level, format!("{}%", (level * 100.0) as u32));
                        }
                    });
                    ui.add(egui::Slider::new(&mut self.shade_level, 0.05..=1.0).text("темнота"));
                }
                ui.horizontal_wrapped(|ui| {
                    for shape in Tip::ALL {
                        ui.selectable_value(&mut self.brush.shape, shape, shape.label());
                    }
                });
                if self.brush.shape == Tip::Cells {
                    ui.weak("Щелчок — клетка, протяжка — прямоугольник клеток.");
                } else if self.brush.shape == Tip::Oval {
                    ui.weak("Протяжка — овал во весь прямоугольник; квадрат даёт круг. Shift держит круг.");
                } else {
                    ui.add(egui::Slider::new(&mut self.brush.radius, 0.25..=10.0).logarithmic(true).text("радиус, кл."));
                    ui.add(egui::Slider::new(&mut self.brush.hardness, 0.0..=1.0).text("жёсткость"));
                    ui.add(egui::Slider::new(&mut self.brush.flow, 0.05..=1.0).text("сила"));
                    ui.weak("[ и ] или Ctrl с колесом — меньше и больше. Мягкий край и неполная сила смешивают материалы.");
                }
            }
            Tool::Door => {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for kind in PortalKind::ALL {
                        if ui.selectable_label(self.door_kind == kind, kind.label()).clicked() && self.door_kind != kind {
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
                });
                ui.add(egui::Slider::new(&mut self.door_width, 1.0..=4.0).step_by(0.5).text("ширина, кл."));
                if self.door_kind != PortalKind::Opening {
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
                ui.weak("Створку, окно или косяк можно взять во вкладке «Двери». Створка уходит в Foundry отдельной картинкой и там открывается.");
            }
            Tool::Roof => {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Крыши:");
                    ui.selectable_value(&mut self.roof_view, RoofView::Auto, "видно");
                    ui.selectable_value(&mut self.roof_view, RoofView::Ghost, "сквозь");
                    ui.selectable_value(&mut self.roof_view, RoofView::Hidden, "скрыть");
                });
                let chosen = match self.sel {
                    Some(Sel::Roof(i)) => Some(i),
                    _ => None,
                };
                let mut st = chosen.map_or_else(|| self.roof_style.clone(), |i| self.map.roofs[i].clone());
                if chosen.is_some() {
                    ui.weak("Выбранная крыша:");
                }
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
                ui.add_space(6.0);
                let chosen = match self.sel {
                    Some(Sel::Label(i)) => Some(i),
                    _ => None,
                };
                let mut st = chosen.map_or_else(|| self.label_style.clone(), |i| self.map.labels[i].clone());
                if chosen.is_some() {
                    ui.weak("Выбранная подпись:");
                }
                let text = ui.add(egui::TextEdit::multiline(&mut st.text).desired_rows(2).desired_width(f32::INFINITY));
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
                match chosen {
                    Some(i) => {
                        if text.gained_focus() || rs.iter().any(|r| r.drag_started() || (r.changed() && !r.dragged())) || picked.inner.changed() {
                            self.checkpoint();
                        }
                        self.map.labels[i] = st;
                    }
                    None => self.label_style = st,
                }
            }
            Tool::Path => {
                ui.add_space(6.0);
                // Ползунки правят выбранную дорогу, а без неё — образец для новых.
                let chosen = match self.sel {
                    Some(Sel::Path(i)) => Some(i),
                    _ => None,
                };
                let mut st = chosen.map_or_else(|| self.path_style.clone(), |i| self.map.paths[i].clone());
                if chosen.is_some() {
                    ui.weak("Выбранная дорога:");
                }
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
            Tool::Scatter => {
                ui.add_space(6.0);
                ui.strong("Набор");
                if self.scatter.set.is_empty() {
                    ui.weak("пусто — щёлкните объекты во вкладке «Объекты»");
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
                ui.add_space(6.0);
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
            ui.add_space(6.0);
            ui.strong(format!("Выбрано объектов: {}", self.picked.len()));
            ui.horizontal(|ui| {
                if ui.add_enabled(self.picked.len() >= 2, egui::Button::new("Объединить в один   Ctrl+G")).clicked() {
                    self.bake_picked();
                }
                if ui.small_button("снять выбор").clicked() {
                    self.picked.clear();
                }
            });
            ui.weak("Объединённый объект ляжет в библиотеку, во вкладку «Объекты», папка «Запечённые».");
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
            let mut casts = self.map.props[i].casts_shadow();
            if ui.checkbox(&mut casts, "Отбрасывает тень").changed() {
                self.checkpoint();
                self.map.props[i].shadow = Some(casts);
            }
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
                    if !matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Path) {
                        self.set_tool(Tool::Dungeon);
                    }
                    self.recolor_path();
                }
            });
        }
        if let Some(m) = materials.get(self.material).filter(|m| matches!(m.paint, Paint::Texture(_))) {
            ui.horizontal(|ui| {
                ui.label("Живой в Foundry:").on_hover_text("Залитое этим материалом в выгрузке станет областью сцены, и модуль «Живая карта» её оживит: рябь и рыбки, течение лавы, пузыри болота.");
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

        ui.add_space(10.0);
        let own = match self.sel {
            Some(Sel::Wall(i)) => Some(self.map.walls[i].style.is_some()),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.strong(if own.is_some() { "Выбранная стена" } else { "Стены" });
            if own == Some(true) && ui.small_button("как у всех").on_hover_text("Убрать свой вид — стена станет как остальные").clicked()
                && let Some(Sel::Wall(i)) = self.sel
            {
                self.checkpoint();
                self.map.walls[i].style = None;
            }
        });
        if own == Some(false) {
            ui.weak("Как у всех. Текстура из библиотеки ляжет только на неё.");
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

        ui.add_space(10.0);
        let mut sh = self.map.shadows.clone();
        let on = ui.checkbox(&mut sh.on, "Тени от стен и объектов");
        let mut rs = Vec::new();
        if sh.on {
            rs.push(ui.add(egui::Slider::new(&mut sh.angle, 0.0..=345.0).step_by(15.0).text("куда падают, °")));
            rs.push(ui.add(egui::Slider::new(&mut sh.dist, 0.0..=1.0).text("длина, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.blur, 0.1..=1.0).text("размытие, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.opacity, 0.05..=1.0).text("темнота")));
        }
        if on.changed() || rs.iter().any(|r| r.drag_started() || (r.changed() && !r.dragged())) {
            self.checkpoint();
        }
        self.map.shadows = sh;

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
            Kind::Roof => Some(match self.sel {
                Some(Sel::Roof(i)) => self.map.roofs[i].paint.clone(),
                _ => self.roof_style.paint.clone(),
            }),
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
                    let mut st = self.wall_style();
                    if let Some(width) = library::builtin_wall_width(&key) {
                        st.width = width;
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
            if let Some(e) = &self.export {
                ui.separator();
                ui.strong(format!("Выгрузка: плитка {} из {}…", (e.next + 1).min(e.tiles.len()), e.tiles.len()));
            } else if let Some(n) = &self.notice {
                ui.separator();
                ui.label(n);
            }
        });
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas = rect;
        self.canvas_ppp = ui.ctx().pixels_per_point();
        if !self.fitted && rect.width() > 50.0 {
            self.cam.fit(rect, self.map.width, self.map.height);
            self.fitted = true;
        }
        self.hover = resp.hover_pos().or(self.hover.filter(|_| resp.dragged()));

        if self.export.is_some() {
            // Выгрузка двигает камеру сама; курсор и инструменты молчат.
            self.hover = None;
            self.draw(&painter, rect);
            return;
        }

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
            let brush = matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade) && !self.brush.shape.drags();
            let (ctrl, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.alt));
            let prop = matches!(self.tool, Tool::Select | Tool::Object);
            if !alt {
                self.alt_wheel = false;
            }
            if notches != 0.0 && self.tool == Tool::Scatter && ctrl {
                self.scatter.radius = (self.scatter.radius * 1.25f32.powf(notches)).clamp(0.25, 10.0);
            } else if notches != 0.0 && brush && ctrl {
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
            Tool::Floor | Tool::Erase | Tool::Dungeon | Tool::Shade if !self.brush.shape.drags() => {
                // Мазок начинается сразу по нажатию, а не после порога протяжки.
                let (pressed, down, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
                if pressed
                    && resp.contains_pointer()
                    && let Some(p) = at
                {
                    self.shade_erase = alt;
                    self.begin_stroke(self.cam.to_world(rect, p));
                } else if self.stroke_last.is_some() {
                    match (down, at) {
                        (true, Some(p)) => self.stroke_to(self.cam.to_world(rect, p)),
                        _ => self.end_stroke(),
                    }
                }
            }
            Tool::Floor | Tool::Erase | Tool::Dungeon | Tool::Shade => {
                self.shade_erase = alt;
                if resp.drag_started_by(PointerButton::Primary) {
                    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
                    self.drag_from = origin.map(|p| self.cam.cell(rect, p));
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    if let (Some(a), Some(p)) = (self.drag_from.take(), pointer) {
                        let b = self.drag_corner(a, self.cam.cell(rect, p), ui.input(|i| i.modifiers.shift));
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
            Tool::Roof => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    match self.roof_at(w) {
                        Some(i) => {
                            self.checkpoint();
                            let r = &self.map.roofs[i];
                            self.roof_grab = Some((i, w, r.a, r.b));
                            self.sel = Some(Sel::Roof(i));
                        }
                        None => self.roof_from = Some(self.cam.cell(rect, o)),
                    }
                }
                if let (Some((i, w0, a, b)), Some(p)) = (self.roof_grab, pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    // Крыша двигается целыми клетками — остаётся на стенах.
                    let w = self.cam.to_world(rect, p);
                    let d = [(w[0] - w0[0]).round(), (w[1] - w0[1]).round()];
                    let r = &mut self.map.roofs[i];
                    r.a = [a[0] + d[0], a[1] + d[1]];
                    r.b = [b[0] + d[0], b[1] + d[1]];
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.roof_grab = None;
                    if let (Some(a), Some(p)) = (self.roof_from.take(), pointer) {
                        let b = self.cam.cell(rect, p);
                        self.place_roof(a, b);
                    }
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    self.sel = self.roof_at(self.cam.to_world(rect, p)).map(Sel::Roof);
                }
            }
            Tool::Text => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    if let Some(i) = self.map.labels.iter().rposition(|l| l.contains(w)) {
                        self.checkpoint();
                        let p = self.map.labels[i].pos;
                        self.label_grab = Some((i, [w[0] - p[0], w[1] - p[1]]));
                        self.sel = Some(Sel::Label(i));
                    }
                }
                if let (Some((i, g)), Some(p)) = (self.label_grab, pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    let w = self.cam.to_world(rect, p);
                    self.map.labels[i].pos = self.snap_pos([w[0] - g[0], w[1] - g[1]], alt);
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.label_grab = None;
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let w = self.cam.to_world(rect, p);
                    match self.map.labels.iter().rposition(|l| l.contains(w)) {
                        Some(i) => self.sel = Some(Sel::Label(i)),
                        None if !self.label_style.text.trim().is_empty() => {
                            self.checkpoint();
                            let pos = self.snap_pos(w, alt);
                            self.map.labels.push(map::Label { pos, ..self.label_style.clone() });
                            self.sel = Some(Sel::Label(self.map.labels.len() - 1));
                        }
                        None => self.sel = None,
                    }
                }
            }
            Tool::Path => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    // Протяжка — от руки: точки через полклетки, на отпускании дорога готова.
                    self.path_draft.push(self.cam.to_world(rect, o));
                    self.path_free = true;
                }
                if self.path_free
                    && resp.dragged_by(PointerButton::Primary)
                    && let (Some(p), Some(last)) = (pointer, self.path_draft.last().copied())
                {
                    let w = self.cam.to_world(rect, p);
                    if ((w[0] - last[0]).powi(2) + (w[1] - last[1]).powi(2)).sqrt() >= 0.5 {
                        self.path_draft.push(w);
                    }
                }
                if self.path_free && resp.drag_stopped_by(PointerButton::Primary) {
                    if let Some(p) = pointer {
                        self.path_draft.push(self.cam.to_world(rect, p));
                    }
                    self.path_free = false;
                    self.finish_path();
                } else if resp.double_clicked() || resp.secondary_clicked() {
                    self.finish_path();
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let w = self.snap_pos(self.cam.to_world(rect, p), alt);
                    if self.path_draft.last() != Some(&w) {
                        self.path_draft.push(w);
                    }
                }
            }
            Tool::Scatter => {
                let (pressed, down, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
                if pressed
                    && resp.contains_pointer()
                    && let Some(p) = at
                {
                    self.checkpoint();
                    let w = self.cam.to_world(rect, p);
                    self.scatter_dab(w, alt);
                    self.scatter.last = Some(w);
                } else if let (true, Some(last), Some(p)) = (down, self.scatter.last, at) {
                    let w = self.cam.to_world(rect, p);
                    // Новый отпечаток — когда кисть ушла на половину радиуса.
                    if ((w[0] - last[0]).powi(2) + (w[1] - last[1]).powi(2)).sqrt() >= self.scatter.radius * 0.5 {
                        self.scatter_dab(w, alt);
                        self.scatter.last = Some(w);
                    }
                } else if !down {
                    self.scatter.last = None;
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
                let shift = ui.input(|i| i.modifiers.shift);
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    match self.pick(w) {
                        // Взяли объект из группы — едет вся группа.
                        Some(Sel::Prop(i)) if self.picked.contains(&i) && self.grip_at(o).is_none() => {
                            self.checkpoint();
                            self.moving_group = Some((w, self.picked.iter().map(|&k| (k, self.map.props[k].pos)).collect()));
                        }
                        Some(_) => {}
                        // Мимо всего — рамка выбора.
                        None if self.grip_at(o).is_none() => self.band_from = Some(w),
                        None => {}
                    }
                }
                if let (Some((from, start)), Some(p)) = (self.moving_group.clone(), pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    let w = self.cam.to_world(rect, p);
                    let mut d = [w[0] - from[0], w[1] - from[1]];
                    if self.settings.snap != alt {
                        d = [snap_half(d[0]), snap_half(d[1])];
                    }
                    for (k, pos) in start {
                        self.map.props[k].pos = [pos[0] + d[0], pos[1] + d[1]];
                    }
                } else if self.band_from.is_none() {
                    self.drag_prop(ui, resp, pointer, alt);
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.moving_group = None;
                    if let (Some(a), Some(p)) = (self.band_from.take(), pointer) {
                        let b = self.cam.to_world(rect, p);
                        let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
                        let inside: Vec<usize> = (0..self.map.props.len())
                            .filter(|&k| {
                                let c = self.map.props[k].pos;
                                c[0] >= lo[0] && c[0] <= hi[0] && c[1] >= lo[1] && c[1] <= hi[1]
                            })
                            .collect();
                        if !shift {
                            self.picked.clear();
                        }
                        for k in inside {
                            if !self.picked.contains(&k) {
                                self.picked.push(k);
                            }
                        }
                        self.sel = None;
                    }
                }
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    let hit = self.pick(self.cam.to_world(rect, p));
                    if shift {
                        // Shift — добавить к группе или убрать из неё.
                        if let Some(Sel::Prop(k)) = self.sel.take() {
                            if !self.picked.contains(&k) {
                                self.picked.push(k);
                            }
                        }
                        if let Some(Sel::Prop(k)) = hit {
                            match self.picked.iter().position(|&x| x == k) {
                                Some(at) => {
                                    self.picked.remove(at);
                                }
                                None => self.picked.push(k),
                            }
                        }
                    } else {
                        self.picked.clear();
                        self.sel = hit;
                    }
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

        // Дороги и реки — поверх пола.
        let paths = std::mem::take(&mut self.map.paths);
        for (i, path) in paths.iter().enumerate() {
            self.draw_path(painter, canvas, path, i as u32);
        }
        self.map.paths = paths;

        // Нарисованная тень — поверх пола и дорог.
        if let Some(shade) = self.map.shade.take() {
            let black = Material { name: String::new(), paint: Paint::Color([0, 0, 0]), vary: false, live: map::Live::Auto };
            self.draw_mask(painter, canvas, &black, &shade, &[], None, view);
            self.map.shade = Some(shade);
        }

        let exporting = self.export.is_some();
        // Протянутая заливка — рамкой, пока кнопку не отпустили.
        if let (Some(a), Some(p)) = (self.drag_from, self.hover.filter(|_| !exporting)) {
            let b = self.drag_corner(a, cam.cell(canvas, p), painter.ctx().input(|i| i.modifiers.shift));
            let r = Rect::from_min_max(
                cam.to_screen(canvas, [a.0.min(b.0) as f32, a.1.min(b.1) as f32]),
                cam.to_screen(canvas, [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0]),
            );
            let blue = Color32::from_rgb(120, 180, 255);
            if self.brush.shape == Tip::Oval {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, blue.gamma_multiply(0.4)), egui::StrokeKind::Inside);
                let n = 72;
                let pts: Vec<Pos2> = (0..n)
                    .map(|k| {
                        let t = k as f32 / n as f32 * std::f32::consts::TAU;
                        r.center() + vec2(t.cos() * r.width() / 2.0, t.sin() * r.height() / 2.0)
                    })
                    .collect();
                painter.add(Shape::closed_line(pts, Stroke::new(2.0, blue)));
            } else {
                painter.rect_stroke(r.intersect(map_rect), 0.0, Stroke::new(2.0, blue), egui::StrokeKind::Inside);
            }
            let (w, h) = ((a.0 - b.0).abs() + 1, (a.1 - b.1).abs() + 1);
            painter.text(r.right_bottom() + vec2(6.0, 4.0), egui::Align2::LEFT_TOP, format!("{w}×{h}"), egui::FontId::proportional(13.0), blue);
        }

        // Сетка. На мелком масштабе она сливается в сплошной фон — тогда её нет.
        // В выгрузке сетки нет: её рисует сам VTT.
        if s >= 10.0 && !exporting {
            let grid = Stroke::new(1.0, Color32::from_black_alpha(60));
            for x in x0..=x1 {
                painter.line_segment([cam.to_screen(canvas, [x as f32, y0 as f32]), cam.to_screen(canvas, [x as f32, y1 as f32])], grid);
            }
            for y in y0..=y1 {
                painter.line_segment([cam.to_screen(canvas, [x0 as f32, y as f32]), cam.to_screen(canvas, [x1 as f32, y as f32])], grid);
            }
        }
        if !exporting {
            painter.rect_stroke(map_rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Outside);
        }

        // Стены этажа ниже — пунктиром, чтобы верхний лёг на них.
        if !self.below.is_empty() && !exporting {
            let ghost = Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 220, 140, 120));
            for (a, b) in &self.below {
                painter.add(Shape::dashed_line(&[cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], ghost, 6.0, 4.0));
            }
        }

        // Объекты — под стенами, как мебель у стены. Сначала их тени.
        let props = self.map.props.clone();
        let shadows = self.map.shadows.clone();
        if shadows.on {
            for prop in props.iter().filter(|p| p.casts_shadow()) {
                let c = prop.corners();
                let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1]))).expand(shadows.dist + shadows.blur * 2.0);
                if bounds.intersects(view) {
                    self.draw_prop_shadow(painter, canvas, prop, &shadows);
                }
            }
        }
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

        // Стены комнат и нарисованные вручную — за вычетом проёмов. Тени стен
        // ложатся и на объекты у стены.
        let style = self.map.wall_style.clone();
        let edges = map::cut_portals(&self.room_edges().to_vec(), &self.map.portals);
        if shadows.on {
            self.draw_wall_shadow(painter, canvas, &edges, style.width, &shadows);
            for w in self.map.walls.clone() {
                let width = w.style.as_ref().map_or(style.width, |s| s.width);
                self.draw_wall_shadow(painter, canvas, &map::cut_portals(&w.segments().collect::<Vec<_>>(), &self.map.portals), width, &shadows);
            }
        }
        self.draw_walls(painter, canvas, &style, &edges);
        let walls = self.map.walls.clone();
        for (i, wall) in walls.iter().enumerate() {
            let segs: Vec<Segment> = wall.segments().collect();
            let own = wall.style.as_ref().unwrap_or(&style);
            if self.sel == Some(Sel::Wall(i)) {
                for (a, b) in &segs {
                    painter.line_segment([cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], Stroke::new(own.width * s + 6.0, Color32::from_rgb(120, 180, 255)));
                }
            }
            self.draw_walls(painter, canvas, own, &map::cut_portals(&segs, &self.map.portals));
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

        // Группа выбранных и рамка выбора.
        if !exporting {
            let green = Color32::from_rgb(140, 220, 160);
            for &k in &self.picked {
                if let Some(p) = self.map.props.get(k) {
                    let q: Vec<Pos2> = p.corners().iter().map(|&c| cam.to_screen(canvas, c)).collect();
                    painter.add(Shape::closed_line(q, Stroke::new(1.5, green)));
                }
            }
            if let (Some(a), Some(h)) = (self.band_from, self.hover) {
                let r = Rect::from_two_pos(cam.to_screen(canvas, a), h);
                painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(140, 220, 160, 25), Stroke::new(1.0, green), egui::StrokeKind::Middle);
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
                    let ghost = Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale, shadow: None };
                    self.draw_prop(painter, canvas, &ghost, Color32::from_white_alpha(150));
                }
            }
            (Tool::Scatter, Some(h)) => {
                let alt = painter.ctx().input(|i| i.modifiers.alt);
                let color = if alt { Color32::from_rgb(255, 130, 110) } else { Color32::from_rgb(140, 220, 160) };
                painter.circle_stroke(h, self.scatter.radius * s, Stroke::new(1.5, color));
            }
            (Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade, Some(h)) if !self.brush.shape.drags() => {
                // Контур кисти: куда ляжет мазок.
                let r = self.brush.radius * s;
                let erase = self.tool == Tool::Erase || (self.tool == Tool::Shade && painter.ctx().input(|i| i.modifiers.alt));
                let color = if erase { Color32::from_rgb(255, 130, 110) } else { Color32::from_rgb(120, 180, 255) };
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

        // Крыши — над всем, что в доме; тень их ложится и в выгрузку, а сами
        // они уходят в Foundry отдельными тайлами.
        let roof_alpha = if exporting { 0.0 } else { self.roof_alpha() };
        if self.roof_view != RoofView::Hidden {
            let roofs = self.map.roofs.clone();
            if shadows.on {
                for r in &roofs {
                    self.draw_roof_shadow(painter, canvas, r, &shadows);
                }
            }
            if roof_alpha > 0.0 {
                for (i, r) in roofs.iter().enumerate() {
                    self.draw_roof(painter, canvas, r, roof_alpha, shadows.light());
                    if self.sel == Some(Sel::Roof(i)) {
                        let q: Vec<Pos2> = r.outline().iter().map(|&p| cam.to_screen(canvas, p)).collect();
                        painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
                    }
                }
            }
        }
        if let (Some(a), Some(p)) = (self.roof_from, self.hover.filter(|_| !exporting)) {
            let b = cam.cell(canvas, p);
            let r = Rect::from_min_max(
                cam.to_screen(canvas, [a.0.min(b.0) as f32, a.1.min(b.1) as f32]),
                cam.to_screen(canvas, [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0]),
            );
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 200, 90)), egui::StrokeKind::Inside);
        }

        // Подписи — поверх всего, с тёмной каймой, чтобы читались на любом полу.
        for (i, label) in self.map.labels.clone().iter().enumerate() {
            self.draw_label(painter, canvas, label);
            if self.sel == Some(Sel::Label(i)) && !exporting {
                let [hw, hh] = label.half_extent();
                let (sn, cs) = label.rot.to_radians().sin_cos();
                let q: Vec<Pos2> = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                    .iter()
                    .map(|[x, y]| cam.to_screen(canvas, [label.pos[0] + x * cs - y * sn, label.pos[1] + x * sn + y * cs]))
                    .collect();
                painter.add(Shape::closed_line(q, Stroke::new(1.5, Color32::from_rgb(120, 180, 255))));
            }
        }

        // Выбранная дорога — осевой линией; начатая — линией до курсора.
        if let Some(Sel::Path(i)) = self.sel
            && !exporting
            && let Some(p) = self.map.paths.get(i)
        {
            let line: Vec<Pos2> = p.curve().iter().map(|&q| cam.to_screen(canvas, q)).collect();
            painter.add(Shape::line(line, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
            for q in &p.points {
                painter.circle_stroke(cam.to_screen(canvas, *q), 4.0, Stroke::new(1.5, Color32::from_rgb(120, 180, 255)));
            }
        }
        if !self.path_draft.is_empty() {
            let mut pts = self.path_draft.clone();
            if let (Some(h), false) = (hover, self.path_free) {
                pts.push(self.snap_pos(cam.to_world(canvas, h), painter.ctx().input(|i| i.modifiers.alt)));
            }
            let line: Vec<Pos2> = map::smooth_line(&pts).iter().map(|&q| cam.to_screen(canvas, q)).collect();
            let gold = Color32::from_rgb(255, 200, 90);
            painter.add(Shape::line(line, Stroke::new((self.path_style.width * s).max(3.0), gold.gamma_multiply(0.35))));
            if !self.path_free {
                for q in &self.path_draft {
                    painter.circle_filled(cam.to_screen(canvas, *q), 4.0, gold);
                }
            }
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
    /// Картинка вдоль отрезка `s`→`e`: длина — отрезок, толщина — по пропорциям
    /// картинки (у FA холст створки 1×1 с доской посередине). Правый край картинки
    /// (петля створки) — у `e`. `false` — картинка ещё не готова.
    fn draw_along(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2]) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let Some((_, px)) = self.textures.get(key, false) else { return false };
        let half = len * px[1] as f32 / px[0].max(1) as f32 / 2.0;
        let (nx, ny) = (-dy / len * half, dx / len * half);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        self.quad(painter, canvas, key, c, full, Color32::WHITE)
    }

    /// Проём: косяк в свой размер посередине, затем створка, две створки или окно.
    /// В выгрузке створок нет — они уходят в Foundry отдельной картинкой.
    fn draw_portal(&mut self, painter: &egui::Painter, canvas: Rect, p: &Portal, wall_width: f32) {
        let (dx, dy) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mid = p.center();
        if let Some(f) = p.frame.clone() {
            let size = self.asset_size(&f);
            let (hx, hy) = (ux * size[0] / 2.0, uy * size[0] / 2.0);
            self.draw_along(painter, canvas, &f, [mid[0] - hx, mid[1] - hy], [mid[0] + hx, mid[1] + hy]);
        }
        let rect_around = |half: f32| {
            let (nx, ny) = (-uy * half, ux * half);
            [[p.a[0] + nx, p.a[1] + ny], [p.b[0] + nx, p.b[1] + ny], [p.b[0] - nx, p.b[1] - ny], [p.a[0] - nx, p.a[1] - ny]]
        };
        let edge = Stroke::new(1.5, Color32::from_rgb(20, 16, 13));
        let to_screen = |c: [[f32; 2]; 4], cam: &Camera| c.iter().map(|&w| cam.to_screen(canvas, w)).collect::<Vec<_>>();
        let plain = |painter: &egui::Painter, cam: &Camera, half: f32, color: Color32| {
            painter.add(Shape::convex_polygon(to_screen(rect_around(half), cam), color, edge));
        };
        let exporting = self.export.is_some();
        match p.kind {
            PortalKind::Opening => {}
            PortalKind::Door | PortalKind::DoubleDoor if exporting => {}
            PortalKind::Door => {
                // Петля (правый край картинки) — у начала проёма: там её повесит и Foundry.
                let key = p.asset.clone();
                if !key.is_some_and(|k| self.draw_along(painter, canvas, &k, p.b, p.a)) {
                    plain(painter, &self.cam, wall_width * 0.35, Color32::from_rgb(132, 92, 54));
                }
            }
            PortalKind::DoubleDoor => {
                // Створки на петлях у краёв проёма, сходятся посередине.
                let key = p.asset.clone().unwrap_or_else(|| library::DEFAULT_DOOR.into());
                self.draw_along(painter, canvas, &key, mid, p.a);
                self.draw_along(painter, canvas, &key, mid, p.b);
            }
            PortalKind::Window => {
                let key = p.asset.clone();
                if !key.is_some_and(|k| self.draw_along(painter, canvas, &k, p.a, p.b)) {
                    plain(painter, &self.cam, wall_width * 0.3, Color32::from_rgb(150, 196, 220));
                }
            }
        }
    }

    /// Слой материала: сетка четырёхугольников по точкам маски, прозрачность вершины —
    /// сила слоя в ней. Края мягкой кисти так и получаются мягкими. `clip` — маска,
    /// за которую слой не выходит (пол комнат — за саму комнату). `above` — слои,
    /// которые лягут поверх: сила в маске — доля слоя в точке, а рисуются слои
    /// друг на друга, поэтому прозрачность пересчитывается (см. [`map::coat_alpha`]).
    fn draw_coat(&mut self, painter: &egui::Painter, canvas: Rect, coat: &Coat, above: &[Coat], clip: Option<&Mask>, view: Rect) {
        let Some(material) = self.map.materials.get(coat.material as usize).cloned() else { return };
        self.draw_mask(painter, canvas, &material, &coat.mask, above, clip, view);
    }

    /// Маска, залитая материалом: сила точки — прозрачность.
    #[allow(clippy::too_many_arguments)]
    fn draw_mask(&mut self, painter: &egui::Painter, canvas: Rect, material: &Material, m: &Mask, above: &[Coat], clip: Option<&Mask>, view: Rect) {
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

    /// Крыша: скаты своей картинкой, каждый со своей освещённостью; гребни на
    /// коньке и рёбрах — ряд того же покрытия вдоль линии; тёмная кромка свеса.
    fn draw_roof(&mut self, painter: &egui::Painter, canvas: Rect, roof: &map::Roof, alpha: f32, light: [f32; 2]) {
        let cam = self.cam;
        let (tex, tile) = match self.textures.get(&roof.paint, false) {
            Some((t, px)) => (Some(t.id()), roofs::tile_cells(px)),
            None => (None, [1.0, 1.0]),
        };
        let (planes, lines) = roof.geometry();
        let flat = roof.shape == map::RoofShape::Flat;
        let tint = |s: f32| {
            let v = (s * alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            if tex.is_some() { Color32::from_rgba_premultiplied(v, v, v, (alpha * 255.0) as u8) } else { Color32::from_rgba_premultiplied((v as f32 * 0.6) as u8, (v as f32 * 0.35) as u8, (v as f32 * 0.3) as u8, (alpha * 255.0) as u8) }
        };
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        let uv = |p: [f32; 2], o: [f32; 2], along: [f32; 2], down: [f32; 2]| {
            if tex.is_none() {
                return egui::epaint::WHITE_UV;
            }
            let d = [p[0] - o[0], p[1] - o[1]];
            pos2((d[0] * along[0] + d[1] * along[1]) / tile[0], (d[0] * down[0] + d[1] * down[1]) / tile[1])
        };
        for pl in &planes {
            let color = tint(map::roof_shade(pl.down, light, flat));
            let base = mesh.vertices.len() as u32;
            for &p in &pl.poly {
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: uv(p, pl.origin, pl.along, pl.down), color });
            }
            for k in 1..pl.poly.len() as u32 - 1 {
                mesh.add_triangle(base, base + k, base + k + 1);
            }
        }
        // Гребни поверх скатов.
        let dark = Color32::from_rgba_premultiplied(0, 0, 0, (alpha * 150.0) as u8);
        let mut edges = Vec::new();
        for &(a, b) in &lines {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let along = [dx / len, dy / len];
            let n = [-along[1] * roofs::CAP / 2.0, along[0] * roofs::CAP / 2.0];
            let quad = [[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]];
            let down = [-along[1], along[0]];
            let color = tint(roofs::CAP_SHADE);
            let base = mesh.vertices.len() as u32;
            for p in quad {
                let mut t = uv(p, a, along, down);
                t.x *= 2.0;
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: t, color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
            edges.push([quad[0], quad[1]]);
            edges.push([quad[3], quad[2]]);
        }
        painter.add(Shape::mesh(mesh));
        for [a, b] in edges {
            painter.line_segment([cam.to_screen(canvas, a), cam.to_screen(canvas, b)], Stroke::new(1.0, dark));
        }
        let out: Vec<Pos2> = roof.outline().iter().map(|&p| cam.to_screen(canvas, p)).collect();
        painter.add(Shape::closed_line(out, Stroke::new(roofs::RIM_PX, Color32::from_rgba_premultiplied(14, 10, 8, (alpha * 230.0) as u8))));
    }

    /// Тень крыши на земле: сдвинутый контур, залитый, с мягким краем.
    fn draw_roof_shadow(&self, painter: &egui::Painter, canvas: Rect, roof: &map::Roof, sh: &map::Shadows) {
        let cam = self.cam;
        let pts = roofs::shadow_outline(roof, sh);
        let n = pts.len();
        let c = [pts.iter().map(|p| p[0]).sum::<f32>() / n as f32, pts.iter().map(|p| p[1]).sum::<f32>() / n as f32];
        let dark = Color32::from_black_alpha((sh.opacity * 255.0).round() as u8);
        let mut mesh = egui::Mesh::default();
        let v = |p: [f32; 2], color| egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: egui::epaint::WHITE_UV, color };
        mesh.vertices.push(v(c, dark));
        for (k, &p) in pts.iter().enumerate() {
            let d = [p[0] - c[0], p[1] - c[1]];
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-6);
            let half = sh.blur / 2.0;
            mesh.vertices.push(v([p[0] - d[0] / l * half, p[1] - d[1] / l * half], dark));
            mesh.vertices.push(v([p[0] + d[0] / l * half, p[1] + d[1] / l * half], Color32::TRANSPARENT));
            let (i, j) = (1 + 2 * k as u32, 1 + 2 * ((k + 1) % n) as u32);
            mesh.add_triangle(0, i, j);
            mesh.add_triangle(i, i + 1, j + 1);
            mesh.add_triangle(i, j + 1, j);
        }
        painter.add(Shape::mesh(mesh));
    }

    /// Подпись: текст по центру своей точки, повёрнутый, с каймой из восьми
    /// тёмных копий вокруг.
    fn draw_label(&self, painter: &egui::Painter, canvas: Rect, label: &map::Label) {
        let px = label.size * self.cam.scale;
        if px < 3.0 || label.text.is_empty() {
            return;
        }
        let font = egui::FontId::proportional(px);
        let color = Color32::from_rgb(label.color[0], label.color[1], label.color[2]);
        let galley = painter.layout(label.text.clone(), font.clone(), color, f32::INFINITY);
        let dark = painter.layout(label.text.clone(), font, Color32::from_black_alpha(200), f32::INFINITY);
        let c = self.cam.to_screen(canvas, label.pos);
        let angle = label.rot.to_radians();
        let half = galley.size() / 2.0;
        // Угол текста — левый верхний; поворот — вокруг него, так что середину
        // сдвигаем повёрнутой половиной размера.
        let rot = egui::emath::Rot2::from_angle(angle);
        let origin = c - rot * half;
        let ring = (px * 0.07).max(1.0);
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let o = vec2(a.cos(), a.sin()) * ring;
            painter.add(egui::epaint::TextShape::new(origin + o, dark.clone(), Color32::BLACK).with_angle(angle));
        }
        painter.add(egui::epaint::TextShape::new(origin, galley, color).with_angle(angle));
    }

    /// Тень объекта: размытый силуэт той же картинки, сдвинутый от света.
    fn draw_prop_shadow(&mut self, painter: &egui::Painter, canvas: Rect, prop: &Prop, sh: &map::Shadows) {
        let blur = ((sh.blur * 10.0).round() as u32).clamp(1, 10);
        let key = format!("{}{blur}:{}", library::SHADOW, prop.asset);
        let Some((tex, info)) = self.textures.get(&key, false) else { return };
        let (id, [tw, th]) = (tex.id(), tex.size());
        let (pad, sw) = (info[0] as f32, info[1].max(1) as f32);
        let sh_h = (th as f32 - 2.0 * pad).max(1.0);
        let _ = tw;
        let (w, h) = (prop.size[0] * prop.scale, prop.size[1] * prop.scale);
        let (hw, hh) = (w / 2.0 + pad / sw * w, h / 2.0 + pad / sh_h * h);
        let (s, c) = prop.rot.to_radians().sin_cos();
        let o = sh.offset();
        let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|[x, y]| [prop.pos[0] + x * c - y * s + o[0], prop.pos[1] + x * s + y * c + o[1]]);
        let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let tint = Color32::from_white_alpha((sh.opacity * 255.0).round() as u8);
        let mut mesh = egui::Mesh::with_texture(id);
        for (p, uv) in corners.iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex { pos: self.cam.to_screen(canvas, *p), uv, color: tint });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
    }

    /// Тень стен: лента шире стены на размытие, сдвинутая от света; края тают.
    fn draw_wall_shadow(&mut self, painter: &egui::Painter, canvas: Rect, segs: &[Segment], width: f32, sh: &map::Shadows) {
        let cam = self.cam;
        let o = sh.offset();
        let (outer_hw, inner_hw) = (width / 2.0 + sh.blur / 2.0, (width / 2.0 - sh.blur / 2.0).max(0.01));
        let dark = Color32::from_black_alpha((sh.opacity * 255.0).round() as u8);
        let mut mesh = egui::Mesh::default();
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let (outer, inner) = (ribbon(&line, closed, outer_hw), ribbon(&line, closed, inner_hw));
            if outer.len() < 2 || outer.len() != inner.len() {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for ((ol, or, _), (il, ir, _)) in outer.iter().zip(&inner) {
                for (p, color) in [(ol, Color32::TRANSPARENT), (il, dark), (ir, dark), (or, Color32::TRANSPARENT)] {
                    let q = [p[0] + o[0], p[1] + o[1]];
                    mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, q), uv: egui::epaint::WHITE_UV, color });
                }
            }
            for k in 0..outer.len() as u32 - 1 {
                let (p, q) = (base + k * 4, base + (k + 1) * 4);
                for r in 0..3 {
                    mesh.add_triangle(p + r, p + r + 1, q + r + 1);
                    mesh.add_triangle(p + r, q + r + 1, q + r);
                }
            }
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Дорога лентой вдоль плавной линии: четыре ряда вершин поперёк — внешний край
    /// прозрачный, внутренние сплошные, между ними край тает. Ширина каждой стороны
    /// гуляет по своему шуму, как берег. Текстура — по сетке, как пол, или вдоль.
    fn draw_path(&mut self, painter: &egui::Painter, canvas: Rect, path: &map::Path, seed: u32) {
        let Some(material) = self.map.materials.get(path.material) else { return };
        let (tex, tile, tint) = match &material.paint {
            Paint::Color(c) => (None, [1.0, 1.0], rgb(*c)),
            Paint::Texture(key) => {
                let key = if material.vary && !path.along { format!("{}{key}", library::VARY) } else { key.clone() };
                match self.textures.get(&key, false) {
                    Some((t, px)) => (Some(t.id()), [(px[0] as f32 / PX_PER_CELL).max(MIN_TILE), (px[1] as f32 / PX_PER_CELL).max(MIN_TILE)], Color32::WHITE),
                    None => (None, [1.0, 1.0], Color32::from_gray(60)),
                }
            }
        };
        let line = path.curve();
        if line.len() < 2 {
            return;
        }
        let mut dist = vec![0.0f32; line.len()];
        for k in 1..line.len() {
            dist[k] = dist[k - 1] + ((line[k][0] - line[k - 1][0]).powi(2) + (line[k][1] - line[k - 1][1]).powi(2)).sqrt();
        }
        let total = dist[line.len() - 1].max(1e-4);
        let hw = path.width / 2.0;
        // Край не тоньше полутора точек экрана, иначе он лесенкой.
        let fade = (hw * path.soft).max(1.5 / self.cam.scale).min(hw);
        let cam = self.cam;
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        for k in 0..line.len() {
            let (a, b) = (line[k.saturating_sub(1)], line[(k + 1).min(line.len() - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let n = [-dy / len, dx / len];
            let d = dist[k];
            let side = |salt: u32| hw * (1.0 + path.rough * 0.8 * (value_noise(d / (0.9 + hw * 0.5), seed.wrapping_mul(2).wrapping_add(salt)) - 0.5));
            let (wl, wr) = (side(0), side(1));
            // На концах дорога тает так же, как по краю.
            let end = ((d.min(total - d)) / fade).clamp(0.0, 1.0);
            let c = line[k];
            let rows = [(wl, 0.0), (wl - fade, 1.0), (-(wr - fade), 1.0), (-wr, 0.0)];
            for (off, a) in rows {
                let p = [c[0] + n[0] * off, c[1] + n[1] * off];
                let uv = match tex {
                    None => egui::epaint::WHITE_UV,
                    Some(_) if path.along => pos2(d / tile[0], (hw - off) / tile[1]),
                    Some(_) => pos2(p[0] / tile[0], p[1] / tile[1]),
                };
                let alpha = (a * end * 255.0).round() as u8;
                let color = if tex.is_some() { Color32::from_white_alpha(alpha) } else { Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), alpha) };
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
            }
            if k > 0 {
                let (p, q) = (((k - 1) * 4) as u32, (k * 4) as u32);
                for r in 0..3 {
                    mesh.add_triangle(p + r, p + r + 1, q + r + 1);
                    mesh.add_triangle(p + r, q + r + 1, q + r);
                }
            }
        }
        painter.add(Shape::mesh(mesh));
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

    /// Начать выгрузку: плитки по размеру холста, камера и выбор запоминаются.
    pub fn start_export(&mut self, dir: PathBuf, name: String, px: u32, quit: bool) {
        let ppp = self.canvas_ppp;
        let tile = [((self.canvas.width() * ppp).floor() as u32).saturating_sub(2).max(64), ((self.canvas.height() * ppp).floor() as u32).saturating_sub(2).max(64)];
        let (w, h) = (self.map.width * px, self.map.height * px);
        let mut tiles = Vec::new();
        for ty in 0..h.div_ceil(tile[1]) {
            for tx in 0..w.div_ceil(tile[0]) {
                tiles.push([tx, ty]);
            }
        }
        self.notice = None;
        let start_level = self.level;
        let sel = self.sel.take();
        self.switch_level(0);
        self.export = Some(Export {
            dir,
            name,
            px,
            tile,
            tiles,
            next: 0,
            image: image::RgbImage::new(w, h),
            done: Vec::new(),
            doing: 0,
            start_level,
            cam: self.cam,
            sel,
            settle: 0,
            asked: false,
            quit,
        });
    }

    /// Шаг выгрузки до отрисовки кадра: поставить камеру на плитку и, когда
    /// картинки догрузились, попросить снимок.
    fn export_step(&mut self, ctx: &egui::Context) {
        if self.export.is_none()
            && let Some((dir, px)) = self.export_args.clone()
            && self.frame >= 10
            && !self.library.loading()
            && self.textures.idle()
            && self.canvas.width() > 100.0
        {
            self.export_args = None;
            let name = self.name();
            self.start_export(dir, name, px, true);
        }
        let ppp = ctx.pixels_per_point();
        let canvas = self.canvas;
        let idle = self.textures.idle() && !self.library.loading();
        let Some(e) = &mut self.export else { return };
        ctx.request_repaint();
        if e.asked {
            return;
        }
        let [tx, ty] = e.tiles[e.next];
        // Начало плитки ложится на целый пиксель экрана: плитки стыкуются без шва.
        let origin = [(canvas.min.x * ppp).ceil(), (canvas.min.y * ppp).ceil()];
        let scale = e.px as f32 / ppp;
        let world = [(tx * e.tile[0]) as f32 / e.px as f32, (ty * e.tile[1]) as f32 / e.px as f32];
        self.cam.scale = scale;
        self.cam.pan = vec2(origin[0] / ppp - canvas.min.x - world[0] * scale, origin[1] / ppp - canvas.min.y - world[1] * scale);
        e.settle += 1;
        if e.settle >= 3 && idle {
            e.asked = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Снимок пришёл: вырезать плитку в большую картинку; последняя — записать файлы.
    fn export_tile(&mut self, ctx: &egui::Context, shot: &egui::ColorImage) {
        let ppp = ctx.pixels_per_point();
        let canvas = self.canvas;
        let Some(e) = &mut self.export else { return };
        let [tx, ty] = e.tiles[e.next];
        let (ox, oy) = ((canvas.min.x * ppp).ceil() as usize, (canvas.min.y * ppp).ceil() as usize);
        let (x0, y0) = (tx * e.tile[0], ty * e.tile[1]);
        let w = e.tile[0].min(e.image.width() - x0);
        let h = e.tile[1].min(e.image.height() - y0);
        let [sw, sh] = shot.size;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (sx, sy) = (ox + x, oy + y);
                if sx < sw && sy < sh {
                    let c = shot.pixels[sy * sw + sx];
                    e.image.put_pixel(x0 + x as u32, y0 + y as u32, image::Rgb([c.r(), c.g(), c.b()]));
                }
            }
        }
        e.next += 1;
        e.asked = false;
        e.settle = 0;
        if e.next < e.tiles.len() {
            return;
        }
        // Этаж снят — следующий, пока не кончатся.
        if e.doing + 1 < self.levels.len() {
            let fresh = image::RgbImage::new(e.image.width(), e.image.height());
            e.done.push(std::mem::replace(&mut e.image, fresh));
            e.doing += 1;
            e.next = 0;
            let k = e.doing;
            self.switch_level(k);
            return;
        }
        let Some(mut e) = self.export.take() else { return };
        let last = std::mem::replace(&mut e.image, image::RgbImage::new(1, 1));
        e.done.push(last);
        self.switch_level(e.start_level);
        self.cam = e.cam;
        self.sel = e.sel;
        let result = self.write_export(&e);
        match &result {
            Ok(msg) => self.notice = Some(msg.clone()),
            Err(err) => self.error = Some(format!("Выгрузка не удалась: {err}")),
        }
        if e.quit {
            match result {
                Ok(msg) => println!("{msg}"),
                Err(err) => eprintln!("{err}"),
            }
            self.recovery.close();
            std::process::exit(0);
        }
    }

    /// Картинка, сцена Foundry и Universal VTT рядом друг с другом; у нескольких
    /// этажей — картинка и `.dd2vtt` на каждый, сцена одна, этажи в ней — уровни.
    fn write_export(&mut self, e: &Export) -> Result<String, String> {
        std::fs::create_dir_all(&e.dir).map_err(|x| x.to_string())?;
        let n = e.done.len();
        let stem = |k: usize| if n == 1 { e.name.clone() } else { format!("{}_{}", e.name, k + 1) };
        let mut jpgs = Vec::new();
        let mut srcs = Vec::new();
        let mut layouts = Vec::new();
        for (k, img) in e.done.iter().enumerate() {
            let img_path = e.dir.join(format!("{}.jpg", stem(k)));
            let mut jpg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(img).map_err(|x| x.to_string())?;
            std::fs::write(&img_path, &jpg).map_err(|x| format!("{}: {x}", img_path.display()))?;
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            layouts.push(export::layout(map, &map.room_edges()));
            srcs.push(export::foundry_src(&img_path));
            jpgs.push(jpg);
        }
        // Створки — отдельными картинками рядом со сценой: Foundry их двигает.
        let mut leaves = std::collections::HashMap::new();
        for key in layouts.iter().flat_map(|l| l.portals.iter()).filter_map(|p| p.leaf.clone()) {
            if leaves.contains_key(&key) {
                continue;
            }
            let bytes = self.textures.bytes(&key)?;
            // Foundry вращает створку вокруг левого конца картинки, а петля у FA
            // и у встроенных нарисована справа: створка уходит отражённой.
            let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(&key)))?.fliph();
            let width = img.width();
            let file = e.dir.join(format!("{}_створка_{}.png", e.name, leaves.len() + 1));
            img.save(&file).map_err(|x| format!("{}: {x}", file.display()))?;
            leaves.insert(key, export::LeafFile { src: export::foundry_src(&file), width });
        }
        // Крыши — картинками по одной, рядом со сценой.
        let mut covers: std::collections::HashMap<String, (image::RgbaImage, [f32; 2])> = std::collections::HashMap::new();
        let mut roof_tiles: Vec<Vec<export::RoofTile>> = Vec::new();
        let mut roof_count = 0;
        for k in 0..n {
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            let light = map.shadows.light();
            let mut tiles = Vec::new();
            for r in &map.roofs {
                if !covers.contains_key(&r.paint) {
                    let bytes = self.textures.bytes(&r.paint)?;
                    let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(&r.paint)))?.to_rgba8();
                    let tile = roofs::tile_cells([img.width(), img.height()]);
                    covers.insert(r.paint.clone(), (img, tile));
                }
                let (tex, tile) = &covers[&r.paint];
                let (img, origin) = roofs::render(r, tex, *tile, e.px as f32, light);
                roof_count += 1;
                let file = e.dir.join(format!("{}_крыша_{roof_count}.png", stem(k)));
                img.save(&file).map_err(|x| format!("{}: {x}", file.display()))?;
                let p = e.px as f32;
                tiles.push(export::RoofTile { src: export::foundry_src(&file), rect: [(origin[0] * p).round() as i64, (origin[1] * p).round() as i64, img.width() as i64, img.height() as i64] });
            }
            roof_tiles.push(tiles);
        }
        let levels: Vec<export::SceneLevel> = (0..n)
            .map(|k| export::SceneLevel { name: if n == 1 { e.name.clone() } else { self.level_name(k) }, image_src: srcs[k].clone(), layout: &layouts[k], roofs: std::mem::take(&mut roof_tiles[k]) })
            .collect();
        let scene = export::foundry_levels(&self.map, &e.name, &levels, e.px, &leaves);
        let scene_path = e.dir.join(format!("{}.json", e.name));
        std::fs::write(&scene_path, serde_json::to_string_pretty(&scene).unwrap_or_default()).map_err(|x| x.to_string())?;
        use base64::Engine;
        for k in 0..n {
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            let uv = export::dd2vtt(map, e.px, &base64::engine::general_purpose::STANDARD.encode(&jpgs[k]), &layouts[k]);
            std::fs::write(e.dir.join(format!("{}.dd2vtt", stem(k))), uv.to_string()).map_err(|x| x.to_string())?;
        }
        let walls: usize = layouts.iter().flat_map(|l| l.walls.iter()).map(|l| l.len().saturating_sub(1)).sum();
        let (w, h) = (e.done[0].width(), e.done[0].height());
        Ok(format!(
            "Выгружено в {}: {}картинка {w}×{h}, стен {walls}, крыш {roof_count}, проёмов {}, огней {}",
            e.dir.display(),
            if n > 1 { format!("этажей {n}, ") } else { String::new() },
            layouts.iter().map(|l| l.portals.len()).sum::<usize>(),
            layouts.iter().map(|l| l.lights.len()).sum::<usize>()
        ))
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
        if self.export.as_ref().is_some_and(|e| e.asked) {
            if let Some(image) = images.first() {
                self.export_tile(ctx, image);
            }
            ctx.request_repaint();
            return;
        }
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

/// Наложить объект на картинку объединения: `ppc` точек на клетку, `origin` —
/// левый верхний угол картинки в клетках. Поворот и размер — как на карте,
/// выборка билинейная, наложение «поверх» по прозрачности.
pub fn bake_prop(out: &mut image::RgbaImage, img: &image::RgbaImage, p: &Prop, origin: [f32; 2], ppc: f32) {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let (sw, sh) = (p.size[0] * p.scale, p.size[1] * p.scale);
    let (s, c) = p.rot.to_radians().sin_cos();
    let cs = p.corners();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for q in cs {
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    let px = |v: f32, o: f32, m: u32| (((v - o) * ppc).floor().max(0.0) as u32).min(m);
    let (x0, x1) = (px(lo[0], origin[0], out.width()), px(hi[0], origin[0], out.width()).saturating_add(1).min(out.width()));
    let (y0, y1) = (px(lo[1], origin[1], out.height()), px(hi[1], origin[1], out.height()).saturating_add(1).min(out.height()));
    let sample = |u: f32, v: f32| -> [f32; 4] {
        let (x, y) = (u * iw - 0.5, v * ih - 0.5);
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (x - fx, y - fy);
        let at = |xx: f32, yy: f32| {
            let (xx, yy) = (xx.clamp(0.0, iw - 1.0) as u32, yy.clamp(0.0, ih - 1.0) as u32);
            let q = img.get_pixel(xx, yy).0;
            // Предумноженные цвета: на прозрачном крае нет тёмной каймы.
            let a = q[3] as f32 / 255.0;
            [q[0] as f32 * a, q[1] as f32 * a, q[2] as f32 * a, q[3] as f32]
        };
        let (a, b, cc, d) = (at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0));
        std::array::from_fn(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (cc[k] * (1.0 - tx) + d[k] * tx) * ty)
    };
    for y in y0..y1 {
        for x in x0..x1 {
            let w = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
            let (dx, dy) = (w[0] - p.pos[0], w[1] - p.pos[1]);
            // В систему объекта: поворот назад.
            let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
            let (u, v) = (lx / sw + 0.5, ly / sh + 0.5);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let src = sample(u, v);
            let sa = src[3] / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let dst = out.get_pixel(x, y).0;
            let da = dst[3] as f32 / 255.0;
            let oa = sa + da * (1.0 - sa);
            let ch = |k: usize| ((src[k] + dst[k] as f32 * da * (1.0 - sa)) / oa).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, image::Rgba([ch(0), ch(1), ch(2), (oa * 255.0).round() as u8]));
        }
    }
}

/// Плавный шум 0..1 вдоль одной оси: случайные значения в целых точках,
/// между ними — сглаженная середина.
fn value_noise(x: f32, seed: u32) -> f32 {
    let h = |i: i32| {
        let mut v = (i as u32).wrapping_mul(0x9E37_79B1) ^ seed.wrapping_mul(0x85EB_CA77);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2C1B_3C6D);
        v ^= v >> 12;
        (v & 0xFFFF) as f32 / 65535.0
    };
    let i = x.floor();
    let t = x - i;
    let t = t * t * (3.0 - 2.0 * t);
    h(i as i32) * (1.0 - t) + h(i as i32 + 1) * t
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_prop_lands_rotated_where_the_map_shows_it() {
        // Картинка 2×1 клетки: левая половина красная, правая синяя.
        let img = image::RgbaImage::from_fn(64, 32, |x, _| if x < 32 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
        let p = Prop { asset: String::new(), pos: [1.0, 1.0], size: [2.0, 1.0], rot: 90.0, scale: 1.0, shadow: None };
        let mut out = image::RgbaImage::new(64, 64);
        bake_prop(&mut out, &img, &p, [0.0, 0.0], 32.0);
        // Поворот по часовой на 90°: левый (красный) конец уходит вверх.
        assert_eq!(out.get_pixel(32, 8).0, [255, 0, 0, 255]);
        assert_eq!(out.get_pixel(32, 56).0, [0, 0, 255, 255]);
        assert_eq!(out.get_pixel(4, 32)[3], 0, "сбоку от повёрнутого объекта пусто");
    }

    #[test]
    fn scatter_random_numbers_stay_in_range_and_vary() {
        let mut s = Scatter::default();
        let v: Vec<f32> = (0..1000).map(|_| s.rand()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "среднее {mean}");
    }
}
