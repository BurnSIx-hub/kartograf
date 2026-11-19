//! Проверка карты перед Foundry: смысл предметов, трубы, наложения, двери, стены, улица и дом,
//! край, подписи, лампы и фонари, проходы. Перенесена 2026-11-18 из `tools/maps/check.py`
//! (Python, через MCP `describe`) один в один: теперь её видно в окне Картографа, а скрипт
//! берёт тот же список через MCP `check_map`. Ложное срабатывание чинить здесь, с тестом.
//!
//! Вход — плоские данные одного этажа ([`Input`]): окно собирает их из карты и картинок, тесты —
//! руками. Координаты — клетки, y вниз; расчёт в f64, как в Python, чтобы пороги не сдвинулись.

use std::collections::{HashMap, HashSet};

type P = [f64; 2];

/// Предмет: имя — файл ассета без расширения (`library::label`), размер — с масштабом.
#[derive(Clone, Debug, Default)]
pub struct Obj {
    /// Номер в `Map::props`.
    pub index: usize,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Поворот, градусы по часовой.
    pub rot: f64,
    pub layer: i32,
    pub canopy: bool,
    /// Видимая часть картинки в клетках от середины, до поворота: лево, верх, право, низ.
    pub vis: Option<[f64; 4]>,
    /// Маска видимой части 16×16 (`library::visible_shape`): строка — число, бит i — столбец i.
    pub mask: Option<Vec<u16>>,
}

/// Крыша — она же дом: прямоугольник по стенам или контур.
#[derive(Clone, Debug, Default)]
pub struct House {
    pub index: usize,
    pub area: Option<[f64; 4]>,
    pub points: Vec<P>,
    /// Плоская крыша — палуба, терраса: по ней ходят, на ней стоит что угодно.
    pub flat: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Door {
    pub index: usize,
    pub a: P,
    pub b: P,
    pub window: bool,
    /// Проём без створки.
    pub opening: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Text {
    pub text: String,
    pub size: f64,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub points: Vec<P>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub width: f64,
    pub height: f64,
    pub objs: Vec<Obj>,
    pub houses: Vec<House>,
    pub doors: Vec<Door>,
    pub labels: Vec<Text>,
    pub walls: Vec<Line>,
    /// Стены по краю закрашенных комнат.
    pub room_walls: Vec<(P, P)>,
    /// Перила помостов — ломаными: через них не пройти.
    pub rails: Vec<Vec<P>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Roof,
    Flat,
    Pipe,
    Overlap,
    Door,
    Wall,
    Street,
    Indoor,
    Edge,
    Label,
    Lamp,
    Lantern,
    Pass,
}

impl Kind {
    pub const ALL: [Kind; 13] = [
        Kind::Roof,
        Kind::Flat,
        Kind::Pipe,
        Kind::Overlap,
        Kind::Door,
        Kind::Wall,
        Kind::Street,
        Kind::Indoor,
        Kind::Edge,
        Kind::Label,
        Kind::Lamp,
        Kind::Lantern,
        Kind::Pass,
    ];

    /// Метка в строке (`[крыша] …`), как была у check.py.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Roof => "крыша",
            Kind::Flat => "плашмя",
            Kind::Pipe => "труба",
            Kind::Overlap => "наложение",
            Kind::Door => "дверь",
            Kind::Wall => "стена",
            Kind::Street => "улица",
            Kind::Indoor => "дом",
            Kind::Edge => "край",
            Kind::Label => "подпись",
            Kind::Lamp => "лампа",
            Kind::Lantern => "фонарь",
            Kind::Pass => "проход",
        }
    }

    /// Заголовок группы в окне проверки.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Roof => "Крыши и полёт",
            Kind::Flat => "Вертикальное лежит плашмя",
            Kind::Pipe => "Трубы в никуда",
            Kind::Overlap => "Предметы друг в друге",
            Kind::Door => "Дверь загорожена",
            Kind::Wall => "Предмет поперёк стены",
            Kind::Street => "Уличное в доме",
            Kind::Indoor => "Домашнее на улице",
            Kind::Edge => "За краем карты",
            Kind::Label => "Подпись на предмете",
            Kind::Lamp => "Настенная лампа не у стены",
            Kind::Lantern => "Фонари",
            Kind::Pass => "Проход перегорожен",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub kind: Kind,
    pub text: String,
    /// Куда смотреть на карте.
    pub at: P,
    /// Предметы, о которых речь (номера в `Map::props`).
    pub objects: Vec<usize>,
}

impl Issue {
    pub fn line(&self) -> String {
        format!("[{}] {}", self.kind.tag(), self.text)
    }
}

// --- Слова в именах ---

const ROOF_OK: &[&str] = &["Chimney", "Steam", "Vent", "Hatch", "Skylight", "Roof_A1_", "Roof_A2_", "Water_Tank", "Pipe_Tank", "Pipe_Brass", "Signal_Beacon", "Antenna", "Weathervane"];
/// На фасаде снаружи, у самой стены: настенный фонарь, бра, вывеска, тент над витриной.
const FACADE: &[&str] = &["Wall_Lamp", "Sconce", "Hanging_Sign", "Tarp_Awning"];
const FLYING: &[&str] = &["Airship_Gnome", "Airship_Cargo", "Bird", "Balloon"];
const VERTICAL: &[&str] = &["Clock_Metal_Brass_Face", "Clock_Metal_Brass_Hand", "Wall_Trophy"];
const CONNECT: &[&str] = &["Boiler", "Tank", "Reactor", "Chimney", "Valve", "Manhole", "Gauge", "Gas_Holder", "Mooring_Mast", "Gas_Cylinders"];
/// Подзорная труба бывает и на смотровой площадке — её здесь нет.
const INDOOR: &[&str] = &[
    "Table_Round", "Table_Rectangle", "Bookshelf", "Desk", "Stove", "Oven", "Cupboard", "Chandelier", "Wall_Lamp", "Bar_Wood", "Stool", "Keg", "Shelf_", "Fireplace",
    "Arranged_Bed", "Globe", "Map_", "Hammer_Rack", "Forge", "Anvil", "Scaffolding", "Engine_Steam", "Sail_Table", "Envelope_Frame",
];
/// Лебёдка подъёмника, автожир в мастерской, комнатный цветок, страж-автоматон, рельсы рудника
/// бывают и под крышей — их здесь нет.
const OUTDOOR: &[&str] = &[
    "Lamp_Street", "Mooring_Post", "Windsock", "Landing_Pad", "Signal_Beacon", "Cart_Wood", "Лавка", "Ящик с цветами", "Sign_Post", "MineCart", "Crane_Steam",
    "Mooring_Mast", "Gas_Holder", "Net_01", "Table_Wrought", "Chair_Wrought", "Sign_Wood",
];
/// Decal_ — плашка пола (разметка, помост, ступени зала): это сам пол, на нём лежат и стоят.
const SKIP: &[&str] = &[
    "Steam", "Track_", "Shadow_", "Rug", "Pipe_Brass", "Manhole", "Floor_", "Landing_Pad", "Puddle", "Map_", "Lantern", "Toolbox", "Cattail", "Lilypad", "Reeds_",
    "Skull_Decor", "Decal_", "Gauge", "Propeller_Metal", "Box_Luggage_Lid_Leather_Brown_Metal_Brass_A11", "Valve",
];
/// Части одного устройства: лафет и ствол, мачта и кофель-планка, куски одного ограждения,
/// груз в вагонетке, колесо и ось, вал мехов, кучи руды, шлака, угля, штабель слитков, брусья
/// одной крепи, кресла одного ряда, капсула в гнезде, поклажа на санях, шкура на раме.
const PARTS: &[(&str, &str)] = &[
    ("Cannon_Base", "Cannon_Metal"),
    ("Mast_Wood", "Rigging_Mast"),
    ("Railing_", "Railing_"),
    ("MineCart", "Ore_Minecart_Fill"),
    ("Water_Wheel", "Water_Wheel"),
    ("Bellows", "_Beam_"),
    ("Ore_Pile", "Ore_Pile"),
    ("Slag_", "Slag_"),
    ("Coal_", "Coal_"),
    ("Ingot_", "Ingot_"),
    ("_Beam_", "_Beam_"),
    ("_Seat_", "_Seat_"),
    ("Cryo_Dock", "Cryo_Capsule"),
    ("Dog_Sled_Wood", "Dog_Sled_Cargo"),
    ("Drying_Rack", "Fur_Pelt"),
];
/// Завал: обломки камня, балки, лом и брёвна штабелем лежат грудой — так и задумано.
const HEAP: &[&str] = &["Rubble_", "Debris_", "Scrap_Metal", "Beam_Metal", "IBeam_Metal", "Fir_Log_Snowy_B"];
/// Кладут поверх другого: сеть на груз, цепь на ящик, подстилку в клетку, саван на доску, покров на алтарь.
const ON_TOP: &[&str] = &["Net_01", "Chain_Metal", "Bedroll", "Shroud", "Bandage", "Candle", "Tang_Metal_Molten", "Altar_Runner"];
const DOOR_FLAT: &[&str] = &["Rug", "Door_Mat", "Steam", "Track_", "Manhole", "Floor_", "Landing_Pad", "Propeller_Wood", "Wall_Lamp", "Alarm_Light"];
/// Крона над стеной — не предмет поперёк неё; труба входит в дом сквозь стену.
const ACROSS_OK: &[&str] = &["Wall_Lamp", "Notice_Board", "Propeller_Wood", "Landing_Pad", "Fireplace", "Oven_Brick", "Pipe_Brass", "Track_", "_Beam_"];
/// Не препятствие в проходе: плоское, над головой, растения, переходы между этажами, зев в обрыве.
const P_FLAT: &[&str] = &[
    "Rug", "Door_Mat", "Steam", "Track_", "Manhole", "Floor_", "Landing_Pad", "Propeller_Wood", "Wall_Lamp", "Alarm_Light", "Decal_", "Shadow_", "Puddle", "Stairs",
    "Ladder", "Hatch", "Trapdoor", "Tree", "Дерево", "Bush", "Reeds_", "Cattail", "Lilypad", "Fern", "Grass", "Pipe_Brass", "Chandelier", "Net_01", "Rope",
    "Chain_Metal", "Cable", "Sunken", "Paper", "Clothes_Line", "Hammock", "Tarp", "Awning", "Banner", "Flag", "Rigging", "Windsock", "Sail_", "Cliff_Entrance",
];
/// Поставлено затем, чтобы загородить (кроме завала): ограда, ворота. В проходе — как стена.
const P_BARRIER: &[&str] = &["Railing_", "Fence", "Gate", "Barricade", "Palisade", "Portcullis"];

// --- Геометрия ---

fn in_poly(p: P, poly: &[P]) -> bool {
    let (x, y) = (p[0], p[1]);
    let mut c = false;
    for i in 0..poly.len() {
        let ([x1, y1], [x2, y2]) = (poly[i], poly[(i + 1) % poly.len()]);
        if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
            c = !c;
        }
    }
    c
}

fn inside_box(p: P, b: [f64; 4], pad: f64) -> bool {
    b[0] - pad <= p[0] && p[0] <= b[2] + pad && b[1] - pad <= p[1] && p[1] <= b[3] + pad
}

fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Рамка вдоль осей вокруг повёрнутого прямоугольника `k` от размера, с полем `pad`.
fn bbox(o: &Obj, k: f64, pad: f64) -> [f64; 4] {
    let a = o.rot.to_radians();
    let (w, h) = (o.w * k, o.h * k);
    let hw = ((w * a.cos()).abs() + (h * a.sin()).abs()) / 2.0 + pad;
    let hh = ((w * a.sin()).abs() + (h * a.cos()).abs()) / 2.0 + pad;
    [o.x - hw, o.y - hh, o.x + hw, o.y + hh]
}

/// Углы видимой части: холст FA с запасом под тень — берётся `k` (обычно 80 %).
fn corners(o: &Obj, k: f64) -> [P; 4] {
    let a = o.rot.to_radians();
    let (w, h) = (o.w * k / 2.0, o.h * k / 2.0);
    let mut out = [[0.0; 2]; 4];
    for (n, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].into_iter().enumerate() {
        let (dx, dy) = (sx * w, sy * h);
        out[n] = [o.x + dx * a.cos() - dy * a.sin(), o.y + dx * a.sin() + dy * a.cos()];
    }
    out
}

fn area2(pts: &[P]) -> f64 {
    (0..pts.len()).map(|i| pts[i][0] * pts[(i + 1) % pts.len()][1] - pts[(i + 1) % pts.len()][0] * pts[i][1]).sum()
}

fn poly_area(pts: &[P]) -> f64 {
    if pts.len() > 2 { area2(pts).abs() / 2.0 } else { 0.0 }
}

/// Пересечение выпуклых многоугольников (Сазерленд — Ходжман); обход любой.
fn clip(subject: &[P], clipper: &[P]) -> Vec<P> {
    let sgn = if area2(clipper) > 0.0 { 1.0 } else { -1.0 };
    let mut out = subject.to_vec();
    for i in 0..clipper.len() {
        let ([ax, ay], [bx, by]) = (clipper[i], clipper[(i + 1) % clipper.len()]);
        let side = |p: P| sgn * ((bx - ax) * (p[1] - ay) - (by - ay) * (p[0] - ax));
        let src = std::mem::take(&mut out);
        for j in 0..src.len() {
            let (p, q) = (src[j], src[(j + 1) % src.len()]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        if out.is_empty() {
            return out;
        }
    }
    out
}

/// Расстояние от точки до отрезка.
fn seg_dist(p: P, a: P, b: P) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if l2 <= 0.0 {
        return dist(p, a);
    }
    let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
    dist(p, [a[0] + t * dx, a[1] + t * dy])
}

/// Целая часть к нулю, как `int()` в Python.
fn trunc(v: f64) -> i64 {
    v as i64
}

/// Отрезки ломаной (замкнутой — с последним к первому).
fn segments(l: &Line) -> Vec<(P, P)> {
    let mut pts = l.points.clone();
    if l.closed && !pts.is_empty() {
        pts.push(pts[0]);
    }
    pts.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Мост, мостки, помост — пол над водой (как `bridge_by_name`): на нём стоят и ходят.
fn is_bridge(low: &str) -> bool {
    low.split(|c: char| !c.is_alphanumeric()).any(|w| w == "bridge" || w == "мост")
}

// --- Сетка: кого сравнивать ---
//
// Проверки сравнивают предмет с предметами, домами, дверями. Перебор всех со всеми на
// 10 000 ящиков шёл почти три минуты, поэтому сначала сетка отбирает соседей — тех,
// чья рамка рядом, — а точное условие остаётся прежним. Порядок соседей — по номеру,
// как в переборе, поэтому и строки выходят в том же порядке, что у check.py.

/// Запас рамок в сетке: она в f32, проверки в f64.
const NEAR_PAD: f64 = 0.01;

fn grid_of(boxes: &[[f64; 4]]) -> crate::grid::Grid {
    let b: Vec<crate::grid::Rect> = boxes.iter().map(|b| [(b[0] - NEAR_PAD) as f32, (b[1] - NEAR_PAD) as f32, (b[2] + NEAR_PAD) as f32, (b[3] + NEAR_PAD) as f32]).collect();
    crate::grid::Grid::new(&b, 8.0)
}

/// Номера рамок сетки возле окна `r`, по возрастанию.
fn near(g: &crate::grid::Grid, r: [f64; 4]) -> Vec<usize> {
    g.near([(r[0] - NEAR_PAD) as f32, (r[1] - NEAR_PAD) as f32, (r[2] + NEAR_PAD) as f32, (r[3] + NEAR_PAD) as f32]).into_iter().map(|i| i as usize).collect()
}

fn poly_box(pts: &[P]) -> [f64; 4] {
    pts.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// Самое большое поле `pad`, с которым проверки спрашивают `roof_at`.
const ROOF_PAD: f64 = 1.0;

struct Ctx<'a> {
    inp: &'a Input,
    low: Vec<String>,
    /// Дома — по крышам: имя и контур.
    rooms: Vec<(String, Vec<P>)>,
    rooms_grid: crate::grid::Grid,
    houses_grid: crate::grid::Grid,
    issues: Vec<Issue>,
}

impl<'a> Ctx<'a> {
    fn new(inp: &'a Input, rooms: Vec<(String, Vec<P>)>) -> Self {
        let rooms_grid = grid_of(&rooms.iter().map(|(_, poly)| poly_box(poly)).collect::<Vec<_>>());
        let houses: Vec<[f64; 4]> = inp
            .houses
            .iter()
            .map(|r| match (&r.points, r.area) {
                (pts, _) if !pts.is_empty() => poly_box(pts),
                (_, Some(a)) => [a[0] - ROOF_PAD, a[1] - ROOF_PAD, a[2] + ROOF_PAD, a[3] + ROOF_PAD],
                // Ни контура, ни рамки — такой дом ничего не накрывает.
                _ => [f64::MAX, f64::MAX, f64::MAX, f64::MAX],
            })
            .collect();
        Ctx { inp, low: inp.objs.iter().map(|o| o.name.to_lowercase()).collect(), rooms, rooms_grid, houses_grid: grid_of(&houses), issues: Vec::new() }
    }

    fn o(&self, k: usize) -> &'a Obj {
        &self.inp.objs[k]
    }

    fn has(&self, k: usize, words: &[&str]) -> bool {
        words.iter().any(|w| self.low[k].contains(&w.to_lowercase()))
    }

    fn bridge(&self, k: usize) -> bool {
        is_bridge(&self.low[k])
    }

    /// Дома, которые могут накрыть точку `p` (по номеру).
    fn houses_near(&self, p: P) -> impl Iterator<Item = &'a House> {
        let houses = &self.inp.houses;
        near(&self.houses_grid, [p[0], p[1], p[0], p[1]]).into_iter().map(move |i| &houses[i])
    }

    fn roof_at(&self, p: P, pad: f64) -> bool {
        debug_assert!(pad <= ROOF_PAD);
        self.houses_near(p).any(|r| if !r.points.is_empty() { in_poly(p, &r.points) } else { r.area.is_some_and(|a| inside_box(p, a, pad)) })
    }

    fn deck_at(&self, p: P) -> bool {
        self.houses_near(p).filter(|r| r.flat).any(|r| if !r.points.is_empty() { in_poly(p, &r.points) } else { r.area.is_some_and(|a| inside_box(p, a, 0.0)) })
    }

    /// Первый (по номеру) дом, в котором точка.
    fn room_of(&self, p: P) -> Option<usize> {
        near(&self.rooms_grid, [p[0], p[1], p[0], p[1]]).into_iter().find(|&i| in_poly(p, &self.rooms[i].1))
    }

    fn say(&mut self, kind: Kind, text: String, at: P, objects: Vec<usize>) {
        self.issues.push(Issue { kind, text, at, objects });
    }

    fn label(&self, k: usize) -> String {
        let o = self.o(k);
        format!("#{} {}", o.index, o.name)
    }
}

/// Все проверки по порядку; порядок и тексты — как у check.py.
pub fn run(inp: &Input) -> Vec<Issue> {
    let mut rooms = Vec::new();
    for r in &inp.houses {
        if !r.points.is_empty() {
            rooms.push((format!("дом {}", r.index), r.points.clone()));
        } else if let Some([x0, y0, x1, y1]) = r.area {
            rooms.push((format!("дом {}", r.index), vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]));
        }
    }
    let mut c = Ctx::new(inp, rooms);
    roofs_and_flight(&mut c);
    for k in 0..inp.objs.len() {
        if c.has(k, VERTICAL) {
            let o = c.o(k);
            c.say(Kind::Flat, c.label(k), [o.x, o.y], vec![o.index]);
        }
    }
    pipes(&mut c);
    overlaps(&mut c);
    doors(&mut c);
    across_walls(&mut c);
    let grid = Grid::new(&c);
    street_and_house(&mut c, &grid);
    edge_and_labels(&mut c);
    lamps(&mut c, &grid);
    passage(&mut c);
    c.issues
}

// 1. Крыши и полёт
fn roofs_and_flight(c: &mut Ctx) {
    for k in 0..c.inp.objs.len() {
        let o = c.o(k);
        let (at, me) = ([o.x, o.y], vec![o.index]);
        if (800..900).contains(&o.layer) {
            if !c.has(k, ROOF_OK) && !c.deck_at(at) {
                c.say(Kind::Roof, format!("{} — такому не место на крыше", c.label(k)), at, me.clone());
            }
            if !c.roof_at(at, 0.1) {
                c.say(Kind::Roof, format!("{} на крыше, но не над крышей ({:.1}, {:.1})", c.label(k), o.x, o.y), at, me.clone());
            }
        }
        // Подъёмный кран не летает, но стрелой выше всего — Картограф ставит его над крышами сам.
        if o.layer >= 900 && !c.has(k, FLYING) && !c.has(k, &["Crane_"]) {
            c.say(Kind::Roof, format!("{} над крышами, а это не летающее", c.label(k)), at, me.clone());
        }
        if c.has(k, FLYING) && !c.has(k, &["Bird_House", "Birdcage"]) && o.layer < 900 {
            c.say(Kind::Roof, format!("{} летающее, а слой {}", c.label(k), o.layer), at, me.clone());
        }
        if o.layer >= 900 && corners(o, 0.8).iter().any(|&q| c.roof_at(q, 0.0)) {
            c.say(Kind::Roof, format!("{} висит над домом — закроет комнату в Foundry", c.label(k)), at, me);
        }
    }
}

// 3. Трубы: концы
fn pipes(c: &mut Ctx) {
    let objs = &c.inp.objs;
    let pipes: Vec<usize> = (0..objs.len()).filter(|&k| objs[k].name.starts_with("Pipe_Brass_Straight")).collect();
    let mut ends: Vec<(usize, P)> = Vec::new();
    for &k in &pipes {
        let o = &objs[k];
        let a = o.rot.to_radians();
        let l = o.h / 2.0;
        let (dx, dy) = (-a.sin() * l, a.cos() * l);
        ends.push((k, [o.x + dx, o.y + dy]));
        ends.push((k, [o.x - dx, o.y - dy]));
    }
    // Колено: при повороте 0 выходы вниз и вправо, на краю клетки.
    for (k, o) in objs.iter().enumerate() {
        if o.name.starts_with("Pipe_Brass_Elbow") {
            let a = o.rot.to_radians();
            for (ux, uy) in [(0.0, 0.5), (0.5, 0.0)] {
                ends.push((k, [o.x + ux * a.cos() - uy * a.sin(), o.y + ux * a.sin() + uy * a.cos()]));
            }
        }
    }
    // Конец лежит на другом куске той же трубы — внахлёст, а не в пустоте.
    let on_pipe = |e: P, q: &Obj| {
        let a = q.rot.to_radians();
        let (ax, ay) = (-a.sin(), a.cos());
        let u = (e[0] - q.x) * ax + (e[1] - q.y) * ay;
        let v = ((e[0] - q.x) * ay - (e[1] - q.y) * ax).abs();
        u.abs() <= q.h / 2.0 + 0.05 && v < 0.2
    };
    // К чему трубу можно подключить — один раз, а не на каждый конец.
    let connect: Vec<usize> = if ends.is_empty() { Vec::new() } else { (0..objs.len()).filter(|&q| c.has(q, CONNECT)).collect() };
    for &(k, e) in &ends {
        if ends.iter().any(|&(q, f)| q != k && dist(e, f) < 0.35) {
            continue;
        }
        if pipes.iter().any(|&q| q != k && on_pipe(e, &objs[q])) {
            continue;
        }
        if connect.iter().any(|&q| q != k && inside_box(e, bbox(&objs[q], 1.0, 0.3), 0.0)) {
            continue;
        }
        let text = format!("#{}: конец ({:.1}, {:.1}) ни к чему не подключён", objs[k].index, e[0], e[1]);
        c.say(Kind::Pipe, text, e, vec![objs[k].index]);
    }
}

/// Куски одного бруса внахлёст (комингс люка, балка): оба — брусья, один поворот, на одной оси.
fn beam_run(c: &Ctx, a: usize, b: usize) -> bool {
    if !(c.has(a, &["_Beam_"]) && c.has(b, &["_Beam_"])) {
        return false;
    }
    let (oa, ob) = (c.o(a), c.o(b));
    if (oa.rot - ob.rot).rem_euclid(180.0).min((ob.rot - oa.rot).rem_euclid(180.0)) > 1.0 {
        return false;
    }
    let t = oa.rot.to_radians(); // при повороте 0 брус идёт вдоль y
    ((ob.x - oa.x) * t.cos() + (ob.y - oa.y) * t.sin()).abs() < 0.05
}

// 4. Наложения одного слоя — по повёрнутым прямоугольникам видимой части, а не по рамкам вдоль
// осей: у кресел дугой и перил по кругу рамки раздуты и пересекаются, хотя картинки нет.
fn overlaps(c: &mut Ctx) {
    let objs = &c.inp.objs;
    let cand: Vec<usize> = (0..objs.len()).filter(|&k| !c.has(k, SKIP) && objs[k].w * objs[k].h >= 1.0 && objs[k].layer < 800 && !objs[k].canopy).collect();
    let boxes: Vec<[f64; 4]> = cand.iter().map(|&k| bbox(&objs[k], 0.8, 0.0)).collect();
    let grid = grid_of(&boxes);
    let mut found = Vec::new();
    for (i, &a) in cand.iter().enumerate() {
        let (oa, ba, pa) = (&objs[a], boxes[i], corners(&objs[a], 0.8));
        // Пары без пересечения рамок отсеялись бы ниже — сетка отсевает их сразу.
        for b in near(&grid, ba).into_iter().filter(|&j| j > i).map(|j| cand[j]) {
            let ob = &objs[b];
            if oa.layer != ob.layer {
                continue;
            }
            // Вещь на помосте — не наложение; два куска моста внахлёст — наложение.
            if c.bridge(a) != c.bridge(b) {
                continue;
            }
            if PARTS.iter().any(|&(x, y)| c.has(a, &[x]) && c.has(b, &[y]) || c.has(a, &[y]) && c.has(b, &[x])) {
                continue;
            }
            if c.has(a, ON_TOP) || c.has(b, ON_TOP) || c.has(a, HEAP) && c.has(b, HEAP) || beam_run(c, a, b) {
                continue;
            }
            // Мелочь на столе (книги, свечи, черепа, ступка) — так и задумано.
            if [a, b].iter().any(|&x| c.has(x, &["Table", "Desk", "Counter", "Bar_"])) && (oa.w * oa.h).min(ob.w * ob.h) <= 2.0 {
                continue;
            }
            let bb = bbox(ob, 0.8, 0.0);
            if ba[2].min(bb[2]) <= ba[0].max(bb[0]) || ba[3].min(bb[3]) <= ba[1].max(bb[1]) {
                continue;
            }
            let pb = corners(ob, 0.8);
            let ix = poly_area(&clip(&pa, &pb));
            let small = poly_area(&pa).min(poly_area(&pb));
            if small > 0.0 && ix / small > 0.3 {
                let text = format!("{} и {} ({:.0}%)", c.label(a), c.label(b), ix / small * 100.0);
                found.push((text, [(oa.x + ob.x) / 2.0, (oa.y + ob.y) / 2.0], vec![oa.index, ob.index]));
            }
        }
    }
    for (text, at, objects) in found {
        c.say(Kind::Overlap, text, at, objects);
    }
}

// 5. Двери: свободный проход
fn doors(c: &mut Ctx) {
    let objs = &c.inp.objs;
    // Предмет мешает двери, если середина его ближе `reach` к створу: сетка — по серединам,
    // окно у двери — на самый большой `reach`.
    let centers: Vec<[f64; 4]> = objs.iter().map(|o| [o.x, o.y, o.x, o.y]).collect();
    let grid = grid_of(&centers);
    let reach_max = objs.iter().map(|o| 0.5 + 0.4 * o.w.min(o.h)).filter(|r| r.is_finite()).fold(0.5, f64::max);
    for d in &c.inp.doors {
        if d.window {
            continue;
        }
        let mid = [(d.a[0] + d.b[0]) / 2.0, (d.a[1] + d.b[1]) / 2.0];
        let window = [d.a[0].min(d.b[0]) - reach_max, d.a[1].min(d.b[1]) - reach_max, d.a[0].max(d.b[0]) + reach_max, d.a[1].max(d.b[1]) + reach_max];
        for (k, o) in near(&grid, window).into_iter().map(|k| (k, &objs[k])) {
            // Проём без створки, заваленный обломками, — завал: проход закрыт намеренно.
            if d.opening && c.has(k, &["Rubble_"]) {
                continue;
            }
            // Плоское на полу (ковёр, разметка, утопленное на дне) и настенная лампа проход не загораживают.
            if o.layer >= 800 || o.layer <= 100 || o.canopy || c.bridge(k) || c.has(k, DOOR_FLAT) || c.has(k, FACADE) {
                continue;
            }
            let reach = 0.5 + 0.4 * o.w.min(o.h);
            if seg_dist([o.x, o.y], d.a, d.b) < reach && o.w * o.h >= 1.0 {
                let text = format!("у двери #{} ({:.1}, {:.1}) стоит {}", d.index, mid[0], mid[1], c.label(k));
                c.issues.push(Issue { kind: Kind::Door, text, at: mid, objects: vec![o.index] });
            }
        }
    }
}

// 6. Предмет через стену дома: углы видимой части по разные стороны стены
fn across_walls(c: &mut Ctx) {
    for k in 0..c.inp.objs.len() {
        let o = c.o(k);
        if o.layer >= 800 || o.canopy || c.bridge(k) || c.has(k, ACROSS_OK) {
            continue;
        }
        if c.has(k, FACADE) && c.roof_at([o.x, o.y], 1.0) {
            continue;
        }
        let mut rs: Vec<Option<usize>> = corners(o, 0.7).iter().map(|&q| c.room_of(q)).collect();
        rs.sort();
        rs.dedup();
        if rs.len() > 1 {
            let names: Vec<String> = rs.iter().map(|r| r.map_or("улица".to_owned(), |i| c.rooms[i].0.clone())).collect();
            c.say(Kind::Wall, format!("{} ({:.1}, {:.1}) стоит поперёк стены: {}", c.label(k), o.x, o.y, names.join(" и ")), [o.x, o.y], vec![o.index]);
        }
    }
}

/// Растр в четверть клетки: стены, двери, лестницы — и что из этого «улица» (заливка от края
/// карты). Обнесённое стенами со всех сторон — палуба под фальшбортом, трюм, двор за оградой — не улица.
struct Grid {
    gw: usize,
    gh: usize,
    solid: Vec<bool>,
    outside: Vec<bool>,
}

const STEP: f64 = 0.25;

impl Grid {
    fn new(c: &Ctx) -> Self {
        let inp = c.inp;
        let (gw, gh) = ((trunc(inp.width / STEP) + 2) as usize, (trunc(inp.height / STEP) + 2) as usize);
        let mut solid = vec![false; gw * gh];
        let mut put = |gx: i64, gy: i64| {
            if gx >= 0 && gy >= 0 && (gx as usize) < gw && (gy as usize) < gh {
                solid[gy as usize * gw + gx as usize] = true;
            }
        };
        let mut line = |a: P, b: P| {
            let n = trunc(dist(a, b) / 0.05).max(1);
            for k in 0..=n {
                let t = k as f64 / n as f64;
                let (gx, gy) = (trunc((a[0] + (b[0] - a[0]) * t) / STEP) + 1, trunc((a[1] + (b[1] - a[1]) * t) / STEP) + 1);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        put(gx + dx, gy + dy);
                    }
                }
            }
        };
        for w in &inp.walls {
            for (a, b) in segments(w) {
                line(a, b);
            }
        }
        for &(a, b) in &inp.room_walls {
            line(a, b);
        }
        // Дверь и проём замыкают стену: помещение за дверью — не улица.
        for d in &inp.doors {
            line(d.a, d.b);
        }
        // Проход в ограде, где стоит трап или лестница, ведёт на другой этаж, а не на улицу.
        for k in 0..inp.objs.len() {
            if c.has(k, &["Ladder", "Stairs", "Trapdoor", "Hatch"]) {
                let b = bbox(c.o(k), 0.8, 0.0);
                for gy in trunc(b[1] / STEP) + 1..trunc(b[3] / STEP) + 2 {
                    for gx in trunc(b[0] / STEP) + 1..trunc(b[2] / STEP) + 2 {
                        if gx >= 0 && gy >= 0 && (gx as usize) < gw && (gy as usize) < gh {
                            solid[gy as usize * gw + gx as usize] = true;
                        }
                    }
                }
            }
        }
        let n = gw * gh;
        let mut outside = vec![false; n];
        let mut todo: Vec<usize> = (0..gw).chain((0..gw).map(|i| (gh - 1) * gw + i)).chain((0..gh).map(|j| j * gw)).chain((0..gh).map(|j| j * gw + gw - 1)).collect();
        for &i in &todo {
            outside[i] = true;
        }
        while let Some(i) = todo.pop() {
            for j in [i.wrapping_sub(1), i + 1, i.wrapping_sub(gw), i + gw] {
                if j < n && !outside[j] && !solid[j] && (j % gw).abs_diff(i % gw) <= 1 {
                    outside[j] = true;
                    todo.push(j);
                }
            }
        }
        Grid { gw, gh, solid, outside }
    }

    fn at(&self, x: f64, y: f64) -> Option<usize> {
        let (gx, gy) = (trunc(x / STEP) + 1, trunc(y / STEP) + 1);
        (gx >= 0 && gy >= 0 && (gx as usize) < self.gw && (gy as usize) < self.gh).then(|| gy as usize * self.gw + gx as usize)
    }
}

// 7. Уличное не в домах, домашнее не на улице
fn street_and_house(c: &mut Ctx, g: &Grid) {
    let (w, h) = (c.inp.width, c.inp.height);
    let walled = |x: f64, y: f64| (0.0..w).contains(&x) && (0.0..h).contains(&y) && g.at(x, y).is_some_and(|i| !g.outside[i]);
    for k in 0..c.inp.objs.len() {
        let o = c.o(k);
        if o.layer >= 800 {
            continue;
        }
        let r = c.room_of([o.x, o.y]);
        if let Some(r) = r
            && c.has(k, OUTDOOR)
        {
            c.say(Kind::Street, format!("{} внутри: {}", c.label(k), c.rooms[r].0), [o.x, o.y], vec![o.index]);
        }
        if r.is_none() && c.has(k, INDOOR) && !walled(o.x, o.y) && !(c.has(k, FACADE) && c.roof_at([o.x, o.y], 1.0)) {
            c.say(Kind::Indoor, format!("{} на улице ({:.1}, {:.1})", c.label(k), o.x, o.y), [o.x, o.y], vec![o.index]);
        }
    }
}

// 8. За краем карты; 9. Подписи не на предметах
fn edge_and_labels(c: &mut Ctx) {
    let (w, h) = (c.inp.width, c.inp.height);
    for k in 0..c.inp.objs.len() {
        // Крона за краем карты — нормально; рельсы и темнота штольни уходят за край — рудник там продолжается.
        if c.has(k, &["Tree", "Дерево"]) || c.o(k).canopy || c.has(k, &["Track_", "Shadow_"]) {
            continue;
        }
        let b = bbox(c.o(k), 0.8, 0.0);
        if b[0] < -0.2 || b[1] < -0.2 || b[2] > w + 0.2 || b[3] > h + 0.2 {
            let o = c.o(k);
            c.say(Kind::Edge, format!("{} вылезает за карту", c.label(k)), [o.x.clamp(0.0, w), o.y.clamp(0.0, h)], vec![o.index]);
        }
    }
    // Подпись лежит на предмете, если в её рамке угол видимой части — он внутри рамки предмета.
    let grid = if c.inp.labels.is_empty() { crate::grid::Grid::default() } else { grid_of(&c.inp.objs.iter().map(|o| bbox(o, 0.8, 0.0)).collect::<Vec<_>>()) };
    for l in &c.inp.labels {
        let (hw, hh) = (l.text.chars().count() as f64 * l.size * 0.29, l.size * 0.6);
        let lb = [l.x - hw, l.y - hh, l.x + hw, l.y + hh];
        for k in near(&grid, lb) {
            let o = c.o(k);
            if o.layer >= 800 || c.has(k, &["Track_", "Steam", "Pipe_Brass"]) {
                continue;
            }
            if corners(o, 0.8).iter().any(|&q| inside_box(q, lb, 0.0)) && o.w * o.h >= 1.0 {
                let text = format!("«{}» лежит на {}", l.text, c.label(k));
                c.issues.push(Issue { kind: Kind::Label, text, at: [l.x, l.y], objects: vec![o.index] });
            }
        }
    }
}

// 10. Настенная лампа — у стены; 11, 12. Фонари
fn lamps(c: &mut Ctx, g: &Grid) {
    let inp = c.inp;
    let mut segs: Vec<(P, P)> = c.rooms.iter().flat_map(|(_, poly)| (0..poly.len()).map(move |i| (poly[i], poly[(i + 1) % poly.len()]))).collect();
    segs.extend(inp.walls.iter().flat_map(segments));
    segs.extend(inp.room_walls.iter().copied());
    for k in 0..inp.objs.len() {
        let o = c.o(k);
        if c.has(k, &["Wall_Lamp"]) && !segs.iter().any(|&(a, b)| seg_dist([o.x, o.y], a, b) < 0.7) {
            c.say(Kind::Lamp, format!("#{} настенная лампа не у стены", o.index), [o.x, o.y], vec![o.index]);
        }
    }
    let (w, h) = (inp.width, inp.height);
    let blocked = |x: f64, y: f64| {
        if !((0.0..w).contains(&x) && (0.0..h).contains(&y)) {
            return true;
        }
        g.at(x, y).is_some_and(|i| g.solid[i]) || c.roof_at([x, y], 0.0) || c.room_of([x, y]).is_some()
    };
    let mut found = Vec::new();
    // 11. Фонарь FA `Lamp_Post` — брус с фонарём на конце, сверху: низ картинки — конец бруса, он
    // упирается в стену или ограду. Брус посреди улицы читается упавшим столбом.
    for k in 0..inp.objs.len() {
        let o = c.o(k);
        if !c.has(k, &["Lamp_Post"]) {
            continue;
        }
        let a = o.rot.to_radians();
        let (ux, uy) = (-a.sin(), a.cos());
        let end = 0.45;
        if ![-0.1, 0.05, 0.2, 0.35].iter().any(|s| blocked(o.x + ux * (end + s), o.y + uy * (end + s))) {
            found.push((format!("{} ({:.1}, {:.1}) кронштейн не упирается в стену", c.label(k), o.x, o.y), [o.x, o.y], o.index));
        }
    }
    // 12. Фонарный столб — у края прохода, а не посреди него: не дальше клетки от стены, дома или
    // ограды. Проход — где до краёв по одной из осей с обеих сторон не больше 8 клеток; на площади
    // шире 8 клеток в обе стороны столб может стоять и в середине.
    let free = |x: f64, y: f64, dx: f64, dy: f64| {
        let mut s = 0.25;
        while s <= 8.0 {
            if blocked(x + dx * s, y + dy * s) {
                return Some(s);
            }
            s += 0.25;
        }
        None
    };
    for k in 0..inp.objs.len() {
        let o = c.o(k);
        if !c.has(k, &["Lamp_Post", "Lamp_Street", "Street_Lamp", "Lamppost"]) {
            continue;
        }
        let (mut spans, mut dists) = (Vec::new(), Vec::new());
        for (dx, dy, axis) in [(1.0, 0.0, "x"), (0.0, 1.0, "y")] {
            let (a, b) = (free(o.x, o.y, -dx, -dy), free(o.x, o.y, dx, dy));
            dists.extend([a, b].into_iter().flatten());
            if let (Some(a), Some(b)) = (a, b) {
                spans.push((a + b, axis, a, b));
            }
        }
        let near = dists.iter().copied().fold(f64::INFINITY, f64::min);
        if spans.is_empty() || near <= 1.0 {
            continue;
        }
        spans.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
        let (span, axis, lo, hi) = spans[0];
        found.push((
            format!("{} ({:.1}, {:.1}) посреди прохода шириной {span:.1}: до краёв по {axis} {lo:.2} и {hi:.2}, до ближайшего края {near:.2}", c.label(k), o.x, o.y),
            [o.x, o.y],
            o.index,
        ));
    }
    for (text, at, i) in found {
        c.say(Kind::Lantern, text, at, vec![i]);
    }
}

// 13. Проход: предметы не перегораживают коридор, выход из комнаты, улочку, галерею. Фишка в
// клетку должна пройти мимо — или обойти не дальше DETOUR клеток. Дальний обход по кольцу не в
// счёт: инкубатор посреди коридора «мешает пройти», хотя кругом дойти можно. Затор — когда по обе
// стороны от предметов есть куда идти: дверь либо место от BIG клеток. Угол обжитой комнаты, куда
// не протиснуться мимо стола, — не затор. И ряд предметов, что стоит сам по себе, не касаясь стен
// (грядки и брёвна на поляне, стол посреди зала), — тоже: его обходят. Двери и проёмы открыты,
// окно — стена. Сетка в 1/10 клетки; центр фишки не ближе RW к линии стены и RO к видимой части
// предмета: между двумя предметами нужно 0,8 клетки, между предметом и стеной — 0,7.
const PS: f64 = 0.1;
const RW: f64 = 0.3;
const RO: f64 = 0.4;
const MARG: f64 = 1.2;
const DETOUR: f64 = 8.0;
const BIG: f64 = 3.0;
/// За краем карты — поле: край карты не стена, у леса и улицы за ним то же самое.
const PAD: f64 = 2.0;

struct Pass {
    pw: usize,
    ph: usize,
    /// Точку закрывает стена или то, что поставлено загородить.
    wall: Vec<bool>,
    /// Точку закрывает стена или предмет.
    blk: Vec<bool>,
    /// Через точку идёт сама линия стены.
    line: Vec<bool>,
    door: Vec<bool>,
    /// Точка → предметы (номера в карте), которые её закрывают.
    own: HashMap<usize, Vec<usize>>,
    cells: HashMap<usize, Vec<usize>>,
    group: HashMap<usize, usize>,
    /// Края карты в точках сетки.
    on_map: [usize; 4],
    min_part: usize,
    big_part: usize,
    far: u32,
    /// Метки обхода: точка пройдена, если `stamp[c] == round`.
    stamp: Vec<u32>,
    step: Vec<u32>,
    round: u32,
}

impl Pass {
    fn span(&self, lo: f64, hi: f64, n: usize) -> std::ops::RangeInclusive<usize> {
        let a = trunc((lo + PAD) / PS).max(0) as usize;
        let b = trunc((hi + PAD) / PS).min(n as i64 - 1);
        if b < a as i64 { 1..=0 } else { a..=b as usize }
    }

    fn cell(&self, x: f64, y: f64) -> usize {
        let j = trunc((y + PAD) / PS).clamp(0, self.ph as i64 - 1) as usize;
        let i = trunc((x + PAD) / PS).clamp(0, self.pw as i64 - 1) as usize;
        j * self.pw + i
    }

    fn near(&self, c: usize) -> impl Iterator<Item = usize> {
        let (pw, n, i) = (self.pw, self.pw * self.ph, c % self.pw);
        [(i > 0).then(|| c - 1), (i < pw - 1).then(|| c + 1), c.checked_sub(pw), Some(c + pw)].into_iter().flatten().filter(move |&q| q < n)
    }

    fn on_map(&self, c: usize) -> bool {
        let (i, j) = (c % self.pw, c / self.pw);
        self.on_map[0] <= i && i < self.on_map[2] && self.on_map[1] <= j && j < self.on_map[3]
    }

    /// Точки не дальше `reach` от отрезка — с квадратом расстояния.
    fn seg(&self, a: P, b: P, reach: f64) -> Vec<(usize, f64)> {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = if dx * dx + dy * dy == 0.0 { 1e-9 } else { dx * dx + dy * dy };
        let mut out = Vec::new();
        for j in self.span(a[1].min(b[1]) - reach, a[1].max(b[1]) + reach, self.ph) {
            let cy = (j as f64 + 0.5) * PS - PAD;
            for i in self.span(a[0].min(b[0]) - reach, a[0].max(b[0]) + reach, self.pw) {
                let cx = (i as f64 + 0.5) * PS - PAD;
                let t = (((cx - a[0]) * dx + (cy - a[1]) * dy) / l2).clamp(0.0, 1.0);
                let (ex, ey) = (cx - (a[0] + t * dx), cy - (a[1] + t * dy));
                if ex * ex + ey * ey < reach * reach {
                    out.push((j * self.pw + i, ex * ex + ey * ey));
                }
            }
        }
        out
    }

    fn mark_wall(&mut self, cells: Vec<(usize, f64)>) {
        for (c, d2) in cells {
            self.wall[c] = true;
            self.blk[c] = true;
            if d2 < 0.5625 * PS * PS {
                self.line[c] = true;
            }
        }
    }

    /// Стена без кусков под дверями и проёмами. Контур комнат идёт под дверью сплошь, у толстой
    /// стены — обеими гранями: вырезается всё, что лежит вдоль двери не дальше 0,75 клетки от неё.
    fn wall_seg(&mut self, a: P, b: P, open: &[&Door]) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy);
        if l < 1e-6 {
            return;
        }
        let (ux, uy) = (dx / l, dy / l);
        let mut gaps = Vec::new();
        for d in open {
            let ([ax, ay], [bx, by]) = (d.a, d.b);
            let dl = (bx - ax).hypot(by - ay);
            let dl = if dl == 0.0 { 1e-9 } else { dl };
            if (ux * (by - ay) - uy * (bx - ax)).abs() / dl > 0.2 {
                continue; // дверь не вдоль этой стены
            }
            if ((ax - a[0]) * uy - (ay - a[1]) * ux).abs().max(((bx - a[0]) * uy - (by - a[1]) * ux).abs()) > 0.75 {
                continue;
            }
            let (ta, tb) = ((ax - a[0]) * ux + (ay - a[1]) * uy, (bx - a[0]) * ux + (by - a[1]) * uy);
            let (ta, tb) = (ta.min(tb), ta.max(tb));
            if tb > 0.0 && ta < l {
                gaps.push((ta.max(0.0), tb.min(l)));
            }
        }
        gaps.sort_by(|p, q| p.partial_cmp(q).unwrap_or(std::cmp::Ordering::Equal));
        gaps.push((l, l));
        let mut at = 0.0f64;
        for (ta, tb) in gaps {
            if ta - at > 1e-6 {
                let cells = self.seg([a[0] + ux * at, a[1] + uy * at], [a[0] + ux * ta, a[1] + uy * ta], RW);
                self.mark_wall(cells);
            }
            at = at.max(tb);
        }
    }

    fn top(&mut self, k: usize) -> usize {
        let mut k = k;
        while self.group[&k] != k {
            let up = self.group[&self.group[&k]];
            self.group.insert(k, up);
            k = up;
        }
        k
    }

    fn fresh(&mut self) -> u32 {
        self.round += 1;
        self.round
    }

    /// Куда фишка дойдёт от куска свободного места не дальше `far` шагов; `None` — дошла до `stop`.
    fn side(&mut self, src: &[usize], stop: Option<(&HashMap<usize, usize>, usize)>) -> Option<Vec<usize>> {
        let g = self.fresh();
        let mut todo: Vec<usize> = Vec::with_capacity(src.len() * 4);
        for &c in src {
            if self.stamp[c] != g {
                self.stamp[c] = g;
                self.step[c] = 0;
                todo.push(c);
            }
        }
        let mut k = 0;
        while k < todo.len() {
            let c = todo[k];
            k += 1;
            if stop.is_some_and(|(part, s)| part.get(&c) == Some(&s)) {
                return None;
            }
            if self.step[c] >= self.far {
                continue;
            }
            let next = self.step[c] + 1;
            let near: Vec<usize> = self.near(c).collect();
            for q in near {
                if !self.blk[q] && self.stamp[q] != g {
                    self.stamp[q] = g;
                    self.step[q] = next;
                    todo.push(q);
                }
            }
        }
        Some(todo)
    }

    /// Касается ли препятствие, в которое входят эти предметы, стены: остров посреди поляны — нет.
    fn walled(&mut self, group: &[usize]) -> bool {
        let g = self.fresh();
        let mut todo = Vec::new();
        for k in group {
            for &c in &self.cells[k] {
                if self.stamp[c] != g {
                    self.stamp[c] = g;
                    todo.push(c);
                }
            }
        }
        while let Some(c) = todo.pop() {
            if self.wall[c] {
                return true;
            }
            let near: Vec<usize> = self.near(c).collect();
            for q in near {
                if self.blk[q] && self.stamp[q] != g {
                    self.stamp[q] = g;
                    todo.push(q);
                }
            }
        }
        false
    }

    fn goes(&self, side: &[usize]) -> bool {
        side.iter().any(|&c| self.door[c]) || side.iter().filter(|&&c| self.on_map(c)).count() >= self.big_part
    }

    /// Перегородили ли эти предметы проход: убери их — и куски свободного места рядом соединятся,
    /// а в обход между ними далеко или никак, и по обе стороны есть куда идти.
    fn blocks(&mut self, group: &[usize]) -> bool {
        let mut own: Vec<usize> = Vec::new();
        for k in group {
            for &c in &self.cells[k] {
                if !self.wall[c] && self.own.get(&c).is_some_and(|ks| ks.iter().all(|q| group.contains(q))) {
                    own.push(c);
                }
            }
        }
        own.sort_unstable();
        own.dedup();
        if own.is_empty() || !self.walled(group) {
            return false;
        }
        let m = trunc(MARG / PS) as usize;
        let i0 = own.iter().map(|c| c % self.pw).min().unwrap_or(0).saturating_sub(m);
        let i1 = (own.iter().map(|c| c % self.pw).max().unwrap_or(0) + m).min(self.pw - 1);
        let j0 = own.iter().map(|c| c / self.pw).min().unwrap_or(0).saturating_sub(m);
        let j1 = (own.iter().map(|c| c / self.pw).max().unwrap_or(0) + m).min(self.ph - 1);
        let pw = self.pw;
        let inside = |q: usize| (i0..=i1).contains(&(q % pw)) && (j0..=j1).contains(&(q / pw));
        let mut part: HashMap<usize, usize> = HashMap::new();
        let mut parts: Vec<Vec<usize>> = Vec::new();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let c0 = j * pw + i;
                if self.blk[c0] || part.contains_key(&c0) {
                    continue;
                }
                let n = parts.len();
                part.insert(c0, n);
                let (mut todo, mut cells) = (vec![c0], Vec::new());
                while let Some(c) = todo.pop() {
                    cells.push(c);
                    for q in self.near(c) {
                        if inside(q) && !self.blk[q] && !part.contains_key(&q) {
                            part.insert(q, n);
                            todo.push(q);
                        }
                    }
                }
                parts.push(cells);
            }
        }
        if parts.iter().filter(|p| p.len() >= self.min_part).count() < 2 {
            return false;
        }
        // Идём только по точкам, которые закрывают одни эти предметы, и смотрим, каких кусков они касаются.
        let own_set: HashSet<usize> = own.iter().copied().collect();
        let mut seen: HashSet<usize> = HashSet::new();
        let mut sides: HashMap<usize, Vec<usize>> = HashMap::new();
        for &c0 in &own {
            if !seen.insert(c0) {
                continue;
            }
            let (mut todo, mut touched) = (vec![c0], HashSet::new());
            while let Some(c) = todo.pop() {
                for q in self.near(c) {
                    if seen.contains(&q) || !inside(q) {
                        continue;
                    }
                    if !self.blk[q] {
                        touched.insert(part[&q]);
                    } else if own_set.contains(&q) {
                        seen.insert(q);
                        todo.push(q);
                    }
                }
            }
            let mut big: Vec<usize> = touched.into_iter().filter(|&k| parts[k].len() >= self.min_part).collect();
            big.sort_by_key(|&k| (std::cmp::Reverse(parts[k].len()), k));
            for &k in big.iter().skip(1) {
                let Some(a) = self.side(&parts[big[0]], Some((&part, k))) else {
                    continue; // обход рядом
                };
                if !sides.contains_key(&k) {
                    let s = self.side(&parts[k], None).unwrap_or_default();
                    sides.insert(k, s);
                }
                if self.goes(&a) && self.goes(&sides[&k]) {
                    return true;
                }
            }
        }
        false
    }
}

/// Видимая часть предмета: центр, полуразмеры, косинус и синус поворота. Без `vis` — 80 % холста
/// (у FA вокруг предмета поля под тень).
fn p_rect(o: &Obj) -> (f64, f64, f64, f64, f64, f64) {
    let a = o.rot.to_radians();
    let (ca, sa) = (a.cos(), a.sin());
    if let Some([l, t, r, b]) = o.vis {
        let (mx, my) = ((l + r) / 2.0, (t + b) / 2.0);
        return (o.x + mx * ca - my * sa, o.y + mx * sa + my * ca, (r - l) / 2.0, (b - t) / 2.0, ca, sa);
    }
    (o.x, o.y, o.w * 0.4, o.h * 0.4, ca, sa)
}

fn passage(c: &mut Ctx) {
    let inp = c.inp;
    let (w, h) = (inp.width, inp.height);
    let (pw, ph) = (((w + 2.0 * PAD) / PS).round() as usize, ((h + 2.0 * PAD) / PS).round() as usize);
    if pw == 0 || ph == 0 {
        return;
    }
    let n = pw * ph;
    let mut p = Pass {
        pw,
        ph,
        wall: vec![false; n],
        blk: vec![false; n],
        line: vec![false; n],
        door: vec![false; n],
        own: HashMap::new(),
        cells: HashMap::new(),
        group: HashMap::new(),
        on_map: [trunc(PAD / PS) as usize, trunc(PAD / PS) as usize, trunc((w + PAD) / PS) as usize, trunc((h + PAD) / PS) as usize],
        min_part: trunc(0.2 / PS / PS) as usize,
        big_part: trunc(BIG / PS / PS) as usize,
        far: trunc(DETOUR / PS) as u32,
        stamp: vec![0; n],
        step: vec![0; n],
        round: 0,
    };
    let open: Vec<&Door> = inp.doors.iter().filter(|d| !d.window).collect();
    for l in &inp.walls {
        for (a, b) in segments(l) {
            p.wall_seg(a, b, &open);
        }
    }
    for &(a, b) in &inp.room_walls {
        p.wall_seg(a, b, &open);
    }
    // Перила помоста — как стена: на помост и с него — только по лестнице.
    for line in &inp.rails {
        for s in line.windows(2) {
            let cells = p.seg(s[0], s[1], RW);
            p.mark_wall(cells);
        }
    }
    for d in &open {
        for (q, _) in p.seg(d.a, d.b, 0.45) {
            p.door[q] = true;
        }
    }
    let mut order = Vec::new(); // предметы прохода в порядке карты
    let mut by_index: HashMap<usize, usize> = HashMap::new();
    for k in 0..inp.objs.len() {
        let o = c.o(k);
        if !solid_obj(c, k) {
            continue;
        }
        let cells = obj_cells(&p, o);
        if c.has(k, HEAP) || c.has(k, P_BARRIER) {
            for &q in &cells {
                p.wall[q] = true;
                p.blk[q] = true;
            }
            continue;
        }
        let i = o.index;
        order.push(i);
        by_index.insert(i, k);
        p.group.insert(i, i);
        for &q in &cells {
            p.blk[q] = true;
            if let Some(first) = p.own.get(&q).map(|v| v[0]) {
                let (a, b) = (p.top(first), p.top(i));
                p.group.insert(a, b);
                p.own.get_mut(&q).map(|v| v.push(i));
            } else {
                p.own.insert(q, vec![i]);
            }
        }
        p.cells.insert(i, cells);
    }
    // Свободное место целиком: куски, между которыми фишке не пройти.
    let mut glab = vec![usize::MAX; n];
    let mut gsize = Vec::new();
    for c0 in 0..n {
        if p.blk[c0] || glab[c0] != usize::MAX {
            continue;
        }
        let id = gsize.len();
        glab[c0] = id;
        let (mut todo, mut cnt) = (vec![c0], 0usize);
        while let Some(q0) = todo.pop() {
            cnt += 1;
            for q in p.near(q0) {
                if !p.blk[q] && glab[q] == usize::MAX {
                    glab[q] = id;
                    todo.push(q);
                }
            }
        }
        gsize.push(cnt);
    }
    // Закуток, куда не попасть (щель между опрокинутой капсулой и дроном), — не проход: он закрыт
    // теми предметами, что его окружают, и связывает их в одно препятствие. Закуток с дверью — чулан.
    let mut nooks: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut nook_of: HashMap<usize, usize> = HashMap::new();
    for q in 0..n {
        if !p.blk[q] && gsize[glab[q]] < p.big_part {
            let at = *nook_of.entry(glab[q]).or_insert_with(|| {
                nooks.push((glab[q], Vec::new()));
                nooks.len() - 1
            });
            nooks[at].1.push(q);
        }
    }
    for (_, cells) in nooks {
        if cells.iter().any(|&q| p.door[q]) {
            continue;
        }
        let mut around: Vec<usize> = cells.iter().flat_map(|&q| p.near(q).collect::<Vec<_>>()).filter_map(|q| p.own.get(&q).cloned()).flatten().collect();
        around.sort_unstable();
        around.dedup();
        for &q in &cells {
            p.blk[q] = true;
            if around.is_empty() {
                p.wall[q] = true;
            } else {
                p.own.insert(q, around.clone());
            }
        }
        for &k in &around {
            p.cells.get_mut(&k).map(|v| v.extend_from_slice(&cells));
            let (a, b) = (p.top(k), p.top(around[0]));
            p.group.insert(a, b);
        }
    }
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for &i in &order {
        let t = p.top(i);
        match groups.iter_mut().find(|(g, _)| *g == t) {
            Some((_, v)) => v.push(i),
            None => groups.push((t, vec![i])),
        }
    }
    let mut found: Vec<Vec<usize>> = Vec::new();
    for (_, group) in groups {
        let alone: Vec<usize> = group.iter().copied().filter(|&k| p.blocks(&[k])).collect();
        for &k in &alone {
            found.push(vec![k]);
        }
        if !alone.is_empty() || group.len() < 2 || !p.blocks(&group) {
            continue;
        }
        // Кто именно: убираем из связки по одному всех, без кого проход остаётся закрыт.
        let mut need = group.clone();
        need.sort_unstable();
        for k in need.clone() {
            let rest: Vec<usize> = need.iter().copied().filter(|&q| q != k).collect();
            if !rest.is_empty() && p.blocks(&rest) {
                need = rest;
            }
        }
        found.push(need);
    }
    for group in found {
        let who: Vec<String> = group.iter().take(4).map(|i| {
            let o = c.o(by_index[i]);
            format!("#{} {} ({:.1}, {:.1})", o.index, o.name, o.x, o.y)
        }).collect();
        let more = if group.len() > 4 { format!(" и ещё {}", group.len() - 4) } else { String::new() };
        let how = if group.len() == 1 { "перегораживает" } else { "вместе перегораживают" };
        let first = c.o(by_index[&group[0]]);
        let text = format!("{}{more} {how} проход: мимо фишке в клетку не пройти, обход дальше {DETOUR:.0} клеток или его нет", who.join(" и "));
        c.say(Kind::Pass, text, [first.x, first.y], group);
    }
}

/// Стоит на полу и выше щиколотки: мебель, ящики, станки, стулья. Мелочь на полу (слой 200) — нет.
fn solid_obj(c: &Ctx, k: usize) -> bool {
    let o = c.o(k);
    if !(250 < o.layer && o.layer < 700) || o.canopy || c.bridge(k) || c.has(k, P_FLAT) || c.has(k, FACADE) {
        return false;
    }
    let (_, _, hw, hh, _, _) = p_rect(o);
    hw.min(hh) >= 0.22 && (0.25..=60.0).contains(&(hw * hh * 4.0))
}

/// Точки, куда центр фишки не встанет из-за предмета: видимая часть со скруглёнными углами
/// (или по маске — круглое, подкова, колбы по углам обходятся по рисунку). Только по свою сторону
/// стены: купол у стены зала не сужает коридор за ней.
fn obj_cells(p: &Pass, o: &Obj) -> Vec<usize> {
    let (x, y, hw, hh, ca, sa) = p_rect(o);
    let rc = 0.3 * hw.min(hh);
    let reach = hw.hypot(hh) + RO;
    let mut cells: HashSet<usize> = HashSet::new();
    if let (Some(mask), Some([l, t, r, b])) = (&o.mask, o.vis) {
        let a = o.rot.to_radians();
        let (oca, osa) = (a.cos(), a.sin());
        let (cw, ch) = ((r - l) / 16.0, (b - t) / 16.0);
        let (hx, hy) = (cw / 2.0, ch / 2.0);
        let ext = hx.hypot(hy) + RO;
        for (v, row) in mask.iter().enumerate() {
            for u in 0..16 {
                if row >> u & 1 == 0 {
                    continue;
                }
                let (mx, my) = (l + (u as f64 + 0.5) * cw, t + (v as f64 + 0.5) * ch);
                let (wx, wy) = (o.x + mx * oca - my * osa, o.y + mx * osa + my * oca);
                for j in p.span(wy - ext, wy + ext, p.ph) {
                    let dy = (j as f64 + 0.5) * PS - PAD - wy;
                    for i in p.span(wx - ext, wx + ext, p.pw) {
                        let dx = (i as f64 + 0.5) * PS - PAD - wx;
                        let (lx, ly) = ((dx * oca + dy * osa).abs() - hx, (dy * oca - dx * osa).abs() - hy);
                        if lx.max(0.0).hypot(ly.max(0.0)) < RO {
                            cells.insert(j * p.pw + i);
                        }
                    }
                }
            }
        }
    } else {
        for j in p.span(y - reach, y + reach, p.ph) {
            let dy = (j as f64 + 0.5) * PS - PAD - y;
            for i in p.span(x - reach, x + reach, p.pw) {
                let dx = (i as f64 + 0.5) * PS - PAD - x;
                let (lx, ly) = ((dx * ca + dy * sa).abs() - (hw - rc), (dy * ca - dx * sa).abs() - (hh - rc));
                if lx.max(0.0).hypot(ly.max(0.0)) < RO + rc {
                    cells.insert(j * p.pw + i);
                }
            }
        }
    }
    let c0 = p.cell(x, y);
    if !cells.contains(&c0) || p.line[c0] {
        let mut v: Vec<usize> = cells.into_iter().collect();
        v.sort_unstable();
        return v; // середина на самой стене: предмет стоит поперёк неё
    }
    let (mut seen, mut todo) = (HashSet::from([c0]), vec![c0]);
    while let Some(q0) = todo.pop() {
        for q in p.near(q0) {
            if cells.contains(&q) && seen.insert(q) && !p.line[q] {
                todo.push(q);
            }
        }
    }
    let mut v: Vec<usize> = seen.into_iter().collect();
    v.sort_unstable();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(index: usize, name: &str, x: f64, y: f64, w: f64, h: f64, layer: i32) -> Obj {
        Obj { index, name: name.into(), x, y, w, h, layer, ..Default::default() }
    }

    fn kinds(inp: &Input) -> Vec<Kind> {
        run(inp).iter().map(|i| i.kind).collect()
    }

    fn house(index: usize, area: [f64; 4]) -> House {
        House { index, area: Some(area), ..Default::default() }
    }

    #[test]
    fn empty_map_is_clean() {
        assert!(run(&Input { width: 20.0, height: 20.0, ..Default::default() }).is_empty());
    }

    #[test]
    fn roof_things_belong_on_roofs() {
        let mut inp = Input { width: 20.0, height: 20.0, houses: vec![house(0, [2.0, 2.0, 8.0, 8.0])], ..Default::default() };
        inp.objs = vec![obj(0, "Chimney_Brick_A", 5.0, 5.0, 1.0, 1.0, 850), obj(1, "Crate_Wood_A", 5.0, 6.0, 1.0, 1.0, 850), obj(2, "Chimney_Brick_A", 12.0, 12.0, 1.0, 1.0, 850)];
        let out = run(&inp);
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].text.contains("#1") && out[0].text.contains("не место на крыше"));
        assert!(out[1].text.contains("#2") && out[1].text.contains("не над крышей"));
        assert_eq!(out[0].objects, vec![1]);
    }

    #[test]
    fn flying_over_a_house_hides_the_room() {
        let mut inp = Input { width: 20.0, height: 20.0, houses: vec![house(0, [2.0, 2.0, 8.0, 8.0])], ..Default::default() };
        inp.objs = vec![obj(0, "Airship_Cargo_A", 5.0, 5.0, 3.0, 2.0, 950), obj(1, "Airship_Cargo_A", 15.0, 15.0, 3.0, 2.0, 950)];
        let out = run(&inp);
        assert_eq!(out.len(), 1, "{out:?}");
        assert!(out[0].text.contains("закроет комнату"));
    }

    #[test]
    fn pipe_end_in_the_air() {
        let mut inp = Input { width: 20.0, height: 20.0, ..Default::default() };
        inp.objs = vec![obj(0, "Pipe_Brass_Straight_A", 5.0, 5.0, 0.5, 2.0, 300), obj(1, "Boiler_Steam_A", 5.0, 7.0, 2.0, 2.0, 300)];
        let out: Vec<Issue> = run(&inp).into_iter().filter(|i| i.kind == Kind::Pipe).collect();
        assert_eq!(out.len(), 1, "верхний конец в пустоте, нижний — у котла: {out:?}");
        assert_eq!(out[0].at, [5.0, 4.0]);
    }

    #[test]
    fn overlap_but_not_a_book_on_a_table() {
        let mut inp = Input { width: 20.0, height: 20.0, ..Default::default() };
        inp.objs = vec![obj(0, "Crate_Wood_A", 5.0, 5.0, 2.0, 2.0, 300), obj(1, "Barrel_Wood_A", 5.3, 5.2, 2.0, 2.0, 300), obj(2, "Table_Round_A", 12.0, 12.0, 2.0, 2.0, 300), obj(3, "Book_A", 12.0, 12.0, 1.0, 1.0, 300)];
        let out: Vec<Issue> = run(&inp).into_iter().filter(|i| i.kind == Kind::Overlap).collect();
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].objects, vec![0, 1]);
        assert!(out[0].text.ends_with("%)"));
    }

    #[test]
    fn door_blocked_by_a_crate() {
        let mut inp = Input { width: 20.0, height: 20.0, ..Default::default() };
        inp.doors = vec![Door { index: 0, a: [5.0, 5.0], b: [6.0, 5.0], ..Default::default() }, Door { index: 1, a: [10.0, 5.0], b: [11.0, 5.0], window: true, ..Default::default() }];
        inp.objs = vec![obj(0, "Crate_Wood_A", 5.5, 5.6, 1.0, 1.0, 300), obj(1, "Crate_Wood_A", 10.5, 5.6, 1.0, 1.0, 300), obj(2, "Rug_A", 5.5, 4.5, 2.0, 2.0, 150)];
        let out: Vec<Issue> = run(&inp).into_iter().filter(|i| i.kind == Kind::Door).collect();
        assert_eq!(out.len(), 1, "окно и ковёр не в счёт: {out:?}");
        assert_eq!(out[0].objects, vec![0]);
    }

    #[test]
    fn street_and_house_things() {
        let mut inp = Input { width: 20.0, height: 20.0, houses: vec![house(3, [2.0, 2.0, 8.0, 8.0])], ..Default::default() };
        inp.objs = vec![obj(0, "Lamp_Street_A", 5.0, 5.0, 1.0, 1.0, 300), obj(1, "Bookshelf_A", 15.0, 15.0, 2.0, 1.0, 300)];
        let k = kinds(&inp);
        assert!(k.contains(&Kind::Street) && k.contains(&Kind::Indoor), "{k:?}");
        // Обнесённое стенами — не улица: шкаф во дворе за оградой без крыши.
        inp.walls = vec![Line { points: vec![[12.0, 12.0], [18.0, 12.0], [18.0, 18.0], [12.0, 18.0]], closed: true }];
        assert!(!kinds(&inp).contains(&Kind::Indoor));
    }

    #[test]
    fn edge_and_label() {
        let mut inp = Input { width: 20.0, height: 20.0, ..Default::default() };
        inp.objs = vec![obj(0, "Crate_Wood_A", 19.8, 5.0, 2.0, 2.0, 300), obj(1, "Tree_Green_A", 19.8, 10.0, 4.0, 4.0, 300), obj(2, "Barrel_Wood_A", 8.0, 15.0, 1.0, 1.0, 300)];
        inp.labels = vec![Text { text: "Склад".into(), size: 1.0, x: 8.0, y: 15.0 }];
        let k = kinds(&inp);
        assert_eq!(k.iter().filter(|&&k| k == Kind::Edge).count(), 1, "дерево за краем — можно: {k:?}");
        assert!(k.contains(&Kind::Label));
    }

    #[test]
    fn wall_lamp_needs_a_wall() {
        let mut inp = Input { width: 20.0, height: 20.0, ..Default::default() };
        inp.walls = vec![Line { points: vec![[2.0, 2.0], [10.0, 2.0]], closed: false }];
        inp.objs = vec![obj(0, "Wall_Lamp_A", 5.0, 2.4, 0.5, 0.5, 300), obj(1, "Wall_Lamp_A", 5.0, 9.0, 0.5, 0.5, 300)];
        let out: Vec<Issue> = run(&inp).into_iter().filter(|i| i.kind == Kind::Lamp).collect();
        assert_eq!(out.iter().map(|i| i.objects[0]).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn crate_blocks_a_corridor_but_not_a_hall() {
        // Коридор шириной 1,5 клетки между двумя стенами, ящик посередине — затор.
        let mut inp = Input { width: 30.0, height: 12.0, ..Default::default() };
        inp.walls = vec![Line { points: vec![[0.0, 5.0], [30.0, 5.0]], closed: false }, Line { points: vec![[0.0, 6.5], [30.0, 6.5]], closed: false }];
        inp.objs = vec![obj(0, "Crate_Wood_A", 15.0, 5.75, 1.0, 1.0, 300)];
        let out: Vec<Issue> = run(&inp).into_iter().filter(|i| i.kind == Kind::Pass).collect();
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].objects, vec![0]);
        // Тот же ящик посреди широкого зала — обходят.
        inp.walls[1].points = vec![[0.0, 11.0], [30.0, 11.0]];
        inp.objs[0].y = 8.0;
        assert!(!kinds(&inp).contains(&Kind::Pass));
    }

    #[test]
    fn bridge_word_is_a_whole_word() {
        assert!(is_bridge("bridge_boardwalk_straight_a_3x4") && is_bridge("старый мост"));
        assert!(!is_bridge("bridgekeeper_hut"));
    }
}
