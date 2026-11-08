//! Колизей: овальная арена на песке, подиум со стеной, ряды трибун уступами до
//! внешней аркады. Устроен, как римский амфитеатр: арена, подиум с почётными
//! местами, кавея рядами (с широким проходом и низкой стенкой посередине у
//! больших), лестницы-проходы поперёк рядов, арки внешней стены, двое ворот по
//! длинной оси и ложа на короткой. Под боевую сетку: ворота — проходы насквозь
//! до площади, напротив ложи — загон для зверей с решёткой в арену.
//!
//! Тени. Трибуны — ступени разной высоты, их тень не смещённый силуэт, как у
//! стен и объектов, а считается по высотам: из каждой точки луч к солнцу, и если
//! что-то поднимается выше луча — точка в тени. Солнце то же, что у теней
//! объектов (`Map::shadows`). Выходит, как днём на настоящей арене: подиум кладёт
//! серп тени на песок у солнечной стены, ступени на солнечной стороне полосаты,
//! на дальней — светлы, а громада внешней стены накрывает площадь за собой.

use super::*;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// Полуширина ворот и прохода к ним.
const GATE: f32 = 1.5;
/// Полуширина ложи и её глубина от стены арены (два ряда).
const BOX: f32 = 3.0;
/// Полуширина загона и его глубина.
const PEN: f32 = 2.0;
/// Где кончается подиум: дорожка за стеной арены.
const S0: f32 = 1.2;
/// Толщина стен по высотам: подиум и внешняя стена.
const WALL_BAND: f32 = 0.35;
/// Высоты, клетки: стена арены (около 4,5 м), дорожка подиума, подъём ряда,
/// стенка над верхним проходом.
const WALL_H: f32 = 3.0;
const WALK_H: f32 = 2.6;
const RISE: f32 = 0.6;
const ATTIC: f32 = 2.8;
/// Тангенс высоты солнца: тень втрое короче… — на клетку высоты 0,8 клетки тени.
const SUN: f32 = 1.25;
/// Насколько темна тень трибун.
const SHADE: f32 = 0.55;
/// Тень у подножия каждой ступени и стены — со всех сторон, не от солнца:
/// свет из-под уступа не достаёт. Насколько темна и как далеко тянется.
const CONTACT: f32 = 0.42;
const CONTACT_REACH: f32 = 0.55;

/// Вид камня: пол рядов, проходов, подиума и стены.
struct Look {
    tiers: &'static str,
    walk: &'static str,
    podium: &'static str,
    wall: &'static str,
}

const LOOKS: [Look; 3] = [
    Look { tiers: "arena_tiers_sand", walk: "arena_walk_sand", podium: "arena_marble", wall: "wall_sandstone" },
    Look { tiers: "arena_tiers_slate", walk: "arena_walk_slate", podium: "arena_marble", wall: "wall_stone_blocks" },
    Look { tiers: "arena_tiers_sand", walk: "arena_walk_slate", podium: "arena_marble", wall: "wall_stone_blocks" },
];

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Seat,
    /// Широкий проход посередине кавеи, за ним низкая стенка.
    Walk,
    /// Верхний проход у внешней стены.
    Top,
}

/// Чаша: всё в своих осях — `u` вдоль длинной стороны, `v` поперёк.
struct Bowl {
    c: [f32; 2],
    /// Полуоси арены.
    a: f32,
    b: f32,
    long_x: bool,
    /// Ряды от подиума наружу: начало, конец, вид, высота.
    rows: Vec<(f32, f32, Row, f32)>,
    /// Внешний край (ось внешней стены — чуть внутри).
    outer: f32,
    /// Трибун нет: карта мала, одна арена за стеной.
    pit: bool,
}

impl Bowl {
    fn world(&self, u: f32, v: f32) -> [f32; 2] {
        if self.long_x { [self.c[0] + u, self.c[1] + v] } else { [self.c[0] + v, self.c[1] + u] }
    }

    fn local(&self, p: [f32; 2]) -> (f32, f32) {
        let (dx, dy) = (p[0] - self.c[0], p[1] - self.c[1]);
        if self.long_x { (dx, dy) } else { (dy, dx) }
    }

    /// Направление в мире из своих осей.
    fn dir(&self, du: f32, dv: f32) -> [f32; 2] {
        if self.long_x { [du, dv] } else { [dv, du] }
    }

    /// Точка кольца `s` (столько клеток от края арены) под углом `t`.
    fn ring(&self, s: f32, t: f32) -> [f32; 2] {
        self.world((self.a + s) * t.cos(), (self.b + s) * t.sin())
    }

    /// На каком кольце точка: эллипс с полуосями `a + s`, `b + s` через неё.
    fn s_at(&self, u: f32, v: f32) -> f32 {
        let f = |s: f32| (u / (self.a + s)).powi(2) + (v / (self.b + s)).powi(2);
        let (mut lo, mut hi) = (-self.b * 0.999, u.abs() + v.abs() + 1.0);
        if f(lo) <= 1.0 {
            return lo;
        }
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if f(mid) > 1.0 { lo = mid } else { hi = mid }
        }
        (lo + hi) / 2.0
    }

    fn passage(&self, v: f32, s: f32) -> bool {
        !self.pit && v.abs() < GATE && s >= 0.0 && s <= self.outer + 0.5
    }

    fn box_depth(&self) -> f32 {
        S0 + 2.0
    }

    fn in_box(&self, u: f32, v: f32, s: f32) -> bool {
        !self.pit && u.abs() < BOX && v < 0.0 && s >= 0.0 && s < self.box_depth()
    }

    fn in_pen(&self, u: f32, v: f32, s: f32) -> bool {
        !self.pit && u.abs() < PEN && v > 0.0 && s >= 0.0 && s < self.box_depth()
    }

    /// Высота точки, клетки.
    fn height(&self, u: f32, v: f32) -> f32 {
        let s = self.s_at(u, v);
        if s < 0.0 || self.passage(v, s) || self.in_pen(u, v, s) {
            return 0.0;
        }
        if self.pit {
            return if s < WALL_BAND { WALL_H } else { 0.0 };
        }
        if s > self.outer {
            return 0.0;
        }
        if s < WALL_BAND {
            return WALL_H;
        }
        if s < S0 || self.in_box(u, v, s) {
            return WALK_H;
        }
        let top = self.rows.last().map_or(WALK_H, |r| r.3);
        if s >= self.outer - WALL_BAND {
            return top + ATTIC;
        }
        for &(from, to, row, h) in &self.rows {
            if s >= from && s < to {
                // Низкая стенка за средним проходом.
                return if row == Row::Walk && s > to - 0.2 { h + 0.9 } else { h };
            }
        }
        top
    }

    /// Вырезы колец: проходы к воротам, ложа и загон (на своей глубине).
    fn cut(&self, s: f32, t: f32) -> bool {
        let (u, v) = ((self.a + s) * t.cos(), (self.b + s) * t.sin());
        self.passage(v, s) || self.in_box(u, v, s) || self.in_pen(u, v, s)
    }

    /// Куски кольца `s` вне вырезов: ломаные с шагом около 0,4 клетки.
    fn arcs(&self, s: f32) -> Vec<Vec<[f32; 2]>> {
        let r = (self.a + self.b) / 2.0 + s;
        let n = ((TAU * r / 0.1).ceil() as usize).max(64);
        let keep = (0.4 / 0.1) as usize;
        let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
        let mut run: Vec<[f32; 2]> = Vec::new();
        let mut since = 0;
        // Начало — в вырезе, если он есть: тогда кольцо не рвётся на стыке.
        let start = (0..n).find(|&k| self.cut(s, k as f32 / n as f32 * TAU)).unwrap_or(0);
        for k in 0..=n {
            let t = (start + k) as f32 / n as f32 * TAU;
            if self.cut(s, t) {
                if run.len() >= 2 {
                    out.push(std::mem::take(&mut run));
                }
                run.clear();
                since = 0;
                continue;
            }
            if run.is_empty() || since + 1 >= keep {
                run.push(self.ring(s, t));
                since = 0;
            } else {
                since += 1;
                // Последняя точка перед вырезом — всегда.
                if self.cut(s, t + TAU / n as f32) {
                    run.push(self.ring(s, t));
                    since = 0;
                }
            }
        }
        if run.len() >= 2 {
            out.push(run);
        }
        out
    }
}

fn rot_from(d: [f32; 2]) -> f32 {
    // Спина предмета — вверх; поворот по часовой, при котором она смотрит в `d`.
    d[0].atan2(-d[1]).to_degrees().rem_euclid(360.0)
}

/// Кольцо стены: точки по эллипсу, хорды `chords` — прямые отрезки (под двери и
/// арки), `stops` — обязательные вершины (стыки со стенами проходов). Углы — в
/// порядке обхода, от `t0` до `t1`.
fn ring_line(bowl: &Bowl, s: f32, t0: f32, t1: f32, chords: &[(f32, f32)], stops: &[f32]) -> Vec<[f32; 2]> {
    let r = (bowl.a + bowl.b) / 2.0 + s;
    let step = 0.7 / r;
    let mut ts: Vec<f32> = vec![t0];
    let mut t = t0;
    while t < t1 - 1e-4 {
        // Ближайшая хорда впереди.
        if let Some(&(_, c1)) = chords.iter().find(|c| (c.0 - t).abs() < 1e-4) {
            ts.push(c1);
            t = c1;
            continue;
        }
        let next_chord = chords.iter().map(|c| c.0).filter(|&c| c > t + 1e-4).fold(t1, f32::min);
        let next_stop = stops.iter().copied().filter(|&c| c > t + 1e-4).fold(t1, f32::min);
        let lim = next_chord.min(next_stop);
        let nt = (t + step).min(lim);
        // Не оставлять огрызок короче трети шага.
        let nt = if lim - nt < step / 3.0 { lim } else { nt };
        ts.push(nt);
        t = nt;
    }
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    ts.into_iter().map(|t| bowl.ring(s, t)).collect()
}

pub(super) fn arena(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as f32, m.height as f32);
    let density = p.options.density;
    let season = p.options.season;
    let long_x = w >= h;
    let (half_long, half_short) = if long_x { (w / 2.0, h / 2.0) } else { (h / 2.0, w / 2.0) };
    // Снаружи — полклетки площади до края карты.
    let (big_a, big_b) = (half_long - 0.6, half_short - 0.6);
    let mut band = (big_b * 0.48).min(11.0);
    band = band.min(big_b - (GATE + 2.0).max(big_b * 0.4));
    let pit = band < S0 + 1.0 + 1.0 + WALL_BAND;
    let (a, b, outer) = if pit { (big_a - 0.4, big_b - 0.4, 0.0) } else { (big_a - band, big_b - band, band) };
    let mut rows = Vec::new();
    if !pit {
        // Ряды: сколько влезет между подиумом и верхним проходом (не уже клетки).
        let room = outer - WALL_BAND - 1.0 - S0;
        let mut seats = room.floor() as i32;
        let mid = seats >= 8;
        if mid {
            seats -= 1;
        }
        let (mut s, mut height) = (S0, WALK_H);
        for k in 0..seats {
            if mid && k == seats / 2 {
                rows.push((s, s + 1.0, Row::Walk, height));
                s += 1.0;
                height += 0.4;
            }
            height += RISE;
            rows.push((s, s + 1.0, Row::Seat, height));
            s += 1.0;
        }
        rows.push((s, outer - WALL_BAND, Row::Top, height));
    }
    let bowl = Bowl { c: [w / 2.0, h / 2.0], a, b, long_x, rows, outer, pit };
    let seats = bowl.rows.iter().filter(|r| r.2 == Row::Seat).count();

    let look = &LOOKS[rng.int(0, LOOKS.len() as i32 - 1) as usize];
    let wall_slot = p.style.walls.as_deref().and_then(plan::wall_id).unwrap_or(look.wall);
    let tier_slot = p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or(look.tiers);

    // ----- пол -----
    let (grass, wet) = town_grass(season);
    ground(m, "cobblestone");
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    patches(m, rng, "dirt", (w * h / 70.0) as i32, (1.0, 2.5), 0.6);
    patches(m, rng, grass, (w * h / 110.0) as i32, (0.8, 2.2), 0.7);
    patches(m, rng, wet, (w * h / 220.0) as i32, (0.6, 1.4), 0.6);
    let (sand, rough, dirt) = (mat(m, "arena_sand"), mat(m, "arena_sand_rough"), mat(m, "dirt"));
    let (tiers, walk, podium) = (mat(m, tier_slot), mat(m, look.walk), mat(m, look.podium));
    // Сетка: кольцо и высота каждой точки маски.
    let mut ring_of = vec![0.0f32; mw * mh];
    let mut height = vec![0.0f32; mw * mh];
    let mut by: BTreeMap<usize, Vec<(usize, f32)>> = BTreeMap::new();
    for j in 0..mh {
        for i in 0..mw {
            let k = j * mw + i;
            let (u, v) = bowl.local(mask::sample_pos(i as isize, j as isize));
            let s = bowl.s_at(u, v);
            ring_of[k] = s;
            height[k] = bowl.height(u, v);
            let floor = if s < 0.0 {
                Some(sand)
            } else if pit {
                (s < WALL_BAND + 0.3).then_some(walk)
            } else if s > outer {
                None
            } else if bowl.passage(v, s) || bowl.in_pen(u, v, s) {
                Some(walk)
            } else if s < S0 || bowl.in_box(u, v, s) {
                Some(podium)
            } else if bowl.rows.iter().any(|r| r.2 == Row::Seat && s >= r.0 && s < r.1) {
                Some(tiers)
            } else {
                Some(walk)
            };
            if let Some(f) = floor {
                by.entry(f).or_default().push((k, 1.0));
            }
        }
    }
    for (f, weights) in &by {
        m.paint(Layer::Floor, *f, weights);
    }
    // Песок истоптан: у ворот, посередине и где бились — пятна крупного песка и земли.
    let in_arena = |ws: Vec<(usize, f32)>| -> Vec<(usize, f32)> {
        ws.into_iter().filter(|(k, _)| ring_of[*k] < -0.3).collect()
    };
    let mut spots = Vec::new();
    for _ in 0..(a * b / 16.0) as i32 + 2 {
        let (t, r) = (rng.random() as f32 * TAU, (rng.random() as f32).sqrt() * 0.85);
        spots.push((bowl.world(a * r * t.cos(), b * r * t.sin()), 0.8 + rng.random() as f32 * 1.4));
    }
    let ws = in_arena(blobs(m, &spots, 0.15, 0.55));
    m.paint(Layer::Floor, rough, &ws);
    let mut trod = vec![(bowl.world(0.0, 0.0), (b * 0.3).max(1.2))];
    if !pit {
        trod.push((bowl.world(a - 1.0, 0.0), 1.8));
        trod.push((bowl.world(-(a - 1.0), 0.0), 1.8));
    }
    let ws = in_arena(blobs(m, &trod, 0.1, 0.3));
    m.paint(Layer::Floor, dirt, &ws);

    // ----- ступени, проходы-лестницы, стенки -----
    let riser = mat(m, wall_slot);
    // Край сиденья — светлый каменный выступ: на нём солнце, под ним тень.
    let lip = podium;
    if !pit {
        let top = bowl.rows.last().map_or(S0, |r| r.0);
        // Лестницы поперёк рядов: через 7–8 клеток, поровну с обеих сторон.
        let mid_r = (a + b) / 2.0 + outer / 2.0;
        let n = (((TAU * mid_r / 7.5) / 2.0).round() as i32 * 2).max(4);
        for i in 0..n {
            let t = (i as f32 + 0.5) / n as f32 * TAU;
            // Не у ворот, ложи и загона — ни на одном кольце.
            if [S0, (S0 + top) / 2.0, top].iter().any(|&s| bowl.cut(s, t) || bowl.cut(s, t + 0.08) || bowl.cut(s, t - 0.08)) {
                continue;
            }
            // Лестница — по краям бортики, посреди каждого ряда полуступень:
            // ряд высок, по нему шагают дважды.
            let side = bowl.dir(-t.sin(), t.cos());
            let off = |q: [f32; 2], k: f32| [q[0] + side[0] * k, q[1] + side[1] * k];
            let end = top;
            for k in [-0.45f32, 0.45] {
                let points = (0..=8).map(|n| off(bowl.ring(S0 + (end - S0) * n as f32 / 8.0, t), k)).collect();
                m.paths.push(Path { points, material: lip, width: 0.07, soft: 0.0, rough: 0.0, along: true });
            }
            for &(from, to, row, _) in &bowl.rows {
                if row == Row::Seat {
                    let q = bowl.ring((from + to) / 2.0, t);
                    m.paths.push(Path { points: vec![off(q, -0.45), q, off(q, 0.45)], material: lip, width: 0.07, soft: 0.0, rough: 0.0, along: true });
                }
            }
        }
        // Край каждого ряда — камень подступенка.
        let mut edges: Vec<(f32, f32, usize)> = vec![(S0, 0.09, lip)];
        for &(_, to, row, _) in &bowl.rows {
            if row != Row::Top {
                edges.push(if row == Row::Walk { (to - 0.1, 0.3, riser) } else { (to, 0.09, lip) });
            }
        }
        for (s, width, material) in edges {
            for line in bowl.arcs(s) {
                m.paths.push(Path { points: line, material, width, soft: 0.0, rough: 0.0, along: true });
            }
        }
        // Бортики ложи.
        for side in [-1.0f32, 1.0] {
            let (p0, p1) = (bowl.world(side * BOX, -(b * (1.0 - (BOX / a).powi(2)).max(0.0).sqrt())), bowl.world(side * BOX, -(b + bowl.box_depth()) * (1.0 - (BOX / (a + bowl.box_depth())).powi(2)).max(0.0).sqrt()));
            m.paths.push(Path { points: vec![p0, p1], material: riser, width: 0.25, soft: 0.0, rough: 0.0, along: true });
        }
    }

    // ----- стены, ворота, арки -----
    let style = wall(wall_slot);
    let g0 = (GATE / b).min(1.0).asin();
    let door_t = (1.0 / a).min(1.0).asin();
    let pen0 = (PEN / a).min(1.0).asin();
    let gate = |x: [f32; 2], y: [f32; 2]| Portal { a: x, b: y, kind: PortalKind::DoubleDoor, asset: Some(asset("door_metal").key), frame: Some(key("frame_stone_3")) };
    // Стена рисуется с концами, продлёнными на полтолщины: куски вокруг проёма
    // кончаются на полтолщины раньше — и торец встаёт ровно в косяк, а не
    // заходит под него. Куски, что упираются в стену прохода, — без отступа.
    let half_w = style.width / 2.0;
    let back = |s: f32, t: f32| half_w / ((bowl.a + s).powi(2) * t.sin().powi(2) + (bowl.b + s).powi(2) * t.cos().powi(2)).sqrt().max(0.1);
    let piece = |m: &mut Map, s: f32, t0: f32, t1: f32, stops: &[f32]| {
        if t1 - t0 > 1e-3 {
            m.walls.push(Wall { points: ring_line(&bowl, s, t0, t1, &[], stops), closed: false, bends: Vec::new(), low: false, style: Some(style.clone()) });
        }
    };
    if pit {
        // Стена кругом, ворота по длинной оси.
        piece(m, 0.0, g0 + back(0.0, g0), PI - g0 - back(0.0, g0), &[]);
        piece(m, 0.0, PI + g0 + back(0.0, g0), TAU - g0 - back(0.0, g0), &[]);
        for (c0, c1) in [(-g0, g0), (PI - g0, PI + g0)] {
            m.portals.push(gate(bowl.ring(0.0, c0), bowl.ring(0.0, c1)));
        }
    } else {
        // Стена арены: от ворот до ворот, на юге — разрыв под решётку загона.
        let (d0, d1) = (FRAC_PI_2 - door_t, FRAC_PI_2 + door_t);
        // У ворот — тоже с отступом: торец дуги ложится в угол со стеной прохода,
        // а не торчит клином в арену.
        let bg = back(0.0, g0);
        piece(m, 0.0, g0 + bg, d0 - back(0.0, d0), &[FRAC_PI_2 - pen0]);
        piece(m, 0.0, d1 + back(0.0, d1), PI - g0 - bg, &[FRAC_PI_2 + pen0]);
        piece(m, 0.0, PI + g0 + bg, TAU - g0 - bg, &[]);
        // Ворота — в полуклетке вглубь прохода: косяки лежат на его стенах, а не
        // висят над песком у их торцов.
        let corner = a * (1.0 - (GATE / b).powi(2)).max(0.0).sqrt();
        for side in [1.0f32, -1.0] {
            let u = side * (corner + 0.5);
            m.portals.push(gate(bowl.world(u, -GATE), bowl.world(u, GATE)));
        }
        m.portals.push(Portal { a: bowl.ring(0.0, d0), b: bowl.ring(0.0, d1), kind: PortalKind::DoubleDoor, asset: Some(asset("door_wood_iron").key), frame: Some(key("frame_stone_2")) });
        // Внешняя стена: две дуги между проходами, в них арки по клетке.
        // У проходов дуга упирается в их стену — там без отступа.
        let s_out = outer - WALL_BAND / 2.0;
        let g_out = (GATE / (b + s_out)).min(1.0).asin();
        let r_out = (a + b) / 2.0 + s_out;
        for (t0, t1) in [(g_out, PI - g_out), (PI + g_out, TAU - g_out)] {
            let count = ((t1 - t0) * r_out / 4.0).floor().max(1.0) as i32;
            let arches: Vec<(f32, f32)> = (0..count)
                .map(|k| t0 + (k as f32 + 0.5) / count as f32 * (t1 - t0))
                .map(|c| {
                    let half = 0.5 / ((a + s_out).powi(2) * c.sin().powi(2) + (b + s_out).powi(2) * c.cos().powi(2)).sqrt();
                    (c - half, c + half)
                })
                .collect();
            let mut from = t0;
            for &(c0, c1) in &arches {
                piece(m, s_out, from, c0 - back(s_out, c0), &[]);
                m.portals.push(Portal { a: bowl.ring(s_out, c0), b: bowl.ring(s_out, c1), kind: PortalKind::Opening, asset: None, frame: Some(key("frame_stone_1")) });
                from = c1 + back(s_out, c1);
            }
            piece(m, s_out, from, t1, &[]);
        }
        // Стены проходов: от ворот до площади.
        for (t_in, t_out) in [(-g0, -g_out), (g0, g_out), (PI - g0, PI - g_out), (PI + g0, PI + g_out)] {
            m.walls.push(Wall { points: vec![bowl.ring(0.0, t_in), bowl.ring(s_out, t_out)], closed: false, bends: Vec::new(), low: false, style: Some(style.clone()) });
        }
        // Загон: две стены вглубь и задняя по кольцу.
        let depth = bowl.box_depth();
        let pen1 = (PEN / (a + depth)).min(1.0).asin();
        let mut pen = vec![bowl.ring(0.0, FRAC_PI_2 - pen0)];
        pen.extend(ring_line(&bowl, depth, FRAC_PI_2 - pen1, FRAC_PI_2 + pen1, &[], &[]));
        pen.push(bowl.ring(0.0, FRAC_PI_2 + pen0));
        m.walls.push(Wall { points: pen, closed: false, bends: Vec::new(), low: false, style: Some(style.clone()) });
    }
    m.wall_style = style;

    // ----- тени трибун по высотам -----
    let light = m.shadows.light();
    let top_h = height.iter().copied().fold(0.0, f32::max);
    let res = mask::RES as f32;
    // Высота между точками маски — по четырём соседям, иначе край тени ступеньками.
    let sample = |x: f32, y: f32| -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| if i < 0.0 || j < 0.0 || i as usize >= mw || j as usize >= mh { 0.0 } else { height[j as usize * mw + i as usize] };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let low = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + low * fy
    };
    let mut shade = vec![0.0f32; mw * mh];
    for j in 0..mh {
        for i in 0..mw {
            let k = j * mw + i;
            let here = height[k];
            let reach = (top_h - here) / SUN;
            let mut dark = 0.0f32;
            let mut t = 0.0625;
            while t <= reach + 0.3 {
                let hq = sample(i as f32 + light[0] * t * res, j as f32 + light[1] * t * res);
                dark = dark.max(((hq - here - t * SUN) / 0.3).clamp(0.0, 1.0));
                if dark >= 1.0 {
                    break;
                }
                t += 0.0625;
            }
            shade[k] = dark;
        }
    }
    // На ровном (песок, площадь) край тени — от стены за десятки клеток, и сетка
    // высот проступает в нём ступеньками: там он размыт до полутени. Полосы под
    // ступенями трибун остаются чёткими.
    for _ in 0..3 {
        let src = shade.clone();
        for j in 0..mh {
            for i in 0..mw {
                let k = j * mw + i;
                if ring_of[k] > -0.15 && ring_of[k] < outer + 0.2 {
                    continue;
                }
                let (mut sum, mut n) = (0.0, 0.0);
                for (di, dj) in [(0i32, 0i32), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (x, y) = (i as i32 + di, j as i32 + dj);
                    if x >= 0 && y >= 0 && (x as usize) < mw && (y as usize) < mh {
                        sum += src[y as usize * mw + x as usize];
                        n += 1.0;
                    }
                }
                shade[k] = sum / n;
            }
        }
    }
    // Под каждым уступом — мягкая тень со всех сторон: чем выше соседний уступ и
    // чем он ближе, тем темнее; к половине клетки сходит на нет.
    let mut contact = vec![0.0f32; mw * mh];
    for j in 0..mh {
        for i in 0..mw {
            let here = height[j * mw + i];
            let mut dark = 0.0f32;
            for d in 0..16 {
                let a = d as f32 / 16.0 * TAU;
                let (dx, dy) = (a.cos(), a.sin());
                for step in 1..=5 {
                    let r = CONTACT_REACH * step as f32 / 5.0;
                    let hq = sample(i as f32 + dx * r * res, j as f32 + dy * r * res);
                    let rise = ((hq - here) / 0.5).clamp(0.0, 1.0);
                    dark = dark.max(rise * (1.0 - r / (CONTACT_REACH + 0.05)).powf(1.5));
                }
            }
            contact[j * mw + i] = dark;
        }
    }
    let shade: Vec<(usize, f32)> = shade
        .into_iter()
        .zip(contact)
        .map(|(sun, near)| 1.0 - (1.0 - sun * SHADE) * (1.0 - near * CONTACT))
        .enumerate()
        .filter(|(_, v)| *v > 0.005)
        .collect();
    m.shade_paint(&shade, 1.0);
    // Объекты отбрасывают тень под то же солнце: подлиннее обычного.
    m.shadows.dist = 0.3;
    m.shadows.fade = 0.5;

    // ----- что стоит -----
    let mut placed: Vec<([f32; 2], f32)> = Vec::new();
    let free = |p: [f32; 2], r: f32, placed: &mut Vec<([f32; 2], f32)>| {
        if placed.iter().all(|(q, rq)| (q[0] - p[0]).hypot(q[1] - p[1]) >= r + rq) {
            placed.push((p, r));
            true
        } else {
            false
        }
    };
    let have = |slot: &str| !missing(slot);
    // Ложа: трон, кресла по бокам, ковёр, жаровни у края, львы за бортиками.
    if !pit {
        let back = bowl.dir(0.0, -1.0);
        let face = rot_from(back);
        let at = |s: f32, u: f32| {
            let v = -(b + s) * (1.0 - (u / (a + s)).powi(2)).max(0.0).sqrt();
            bowl.world(u, v)
        };
        drop(m, "arena_rug", at(1.9, 0.0), face + 90.0, 1.0);
        drop(m, if have("throne") { "throne" } else { "armchair" }, at(2.3, 0.0), face, 1.0);
        let chair = asset("armchair");
        for u in [-1.4f32, 1.4] {
            drop_pick(m, &chair, at(2.2, u), face, 0.9);
        }
        let brazier = asset("arena_brazier");
        for u in [-(BOX - 0.7), BOX - 0.7] {
            drop_pick(m, &brazier, at(0.85, u), 0.0, 0.6);
        }
        let lion = asset("arena_statue");
        for u in [-(BOX + 1.1), BOX + 1.1] {
            drop_pick(m, &lion, at(2.0, u), face, 0.9);
        }
        placed.push((at(1.8, 0.0), BOX));
        // Жаровни по дорожке подиума — через 7 клеток, кроме вырезов.
        let r0 = (a + b) / 2.0 + 0.8;
        let n = ((TAU * r0 / 7.0).round() as i32).max(4);
        for k in 0..n {
            let t = (k as f32 + 0.25) / n as f32 * TAU;
            if [t - 0.15, t, t + 0.15].iter().any(|&q| bowl.cut(0.8, q)) {
                continue;
            }
            drop_pick(m, &brazier, bowl.ring(0.78, t), 0.0, 0.55);
        }
        // Проходы: стойки с оружием у стен, у выхода на площадь — львы.
        let racks = ["weapon_rack_spears", "weapon_rack", "weapon_rack_axes", "weapon_rack_maces"];
        for side in [1.0f32, -1.0] {
            let u_in = side * a * (1.0 - (GATE / b).powi(2)).max(0.0).sqrt();
            let u_out = side * (a + outer) * (1.0 - (GATE / (b + outer)).powi(2)).max(0.0).sqrt();
            let mid = (u_in + u_out) / 2.0;
            if (u_out - u_in).abs() >= 3.0 {
                for (k, wall_side) in [-1.0f32, 1.0].into_iter().enumerate() {
                    let rack = racks[(rng.int(0, 1) as usize + k * 2) % racks.len()];
                    let pos = bowl.world(mid, wall_side * (GATE - 0.42));
                    drop(m, rack, pos, rot_from(bowl.dir(0.0, wall_side)), 0.8);
                }
            }
            for v in [-(GATE + 1.0), GATE + 1.0] {
                let pos = bowl.world(u_out + side * 1.2, v);
                if pos[0] > 0.6 && pos[1] > 0.6 && pos[0] < w - 0.6 && pos[1] < h - 0.6 {
                    drop_pick(m, &lion, pos, rot_from(bowl.dir(-side, 0.0)), 0.8);
                }
            }
            placed.push((bowl.world(a - 1.0, 0.0), 2.0));
            placed.push((bowl.world(-(a - 1.0), 0.0), 2.0));
        }
        // Загон: клетки у задней стены, кости в углу.
        let depth = bowl.box_depth();
        let pen_mid = |s: f32, u: f32| bowl.world(u, (b + s) * (1.0 - (u / (a + s)).powi(2)).max(0.0).sqrt());
        let cage = if have("arena_cage") { "arena_cage" } else { "crate" };
        drop(m, cage, pen_mid(depth - 1.0, -0.9), rot_from(bowl.dir(0.0, 1.0)), 0.9);
        if have("arena_bones") {
            drop(m, "arena_bones", pen_mid(depth - 1.1, 1.0), rng.int(0, 3) as f32 * 90.0, 0.5);
        } else {
            drop(m, "sacks", pen_mid(depth - 1.0, 1.0), 0.0, 0.8);
        }
        placed.push((pen_mid(0.0, 0.0), 1.5));
    } else {
        placed.push((bowl.world(a - 1.0, 0.0), 1.5));
        placed.push((bowl.world(-(a - 1.0), 0.0), 1.5));
    }

    // Арена: середина, укрытия зеркально, оружие и кровь вразброс.
    let inside_arena = |u: f32, v: f32, margin: f32| (u / (a - margin).max(0.5)).powi(2) + (v / (b - margin).max(0.5)).powi(2) <= 1.0;
    let centre = rng.int(0, 2);
    let mut cover: Vec<(f32, f32)> = Vec::new();
    match centre {
        0 if have("arena_grate") && b >= 4.0 => {
            drop(m, "arena_grate", bowl.world(0.0, 0.0), 0.0, 1.0);
            placed.push((bowl.world(0.0, 0.0), 2.0));
            if have("arena_trapdoor") && a >= 9.0 {
                for (su, sv) in [(1.0f32, 1.0f32), (-1.0, -1.0)] {
                    let pos = bowl.world(su * a * 0.5, sv * b * 0.42);
                    drop(m, "arena_trapdoor", pos, 0.0, 1.0);
                    placed.push((pos, 1.4));
                }
            }
        }
        1 if b >= 4.0 => {
            drop(m, "arena_obelisk", bowl.world(0.0, 0.0), rot_from(bowl.dir(0.0, -1.0)), 1.0);
            placed.push((bowl.world(0.0, 0.0), 1.6));
            if a >= 8.0 {
                for (su, sv) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                    cover.push((su * 3.2, sv * (b * 0.45).min(3.0)));
                }
            }
        }
        _ if a >= 6.0 && b >= 3.5 => {
            for (su, sv) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                cover.push((su * a * 0.33, sv * b * 0.38));
            }
        }
        _ => {}
    }
    for (u, v) in cover {
        let pos = bowl.world(u, v);
        if free(pos, 0.8, &mut placed) {
            drop(m, "pillar", pos, 0.0, 1.0);
        }
    }
    // Рогатки — парами крест-накрест: арену готовили к бою, а не бросили как попало.
    let barricade = if have("arena_barricade") { "arena_barricade" } else { "boulder" };
    let pairs = (1.0 + density * 2.0).round() as i32;
    let mut made = 0;
    for _ in 0..pairs * 30 {
        if made >= pairs || a < 5.0 {
            break;
        }
        let (u, v) = (1.5 + rng.random() as f32 * (a * 0.8 - 1.5), 0.6 + rng.random() as f32 * (b * 0.75 - 0.6));
        if !inside_arena(u, v, 1.6) || (u > a * 0.55 && v < GATE + 1.0) {
            continue;
        }
        let turn = rng.int(0, 3) as f32 * 45.0;
        let spots = [(u, v), (-u, -v)];
        let worlds: Vec<[f32; 2]> = spots.iter().map(|&(x, y)| bowl.world(x, y)).collect();
        if !worlds.iter().all(|&q| placed.iter().all(|(o, r)| (o[0] - q[0]).hypot(o[1] - q[1]) >= r + 1.3)) {
            continue;
        }
        for (k, q) in worlds.into_iter().enumerate() {
            placed.push((q, 1.3));
            drop(m, barricade, q, turn + k as f32 * 180.0, if barricade == "boulder" { 0.9 } else { 1.0 });
        }
        made += 1;
    }
    // Брошенное оружие и кровь: следы прошлых боёв.
    let scatter = |m: &mut Map, rng: &mut Rng, slot: &str, count: i32, scale: f32| {
        if missing(slot) {
            return;
        }
        let mut done = 0;
        for _ in 0..count * 20 {
            if done >= count {
                break;
            }
            let (t, r) = (rng.random() as f32 * TAU, (rng.random() as f32).sqrt() * 0.85);
            let (u, v) = (a * r * t.cos(), b * r * t.sin());
            if !inside_arena(u, v, 0.8) {
                continue;
            }
            drop(m, slot, bowl.world(u, v), rng.random() as f32 * 360.0, scale);
            done += 1;
        }
    };
    let fights = (a * b / 20.0).clamp(2.0, 10.0);
    scatter(m, rng, "arena_blood", (fights * (0.6 + density)) as i32, 0.9);
    scatter(m, rng, "arena_weapon", (fights * (0.4 + density * 0.6)) as i32, 0.75);
    if p.mood == Mood::Abandoned {
        scatter(m, rng, "rubble", fights as i32, 0.8);
    }

    // Площадь: торговцы у арок, телеги, фонари, деревья по углам.
    let plaza = |q: [f32; 2], keep: f32| {
        let (u, v) = bowl.local(q);
        let s = bowl.s_at(u, v);
        q[0] > 1.0 && q[1] > 1.0 && q[0] < w - 1.0 && q[1] < h - 1.0 && s > outer + keep && !(v.abs() < GATE + 1.6 && u.abs() > a)
    };
    let stuff = ["market_stall", "market_stall_blue", "market_stall_ochre", "cart", "barrel", "crate", "sacks", "street_lamp", "street_lamp"];
    let want = ((w * h - PI * (a + outer) * (b + outer)) / 45.0).max(0.0) as i32;
    let mut out = 0;
    for _ in 0..want * 25 {
        if out >= want {
            break;
        }
        let q = [rng.random() as f32 * w, rng.random() as f32 * h];
        let n = pick(rng, &stuff);
        let keep = if n.starts_with("market") || n == "cart" { 2.0 } else { 1.0 };
        if !plaza(q, keep) || !free(q, keep, &mut placed) {
            continue;
        }
        let (u, v) = bowl.local(q);
        drop(m, n, q, rot_from(bowl.dir(u, v)), 0.9);
        out += 1;
    }
    for corner in [[1.6, 1.6], [w - 1.6, 1.6], [1.6, h - 1.6], [w - 1.6, h - 1.6]] {
        if plaza(corner, 1.8) && free(corner, 1.6, &mut placed) && rng.chance(0.75) {
            let tree = season_tree(rng, season);
            drop(m, tree, corner, rng.int(0, 3) as f32 * 90.0, 0.8);
        }
    }
    if !p.objects.is_empty() {
        let mut sp = Spots::new(m);
        let avoid: Vec<Path> = Vec::new();
        around(m, &mut sp, rng, &p.objects, bowl.world(0.0, 0.0), "арена", &avoid, (b * 0.4).max(1.5), warn);
    }
    seats.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Тень по высотам: серп на песке у солнечной стены, полосы под ступенями
    /// на солнечной стороне, на дальней — только мягкая у подножий.
    #[test]
    fn stands_cast_shadows_toward_the_arena_on_the_sun_side() {
        let mut p = Plan::new(Kind::Arena);
        p.seed = "кровь".into();
        let mut m = Map::new(40, 30);
        build(&mut m, &p, 0, crate::palette::Palette::builtin()).unwrap();
        let sh = m.shade.as_ref().expect("тень трибун");
        // Луч от середины к углу: против солнца (левый верх) и по солнцу (правый низ).
        let walk = |dx: f32, dy: f32| -> Vec<u8> {
            (0..64).map(|k| {
                let (x, y) = (20.0 + dx * k as f32 * 0.25, 15.0 + dy * k as f32 * 0.25);
                sh.data[(y * 4.0) as usize * sh.w + (x * 4.0) as usize]
            }).collect()
        };
        let sun = walk(-1.0, -0.75);
        let far = walk(1.0, 0.75);
        let dark = |v: &[u8]| v.iter().filter(|&&x| x > 100).count();
        assert!(dark(&sun[20..34]) >= 4, "нет серпа у солнечной стены: {sun:?}");
        let stripes = sun[34..50].windows(2).filter(|w| w[0] < 60 && w[1] > 100).count();
        assert!(stripes >= 2, "нет полос под ступенями: {sun:?}");
        // На дальней стороне солнце в лицо ступеням: только мягкая тень у подножий.
        assert!(far[20..50].iter().all(|&x| x < 110), "густая тень на солнечной стороне: {far:?}");
        let feet = far[34..50].windows(2).filter(|w| w[0] < 10 && w[1] > 25).count();
        assert!(feet >= 2, "нет тени под ступенями на дальней стороне: {far:?}");
    }

    /// Проёмы чисты: ни стена, ни её продлённый торец не заходят в арку,
    /// ворота и решётку загона.
    #[test]
    fn walls_stop_at_every_opening() {
        for (w, h, seed) in [(40, 30, "кровь"), (70, 50, "лев"), (16, 12, "яма"), (30, 44, "башня")] {
            let mut p = Plan::new(Kind::Arena);
            p.seed = seed.into();
            let mut m = Map::new(w, h);
            build(&mut m, &p, 0, crate::palette::Palette::builtin()).unwrap();
            assert!(m.portals.iter().any(|q| q.kind == PortalKind::Opening) || w < 20, "нет арок {w}×{h}");
            for po in &m.portals {
                for k in 1..10 {
                    let f = k as f32 / 10.0;
                    let q = [po.a[0] + (po.b[0] - po.a[0]) * f, po.a[1] + (po.b[1] - po.a[1]) * f];
                    for wall in &m.walls {
                        let half = wall.style.as_ref().map_or(0.2, |st| st.width / 2.0);
                        let d = wall.distance(q);
                        assert!(d >= half * 0.9, "{w}×{h}: стена в проёме {:?} {:?} у {q:?}: {d}", po.kind, po.a);
                    }
                }
            }
        }
    }
}
