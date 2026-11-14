//! Вечер и ночь на снимке карты (`view` и `render_picture` с `lighting`): огни
//! предметов светят поверх готовой дневной картинки, как в Foundry. Стены, двери,
//! окна и сами огни — из того же разбора, что уходит в Foundry
//! ([`crate::export::layout`]): стены уже разрезаны проёмами, низкие стены и пустые
//! проёмы свет пропускают, окно — больше половины, закрытая дверь держит его.
//!
//! Свет считается на своей сетке — не чаще [`PER_CELL`] точек на клетку, сколько бы
//! точек ни было у снимка: он плавный, а лучей так меньше в десятки раз. Сетка
//! размывается для мягких полутеней и растягивается на картинку; строки считаются
//! во всех ядрах.
//!
//! Огни не складываются в белое пятно: свет насыщается мягко (`1 − e^(−k·L)`), как
//! в Foundry, где яркий свет — это дневная картинка, а не ярче её. Рисунок пола
//! под шестью бра остаётся рисунком; само ядро огня чуть светится поверх.

use crate::export::{Layout, PortalOut};
use crate::map::PortalKind;
use crate::mask::{Mask, RES};
use image::RgbImage;

/// Время суток снимка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Time {
    /// Как есть: дневная картинка без своего света (под динамический свет Foundry).
    Day,
    Dusk,
    Night,
}

impl Time {
    pub fn parse(s: &str) -> Result<Time, String> {
        match s {
            "day" => Ok(Time::Day),
            "dusk" => Ok(Time::Dusk),
            "night" => Ok(Time::Night),
            _ => Err("lighting: day, dusk или night".into()),
        }
    }

    /// Рассеянный свет снаружи и в комнатах, во сколько раз сильнее огни.
    fn ambient(self) -> ([f32; 3], [f32; 3], f32) {
        match self {
            Time::Day => ([1.0; 3], [1.0; 3], 0.0),
            Time::Dusk => ([0.58, 0.65, 0.80], [0.55, 0.53, 0.55], 1.1),
            Time::Night => ([0.36, 0.44, 0.62], [0.35, 0.34, 0.36], 1.3),
        }
    }
}

/// Огонь: середина, радиус яркого и тусклого в клетках, цвет 0..1, сила (магия
/// тише пламени); `roofed` — под крышей (на саму крышу он не светит).
pub struct Source {
    pub at: [f32; 2],
    pub bright: f32,
    pub dim: f32,
    pub color: [f32; 3],
    pub power: f32,
    pub roofed: bool,
}

/// Преграда свету: отрезок и доля света, которая сквозь него проходит.
pub struct Blocker {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub pass: f32,
}

pub struct Scene {
    pub sources: Vec<Source>,
    pub blockers: Vec<Blocker>,
    /// Пол комнат: внутри — свой рассеянный свет.
    pub inside: Mask,
    /// Контуры крыш, если они на снимке целые: под ними внутренний свет не виден.
    pub roofs: Vec<Vec<[f32; 2]>>,
}

/// Сквозь окно проходит столько света.
pub const WINDOW_PASS: f32 = 0.6;
/// Точек карты света на клетку, не больше: тени и так размыты.
pub const PER_CELL: f32 = 10.0;
/// Мягкое насыщение: свет одного огня в яркой зоне — почти весь путь до `PEAK`.
const SATURATE: f32 = 1.9;
/// Предел освещённости: чуть светлее дневной картинки, и не выше, сколько огней ни сложи.
const PEAK: f32 = 1.2;
/// Тусклая зона на краю яркой — такая доля яркого.
const DIM: f32 = 0.6;
/// Ядро огня, клетки: только здесь свет ложится поверх картинки сиянием.
const CORE: f32 = 0.35;
const HALO: f32 = 0.3;

fn hex(c: &str) -> [f32; 3] {
    let c = c.trim_start_matches('#');
    let byte = |i: usize| c.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).unwrap_or(255) as f32 / 255.0;
    [byte(0), byte(2), byte(4)]
}

fn inside_poly(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut inside = false;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            inside = !inside;
        }
    }
    inside
}

impl Scene {
    /// Сцена света по разбору этажа для Foundry.
    pub fn new(layout: &Layout, inside: Mask, roofs: Vec<Vec<[f32; 2]>>) -> Scene {
        let mut blockers = Vec::new();
        for w in &layout.walls {
            for s in w.windows(2) {
                blockers.push(Blocker { a: s[0], b: s[1], pass: 0.0 });
            }
        }
        for PortalOut { seg, kind, .. } in &layout.portals {
            let pass = if *kind == PortalKind::Window { WINDOW_PASS } else { 0.0 };
            blockers.push(Blocker { a: seg.0, b: seg.1, pass });
        }
        let sources = layout
            .lights
            .iter()
            // Цвет огня — наполовину с белым: в Foundry он ложится полупрозрачно, иначе
            // жёлтый фонарь красил траву в салатовый. Магический свет — холодный и на
            // тёмно-синей ночи кажется белее пламени, поэтому тише.
            .map(|l| Source {
                at: l.at,
                bright: l.bright,
                dim: l.dim.max(l.bright + 0.1),
                color: hex(l.color).map(|c| 0.5 + 0.5 * c),
                power: if l.animation == "energy" { 0.75 } else { 1.0 },
                roofed: roofs.iter().any(|r| inside_poly(l.at, r)),
            })
            .collect();
        Scene { sources, blockers, inside, roofs }
    }
}

/// Доля света, дошедшая от `s` до `p` сквозь преграды. Пересечения у самого огня
/// (ближе `skip` клетки) не в счёт: бра висит на стене, свеча стоит у окна.
fn visibility(s: [f32; 2], p: [f32; 2], blockers: &[&Blocker], skip: f32) -> f32 {
    let (dx, dy) = (p[0] - s[0], p[1] - s[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let mut v = 1.0;
    for b in blockers {
        let (ex, ey) = (b.b[0] - b.a[0], b.b[1] - b.a[1]);
        let den = dx * ey - dy * ex;
        if den.abs() < 1e-9 {
            continue;
        }
        let (wx, wy) = (b.a[0] - s[0], b.a[1] - s[1]);
        let t = (wx * ey - wy * ex) / den;
        let u = (wx * dy - wy * dx) / den;
        if t > 0.0 && t < 1.0 && (0.0..=1.0).contains(&u) && t * len > skip {
            v *= b.pass;
            if v <= 0.0 {
                return 0.0;
            }
        }
    }
    v
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Разложить строки `data` (по `row` значений) по ядрам: `f(первая строка, полоса)`.
fn par_rows<T: Send>(data: &mut [T], row: usize, f: impl Fn(usize, &mut [T]) + Sync) {
    let rows = data.len() / row.max(1);
    let n = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, rows.max(1));
    if n == 1 {
        f(0, data);
        return;
    }
    let per = rows.div_ceil(n);
    std::thread::scope(|s| {
        for (k, band) in data.chunks_mut(per * row).enumerate() {
            let f = &f;
            s.spawn(move || f(k * per, band));
        }
    });
}

/// Точка карты света: множитель картинки по каналам и сияние ядер огней поверх.
type Texel = [f32; 6];

/// Карта света в точках сетки `lw × lh`, каждая — `cell` клеток, от угла `origin`.
fn light_map(scene: &Scene, time: Time, origin: [f32; 2], cell: f32, lw: usize, lh: usize) -> Vec<Texel> {
    let (out_amb, in_amb, gain) = time.ambient();
    let at = |i: usize, j: usize| [origin[0] + (i as f32 + 0.5) * cell, origin[1] + (j as f32 + 0.5) * cell];
    let boxes: Vec<[f32; 4]> = scene.roofs.iter().map(|r| r.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])).collect();
    let under_roof = |p: [f32; 2]| scene.roofs.iter().zip(&boxes).any(|(r, b)| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3] && inside_poly(p, r));
    // Преграды у каждого огня и его рамка в точках сетки — один раз на всех.
    let span = |c: f32, o: f32, n: usize| (((c - o) / cell).floor().max(0.0) as usize).min(n);
    let lit: Vec<(&Source, Vec<&Blocker>, [usize; 4])> = scene
        .sources
        .iter()
        .map(|s| {
            let near = scene.blockers.iter().filter(|b| crate::map::segment_distance(s.at, b.a, b.b) < s.dim).collect();
            let frame = [span(s.at[0] - s.dim, origin[0], lw), (span(s.at[0] + s.dim, origin[0], lw) + 1).min(lw), span(s.at[1] - s.dim, origin[1], lh), (span(s.at[1] + s.dim, origin[1], lh) + 1).min(lh)];
            (s, near, frame)
        })
        .collect();
    let mut map = vec![[0.0; 6]; lw * lh];
    par_rows(&mut map, lw, |j0, band| {
        let rows = band.len() / lw.max(1);
        // Сумма огней в точке до насыщения и признак «под крышей».
        let mut sum = vec![[0.0f32; 3]; band.len()];
        let roofed: Vec<bool> = (0..band.len()).map(|k| !scene.roofs.is_empty() && under_roof(at(k % lw, j0 + k / lw))).collect();
        for (s, near, [i0, i1, ja, jb]) in &lit {
            for j in (*ja).max(j0)..(*jb).min(j0 + rows) {
                for i in *i0..*i1 {
                    let p = at(i, j);
                    let d = ((p[0] - s.at[0]).powi(2) + (p[1] - s.at[1]).powi(2)).sqrt();
                    if d >= s.dim {
                        continue;
                    }
                    let k = (j - j0) * lw + i;
                    let f = if d <= s.bright { 1.0 } else { smooth(1.0 - (d - s.bright) / (s.dim - s.bright)) * DIM };
                    // Крыша выше стен: огонь под ней на неё не светит, уличный — светит без преград.
                    let vis = if roofed[k] { if s.roofed { 0.0 } else { 1.0 } } else { visibility(s.at, p, near, 0.15) };
                    if vis <= 0.0 {
                        continue;
                    }
                    let core = if d < CORE && !roofed[k] { (1.0 - d / CORE).powi(2) * HALO * s.power } else { 0.0 };
                    for c in 0..3 {
                        sum[k][c] += s.color[c] * f * vis * gain * s.power;
                        band[k][3 + c] += s.color[c] * core * vis;
                    }
                }
            }
        }
        for (k, t) in band.iter_mut().enumerate() {
            let p = at(k % lw, j0 + k / lw);
            let room = scene.inside.get((p[0] * RES as f32).floor() as isize, (p[1] * RES as f32).floor() as isize) > 127;
            let amb = if room && !roofed[k] { in_amb } else { out_amb };
            for c in 0..3 {
                // Огни поднимают свет от рассеянного к пределу, но не выше его.
                let s = 1.0 - (-SATURATE * sum[k][c]).exp();
                t[c] = amb[c] + (PEAK - amb[c]).max(0.0) * s;
            }
        }
    });
    map
}

/// Размытие Гаусса по строкам и столбцам: мягкие полутени.
fn blur(map: &mut [Texel], w: usize, h: usize, sigma: f32) {
    let r = (sigma * 2.5).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r).map(|x| (-(x * x) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = kernel.iter().sum();
    let mut tmp = map.to_vec();
    for pass in 0..2 {
        let (src, dst): (&[Texel], &mut [Texel]) = if pass == 0 { (&*map, &mut tmp[..]) } else { (&tmp[..], &mut *map) };
        par_rows(dst, w, |j0, band| {
            for (k, out) in band.iter_mut().enumerate() {
                let (i, j) = (k % w, j0 + k / w);
                let mut acc = [0.0; 6];
                for (o, kv) in (-r..=r).zip(&kernel) {
                    let (x, y) = if pass == 0 { ((i as isize + o).clamp(0, w as isize - 1) as usize, j) } else { (i, (j as isize + o).clamp(0, h as isize - 1) as usize) };
                    for (a, v) in acc.iter_mut().zip(src[y * w + x]) {
                        *a += v * kv;
                    }
                }
                *out = acc.map(|v| v / sum);
            }
        });
    }
}

/// Светлое подходит к белому плавно: насыщенный пол не срезается в сплошное пятно.
fn rolloff(v: f32) -> f32 {
    const KNEE: f32 = 0.8;
    if v <= KNEE { v.max(0.0) } else { KNEE + (1.0 - KNEE) * (1.0 - (-(v - KNEE) / (1.0 - KNEE)).exp()) }
}

/// Наложить вечер или ночь на снимок области `area` (клетки). День — как есть.
pub fn apply(img: &mut RgbImage, area: [f32; 4], scene: &Scene, time: Time) {
    if time == Time::Day || img.width() == 0 || img.height() == 0 {
        return;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let ppc = w as f32 / (area[2] - area[0]).max(1e-3);
    // Точка карты света — не мельче четырёх точек картинки и не мельче 1/PER_CELL клетки.
    let cell = (4.0 / ppc).max(1.0 / PER_CELL);
    let scale = cell * ppc;
    let (lw, lh) = (((w as f32 / scale).ceil() as usize).max(1), ((h as f32 / scale).ceil() as usize).max(1));
    let mut map = light_map(scene, time, [area[0], area[1]], cell, lw, lh);
    blur(&mut map, lw, lh, 1.3);
    let map = &map;
    let buf: &mut [u8] = img;
    par_rows(buf, w * 3, |y0, band| {
        for (k, px) in band.chunks_exact_mut(3).enumerate() {
            let (x, y) = ((k % w) as f32 + 0.5, (y0 + k / w) as f32 + 0.5);
            let (fx, fy) = ((x / scale - 0.5).clamp(0.0, lw as f32 - 1.0), (y / scale - 0.5).clamp(0.0, lh as f32 - 1.0));
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(lw - 1), (y0 + 1).min(lh - 1));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let (a, b, c, d) = (&map[y0 * lw + x0], &map[y0 * lw + x1], &map[y1 * lw + x0], &map[y1 * lw + x1]);
            for ch in 0..3 {
                let l = |n: usize| (a[n] * (1.0 - tx) + b[n] * tx) * (1.0 - ty) + (c[n] * (1.0 - tx) + d[n] * tx) * ty;
                let v = px[ch] as f32 / 255.0 * l(ch) + l(3 + ch);
                px[ch] = (rolloff(v) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(at: [f32; 2], color: &'static str, animation: &'static str) -> crate::export::Light {
        crate::export::Light { at, bright: 2.0, dim: 5.0, color, animation }
    }

    fn layout(walls: Vec<Vec<[f32; 2]>>, portals: Vec<PortalOut>, lights: Vec<crate::export::Light>) -> Layout {
        Layout { tents: Vec::new(), walls, low: Vec::new(), cliffs: Vec::new(), portals, lights, zones: Vec::new(), passages: Vec::new() }
    }

    fn room_scene(window: bool) -> Scene {
        // Комната 4×4 клетки, огонь посередине; в правой стене — окно или стена целиком.
        let mut walls = vec![vec![[2.0, 2.0], [6.0, 2.0], [6.0, 3.5]], vec![[6.0, 4.5], [6.0, 6.0], [2.0, 6.0], [2.0, 2.0]]];
        let mut portals = Vec::new();
        if window {
            portals.push(PortalOut { seg: ([6.0, 3.5], [6.0, 4.5]), kind: PortalKind::Window, leaf: None });
        } else {
            walls.push(vec![[6.0, 3.5], [6.0, 4.5]]);
        }
        Scene::new(&layout(walls, portals, vec![light([4.0, 4.0], "#ffffff", "flame")]), Mask::for_cells(10, 10), Vec::new())
    }

    fn brightness(scene: &Scene, p: [f32; 2]) -> f32 {
        // Одна точка карты света в клетке p.
        let m = light_map(scene, Time::Night, [p[0] - 0.05, p[1] - 0.05], 0.1, 1, 1);
        m[0][0]
    }

    #[test]
    fn walls_hold_light_and_windows_let_some_out() {
        let shut = room_scene(false);
        let open = room_scene(true);
        let out = [7.0, 4.0];
        let dark = brightness(&shut, out);
        assert!((dark - 0.36).abs() < 1e-3, "за глухой стеной — только ночь: {dark}");
        let lit = brightness(&open, out);
        assert!(lit > dark + 0.2, "из окна свет ложится наружу: {lit}");
        let inside = brightness(&open, [4.5, 4.0]);
        assert!(inside > lit, "в комнате ярче, чем за окном");
        // Мимо окна (выше) — снова темно.
        assert!((brightness(&open, [7.0, 2.6]) - dark).abs() < 1e-3);
    }

    #[test]
    fn interior_light_does_not_shine_on_its_roof() {
        let roof = vec![vec![[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]]];
        let s = Scene::new(&layout(Vec::new(), Vec::new(), vec![light([4.0, 4.0], "#ffffff", "flame")]), Mask::for_cells(10, 10), roof);
        assert!(s.sources[0].roofed);
        assert!((brightness(&s, [4.5, 4.0]) - 0.36).abs() < 1e-3, "крыша над огнём тёмная");
        assert!(brightness(&s, [7.0, 4.0]) > 0.5, "двор рядом освещён");
    }

    #[test]
    fn day_leaves_the_picture_alone() {
        let mut img = RgbImage::from_pixel(8, 8, image::Rgb([100, 120, 140]));
        apply(&mut img, [0.0, 0.0, 8.0, 8.0], &room_scene(true), Time::Day);
        assert_eq!(img.get_pixel(3, 3).0, [100, 120, 140]);
        let dark = Scene { sources: Vec::new(), blockers: Vec::new(), inside: Mask::for_cells(8, 8), roofs: Vec::new() };
        apply(&mut img, [0.0, 0.0, 8.0, 8.0], &dark, Time::Night);
        assert_eq!(img.get_pixel(3, 3).0, [36, 53, 87], "без огней — ночной рассеянный свет");
    }

    /// Восемь огней в одной точке не выжигают пол: свет не выше предела, светлые и
    /// тёмные доски паркета различимы.
    #[test]
    fn many_lights_keep_the_floor_pattern() {
        let lights = (0..8).map(|k| light([4.0 + 0.1 * k as f32, 4.0], if k % 2 == 0 { "#78b8ff" } else { "#ff9a3c" }, "flame")).collect();
        let s = Scene::new(&layout(Vec::new(), Vec::new(), lights), Mask::for_cells(10, 10), Vec::new());
        let m = brightness(&s, [5.0, 4.5]);
        assert!(m <= PEAK + 1e-3, "свет не выше предела: {m}");
        // Полосы паркета: светлая 230 и тёмная 150, по 4 точки.
        let mut img = RgbImage::from_fn(80, 80, |x, _| if (x / 4) % 2 == 0 { image::Rgb([230, 226, 220]) } else { image::Rgb([150, 140, 130]) });
        apply(&mut img, [0.0, 0.0, 8.0, 8.0], &s, Time::Night);
        let (light, dark) = (img.get_pixel(48, 46).0, img.get_pixel(52, 46).0);
        assert!(light.iter().all(|&v| v < 252), "светлые доски не белые: {light:?}");
        assert!(light[0] as i32 - dark[0] as i32 > 25, "рисунок виден: {light:?} / {dark:?}");
    }

    /// Магический свет тише пламени той же силы.
    #[test]
    fn arcane_is_softer_than_flame() {
        let flame = Scene::new(&layout(Vec::new(), Vec::new(), vec![light([4.0, 4.0], "#ffffff", "flame")]), Mask::for_cells(10, 10), Vec::new());
        let magic = Scene::new(&layout(Vec::new(), Vec::new(), vec![light([4.0, 4.0], "#ffffff", "energy")]), Mask::for_cells(10, 10), Vec::new());
        assert!(brightness(&magic, [5.5, 4.0]) < brightness(&flame, [5.5, 4.0]));
    }

    /// На крупном снимке сетка света не мельче PER_CELL на клетку, а картинка та же,
    /// что на мелком: свет сравнивается в одной и той же клетке.
    #[test]
    fn big_pictures_use_the_same_light_grid() {
        let s = room_scene(true);
        let shot = |px: u32| {
            let mut img = RgbImage::from_pixel(10 * px, 10 * px, image::Rgb([200, 200, 200]));
            apply(&mut img, [0.0, 0.0, 10.0, 10.0], &s, Time::Night);
            let p = |x: f32, y: f32| img.get_pixel((x * px as f32) as u32, (y * px as f32) as u32).0[0] as i32;
            [p(4.5, 4.0), p(7.0, 4.0), p(8.0, 8.0)]
        };
        let (small, big) = (shot(40), shot(140));
        for (a, b) in small.iter().zip(big) {
            assert!((a - b).abs() <= 3, "40 px {small:?} и 140 px {big:?}");
        }
    }

    /// Обсерватория 24×22 на 140 точек: 24 огня ложатся быстро.
    #[test]
    #[ignore]
    fn big_picture_is_fast() {
        let walls = vec![(0..=8).map(|i| { let a = (22.5 + 45.0 * i as f32).to_radians(); [9.5 + 6.0 * a.cos(), 10.5 + 6.0 * a.sin()] }).collect()];
        let lights = (0..24).map(|k| light([3.0 + (k % 6) as f32 * 3.0, 3.0 + (k / 6) as f32 * 4.5], "#78b8ff", if k % 3 == 0 { "energy" } else { "flame" })).collect();
        let s = Scene::new(&layout(walls, Vec::new(), lights), Mask::for_cells(24, 22), Vec::new());
        let mut img = RgbImage::from_pixel(24 * 140, 22 * 140, image::Rgb([180, 170, 160]));
        let t = std::time::Instant::now();
        apply(&mut img, [0.0, 0.0, 24.0, 22.0], &s, Time::Night);
        let ms = t.elapsed().as_millis();
        eprintln!("3360×3080, 24 огня: {ms} мс");
        assert!(ms < 3000);
    }
}
