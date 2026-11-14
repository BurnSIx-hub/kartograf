//! Вечер и ночь на снимке карты (`view` и `render_picture` с `lighting`): огни
//! предметов светят поверх готовой дневной картинки, как в Foundry. Стены, двери,
//! окна и сами огни — из того же разбора, что уходит в Foundry
//! ([`crate::export::layout`]): стены уже разрезаны проёмами, низкие стены и пустые
//! проёмы свет пропускают, окно — больше половины, закрытая дверь держит его.
//! Свет считается на сетке в четверть точек картинки, размывается для мягких
//! полутеней и растягивается обратно.

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

    /// Рассеянный свет снаружи и в комнатах, во сколько раз ярче огни.
    fn ambient(self) -> ([f32; 3], [f32; 3], f32) {
        match self {
            Time::Day => ([1.0; 3], [1.0; 3], 0.0),
            Time::Dusk => ([0.58, 0.65, 0.80], [0.55, 0.53, 0.55], 1.1),
            Time::Night => ([0.36, 0.44, 0.62], [0.35, 0.34, 0.36], 1.3),
        }
    }
}

/// Огонь: середина, радиус яркого и тусклого в клетках, цвет 0..1; `roofed` — под
/// крышей (на саму крышу он не светит).
pub struct Source {
    pub at: [f32; 2],
    pub bright: f32,
    pub dim: f32,
    pub color: [f32; 3],
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
            // жёлтый фонарь красил траву в салатовый.
            .map(|l| Source { at: l.at, bright: l.bright, dim: l.dim.max(l.bright + 0.1), color: hex(l.color).map(|c| 0.5 + 0.5 * c), roofed: roofs.iter().any(|r| inside_poly(l.at, r)) })
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

/// Карта света в точках сетки `lw × lh`, каждая — `cell` клеток, от угла `origin`.
fn light_map(scene: &Scene, time: Time, origin: [f32; 2], cell: f32, lw: usize, lh: usize) -> Vec<[f32; 3]> {
    let (out_amb, in_amb, gain) = time.ambient();
    let at = |i: usize, j: usize| [origin[0] + (i as f32 + 0.5) * cell, origin[1] + (j as f32 + 0.5) * cell];
    let mut roofed = vec![false; lw * lh];
    if !scene.roofs.is_empty() {
        let boxes: Vec<[f32; 4]> = scene.roofs.iter().map(|r| r.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])).collect();
        for j in 0..lh {
            for i in 0..lw {
                let p = at(i, j);
                roofed[j * lw + i] = scene.roofs.iter().zip(&boxes).any(|(r, b)| p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3] && inside_poly(p, r));
            }
        }
    }
    let mut map: Vec<[f32; 3]> = (0..lw * lh)
        .map(|k| {
            let p = at(k % lw, k / lw);
            let room = scene.inside.get((p[0] * RES as f32).floor() as isize, (p[1] * RES as f32).floor() as isize) > 127;
            if room && !roofed[k] { in_amb } else { out_amb }
        })
        .collect();
    for s in &scene.sources {
        let near: Vec<&Blocker> = scene.blockers.iter().filter(|b| crate::map::segment_distance(s.at, b.a, b.b) < s.dim).collect();
        let span = |c: f32, o: f32, n: usize| (((c - o) / cell).floor().max(0.0) as usize).min(n);
        let (i0, i1) = (span(s.at[0] - s.dim, origin[0], lw), span(s.at[0] + s.dim, origin[0], lw) + 1);
        let (j0, j1) = (span(s.at[1] - s.dim, origin[1], lh), span(s.at[1] + s.dim, origin[1], lh) + 1);
        for j in j0..j1.min(lh) {
            for i in i0..i1.min(lw) {
                let p = at(i, j);
                let d = ((p[0] - s.at[0]).powi(2) + (p[1] - s.at[1]).powi(2)).sqrt();
                if d >= s.dim {
                    continue;
                }
                let k = j * lw + i;
                let f = if d <= s.bright { 1.0 } else { smooth(1.0 - (d - s.bright) / (s.dim - s.bright)) * 0.75 };
                // Крыша выше стен: огонь под ней на неё не светит, уличный — светит без преград.
                let vis = if roofed[k] { if s.roofed { 0.0 } else { 1.0 } } else { visibility(s.at, p, &near, 0.15) };
                if vis > 0.0 {
                    for (m, col) in map[k].iter_mut().zip(s.color) {
                        *m += col * f * vis * gain;
                    }
                }
            }
        }
    }
    map
}

/// Размытие Гаусса по строкам и столбцам: мягкие полутени.
fn blur(map: &mut [[f32; 3]], w: usize, h: usize, sigma: f32) {
    let r = (sigma * 2.5).ceil() as isize;
    let kernel: Vec<f32> = (-r..=r).map(|x| (-(x * x) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = kernel.iter().sum();
    let mut tmp = map.to_vec();
    for pass in 0..2 {
        for j in 0..h {
            for i in 0..w {
                let mut acc = [0.0; 3];
                for (o, kv) in (-r..=r).zip(&kernel) {
                    let (x, y) = if pass == 0 { ((i as isize + o).clamp(0, w as isize - 1) as usize, j) } else { (i, (j as isize + o).clamp(0, h as isize - 1) as usize) };
                    let src = if pass == 0 { map[y * w + x] } else { tmp[y * w + x] };
                    for c in 0..3 {
                        acc[c] += src[c] * kv;
                    }
                }
                let dst = if pass == 0 { &mut tmp[j * w + i] } else { &mut map[j * w + i] };
                *dst = acc.map(|v| v / sum);
            }
        }
    }
}

/// Наложить вечер или ночь на снимок области `area` (клетки). День — как есть.
pub fn apply(img: &mut RgbImage, area: [f32; 4], scene: &Scene, time: Time) {
    if time == Time::Day || img.width() == 0 {
        return;
    }
    const STEP: u32 = 4;
    let ppc = img.width() as f32 / (area[2] - area[0]).max(1e-3);
    let (lw, lh) = (img.width().div_ceil(STEP) as usize, img.height().div_ceil(STEP) as usize);
    let cell = STEP as f32 / ppc;
    let mut map = light_map(scene, time, [area[0], area[1]], cell, lw, lh);
    blur(&mut map, lw, lh, 1.3);
    let sample = |x: f32, y: f32| {
        let (fx, fy) = ((x / STEP as f32 - 0.5).clamp(0.0, lw as f32 - 1.0), (y / STEP as f32 - 0.5).clamp(0.0, lh as f32 - 1.0));
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(lw - 1), (y0 + 1).min(lh - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let mut out = [0.0; 3];
        for c in 0..3 {
            let top = map[y0 * lw + x0][c] * (1.0 - tx) + map[y0 * lw + x1][c] * tx;
            let bottom = map[y1 * lw + x0][c] * (1.0 - tx) + map[y1 * lw + x1][c] * tx;
            out[c] = top * (1.0 - ty) + bottom * ty;
        }
        out
    };
    for (x, y, p) in img.enumerate_pixels_mut() {
        let l = sample(x as f32 + 0.5, y as f32 + 0.5);
        for c in 0..3 {
            // Ярче единицы — мягкое сияние поверх: ядро огня светится, а не выгорает.
            let glow = (l[c] - 1.05).max(0.0) * 70.0;
            p[c] = (p[c] as f32 * l[c] + glow).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room_scene(window: bool) -> Scene {
        // Комната 4×4 клетки, огонь посередине; в правой стене — окно или стена целиком.
        let mut walls = vec![vec![[2.0, 2.0], [6.0, 2.0], [6.0, 3.5]], vec![[6.0, 4.5], [6.0, 6.0], [2.0, 6.0], [2.0, 2.0]]];
        let mut portals = Vec::new();
        if window {
            portals.push(PortalOut { seg: ([6.0, 3.5], [6.0, 4.5]), kind: PortalKind::Window, leaf: None });
        } else {
            walls.push(vec![[6.0, 3.5], [6.0, 4.5]]);
        }
        let layout = Layout { tents: Vec::new(), walls, low: Vec::new(), cliffs: Vec::new(), portals, lights: vec![crate::export::Light { at: [4.0, 4.0], bright: 2.0, dim: 5.0, color: "#ffffff", animation: "flame" }], zones: Vec::new(), passages: Vec::new() };
        Scene::new(&layout, Mask::for_cells(10, 10), Vec::new())
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
        let layout = Layout { tents: Vec::new(), walls: Vec::new(), low: Vec::new(), cliffs: Vec::new(), portals: Vec::new(), lights: vec![crate::export::Light { at: [4.0, 4.0], bright: 2.0, dim: 5.0, color: "#ffffff", animation: "flame" }], zones: Vec::new(), passages: Vec::new() };
        let roof = vec![vec![[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]]];
        let s = Scene::new(&layout, Mask::for_cells(10, 10), roof);
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
}
