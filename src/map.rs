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
/// 3 — маски кусками ([`crate::mask`]); 2 и 1 читаются.
pub const FORMAT: u32 = 3;

/// Наибольшая сторона карты, клеток. Маски кусками держат память по нарисованному, а не
/// по размеру: пустая карта 1000×1000 — столько же, сколько 40×40.
pub const MAX_SIDE: u32 = 1000;

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
    /// Оживает ли в Foundry (модуль «Wild Canvas»): вода, лава, болото.
    #[serde(default, skip_serializing_if = "Live::is_auto")]
    pub live: Live,
    /// Поворот узора, градусы по часовой: паркет и плитка вдоль скошенной стены.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rot: f32,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
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
    /// Неон по швам пола («Wild Canvas»): швы киберпанк-пола совпадают с сеткой.
    Neon,
}

impl Live {
    pub const CHOICES: [Live; 6] = [Live::Auto, Live::Off, Live::Water, Live::Lava, Live::Swamp, Live::Neon];

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
            Live::Neon => "неон",
        }
    }

    /// Имя эффекта для модуля Foundry.
    pub fn effect(self) -> Option<&'static str> {
        match self {
            Live::Water => Some("water"),
            Live::Lava => Some("lava"),
            Live::Swamp => Some("swamp"),
            Live::Neon => Some("neon"),
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
        } else if has(&["alloy_grid", "neon", "неон"]) {
            Live::Neon
        } else if has(&["swamp", "bog", "marsh", "болот", "топь"]) {
            Live::Swamp
        } else if has(&["water", "вода", "мелковод", "глубина", "river", "lake", "pond", "sea", "ocean"]) {
            Live::Water
        } else {
            Live::Off
        }
    }
}

impl Material {
    /// Глубокая вода — море, глубина: дна не видно, бликов на нём нет.
    pub fn deep(&self) -> bool {
        let key = match &self.paint {
            Paint::Texture(k) => crate::library::label(k).to_lowercase() + " " + &self.name.to_lowercase(),
            Paint::Color(_) => self.name.to_lowercase(),
        };
        ["deep", "глубин", "ocean", "океан", "sea_", "море"].iter().any(|w| key.contains(w))
    }

    /// Мутная вода — непрозрачная у FA (`Water_Opaque…`), болотная: дна не видно,
    /// пены у берега и рыб нет — в Foundry она стоит тёмным зеркалом.
    pub fn murky(&self) -> bool {
        let key = match &self.paint {
            Paint::Texture(k) => crate::library::label(k).to_lowercase() + " " + &self.name.to_lowercase(),
            Paint::Color(_) => self.name.to_lowercase(),
        };
        ["opaque", "murky", "мутн"].iter().any(|w| key.contains(w))
    }

    /// Вода, в которой рыбы нет: криожидкость, охладитель, кислота, яд. Блики и пена
    /// остаются, а рыб Wild Canvas не рисует («Без рыбы»).
    pub fn fishless(&self) -> bool {
        let key = match &self.paint {
            Paint::Texture(k) => crate::library::label(k).to_lowercase() + " " + &self.name.to_lowercase(),
            Paint::Color(_) => self.name.to_lowercase(),
        };
        ["cryo", "крио", "coolant", "охлад", "acid", "кислот", "toxic", "токсич"].iter().any(|w| key.contains(w))
    }

    /// Стоячая вода в помещении — разлив на полу, лужа, криожидкость в коридоре станции:
    /// ни зыби, ни прибоя у стен. В Foundry — тихое зеркало: свет на дне еле ходит.
    pub fn still(&self) -> bool {
        let key = match &self.paint {
            Paint::Texture(k) => crate::library::label(k).to_lowercase() + " " + &self.name.to_lowercase(),
            Paint::Color(_) => self.name.to_lowercase(),
        };
        ["still", "stagnant", "spill", "puddle", "стояч", "разлив", "лужа", "cryo", "крио", "coolant", "охлад"].iter().any(|w| key.contains(w))
    }

    /// По чему звучат шаги в «Wild Canvas»: буква поверхности или `None` — молчать.
    /// Вода и болото — от живого материала; остальное по имени картинки и материала.
    /// Порядок проверок важен: «песчаник» — камень, а не песок; «иней на траве» — снег.
    pub fn surface(&self) -> Option<char> {
        match self.live() {
            Live::Water => return Some('w'),
            Live::Swamp => return Some('m'),
            Live::Lava => return None,
            _ => {}
        }
        let key = match &self.paint {
            Paint::Texture(k) => format!("{} {} {}", k.to_lowercase(), crate::library::label(k).to_lowercase(), self.name.to_lowercase()),
            Paint::Color(_) => self.name.to_lowercase(),
        };
        let has = |ws: &[&str]| ws.iter().any(|w| key.contains(w));
        if has(&["metal", "iron", "steel", "grate", "металл", "желез", "решётк"]) {
            Some('e')
        } else if has(&["snow", "frost", "ice", "снег", "иней", "лёд", "лед"]) {
            Some('n')
        } else if has(&["gravel", "pebble", "гравий", "галька", "щебень"]) {
            Some('g')
        } else if has(&["mud", "грязь", "wet"]) {
            Some('m')
        } else if has(&["shack", "rotten", "old_wood", "broken", "гнил", "лачуг", "скрип"]) {
            Some('k')
        } else if has(&["wood", "plank", "board", "parquet", "deck", "floor_h", "доск", "дерев", "паркет", "настил"]) {
            Some('o')
        } else if has(&[
            "stone", "cobble", "flagstone", "tile", "brick", "marble", "slate", "rock", "dungeon", "cave", "granite", "basalt", "volcanic",
            "камен", "камн", "плит", "брусчат", "кирпич", "мрамор", "пещер", "песчаник", "скал",
        ]) {
            Some('s')
        } else if has(&["sand", "beach", "desert", "dune", "песок", "песч", "пустын", "пляж"]) {
            Some('a')
        } else if has(&["grass", "moss", "meadow", "lawn", "flora", "трав", "мох", "луг", "подстилк", "forest_floor"]) {
            Some('r')
        } else if has(&["dirt", "earth", "soil", "path", "road", "ash", "земл", "почв", "тропа", "дорог", "пепел"]) {
            Some('d')
        } else {
            None
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
        .map(|t| Material { name: t.1.into(), paint: Paint::Texture(format!("{}{}", crate::library::BUILTIN, t.0)), vary: t.2, live: Live::Auto, rot: 0.0 })
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
    /// Низкая: парапет, зубцы, край хода по стене. В VTT держит шаг, а зрение и
    /// свет — «ограниченно»: поверх одной видно, сквозь две — нет (со стены видно
    /// и поле, и город, а с поля через всю стену в город — нет).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub low: bool,
    /// Свой вид; без него — как у всей карты.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<WallStyle>,
}

pub type Segment = ([f32; 2], [f32; 2]);

impl Wall {
    /// Труба по полу (полоса `Pipe_…`): рисуется как стена, но движение и
    /// свет в VTT не держит.
    pub fn is_decor(&self) -> bool {
        matches!(&self.style, Some(WallStyle { paint: Paint::Texture(k), .. }) if crate::library::label(k).starts_with("Pipe_"))
    }

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
    /// Подкраска картинки (умножением): встроенное дерево осенью, зимой, в магическом лесу.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<[u8; 3]>,
    /// Палатка: с какой стороны картинки вход (0 — верх, 1 — право, 2 — низ, 3 — лево).
    /// Нет — снизу, как у встроенных лавок. С остальных сторон в Foundry — стены полога.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<u8>,
    /// Свой свет поверх вида (`light`): радиус, цвет, анимация, её скорость и сила.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<GlowTune>,
    /// Какая доля предмета под водой, от низа картинки (0–1): кресло по спинку, упавшая
    /// капсула. Нет — предмет над водой целиком.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sunk: Option<f32>,
    /// Отражён слева направо (в Dungeondraft — A при постановке). Поворот — уже после.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// Слои высоты объектов — числа те же, что в Dungeondraft. Внутри слоя выше тот,
/// кто поставлен позже. Стены рисуются между 400 и 700, крыши — между 700 и 900.
pub const LAYERS: [(i16, &str); 7] = [
    (100, "Пол: ковры, лужи"),
    (200, "Низкие: мелочь на полу"),
    (300, "Обычные"),
    (400, "Высокие: деревья, шкафы"),
    (700, "Над стенами"),
    (800, "На крыше: дымоходы, вытяжки"),
    (900, "Над крышами"),
];
pub const LAYER_DEFAULT: i16 = 300;
/// С этого слоя объект рисуется поверх стен, с `ON_ROOF` — на крыше, с `ABOVE_ROOFS` — поверх крыш.
pub const ABOVE_WALLS: i16 = 700;
/// На крыше: дымоход, вытяжка, люк. Рисуется поверх крыши и тает вместе с ней; в Foundry
/// запекается в картинку своей крыши — игрок, вошедший в дом, его не видит.
pub const ON_ROOF: i16 = 800;
/// Над крышами, отдельно от них: дирижабль, птицы.
pub const ABOVE_ROOFS: i16 = 900;

/// Слой по имени: плоское — на пол, деревья — над остальным, подъёмный кран — над
/// всем: стрела выше голов и крыш.
pub fn layer_by_name(key: &str) -> i16 {
    let k = crate::library::label(key).to_lowercase();
    if flat_by_name(key) {
        100
    } else if k.starts_with("crane_") {
        ABOVE_ROOFS
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
    /// Магический свет: рунный кристалл, реактор, светящиеся руны — холодный, голубой.
    Arcane,
    /// Аварийная лампа станции: красный свет у стены, луч медленно ходит по кругу.
    Alarm,
    /// Электрический светильник станции, лаборатории, города будущего: холодный белый, горит
    /// ровно. Только по выбору: по имени не узнаётся (у киберпанка «Neon» в имени половины мебели).
    Neon,
}

impl Glow {
    pub const CHOICES: [Glow; 9] = [Glow::Off, Glow::Campfire, Glow::Fireplace, Glow::Brazier, Glow::Candle, Glow::Lamp, Glow::Arcane, Glow::Alarm, Glow::Neon];

    pub fn label(self) -> &'static str {
        match self {
            Glow::Off => "нет",
            Glow::Campfire => "костёр",
            Glow::Fireplace => "камин",
            Glow::Brazier => "жаровня",
            Glow::Candle => "свеча",
            Glow::Lamp => "фонарь",
            Glow::Arcane => "магия",
            Glow::Alarm => "тревога",
            Glow::Neon => "неон",
        }
    }

    /// Радиус яркого и тусклого в клетках, цвет и анимация Foundry (пусто — горит ровно).
    /// `None` — не светит.
    pub fn params(self) -> Option<(f32, f32, &'static str, &'static str)> {
        match self {
            Glow::Off => None,
            Glow::Campfire => Some((3.0, 6.0, "#ff9a3c", "flame")),
            Glow::Fireplace => Some((2.0, 5.0, "#ff8a2c", "flame")),
            Glow::Brazier => Some((2.0, 4.0, "#ffa040", "torch")),
            Glow::Candle => Some((1.0, 2.0, "#ffd9a0", "torch")),
            Glow::Lamp => Some((3.0, 6.0, "#ffc870", "torch")),
            // Магический светильник тише камина: бра и шары по стенам не заливают зал.
            Glow::Arcane => Some((1.5, 4.0, "#78b8ff", "energy")),
            // Аварийная лампа: красный луч медленно ходит по кругу — «вращающийся свет» Foundry.
            // «Сирена» для этого не годится: даже на скорости 1 она делает 1,6 оборота в секунду,
            // и десяток ламп рябит на весь зал.
            Glow::Alarm => Some((0.4, 2.2, "#ff3020", "revolving")),
            // Потолочный светильник станции: как магический по размеру, но белее и без
            // «энергии» — та кружит по полу и воде рябью.
            Glow::Neon => Some((1.5, 4.0, "#cfe8ff", "")),
        }
    }

    /// Как свет ложится в Foundry: плотность цвета, способ окраски (`coloration` источника
    /// света: 1 — обычная, 4 — «выжигание»), мягкость края (`attenuation`) и насколько свет
    /// высветляет пол (`luminosity`).
    pub fn look(self) -> (f32, u8, f32, f32) {
        match self {
            // Красный поверх светлого пола и голубой воды при обычной окраске (1) даёт розово-серую
            // дымку на всё пятно, «выжигание» (4) на радиусе в две клетки делает тёмные предметы
            // ядовито-красными. «Внутренний ореол» (2) держит красное у самой лампы и мягко тает;
            // пол свет не высветляет.
            Glow::Alarm => (0.45, 2, 0.7, 0.0),
            // Почти белый: цвет едва подкрашивает пол, чтобы десяток светильников не синил зал.
            Glow::Neon => (0.2, 1, 0.5, 0.5),
            _ => (0.35, 1, 0.5, 0.5),
        }
    }

    /// Скорость и сила анимации света в Foundry (1–10).
    pub fn tempo(self) -> (u8, u8) {
        match self {
            // Тревога: оборот луча за 4,5 с (скорость 7 — 1,4 рад/с), луч шириной 144° (сила 8).
            Glow::Alarm => (7, 8),
            _ => (3, 3),
        }
    }
}

/// Анимации источника света в Foundry 14 (`CONFIG.Canvas.lightAnimations`).
pub const LIGHT_ANIMATIONS: [&str; 23] = [
    "flame", "torch", "revolving", "siren", "pulse", "reactivepulse", "chroma", "wave", "fog", "sunburst", "dome", "emanation", "hexa", "ghost", "energy", "vortex", "witchwave",
    "rainbowswirl", "radialrainbow", "fairy", "grid", "starlight", "smokepatch",
];

/// Свой свет предмета поверх вида света ([`Glow`]): что задано — вместо того, что даёт вид.
/// Так одна лампа на одной карте светит по-своему, а вид света у всех карт остаётся прежним.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlowTune {
    /// Радиус тусклого света в клетках; яркий меняется в той же доле.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Анимация Foundry из [`LIGHT_ANIMATIONS`]; пустая строка — горит ровно.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    /// Скорость анимации, 0–10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u8>,
    /// Сила анимации, 1–10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intensity: Option<u8>,
}

impl GlowTune {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Свет предмета целиком — вид света и свои поправки: то, что уходит в Foundry и в ночной снимок.
#[derive(Clone, Debug, PartialEq)]
pub struct Shine {
    pub bright: f32,
    pub dim: f32,
    pub color: String,
    pub animation: String,
    pub look: (f32, u8, f32, f32),
    pub tempo: (u8, u8),
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
        // Зола, холодное кострище и вертел без огня не светят; тлеющие угли — слабо.
        // В паках FA костры и очаги парами: `Campfire_A` — сложенные дрова, `Campfire_A_Lit` —
        // огонь (так же `Firepit_Stone_…` и `…_Lit`): без `Lit` огня нет.
        let fa_pair = key.starts_with("pack:") && ["campfire", "firepit"].contains(&words.first().map_or("", |w| w.as_str()));
        if has(&["cold", "ash", "ashes", "unlit", "spit"]) {
            None
        } else if words.iter().any(|w| w.starts_with("ember")) {
            Some(Glow::Brazier)
        } else if has(&["lit"]) || !fa_pair {
            Some(Glow::Campfire)
        } else {
            None
        }
    } else if has(&["fireplace", "hearth", "камин", "очаг"]) || pair("fire", "place") || pair("forge", "lit") {
        Some(Glow::Fireplace)
    } else if has(&["brazier", "torch", "torches", "sconce", "жаровня", "факел"]) {
        Some(Glow::Brazier)
    } else if has(&["candle", "candles", "candlestick", "candelabra", "chandelier", "свеча", "свечи", "люстра"]) {
        Some(Glow::Candle)
    } else if has(&["alarm", "siren", "тревога", "сирена"]) {
        Some(Glow::Alarm)
    } else if has(&["lamp", "lamps", "lantern", "lanterns", "streetlight", "lamppost", "фонарь", "лампа"]) || pair("street", "light") {
        Some(Glow::Lamp)
    } else {
        None
    }
}

/// Крона по имени: встроенное дерево и кроны FA (`Tree_Tops`, ствол у них отдельно).
/// У прочих чужих деревьев листва может быть любой или не быть вовсе (сухие, зимние),
/// поэтому они остаются в картинке, пока крону не включат у объекта рукой.
/// Голое дерево (FA `Bare_Tree`): сквозь ветки и так видно — в выгрузке оно
/// тайлом над токенами без затухания, а в картинке под ним ничего, даже пня.
pub fn is_bare_tree(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("bare_tree") || lower.contains("tree_bare") || lower.ends_with("builtin:tree_bare")
}

/// Сила пола этажа `m` в точке (0–255): местность и пол комнат (в пределах комнат).
pub fn floor_at(m: &Map, p: [f32; 2]) -> u8 {
    let (i, j) = ((p[0] * crate::mask::RES as f32).floor() as isize, (p[1] * crate::mask::RES as f32).floor() as isize);
    let ground = m.terrain.iter().map(|c| c.mask.get(i, j)).max().unwrap_or(0);
    let room = m.room_floor.iter().map(|c| c.mask.get(i, j)).max().unwrap_or(0).min(m.rooms.get(i, j));
    ground.max(room)
}

/// Крыша этажа ниже (контур `ring`), больше чем наполовину накрытая полом этажей
/// выше (`above`): сверху её не видно — пол накрывает, а сквозь дыры в нём (световой
/// люк, решётка) видна комната под ней, а не крыша. Как в выгрузке (`export::Solid`).
pub fn under_floor(ring: &[[f32; 2]], above: &[&Map]) -> bool {
    if ring.len() < 3 || above.is_empty() {
        return false;
    }
    let (lo, hi) = ring.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(l, h), q| ([l[0].min(q[0]), l[1].min(q[1])], [h[0].max(q[0]), h[1].max(q[1])]));
    let inside = |x: f32, y: f32| {
        let mut c = false;
        for k in 0..ring.len() {
            let (a, b) = (ring[k], ring[(k + 1) % ring.len()]);
            if (a[1] > y) != (b[1] > y) && x < a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                c = !c;
            }
        }
        c
    };
    const STEP: f32 = 0.25;
    let (mut all, mut covered) = (0u32, 0u32);
    let mut y = lo[1] + STEP / 2.0;
    while y < hi[1] {
        let mut x = lo[0] + STEP / 2.0;
        while x < hi[0] {
            if inside(x, y) {
                all += 1;
                if above.iter().any(|m| floor_at(m, [x, y]) >= 191) {
                    covered += 1;
                }
            }
            x += STEP;
        }
        y += STEP;
    }
    all > 0 && covered * 2 > all
}

/// У кроны есть ствол — в картинке под её тайлом остаётся пень: деревья по имени и
/// из папки `trees`. Парус, полог и прочее, отмеченное кроной рукой, ствола не имеют.
pub fn has_trunk(key: &str) -> bool {
    let lower = key.to_lowercase();
    key == crate::library::TREE || lower.contains("tree") || lower.contains("дерев")
}

/// Куст: в Foundry уходит тайлом, который качает ветер «Wild Canvas», как крону.
pub fn bush_by_name(key: &str) -> bool {
    // Ключ своей папки — путь Windows с `\`: папку `bushes` иначе не узнать.
    // Встроенный набор `set:bushes/…` — та же папка.
    let lower = key.to_lowercase().replace('\\', "/").replacen(crate::library::SET, "/", 1);
    key == format!("{}bush", crate::library::BUILTIN) || (lower.contains("/bushes/") && !lower.contains("dead") && !lower.contains("shadow"))
}

/// Навес лавки: в Foundry — тайл над токенами, гаснет, когда под ним кто-то, и
/// прячет от дождя.
pub fn awning_by_name(key: &str) -> bool {
    key.starts_with(&format!("{}market_stall", crate::library::BUILTIN)) || key.contains("/Arranged_Tarps/")
}

/// Неоновая колонка киберпанка: в Foundry — свой тайл, «Wild Canvas» оживляет
/// её под музыку.
pub fn speaker_by_name(key: &str) -> bool {
    living_by_name(key).is_some()
}

/// Живой предмет набора и его пометка для «Wild Canvas» (ключ, значение):
/// колонка играет, у охладителя крутятся вентиляторы, у стоек мигают огни.
/// Картинка при выгрузке переименовывается, поэтому вид — в пометке.
pub fn living_by_name(key: &str) -> Option<(&'static str, String)> {
    let file = crate::library::label(key);
    if file.starts_with("Speaker_Stack") {
        return Some(("speaker", "on".into()));
    }
    // Ветроуказатель: «Wild Canvas» гнёт конус от шеста (шест слева на картинке).
    if file.starts_with("Windsock") {
        return Some(("sock", "1".into()));
    }
    ["Cooling_Unit_A", "Server_Rack_A", "Server_Rack_B", "Generator_Portable_A", "Robot_Arm_Industrial_A", "Floor_Fan_Industrial_A", "Drone_Delivery_A", "Treadmill_A", "Airship_Gnome_Brass_A", "Signal_Beacon_Arcane_A"].iter().find(|m| file.starts_with(**m)).map(|m| ("machine", (*m).to_owned()))
        // Свой механизм из `live.json` папки ассетов: раскладку «Wild Canvas» получит в пометке плитки.
        .or_else(|| live_layout(key).map(|(name, _)| ("machine", name)))
}

/// Свои живые механизмы — из файла `live.json` в папке ассетов: начало имени файла картинки
/// (как у встроенных) → раскладка для «Wild Canvas»: вентиляторы, лампы, огоньки в долях
/// картинки. Новый вентилятор или пульт оживает записью в файле, без выпуска программ:
/// раскладка уходит в пометку плитки, и модулю не нужно знать ассет по имени.
static LIVE_TABLE: std::sync::RwLock<Vec<(String, String)>> = std::sync::RwLock::new(Vec::new());

/// Заменить таблицу своих механизмов: пары «начало имени файла — раскладка (JSON)».
pub fn set_live_table(table: Vec<(String, String)>) {
    if let Ok(mut t) = LIVE_TABLE.write() {
        *t = table;
    }
}

/// Имена своих механизмов из таблицы — для ответа `reload_assets`.
pub fn live_names() -> Vec<String> {
    LIVE_TABLE.read().map(|t| t.iter().map(|(name, _)| name.clone()).collect()).unwrap_or_default()
}

/// Свой механизм по ключу ассета: имя из таблицы и раскладка (JSON). Из подходящих имён —
/// самое длинное: `Fan_Wall_B` главнее `Fan_`.
pub fn live_layout(key: &str) -> Option<(String, String)> {
    let file = crate::library::label(key);
    let t = LIVE_TABLE.read().ok()?;
    t.iter().filter(|(name, _)| file.starts_with(name.as_str())).max_by_key(|(name, _)| name.len()).cloned()
}

/// Разобрать `live.json`: объект «имя → раскладка-объект». Всё остальное пропускается:
/// испорченный файл не мешает открыть папку ассетов.
pub fn parse_live_table(text: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else { return Vec::new() };
    o.into_iter().filter(|(name, layout)| !name.is_empty() && layout.is_object()).map(|(name, layout)| (name, layout.to_string())).collect()
}

/// Дымоход, из которого идёт дым («Wild Canvas» пускает клубы над крышей): не
/// вытяжка-колпак, не заглушённый и не картинка самого дыма.
pub fn smokes_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    k.contains("chimney") && !["smoke", "hood", "plug"].iter().any(|w| k.contains(w))
}

/// Лестница, трап, люк: стоят в одном месте на соседних этажах — в Foundry между
/// этими уровнями переход. Тень спуска, перила и упавшая стремянка — не переход.
pub fn passage_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["stairs", "staircase", "ladder", "hatch", "trapdoor", "лестниц", "люк", "трап"].iter().any(|w| k.contains(w))
        && !["shade", "overlay", "railing", "fallen", "aframe", "bed_ladder", "broken"].iter().any(|w| k.contains(w))
}

/// Лестница или трап, по которым поднимаются: у края плоской крыши они
/// размыкают перила. Люк — не подъём: он в полу.
pub fn climb_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    passage_by_name(key) && !["hatch", "trapdoor", "люк"].iter().any(|w| k.contains(w))
}

/// Рамка предмета на карте в клетках с поворотом: по видимой части картинки
/// (`visible` — доли холста от середины, см. `library::visible_box`) или по всему холсту.
pub fn prop_rect(p: &Prop, visible: Option<[f32; 4]>) -> [f32; 4] {
    let v = visible.unwrap_or([-0.5, -0.5, 0.5, 0.5]);
    let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
    let (s, c) = p.rot.to_radians().sin_cos();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for (u, t) in [(v[0], v[1]), (v[2], v[1]), (v[2], v[3]), (v[0], v[3])] {
        let (x, y) = (u * w, t * h);
        let q = [p.pos[0] + x * c - y * s, p.pos[1] + x * s + y * c];
        lo = [lo[0].min(q[0]), lo[1].min(q[1])];
        hi = [hi[0].max(q[0]), hi[1].max(q[1])];
    }
    [lo[0], lo[1], hi[0], hi[1]]
}

/// Гамак: в Foundry «Wild Canvas» качает его, как качка судна, — крюки стоят, полотно ходит.
pub fn swing_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    k.contains("hammock") || k.contains("гамак")
}

/// Поворотный кран: в Foundry «Wild Canvas» медленно поворачивает его вокруг оси
/// туда, где стрела ни во что не упрётся. Ось — доли картинки (середина круга
/// поворотной платформы); стрела — вверх по картинке.
pub fn slew_by_name(key: &str) -> Option<[f32; 2]> {
    let file = crate::library::label(key);
    [("Crane_Steam_Gnome_Brass_A", [0.504, 0.779])].iter().find(|(m, _)| file.starts_with(*m)).map(|(_, p)| *p)
}

/// Что ездит по рельсам: вагонетка, вагон, паровоз и груз в них (`…_Fill`).
/// Крепь, фонарь, упор на пути стоят.
pub fn rides_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["cart", "wagon", "locomotive", "trolley", "вагон", "_fill"].iter().any(|w| k.contains(w)) && !k.starts_with("track_")
}

/// Валун, скала: стоя в воде, в Foundry он препятствие — течение обтекает его.
pub fn rock_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["boulder", "rock_", "rocks_"].iter().any(|w| k.starts_with(w))
}

/// Мост по имени файла: встроенный и библиотечные (`Log_Bridge_…`, `Bridge_Boardwalk_…`).
pub fn bridge_by_name(key: &str) -> bool {
    let file = key.rsplit(['/', '\\', '|']).next().unwrap_or(key).to_lowercase();
    file.split(|c: char| !c.is_alphanumeric()).any(|w| w == "bridge" || w == "мост")
}

/// Летучее судно: в Foundry «Wild Canvas» качает его в воздухе, под ним ползёт тень.
pub fn hover_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["airship", "zeppelin", "blimp", "dirigible", "balloon", "дирижабл", "аэростат"].iter().any(|w| k.contains(w))
}

/// Плавает на воде: лист бумаги, поплавок. На живой воде «Wild Canvas» слегка покачивает
/// его в Foundry; на сухом полу он лежит смирно.
pub fn afloat_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["paper", "float", "buoy", "бумаг", "плава", "поплав"].iter().any(|w| k.contains(w))
}

/// Паутина: в Foundry — свой тайл, его чуть покачивает модуль Картографа.
pub fn web_by_name(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("cobweb") || lower.contains("spider_web")
}

/// Сундук с сокровищем FA (закрытый): в Foundry открывается. Открытый с монетами и
/// пустой лежат рядом под тем же именем с хвостом `_Coins_Mixed` и `_Empty_A`.
/// Простой сундук FA (`Chest_Medium_Wood_Red_A1`) открывается в `…_Open_A1` —
/// без крышки, пустой. Встроенный — в свои открытый и пустой.
pub fn chest_variants(key: &str) -> Option<(String, String)> {
    let b = crate::library::BUILTIN;
    if let Some(id) = key.strip_prefix(b) {
        return (id == "chest").then(|| (format!("{b}chest_open"), format!("{b}chest_empty")));
    }
    let (dir, file) = key.rsplit_once('/')?;
    let (stem, ext) = file.rsplit_once('.')?;
    if file.starts_with("Treasure_Chest_") {
        if ["Coins", "Empty", "False_Bottom"].iter().any(|w| file.contains(w)) {
            return None;
        }
        let (base, size) = stem.rsplit_once('_')?;
        return Some((format!("{dir}/{base}_Coins_Mixed_{size}.{ext}"), format!("{dir}/{base}_Empty_A_{size}.{ext}")));
    }
    if file.starts_with("Chest_") && !stem.contains("_Open") {
        let (rest, size) = stem.rsplit_once('_')?;
        let (base, var) = rest.rsplit_once('_')?;
        let open = format!("{dir}/{base}_Open_{var}_{size}.{ext}");
        return Some((open.clone(), open));
    }
    None
}

/// Верёвка с бельём: в Foundry — тайл, который колышет ветер.
pub fn line_by_name(key: &str) -> bool {
    key.contains("Clothes_Line")
}

pub fn canopy_by_name(key: &str) -> bool {
    key == crate::library::TREE || key.contains("/Tree_Tops/")
}

/// Что предмет делает по имени своего файла. Имя переводится в нижний регистр и
/// сверяется с десятками слов — на карте в 10 000 предметов это сотни тысяч строк
/// за кадр (слой нужен и для порядка рисования). Поэтому один раз на ключ: ответ
/// зависит только от ключа.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameTraits {
    pub layer: i16,
    pub flat: bool,
    pub canopy: bool,
    pub bush: bool,
    pub awning: bool,
    pub line: bool,
    pub bridge: bool,
    pub living: bool,
    pub hover: bool,
    pub swing: bool,
    pub web: bool,
    pub stairs_shade: bool,
}

pub fn name_traits(key: &str) -> NameTraits {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static SEEN: RefCell<HashMap<Box<str>, NameTraits>> = RefCell::new(HashMap::new());
    }
    if let Some(t) = SEEN.with(|s| s.borrow().get(key).copied()) {
        return t;
    }
    let t = NameTraits {
        layer: layer_by_name(key),
        flat: flat_by_name(key),
        canopy: canopy_by_name(key),
        bush: bush_by_name(key),
        awning: awning_by_name(key),
        line: line_by_name(key),
        bridge: bridge_by_name(key),
        living: living_by_name(key).is_some(),
        hover: hover_by_name(key),
        swing: swing_by_name(key),
        web: web_by_name(key),
        stairs_shade: key.ends_with("stairs_shade"),
    };
    SEEN.with(|s| s.borrow_mut().insert(key.into(), t));
    t
}

impl Prop {
    /// Углы на карте: левый верхний, правый верхний, правый нижний, левый нижний —
    /// углы картинки. У отражённого левый край картинки справа: на те же углы она
    /// ложится зеркально.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let (hw, hh) = (self.size[0] * self.scale / 2.0, self.size[1] * self.scale / 2.0);
        let hw = if self.flip { -hw } else { hw };
        let (s, c) = self.rot.to_radians().sin_cos();
        [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|[x, y]| [self.pos[0] + x * c - y * s, self.pos[1] + x * s + y * c])
    }

    pub fn casts_shadow(&self) -> bool {
        // Предмет в воде тени не даёт: низ его под водой, а тень всей картинки легла бы на воду.
        self.shadow.unwrap_or_else(|| !name_traits(&self.asset).flat && self.sunk() <= 0.0)
    }

    /// Какая доля предмета под водой, от низа картинки: 0 — весь над водой, 1 — весь под ней.
    pub fn sunk(&self) -> f32 {
        self.sunk.filter(|v| v.is_finite()).map_or(0.0, |v| v.clamp(0.0, 1.0))
    }

    /// Чем светит в Foundry: выбранное вручную или узнанное по имени.
    pub fn light(&self) -> Option<Glow> {
        self.light.or_else(|| glow_by_name(&self.asset)).filter(|g| *g != Glow::Off)
    }

    /// Свет предмета: вид ([`Prop::light`]) с размером по масштабу и своими поправками (`glow`).
    pub fn shine(&self) -> Option<Shine> {
        let glow = self.light()?;
        let (bright, dim, color, animation) = glow.params()?;
        let k = self.scale.sqrt();
        let (mut bright, mut dim) = (bright * k, dim * k);
        let (mut color, mut animation, mut tempo) = (color.to_owned(), animation.to_owned(), glow.tempo());
        if let Some(t) = &self.glow {
            if let Some(r) = t.radius {
                let r = r.clamp(0.2, 30.0);
                bright *= r / dim;
                dim = r;
            }
            if let Some([r, g, b]) = t.color {
                color = format!("#{r:02x}{g:02x}{b:02x}");
            }
            if let Some(a) = &t.animation {
                animation = a.clone();
            }
            tempo = (t.speed.unwrap_or(tempo.0).min(10), t.intensity.unwrap_or(tempo.1).clamp(1, 10));
        }
        Some(Shine { bright, dim, color, animation, look: glow.look(), tempo })
    }

    /// Крона: в Foundry тайлом над токенами, в картинке пень.
    pub fn is_canopy(&self) -> bool {
        self.canopy.unwrap_or_else(|| name_traits(&self.asset).canopy)
    }

    /// В Foundry — своим тайлом в родном разрешении картинки, а не в фоне:
    /// в фоне масштаба сцены (часто 100 точек на клетку) предмет мылится.
    /// Кроме крон, кустов, навесов и белья (у них свой тайл) и плоского на полу
    /// (ковры, лужи, готовые тени — их полупрозрачность в фоне не мешает).
    pub fn own_tile(&self) -> bool {
        // Тень спуска лежит поверх ступеней: в фоне её накрыл бы тайл лестницы.
        let t = name_traits(&self.asset);
        let over_stairs = t.stairs_shade;
        // Мост и помост — пол над водой: на них стоят хижины, а стены в Foundry
        // впечатаны в фон — тайл моста их бы накрыл.
        // Вентилятор в полу плоский, но крутится, а «Wild Canvas» крутит только тайл:
        // живое плоское — своим тайлом, без тени.
        let alive = t.living;
        // Целиком под водой — лежит на дне, в картинке карты: вода «Wild Canvas» идёт поверх.
        // Наполовину в воде — тайлом идёт сухой верх, низ остаётся в фоне.
        let on_bottom = self.sunk() >= 1.0;
        !on_bottom && !self.is_canopy() && !t.bush && !t.awning && !t.line && !t.bridge && (!t.flat || over_stairs || alive)
    }

    /// Слой «над стенами» — колонны, столбики перил: в Foundry они в тайле стен
    /// этажа (поверх стен, над токенами), своего тайла у них нет.
    pub fn with_walls(&self) -> bool {
        (ABOVE_WALLS..ON_ROOF).contains(&self.layer())
    }

    /// Тень предмета — в картинке его же тайла, а не в фоне: Мастер сдвинет или
    /// уберёт тайл в Foundry — тень уйдёт с ним. Кроме того, что «Wild Canvas»
    /// двигает и размечает по долям тайла (парящее, гамак, машины, паутина): у них
    /// тень остаётся в фоне.
    pub fn carries_shadow(&self) -> bool {
        let t = name_traits(&self.asset);
        self.own_tile() && self.casts_shadow() && self.layer() < ABOVE_WALLS && !t.hover && !t.swing && !t.web && !t.living
    }

    /// Тень предмета в картинке пола. `tiles` — выгрузка с тайлами: там тень тайла рисует
    /// «Wild Canvas» по пометке, а крану — сам, вместе со стрелой. Запечённая в пол, тень
    /// крана стояла бы на месте, пока стрела поворачивается.
    pub fn floor_shadow(&self, tiles: bool) -> bool {
        self.casts_shadow() && !self.is_canopy() && !(tiles && (self.carries_shadow() || slew_by_name(&self.asset).is_some()))
    }

    pub fn layer(&self) -> i16 {
        // Целиком под водой — на дне, слоем пола: под всем остальным.
        self.layer.unwrap_or_else(|| if self.sunk() >= 1.0 { 100 } else { name_traits(&self.asset).layer })
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

/// Крыша по контуру дома из OSM: многоугольник с дворами (кольца в клетках).
/// Каждая точка ската сходит к ближайшей стороне контура — так у любого
/// многоугольника выходят вальмы и коньки; плоская — с парапетом. Прямоугольная
/// [`Roof`] на повёрнутые дома и дворы не ложится.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutlineRoof {
    /// Первый кусок — весь дом, дальше его части (`building:part`) снизу вверх:
    /// купола собора, шатры башен, своды пассажа — каждая своим цветом и видом.
    pub pieces: Vec<RoofPiece>,
}

/// Кусок крыши по контуру: кольца в клетках (внутренние — дворы), вид и покрытие.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoofPiece {
    pub rings: Vec<Vec<[f32; 2]>>,
    pub form: RoofForm,
    /// Картинка покрытия, бывает в цвете (`tint:`).
    pub paint: String,
    /// Верх куска над землёй, в клетках — по нему высшие куски кладут тень на
    /// низшие ([`crate::roofs::SUN_RISE`]).
    #[serde(default)]
    pub height: f32,
    /// Низ ската, в клетках (у плоской — верх). Сторона, за которой верх соседнего
    /// куска не ниже этого, — стык, а не край: без тёмной кромки и парапета, к ней
    /// нет ската. Грани одной вальмы в OSM разной высоты (25 и 23 м), но сходятся.
    #[serde(default)]
    pub eaves: f32,
    /// Куда смотрит односкатная ([`RoofForm::Shed`]): вниз по скату, в клетках.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<[f32; 2]>,
    /// Конёк двускатной, свода и рядов — поперёк длинной стороны (`roof:orientation=across`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub across: bool,
    /// Свес за стены, в клетках: тогда кольца — контур стен, а край крыши шире на
    /// свес ([`RoofPiece::edge_rings`]) — так у крыш, заданных руками. Пусто — кольца
    /// и есть край крыши: дома из OSM и карты до 0.2.30, где свес запекался в точки
    /// (и правка контура грозила удвоить его).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overhang: Option<f32>,
}

/// Как сходит кусок крыши.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoofForm {
    /// Ровная, с парапетом по краю.
    Flat,
    /// Скатами к каждой стороне; у широкого дома скат не длиннее
    /// [`crate::roofs::SLOPE_MAX`], выше — плоский верх.
    Pitched,
    /// Скатами до самой макушки: шатёр, шпиль, купол.
    Spire,
    /// Одна плоскость под уклон в сторону [`RoofPiece::down`] (`skillion`): в OSM
    /// так по граням собирают вальму — каждая грань своей частью.
    Shed,
    /// Два ската от конька вдоль длинной стороны, торцы — щипцы.
    Gabled,
    /// Свод: как двускатная, но свет по скату меняется плавно.
    Round,
    /// Купол: свет плавно по кругу от макушки, ряды покрытия — меридианами.
    /// Гранями, как шатёр, он выходил пёстрым зонтиком.
    Dome,
    /// Ряды двускатных бок о бок, коньки вдоль длинной стороны, между рядами
    /// ендовы: так крыты рынки, вокзалы, склады. Вальмой такой зал выходил
    /// ровным полем с каймой.
    Rows,
}

impl RoofForm {
    /// Порядок на панели: как у прямоугольных ([`RoofShape::ALL`]), потом своды,
    /// потом ряды.
    pub const ALL: [RoofForm; 8] = [RoofForm::Gabled, RoofForm::Pitched, RoofForm::Shed, RoofForm::Flat, RoofForm::Spire, RoofForm::Round, RoofForm::Dome, RoofForm::Rows];

    pub fn label(self) -> &'static str {
        match self {
            RoofForm::Gabled => "Двускатная",
            RoofForm::Pitched => "Вальмовая",
            RoofForm::Shed => "Односкатная",
            RoofForm::Flat => "Плоская",
            RoofForm::Spire => "Шатёр",
            RoofForm::Round => "Свод",
            RoofForm::Dome => "Купол",
            RoofForm::Rows => "Ряды скатов",
        }
    }

    /// Поворачивается ли клавишей R: конёк поперёк или скат в другую сторону.
    pub fn turns(self) -> bool {
        matches!(self, RoofForm::Gabled | RoofForm::Round | RoofForm::Shed | RoofForm::Rows)
    }
}

impl RoofPiece {
    pub fn contains(&self, p: [f32; 2]) -> bool {
        crate::roofs::inside_rings(&self.rings, p)
    }

    /// Край крыши: кольца со свесом — дом наружу, двор внутрь двора.
    pub fn edge_rings(&self) -> Vec<Vec<[f32; 2]>> {
        match self.overhang {
            Some(o) if o > 0.0 => self.rings.iter().enumerate().map(|(i, r)| grow_ring(r, if i == 0 { o } else { -o })).collect(),
            _ => self.rings.clone(),
        }
    }

    /// Односкатной без стороны ската — скат от самой длинной стороны внешнего
    /// кольца внутрь: иначе она рисуется вальмой.
    pub fn fit_form(&mut self) {
        if self.form != RoofForm::Shed || self.down.is_some() {
            return;
        }
        let Some(r) = self.rings.first().filter(|r| r.len() >= 3) else { return };
        let len = |k: usize| {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        };
        let k = (0..r.len()).max_by(|&x, &y| len(x).total_cmp(&len(y))).unwrap_or(0);
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        let l = len(k).max(1e-6);
        let mut n = [(a[1] - b[1]) / l, (b[0] - a[0]) / l];
        // К середине кольца, а не наружу.
        let c = r.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0] / r.len() as f32, s[1] + p[1] / r.len() as f32]);
        if (c[0] - a[0]) * n[0] + (c[1] - a[1]) * n[1] < 0.0 {
            n = [-n[0], -n[1]];
        }
        self.down = Some(n);
    }

    /// Клавиша R: у двускатной, свода и рядов коньки поперёк, у односкатной скат
    /// на четверть по часовой. Остальные не поворачиваются — `false`.
    pub fn turn(&mut self) -> bool {
        match self.form {
            RoofForm::Gabled | RoofForm::Round | RoofForm::Rows => self.across = !self.across,
            RoofForm::Shed => {
                self.fit_form();
                self.down = self.down.map(|d| [-d[1], d[0]]);
            }
            _ => return false,
        }
        true
    }
}

impl OutlineRoof {
    /// Точка под крышей: внутри первого куска — он весь дом.
    pub fn contains(&self, p: [f32; 2]) -> bool {
        self.pieces.first().is_some_and(|k| k.contains(p))
    }

    /// Левый верхний и правый нижний углы, в клетках.
    pub fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for k in &self.pieces {
            let o = k.overhang.unwrap_or(0.0).max(0.0);
            for p in k.rings.iter().flatten() {
                lo = [lo[0].min(p[0] - o), lo[1].min(p[1] - o)];
                hi = [hi[0].max(p[0] + o), hi[1].max(p[1] + o)];
            }
        }
        (lo, hi)
    }
}

/// Кольцо, раздвинутое наружу на `d` клеток по нормалям сторон (углы — по
/// биссектрисе, острый — не дальше трёх `d`); с минусом — внутрь.
pub fn grow_ring(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    let area: f32 = (0..n).map(|k| pts[k][0] * pts[(k + 1) % n][1] - pts[(k + 1) % n][0] * pts[k][1]).sum();
    // При площади больше нуля (по часовой на экране, y вниз) наружу — правая нормаль.
    let out = |a: [f32; 2], b: [f32; 2]| {
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let l = (ex * ex + ey * ey).sqrt().max(1e-6);
        if area > 0.0 { [ey / l, -ex / l] } else { [-ey / l, ex / l] }
    };
    (0..n)
        .map(|k| {
            let (p, prev, next) = (pts[k], pts[(k + n - 1) % n], pts[(k + 1) % n]);
            let (n1, n2) = (out(prev, p), out(p, next));
            let dot = n1[0] * n2[0] + n1[1] * n2[1];
            let mut m = [(n1[0] + n2[0]) / (1.0 + dot).max(1e-3), (n1[1] + n2[1]) / (1.0 + dot).max(1e-3)];
            let ml = (m[0] * m[0] + m[1] * m[1]).sqrt();
            if ml > 3.0 {
                m = [m[0] * 3.0 / ml, m[1] * 3.0 / ml];
            }
            [p[0] + m[0] * d, p[1] + m[1] * d]
        })
        .collect()
}

/// Стальной отлив по краю плоской крыши «Киберпанка».
pub const CYBER_PARAPET: &str = "set:cyberpunk/walls/Roof_Parapet_Steel_A.webp";

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
    /// Перила по краю плоской крыши — картинка стены (шканцы корабля, балкон).
    /// Нет — у покрытия «Киберпанка» стальной отлив, у прочих края нет.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail: Option<String>,
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
        Self { a: lo, b: hi, shape, paint, turn: 0, overhang: 0.25, rail: None }
    }

    /// Прямоугольник со свесом.
    pub(crate) fn outer(&self) -> ([f32; 2], [f32; 2]) {
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

    /// Парапет по краю плоской крыши: свои перила (`rail`), иначе у покрытия из
    /// набора «Киберпанк» — стальной отлив (полоса 2 × 0,25 клетки).
    pub fn parapet(&self) -> Option<&str> {
        if self.shape != RoofShape::Flat {
            return None;
        }
        match &self.rail {
            Some(k) => Some(k.as_str()),
            None => (crate::library::set_of(&self.paint) == Some("cyberpunk")).then_some(CYBER_PARAPET),
        }
    }

    /// Сливы и воронки крыши с парапетом: картинка, середина, поворот. Слив
    /// (1×1) — на длинных сторонах, на трети длины от разных концов, носиком
    /// наружу: полоса картинки 0,375..0,625 ложится на парапет, так что середина —
    /// в 0,125 клетки от края внутрь. Воронки — по средней линии, одна или две.
    pub fn fixtures(&self) -> Vec<(&'static str, [f32; 2], f32)> {
        const SCUPPER: &str = "set:cyberpunk/objects/Roof_Scupper_A_1x1.webp";
        const DRAIN: &str = "set:cyberpunk/objects/Roof_Drain_A_1x1.webp";
        if self.parapet() != Some(CYBER_PARAPET) {
            return Vec::new();
        }
        let (lo, hi) = self.outer();
        let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
        let mut out = Vec::new();
        let edge = 0.125;
        if w >= h && w >= 2.25 {
            out.push((SCUPPER, [lo[0] + w / 3.0, lo[1] + edge], 0.0));
            out.push((SCUPPER, [lo[0] + w * 2.0 / 3.0, hi[1] - edge], 180.0));
        } else if h > w && h >= 2.25 {
            out.push((SCUPPER, [hi[0] - edge, lo[1] + h / 3.0], 90.0));
            out.push((SCUPPER, [lo[0] + edge, lo[1] + h * 2.0 / 3.0], 270.0));
        }
        let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        if w.min(h) >= 2.0 {
            if w.max(h) >= 5.0 {
                let (a, b) = if w >= h { ([lo[0] + w / 3.0, c[1]], [lo[0] + w * 2.0 / 3.0, c[1]]) } else { ([c[0], lo[1] + h / 3.0], [c[0], lo[1] + h * 2.0 / 3.0]) };
                out.push((DRAIN, a, 0.0));
                out.push((DRAIN, b, 0.0));
            } else {
                out.push((DRAIN, c, 0.0));
            }
        }
        out
    }

    /// Конёк вдоль x: по длинной стороне, `turn` — поперёк.
    pub(crate) fn along_x(&self) -> bool {
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

/// Плоское по имени: лежит на полу и тени не даёт. Папоротник и кувшинки стелются
/// по земле и воде: в Foundry им место в картинке карты, а не сотней тайлов.
/// Кабели лежат на полу под мебелью, а не поверх пульта и кресла. Утопленное
/// (`Sunken_…`) лежит на дне: в картинке карты, и вода «Wild Canvas» идёт поверх него.
pub fn flat_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    ["rug", "carpet", "mat_", "drone_pad", "floor_fan", "manhole", "shadow", "puddle", "stain", "blood", "decal", "fern_", "lilypad", "cable", "sunken", "submerged", "ковёр", "ковер", "ковр", "тень", "лужа", "кровь", "пятно", "кабел", "утопл", "затонув"].iter().any(|w| k.contains(w))
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
    /// Темнота у подножия стен с обеих сторон — не от солнца, а от того, что
    /// в угол между полом и стеной свет не доходит. Под крышей тени от солнца
    /// нет, и без неё комнаты плоские.
    #[serde(default = "Shadows::ao_default")]
    pub ao: f32,
}

impl Default for Shadows {
    /// Карты, нарисованные до теней, открываются без них.
    fn default() -> Self {
        Self { on: false, angle: 45.0, dist: 0.2, blur: 0.3, opacity: 0.55, fade: 0.0, ao: Self::ao_default() }
    }
}

impl Shadows {
    fn ao_default() -> f32 {
        0.5
    }

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
    // Прямая из двух точек — тоже шагами: дорога тает к концам, и без точек
    // посередине она таяла целиком (дорогу в два щелчка не было видно).
    if let [a, b] = pts {
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let steps = ((len / 0.15).ceil() as usize).clamp(1, 400);
        return (0..=steps).map(|k| std::array::from_fn(|c| a[c] + (b[c] - a[c]) * k as f32 / steps as f32)).collect();
    }
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


fn still(v: &f32) -> bool {
    *v == 0.0
}

/// Участок текущей воды: многоугольник в клетках, скорость (клеток в секунду) и
/// куда течёт (градусы по часовой от «вправо»).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    pub points: Vec<[f32; 2]>,
    pub current: f32,
    pub drift: f32,
}

fn drift_down() -> f32 {
    90.0
}

fn at_start(o: &[i32; 2]) -> bool {
    *o == [0, 0]
}

fn is_down(v: &f32) -> bool {
    (*v - 90.0).abs() < 1e-3
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
    /// Сид, из которого карта сгенерирована; пусто — нарисована руками.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub seed: String,
    /// Частицы от крон в «Wild Canvas» (`leaves`, `fireflies`); пусто — без них.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub particles: String,
    /// Погода сцены Foundry (`snow`); пусто — без погоды.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub weather: String,
    /// Звук места в «Wild Canvas» (`forest-autumn`, `cave-1`…); пусто — без него.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ambience: String,
    /// Звук внутри комнат (`tavern`, `house`): при выгрузке комнаты обводятся областью
    /// «Wild Canvas», внутри неё играет он, снаружи — `ambience`. Пусто — без неё.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub indoor: String,
    /// Свет сцены Foundry (`day`, `dim`, `dark`, см. `plan::SCENE_LIGHTS`); пусто — по звуку
    /// места. Отдельно от звука: станция гудит как станция, а светло в ней или темно — выбор карты.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scene_light: String,
    /// Течение воды в «Wild Canvas», клеток в секунду: корабль на ходу стоит, а
    /// вода уходит мимо борта, за кормой след. 0 — вода стоит.
    #[serde(default, skip_serializing_if = "still")]
    pub current: f32,
    /// Куда течёт вода, градусы по часовой от «вправо»: 90 — вниз, корабль идёт вверх.
    #[serde(default = "drift_down", skip_serializing_if = "is_down")]
    pub drift: f32,
    /// Участки, где вода течёт иначе, чем по всей карте: канал бежит, а гавань стоит.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,
    /// Пути вагонеток, точки в клетках. Всё, что стоит на пути (кроме самих рельсов),
    /// в Foundry ездит по нему туда и обратно сцепкой. Первая точка — депо (рудник,
    /// тоннель): там вагонетка пропадает в темноте и ждёт.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rails: Vec<Vec<[f32; 2]>>,
    /// На сколько клеток карта выросла влево и вверх (`grow`) с тех пор, как её начали:
    /// всё нарисованное сдвинулось на столько же. Выгрузка отдаёт это модулю Foundry —
    /// обновляя сцену, он сдвигает поставленные там фишки вслед за картой.
    #[serde(default, skip_serializing_if = "at_start")]
    pub origin: [i32; 2],
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
    /// Крыши по контурам домов из OSM (см. [`OutlineRoof`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outline_roofs: Vec<OutlineRoof>,
    /// Помосты: площадки выше пола со стенкой, тенью и лестницами (см. [`crate::platform`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<crate::platform::Platform>,
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
    /// Высота земли; нет — карта ровная.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relief: Option<crate::relief::Relief>,
    /// Образец: картинка, по которой обводят карту своими кистями. Только в
    /// редакторе — в выгрузку и на снимки не идёт.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Trace>,
    /// Эскиз: картинка в окошке сбоку, для сверки.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sketch: String,
}

/// Образец под обводку ([`Map::trace`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    /// Файл картинки.
    pub path: String,
    /// Левый верхний угол, клетки.
    pub pos: [f32; 2],
    /// Ширина, клетки; высота — по сторонам картинки.
    pub width: f32,
    /// Непрозрачность, 0–1.
    pub opacity: f32,
    /// Поверх нарисованного; иначе — под ним, видно только в пустоте.
    #[serde(default = "yes")]
    pub on_top: bool,
    #[serde(default = "yes")]
    pub shown: bool,
}

fn yes() -> bool {
    true
}

impl Trace {
    /// Новый образец во всю ширину карты.
    pub fn new(path: String, map_width: u32) -> Self {
        Self { path, pos: [0.0, 0.0], width: map_width as f32, opacity: 0.5, on_top: true, shown: true }
    }
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
            coats.push(Coat { material, mask: template.blank_like() });
            coats.len() - 1
        }
    };
    for &(i, v) in weights {
        for (n, c) in coats.iter_mut().enumerate() {
            let x = c.mask.at(i) as f32;
            c.mask.put(i, if n == k { (x + v * (255.0 - x)).round() as u8 } else { (x * (1.0 - v)).round() as u8 });
        }
    }
}

fn fade(mask: &mut Mask, weights: &[(usize, f32)]) {
    for &(i, v) in weights {
        mask.put(i, (mask.at(i) as f32 * (1.0 - v)).round() as u8);
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
            seed: String::new(),
            particles: String::new(),
            weather: String::new(),
            ambience: String::new(),
            indoor: String::new(),
            scene_light: String::new(),
            current: 0.0,
            drift: drift_down(),
            flows: Vec::new(),
            rails: Vec::new(),
            origin: [0, 0],
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
            relief: None,
            trace: None,
            sketch: String::new(),
            labels: Vec::new(),
            roofs: Vec::new(),
            outline_roofs: Vec::new(),
            platforms: Vec::new(),
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
        let template = self.rooms.blank_like();
        match layer {
            Layer::Floor => lay(&mut self.terrain, m, &template, weights),
            Layer::Rooms => {
                lay(&mut self.room_floor, m, &template, weights);
                for &(i, v) in weights {
                    let x = self.rooms.at(i) as f32;
                    self.rooms.put(i, (x + v * (255.0 - x)).round() as u8);
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

    /// Стереть один материал местности (кисть пола с Alt): слабеет только его слой.
    pub fn fade_material(&mut self, material: usize, weights: &[(usize, f32)]) {
        if let Some(c) = self.terrain.iter_mut().find(|c| c.material as usize == material) {
            fade(&mut c.mask, weights);
        }
    }

    /// Стереть комнаты (кисть подземелья с Alt): комнаты и их пол, местность цела.
    pub fn erase_rooms(&mut self, weights: &[(usize, f32)]) {
        for c in self.room_floor.iter_mut() {
            fade(&mut c.mask, weights);
        }
        fade(&mut self.rooms, weights);
    }

    /// Тень кистью: точка темнеет до `level` × сила отпечатка, но не светлеет —
    /// ступени 20/40/60% ложатся ровно, сколько ни води.
    pub fn shade_paint(&mut self, weights: &[(usize, f32)], level: f32) {
        let template = &self.rooms;
        let m = self.shade.get_or_insert_with(|| template.blank_like());
        for &(i, v) in weights {
            let t = (level * v * 255.0).round().clamp(0.0, 255.0) as u8;
            m.put(i, m.at(i).max(t));
        }
    }

    pub fn shade_fade(&mut self, weights: &[(usize, f32)]) {
        if let Some(m) = &mut self.shade {
            fade(m, weights);
        }
    }

    /// Выбросить опустевшие слои — после мазка. Куски масок, ставшие одной силы
    /// (залитые целиком, стёртые), — снова без памяти под точки.
    pub fn prune(&mut self) {
        self.compact();
        if self.shade.as_ref().is_some_and(|m| m.is_empty()) {
            self.shade = None;
        }
        if self.relief.as_ref().is_some_and(|r| r.is_flat()) {
            self.relief = None;
        }
        if let Some(r) = &mut self.relief {
            r.quantize();
        }
        self.terrain.retain(|c| !c.mask.is_empty());
        self.room_floor.retain(|c| !c.mask.is_empty());
    }

    /// Сжать правленые куски всех масок этажа (см. [`Mask::compact`]).
    pub fn compact(&mut self) {
        for c in self.terrain.iter_mut().chain(self.room_floor.iter_mut()) {
            c.mask.compact();
        }
        self.rooms.compact();
        if let Some(s) = &mut self.shade {
            s.compact();
        }
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
        if let Some(i) = self.materials.iter().position(|x| x.paint == m.paint && x.rot == m.rot) {
            return Some(i);
        }
        if self.materials.len() >= 255 {
            return None;
        }
        self.materials.push(m);
        Some(self.materials.len() - 1)
    }

    /// Тот же материал, что `m`, но оживающий как `live`: свой номер, если `live` другой
    /// (`material_index` сводит материалы только по картинке и повороту).
    pub fn material_with_live(&mut self, m: usize, live: Live) -> Option<usize> {
        let base = self.materials.get(m)?.clone();
        if base.live == live {
            return Some(m);
        }
        if let Some(i) = self.materials.iter().position(|x| x.paint == base.paint && x.rot == base.rot && x.live == live) {
            return Some(i);
        }
        if self.materials.len() >= 255 {
            return None;
        }
        self.materials.push(Material { live, ..base });
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
    /// Материал, что виден сверху в точке (номер): пол комнаты, иначе местность.
    pub fn material_under(&self, p: [f32; 2]) -> Option<usize> {
        let (i, j) = ((p[0] * mask::RES as f32).floor() as isize, (p[1] * mask::RES as f32).floor() as isize);
        let room = if self.rooms.get(i, j) >= 128 { dominant(&self.room_floor, i, j) } else { 0 };
        let m = if room != 0 { room } else { dominant(&self.terrain, i, j) };
        (m != 0).then(|| m as usize - 1)
    }

    /// В точке сверху живая вода (та, что оживает водой «Wild Canvas»).
    pub fn on_water(&self, p: [f32; 2]) -> bool {
        self.material_under(p).and_then(|m| self.materials.get(m)).is_some_and(|m| m.live().effect() == Some("water"))
    }

    pub fn material_mask(&self, m: usize) -> Mask {
        let mut out = self.rooms.blank_like();
        let (w, h) = (out.w, out.h);
        let pick = |mask: &Mask, rooms: Option<&Mask>, out: &mut Mask| {
            for j in 0..h {
                for i in 0..w {
                    // Пустой кусок слоя ничего не добавит.
                    if mask.uniform_at(i, j) == Some(0) {
                        continue;
                    }
                    let v = mask.get(i as isize, j as isize);
                    let v = rooms.map_or(v, |r| v.min(r.get(i as isize, j as isize)));
                    let o = out.get(i as isize, j as isize);
                    if v > o {
                        out.set(i, j, v);
                    }
                }
            }
        };
        for c in self.terrain.iter().filter(|c| c.material as usize == m) {
            pick(&c.mask, None, &mut out);
        }
        for c in self.room_floor.iter().filter(|c| c.material as usize == m) {
            pick(&c.mask, Some(&self.rooms), &mut out);
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
                            out.set(i, j, 255);
                        }
                    }
                }
            }
        }
        out.compact();
        out
    }

    /// Новый пустой этаж того же размера, с той же палитрой и видом стен.
    /// Карта шире: поля по краям в клетках; всё нарисованное сдвигается на
    /// `left`, `top`. Этажи — так же: размер у них общий. Так к готовой карте
    /// пристраивают окрестности: корабль — к причалу, дом — к улице.
    pub fn grow(&mut self, left: u32, top: u32, right: u32, bottom: u32) {
        let (w, h) = (self.width + left + right, self.height + top + bottom);
        let d = [left as f32, top as f32];
        let mv = move |p: &mut [f32; 2]| {
            p[0] += d[0];
            p[1] += d[1];
        };
        self.rooms = self.rooms.padded(w, h, left, top);
        for c in self.terrain.iter_mut().chain(self.room_floor.iter_mut()) {
            c.mask = c.mask.padded(w, h, left, top);
        }
        if let Some(s) = &mut self.shade {
            *s = s.padded(w, h, left, top);
        }
        if let Some(r) = &mut self.relief {
            *r = r.padded(w, h, left, top);
        }
        if let Some(t) = &mut self.trace {
            mv(&mut t.pos);
        }
        for wall in &mut self.walls {
            wall.points.iter_mut().for_each(mv);
        }
        for p in &mut self.portals {
            mv(&mut p.a);
            mv(&mut p.b);
        }
        for p in &mut self.props {
            mv(&mut p.pos);
        }
        for p in &mut self.paths {
            p.points.iter_mut().for_each(mv);
        }
        for f in &mut self.flows {
            f.points.iter_mut().for_each(mv);
        }
        self.rails.iter_mut().flatten().for_each(mv);
        for l in &mut self.labels {
            mv(&mut l.pos);
        }
        for r in &mut self.roofs {
            mv(&mut r.a);
            mv(&mut r.b);
        }
        for r in &mut self.outline_roofs {
            for piece in &mut r.pieces {
                piece.rings.iter_mut().flatten().for_each(mv);
            }
        }
        for p in &mut self.platforms {
            p.points.iter_mut().for_each(mv);
            p.stairs.iter_mut().for_each(|s| mv(&mut s.at));
        }
        // Ключ отрезка — середина с шагом 1/64 клетки.
        let (kx, ky) = (64 * left as i32, 64 * top as i32);
        self.edge_looks = std::mem::take(&mut self.edge_looks).into_iter().map(|((x, y), v)| ((x + kx, y + ky), v)).collect();
        self.width = w;
        self.height = h;
        self.origin[0] += left as i32;
        self.origin[1] += top as i32;
        for l in &mut self.levels {
            l.grow(left, top, right, bottom);
        }
    }

    /// Район карты отдельной картой: `w`×`h` клеток от клетки `x0, y0`, всё нарисованное
    /// сдвинуто на `-x0, -y0`. Что целиком вне района, не берётся; что задевает край, —
    /// целиком, как стоит: стена, крыша, дерево у края не режутся. Этажей у района нет —
    /// каждый этаж режется сам. Для выгрузки района отдельной сценой.
    pub fn crop(&self, x0: u32, y0: u32, w: u32, h: u32) -> Map {
        use crate::grid;
        let d = [-(x0 as f32), -(y0 as f32)];
        let mv = move |p: &mut [f32; 2]| {
            p[0] += d[0];
            p[1] += d[1];
        };
        let area = [0.0, 0.0, w as f32, h as f32];
        let near = |b: grid::Rect| grid::touch(&b, &area);
        let mut m = self.clone();
        m.levels = Vec::new();
        m.rooms = self.rooms.cropped(w, h, x0, y0);
        for c in m.terrain.iter_mut().chain(m.room_floor.iter_mut()) {
            c.mask = c.mask.cropped(w, h, x0, y0);
        }
        m.shade = self.shade.as_ref().map(|s| s.cropped(w, h, x0, y0));
        m.relief = self.relief.as_ref().map(|r| r.cropped(w, h, x0, y0));
        if let Some(t) = &mut m.trace {
            mv(&mut t.pos);
        }
        m.walls.retain_mut(|wall| {
            wall.points.iter_mut().for_each(mv);
            near(grid::grow(grid::bounds(&wall.points), 1.0))
        });
        m.portals.retain_mut(|p| {
            mv(&mut p.a);
            mv(&mut p.b);
            near(grid::bounds(&[p.a, p.b]))
        });
        m.props.retain_mut(|p| {
            mv(&mut p.pos);
            near(grid::bounds(&p.corners()))
        });
        m.paths.retain_mut(|p| {
            p.points.iter_mut().for_each(mv);
            near(grid::grow(grid::bounds(&p.points), p.width.abs()))
        });
        for f in &mut m.flows {
            f.points.iter_mut().for_each(mv);
        }
        m.rails.iter_mut().flatten().for_each(mv);
        m.labels.retain_mut(|l| {
            mv(&mut l.pos);
            near([l.pos[0], l.pos[1], l.pos[0], l.pos[1]])
        });
        m.roofs.retain_mut(|r| {
            mv(&mut r.a);
            mv(&mut r.b);
            near(grid::bounds(&r.outline()))
        });
        m.outline_roofs.retain_mut(|r| {
            for piece in &mut r.pieces {
                piece.rings.iter_mut().flatten().for_each(mv);
            }
            let (lo, hi) = r.bounds();
            near([lo[0], lo[1], hi[0], hi[1]])
        });
        m.platforms.retain_mut(|p| {
            p.points.iter_mut().for_each(mv);
            p.stairs.iter_mut().for_each(|s| mv(&mut s.at));
            let (lo, hi) = p.bounds();
            near([lo[0], lo[1], hi[0], hi[1]])
        });
        // Ключ отрезка — середина с шагом 1/64 клетки.
        let (kx, ky) = (64 * x0 as i32, 64 * y0 as i32);
        let (kw, kh) = (64 * w as i32, 64 * h as i32);
        m.edge_looks = self.edge_looks.iter().map(|(&(x, y), &v)| ((x - kx, y - ky), v)).filter(|((x, y), _)| (-64..=kw + 64).contains(x) && (-64..=kh + 64).contains(y)).collect();
        m.width = w;
        m.height = h;
        m.origin = [self.origin[0] - x0 as i32, self.origin[1] - y0 as i32];
        m
    }

    pub fn new_level(&self) -> Map {
        let mut m = Map::new(self.width, self.height);
        m.materials = self.materials.clone();
        m.wall_style = self.wall_style.clone();
        m.shadows = self.shadows.clone();
        m.origin = self.origin;
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
        let mut map: Map = if format == 1 { Self::from_v1(serde_json::from_value(value).map_err(|e| format!("файл не читается как карта: {e}"))?)? } else { serde_json::from_value(value).map_err(|e| format!("файл не читается как карта: {e}"))? };
        // Прочитанная карта в памяти — уже нынешнего устройства: сохранится она нынешним
        // форматом, и старая программа скажет, что файл новее, а не споткнётся о маски.
        map.format = FORMAT;
        for l in &mut map.levels {
            l.format = FORMAT;
        }
        let (w, h) = (map.width as usize * mask::RES, map.height as usize * mask::RES);
        let fits = |m: &Mask| m.w == w && m.h == h;
        if map.width == 0 || map.height == 0 || !fits(&map.rooms) || map.shade.as_ref().is_some_and(|m| !fits(m)) || map.relief.as_ref().is_some_and(|r| r.w != map.rooms.w || r.h != map.rooms.h) || !map.terrain.iter().chain(&map.room_floor).all(|c| fits(&c.mask)) {
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
    fn airfield_things_come_alive_by_name() {
        // Дым — из трубы, не из колпака-вытяжки, не из заглушки и не из картинки дыма.
        assert!(smokes_by_name("objects/Chimney_Stone_Earthy_A_1x1.webp") && smokes_by_name("x/Chimney_Metal_Brass_Square_A1_1x1.webp"));
        assert!(!smokes_by_name("x/Chimney_Metal_Brass_Hood_Large_A1_2x1.webp") && !smokes_by_name("x/Chimney_Smoke_A_2x2.webp"));
        assert!(!smokes_by_name("x/Chimney_Metal_Brass_Plug_Small_A1_1x1.webp"));
        // Ветроуказатель качается, маяк искрит; дирижабль — по-прежнему машина.
        assert_eq!(living_by_name("objects/steampunk/Windsock_Striped_A_3x1.webp"), Some(("sock", "1".to_owned())));
        assert_eq!(living_by_name("objects/steampunk/Signal_Beacon_Arcane_A_2x2.webp"), Some(("machine", "Signal_Beacon_Arcane_A".to_owned())));
        assert_eq!(living_by_name("objects/steampunk/Airship_Gnome_Brass_A_5x10.webp").map(|l| l.0), Some("machine"));
        // Свой механизм — из `live.json` папки ассетов: оживает без правки программы, плоский идёт плиткой.
        let fan = "objects/scifi/Vent_Fan_Probe_Zeta_B_2x2.webp";
        assert_eq!(living_by_name(fan), None);
        let table = parse_live_table(r#"{"Vent_Fan_Probe_Zeta": {"fans": [[0.5, 0.5, 0.4, 2]]}, "Vent_Fan_Probe_Zeta_B": {"fans": [[0.5, 0.5, 0.3]]}, "плохая": 5, "": {}}"#);
        assert_eq!(table.len(), 2, "раскладка — только объект с именем: {table:?}");
        set_live_table(table);
        assert_eq!(living_by_name(fan), Some(("machine", "Vent_Fan_Probe_Zeta_B".to_owned())), "из двух имён — длинное");
        assert_eq!(live_layout(fan).map(|l| l.1).as_deref(), Some(r#"{"fans":[[0.5,0.5,0.3]]}"#));
        assert_eq!(live_layout("objects/scifi/Vent_Fan_Probe_Zeta_A_1x1.webp").map(|l| l.0).as_deref(), Some("Vent_Fan_Probe_Zeta"));
        assert!(live_names().contains(&"Vent_Fan_Probe_Zeta".to_owned()));
        assert_eq!(living_by_name("objects/scifi/Cooling_Unit_A_2x1.webp"), Some(("machine", "Cooling_Unit_A".to_owned())), "встроенные — как были");
        assert!(parse_live_table("не json").is_empty() && parse_live_table("[1, 2]").is_empty());
        set_live_table(Vec::new());
        assert_eq!(living_by_name(fan), None);
    }

    #[test]
    fn grown_map_keeps_everything_in_place_relative_to_each_other() {
        // Палуба 4×3 с комнатой, стеной, дверью, предметом и крышей; поле слева 2, сверху 1.
        let mut m = Map::new(4, 3);
        m.rooms = m.rooms.padded(4, 3, 0, 0);
        m.rooms.set(crate::mask::RES + 1, crate::mask::RES * 1 + 1, 255);
        m.walls.push(Wall { points: vec![[0.0, 0.0], [4.0, 0.0]], closed: false, bends: Vec::new(), low: false, style: None });
        m.portals.push(Portal { a: [1.0, 3.0], b: [2.0, 3.0], kind: PortalKind::Door, asset: None, frame: None });
        m.props.push(Prop { asset: "x".into(), pos: [3.5, 1.5], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false });
        m.roofs.push(Roof::new([1.0, 1.0], [3.0, 2.0], RoofShape::Flat, "r".into()));
        m.edge_looks.insert((64, 0), 1);
        let mut deck = m.new_level();
        deck.props.push(m.props[0].clone());
        m.levels.push(deck);
        m.grow(2, 1, 3, 0);
        assert_eq!((m.width, m.height), (9, 4));
        assert_eq!((m.rooms.w, m.rooms.h), (9 * crate::mask::RES, 4 * crate::mask::RES));
        let r = crate::mask::RES;
        assert_eq!(m.rooms.at((r * 2 + 1) * m.rooms.w + r * 3 + 1), 255, "комната на клетку ниже и на две правее");
        assert_eq!(m.walls[0].points, vec![[2.0, 1.0], [6.0, 1.0]]);
        assert_eq!((m.portals[0].a, m.portals[0].b), ([3.0, 4.0], [4.0, 4.0]));
        assert_eq!(m.props[0].pos, [5.5, 2.5]);
        assert_eq!((m.roofs[0].a, m.roofs[0].b), ([3.0, 2.0], [5.0, 3.0]));
        assert_eq!(m.edge_looks.get(&(64 + 128, 64)), Some(&1));
        assert_eq!((m.levels[0].width, m.levels[0].height, m.levels[0].props[0].pos), (9, 4, [5.5, 2.5]));
        // Сдвиг копится и есть у каждого этажа: по нему модуль Foundry двигает фишки.
        assert_eq!((m.origin, m.levels[0].origin), ([2, 1], [2, 1]));
        m.grow(0, 5, 0, 0);
        assert_eq!((m.origin, m.new_level().origin), ([2, 6], [2, 6]));
        // В файле — только если карта росла; старый файл без поля читается с нулём.
        let fresh = serde_json::to_value(Map::new(2, 2)).unwrap();
        assert!(fresh.get("origin").is_none());
        let mut old = serde_json::to_value(&m).unwrap();
        assert_eq!(old["origin"], serde_json::json!([2, 6]));
        old.as_object_mut().unwrap().remove("origin");
        assert_eq!(serde_json::from_value::<Map>(old).unwrap().origin, [0, 0]);
    }

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
        assert_eq!(m.shade.as_ref().unwrap().at(0), 102);
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
    fn paper_floats_on_live_water_and_floor_fan_gets_a_tile() {
        let mut m = Map::new(10, 10);
        let w = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        m.paint(Layer::Floor, w, &crate::mask::cells(m.rooms.w, m.rooms.h, (2, 2), (7, 7)));
        assert!(m.on_water([4.5, 4.5]) && !m.on_water([8.5, 8.5]));
        assert!(afloat_by_name("pack:fa|textures/objects/Paper_Sheet_A_White_1x1.webp") && !afloat_by_name("pack:fa|textures/objects/Barrel_Wood_Dark_A_1x1.webp"));
        // Вентилятор в полу плоский, но живой: своим тайлом (иначе «Wild Canvas» нечего
        // крутить), без своей тени. Ковёр — плоский и неживой: остаётся в картинке карты.
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [3.0, 3.0], size: [2.0, 2.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        let fan = prop("set:cyberpunk/objects/Floor_Fan_Industrial_A_2x2.webp");
        assert!(flat_by_name(&fan.asset) && fan.own_tile() && !fan.carries_shadow());
        assert!(!prop("pack:fa|textures/objects/Rug_Rectangle_C1_Colorable_3x2.webp").own_tile());
        // Тень в полу: в окне — у всех; в выгрузке с тайлами бочка несёт её сама, а тень крана
        // «Wild Canvas» поворачивает со стрелой — в полу её нет.
        let barrel = prop("pack:fa|textures/objects/Barrel_Wood_Dark_A_1x1.webp");
        let crane = prop("C:/a/objects/steampunk/Crane_Steam_Gnome_Brass_A_4x9.webp");
        assert_eq!((barrel.floor_shadow(false), barrel.floor_shadow(true)), (true, false));
        assert_eq!((crane.floor_shadow(false), crane.floor_shadow(true)), (true, false));
        assert!(!crane.carries_shadow(), "кран над крышами — своей тени в тайле у него нет");
    }

    #[test]
    fn bridges_are_baked_under_walls_not_own_tiles() {
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [3.0, 3.0], size: [3.0, 4.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        assert!(!prop("C:/a/objects/bridges/Bridge_Boardwalk_Platform_A_5x4.webp").own_tile());
        assert!(!prop("pack:fa|textures/objects/Log_Bridge_Wood_Dark_A1_11x4.webp").own_tile());
        // Мост тень бросает, как раньше.
        assert!(prop("C:/a/objects/bridges/Bridge_Rope_Straight_A_2x4.webp").casts_shadow());
        assert!(prop("pack:fa|textures/objects/Barrel_Wood_Dark_A_1x1.webp").own_tile());
        assert!(!bridge_by_name("pack:fa|textures/objects/Cambridge_Sign_A_1x1.webp"));
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
                let above = m.terrain[k + 1..].iter().map(|c| c.mask.at(i) as u32).sum();
                background *= 1.0 - coat_alpha(c.mask.at(i), above) as f32 / 255.0;
            }
            assert!(background < 0.02, "в точке {i} просвечивает фон: {background:.2}");
        }
    }

    #[test]
    fn closed_wall_has_a_segment_back_to_the_start() {
        let w = Wall { points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]], closed: true, bends: Vec::new(), low: false, style: None };
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

    /// Дорога из двух точек: линия с точками посередине, иначе она тает целиком.
    #[test]
    fn two_point_road_has_a_middle() {
        let line = smooth_line(&[[28.0, 27.6], [28.0, 31.4]]);
        assert!(line.len() > 10, "{}", line.len());
        assert_eq!(line[0], [28.0, 27.6]);
        assert!((line[line.len() - 1][1] - 31.4).abs() < 1e-4);
    }

    #[test]
    fn lights_and_canopies_by_name_and_by_hand() {
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [0.0, 0.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        assert_eq!(prop("builtin:campfire").light(), Some(Glow::Campfire));
        assert_eq!(prop("builtin:street_lamp").light(), Some(Glow::Lamp));
        assert_eq!(prop("pack:fa.dungeondraft_pack|textures/objects/Fire_Pit_Stone_A_1x1.webp").light(), Some(Glow::Campfire));
        // Костры FA парами: без `Lit` — дрова или зола, огня нет; угли тлеют слабо.
        let fa = |n: &str| prop(&format!("pack:fa|textures/objects/{n}.webp")).light();
        assert_eq!(fa("Campfire_A_1x1"), None);
        assert_eq!(fa("Campfire_A_Lit_1x1"), Some(Glow::Campfire));
        assert_eq!(fa("Campfire_A_Cold_1x1"), None);
        assert_eq!(fa("Campfire_Ash_A1_1x1"), None);
        assert_eq!(fa("Campfire_A_Embers1_1x1"), Some(Glow::Brazier));
        assert_eq!(fa("Campfire_Spit_Pig_01_A1_1x2"), None);
        assert_eq!(fa("Firepit_Stone_Earthy_A1_1x1"), None);
        assert_eq!(fa("Firepit_Stone_Earthy_A1_Lit_1x1"), Some(Glow::Campfire));
        // Свой костёр без пары горит, как раньше.
        assert_eq!(prop("C:\\assets\\objects\\Campfire_Big_A_2x2.webp").light(), Some(Glow::Campfire));
        assert_eq!(prop("C:\\assets\\objects\\Chandelier_Iron_2x2.webp").light(), Some(Glow::Candle));
        assert_eq!(prop("pack:x|textures/objects/Lamp_Post_A.webp").light(), Some(Glow::Lamp));
        assert_eq!(prop("pack:x|textures/objects/Barrel_A.webp").light(), None);
        // Горн FA горит, холодный — нет; магический свет — голубой.
        assert_eq!(prop("pack:fa|textures/objects/Forge_Lit_Stone_Earthy_Large_Square_A1_3x3.webp").light(), Some(Glow::Fireplace));
        assert_eq!(prop("pack:fa|textures/objects/Forge_Stone_Earthy_Large_Square_A1_3x3.webp").light(), None);
        assert_eq!(Glow::Arcane.params().map(|p| p.2), Some("#78b8ff"));
        // Пол киберпанка без неона — свой материал рядом с неоновым, повторно — тот же.
        let mut m = Map::new(4, 4);
        let grid = m.material_index(Material { name: "Floor_Alloy_Grid_A".into(), paint: Paint::Texture("set:cyberpunk/floors/Floor_Alloy_Grid_A.webp".into()), vary: false, live: Live::Auto, rot: 0.0 }).unwrap();
        assert_eq!(m.materials[grid].live(), Live::Neon);
        let still = m.material_with_live(grid, Live::Off).unwrap();
        assert_ne!(still, grid);
        assert_eq!((m.materials[still].live(), m.materials[grid].live()), (Live::Off, Live::Neon));
        assert_eq!(m.material_with_live(grid, Live::Off), Some(still));
        assert_eq!(m.material_with_live(grid, Live::Auto), Some(grid));
        // Аварийная лампа станции — по имени, красная, луч медленно ходит по кругу.
        assert_eq!(glow_by_name("set:scifi/objects/Alarm_Light_Red_A_1x1.webp"), Some(Glow::Alarm));
        // «Вращающийся свет» Foundry, а не сирена; красит ореолом у лампы. У остальных — как было.
        assert_eq!(Glow::Alarm.params().map(|p| (p.2, p.3)), Some(("#ff3020", "revolving")));
        assert_eq!((Glow::Alarm.tempo(), Glow::Alarm.look().1), ((7, 8), 2));
        assert_eq!(Glow::Lamp.tempo(), (3, 3));
        assert_eq!(Glow::Lamp.look(), (0.35, 1, 0.5, 0.5));
        // Светильник станции: холодный, ровный, только по выбору — «Neon» в имени мебели не свет.
        assert_eq!(Glow::Neon.params(), Some((1.5, 4.0, "#cfe8ff", "")));
        assert!(Glow::CHOICES.contains(&Glow::Neon));
        assert_eq!(glow_by_name("set:cyberpunk/objects/Armchair_Neon_A_1x1.webp"), None);
        assert_eq!(glow_by_name("set:cyberpunk/objects/Neon_Lamp_Floor_A_1x1.webp"), Some(Glow::Lamp));
        let arcane = Glow::Arcane.params().unwrap();
        assert!(Glow::Alarm.params().unwrap().1 < arcane.1);
        // Кабели и утопленное — плашмя: под мебелью, без тени, в картинке карты.
        for flat in ["set:cyberpunk/objects/Cables_Floor_A_3x1.webp", "pack:fa/objects/Cable_Metal_Gray_A3_1x1.webp", "set:scifi/objects/Sunken_Cryo_Capsule_A_2x1.webp"] {
            assert!(flat_by_name(flat), "{flat}");
            assert_eq!(layer_by_name(flat), 100, "{flat}");
        }
        assert!(!flat_by_name("set:scifi/objects/Cryo_Capsule_Homunculus_A_2x1.webp"));
        assert!(Glow::CHOICES.contains(&Glow::Arcane));
        // Рукой: выключить похожее по имени и зажечь непохожее.
        assert_eq!(Prop { light: Some(Glow::Off), ..prop("builtin:campfire") }.light(), None);
        assert_eq!(Prop { light: Some(Glow::Brazier), ..prop("pack:x|textures/objects/Barrel_A.webp") }.light(), Some(Glow::Brazier));
        // Свой свет: что задано — вместо того, что даёт вид; остальное от вида.
        let lamp = Prop { light: Some(Glow::Alarm), ..prop("pack:x|textures/objects/Barrel_A.webp") };
        let plain = lamp.shine().unwrap();
        assert_eq!((plain.dim, plain.color.as_str(), plain.animation.as_str(), plain.tempo), (2.2, "#ff3020", "revolving", (7, 8)));
        let tuned = Prop { glow: Some(GlowTune { radius: Some(4.4), color: Some([0, 128, 255]), animation: Some(String::new()), speed: Some(2), intensity: None }), ..lamp.clone() }.shine().unwrap();
        assert_eq!((tuned.dim, tuned.color.as_str(), tuned.animation.as_str(), tuned.tempo), (4.4, "#0080ff", "", (2, 8)));
        assert!((tuned.bright - 0.8).abs() < 1e-5, "яркий круг — в той же доле: {}", tuned.bright);
        assert_eq!(tuned.look, plain.look);
        // В файле карты пустые поля не пишутся, старая карта читается без них.
        let saved = serde_json::to_string(&Prop { glow: Some(GlowTune { speed: Some(1), ..Default::default() }), ..lamp.clone() }).unwrap();
        assert!(saved.contains(r#""glow":{"speed":1}"#) && !saved.contains("sunk"), "{saved}");
        let old: Prop = serde_json::from_str(&serde_json::to_string(&lamp).unwrap()).unwrap();
        assert_eq!((old.glow, old.sunk), (None, None));
        assert!(prop("builtin:tree").shine().is_none());
        // Ватерлиния: кресло по спинку в воде — без тени, сухой верх своим тайлом; целиком под
        // водой — на дне, слоем пола, в картинке карты.
        let seat = prop("set:scifi/objects/Cinema_Seat_Blue_A_1x1.webp");
        assert!((seat.sunk(), seat.casts_shadow(), seat.own_tile()) == (0.0, true, true));
        let half = Prop { sunk: Some(0.6), ..seat.clone() };
        assert!((half.sunk(), half.casts_shadow(), half.own_tile(), half.carries_shadow()) == (0.6, false, true, false));
        assert_eq!(half.layer(), seat.layer());
        let drowned = Prop { sunk: Some(1.0), ..seat.clone() };
        assert!((drowned.own_tile(), drowned.casts_shadow(), drowned.layer()) == (false, false, 100));
        assert_eq!(Prop { sunk: Some(7.5), ..seat.clone() }.sunk(), 1.0);
        assert_eq!(Prop { sunk: Some(f32::NAN), ..seat.clone() }.sunk(), 0.0);
        assert!(Prop { sunk: Some(0.6), shadow: Some(true), ..seat.clone() }.casts_shadow(), "тень рукой — главнее");
        assert!(serde_json::to_string(&half).unwrap().contains(r#""sunk":0.6"#));
        assert!(prop("builtin:tree").is_canopy());
        // Чужие деревья — только рукой: листва у них любая, сухим крона ни к чему.
        assert!(!prop("pack:fa.dungeondraft_pack|textures/objects/Oak_Tree_A_3x3.webp").is_canopy());
        assert!(!prop("pack:x|textures/objects/Dead_Tree_Roots_A_6x6.webp").is_canopy());
        assert!(!prop("builtin:stump").is_canopy());
        assert!(is_bare_tree("pack:fa.dungeondraft_pack|textures/objects/Nature/Trees/Bare_Trees/Bare_Tree_A1_6x6.webp"));
        assert!(!is_bare_tree("pack:fa.dungeondraft_pack|textures/objects/Nature/Trees/Tree_Tops/Tree_Top_Oak_A_5x5.webp"));
        // Крыша нижнего этажа, накрытая полом верхнего больше чем наполовину, сверху не видна.
        let mut deck = Map::new(10, 10);
        let ring = [[2.0, 2.0], [8.0, 2.0], [8.0, 8.0], [2.0, 8.0]];
        assert!(!under_floor(&ring, &[&deck]), "над крышей пусто");
        let mut m = Mask::for_cells(10, 10);
        for j in 0..m.h {
            for i in 0..m.w {
                if (i as f32 + 0.5) / crate::mask::RES as f32 >= 4.0 {
                    m.set(i, j, 255);
                }
            }
        }
        deck.terrain.push(Coat { material: 0, mask: m });
        assert!(under_floor(&ring, &[&deck]), "две трети крыши под полом");
        assert!(!under_floor(&[[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]], &[&deck]), "под полом только пятая часть");
        assert_eq!(floor_at(&deck, [6.0, 1.0]), 255);
        assert_eq!(floor_at(&deck, [1.0, 1.0]), 0);
        // Пень под кроной — только у дерева; парус, отмеченный кроной, уходит в тайл целиком.
        assert!(has_trunk("builtin:tree") && has_trunk("pack:fa.dungeondraft_pack|textures/objects/Nature/Trees/Tree_Tops/Tree_Top_Oak_A_5x5.webp"));
        assert!(has_trunk("C:\\Картограф\\ассеты\\objects\\trees\\Willow_Black_Ancient_A_9x9.webp"));
        assert!(!has_trunk("pack:fa.dungeondraft_pack|textures/objects/Vehicles/Ships/Sails/Sail_Yard_Light_Tan_A1_14x3.webp"));
        assert!(prop("pack:fa.dungeondraft_pack|textures/objects/Nature/Trees/Tree_Tops/Tree_Top_Oak_A_5x5.webp").is_canopy());
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

    /// Свои карты пользователя: прочитать, записать нынешним форматом, прочитать снова —
    /// то же самое. Файлы — в `KARTA_FILES` через `;`. Запуск:
    /// `KARTA_FILES=… cargo test --release real_maps_round_trip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_maps_round_trip() {
        let files = std::env::var("KARTA_FILES").expect("KARTA_FILES");
        for f in files.split(';').filter(|f| !f.is_empty()) {
            let text = std::fs::read_to_string(f).expect(f);
            let t0 = std::time::Instant::now();
            let a = Map::from_json(&text).expect(f);
            let t1 = std::time::Instant::now();
            let json = a.to_json();
            let t2 = std::time::Instant::now();
            let b = Map::from_json(&json).expect(f);
            assert_eq!(a, b, "{f}");
            let chunks: usize = std::iter::once(&a).chain(&a.levels).map(|m| m.rooms.data_chunks() + m.terrain.iter().chain(&m.room_floor).map(|c| c.mask.data_chunks()).sum::<usize>()).sum();
            println!("{f}: {:.0} → {:.0} КБ, чтение {:.0} мс, запись {:.0} мс, кусков с точками {chunks}", text.len() as f64 / 1024.0, json.len() as f64 / 1024.0, (t1 - t0).as_secs_f64() * 1000.0, (t2 - t1).as_secs_f64() * 1000.0);
        }
    }

    #[test]
    fn remembered_name_traits_match_the_name_rules() {
        // Запомненное по ключу — то же, что правила имени, и второй раз тоже.
        let keys = [
            "builtin:tree",
            "builtin:chair",
            "builtin:bush",
            "builtin:market_stall_red",
            "pack:fa.dungeondraft_pack|textures/objects/Rug_Round_A.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Nature/Trees/Tree_Tops/Tree_Top_Oak_A_5x5.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Hammock_A_3x2.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Cobweb_Corner_A.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Windsock_A_2x1.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Airship_Small_A_8x4.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Clothes_Line_A.webp",
            "pack:fa.dungeondraft_pack|textures/objects/Arranged_Tarps/Tarp_A.webp",
            "C:\\Картограф\\ассеты\\objects\\bridges\\Bridge_Pier_End_A_3x3.webp",
            "C:\\Картограф\\ассеты\\objects\\bushes\\Bush_Round_A_2x2.webp",
            "builtin:stairs_shade",
            "Crane_Harbour_A_6x6.webp",
        ];
        for _ in 0..2 {
            for k in keys {
                let t = name_traits(k);
                assert_eq!(
                    (t.layer, t.flat, t.canopy, t.bush, t.awning, t.line, t.bridge, t.living, t.hover, t.swing, t.web, t.stairs_shade),
                    (layer_by_name(k), flat_by_name(k), canopy_by_name(k), bush_by_name(k), awning_by_name(k), line_by_name(k), bridge_by_name(k), living_by_name(k).is_some(), hover_by_name(k), swing_by_name(k), web_by_name(k), k.ends_with("stairs_shade")),
                    "{k}"
                );
            }
        }
    }

    #[test]
    fn rotated_prop_hit_test() {
        let p = Prop { asset: "x".into(), pos: [5.0, 5.0], size: [4.0, 1.0], rot: 90.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
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
        let i = m.material_index(Material { name: "Плитка".into(), paint: Paint::Texture("C:/t/tile.png".into()), vary: false, live: Live::Auto, rot: 0.0 }).unwrap();
        let n = builtin_materials().len();
        assert_eq!(i, n);
        assert_eq!(m.material_index(Material { name: "другое имя".into(), paint: Paint::Texture("C:/t/tile.png".into()), vary: false, live: Live::Auto, rot: 0.0 }), Some(n));
        m.walls.push(Wall { points: vec![[1.0, 1.0], [3.0, 1.0]], closed: false, bends: Vec::new(), low: false, style: None });
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
        let w = Wall { points: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 3.0]], closed: false, bends: vec![1.0], low: false, style: None };
        assert!(w.segments().count() > 4 && w.distance([2.0, -1.0]) < 0.02);
    }

    #[test]
    fn trace_and_sketch_are_kept_in_the_file_and_move_with_the_map() {
        let mut m = Map::new(10, 8);
        let plain = serde_json::to_string(&m).unwrap();
        assert!(!plain.contains("trace") && !plain.contains("sketch"), "без образца в файле его нет");
        m.trace = Some(Trace { opacity: 0.3, on_top: false, ..Trace::new("C:/книга/карта.webp".into(), m.width) });
        m.sketch = "C:/книга/эскиз.png".into();
        let back: Map = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!((back.trace.clone(), back.sketch.clone()), (m.trace.clone(), m.sketch.clone()));
        assert_eq!(back.trace.as_ref().map(|t| t.width), Some(10.0), "новый образец — во всю ширину карты");
        m.grow(2, 3, 0, 0);
        assert_eq!(m.trace.as_ref().map(|t| t.pos), Some([2.0, 3.0]), "образец сдвинулся с картой");
        // Файл без «показывать» и «поверх» (вручную) — образец виден и поверх.
        let t: Trace = serde_json::from_str(r#"{"path":"a.png","pos":[0,0],"width":5,"opacity":0.5}"#).unwrap();
        assert!(t.shown && t.on_top);
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
