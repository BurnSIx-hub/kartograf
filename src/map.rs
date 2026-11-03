//! Карта как данные. Всё в единицах клетки, без пикселей: масштаб — забота холста,
//! а файл не меняется от того, как близко его рассматривали.
//!
//! Пол лежит масками (см. `mask.rs`), а не целыми клетками, — так работают круглые
//! и мягкие кисти:
//! - `terrain` — местность: слои материалов без стен (трава, камень двора);
//! - `rooms` — где подземелье: по краю этой маски стены встают сами, как у кисти
//!   подземелья в Dungeondraft; `room_floor` — чем выложен пол комнат. Стены комнат
//!   не хранятся, а выводятся из маски, поэтому стереть кусок комнаты — значит
//!   и стену подвинуть.
//!
//! Первый формат файла хранил материал целыми клетками; такие карты открываются
//! и переводятся в маски.

use std::collections::BTreeMap;
use crate::mask::{self, Mask};
use serde::{Deserialize, Serialize};

pub const EXT: &str = "karta";
pub const FORMAT: u32 = 2;

/// Чем закрашено: цветом или картинкой с диска.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    Color([u8; 3]),
    /// Путь к картинке. Картинка повторяется плиткой.
    Texture(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    pub paint: Paint,
    /// Плитка перемешивается против заметного повтора (`library::vary`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vary: bool,
    /// Оживает ли в Foundry (модуль «Живая карта»): вода, лава, болото.
    #[serde(default, skip_serializing_if = "Live::is_auto")]
    pub live: Live,
}

/// Живой материал: в выгрузке всё, что им залито, становится областью сцены
/// с пометкой эффекта. `Auto` — по имени картинки.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Live {
    #[default]
    Auto,
    Off,
    Water,
    Lava,
    Swamp,
}

impl Live {
    pub const CHOICES: [Live; 5] = [Live::Auto, Live::Off, Live::Water, Live::Lava, Live::Swamp];

    pub fn is_auto(&self) -> bool {
        *self == Live::Auto
    }

    pub fn label(self) -> &'static str {
        match self {
            Live::Auto => "по имени",
            Live::Off => "нет",
            Live::Water => "вода",
            Live::Lava => "лава",
            Live::Swamp => "болото",
        }
    }

    /// Имя эффекта для модуля Foundry.
    pub fn effect(self) -> Option<&'static str> {
        match self {
            Live::Water => Some("water"),
            Live::Lava => Some("lava"),
            Live::Swamp => Some("swamp"),
            Live::Auto | Live::Off => None,
        }
    }
}

impl Material {
    /// Чем оживает: выбранное вручную или узнанное по имени картинки.
    pub fn live(&self) -> Live {
        if self.live != Live::Auto {
            return self.live;
        }
        let key = match &self.paint {
            Paint::Texture(k) => crate::library::label(k).to_lowercase() + " " + &self.name.to_lowercase(),
            Paint::Color(_) => return Live::Off,
        };
        let has = |ws: &[&str]| ws.iter().any(|w| key.contains(w));
        if has(&["lava", "magma", "лава", "магма"]) {
            Live::Lava
        } else if has(&["swamp", "bog", "marsh", "болот", "топь"]) {
            Live::Swamp
        } else if has(&["water", "вода", "мелковод", "глубина", "river", "lake", "pond", "sea", "ocean"]) {
            Live::Water
        } else {
            Live::Off
        }
    }
}

/// Вид стен: и нарисованных вручную, и тех, что встают вокруг комнат.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WallStyle {
    pub paint: Paint,
    /// Толщина в клетках.
    pub width: f32,
}

impl Default for WallStyle {
    fn default() -> Self {
        Self { paint: Paint::Color([74, 64, 56]), width: 0.25 }
    }
}

/// Палитра новой карты — встроенные текстуры. Первые четыре стоят на местах
/// прежних цветов (камень, дерево, земля, трава): карты формата 1 без своей
/// палитры открываются с ними.
pub fn builtin_materials() -> Vec<Material> {
    crate::library::BUILTIN_TEXTURES
        .iter()
        .map(|t| Material { name: t.1.into(), paint: Paint::Texture(format!("{}{}", crate::library::BUILTIN, t.0)), vary: t.2, live: Live::Auto })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
    Floor,
    Rooms,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wall {
    /// Вершины на узлах сетки.
    pub points: Vec<[f32; 2]>,
    /// Замкнутая стена соединяет последнюю вершину с первой.
    pub closed: bool,
    /// Прогиб каждого отрезка в клетках: насколько середина дуги отошла от прямой,
    /// плюс — влево по ходу стены. Пусто или 0 — прямой отрезок.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bends: Vec<f32>,
    /// Свой вид; без него — как у всей карты.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<WallStyle>,
}

pub type Segment = ([f32; 2], [f32; 2]);

impl Wall {
    /// Отрезки между вершинами, без изгиба.
    pub fn chords(&self) -> impl Iterator<Item = Segment> + '_ {
        let n = self.points.len();
        let count = if self.closed && n > 2 { n } else { n.saturating_sub(1) };
        (0..count).map(move |i| (self.points[i], self.points[(i + 1) % n]))
    }

    pub fn bend(&self, i: usize) -> f32 {
        self.bends.get(i).copied().unwrap_or(0.0)
    }

    pub fn set_bend(&mut self, i: usize, h: f32) {
        if self.bends.len() <= i {
            self.bends.resize(i + 1, 0.0);
        }
        self.bends[i] = h;
        while self.bends.last() == Some(&0.0) {
            self.bends.pop();
        }
    }

    /// Отрезки стены; изогнутые — мелкими кусками дуги.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        self.chords().enumerate().flat_map(move |(i, (a, b))| {
            let pts = arc_points(a, b, self.bend(i));
            (0..pts.len() - 1).map(move |k| (pts[k], pts[k + 1])).collect::<Vec<_>>()
        })
    }

    /// Расстояние от точки до ближайшего отрезка стены.
    pub fn distance(&self, p: [f32; 2]) -> f32 {
        self.segments().map(|(a, b)| segment_distance(p, a, b)).fold(f32::INFINITY, f32::min)
    }
}

/// Окружность дуги через `a` и `b` с прогибом `h` (плюс — влево от `a`→`b`):
/// центр, радиус и левая нормаль хорды. `None` — дуги нет, отрезок прямой.
fn arc_circle(a: [f32; 2], b: [f32; 2], h: f32) -> Option<([f32; 2], f32, [f32; 2])> {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 || h.abs() < 1e-4 {
        return None;
    }
    let n = [dy / len, -dx / len];
    let c = len / 2.0;
    let r = (c * c + h * h) / (2.0 * h.abs());
    let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let k = -h.signum() * (r - h.abs());
    Some(([mid[0] + n[0] * k, mid[1] + n[1] * k], r, n))
}

/// Наибольший прогиб — полукруг: дальше дуга загибается назад за концы.
pub fn max_bend(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt() / 2.0
}

/// Точки дуги от `a` до `b`; прямой отрезок — две точки.
pub fn arc_points(a: [f32; 2], b: [f32; 2], h: f32) -> Vec<[f32; 2]> {
    let Some((c, r, _)) = arc_circle(a, b, h) else { return vec![a, b] };
    let a0 = (a[1] - c[1]).atan2(a[0] - c[0]);
    let a1 = (b[1] - c[1]).atan2(b[0] - c[0]);
    // Идём через вершину дуги: она слева от хорды при h > 0.
    let mut sweep = a1 - a0;
    let apex_side = h.signum();
    let tau = std::f32::consts::TAU;
    sweep = sweep.rem_euclid(tau);
    // Проверим середину: не та сторона — берём дугу в другую сторону.
    let mid_ang = a0 + sweep / 2.0;
    let m = [c[0] + r * mid_ang.cos(), c[1] + r * mid_ang.sin()];
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let side = (m[0] - a[0]) * dy - (m[1] - a[1]) * dx;
    if side.signum() != apex_side {
        sweep -= tau;
    }
    let n = ((sweep.abs() * r / 0.12).ceil() as usize).clamp(4, 96);
    (0..=n)
        .map(|k| {
            if k == 0 {
                return a;
            }
            if k == n {
                return b;
            }
            let t = a0 + sweep * k as f32 / n as f32;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

pub fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortalKind {
    /// Пустой проём: стены нет, двери тоже.
    Opening,
    Door,
    /// Две створки на петлях по краям проёма.
    DoubleDoor,
    Window,
    /// Тайный ход: на картинке стена цельная, в редакторе — пунктирная пометка,
    /// в Foundry — потайная дверь.
    Secret,
}

impl PortalKind {
    pub const ALL: [PortalKind; 5] = [PortalKind::Opening, PortalKind::Door, PortalKind::DoubleDoor, PortalKind::Window, PortalKind::Secret];

    pub fn label(self) -> &'static str {
        match self {
            PortalKind::Opening => "Проём",
            PortalKind::Door => "Дверь",
            PortalKind::DoubleDoor => "Двойная",
            PortalKind::Window => "Окно",
            PortalKind::Secret => "Тайный ход",
        }
    }

    /// Не рвёт стену на картинке: тайный ход снаружи неотличим от стены.
    pub fn hidden(self) -> bool {
        self == PortalKind::Secret
    }

    /// Есть ли створка, которая открывается.
    pub fn is_door(self) -> bool {
        matches!(self, PortalKind::Door | PortalKind::DoubleDoor)
    }
}

/// Дверь или окно: кусок стены от `a` до `b`, где стена прервана.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Portal {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub kind: PortalKind,
    /// Картинка створки или подоконника; без неё — простая плашка.
    #[serde(default)]
    pub asset: Option<String>,
    /// Косяк: картинка в свой размер, посередине проёма.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

impl Portal {
    pub fn center(&self) -> [f32; 2] {
        [(self.a[0] + self.b[0]) / 2.0, (self.a[1] + self.b[1]) / 2.0]
    }

    /// Окно из паков: подоконник FA и рама поверх, в толщину стены. Встроенное
    /// окно со стеклом — в свою толщину.
    pub fn own_window(&self) -> bool {
        self.kind == PortalKind::Window && self.asset.as_deref().is_some_and(|k| !k.starts_with(crate::library::BUILTIN))
    }
}

/// Объект на карте: картинка с центром, размером в клетках и поворотом.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prop {
    pub asset: String,
    pub pos: [f32; 2],
    /// Размер в клетках без учёта `scale`.
    pub size: [f32; 2],
    /// Поворот в градусах по часовой стрелке.
    pub rot: f32,
    pub scale: f32,
    /// Отбрасывает ли тень; `None` — по имени: ковры, лужи и готовые тени плоские.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
    /// Чем светит в Foundry; `None` — по имени: костры, камины, жаровни, фонари, свечи.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<Glow>,
    /// Крона над токенами в Foundry (в картинке остаётся пень); `None` — по имени: деревья.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canopy: Option<bool>,
    /// Слой высоты, как в Dungeondraft: кто кого накрывает. `None` — по имени.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<i16>,
}

/// Слои высоты объектов — числа те же, что в Dungeondraft. Внутри слоя выше тот,
/// кто поставлен позже. Стены рисуются между 400 и 700, крыши — между 700 и 900.
pub const LAYERS: [(i16, &str); 6] = [
    (100, "Пол: ковры, лужи"),
    (200, "Низкие: мелочь на полу"),
    (300, "Обычные"),
    (400, "Высокие: деревья, шкафы"),
    (700, "Над стенами"),
    (900, "Над крышами"),
];
pub const LAYER_DEFAULT: i16 = 300;
/// С этого слоя объект рисуется поверх стен, с `ABOVE_ROOFS` — поверх крыш.
pub const ABOVE_WALLS: i16 = 700;
pub const ABOVE_ROOFS: i16 = 900;

/// Слой по имени: плоское — на пол, деревья — над остальным.
pub fn layer_by_name(key: &str) -> i16 {
    let k = crate::library::label(key).to_lowercase();
    if flat_by_name(key) {
        100
    } else if key == crate::library::TREE || ["tree", "дерев"].iter().any(|w| k.contains(w)) {
        400
    } else {
        LAYER_DEFAULT
    }
}

pub fn layer_label(layer: i16) -> String {
    LAYERS.iter().find(|l| l.0 == layer).map_or_else(|| layer.to_string(), |l| l.1.to_owned())
}

/// Огонь или лампа: как светит в Foundry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Glow {
    /// Не светит, хотя по имени похоже.
    Off,
    Campfire,
    Fireplace,
    Brazier,
    Candle,
    Lamp,
}

impl Glow {
    pub const CHOICES: [Glow; 6] = [Glow::Off, Glow::Campfire, Glow::Fireplace, Glow::Brazier, Glow::Candle, Glow::Lamp];

    pub fn label(self) -> &'static str {
        match self {
            Glow::Off => "нет",
            Glow::Campfire => "костёр",
            Glow::Fireplace => "камин",
            Glow::Brazier => "жаровня",
            Glow::Candle => "свеча",
            Glow::Lamp => "фонарь",
        }
    }

    /// Радиус яркого и тусклого в клетках, цвет и анимация Foundry. `None` — не светит.
    pub fn params(self) -> Option<(f32, f32, &'static str, &'static str)> {
        match self {
            Glow::Off => None,
            Glow::Campfire => Some((3.0, 6.0, "#ff9a3c", "flame")),
            Glow::Fireplace => Some((2.0, 5.0, "#ff8a2c", "flame")),
            Glow::Brazier => Some((2.0, 4.0, "#ffa040", "torch")),
            Glow::Candle => Some((1.0, 2.0, "#ffd9a0", "torch")),
            Glow::Lamp => Some((3.0, 6.0, "#ffc870", "torch")),
        }
    }
}

/// Слова имени ассета в нижнем регистре: `Oak_Tree_A_3x3` → oak, tree, a, 3x3.
/// У встроенных — слова ключа (`street_lamp` → street, lamp), у чужих — имени файла.
pub fn name_words(key: &str) -> Vec<String> {
    let name = key.strip_prefix(crate::library::BUILTIN).map_or_else(|| crate::library::label(key), str::to_owned).to_lowercase();
    name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// Чем светит по имени. Встроенные — по ключу, чужие — по словам в названии файла.
pub fn glow_by_name(key: &str) -> Option<Glow> {
    let words = name_words(key);
    let has = |ws: &[&str]| ws.iter().any(|w| words.iter().any(|x| x == w));
    let pair = |a: &str, b: &str| words.iter().any(|x| x == a) && words.iter().any(|x| x == b);
    if has(&["campfire", "bonfire", "firepit", "костёр", "костер"]) || pair("fire", "pit") || pair("camp", "fire") {
        Some(Glow::Campfire)
    } else if has(&["fireplace", "hearth", "камин", "очаг"]) || pair("fire", "place") {
        Some(Glow::Fireplace)
    } else if has(&["brazier", "torch", "torches", "sconce", "жаровня", "факел"]) {
        Some(Glow::Brazier)
    } else if has(&["candle", "candles", "candlestick", "candelabra", "chandelier", "свеча", "свечи", "люстра"]) {
        Some(Glow::Candle)
    } else if has(&["lamp", "lamps", "lantern", "lanterns", "streetlight", "lamppost", "фонарь", "лампа"]) || pair("street", "light") {
        Some(Glow::Lamp)
    } else {
        None
    }
}

/// Крона по имени: только встроенное дерево. У чужих деревьев листва может быть любой
/// или не быть вовсе (сухие, зимние), поэтому они остаются в картинке, пока крону не
/// включат у объекта рукой.
pub fn canopy_by_name(key: &str) -> bool {
    key == crate::library::TREE
}

impl Prop {
    /// Углы на карте: левый верхний, правый верхний, правый нижний, левый нижний.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let (hw, hh) = (self.size[0] * self.scale / 2.0, self.size[1] * self.scale / 2.0);
        let (s, c) = self.rot.to_radians().sin_cos();
        [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|[x, y]| [self.pos[0] + x * c - y * s, self.pos[1] + x * s + y * c])
    }

    pub fn casts_shadow(&self) -> bool {
        self.shadow.unwrap_or_else(|| !flat_by_name(&self.asset))
    }

    /// Чем светит в Foundry: выбранное вручную или узнанное по имени.
    pub fn light(&self) -> Option<Glow> {
        self.light.or_else(|| glow_by_name(&self.asset)).filter(|g| *g != Glow::Off)
    }

    /// Крона: в Foundry тайлом над токенами, в картинке пень.
    pub fn is_canopy(&self) -> bool {
        self.canopy.unwrap_or_else(|| canopy_by_name(&self.asset))
    }

    pub fn layer(&self) -> i16 {
        self.layer.unwrap_or_else(|| layer_by_name(&self.asset))
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        let (s, c) = (-self.rot).to_radians().sin_cos();
        let (dx, dy) = (p[0] - self.pos[0], p[1] - self.pos[1]);
        let (x, y) = (dx * c - dy * s, dx * s + dy * c);
        x.abs() <= self.size[0] * self.scale / 2.0 && y.abs() <= self.size[1] * self.scale / 2.0
    }
}

/// Форма крыши. Как в Dungeondraft, крыша ставится прямоугольником поверх
/// дома; сложный дом — несколько крыш внахлёст.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoofShape {
    /// Два ската, конёк вдоль.
    Gable,
    /// Четыре ската: торцы тоже скатами.
    Hip,
    /// Один скат.
    Shed,
    Flat,
    /// Восьмигранный шатёр — для башен.
    Tent,
}

impl RoofShape {
    pub const ALL: [RoofShape; 5] = [RoofShape::Gable, RoofShape::Hip, RoofShape::Shed, RoofShape::Flat, RoofShape::Tent];

    pub fn label(self) -> &'static str {
        match self {
            RoofShape::Gable => "Двускатная",
            RoofShape::Hip => "Вальмовая",
            RoofShape::Shed => "Односкатная",
            RoofShape::Flat => "Плоская",
            RoofShape::Tent => "Шатёр",
        }
    }
}

/// Крыша над прямоугольником клеток.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Roof {
    /// Углы по стенам, в клетках: левый верхний и правый нижний.
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub shape: RoofShape,
    /// Картинка покрытия: ряды поперёк, низ картинки — к свесу.
    pub paint: String,
    /// Четверти поворота: конёк поперёк, у односкатной — куда скат.
    #[serde(default)]
    pub turn: u8,
    /// Насколько крыша выходит за стены, в клетках.
    pub overhang: f32,
}

/// Скат крыши: выпуклый многоугольник и как на нём лежит покрытие —
/// `along` вдоль рядов (x картинки), `down` вниз по скату (y картинки).
#[derive(Clone, Debug, PartialEq)]
pub struct RoofPlane {
    pub poly: Vec<[f32; 2]>,
    pub origin: [f32; 2],
    pub along: [f32; 2],
    pub down: [f32; 2],
}

impl RoofPlane {
    fn new(poly: Vec<[f32; 2]>, origin: [f32; 2], down: [f32; 2]) -> Self {
        // Поворот на четверть: при `down` вниз ряды идут слева направо.
        Self { poly, origin, along: [down[1], -down[0]], down }
    }

    /// Насколько точка внутри (плюс) или снаружи (минус), в клетках.
    pub fn inside(&self, p: [f32; 2]) -> f32 {
        convex_depth(&self.poly, p)
    }
}

/// Глубина точки в выпуклом многоугольнике: до ближайшей стороны, снаружи — с минусом.
pub fn convex_depth(poly: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = poly.len();
    let area: f32 = (0..n).map(|i| poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1]).sum();
    let s = area.signum();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            s * ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx) / -len
        })
        .fold(f32::INFINITY, f32::min)
}

impl Roof {
    pub fn new(a: [f32; 2], b: [f32; 2], shape: RoofShape, paint: String) -> Self {
        let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
        Self { a: lo, b: hi, shape, paint, turn: 0, overhang: 0.25 }
    }

    /// Прямоугольник со свесом.
    fn outer(&self) -> ([f32; 2], [f32; 2]) {
        let o = self.overhang;
        ([self.a[0] - o, self.a[1] - o], [self.b[0] + o, self.b[1] + o])
    }

    /// Восемь углов шатра — вписаны в прямоугольник, стороны по осям.
    fn octagon(&self) -> Vec<[f32; 2]> {
        let (lo, hi) = self.outer();
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let (rx, ry) = ((hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0);
        // Восьмиугольник, а не окружность: стороны касаются рамки.
        let k = 1.0 / (std::f32::consts::PI / 8.0).cos();
        (0..8)
            .map(|i| {
                let t = (i as f32 + 0.5) * std::f32::consts::FRAC_PI_4;
                [c[0] + t.cos() * rx * k.min(1.08), c[1] + t.sin() * ry * k.min(1.08)]
            })
            .collect()
    }

    /// Контур крыши сверху.
    pub fn outline(&self) -> Vec<[f32; 2]> {
        if self.shape == RoofShape::Tent {
            return self.octagon();
        }
        let (lo, hi) = self.outer();
        vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        convex_depth(&self.outline(), p) >= 0.0
    }

    /// Конёк вдоль x: по длинной стороне, `turn` — поперёк.
    fn along_x(&self) -> bool {
        let (w, h) = (self.b[0] - self.a[0], self.b[1] - self.a[1]);
        (w >= h) != (self.turn % 2 == 1)
    }

    /// Скаты и линии коньков и рёбер (на них ложатся гребни).
    pub fn geometry(&self) -> (Vec<RoofPlane>, Vec<Segment>) {
        let (lo, hi) = self.outer();
        match self.shape {
            RoofShape::Flat => (vec![RoofPlane::new(self.outline(), lo, [0.0, 1.0])], Vec::new()),
            RoofShape::Shed => {
                let down = [[0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [1.0, 0.0]][(self.turn % 4) as usize];
                // Начало — у верхнего края ската.
                let origin = [if down[0] < 0.0 { hi[0] } else { lo[0] }, if down[1] < 0.0 { hi[1] } else { lo[1] }];
                (vec![RoofPlane::new(self.outline(), origin, down)], Vec::new())
            }
            RoofShape::Tent => {
                let pts = self.octagon();
                let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
                let planes = (0..8)
                    .map(|i| {
                        let (p, q) = (pts[i], pts[(i + 1) % 8]);
                        let m = [(p[0] + q[0]) / 2.0 - c[0], (p[1] + q[1]) / 2.0 - c[1]];
                        let l = (m[0] * m[0] + m[1] * m[1]).sqrt().max(1e-6);
                        RoofPlane::new(vec![c, p, q], c, [m[0] / l, m[1] / l])
                    })
                    .collect();
                (planes, pts.iter().map(|&p| (c, p)).collect())
            }
            RoofShape::Gable | RoofShape::Hip => {
                // Считаем как будто конёк вдоль x; иначе меняем оси местами.
                let swap = !self.along_x();
                let f = |p: [f32; 2]| if swap { [p[1], p[0]] } else { p };
                let (lo, hi) = (f(lo), f(hi));
                let m = (lo[1] + hi[1]) / 2.0;
                let half = (hi[1] - lo[1]) / 2.0;
                let r = if self.shape == RoofShape::Hip { half.min((hi[0] - lo[0]) / 2.0) } else { 0.0 };
                let (r0, r1) = ([lo[0] + r, m], [hi[0] - r, m]);
                let mut planes = vec![
                    (vec![lo, [hi[0], lo[1]], r1, r0], r0, [0.0, -1.0]),
                    (vec![r0, r1, hi, [lo[0], hi[1]]], r0, [0.0, 1.0]),
                ];
                let mut lines = vec![(r0, r1)];
                if r > 0.0 {
                    planes.push((vec![lo, r0, [lo[0], hi[1]]], r0, [-1.0, 0.0]));
                    planes.push((vec![[hi[0], lo[1]], hi, r1], r1, [1.0, 0.0]));
                    lines.extend([(lo, r0), ([lo[0], hi[1]], r0), ([hi[0], lo[1]], r1), (hi, r1)]);
                }
                let planes = planes.into_iter().map(|(poly, o, d)| RoofPlane::new(poly.into_iter().map(f).collect(), f(o), f(d))).collect();
                (planes, lines.into_iter().map(|(a, b)| (f(a), f(b))).filter(|(a, b)| a != b).collect())
            }
        }
    }
}

/// Освещённость ската: обращённый к свету светлее. `light` — откуда свет.
pub fn roof_shade(down: [f32; 2], light: [f32; 2], flat: bool) -> f32 {
    if flat {
        return 0.88;
    }
    0.7 + 0.3 * (down[0] * light[0] + down[1] * light[1])
}

/// Подпись на карте: название комнаты, улицы, метка.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub text: String,
    /// Середина надписи, в клетках.
    pub pos: [f32; 2],
    /// Высота букв в клетках.
    pub size: f32,
    pub color: [u8; 3],
    /// Поворот в градусах по часовой.
    #[serde(default)]
    pub rot: f32,
    /// Только для ведущего: видна в редакторе, в картинку для игроков не идёт.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gm: bool,
}

impl Label {
    pub fn new(text: String, pos: [f32; 2]) -> Self {
        Self { text, pos, size: 0.6, color: [240, 228, 200], rot: 0.0, gm: false }
    }

    /// Приблизительная рамка в своих осях: полуширина и полувысота.
    pub fn half_extent(&self) -> [f32; 2] {
        let longest = self.text.lines().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
        let lines = self.text.lines().count().max(1) as f32;
        [longest * self.size * 0.29, lines * self.size * 0.6]
    }

    pub fn contains(&self, p: [f32; 2]) -> bool {
        let (s, c) = (-self.rot).to_radians().sin_cos();
        let (dx, dy) = (p[0] - self.pos[0], p[1] - self.pos[1]);
        let (x, y) = (dx * c - dy * s, dx * s + dy * c);
        let [hw, hh] = self.half_extent();
        x.abs() <= hw && y.abs() <= hh
    }
}

/// Плоское по имени: лежит на полу и тени не даёт.
pub fn flat_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["rug", "carpet", "mat_", "shadow", "puddle", "stain", "blood", "decal", "ковёр", "ковер", "ковр", "тень", "лужа", "кровь", "пятно"].iter().any(|w| k.contains(w))
}

/// Падающие тени: свет с одной стороны, тень — смещённый размытый силуэт стен
/// и объектов.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shadows {
    pub on: bool,
    /// Куда падает тень, градусы по часовой от «вправо»: 45 — вправо-вниз.
    pub angle: f32,
    /// Насколько отходит, в клетках.
    pub dist: f32,
    /// Ширина размытия, в клетках.
    pub blur: f32,
    pub opacity: f32,
    /// Затухание: 0 — тень ровная до края, 1 — тает от самой стены до конца.
    #[serde(default)]
    pub fade: f32,
}

impl Default for Shadows {
    /// Карты, нарисованные до теней, открываются без них.
    fn default() -> Self {
        Self { on: false, angle: 45.0, dist: 0.2, blur: 0.3, opacity: 0.55, fade: 0.0 }
    }
}

impl Shadows {
    pub fn offset(&self) -> [f32; 2] {
        let (s, c) = self.angle.to_radians().sin_cos();
        [c * self.dist, s * self.dist]
    }

    /// Тень слоями: каждый дальше и размытее прошлого и вложен в него, так что у стены
    /// ложатся все, а к дальнему краю остаётся один. Слоёв с разным сдвигом хватает и
    /// стенам, и объектам, и крышам, а в углах ничего не темнеет вдвое: слой один на контур.
    pub fn layers(&self) -> Vec<Shadows> {
        const LAYERS: usize = 10;
        let s = self.fade.clamp(0.0, 1.0);
        if s <= 0.0 {
            return vec![self.clone()];
        }
        let opacity = 1.0 - (1.0 - self.opacity.min(0.99)).powf(1.0 / LAYERS as f32);
        (1..=LAYERS)
            .map(|k| {
                let reach = self.dist * (1.0 - s + s * k as f32 / LAYERS as f32);
                Shadows { dist: reach * (1.0 - s / 2.0), blur: self.blur + reach * s, opacity, ..self.clone() }
            })
            .collect()
    }

    /// Откуда свет: против направления тени.
    pub fn light(&self) -> [f32; 2] {
        let (s, c) = self.angle.to_radians().sin_cos();
        [-c, -s]
    }
}

/// Дорога, тропа, ручей, ковёр: полоса пола вдоль плавной линии через вершины.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Path {
    pub points: Vec<[f32; 2]>,
    /// Номер в `Map::materials`.
    pub material: usize,
    /// Ширина в клетках.
    pub width: f32,
    /// Доля полуширины, на которой край тает: 0 — резкий.
    #[serde(default)]
    pub soft: f32,
    /// Неровность края: насколько ширина гуляет, доля ширины.
    #[serde(default)]
    pub rough: f32,
    /// Текстура идёт вдоль линии (ковёр, мостовая), а не лежит по сетке, как пол.
    #[serde(default)]
    pub along: bool,
}

impl Path {
    /// Образец для новой дороги: всё, кроме точек.
    pub fn template(material: usize) -> Self {
        Self { points: Vec::new(), material, width: 2.0, soft: 0.35, rough: 0.25, along: false }
    }

    /// Плавная линия через вершины (кривая Катмулла — Рома), шагом около 0.15 клетки.
    pub fn curve(&self) -> Vec<[f32; 2]> {
        smooth_line(&self.points)
    }

    pub fn distance(&self, p: [f32; 2]) -> f32 {
        let c = self.curve();
        c.windows(2).map(|w| segment_distance(p, w[0], w[1])).fold(f32::INFINITY, f32::min)
    }
}

/// Кривая Катмулла — Рома через точки; у концов — повтор крайней точки.
pub fn smooth_line(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let n = pts.len();
    let at = |i: isize| pts[i.clamp(0, n as isize - 1) as usize];
    let mut out = vec![pts[0]];
    for i in 0..n - 1 {
        let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
        let len = ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt();
        let steps = ((len / 0.15).ceil() as usize).clamp(1, 400);
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let w = [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0];
            out.push(std::array::from_fn(|c| w[0] * p0[c] + w[1] * p1[c] + w[2] * p2[c] + w[3] * p3[c]));
        }
    }
    out
}

/// Отрезки стен за вычетом проёмов дверей и окон, лежащих на них, — для выгрузки:
/// тайный ход тоже вырезан, в VTT на его месте потайная дверь.
pub fn cut_portals(segs: &[Segment], portals: &[Portal]) -> Vec<Segment> {
    cut(segs, portals.iter(), 0.0)
}

/// То же для рисования: концы стены продлеваются на полтолщины, поэтому дыра шире
/// на `grow` с каждой стороны — и стена встаёт в край проёма, а не заходит в него.
/// Тайные ходы стену не рвут.
pub fn cut_portals_wide(segs: &[Segment], portals: &[Portal], grow: f32) -> Vec<Segment> {
    cut(segs, portals.iter().filter(|p| !p.kind.hidden()), grow)
}

fn cut<'a>(segs: &[Segment], portals: impl Iterator<Item = &'a Portal>, grow: f32) -> Vec<Segment> {
    let portals: Vec<&Portal> = portals.collect();
    let mut out = Vec::new();
    for &(a, b) in segs {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            continue;
        }
        let t = |p: [f32; 2]| ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len;
        let mut holes: Vec<(f32, f32)> = portals
            .iter()
            .filter(|p| segment_line_distance(p.a, a, b) < 0.02 && segment_line_distance(p.b, a, b) < 0.02)
            .map(|p| {
                let (t0, t1) = (t(p.a), t(p.b));
                ((t0.min(t1) - grow).max(0.0), (t0.max(t1) + grow).min(len))
            })
            .filter(|(s, e)| e > s)
            .collect();
        holes.sort_by(|x, y| x.0.total_cmp(&y.0));
        let at = |t: f32| [a[0] + dx * t / len, a[1] + dy * t / len];
        let mut from = 0.0;
        for (s, e) in holes {
            if s > from {
                out.push((at(from), at(s)));
            }
            from = from.max(e);
        }
        if from < len {
            out.push((at(from), b));
        }
    }
    out
}

/// Расстояние от точки до прямой через отрезок (не до самого отрезка).
fn segment_line_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        return f32::INFINITY;
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
}


fn default_materials() -> Vec<Material> {
    builtin_materials()
}

/// Слой материала: где и с какой силой он лежит.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Coat {
    /// Номер в `Map::materials`.
    pub material: u8,
    pub mask: Mask,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Map {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub materials: Vec<Material>,
    /// Местность: слои снизу вверх.
    pub terrain: Vec<Coat>,
    /// Где комнаты: по краю этой маски встают стены.
    pub rooms: Mask,
    /// Пол комнат: слои снизу вверх, видны только внутри `rooms`.
    pub room_floor: Vec<Coat>,
    pub walls: Vec<Wall>,
    /// Вид новых стен. Уже стоящие его не меняют: у них свой (см. `edge_looks`).
    #[serde(default)]
    pub wall_style: WallStyle,
    /// Виды стен комнат, которые есть на карте.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wall_looks: Vec<WallStyle>,
    /// Вид каждого отрезка стены комнат: середина отрезка → номер в `wall_looks`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty", with = "edge_looks_text")]
    pub edge_looks: BTreeMap<(i32, i32), u8>,
    #[serde(default)]
    pub portals: Vec<Portal>,
    #[serde(default)]
    pub props: Vec<Prop>,
    /// Дороги и реки — поверх пола, под объектами.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<Path>,
    #[serde(default)]
    pub shadows: Shadows,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roofs: Vec<Roof>,
    /// Имя этажа; пусто — «Этаж N».
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Остальные этажи снизу вверх — только у карты в файле, которая сама первый
    /// этаж. Размер у всех один: в Foundry это уровни одной сцены.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Map>,
    /// Нарисованная тень: насколько темно в точке, 255 — чёрное.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade: Option<Mask>,
}

/// Ключ отрезка стены комнаты — его середина с шагом 1/64 клетки.
pub fn edge_key(s: &Segment) -> (i32, i32) {
    (((s.0[0] + s.1[0]) * 32.0).round() as i32, ((s.0[1] + s.1[1]) * 32.0).round() as i32)
}

/// `edge_looks` в файле — одной строкой «x,y,вид …»: отрезков тысячи.
mod edge_looks_text {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(map: &BTreeMap<(i32, i32), u8>, s: S) -> Result<S::Ok, S::Error> {
        let text: Vec<String> = map.iter().map(|((x, y), v)| format!("{x},{y},{v}")).collect();
        s.serialize_str(&text.join(" "))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<(i32, i32), u8>, D::Error> {
        let text = String::deserialize(d)?;
        Ok(text
            .split_whitespace()
            .filter_map(|e| {
                let mut p = e.split(',');
                Some(((p.next()?.parse().ok()?, p.next()?.parse().ok()?), p.next()?.parse().ok()?))
            })
            .collect())
    }
}

/// Первый формат: материал целыми клетками.
#[derive(Deserialize)]
struct MapV1 {
    width: u32,
    height: u32,
    #[serde(default = "default_materials")]
    materials: Vec<Material>,
    floor: Vec<u8>,
    #[serde(default)]
    rooms: Vec<u8>,
    walls: Vec<Wall>,
    #[serde(default)]
    wall_style: WallStyle,
    #[serde(default)]
    portals: Vec<Portal>,
    #[serde(default)]
    props: Vec<Prop>,
}

/// Наложить силу `v` (0..1) на точку: материал растёт, остальные слои уступают.
fn lay(coats: &mut Vec<Coat>, material: u8, template: &Mask, weights: &[(usize, f32)]) {
    let k = match coats.iter().position(|c| c.material == material) {
        Some(k) => k,
        None => {
            coats.push(Coat { material, mask: Mask { data: vec![0; template.data.len()], ..template.clone() } });
            coats.len() - 1
        }
    };
    for &(i, v) in weights {
        for (n, c) in coats.iter_mut().enumerate() {
            let x = c.mask.data[i] as f32;
            c.mask.data[i] = if n == k { (x + v * (255.0 - x)).round() as u8 } else { (x * (1.0 - v)).round() as u8 };
        }
    }
}

fn fade(mask: &mut Mask, weights: &[(usize, f32)]) {
    for &(i, v) in weights {
        mask.data[i] = (mask.data[i] as f32 * (1.0 - v)).round() as u8;
    }
}

/// Прозрачность слоя при рисовании. В маске — доля слоя в точке: кисть, прибавляя
/// одному, убавляет остальным, и вместе их не больше 255. Рисуются же слои поверх
/// друг друга, и с долей вместо прозрачности на мягком крае просвечивал фон (тень
/// между двумя слоями по 50% — четверть фона). Слою достаётся доля от того, что
/// оставят верхние: `share / (255 − above)`, где `above` — сумма долей выше.
pub fn coat_alpha(share: u8, above: u32) -> u8 {
    let rest = 255u32.saturating_sub(above);
    if share as u32 >= rest {
        return 255;
    }
    ((share as u32 * 255 + rest / 2) / rest) as u8
}

fn dominant(coats: &[Coat], i: isize, j: isize) -> u8 {
    coats.iter().rev().map(|c| (c.material, c.mask.get(i, j))).filter(|(_, v)| *v >= 128).map(|(m, _)| m + 1).next().unwrap_or(0)
}

impl Map {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            format: FORMAT,
            width,
            height,
            materials: builtin_materials(),
            terrain: Vec::new(),
            rooms: Mask::for_cells(width, height),
            room_floor: Vec::new(),
            walls: Vec::new(),
            wall_style: WallStyle::default(),
            wall_looks: Vec::new(),
            edge_looks: BTreeMap::new(),
            portals: Vec::new(),
            props: Vec::new(),
            paths: Vec::new(),
            shadows: Shadows { on: true, ..Shadows::default() },
            shade: None,
            labels: Vec::new(),
            roofs: Vec::new(),
            name: String::new(),
            levels: Vec::new(),
        }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    /// Мазок: материал `material` кладётся по весам; для комнат растёт и сама комната.
    pub fn paint(&mut self, layer: Layer, material: usize, weights: &[(usize, f32)]) {
        let m = material as u8;
        let template = self.rooms.clone();
        match layer {
            Layer::Floor => lay(&mut self.terrain, m, &template, weights),
            Layer::Rooms => {
                lay(&mut self.room_floor, m, &template, weights);
                for &(i, v) in weights {
                    let x = self.rooms.data[i] as f32;
                    self.rooms.data[i] = (x + v * (255.0 - x)).round() as u8;
                }
            }
        }
    }

    /// Стирание: слабеет всё — и местность, и комнаты.
    pub fn erase(&mut self, weights: &[(usize, f32)]) {
        for c in self.terrain.iter_mut().chain(self.room_floor.iter_mut()) {
            fade(&mut c.mask, weights);
        }
        fade(&mut self.rooms, weights);
    }

    /// Тень кистью: точка темнеет до `level` × сила отпечатка, но не светлеет —
    /// ступени 20/40/60% ложатся ровно, сколько ни води.
    pub fn shade_paint(&mut self, weights: &[(usize, f32)], level: f32) {
        let template = &self.rooms;
        let m = self.shade.get_or_insert_with(|| Mask { data: vec![0; template.data.len()], ..template.clone() });
        for &(i, v) in weights {
            let t = (level * v * 255.0).round().clamp(0.0, 255.0) as u8;
            m.data[i] = m.data[i].max(t);
        }
    }

    pub fn shade_fade(&mut self, weights: &[(usize, f32)]) {
        if let Some(m) = &mut self.shade {
            fade(m, weights);
        }
    }

    /// Выбросить опустевшие слои — после мазка.
    pub fn prune(&mut self) {
        if self.shade.as_ref().is_some_and(|m| m.is_empty()) {
            self.shade = None;
        }
        self.terrain.retain(|c| !c.mask.is_empty());
        self.room_floor.retain(|c| !c.mask.is_empty());
    }

    /// Целые клетки прямоугольника. `true` — что-то поменялось.
    pub fn paint_rect(&mut self, layer: Layer, a: (i32, i32), b: (i32, i32), material: Option<usize>) -> bool {
        let weights = mask::cells(self.rooms.w, self.rooms.h, a, b);
        self.paint_weights(layer, &weights, material)
    }

    /// Овал, вписанный в прямоугольник клеток. `true` — что-то поменялось.
    pub fn paint_oval(&mut self, layer: Layer, a: (i32, i32), b: (i32, i32), material: Option<usize>) -> bool {
        let weights = mask::oval(self.rooms.w, self.rooms.h, a, b);
        self.paint_weights(layer, &weights, material)
    }

    fn paint_weights(&mut self, layer: Layer, weights: &[(usize, f32)], material: Option<usize>) -> bool {
        let before = self.clone();
        match material {
            Some(m) => self.paint(layer, m, weights),
            None => self.erase(weights),
        }
        self.prune();
        *self != before
    }

    /// Материал (номер + 1) в середине клетки; 0 — пусто.
    pub fn at(&self, layer: Layer, x: i32, y: i32) -> u8 {
        if !self.contains(x, y) {
            return 0;
        }
        let c = (mask::RES / 2) as isize;
        let (i, j) = (x as isize * mask::RES as isize + c, y as isize * mask::RES as isize + c);
        match layer {
            Layer::Floor => dominant(&self.terrain, i, j),
            Layer::Rooms if self.rooms.get(i, j) >= 128 => dominant(&self.room_floor, i, j),
            Layer::Rooms => 0,
        }
    }

    pub fn floor_at(&self, x: i32, y: i32) -> u8 {
        self.at(Layer::Floor, x, y)
    }

    pub fn room_at(&self, x: i32, y: i32) -> u8 {
        self.at(Layer::Rooms, x, y)
    }

    /// Сколько клеток закрашено (по середине клетки).
    pub fn count(&self, layer: Layer) -> usize {
        (0..self.height as i32).flat_map(|y| (0..self.width as i32).map(move |x| (x, y))).filter(|&(x, y)| self.at(layer, x, y) != 0).count()
    }

    pub fn floor_count(&self) -> usize {
        self.count(Layer::Floor)
    }

    /// Номер материала в палитре карты; новый дописывается в конец.
    pub fn material_index(&mut self, m: Material) -> Option<usize> {
        if let Some(i) = self.materials.iter().position(|x| x.paint == m.paint) {
            return Some(i);
        }
        if self.materials.len() >= 255 {
            return None;
        }
        self.materials.push(m);
        Some(self.materials.len() - 1)
    }

    /// Выгнуть стену комнаты: край `a`–`b` становится дугой с прогибом `h`
    /// (плюс — влево от `a`→`b`). Наружу комната прирастает полом `material`,
    /// внутрь — теряет кусок; местность за стеной не трогается.
    pub fn bend_room(&mut self, a: [f32; 2], b: [f32; 2], h: f32, material: usize) {
        let Some((c, r, n)) = arc_circle(a, b, h) else { return };
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let s = h.signum();
        let res = mask::RES as f32;
        let half = max_bend(a, b);
        let probe = [mid[0] + n[0] * s * 0.6 / res, mid[1] + n[1] * s * 0.6 / res];
        let (pi, pj) = ((probe[0] * res).floor() as isize, (probe[1] * res).floor() as isize);
        let grow = self.rooms.get(pi, pj) < 128;
        let pts = arc_points(a, b, h);
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &pts {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let (w, hgt) = (self.rooms.w as isize, self.rooms.h as isize);
        let cl = |v: f32, m: isize| (v.floor() as isize).clamp(0, m);
        let mut weights = Vec::new();
        for j in cl((lo[1] - 0.5) * res, hgt)..cl((hi[1] + 0.5) * res + 1.0, hgt) {
            for i in cl((lo[0] - 0.5) * res, w)..cl((hi[0] + 0.5) * res + 1.0, w) {
                let p = mask::sample_pos(i, j);
                // Внутри круга и по ту сторону хорды, куда выгнули; у хорды — с запасом,
                // чтобы шов с прежней комнатой не остался полупрозрачным.
                let side = ((p[0] - mid[0]) * n[0] + (p[1] - mid[1]) * n[1]) * s;
                let round = r - ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt();
                // Запас за хордой — только между её концами, иначе у концов по
                // соседним стенам расплываются пятна.
                let along = (p[0] - mid[0]) * -n[1] + (p[1] - mid[1]) * n[0];
                let ends = if side < 0.0 { half - along.abs() } else { f32::MAX };
                let v = (0.5 + round.min(side + 0.3).min(ends) * res).clamp(0.0, 1.0);
                if v > 0.0 {
                    weights.push(((j * w + i) as usize, v));
                }
            }
        }
        if grow {
            self.paint(Layer::Rooms, material, &weights);
        } else {
            for c in self.room_floor.iter_mut() {
                fade(&mut c.mask, &weights);
            }
            fade(&mut self.rooms, &weights);
        }
        self.prune();
    }

    /// Где лежит материал `m` (сила 0..255): местность, пол комнат в комнатах
    /// и дороги этим материалом — для живых областей выгрузки.
    pub fn material_mask(&self, m: usize) -> Mask {
        let mut out = Mask { data: vec![0; self.rooms.data.len()], ..self.rooms.clone() };
        for c in self.terrain.iter().filter(|c| c.material as usize == m) {
            for (o, v) in out.data.iter_mut().zip(&c.mask.data) {
                *o = (*o).max(*v);
            }
        }
        for c in self.room_floor.iter().filter(|c| c.material as usize == m) {
            for ((o, v), r) in out.data.iter_mut().zip(&c.mask.data).zip(&self.rooms.data) {
                *o = (*o).max((*v).min(*r));
            }
        }
        let res = mask::RES as f32;
        for p in self.paths.iter().filter(|p| p.material == m) {
            let line = p.curve();
            let hw = p.width / 2.0;
            for w in line.windows(2) {
                let (lo, hi) = ([w[0][0].min(w[1][0]) - hw, w[0][1].min(w[1][1]) - hw], [w[0][0].max(w[1][0]) + hw, w[0][1].max(w[1][1]) + hw]);
                let cl = |v: f32, n: usize| ((v * res).floor().max(0.0) as usize).min(n);
                for j in cl(lo[1], out.h)..cl(hi[1] + 1.0 / res, out.h) {
                    for i in cl(lo[0], out.w)..cl(hi[0] + 1.0 / res, out.w) {
                        if segment_distance(mask::sample_pos(i as isize, j as isize), w[0], w[1]) <= hw {
                            out.data[j * out.w + i] = 255;
                        }
                    }
                }
            }
        }
        out
    }

    /// Новый пустой этаж того же размера, с той же палитрой и видом стен.
    pub fn new_level(&self) -> Map {
        let mut m = Map::new(self.width, self.height);
        m.materials = self.materials.clone();
        m.wall_style = self.wall_style.clone();
        m.shadows = self.shadows.clone();
        m
    }

    /// Стены вокруг комнат: контур маски комнат.
    pub fn room_edges(&self) -> Vec<Segment> {
        mask::contour(&self.rooms)
    }

    fn look_id(&mut self, st: &WallStyle) -> u8 {
        match self.wall_looks.iter().position(|l| l == st) {
            Some(i) => i as u8,
            None if self.wall_looks.len() < 255 => {
                self.wall_looks.push(st.clone());
                (self.wall_looks.len() - 1) as u8
            }
            None => 254,
        }
    }

    /// После правки комнат: стоявшие отрезки хранят свой вид, новые берут вид новых стен.
    /// Ключи всех картинок, что стоят на карте: пол, стены, проёмы, объекты, крыши.
    pub fn used_assets(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        let mut paint = |p: &Paint| {
            if let Paint::Texture(k) = p {
                out.insert(k.clone());
            }
        };
        self.materials.iter().for_each(|m| paint(&m.paint));
        paint(&self.wall_style.paint);
        self.wall_looks.iter().for_each(|w| paint(&w.paint));
        self.walls.iter().filter_map(|w| w.style.as_ref()).for_each(|w| paint(&w.paint));
        for p in &self.portals {
            out.extend(p.asset.iter().chain(p.frame.iter()).cloned());
        }
        out.extend(self.props.iter().map(|p| p.asset.clone()));
        out.extend(self.roofs.iter().map(|r| r.paint.clone()));
        out
    }

    pub fn sync_edge_looks(&mut self, edges: &[Segment]) {
        let fresh = self.wall_style.clone();
        let id = self.look_id(&fresh);
        let old = std::mem::take(&mut self.edge_looks);
        self.edge_looks = edges.iter().map(|s| edge_key(s)).map(|k| (k, old.get(&k).copied().unwrap_or(id))).collect();
    }

    pub fn edge_look(&self, key: (i32, i32)) -> WallStyle {
        self.edge_looks.get(&key).and_then(|&i| self.wall_looks.get(i as usize)).unwrap_or(&self.wall_style).clone()
    }

    pub fn set_edge_look(&mut self, edges: &[Segment], st: &WallStyle) {
        let id = self.look_id(st);
        for s in edges {
            self.edge_looks.insert(edge_key(s), id);
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        // Блокнот и PowerShell пишут UTF-8 с меткой в начале.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("файл не читается как карта: {e}"))?;
        let format = value.get("format").and_then(|f| f.as_u64()).unwrap_or(1) as u32;
        if format > FORMAT {
            return Err(format!("карта записана более новой версией (формат {format})"));
        }
        let map = if format == 1 { Self::from_v1(serde_json::from_value(value).map_err(|e| format!("файл не читается как карта: {e}"))?)? } else { serde_json::from_value(value).map_err(|e| format!("файл не читается как карта: {e}"))? };
        let (w, h) = (map.width as usize * mask::RES, map.height as usize * mask::RES);
        let fits = |m: &Mask| m.w == w && m.h == h;
        if map.width == 0 || map.height == 0 || !fits(&map.rooms) || map.shade.as_ref().is_some_and(|m| !fits(m)) || !map.terrain.iter().chain(&map.room_floor).all(|c| fits(&c.mask)) {
            return Err("размер карты не сходится с размером слоёв".into());
        }
        if map.terrain.iter().chain(&map.room_floor).any(|c| c.material as usize >= map.materials.len()) || map.paths.iter().any(|p| p.material >= map.materials.len()) {
            return Err("в карте неизвестный материал пола".into());
        }
        for (k, l) in map.levels.iter().enumerate() {
            if l.width != map.width || l.height != map.height || !l.levels.is_empty() {
                return Err(format!("этаж {} другого размера", k + 2));
            }
            let mut v = serde_json::to_value(l).map_err(|e| e.to_string())?;
            v["format"] = serde_json::json!(FORMAT);
            Self::from_json(&v.to_string()).map_err(|e| format!("этаж {}: {e}", k + 2))?;
        }
        Ok(map)
    }

    /// Карта первого формата: каждая закрашенная клетка становится квадратом маски.
    fn from_v1(old: MapV1) -> Result<Self, String> {
        let n = (old.width * old.height) as usize;
        let rooms = if old.rooms.is_empty() { vec![0; n] } else { old.rooms };
        if old.width == 0 || old.height == 0 || old.floor.len() != n || rooms.len() != n {
            return Err("размер карты не сходится с числом клеток".into());
        }
        if old.floor.iter().chain(&rooms).any(|&c| c as usize > old.materials.len()) {
            return Err("в карте неизвестный материал пола".into());
        }
        let mut map = Map::new(old.width, old.height);
        map.materials = old.materials;
        for (layer, cells) in [(Layer::Floor, &old.floor), (Layer::Rooms, &rooms)] {
            for (k, &v) in cells.iter().enumerate() {
                if v != 0 {
                    let (x, y) = ((k as u32 % old.width) as i32, (k as u32 / old.width) as i32);
                    let weights = mask::cells(map.rooms.w, map.rooms.h, (x, y), (x, y));
                    map.paint(layer, v as usize - 1, &weights);
                }
            }
        }
        map.walls = old.walls;
        map.wall_style = old.wall_style;
        map.portals = old.portals;
        map.props = old.props;
        Ok(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roof_planes_cover_outline_and_face_out() {
        for shape in RoofShape::ALL {
            let mut r = Roof::new([2.0, 1.0], [8.0, 5.0], shape, "x".into());
            for turn in 0..4 {
                r.turn = turn;
                let (planes, lines) = r.geometry();
                let out = r.outline();
                // Каждая точка внутри контура — ровно на одном скате (кроме рёбер).
                let c = [5.0, 3.0];
                for p in [[0.0f32, 0.0], [1.0, 0.3], [-0.6, 0.9], [2.5, -1.8], [-2.7, 1.9]] {
                    let q = [c[0] + p[0], c[1] + p[1]];
                    if convex_depth(&out, q) <= 0.05 {
                        continue;
                    }
                    let hits = planes.iter().filter(|pl| pl.inside(q) > 1e-3).count();
                    assert!(hits <= 1 && planes.iter().any(|pl| pl.inside(q) > -1e-3), "{shape:?} {turn} {q:?}");
                }
                // Скаты смотрят от конька наружу: середина ската дальше по `down`, чем начало.
                for pl in &planes {
                    let n = pl.poly.len() as f32;
                    let m = [pl.poly.iter().map(|p| p[0]).sum::<f32>() / n, pl.poly.iter().map(|p| p[1]).sum::<f32>() / n];
                    assert!((m[0] - pl.origin[0]) * pl.down[0] + (m[1] - pl.origin[1]) * pl.down[1] > 0.0, "{shape:?}");
                }
                match shape {
                    RoofShape::Gable => assert_eq!((planes.len(), lines.len()), (2, 1)),
                    // Поперёк длинной стороны вальма сходится в пирамиду — без конька.
                    RoofShape::Hip => assert!(planes.len() == 4 && (lines.len() == 5 || (turn % 2 == 1 && lines.len() == 4))),
                    RoofShape::Tent => assert_eq!((planes.len(), lines.len()), (8, 8)),
                    _ => assert_eq!(planes.len(), 1),
                }
            }
        }
        // Конёк по длинной стороне, поворот кладёт его поперёк.
        let r = Roof::new([0.0, 0.0], [6.0, 2.0], RoofShape::Gable, "x".into());
        let ridge = r.geometry().1[0];
        assert_eq!(ridge.0[1], ridge.1[1]);
        let t = Roof { turn: 1, ..r };
        let ridge = t.geometry().1[0];
        assert_eq!(ridge.0[0], ridge.1[0]);
    }

    #[test]
    fn levels_round_trip_and_size_is_checked() {
        let mut m = Map::new(5, 4);
        let mut up = m.new_level();
        up.name = "Чердак".into();
        m.levels.push(up);
        let back = Map::from_json(&m.to_json()).unwrap();
        assert_eq!(back.levels.len(), 1);
        assert_eq!(back.levels[0].name, "Чердак");
        m.levels[0] = Map::new(6, 4);
        assert!(Map::from_json(&m.to_json()).is_err());
    }

    #[test]
    fn shade_steps_do_not_stack() {
        let mut m = Map::new(4, 4);
        let w = mask::cells(m.rooms.w, m.rooms.h, (0, 0), (1, 1));
        m.shade_paint(&w, 0.4);
        m.shade_paint(&w, 0.4);
        assert_eq!(m.shade.as_ref().unwrap().data[0], 102);
        m.shade_fade(&w);
        m.prune();
        assert!(m.shade.is_none());
        // Старая карта без теней открывается с выключенными тенями.
        let mut j: serde_json::Value = serde_json::from_str(&Map::new(2, 2).to_json()).unwrap();
        j.as_object_mut().unwrap().remove("shadows");
        assert!(!Map::from_json(&j.to_string()).unwrap().shadows.on);
        assert!(Map::new(2, 2).shadows.on);
    }

    #[test]
    fn path_curve_passes_through_points() {
        let mut p = Path::template(0);
        p.points = vec![[0.0, 0.0], [4.0, 2.0], [8.0, 0.0]];
        let c = p.curve();
        for q in &p.points {
            assert!(c.iter().any(|v| (v[0] - q[0]).abs() < 1e-4 && (v[1] - q[1]).abs() < 1e-4), "{q:?}");
        }
        assert!(c.windows(2).all(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt() < 0.2));
        assert!(p.distance([4.0, 2.0]) < 1e-3 && p.distance([4.0, 5.0]) > 2.5);
    }

    #[test]
    fn placed_room_walls_keep_their_look() {
        let mut m = Map::new(10, 10);
        let stone = WallStyle { paint: Paint::Color([1, 2, 3]), width: 0.3 };
        let brick = WallStyle { paint: Paint::Color([9, 8, 7]), width: 0.2 };
        m.wall_style = stone.clone();
        m.paint_rect(Layer::Rooms, (1, 1), (2, 2), Some(0));
        let first = m.room_edges();
        m.sync_edge_looks(&first);
        // Вид новых стен сменили и поставили вторую комнату: первая прежняя.
        m.wall_style = brick.clone();
        m.paint_rect(Layer::Rooms, (6, 6), (7, 7), Some(0));
        let all = m.room_edges();
        m.sync_edge_looks(&all);
        assert_eq!(m.edge_look(edge_key(&first[0])), stone);
        let new = all.iter().find(|s| s.0[0] >= 6.0).unwrap();
        assert_eq!(m.edge_look(edge_key(new)), brick);
        // Участок меняется отдельно и переживает файл.
        m.set_edge_look(&[first[0]], &brick);
        let back = Map::from_json(&m.to_json()).unwrap();
        assert_eq!(back.edge_look(edge_key(&first[0])), brick);
        assert_eq!(back.edge_look(edge_key(&first[1])), stone);
    }

    #[test]
    fn shadow_layers_add_up_and_reach_the_same_edge() {
        let sh = Shadows { on: true, dist: 1.0, blur: 0.2, opacity: 0.6, fade: 1.0, ..Shadows::default() };
        let layers = sh.layers();
        let left: f32 = layers.iter().map(|l| 1.0 - l.opacity).product();
        assert!((1.0 - left - 0.6).abs() < 1e-3, "у стены темнота та же");
        let last = layers.last().unwrap();
        assert!((last.dist + last.blur / 2.0 - 1.1).abs() < 1e-3, "край на том же месте");
        assert_eq!(Shadows { fade: 0.0, ..sh }.layers().len(), 1);
    }

    #[test]
    fn wide_cut_leaves_room_for_wall_caps() {
        let d = Portal { a: [2.0, 0.0], b: [3.0, 0.0], kind: PortalKind::Door, asset: None, frame: None };
        let cut = cut_portals_wide(&[([0.0, 0.0], [5.0, 0.0])], &[d], 0.1);
        assert_eq!(cut, vec![([0.0, 0.0], [1.9, 0.0]), ([3.1, 0.0], [5.0, 0.0])]);
    }

    #[test]
    fn paint_is_clipped_to_the_map() {
        let mut m = Map::new(10, 5);
        assert!(m.paint_rect(Layer::Floor, (8, 3), (14, -2), Some(0)));
        assert_eq!(m.floor_count(), 2 * 4, "столбцы 8–9, строки 0–3");
        assert!(!m.paint_rect(Layer::Floor, (8, 0), (9, 3), Some(0)), "повторная заливка тем же ничего не меняет");
        assert_eq!(m.floor_at(-1, 0), 0);
        assert_eq!(m.count(Layer::Rooms), 0, "слои не мешаются");
    }

    #[test]
    fn new_material_covers_the_old_one() {
        let mut m = Map::new(6, 4);
        m.paint_rect(Layer::Floor, (0, 0), (5, 3), Some(0));
        m.paint_rect(Layer::Floor, (1, 1), (2, 2), Some(2));
        assert_eq!((m.floor_at(1, 1), m.floor_at(4, 1)), (3, 1));
        // Стирание и вычистка пустых слоёв.
        m.paint_rect(Layer::Floor, (0, 0), (5, 3), None);
        assert!(m.terrain.is_empty());
    }

    #[test]
    fn soft_edge_between_coats_shows_no_background() {
        // Мягкий мазок вторым материалом поверх залитого первым.
        let mut m = Map::new(4, 1);
        m.paint_rect(Layer::Floor, (0, 0), (3, 0), Some(0));
        let w = mask::RES as usize * 4;
        let weights: Vec<(usize, f32)> = (0..w).map(|i| (i, i as f32 / (w - 1) as f32)).collect();
        m.paint(Layer::Floor, 1, &weights);
        for i in 0..w {
            // Рисование поверх: остаток фона — произведение (1 − прозрачность).
            let mut background = 1.0;
            for (k, c) in m.terrain.iter().enumerate() {
                let above = m.terrain[k + 1..].iter().map(|c| c.mask.data[i] as u32).sum();
                background *= 1.0 - coat_alpha(c.mask.data[i], above) as f32 / 255.0;
            }
            assert!(background < 0.02, "в точке {i} просвечивает фон: {background:.2}");
        }
    }

    #[test]
    fn closed_wall_has_a_segment_back_to_the_start() {
        let w = Wall { points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]], closed: true, bends: Vec::new(), style: None };
        assert_eq!(w.segments().count(), 3);
        assert!((w.distance([1.0, 2.0]) - 1.0).abs() < 0.01, "ближе всего обратный отрезок");
        let open = Wall { closed: false, ..w };
        assert_eq!(open.segments().count(), 2);
    }

    #[test]
    fn room_walls_follow_the_outline() {
        let mut m = Map::new(20, 12);
        m.paint_rect(Layer::Rooms, (2, 2), (7, 6), Some(0));
        assert_eq!(m.room_edges().len(), 4, "прямоугольник — четыре стены");
        m.paint_rect(Layer::Rooms, (5, 5), (10, 8), Some(1));
        let total: f32 = m.room_edges().iter().map(|(a, b)| (b[0] - a[0]).abs() + (b[1] - a[1]).abs()).sum();
        assert!((total - 32.0).abs() < 1e-3, "{total}");
        assert_eq!((m.room_at(3, 3), m.room_at(9, 7)), (1, 2));
    }

    #[test]
    fn portals_cut_only_their_own_wall() {
        let segs = vec![([0.0, 2.0], [6.0, 2.0]), ([6.0, 2.0], [6.0, 5.0])];
        let door = Portal { a: [2.0, 2.0], b: [3.0, 2.0], kind: PortalKind::Door, asset: None, frame: None };
        let window = Portal { a: [4.5, 2.0], b: [5.5, 2.0], kind: PortalKind::Window, asset: None, frame: None };
        let cut = cut_portals(&segs, &[door, window]);
        assert_eq!(cut, vec![([0.0, 2.0], [2.0, 2.0]), ([3.0, 2.0], [4.5, 2.0]), ([5.5, 2.0], [6.0, 2.0]), ([6.0, 2.0], [6.0, 5.0])]);
    }

    #[test]
    fn lights_and_canopies_by_name_and_by_hand() {
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [0.0, 0.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None };
        assert_eq!(prop("builtin:campfire").light(), Some(Glow::Campfire));
        assert_eq!(prop("builtin:street_lamp").light(), Some(Glow::Lamp));
        assert_eq!(prop("pack:fa.dungeondraft_pack|textures/objects/Fire_Pit_Stone_A_1x1.webp").light(), Some(Glow::Campfire));
        assert_eq!(prop("C:\\assets\\objects\\Chandelier_Iron_2x2.webp").light(), Some(Glow::Candle));
        assert_eq!(prop("pack:x|textures/objects/Lamp_Post_A.webp").light(), Some(Glow::Lamp));
        assert_eq!(prop("pack:x|textures/objects/Barrel_A.webp").light(), None);
        // Рукой: выключить похожее по имени и зажечь непохожее.
        assert_eq!(Prop { light: Some(Glow::Off), ..prop("builtin:campfire") }.light(), None);
        assert_eq!(Prop { light: Some(Glow::Brazier), ..prop("pack:x|textures/objects/Barrel_A.webp") }.light(), Some(Glow::Brazier));
        assert!(prop("builtin:tree").is_canopy());
        // Чужие деревья — только рукой: листва у них любая, сухим крона ни к чему.
        assert!(!prop("pack:fa.dungeondraft_pack|textures/objects/Oak_Tree_A_3x3.webp").is_canopy());
        assert!(!prop("pack:x|textures/objects/Dead_Tree_Roots_A_6x6.webp").is_canopy());
        assert!(!prop("builtin:stump").is_canopy());
        assert!(!Prop { canopy: Some(false), ..prop("builtin:tree") }.is_canopy());
        // Слой по имени: ковёр на полу, дерево выше обычного, своё — как выбрано.
        assert_eq!(prop("pack:fa.dungeondraft_pack|textures/objects/Rug_Round_A.webp").layer(), 100);
        assert_eq!(prop("builtin:tree").layer(), 400);
        assert_eq!(prop("builtin:chair").layer(), LAYER_DEFAULT);
        assert_eq!(Prop { layer: Some(ABOVE_WALLS), ..prop("builtin:chair") }.layer(), ABOVE_WALLS);
        assert!(Prop { canopy: Some(true), ..prop("pack:fa.dungeondraft_pack|textures/objects/Oak_Tree_A_3x3.webp") }.is_canopy());
        // В файле необязательные поля не пишутся, старые карты читаются.
        let json = serde_json::to_string(&prop("builtin:tree")).unwrap();
        assert!(!json.contains("light") && !json.contains("canopy"));
        let back: Prop = serde_json::from_str(&json).unwrap();
        assert_eq!(back, prop("builtin:tree"));
    }

    #[test]
    fn rotated_prop_hit_test() {
        let p = Prop { asset: "x".into(), pos: [5.0, 5.0], size: [4.0, 1.0], rot: 90.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None };
        assert!(p.contains([5.0, 6.8]), "повёрнут на 90°: длинная сторона по вертикали");
        assert!(!p.contains([6.8, 5.0]));
        let c = p.corners();
        assert!((c[0][0] - 5.5).abs() < 1e-4 && (c[0][1] - 3.0).abs() < 1e-4);
    }

    #[test]
    fn json_round_trip_and_damage() {
        let mut m = Map::new(6, 4);
        m.paint_rect(Layer::Floor, (1, 1), (2, 2), Some(2));
        m.paint_rect(Layer::Rooms, (3, 0), (4, 1), Some(0));
        let i = m.material_index(Material { name: "Плитка".into(), paint: Paint::Texture("C:/t/tile.png".into()), vary: false, live: Live::Auto }).unwrap();
        let n = builtin_materials().len();
        assert_eq!(i, n);
        assert_eq!(m.material_index(Material { name: "другое имя".into(), paint: Paint::Texture("C:/t/tile.png".into()), vary: false, live: Live::Auto }), Some(n));
        m.walls.push(Wall { points: vec![[1.0, 1.0], [3.0, 1.0]], closed: false, bends: Vec::new(), style: None });
        assert_eq!(Map::from_json(&m.to_json()).unwrap(), m);
        let broken = m.to_json().replace("\"width\": 6", "\"width\": 7");
        assert!(Map::from_json(&broken).is_err());
    }

    #[test]
    fn first_format_files_still_open() {
        let old = r#"{"format":1,"width":3,"height":1,"floor":[2,0,0],"rooms":[0,1,1],"walls":[]}"#;
        let m = Map::from_json(old).unwrap();
        assert_eq!(Map::from_json(&format!("\u{feff}{old}")).unwrap(), m, "метка BOM в начале не мешает");
        assert_eq!(m.materials, builtin_materials());
        assert_eq!((m.floor_at(0, 0), m.room_at(1, 0), m.room_at(0, 0)), (2, 1, 0));
        assert_eq!(m.room_edges().len(), 4);
        assert_eq!(m.format, FORMAT);
    }
    #[test]
    fn arc_passes_through_its_apex_and_ends() {
        let pts = arc_points([0.0, 0.0], [4.0, 0.0], 1.0);
        assert_eq!((pts[0], *pts.last().unwrap()), ([0.0, 0.0], [4.0, 0.0]));
        // Плюс — влево по ходу: при ходе вправо это вверх, к меньшим y.
        let apex = pts.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!((apex + 1.0).abs() < 0.01, "вершина дуги {apex}");
        assert!(pts.iter().all(|p| p[1] <= 1e-4), "дуга вся по одну сторону");
        assert_eq!(arc_points([0.0, 0.0], [4.0, 0.0], 0.0).len(), 2);
        let w = Wall { points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]], closed: false, bends: vec![1.0], style: None };
        assert!(w.segments().count() > 4 && w.distance([2.0, -1.0]) < 0.02);
    }

    #[test]
    fn bending_a_room_wall_grows_or_cuts_the_room() {
        let mut m = Map::new(10, 10);
        m.paint_rect(Layer::Rooms, (2, 2), (7, 7), Some(0));
        let before = m.rooms.area();
        // Верхняя стена от (2,2) до (8,2), прогиб наружу — вверх, влево по ходу вправо.
        m.bend_room([2.0, 2.0], [8.0, 2.0], 2.0, 0);
        let grown = m.rooms.area();
        // Сегмент круга: хорда 6, прогиб 2 — площадь около 8,5 клетки.
        assert!((grown - before - 8.5).abs() < 1.0, "приросло {}", grown - before);
        assert_eq!(m.room_at(4, 0), 1, "пол под дугой того же материала");
        // Снаружи у конца дуги, вдоль боковой стены, комната не расплылась.
        assert!(m.rooms.get(7, 8) == 0 && m.rooms.get(32, 8) == 0, "пятна у концов: {} {}", m.rooms.get(7, 8), m.rooms.get(32, 8));
        // Левая стена внутрь: от (2,8) до (2,2) влево по ходу — наружу, значит минус.
        m.bend_room([2.0, 8.0], [2.0, 2.0], -1.5, 0);
        assert!(m.rooms.area() < grown - 4.0);
        assert_eq!(m.room_at(2, 4), 0, "вогнутая стена вырезала пол");
    }
}
