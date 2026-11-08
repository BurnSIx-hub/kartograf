//! Окно редактора: меню, панель инструментов с библиотекой, холст и строка состояния.

use crate::export;
use crate::generate::{self, Options};
use crate::library::{self, Kind, Library, PX_PER_CELL, Textures};
use crate::blend::Blend;
use crate::map::{self, Coat, Layer, Map, Material, Paint, Portal, PortalKind, Prop, Segment, Wall};
use crate::mask::{self, Brush, Mask, RES, Shape as Tip};
use crate::palette::Palette;
use crate::plan::Plan;
use crate::procgen;
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
const THUMB: f32 = 84.0;

const BG: Color32 = Color32::from_rgb(27, 28, 31);
const PANEL: Color32 = Color32::from_rgb(37, 39, 43);
const FIELD: Color32 = Color32::from_rgb(48, 50, 55);
const LINE: Color32 = Color32::from_rgb(58, 61, 67);
const TEXT: Color32 = Color32::from_rgb(232, 230, 225);
const MUTED: Color32 = Color32::from_rgb(155, 156, 159);
const ACCENT: Color32 = Color32::from_rgb(224, 160, 74);
const ON_ACCENT: Color32 = Color32::from_rgb(36, 24, 7);

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

/// Ячейка левого столбца: родственные инструменты под одной кнопкой.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Select,
    Brush,
    Wall,
    Roof,
    Object,
    Path,
    Text,
}

impl Group {
    const ALL: [Group; 7] = [Group::Select, Group::Brush, Group::Wall, Group::Roof, Group::Object, Group::Path, Group::Text];

    fn has(self, tool: Tool) -> bool {
        match self {
            Group::Select => tool == Tool::Select,
            Group::Brush => matches!(tool, Tool::Dungeon | Tool::Floor | Tool::Shade | Tool::Erase),
            Group::Wall => matches!(tool, Tool::Wall | Tool::Door),
            Group::Roof => tool == Tool::Roof,
            Group::Object => matches!(tool, Tool::Object | Tool::Scatter),
            Group::Path => tool == Tool::Path,
            Group::Text => tool == Tool::Text,
        }
    }

    fn first(self) -> Tool {
        match self {
            Group::Select => Tool::Select,
            Group::Brush => Tool::Dungeon,
            Group::Wall => Tool::Wall,
            Group::Roof => Tool::Roof,
            Group::Object => Tool::Object,
            Group::Path => Tool::Path,
            Group::Text => Tool::Text,
        }
    }
}

/// Строка списка режимов ячейки.
#[derive(Clone, Copy)]
enum Mode {
    Tool(Tool),
    Door(PortalKind),
    Roof(map::RoofShape),
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
    /// Стена комнаты: ключ отрезка и весь ли контур.
    Edge((i32, i32), bool),
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
    /// `--plan файл.json`: построить карту по плану при запуске.
    pub plan: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub textures_dir: Option<PathBuf>,
    #[serde(default)]
    pub packs: Vec<PathBuf>,
    /// Объекты встают по сетке; Alt на время меняет на обратное.
    #[serde(default = "yes")]
    pub snap: bool,
    /// Генераторы строят своими ассетами из библиотеки, а не встроенными.
    #[serde(default)]
    pub gen_library: bool,
    /// Выгрузка: кроны деревьев отдельными тайлами над токенами, в картинке пни.
    #[serde(default = "yes")]
    pub export_canopies: bool,
    /// Крыши в редакторе: 0 — видно, 1 — сквозь, 2 — скрыть. Запоминается.
    #[serde(default)]
    pub roof_view: u8,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self { textures_dir: None, packs: Vec::new(), snap: true, gen_library: false, export_canopies: true, roof_view: 0 }
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
    /// Пни под кронами из библиотеки (генерация из библиотеки); пусто — встроенный.
    stumps: Vec<String>,
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
    /// Кроны деревьев — тайлами над токенами; иначе деревья остаются в картинке как есть.
    canopies: bool,
    /// Модуль: обновить последнюю сцену, а не заводить новую.
    update: bool,
}

/// Окно выгрузки: сколько пикселей на клетку; в модуле ещё имя сцены и новая/последняя.
struct ExportDialog {
    px: u32,
    name: String,
    update: bool,
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
    /// Затухание тени кистью: 0 — ровная до края, 1 — тает от середины мазка.
    pub shade_soft: f32,
    shade_erase: bool,
    /// Образец новой подписи; тащат подпись — откуда взяли.
    pub label_style: map::Label,
    label_grab: Option<(usize, [f32; 2])>,
    /// Образец новой крыши, вид крыш, протяжка новой и перенос стоящей.
    pub roof_style: map::Roof,
    pub roof_view: RoofView,
    roof_from: Option<(i32, i32)>,
    tap_from: Option<Pos2>,
    groups: Vec<Vec<usize>>,
    /// Открытая шторка: индексы вариантов и миниатюра, над которой она висит.
    pub drawer: Option<(Vec<usize>, Rect)>,
    drawer_frame: u64,
    /// Где на экране первая миниатюра библиотеки и открытая шторка — для самопроверки.
    pub first_thumb: Rect,
    /// План из командной строки ждёт, пока прочитаются паки.
    pending_plan: Option<PathBuf>,
    pub drawer_area: Rect,
    family_pick: std::collections::HashMap<String, String>,
    pub fly: Option<usize>,
    fly_rect: Rect,
    fly_frame: u64,
    pub group_last: [Tool; 7],
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
    pub update: crate::update::Updater,
    pub settings: Settings,
    pub library: Library,
    pub textures: Textures,
    blend: Blend,
    pub browse: Kind,
    pub query: String,
    filtered: Vec<usize>,
    filter_key: (Kind, String, usize, usize),
    /// Видимая полоса картинки окна поперёк стены, доля высоты холста.
    window_bands: std::collections::HashMap<String, f32>,
    /// Стекло к подоконнику FA (ключ подоконника и большое ли окно → окно).
    window_glass: std::collections::HashMap<(String, bool), Option<String>>,
    /// Библиотека показывает только то, что уже стоит на карте.
    used_only: bool,
    /// Время года последнего леса: окно генератора открывается с ним.
    gen_season: crate::plan::Season,
    gen_cave: crate::plan::CaveKind,
    gen_district: crate::plan::District,
    gen_dungeon: crate::plan::DungeonKind,
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
    /// Под каким именем эта карта последний раз ушла на сцену (модуль).
    scene_name: Option<String>,
    /// Выгрузка из командной строки: куда и с каким разрешением.
    export_args: Option<(PathBuf, u32)>,
    /// Сообщение в строке состояния: что сделано.
    pub notice: Option<String>,
    confirm_close: bool,
    allow_close: bool,
}

pub fn default_data_dir() -> PathBuf {
    #[cfg(target_arch = "wasm32")]
    return PathBuf::from("/kartograf");
    #[cfg(not(target_arch = "wasm32"))]
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
        let online = args.selftest.is_none() && args.shot.is_none() && args.export.is_none();
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        apply_theme(&cc.egui_ctx);
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
            shade_soft: 0.5,
            shade_erase: false,
            label_style: map::Label::new("Комната".into(), [0.0, 0.0]),
            label_grab: None,
            roof_style: map::Roof::new([0.0, 0.0], [1.0, 1.0], map::RoofShape::Gable, library::DEFAULT_ROOF.into()),
            roof_view: match settings.roof_view {
                1 => RoofView::Ghost,
                2 => RoofView::Hidden,
                _ => RoofView::Auto,
            },
            roof_from: None,
            tap_from: None,
            groups: Vec::new(),
            drawer: None,
            drawer_frame: 0,
            family_pick: Default::default(),
            fly: None,
            fly_rect: Rect::NOTHING,
            fly_frame: 0,
            group_last: Group::ALL.map(Group::first),
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
            update: crate::update::Updater::start(online.then(|| cc.egui_ctx.clone())),
            settings,
            library,
            textures: Textures::new(&cc.egui_ctx),
            blend: Blend::default(),
            browse: Kind::Floor,
            query: String::new(),
            filtered: Vec::new(),
            filter_key: (Kind::Floor, String::from("\u{0}"), 0, 0),
            window_bands: std::collections::HashMap::new(),
            window_glass: std::collections::HashMap::new(),
            used_only: false,
            gen_season: crate::plan::Season::Summer,
            gen_cave: crate::plan::CaveKind::Stone,
            gen_district: crate::plan::District::Mixed,
            gen_dungeon: crate::plan::DungeonKind::Crypt,
            first_thumb: Rect::NOTHING,
            pending_plan: None,
            drawer_area: Rect::NOTHING,
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
            scene_name: None,
            export_args: args.export.map(|d| (d, args.px.unwrap_or(EXPORT_PX))),
            notice: None,
            confirm_close: false,
            allow_close: false,
        };
        if let Some(file) = args.file {
            app.open_path(&file);
        } else if app.test.is_none() && app.shot.is_none() && app.export_args.is_none() && args.plan.is_none() && app.orphans.is_empty() {
            // Запуск без карты — сразу спросить размер новой.
            app.new_map = Some(NewMap { width: app.map.width, height: app.map.height });
        }
        // План строится, когда прочитаны паки: иначе план «из библиотеки» взял бы встроенное.
        app.pending_plan = args.plan;
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
            Sel::Edge(..) => true,
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
            let mut weights = if self.brush.shape == Tip::Oval { mask::oval(w, h, a, b) } else { mask::cells(w, h, a, b) };
            mask::soften(w, h, &mut weights, self.shade_soft);
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
        let mut brush = self.brush;
        if self.tool == Tool::Shade {
            brush.hardness = 1.0 - self.shade_soft;
        }
        let mut weights = mask::dab(self.map.rooms.w, self.map.rooms.h, &brush, w, self.dabs);
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
            let edges = self.map.room_edges();
            self.map.sync_edge_looks(&edges);
            self.edges = edges;
            self.edges_dirty = false;
        }
        &self.edges
    }

    /// Отрезки выбранной стены комнаты: прямой участок или весь её контур.
    fn edge_run(&self, key: (i32, i32), whole: bool) -> Vec<Segment> {
        for line in mask::join(self.edges.clone()) {
            let Some(k) = line.windows(2).position(|p| map::edge_key(&(p[0], p[1])) == key) else { continue };
            let seg = |i: usize| (line[i], line[i + 1]);
            if whole {
                return (0..line.len() - 1).map(seg).collect();
            }
            let dir = |p: [f32; 2], q: [f32; 2]| {
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                [dx / l, dy / l]
            };
            let d0 = dir(line[k], line[k + 1]);
            let same = |i: usize| {
                let e = dir(line[i], line[i + 1]);
                d0[0] * e[0] + d0[1] * e[1] > 0.9995
            };
            let (mut s, mut e) = (k, k);
            while s > 0 && same(s - 1) {
                s -= 1;
            }
            while e + 2 < line.len() && same(e + 1) {
                e += 1;
            }
            return (s..=e).map(seg).collect();
        }
        Vec::new()
    }

    /// Все стоящие стены — в один вид.
    fn apply_wall_style_to_all(&mut self) {
        self.checkpoint();
        let st = self.map.wall_style.clone();
        let edges = self.room_edges().to_vec();
        self.map.set_edge_look(&edges, &st);
        for w in &mut self.map.walls {
            w.style = None;
        }
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
    fn editing_path(&self) -> bool {
        matches!(self.sel, Some(Sel::Path(_))) && matches!(self.tool, Tool::Select | Tool::Path)
    }

    /// Выбранная дорога перекрашивается в выбранный материал.
    fn recolor_path(&mut self) {
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
    fn put_wall_style(&mut self, st: map::WallStyle) {
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
        let palette = if library_mode || cyber {
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
        } else {
            Palette::builtin()
        };
        match procgen::build(&mut self.map, plan, self.material, palette) {
            Ok(r) => {
                // Сид пустого плана случайный и наружу не выходит — тогда карта без сида.
                self.map.seed = plan.seed.clone();
                // Время года — у всего, что под открытым небом, кроме пещеры и подземелья.
                let outdoor = matches!(plan.kind, generate::Kind::Forest | generate::Kind::Town | generate::Kind::House | generate::Kind::Tavern | generate::Kind::Arena);
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
            }));
        }
        true
    }

    // ----- файлы -----

    fn reset_doc(&mut self, mut map: Map, path: Option<PathBuf>) {
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
        let dialog = crate::plat::FileDialog::new().add_filter("Карта", &[map::EXT]).set_file_name(format!("{}.{}", self.name(), map::EXT));
        if let Some(path) = dialog.save_file() {
            self.save_to(&path);
        }
    }

    fn open(&mut self) {
        if let Some(path) = crate::plat::FileDialog::new().add_filter("Карта", &[map::EXT]).pick_file() {
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

    fn frame_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame += 1;
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
        // Как в привычных редакторах карт: слева инструменты и их настройки, справа библиотека.
        egui::Panel::left("tools").exact_size(64.0).resizable(false).frame(egui::Frame::new().fill(PANEL)).show(ui, |ui| self.tool_cells(ui));
        egui::Panel::left("options").default_size(250.0).size_range(200.0..=420.0).resizable(true).show(ui, |ui| self.options_bar(ui));
        egui::Panel::right("library").default_size(330.0).size_range(200.0..=700.0).resizable(true).show(ui, |ui| self.shelf(ui));
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

    fn update_window(&mut self, ctx: &egui::Context) {
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

    fn windows(&mut self, ctx: &egui::Context) {
        self.update_window(ctx);
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
                    ui.weak("Картинки лягут в данные Foundry, в kartograf/<название сцены>.");
                    ui.add_space(6.0);
                    go = ui.add_enabled(!dlg.name.trim().is_empty(), egui::Button::new("На сцену")).clicked();
                } else {
                    ui.weak("Рядом лягут картинка, сцена для Foundry (.json), Universal VTT (.dd2vtt) и картинки створок, крыш и крон — их надо держать рядом со сценой. В Foundry: создать сцену, правой кнопкой — «Import Data», выбрать .json.");
                    ui.add_space(6.0);
                    go = ui.button("Выбрать место и выгрузить").clicked();
                }
            });
            if go && cfg!(target_arch = "wasm32") {
                // Своя папка на каждую выгрузку: прежние сцены не теряют картинок,
                // а обновлённая не берёт старую картинку из кэша браузера.
                let name: String = dlg.name.trim().chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect();
                let stamp = crate::plat::SystemTime::now().duration_since(crate::plat::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                self.start_export(PathBuf::from("kartograf").join(&name).join(stamp.to_string()), name.clone(), dlg.px, false);
                if let Some(e) = &mut self.export {
                    e.update = dlg.update;
                }
                self.scene_name = Some(name);
            } else if go {
                let dir = export::foundry_data_dir().map(|d| d.join("kartograf"));
                if let Some(d) = &dir {
                    let _ = std::fs::create_dir_all(d);
                }
                let mut dialog = crate::plat::FileDialog::new().add_filter("Сцена Foundry", &["json"]).set_file_name(format!("{}.json", self.name()));
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
                    } else if matches!(opt.kind, generate::Kind::Town | generate::Kind::House | generate::Kind::Tavern | generate::Kind::Arena) {
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

    fn set_door_kind(&mut self, kind: PortalKind) {
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
                        let mut o = Options { kind, seed: generate::new_seed(), library: self.settings.gen_library, season: self.gen_season, cave: self.gen_cave, district: self.gen_district, dungeon: self.gen_dungeon, ..Default::default() };
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
    fn biome_ui(&mut self, ui: &mut egui::Ui) {
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

    /// Подсказка к инструменту — в строке состояния.
    fn hint(&self) -> &'static str {
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
            Tool::Path => "Щелчки ставят точки — линия через них плавная; протяжка рисует от руки. Enter, двойной щелчок или правая кнопка — закончить. Материал — в строке над картой: земля — тропа, вода — ручей, ткань — ковёр.",
            Tool::Scatter => "Ведите кистью — объекты из набора ложатся вразнобой. Alt — стирать их. Ctrl с колесом — размер кисти. Объекты в набор — щелчком на полке.",
            Tool::Object => "Щелчок ставит объект, протяжка двигает уже стоящий. Alt — на время наоборот с привязкой, Alt с колесом — размер, R — поворот.",
        }
    }

    /// Левый столбец: семь ячеек на всю высоту. Ячейка — инструмент, что был в ней
    /// последним; стрелка или правая кнопка открывают её режимы.
    fn tool_cells(&mut self, ui: &mut egui::Ui) {
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
    fn mode_label(&self) -> String {
        match self.tool {
            Tool::Door => self.door_kind.label().to_owned(),
            Tool::Roof => self.roof_style.shape.label().to_owned(),
            t => t.label().to_owned(),
        }
    }

    fn group_modes(&self, group: Group) -> Vec<(String, Mode)> {
        let tool = |t: Tool| (format!("{}   {}", t.label(), t.key().name()), Mode::Tool(t));
        match group {
            Group::Brush => [Tool::Dungeon, Tool::Floor, Tool::Shade, Tool::Erase].map(tool).to_vec(),
            Group::Object => [Tool::Object, Tool::Scatter].map(tool).to_vec(),
            Group::Wall => std::iter::once(tool(Tool::Wall)).chain(PortalKind::ALL.map(|k| (k.label().to_owned(), Mode::Door(k)))).collect(),
            Group::Roof => map::RoofShape::ALL.map(|s| (s.label().to_owned(), Mode::Roof(s))).to_vec(),
            _ => Vec::new(),
        }
    }

    /// Всплывающий список режимов рядом с ячейкой.
    fn tool_flyout(&mut self, ctx: &egui::Context) {
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
    fn levels_overlay(&mut self, ctx: &egui::Context) {
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

    fn material_combo(&mut self, ui: &mut egui::Ui) {
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
    fn material_picker(&mut self, ui: &mut egui::Ui) {
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
    fn path_panel(&mut self, ui: &mut egui::Ui) {
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
    fn options_bar(&mut self, ui: &mut egui::Ui) {
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
                    Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade => {
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
    fn shadows_ui(&mut self, ui: &mut egui::Ui) {
        let mut sh = self.map.shadows.clone();
        let on = ui.checkbox(&mut sh.on, "Тени от стен и объектов");
        let mut rs = Vec::new();
        if sh.on {
            rs.push(ui.add(egui::Slider::new(&mut sh.angle, 0.0..=345.0).step_by(15.0).text("куда падают, °")));
            rs.push(ui.add(egui::Slider::new(&mut sh.dist, 0.0..=3.0).text("длина, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.blur, 0.1..=1.0).text("размытие, кл.")));
            rs.push(ui.add(egui::Slider::new(&mut sh.opacity, 0.05..=1.0).text("темнота")));
            rs.push(ui.add(egui::Slider::new(&mut sh.fade, 0.0..=1.0).text("затухание")));
        }
        if on.changed() || rs.iter().any(|r| r.drag_started() || (r.changed() && !r.dragged())) {
            self.checkpoint();
        }
        self.map.shadows = sh;

    }

    /// Полка ассетов: слева разделы, справа поиск и большие миниатюры.
    fn shelf(&mut self, ui: &mut egui::Ui) {
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
    fn variant_drawer(&mut self, ctx: &egui::Context) -> Option<String> {
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

    fn browser(&mut self, ui: &mut egui::Ui) {
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
            ui.separator();
            let hint = self.hint();
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate()).on_hover_text(hint);
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
            self.tap_from = None;
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
        // Выгнутая стена остаётся какой была, а не берёт вид новых стен.
        let run = self.edges.iter().min_by(|x, y| map::segment_distance(mid, x.0, x.1).total_cmp(&map::segment_distance(mid, y.0, y.1))).copied();
        let keep = run.map(|s| self.map.edge_look(map::edge_key(&s)));
        let fresh = keep.map(|k| std::mem::replace(&mut self.map.wall_style, k));
        self.map.bend_room(a, b, h, material);
        self.edges_dirty = true;
        self.room_edges();
        if let Some(st) = fresh {
            self.map.wall_style = st;
        }
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

    /// Нажатие для инструментов, ставящих по точке: отпустили — вот где нажимали.
    /// Быстрый щелчок на ходу тянет мышь на десятки точек, для них это не протяжка.
    fn tap(&mut self, ui: &egui::Ui, resp: &egui::Response) -> Option<Pos2> {
        let (pressed, released, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.press_origin().or(i.pointer.interact_pos())));
        if pressed && resp.contains_pointer() {
            self.tap_from = at;
        }
        if released { self.tap_from.take() } else { None }
    }

    fn handle_tool(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let rect = self.canvas;
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let alt = ui.input(|i| i.modifiers.alt);
        if self.handle_bend(ui, resp, pointer) {
            return;
        }
        let tap = self.tap(ui, resp);
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
                if let Some(p) = tap {
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
                if let Some(p) = tap {
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
                let moved = self.moving.is_some();
                self.drag_prop(ui, resp, pointer, alt);
                if let (Some(p), false) = (tap, moved) {
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

        // Объекты — под стенами, как мебель у стены; что выше слоем — позже.
        let shadows = self.map.shadows.clone();
        self.draw_props(painter, canvas, view, exporting, i16::MIN..map::ABOVE_WALLS);

        // Стены комнат и нарисованные вручную — за вычетом проёмов. Тени стен
        // ложатся и на объекты у стены.
        let style = self.map.wall_style.clone();
        let mut looks: Vec<(map::WallStyle, Vec<Segment>)> = Vec::new();
        for s in self.room_edges().to_vec() {
            let st = self.map.edge_look(map::edge_key(&s));
            match looks.iter_mut().find(|l| l.0 == st) {
                Some(l) => l.1.push(s),
                None => looks.push((st, vec![s])),
            }
        }
        for l in &mut looks {
            l.1 = map::cut_portals_wide(&l.1, &self.map.portals, l.0.width / 2.0);
        }
        if shadows.on {
            // Стена под крышей своей тени не даёт: её накрывает тень крыши, а вдвоём они
            // чернили двери и всё у стены на теневой стороне дома.
            let roofs = if self.roof_view == RoofView::Hidden && self.export.is_none() { Vec::new() } else { self.map.roofs.clone() };
            let open = |segs: &[Segment]| -> Vec<Segment> {
                segs.iter().copied().filter(|s| !roofs.iter().any(|r| r.contains([(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0]))).collect()
            };
            for sh in shadows.layers() {
                for (st, edges) in &looks {
                    self.draw_wall_shadow(painter, canvas, &open(edges), st.width, &sh);
                }
                for w in self.map.walls.clone() {
                    let width = w.style.as_ref().map_or(style.width, |s| s.width);
                    self.draw_wall_shadow(painter, canvas, &open(&map::cut_portals_wide(&w.segments().collect::<Vec<_>>(), &self.map.portals, width / 2.0)), width, &sh);
                }
            }
        }
        if let Some(Sel::Edge(key, whole)) = self.sel {
            let width = self.map.edge_look(key).width;
            for (a, b) in self.edge_run(key, whole) {
                painter.line_segment([cam.to_screen(canvas, a), cam.to_screen(canvas, b)], Stroke::new(width * s + 6.0, Color32::from_rgb(120, 180, 255)));
            }
        }
        for (st, edges) in &looks {
            self.draw_walls(painter, canvas, st, edges);
        }
        let walls = self.map.walls.clone();
        for (i, wall) in walls.iter().enumerate() {
            let segs: Vec<Segment> = wall.segments().collect();
            let own = wall.style.as_ref().unwrap_or(&style);
            if self.sel == Some(Sel::Wall(i)) {
                for (a, b) in &segs {
                    painter.line_segment([cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], Stroke::new(own.width * s + 6.0, Color32::from_rgb(120, 180, 255)));
                }
            }
            self.draw_walls(painter, canvas, own, &map::cut_portals_wide(&segs, &self.map.portals, own.width / 2.0));
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
                    let ghost = Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
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

        // Слой «над стенами» — между стенами и крышами.
        self.draw_props(painter, canvas, view, exporting, map::ABOVE_WALLS..map::ABOVE_ROOFS);

        // Крыши — над всем, что в доме; тень их ложится и в выгрузку, а сами
        // они уходят в Foundry отдельными тайлами.
        let roof_alpha = if exporting { 0.0 } else { self.roof_alpha() };
        // Скрытые крыши — только в редакторе: в выгрузку они идут всегда, с тенями.
        if self.roof_view != RoofView::Hidden || exporting {
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

        self.draw_props(painter, canvas, view, exporting, map::ABOVE_ROOFS..i16::MAX);

        // Подписи — поверх всего, с тёмной каймой, чтобы читались на любом полу.
        for (i, label) in self.map.labels.clone().iter().enumerate() {
            if exporting && label.gm {
                continue;
            }
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
        self.draw_across(painter, canvas, key, s, e, None)
    }

    /// Как `draw_along`, но картинка отражена поперёк: та же сторона смотрит туда же.
    fn draw_mirrored(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2]) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let Some((_, px)) = self.textures.get(key, false) else { return false };
        let half = len * px[1] as f32 / px[0].max(1) as f32 / 2.0;
        let (nx, ny) = (-dy / len * half, dx / len * half);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let flipped = [pos2(0.0, 1.0), pos2(1.0, 1.0), pos2(1.0, 0.0), pos2(0.0, 0.0)];
        self.quad(painter, canvas, key, c, flipped, Color32::WHITE)
    }

    /// Картинка, нарисованная стоя, вдоль отрезка: её высота идёт по отрезку.
    fn draw_turned(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2], thick: f32) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return false;
        }
        let (nx, ny) = (-dy / len * thick / 2.0, dx / len * thick / 2.0);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let turned = [pos2(0.0, 0.0), pos2(0.0, 1.0), pos2(1.0, 1.0), pos2(1.0, 0.0)];
        self.quad(painter, canvas, key, c, turned, Color32::WHITE)
    }

    /// То же, но толщина холста поперёк отрезка задана (`thick`, клетки).
    fn draw_across(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2], thick: Option<f32>) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let Some((_, px)) = self.textures.get(key, false) else { return false };
        let half = thick.unwrap_or(len * px[1] as f32 / px[0].max(1) as f32) / 2.0;
        let (nx, ny) = (-dy / len * half, dx / len * half);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        self.quad(painter, canvas, key, c, full, Color32::WHITE)
    }

    /// Толщина холста подоконника поперёк стены: видимая доска — чуть толще стены,
    /// как в Dungeondraft. Нарисована FA под свои стены, у нас они тоньше.
    fn sill_thick(&mut self, key: &str, wall_width: f32) -> f32 {
        wall_width * 1.1 / self.band(key)
    }

    /// Доля холста поперёк, занятая рисунком (подоконник, косяк, полоса стены).
    fn band(&mut self, key: &str) -> f32 {
        let sources = &self.textures.sources;
        *self.window_bands.entry(key.to_owned()).or_insert_with(|| library::visible_box(sources, key).map_or(1.0, |[_, t, _, b]| (b - t).max(0.05)))
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
        if p.own_window() {
            // В разрезе стены подоконник FA и рама поверх, по всей длине проёма.
            let key = p.asset.clone().unwrap_or_default();
            let thick = self.sill_thick(&key, wall_width);
            if !self.draw_across(painter, canvas, &key, p.a, p.b, Some(thick)) {
                let (nx, ny) = (-uy * wall_width * 0.3, ux * wall_width * 0.3);
                let q = [[p.a[0] + nx, p.a[1] + ny], [p.b[0] + nx, p.b[1] + ny], [p.b[0] - nx, p.b[1] - ny], [p.a[0] - nx, p.a[1] - ny]];
                let q: Vec<Pos2> = q.iter().map(|&w| self.cam.to_screen(canvas, w)).collect();
                painter.add(Shape::convex_polygon(q, Color32::from_rgb(150, 196, 220), Stroke::new(1.5, Color32::from_rgb(20, 16, 13))));
            } else {
                if let Some(f) = p.frame.clone() {
                    self.draw_across(painter, canvas, &f, p.a, p.b, Some(thick));
                }
                // Само окно — стекло с переплётом поверх подоконника.
                let assets = &self.library.assets;
                let glass = self.window_glass.entry((key.clone(), len >= 1.5)).or_insert_with(|| library::window_glass(assets, &key, len)).clone();
                if let Some(g) = glass {
                    // Часть стёкол FA нарисована стоя: такое кладётся вдоль проёма
                    // поворотом, иначе ложилось поперёк стены, ребром.
                    let standing = self.textures.get(&g, false).is_some_and(|(_, px)| px[1] > px[0]);
                    if standing {
                        self.draw_turned(painter, canvas, &g, p.a, p.b, thick);
                    } else {
                        self.draw_across(painter, canvas, &g, p.a, p.b, Some(thick));
                    }
                }
            }
            return;
        }
        if let Some(f) = p.frame.clone() {
            let size = self.asset_size(&f);
            // Косяк подобран под целую ширину; проём уже или шире — стойки те же,
            // меняется просвет между ними.
            let along = (size[0] + len - len.round()).max(len);
            let (hx, hy) = (ux * along / 2.0, uy * along / 2.0);
            // Стойки косяка ровно в толщину стены. Рисунок занимает не весь холст
            // поперёк (у FA — две пятых), и холст растягивается так, чтобы в стену
            // попала видимая часть: иначе стойки выходили тоньше стены и она торчала.
            // У стен FA камень тоже не во всю полосу — стойка по видимой стене.
            let band = self.band(&f);
            let seen = match self.map.wall_style.paint.clone() {
                Paint::Texture(w) => self.band(&w),
                Paint::Color(_) => 1.0,
            };
            let thick = Some(wall_width * seen / band);
            self.draw_across(painter, canvas, &f, [mid[0] - hx, mid[1] - hy], [mid[0] + hx, mid[1] + hy], thick);
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
                // Вторая — отражение первой: поворот на 180° показывал её другой стороной.
                let key = p.asset.clone().unwrap_or_else(|| library::DEFAULT_DOOR.into());
                self.draw_along(painter, canvas, &key, mid, p.a);
                self.draw_mirrored(painter, canvas, &key, mid, p.b);
            }
            PortalKind::Window => {
                let key = p.asset.clone();
                if !key.is_some_and(|k| self.draw_along(painter, canvas, &k, p.a, p.b)) {
                    plain(painter, &self.cam, wall_width * 0.3, Color32::from_rgb(150, 196, 220));
                }
            }
            // Тайный ход виден только ведущему: пунктир по стене, в выгрузку не попадает.
            PortalKind::Secret if exporting => {}
            PortalKind::Secret => {
                let pts: Vec<Pos2> = to_screen(rect_around(wall_width * 0.5 + 0.08), &self.cam);
                let mut ring = pts.clone();
                ring.push(pts[0]);
                painter.extend(Shape::dashed_line(&ring, Stroke::new(1.5, ACCENT), 5.0, 4.0));
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
        // За краем карты берётся крайняя точка: иначе слой тает к границе и у неё виден фон.
        let (last_i, last_j) = (m.w as isize - 1, m.h as isize - 1);
        let (map_w, map_h) = (m.w as f32 / res, m.h as f32 / res);
        let value = |i: isize, j: isize| {
            let (i, j) = (i.clamp(0, last_i), j.clamp(0, last_j));
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
            let w = [((fx + 0.5) / res).clamp(0.0, map_w), ((fy + 0.5) / res).clamp(0.0, map_h)];
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
                    } else if detail > 1 {
                        // Ровный кусок вблизи — веер с теми же точками по краям, что у
                        // дроблёного соседа: стык без Т-образных узлов и щелей. Раньше
                        // сплошной кусок раздувался на полпикселя, и полоса раздува
                        // ложилась на соседей светлой сеткой по шагу маски (зимний лес
                        // в Foundry, 2026-11-04).
                        let n = detail as f32;
                        let (x0, y0, d) = (i as f32, j as f32, s as f32);
                        mesh.vertices.push(vertex(x0 + d / 2.0, y0 + d / 2.0, a[0]));
                        let ring: Vec<(f32, f32)> = (0..detail)
                            .map(|k| (k as f32 / n, 0.0))
                            .chain((0..detail).map(|k| (1.0, k as f32 / n)))
                            .chain((0..detail).map(|k| (1.0 - k as f32 / n, 1.0)))
                            .chain((0..detail).map(|k| (0.0, 1.0 - k as f32 / n)))
                            .collect();
                        for &(u, v) in &ring {
                            mesh.vertices.push(vertex(x0 + u * d, y0 + v * d, a[0]));
                        }
                        let count = ring.len() as u32;
                        for k in 0..count {
                            mesh.add_triangle(base, base + 1 + k, base + 1 + (k + 1) % count);
                        }
                    } else {
                        for ((x, y), alpha) in corners.into_iter().zip(a) {
                            mesh.vertices.push(vertex(x as f32, y as f32, alpha));
                        }
                        mesh.add_triangle(base, base + 1, base + 2);
                        mesh.add_triangle(base, base + 2, base + 3);
                    }
                }
                i += s;
            }
            j += s;
        }
        if mesh.is_empty() {
            return;
        }
        if material.vary && tex.is_some() {
            self.blend.paint(painter, mesh);
        } else {
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
            // Скат темнит и то, что видно сквозь него: у полупрозрачной крыши тень та же, что у целой.
            let a = ((1.0 - s * (1.0 - alpha)) * 255.0).round().clamp(0.0, 255.0) as u8;
            if tex.is_some() { Color32::from_rgba_premultiplied(v, v, v, a) } else { Color32::from_rgba_premultiplied((v as f32 * 0.6) as u8, (v as f32 * 0.35) as u8, (v as f32 * 0.3) as u8, a) }
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
        // Парапет плоской крыши: тень на кровлю, потом сам отлив.
        if let Some(pk) = roof.parapet() {
            let (lo, hi) = (roof.outline()[0], roof.outline()[2]);
            let w = roofs::PARAPET;
            let mut shade = egui::Mesh::default();
            let dim = Color32::from_rgba_premultiplied(0, 0, 0, (alpha * (1.0 - roofs::PARAPET_SHADE) * 255.0) as u8);
            for (poly, ..) in roofs::parapet_sides([lo[0] + w, lo[1] + w], [hi[0] - w, hi[1] - w], roofs::PARAPET_SHADOW) {
                let base = shade.vertices.len() as u32;
                for (k, p) in poly.iter().enumerate() {
                    shade.colored_vertex(cam.to_screen(canvas, *p), if k < 2 { dim } else { Color32::TRANSPARENT });
                }
                shade.add_triangle(base, base + 1, base + 2);
                shade.add_triangle(base, base + 2, base + 3);
            }
            painter.add(Shape::mesh(shade));
            if let Some((t, _)) = self.textures.get(pk, false) {
                let mut band = egui::Mesh::with_texture(t.id());
                let color = tint(1.0);
                for (poly, o, along, down) in roofs::parapet_sides(lo, hi, w) {
                    let base = band.vertices.len() as u32;
                    for p in poly {
                        let d = [p[0] - o[0], p[1] - o[1]];
                        let uv = pos2((d[0] * along[0] + d[1] * along[1]) / roofs::PARAPET_TILE, (d[0] * down[0] + d[1] * down[1]) / w);
                        band.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
                    }
                    band.add_triangle(base, base + 1, base + 2);
                    band.add_triangle(base, base + 2, base + 3);
                }
                painter.add(Shape::mesh(band));
            }
            // Сливы и воронки — поверх, клеткой с поворотом.
            for (key, at, rot) in roof.fixtures() {
                let Some((t, _)) = self.textures.get(key, false) else { continue };
                let mut item = egui::Mesh::with_texture(t.id());
                let (s, c) = rot.to_radians().sin_cos();
                let color = Color32::from_white_alpha((alpha * 255.0) as u8);
                for (l, uv) in [([-0.5f32, -0.5f32], pos2(0.0, 0.0)), ([0.5, -0.5], pos2(1.0, 0.0)), ([0.5, 0.5], pos2(1.0, 1.0)), ([-0.5, 0.5], pos2(0.0, 1.0))] {
                    let p = [at[0] + c * l[0] - s * l[1], at[1] + s * l[0] + c * l[1]];
                    item.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
                }
                item.add_triangle(0, 1, 2);
                item.add_triangle(0, 2, 3);
                painter.add(Shape::mesh(item));
            }
        }
        for [a, b] in edges {
            painter.line_segment([cam.to_screen(canvas, a), cam.to_screen(canvas, b)], Stroke::new(1.0, dark));
        }
        let out: Vec<Pos2> = roof.outline().iter().map(|&p| cam.to_screen(canvas, p)).collect();
        painter.add(Shape::closed_line(out, Stroke::new(roofs::RIM_PX, Color32::from_rgba_premultiplied(14, 10, 8, (alpha * 230.0) as u8))));
    }

    /// Тень крыши на земле: сдвинутый контур, залитый, с мягким краем.
    fn draw_roof_shadow(&self, painter: &egui::Painter, canvas: Rect, roof: &map::Roof, sh: &map::Shadows) {
        let cam = self.cam;
        // От стен, а не от края свеса: без крыши на земле не остаётся резкой полосы.
        let (lo, hi) = ([roof.a[0].min(roof.b[0]), roof.a[1].min(roof.b[1])], [roof.a[0].max(roof.b[0]), roof.a[1].max(roof.b[1])]);
        let pts: Vec<[f32; 2]> = roof.outline().into_iter().map(|p| [p[0].clamp(lo[0], hi[0]), p[1].clamp(lo[1], hi[1])]).collect();
        let n = pts.len();
        let c = [pts.iter().map(|p| p[0]).sum::<f32>() / n as f32, pts.iter().map(|p| p[1]).sum::<f32>() / n as f32];
        let o = roofs::shadow_offset(sh);
        let len = (o[0] * o[0] + o[1] * o[1]).sqrt();
        if len < 1e-4 {
            return;
        }
        let along = [o[0] / len * sh.blur, o[1] / len * sh.blur];
        let shift = |p: [f32; 2], d: [f32; 2]| [p[0] + d[0], p[1] + d[1]];
        let dark = Color32::from_black_alpha((sh.opacity * 255.0).round() as u8);
        let far = Color32::from_black_alpha((sh.opacity * (1.0 - sh.fade) * 255.0).round() as u8);
        let mut mesh = egui::Mesh::default();
        let mut quad = |a: [([f32; 2], Color32); 4]| {
            let base = mesh.vertices.len() as u32;
            for (p, color) in a {
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: egui::epaint::WHITE_UV, color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        };
        // Сторона в тени: тень идёт полосой от края крыши; на границе с освещённой
        // стороной тает вбок.
        let shaded = |k: usize| {
            let (p, q) = (pts[k], pts[(k + 1) % n]);
            let m = [(p[0] + q[0]) / 2.0 - c[0], (p[1] + q[1]) / 2.0 - c[1]];
            let (nx, ny) = (q[1] - p[1], p[0] - q[0]);
            let side = if nx * m[0] + ny * m[1] >= 0.0 { 1.0 } else { -1.0 };
            (nx * o[0] + ny * o[1]) * side > 0.0
        };
        for k in 0..n {
            let (p, q) = (pts[k], pts[(k + 1) % n]);
            if shaded(k) {
                quad([(p, dark), (q, dark), (shift(q, o), far), (shift(p, o), far)]);
                quad([(shift(p, o), far), (shift(q, o), far), (shift(shift(q, o), along), Color32::TRANSPARENT), (shift(shift(p, o), along), Color32::TRANSPARENT)]);
            }
            if shaded(k) != shaded((k + 1) % n) {
                let mut s = [-along[1], along[0]];
                if s[0] * (q[0] - c[0]) + s[1] * (q[1] - c[1]) < 0.0 {
                    s = [-s[0], -s[1]];
                }
                quad([(q, dark), (shift(q, o), far), (shift(shift(q, o), s), Color32::TRANSPARENT), (shift(q, s), Color32::TRANSPARENT)]);
            }
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
            let rows = shadow_rows(&line, closed, outer_hw, inner_hw);
            if rows.len() < 2 {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for row in &rows {
                for (p, solid) in row {
                    let q = [p[0] + o[0], p[1] + o[1]];
                    let color = if *solid { dark } else { Color32::TRANSPARENT };
                    mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, q), uv: egui::epaint::WHITE_UV, color });
                }
            }
            for k in 0..rows.len() as u32 - 1 {
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
        let organic = tex.is_some() && material.vary && !path.along;
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
        if organic {
            self.blend.paint(painter, mesh);
        } else {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Стены лентой: отрезки склеиваются в ломаные, у каждой вершины лента
    /// поворачивает по биссектрисе — без нахлёстов и швов. Обводка — по двум краям
    /// ленты, текстура идёт вдоль без разрывов; на замкнутой стене её шаг подогнан,
    /// чтобы целое число повторов сошлось на стыке.
    /// Порядок рисования объектов: по слою, внутри слоя — как поставлены.
    fn prop_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.map.props.len()).collect();
        order.sort_by_key(|&i| self.map.props[i].layer());
        order
    }

    /// Объекты слоёв из `layers`: сначала все их тени, потом сами.
    fn draw_props(&mut self, painter: &egui::Painter, canvas: Rect, view: Rect, exporting: bool, layers: std::ops::Range<i16>) {
        let cam = self.cam;
        let mut order: Vec<usize> = (0..self.map.props.len()).filter(|&i| layers.contains(&self.map.props[i].layer())).collect();
        if order.is_empty() {
            return;
        }
        order.sort_by_key(|&i| self.map.props[i].layer());
        // Объекты на время рисования вынуты из карты, а не скопированы: рисованию
        // нужен `&mut self` (картинки грузятся), а карту оно не читает.
        let props = std::mem::take(&mut self.map.props);
        let shadows = self.map.shadows.clone();
        if shadows.on {
            for prop in order.iter().map(|&i| &props[i]).filter(|p| p.casts_shadow()) {
                let c = prop.corners();
                let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1]))).expand(shadows.dist + shadows.blur * 2.0);
                if bounds.intersects(view) {
                    for sh in shadows.layers() {
                        self.draw_prop_shadow(painter, canvas, prop, &sh);
                    }
                }
            }
        }
        for i in order {
            let prop = &props[i];
            let c = prop.corners();
            let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1])));
            if !bounds.intersects(view) {
                continue;
            }
            let up = exporting && self.export.as_ref().is_some_and(|e| e.canopies);
            if up && !prop.is_canopy() && (map::bush_by_name(&prop.asset) || map::awning_by_name(&prop.asset) || map::line_by_name(&prop.asset) || map::web_by_name(&prop.asset) || map::chest_variants(&prop.asset).is_some() || map::speaker_by_name(&prop.asset)) {
                // Куст, навес, бельё, паутина, сундук и колонка уйдут в свой тайл целиком.
            } else if exporting && self.export.as_ref().is_some_and(|e| e.canopies) && prop.is_canopy() && map::is_bare_tree(&prop.asset) {
                // Голое дерево целиком уйдёт в тайл; пень под ветками смотрится плохо.
            } else if exporting && self.export.as_ref().is_some_and(|e| e.canopies) && prop.is_canopy() {
                // Крона уйдёт в свой тайл: под ней в картинке пень.
                // Пень размером со ствол, а не со всю крону.
                let asset = self.export.as_ref().map(|e| &e.stumps).filter(|s| !s.is_empty()).map_or_else(
                    || library::STUMP.to_owned(),
                    // Вариант пня от места дерева: тот же на каждой плитке выгрузки.
                    |s| s[(prop.pos[0] * 7.0 + prop.pos[1] * 13.0).abs() as usize % s.len()].clone(),
                );
                let stump = Prop { asset, size: [3.0, 3.0], scale: prop.scale * prop.size[0].max(prop.size[1]) * library::STUMP_SHARE / 3.0, ..prop.clone() };
                self.draw_prop(painter, canvas, &stump, Color32::WHITE);
            } else {
                let tint = prop.tint.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
                self.draw_prop(painter, canvas, prop, tint);
            }
            if self.sel == Some(Sel::Prop(i)) {
                let q: Vec<Pos2> = c.iter().map(|&p| cam.to_screen(canvas, p)).collect();
                painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
            }
        }
        self.map.props = props;
    }

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
        let stumps = if self.settings.gen_library { Palette::from_library(&self.library.assets, Box::new(|_| None)).keys("stump") } else { Vec::new() };
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
            canopies: self.settings.export_canopies,
            update: false,
            stumps,
        });
    }

    /// Шаг выгрузки до отрисовки кадра: поставить камеру на плитку и, когда
    /// картинки догрузились, попросить снимок.
    fn export_step(&mut self, ctx: &egui::Context) {
        if self.export.is_none()
            && let Some((dir, px)) = self.export_args.clone()
            && self.frame >= 10
            && self.pending_plan.is_none()
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
        // Файлы копятся здесь: программа пишет их на диск, модуль отдаёт Foundry.
        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let n = e.done.len();
        let stem = |k: usize| if n == 1 { e.name.clone() } else { format!("{}_{}", e.name, k + 1) };
        let mut jpgs = Vec::new();
        let mut srcs = Vec::new();
        let mut layouts = Vec::new();
        // Контуры палаток по картинке — один раз на картинку.
        let mut hulls: std::collections::HashMap<String, Vec<[f32; 2]>> = std::collections::HashMap::new();
        for (k, img) in e.done.iter().enumerate() {
            // JPEG нужен и для Universal VTT: он лежит внутри .dd2vtt.
            let mut jpg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(img).map_err(|x| x.to_string())?;
            let big = img.width().max(img.height()) > export::WEBP_MAX_SIDE;
            let img_path = e.dir.join(format!("{}.{}", stem(k), if big { "jpg" } else { "webp" }));
            let bytes = if big { jpg.clone() } else { export::webp_bytes(&image::DynamicImage::ImageRgb8(img.clone()).to_rgba8(), 88.0) };
            files.push((img_path.clone(), bytes));
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            let mut lay = export::layout(map, &map.room_edges());
            // Стены палаток — по нарисованному пологу, а не по полям картинки.
            let mut tents = Vec::new();
            for p in map.props.iter().filter(|p| map::awning_by_name(&p.asset)) {
                if !hulls.contains_key(&p.asset) {
                    let hull = self.textures.bytes(&p.asset).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| export::alpha_hull(&i.to_rgba8())).unwrap_or_default();
                    hulls.insert(p.asset.clone(), hull);
                }
                tents.extend(export::tent_walls(p, Some(&hulls[&p.asset])));
            }
            lay.tents = tents;
            layouts.push(lay);
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
            let file = e.dir.join(format!("{}_створка_{}.webp", e.name, leaves.len() + 1));
            files.push((file.clone(), export::webp_bytes(&img.to_rgba8(), 90.0)));
            leaves.insert(key, export::LeafFile { src: export::foundry_src(&file), width });
        }
        // Крыши — картинками по одной, рядом со сценой.
        let mut covers: std::collections::HashMap<String, (image::RgbaImage, [f32; 2])> = std::collections::HashMap::new();
        let mut roof_tiles: Vec<Vec<export::RoofTile>> = Vec::new();
        let mut roof_count = 0;
        let mut canopies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
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
                let fixtures = r.fixtures();
                for pk in r.parapet().into_iter().chain(fixtures.iter().map(|f| f.0)) {
                    if !covers.contains_key(pk) {
                        let bytes = self.textures.bytes(pk)?;
                        let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(pk)))?.to_rgba8();
                        covers.insert(pk.to_owned(), (img, [roofs::PARAPET_TILE, roofs::PARAPET]));
                    }
                }
                let (tex, tile) = &covers[&r.paint];
                let parapet = r.parapet().map(|pk| &covers[pk].0);
                let items: Vec<roofs::Fixture> = fixtures.iter().map(|f| (&covers[f.0].0, f.1, f.2)).collect();
                let (img, origin) = roofs::render(r, tex, *tile, e.px as f32, light, parapet, &items);
                roof_count += 1;
                let file = e.dir.join(format!("{}_крыша_{roof_count}.webp", stem(k)));
                files.push((file.clone(), export::webp_bytes(&img, 88.0)));
                let p = e.px as f32;
                tiles.push(export::RoofTile { src: export::foundry_src(&file), rect: [(origin[0] * p).round() as i64, (origin[1] * p).round() as i64, img.width() as i64, img.height() as i64], rotation: 0.0, shelter: true, fade: true, leaves: false, low: false, wind: 1.0, rope: false, web: None, chest: None, living: None, scuppers: fixtures.iter().filter(|f| f.0.contains("Scupper")).map(|f| [f.1[0] * p, f.1[1] * p, f.2]).collect() });
            }
            // Кроны — один файл на вид дерева, выше токенов, как крыши.
            for prop in map.props.iter().filter(|p| e.canopies && (p.is_canopy() || map::bush_by_name(&p.asset) || map::awning_by_name(&p.asset) || map::line_by_name(&p.asset))) {
                // Подкрашенная крона — свой файл: та же картинка в другом цвете.
                let look = prop.tint.map_or_else(|| prop.asset.clone(), |t| format!("{}#{t:?}", prop.asset));
                if !canopies.contains_key(&look) {
                    let bytes = self.textures.bytes(&prop.asset)?;
                    let mut img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(&prop.asset)))?;
                    if let Some(t) = prop.tint {
                        let mut rgba = img.to_rgba8();
                        for p in rgba.pixels_mut() {
                            for c in 0..3 {
                                p[c] = (p[c] as u16 * t[c] as u16 / 255) as u8;
                            }
                        }
                        img = image::DynamicImage::ImageRgba8(rgba);
                    }
                    let side = (prop.size[0].max(prop.size[1]) * e.px as f32 * 1.5).round() as u32;
                    let img = if img.width().max(img.height()) > side { img.resize(side, side, image::imageops::FilterType::Lanczos3) } else { img };
                    let file = e.dir.join(format!("{}_дерево_{}.webp", e.name, canopies.len() + 1));
                    files.push((file.clone(), export::webp_bytes(&img.to_rgba8(), 88.0)));
                    canopies.insert(look.clone(), export::foundry_src(&file));
                }
                let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                let mut tile = export::canopy_tile(canopies[&look].clone(), prop.pos, size, prop.rot, e.px);
                tile.fade = !map::is_bare_tree(&prop.asset);
                // Навес не качается, но держит дождь; куст и крона — под ветром.
                tile.shelter = !prop.is_canopy() && map::awning_by_name(&prop.asset);
                tile.low = tile.shelter;
                // Бельё колышется, но не гаснет: оно выше голов, и листья с него не летят.
                if map::line_by_name(&prop.asset) {
                    tile.fade = false;
                    tile.leaves = false;
                    tile.rope = true;
                }
                // Куст ниже кроны и жёстче: лишь колышется.
                if map::bush_by_name(&prop.asset) {
                    tile.wind = 0.35;
                }
                tiles.push(tile);
            }
            // Паутина и сундуки — тайлами на полу: паутину покачивает, сундук
            // открывается. Картинка — одна на ассет.
            for prop in map.props.iter().filter(|p| e.canopies && (map::web_by_name(&p.asset) || map::chest_variants(&p.asset).is_some() || map::speaker_by_name(&p.asset))) {
                let mut save = |key: &str, files: &mut Vec<(PathBuf, Vec<u8>)>| -> Result<String, String> {
                    if let Some(src) = canopies.get(key) {
                        return Ok(src.clone());
                    }
                    let bytes = self.textures.bytes(key)?;
                    let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(key)))?;
                    let side = (prop.size[0].max(prop.size[1]) * e.px as f32 * 1.5).round() as u32;
                    let img = if img.width().max(img.height()) > side { img.resize(side, side, image::imageops::FilterType::Lanczos3) } else { img };
                    let file = e.dir.join(format!("{}_предмет_{}.webp", e.name, canopies.len() + 1));
                    files.push((file.clone(), export::webp_bytes(&img.to_rgba8(), 90.0)));
                    let src = export::foundry_src(&file);
                    canopies.insert(key.to_owned(), src.clone());
                    Ok(src)
                };
                let src = save(&prop.asset, &mut files)?;
                let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                let mut tile = export::canopy_tile(src.clone(), prop.pos, size, prop.rot, e.px);
                tile.fade = false;
                tile.leaves = false;
                if let Some(living) = map::living_by_name(&prop.asset) {
                    tile.living = Some(living);
                    tiles.push(tile);
                    continue;
                }
                match map::chest_variants(&prop.asset) {
                    // Нет открытого или пустого в библиотеке — тот же закрытый.
                    Some((open, empty)) => {
                        let open = save(&open, &mut files).unwrap_or_else(|_| src.clone());
                        let empty = save(&empty, &mut files).unwrap_or_else(|_| open.clone());
                        tile.chest = Some((open, empty));
                    }
                    None => tile.web = Some(library::strand_ends(&self.textures.sources, &prop.asset)),
                }
                tiles.push(tile);
            }
            roof_tiles.push(tiles);
        }
        let levels: Vec<export::SceneLevel> = (0..n)
            .map(|k| export::SceneLevel { name: if n == 1 { e.name.clone() } else { self.level_name(k) }, image_src: srcs[k].clone(), layout: &layouts[k], roofs: std::mem::take(&mut roof_tiles[k]) })
            .collect();
        let scene = export::foundry_levels(&self.map, &e.name, &levels, e.px, &leaves);
        let scene_path = e.dir.join(format!("{}.json", e.name));
        files.push((scene_path, serde_json::to_string_pretty(&scene).unwrap_or_default().into_bytes()));
        // Universal VTT — для других VTT; модулю он не нужен, сцена создаётся сразу.
        #[cfg(not(target_arch = "wasm32"))]
        {
            use base64::Engine;
            for k in 0..n {
                let map = if k == self.level { &self.map } else { &self.levels[k] };
                let uv = export::dd2vtt(map, e.px, &base64::engine::general_purpose::STANDARD.encode(&jpgs[k]), &layouts[k]);
                files.push((e.dir.join(format!("{}.dd2vtt", stem(k))), uv.to_string().into_bytes()));
            }
            std::fs::create_dir_all(&e.dir).map_err(|x| x.to_string())?;
            for (path, bytes) in &files {
                std::fs::write(path, bytes).map_err(|x| format!("{}: {x}", path.display()))?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = &jpgs;
            crate::plat::to_scene(&files, e.update)?;
        }
        let walls: usize = layouts.iter().flat_map(|l| l.walls.iter()).map(|l| l.len().saturating_sub(1)).sum();
        let (w, h) = (e.done[0].width(), e.done[0].height());
        Ok(format!(
            "{} {}: {}картинка {w}×{h}, стен {walls}, крыш {roof_count}, проёмов {}, огней {}",
            if cfg!(target_arch = "wasm32") { "На сцену —" } else { "Выгружено в" },
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
        // В модуле выбранное в окне браузера приходит не сразу, а кадром позже.
        #[cfg(target_arch = "wasm32")]
        for f in crate::plat::take_picked() {
            if f.extension().is_some_and(|e| e == "dungeondraft_pack") {
                self.add_pack(f);
            }
        }
        self.frame_ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Сюда доходят, только закрыв окно по-хорошему: карта сохранена или от правок
        // отказались. Копия на случай сбоя больше не нужна.
        self.recovery.close();
        if self.update.restart {
            crate::update::relaunch();
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(test) = &mut self.test {
            test.feed(raw);
        }
    }
}

/// Строки сида в окне генератора: «Сид этой карты» — из чего построена открытая
/// (только если её строили), «Сид» — для следующей.
fn seed_rows(ui: &mut egui::Ui, seed: &mut String, this_map: &str) {
    if !this_map.is_empty() {
        ui.label("Сид этой карты");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut this_map.to_owned()).desired_width(110.0).interactive(false));
            if ui.button("Взять").on_hover_text("Построить следующую карту с этим же сидом").clicked() {
                *seed = this_map.to_owned();
            }
        });
        ui.end_row();
    }
    ui.label("Сид");
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(seed).desired_width(110.0));
        if ui.button("🎲").on_hover_text("Новый сид").clicked() {
            *seed = generate::new_seed();
        }
    });
    ui.end_row();
}

/// Оформление: графит и янтарь.
fn apply_theme(ctx: &egui::Context) {
    use egui::CornerRadius;
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        let v = &mut s.visuals;
        v.panel_fill = PANEL;
        v.window_fill = PANEL;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.extreme_bg_color = Color32::from_rgb(22, 23, 25);
        v.faint_bg_color = FIELD;
        v.override_text_color = None;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, ON_ACCENT);
        v.hyperlink_color = ACCENT;
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(6);
        for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(4);
        }
        v.widgets.noninteractive.bg_fill = PANEL;
        v.widgets.noninteractive.weak_bg_fill = PANEL;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.inactive.bg_fill = FIELD;
        v.widgets.inactive.weak_bg_fill = FIELD;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        v.widgets.hovered.bg_fill = Color32::from_rgb(62, 64, 70);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(62, 64, 70);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.active.weak_bg_fill = ACCENT;
        // Этим же цветом egui пишет `ui.strong`, поэтому он светлый.
        v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        s.spacing.button_padding = vec2(8.0, 3.0);
        s.spacing.item_spacing = vec2(6.0, 5.0);
    });
}

/// Значок ячейки инструмента, чертится линиями.
fn tool_icon(p: &egui::Painter, r: Rect, group: Group, ink: Color32) {
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let line = Stroke::new(1.8, ink);
    let poly = |pts: &[(f32, f32)]| p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), line));
    match group {
        Group::Select => {
            let pts = [(0.25, 0.08), (0.25, 0.82), (0.44, 0.64), (0.58, 0.94), (0.70, 0.88), (0.56, 0.59), (0.82, 0.57), (0.25, 0.08)];
            poly(&pts);
        }
        Group::Brush => {
            poly(&[(0.92, 0.08), (0.45, 0.55)]);
            poly(&[(0.38, 0.50), (0.50, 0.62), (0.40, 0.86), (0.10, 0.92), (0.22, 0.62), (0.38, 0.50)]);
        }
        Group::Wall => {
            for y in [0.15, 0.40, 0.65, 0.90] {
                poly(&[(0.05, y), (0.95, y)]);
            }
            poly(&[(0.05, 0.15), (0.05, 0.90)]);
            poly(&[(0.95, 0.15), (0.95, 0.90)]);
            poly(&[(0.5, 0.15), (0.5, 0.40)]);
            poly(&[(0.28, 0.40), (0.28, 0.65)]);
            poly(&[(0.72, 0.40), (0.72, 0.65)]);
            poly(&[(0.5, 0.65), (0.5, 0.90)]);
        }
        Group::Roof => {
            poly(&[(0.02, 0.55), (0.5, 0.12), (0.98, 0.55)]);
            poly(&[(0.18, 0.42), (0.18, 0.92), (0.82, 0.92), (0.82, 0.42)]);
        }
        Group::Object => {
            poly(&[(0.5, 0.06), (0.92, 0.28), (0.92, 0.74), (0.5, 0.96), (0.08, 0.74), (0.08, 0.28), (0.5, 0.06)]);
            poly(&[(0.08, 0.28), (0.5, 0.5), (0.92, 0.28)]);
            poly(&[(0.5, 0.5), (0.5, 0.96)]);
        }
        Group::Path => {
            let pts: Vec<(f32, f32)> = (0..=24).map(|k| {
                let t = k as f32 / 24.0;
                (0.12 + 0.76 * t, 0.5 + 0.34 * (t * std::f32::consts::TAU).sin())
            }).collect();
            poly(&pts);
        }
        Group::Text => {
            p.text(r.center(), egui::Align2::CENTER_CENTER, "Т", egui::FontId::proportional(25.0), ink);
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
    ribbon_ext(line, closed, hw, hw)
}

/// Ряды тени стены: по четыре вершины поперёк, `true` — сплошная. Края тают и
/// вдоль, и на концах незамкнутой линии: там лишний прозрачный ряд.
fn shadow_rows(line: &[[f32; 2]], closed: bool, outer_hw: f32, inner_hw: f32) -> Vec<[([f32; 2], bool); 4]> {
    // Продление одно на обе ленты, иначе ряды идут наискось и на концах торчат клинья.
    let (outer, inner) = (ribbon_ext(line, closed, outer_hw, inner_hw), ribbon_ext(line, closed, inner_hw, inner_hw));
    if outer.len() < 2 || outer.len() != inner.len() {
        return Vec::new();
    }
    let mut rows: Vec<[([f32; 2], bool); 4]> = outer.iter().zip(&inner).map(|(o, i)| [(o.0, false), (i.0, true), (i.1, true), (o.1, false)]).collect();
    if !closed {
        let fade = outer_hw - inner_hw;
        let mid = |r: &[([f32; 2], bool); 4]| [(r[0].0[0] + r[3].0[0]) / 2.0, (r[0].0[1] + r[3].0[1]) / 2.0];
        let cap = |end: &[([f32; 2], bool); 4], next: &[([f32; 2], bool); 4]| {
            let (e, n) = (mid(end), mid(next));
            let (dx, dy) = (e[0] - n[0], e[1] - n[1]);
            let l = (dx * dx + dy * dy).sqrt().max(1e-6);
            end.map(|(p, _)| ([p[0] + dx / l * fade, p[1] + dy / l * fade], false))
        };
        let (first, last) = (cap(&rows[0], &rows[1]), cap(&rows[rows.len() - 1], &rows[rows.len() - 2]));
        rows.insert(0, first);
        rows.push(last);
    }
    rows
}

/// Лента с полушириной `hw`; концы незамкнутой линии продлены на `ext`.
fn ribbon_ext(line: &[[f32; 2]], closed: bool, hw: f32, ext: f32) -> Vec<([f32; 2], [f32; 2], f32)> {
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
        pts[0] = [pts[0][0] + d0[0] * ext, pts[0][1] + d0[1] * ext];
        let d1 = dir(pts[n - 2], pts[n - 1]);
        pts[n - 1] = [pts[n - 1][0] + d1[0] * ext, pts[n - 1][1] + d1[1] * ext];
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
    fn single_wall_shadow_fades_at_both_ends() {
        let rows = shadow_rows(&[[0.0, 0.0], [0.0, 3.0]], false, 0.3, 0.1);
        assert_eq!(rows.len(), 4);
        // Концы прозрачны целиком и лежат прямо поперёк стены.
        for row in [&rows[0], &rows[3]] {
            assert!(row.iter().all(|(_, solid)| !solid));
            assert!(row.iter().all(|(p, _)| (p[1] - row[0].0[1]).abs() < 1e-5));
        }
        assert!((rows[0][0].0[1] + 0.3).abs() < 1e-5 && (rows[3][0].0[1] - 3.3).abs() < 1e-5);
        // Внутри ряды тоже поперёк: клиньев нет.
        for row in [&rows[1], &rows[2]] {
            assert!(row.iter().all(|(p, _)| (p[1] - row[0].0[1]).abs() < 1e-5));
            assert!(row[1].1 && row[2].1 && !row[0].1 && !row[3].1);
        }
    }

    #[test]
    fn baked_prop_lands_rotated_where_the_map_shows_it() {
        // Картинка 2×1 клетки: левая половина красная, правая синяя.
        let img = image::RgbaImage::from_fn(64, 32, |x, _| if x < 32 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
        let p = Prop { asset: String::new(), pos: [1.0, 1.0], size: [2.0, 1.0], rot: 90.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
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
