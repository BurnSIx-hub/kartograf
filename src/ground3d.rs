//! Земля этажа для 3D ([`crate::scene3d::Ground`]) из карты: рельеф, помосты с лестницами и
//! пандусами, марши лестниц между этажами.
//!
//! Помост на карте — площадка выше пола со стенкой, чья видимая ширина — высота на глаз
//! ([`crate::platform`]). В 3D он — тело этой высоты ([`RISE`]): помосты один в другом —
//! ярусы, каждый на верху того, что под ним. Лестница помоста — ступени той же глубины, что
//! на карте; верхняя — на ступень ниже площадки. Пандус — та же лестница гладким скатом.
//!
//! Лестница-предмет между этажами (ступени из пака, не стремянка и не люк) — марш до пола
//! этажа выше: подступёнок ниже шага фишки 3D Canvas, верх — к тому концу, где наверху пол.

use crate::map::{Map, Prop};
use crate::platform::{self, Stair, StairGeo, TREAD};
use crate::scene3d::{Block, Cover, Field, Ground, Picture, STOREY, Skin, Top};

/// Видимая ширина стенки помоста → его настоящая высота: ступень на карте — 0,08 клетки
/// стенки, на деле — 0,12 (7 дюймов); помост в пять ступеней — 0,6 клетки (3 фута).
pub const RISE: f32 = 1.5;
/// Сетка рельефа: шаг в клетках и предел точек (у огромной карты шаг крупнее).
const FIELD_STEP: f32 = 0.5;
const FIELD_MAX: f32 = 250_000.0;
/// Подступёнок марша между этажами: ниже шага фишки 3D Canvas (треть клетки).
const FLIGHT_RISE: f32 = 0.3;
/// Бока марша.
const FLIGHT_SIDE: [u8; 3] = [92, 80, 68];
/// Пол наверху ищется на столько за концом марша, клетки.
const FLIGHT_PROBE: f32 = 0.35;

/// Лестница-предмет, по которой ходят между этажами: марш ступеней — не стремянка и не люк.
pub fn flight_by_name(key: &str) -> bool {
    let k = crate::library::label(key).to_lowercase();
    crate::map::passage_by_name(key) && ["stair", "лестниц"].iter().any(|w| k.contains(w)) && !["ladder", "стремян"].iter().any(|w| k.contains(w))
}

/// Та же лестница на этаже выше, над маршем этажа `below` (на карте переход рисуют на обоих
/// этажах): в 3D её место — сам марш снизу, видный в дыру. `skip` — как у [`ground`].
pub fn over_flight(p: &Prop, below: &Map, skip: &dyn Fn(&Prop) -> bool) -> bool {
    flight_by_name(&p.asset) && below.props.iter().any(|q| flight_by_name(&q.asset) && !skip(q) && (q.pos[0] - p.pos[0]).hypot(q.pos[1] - p.pos[1]) < 1.0)
}

/// Земля этажа `map` и номера предметов, ставших маршами. `upper` — где пол у этажа выше
/// (этажа выше нет — маршей нет). `picture(key)` — картинка предмета для верха марша (нет —
/// марш без рисунка, цветом); `skip(prop)` — у предмета своя 3D-модель, марш ему не нужен.
pub fn ground(map: &Map, upper: Option<&Cover>, picture: &dyn Fn(&str) -> Option<Picture>, skip: &dyn Fn(&Prop) -> bool) -> (Ground, Vec<usize>) {
    let mut g = Ground { relief: field(map), blocks: Vec::new() };
    for pl in &map.platforms {
        let poly = platform::tidy(&pl.points);
        if poly.len() < 3 || pl.height <= 0.0 {
            continue;
        }
        // Ярус встаёт на то, что под ним: верх — над самой высокой точкой, бока — до самой низкой.
        let under: Vec<f32> = poly.iter().map(|&p| g.at(p)).collect();
        let lo = under.iter().copied().fold(f32::MAX, f32::min);
        let hi = under.iter().copied().fold(f32::MIN, f32::max);
        let top = hi + pl.height * RISE;
        let mut stairs = Vec::new();
        for st in &pl.stairs {
            if let Some(geo) = platform::stair_geo(&poly, st) {
                let foot = g.at(geo.pt(geo.run, 0.0));
                stairs.extend(stair_blocks(&geo, st, top, foot, pl.side));
            }
        }
        g.blocks.push(Block { poly, base: lo, top: Top::Flat(top), side: pl.side, skin: None });
        g.blocks.extend(stairs);
    }
    let mut flights = Vec::new();
    if let Some(up) = upper {
        for (i, p) in map.props.iter().enumerate() {
            if !flight_by_name(&p.asset) || skip(p) {
                continue;
            }
            if let Some(blocks) = flight(p, &g, up, picture(&p.asset)) {
                g.blocks.extend(blocks);
                flights.push(i);
            }
        }
    }
    (g, flights)
}

/// Рельеф карты сеткой высот; ровный — нет.
fn field(map: &Map) -> Option<Field> {
    let r = map.relief.as_ref().filter(|r| !r.is_flat())?;
    let (w, h) = (map.width as f32, map.height as f32);
    let step = FIELD_STEP.max((w * h / FIELD_MAX).sqrt());
    let (nw, nh) = ((w / step).ceil() as usize + 1, (h / step).ceil() as usize + 1);
    let data = (0..nw * nh).map(|k| r.at([((k % nw) as f32 * step).min(w), ((k / nw) as f32 * step).min(h)])).collect();
    Some(Field { w: nw, h: nh, step, data })
}

/// Лестница помоста: ступени от края наружу (верхняя — на ступень ниже площадки) или пандус.
fn stair_blocks(g: &StairGeo, st: &Stair, top: f32, foot: f32, side: [u8; 3]) -> Vec<Block> {
    if st.ramp {
        let k = (top - foot) / g.run.max(1e-3);
        return vec![Block { poly: g.outline().to_vec(), base: foot, top: Top::Slope { at: g.o, h: top, rise: [-g.nrm[0] * k, -g.nrm[1] * k] }, side, skin: None }];
    }
    let n = st.steps.clamp(1, 16) as usize;
    (0..n)
        .map(|k| {
            let (s0, s1) = (k as f32 * TREAD, (k + 1) as f32 * TREAD);
            let h = foot + (top - foot) * (n - k) as f32 / (n + 1) as f32;
            Block { poly: vec![g.pt(s0, -1.0), g.pt(s0, 1.0), g.pt(s1, 1.0), g.pt(s1, -1.0)], base: foot, top: Top::Flat(h), side, skin: None }
        })
        .collect()
}

/// Марш лестницы-предмета до пола этажа выше: вдоль длинной стороны картинки, наверх — к концу,
/// за которым наверху пол (не понять — к верху картинки). Ступени несут рисунок предмета.
fn flight(p: &Prop, g: &Ground, up: &Cover, picture: Option<Picture>) -> Option<Vec<Block>> {
    let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
    if w.min(h) < 0.1 {
        return None;
    }
    let (s, c) = p.rot.to_radians().sin_cos();
    // Оси картинки на карте: x — вправо по ней, y — вниз (как у `map::prop_rect`).
    let (ux, uy) = ([c, s], [-s, c]);
    let (along, len, wide, across) = if h >= w { (uy, h, w, ux) } else { (ux, w, h, uy) };
    let end = |sgn: f32| [p.pos[0] + along[0] * sgn * (len / 2.0 + FLIGHT_PROBE), p.pos[1] + along[1] * sgn * (len / 2.0 + FLIGHT_PROBE)];
    // Куда подъём: к концу с полом наверху; у картинки «вдоль x» без подсказки — вправо.
    let dir = match (up.has(end(-1.0)), up.has(end(1.0))) {
        (false, true) => 1.0,
        (true, false) => -1.0,
        _ if along == ux => 1.0,
        _ => -1.0,
    };
    let fwd = [along[0] * dir, along[1] * dir];
    let bottom = [p.pos[0] - fwd[0] * len / 2.0, p.pos[1] - fwd[1] * len / 2.0];
    let base = g.at(bottom);
    let n = (STOREY / FLIGHT_RISE).ceil() as usize;
    let tread = len / n as f32;
    let skin = picture.map(|picture| {
        let corner = [p.pos[0] - ux[0] * w / 2.0 - uy[0] * h / 2.0, p.pos[1] - ux[1] * w / 2.0 - uy[1] * h / 2.0];
        let (origin, u) = if p.flip { ([corner[0] + ux[0] * w, corner[1] + ux[1] * w], [-ux[0] * w, -ux[1] * w]) } else { (corner, [ux[0] * w, ux[1] * w]) };
        Skin { picture, origin, u, v: [uy[0] * h, uy[1] * h] }
    });
    let pt = |t: f32, x: f32| [bottom[0] + fwd[0] * t + across[0] * x, bottom[1] + fwd[1] * t + across[1] * x];
    let hw = wide / 2.0;
    Some(
        (0..n)
            .map(|i| {
                let (t0, t1) = (i as f32 * tread, (i + 1) as f32 * tread);
                Block { poly: vec![pt(t0, -hw), pt(t0, hw), pt(t1, hw), pt(t1, -hw)], base, top: Top::Flat(base.max(STOREY * (i + 1) as f32 / n as f32)), side: FLIGHT_SIDE, skin: skin.clone() }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Platform;
    use crate::scene3d::STEP;

    fn map(w: u32, h: u32) -> Map {
        let mut m = Map::new(w, h);
        m.props.clear();
        m
    }

    fn none(_: &str) -> Option<Picture> {
        None
    }

    fn no(_: &Prop) -> bool {
        false
    }

    #[test]
    fn a_dais_stands_its_height_and_its_steps_climb_a_step_at_a_time() {
        let mut m = map(12, 12);
        let mut pl = Platform::new(vec![[3.0, 3.0], [9.0, 3.0], [9.0, 7.0], [3.0, 7.0]], 0.4);
        pl.stairs.push(Stair { at: [6.0, 7.0], width: 2.0, steps: 4, ramp: false });
        m.platforms.push(pl);
        let (g, _) = ground(&m, None, &none, &no);
        let top = 0.4 * RISE;
        assert!((g.at([6.0, 5.0]) - top).abs() < 1e-5);
        assert_eq!(g.at([1.0, 1.0]), 0.0);
        // Помост выше шага: с пола на него не шагнуть.
        assert!(g.step([6.0, 5.0], 0.0).is_none());
        // По ступеням снизу вверх: каждая не выше шага.
        let mut feet = 0.0;
        let mut y = 7.0 + 4.0 * TREAD + 0.2;
        while y > 6.5 {
            y -= 0.1;
            feet = g.step([6.0, y], feet).expect("ступень не выше шага");
        }
        assert!((feet - top).abs() < 1e-5, "поднялись на помост: {feet}");
    }

    #[test]
    fn a_tier_inside_a_tier_stands_on_it() {
        let mut m = map(12, 12);
        m.platforms.push(Platform::new(vec![[2.0, 2.0], [10.0, 2.0], [10.0, 10.0], [2.0, 10.0]], 0.08));
        m.platforms.push(Platform::new(vec![[4.0, 4.0], [8.0, 4.0], [8.0, 8.0], [4.0, 8.0]], 0.08));
        let (g, _) = ground(&m, None, &none, &no);
        assert!((g.at([6.0, 6.0]) - 2.0 * 0.08 * RISE).abs() < 1e-5);
        assert!((g.blocks[1].base - 0.08 * RISE).abs() < 1e-5, "бока второго — до верха первого");
        // Ярусы по ступени — шагом.
        assert!(g.step([6.0, 6.0], 0.08 * RISE).is_some());
    }

    #[test]
    fn a_ramp_is_one_smooth_slope_from_the_edge_to_the_floor() {
        let mut m = map(12, 12);
        let mut pl = Platform::new(vec![[3.0, 3.0], [9.0, 3.0], [9.0, 7.0], [3.0, 7.0]], 0.4);
        pl.stairs.push(Stair { at: [6.0, 7.0], width: 2.0, steps: 3, ramp: true });
        m.platforms.push(pl);
        let (g, _) = ground(&m, None, &none, &no);
        assert_eq!(g.blocks.len(), 2);
        let run = 3.0 * TREAD;
        let at = |s: f32| g.at([6.0, 7.0 + s]);
        assert!((at(0.01) - 0.6).abs() < 0.02 && (at(run / 2.0) - 0.3).abs() < 0.02 && at(run - 0.01) < 0.02, "{} {} {}", at(0.01), at(run / 2.0), at(run - 0.01));
    }

    #[test]
    fn a_flight_climbs_to_the_floor_above_towards_the_end_with_floor_up_there() {
        let mut m = map(10, 10);
        m.props.push(Prop { asset: "Stairs_Stone_A_2x4".into(), pos: [5.0, 5.0], size: [2.0, 4.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false });
        // Наверху пол только к югу от марша (y > 7): подъём — на юг, хотя картинка смотрит на север.
        let cover = Cover { w: 10, h: 10, per: 1.0, data: (0..100).map(|k| k / 10 >= 7).collect() };
        let (g, flights) = ground(&m, Some(&cover), &none, &no);
        assert_eq!(flights, vec![0]);
        assert!(g.at([5.0, 6.9]) > STOREY - 0.31 && g.at([5.0, 3.1]) < 0.31, "{} {}", g.at([5.0, 6.9]), g.at([5.0, 3.1]));
        // Снизу до верха — шагами.
        let mut feet = 0.0;
        let mut y = 2.8;
        while y < 6.95 {
            y += 0.1;
            feet = g.step([5.0, y], feet).expect("ступень марша не выше шага");
        }
        assert!((feet - STOREY).abs() < 1e-4 && STOREY / 7.0 < STEP);
        // Без этажа выше марша нет.
        assert!(ground(&m, None, &none, &no).1.is_empty());
    }

    #[test]
    fn a_ladder_or_a_hatch_is_not_a_flight() {
        assert!(flight_by_name("x/Stairs_Wood_A_2x3.webp") && flight_by_name("Лестница каменная"));
        assert!(!flight_by_name("x/Ladder_Wood_1x3.webp") && !flight_by_name("x/Floor_Hatch_A_1x1.webp") && !flight_by_name("builtin:stairs_shade"));
    }
}
