//! Церковь с погостом. Устроена, как настоящая западная церковь:
//!
//! - Ось — с запада на восток, алтарь на востоке (на карте — справа).
//! - Неф — для прихожан: скамьи двумя блоками, посередине проход к алтарю;
//!   у входа купель. Восточный конец нефа — помост на ступень выше: с него
//!   священник читает и проповедует (кафедра и аналой). За помостом —
//!   преграда с проходом посередине, за ней алтарная часть.
//! - Алтарная часть уже нефа: хоры (скамьи лицом друг к другу) и орган, у
//!   восточной стены — престол с канделябрами; восточный конец плоский или
//!   полукруглой апсидой.
//! - Башня-колокольня — с запада, через неё парадный вход; обычный вход —
//!   с юга, западнее середины. Ризница — к северу от алтарной части, в ней
//!   спуск в крипту. С севера к нефу пристроена исповедальня: две кабинки,
//!   между ними решётка.
//! - Большие церкви: боковые нефы за аркадой колонн с шагом пролёта, на
//!   ступень выше среднего; у их восточных концов боковой алтарь и гробница;
//!   крестовые — трансепт с приделами и башня над средокрестием.
//! - Погост за низкой каменной оградой; к дверям — мощёные дорожки; могилы
//!   рядами, головой на запад, надгробие в изголовье. Больше всего их к югу и
//!   востоку от церкви, к северу — мало: северная сторона считалась худшей.
//!
//! Ступени и помосты — не стены: их видно по тени на нижнем полу (`ledge`).
//! Тип выбирается по месту на карте, иногда — поменьше.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Type {
    /// Часовня: неф и апсида, вход с запада.
    Chapel,
    /// Приходская: башня, неф, алтарная часть, исповедальня, ризница.
    Parish,
    /// Трёхнефная: то же с боковыми нефами за аркадой.
    Aisled,
    /// Крестовая: трёхнефная, трансепт, башня над средокрестием.
    Cross,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Part {
    Nave,
    /// Боковой неф: `true` — северный.
    Aisle(bool),
    Chancel,
    Tower,
    Confession,
    Vestry,
    /// Рукав трансепта: `true` — северный.
    Transept(bool),
    Crossing,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Part::Nave => "Неф",
            Part::Aisle(true) => "Северный неф",
            Part::Aisle(false) => "Южный неф",
            Part::Chancel => "Алтарная часть",
            Part::Tower => "Колокольня",
            Part::Confession => "Исповедальня",
            Part::Vestry => "Ризница",
            Part::Transept(true) => "Северный придел",
            Part::Transept(false) => "Южный придел",
            Part::Crossing => "Средокрестие",
        }
    }

    /// Отделена от церкви стеной с дверью, а не открыта аркой.
    fn walled(self) -> bool {
        matches!(self, Part::Tower | Part::Confession | Part::Vestry)
    }
}

/// Ширина нефа (чётная: ось — по линии сетки), боковых нефов, шаг аркады.
const NAVE: i32 = 8;
const AISLE: i32 = 3;
const BAY: i32 = 3;
/// Глубина помоста перед преградой.
const DAIS: i32 = 2;
/// Тень у ступени: как далеко тянется по нижнему полу и насколько темна.
const LEDGE_REACH: f32 = 0.6;
const LEDGE_DARK: f32 = 0.5;
/// Бордюр по краю уступа — как в квартале у мостовой.
const CURB: f32 = 0.22;
/// Насколько колонна аркады отступает от края уступа на возвышенность.
const PIER: f32 = 0.6;

/// Церковь в своих осях: `x` с запада на восток, ось нефа — `y = 0`.
struct Lay {
    pieces: Vec<(Part, Room)>,
    /// Апсида: середина и радиус.
    apse: Option<([f32; 2], f32)>,
    /// Колонны аркады и средокрестия.
    pillars: Vec<[f32; 2]>,
    ty: Type,
}

impl Lay {
    fn get(&self, part: Part) -> Option<Room> {
        self.pieces.iter().find(|(p, _)| *p == part).map(|(_, r)| *r)
    }

    fn bbox(&self) -> Room {
        let mut x0 = self.pieces.iter().map(|(_, r)| r.x).min().unwrap_or(0);
        let mut x1 = self.pieces.iter().map(|(_, r)| r.x + r.w).max().unwrap_or(0);
        let y0 = self.pieces.iter().map(|(_, r)| r.y).min().unwrap_or(0);
        let y1 = self.pieces.iter().map(|(_, r)| r.y + r.h).max().unwrap_or(0);
        if let Some((c, r)) = self.apse {
            x0 = x0.min((c[0] - r).floor() as i32);
            x1 = x1.max((c[0] + r).ceil() as i32);
        }
        Room { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }

    fn shift(&mut self, dx: i32, dy: i32) {
        for (_, r) in &mut self.pieces {
            r.x += dx;
            r.y += dy;
        }
        if let Some((c, _)) = &mut self.apse {
            c[0] += dx as f32;
            c[1] += dy as f32;
        }
        for p in &mut self.pillars {
            p[0] += dx as f32;
            p[1] += dy as f32;
        }
    }

    /// Клетка под крышей церкви.
    fn covers(&self, x: i32, y: i32) -> bool {
        let q = [x as f32 + 0.5, y as f32 + 0.5];
        self.pieces.iter().any(|(_, r)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) || self.apse.is_some_and(|(c, r)| q[0] > c[0] && (q[0] - c[0]).hypot(q[1] - c[1]) < r)
    }

    /// Преграда перед алтарём: линия `x` и её концы по `y`. У часовни алтарная
    /// часть — апсида и две клетки нефа перед ней: в одной апсиде престолу тесно.
    fn screen(&self) -> (i32, i32, i32) {
        match self.get(Part::Chancel) {
            Some(c) => (c.x, c.y, c.y + c.h),
            None => {
                let n = self.get(Part::Nave).expect("неф есть всегда");
                (n.x + n.w - 2, n.y, n.y + n.h)
            }
        }
    }

    /// Помост перед преградой: в нефе или, у крестовой, в средокрестии.
    fn dais(&self) -> Room {
        let (sx, _, _) = self.screen();
        let host = self.get(Part::Crossing).or(self.get(Part::Nave)).expect("неф есть всегда");
        Room { x: sx - DAIS, y: host.y, w: DAIS, h: host.h }
    }
}

/// Раскладка: `len` — длина нефа, `chancel` — алтарной части.
fn layout(ty: Type, len: i32, chancel: i32, apse: bool) -> Lay {
    let half = NAVE / 2;
    let mut pieces = Vec::new();
    let mut pillars = Vec::new();
    let mut apse_at = None;
    let arcade = |pillars: &mut Vec<[f32; 2]>, x0: i32| {
        for k in 1..len / BAY {
            // На возвышенности бокового нефа, рядом с бордюром, а не на нём.
            for y in [-half as f32 - PIER, half as f32 + PIER] {
                pillars.push([(x0 + k * BAY) as f32, y]);
            }
        }
    };
    match ty {
        Type::Chapel => {
            let n = 6;
            pieces.push((Part::Nave, Room { x: 0, y: -n / 2, w: len, h: n }));
            apse_at = Some(([len as f32, 0.0], (n / 2) as f32));
        }
        Type::Parish | Type::Aisled => {
            let aisles = ty == Type::Aisled;
            pieces.push((Part::Tower, Room { x: 0, y: -2, w: 4, h: 4 }));
            pieces.push((Part::Nave, Room { x: 4, y: -half, w: len, h: NAVE }));
            let ce = 4 + len;
            let cw = 6;
            pieces.push((Part::Chancel, Room { x: ce, y: -cw / 2, w: chancel, h: cw }));
            let outer = if aisles { half + AISLE } else { half };
            if aisles {
                pieces.push((Part::Aisle(true), Room { x: 4, y: -outer, w: len, h: AISLE }));
                pieces.push((Part::Aisle(false), Room { x: 4, y: half, w: len, h: AISLE }));
                arcade(&mut pillars, 4);
            }
            // Исповедальня — к северной стене у второго пролёта.
            pieces.push((Part::Confession, Room { x: 4 + BAY - 1, y: -outer - 2, w: 4, h: 2 }));
            // Ризница — к северу от алтарной части; в ней спуск, ей нужно 4 клетки.
            let vh = (outer - cw / 2).max(4);
            pieces.push((Part::Vestry, Room { x: ce, y: -cw / 2 - vh, w: 5.min(chancel), h: vh }));
            if apse {
                apse_at = Some(([(ce + chancel) as f32, 0.0], (cw / 2) as f32));
            }
        }
        Type::Cross => {
            let outer = half + AISLE;
            pieces.push((Part::Nave, Room { x: 0, y: -half, w: len, h: NAVE }));
            pieces.push((Part::Aisle(true), Room { x: 0, y: -outer, w: len, h: AISLE }));
            pieces.push((Part::Aisle(false), Room { x: 0, y: half, w: len, h: AISLE }));
            arcade(&mut pillars, 0);
            pieces.push((Part::Confession, Room { x: BAY - 1, y: -outer - 2, w: 4, h: 2 }));
            let cx = len;
            pieces.push((Part::Crossing, Room { x: cx, y: -half, w: NAVE, h: NAVE }));
            let arm = outer + 3 - half;
            pieces.push((Part::Transept(true), Room { x: cx, y: -half - arm, w: NAVE, h: arm }));
            pieces.push((Part::Transept(false), Room { x: cx, y: half, w: NAVE, h: arm }));
            // Западные опоры средокрестия — в углах боковых нефов; с востока
            // средокрестие держит преграда.
            for y in [-half as f32 - PIER, half as f32 + PIER] {
                pillars.push([cx as f32 - PIER, y]);
            }
            let ce = cx + NAVE;
            pieces.push((Part::Chancel, Room { x: ce, y: -half, w: chancel, h: NAVE }));
            pieces.push((Part::Vestry, Room { x: ce, y: -half - 4, w: 5, h: 4 }));
            if apse {
                apse_at = Some(([(ce + chancel) as f32, 0.0], half as f32));
            }
        }
    }
    Lay { pieces, apse: apse_at, pillars, ty }
}

/// Самая крупная церковь, что встаёт на карту с погостом вокруг (`margin` клеток).
fn fit(w: i32, h: i32, rng: &mut Rng, density: f32) -> Option<Lay> {
    let apse = rng.chance(0.55);
    // Иногда — поменьше, чем влезает: на погосте просторнее.
    let modest = rng.chance((0.3 - density * 0.2) as f64);
    // Погосту — хотя бы шесть клеток с каждой стороны; не выходит — меньше.
    for margin in [6, 4, 2, 1] {
        let mut found: Option<Lay> = None;
        for ty in [Type::Cross, Type::Aisled, Type::Parish, Type::Chapel] {
            let (lens, chancels): (Vec<i32>, Vec<i32>) = match ty {
                Type::Chapel => ((10..=13).rev().collect(), vec![0]),
                Type::Parish => ((10..=16).rev().collect(), vec![8, 7, 6]),
                Type::Aisled => ([18, 15, 12].into(), vec![9, 8, 7]),
                Type::Cross => ([18, 15, 12].into(), vec![10, 9, 8]),
            };
            'sizes: for &len in &lens {
                for &c in &chancels {
                    let lay = layout(ty, len, c, apse || ty == Type::Chapel);
                    let b = lay.bbox();
                    if b.w + 2 * margin <= w && b.h + 2 * margin <= h {
                        found = Some(lay);
                        break 'sizes;
                    }
                }
            }
            if let Some(f) = found.take() {
                found = Some(if modest && f.ty > Type::Chapel { smaller(f, w, h, margin, apse) } else { f });
                break;
            }
        }
        if let Some(mut lay) = found {
            let b = lay.bbox();
            // Погост просторнее к югу (туда выходит дверь) — церковь чуть севернее.
            let dx = (w - b.w) / 2 - b.x;
            let dy = ((h - b.h) / 2 - 1).max(margin) - b.y;
            lay.shift(dx, dy);
            return Some(lay);
        }
    }
    None
}

/// Тип на ступень меньше того, что влез, — если и он влезает.
fn smaller(big: Lay, w: i32, h: i32, margin: i32, apse: bool) -> Lay {
    let lay = match big.ty {
        Type::Cross => layout(Type::Aisled, 15, 8, apse),
        Type::Aisled => layout(Type::Parish, 13, 7, apse),
        _ => layout(Type::Chapel, 10, 0, true),
    };
    let b = lay.bbox();
    if b.w + 2 * margin <= w && b.h + 2 * margin <= h { lay } else { big }
}

/// Отрезок по линии сетки: вертикальный — `x = c`, от `s` до `e` по `y`.
fn seg(vertical: bool, c: i32, s: f32, e: f32) -> ([f32; 2], [f32; 2]) {
    if vertical { ([c as f32, s], [c as f32, e]) } else { ([s, c as f32], [e, c as f32]) }
}

/// Ступень: край верхнего пола по линии сетки (`vertical` — `x = c`, от `s` до
/// `e`), нижний пол — в сторону `low` (±1). На нижнем полу у края — тень,
/// она и показывает, что там уступ.
fn ledge(m: &mut Map, vertical: bool, c: f32, s: f32, e: f32, low: f32) {
    let res = mask::RES as f32;
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut out = Vec::new();
    let (a0, a1) = (s.min(e), s.max(e));
    let (b0, b1) = if low > 0.0 { (c, c + LEDGE_REACH) } else { (c - LEDGE_REACH, c) };
    let (x0, x1, y0, y1) = if vertical { (b0, b1, a0, a1) } else { (a0, a1, b0, b1) };
    for j in ((y0 * res).floor().max(0.0) as usize)..((y1 * res).ceil() as usize).min(mh) {
        for i in ((x0 * res).floor().max(0.0) as usize)..((x1 * res).ceil() as usize).min(mw) {
            let p = mask::sample_pos(i as isize, j as isize);
            let d = (if vertical { p[0] } else { p[1] } - c) * low;
            if !(0.0..LEDGE_REACH).contains(&d) {
                continue;
            }
            out.push((j * mw + i, (1.0 - d / LEDGE_REACH).powf(1.6)));
        }
    }
    m.shade_paint(&out, LEDGE_DARK);
}

/// Бордюр — одной полосой камня по ломаной вдоль края верхнего пола: углы —
/// поворотом той же полосы, а не стыком двух. Концы, что упираются в стену,
/// заходят под неё. Камень встроенный: у стен FA блоки с просветами, полоса
/// из них рвётся.
fn curb(m: &mut Map, corners: &[[f32; 2]]) {
    let k = key("wall_stone_blocks");
    let stone = m.material_index(Material { name: library::label(&k), paint: Paint::Texture(k), vary: false, live: Live::Auto }).unwrap_or(0);
    // Частыми точками: дорога из пары точек тает целиком, а у углов точки
    // поближе — поворот выходит плавным, но коротким.
    let mut points: Vec<[f32; 2]> = Vec::new();
    for pair in corners.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        let at = |t: f32| [a[0] + (b[0] - a[0]) * t / len, a[1] + (b[1] - a[1]) * t / len];
        if points.is_empty() {
            points.push(a);
        }
        let mut t = 0.15f32.min(len / 3.0);
        while t < len - 0.15 {
            points.push(at(t));
            t += 0.5;
        }
        points.push(at((len - 0.15).max(len * 2.0 / 3.0)));
        points.push(b);
    }
    m.paths.push(Path { points, material: stone, width: CURB, soft: 0.0, rough: 0.0, along: true });
}

pub(super) fn church(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    let season = p.options.season;
    let look = forest_look(season);
    ground(m, look.ground);
    patches(m, rng, look.patches, w * h / 60, (2.0, 4.0), 0.6);
    let Some(lay) = fit(w, h, rng, p.options.density) else {
        warn.push(format!("церковь не помещается на карту {w}×{h}"));
        return 0;
    };

    // Полы: неф и всё прочее — плиты; алтарная часть и помост — мрамор;
    // ризница и исповедальня — доски.
    let flags = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("flagstones"));
    let marble = mat(m, "temple_floor");
    let planks = mat(m, "wood_planks");
    for &(part, r) in &lay.pieces {
        let f = match part {
            Part::Chancel => marble,
            Part::Vestry | Part::Confession => planks,
            _ => flags,
        };
        m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(f));
    }
    if let Some((c, r)) = lay.apse {
        let ri = r as i32;
        let (cx, cy) = (c[0] as i32, c[1] as i32);
        m.paint_oval(Layer::Rooms, (cx - ri, cy - ri), (cx + ri - 1, cy + ri - 1), Some(marble));
    }
    let dais = lay.dais();
    m.paint_rect(Layer::Rooms, (dais.x, dais.y), (dais.x + dais.w - 1, dais.y + dais.h - 1), Some(marble));
    let outer = match p.style.walls.as_deref().and_then(plan::wall_id) {
        Some(st) => wall(st),
        None => wall(pick(rng, &["wall_stone_blocks", "wall_stone_blocks", "wall_fieldstone", "wall_sandstone"])),
    };
    let inner = WallStyle { width: outer.width * 0.8, ..outer.clone() };
    m.wall_style = outer.clone();
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    let axis = lay.get(Part::Nave).map_or(h / 2, |n| n.y + n.h / 2);
    let mut doors: Vec<[f32; 2]> = Vec::new();
    let inner_wall = |m: &mut Map, a: [f32; 2], b: [f32; 2]| {
        m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(inner.clone()) });
    };
    let door = |m: &mut Map, doors: &mut Vec<[f32; 2]>, a: [f32; 2], b: [f32; 2], kind: PortalKind| {
        m.portals.push(portal(a, b, kind, true));
        doors.push([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    };

    // Перегородки: башня, исповедальня и ризница отделены стеной, остальное
    // открыто арками.
    for (i, &(pa, ra)) in lay.pieces.iter().enumerate() {
        for &(pb, rb) in &lay.pieces[i + 1..] {
            let Some(s) = shared(ra, rb) else { continue };
            if !pa.walled() && !pb.walled() {
                continue;
            }
            inner_wall(m, s.0, s.1);
            let vertical = s.0[0] == s.1[0];
            let (lo, hi) = if vertical { (s.0[1], s.1[1]) } else { (s.0[0], s.1[0]) };
            let c = if vertical { s.0[0] } else { s.0[1] } as i32;
            let pair = |q: Part| pa == q || pb == q;
            if pair(Part::Tower) {
                let mid = ((lo + hi) / 2.0).floor();
                let (a, b) = seg(vertical, c, mid - 1.0, mid + 1.0);
                door(m, &mut doors, a, b, PortalKind::Opening);
            } else if pair(Part::Vestry) && pair(Part::Chancel) {
                let mid = ((lo + hi) / 2.0).floor();
                let (a, b) = seg(vertical, c, mid, mid + 1.0);
                door(m, &mut doors, a, b, PortalKind::Door);
            } else if pair(Part::Confession) {
                // По двери в каждую кабинку.
                let cf = lay.get(Part::Confession).expect("исповедальня");
                for x in [cf.x, cf.x + cf.w - 1] {
                    let (a, b) = seg(false, c, x as f32, x as f32 + 1.0);
                    door(m, &mut doors, a, b, PortalKind::Door);
                }
            }
        }
    }
    // Исповедальня: перегородка с решёткой между священником и кающимся.
    if let Some(cf) = lay.get(Part::Confession) {
        let mx = cf.x + cf.w / 2;
        let (a, b) = seg(true, mx, cf.y as f32, (cf.y + cf.h) as f32);
        inner_wall(m, a, b);
        let (a, b) = seg(true, mx, cf.y as f32 + 0.5, cf.y as f32 + 1.5);
        m.portals.push(portal(a, b, PortalKind::Window, true));
        // Священник и кающийся — лицом к решётке.
        let chair = asset("chair");
        drop_pick(m, &chair, [mx as f32 + 0.75, cf.y as f32 + 1.0], 90.0, 1.0);
        drop_pick(m, &chair, [mx as f32 - 0.75, cf.y as f32 + 1.0], 270.0, 1.0);
    }
    // Преграда перед алтарной частью: стена с проходом посередине.
    let (sx, sy0, sy1) = lay.screen();
    let (a, b) = seg(true, sx, sy0 as f32, sy1 as f32);
    // Деревянная, как настоящая алтарная преграда: тонкий камень FA на ней —
    // блоки с выступами.
    m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(WallStyle { paint: Paint::Texture(key("wall_wood")), width: 0.2 }) });
    // Проход — чистый проём: косяки двери шире тонкой преграды и торчат из неё.
    let (a, b) = seg(true, sx, axis as f32 - 1.0, axis as f32 + 1.0);
    m.portals.push(opening(a, b));
    doors.push([sx as f32, axis as f32]);

    // Входы снаружи: западный (через башню или в торце нефа), южный — западнее
    // середины, у крестовой ещё в южном приделе; у алтарной части — дверь священника.
    let mut entries: Vec<([f32; 2], i32)> = Vec::new();
    let west = lay.get(Part::Tower).or(lay.get(Part::Nave)).expect("неф есть всегда");
    let (a, b) = seg(true, west.x, axis as f32 - 1.0, axis as f32 + 1.0);
    door(m, &mut doors, a, b, PortalKind::DoubleDoor);
    entries.push(([west.x as f32, axis as f32], 3));
    let south_wall = lay.get(Part::Aisle(false)).or(lay.get(Part::Nave)).expect("неф есть всегда");
    if lay.ty != Type::Chapel {
        let nave = lay.get(Part::Nave).expect("неф есть всегда");
        let x = (nave.x + BAY) as f32;
        let y = south_wall.y + south_wall.h;
        let (a, b) = seg(false, y, x, x + 2.0);
        door(m, &mut doors, a, b, PortalKind::DoubleDoor);
        entries.push(([x + 1.0, y as f32], 2));
    }
    if let Some(r) = lay.get(Part::Transept(false)) {
        let mx = (r.x + r.w / 2) as f32;
        let (a, b) = seg(false, r.y + r.h, mx - 1.0, mx + 1.0);
        door(m, &mut doors, a, b, PortalKind::DoubleDoor);
        entries.push(([mx, (r.y + r.h) as f32], 2));
    }
    if let Some(r) = lay.get(Part::Chancel)
        && lay.ty != Type::Cross
        && !lay.covers(r.x + 2, r.y + r.h)
    {
        let x = (r.x + 2) as f32;
        let (a, b) = seg(false, r.y + r.h, x, x + 1.0);
        door(m, &mut doors, a, b, PortalKind::Door);
    }

    // Окна — по одному в пролёт на внешних стенах; в плоском восточном торце —
    // большое; в апсиде — три.
    for &(part, r) in &lay.pieces {
        if matches!(part, Part::Confession | Part::Crossing) || (part == Part::Nave && lay.ty >= Type::Aisled) {
            continue;
        }
        for side in 0..4 {
            let vertical = side % 2 == 1;
            let len = if vertical { r.h } else { r.w };
            let c = match side {
                0 => r.y,
                1 => r.x + r.w,
                2 => r.y + r.h,
                _ => r.x,
            };
            let (start, step) = if part == Part::Tower || part == Part::Vestry { (len / 2, len) } else { (1, BAY) };
            let mut k = start;
            while k < len - 1 {
                let along = if vertical { r.y + k } else { r.x + k };
                let (ox, oy) = match side {
                    0 => (along, r.y - 1),
                    1 => (r.x + r.w, along),
                    2 => (along, r.y + r.h),
                    _ => (r.x - 1, along),
                };
                let mid = if vertical { [c as f32, along as f32 + 0.5] } else { [along as f32 + 0.5, c as f32] };
                let near_door = doors.iter().any(|d| (d[0] - mid[0]).abs() + (d[1] - mid[1]).abs() < 2.0);
                let east_end = part == Part::Chancel && side == 1;
                // У восточного конца северного бокового нефа стоит орган — окна там нет.
                let organ_wall = part == Part::Aisle(true) && side == 0 && along >= r.x + r.w - 3;
                // В ризнице у северной и восточной стен — спуск в крипту.
                let stairs_wall = part == Part::Vestry && (side == 0 || side == 1);
                if !lay.covers(ox, oy) && !near_door && !east_end && !organ_wall && !stairs_wall {
                    let (a, b) = seg(vertical, c, along as f32, along as f32 + 1.0);
                    m.portals.push(portal(a, b, PortalKind::Window, false));
                }
                k += step;
            }
            if part == Part::Chancel && side == 1 && lay.apse.is_none() {
                let my = (r.y + r.h / 2) as f32;
                let (a, b) = seg(true, c, my - 1.0, my + 1.0);
                m.portals.push(portal(a, b, PortalKind::Window, false));
            }
        }
    }
    if let Some((c, r)) = lay.apse {
        for deg in [-50.0f32, 0.0, 50.0] {
            let t = deg.to_radians();
            let (dx, dy) = (t.cos(), t.sin());
            let q = [c[0] + dx * r, c[1] + dy * r];
            let a = [q[0] + dy * 0.5, q[1] - dx * 0.5];
            let b = [q[0] - dy * 0.5, q[1] + dx * 0.5];
            m.portals.push(portal(a, b, PortalKind::Window, false));
        }
    }

    // Уступы: помост над нефом, боковые нефы над средним — тень на нижнем полу.
    ledge(m, true, dais.x as f32, dais.y as f32, (dais.y + dais.h) as f32, -1.0);
    // Боковые нефы на уровне помоста: край аркады — до помоста; у крестовой
    // неф кончается у средокрестия, и край поворачивает к наружной стене.
    if let (Some(n), true) = (lay.get(Part::Nave), lay.ty >= Type::Aisled) {
        let x1 = if lay.ty == Type::Cross { n.x + n.w } else { dais.x };
        ledge(m, false, n.y as f32, n.x as f32, x1 as f32, 1.0);
        ledge(m, false, (n.y + n.h) as f32, n.x as f32, x1 as f32, -1.0);
        if lay.ty == Type::Cross {
            for north in [true, false] {
                if let Some(a) = lay.get(Part::Aisle(north)) {
                    ledge(m, true, (a.x + a.w) as f32, a.y as f32, (a.y + a.h) as f32, 1.0);
                }
            }
            // Помост в средокрестии открыт к рукавам трансепта — бордюр обходит его кругом.
            ledge(m, false, dais.y as f32, dais.x as f32, (dais.x + dais.w) as f32, -1.0);
            ledge(m, false, (dais.y + dais.h) as f32, dais.x as f32, (dais.x + dais.w) as f32, 1.0);
        }
    }
    // Бордюры по контуру уступов — каждый одной ломаной. Полоса лежит на верхнем
    // полу у самого края; в стену уходит на 0,3 клетки.
    let o = CURB / 2.0;
    let (dx, dy0, dy1) = (dais.x as f32, dais.y as f32, (dais.y + dais.h) as f32);
    match (lay.ty, lay.get(Part::Nave)) {
        (Type::Cross, Some(n)) => {
            let (nx, ny0, ny1, cx) = (n.x as f32, n.y as f32, (n.y + n.h) as f32, (n.x + n.w) as f32);
            if let (Some(an), Some(as_)) = (lay.get(Part::Aisle(true)), lay.get(Part::Aisle(false))) {
                curb(m, &[[nx - 0.3, ny0 - o], [cx - o, ny0 - o], [cx - o, an.y as f32 - 0.3]]);
                curb(m, &[[nx - 0.3, ny1 + o], [cx - o, ny1 + o], [cx - o, (as_.y + as_.h) as f32 + 0.3]]);
            }
            let sx = lay.screen().0 as f32;
            curb(m, &[[sx + 0.3, dy0 + o], [dx + o, dy0 + o], [dx + o, dy1 - o], [sx + 0.3, dy1 - o]]);
        }
        (Type::Aisled, Some(n)) => {
            let (nx, ny0, ny1) = (n.x as f32, n.y as f32, (n.y + n.h) as f32);
            curb(m, &[[nx - 0.3, ny0 - o], [dx + o, ny0 - o], [dx + o, ny1 + o], [nx - 0.3, ny1 + o]]);
        }
        _ => curb(m, &[[dx + o, dy0 - 0.3], [dx + o, dy1 + 0.3]]),
    }

    // Крыши: пониже — раньше, чтобы высокие ложились сверху. Шатёр апсиды под
    // крышей алтарной части — из-под неё видна полукруглая половина.
    let roof = match p.style.roof.as_deref().and_then(plan::roof_id) {
        Some(r) => r,
        None => pick(rng, &["roof_slate_gray", "roof_slate_gray", "roof_slate_blue", "roof_clay_red", "roof_shingle"]),
    };
    let rk = key(roof);
    let rect = |r: Room| ([r.x as f32, r.y as f32], [(r.x + r.w) as f32, (r.y + r.h) as f32]);
    let push = |m: &mut Map, (a, b): ([f32; 2], [f32; 2]), shape: RoofShape, turn: u8| {
        m.roofs.push(Roof { a, b, shape, paint: rk.clone(), turn, overhang: 0.3 });
    };
    for &(part, r) in &lay.pieces {
        match part {
            Part::Aisle(north) => push(m, rect(r), RoofShape::Shed, if north { 2 } else { 0 }),
            Part::Vestry | Part::Confession => push(m, rect(r), RoofShape::Shed, 2),
            _ => {}
        }
    }
    if let Some((c, r)) = lay.apse {
        push(m, ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]), RoofShape::Tent, 0);
    }
    for part in [Part::Chancel, Part::Transept(true), Part::Transept(false), Part::Nave] {
        if let Some(r) = lay.get(part) {
            push(m, rect(r), RoofShape::Gable, u8::from(matches!(part, Part::Transept(_))));
        }
    }
    // Башни квадратные — крыша пирамидой на четыре ската, по стенам.
    for part in [Part::Crossing, Part::Tower] {
        if let Some(r) = lay.get(part) {
            push(m, rect(r), RoofShape::Hip, 0);
        }
    }

    // ----- убранство -----
    let mut sp = Spots::new(m);
    for y in 0..h {
        for x in 0..w {
            if !lay.covers(x, y) {
                sp.block(x, y, 1, 1);
            }
        }
    }
    for d in &doors {
        sp.block(d[0].floor() as i32 - 1, d[1].floor() as i32 - 1, 3, 3);
    }
    // Колонны, скамьи, канделябры — одним вариантом на всю церковь.
    let pillar = asset("pillar_stone");
    let pew = asset("pew");
    let candela = asset("candelabra");
    for q in &lay.pillars {
        drop_pick(m, &pillar, *q, 0.0, 0.9);
        sp.block(q[0].round() as i32 - 1, q[1].round() as i32 - 1, 2, 2);
    }
    let candle = |m: &mut Map, q: [f32; 2]| {
        if !missing("candelabra") {
            drop_pick(m, &candela, q, 0.0, 1.0);
        }
    };
    // Короткие ряды скамей лицом на восток, по одной на ряд, с серединой `y`.
    let pew_rows = |m: &mut Map, x_east: f32, y: f32, rows: i32| {
        for k in 0..rows {
            drop_pick(m, &pew, [x_east - k as f32 * 1.2, y], 270.0, 1.0);
        }
    };

    // Статуи в церкви — одной работы: один камень, одна фигура.
    let statue = asset("statue_figure");
    // Престол у восточной стены (или в апсиде), по бокам канделябры, за ним — статуя.
    let east = lay.get(Part::Chancel).or(lay.get(Part::Nave)).expect("неф есть всегда");
    let altar = if missing("altar_cloth") { "altar" } else { "altar_cloth" };
    let ax = match lay.apse {
        Some((c, _)) => c[0] + 0.6,
        None => (east.x + east.w) as f32 - 1.0,
    };
    drop(m, altar, [ax, axis as f32], 90.0, 1.0);
    candle(m, [ax, axis as f32 - 1.6]);
    candle(m, [ax, axis as f32 + 1.6]);
    if let Some((c, r)) = lay.apse
        && r >= 3.0
    {
        drop_pick(m, &statue, [c[0] + r - 1.1, axis as f32], 90.0, 0.8);
    }
    let sanctuary = (ax - 1.5).floor() as i32;

    // Хоры: скамьи вдоль северной и южной стен алтарной части, лицом друг к другу;
    // у северной стены первым — орган.
    // Перед дверью — ни скамьи, ни органа.
    let clear = |doors: &[[f32; 2]], q: [f32; 2]| doors.iter().all(|d| (d[0] - q[0]).abs() > 1.6 || (d[1] - q[1]).abs() > 1.6);
    if let Some(ch) = lay.get(Part::Chancel) {
        let mut north: Vec<[f32; 2]> = Vec::new();
        let mut x = ch.x + 1;
        while x + 2 <= sanctuary - 1 {
            let n = [x as f32 + 1.0, ch.y as f32 + WALL_GAP + 0.5];
            if clear(&doors, n) {
                north.push(n);
            }
            let s = [x as f32 + 1.0, (ch.y + ch.h) as f32 - WALL_GAP - 0.5];
            if clear(&doors, s) {
                drop_pick(m, &pew, s, 180.0, 1.0);
            }
            x += 2;
        }
        // У приходской боковых нефов нет — орган в хорах, ближе к алтарю.
        let organ_here = lay.ty == Type::Parish && !missing("organ");
        let last = north.len().saturating_sub(1);
        for (i, q) in north.into_iter().enumerate() {
            if organ_here && i == last {
                drop(m, "organ", q, 0.0, 1.0);
            } else {
                drop_pick(m, &pew, q, 0.0, 1.0);
            }
        }
    }

    // Помост: одна кафедра посередине у края, книгой к прихожанам — священник
    // за ней лицом к нефу; к преграде обходят по бокам.
    if !missing("lectern") {
        drop(m, "lectern", [dais.x as f32 + 0.6, axis as f32], 270.0, 1.0);
    }
    sp.block(dais.x - 1, dais.y, dais.w + 1, dais.h);

    // Неф: ряды скамей лицом к алтарю, посередине проход в две клетки.
    let nave = lay.get(Part::Nave).expect("неф есть всегда");
    let side = if nave.h >= 8 { 1 } else { 0 };
    let per = ((nave.h / 2 - 1 - side) / 2).max(1);
    let x0 = nave.x + 3;
    let x1 = if lay.ty == Type::Cross { nave.x + nave.w - 1 } else { dais.x - 1 };
    let mut x = x0 as f32 + 0.5;
    while x + 0.5 <= x1 as f32 {
        for k in 0..per {
            let yn = (nave.y + side + k * 2) as f32 + 1.0;
            let ys = (axis + 1 + k * 2) as f32 + 1.0;
            drop_pick(m, &pew, [x, yn], 270.0, 1.0);
            drop_pick(m, &pew, [x, ys], 270.0, 1.0);
        }
        x += 1.2;
    }
    sp.block(x0, nave.y, x1 - x0, nave.h);
    // Купель — у входа, южнее оси.
    let (fx, fy) = (nave.x as f32 + 1.5, axis as f32 + 2.2);
    if !missing("font_base") {
        drop(m, "font_base", [fx, fy], 0.0, 0.7);
        if !missing("font_water") {
            drop(m, "font_water", [fx, fy], 0.0, 0.62);
        }
    } else {
        // Встроенный фонтан — во много клеток: ужать до купели в клетку с третью.
        let f = asset("fountain");
        drop_pick(m, &f, [fx, fy], 0.0, 1.3 / f.size[0].max(f.size[1]));
    }
    sp.block(nave.x, axis + 1, 3, 3);

    // Колокольня: лестница-стремянка наверх в северо-западном углу; проход от
    // западной двери к арке свободен.
    if let Some(t) = lay.get(Part::Tower)
        && !missing("ladder")
    {
        drop(m, "ladder", [t.x as f32 + 0.7, t.y as f32 + WALL_GAP + 0.5], 0.0, 1.0);
    }
    // Ризница: спуск в крипту у северной стены — сперва он, и две клетки перед
    // ним свободны; потом облачения, утварь, стол.
    if let Some(r) = lay.get(Part::Vestry) {
        crypt_stairs(m, &mut sp, rng, r);
        for n in ["wardrobe", "chest", "desk"] {
            by_side(m, &mut sp, rng, r, n, 1.0, false, &[0, 3]);
        }
    }
    // Боковые нефы: у восточного конца северного — гробница, южного — боковой
    // алтарь; по внешней стене — статуи и свечи.
    let side_altar = asset(if missing("altar_cloth") { "altar" } else { "altar_cloth" });
    for north in [true, false] {
        let Some(r) = lay.get(Part::Aisle(north)) else { continue };
        let wall_y = if north { r.y } else { r.y + r.h };
        let inward = if north { 1.0 } else { -1.0 };
        let ex = (r.x + r.w) as f32;
        let my = r.y as f32 + r.h as f32 / 2.0;
        if north {
            if !missing("organ") {
                drop(m, "organ", [ex - 1.4, wall_y as f32 + inward * (WALL_GAP + 0.5)], 0.0, 1.0);
            }
        } else {
            // Боковой алтарь у восточной стены, канделябры — по обе его стороны.
            drop_pick(m, &side_altar, [ex - 0.7, my], 90.0, 0.85);
            candle(m, [ex - 0.7, my - 1.2]);
            candle(m, [ex - 0.7, my + 1.2]);
        }
        // У колонн (между окнами) — по очереди статуя и канделябр, зеркально в обоих
        // нефах: место, занятое дверью хоть с одной стороны, пустует с обеих.
        let free = |q: [f32; 2]| clear(&doors, q) && doors.iter().all(|d| (d[0] - q[0]).abs() + (d[1] - q[1]).abs() > 2.2);
        let both = |x: f32| [true, false].iter().all(|&nn| lay.get(Part::Aisle(nn)).is_some_and(|a| free([x, if nn { a.y as f32 + 0.7 } else { (a.y + a.h) as f32 - 0.7 }])));
        // Места: у западной стены и против каждой колонны; занятое дверью — мимо,
        // чередование идёт по поставленным, а не по номеру колонны.
        let mut xs = vec![r.x as f32 + 0.8];
        let mut k = 1;
        while k * BAY < r.w - 3 {
            xs.push((r.x + k * BAY) as f32);
            k += 1;
        }
        let mut placed = 0;
        for x in xs {
            let q = [x, wall_y as f32 + inward * 0.7];
            if !both(x) {
                continue;
            }
            if placed % 2 == 0 {
                drop_pick(m, &statue, q, if north { 0.0 } else { 180.0 }, 0.6);
            } else {
                candle(m, q);
            }
            placed += 1;
        }
    }
    // Приделы в трансепте: алтарь у восточной стены, канделябры, два коротких
    // ряда скамей перед ним, статуя святого в дальнем углу.
    for north in [true, false] {
        let Some(r) = lay.get(Part::Transept(north)) else { continue };
        let ex = (r.x + r.w) as f32;
        let my = (r.y + r.h / 2) as f32 + if north { -0.5 } else { 0.5 };
        drop(m, "altar", [ex - 0.8, my], 90.0, 0.8);
        candle(m, [ex - 0.8, my - 1.5]);
        candle(m, [ex - 0.8, my + 1.5]);
        pew_rows(m, ex - 3.0, my, 2);
        let corner_y = if north { r.y as f32 + 1.0 } else { (r.y + r.h) as f32 - 1.0 };
        drop_pick(m, &statue, [r.x as f32 + 1.0, corner_y], if north { 0.0 } else { 180.0 }, 0.7);
    }
    // Подписи для ведущего.
    for &(part, r) in &lay.pieces {
        let mut l = Label::new(part.name().to_owned(), [r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0]);
        l.size = 0.5;
        l.gm = true;
        m.labels.push(l);
    }
    if p.mood == Mood::Abandoned {
        for &(_, r) in &lay.pieces {
            furnish(m, &mut sp, rng, r, Role::Ruin);
        }
    }

    churchyard(m, &lay, &entries, rng, season);
    if p.mood == Mood::Abandoned {
        let all: Vec<Room> = lay.pieces.iter().map(|(_, r)| *r).collect();
        abandon(m, rng, &all);
    }
    lay.pieces.len()
}

/// Погост: ограда с воротами против входов, мощёные дорожки к дверям,
/// деревья, ряды могил.
fn churchyard(m: &mut Map, lay: &Lay, entries: &[([f32; 2], i32)], rng: &mut Rng, season: Season) {
    let (w, h) = (m.width as i32, m.height as i32);
    let b = lay.bbox();
    let stone = mat(m, "cobblestone");
    let fenced = b.x >= 3 && b.y >= 3 && w - (b.x + b.w) >= 3 && h - (b.y + b.h) >= 3;
    let mut sp = Spots::new(m);
    sp.block(b.x - 1, b.y - 1, b.w + 2, b.h + 2);
    // Дорожка — прямо от двери за край карты: вдоль оси двери, ровной полосой.
    let mut gates: Vec<(i32, f32)> = Vec::new();
    let mut lanes: Vec<Path> = Vec::new();
    for &(door, side) in entries {
        let to = match side {
            2 => [door[0], h as f32 + 1.0],
            _ => [-1.0, door[1]],
        };
        gates.push(if side == 2 { (2, door[0]) } else { (3, door[1]) });
        // Частыми точками: у дороги из двух точек оба конца тают и она пропадает.
        let n = ((to[0] - door[0]).abs() + (to[1] - door[1]).abs()).ceil().max(2.0) as usize;
        let points = (0..=n).map(|k| {
            let t = k as f32 / n as f32;
            [door[0] + (to[0] - door[0]) * t, door[1] + (to[1] - door[1]) * t]
        }).collect();
        lanes.push(Path { points, material: stone, width: 2.0, soft: 0.25, rough: 0.15, along: false });
    }
    // Ограда: низкая каменная, разрывы под воротами, по столбу с каждой стороны.
    if fenced {
        let st = WallStyle { width: 0.25, ..wall("wall_fieldstone") };
        let post = asset("pillar_stone");
        let (x0, y0, x1, y1) = (1, 1, w - 1, h - 1);
        for side in 0..4 {
            let vertical = side % 2 == 1;
            let (c, lo, hi) = match side {
                0 => (y0, x0, x1),
                1 => (x1, y0, y1),
                2 => (y1, x0, x1),
                _ => (x0, y0, y1),
            };
            let mut cuts: Vec<f32> = gates.iter().filter(|g| g.0 == side).map(|g| g.1).collect();
            cuts.sort_by(f32::total_cmp);
            let mut s = lo as f32;
            for g in cuts {
                // Ворота в три клетки ровно по оси дорожки.
                let gl = g - 1.5;
                if gl > s {
                    let (a, bb) = seg(vertical, c, s, gl);
                    m.walls.push(Wall { points: vec![a, bb], closed: false, bends: Vec::new(), style: Some(st.clone()) });
                }
                for e in [gl, gl + 3.0] {
                    let q = if vertical { [c as f32, e] } else { [e, c as f32] };
                    drop_pick(m, &post, q, 0.0, 0.55);
                }
                s = gl + 3.0;
            }
            if (hi as f32) > s {
                let (a, bb) = seg(vertical, c, s, hi as f32);
                m.walls.push(Wall { points: vec![a, bb], closed: false, bends: Vec::new(), style: Some(st.clone()) });
            }
        }
        sp.block(0, 0, w, 2);
        sp.block(0, h - 2, w, 2);
        sp.block(0, 0, 2, h);
        sp.block(w - 2, 0, 2, h);
    }
    for l in &lanes {
        m.paths.push(l.clone());
    }
    let on_lane = |q: [f32; 2]| lanes.iter().any(|l| l.distance(q) < 1.8);

    // Деревья по краям погоста.
    let trees = ((w + h) / 14).max(1);
    for _ in 0..trees * 3 {
        let q = [rng.int(2, w - 3) as f32 + 0.5, rng.int(2, h - 3) as f32 + 0.5];
        let edge = q[0] < 5.0 || q[1] < 5.0 || q[0] > w as f32 - 5.0 || q[1] > h as f32 - 5.0;
        let (cx, cy) = (q[0] as i32, q[1] as i32);
        if edge && !on_lane(q) && sp.free(cx - 1, cy - 1, 3, 3) {
            drop(m, season_tree(rng, season), q, rng.int(0, 3) as f32 * 90.0, 0.8);
            sp.block(cx - 1, cy - 1, 3, 3);
        }
    }
    // Скамья у дорожки к южной двери.
    if let Some(&(door, _)) = entries.iter().find(|e| e.1 == 2) {
        let q = [door[0] + 2.2, door[1] + 3.0];
        if sp.free(q[0] as i32 - 1, q[1] as i32, 2, 1) && !on_lane(q) {
            drop(m, "bench", q, 270.0, 1.0);
            sp.block(q[0] as i32 - 1, q[1] as i32, 2, 1);
        }
    }

    // Могилы: 2×1 клетки, рядами с севера на юг, головой на запад.
    let dirt = mat(m, "dirt");
    let library = !missing("tombstone");
    let mut gy = 2;
    while gy + 1 < h - 1 {
        let mut gx = 2 + (gy / 2) % 2;
        while gx + 2 < w - 1 {
            let q = [gx as f32 + 1.0, gy as f32 + 0.5];
            let south = gy > b.y + b.h;
            let north = gy + 1 < b.y;
            let eastw = gx > b.x + b.w;
            let chance = if south || eastw { 0.8 } else if north { 0.3 } else { 0.55 };
            if sp.free(gx, gy, 2, 1) && !on_lane(q) && rng.chance(chance) {
                sp.block(gx, gy, 3, 2);
                grave(m, rng, q, library, dirt);
            }
            gx += 3;
        }
        gy += 2;
    }
}

/// Одна могила с серединой в `q` (2 клетки вдоль, 1 поперёк), изголовье — на западе.
fn grave(m: &mut Map, rng: &mut Rng, q: [f32; 2], library: bool, dirt: usize) {
    let jitter = |rng: &mut Rng| (rng.random() as f32 - 0.5) * 0.2;
    let head = [q[0] - 0.85 + jitter(rng), q[1] + jitter(rng)];
    if !library {
        let (x, y) = (q[0].floor() as i32 - 1, q[1].floor() as i32);
        m.paint_rect(Layer::Floor, (x, y), (x + 1, y), Some(dirt));
        drop(m, "boulder", head, 0.0, 0.35);
        return;
    }
    let roll = rng.random();
    let tilt = rng.int(-6, 6) as f32;
    if roll < 0.45 {
        drop(m, "grave_mound", [q[0] + 0.15, q[1]], 90.0, 0.8);
        drop(m, "tombstone", head, 90.0 + tilt, 1.0);
    } else if roll < 0.75 {
        drop(m, "grave_kerb", [q[0] + 0.1, q[1]], 90.0, 0.6);
        drop(m, "tombstone", head, 90.0, 1.0);
    } else if roll < 0.95 {
        drop(m, "tombstone", head, 90.0 + tilt * 2.0, 1.0);
    } else {
        drop(m, "grave_open", [q[0] + 0.1, q[1]], 90.0, 0.6);
    }
}

/// Спуск в крипту — пролёт ступеней FA, как в подземелье: нижняя ступень у стены,
/// низ темнеет, по бокам перила, две клетки перед сходом свободны. Своих ступеней
/// нет — встроенная лестница вниз.
fn crypt_stairs(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room) {
    let flight = !missing("stairs_down");
    let p = if flight { asset("stairs_down") } else { Palette::builtin_pick("stairs") };
    let slot = if flight { "stairs_down" } else { "stairs" };
    // Пролёт — две полосы рядом: место размечается по удвоенной ширине.
    let mut place = p.clone();
    if let Some(v) = place.visible.as_mut().filter(|_| flight) {
        // Две полосы ступеней и перила по бокам — перила не лезут на стену.
        v[1] = -1.5;
        v[3] = 1.5;
    }
    // К восточной стене (дверь ризницы — в южной), иначе к северной.
    let n0 = m.props.len();
    let Some(side) = [1, 0].into_iter().find(|&s| by_side_pick(m, sp, rng, r, &place, slot, 1.0, false, &[s])) else { return };
    if m.props.len() != n0 + 1 {
        return;
    }
    let mut st = m.props.pop().unwrap_or_else(|| unreachable!());
    let away: [f32; 2] = [[0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [1.0, 0.0]][side as usize];
    let across = [-away[1], away[0]];
    if flight {
        st.pos = [st.pos[0] - away[0] * WALL_GAP, st.pos[1] - away[1] * WALL_GAP];
    }
    let lanes: &[f32] = if flight && place.visible.is_some() { &[-0.5, 0.5] } else { &[0.0] };
    let at = |k: f32| [st.pos[0] + across[0] * k * st.scale, st.pos[1] + across[1] * k * st.scale];
    for &k in lanes {
        m.props.push(Prop { pos: at(k), ..st.clone() });
    }
    if flight {
        // Низ пролёта (у стены) темнеет: лестница ведёт вниз.
        for &k in lanes {
            drop_pick(m, &Palette::builtin_pick("stairs_shade"), at(k), st.rot, st.scale);
            if let Some(shade) = m.props.last_mut() {
                shade.layer = Some(crate::map::LAYER_DEFAULT);
            }
        }
        if !missing("stair_rail") {
            let rail = asset("stair_rail");
            let edge = lanes.iter().cloned().fold(0.0f32, f32::max) + 0.5;
            for k in [-edge, edge] {
                drop_pick(m, &rail, at(k), st.rot, st.scale);
            }
        }
    }
    // Перед сходом две клетки свободны.
    let half_len = 1.5 * st.scale;
    for d in [0.5, 1.5] {
        for &k in lanes {
            for o in [-0.25, 0.25] {
                let q = at(k + o);
                let q = [q[0] + away[0] * (half_len + d), q[1] + away[1] * (half_len + d)];
                sp.block(q[0].floor() as i32, q[1].floor() as i32, 1, 1);
            }
        }
    }
}
