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
    Relief,
    Text,
    Roof,
}

/// Кисть «Рельеф»: что делает мазок.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReliefMode {
    Raise,
    Lower,
    Flatten,
    Smooth,
}

impl ReliefMode {
    const ALL: [ReliefMode; 4] = [ReliefMode::Raise, ReliefMode::Lower, ReliefMode::Flatten, ReliefMode::Smooth];

    fn label(self) -> &'static str {
        match self {
            ReliefMode::Raise => "Поднять",
            ReliefMode::Lower => "Опустить",
            ReliefMode::Flatten => "Выровнять",
            ReliefMode::Smooth => "Сгладить",
        }
    }
}

impl Tool {
    const ALL: [Tool; 13] = [Tool::Select, Tool::Dungeon, Tool::Floor, Tool::Path, Tool::Shade, Tool::Relief, Tool::Erase, Tool::Wall, Tool::Door, Tool::Object, Tool::Scatter, Tool::Roof, Tool::Text];

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
            Tool::Relief => "Рельеф",
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
            Tool::Relief => Key::M,
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
            Group::Brush => matches!(tool, Tool::Dungeon | Tool::Floor | Tool::Shade | Tool::Relief | Tool::Erase),
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
    /// Крыша по контуру из OSM и её кусок: 0 — весь дом, дальше части.
    Outline(usize, usize),
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
    /// `--osm файл.osm`: местность из выгрузки OpenStreetMap при запуске.
    pub osm: Option<PathBuf>,
    /// `--place "широта,долгота"` (или ссылка на карту): местность по координатам
    /// при запуске, размером `--cells ШxВ` (по умолчанию 40×30).
    pub place: Option<String>,
    pub cells: Option<[u32; 2]>,
    /// `--trees 0..1`: досадить рощи на свободную сушу.
    pub trees: Option<f32>,
}

/// Окно «местность по координатам»: что вписано и размер в клетках.
struct PlaceForm {
    text: String,
    cells: [u32; 2],
    /// Досадить рощи, 0..1 (`osm::Options::trees`).
    trees: f32,
}

/// Сборка места в фоне: ход и готовое.
#[derive(Default)]
struct PlaceJob {
    rx: Option<std::sync::mpsc::Receiver<crate::place::Msg>>,
    step: String,
    error: Option<String>,
    ready: Option<crate::place::Place>,
    /// Из `--place` при запуске — ждёт первого кадра.
    start: Option<(f64, f64, [u32; 2])>,
    /// Досадить рощи при импорте собранного места.
    trees: f32,
    /// В модуле: место, для которого фоном качается нужное (`plat::net_prefetch`).
    #[cfg(target_arch = "wasm32")]
    fetch: Option<(f64, f64, [u32; 2])>,
    /// В модуле: всё скачано — собрать следующим кадром, одним куском.
    #[cfg(target_arch = "wasm32")]
    analyse: Option<(f64, f64, [u32; 2])>,
}

impl PlaceJob {
    fn busy(&self) -> bool {
        #[cfg(target_arch = "wasm32")]
        if self.fetch.is_some() || self.analyse.is_some() {
            return true;
        }
        self.rx.is_some() || self.ready.is_some() || self.start.is_some()
    }
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
    /// Размер плитки в пикселях экрана: по холсту на первом шаге, когда окно развёрнуто.
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
    /// Только картинка области (MCP `view`, `render_picture`): предметы и двери в ней,
    /// файлов сцены нет, этаж — текущий. Итог — в [`App::picture`].
    picture: bool,
    /// Левый верхний угол снимаемой области, клетки.
    origin: [f32; 2],
    /// Крыши в картинке: 1 — целые, как их видят игроки; 0 — без крыш.
    roof_alpha: f32,
    /// Кадров до раскладки плиток: развёрнутому окну дать принять размер.
    wait: u32,
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
    /// Выведенное из рельефа для отрисовки — по отпечатку высот и солнца.
    relief_look: Option<(u64, std::rc::Rc<crate::relief::Look>)>,
    /// Тени крон на земле ([`roofs::tree_shade`]) — по отпечатку деревьев, крыш и солнца.
    tree_shade: Option<(u64, Option<std::rc::Rc<Mask>>)>,
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
    /// Кисть «Рельеф»: что делает, на сколько клеток поднимает; Alt — наоборот.
    pub relief_mode: ReliefMode,
    pub relief_height: f32,
    relief_alt: bool,
    /// Высоты до мазка и высота, к которой ровняет «Выровнять».
    relief_base: Option<crate::relief::Relief>,
    relief_target: f32,
    /// Образец новой подписи; тащат подпись — откуда взяли.
    pub label_style: map::Label,
    label_grab: Option<(usize, [f32; 2])>,
    /// Образец новой крыши, вид крыш, протяжка новой и перенос стоящей.
    pub roof_style: map::Roof,
    pub roof_view: RoofView,
    /// Крыши по контуру, уже нарисованные точками: по отпечатку крыши и света —
    /// картинка, левый верхний угол и размер в клетках.
    outline_cache: std::collections::HashMap<u64, (egui::TextureHandle, [f32; 2], [f32; 2])>,
    /// Покрытия крыш по контуру точками (в цвете ключа).
    cover_cache: std::collections::HashMap<String, std::sync::Arc<image::RgbaImage>>,
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
    /// Выгрузка OpenStreetMap из командной строки — тоже.
    pending_osm: Option<PathBuf>,
    place_form: Option<PlaceForm>,
    /// Управление из MCP ([`crate::control`]): приём команд и ответ, что ждёт кадров.
    #[cfg(not(target_arch = "wasm32"))]
    control: Option<crate::control::Control>,
    #[cfg(not(target_arch = "wasm32"))]
    control_wait: Option<tools::Waiting>,
    /// Идёт `batch` из MCP: этажи, у которых точка отмены уже есть, — вся пачка
    /// отменяется одним шагом на этаж.
    batch: Option<Vec<usize>>,
    /// Окно «Место на карте» у формы места.
    map_pick: Option<crate::mappick::MapPick>,
    place: PlaceJob,
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
    window_bands: std::collections::HashMap<String, [f32; 2]>,
    /// Код окна в Windows: развернуть свёрнутое на время снимка.
    #[cfg(not(target_arch = "wasm32"))]
    window_code: isize,
    /// Окно свернули, а ИИ просил снимок: развёрнуто тогда-то (секунды `egui`),
    /// свернуть обратно, когда работа кончится.
    #[cfg(not(target_arch = "wasm32"))]
    shown_for_ai: Option<f64>,
    /// Видимые рамки лестниц и трапов по ассету: размыкают перила у крыши.
    visible_boxes: std::collections::HashMap<String, Option<[f32; 4]>>,
    /// Стекло к подоконнику FA (ключ подоконника и большое ли окно → окно).
    window_glass: std::collections::HashMap<(String, bool), Option<String>>,
    /// Библиотека показывает только то, что уже стоит на карте.
    used_only: bool,
    /// Время года последнего леса: окно генератора открывается с ним.
    gen_season: crate::plan::Season,
    gen_hills: crate::plan::Hills,
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
    /// Модуль: карты в данных Foundry для окна «Открыть» (окна файлов у браузера нет).
    open_list: Option<Vec<PathBuf>>,
    pub generator: Option<Options>,
    new_map: Option<NewMap>,
    export: Option<Export>,
    /// Готовая картинка карты с предметами ([`App::start_picture`]) — её забирает MCP.
    picture: Option<image::RgbImage>,
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
    return PathBuf::from(crate::plat::DATA_ROOT);
    #[cfg(not(target_arch = "wasm32"))]
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kartograf")
}

/// Пикселей на клетку в выгрузке по умолчанию: столько у Foundry по умолчанию
/// не бывает (там 100), но карта с мелкими предметами при 100 мылится.
const EXPORT_PX: u32 = 256;
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
        let settings: Settings = crate::plat::fs::read(&data_dir.join("settings.json"))
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
            relief_look: None,
            tree_shade: None,
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
            relief_mode: ReliefMode::Raise,
            relief_height: 2.0,
            relief_alt: false,
            relief_base: None,
            relief_target: 0.0,
            label_style: map::Label::new("Комната".into(), [0.0, 0.0]),
            label_grab: None,
            roof_style: map::Roof::new([0.0, 0.0], [1.0, 1.0], map::RoofShape::Gable, library::DEFAULT_ROOF.into()),
            roof_view: match settings.roof_view {
                1 => RoofView::Ghost,
                2 => RoofView::Hidden,
                _ => RoofView::Auto,
            },
            outline_cache: std::collections::HashMap::new(),
            cover_cache: std::collections::HashMap::new(),
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
            #[cfg(not(target_arch = "wasm32"))]
            window_code: 0,
            #[cfg(not(target_arch = "wasm32"))]
            shown_for_ai: None,
            visible_boxes: std::collections::HashMap::new(),
            window_glass: std::collections::HashMap::new(),
            used_only: false,
            gen_season: crate::plan::Season::Summer,
            gen_hills: crate::plan::Hills::Auto,
            gen_cave: crate::plan::CaveKind::Stone,
            gen_district: crate::plan::District::Mixed,
            gen_dungeon: crate::plan::DungeonKind::Crypt,
            first_thumb: Rect::NOTHING,
            pending_plan: None,
            pending_osm: None,
            place_form: None,
            #[cfg(not(target_arch = "wasm32"))]
            control: None,
            #[cfg(not(target_arch = "wasm32"))]
            control_wait: None,
            batch: None,
            map_pick: None,
            place: PlaceJob::default(),
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
            open_list: None,
            generator: None,
            new_map: None,
            export: None,
            picture: None,
            export_dialog: None,
            scene_name: None,
            export_args: args.export.map(|d| (d, args.px.unwrap_or(EXPORT_PX))),
            notice: None,
            confirm_close: false,
            allow_close: false,
        };
        if let Some(file) = args.file {
            app.open_path(&file);
        } else if app.test.is_none() && app.shot.is_none() && app.export_args.is_none() && args.plan.is_none() && args.osm.is_none() && args.place.is_none() && app.orphans.is_empty() {
            // Запуск без карты — сразу спросить размер новой.
            app.new_map = Some(NewMap { width: app.map.width, height: app.map.height });
        }
        // План строится, когда прочитаны паки: иначе план «из библиотеки» взял бы встроенное.
        app.pending_plan = args.plan;
        app.pending_osm = args.osm;
        app.place.start = args.place.as_deref().and_then(crate::place::parse_place).map(|(lat, lon)| (lat, lon, args.cells.unwrap_or([40, 30])));
        app.place.trees = args.trees.unwrap_or(0.0).clamp(0.0, 1.0);
        // Окно для человека (не самопроверка, не снимок, не выгрузка) принимает команды MCP.
        #[cfg(not(target_arch = "wasm32"))]
        if app.test.is_none() && app.shot.is_none() && app.export_args.is_none() {
            app.control = crate::control::start(cc.egui_ctx.clone(), &app.data_dir);
        }
        app
    }

    // ----- правки и история -----
}

// Остальное у `App` — по темам, в `src/app/`.
mod history;
mod brush;
mod edit;
mod build;
mod files;
mod frame;
mod panels;
mod shelf;
mod input;
mod draw;
mod draw_ground;
mod draw_scene;
mod exporting;

/// Команды ИИ (MCP) — отдельным файлом: `app.rs` и так велик.
#[cfg(not(target_arch = "wasm32"))]
mod tools;

impl eframe::App for App {
    /// Свёрнутое окно eframe не рисует и `ui` не зовёт — только это: команды ИИ
    /// идут и отсюда, а снимок и выгрузка разворачивают окно на время.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.window_code == 0 {
                self.window_code = crate::plat::window_code(_frame);
            }
            self.control_while_minimized(ctx);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // В модуле выбранное в окне браузера приходит не сразу, а кадром позже.
        #[cfg(target_arch = "wasm32")]
        for f in crate::plat::take_picked() {
            if let Some(dir) = f.to_str().and_then(|s| s.strip_prefix("dir:")) {
                self.set_textures_dir(PathBuf::from(dir));
            } else if f.extension().is_some_and(|e| e == "dungeondraft_pack") {
                self.add_pack(f);
            }
        }
        self.frame_ui(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Сюда доходят, только закрыв окно по-хорошему: карта сохранена или от правок
        // отказались. Копия на случай сбоя больше не нужна.
        self.recovery.close();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(c) = &self.control {
            c.close();
        }
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

/// Отношения OSM целиком (`relation/{id}/full`): неполные в выгрузке природные
/// контуры. Что не скачалось (нет сети), пропускается — импорт предупредит.
#[cfg(not(target_arch = "wasm32"))]
fn osm_relations(ids: &[i64]) -> Vec<String> {
    use std::time::Duration;
    if ids.is_empty() {
        return Vec::new();
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(8))).timeout_global(Some(Duration::from_secs(30))).build().into();
    let ua = format!("Kartograf/{} (OSM import)", env!("CARGO_PKG_VERSION"));
    let mut out = Vec::new();
    for id in ids {
        let url = format!("https://api.openstreetmap.org/api/0.6/relation/{id}/full");
        match agent.get(&url).header("User-Agent", &ua).call().and_then(|mut r| r.body_mut().read_to_string()) {
            Ok(text) => out.push(text),
            // Первый отказ — скорее всего, сети нет: остальные не ждём.
            Err(_) if out.is_empty() => break,
            Err(_) => {}
        }
    }
    out
}

#[cfg(target_arch = "wasm32")]
fn osm_relations(_ids: &[i64]) -> Vec<String> {
    Vec::new()
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
