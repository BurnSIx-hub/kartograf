//! План карты: замысел до постройки. Генераторы не решают сами, что и где будет, —
//! они строят карту по плану, а план пишет случай, человек или языковая модель.
//!
//! План короткий и читается руками: вид карты, размер, комнаты с назначением,
//! связи между ними (дверь, двойная дверь, проём, тайный ход, мост), настроение,
//! стиль и обязательные объекты. Имена полей английские, значения тоже, но русские
//! синонимы понимаются при чтении (`"роль": "кухня"` — нет, а `"role": "кухня"` — да).
//!
//! Что из плана берёт каждый вид карты — в `plans/README.md`. Поля, которые вид
//! не использует, при постройке попадают в предупреждения, а не в ошибки.
//!
//! Один и тот же план с тем же зерном даёт одну и ту же карту (см. `procgen`).

use crate::generate::{Kind, Options};
use crate::library;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const VERSION: u32 = 1;
/// Имя «комнаты» снаружи: связь с ней — вход в здание или выход из подземелья.
pub const OUTSIDE: &str = "outside";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    pub kind: Kind,
    /// Как назвать карту; только для человека.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Ширина и высота в клетках; без них — размер открытой карты.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    /// Зерно случайности: то же зерно — та же карта. Пусто — случайное при постройке.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub seed: String,
    #[serde(default, skip_serializing_if = "Mood::is_default")]
    pub mood: Mood,
    /// Чем строить: встроенными ассетами или своими из библиотеки (чего в ней
    /// нет — встроенными).
    #[serde(default, skip_serializing_if = "Assets::is_builtin")]
    pub assets: Assets,
    #[serde(default, skip_serializing_if = "Style::is_empty")]
    pub style: Style,
    /// С какой стороны река; `auto` — как решит генератор, `none` — без реки.
    #[serde(default, skip_serializing_if = "River::is_auto")]
    pub river: River,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<RoomSpec>,
    /// Связи между комнатами. Пусто — строитель соединит комнаты сам.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// Объекты вне комнат: во дворе, на площади, у стоянки.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    /// Настройки, которые раньше были только в окне генератора.
    #[serde(default, skip_serializing_if = "Opts::is_default")]
    pub options: Opts,
}

/// Комната дома или подземелья.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomSpec {
    /// Имя для связей: латиницей или кириллицей, без пробелов внутри не обязательно.
    pub id: String,
    /// Подпись на карте; пусто — без подписи.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Role::is_auto")]
    pub role: Role,
    #[serde(default, skip_serializing_if = "Size::is_medium")]
    pub size: Size,
    /// Покрытие пола: имя встроенной текстуры (`stone_tiles`, «Плитка»).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<String>,
    /// Что обязательно стоит в комнате, кроме мебели по назначению.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
}

impl RoomSpec {
    pub fn new(id: impl Into<String>, role: Role) -> Self {
        Self { id: id.into(), name: String::new(), role, size: Size::default(), floor: None, objects: Vec::new() }
    }
}

/// Назначение комнаты: по нему ставится мебель. `auto` — строитель решит сам.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    #[default]
    Auto,
    /// Главная комната: зал таверны или гостиная дома, через неё вход.
    #[serde(alias = "зал")]
    Hall,
    #[serde(alias = "гостиная")]
    Living,
    #[serde(alias = "кухня")]
    Kitchen,
    #[serde(alias = "спальня")]
    Bedroom,
    #[serde(alias = "кладовая", alias = "склад", alias = "подвал")]
    Storage,
    #[serde(alias = "библиотека", alias = "кабинет")]
    Library,
    #[serde(alias = "святилище", alias = "храм")]
    Shrine,
    #[serde(alias = "оружейная")]
    Armoury,
    #[serde(alias = "сокровищница")]
    Treasure,
    #[serde(alias = "казарма")]
    Barracks,
    #[serde(alias = "вход")]
    Entrance,
    #[serde(alias = "выход")]
    Exit,
    #[serde(alias = "руины")]
    Ruin,
    #[serde(alias = "пусто", alias = "пустая")]
    Empty,
}

impl Role {
    pub fn is_auto(&self) -> bool {
        *self == Role::Auto
    }

    /// Главная комната здания.
    pub fn is_hall(self) -> bool {
        matches!(self, Role::Hall | Role::Living)
    }
}

/// Размер комнаты: словом или клетками `[ширина, высота]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Size {
    Named(SizeName),
    Cells([u32; 2]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SizeName {
    #[serde(alias = "маленькая", alias = "малая")]
    Small,
    #[serde(alias = "средняя", alias = "обычная")]
    Medium,
    #[serde(alias = "большая")]
    Large,
}

impl Default for Size {
    fn default() -> Self {
        Size::Named(SizeName::Medium)
    }
}

impl Size {
    pub fn is_medium(&self) -> bool {
        *self == Size::default()
    }

    /// Сколько примерно клеток занимает.
    pub fn area(self) -> u32 {
        match self {
            Size::Named(SizeName::Small) => 12,
            Size::Named(SizeName::Medium) => 20,
            Size::Named(SizeName::Large) => 36,
            Size::Cells([w, h]) => w * h,
        }
    }

    /// Вес при делении дома: во столько раз больше средней комнаты.
    pub fn weight(self) -> f32 {
        match self {
            Size::Named(SizeName::Small) => 0.6,
            Size::Named(SizeName::Medium) => 1.0,
            Size::Named(SizeName::Large) => 1.8,
            Size::Cells([w, h]) => (w * h) as f32 / 20.0,
        }
    }
}

/// Связь двух комнат. Вторая может быть `outside` — тогда это вход.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "LinkKind::is_door")]
    pub kind: LinkKind,
}

impl Link {
    #[cfg(test)]
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: LinkKind) -> Self {
        Self { from: from.into(), to: to.into(), kind }
    }

    pub fn touches(&self, id: &str) -> bool {
        self.from == id || self.to == id
    }

    /// Другой конец связи.
    pub fn other(&self, id: &str) -> &str {
        if self.from == id { &self.to } else { &self.from }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    #[default]
    #[serde(alias = "дверь")]
    Door,
    #[serde(alias = "double", alias = "двойная", alias = "двойная дверь", alias = "ворота")]
    DoubleDoor,
    #[serde(alias = "проём", alias = "проем", alias = "арка")]
    Opening,
    #[serde(alias = "тайный", alias = "тайный ход", alias = "потайная дверь")]
    Secret,
    /// Через реку: у входа дорожка переходит воду по мосту.
    #[serde(alias = "мост")]
    Bridge,
}

impl LinkKind {
    pub fn is_door(&self) -> bool {
        *self == LinkKind::Door
    }

    pub fn label(self) -> &'static str {
        match self {
            LinkKind::Door => "дверь",
            LinkKind::DoubleDoor => "двойная дверь",
            LinkKind::Opening => "проём",
            LinkKind::Secret => "тайный ход",
            LinkKind::Bridge => "мост",
        }
    }
}

/// Чем строить.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Assets {
    #[default]
    #[serde(alias = "встроенные", alias = "default")]
    Builtin,
    /// Паки и папка пользователя; слоты без замены — встроенные.
    #[serde(alias = "свои", alias = "библиотека", alias = "user", alias = "own")]
    Library,
}

impl Assets {
    pub fn is_builtin(&self) -> bool {
        *self == Assets::Builtin
    }
}

/// Настроение: как освещено и насколько обжито.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    #[default]
    #[serde(alias = "cozy", alias = "спокойно", alias = "уютно", alias = "обычно")]
    Calm,
    /// Темнее и тени гуще.
    #[serde(alias = "мрачно", alias = "dark")]
    Gloomy,
    /// Обломки, мох, следы запустения.
    #[serde(alias = "заброшено", alias = "ruined", alias = "руины")]
    Abandoned,
}

impl Mood {
    pub fn is_default(&self) -> bool {
        *self == Mood::Calm
    }
}

/// Чем выложено: имена встроенных стен, текстур пола и покрытий крыш
/// (`timber`, `wall_timber` или «Фахверк» — всё одно).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub walls: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roof: Option<String>,
}

impl Style {
    pub fn is_empty(&self) -> bool {
        *self == Style::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum River {
    #[default]
    #[serde(alias = "авто")]
    Auto,
    #[serde(alias = "нет", alias = "no")]
    None,
    #[serde(alias = "север", alias = "сверху")]
    North,
    #[serde(alias = "восток", alias = "справа")]
    East,
    #[serde(alias = "юг", alias = "снизу")]
    South,
    #[serde(alias = "запад", alias = "слева")]
    West,
}

impl River {
    pub fn is_auto(&self) -> bool {
        *self == River::Auto
    }

    /// Сторона света: 0 — север, 1 — восток, 2 — юг, 3 — запад.
    pub fn side(self) -> Option<i32> {
        match self {
            River::North => Some(0),
            River::East => Some(1),
            River::South => Some(2),
            River::West => Some(3),
            River::Auto | River::None => None,
        }
    }
}

/// Звуки мест «Wild Canvas»: ключ флага сцены `ambience` и подпись.
/// Ключи — те же, что в `PLACES` модуля (`scripts/ambience.js`).
pub const PLACES: [(&str, &str); 16] = [
    ("", "Без звука"),
    ("forest-summer", "Лес летом"),
    ("forest-spring", "Лес весной"),
    ("forest-autumn", "Лес осенью"),
    ("forest-winter", "Лес зимой"),
    ("forest-magic", "Магический лес"),
    ("cave-1", "Пещера: гул"),
    ("cave-2", "Пещера: жуткая"),
    ("cave-3", "Пещера: капли"),
    ("dungeon-1", "Подземелье: сквозняк"),
    ("dungeon-2", "Подземелье: жуткое"),
    ("dungeon-3", "Подземелье: тёмное"),
    ("town-1", "Квартал: улица"),
    ("town-2", "Квартал: деревня"),
    ("tavern", "Таверна"),
    ("house", "Дом с камином"),
];

/// Что летит от крон: ключ флага тайла `particles` и подпись.
pub const PARTICLES: [(&str, &str); 3] = [("", "Ничего"), ("leaves", "Листопад"), ("fireflies", "Светлячки")];

/// Погода сцены Foundry: ключ и подпись.
pub const WEATHERS: [(&str, &str); 2] = [("", "Ясно"), ("snow", "Снег")];

/// Что идёт с лесом этого звука: осенью листья, в магическом — светлячки, зимой снег.
/// Не лес — ничего.
pub fn place_defaults(place: &str) -> (&'static str, &'static str) {
    match place {
        "forest-autumn" => ("leaves", ""),
        "forest-magic" => ("fireflies", ""),
        "forest-winter" => ("", "snow"),
        _ => ("", ""),
    }
}

/// Время года леса: какие деревья, кусты и земля.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Season {
    #[default]
    #[serde(alias = "лето")]
    Summer,
    #[serde(alias = "весна")]
    Spring,
    #[serde(alias = "осень")]
    Autumn,
    #[serde(alias = "зима")]
    Winter,
    #[serde(alias = "магический", alias = "магия")]
    Magic,
}

impl Season {
    pub const ALL: [Season; 5] = [Season::Summer, Season::Spring, Season::Autumn, Season::Winter, Season::Magic];

    pub fn label(self) -> &'static str {
        match self {
            Season::Summer => "Летний",
            Season::Spring => "Весенний",
            Season::Autumn => "Осенний",
            Season::Winter => "Зимний",
            Season::Magic => "Магический",
        }
    }

    /// Название для окна генератора у дома, таверны, квартала.
    pub fn name(self) -> &'static str {
        match self {
            Season::Summer => "Лето",
            Season::Spring => "Весна",
            Season::Autumn => "Осень",
            Season::Winter => "Зима",
            Season::Magic => "Магия",
        }
    }

    pub fn is_summer(&self) -> bool {
        *self == Season::Summer
    }

    /// Что «Wild Canvas» пускает от крон леса: листопад, светлячков.
    pub fn particles(self) -> &'static str {
        match self {
            Season::Autumn => "leaves",
            Season::Magic => "fireflies",
            _ => "",
        }
    }

    /// Звук леса в «Wild Canvas».
    pub fn ambience(self) -> &'static str {
        match self {
            Season::Summer => "forest-summer",
            Season::Spring => "forest-spring",
            Season::Autumn => "forest-autumn",
            Season::Winter => "forest-winter",
            Season::Magic => "forest-magic",
        }
    }

    /// Погода сцены Foundry: зимой свой снег Foundry.
    pub fn weather(self) -> &'static str {
        if self == Season::Winter { "snow" } else { "" }
    }
}

/// Вид пещеры: из чего пол и стены, чем полны водоёмы, что растёт.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaveKind {
    #[default]
    #[serde(alias = "каменная", alias = "камень")]
    Stone,
    #[serde(alias = "ледяная", alias = "лёд", alias = "лед")]
    Ice,
    #[serde(alias = "песчаная", alias = "песок")]
    Sand,
    #[serde(alias = "болотная", alias = "болото")]
    Swamp,
    #[serde(alias = "вулканическая", alias = "огненная", alias = "лава")]
    Volcanic,
    #[serde(alias = "грибная", alias = "грибы")]
    Fungal,
}

impl CaveKind {
    pub const ALL: [CaveKind; 6] = [CaveKind::Stone, CaveKind::Ice, CaveKind::Sand, CaveKind::Swamp, CaveKind::Volcanic, CaveKind::Fungal];

    pub fn label(self) -> &'static str {
        match self {
            CaveKind::Stone => "Каменная",
            CaveKind::Ice => "Ледяная",
            CaveKind::Sand => "Песчаная",
            CaveKind::Swamp => "Болотная",
            CaveKind::Volcanic => "Вулканическая",
            CaveKind::Fungal => "Грибная",
        }
    }

    pub fn is_stone(&self) -> bool {
        *self == CaveKind::Stone
    }

    /// Звук пещеры в «Wild Canvas»: у льда и болота капель, у грибной — жуть.
    pub fn ambience(self) -> &'static str {
        match self {
            CaveKind::Ice | CaveKind::Swamp => "cave-3",
            CaveKind::Fungal => "cave-2",
            CaveKind::Stone | CaveKind::Sand | CaveKind::Volcanic => "cave-1",
        }
    }

    /// Грибная светится спорами — светлячки «Wild Canvas».
    pub fn particles(self) -> &'static str {
        if self == CaveKind::Fungal { "fireflies" } else { "" }
    }
}

/// Подземелье: из чего сложено и что в комнатах. Пол и стены — по виду.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DungeonKind {
    /// Склеп: усыпальницы, оссуарии, катакомбы с нишами, часовня.
    #[default]
    #[serde(alias = "склеп", alias = "гробница", alias = "катакомбы")]
    Crypt,
    /// Темница: камеры, караульная, пыточная.
    #[serde(alias = "темница", alias = "тюрьма", alias = "подземелье замка")]
    Prison,
    /// Храм: святилище с колоннадой, молельни, купель.
    #[serde(alias = "храм", alias = "святилище")]
    Temple,
    /// Руины: завалы, лужи, логово.
    #[serde(alias = "руины", alias = "развалины")]
    Ruins,
    /// Киберпанк — для своей вкладки, в списке видов подземелья его нет.
    #[serde(alias = "киберпанк")]
    Cyber,
}

impl DungeonKind {
    pub const ALL: [DungeonKind; 4] = [DungeonKind::Crypt, DungeonKind::Prison, DungeonKind::Temple, DungeonKind::Ruins];

    pub fn label(self) -> &'static str {
        match self {
            DungeonKind::Crypt => "Склеп",
            DungeonKind::Prison => "Темница",
            DungeonKind::Temple => "Храм",
            DungeonKind::Ruins => "Руины",
            DungeonKind::Cyber => "Киберпанк",
        }
    }

    pub fn is_crypt(&self) -> bool {
        *self == DungeonKind::Crypt
    }
}

/// Городской квартал: что стоит вдоль улиц.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum District {
    /// Как в старом городе: лавки и мастерские на главной, жильё в переулках,
    /// на углу бывает таверна.
    #[default]
    #[serde(alias = "обычный", alias = "жилой")]
    Mixed,
    /// Постоялый двор — самый большой дом квартала, лицом на площадь или главную
    /// улицу, с конюшней во дворе; вокруг обычный квартал.
    #[serde(alias = "таверна", alias = "постоялый двор", alias = "трактир")]
    Inn,
    /// Торговый: по улицам почти одни лавки разного товара, склады у заднего
    /// проезда, рыночная площадь со многими прилавками.
    #[serde(alias = "торговый", alias = "рынок", alias = "рыночный")]
    Market,
}

impl District {
    pub const ALL: [District; 3] = [District::Mixed, District::Inn, District::Market];

    pub fn label(self) -> &'static str {
        match self {
            District::Mixed => "Обычный",
            District::Inn => "С таверной",
            District::Market => "Торговый",
        }
    }

    pub fn is_mixed(&self) -> bool {
        *self == District::Mixed
    }
}

/// Настройки генератора; у каждого вида карты свои значимые.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opts {
    /// Насколько плотно: деревья, дома, камни, комнаты. 0..1.
    #[serde(default = "half")]
    pub density: f32,
    /// Подземелье: доля лишних коридоров, дающих кольца.
    #[serde(default = "tenth")]
    pub loops: f32,
    /// Подземелье: двери и объекты по комнатам.
    #[serde(default = "yes")]
    pub fill: bool,
    /// Подземелье: сторона комнаты от и до, в клетках, для комнат без своего размера.
    #[serde(default = "room_size")]
    pub room_size: [u32; 2],
    /// Лес, дом, таверна, квартал: время года.
    #[serde(default, skip_serializing_if = "Season::is_summer")]
    pub season: Season,
    /// Пещера: каменная, ледяная, песчаная…
    #[serde(default, skip_serializing_if = "CaveKind::is_stone")]
    pub cave: CaveKind,
    /// Квартал: обычный, с таверной, торговый.
    #[serde(default, skip_serializing_if = "District::is_mixed")]
    pub district: District,
    /// Подземелье: склеп, темница, храм, руины.
    #[serde(default, skip_serializing_if = "DungeonKind::is_crypt")]
    pub dungeon: DungeonKind,
}

fn half() -> f32 {
    0.5
}

fn tenth() -> f32 {
    0.1
}

fn yes() -> bool {
    true
}

fn room_size() -> [u32; 2] {
    [5, 10]
}

impl Default for Opts {
    fn default() -> Self {
        Self { density: 0.5, loops: 0.1, fill: true, room_size: [5, 10], season: Season::Summer, cave: CaveKind::Stone, district: District::Mixed, dungeon: DungeonKind::Crypt }
    }
}

impl Opts {
    pub fn is_default(&self) -> bool {
        *self == Opts::default()
    }
}

// ----- имена ассетов -----

/// Без приставки `builtin:` и `wall_`, в нижнем регистре.
fn bare(name: &str) -> String {
    name.trim().strip_prefix(library::BUILTIN).unwrap_or(name.trim()).to_lowercase()
}

fn find<'a>(name: &str, table: impl Iterator<Item = (&'a str, &'a str)>) -> Option<&'a str> {
    let n = bare(name);
    table.into_iter().find(|(id, label)| *id == n || label.to_lowercase() == n || id.strip_prefix("wall_").is_some_and(|s| s == n) || id.strip_prefix("roof_").is_some_and(|s| s == n)).map(|(id, _)| id)
}

/// Встроенный объект по имени или подписи: `chest`, «Сундук».
pub fn object_id(name: &str) -> Option<&'static str> {
    find(name, library::BUILTIN_OBJECTS.iter().map(|t| (t.0, t.1)))
}

/// Встроенная текстура пола.
pub fn texture_id(name: &str) -> Option<&'static str> {
    find(name, library::BUILTIN_TEXTURES.iter().map(|t| (t.0, t.1)))
}

pub fn wall_id(name: &str) -> Option<&'static str> {
    find(name, library::BUILTIN_WALLS.iter().map(|t| (t.0, t.1)))
}

pub fn roof_id(name: &str) -> Option<&'static str> {
    find(name, library::BUILTIN_ROOFS.iter().map(|t| (t.0, t.1)))
}

// ----- размер здания -----

/// Дом не растёт вместе с картой: дом на 40 клеток — это уже не дом.
pub fn building_size(kind: Kind, w: u32, h: u32) -> (i32, i32) {
    let (w, h) = (w as i32, h as i32);
    let (max_w, max_h) = if kind == Kind::Tavern { (28, 22) } else { (18, 14) };
    (((w as f32 * 0.8) as i32).min(max_w).clamp(6, (w - 2).max(6)), ((h as f32 * 0.7) as i32).min(max_h).clamp(6, (h - 4).max(6)))
}

impl Plan {
    pub fn new(kind: Kind) -> Self {
        Self {
            version: VERSION,
            kind,
            name: String::new(),
            size: None,
            seed: String::new(),
            mood: Mood::Calm,
            assets: Assets::Builtin,
            style: Style::default(),
            river: River::Auto,
            rooms: Vec::new(),
            links: Vec::new(),
            objects: Vec::new(),
            options: Opts::default(),
        }
    }

    #[cfg(test)]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Разбор без проверки смысла: только форма.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("файл не читается как план: {e}"))?;
        let version = value.get("version").and_then(|v| v.as_u64()).ok_or("в плане нет поля version")?;
        if version > VERSION as u64 {
            return Err(format!("план записан более новой версией (формат {version}, программа знает {VERSION})"));
        }
        serde_json::from_value(value).map_err(|e| format!("план не разбирается: {e}"))
    }

    /// Случайный план из настроек окна генератора — чтобы случайная карта шла
    /// тем же путём, что и карта по плану.
    pub fn random(opt: &Options, size: [u32; 2]) -> Self {
        let mut plan = Plan::new(opt.kind);
        plan.size = Some(size);
        plan.seed = opt.seed.clone();
        plan.assets = if opt.library { Assets::Library } else { Assets::Builtin };
        let least = size[0].min(size[1]).saturating_sub(2).max(2);
        let lo = opt.min_size.clamp(2, least);
        plan.options = Opts { density: opt.density, loops: opt.loops, fill: opt.fill, room_size: [lo, opt.max_size.clamp(lo, least)], season: opt.season, cave: opt.cave, district: opt.district, dungeon: opt.dungeon };
        match opt.kind {
            Kind::Dungeon | Kind::Cyber => plan.rooms = (1..=opt.rooms.max(1)).map(|i| RoomSpec::new(format!("r{i}"), Role::Auto)).collect(),
            Kind::House | Kind::Tavern => {
                let tavern = opt.kind == Kind::Tavern;
                let (bw, bh) = building_size(opt.kind, size[0], size[1]);
                // Столько же комнат, сколько давала нарезка дома с таким порогом площади.
                let area = 30.0 + (1.0 - opt.density) * 36.0;
                let rest = if tavern { (bw * bh) as f32 * 0.4 } else { (bw * bh) as f32 };
                let n = ((rest / area).round() as usize).clamp(if tavern { 1 } else { 2 }, 7);
                let hall = if tavern { Role::Hall } else { Role::Living };
                let order = [Role::Kitchen, Role::Storage, Role::Bedroom, Role::Bedroom, Role::Library, Role::Bedroom, Role::Storage];
                plan.rooms.push(RoomSpec { size: Size::Named(SizeName::Large), ..RoomSpec::new("hall", hall) });
                for (k, role) in order.into_iter().take(if tavern { n } else { n - 1 }).enumerate() {
                    plan.rooms.push(RoomSpec::new(format!("{}{}", role_id(role), k + 1), role));
                }
            }
            Kind::Cave | Kind::Forest | Kind::Town | Kind::Walls | Kind::Arena | Kind::Church | Kind::Jarl => {}
        }
        // На крошечной карте столько комнат не поместится: лишние отпадают с конца.
        while plan.rooms.len() > 1 && plan.validate(size).is_err() {
            plan.rooms.pop();
        }
        if plan.validate(size).is_err()
            && let Some(r) = plan.rooms.first_mut()
        {
            r.size = Size::default();
        }
        plan
    }

    pub fn room(&self, id: &str) -> Option<&RoomSpec> {
        self.rooms.iter().find(|r| r.id == id)
    }

    pub fn room_index(&self, id: &str) -> Option<usize> {
        self.rooms.iter().position(|r| r.id == id)
    }

    /// Связи комнаты с другими комнатами (не наружу).
    pub fn inner_links(&self) -> impl Iterator<Item = &Link> {
        self.links.iter().filter(|l| l.from != OUTSIDE && l.to != OUTSIDE)
    }

    /// Комната, с которой начинается карта: связанная с улицей, с ролью входа или зала, иначе первая.
    pub fn entrance(&self) -> Option<usize> {
        self.links
            .iter()
            .find(|l| l.touches(OUTSIDE))
            .and_then(|l| self.room_index(l.other(OUTSIDE)))
            .or_else(|| self.rooms.iter().position(|r| r.role == Role::Entrance))
            .or_else(|| self.rooms.iter().position(|r| r.role.is_hall()))
            .or_else(|| (!self.rooms.is_empty()).then_some(0))
    }

    /// Ширина и высота карты, которую построит план.
    pub fn map_size(&self, fallback: [u32; 2]) -> [u32; 2] {
        self.size.unwrap_or(fallback)
    }

    /// Проверка смысла: все ошибки разом, по строке на каждую. `fallback` — размер
    /// карты, если план своего не задаёт.
    pub fn validate(&self, fallback: [u32; 2]) -> Result<(), String> {
        let mut errors: Vec<String> = Vec::new();
        if self.version > VERSION {
            errors.push(format!("план записан более новой версией (формат {}, программа знает {VERSION})", self.version));
        }
        let [w, h] = self.map_size(fallback);
        if !(4..=400).contains(&w) || !(4..=400).contains(&h) {
            errors.push(format!("размер карты {w}×{h}: сторона от 4 до 400 клеток"));
        }
        let has_rooms = matches!(self.kind, Kind::Dungeon | Kind::Cyber | Kind::House | Kind::Tavern);
        if has_rooms && self.rooms.is_empty() {
            errors.push(format!("{}: нужна хотя бы одна комната", self.kind.label()));
        }
        let mut ids = BTreeSet::new();
        for r in &self.rooms {
            if r.id.trim().is_empty() {
                errors.push("у комнаты пустое имя (id)".into());
            } else if r.id == OUTSIDE {
                errors.push(format!("имя «{OUTSIDE}» занято: так зовётся улица"));
            } else if !ids.insert(r.id.as_str()) {
                errors.push(format!("две комнаты с именем «{}»", r.id));
            }
            if let Some(f) = &r.floor
                && texture_id(f).is_none()
            {
                errors.push(format!("комната «{}»: пол «{f}» неизвестен", r.id));
            }
            for o in &r.objects {
                if object_id(o).is_none() {
                    errors.push(format!("комната «{}»: объект «{o}» неизвестен", r.id));
                }
            }
            if let Size::Cells([cw, ch]) = r.size
                && (cw < 2 || ch < 2)
            {
                errors.push(format!("комната «{}»: сторона меньше двух клеток", r.id));
            }
        }
        for o in &self.objects {
            if object_id(o).is_none() {
                errors.push(format!("объект «{o}» неизвестен"));
            }
        }
        if let Some(s) = &self.style.walls
            && wall_id(s).is_none()
        {
            errors.push(format!("стены «{s}» неизвестны"));
        }
        if let Some(s) = &self.style.floor
            && texture_id(s).is_none()
        {
            errors.push(format!("пол «{s}» неизвестен"));
        }
        if let Some(s) = &self.style.roof
            && roof_id(s).is_none()
        {
            errors.push(format!("крыша «{s}» неизвестна"));
        }
        for l in &self.links {
            for end in [&l.from, &l.to] {
                if end != OUTSIDE && self.room(end).is_none() {
                    errors.push(format!("связь «{}» — «{}»: комнаты «{end}» нет", l.from, l.to));
                }
            }
            if l.from == l.to {
                errors.push(format!("связь «{}» сама с собой", l.from));
            }
        }
        // Достижимость: если связи между комнатами заданы, по ним должно быть куда
        // дойти из входа. Один только вход — не связи: комнаты соединит строитель.
        if self.inner_links().next().is_some() && errors.is_empty() {
            let start = self.entrance().unwrap_or(0);
            let mut seen = vec![false; self.rooms.len()];
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(i) = queue.pop_front() {
                let id = &self.rooms[i].id;
                for l in self.inner_links().filter(|l| l.touches(id)) {
                    if let Some(j) = self.room_index(l.other(id))
                        && !seen[j]
                    {
                        seen[j] = true;
                        queue.push_back(j);
                    }
                }
            }
            for (i, r) in self.rooms.iter().enumerate() {
                if !seen[i] {
                    errors.push(format!("комната «{}» недостижима: нет пути по связям из «{}»", r.id, self.rooms[start].id));
                }
            }
        }
        // Помещается ли.
        match self.kind {
            Kind::House | Kind::Tavern => {
                let (bw, bh) = building_size(self.kind, w, h);
                let need: u32 = self.rooms.iter().map(|r| r.size.area().max(16)).sum();
                for r in &self.rooms {
                    if let Size::Cells([cw, ch]) = r.size
                        && (cw as i32 > bw || ch as i32 > bh)
                    {
                        errors.push(format!("комната «{}» {cw}×{ch} больше дома {bw}×{bh}", r.id));
                    }
                }
                if need > (bw * bh) as u32 {
                    errors.push(format!("комнаты не помещаются: им нужно около {need} клеток, а дом на карте {w}×{h} выходит {bw}×{bh} — {} клеток", bw * bh));
                }
            }
            Kind::Dungeon | Kind::Cyber => {
                let [lo, hi] = self.options.room_size;
                if lo < 2 || hi < lo {
                    errors.push(format!("размер комнат от {lo} до {hi}: от не меньше 2 и не больше до"));
                }
                if lo + 2 > w.min(h) {
                    errors.push(format!("комната от {lo} клеток не помещается в карту {w}×{h}"));
                }
                // Считаем по наименьшему размеру каждой комнаты: генератор ставит, что влезет.
                let third = hi.saturating_sub(lo) / 3;
                let mut need = 0u32;
                for r in &self.rooms {
                    let [cw, ch] = match r.size {
                        Size::Cells(c) => c,
                        Size::Named(SizeName::Small | SizeName::Medium) => [lo, lo],
                        Size::Named(SizeName::Large) => [hi - third, hi - third],
                    };
                    if cw + 2 > w || ch + 2 > h {
                        errors.push(format!("комната «{}» {cw}×{ch} не помещается в карту {w}×{h}", r.id));
                    }
                    need += (cw + 1) * (ch + 1);
                }
                if need > w * h {
                    errors.push(format!("комнаты не помещаются: им нужно хотя бы {need} клеток, на карте {w}×{h} их {}", w * h));
                }
            }
            Kind::Cave | Kind::Forest | Kind::Town | Kind::Walls | Kind::Arena | Kind::Church | Kind::Jarl => {}
        }
        if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
    }

    /// Чего вид карты не умеет: поля заданы, а строитель их пропустит.
    pub fn unused(&self) -> Vec<String> {
        let mut out = Vec::new();
        let kind = self.kind.label();
        match self.kind {
            Kind::Cave | Kind::Forest | Kind::Town | Kind::Walls | Kind::Arena | Kind::Church | Kind::Jarl => {
                if !self.rooms.is_empty() {
                    out.push(format!("{kind}: комнаты не используются ({} шт. пропущено)", self.rooms.len()));
                }
                if !self.links.is_empty() {
                    out.push(format!("{kind}: связи не используются"));
                }
                if self.style.walls.is_some() && !matches!(self.kind, Kind::Town | Kind::Walls | Kind::Arena | Kind::Church | Kind::Jarl) {
                    out.push(format!("{kind}: стены из стиля не используются"));
                }
                if self.style.roof.is_some() && !matches!(self.kind, Kind::Town | Kind::Walls | Kind::Church | Kind::Jarl) {
                    out.push(format!("{kind}: крыша из стиля не используется"));
                }
                if self.kind == Kind::Forest && matches!(self.river, River::North | River::South) {
                    out.push("лес: река идёт с севера на юг, подходит только восток или запад".into());
                }
                if self.kind != Kind::Forest && !self.river.is_auto() {
                    out.push(format!("{kind}: река не поддерживается"));
                }
            }
            Kind::Dungeon | Kind::Cyber => {
                if self.style.roof.is_some() {
                    out.push("подземелье: крыша не нужна".into());
                }
                if !self.river.is_auto() {
                    out.push("подземелье: река не поддерживается".into());
                }
                if self.links.iter().any(|l| l.kind == LinkKind::Bridge) {
                    out.push("подземелье: мост здесь — обычный коридор".into());
                }
            }
            Kind::House | Kind::Tavern => {
                if self.style.roof.is_some() {
                    out.push(format!("{kind}: крыша не рисуется, карта показывает дом изнутри"));
                }
            }
        }
        out
    }
}

/// Латинское имя роли для имён комнат случайного плана.
fn role_id(role: Role) -> &'static str {
    match role {
        Role::Kitchen => "kitchen",
        Role::Storage => "storage",
        Role::Bedroom => "bedroom",
        Role::Library => "library",
        _ => "room",
    }
}

/// Граф комнат по связям: кому с кем, без улицы.
pub fn adjacency(plan: &Plan) -> BTreeMap<usize, Vec<usize>> {
    let mut g: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for l in plan.inner_links() {
        if let (Some(a), Some(b)) = (plan.room_index(&l.from), plan.room_index(&l.to)) {
            g.entry(a).or_default().push(b);
            g.entry(b).or_default().push(a);
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tavern() -> Plan {
        Plan::from_json(
            r#"{
            "version": 1, "kind": "tavern", "size": [36, 28], "seed": "ясень",
            "rooms": [
                {"id": "hall", "role": "зал", "size": "large", "objects": ["камин"]},
                {"id": "kitchen", "role": "kitchen"},
                {"id": "cellar", "role": "storage", "name": "Подвал"},
                {"id": "pantry", "role": "storage"}
            ],
            "links": [
                {"from": "hall", "to": "outside", "kind": "double_door"},
                {"from": "hall", "to": "kitchen"},
                {"from": "kitchen", "to": "pantry"},
                {"from": "hall", "to": "cellar"},
                {"from": "cellar", "to": "pantry", "kind": "тайный ход"}
            ]
        }"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_russian_synonyms_and_round_trips() {
        let p = tavern();
        assert_eq!(p.kind, Kind::Tavern);
        assert_eq!(p.rooms[0].role, Role::Hall);
        assert_eq!(p.rooms[0].size, Size::Named(SizeName::Large));
        assert_eq!(p.links[4].kind, LinkKind::Secret);
        assert_eq!(p.links[1].kind, LinkKind::Door);
        assert!(p.validate([30, 20]).is_ok());
        let again = Plan::from_json(&p.to_json()).unwrap();
        assert_eq!(again, p);
        // В файле — английские имена без лишних полей.
        let json = p.to_json();
        assert!(json.contains("\"role\": \"hall\"") && json.contains("\"kind\": \"secret\"") && !json.contains("\"mood\""));
    }

    #[test]
    fn errors_are_named() {
        let mut p = tavern();
        p.links.push(Link::new("hall", "attic", LinkKind::Door));
        let e = p.validate([30, 20]).unwrap_err();
        assert!(e.contains("комнаты «attic» нет"), "{e}");

        let mut p = tavern();
        p.rooms.push(RoomSpec::new("vault", Role::Treasure));
        let e = p.validate([30, 20]).unwrap_err();
        assert!(e.contains("«vault» недостижима"), "{e}");

        let mut p = tavern();
        p.size = Some([12, 10]);
        p.rooms.push(RoomSpec { size: Size::Cells([20, 20]), ..RoomSpec::new("big", Role::Bedroom) });
        p.links.push(Link::new("hall", "big", LinkKind::Door));
        let e = p.validate([30, 20]).unwrap_err();
        assert!(e.contains("больше дома") && e.contains("не помещаются"), "{e}");

        let mut p = tavern();
        p.objects.push("дракон".into());
        p.rooms[1].floor = Some("лава из стекла".into());
        let e = p.validate([30, 20]).unwrap_err();
        assert!(e.contains("объект «дракон» неизвестен") && e.contains("пол «лава из стекла» неизвестен"), "{e}");

        assert!(Plan::from_json(r#"{"version": 9, "kind": "house"}"#).unwrap_err().contains("более новой версией"));
        assert!(Plan::from_json(r#"{"version": 1, "kind": "замок"}"#).unwrap_err().contains("не разбирается"));
        assert!(Plan::from_json("{{").unwrap_err().contains("не читается"));
    }

    #[test]
    fn dungeon_fit_is_checked() {
        let mut p = Plan::new(Kind::Dungeon);
        p.size = Some([20, 15]);
        p.rooms = (0..12).map(|i| RoomSpec { size: Size::Named(SizeName::Large), ..RoomSpec::new(format!("r{i}"), Role::Auto) }).collect();
        let e = p.validate([30, 20]).unwrap_err();
        assert!(e.contains("не помещаются"), "{e}");
        p.rooms.truncate(2);
        assert!(p.validate([30, 20]).is_ok());
        p.options.room_size = [8, 6];
        assert!(p.validate([30, 20]).unwrap_err().contains("от не меньше 2"));
    }

    #[test]
    fn random_plan_mirrors_options() {
        let opt = Options { kind: Kind::Dungeon, rooms: 7, seed: "x".into(), ..Default::default() };
        let p = Plan::random(&opt, [40, 30]);
        assert_eq!(p.rooms.len(), 7);
        assert_eq!(p.options.room_size, [5, 10]);
        assert!(p.validate([40, 30]).is_ok());
        let h = Plan::random(&Options { kind: Kind::House, ..Default::default() }, [30, 24]);
        assert!(h.rooms[0].role.is_hall() && h.rooms.len() >= 3 && h.validate([30, 24]).is_ok(), "{}", h.to_json());
        let t = Plan::random(&Options { kind: Kind::Tavern, ..Default::default() }, [40, 30]);
        assert_eq!(t.rooms[0].role, Role::Hall);
        assert!(Plan::random(&Options { kind: Kind::Forest, ..Default::default() }, [40, 30]).rooms.is_empty());
    }

    #[test]
    fn asset_names_resolve_both_ways() {
        assert_eq!(object_id("Камин"), Some("fireplace"));
        assert_eq!(object_id("builtin:chest"), Some("chest"));
        assert_eq!(wall_id("timber"), Some("wall_timber"));
        assert_eq!(wall_id("Фахверк"), Some("wall_timber"));
        assert_eq!(roof_id("clay_red"), Some("roof_clay_red"));
        assert_eq!(texture_id("плитка"), Some("stone_tiles"));
        assert_eq!(object_id("единорог"), None);
    }
}
