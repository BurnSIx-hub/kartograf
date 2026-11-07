//! Подземелье. Как его строят ведущие и картографы:
//!
//! - **Растёт от входа**, как у Watabou (One Page Dungeon): от комнаты отходят
//!   коридоры, на их концах новые комнаты; часть ветвей парные — по обе стороны
//!   оси, как в настоящей постройке. Полная симметрия мертва, поэтому пары не всегда
//!   и не одного размера.
//! - **Кольца, а не дерево** (Jaquays, «xandering» у Justin Alexander): ветвление
//!   даёт выбор, но оставляет путь линейным; настоящий выбор — когда дальние друг
//!   от друга по ходу комнаты соединены коротким путём. Часть таких путей тайные —
//!   срезка. На большом подземелье второй вход.
//! - **Пять комнат** (Johnn Four): вход со стражей, середина, главный зал — самый
//!   большой и в глубине, за ним выход вниз; награда — в тупике в стороне от главного
//!   пути, порой за тайной дверью.
//! - **Настоящая кладка**: склепы с саркофагами, оссуарии, катакомбные галереи с
//!   гробами в нишах вдоль стен (loculi), часовни; темницы с рядами камер по сторонам
//!   прохода, караульные, пыточные; храмы с колоннадой и алтарём напротив входа,
//!   купель; руины с завалами и лужами. Большие залы держат колонны.
//! - **Вид** ([`DungeonKind`]) решает пол, стены и комнаты: пол только каменный, под
//!   вид — плиты склепа, булыжник темницы, мрамор храма, мшистые плиты руин. Свой пол
//!   и стены из плана главнее.
//! - Комнаты пронумерованы для ведущего, как на картах Dyson Logos: номер и что это.

use super::*;
use crate::plan::DungeonKind;

/// Форма комнаты: прямоугольник, круглая, восьмиугольная (срезанные углы).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Rect,
    Round,
    Octagon,
}

#[derive(Clone, Debug)]
struct Chamber {
    r: Room,
    shape: Shape,
    parent: Option<usize>,
    /// Сторона, к которой подходит коридор от родителя (у входа — от края карты).
    entry: i32,
    /// Задуман главным залом.
    big: bool,
}

impl Chamber {
    /// Клетка карты — пол этой комнаты.
    /// Точка `p` внутри формы комнаты с запасом `mg` до стены.
    fn inside(&self, p: [f32; 2], mg: f32) -> bool {
        let r = self.r;
        let (w, h) = (r.w as f32, r.h as f32);
        let (u, v) = (p[0] - r.x as f32, p[1] - r.y as f32);
        if u < mg - 1e-4 || v < mg - 1e-4 || u > w - mg + 1e-4 || v > h - mg + 1e-4 {
            return false;
        }
        match self.shape {
            Shape::Rect => true,
            Shape::Octagon => u.min(w - u) + v.min(h - v) + 1e-4 >= (r.w.min(r.h) / 3).max(1) as f32 + mg * 1.42,
            Shape::Round => {
                let (cx, cy) = (w / 2.0, h / 2.0);
                ((u - cx) / (cx - mg)).powi(2) + ((v - cy) / (cy - mg)).powi(2) <= 1.0 + 1e-4
            }
        }
    }

    /// Клетка целиком в комнате: у круглой стена не задевает ни одного её угла.
    /// Только в такие клетки ставятся предметы, иначе они уходят в стену.
    fn whole(&self, x: i32, y: i32) -> bool {
        self.has(x, y) && (self.shape != Shape::Round || [(0, 0), (1, 0), (0, 1), (1, 1)].iter().all(|&(i, j)| self.inside([(x + i) as f32, (y + j) as f32], 0.1)))
    }

    fn has(&self, x: i32, y: i32) -> bool {
        let r = self.r;
        if x < r.x || y < r.y || x >= r.x + r.w || y >= r.y + r.h {
            return false;
        }
        let (lx, ly) = (x - r.x, y - r.y);
        match self.shape {
            Shape::Rect => true,
            Shape::Octagon => {
                let c = (r.w.min(r.h) / 3).max(1);
                lx.min(r.w - 1 - lx) + ly.min(r.h - 1 - ly) >= c
            }
            Shape::Round => {
                let (cx, cy) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
                let (px, py) = (lx as f32 + 0.5 - cx, ly as f32 + 0.5 - cy);
                (px / cx).powi(2) + (py / cy).powi(2) <= 1.08
            }
        }
    }
}

/// Коридор шириной в две клетки между комнатами `a` и `b` (у второго входа `b` —
/// край карты, `usize::MAX`).
#[derive(Clone, Debug)]
struct Hall {
    a: usize,
    b: usize,
    cells: Vec<(i32, i32)>,
    door_a: Segment,
    door_b: Segment,
    secret: bool,
}

/// Чем стала комната.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Entrance,
    Climax,
    Treasure,
    Exit,
    // Склеп.
    Tomb,
    Ossuary,
    Catacomb,
    Chapel,
    // Темница.
    Cells,
    Torture,
    Guard,
    Warden,
    // Храм.
    Shrine,
    Pool,
    Library,
    // Руины.
    Collapsed,
    Lair,
    Flooded,
    Store,
    /// Роль из плана.
    Plan(Role),
}

impl Part {
    fn name(self, kind: DungeonKind) -> &'static str {
        match self {
            Part::Entrance => "Вход",
            Part::Climax => match kind {
                DungeonKind::Crypt => "Гробница",
                DungeonKind::Prison => "Яма",
                DungeonKind::Temple => "Святилище",
                DungeonKind::Ruins => "Логово",
            },
            Part::Treasure => "Тайник",
            Part::Exit => "Спуск",
            Part::Tomb => "Усыпальница",
            Part::Ossuary => "Оссуарий",
            Part::Catacomb => "Катакомба",
            Part::Chapel => "Часовня",
            Part::Cells => "Камеры",
            Part::Torture => "Пыточная",
            Part::Guard => "Караульная",
            Part::Warden => "Комната смотрителя",
            Part::Shrine => "Молельня",
            Part::Pool => "Купель",
            Part::Library => "Книгохранилище",
            Part::Collapsed => "Обвал",
            Part::Lair => "Лёжка",
            Part::Flooded => "Затопленный зал",
            Part::Store => "Кладовая",
            Part::Plan(_) => "",
        }
    }
}

/// Пол, стены и комнаты вида.
pub(super) fn floor_slot(kind: DungeonKind) -> &'static str {
    match kind {
        DungeonKind::Crypt => "crypt_floor",
        DungeonKind::Prison => "prison_floor",
        DungeonKind::Temple => "temple_floor",
        DungeonKind::Ruins => "ruin_floor",
    }
}

pub(super) fn wall_slot(kind: DungeonKind, rng: &mut Rng) -> &'static str {
    match kind {
        DungeonKind::Crypt => pick(rng, &["wall_stone_blocks", "wall_stone_blocks", "wall_brick"]),
        DungeonKind::Prison => pick(rng, &["wall_fieldstone", "wall_stone_blocks"]),
        DungeonKind::Temple => pick(rng, &["wall_sandstone", "wall_stone_blocks"]),
        DungeonKind::Ruins => pick(rng, &["wall_ruined_blocks", "wall_ruined"]),
    }
}

fn pool(kind: DungeonKind) -> &'static [Part] {
    match kind {
        DungeonKind::Crypt => &[Part::Tomb, Part::Ossuary, Part::Catacomb, Part::Chapel, Part::Tomb, Part::Catacomb, Part::Store],
        DungeonKind::Prison => &[Part::Cells, Part::Guard, Part::Torture, Part::Cells, Part::Warden, Part::Store, Part::Cells],
        DungeonKind::Temple => &[Part::Shrine, Part::Pool, Part::Library, Part::Shrine, Part::Store, Part::Chapel],
        DungeonKind::Ruins => &[Part::Collapsed, Part::Lair, Part::Flooded, Part::Store, Part::Collapsed, Part::Ossuary],
    }
}

// ----- раскладка -----

fn one(rng: &mut Rng, of: &[i32]) -> i32 {
    of[rng.int(0, of.len() as i32 - 1) as usize]
}

struct Grid {
    w: i32,
    h: i32,
    /// 0 — скала, 1 — комната, 2 — коридор.
    occ: Vec<u8>,
}

impl Grid {
    fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h { 255 } else { self.occ[(y * self.w + x) as usize] }
    }

    fn set(&mut self, x: i32, y: i32, v: u8) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.occ[(y * self.w + x) as usize] = v;
        }
    }

    /// Прямоугольник с каймой `m` — сплошь скала и внутри карты (кайма у края карты
    /// не нужна, но сама комната — не ближе клетки к краю).
    fn free(&self, r: Room, m: i32) -> bool {
        if r.x < 1 || r.y < 1 || r.x + r.w > self.w - 1 || r.y + r.h > self.h - 1 {
            return false;
        }
        (r.y - m..r.y + r.h + m).all(|y| (r.x - m..r.x + r.w + m).all(|x| matches!(self.get(x, y), 0 | 255)))
    }
}

/// Коридор от стороны `side` комнаты `p`: полоса с `s0` (вдоль стороны) длиной `len`.
/// Клетки коридора, дверь у `p` и дверь на дальнем конце.
fn strip(p: Room, side: i32, s0: i32, len: i32) -> (Vec<(i32, i32)>, Segment, Segment) {
    let mut cells = Vec::new();
    let (s, s2) = (s0 as f32, s0 as f32 + 2.0);
    let (da, db);
    match side {
        0 => {
            for k in 1..=len {
                cells.extend([(s0, p.y - k), (s0 + 1, p.y - k)]);
            }
            da = ([s, p.y as f32], [s2, p.y as f32]);
            db = ([s, (p.y - len) as f32], [s2, (p.y - len) as f32]);
        }
        2 => {
            for k in 0..len {
                cells.extend([(s0, p.y + p.h + k), (s0 + 1, p.y + p.h + k)]);
            }
            da = ([s, (p.y + p.h) as f32], [s2, (p.y + p.h) as f32]);
            db = ([s, (p.y + p.h + len) as f32], [s2, (p.y + p.h + len) as f32]);
        }
        1 => {
            for k in 0..len {
                cells.extend([(p.x + p.w + k, s0), (p.x + p.w + k, s0 + 1)]);
            }
            da = ([(p.x + p.w) as f32, s], [(p.x + p.w) as f32, s2]);
            db = ([(p.x + p.w + len) as f32, s], [(p.x + p.w + len) as f32, s2]);
        }
        _ => {
            for k in 1..=len {
                cells.extend([(p.x - k, s0), (p.x - k, s0 + 1)]);
            }
            da = ([p.x as f32, s], [p.x as f32, s2]);
            db = ([(p.x - len) as f32, s], [(p.x - len) as f32, s2]);
        }
    }
    (cells, da, db)
}

/// Клетки комнаты у её стороны `side` на месте `s0, s0 + 1` — пол (дверь встанет).
fn opens(c: &Chamber, side: i32, s0: i32) -> bool {
    let r = c.r;
    let at = |k: i32| match side {
        0 => (s0 + k, r.y),
        2 => (s0 + k, r.y + r.h - 1),
        1 => (r.x + r.w - 1, s0 + k),
        _ => (r.x, s0 + k),
    };
    (0..2).all(|k| {
        let (x, y) = at(k);
        c.has(x, y)
    })
}

fn side_len(r: Room, side: i32) -> i32 {
    if side % 2 == 0 { r.w } else { r.h }
}

/// Начало полосы двери на стороне: у прямоугольника где угодно, у круглой и
/// восьмиугольной — посередине.
fn door_at(c: &Chamber, side: i32, centered: bool, rng: &mut Rng) -> i32 {
    let len = side_len(c.r, side);
    let base = if side % 2 == 0 { c.r.x } else { c.r.y };
    if centered || c.shape != Shape::Rect || len < 6 { base + (len - 2) / 2 } else { base + rng.int(1, len - 3) }
}

/// Пристроить к комнате `pi` новую со стороны `side`. `None` — не встала.
#[allow(clippy::too_many_arguments)]
fn attach(g: &Grid, all: &[Chamber], pi: usize, side: i32, (cw, ch): (i32, i32), shape: Shape, len: i32, centered: bool, rng: &mut Rng) -> Option<(Chamber, Hall)> {
    let p = &all[pi];
    if side_len(p.r, side) < 4 {
        return None;
    }
    let s0 = door_at(p, side, centered, rng);
    if !opens(p, side, s0) {
        return None;
    }
    let mut child = Chamber { r: Room { x: 0, y: 0, w: cw, h: ch }, shape, parent: Some(pi), entry: (side + 2) % 4, big: false };
    let clen = side_len(child.r, child.entry);
    let uc = if centered || shape != Shape::Rect || clen < 6 { (clen - 2) / 2 } else { rng.int(1, clen - 3) };
    let r = p.r;
    child.r = match side {
        0 => Room { x: s0 - uc, y: r.y - len - ch, w: cw, h: ch },
        2 => Room { x: s0 - uc, y: r.y + r.h + len, w: cw, h: ch },
        1 => Room { x: r.x + r.w + len, y: s0 - uc, w: cw, h: ch },
        _ => Room { x: r.x - len - cw, y: s0 - uc, w: cw, h: ch },
    };
    if !g.free(child.r, 1) || !opens(&child, child.entry, s0) {
        return None;
    }
    let (cells, da, db) = strip(r, side, s0, len);
    // Коридор и клетка по бокам от него — скала: соседние ходы не сливаются.
    for &(x, y) in &cells {
        let (a, b) = if side % 2 == 0 { ((x - 1, y), (x + 1, y)) } else { ((x, y - 1), (x, y + 1)) };
        if g.get(x, y) != 0 || [a, b].iter().any(|&(i, j)| g.get(i, j) == 1 || g.get(i, j) == 2) {
            return None;
        }
    }
    Some((child, Hall { a: pi, b: usize::MAX, cells, door_a: da, door_b: db, secret: false }))
}

fn commit(g: &mut Grid, c: &Chamber, h: &Hall) {
    for y in c.r.y..c.r.y + c.r.h {
        for x in c.r.x..c.r.x + c.r.w {
            if c.has(x, y) {
                g.set(x, y, 1);
            }
        }
    }
    for &(x, y) in &h.cells {
        g.set(x, y, 2);
    }
}

/// Размер и форма новой комнаты.
fn chamber_size(rng: &mut Rng, lo: i32, hi: i32, big: bool) -> ((i32, i32), Shape) {
    if big {
        let (a, b) = (rng.int(hi + 1, hi + 4), rng.int(hi, hi + 3));
        let shape = if rng.chance(0.25) { Shape::Octagon } else { Shape::Rect };
        return (if rng.chance(0.5) { (a, b) } else { (b, a) }, shape);
    }
    let roll = rng.random();
    if roll < 0.14 && hi >= 5 {
        // Круглая — нечётная и квадратная: дверь ровно посередине.
        let s = rng.int(lo.max(5), hi.max(5)) | 1;
        return ((s, s), Shape::Round);
    }
    let (w, h) = (rng.int(lo, hi), rng.int(lo, hi));
    if roll < 0.28 && w.min(h) >= 6 {
        return ((w, h), Shape::Octagon);
    }
    // Изредка — длинная галерея.
    if roll > 0.88 {
        let long = rng.int(hi, hi + 4);
        return (if rng.chance(0.5) { (long, lo.max(4)) } else { (lo.max(4), long) }, Shape::Rect);
    }
    ((w, h), Shape::Rect)
}

struct Layout {
    rooms: Vec<Chamber>,
    halls: Vec<Hall>,
    /// Ворота у края карты: отрезок на краю и клетки прохода.
    gate: (Segment, Vec<(i32, i32)>),
    /// Второй вход: отрезок на краю, коридор.
    back_door: Option<Segment>,
}

fn grow(w: i32, h: i32, n: usize, lo: i32, hi: i32, loops: f32, rng: &mut Rng) -> Layout {
    let mut g = Grid { w, h, occ: vec![0; (w * h) as usize] };
    // Вход — с длинной стороны карты, чтобы подземелью было куда расти.
    let side = if w >= h { one(rng, &[1, 3]) } else { one(rng, &[0, 2]) };
    let ((cw, ch), _) = chamber_size(rng, lo, hi, false);
    let (cw, ch) = (cw.min(w - 4), ch.min(h - 4));
    let gl = rng.int(2, 3);
    let jitter = |rng: &mut Rng, span: i32, size: i32| ((span - size) / 2 + rng.int(-(span / 6), span / 6)).clamp(1, (span - size - 1).max(1));
    let first = match side {
        3 => Room { x: gl, y: jitter(rng, h, ch), w: cw, h: ch },
        1 => Room { x: w - gl - cw, y: jitter(rng, h, ch), w: cw, h: ch },
        0 => Room { x: jitter(rng, w, cw), y: gl, w: cw, h: ch },
        _ => Room { x: jitter(rng, w, cw), y: h - gl - ch, w: cw, h: ch },
    };
    let entrance = Chamber { r: first, shape: Shape::Rect, parent: None, entry: side, big: false };
    let s0 = door_at(&entrance, side, true, rng);
    // Ворота: полоса от края карты до комнаты.
    let gate_cells: Vec<(i32, i32)> = match side {
        3 => (0..first.x).flat_map(|x| [(x, s0), (x, s0 + 1)]).collect(),
        1 => (first.x + first.w..w).flat_map(|x| [(x, s0), (x, s0 + 1)]).collect(),
        0 => (0..first.y).flat_map(|y| [(s0, y), (s0 + 1, y)]).collect(),
        _ => (first.y + first.h..h).flat_map(|y| [(s0, y), (s0 + 1, y)]).collect(),
    };
    let (s, s2) = (s0 as f32, s0 as f32 + 2.0);
    let gate_seg = match side {
        3 => ([0.0, s], [0.0, s2]),
        1 => ([w as f32, s], [w as f32, s2]),
        0 => ([s, 0.0], [s2, 0.0]),
        _ => ([s, h as f32], [s2, h as f32]),
    };
    let mut rooms = vec![entrance];
    let mut halls: Vec<Hall> = Vec::new();
    commit(&mut g, &rooms[0], &Hall { a: 0, b: 0, cells: gate_cells.clone(), door_a: gate_seg, door_b: gate_seg, secret: false });
    // Свободные стороны комнат: (комната, сторона, неудач).
    let mut frontier: Vec<(usize, i32, i32)> = (0..4).filter(|s| *s != side).map(|s| (0, s, 0)).collect();
    let big_at = (n * 2 / 3).max(2);
    let mut big_done = false;
    let mut tries = 0;
    while rooms.len() < n && !frontier.is_empty() && tries < n * 80 {
        tries += 1;
        let fi = rng.int(0, frontier.len() as i32 - 1) as usize;
        let (pi, sd, fails) = frontier[fi];
        let big = !big_done && rooms.len() >= big_at;
        let (dims, shape) = chamber_size(rng, lo, hi, big);
        let len = one(rng, &[1, 2, 2, 3, 3, 4, 5]);
        let symmetric = rng.chance(0.35) && rooms[pi].entry % 2 != sd % 2;
        match attach(&g, &rooms, pi, sd, dims, shape, len, symmetric, rng) {
            Some((mut c, mut hall)) => {
                c.big = big;
                big_done |= big;
                let ci = rooms.len();
                hall.b = ci;
                commit(&mut g, &c, &hall);
                frontier.swap_remove(fi);
                for s in (0..4).filter(|s| *s != c.entry) {
                    frontier.push((ci, s, 0));
                }
                rooms.push(c);
                halls.push(hall);
                // Пара по другую сторону оси — тех же размеров и формы.
                if symmetric && rooms.len() < n {
                    let other = (sd + 2) % 4;
                    if let Some(k) = frontier.iter().position(|f| f.0 == pi && f.1 == other)
                        && let Some((c2, mut h2)) = attach(&g, &rooms, pi, other, dims, shape, len, true, rng)
                    {
                        let ci2 = rooms.len();
                        h2.b = ci2;
                        commit(&mut g, &c2, &h2);
                        frontier.swap_remove(k);
                        for s in (0..4).filter(|s| *s != c2.entry) {
                            frontier.push((ci2, s, 0));
                        }
                        rooms.push(c2);
                        halls.push(h2);
                    }
                }
            }
            None if fails >= 5 => {
                frontier.swap_remove(fi);
            }
            None => frontier[fi].2 += 1,
        }
    }
    // Кольца: комнаты, далёкие по ходу, но близкие на карте, — прямым коридором.
    // Шагов по дереву между комнатами: до общего предка и обратно.
    let tree = |a: usize, b: usize| -> usize {
        let chain = |mut i: usize| {
            let mut v = vec![i];
            while let Some(p) = rooms[i].parent {
                v.push(p);
                i = p;
            }
            v
        };
        let (ca, cb) = (chain(a), chain(b));
        cb.iter().enumerate().find_map(|(j, x)| ca.iter().position(|y| y == x).map(|i| i + j)).unwrap_or(ca.len() + cb.len())
    };
    let want = if loops <= 0.02 { 0 } else { ((rooms.len() as f32 * loops * 0.5).round() as usize).max(1) };
    let mut cands: Vec<(usize, usize, usize)> = Vec::new();
    for a in 0..rooms.len() {
        for b in a + 1..rooms.len() {
            let d = tree(a, b);
            if d >= 3 {
                cands.push((d, a, b));
            }
        }
    }
    cands.sort_by_key(|c| std::cmp::Reverse(c.0));
    let mut made = 0;
    for (_, a, b) in cands {
        if made >= want {
            break;
        }
        if let Some(hall) = bridge(&g, &rooms, a, b) {
            let mut hall = hall;
            hall.secret = rng.chance(0.35);
            for &(x, y) in &hall.cells {
                g.set(x, y, 2);
            }
            halls.push(hall);
            made += 1;
        }
    }
    // Второй вход — у большого подземелья: из глубокой комнаты прямо к краю карты.
    let mut back_door = None;
    if rooms.len() >= 7 && rng.chance(0.7) {
        let mut best: Option<(usize, Vec<(i32, i32)>, Segment, Segment)> = None;
        for (i, c) in rooms.iter().enumerate().skip(1) {
            for sd in (0..4).filter(|s| *s != side && *s != c.entry) {
                let s0 = door_at(c, sd, true, rng);
                if side_len(c.r, sd) < 4 || !opens(c, sd, s0) {
                    continue;
                }
                let dist = match sd {
                    0 => c.r.y,
                    2 => h - c.r.y - c.r.h,
                    1 => w - c.r.x - c.r.w,
                    _ => c.r.x,
                };
                if !(1..=6).contains(&dist) {
                    continue;
                }
                let (cells, da, db) = strip(c.r, sd, s0, dist);
                let clear = cells.iter().all(|&(x, y)| {
                    let (a, b) = if sd % 2 == 0 { ((x - 1, y), (x + 1, y)) } else { ((x, y - 1), (x, y + 1)) };
                    g.get(x, y) == 0 && [a, b].iter().all(|&(i, j)| matches!(g.get(i, j), 0 | 255))
                });
                if clear && best.as_ref().is_none_or(|b| tree(0, i) > tree(0, b.0)) {
                    best = Some((i, cells, da, db));
                }
            }
        }
        if let Some((i, cells, da, db)) = best {
            for &(x, y) in &cells {
                g.set(x, y, 2);
            }
            halls.push(Hall { a: i, b: usize::MAX, cells, door_a: da, door_b: db, secret: false });
            back_door = Some(db);
        }
    }
    Layout { rooms, halls, gate: (gate_seg, gate_cells), back_door }
}

/// Прямой коридор между комнатами `a` и `b`, если они друг против друга и между ними
/// только скала.
fn bridge(g: &Grid, rooms: &[Chamber], a: usize, b: usize) -> Option<Hall> {
    let (ra, rb) = (rooms[a].r, rooms[b].r);
    // Одна над другой: общий отрезок по x.
    for vertical in [true, false] {
        let (lo, hi) = if vertical { (ra.x.max(rb.x), (ra.x + ra.w).min(rb.x + rb.w)) } else { (ra.y.max(rb.y), (ra.y + ra.h).min(rb.y + rb.h)) };
        if hi - lo < 4 {
            continue;
        }
        let (top, bot) = if vertical {
            if ra.y + ra.h <= rb.y { (a, b) } else if rb.y + rb.h <= ra.y { (b, a) } else { continue }
        } else if ra.x + ra.w <= rb.x {
            (a, b)
        } else if rb.x + rb.w <= ra.x {
            (b, a)
        } else {
            continue;
        };
        let (rt, rbt) = (rooms[top].r, rooms[bot].r);
        let gap = if vertical { rbt.y - rt.y - rt.h } else { rbt.x - rt.x - rt.w };
        if !(1..=8).contains(&gap) {
            continue;
        }
        let mid = (lo + hi) / 2 - 1;
        let mut order: Vec<i32> = (lo + 1..=hi - 3).collect();
        order.sort_by_key(|s| (s - mid).abs());
        for s0 in order {
            let (sa, sb) = if vertical { (2, 0) } else { (1, 3) };
            if !opens(&rooms[top], sa, s0) || !opens(&rooms[bot], sb, s0) {
                continue;
            }
            let (cells, da, db) = strip(rt, sa, s0, gap);
            let clear = cells.iter().all(|&(x, y)| {
                let (p, q) = if vertical { ((x - 1, y), (x + 1, y)) } else { ((x, y - 1), (x, y + 1)) };
                g.get(x, y) == 0 && [p, q].iter().all(|&(i, j)| matches!(g.get(i, j), 0 | 255))
            });
            if clear {
                return Some(Hall { a: top, b: bot, cells, door_a: da, door_b: db, secret: false });
            }
        }
    }
    None
}

// ----- всё вместе -----

pub(super) fn dungeon(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    let kind = p.options.dungeon;
    let [lo, hi] = p.options.room_size;
    let (lo, hi) = ((lo as i32).max(4), (hi as i32).max(lo as i32 + 1).max(5));
    let n = p.rooms.len().max(1);
    let lay = grow(w, h, n, lo, hi, p.options.loops, &mut Rng::new(&format!("{}/dungeon", p.seed)));
    let rooms = &lay.rooms;
    for spec in p.rooms[rooms.len().min(p.rooms.len())..].iter().filter(|r| r.role != Role::Auto || !r.name.is_empty() || !r.objects.is_empty()) {
        warn.push(format!("комната «{}» не поместилась", spec.id));
    }
    // Пол и стены — по виду; свои из плана главнее. Стены, косяки и камень
    // предметов — одного тона.
    let floor = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or(floor_slot(kind)));
    let walls = wall_slot(kind, rng);
    let tone = tone_of(kind, rng);
    m.wall_style = match p.style.walls.as_deref().and_then(plan::wall_id) {
        Some(own) => wall(own),
        None => wall_toned(walls, tone),
    };
    let kit = Kit::new(tone);
    ground(m, "rocky_ground");
    // Комнаты — по маске, мельче клетки: круглые и восьмиугольные с ровным краем,
    // а не лесенкой из клеток.
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut weights: Vec<(usize, f32)> = Vec::new();
    for c in rooms {
        weights.extend(shape_mask(c, mw, mh));
    }
    let mut cells: Vec<(i32, i32)> = Vec::new();
    for hall in &lay.halls {
        cells.extend(hall.cells.iter().copied());
    }
    cells.extend(lay.gate.1.iter().copied());
    // Круглая комната у оси чуть отступает от своей рамки: коридор доводится до неё
    // клеткой внутри, иначе у двери щель.
    for hall in &lay.halls {
        for (seg, i) in [(hall.door_a, hall.a), (hall.door_b, hall.b)] {
            let Some(c) = rooms.get(i).filter(|c| c.shape == Shape::Round) else { continue };
            let (x, y) = (seg.0[0] as i32, seg.0[1] as i32);
            let horizontal = seg.0[1] == seg.1[1];
            let (x, y) = if horizontal { (x, if c.has(x, y) { y } else { y - 1 }) } else { (if c.has(x, y) { x } else { x - 1 }, y) };
            cells.extend(if horizontal { [(x, y), (x + 1, y)] } else { [(x, y), (x, y + 1)] });
        }
    }
    weights.extend(cells.iter().flat_map(|&c| mask::cells(mw, mh, c, c)));
    m.paint(Layer::Rooms, floor, &weights);
    for (spec, c) in p.rooms.iter().zip(rooms) {
        if let Some(t) = spec.floor.as_deref().and_then(plan::texture_id) {
            let t = mat(m, t);
            let own: Vec<(usize, f32)> = (c.r.y..c.r.y + c.r.h).flat_map(|y| (c.r.x..c.r.x + c.r.w).map(move |x| (x, y))).filter(|&(x, y)| c.has(x, y)).flat_map(|c| mask::cells(mw, mh, c, c)).collect();
            m.paint(Layer::Rooms, t, &own);
        }
    }

    // Что где: вход, главный зал и за ним спуск, тайник в тупике в стороне.
    let parts = assign(p, rooms, kind, rng);
    // Вода — в купели и затопленном зале, до стен: стены по краю пола.
    for (i, c) in rooms.iter().enumerate() {
        if matches!(parts[i], Part::Pool | Part::Flooded) {
            water(m, rng, c, parts[i] == Part::Pool);
        }
    }
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    dark_outside(m, if p.mood == Mood::Gloomy { 0.95 } else { 0.85 });

    // Ворота, второй вход, двери коридоров.
    let gate_kind = p.links.iter().find(|l| l.touches(OUTSIDE)).map_or(LinkKind::DoubleDoor, |l| l.kind);
    let (ga, gb) = lay.gate.0;
    m.portals.push(match gate_kind {
        LinkKind::Opening => opening(ga, gb),
        LinkKind::Secret => portal(ga, gb, PortalKind::Secret, false),
        _ => door(m, ga, gb, PortalKind::DoubleDoor, &kit),
    });
    if let Some((a, b)) = lay.back_door {
        m.portals.push(if kind == DungeonKind::Ruins { opening(a, b) } else { portal(a, b, PortalKind::Secret, false) });
    }
    // Тайник прячется за тайной дверью — через раз.
    let hidden = parts.iter().position(|p| *p == Part::Treasure).filter(|_| rng.chance(0.5));
    let mut sp = Spots::new(m);
    for hall in &lay.halls {
        let to_hidden = hidden.is_some_and(|t| hall.b == t && rooms[t].parent == Some(hall.a));
        let ends: [(Segment, usize); 2] = [(hall.door_a, hall.a), (hall.door_b, hall.b)];
        for (k, (seg, room)) in ends.into_iter().enumerate() {
            if room == usize::MAX {
                continue;
            }
            block_door(&mut sp, &rooms[room], seg);
            let kind_here = if hall.secret || to_hidden {
                // Тайная — с одной стороны; с другой коридор виден.
                if k == 1 { Some(PortalKind::Secret) } else { None }
            } else {
                let doors = match kind {
                    DungeonKind::Prison => 0.7,
                    DungeonKind::Crypt => 0.45,
                    DungeonKind::Temple => 0.4,
                    DungeonKind::Ruins => 0.15,
                };
                (k == 1 || rng.chance(0.3)).then_some(()).filter(|_| rng.chance(doors)).map(|_| PortalKind::DoubleDoor)
            };
            if let Some(pk) = kind_here {
                let d = door(m, seg.0, seg.1, pk, &kit);
                m.portals.push(d);
            }
        }
    }

    // Подписи для ведущего: номер по ходу от входа и что это.
    let order = bfs(rooms, &lay.halls);
    for (num, &i) in order.iter().enumerate() {
        let c = &rooms[i];
        let spec = p.rooms.get(i);
        let name = spec.map(|s| s.name.trim()).filter(|s| !s.is_empty()).unwrap_or(parts[i].name(kind));
        let text = if name.is_empty() { format!("{}", num + 1) } else { format!("{}. {name}", num + 1) };
        let mut l = Label::new(text, [c.r.x as f32 + c.r.w as f32 / 2.0, c.r.y as f32 + c.r.h as f32 / 2.0]);
        l.size = 0.5;
        l.gm = true;
        m.labels.push(l);
    }

    if p.options.fill {
        for (i, c) in rooms.iter().enumerate() {
            // Вне формы комнаты и под её стеной — скала: туда ничего не ставить.
            // Кольцо клеток вокруг — чтобы крупный предмет не вылез за стену.
            let r = c.r;
            for y in r.y..r.y + r.h {
                for x in r.x..r.x + r.w {
                    if !c.whole(x, y) {
                        sp.block(x, y, 1, 1);
                    }
                }
            }
            sp.block(r.x - 1, r.y - 1, r.w + 2, 1);
            sp.block(r.x - 1, r.y + r.h, r.w + 2, 1);
            sp.block(r.x - 1, r.y, 1, r.h);
            sp.block(r.x + r.w, r.y, 1, r.h);
            let doors: Vec<Segment> = lay.halls.iter().filter(|hl| hl.a == i || hl.b == i).map(|hl| if hl.a == i { hl.door_a } else { hl.door_b }).collect();
            // Спуск — первым: у стены напротив входа (в главном зале — у боковой),
            // и место перед ним свободно.
            if parts[i] == Part::Exit || (parts[i] == Part::Climax && !parts.contains(&Part::Exit)) {
                stairs_down(m, &mut sp, rng, &kit, c, parts[i] == Part::Exit);
            }
            // Главное в комнате — до прохода: саркофаг, алтарь, статуя встают на своё место.
            let at = centrepiece(m, &mut sp, &kit, kind, parts[i], c);
            // Путь между дверями свободен: проходная комната не перегорожена.
            let inner: Vec<[f32; 2]> = doors.iter().map(|d| inside(c, *d)).collect();
            for w in inner.windows(2) {
                walkway(&mut sp, w[0], w[1], c.r);
            }
            // Камеры — в тупике с одной дверью; иначе эта комната — пыточная.
            let jail = parts[i] == Part::Cells && doors.len() == 1 && jail_cells(m, &mut sp, rng, &kit, c, doors[0]);
            if !jail {
                dress(m, &mut sp, rng, &kit, kind, parts[i], c, at);
            }
            if p.mood == Mood::Abandoned {
                furnish(m, &mut sp, rng, c.r, Role::Ruin);
            }
            if let Some(spec) = p.rooms.get(i) {
                wanted(m, &mut sp, rng, c.r, &spec.objects, &format!("комната «{}»", spec.id), warn);
            }
        }
        wanted(m, &mut sp, rng, rooms[0].r, &p.objects, "подземелье", warn);
    }
    rooms.len()
}

/// Клетки маски комнаты (маска мельче клетки в `mask::RES` раз).
fn shape_mask(c: &Chamber, mw: usize, mh: usize) -> Vec<(usize, f32)> {
    let r = c.r;
    let (a, b) = ((r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1));
    match c.shape {
        Shape::Rect => mask::cells(mw, mh, a, b),
        Shape::Round => mask::oval(mw, mh, a, b),
        Shape::Octagon => {
            // Срез углов под 45°: сумма расстояний до ближних сторон не меньше среза.
            let k = mask::RES as f32;
            let cut = (r.w.min(r.h) / 3).max(1) as f32;
            let mut out = Vec::new();
            for j in (r.y as usize * mask::RES)..((r.y + r.h) as usize * mask::RES).min(mh) {
                for i in (r.x as usize * mask::RES)..((r.x + r.w) as usize * mask::RES).min(mw) {
                    let (u, v) = ((i as f32 + 0.5) / k - r.x as f32, (j as f32 + 0.5) / k - r.y as f32);
                    if u.min(r.w as f32 - u) + v.min(r.h as f32 - v) >= cut {
                        out.push((j * mw + i, 1.0));
                    }
                }
            }
            out
        }
    }
}

/// Комнаты по ходу от входа (через все коридоры).
fn bfs(rooms: &[Chamber], halls: &[Hall]) -> Vec<usize> {
    let mut seen = vec![false; rooms.len()];
    let mut out = vec![0];
    seen[0] = true;
    let mut k = 0;
    while k < out.len() {
        let i = out[k];
        k += 1;
        let mut next: Vec<usize> = halls.iter().filter_map(|h| if h.a == i { Some(h.b) } else if h.b == i { Some(h.a) } else { None }).filter(|&j| j < rooms.len() && !seen[j]).collect();
        next.sort();
        for j in next {
            seen[j] = true;
            out.push(j);
        }
    }
    out
}

/// Назначение комнат. Роли из плана главнее.
fn assign(p: &Plan, rooms: &[Chamber], kind: DungeonKind, rng: &mut Rng) -> Vec<Part> {
    let n = rooms.len();
    let area = |c: &Chamber| c.r.w * c.r.h;
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(pp) = rooms[i].parent {
            d += 1;
            i = pp;
        }
        d
    };
    let mut parts = vec![Part::Store; n];
    parts[0] = Part::Entrance;
    if n == 1 {
        return parts;
    }
    let climax = rooms.iter().position(|c| c.big).unwrap_or_else(|| (1..n).max_by_key(|&i| area(&rooms[i]) + 6 * depth(i)).unwrap_or(1));
    parts[climax] = Part::Climax;
    let leaf = |i: usize| !rooms.iter().any(|c| c.parent == Some(i));
    // Главный путь: от входа к главному залу.
    let mut path = vec![climax];
    while let Some(pp) = rooms[*path.last().unwrap()].parent {
        path.push(pp);
    }
    // Спуск — тупик, лучше прямоугольный (в круглом пролёт не встаёт поперёк
    // стены) и за главным залом, затем поглубже; лестница в самом главном зале —
    // только если тупиков нет вовсе.
    let exit = (1..n).filter(|&i| leaf(i) && i != climax).max_by_key(|&i| (rooms[i].shape == Shape::Rect, rooms[i].parent == Some(climax), depth(i)));
    if let Some(e) = exit {
        parts[e] = Part::Exit;
    }
    // Тайник — самый глубокий тупик в стороне от главного пути.
    if let Some(t) = (1..n).filter(|&i| leaf(i) && !path.contains(&i) && Some(i) != exit).max_by_key(|&i| depth(i)) {
        parts[t] = Part::Treasure;
    }
    let pool = pool(kind);
    let mut k = rng.int(0, pool.len() as i32 - 1) as usize;
    for part in parts.iter_mut().skip(1) {
        if *part == Part::Store {
            *part = pool[k % pool.len()];
            k += 1;
        }
    }
    // Камеры строятся в тупике прямоугольной комнаты: «камеры» меняются местами с
    // тем, что досталось таким тупикам.
    let dead_end = |i: usize| leaf(i) && rooms[i].shape == Shape::Rect && rooms[i].r.w.min(rooms[i].r.h) >= 5;
    for i in 1..n {
        if parts[i] == Part::Cells && !dead_end(i)
            && let Some(j) = (1..n).find(|&j| dead_end(j) && !matches!(parts[j], Part::Cells | Part::Treasure | Part::Exit | Part::Climax))
        {
            parts.swap(i, j);
        }
    }
    // Роли плана — поверх.
    for (i, spec) in p.rooms.iter().enumerate().take(n) {
        if spec.role != Role::Auto {
            parts[i] = match spec.role {
                Role::Entrance => Part::Entrance,
                Role::Exit => Part::Exit,
                Role::Treasure => Part::Treasure,
                r => Part::Plan(r),
            };
        }
    }
    // Лестница нужна всегда: нет спуска — она в главном зале (см. `dress`).
    parts
}

/// Около двери внутри комнаты свободно.
fn block_door(sp: &mut Spots, c: &Chamber, seg: Segment) {
    let (a, b) = seg;
    if a[1] == b[1] {
        let y = a[1] as i32;
        let x = a[0] as i32;
        let row = if c.has(x, y) { y } else { y - 2 };
        sp.block(x - 1, row, 4, 2);
    } else {
        let x = a[0] as i32;
        let y = a[1] as i32;
        let col = if c.has(x, y) { x } else { x - 2 };
        sp.block(col, y - 1, 2, 4);
    }
}

/// Вода: в купели — бассейн посередине, в затопленном зале — разливы пятнами.
fn water(m: &mut Map, rng: &mut Rng, c: &Chamber, pool: bool) {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let wet = mat(m, "water_shallow");
    let r = c.r;
    // Вода — пятна, а не клетки: круги (у купели — овал посередине) с рваной
    // шумом кромкой и мягким краем, по точкам маски.
    let (rw, rh) = (r.w as f32, r.h as f32);
    let blobs: Vec<([f32; 2], [f32; 2])> = if pool {
        vec![([r.x as f32 + rw / 2.0, r.y as f32 + rh / 2.0], [(rw / 3.0).max(1.2), (rh / 3.0).max(1.2)])]
    } else {
        (0..(r.w * r.h / 12).max(2))
            .map(|_| {
                let at = [r.x as f32 + 1.0 + (rng.random() as f32) * (rw - 2.0), r.y as f32 + 1.0 + (rng.random() as f32) * (rh - 2.0)];
                let rad = 0.9 + (rng.random() as f32) * 1.6;
                (at, [rad * (0.8 + (rng.random() as f32) * 0.4), rad * (0.8 + (rng.random() as f32) * 0.4)])
            })
            .collect()
    };
    let seed = rng.int(0, 1 << 20) as u32;
    let k = mask::RES as f32;
    let mut weights = Vec::new();
    for j in (r.y as usize * mask::RES)..((r.y + r.h) as usize * mask::RES).min(mh) {
        for i in (r.x as usize * mask::RES)..((r.x + r.w) as usize * mask::RES).min(mw) {
            let p = [(i as f32 + 0.5) / k, (j as f32 + 0.5) / k];
            if !c.inside(p, 0.05) {
                continue;
            }
            let f = blobs.iter().map(|(o, rad)| 1.0 - (((p[0] - o[0]) / rad[0]).powi(2) + ((p[1] - o[1]) / rad[1]).powi(2)).sqrt()).fold(f32::MIN, f32::max);
            let f = f + (wobble(p[0] * 1.3, p[1] * 1.3, seed) - 0.5) * 0.55 + (wobble(p[0] * 3.1, p[1] * 3.1, seed ^ 0x9e37) - 0.5) * 0.2;
            let w = ((f + 0.05) / 0.25).clamp(0.0, 1.0);
            if w > 0.0 {
                weights.push((j * mw + i, w * w * (3.0 - 2.0 * w)));
            }
        }
    }
    if !weights.is_empty() {
        m.paint(Layer::Rooms, wet, &weights);
    }
}

/// Гладкий шум 0..1 по сетке целых точек: кромка воды.
fn wobble(x: f32, y: f32, seed: u32) -> f32 {
    let h = |i: i32, j: i32| -> f32 {
        let mut v = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5bd1_e995);
        v ^= v >> 15;
        (v & 0xffff) as f32 / 65535.0
    };
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - xi as f32, y - yi as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = h(xi, yi) + (h(xi + 1, yi) - h(xi, yi)) * sx;
    let low = h(xi, yi + 1) + (h(xi + 1, yi + 1) - h(xi, yi + 1)) * sx;
    top + (low - top) * sy
}

// ----- обстановка -----

/// Точка на полклетки внутрь комнаты от середины двери.
fn inside(c: &Chamber, seg: Segment) -> [f32; 2] {
    let mid = [(seg.0[0] + seg.1[0]) / 2.0, (seg.0[1] + seg.1[1]) / 2.0];
    if seg.0[1] == seg.1[1] {
        let y = seg.0[1] as i32;
        [mid[0], if c.has(seg.0[0] as i32, y) { y as f32 + 0.5 } else { y as f32 - 0.5 }]
    } else {
        let x = seg.0[0] as i32;
        [if c.has(x, seg.0[1] as i32) { x as f32 + 0.5 } else { x as f32 - 0.5 }, mid[1]]
    }
}

/// Один вариант каждого предмета на всё подземелье и один камень: колонны всюду
/// одни, жаровни одного вида, саркофаги и алтари из того же камня, что колонны.
struct Kit {
    pillar: Pick,
    broken: Pick,
    sarcophagus: Pick,
    altar: Pick,
    statue: Pick,
    brazier: Pick,
    coffin: Pick,
    candles: Pick,
    bones: Pick,
    cage: Pick,
    rack: Pick,
    chest: Pick,
    bench: Pick,
    throne: Pick,
    stairs: Pick,
    /// Косяки FA в тон: на одну клетку и на две; решётка камеры.
    frame1: Pick,
    frame2: Pick,
    bars: Pick,
    /// Одна дверь на всё подземелье: окованная, иначе деревянная.
    door: Pick,
    /// Перила лестницы FA в тон, если есть.
    rail: Option<Pick>,
}

/// Тон камня — слово в именах FA: Slate, Volcanic, Sandstone, Earthy, Redrock.
fn tone_of(kind: DungeonKind, rng: &mut Rng) -> &'static str {
    match kind {
        DungeonKind::Crypt => pick(rng, &["slate", "volcanic"]),
        DungeonKind::Prison => pick(rng, &["volcanic", "slate"]),
        DungeonKind::Temple => "sandstone",
        DungeonKind::Ruins => pick(rng, &["earthy", "redrock"]),
    }
}

impl Kit {
    fn new(tone: &str) -> Self {
        let t = |slot: &str| asset_toned(slot, tone);
        Kit {
            stairs: if missing("stairs_down") { Palette::builtin_pick("stairs") } else { t("stairs_down") },
            frame1: t("frame_door_stone"),
            frame2: t("frame_double_stone"),
            bars: asset("door_bars"),
            door: asset(if missing("door_wood_iron") { "door_wood" } else { "door_wood_iron" }),
            rail: (!missing("stair_rail")).then(|| t("stair_rail")),
            pillar: t(or("pillar_stone", "pillar")),
            broken: t(or("pillar_broken", "rubble")),
            sarcophagus: t(or("sarcophagus", "altar")),
            altar: t("altar"),
            statue: t("statue_figure"),
            brazier: asset("brazier"),
            coffin: asset(or("coffin", "chest")),
            candles: asset(or("candles", "brazier")),
            bones: asset(or("bones", "rubble")),
            cage: asset(or("cage", "stocks")),
            rack: asset("weapon_rack"),
            chest: asset(or("treasure_chest", "chest")),
            bench: asset("bench"),
            throne: asset(or("throne", "statue")),
        }
    }
}

/// Слот или запасной, если в библиотеке и среди встроенных его нет.
fn or<'a>(slot: &'a str, fallback: &'a str) -> &'a str {
    if missing(slot) { fallback } else { slot }
}

/// Колонны рядами вдоль длинной стороны, в двух клетках от стен; у руин часть
/// обломаны.
fn colonnade(m: &mut Map, sp: &mut Spots, c: &Chamber, kit: &Kit, broken: bool, rng: &mut Rng) {
    let r = c.r;
    if r.w.min(r.h) < 7 {
        return;
    }
    let horiz = r.w >= r.h;
    let (len, across) = if horiz { (r.w, r.h) } else { (r.h, r.w) };
    let lines = [2, across - 3];
    let mut u = 2;
    while u < len - 2 {
        for &v in &lines {
            let (x, y) = if horiz { (r.x + u, r.y + v) } else { (r.x + v, r.y + u) };
            if !c.has(x, y) {
                continue;
            }
            let (p, slot) = if broken && rng.chance(0.4) { (&kit.broken, "pillar_broken") } else { (&kit.pillar, "pillar_stone") };
            put_pick(m, sp, p, slot, x, y, 0.0, 1.0);
        }
        u += 3;
    }
}

/// Предмет посередине комнаты, спиной к стороне `back`; занятый прямоугольник.
fn centre(m: &mut Map, sp: &mut Spots, c: &Chamber, p: &Pick, slot: &str, back: i32, scale: f32) -> Option<Room> {
    let rot = (back as f32 * 90.0 + p.turn) % 360.0;
    let [l, t, r, u] = bounds(p, slot, rot, scale);
    let (fw, fh) = (cells_of(r - l), cells_of(u - t));
    let (x, y) = (c.r.x + (c.r.w - fw) / 2, c.r.y + (c.r.h - fh) / 2);
    put_pick(m, sp, p, slot, x, y, rot, scale).then_some(Room { x, y, w: fw, h: fh })
}

/// Предмет посередине стены напротив входа, спиной к ней; занятый прямоугольник.
fn far_mid(m: &mut Map, sp: &mut Spots, c: &Chamber, p: &Pick, slot: &str, scale: f32) -> Option<Room> {
    let far = (c.entry + 2) % 4;
    let rot = (far as f32 * 90.0 + p.turn) % 360.0;
    let [l, t, rr, u] = bounds(p, slot, rot, scale);
    let (fw, fh) = (cells_of(rr - l), cells_of(u - t));
    let r = c.r;
    let (x, y) = match far {
        0 => (r.x + (r.w - fw) / 2, r.y),
        2 => (r.x + (r.w - fw) / 2, r.y + r.h - fh),
        1 => (r.x + r.w - fw, r.y + (r.h - fh) / 2),
        _ => (r.x, r.y + (r.h - fh) / 2),
    };
    // У круглой и восьмиугольной комнаты середина стены отступает — внутрь, пока
    // предмет не встанет в целые клетки.
    let step = match far {
        0 => (0, 1),
        2 => (0, -1),
        1 => (-1, 0),
        _ => (1, 0),
    };
    let fits = |x: i32, y: i32| (0..fw).all(|i| (0..fh).all(|j| c.whole(x + i, y + j)));
    let inset = (0..r.w.max(r.h) / 2).find(|&k| fits(x + step.0 * k, y + step.1 * k))?;
    let (x, y) = (x + step.0 * inset, y + step.1 * inset);
    if !sp.free(x, y, fw, fh) {
        return None;
    }
    sp.block(x, y, fw, fh);
    let mut pos = [x as f32 + fw as f32 / 2.0 - (l + rr) / 2.0, y as f32 + fh as f32 / 2.0 - (t + u) / 2.0];
    // Вплотную к стене.
    if inset == 0 {
        match far {
            0 => pos[1] = y as f32 + WALL_GAP - t,
            2 => pos[1] = (y + fh) as f32 - WALL_GAP - u,
            1 => pos[0] = (x + fw) as f32 - WALL_GAP - rr,
            _ => pos[0] = x as f32 + WALL_GAP - l,
        }
    } else {
        pos = slide(c, pos, [l, t, rr, u], [-step.0 as f32, -step.1 as f32]);
    }
    drop_pick(m, p, pos, rot, scale);
    Some(Room { x, y, w: fw, h: fh })
}

/// Сдвинуть предмет с видимой рамкой `b` от точки `pos` в сторону `dir`, пока он
/// целиком в форме комнаты с запасом до стены: к кривой стене — вплотную.
fn slide(c: &Chamber, mut pos: [f32; 2], b: [f32; 4], dir: [f32; 2]) -> [f32; 2] {
    let ok = |p: [f32; 2]| [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]].iter().all(|o| c.inside([p[0] + o[0], p[1] + o[1]], WALL_GAP));
    for _ in 0..40 {
        let next = [pos[0] + dir[0] * 0.05, pos[1] + dir[1] * 0.05];
        if !ok(next) {
            break;
        }
        pos = next;
    }
    pos
}

/// Предмет спиной к стене `side` в доле `t` вдоль неё (0 и 1 — углы). Не дальше
/// `gap` клеток от любого из `near` — тогда `None`. В круглой комнате отступает
/// внутрь до целых клеток и прижимается к кривой стене.
#[allow(clippy::too_many_arguments)]
fn wall_spot(m: &mut Map, sp: &mut Spots, c: &Chamber, p: &Pick, slot: &str, side: i32, t: f32, near: &[[f32; 2]]) -> Option<([f32; 2], Room)> {
    let rot = (side as f32 * 90.0 + p.turn) % 360.0;
    let [l, tp, rr, u] = bounds(p, slot, rot, 1.0);
    let (fw, fh) = (cells_of(rr - l), cells_of(u - tp));
    let r = c.r;
    if fw > r.w || fh > r.h {
        return None;
    }
    // От ближнего конца стены: доли t и 1 − t встают зеркально.
    let along = |lo: i32, span: i32| if t <= 0.5 { lo + (span as f32 * t).round() as i32 } else { lo + span - (span as f32 * (1.0 - t)).round() as i32 };
    let (out, end) = match side {
        0 => ([0.0, -1.0], [if t < 0.5 { -1.0 } else { 1.0 }, 0.0]),
        2 => ([0.0, 1.0], [if t < 0.5 { -1.0 } else { 1.0 }, 0.0]),
        1 => ([1.0, 0.0], [0.0, if t < 0.5 { -1.0 } else { 1.0 }]),
        _ => ([-1.0, 0.0], [0.0, if t < 0.5 { -1.0 } else { 1.0 }]),
    };
    for inset in 0..(r.w.min(r.h) / 2).max(1) {
        let (x, y) = match side {
            0 => (along(r.x, r.w - fw), r.y + inset),
            2 => (along(r.x, r.w - fw), r.y + r.h - fh - inset),
            1 => (r.x + r.w - fw - inset, along(r.y, r.h - fh)),
            _ => (r.x + inset, along(r.y, r.h - fh)),
        };
        if !(0..fw).all(|i| (0..fh).all(|j| c.whole(x + i, y + j))) {
            continue;
        }
        let mid = [x as f32 + fw as f32 / 2.0, y as f32 + fh as f32 / 2.0];
        if !sp.free(x, y, fw, fh) || near.iter().any(|q| (q[0] - mid[0]).abs().max((q[1] - mid[1]).abs()) < 2.5) {
            return None;
        }
        sp.block(x, y, fw, fh);
        let mut pos = [mid[0] - (l + rr) / 2.0, mid[1] - (tp + u) / 2.0];
        pos = slide(c, pos, [l, tp, rr, u], out);
        if t <= 0.0 || t >= 1.0 {
            pos = slide(c, pos, [l, tp, rr, u], end);
        }
        drop_pick(m, p, pos, rot, 1.0);
        return Some((mid, Room { x, y, w: fw, h: fh }));
    }
    None
}

/// `n` одинаковых предметов у стен `sides`: сперва по углам, потом вдоль стены
/// врозь — не впритык друг к другу. Сколько встало.
fn spread(m: &mut Map, sp: &mut Spots, c: &Chamber, p: &Pick, slot: &str, n: i32, sides: &[i32]) -> i32 {
    // Пара у одной стены — зеркально: обе в доле t и 1 − t или ни одной.
    if n == 2 && sides.len() == 1 {
        for t in [0.0, 0.15, 0.3, 0.4] {
            let Some((_, a)) = wall_spot(m, sp, c, p, slot, sides[0], t, &[]) else { continue };
            if wall_spot(m, sp, c, p, slot, sides[0], 1.0 - t, &[]).is_some() {
                return 2;
            }
            m.props.pop();
            sp.unblock(a.x, a.y, a.w, a.h);
        }
        // Не влезла (стена короткая, срезана) — зеркально на двух соседних стенах
        // у её конца.
        let s = sides[0];
        let (one, two) = ((s + 1) % 4, (s + 3) % 4);
        for t in [0.0, 0.15, 0.3] {
            let t = if s == 1 || s == 2 { 1.0 - t } else { t };
            let Some((_, a)) = wall_spot(m, sp, c, p, slot, one, t, &[]) else { continue };
            if wall_spot(m, sp, c, p, slot, two, t, &[]).is_some() {
                return 2;
            }
            m.props.pop();
            sp.unblock(a.x, a.y, a.w, a.h);
        }
        return 0;
    }
    let mut placed: Vec<[f32; 2]> = Vec::new();
    for t in [0.0, 1.0, 0.2, 0.8, 0.35, 0.65, 0.5] {
        for &side in sides {
            if placed.len() as i32 >= n {
                return n;
            }
            if let Some((at, _)) = wall_spot(m, sp, c, p, slot, side, t, &placed) {
                placed.push(at);
            }
        }
    }
    placed.len() as i32
}

/// Дверь подземелья в устье коридора: косяк FA в тон камня, если такой есть.
/// Стены коридора стоят на линиях клеток, и его просвет на толщину стены уже
/// двух клеток: дверь — ровно в просвет, стойки косяка ложатся на торцы стен.
fn door(m: &Map, a: [f32; 2], b: [f32; 2], pk: PortalKind, kit: &Kit) -> Portal {
    let len = ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()).round() as i32;
    let hw = m.wall_style.width / 2.0;
    // Не `signum`: у нуля он 1, и проём уходил наискось.
    let sign = |v: f32| if v.abs() < 1e-4 { 0.0 } else { v.signum() };
    let (ux, uy) = (sign(b[0] - a[0]), sign(b[1] - a[1]));
    let (a, b) = ([a[0] + ux * hw, a[1] + uy * hw], [b[0] - ux * hw, b[1] - uy * hw]);
    let mut d = portal(a, b, pk, false);
    let f = match len {
        1 => Some(&kit.frame1),
        2 => Some(&kit.frame2),
        _ => None,
    };
    if matches!(pk, PortalKind::Door | PortalKind::DoubleDoor) {
        d.asset = Some(kit.door.key.clone());
    }
    if let Some(f) = f.filter(|f| f.own && pk != PortalKind::Secret) {
        d.frame = Some(f.key.clone());
    }
    d
}

/// По обе стороны от занятого прямоугольника `at` вдоль стены напротив входа —
/// по предмету (свечи у алтаря, жаровни у статуи).
fn flank(m: &mut Map, sp: &mut Spots, c: &Chamber, at: Room, p: &Pick, slot: &str, scale: f32) {
    let far = (c.entry + 2) % 4;
    let spots = if far % 2 == 0 { [(at.x - 1, at.y + (at.h - 1) / 2), (at.x + at.w, at.y + (at.h - 1) / 2)] } else { [(at.x + (at.w - 1) / 2, at.y - 1), (at.x + (at.w - 1) / 2, at.y + at.h)] };
    for (x, y) in spots {
        if c.has(x, y) {
            put_pick(m, sp, p, slot, x, y, 0.0, scale);
        }
    }
}

/// Стол или конторка у стены напротив входа и стул перед ним, лицом к столу.
fn desk_and_chair(m: &mut Map, sp: &mut Spots, rng: &mut Rng, c: &Chamber, desk: &str) {
    let far = (c.entry + 2) % 4;
    let p = asset(desk);
    let placed = far_mid(m, sp, c, &p, desk, FURNITURE).is_some() || by_side_pick(m, sp, rng, c.r, &p, desk, FURNITURE, false, &[far]);
    if !placed {
        return;
    }
    let Some(d) = m.props.last().cloned() else { return };
    let chair = asset("chair");
    let [l, t, r, u] = bounds(&p, desk, d.rot, FURNITURE);
    let depth = if far % 2 == 0 { u - t } else { r - l };
    // От стены в комнату.
    let inward: [f32; 2] = [[0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [1.0, 0.0]][far as usize];
    let centre = [d.pos[0] + (l + r) / 2.0, d.pos[1] + (t + u) / 2.0];
    let at = [centre[0] + inward[0] * (depth / 2.0 + 0.45), centre[1] + inward[1] * (depth / 2.0 + 0.45)];
    // Сиденье спиной от стола: у северной стены стол — стул смотрит на север.
    let rot = (far as f32 * 90.0 + 180.0 + chair.turn) % 360.0;
    let [cl, ct, cr, cu] = bounds(&chair, "chair", rot, FURNITURE);
    drop_pick(m, &chair, [at[0] - (cl + cr) / 2.0, at[1] - (ct + cu) / 2.0], rot, FURNITURE);
    sp.block(at[0].floor() as i32, at[1].floor() as i32, 1, 1);
}

/// Ряды предметов поперёк комнаты, в шаге через клетку (саркофаги, скамьи), от
/// стены напротив входа к двери; посередине — проход. `gap` — клеток от дальней стены.
#[allow(clippy::too_many_arguments)]
fn rows(m: &mut Map, sp: &mut Spots, c: &Chamber, p: &Pick, slot: &str, scale: f32, gap: i32, face_far: bool) -> usize {
    let r = c.r;
    let far = (c.entry + 2) % 4;
    // Повёрнут к стене напротив входа (скамья — лицом к алтарю) или вдоль прохода.
    let rot = if face_far { (far as f32 * 90.0 + 180.0 + p.turn) % 360.0 } else { (far as f32 * 90.0 + 90.0 + p.turn) % 360.0 };
    let [l, t, rr, u] = bounds(p, slot, rot, scale);
    let (fw, fh) = (cells_of(rr - l), cells_of(u - t));
    // В рамке дальней стены: `a` вдоль неё, `b` от неё к двери.
    let (along, deep) = if far % 2 == 0 { (r.w, r.h) } else { (r.h, r.w) };
    let (pa, pb) = if far % 2 == 0 { (fw, fh) } else { (fh, fw) };
    let to_cell = |a: i32, b: i32| -> (i32, i32) {
        match far {
            0 => (r.x + a, r.y + b),
            2 => (r.x + a, r.y + r.h - b - fh),
            1 => (r.x + r.w - b - fw, r.y + a),
            _ => (r.x + b, r.y + a),
        }
    };
    // Проход посередине в две клетки; по бокам — столбцы предметов.
    let aisle = (along - 2) / 2;
    let left: Vec<i32> = (1..).map(|k| aisle - k * (pa + 1) + 1).take_while(|a| *a >= 1).collect();
    let right: Vec<i32> = (0..).map(|k| aisle + 2 + k * (pa + 1)).take_while(|a| a + pa < along).collect();
    let mut placed = 0;
    let mut b = gap;
    while b + pb <= deep - 3 {
        for &a in left.iter().chain(&right) {
            let (x, y) = to_cell(a, b);
            if (0..fw).all(|i| (0..fh).all(|j| c.has(x + i, y + j))) && put_pick(m, sp, p, slot, x, y, rot, scale) {
                placed += 1;
            }
        }
        b += pb + 1;
    }
    placed
}

/// Главный предмет комнаты: ставится раньше прохода между дверями. Где встал.
fn centrepiece(m: &mut Map, sp: &mut Spots, kit: &Kit, kind: DungeonKind, part: Part, c: &Chamber) -> Option<Room> {
    let far = (c.entry + 2) % 4;
    match (part, kind) {
        (Part::Climax, DungeonKind::Crypt) => centre(m, sp, c, &kit.sarcophagus, "sarcophagus", far, 1.0),
        (Part::Climax, DungeonKind::Prison) => centre(m, sp, c, &kit.cage, "cage", 0, 1.0),
        (Part::Climax, DungeonKind::Temple) | (Part::Chapel, _) => far_mid(m, sp, c, &kit.altar, "altar", 1.0),
        (Part::Climax, DungeonKind::Ruins) => far_mid(m, sp, c, &kit.throne, "throne", 1.0),
        (Part::Shrine | Part::Pool, _) => far_mid(m, sp, c, &kit.statue, "statue_figure", 1.0),
        (Part::Treasure, _) => far_mid(m, sp, c, &kit.chest, "chest", FURNITURE),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn dress(m: &mut Map, sp: &mut Spots, rng: &mut Rng, kit: &Kit, kind: DungeonKind, part: Part, c: &Chamber, at: Option<Room>) {
    let r = c.r;
    let area = r.w * r.h;
    let far = (c.entry + 2) % 4;
    let all = [0, 1, 2, 3];
    // Паутина — по углам склепа и руин.
    let webs = |m: &mut Map, _sp: &mut Spots, rng: &mut Rng, n: i32| {
        if matches!(kind, DungeonKind::Crypt | DungeonKind::Ruins) && !missing("cobweb") && c.shape == Shape::Rect {
            cobwebs(m, rng, c, n);
        }
    };
    // В углу, а где угла нет (срезан, занят) — у той же стены.
    let corners = |m: &mut Map, sp: &mut Spots, _rng: &mut Rng, p: &Pick, slot: &str, n: i32, sides: &[i32]| {
        spread(m, sp, c, p, slot, n, sides);
    };
    match part {
        Part::Entrance => {
            match kind {
                DungeonKind::Prison => {
                    // Пост стражи: стол со стулом, стойка у стены.
                    desk_and_chair(m, sp, rng, c, "table");
                    by_side_pick(m, sp, rng, r, &kit.rack, "weapon_rack", 1.0, false, &all);
                }
                DungeonKind::Ruins => {
                    for _ in 0..3 {
                        by_wall(m, sp, rng, r, "rubble", 1.0, false);
                    }
                }
                _ => {
                    // Две статуи-стража в углах напротив входа.
                    corners(m, sp, rng, &kit.statue, "statue_figure", 2, &[far]);
                }
            }
            // Жаровни по сторонам от входа.
            corners(m, sp, rng, &kit.brazier, "brazier", 2, &[c.entry]);
            webs(m, sp, rng, 1);
        }
        Part::Climax => {
            // Статуи — до колонн: иначе колонны занимают углы и пара не встаёт.
            if matches!(kind, DungeonKind::Crypt | DungeonKind::Temple) {
                corners(m, sp, rng, &kit.statue, "statue_figure", 2, &[far]);
            }
            colonnade(m, sp, c, kit, kind == DungeonKind::Ruins, rng);
            match kind {
                DungeonKind::Crypt => {
                    // Гробница: саркофаг посередине, свечи у его изголовья, статуи по углам.
                    if let Some(at) = at {
                        flank(m, sp, c, at, &kit.candles, "candles", 1.0);
                    }
                }
                DungeonKind::Prison => {
                    // Яма: клетка посередине, цепи по стенам.
                    for _ in 0..3 {
                        if !missing("chains") {
                            by_wall(m, sp, rng, r, "chains", 1.0, false);
                        }
                    }
                }
                DungeonKind::Temple => {
                    // Святилище: алтарь у стены напротив входа, свечи по бокам, статуи в углах.
                    if let Some(at) = at {
                        flank(m, sp, c, at, &kit.candles, "candles", 1.0);
                    }
                }
                DungeonKind::Ruins => {
                    // Логово: трон у дальней стены, кости вдоль стен, кострище посередине.
                    centre(m, sp, c, &asset("campfire"), "campfire", 0, 1.0);
                    for _ in 0..3 {
                        by_side_pick(m, sp, rng, r, &kit.bones, "bones", 0.9, false, &all);
                    }
                }
            }
            corners(m, sp, rng, &kit.brazier, "brazier", 2, &[c.entry]);
            webs(m, sp, rng, 2);
        }
        Part::Treasure => {
            // Сундук с сокровищем — главное в тайнике: не встал у дальней стены —
            // у любой.
            if at.is_none() {
                spread(m, sp, c, &kit.chest, "chest", 1, &[0, 1, 2, 3]);
            }
            by_wall(m, sp, rng, r, "chest_open", FURNITURE, true);
            if !missing("storage_pile") {
                by_wall(m, sp, rng, r, "storage_pile", 1.0, true);
            }
            webs(m, sp, rng, 2);
        }
        Part::Exit => {
            corners(m, sp, rng, &kit.brazier, "brazier", 1, &[c.entry]);
            webs(m, sp, rng, 1);
        }
        Part::Tomb => {
            // Усыпальница: саркофаги ровными рядами, проход к дальней стене посередине.
            if rows(m, sp, c, &kit.sarcophagus, "sarcophagus", 1.0, 1, false) == 0 {
                centre(m, sp, c, &kit.sarcophagus, "sarcophagus", far, 1.0);
            }
            corners(m, sp, rng, &kit.candles, "candles", 2, &[far]);
            webs(m, sp, rng, 2);
        }
        Part::Ossuary => {
            // Оссуарий: кости грудами вдоль стен.
            for _ in 0..((r.w + r.h) / 3).clamp(3, 8) {
                by_side_pick(m, sp, rng, r, &kit.bones, "bones", 0.9, false, &all);
            }
            webs(m, sp, rng, 3);
        }
        Part::Catacomb => {
            // Катакомба: гробы в нишах вдоль стен (loculi), проход посередине свободен.
            for _ in 0..((r.w + r.h) / 2).clamp(4, 14) {
                by_side_pick(m, sp, rng, r, &kit.coffin, "coffin", 1.0, false, &all);
            }
            webs(m, sp, rng, 2);
        }
        Part::Chapel => {
            // Часовня: алтарь, свечи по бокам, скамьи рядами лицом к алтарю.
            if let Some(at) = at {
                flank(m, sp, c, at, &kit.candles, "candles", 1.0);
            }
            if c.shape == Shape::Rect {
                rows(m, sp, c, &kit.bench, "bench", FURNITURE, 3, true);
            }
        }
        Part::Torture | Part::Cells => {
            // Стол — у стены, жаровня в углу, клетка, цепи, стойка.
            by_side_pick(m, sp, rng, r, &asset("table"), "table", FURNITURE, false, &all);
            corners(m, sp, rng, &kit.brazier, "brazier", 1, &all);
            by_side_pick(m, sp, rng, r, &kit.cage, "cage", 1.0, true, &all);
            for _ in 0..2 {
                if !missing("chains") {
                    by_wall(m, sp, rng, r, "chains", 1.0, false);
                }
            }
            by_side_pick(m, sp, rng, r, &kit.rack, "weapon_rack", 1.0, false, &all);
        }
        Part::Guard => {
            // Караульная: стол со стулом, стойки, бочка воды, нары.
            desk_and_chair(m, sp, rng, c, "table");
            for _ in 0..2 {
                by_side_pick(m, sp, rng, r, &kit.rack, "weapon_rack", 1.0, false, &all);
            }
            by_wall(m, sp, rng, r, "barrel", FURNITURE, true);
            by_wall(m, sp, rng, r, "bed", FURNITURE, true);
            corners(m, sp, rng, &kit.brazier, "brazier", 1, &all);
        }
        Part::Warden => {
            desk_and_chair(m, sp, rng, c, "desk");
            by_wall(m, sp, rng, r, "chest", FURNITURE, true);
            by_wall(m, sp, rng, r, "bookshelf", 1.0, false);
            by_wall(m, sp, rng, r, "bed", FURNITURE, true);
        }
        Part::Shrine => {
            colonnade(m, sp, c, kit, false, rng);
            if let Some(at) = at {
                flank(m, sp, c, at, &kit.brazier, "brazier", 1.0);
            }
        }
        Part::Pool => {
            corners(m, sp, rng, &kit.brazier, "brazier", 4, &all);
        }
        Part::Library => {
            // Книгохранилище: шкафы по стенам, конторка у дальней стены.
            desk_and_chair(m, sp, rng, c, "desk");
            for _ in 0..(r.w + r.h) / 3 {
                by_wall(m, sp, rng, r, "bookshelf", 1.0, false);
            }
        }
        Part::Collapsed => {
            for _ in 0..(area / 8).clamp(3, 8) {
                if rng.chance(0.3) {
                    by_side_pick(m, sp, rng, r, &kit.broken, "pillar_broken", 1.0, false, &all);
                } else {
                    let n = pick(rng, &["rubble", "rubble", "boulder"]);
                    by_wall(m, sp, rng, r, n, 1.0, false);
                }
            }
            webs(m, sp, rng, 2);
        }
        Part::Lair => {
            centre(m, sp, c, &asset("campfire"), "campfire", 0, 1.0);
            for _ in 0..2 {
                by_side_pick(m, sp, rng, r, &kit.bones, "bones", 0.9, false, &all);
            }
            if !missing("hay") {
                by_wall(m, sp, rng, r, "hay", 1.0, true);
            }
            by_wall(m, sp, rng, r, "barrel", FURNITURE, true);
            webs(m, sp, rng, 2);
        }
        Part::Flooded => {
            for _ in 0..3 {
                by_wall(m, sp, rng, r, "rubble", 1.0, false);
            }
            webs(m, sp, rng, 2);
        }
        Part::Store => furnish(m, sp, rng, r, Role::Storage),
        Part::Plan(role) => furnish(m, sp, rng, r, role),
    }
}

/// Спуск на нижний ярус — встроенная «Лестница вниз», у стены напротив входа: лестницы
/// FA деревянные и ведут наверх, в подземелье они читаются как чужие.
fn stairs_down(m: &mut Map, sp: &mut Spots, rng: &mut Rng, kit: &Kit, c: &Chamber, far_wall: bool) {
    let p = kit.stairs.clone();
    let slot = if p.own { "stairs_down" } else { "stairs" };
    // Ступени FA — полоса в клетку; пролёт из двух полос рядом, в две клетки
    // шириной: место под него размечается по удвоенной ширине.
    let mut place = p.clone();
    if let Some(v) = place.visible.as_mut().filter(|_| p.own) {
        v[1] = -1.0;
        v[3] = 1.0;
    }
    let far = (c.entry + 2) % 4;
    let sides = if far_wall { [far, (far + 1) % 4, (far + 3) % 4] } else { [(far + 1) % 4, (far + 3) % 4, far] };
    let n0 = m.props.len();
    let placed = (far_wall && far_mid(m, sp, c, &place, slot, 1.0).is_some()) || sides.iter().any(|&s| spread(m, sp, c, &place, slot, 1, &[s]) == 1);
    if !placed && !by_side_pick(m, sp, rng, c.r, &p, slot, 0.7, false, &[0, 1, 2, 3]) {
        drop_pick(m, &p, [c.r.x as f32 + c.r.w as f32 / 2.0, c.r.y as f32 + c.r.h as f32 / 2.0], 0.0, 0.6);
    }
    if !p.own || m.props.len() != n0 + 1 {
        return;
    }
    let mut st = m.props.pop().unwrap_or_else(|| unreachable!());
    // Пролёт уходит под стену: нижняя ступень у её середины, а не в зазоре.
    let a = st.rot.to_radians();
    let (up, across) = ([a.cos(), a.sin()], [-a.sin(), a.cos()]);
    st.pos = [st.pos[0] - up[0] * WALL_GAP, st.pos[1] - up[1] * WALL_GAP];
    let wide = place.visible.is_some();
    let lanes: &[f32] = if wide { &[-0.5, 0.5] } else { &[0.0] };
    let at = |k: f32| [st.pos[0] + across[0] * k * st.scale, st.pos[1] + across[1] * k * st.scale];
    for &k in lanes {
        m.props.push(crate::map::Prop { pos: at(k), ..st.clone() });
    }
    // Низ пролёта (к стене) темнеет — видно, что лестница ведёт вниз. Слой — как у
    // ступеней, иначе «тень» по имени ложится под них.
    for &k in lanes {
        drop_pick(m, &Palette::builtin_pick("stairs_shade"), at(k), st.rot, st.scale);
        if let Some(shade) = m.props.last_mut() {
            shade.layer = Some(crate::map::LAYER_DEFAULT);
        }
    }
    // Перила по обеим внешним сторонам.
    if let Some(rail) = kit.rail.as_ref() {
        let edge = lanes.iter().cloned().fold(0.0f32, f32::max) + 0.5;
        for k in [-edge, edge] {
            drop_pick(m, rail, at(k), st.rot, st.scale);
        }
    }
    // Перед пролётом две клетки свободны: костёр и мебель не встают на сход.
    let half_len = 1.5 * st.scale;
    for d in [0.5, 1.5] {
        for &k in lanes {
            for o in [-0.25, 0.25] {
                let q = at(k + o);
                let q = [q[0] + up[0] * (half_len + d), q[1] + up[1] * (half_len + d)];
                sp.block(q[0].floor() as i32, q[1].floor() as i32, 1, 1);
            }
        }
    }
}

/// Паутина натянута через угол: концы на двух стенах, под 45°. Берётся длинная
/// паутина FA (1×2, 1×3) — нить между двумя концами; углы разные. В Foundry её
/// концы стоят, а середина чуть колышется.
fn cobwebs(m: &mut Map, rng: &mut Rng, c: &Chamber, n: i32) {
    let r = c.r;
    let mut free = vec![0, 1, 2, 3];
    let mut left = n;
    while left > 0 && !free.is_empty() {
        let k = free.remove(rng.int(0, free.len() as i32 - 1) as usize);
        // Белая: чёрная на тёмном камне не видна. Нить в картинках FA бывает
        // наискось — концы берутся по самому рисунку.
        let p = asset_toned("cobweb", "white");
        let Some([x0, y0, x1, y1]) = strand_ends(&p) else { return };
        let lv = (x1 - x0).hypot(y1 - y0);
        if lv < 0.3 {
            continue;
        }
        // Длина нити — от 1,5 до 3 клеток; концы на линиях стен, под их полосой.
        let d = lv.clamp(1.5, 3.0f32.min(r.w.min(r.h) as f32 - 1.0).max(1.5));
        let s = d / lv;
        let e = d * std::f32::consts::FRAC_1_SQRT_2;
        let (corner, sign) = match k {
            0 => ([r.x as f32, r.y as f32], [1.0, 1.0]),
            1 => ([(r.x + r.w) as f32, r.y as f32], [-1.0, 1.0]),
            2 => ([(r.x + r.w) as f32, (r.y + r.h) as f32], [-1.0, -1.0]),
            _ => ([r.x as f32, (r.y + r.h) as f32], [1.0, -1.0]),
        };
        let (p0, p1) = ([corner[0] + sign[0] * e, corner[1]], [corner[0], corner[1] + sign[1] * e]);
        // За обоими концами — стена на всём пролёте от угла, а не устье коридора.
        let rock = |q: [f32; 2]| m.rooms.get((q[0] * mask::RES as f32) as isize, (q[1] * mask::RES as f32) as isize) == 0;
        let walled = (0..=6).all(|i| {
            let t = i as f32 / 6.0 * (e + 0.2);
            rock([corner[0] + sign[0] * t, corner[1] - sign[1] * 0.3]) && rock([corner[0] - sign[0] * 0.3, corner[1] + sign[1] * t])
        });
        if !walled {
            continue;
        }
        let th = (p1[1] - p0[1]).atan2(p1[0] - p0[0]) - (y1 - y0).atan2(x1 - x0);
        let (sn, cs) = th.sin_cos();
        let mid = [(x0 + x1) / 2.0 * s, (y0 + y1) / 2.0 * s];
        let pm = [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0];
        let centre = [pm[0] - (mid[0] * cs - mid[1] * sn), pm[1] - (mid[0] * sn + mid[1] * cs)];
        drop_pick(m, &p, centre, th.to_degrees().rem_euclid(360.0), s);
        left -= 1;
    }
}

/// Камеры темницы: проход от двери комнаты к дальней стене, по обе его стороны
/// камеры за решёткой, у каждой дверь в проход. Только тупиковая прямоугольная
/// комната с одной дверью `door`; `false` — не вышло, тогда комната — пыточная.
fn jail_cells(m: &mut Map, sp: &mut Spots, rng: &mut Rng, kit: &Kit, c: &Chamber, door: Segment) -> bool {
    let r = c.r;
    if c.shape != Shape::Rect {
        return false;
    }
    // `u` — от стены входа вглубь, `v` — поперёк; проход — две клетки двери.
    let along_x = c.entry % 2 == 1;
    let (len, across) = if along_x { (r.w, r.h) } else { (r.h, r.w) };
    let aisle = if along_x { door.0[1] as i32 - r.y } else { door.0[0] as i32 - r.x };
    let (near, far) = (aisle, across - aisle - 2);
    if len < 5 || near.max(far) < 2 {
        return false;
    }
    let flip = !matches!(c.entry, 0 | 3);
    let at = |u: f32, v: f32| -> [f32; 2] {
        let u = if flip { len as f32 - u } else { u };
        if along_x { [r.x as f32 + u, r.y as f32 + v] } else { [r.x as f32 + v, r.y as f32 + u] }
    };
    let cell = |u: i32, v: i32| -> (i32, i32) {
        let u = if flip { len - 1 - u } else { u };
        if along_x { (r.x + u, r.y + v) } else { (r.x + v, r.y + u) }
    };
    let bars = WallStyle { width: 0.16, ..wall("wall_metal") };
    let line = |m: &mut Map, a: [f32; 2], b: [f32; 2]| m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(bars.clone()) });
    let cw = if len >= 9 { 3 } else { 2 };
    // У входа две клетки свободны — дверь не упирается в решётку.
    let start = 2;
    // Стороны прохода: (от, до) поперёк, линия решётки, ряд у дальней стены камеры.
    let sides: Vec<(i32, i32, i32, i32)> = [(0, near, near, 0), (aisle + 2, across, aisle + 2, across - 1)].into_iter().filter(|s| s.1 - s.0 >= 2).collect();
    for &(v0, v1, bar, _) in &sides {
        line(m, at(start as f32, bar as f32), at(len as f32, bar as f32));
        line(m, at(start as f32, v0 as f32), at(start as f32, v1 as f32));
    }
    let mut u = start;
    while u + cw <= len {
        for &(v0, v1, bar, back) in &sides {
            if u + cw < len {
                line(m, at((u + cw) as f32, v0 as f32), at((u + cw) as f32, v1 as f32));
            }
            let d = u + cw / 2;
            let (a, b) = (at(d as f32, bar as f32), at(d as f32 + 1.0, bar as f32));
            // Дверь камеры — решётчатая, без косяка: решётка тоньше стены.
            let mut gate = portal(a, b, PortalKind::Door, false);
            gate.frame = None;
            if kit.bars.own {
                gate.asset = Some(kit.bars.key.clone());
            }
            m.portals.push(gate);
            // В камере у дальней стены — подстилка, кости или цепи; крупное
            // сдвигается внутрь комнаты, а не за стену.
            let (x, y) = cell(u + rng.int(0, cw - 1), back);
            let what = pick(rng, &[or("bones", "sacks"), "sacks", or("hay", "sacks"), or("chains", "barrel")]);
            let p = asset(what);
            let [l, t, rr, b] = bounds(&p, what, 0.0, 0.9);
            let (x, y) = (x.min(r.x + r.w - cells_of(rr - l)), y.min(r.y + r.h - cells_of(b - t)));
            put_pick(m, sp, &p, what, x, y, 0.0, 0.9);
        }
        u += cw;
    }
    // Короткий хвост у дальней стены — глухой, за решёткой.
    for &(v0, v1, _, _) in &sides {
        if u < len {
            line(m, at(u as f32, v0 as f32), at(u as f32, v1 as f32));
        }
    }
    // У входа — табурет стражника.
    let (x, y) = cell(0, if near >= 2 { 0 } else { across - 1 });
    put(m, sp, "stool", x, y, 0.0, FURNITURE);
    sp.block(r.x, r.y, r.w, r.h);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Options;

    fn make(kind: DungeonKind, seed: &str, size: [u32; 2], rooms: u32) -> (Map, Report) {
        let o = Options { kind: Kind::Dungeon, seed: seed.into(), rooms, dungeon: kind, ..Options::default() };
        let plan = Plan::random(&o, size);
        let mut m = Map::new(size[0], size[1]);
        let r = build(&mut m, &plan, 0, Palette::builtin()).unwrap();
        (m, r)
    }

    #[test]
    fn every_kind_builds_rooms_loops_and_a_way_down() {
        for kind in DungeonKind::ALL {
            for seed in ["а", "б", "в"] {
                let (m, r) = make(kind, seed, [50, 40], 12);
                assert!(r.count >= 8, "{kind:?} {seed}: комнат {}", r.count);
                assert!(m.portals.iter().any(|p| p.kind == PortalKind::DoubleDoor || p.kind == PortalKind::Opening), "{kind:?}: нет ворот");
                assert!(m.props.iter().any(|p| p.asset.ends_with("stairs")), "{kind:?} {seed}: нет спуска");
                assert!(m.labels.iter().any(|l| l.text.starts_with("1. Вход")), "{kind:?}: нет подписи входа");
            }
        }
    }

    #[test]
    fn rooms_never_overlap_and_loops_appear() {
        let mut loops = 0;
        for seed in ["кольцо", "петля", "круг", "обход"] {
            let lay = grow(50, 40, 12, 5, 10, 0.5, &mut Rng::new(seed));
            for (i, a) in lay.rooms.iter().enumerate() {
                for b in &lay.rooms[i + 1..] {
                    let (ra, rb) = (a.r, b.r);
                    assert!(ra.x + ra.w < rb.x || rb.x + rb.w < ra.x || ra.y + ra.h < rb.y || rb.y + rb.h < ra.y, "комнаты слиплись");
                }
            }
            loops += lay.halls.len() + 1 - lay.rooms.len();
        }
        assert!(loops >= 4, "колец {loops}");
    }

    #[test]
    fn floors_are_stone_for_every_kind() {
        for kind in DungeonKind::ALL {
            let (m, _) = make(kind, "пол", [40, 30], 8);
            let names: Vec<String> = m.room_floor.iter().map(|c| match &m.materials[c.material as usize].paint {
                Paint::Texture(k) => k.clone(),
                _ => String::new(),
            }).collect();
            assert!(names.iter().all(|k| !k.contains("wood") && !k.contains("grass")), "{kind:?}: {names:?}");
        }
    }
}

#[cfg(test)]
mod fa_props {
    use super::*;
    use crate::generate::Options;

    /// Что ставится из паков FA в комнаты каждого вида. Только на машине с паками.
    #[test]
    #[ignore]
    fn print_dungeon_props() {
        let dir = std::path::Path::new("E:/D&D/DD ASSET NEW/DungeonDraft Assets/V3.5");
        let mut assets = Vec::new();
        for f in ["FA_Objects_A_v3.52.dungeondraft_pack", "FA_Objects_B_v3.82.dungeondraft_pack", "FA_Textures_A_v3.59.dungeondraft_pack", "FA_Textures_B_v3.52.dungeondraft_pack"] {
            assets.extend(library::pack_assets(&dir.join(f)).unwrap());
        }
        for kind in DungeonKind::ALL {
            let o = Options { kind: Kind::Dungeon, seed: format!("{kind:?}1").to_lowercase(), rooms: 12, dungeon: kind, ..Options::default() };
            let mut plan = Plan::random(&o, [50, 40]);
            plan.assets = Assets::Library;
            plan.options.room_size = [5, 9];
            plan.options.loops = 0.4;
            let mut m = Map::new(50, 40);
            build(&mut m, &plan, 0, Palette::from_library(&assets, Box::new(|_| Some([256, 256])))).unwrap();
            println!("==== {kind:?}");
            for l in &m.labels {
                println!("  [{}] at {:?}", l.text, l.pos);
            }
            for p in &m.props {
                let name = p.asset.rsplit('/').next().unwrap_or(&p.asset);
                println!("    {name} pos {:.1},{:.1} size {:?} scale {:.2} rot {}", p.pos[0], p.pos[1], p.size, p.scale, p.rot);
            }
        }
    }
}

