//! Кровати 3D — модели Blender (`tools/blender/beds.py` → `assets/beds3d`) к рисункам кроватей FA
//! по имени: кровать (одинарная, двойная), гамак на распорках и без, скатка расстеленная и
//! свёрнутая, спальный мешок, матрас, подстилка из сена, детская кроватка, носилки.
//!
//! Модель встаёт туда, где на рисунке нарисована сама кровать ([`Spot`]):
//! - кровать — по рамке ([`frame_rect`]): у собранных рисунков FA (`Arranged_Bed_…`) вокруг неё
//!   лежат вещи — сапоги, сундук, одежда; они остаются рисунком на полу вокруг модели;
//! - гамак, скатка, спальник, матрас — по главной оси нарисованного ([`spot_axis`]): гамак FA
//!   висит наискосок, матрас бывает повёрнут;
//! - сено, детская кроватка, носилки — во всё нарисованное; под сеном — ещё и сам рисунок.
//!
//! Ткань модели (одеяло, подушки, матрас, полотно, сено) запечена нейтрально-серой: на неё сверху
//! ложится сам рисунок ([`paint`]) — цвет и узор одеяла, мех, вещи на постели те же, что на карте,
//! а складки, плетение и затенение — от модели. Свесы и бока берут цвет чуть внутрь от края: у
//! края нарисованного — тёмная обводка. Дерево перекрашивается в породу из имени, как створки
//! дверей; верёвки, ремни и железо — как есть.

use crate::model3d::{self, Model3d};
use crate::scene3d::{Alpha, Look, Maps, Material, Mesh, Node, Part, Picture, Scene3d};
use image::RgbaImage;
use std::sync::OnceLock;

/// Версия моделей, укладки и раскраски: меняется — кровати пишутся заново.
pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Single,
    Double,
    Hammock,
    Gathered,
    Bedroll,
    Roll,
    SleepingBag,
    Mattress,
    Hay,
    Cot,
    Stretcher,
}

#[cfg(test)]
const KINDS: [Kind; 11] = [Kind::Single, Kind::Double, Kind::Hammock, Kind::Gathered, Kind::Bedroll, Kind::Roll, Kind::SleepingBag, Kind::Mattress, Kind::Hay, Kind::Cot, Kind::Stretcher];

impl Kind {
    /// Имя вида — как у модели в `assets/beds3d/bed_<имя>.glb`.
    #[cfg(test)]
    fn name(self) -> &'static str {
        match self {
            Kind::Single => "single",
            Kind::Double => "double",
            Kind::Hammock => "hammock",
            Kind::Gathered => "gathered",
            Kind::Bedroll => "bedroll",
            Kind::Roll => "roll",
            Kind::SleepingBag => "sleeping_bag",
            Kind::Mattress => "mattress",
            Kind::Hay => "hay",
            Kind::Cot => "cot",
            Kind::Stretcher => "stretcher",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Kind::Single => include_bytes!("../assets/beds3d/bed_single.glb"),
            Kind::Double => include_bytes!("../assets/beds3d/bed_double.glb"),
            Kind::Hammock => include_bytes!("../assets/beds3d/bed_hammock.glb"),
            Kind::Gathered => include_bytes!("../assets/beds3d/bed_gathered.glb"),
            Kind::Bedroll => include_bytes!("../assets/beds3d/bed_bedroll.glb"),
            Kind::Roll => include_bytes!("../assets/beds3d/bed_roll.glb"),
            Kind::SleepingBag => include_bytes!("../assets/beds3d/bed_sleeping_bag.glb"),
            Kind::Mattress => include_bytes!("../assets/beds3d/bed_mattress.glb"),
            Kind::Hay => include_bytes!("../assets/beds3d/bed_hay.glb"),
            Kind::Cot => include_bytes!("../assets/beds3d/bed_cot.glb"),
            Kind::Stretcher => include_bytes!("../assets/beds3d/bed_stretcher.glb"),
        }
    }

    /// Длинная сторона модели — вдоль x (гамаки, свёрнутая скатка); у прочих — вдоль z,
    /// изголовье к −z (к верху рисунка).
    fn along_x(self) -> bool {
        matches!(self, Kind::Hammock | Kind::Gathered | Kind::Roll)
    }
}

/// Как модель ложится в рисунок.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// По рамке кровати среди вещей ([`frame_rect`]); одинарная или двойная — по её ширине.
    Frame,
    /// По главной оси нарисованного ([`spot_axis`]).
    Axis,
    /// Во всё нарисованное, без поворота.
    Whole,
}

/// Имя рисунка строчными без расширения: `…/Hammock_Brown_A_2x2.webp` → `hammock_brown_a_2x2`.
fn stem(key: &str) -> String {
    let path = crate::fa3d::inner_path(key);
    let name = path.rsplit('/').next().unwrap_or(&path);
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_owned()
}

/// Модель и укладка по ключу рисунка; не кровать (или сломанная, упавшая, одно одеяло без
/// кровати) — `None`. У `Fit::Frame` вид — «кровать»: одинарная или двойная выйдет по рамке.
pub fn kind_for(key: &str) -> Option<(Kind, Fit)> {
    let name = stem(key);
    let path = crate::fa3d::inner_path(key);
    let has = |w: &str| name.contains(w);
    let any = |ws: &[&str]| ws.iter().any(|w| has(w));
    let starts = |ws: &[&str]| ws.iter().any(|w| name.starts_with(w));
    if any(&["broken", "fallen"]) || path.contains("/broken/") {
        return None;
    }
    let cells = crate::library::size_from_name(&name);
    let small = cells.is_some_and(|c| c[0] <= 1.0 && c[1] <= 1.0);
    let out = if starts(&["arranged_bed_wood", "single_bed_", "double_bed_"]) && !any(&["blanket", "frame", "headboard", "ladder", "mattress"]) {
        (Kind::Single, Fit::Frame)
    } else if starts(&["hammock_"]) {
        (if has("_b_") { Kind::Gathered } else { Kind::Hammock }, Fit::Axis)
    } else if starts(&["rolled_sleeping_bag"]) {
        (Kind::Roll, Fit::Axis)
    } else if starts(&["sleeping_bag"]) {
        (Kind::SleepingBag, Fit::Axis)
    } else if starts(&["bedroll_", "blanket_bedroll"]) {
        (if small { Kind::Roll } else { Kind::Bedroll }, Fit::Axis)
    } else if starts(&["bed_mattress", "single_bed_mattress", "metal_bed_mattress"]) {
        (Kind::Mattress, Fit::Axis)
    } else if starts(&["hay_bedding"]) {
        (Kind::Hay, Fit::Whole)
    } else if starts(&["cot_wood"]) && !has("mobile") {
        (Kind::Cot, Fit::Whole)
    } else if starts(&["stretcher_"]) {
        (Kind::Stretcher, Fit::Whole)
    } else {
        return None;
    };
    Some(out)
}

/// Породы дерева в именах рисунков FA и средний тон их дерева на рисунках кроватей (брусья
/// изголовья и изножья): FA красит его темнее, чем на рисунках дверей (`doors3d`) — ясень на
/// кровати серо-бурый, а не белёсый. Порода не названа — [`WOOD`].
const WOODS: [(&str, [u8; 3]); 5] = [("ashen", [98, 78, 54]), ("light", [133, 91, 45]), ("walnut", [70, 49, 27]), ("red", [90, 38, 9]), ("dark", [108, 64, 26])];
const WOOD: [u8; 3] = [104, 68, 36];

/// Средний тон дерева по имени рисунка: порода после `wood_`, иначе последняя названная
/// (`Stretcher_Red_Dark` — ткань красная, жерди тёмные).
pub fn wood_tint(key: &str) -> [u8; 3] {
    let name = stem(key);
    if let Some(i) = name.find("wood_")
        && let Some((_, c)) = WOODS.iter().find(|(w, _)| name[i + 5..].starts_with(w))
    {
        return *c;
    }
    WOODS.iter().filter_map(|(w, c)| name.rfind(w).map(|i| (i, *c))).max_by_key(|(i, _)| *i).map_or(WOOD, |(_, c)| c)
}

/// Средний тон дерева кровати на самом рисунке — по брусьям изголовья и изножья рамки `r`
/// (середина по каждому каналу: горшки на изголовье и свисающее одеяло — меньшинство). Брусьев
/// не видно — `None`.
fn frame_wood(img: &RgbaImage, r: [f32; 4]) -> Option<[u8; 3]> {
    let (l, t, rr, b) = (r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32);
    let mut px: Vec<[u8; 3]> = Vec::new();
    for (y0, y1) in [(t + 6, t + 20), (b.saturating_sub(18), b.saturating_sub(6))] {
        for y in y0..y1.min(img.height()) {
            for x in (l + 14)..rr.saturating_sub(14).min(img.width()) {
                let p = img.get_pixel(x, y);
                if p[3] > 200 {
                    px.push([p[0], p[1], p[2]]);
                }
            }
        }
    }
    if px.len() < 50 {
        return None;
    }
    Some([0, 1, 2].map(|c| {
        let mut v: Vec<u8> = px.iter().map(|p| p[c]).collect();
        v.sort_unstable();
        v[v.len() / 2]
    }))
}

// --- Где на рисунке кровать -------------------------------------------------------------------

/// Где на рисунке модель (точки рисунка, y — вниз): середина, ось (единичная; знак — любой),
/// длина вдоль оси и ширина поперёк.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub c: [f32; 2],
    pub d: [f32; 2],
    pub len: f32,
    pub wid: f32,
}

impl Spot {
    /// Прямоугольник `[x0, y0, x1, y1]`, ось — вниз.
    fn rect([x0, y0, x1, y1]: [f32; 4]) -> Spot {
        Spot { c: [(x0 + x1) / 2.0, (y0 + y1) / 2.0], d: [0.0, 1.0], len: y1 - y0, wid: x1 - x0 }
    }
}

/// Высота рамки кровати FA (от верха изголовья до низа изножья), клеток: у всех рамок FA одна.
const TALL: f32 = 413.0 / 256.0;

/// Рамка кровати на рисунке, `[x0, y0, x1, y1]` в точках (`ppc` — точек на клетку). Царги — две
/// самые длинные тёмные вертикальные линии (обводка FA) на расстоянии в ширину кровати, изголовье
/// и изножье — горизонтальные линии во всю ширину между ними. Вещи вокруг (сапоги, сундук, доска,
/// одежда на краю) рамкой не считаются: они короче или не на том расстоянии. Изголовье — первая
/// такая линия сверху (горшки на нём — не во всю ширину), изножье — ближайшая к высоте рамки FA
/// (одеяло, свисающее ниже, — нет). Нет рамки — `None`.
pub fn frame_rect(img: &RgbaImage, ppc: f32) -> Option<[f32; 4]> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w < 16 || h < 16 {
        return None;
    }
    // Яркость поверх серого (прозрачное — светлее обводки).
    let l: Vec<f32> = img
        .pixels()
        .map(|p| {
            let a = p[3] as f32 / 255.0;
            (0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) / 255.0 * a + 0.62 * (1.0 - a)
        })
        .collect();
    let at = |x: isize, y: isize| l[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize];
    // Тёмная линия — темнее соседей по обе стороны (на 3–4 точки).
    let (mut v, mut hz) = (vec![false; w * h], vec![false; w * h]);
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let c = at(xi, yi);
            v[y * w + x] = c < at(xi - 3, yi).max(at(xi - 4, yi)).min(at(xi + 3, yi).max(at(xi + 4, yi))) - 0.07;
            hz[y * w + x] = c < at(xi, yi - 3).max(at(xi, yi - 4)).min(at(xi, yi + 3).max(at(xi, yi + 4))) - 0.07;
        }
    }
    // Шире на точку: вертикальные — вбок, горизонтальные — вверх и вниз.
    let v3: Vec<bool> = (0..w * h).map(|i| v[i] || (i % w > 0 && v[i - 1]) || (i % w + 1 < w && v[i + 1])).collect();
    let h3: Vec<bool> = (0..w * h).map(|i| hz[i] || (i >= w && hz[i - w]) || (i + w < w * h && hz[i + w])).collect();
    let mut sums = vec![0u32; h * (w + 1)];
    for y in 0..h {
        for x in 0..w {
            sums[y * (w + 1) + x + 1] = sums[y * (w + 1) + x] + h3[y * w + x] as u32;
        }
    }
    // Доля горизонтальной линии в строке `y` между царгами `l` и `r`.
    let cover = |y: usize, l: usize, r: usize| -> f32 {
        let (a, b) = (l + 6, r.saturating_sub(5));
        if b <= a { 0.0 } else { (sums[y * (w + 1) + b] - sums[y * (w + 1) + a]) as f32 / (b - a) as f32 }
    };
    let col: Vec<usize> = (0..w).map(|x| (0..h).filter(|&y| v3[y * w + x]).count()).collect();
    let mut order: Vec<usize> = (0..w).collect();
    order.sort_by(|a, b| col[*b].cmp(&col[*a]));
    let mut peaks: Vec<usize> = Vec::new();
    for x in order {
        if (col[x] as f32) < 0.25 * h as f32 || peaks.len() >= 14 {
            break;
        }
        if peaks.iter().all(|p| p.abs_diff(x) > 6) {
            peaks.push(x);
        }
    }
    // Самый длинный отрезок линии в столбце (разрывы до 30 точек): начало, конец, точек.
    let run = |x: usize| {
        let (mut best, mut s, mut n, mut prev) = ((0, 0, 0), 0, 0, None::<usize>);
        for y in (0..h).filter(|&y| v3[y * w + x]) {
            if prev.is_none_or(|p| y - p > 30) {
                s = y;
                n = 0;
            }
            n += 1;
            prev = Some(y);
            if n > best.2 {
                best = (s, y, n);
            }
        }
        best
    };
    let mut best: Option<(usize, usize, usize, Run, Run)> = None;
    for &l in &peaks {
        for &r in &peaks {
            if r <= l || !(0.55..=1.95).contains(&((r - l) as f32 / ppc)) {
                continue;
            }
            let (gl, gr) = (run(l), run(r));
            let bars = (0..h).any(|y| cover(y, l, r) > 0.75);
            let score = gl.2 + gr.2 + if bars { 60 } else { 0 };
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, l, r, gl, gr));
            }
        }
    }
    let (_, l, r, gl, gr) = best?;
    let tall = TALL * ppc;
    let top = (0..h / 2).find(|&y| cover(y, l, r) >= 0.75).unwrap_or(gl.0.min(gr.0));
    let want = top as f32 + tall;
    let near = (0..h).filter(|&y| cover(y, l, r) >= 0.6 && (y as f32 - want).abs() <= 0.12 * tall).min_by(|a, b| (*a as f32 - want).abs().total_cmp(&(*b as f32 - want).abs()));
    let bottom = match near {
        Some(mut y) => {
            while y + 1 < h && cover(y + 1, l, r) >= 0.6 {
                y += 1;
            }
            y as f32 + 1.0
        }
        None => want.min(h as f32),
    };
    let tc = (bottom - top as f32) / ppc;
    (1.2..=2.1).contains(&tc).then_some([l as f32, top as f32, r as f32 + 1.0, bottom])
}

/// Отрезок линии в столбце: первая и последняя строка, точек в нём.
type Run = (usize, usize, usize);

/// Нарисованное (непрозрачное) — прямоугольник в точках.
fn opaque_rect(img: &RgbaImage) -> Option<[f32; 4]> {
    let b = crate::fa3d::opaque_box(img)?;
    let (w, h) = (img.width() as f32, img.height() as f32);
    Some([b[0] * w, b[1] * h, b[2] * w, b[3] * h])
}

/// Нарисованное по главной оси: середина, ось, длина и ширина (точки). Почти вдоль края
/// рисунка (до 4°) — ровно вдоль него.
pub fn spot_axis(img: &RgbaImage) -> Option<Spot> {
    let pts: Vec<[f32; 2]> = img.enumerate_pixels().filter(|(_, _, p)| p[3] > 100).map(|(x, y, _)| [x as f32 + 0.5, y as f32 + 0.5]).collect();
    if pts.len() < 16 {
        return None;
    }
    let n = pts.len() as f32;
    let m = [pts.iter().map(|p| p[0]).sum::<f32>() / n, pts.iter().map(|p| p[1]).sum::<f32>() / n];
    let (mut a, mut b, mut c) = (0.0f32, 0.0f32, 0.0f32);
    for p in &pts {
        let (x, y) = (p[0] - m[0], p[1] - m[1]);
        a += x * x;
        b += x * y;
        c += y * y;
    }
    let mut angle = 0.5 * (2.0 * b).atan2(a - c);
    let quarter = std::f32::consts::FRAC_PI_2;
    let snap = (angle / quarter).round() * quarter;
    if (angle - snap).abs() < 4f32.to_radians() {
        angle = snap;
    }
    let d = [angle.cos(), angle.sin()];
    let e = [-d[1], d[0]];
    let (mut t0, mut t1, mut s0, mut s1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in &pts {
        let (x, y) = (p[0] - m[0], p[1] - m[1]);
        let (t, s) = (x * d[0] + y * d[1], x * e[0] + y * e[1]);
        (t0, t1, s0, s1) = (t0.min(t), t1.max(t), s0.min(s), s1.max(s));
    }
    let (tc, sc) = ((t0 + t1) / 2.0, (s0 + s1) / 2.0);
    Some(Spot { c: [m[0] + d[0] * tc + e[0] * sc, m[1] + d[1] * tc + e[1] * sc], d, len: t1 - t0 + 1.0, wid: s1 - s0 + 1.0 })
}

/// Матрица glTF (по столбцам) из модели с рамкой `bounds` в пространство предмета рисунка
/// размером `frame` клеток и `px` точек: длинная сторона модели (`along_x` — x, иначе z,
/// изголовье к −z) — по оси `spot`, к низу рисунка (у вдоль x — вправо); по сторонам — в размер
/// `spot`, по высоте — меньше, если крупнее настоящей (как предметы FA, [`crate::fa3d::place`]).
pub fn placement(bounds: [[f32; 3]; 2], along_x: bool, spot: Spot, frame: [f32; 2], px: [u32; 2]) -> [f32; 16] {
    let [lo, hi] = bounds;
    let ppc = px[0] as f32 / frame[0].max(1e-3);
    let mut d = spot.d;
    let flip = if along_x { d[0] < -1e-4 || (d[0].abs() <= 1e-4 && d[1] < 0.0) } else { d[1] < -1e-4 || (d[1].abs() <= 1e-4 && d[0] < 0.0) };
    if flip {
        d = [-d[0], -d[1]];
    }
    let (lm, wm) = if along_x { (hi[0] - lo[0], hi[2] - lo[2]) } else { (hi[2] - lo[2], hi[0] - lo[0]) };
    let (sl, sa) = (spot.len / ppc / lm.max(1e-3), spot.wid / ppc / wm.max(1e-3));
    let g = (sl * sa).sqrt();
    let sy = if g > 1.0 { g.sqrt() } else { g };
    let s = if along_x { [sl, sy, sa] } else { [sa, sy, sl] };
    let th = if along_x { (-d[1]).atan2(d[0]) } else { d[0].atan2(d[1]) };
    let (sn, cs) = th.sin_cos();
    let r = [[cs, 0.0, sn], [0.0, 1.0, 0.0], [-sn, 0.0, cs]];
    let a: [[f32; 3]; 3] = [0, 1, 2].map(|i| [0, 1, 2].map(|j| r[i][j] * s[j]));
    // По высоте — от пола модели (у всех он на нуле; гамак висит над ним).
    let mc = [(lo[0] + hi[0]) / 2.0, 0.0, (lo[2] + hi[2]) / 2.0];
    let ic = [(spot.c[0] / px[0] as f32 - 0.5) * frame[0], 0.0, (spot.c[1] / px[1] as f32 - 0.5) * frame[1]];
    let t: [f32; 3] = [0, 1, 2].map(|i| ic[i] - (0..3).map(|j| a[i][j] * mc[j]).sum::<f32>());
    [a[0][0], a[1][0], a[2][0], 0.0, a[0][1], a[1][1], a[2][1], 0.0, a[0][2], a[1][2], a[2][2], 0.0, t[0], t[1], t[2], 1.0]
}

// --- Модель и её раскраска ----------------------------------------------------------------------

/// Что на детали: ткань (красит рисунок), дерево (порода из имени), прочее (как есть).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Cloth = 1,
    Wood = 2,
    Keep = 3,
}

/// Модель из Blender: куски по классам (материалы `cloth`, `wood`, `keep` с одними картами),
/// карты, рамка.
struct Bed {
    model: Model3d,
    classes: Vec<Class>,
    normal: RgbaImage,
    orm: RgbaImage,
    bounds: [[f32; 3]; 2],
}

/// Имена материалов кусков `.glb` (одна сетка) по порядку.
fn part_materials(bytes: &[u8]) -> Option<Vec<String>> {
    let (json, _) = model3d::split_glb(bytes).ok()?;
    let j: serde_json::Value = serde_json::from_slice(json).ok()?;
    let meshes = j["meshes"].as_array()?;
    if meshes.len() != 1 {
        return None;
    }
    meshes[0]["primitives"].as_array()?.iter().map(|p| j["materials"][p["material"].as_u64()? as usize]["name"].as_str().map(str::to_owned)).collect()
}

fn read(bytes: &[u8]) -> Option<Bed> {
    let model = model3d::read_gltf(bytes, None, 1.0).ok()?;
    let (normal, orm) = model3d::material_maps(bytes).ok()?;
    let names = part_materials(bytes)?;
    if names.len() != model.parts.len() || model.images.first()?.dimensions() != orm.dimensions() {
        return None;
    }
    let classes = names
        .iter()
        .map(|n| match n.as_str() {
            n if n.starts_with("cloth") => Class::Cloth,
            n if n.starts_with("wood") => Class::Wood,
            _ => Class::Keep,
        })
        .collect();
    let bounds = model.bounds()?;
    Some(Bed { model, classes, normal, orm, bounds })
}

/// Модель вида (один раз на вид).
fn bed(kind: Kind) -> Option<&'static Bed> {
    static BEDS: [OnceLock<Option<Bed>>; 11] = [const { OnceLock::new() }; 11];
    BEDS[kind as usize].get_or_init(|| read(kind.bytes())).as_ref()
}

/// Точка развёртки на ткани: номер, место на модели, нормаль.
type ClothTexel = (usize, [f32; 3], [f32; 3]);

/// Точки развёртки: класс детали на каждой (0 — пусто) и у ткани — место на модели и нормаль.
/// Треугольники кладутся с запасом в полторы точки (по краям островов — поле запекания), ближняя
/// деталь побеждает.
fn texels(bed: &Bed, w: usize, h: usize) -> (Vec<u8>, Vec<ClothTexel>) {
    let mut cls = vec![0u8; w * h];
    let mut dist = vec![f32::MAX; w * h];
    let mut at: Vec<Option<([f32; 3], [f32; 3])>> = vec![None; w * h];
    for (part, &class) in bed.model.parts.iter().zip(&bed.classes) {
        let v = &part.mesh.v;
        for t in part.mesh.idx.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| v[i as usize]);
            let [pa, pb, pc] = [a, b, c].map(|q| [q.uv[0] * w as f32, q.uv[1] * h as f32]);
            let cross = |o: [f32; 2], p: [f32; 2], q: [f32; 2]| (p[0] - o[0]) * (q[1] - o[1]) - (p[1] - o[1]) * (q[0] - o[0]);
            let area = cross(pa, pb, pc);
            if area.abs() < 1e-6 {
                continue;
            }
            let len = |p: [f32; 2], q: [f32; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt().max(1e-6);
            let edges = [len(pb, pc), len(pc, pa), len(pa, pb)];
            let x0 = (pa[0].min(pb[0]).min(pc[0]) - 2.0).floor().max(0.0) as usize;
            let x1 = ((pa[0].max(pb[0]).max(pc[0]) + 2.0).ceil() as usize).min(w);
            let y0 = (pa[1].min(pb[1]).min(pc[1]) - 2.0).floor().max(0.0) as usize;
            let y1 = ((pa[1].max(pb[1]).max(pc[1]) + 2.0).ceil() as usize).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = [x as f32 + 0.5, y as f32 + 0.5];
                    let l = [cross(pb, pc, p) / area, cross(pc, pa, p) / area, cross(pa, pb, p) / area];
                    // Насколько точка вне треугольника (точки развёртки).
                    let out = (0..3).filter(|&k| l[k] < 0.0).map(|k| -l[k] * area.abs() / edges[k]).fold(0.0f32, f32::max);
                    let i = y * w + x;
                    if out > 1.5 || out >= dist[i] {
                        continue;
                    }
                    dist[i] = out;
                    cls[i] = class as u8;
                    if class == Class::Cloth {
                        let k = l.map(|q| q.max(0.0));
                        let sum = (k[0] + k[1] + k[2]).max(1e-6);
                        let mix = |f: &dyn Fn(&crate::scene3d::Vertex) -> [f32; 3]| [0, 1, 2].map(|q| (k[0] * f(&a)[q] + k[1] * f(&b)[q] + k[2] * f(&c)[q]) / sum);
                        at[i] = Some((mix(&|v| v.p), mix(&|v| v.n)));
                    }
                }
            }
        }
    }
    let cloth = at.into_iter().enumerate().filter(|(i, _)| cls[*i] == Class::Cloth as u8).filter_map(|(i, a)| a.map(|(p, n)| (i, p, n))).collect();
    (cls, cloth)
}

/// Насколько внутрь от края нарисованного берёт цвет отвесный бок ткани, клеток.
const INSET: f32 = 0.05;

/// Цвет рисунка в точке (дробные точки, y — вниз), с весами соседних точек.
fn sample(img: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let mut out = [0.0f32; 4];
    for (dx, dy, k) in [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)] {
        let p = img.get_pixel((x0 + dx).clamp(0, w - 1) as u32, (y0 + dy).clamp(0, h - 1) as u32);
        for c in 0..4 {
            out[c] += p[c] as f32 * k;
        }
    }
    out
}

/// Картинка цвета модели, уложенной матрицей `m` в рисунок `img` (рамка `frame` клеток, кровать —
/// в `spot`): ткань — цвет рисунка над ней с тонами запечённой ткани (средний тон ткани — цвет
/// рисунка как есть), дерево — средним тоном `wood` с сохранением разброса тонов (волокна,
/// стёртые рёбра, грязь), прочее — как есть.
fn paint(bed: &Bed, m: &[f32; 16], img: &RgbaImage, frame: [f32; 2], spot: Spot, wood: [u8; 3]) -> Option<RgbaImage> {
    let mut base = bed.model.images.first()?.clone();
    let (w, h) = (base.width() as usize, base.height() as usize);
    let (cls, cloth) = texels(bed, w, h);
    let mean = |class: Class| {
        let (mut sum, mut n) = ([0.0f64; 3], 0usize);
        for (i, p) in base.pixels().enumerate() {
            if cls[i] == class as u8 {
                (0..3).for_each(|c| sum[c] += p[c] as f64);
                n += 1;
            }
        }
        sum.map(|s| (s / n.max(1) as f64).max(1.0) as f32)
    };
    let (mc, mw) = (mean(Class::Cloth), mean(Class::Wood));
    // Средний цвет нарисованного у кровати: если над точкой ткани пусто и рядом тоже.
    let (pw, ph) = (img.width() as f32, img.height() as f32);
    let reach = spot.len.max(spot.wid) / 2.0;
    let (mut avg, mut n) = ([0.0f64; 3], 0usize);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 128 && (x as f32 - spot.c[0]).abs() <= reach && (y as f32 - spot.c[1]).abs() <= reach {
            (0..3).for_each(|c| avg[c] += p[c] as f64);
            n += 1;
        }
    }
    let avg = avg.map(|s| (s / n.max(1) as f64) as f32);
    let col = |k: usize| [m[k * 4], m[k * 4 + 1], m[k * 4 + 2]];
    let cols = [col(0), col(1), col(2)];
    let s2 = cols.map(|c| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).max(1e-9));
    for (i, p, nrm) in cloth {
        let q = [0, 1, 2].map(|r| cols[0][r] * p[0] + cols[1][r] * p[1] + cols[2][r] * p[2] + m[12 + r]);
        // Нормаль в рисунке — обратным растяжением.
        let mut nn = [0, 1, 2].map(|r| (0..3).map(|j| cols[j][r] * nrm[j] / s2[j]).sum::<f32>());
        let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt().max(1e-9);
        nn = nn.map(|c| c / l);
        let (sx, sz) = (q[0] - nn[0] * INSET, q[2] - nn[2] * INSET);
        let mut x = (sx / frame[0] + 0.5) * pw;
        let mut y = (sz / frame[1] + 0.5) * ph;
        let mut c = sample(img, x, y);
        // Над точкой пусто (край полотна гамака шире нарисованного, бок свеса) — шагами к
        // середине кровати и ещё на 6 точек за край нарисованного: на краю — тёмная обводка.
        if c[3] < 128.0 {
            let mut inside = 0;
            for _ in 0..14 {
                let (dx, dy) = (spot.c[0] - x, spot.c[1] - y);
                let d = (dx * dx + dy * dy).sqrt().max(1e-6);
                let step = d.min(3.0);
                x += dx / d * step;
                y += dy / d * step;
                c = sample(img, x, y);
                if c[3] >= 128.0 || inside > 0 {
                    inside += 1;
                }
                if inside > 2 {
                    break;
                }
            }
        }
        let c = if c[3] >= 128.0 { [c[0] * 255.0 / c[3], c[1] * 255.0 / c[3], c[2] * 255.0 / c[3]].map(|v| v.min(255.0)) } else { avg };
        let px = base.get_pixel_mut((i % w) as u32, (i / w) as u32);
        for k in 0..3 {
            px[k] = (c[k] * px[k] as f32 / mc[k]).round().clamp(0.0, 255.0) as u8;
        }
    }
    let kw = [0, 1, 2].map(|c| wood[c] as f32 / mw[c]);
    for (i, p) in base.pixels_mut().enumerate() {
        if cls[i] == Class::Wood as u8 {
            for c in 0..3 {
                p[c] = (p[c] as f32 * kw[c]).round().min(255.0) as u8;
            }
        }
        p[3] = 255;
    }
    Some(base)
}

/// `.glb` модели (в её осях) с картинкой цвета `base` и картами.
fn write(bed: &Bed, base: &RgbaImage) -> Option<Vec<u8>> {
    let mut mesh = Mesh::default();
    for p in &bed.model.parts {
        let at = mesh.v.len() as u32;
        mesh.v.extend_from_slice(&p.mesh.v);
        mesh.idx.extend(p.mesh.idx.iter().map(|i| i + at));
    }
    // Цвет — со сжатием (в нём рисунок FA; без потерь он вдесятеро тяжелее); карты нормалей и ORM —
    // без потерь (со сжатием их каналы перемешиваются) и вдвое мельче: они одни на все рисунки
    // вида, а в каждой кровати сцены — своей копией.
    let maps = [&bed.normal, &bed.orm].map(|img| if img.width() > 512 { image::imageops::resize(img, img.width() / 2, img.height() / 2, image::imageops::FilterType::Triangle) } else { img.clone() });
    let images = [base, &maps[0], &maps[1]];
    let bytes: Vec<(Vec<u8>, &str)> = std::iter::once(crate::export::webp_bytes(base, 90.0)).chain(maps.iter().map(|m| webp::Encoder::from_rgba(m.as_raw(), m.width(), m.height()).encode_lossless().to_vec())).map(|b| (b, "image/webp")).collect();
    let scene = Scene3d {
        nodes: vec![Node { name: "bed".into(), part: Part::Wall, storey: 0, at: [0.0; 3], yaw: 0.0, prims: vec![(0, mesh)], door: None }],
        materials: vec![Material { look: Look::Picture(0), alpha: Alpha::Opaque, two_sided: false, repeat: false, opacity: 1.0, maps: Some(Maps { normal: 1, orm: 2 }) }],
        pictures: images.iter().enumerate().map(|(i, img)| Picture { key: format!("bed#{i}"), size: [img.width(), img.height()], alpha: false }).collect(),
    };
    crate::glb::write(&scene, &bytes, "bed").ok()
}

/// Кровать к рисунку `key` (картинка `img`, `frame` клеток): `.glb` в пространстве предмета —
/// модель на месте кровати на рисунке, рамка рисунка точками без площади (как у моделей
/// [`crate::fa3d`]), вещи вокруг кровати — рисунком на полу, под сеном — весь рисунок. Не кровать
/// или кровати на рисунке не нашлось — `None`.
pub fn build(key: &str, img: &RgbaImage, frame: [f32; 2]) -> Option<Vec<u8>> {
    let (kind, fit) = kind_for(key)?;
    let ppc = img.width() as f32 / frame[0].max(1e-3);
    let (kind, spot, bed_rect) = match fit {
        Fit::Frame => {
            let r = frame_rect(img, ppc)?;
            (if (r[2] - r[0]) / ppc < 1.1 { Kind::Single } else { Kind::Double }, Spot::rect(r), Some(r))
        }
        Fit::Axis => (kind, spot_axis(img)?, None),
        Fit::Whole => (kind, Spot::rect(opaque_rect(img)?), None),
    };
    let b = bed(kind)?;
    let m = placement(b.bounds, kind.along_x(), spot, frame, [img.width(), img.height()]);
    let wood = bed_rect.and_then(|r| frame_wood(img, r)).unwrap_or_else(|| wood_tint(key));
    let base = paint(b, &m, img, frame, spot, wood)?;
    let glb = write(b, &base)?;
    // На полу: у кровати — вещи вокруг (сама кровать стёрта), у сена — весь рисунок.
    let floor = match (bed_rect, kind) {
        (Some(r), _) => {
            let mut rest = img.clone();
            for (x, y, p) in rest.enumerate_pixels_mut() {
                if (x as f32) >= r[0] - 3.0 && (x as f32) < r[2] + 3.0 && (y as f32) >= r[1] - 3.0 && (y as f32) < r[3] + 3.0 {
                    p[3] = 0;
                }
            }
            let left = rest.pixels().filter(|p| p[3] >= 128).count();
            (left as f32 > 0.02 * (img.width() * img.height()) as f32).then_some((rest, 0.004))
        }
        (None, Kind::Hay) => Some((img.clone(), 0.003)),
        _ => None,
    };
    let pic = floor.as_ref().map(|(pic, _)| crate::export::webp_bytes(pic, 90.0));
    let lid = pic.as_deref().zip(floor.as_ref().map(|(_, y)| *y));
    let box_ = [[-frame[0] / 2.0, 0.0, -frame[1] / 2.0], [frame[0] / 2.0, 0.0, frame[1] / 2.0]];
    model3d::place_gltf(&glb, m, box_, lid).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(p: &str) -> String {
        format!("pack:E:\\D&D\\FA_Objects_A_v3.52.dungeondraft_pack|textures/objects/Furniture/Bedding/{p}")
    }

    #[test]
    fn names_pick_the_bed_and_the_way_it_lies() {
        let k = |p: &str| kind_for(&key(p));
        assert_eq!(k("Arranged_Bedding/Arranged_Bed_Wood_Dark_A1_2x2.webp"), Some((Kind::Single, Fit::Frame)));
        assert_eq!(k("Beds/Single_Beds/Single_Bed_Ashen_A_1x2.webp"), Some((Kind::Single, Fit::Frame)));
        assert_eq!(k("Beds/Double_Beds/Double_Bed_Dark_C_2x2.webp"), Some((Kind::Single, Fit::Frame)), "двойная — по ширине рамки");
        assert_eq!(k("Beds/Single_Beds/Single_Bed_Blankets/Single_Bed_Blue_Blanket_B_1x2.webp"), None, "одеяло без кровати");
        assert_eq!(k("Beds/Bed_Frames/Single_Bed_Frames/Single_Bed_Frame_Wood_Dark_A_1x2.webp"), None);
        assert_eq!(k("Hammocks/Hammock_Wood_Dark_White_A_2x2.webp"), Some((Kind::Hammock, Fit::Axis)));
        assert_eq!(k("Hammocks/Hammock_Brown_B_2x2.webp"), Some((Kind::Gathered, Fit::Axis)));
        assert_eq!(k("../../Decor/Clutter/Adventuring_Gear/Bedrolls_and_Sleeping_Bags/Bedrolls/Bedroll_Cloth_Red_D_1x1.webp"), Some((Kind::Roll, Fit::Axis)));
        assert_eq!(k("../../Decor/Clutter/Adventuring_Gear/Bedrolls_and_Sleeping_Bags/Bedrolls/Bedroll_Brown_A_1x2.webp"), Some((Kind::Bedroll, Fit::Axis)));
        assert_eq!(k("../../Decor/Clutter/Adventuring_Gear/Bedrolls_and_Sleeping_Bags/Sleeping_Bag_Brown_A1_1x2.webp"), Some((Kind::SleepingBag, Fit::Axis)));
        assert_eq!(k("../../Decor/Clutter/Adventuring_Gear/Bedrolls_and_Sleeping_Bags/Rolled_Sleeping_Bag_Brown_A1_1x1.webp"), Some((Kind::Roll, Fit::Axis)));
        assert_eq!(k("Beds/Mattresses/Single_Bed_Mattress_Yellowed_A_1x2.webp"), Some((Kind::Mattress, Fit::Axis)));
        assert_eq!(k("Hay_Bedding/Hay_Bedding_B_2x3.webp"), Some((Kind::Hay, Fit::Whole)));
        assert_eq!(k("../../Decor/Nursery/Cots/Cot_Wood_Dark_Bedding_White_B1_1x2.webp"), Some((Kind::Cot, Fit::Whole)));
        assert_eq!(k("../../Decor/Nursery/Cots/Broken/Crib_Broken_Wood_Dark_A1_1x1.webp"), None);
        assert_eq!(k("Stretchers/Stretcher_Red_Dark_A_1x2.webp"), Some((Kind::Stretcher, Fit::Whole)));
        assert_eq!(kind_for("tint:96322a:C:\\Ассеты\\Barrel_Wood_Dark_A_1x1.png"), None);
        // Порода: после «wood_», иначе последняя названная.
        assert_eq!(wood_tint(&key("Hammocks/Hammock_Wood_Ashen_Red_A_2x2.webp")), WOODS[0].1);
        assert_eq!(wood_tint(&key("Stretchers/Stretcher_Red_Dark_A_1x2.webp")), WOODS[4].1);
        assert_eq!(wood_tint(&key("Hammocks/Hammock_Brown_A_2x2.webp")), WOOD);
    }

    fn fill(img: &mut RgbaImage, a: u32, b: u32, c: u32, d: u32, px: image::Rgba<u8>) {
        for y in b..d {
            for x in a..c {
                img.put_pixel(x, y, px);
            }
        }
    }

    /// Кровать FA на рисунке шириной `w` точек (256 на клетку, высота — две клетки): рамка
    /// 0,8 × 1,61 клетки с обводкой от `x0`, зелёное одеяло внутри.
    fn bed_picture(w: u32, x0: u32) -> (RgbaImage, [f32; 4]) {
        let mut img = RgbaImage::new(w, 512);
        let ink = image::Rgba([20, 14, 10, 255]);
        let (y0, x1, y1) = (55u32, x0 + 205, 468u32);
        fill(&mut img, x0, y0, x1, y1, ink);
        fill(&mut img, x0 + 3, y0 + 3, x1 - 3, y1 - 3, image::Rgba([120, 80, 50, 255]));
        fill(&mut img, x0 + 12, y0 + 28, x1 - 12, y1 - 22, ink);
        fill(&mut img, x0 + 15, y0 + 31, x1 - 15, y1 - 25, image::Rgba([40, 120, 50, 255]));
        (img, [x0 as f32, y0 as f32, x1 as f32, y1 as f32])
    }

    /// Собранная кровать FA 2×2: кровать, сбоку сундук, сверху на изголовье — горшки (уже
    /// рамки), внизу — сапоги.
    fn arranged() -> (RgbaImage, [f32; 4]) {
        let (mut img, r) = bed_picture(512, 150);
        let ink = image::Rgba([20, 14, 10, 255]);
        fill(&mut img, 362, 200, 440, 300, ink);
        fill(&mut img, 365, 203, 437, 297, image::Rgba([90, 60, 40, 255]));
        fill(&mut img, 190, 20, 260, 58, ink);
        fill(&mut img, 193, 23, 257, 55, image::Rgba([150, 100, 60, 255]));
        fill(&mut img, 200, 475, 240, 505, ink);
        (img, r)
    }

    #[test]
    fn the_frame_is_found_among_the_things_around_the_bed() {
        let (img, want) = arranged();
        let got = frame_rect(&img, 256.0).expect("рамка");
        for k in 0..4 {
            assert!((got[k] - want[k]).abs() <= 4.0, "{got:?} ≠ {want:?}");
        }
        assert_eq!(frame_rect(&RgbaImage::from_pixel(256, 512, image::Rgba([90, 60, 40, 255])), 256.0), None, "сплошное — не кровать");
    }

    #[test]
    fn the_axis_follows_a_slanted_hammock_and_snaps_to_a_square_one() {
        let mut img = RgbaImage::new(512, 512);
        for t in 0..600 {
            for s in -40..40 {
                let (x, y) = (40.0 + t as f32 * 0.7071 - s as f32 * 0.7071, 40.0 + t as f32 * 0.7071 + s as f32 * 0.7071);
                if (0.0..512.0).contains(&x) && (0.0..512.0).contains(&y) {
                    img.put_pixel(x as u32, y as u32, image::Rgba([200, 200, 200, 255]));
                }
            }
        }
        let s = spot_axis(&img).unwrap();
        assert!((s.d[0].abs() - 0.7071).abs() < 0.02 && (s.d[0] * s.d[1]) > 0.0, "наискосок: {:?}", s.d);
        assert!((s.len - 600.0).abs() < 30.0 && (s.wid - 80.0).abs() < 12.0, "{s:?}");
        let mut flat = RgbaImage::new(256, 512);
        for y in 60..450 {
            for x in 40..200 {
                flat.put_pixel(x + (y - 60) / 60, y, image::Rgba([1, 2, 3, 255]));
            }
        }
        let s = spot_axis(&flat).unwrap();
        assert!(s.d[0].abs() < 1e-6, "почти отвесный — ровно: {:?}", s.d);
    }

    #[test]
    fn every_bed_reads_with_cloth_and_three_maps() {
        for k in KINDS {
            let b = bed(k).unwrap_or_else(|| panic!("{}", k.name()));
            assert!(b.classes.contains(&Class::Cloth), "{}: ткань", k.name());
            let tris: usize = b.model.parts.iter().map(|p| p.mesh.idx.len() / 3).sum();
            assert!(tris < 12_000, "{}: {tris} треугольников", k.name());
            assert_eq!(b.normal.dimensions(), b.orm.dimensions());
            let [lo, hi] = b.bounds;
            let hangs = matches!(k, Kind::Hammock | Kind::Gathered);
            assert!(lo[1] > -0.02 && hi[1] > 0.03 && (hangs || lo[1] < 0.02), "{}: на полу, {lo:?} {hi:?}", k.name());
            if hangs {
                assert!(lo[1] > 0.3 && hi[1] > 0.8, "{}: висит, кольца выше головы сидящего", k.name());
            }
        }
        assert!(bed(Kind::Single).unwrap().classes.contains(&Class::Wood));
        assert!(bed(Kind::Hammock).unwrap().classes.contains(&Class::Keep), "верёвки и кольца — как есть");
    }

    fn apply(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
    }

    #[test]
    fn the_bed_stands_in_its_frame_head_up_and_the_hammock_along_its_slant() {
        let (img, r) = arranged();
        let b = bed(Kind::Single).unwrap();
        let m = placement(b.bounds, false, Spot::rect(r), [2.0, 2.0], [512, 512]);
        let [lo, hi] = b.bounds;
        let (a, z) = (apply(&m, lo), apply(&m, hi));
        let cell = |px: f32| px / 256.0 - 1.0;
        assert!((a[0] - cell(r[0])).abs() < 1e-3 && (z[0] - cell(r[2])).abs() < 1e-3, "по ширине рамки: {a:?} {z:?}");
        assert!((a[2] - cell(r[1])).abs() < 1e-3 && (z[2] - cell(r[3])).abs() < 1e-3, "изголовье вверху рисунка: {a:?} {z:?}");
        assert!(a[1].abs() < 0.02, "на полу");
        assert!(frame_rect(&img, 256.0).is_some());
        // Гамак — вдоль оси рисунка (вправо-вниз), кольца — у концов.
        let h = bed(Kind::Hammock).unwrap();
        let s = Spot { c: [256.0, 256.0], d: [-0.7071, -0.7071], len: 620.0, wid: 240.0 };
        let m = placement(h.bounds, true, s, [2.0, 2.0], [512, 512]);
        let [lo, hi] = h.bounds;
        let end = apply(&m, [hi[0], 0.0, (lo[2] + hi[2]) / 2.0]);
        assert!(end[0] > 0.8 && end[2] > 0.8, "конец +x — вправо-вниз: {end:?}");
    }

    #[test]
    fn the_blanket_takes_the_picture_and_the_things_around_stay_on_the_floor() {
        let (img, _) = arranged();
        let k = key("Arranged_Bedding/Arranged_Bed_Wood_Dark_A1_2x2.webp");
        let glb = build(&k, &img, [2.0, 2.0]).expect("кровать");
        let (json, _) = model3d::split_glb(&glb).unwrap();
        let j: serde_json::Value = serde_json::from_slice(json).unwrap();
        let names: Vec<&str> = j["materials"].as_array().unwrap().iter().filter_map(|m| m["name"].as_str()).collect();
        assert!(names.contains(&"top"), "вещи вокруг — рисунком на полу: {names:?}");
        let model = model3d::read_gltf(&glb, None, 1.0).unwrap();
        let [lo, hi] = model.bounds().unwrap();
        assert!(lo[0] >= -1.001 && hi[0] <= 1.001 && lo[2] >= -1.001 && hi[2] <= 1.001, "в рамке рисунка");
        // Одеяло — зелёное, как на рисунке: среди точек ткани зелёных больше всего.
        let b = bed(Kind::Single).unwrap();
        let m = placement(b.bounds, false, Spot::rect(frame_rect(&img, 256.0).unwrap()), [2.0, 2.0], [512, 512]);
        let base = paint(b, &m, &img, [2.0, 2.0], Spot::rect(frame_rect(&img, 256.0).unwrap()), wood_tint(&k)).unwrap();
        let (cls, cloth) = texels(b, base.width() as usize, base.height() as usize);
        let green = cloth.iter().filter(|(i, _, _)| {
            let p = base.get_pixel((*i % base.width() as usize) as u32, (*i / base.width() as usize) as u32);
            p[1] as i32 > p[0] as i32 + 20 && p[1] as i32 > p[2] as i32 + 20
        });
        assert!(green.count() * 2 > cloth.len(), "одеяло и постель — цветом рисунка");
        assert!(cls.contains(&(Class::Wood as u8)));
        // Без вещей вокруг — без рисунка на полу.
        let (bare, _) = bed_picture(256, 25);
        let glb = build(&key("Beds/Single_Beds/Single_Bed_Dark_A_1x2.webp"), &bare, [1.0, 2.0]).expect("кровать");
        assert!(!String::from_utf8_lossy(model3d::split_glb(&glb).unwrap().0).contains("\"top\""));
    }
}
