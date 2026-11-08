//! Городской квартал. Устроен, как старый европейский город и как улицы у
//! картографов D&D (Forgotten Adventures, 2-Minute Tabletop, Czepeku):
//!
//! - Проезды по старшинству. Главная улица по длинной стороне карты — с
//!   тротуарами и бордюром, 4–6 клеток. Переулки поперёк неё, 2–3 клетки; по
//!   разные стороны главной они сходятся не всегда, бывают и Т-перекрёстки.
//!   У глубоких кварталов сзади проезд вдоль дворов, 2 клетки, без мостовой.
//! - Земля вдоль улицы нарезана на участки, как бургажные наделы: узкие и
//!   глубокие, фасадом на улицу. Дом во всю ширину участка, соседи — стена к
//!   стене, за домом двор с забором. Изредка между участками щель-проход во дворы.
//! - Что за дом, решает улица. На главной — лавки, пекарня, кузня, на углу —
//!   таверна; в переулках больше жилья; у заднего проезда — склады. Узкий дом
//!   стоит к улице торцом, конёк поперёк неё, широкий — вдоль.
//! - Внутри — как у настоящих: спереди лавка, мастерская или зал с очагом, за
//!   ними кухня, спальня, кладовая. Жильё «над лавкой» здесь внизу: второго этажа
//!   на карте нет. Комнаты крупнее настоящих, под фигурки, как и мебель (`FURNITURE`).
//! - Двор: нужник в дальнем углу, поленница у задней стены, бельё, колодец на
//!   квартал, кусты; у кузни телега и уголь, у таверны сено и поилка.
//! - Улица живёт у фасадов: бочки, ящики, цветы у стен, фонари по тротуару.
//!   Середина проезжей части свободна, у бордюра бывает телега.
//! - Вид квартала ([`District`]). «С таверной» — постоялый двор, как английский
//!   coaching inn: самый большой дом, лицом на площадь или главную улицу, за ним
//!   двор с конюшней. «Торговый» — по улицам почти одни лавки (кто чем торгует —
//!   [`Goods`]), у заднего проезда склады купцов, площадь рыночная, прилавков много.
//! - Таверна внутри — как у старых трактиров и у картографов: общий зал со стойкой
//!   у задней стены, за стойкой дверь кухни, рядом погреб с бочками; у широкой —
//!   отдельная зала (parlour) с камином и комнаты постояльцев. См. [`tavern_hall`].

use super::*;
use crate::plan::District;

/// Ранг проезда: главная улица, переулок, задний проезд.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Rank {
    Main,
    Lane,
    Back,
}

#[derive(Clone, Copy, Debug)]
struct Street {
    r: Room,
    rank: Rank,
    /// Площадь: мощёная целиком, дома смотрят на неё, как на главную улицу.
    square: bool,
}

/// Назначение участка.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Trade {
    Home,
    Shop,
    Bakery,
    Smith,
    Tavern,
    Store,
}

#[derive(Clone, Copy, Debug)]
struct Plot {
    house: Room,
    yard: Option<Room>,
    /// Сторона дома, что смотрит на улицу (0 — север, 1 — восток, 2 — юг, 3 — запад).
    face: i32,
    trade: Trade,
    rank: Rank,
    /// Чем торгует лавка; у прочих — `General`, не важно.
    goods: Goods,
}

/// Товар лавки: от него полки и что на них. Встроенных картинок мало, своё из
/// паков (полки с товаром, стойки оружия) берётся, если есть.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Goods {
    /// Лавка всего понемногу: ящики, бочки, мешки.
    General,
    /// Оружейник: стойки с мечами, топорами, копьями.
    Arms,
    /// Книги и свитки: шкафы, конторка.
    Books,
    /// Травник, аптекарь: полки со склянками, котёл, стол с утварью.
    Herbs,
}

fn goods(rng: &mut Rng) -> Goods {
    match rng.int(0, 5) {
        0..=2 => Goods::General,
        3 => Goods::Arms,
        4 => Goods::Books,
        _ => Goods::Herbs,
    }
}

/// Комната дома по назначению.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Use {
    /// Зал с очагом и столом; `true` — в нём же и спят (дом в одну комнату).
    Living(bool),
    Kitchen,
    Bedroom,
    Storage,
    Shop,
    Bakehouse,
    Forge,
    Hall,
    Office,
    /// Отдельная зала таверны — для чистой публики и для ужина: камин, один стол.
    Parlour,
    /// Погреб таверны: бочки, ящики; дверь — в кухню.
    Cellar,
    /// Комната постояльца: кровать, сундук.
    Guest,
}

/// Ширина вдоль улицы и глубина дома по назначению, в клетках: от и до.
fn dims(t: Trade) -> (i32, i32, i32, i32) {
    match t {
        Trade::Home => (5, 7, 6, 9),
        Trade::Shop => (5, 7, 7, 10),
        Trade::Bakery => (6, 8, 7, 10),
        Trade::Smith => (6, 8, 7, 9),
        Trade::Tavern => (10, 13, 9, 12),
        Trade::Store => (6, 9, 7, 10),
    }
}

/// Сколько чего на карту: таверна одна, кузен и пекарен немного.
#[derive(Default)]
struct Tally {
    tavern: bool,
    smiths: i32,
    bakeries: i32,
}

/// Назначение участка. `corner_tavern` — можно ли таверну на углу: у квартала «с
/// таверной» её место выбрано заранее ([`inn_site`]).
fn choose(rank: Rank, corner: bool, depth: i32, t: &mut Tally, rng: &mut Rng, district: District, corner_tavern: bool) -> Trade {
    if corner_tavern && corner && !t.tavern && depth >= 9 && rank != Rank::Back && (rank == Rank::Main || rng.chance(0.5)) {
        t.tavern = true;
        return Trade::Tavern;
    }
    let roll = rng.random();
    let trade = if district == District::Market {
        // Торговый: на улицах лавки, где-то пекарня и кузня; у заднего проезда —
        // склады купцов. Жильё — в глубине переулков.
        match rank {
            Rank::Main if roll < 0.70 => Trade::Shop,
            Rank::Main if roll < 0.82 => Trade::Bakery,
            Rank::Main if roll < 0.90 => Trade::Smith,
            Rank::Main => Trade::Store,
            Rank::Lane if roll < 0.60 => Trade::Shop,
            Rank::Lane if roll < 0.70 => Trade::Smith,
            Rank::Lane if roll < 0.82 => Trade::Store,
            Rank::Back if roll < 0.65 => Trade::Store,
            _ => Trade::Home,
        }
    } else {
        match rank {
            Rank::Main if roll < 0.36 => Trade::Shop,
            Rank::Main if roll < 0.48 => Trade::Bakery,
            Rank::Main if roll < 0.60 => Trade::Smith,
            Rank::Lane if roll < 0.15 => Trade::Shop,
            Rank::Lane if roll < 0.25 => Trade::Smith,
            Rank::Back if roll < 0.35 => Trade::Store,
            _ => Trade::Home,
        }
    };
    let bakeries = if district == District::Market { 2 } else { 1 };
    match trade {
        Trade::Smith if t.smiths >= 2 => {
            if district == District::Market {
                Trade::Shop
            } else {
                Trade::Home
            }
        }
        Trade::Bakery if t.bakeries >= bakeries => Trade::Shop,
        Trade::Smith => {
            t.smiths += 1;
            trade
        }
        Trade::Bakery => {
            t.bakeries += 1;
            trade
        }
        _ => trade,
    }
}

// ----- геометрия участка: вдоль улицы `u`, вглубь от фасада `v` -----

/// Прямоугольник в рамке участка: `u0` вдоль улицы, `v0` от стороны `face` вглубь.
fn sub(r: Room, face: i32, u0: i32, v0: i32, du: i32, dv: i32) -> Room {
    match face {
        0 => Room { x: r.x + u0, y: r.y + v0, w: du, h: dv },
        2 => Room { x: r.x + u0, y: r.y + r.h - v0 - dv, w: du, h: dv },
        3 => Room { x: r.x + v0, y: r.y + u0, w: dv, h: du },
        _ => Room { x: r.x + r.w - v0 - dv, y: r.y + u0, w: dv, h: du },
    }
}

fn along(r: Room, face: i32) -> i32 {
    if face % 2 == 0 { r.w } else { r.h }
}

fn deep(r: Room, face: i32) -> i32 {
    if face % 2 == 0 { r.h } else { r.w }
}

/// Сторона прямоугольника отрезком по узлам сетки.
fn side_seg(r: Room, side: i32) -> Segment {
    let (ax, ay, bx, by) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
    match side {
        0 => ([ax, ay], [bx, ay]),
        1 => ([bx, ay], [bx, by]),
        2 => ([ax, by], [bx, by]),
        _ => ([ax, ay], [ax, by]),
    }
}

/// Клетка внутри прямоугольника.
fn contains(r: Room, x: i32, y: i32) -> bool {
    x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h
}

/// Прямоугольник в рамке длинной оси карты: `u` вдоль главной улицы, `v` поперёк.
fn world(horiz: bool, u: i32, v: i32, du: i32, dv: i32) -> Room {
    if horiz { Room { x: u, y: v, w: du, h: dv } } else { Room { x: v, y: u, w: dv, h: du } }
}

// ----- улицы и кварталы -----

struct Layout {
    streets: Vec<Street>,
    blocks: Vec<Room>,
    /// Главная улица целиком (с тротуарами) и её проезжая часть.
    main: Room,
    road: Room,
    horiz: bool,
}

fn layout(w: i32, h: i32, rng: &mut Rng, density: f32, market: bool) -> Layout {
    let horiz = w >= h;
    let (long, short) = if horiz { (w, h) } else { (h, w) };
    let main_w = if short >= 24 { 6 } else if short >= 14 { 5 } else { 4 };
    let walk = i32::from(main_w >= 5);
    let wobble = short / 10;
    let off = ((short - main_w) / 2 + rng.int(-wobble, wobble)).clamp(3.min(short / 3), (short - main_w - 3).max(3.min(short / 3)));
    let main = world(horiz, 0, off, long, main_w);
    let road = world(horiz, 0, off + walk, long, main_w - 2 * walk);
    let mut streets = vec![Street { r: main, rank: Rank::Main, square: false }];

    // Переулки: по одной раскладке на обе стороны, каждая берёт свой с вероятностью.
    let gap = (19.0 - density * 6.0) as i32;
    let mut lanes: Vec<(i32, i32)> = Vec::new();
    let mut u = rng.int(gap / 2 + 2, gap);
    while u + 3 + 7 <= long {
        lanes.push((u, rng.int(2, 3)));
        u += rng.int(gap - 3, gap + 3);
    }
    let halves = [(0, off), (off + main_w, short - off - main_w)];
    let mut blocks = Vec::new();
    // Задний проезд — на одном расстоянии от главной по всей стороне.
    for (k, &(v0, dv)) in halves.iter().enumerate() {
        if dv < 4 {
            continue;
        }
        let used: Vec<(i32, i32)> = lanes.iter().copied().filter(|_| rng.chance(0.72)).collect();
        let mut cuts: Vec<(i32, i32)> = Vec::new();
        for &(lu, lw) in &used {
            streets.push(Street { r: world(horiz, lu, v0, lw, dv), rank: Rank::Lane, square: false });
            cuts.push((lu, lw));
        }
        let back = if dv >= 20 { Some(rng.int(9, 11).min(dv - 9)) } else { None };
        let mut start = 0;
        for &(lu, lw) in cuts.iter().chain(std::iter::once(&(long, 0))) {
            if lu - start >= 4 {
                match back {
                    // Ближе к главной — свой квартал, за проездом — другой.
                    Some(bd) => {
                        let (near, far) = if k == 0 { ((v0 + dv - bd, bd), (v0, dv - bd - 2)) } else { ((v0, bd), (v0 + bd + 2, dv - bd - 2)) };
                        let lane_v = if k == 0 { v0 + dv - bd - 2 } else { v0 + bd };
                        blocks.push(world(horiz, start, near.0, lu - start, near.1));
                        blocks.push(world(horiz, start, far.0, lu - start, far.1));
                        streets.push(Street { r: world(horiz, start, lane_v, lu - start, 2), rank: Rank::Back, square: false });
                    }
                    None => blocks.push(world(horiz, start, v0, lu - start, dv)),
                }
            }
            start = lu + lw;
        }
    }

    // Площадь у главной улицы, ближе к середине: на большой карте; у торгового
    // квартала — и на карте поменьше, и шире: это рыночная площадь.
    if (long >= 34 && short >= 18) || (market && long >= 26 && short >= 16) {
        let touches_main = |b: &Room| if horiz { b.y + b.h == main.y || b.y == main.y + main.h } else { b.x + b.w == main.x || b.x == main.x + main.w };
        let centre = long / 2;
        let mid = |b: &Room| if horiz { b.x + b.w / 2 } else { b.y + b.h / 2 };
        let len = |b: &Room| if horiz { b.w } else { b.h };
        let depth = |b: &Room| if horiz { b.h } else { b.w };
        if let Some(i) = (0..blocks.len()).filter(|&i| touches_main(&blocks[i]) && len(&blocks[i]) >= 9 && depth(&blocks[i]) >= 6).min_by_key(|&i| (mid(&blocks[i]) - centre).abs()) {
            let b = blocks.remove(i);
            let (bu, bv, bl, bd) = if horiz { (b.x, b.y, b.w, b.h) } else { (b.y, b.x, b.h, b.w) };
            let near_main = if horiz { b.y + b.h == main.y } else { b.x + b.w == main.x };
            let mut sl = bl.min(if market { rng.int(13, 16) } else { rng.int(10, 13) });
            // Узкий остаток по бокам — голая полоса: пусть уж площадь шире.
            if (bl - sl) / 2 < 4 {
                sl = bl;
            }
            let su = bu + (bl - sl) / 2;
            let mut sd = rng.int(6, 8).min(bd);
            if bd - sd < 6 {
                sd = bd;
            }
            let sv = if near_main { bv + bd - sd } else { bv };
            streets.push(Street { r: world(horiz, su, sv, sl, sd), rank: Rank::Main, square: true });
            // Что осталось от квартала: слева, справа и за площадью — прямоугольниками.
            for (u0, du, v0, dv) in [(bu, su - bu, bv, bd), (su + sl, bu + bl - su - sl, bv, bd), (su, sl, if near_main { bv } else { bv + sd }, bd - sd)] {
                if du >= 4 && dv >= 4 {
                    blocks.push(world(horiz, u0, v0, du, dv));
                }
            }
        }
    }
    Layout { streets, blocks, main, road, horiz }
}

/// Клетка за серединой стороны `side` квартала.
fn outside(b: Room, side: i32) -> (i32, i32) {
    match side {
        0 => (b.x + b.w / 2, b.y - 1),
        1 => (b.x + b.w, b.y + b.h / 2),
        2 => (b.x + b.w / 2, b.y + b.h),
        _ => (b.x - 1, b.y + b.h / 2),
    }
}

/// Какой проезд за стороной `side` квартала: по клетке посередине стороны.
fn facing(b: Room, side: i32, streets: &[Street]) -> Option<Rank> {
    let (x, y) = outside(b, side);
    streets.iter().filter(|s| contains(s.r, x, y)).map(|s| s.rank).min()
}

/// Где встанет постоялый двор: квартал и его сторона. Лучше всего — лицом на
/// площадь, иначе на главную улицу ближе к середине карты. Вглубь лучше 12 клеток
/// и больше: зал в 8 клеток (стойка с табуретами берёт четыре, остальное — столы)
/// и кухня; на маленькой карте — сколько есть, от 9.
fn inn_site(blocks: &[Room], streets: &[Street], w: i32, h: i32) -> Option<(usize, i32)> {
    for (main_only, dmin) in [(true, 12), (false, 12), (true, 10), (false, 9)] {
        let mut best: Option<((i32, i32, i32), usize, i32)> = None;
        for (i, &b) in blocks.iter().enumerate() {
            for side in 0..4 {
                let Some(rank) = facing(b, side, streets) else { continue };
                if rank == Rank::Back || (main_only && rank != Rank::Main) || along(b, side) < 12 {
                    continue;
                }
                // Напротив тоже улица — участок только до середины квартала.
                let d = if facing(b, (side + 2) % 4, streets).is_some() { deep(b, side) / 2 } else { deep(b, side) };
                if d < dmin {
                    continue;
                }
                let (x, y) = outside(b, side);
                let square = streets.iter().any(|s| s.square && contains(s.r, x, y));
                let (cx, cy) = b.center();
                let key = (i32::from(square), i32::from(d >= 13), -((cx - w / 2).abs() + (cy - h / 2).abs()));
                if best.as_ref().is_none_or(|(k, _, _)| key > *k) {
                    best = Some((key, i, side));
                }
            }
        }
        if let Some((_, i, side)) = best {
            return Some((i, side));
        }
    }
    None
}

/// Участки квартала: ряды вдоль каждой стороны, что выходит на проезд, — от
/// старшей улицы к младшей; угол достаётся старшей. `inn` — сторона, где встанет
/// постоялый двор: он размечается первым, посередине стороны.
fn plots(b: Room, streets: &[Street], rng: &mut Rng, tally: &mut Tally, district: District, corner_tavern: bool, inn: Option<i32>) -> Vec<Plot> {
    let mut faces: Vec<(i32, Rank)> = (0..4).filter_map(|s| facing(b, s, streets).map(|r| (s, r))).collect();
    faces.sort_by_key(|&(s, r)| (r, -along(b, s)));
    let mut owned = vec![false; (b.w * b.h) as usize];
    let own = |o: &Vec<bool>, x: i32, y: i32| o[((y - b.y) * b.w + (x - b.x)) as usize];
    let mut out = Vec::new();
    for (k, &(side, rank)) in faces.iter().enumerate() {
        let (len, d) = (along(b, side), deep(b, side));
        let opposite = faces.iter().position(|&(s, _)| s == (side + 2) % 4);
        // Напротив тоже улица — двор до середины квартала.
        let pd = match opposite {
            Some(j) if j > k => (d + 1) / 2,
            Some(_) => d / 2,
            None => d,
        };
        // Сколько свободно вглубь у столбца `u` (от фасада).
        let free_depth = |o: &Vec<bool>, u: i32| -> i32 {
            (0..pd).take_while(|&v| {
                let c = sub(b, side, u, v, 1, 1);
                !own(o, c.x, c.y)
            })
            .count() as i32
        };
        // Постоялый двор — до прочих участков стороны: ближе к середине, где вглубь
        // хватает места. Узкий остаток с краю не оставляется: двор тянется до угла.
        if inn == Some(side) && !tally.tavern && len >= 11 {
            let want = rng.int(13, 17).min(len);
            let mid = (len - want) / 2;
            let mut starts: Vec<i32> = (0..=len - want).collect();
            starts.sort_by_key(|u| (u - mid).abs());
            let site = starts.into_iter().find_map(|u0| {
                let (mut u0, mut f) = (u0, want);
                if (1..5).contains(&u0) {
                    f += u0;
                    u0 = 0;
                }
                let right = len - u0 - f;
                if (1..5).contains(&right) {
                    f += right;
                }
                let col = (u0..u0 + f).map(|q| free_depth(&owned, q)).min().unwrap_or(0);
                (col >= 9).then_some((u0, f, col))
            });
            if let Some((u0, f, col)) = site {
                let mut hd = if col >= 17 { rng.int(12, 14).min(col - 5) } else { rng.int(12, 14).min(col) };
                if col > hd && col - hd < 3 {
                    hd = col;
                }
                let house = sub(b, side, u0, 0, f, hd);
                let yard = (col - hd >= 2).then(|| sub(b, side, u0, hd, f, col - hd));
                for r in std::iter::once(house).chain(yard) {
                    for y in r.y..r.y + r.h {
                        for x in r.x..r.x + r.w {
                            owned[((y - b.y) * b.w + (x - b.x)) as usize] = true;
                        }
                    }
                }
                tally.tavern = true;
                out.push(Plot { house, yard, face: side, trade: Trade::Tavern, rank, goods: Goods::General });
            }
        }
        // С какого конца угол: у угла таверна или лавка побольше.
        let before = if side % 2 == 0 { 3 } else { 0 };
        let corner_lo = faces.iter().any(|&(s, _)| s == before);
        let mut u = 0;
        while u < len {
            if free_depth(&owned, u) < 5 {
                u += 1;
                continue;
            }
            let run = (u..len).take_while(|&q| free_depth(&owned, q) >= 5).count() as i32;
            if run < 4 {
                u += run.max(1);
                continue;
            }
            let corner = u == 0 && corner_lo;
            let trade = choose(rank, corner, pd, tally, rng, district, corner_tavern);
            let (fmin, fmax, dmin, dmax) = dims(trade);
            let mut f = rng.int(fmin, fmax).min(run);
            // Остаток меньше дома — дом шире.
            if run - f > 0 && run - f < 4 {
                f = run;
            }
            let col = (u..u + f).map(|q| free_depth(&owned, q)).min().unwrap_or(0);
            let mut hd = rng.int(dmin, dmax).min(col);
            if col > hd && col - hd < 3 && col <= dmax + 2 {
                hd = col;
            }
            if hd < 5 || f < 4 {
                u += 1;
                continue;
            }
            let house = sub(b, side, u, 0, f, hd);
            let yd = col - hd;
            let yard = (yd >= 2).then(|| sub(b, side, u, hd, f, yd));
            for r in std::iter::once(house).chain(yard) {
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        owned[((y - b.y) * b.w + (x - b.x)) as usize] = true;
                    }
                }
            }
            let goods = if trade == Trade::Shop { goods(rng) } else { Goods::General };
            out.push(Plot { house, yard, face: side, trade, rank, goods });
            u += f;
            // Щель между домами — проход во дворы.
            if yard.is_some() && u + 5 < len && rng.chance(0.12) {
                u += 1;
            }
        }
    }
    out
}

// ----- дом -----

/// Стены и крыша дома: по улице и назначению, если план не сказал своё.
fn look(rng: &mut Rng, plot: &Plot, plan: Option<(&'static str, &'static str)>, street_roofs: &[&str]) -> (&'static str, String) {
    if let Some((st, roof)) = plan {
        return (st, roof.to_owned());
    }
    let st = match plot.trade {
        Trade::Smith => pick(rng, &["wall_fieldstone", "wall_stone_blocks", "wall_brick"]),
        Trade::Tavern => pick(rng, &["wall_timber", "wall_timber", "wall_brick"]),
        Trade::Store => pick(rng, &["wall_wood", "wall_brick", "wall_fieldstone"]),
        _ if plot.rank == Rank::Main => pick(rng, &["wall_timber", "wall_timber", "wall_timber", "wall_brick", "wall_stone_blocks"]),
        _ => pick(rng, &["wall_timber", "wall_wood", "wall_fieldstone", "wall_timber"]),
    };
    // Крыши улицы — из двух-трёх покрытий, изредка чужое; у камня — сланец или дранка.
    let roof = if rng.chance(0.2) {
        match st {
            "wall_fieldstone" | "wall_stone_blocks" => pick(rng, &["roof_slate_gray", "roof_slate_blue", "roof_shingle"]),
            _ => pick(rng, &["roof_clay_red", "roof_clay_brown", "roof_shingle", "roof_thatch"]),
        }
        .to_owned()
    } else {
        street_roofs[rng.int(0, street_roofs.len() as i32 - 1) as usize].to_owned()
    };
    (st, roof)
}

/// Комнаты дома по назначению: ряды от фасада вглубь, в ряду — слева направо.
fn rooms(plot: &Plot, rng: &mut Rng) -> Vec<(Room, Use)> {
    let (r, face) = (plot.house, plot.face);
    let (f, d) = (along(r, face), deep(r, face));
    let mut rows: Vec<(i32, Vec<(i32, Use)>)> = Vec::new();
    // Задний ряд: одна комната во всю ширину или две, если обе не уже четырёх.
    let back = |pair: (Use, Use), single: Use, rng: &mut Rng| -> Vec<(i32, Use)> {
        if f >= 8 {
            let a = rng.int(4, f - 4);
            vec![(a, pair.0), (f - a, pair.1)]
        } else {
            vec![(f, single)]
        }
    };
    match plot.trade {
        Trade::Home if d < 7 || f < 5 => rows.push((d, vec![(f, Use::Living(true))])),
        Trade::Home if d >= 11 => {
            let front = (d * 2 / 5).max(4);
            let mid = (d - front) / 2;
            rows.push((front, vec![(f, Use::Living(false))]));
            rows.push((mid, vec![(f, Use::Kitchen)]));
            rows.push((d - front - mid, vec![(f, Use::Bedroom)]));
        }
        Trade::Home => {
            let front = (d - 3).min(d * 3 / 5).max(4);
            rows.push((front, vec![(f, Use::Living(false))]));
            rows.push((d - front, back((Use::Kitchen, Use::Bedroom), Use::Bedroom, rng)));
        }
        Trade::Shop | Trade::Bakery => {
            let front = if d >= 10 { 5 } else { 4 };
            rows.push((front, vec![(f, Use::Shop)]));
            if d - front >= 3 {
                let rear = if plot.trade == Trade::Bakery { back((Use::Bakehouse, Use::Living(true)), Use::Bakehouse, rng) } else { back((Use::Storage, Use::Living(true)), Use::Living(true), rng) };
                rows.push((d - front, rear));
            } else {
                rows[0].0 = d;
            }
        }
        Trade::Smith => {
            if d >= 8 {
                rows.push((d - 3, vec![(f, Use::Forge)]));
                rows.push((3, vec![(f, Use::Living(true))]));
            } else {
                rows.push((d, vec![(f, Use::Forge)]));
            }
        }
        Trade::Tavern => {
            // Как у старых постоялых дворов: спереди общий зал; у широкой сбоку от
            // него отдельная зала (parlour). Сзади кухня — под залом, прямо за
            // стойкой; рядом погреб с бочками, дверь в него из кухни; под залой —
            // комнаты постояльцев.
            let rear = if d >= 10 { 4 } else { 4.min(d - 5) };
            let wide = f >= 13 && rear >= 4;
            let left = rng.chance(0.5);
            let par = if wide { rng.int(5, 6) } else { 0 };
            let front = if !wide {
                vec![(f, Use::Hall)]
            } else if left {
                vec![(par, Use::Parlour), (f - par, Use::Hall)]
            } else {
                vec![(f - par, Use::Hall), (par, Use::Parlour)]
            };
            rows.push((d - rear, front));
            let mut row = if wide {
                let k = if f >= 16 { 6 } else { 5 };
                let rest = f - k - 4;
                let mut v = vec![(4, Use::Cellar), (k, Use::Kitchen)];
                if rest >= 8 {
                    v.insert(0, (rest - rest / 2, Use::Guest));
                    v.insert(0, (rest / 2, Use::Guest));
                } else if rest >= 3 {
                    v.insert(0, (rest, Use::Guest));
                } else {
                    v[1].0 += rest;
                }
                // Кухня — у дальнего от залы края, под общим залом.
                if !left {
                    v.reverse();
                }
                v
            } else {
                let k = (f * 3 / 5).max(4);
                let mut v = vec![(k, Use::Kitchen), (f - k, Use::Cellar)];
                if rng.chance(0.5) {
                    v.reverse();
                }
                v
            };
            row.retain(|(w, _)| *w > 0);
            rows.push((rear, row));
        }
        Trade::Store => {
            if d >= 9 && f >= 7 {
                rows.push((d - 3, vec![(f, Use::Storage)]));
                rows.push((3, vec![(4, Use::Office), (f - 4, Use::Storage)]));
            } else {
                rows.push((d, vec![(f, Use::Storage)]));
            }
        }
    }
    let mut out = Vec::new();
    let mut v = 0;
    for (dv, row) in rows {
        let mut u = 0;
        for (du, what) in row {
            out.push((sub(r, face, u, v, du, dv), what));
            u += du;
        }
        v += dv;
    }
    out
}

/// Пол комнаты по назначению.
fn floor_of(u: Use) -> &'static str {
    match u {
        Use::Forge => "flagstones",
        Use::Bakehouse => "brick_floor",
        Use::Kitchen => "stone_tiles",
        Use::Cellar => "flagstones",
        _ => "wood_planks",
    }
}

/// Занятые куски линии (вертикальная ли, координата) — двери и окна не ложатся друг на друга.
type Taken = BTreeMap<(bool, i32), Vec<(i32, i32)>>;

/// Свободное место шириной `width` на стороне `seg`, не ближе `margin` к концам;
/// вокруг занятого — клетка. Из годных — случайное.
fn spot(seg: Segment, width: i32, margin: i32, taken: &Taken, rng: &mut Rng) -> Option<(i32, i32)> {
    let vertical = seg.0[0] == seg.1[0];
    let line = (vertical, if vertical { seg.0[0] } else { seg.0[1] } as i32);
    let (lo, hi) = if vertical { (seg.0[1].min(seg.1[1]) as i32, seg.0[1].max(seg.1[1]) as i32) } else { (seg.0[0].min(seg.1[0]) as i32, seg.0[0].max(seg.1[0]) as i32) };
    let busy = taken.get(&line).cloned().unwrap_or_default();
    let ok: Vec<i32> = (lo + margin..=hi - margin - width).filter(|&s| busy.iter().all(|&(a, b)| s + width < a || s > b)).collect();
    if ok.is_empty() {
        return None;
    }
    let s = ok[rng.int(0, ok.len() as i32 - 1) as usize];
    Some((s, s + width))
}

/// Поставить проём на стороне `seg` в `s..e` и пометить место.
fn open(m: &mut Map, sp: &mut Spots, taken: &mut Taken, seg: Segment, (s, e): (i32, i32), kind: PortalKind, wood: bool) {
    let vertical = seg.0[0] == seg.1[0];
    let c = if vertical { seg.0[0] } else { seg.0[1] };
    taken.entry((vertical, c as i32)).or_default().push((s, e));
    let (a, b) = if vertical { ([c, s as f32], [c, e as f32]) } else { ([s as f32, c], [e as f32, c]) };
    m.portals.push(portal(a, b, kind, wood));
    // Перед дверью и окном с обеих сторон свободно: к окну мебель не ставят.
    let ci = c as i32;
    if vertical {
        sp.block(ci - 1, s, 2, e - s);
    } else {
        sp.block(s, ci - 1, e - s, 2);
    }
}

/// Дом на участке: пол, перегородки, двери, окна, мебель, крыша. Возвращает
/// стиль наружных стен — его кладут на контур после всех домов.
fn build_house(m: &mut Map, sp: &mut Spots, rng: &mut Rng, plot: &Plot, st: &str, roof: &str, taken: &mut Taken) -> WallStyle {
    let outer = wall(st);
    let wood = !["stone", "brick"].iter().any(|s| st.contains(s));
    let inner = WallStyle { width: outer.width * 0.8, ..outer.clone() };
    let face = plot.face;
    let back_side = (face + 2) % 4;
    let parts = rooms(plot, rng);
    for &(r, u) in &parts {
        let floor = mat(m, floor_of(u));
        m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(floor));
    }
    // Перегородки и двери между рядами: каждая комната следующего ряда — в ту,
    // с кем общая стена длиннее.
    for (i, &(a, _)) in parts.iter().enumerate() {
        for &(b, _) in &parts[i + 1..] {
            if let Some(seg) = shared(a, b) {
                m.walls.push(Wall { points: vec![seg.0, seg.1], closed: false, bends: Vec::new(), style: Some(inner.clone()) });
            }
        }
    }
    let depth_of = |r: Room| -> i32 {
        let h = plot.house;
        match face {
            0 => r.y - h.y,
            2 => h.y + h.h - r.y - r.h,
            3 => r.x - h.x,
            _ => h.x + h.w - r.x - r.w,
        }
    };
    // Дверь кухни таверны: перед ней в зале встанет стойка.
    let mut kitchen_door: Option<[f32; 2]> = None;
    for (i, &(b, ub)) in parts.iter().enumerate() {
        if depth_of(b) == 0 {
            continue;
        }
        // Погреб открывается в кухню: бочки носят оттуда, а не через зал.
        let via = (ub == Use::Cellar).then(|| parts.iter().find(|(_, u)| *u == Use::Kitchen).and_then(|(a, _)| shared(*a, b))).flatten();
        let best = via.or_else(|| parts.iter().enumerate().filter(|(j, (a, _))| *j != i && depth_of(*a) < depth_of(b)).filter_map(|(_, (a, _))| shared(*a, b)).max_by_key(seg_len));
        if let Some(seg) = best
            && let Some(at) = spot(seg, 1, 1, taken, rng).or_else(|| spot(seg, 1, 0, taken, rng))
        {
            open(m, sp, taken, seg, at, PortalKind::Door, true);
            if ub == Use::Kitchen {
                kitchen_door = Some(door_mid(seg, at));
            }
        }
    }
    // Вход с улицы — в переднюю комнату (у таверны — в общий зал); у лавок и
    // складов дверь шире, у кузни ворота нараспашку.
    let fronts: Vec<(Room, Use)> = parts.iter().copied().filter(|(r, _)| depth_of(*r) == 0).collect();
    let (entry, entry_use) = fronts.iter().copied().find(|(_, u)| *u == Use::Hall).unwrap_or(fronts[fronts.len() / 2]);
    let fseg = side_seg(entry, face);
    let f = seg_len(&fseg);
    let (width, kind) = match plot.trade {
        Trade::Tavern | Trade::Store if f >= 6 => (2, PortalKind::DoubleDoor),
        Trade::Smith if f >= 6 => (2, PortalKind::Opening),
        Trade::Shop | Trade::Bakery if f >= 7 => (2, PortalKind::DoubleDoor),
        _ => (1, PortalKind::Door),
    };
    let mut entry_door: Option<[f32; 2]> = None;
    if let Some(at) = spot(fseg, width, 1, taken, rng) {
        open(m, sp, taken, fseg, at, kind, wood);
        entry_door = Some(door_mid(fseg, at));
    }
    // Окна на улицу: у каждой передней комнаты, сколько встанет через клетку; у лавки
    // и зала — по две клетки.
    for &(r, u) in &fronts {
        let seg = side_seg(r, face);
        let ww = if matches!(u, Use::Shop | Use::Hall) && seg_len(&seg) >= 6 { 2 } else { 1 };
        for _ in 0..(seg_len(&seg) / 3).max(1) {
            if let Some(at) = spot(seg, ww, 1, taken, rng) {
                open(m, sp, taken, seg, at, PortalKind::Window, wood);
            }
        }
    }
    // Задняя дверь — во двор, из кухни, пекарни или жилой; задние комнаты с окном во двор.
    let rear: Vec<(Room, Use)> = parts.iter().copied().filter(|(r, _)| side_seg(*r, back_side) == side_seg(plot.house, back_side) || on_back(*r, plot.house, back_side)).collect();
    if plot.yard.is_some() {
        let door_room = rear.iter().find(|(_, u)| matches!(u, Use::Kitchen | Use::Bakehouse)).or_else(|| rear.iter().find(|(_, u)| matches!(u, Use::Living(_) | Use::Forge | Use::Storage))).or(rear.first()).copied();
        if let Some((r, _)) = door_room {
            let seg = side_seg(r, back_side);
            let w = if plot.trade == Trade::Store && seg_len(&seg) >= 5 { 2 } else { 1 };
            if let Some(at) = spot(seg, w, 1, taken, rng) {
                open(m, sp, taken, seg, at, if w == 2 { PortalKind::DoubleDoor } else { PortalKind::Door }, wood);
            }
        }
        for &(r, _) in &rear {
            let seg = side_seg(r, back_side);
            if let Some(at) = spot(seg, 1, 1, taken, rng) {
                open(m, sp, taken, seg, at, PortalKind::Window, wood);
            }
        }
    }
    for &(r, u) in &parts {
        let door = (entry_use == u && r == entry).then_some(entry_door).flatten();
        dress(m, sp, rng, r, u, plot, door, kitchen_door);
    }
    let h = plot.house;
    let squareish = h.w.min(h.h) * 5 >= h.w.max(h.h) * 4 && h.w.min(h.h) >= 9;
    let shape = if squareish { RoofShape::Hip } else { RoofShape::Gable };
    m.roofs.push(Roof { a: [h.x as f32, h.y as f32], b: [(h.x + h.w) as f32, (h.y + h.h) as f32], shape, paint: key(roof), turn: u8::from(h.h > h.w), overhang: 0.3 });
    outer
}

/// Середина проёма `s..e` на стороне `seg`.
fn door_mid(seg: Segment, (s, e): (i32, i32)) -> [f32; 2] {
    let mid = (s + e) as f32 / 2.0;
    if seg.0[0] == seg.1[0] { [seg.0[0], mid] } else { [mid, seg.0[1]] }
}

/// Комната касается задней стены дома.
fn on_back(r: Room, h: Room, back: i32) -> bool {
    match back {
        0 => r.y == h.y,
        1 => r.x + r.w == h.x + h.w,
        2 => r.y + r.h == h.y + h.h,
        _ => r.x == h.x,
    }
}

/// Мебель по назначению. Фасад дома (`plot.face`) — окна и вход, очаг туда не
/// ставят. `door` — вход с улицы, если он в этой комнате; `kitchen` — дверь кухни.
#[allow(clippy::too_many_arguments)]
fn dress(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, u: Use, plot: &Plot, door: Option<[f32; 2]>, kitchen: Option<[f32; 2]>) {
    let face = plot.face;
    let f = FURNITURE;
    let back = (face + 2) % 4;
    let not_face: Vec<i32> = (0..4).filter(|s| *s != face).collect();
    let sides: Vec<i32> = (0..4).filter(|s| s % 2 != face % 2).collect();
    let shelf = if missing("shelf_full") { "bookshelf" } else { "shelf_full" };
    match u {
        Use::Living(sleep) => {
            if !hearth(m, sp, rng, r, &not_face) {
                by_side(m, sp, rng, r, "fireplace", 1.0, false, &not_face);
            }
            if sleep {
                let bed = if r.w * r.h >= 30 { "bed_double" } else { "bed" };
                by_side(m, sp, rng, r, bed, f, true, &not_face);
            }
            let (cw, ch) = dining_cell();
            let placed = (0..8).any(|_| {
                if r.w < cw + 2 || r.h < ch + 2 {
                    return false;
                }
                let (x, y) = (rng.int(r.x + 1, r.x + r.w - cw - 1), rng.int(r.y + 1, r.y + r.h - ch - 1));
                dining(m, sp, rng, x, y)
            });
            if !placed {
                inside(m, sp, rng, r, "table", f);
            }
            by_side(m, sp, rng, r, shelf, 1.0, false, &not_face);
            by_side(m, sp, rng, r, "chest", f, true, &not_face);
            if rng.chance(0.6) {
                let n = pick(rng, &["plant_pot", "barrel", "wardrobe"]);
                by_wall(m, sp, rng, r, n, f, true);
            }
        }
        Use::Kitchen => furnish(m, sp, rng, r, Role::Kitchen),
        Use::Bedroom => furnish(m, sp, rng, r, Role::Bedroom),
        Use::Storage => furnish(m, sp, rng, r, Role::Storage),
        Use::Office => {
            if !inside(m, sp, rng, r, "desk", f) {
                by_wall(m, sp, rng, r, "desk", f, false);
            }
            inside(m, sp, rng, r, "chair", f);
            by_wall(m, sp, rng, r, "chest", f, true);
            by_wall(m, sp, rng, r, shelf, 1.0, false);
        }
        Use::Shop => {
            // Прилавок поперёк, у дальней стены или сбоку; за ним полки и касса,
            // у входа место покупателю.
            let before = m.props.len();
            let counter = if missing("shop_counter") { "bar_counter" } else { "shop_counter" };
            let tries = [(1.0, &[back][..]), (1.0, &sides[..]), (0.75, &[back][..]), (0.75, &sides[..])];
            tries.iter().any(|(s, at)| by_side(m, sp, rng, r, counter, *s, false, at));
            if counter == "shop_counter" {
                by_side(m, sp, rng, r, counter, 1.0, false, &sides);
            }
            match plot.goods {
                Goods::General => {
                    for _ in 0..(r.w + r.h) / 4 {
                        by_side(m, sp, rng, r, shelf, 1.0, false, &sides);
                    }
                    by_side(m, sp, rng, r, "chest", f, true, &[back]);
                    for _ in 0..3 {
                        let n = pick(rng, &["crate", "barrel", "sacks"]);
                        by_side(m, sp, rng, r, n, f, true, &not_face);
                    }
                    if !missing("table_clutter") {
                        dining_free_table(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2);
                    }
                }
                Goods::Arms => {
                    // Стойки вдоль боковых стен, товар напоказ; у задней — сундук.
                    for _ in 0..(r.w + r.h) / 4 {
                        let n = pick(rng, &["weapon_rack", "weapon_rack_axes", "weapon_rack_spears", "weapon_rack_maces", "weapon_rack_bows", "weapon_rack_mixed"]);
                        if !by_side(m, sp, rng, r, n, 1.0, false, &sides) {
                            by_side(m, sp, rng, r, n, 1.0, false, &not_face);
                        }
                    }
                    by_side(m, sp, rng, r, "chest_open", f, true, &[back]);
                    by_side(m, sp, rng, r, "crate", f, true, &not_face);
                    by_side(m, sp, rng, r, "barrel", f, true, &not_face);
                }
                Goods::Books => {
                    // Шкафы по стенам, посередине конторка писца.
                    for _ in 0..(r.w + r.h) / 3 {
                        by_side(m, sp, rng, r, "bookshelf", 1.0, false, &not_face);
                    }
                    let (x, y) = (r.x + r.w / 2 - 1, r.y + r.h / 2);
                    if put(m, sp, "desk", x, y, 0.0, f) {
                        put(m, sp, "chair", x + 1, y - 1, 0.0, f);
                    }
                    by_side(m, sp, rng, r, "chest", f, true, &[back]);
                    by_wall(m, sp, rng, r, "plant_pot", f, true);
                    by_wall(m, sp, rng, r, "crate", f, true);
                }
                Goods::Herbs => {
                    // Полки со склянками, котёл для отваров, стол со ступками; мешки
                    // трав и горшки с зеленью.
                    for _ in 0..(r.w + r.h) / 4 {
                        by_side(m, sp, rng, r, shelf, 1.0, false, &sides);
                    }
                    by_side(m, sp, rng, r, "cauldron", f, false, &not_face);
                    if !dining_free_table(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2) {
                        inside(m, sp, rng, r, "table", f);
                    }
                    for n in ["sacks", "plant_pot", "plant_pot"] {
                        by_side(m, sp, rng, r, n, f, true, &not_face);
                    }
                }
            }
            // В тесной лавке шкафы и прилавок не встают — тогда товар в ящиках и бочках.
            for _ in 0..6 {
                if m.props.len() - before >= 4 {
                    break;
                }
                let n = pick(rng, &["crate", "barrel", "sacks", "crate"]);
                by_wall(m, sp, rng, r, n, f, true);
            }
        }
        Use::Bakehouse => {
            let oven = if missing("oven") { "fireplace" } else { "oven" };
            by_side(m, sp, rng, r, oven, 1.0, false, &[back]);
            if r.w.max(r.h) >= 7 {
                by_side(m, sp, rng, r, oven, 1.0, false, &sides);
            }
            if !dining_free_table(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2) {
                inside(m, sp, rng, r, "table", f);
            }
            for n in ["sacks", "sacks", "sacks", "barrel"] {
                by_wall(m, sp, rng, r, n, f, true);
            }
            if !missing("firewood_pile") {
                by_wall(m, sp, rng, r, "firewood_pile", 1.0, true);
            }
        }
        Use::Forge => {
            // Горн у дальней стены, наковальня перед ним, у боковых — стойки с готовым.
            let forge = if missing("forge") { "brazier" } else { "forge" };
            let fs = if missing("forge") { 1.6 } else { 1.0 };
            if !by_side(m, sp, rng, r, forge, fs, false, &[back]) {
                by_side(m, sp, rng, r, forge, fs, false, &sides);
            }
            let near = sub(r, back, 1, 2, (along(r, back) - 2).max(1), 2.min(deep(r, back)));
            if !inside(m, sp, rng, near, "anvil", f) {
                inside(m, sp, rng, r, "anvil", f);
            }
            for _ in 0..2 {
                let n = pick(rng, &["weapon_rack", "weapon_rack_axes", "weapon_rack_spears", "weapon_rack_mixed"]);
                by_side(m, sp, rng, r, n, 1.0, false, &sides);
            }
            for n in ["barrel", "crate", "sacks", "sacks", "chest_open"] {
                by_side(m, sp, rng, r, n, f, true, &not_face);
            }
            if !missing("table_clutter") {
                dining_free_table(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2);
            } else {
                by_side(m, sp, rng, r, "table", f, false, &sides);
            }
            // Наковальня одна; в большой кузне — ещё стойка и бочка с водой для закалки.
            if r.w * r.h >= 36 {
                let n = pick(rng, &["weapon_rack_maces", "weapon_rack_bows", "weapon_rack"]);
                by_side(m, sp, rng, r, n, 1.0, false, &[back]);
                let water = if missing("water_barrel") { "barrel" } else { "water_barrel" };
                by_side(m, sp, rng, r, water, if water == "barrel" { f } else { 1.0 }, true, &not_face);
            }
        }
        Use::Hall => tavern_hall(m, sp, rng, r, face, door, kitchen),
        Use::Parlour => {
            // Отдельная зала: свой камин с креслами, обеденный стол посередине,
            // полка, цветы. Сюда подают ужин, сюда же ведут гостей почище.
            if !hearth(m, sp, rng, r, &not_face) {
                by_side(m, sp, rng, r, "fireplace", 1.0, false, &not_face);
            }
            let (cw, ch) = dining_cell();
            let (cx, cy) = (r.x + (r.w - cw) / 2, r.y + (r.h - ch) / 2);
            let mut spots: Vec<(i32, i32)> = (r.y..=r.y + r.h - ch).flat_map(|y| (r.x..=r.x + r.w - cw).map(move |x| (x, y))).collect();
            spots.sort_by_key(|&(x, y)| (x - cx).abs() + (y - cy).abs());
            if !spots.into_iter().any(|(x, y)| dining(m, sp, rng, x, y)) {
                inside(m, sp, rng, r, "table_round", 1.0);
            }
            by_side(m, sp, rng, r, shelf, 1.0, false, &not_face);
            by_wall(m, sp, rng, r, "plant_pot", f, true);
            if rng.chance(0.5) {
                by_side(m, sp, rng, r, "sofa", f, false, &not_face);
            }
        }
        Use::Cellar => {
            // Бочки рядами вдоль стен, между ними ящики и мешки.
            let piles = !missing("storage_pile");
            // Груда FA — одна: их картинки шире своих клеток и ложатся друг на друга.
            let mut pile = piles;
            for _ in 0..(r.w * r.h / 3).clamp(4, 14) {
                let n = if pile { "storage_pile" } else { pick(rng, &["barrel", "barrel", "barrel", "barrel", "crate", "sacks"]) };
                pile = false;
                by_wall(m, sp, rng, r, n, if n == "storage_pile" { 1.0 } else { f }, false);
            }
        }
        Use::Guest => {
            // Комната внаём: кровать в углу, сундук, табурет; побольше — и шкаф.
            by_wall(m, sp, rng, r, "bed", f, true);
            by_wall(m, sp, rng, r, "chest", f, true);
            by_wall(m, sp, rng, r, "stool", f, false);
            if r.w * r.h >= 24 && rng.chance(0.6) {
                by_wall(m, sp, rng, r, "wardrobe", f, true);
            }
        }
    }
}

/// Зал таверны — как у старых трактиров и у картографов (FA, 2-Minute Tabletop):
///
/// - стойка вдоль задней стены, и дверь кухни — за её краем: хозяин выходит из
///   кухни прямо к прилавку; между стойкой и стеной бочки и полки, перед ней табуреты;
/// - очаг с креслами у боковой стены;
/// - от входа к стойке свободный проход; столы по бокам — длинные со скамьями,
///   как козлы-столы старых трактиров, и малые со стульями;
/// - в большом зале — лестница наверх, к комнатам постояльцев, и помост для
///   музыканта в переднем углу;
/// - вдоль стен скамьи, бочки, полки.
fn tavern_hall(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, face: i32, door: Option<[f32; 2]>, kitchen: Option<[f32; 2]>) {
    let back = (face + 2) % 4;
    let (len, depth) = (along(r, back), deep(r, back));
    // Рамка задней стены: `u` вдоль неё, `v` от неё в зал.
    let to_world = |u: f32, v: f32| -> [f32; 2] {
        match back {
            0 => [r.x as f32 + u, r.y as f32 + v],
            1 => [(r.x + r.w) as f32 - v, r.y as f32 + u],
            2 => [r.x as f32 + u, (r.y + r.h) as f32 - v],
            _ => [r.x as f32 + v, r.y as f32 + u],
        }
    };
    let cells = |u0: i32, v0: i32, du: i32, dv: i32| -> Room {
        match back {
            0 => Room { x: r.x + u0, y: r.y + v0, w: du, h: dv },
            1 => Room { x: r.x + r.w - v0 - dv, y: r.y + u0, w: dv, h: du },
            2 => Room { x: r.x + u0, y: r.y + r.h - v0 - dv, w: du, h: dv },
            _ => Room { x: r.x + v0, y: r.y + u0, w: dv, h: du },
        }
    };
    let u_of = |p: [f32; 2]| if back % 2 == 0 { p[0] - r.x as f32 } else { p[1] - r.y as f32 };
    // Предмет спиной к задней стене, серединой в точку рамки.
    let at_wall = |m: &mut Map, p: &Pick, slot: &str, u: f32, v: f32, scale: f32| {
        let rot = (back as f32 * 90.0 + p.turn) % 360.0;
        let [l, t, rr, uu] = bounds(p, slot, rot, scale);
        let c = to_world(u, v);
        drop_pick(m, p, [c[0] - (l + rr) / 2.0, c[1] - (t + uu) / 2.0], rot, scale);
    };
    let extent = |b: [f32; 4]| if back % 2 == 0 { (b[2] - b[0], b[3] - b[1]) } else { (b[3] - b[1], b[2] - b[0]) };

    // Стойка — не длиннее половины стены.
    let bar = asset("bar_counter");
    let rot = (back as f32 * 90.0 + bar.turn) % 360.0;
    let (len1, _) = extent(bounds(&bar, "bar_counter", rot, 1.0));
    let scale = (len as f32 * 0.55 / len1).clamp(0.5, 1.0);
    let (blen, bdep) = extent(bounds(&bar, "bar_counter", rot, scale));
    let ku = kitchen.map(u_of).filter(|k| *k > 0.0 && *k < len as f32);
    let hi = (len as f32 - blen - 1.0).max(1.0);
    let ustart = match ku {
        Some(k) => (k - 1.0).clamp(1.0, hi),
        None if rng.chance(0.5) => 1.0,
        None => hi,
    };
    let v0 = if depth >= 9 { 1.8 } else { 1.5 };
    at_wall(m, &bar, "bar_counter", ustart + blen / 2.0, v0 + bdep / 2.0, scale);
    let zone = cells(ustart.floor() as i32 - 1, 0, blen.ceil() as i32 + 2, (v0 + bdep + 1.2).ceil() as i32);
    sp.block(zone.x, zone.y, zone.w, zone.h);
    // За стойкой: бочки и полки вдоль стены, кроме места у двери кухни.
    for k in 0..(blen / 1.6) as i32 {
        let pu = ustart + 0.8 + k as f32 * 1.6;
        if ku.is_some_and(|d| (pu - d).abs() < 1.4) {
            continue;
        }
        let n = if !missing("shelf_full") && k % 2 == 1 { "shelf_full" } else { pick(rng, &["barrel", "barrel", "crate"]) };
        let p = asset(n);
        let s = if n == "shelf_full" { 1.0 } else { FURNITURE * 0.8 };
        let (_, d) = extent(bounds(&p, n, (back as f32 * 90.0 + p.turn) % 360.0, s));
        at_wall(m, &p, n, pu, WALL_GAP + d.min(v0 - 0.3) / 2.0, s);
    }
    // Табуреты перед стойкой.
    let stool = asset("stool");
    for k in 0..(blen as i32 + 1) / 2 {
        at_wall(m, &stool, "stool", ustart + 0.9 + k as f32 * 2.0, v0 + bdep + 0.55, FURNITURE * 0.85);
    }
    // Проход от входа к стойке.
    if let Some(d) = door {
        let inward: [f32; 2] = [[0.0, 1.0], [-1.0, 0.0], [0.0, -1.0], [1.0, 0.0]][face as usize];
        let gate = [d[0] + inward[0] * 0.5, d[1] + inward[1] * 0.5];
        walkway(sp, gate, to_world(ustart + blen / 2.0, v0 + bdep + 1.5), r);
    }
    // Очаг — у боковой стены.
    let lateral: Vec<i32> = (0..4).filter(|s| s % 2 != face % 2).collect();
    if !hearth(m, sp, rng, r, &lateral) {
        by_side(m, sp, rng, r, "fireplace", 1.0, false, &lateral);
    }
    let big = len * depth >= 90;
    // Лестница наверх — к комнатам постояльцев, в углу у боковой стены.
    if big && !by_side(m, sp, rng, r, "stairs", 1.0, true, &lateral) {
        by_side(m, sp, rng, r, "stairs", 0.7, true, &lateral);
    }
    // Помост для музыканта: ковёр и табурет в переднем углу, подальше от двери.
    if big && rng.chance(0.75) {
        let fl = along(r, face);
        let corners = [sub(r, face, 1, 1, 3, 3), sub(r, face, fl - 4, 1, 3, 3)];
        let dist = |c: &Room| door.map_or(0.0, |d| (c.x as f32 + 1.5 - d[0]).abs() + (c.y as f32 + 1.5 - d[1]).abs());
        let stage = if dist(&corners[0]) >= dist(&corners[1]) { corners[0] } else { corners[1] };
        if sp.free(stage.x, stage.y, stage.w, stage.h) {
            sp.block(stage.x, stage.y, stage.w, stage.h);
            let c = [stage.x as f32 + 1.5, stage.y as f32 + 1.5];
            let rug = asset("rug_round");
            let [l, t, rr, uu] = bounds(&rug, "rug_round", 0.0, 1.0);
            let s = (2.8 / (rr - l).max(uu - t)).min(1.5);
            drop_pick(m, &rug, c, 0.0, s);
            drop(m, "stool", c, (back as f32 * 90.0) % 360.0, FURNITURE * 0.85);
        }
    }
    // Столы: по рядам, клетка прохода между соседями.
    let (cw, ch) = dining_cell();
    let horiz = rng.chance(0.5);
    let (tw, th) = if horiz { (4, 3) } else { (3, 4) };
    // В большинстве залов — длинные столы, малые между ними; иногда наоборот.
    let long_first = rng.chance(0.75);
    // Своё место свободно; клетка прохода справа и снизу занимается после — проход
    // к стойке и полоса у камина сами служат проходом.
    let fits = |sp: &Spots, x: i32, y: i32, w: i32, h: i32| x + w < r.x + r.w && y + h < r.y + r.h && sp.free(x, y, w, h);
    for y in r.y + 1..r.y + r.h - 1 {
        for x in r.x + 1..r.x + r.w - 1 {
            let long = long_first || rng.chance(0.3);
            let (w, h) = if long && fits(sp, x, y, tw, th) {
                trestle(m, rng, x, y, horiz);
                (tw, th)
            } else if long && fits(sp, x, y, th, tw) {
                trestle(m, rng, x, y, !horiz);
                (th, tw)
            } else if fits(sp, x, y, cw, ch) && dining(m, sp, rng, x, y) {
                (cw, ch)
            } else if fits(sp, x, y, 3, 3) && seat_table(m, sp, rng, x, y) {
                (3, 3)
            } else {
                continue;
            };
            sp.block(x, y, w + 1, h + 1);
        }
    }
    dress_walls(m, sp, rng, r);
}

/// Длинный стол со скамьями по обе стороны — как козлы-столы старых трактиров:
/// два стола торцом к торцу, по скамье с каждой стороны. Место — 4×3 клетки от
/// `x`, `y` (`horiz`) или 3×4.
fn trestle(m: &mut Map, rng: &mut Rng, x: i32, y: i32, horiz: bool) {
    let (w, h) = if horiz { (4.0, 3.0) } else { (3.0, 4.0) };
    let c = [x as f32 + w / 2.0, y as f32 + h / 2.0];
    let rot: f32 = if horiz { 0.0 } else { 90.0 };
    // Вдоль стола `a`, поперёк `b` — в клетки карты.
    let at = |a: f32, b: f32| if horiz { [c[0] + a, c[1] + b] } else { [c[0] - b, c[1] + a] };
    let table = asset("table");
    let [l, t, r, u] = bounds(&table, "table", 0.0, 1.0);
    let ts = (1.8 / (r - l)).min(0.95 / (u - t));
    let half = (u - t) * ts / 2.0;
    let bench = asset("bench");
    let [bl, bt, br, bu] = bounds(&bench, "bench", 0.0, 1.0);
    let bs = (1.8 / (br - bl)).min(0.7 / (bu - bt));
    let off = half + (bu - bt) * bs * 0.45;
    for (k, side) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let brot = (rot + 180.0 * k) % 360.0;
        let [bl2, bt2, br2, bu2] = bounds(&bench, "bench", brot, bs);
        for a in [-0.9f32, 0.9] {
            if rng.chance(0.9) {
                let p = at(a, side * off);
                drop_pick(m, &bench, [p[0] - (bl2 + br2) / 2.0, p[1] - (bt2 + bu2) / 2.0], brot, bs);
            }
        }
    }
    let [tl, tt, tr, tu] = bounds(&table, "table", rot, ts);
    for a in [-0.9f32, 0.9] {
        let p = at(a, 0.0);
        drop_pick(m, &table, [p[0] - (tl + tr) / 2.0, p[1] - (tt + tu) / 2.0], rot, ts);
    }
    if !missing("table_clutter") && rng.chance(0.7) {
        let stuff = asset("table_clutter");
        let [sl, st, sr, su] = bounds(&stuff, "table_clutter", rot, 1.0);
        let fit = (1.7 / (sr - sl).max(su - st)).min(1.0);
        let p = at(if rng.chance(0.5) { -0.9 } else { 0.9 }, 0.0);
        drop_pick(m, &stuff, [p[0] - (sl + sr) / 2.0 * fit, p[1] - (st + su) / 2.0 * fit], rot, fit);
    }
}

/// Круглый стол с табуретами по четырём сторонам в квадрате 3×3 от `x`, `y`.
fn seat_table(m: &mut Map, sp: &mut Spots, rng: &mut Rng, x: i32, y: i32) -> bool {
    if !sp.free(x, y, 3, 3) {
        return false;
    }
    sp.block(x, y, 3, 3);
    let c = [x as f32 + 1.5, y as f32 + 1.5];
    let seat = asset(pick(rng, &["stool", "chair"]));
    for (k, rot) in [0.0f32, 90.0, 180.0, 270.0].into_iter().enumerate() {
        if k > 1 && !rng.chance(0.8) {
            continue;
        }
        let a = (rot - 90.0).to_radians();
        drop_pick(m, &seat, [c[0] + 1.05 * a.cos(), c[1] + 1.05 * a.sin()], rot, FURNITURE * 0.9);
    }
    let table = asset("table_round");
    let [l, t, r, u] = bounds(&table, "table_round", 0.0, 1.0);
    let s = (1.6 / (r - l).max(u - t)).min(1.2);
    drop_pick(m, &table, c, 0.0, s);
    true
}

// ----- двор -----

/// Постройка в конце глубокого двора: сарай, конюшня, дровяник. Двор укорачивается,
/// постройка встаёт во всю его ширину, дверь — к дому.
fn outbuilding(m: &mut Map, sp: &mut Spots, rng: &mut Rng, plot: &mut Plot, taken: &mut Taken) -> Option<(Room, WallStyle)> {
    let y = plot.yard?;
    let face = plot.face;
    let (f, d) = (along(y, face), deep(y, face));
    // У постоялого двора конюшня есть всегда: гости приезжают верхом и в каретах.
    if d < 7 || f < 4 || (plot.trade != Trade::Tavern && !rng.chance(0.55)) {
        return None;
    }
    let od = rng.int(3, 4).min(d - 3);
    let shed = sub(y, face, 0, d - od, f, od);
    plot.yard = Some(sub(y, face, 0, 0, f, d - od));
    let st = pick(rng, &["wall_wood", "wall_wood", "wall_timber"]);
    let style = wall(st);
    let planks = mat(m, if plot.trade == Trade::Tavern { "dirt" } else { "wood_planks" });
    m.paint_rect(Layer::Rooms, (shed.x, shed.y), (shed.x + shed.w - 1, shed.y + shed.h - 1), Some(planks));
    let seg = side_seg(shed, face);
    let (w, kind) = if f >= 5 && plot.trade != Trade::Home { (2, PortalKind::DoubleDoor) } else { (1, PortalKind::Door) };
    if let Some(at) = spot(seg, w, 1, taken, rng).or_else(|| spot(seg, 1, 0, taken, rng)) {
        open(m, sp, taken, seg, at, kind, true);
    }
    let fr = FURNITURE;
    let far = (face + 2) % 4;
    match plot.trade {
        // Конюшня: сено и поилка у дальней стены.
        Trade::Tavern => {
            for n in ["hay", "hay", "trough", "barrel"] {
                if !missing(n) {
                    by_side(m, sp, rng, shed, n, 1.0, true, &[far]);
                }
            }
        }
        // Дровяник и уголь.
        Trade::Smith | Trade::Bakery => {
            for n in ["firewood_pile", "firewood_pile", "sacks", "sacks", "barrel", "crate"] {
                if !missing(n) {
                    by_wall(m, sp, rng, shed, n, if n == "firewood_pile" { 1.0 } else { fr }, true);
                }
            }
        }
        _ => furnish(m, sp, rng, shed, Role::Storage),
    }
    let roof = pick(rng, &["roof_thatch", "roof_shingle", "roof_shingle"]);
    m.roofs.push(Roof { a: [shed.x as f32, shed.y as f32], b: [(shed.x + shed.w) as f32, (shed.y + shed.h) as f32], shape: RoofShape::Gable, paint: key(roof), turn: u8::from(shed.h > shed.w), overhang: 0.25 });
    Some((shed, style))
}

/// Края клеток, по которым идёт забор: снизу/справа от клетки, ключ — (вертикальный, линия, место).
type Fence = BTreeMap<(bool, i32), Vec<i32>>;

/// Двор: забор по краям (кроме стены своего дома и края карты), калитка на
/// проезд, хозяйство по назначению.
#[allow(clippy::too_many_arguments)]
fn build_yard(m: &mut Map, sp: &mut Spots, rng: &mut Rng, plot: &Plot, houses: &[Room], streets: &[Street], fence: &mut Fence, well: &mut bool, season: Season) {
    let Some(y) = plot.yard else { return };
    let (w, h) = (m.width as i32, m.height as i32);
    let is_house = |x: i32, yy: i32| houses.iter().any(|r| contains(*r, x, yy));
    let is_street = |x: i32, yy: i32| streets.iter().any(|s| contains(s.r, x, yy));
    // Забор: каждая сторона клетки на краю двора.
    let mut gate_done = false;
    for cy in y.y..y.y + y.h {
        for cx in y.x..y.x + y.w {
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if contains(y, nx, ny) || nx < 0 || ny < 0 || nx >= w || ny >= h || is_house(nx, ny) {
                    continue;
                }
                // Калитка — одна клетка на проезд, посередине стороны.
                if is_street(nx, ny) && !gate_done {
                    let mid = if dx != 0 { cy == y.y + y.h / 2 } else { cx == y.x + y.w / 2 };
                    if mid {
                        gate_done = true;
                        continue;
                    }
                }
                let (vertical, line, at) = match (dx, dy) {
                    (0, -1) => (false, cy, cx),
                    (0, 1) => (false, cy + 1, cx),
                    (-1, 0) => (true, cx, cy),
                    _ => (true, cx + 1, cy),
                };
                let v = fence.entry((vertical, line)).or_default();
                if !v.contains(&at) {
                    v.push(at);
                }
            }
        }
    }
    let face = plot.face;
    // Где дом: у задней стены дома — поленница, в дальнем углу — нужник.
    let house_side = face;
    let far = (face + 2) % 4;
    if !missing("firewood_pile") {
        by_side(m, sp, rng, y, "firewood_pile", 1.0, true, &[house_side]);
    }
    if !missing("privy") && along(y, face) >= 4 && deep(y, face) >= 3 && rng.chance(0.65) {
        let u0 = if rng.chance(0.5) { 0 } else { along(y, face) - 2 };
        let shed = sub(y, face, u0, deep(y, face) - 2, 2, 2);
        if sp.free(shed.x, shed.y, shed.w, shed.h) {
            sp.block(shed.x, shed.y, shed.w, shed.h);
            // Будка: три стены из досок, вход к дому.
            let style = WallStyle { width: 0.14, ..wall("wall_wood") };
            let s = |k: i32| side_seg(shed, k);
            let sides: Vec<i32> = (0..4).filter(|k| *k != house_side).collect();
            for k in sides {
                let seg = s(k);
                m.walls.push(Wall { points: vec![seg.0, seg.1], closed: false, bends: Vec::new(), style: Some(style.clone()) });
            }
            let c = [shed.x as f32 + 1.0, shed.y as f32 + 1.0];
            drop(m, "privy", c, (far as f32 * 90.0 + asset("privy").turn) % 360.0, 1.0);
        }
    }
    let area = y.w * y.h;
    // Вода во дворе — бочка с чистой: колодец FA сверху — тёмная муть.
    if !*well && area >= 12 && rng.chance(0.3) {
        let n = if missing("water_barrel") { "well" } else { "water_barrel" };
        if by_side(m, sp, rng, y, n, 1.0, true, &[house_side]) || inside(m, sp, rng, y, n, 1.0) {
            *well = true;
        }
    }
    if !missing("clothes_line") && area >= 12 && rng.chance(0.45) {
        clothes_line(m, rng, y, face);
    }
    let stuff: &[&str] = match plot.trade {
        Trade::Smith => &["cart", "sacks", "sacks", "barrel", "crate"],
        Trade::Tavern => &["hay", "hay", "trough", "cart", "barrel", "barrel"],
        Trade::Bakery => &["sacks", "sacks", "barrel"],
        Trade::Store => &["crate", "crate", "storage_pile", "cart", "barrel"],
        _ => &["barrel", "crate", "planter"],
    };
    for &n in stuff {
        if !missing(n) {
            by_wall(m, sp, rng, y, n, if n == "cart" { 1.0 } else { 0.95 }, n != "cart");
        }
    }
    // Огород и зелень: куст, иногда дерево, если двор просторный.
    for _ in 0..(area / 10).min(3) {
        if let Some(b) = season_bush(rng, season) {
            inside(m, sp, rng, y, b, 0.9);
        }
    }
    if area >= 20 && rng.chance(0.4) {
        let t = season_tree(rng, season);
        inside(m, sp, rng, y, t, 0.7);
    }
}

/// Верёвка с бельём поперёк двора — от забора до забора (или стены соседа),
/// параллельно задней стене дома. Из вариантов — тот, что ближе по длине; концы
/// тянутся до столбов.
fn clothes_line(m: &mut Map, rng: &mut Rng, y: Room, face: i32) {
    let width = along(y, face) as f32;
    let depth = deep(y, face);
    if depth < 3 {
        return;
    }
    let mut best: Option<(f32, Pick)> = None;
    for _ in 0..8 {
        let p = asset("clothes_line");
        let len = p.size[0].max(p.size[1]);
        let off = (len - width).abs();
        if best.as_ref().is_none_or(|b| off < b.0) {
            best = Some((off, p));
        }
    }
    let Some((_, p)) = best else { return };
    let long_x = p.size[0] >= p.size[1];
    let len = p.size[0].max(p.size[1]);
    let v = rng.int(1, depth - 2) as f32 + 0.5;
    let c = sub(y, face, 0, 0, along(y, face), depth);
    // Середина двора поперёк и выбранная глубина — в клетках карты.
    let pos = match face {
        0 => [c.x as f32 + width / 2.0, c.y as f32 + v],
        2 => [c.x as f32 + width / 2.0, (c.y + c.h) as f32 - v],
        3 => [c.x as f32 + v, c.y as f32 + width / 2.0],
        _ => [(c.x + c.w) as f32 - v, c.y as f32 + width / 2.0],
    };
    let along_x = face % 2 == 0;
    let rot = if along_x == long_x { 0.0 } else { 90.0 };
    drop_pick(m, &p, pos, rot, width / len);
}

/// Забор отрезками: соседние края на одной линии сливаются.
fn emit_fence(m: &mut Map, fence: &Fence, style: &WallStyle) {
    for (&(vertical, line), at) in fence {
        let mut at = at.clone();
        at.sort();
        let mut i = 0;
        while i < at.len() {
            let s = at[i];
            let mut e = s + 1;
            while i + 1 < at.len() && at[i + 1] == e {
                e += 1;
                i += 1;
            }
            let (a, b) = if vertical { ([line as f32, s as f32], [line as f32, e as f32]) } else { ([s as f32, line as f32], [e as f32, line as f32]) };
            m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(style.clone()) });
            i += 1;
        }
    }
}

// ----- всё вместе -----

/// Клетки прямоугольников разом — одним мазком.
fn paint_rects(m: &mut Map, rects: &[Room], material: usize) {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut mask = Mask { w: mw, h: mh, data: vec![0; mw * mh] };
    for r in rects.iter().filter(|r| r.w > 0 && r.h > 0) {
        for (i, _) in mask::cells(mw, mh, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1)) {
            mask.data[i] = 255;
        }
    }
    let weights: Vec<(usize, f32)> = mask.data.iter().enumerate().filter(|(_, v)| **v > 0).map(|(i, _)| (i, 1.0)).collect();
    if !weights.is_empty() {
        m.paint(Layer::Floor, material, &weights);
    }
}

fn rect_mask(m: &Map, rects: &[Room]) -> Mask {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut mask = Mask { w: mw, h: mh, data: vec![0; mw * mh] };
    for r in rects.iter().filter(|r| r.w > 0 && r.h > 0) {
        for (i, _) in mask::cells(mw, mh, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1)) {
            mask.data[i] = 255;
        }
    }
    mask
}

pub(super) fn quarter(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    let season = p.options.season;
    let (grass, wet) = town_grass(season);
    ground(m, "dirt");
    let (count, size) = if season == Season::Winter { (w * h / 50, (2.0, 4.5)) } else { (w * h / 90, (1.2, 3.0)) };
    patches(m, rng, grass, count, size, 0.75);
    patches(m, rng, wet, w * h / 160, (0.8, 2.0), 0.6);

    let district = p.options.district;
    let lay = layout(w, h, rng, p.options.density, district == District::Market);
    let horiz = lay.horiz;
    // Мостовая: проезжая часть и переулки — брусчатка, тротуары и площадь — плиты,
    // задний проезд — утоптанная земля с гравием.
    let walks: Vec<Room> = {
        let (mn, rd) = (lay.main, lay.road);
        if horiz { vec![Room { h: rd.y - mn.y, ..mn }, Room { y: rd.y + rd.h, h: mn.y + mn.h - rd.y - rd.h, ..mn }] } else { vec![Room { w: rd.x - mn.x, ..mn }, Room { x: rd.x + rd.w, w: mn.x + mn.w - rd.x - rd.w, ..mn }] }
    };
    // Переулок доходит до проезжей части: тротуар у его устья — тоже переулок.
    let lanes: Vec<Room> = lay
        .streets
        .iter()
        .filter(|s| s.rank == Rank::Lane)
        .map(|s| {
            let (r, mn, rd) = (s.r, lay.main, lay.road);
            if horiz {
                if r.y + r.h == mn.y { Room { h: r.h + rd.y - mn.y, ..r } } else { Room { y: rd.y + rd.h, h: r.h + (r.y - rd.y - rd.h), ..r } }
            } else if r.x + r.w == mn.x {
                Room { w: r.w + rd.x - mn.x, ..r }
            } else {
                Room { x: rd.x + rd.w, w: r.w + (r.x - rd.x - rd.w), ..r }
            }
        })
        .collect();
    let backs: Vec<Room> = lay.streets.iter().filter(|s| s.rank == Rank::Back).map(|s| s.r).collect();
    let squares: Vec<Room> = lay.streets.iter().filter(|s| s.square).map(|s| s.r).collect();
    let cobble = mat(m, "cobblestone");
    let flag = mat(m, "flagstones");
    let paving = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("flagstones"));
    paint_rects(m, &walks, flag);
    paint_rects(m, &squares, paving);
    let mut roads = vec![lay.road];
    roads.extend(lanes.iter().copied());
    paint_rects(m, &roads, cobble);
    let dirt = mat(m, "dirt");
    paint_rects(m, &backs, dirt);
    patches_in(m, rng, &backs, "gravel", 0.4);
    // Улица грязная: земля наползает, у переулков брусчатка стёрта сильнее.
    let mut worn = Vec::new();
    for r in roads.iter().chain(&backs) {
        let lane = r.w.min(r.h) <= 3;
        for _ in 0..(r.w * r.h / if lane { 8 } else { 14 }) {
            worn.push(([r.x as f32 + rng.random() as f32 * r.w as f32, r.y as f32 + rng.random() as f32 * r.h as f32], 0.5 + rng.random() as f32 * 0.8));
        }
    }
    let weights = blobs(m, &worn, 0.1, 0.65);
    m.paint(Layer::Floor, dirt, &weights);
    // Бордюр — по краю проезжей части и переулков.
    let curb = rect_mask(m, &roads);
    curbs(m, &curb);

    // Участки и дома.
    let mut tally = Tally::default();
    let mut all: Vec<Plot> = Vec::new();
    let inn_at = if district == District::Inn { inn_site(&lay.blocks, &lay.streets, w, h) } else { None };
    let corner_tavern = district != District::Inn || inn_at.is_none();
    for (i, &b) in lay.blocks.iter().enumerate() {
        let inn = inn_at.filter(|(j, _)| *j == i).map(|(_, s)| s);
        all.extend(plots(b, &lay.streets, rng, &mut tally, district, corner_tavern, inn));
    }
    m.wall_style = wall("wall_timber");
    let mut sp = Spots::new(m);
    let mut taken: Taken = BTreeMap::new();
    let plan_look = match (p.style.walls.as_deref().and_then(plan::wall_id), p.style.roof.as_deref().and_then(plan::roof_id)) {
        (None, None) => None,
        (st, roof) => Some((st.unwrap_or("wall_timber"), roof.unwrap_or("roof_clay_red"))),
    };
    // На каждую улицу — свой набор покрытий: у главной черепица и сланец, в переулках
    // попроще, у заднего проезда солома и дранка.
    const SETS: [&[&str]; 3] = [
        &["roof_clay_red", "roof_clay_brown", "roof_slate_gray", "roof_clay_red", "roof_slate_blue"],
        &["roof_clay_brown", "roof_shingle", "roof_clay_red", "roof_thatch", "roof_slate_gray"],
        &["roof_thatch", "roof_shingle", "roof_clay_brown"],
    ];
    let street_roofs: Vec<Vec<&str>> = SETS
        .iter()
        .map(|set| {
            let mut v: Vec<&str> = set.to_vec();
            for i in (1..v.len()).rev() {
                let j = rng.int(0, i as i32) as usize;
                v.swap(i, j);
            }
            v.truncate(3);
            v
        })
        .collect();
    let houses: Vec<Room> = all.iter().map(|p| p.house).collect();
    let mut styles: Vec<WallStyle> = Vec::new();
    for plot in &all {
        let (st, roof) = look(rng, plot, plan_look, &street_roofs[plot.rank as usize]);
        styles.push(build_house(m, &mut sp, rng, plot, st, &roof, &mut taken));
    }
    // Постройки в глубоких дворах — до заборов: двор после них короче.
    let mut sheds: Vec<(Room, WallStyle)> = Vec::new();
    for plot in all.iter_mut() {
        if let Some(s) = outbuilding(m, &mut sp, rng, plot, &mut taken) {
            sheds.push(s);
        }
    }
    let mut houses = houses;
    houses.extend(sheds.iter().map(|s| s.0));
    styles.extend(sheds.iter().map(|s| s.1.clone()));
    // Стены между соседями — одна на двоих, наружной толщины.
    for (i, a) in houses.iter().enumerate() {
        for b in &houses[i + 1..] {
            if let Some(seg) = shared(*a, *b) {
                m.walls.push(Wall { points: vec![seg.0, seg.1], closed: false, bends: Vec::new(), style: Some(styles[i].clone()) });
            }
        }
    }
    let edges = m.room_edges();
    for (r, st) in houses.iter().zip(&styles) {
        let (ax, ay, bx, by) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
        let own: Vec<_> = edges.iter().copied().filter(|s| [s.0, s.1].iter().all(|q| q[0] >= ax && q[0] <= bx && q[1] >= ay && q[1] <= by)).collect();
        m.set_edge_look(&own, st);
    }
    // Двор каждого участка; колодец — один на квартал.
    let mut fence: Fence = BTreeMap::new();
    let mut wells = i32::from(!squares.is_empty());
    for &b in &lay.blocks {
        let mut well = wells >= 2;
        for plot in all.iter().filter(|p| contains(b, p.house.x, p.house.y)) {
            build_yard(m, &mut sp, rng, plot, &houses, &lay.streets, &mut fence, &mut well, season);
        }
        if well && wells < 2 {
            wells += 1;
        }
    }
    // Земля во дворах: у жилья трава пятнами, у кузни и таверны — грязь.
    let green: Vec<Room> = all.iter().filter(|p| matches!(p.trade, Trade::Home | Trade::Shop)).filter_map(|p| p.yard).collect();
    let trodden: Vec<Room> = all.iter().filter(|p| matches!(p.trade, Trade::Smith | Trade::Tavern | Trade::Store | Trade::Bakery)).filter_map(|p| p.yard).collect();
    patches_in(m, rng, &green, grass, 1.6);
    patches_in(m, rng, &trodden, wet, 0.8);
    let fstyle = WallStyle { width: 0.18, ..wall(if rng.chance(0.7) { "wall_wood" } else { "wall_hedge" }) };
    emit_fence(m, &fence, &fstyle);
    // Что в кварталах осталось ничьим — пустырь: куст или дерево.
    for &b in &lay.blocks {
        for _ in 0..(b.w * b.h / 30) {
            let (x, y) = (rng.int(b.x, b.x + b.w - 1), rng.int(b.y, b.y + b.h - 1));
            let free = !all.iter().any(|p| contains(p.house, x, y) || p.yard.is_some_and(|r| contains(r, x, y)));
            if free {
                let cell = Room { x, y, w: 1, h: 1 };
                if let Some(n) = season_bush(rng, season) {
                    inside(m, &mut sp, rng, cell, n, 0.9);
                }
            }
        }
    }

    // Улица у фасадов: товар у лавок, бочки и цветы у домов.
    for plot in &all {
        let front = sub(plot.house, plot.face, 0, -1, along(plot.house, plot.face), 1);
        let n = match plot.trade {
            Trade::Shop | Trade::Bakery if district == District::Market => 3,
            Trade::Shop | Trade::Bakery | Trade::Smith | Trade::Store => 2,
            Trade::Tavern => 3,
            Trade::Home => i32::from(rng.chance(0.4)),
        };
        for _ in 0..n {
            let what: &[&str] = match plot.trade {
                Trade::Tavern => &["barrel", "bench", "planter"],
                Trade::Smith => &["barrel", "crate", "weapon_rack"],
                Trade::Home => &["planter", "barrel", "crate"],
                _ => &["crate", "barrel", "sacks", "planter"],
            };
            let n = pick(rng, what);
            let fcells = along(plot.house, plot.face);
            let k = rng.int(0, fcells - 1);
            let c = sub(front, plot.face, k, 0, 1, 1);
            put(m, &mut sp, n, c.x, c.y, (plot.face as f32 * 90.0) % 360.0, 0.8);
        }
    }
    // Фонари по тротуару через 8–10 клеток, по очереди на две стороны; телеги у бордюра.
    let rd = lay.road;
    let long = if horiz { w } else { h };
    let mut t = rng.int(2, 5);
    let mut side = 0;
    while t < long - 1 {
        let wk = walks[side % walks.len()];
        let (x, y) = if horiz { (t, wk.y) } else { (wk.x, t) };
        let in_lane = lanes.iter().any(|l| contains(Room { x: l.x - 1, y: l.y - 1, w: l.w + 2, h: l.h + 2 }, x, y));
        if wk.w > 0 && wk.h > 0 && !in_lane {
            put(m, &mut sp, "street_lamp", x, y, 0.0, 0.7);
        }
        side += 1;
        t += rng.int(8, 10);
    }
    for _ in 0..(long / 22).max(1) {
        if rng.chance(0.7) {
            let t = rng.int(2, long - 5);
            let near = rng.chance(0.5);
            let (x, y, rot) = if horiz { (t, if near { rd.y } else { rd.y + rd.h - 2 }, 90.0) } else { (if near { rd.x } else { rd.x + rd.w - 2 }, t, 0.0) };
            put(m, &mut sp, "cart", x, y, rot, 1.0);
        }
    }
    // В переулке мелочь у стен, проход посередине свободен.
    for r in &lanes {
        for _ in 0..(r.w * r.h / 18) {
            let n = pick(rng, &["barrel", "crate", "sacks", "barrel"]);
            by_wall(m, &mut sp, rng, *r, n, 0.8, false);
        }
    }

    // Площадь: колодец или фонтан посередине, лавки с навесами по краю, доска объявлений.
    if let Some(&b) = squares.first() {
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
        let centre = if b.w >= 9 && b.h >= 7 { "fountain" } else { "well" };
        let c = asset(centre);
        let s = c.size[0] as i32;
        put_pick(m, &mut sp, &c, centre, cx - s / 2, cy - s / 2, 0.0, 1.0);
        let stalls = if district == District::Market { (b.w * b.h / 12).clamp(4, 9) } else { (b.w * b.h / 20).clamp(2, 5) };
        for _ in 0..stalls {
            let n = pick(rng, &["market_stall", "market_stall_blue", "market_stall_ochre"]);
            if by_wall(m, &mut sp, rng, b, n, 1.0, false)
                && let Some(p) = m.props.last_mut()
            {
                p.open = Some(open_toward(p, [cx as f32 + 0.5, cy as f32 + 0.5]));
            }
        }
        by_wall(m, &mut sp, rng, b, "notice_board", 1.0, false);
        for _ in 0..2 {
            let bench = if missing("bench_stone") { "bench" } else { "bench_stone" };
            by_wall(m, &mut sp, rng, b, bench, 1.0, false);
        }
        for (x, y) in [(b.x, b.y), (b.x + b.w - 1, b.y + b.h - 1)] {
            put(m, &mut sp, "street_lamp", x, y, 0.0, 0.7);
        }
        wanted(m, &mut sp, rng, b, &p.objects, "площадь", warn);
    } else if !p.objects.is_empty() {
        let r = walks.iter().copied().max_by_key(|r| r.w * r.h).unwrap_or(lay.main);
        wanted(m, &mut sp, rng, r, &p.objects, "улица", warn);
    }

    if p.mood == Mood::Abandoned {
        abandon(m, rng, &houses);
    }
    let edges = m.room_edges();
    let looks = std::mem::take(&mut m.edge_looks);
    m.sync_edge_looks(&edges);
    m.edge_looks.extend(looks);
    all.len()
}

/// Сторона картинки (0 — верх, 1 — право, 2 — низ, 3 — лево), что смотрит на точку `to`.
fn open_toward(p: &Prop, to: [f32; 2]) -> u8 {
    let (dx, dy) = (to[0] - p.pos[0], to[1] - p.pos[1]);
    // В координаты картинки: обратный поворот.
    let (s, c) = (-p.rot.to_radians()).sin_cos();
    let (lx, ly) = (dx * c - dy * s, dx * s + dy * c);
    if lx.abs() > ly.abs() {
        if lx > 0.0 { 1 } else { 3 }
    } else if ly > 0.0 {
        2
    } else {
        0
    }
}

/// Пятна внутри прямоугольников: гравий по заднему проезду.
fn patches_in(m: &mut Map, rng: &mut Rng, rects: &[Room], name: &str, part: f32) {
    let mut spots = Vec::new();
    for r in rects {
        for _ in 0..((r.w * r.h) as f32 * part / 4.0) as i32 {
            spots.push(([r.x as f32 + rng.random() as f32 * r.w as f32, r.y as f32 + rng.random() as f32 * r.h as f32], 0.5 + rng.random() as f32 * 0.6));
        }
    }
    if spots.is_empty() {
        return;
    }
    let material = mat(m, name);
    let weights = blobs(m, &spots, 0.2, 0.6);
    m.paint(Layer::Floor, material, &weights);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Options;

    fn town(w: u32, h: u32, seed: &str) -> (Map, Report) {
        let o = Options { kind: Kind::Town, seed: seed.into(), ..Options::default() };
        let plan = Plan::random(&o, [w, h]);
        let mut m = Map::new(w, h);
        let r = build(&mut m, &plan, 0, Palette::builtin()).unwrap();
        (m, r)
    }

    #[test]
    fn houses_stand_wall_to_wall_and_never_overlap() {
        for seed in ["а", "б", "в", "г"] {
            let (m, r) = town(48, 36, seed);
            assert!(r.count >= 8, "домов {}", r.count);
            for (i, a) in m.roofs.iter().enumerate() {
                for b in &m.roofs[i + 1..] {
                    assert!(a.b[0] <= b.a[0] || b.b[0] <= a.a[0] || a.b[1] <= b.a[1] || b.b[1] <= a.a[1], "дома наехали");
                }
            }
            // Есть соседи вплотную: общая стена.
            let touching = m.roofs.iter().enumerate().any(|(i, a)| m.roofs[i + 1..].iter().any(|b| a.b[0] == b.a[0] || b.b[0] == a.a[0] || a.b[1] == b.a[1] || b.b[1] == a.a[1]));
            assert!(touching);
            assert!(m.paths.iter().any(|p| p.along), "нет бордюра");
        }
    }

    #[test]
    fn every_house_has_a_street_door_and_furniture() {
        let (m, _) = town(48, 36, "двери");
        for roof in &m.roofs {
            let (a, b) = (roof.a, roof.b);
            let on_edge = |q: [f32; 2]| ((q[0] - a[0]).abs() < 0.01 || (q[0] - b[0]).abs() < 0.01) && q[1] >= a[1] && q[1] <= b[1] || ((q[1] - a[1]).abs() < 0.01 || (q[1] - b[1]).abs() < 0.01) && q[0] >= a[0] && q[0] <= b[0];
            let doors = m.portals.iter().filter(|p| p.kind != PortalKind::Window && on_edge(p.center())).count();
            assert!(doors >= 1, "дом без входа {a:?}–{b:?}");
            let inside = m.props.iter().filter(|p| p.pos[0] > a[0] && p.pos[0] < b[0] && p.pos[1] > a[1] && p.pos[1] < b[1]).count();
            let names: Vec<&str> = m.props.iter().filter(|p| p.pos[0] > a[0] && p.pos[0] < b[0] && p.pos[1] > a[1] && p.pos[1] < b[1]).map(|p| p.asset.as_str()).collect();
            assert!(inside >= 3, "пустой дом {a:?}–{b:?}: {names:?}");
        }
        assert!(m.props.iter().any(|p| p.asset.ends_with("bed") || p.asset.ends_with("bed_double")), "негде спать");
    }

    fn town_of(w: u32, h: u32, seed: &str, district: District) -> (Map, Report) {
        let o = Options { kind: Kind::Town, seed: seed.into(), district, ..Options::default() };
        let plan = Plan::random(&o, [w, h]);
        let mut m = Map::new(w, h);
        let r = build(&mut m, &plan, 0, Palette::builtin()).unwrap();
        (m, r)
    }

    #[test]
    fn inn_quarter_has_a_big_inn_with_bar_kitchen_and_cellar() {
        for seed in ["а", "б", "в", "г", "д", "е"] {
            let (m, _) = town_of(48, 36, seed, District::Inn);
            // Самый большой дом квартала — постоялый двор, не меньше 13×9.
            let area = |r: &Roof| ((r.b[0] - r.a[0]) * (r.b[1] - r.a[1])) as i32;
            let inn = m.roofs.iter().max_by_key(|r| area(r)).unwrap();
            assert!(area(inn) >= 13 * 9, "{seed}: самый большой дом {}", area(inn));
            let (a, b) = (inn.a, inn.b);
            let inside = |p: &Prop| p.pos[0] > a[0] && p.pos[0] < b[0] && p.pos[1] > a[1] && p.pos[1] < b[1];
            assert!(m.props.iter().any(|p| inside(p) && p.asset.ends_with("bar_counter")), "{seed}: стойка не в постоялом дворе");
            assert!(m.props.iter().filter(|p| inside(p) && p.asset.ends_with("fireplace")).count() >= 2, "{seed}: очагов меньше двух");
            assert!(m.props.iter().filter(|p| inside(p) && p.asset.ends_with("barrel")).count() >= 4, "{seed}: погреб пуст");
        }
    }

    #[test]
    fn market_streets_are_lined_with_shops() {
        let mut rng = Rng::new("рынок");
        let mut t = Tally::default();
        let n = 400;
        let shops = (0..n).filter(|_| matches!(choose(Rank::Main, false, 9, &mut t, &mut rng, District::Market, true), Trade::Shop | Trade::Bakery)).count();
        assert!(shops * 10 >= n * 8, "лавок {shops} из {n}");
        let (m, _) = town_of(40, 28, "рынок", District::Market);
        let stalls = m.props.iter().filter(|p| p.asset.contains("market_stall")).count();
        assert!(stalls >= 4, "прилавков {stalls}");
    }

    #[test]
    fn small_map_still_builds() {
        let (_, r) = town(24, 16, "тесно");
        assert!(r.count >= 2, "{}", r.count);
    }
}

#[cfg(test)]
mod fa_slots {
    use super::*;

    /// Какие файлы паков FA берутся в слоты квартала. Только на машине с паками.
    #[test]
    #[ignore]
    fn print_fa_slots() {
        let dir = std::path::Path::new("E:/D&D/DD ASSET NEW/DungeonDraft Assets/V3.5");
        let mut assets = Vec::new();
        for f in ["FA_Objects_A_v3.52.dungeondraft_pack", "FA_Objects_B_v3.82.dungeondraft_pack"] {
            assets.extend(library::pack_assets(&dir.join(f)).unwrap());
        }
        let pal = Palette::from_library(&assets, Box::new(crate::palette::fake_measure));
        if let Ok(seed) = std::env::var("WINDOWS") {
            let mut plan = Plan::random(&crate::generate::Options { kind: Kind::Town, seed: seed.clone(), library: true, ..Default::default() }, [34, 46]);
            plan.assets = Assets::Library;
            let mut m = Map::new(34, 46);
            build(&mut m, &plan, 0, Palette::from_library(&assets, Box::new(crate::palette::fake_measure))).unwrap();
            let name = |k: &str| k.rsplit('/').next().unwrap_or(k).to_owned();
            for p in m.portals.iter().filter(|p| p.kind == PortalKind::Window) {
                let len = (p.b[0] - p.a[0]).abs() + (p.b[1] - p.a[1]).abs();
                let sill = p.asset.clone().unwrap_or_default();
                let glass = library::window_glass(&assets, &sill, len).map(|g| name(&g));
                println!("{:?} {:?} sill {} frame {:?} glass {:?}", p.a, p.b, name(&sill), p.frame.as_deref().map(name), glass);
            }
        }
        let slots = std::env::var("SLOTS").unwrap_or_default();
        for slot in slots.split(',') {
            let keys = pal.keys(slot);
            println!("== {slot}: {}", keys.len());
            let mut names: Vec<String> = keys.iter().map(|k| k.rsplit('/').next().unwrap_or(k).split("_1x").next().unwrap().to_owned()).collect();
            names.sort();
            names.dedup_by(|a, b| a.split('_').take(3).eq(b.split('_').take(3)));
            for n in names.iter().take(40) {
                println!("   {n}");
            }
        }
    }
}
