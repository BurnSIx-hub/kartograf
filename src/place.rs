//! Местность по координатам — то, что делали скрипты `tools/osm` (satlib, photo,
//! cover, eu, nfi, trees), внутри программы: выгрузка OSM по рамке, снимок Esri
//! World Imagery в самом крупном масштабе, какой есть у места, слои Copernicus
//! (порода леса, мокрые земли), леса NFI (Англия) и анализ снимка — подсказка
//! [`Cover`] с кронами для импорта.
//!
//! Правила и пороги — те же, что в скриптах; откуда каждое число — в их
//! комментариях и в заметке «уроки местности». Скрипты остаются для кругов сверки.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use crate::procgen::osm::{CELL_M, Cover, Crown};
use image::RgbImage;
use std::path::{Path, PathBuf};

// ---------- рамка и координаты ----------

/// Рамка участка в градусах.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

impl Frame {
    /// Рамка `cells` клеток по 5 футов вокруг точки.
    pub fn around(lat: f64, lon: f64, cells: [u32; 2]) -> Self {
        let (w, h) = (cells[0] as f64 * CELL_M as f64, cells[1] as f64 * CELL_M as f64);
        let dlat = h / 2.0 / 110_574.0;
        let dlon = w / 2.0 / (111_320.0 * lat.to_radians().cos());
        Self { south: lat - dlat, west: lon - dlon, north: lat + dlat, east: lon + dlon }
    }

    /// Рамка из `<bounds>` выгрузки: по ней импорт кладёт карту, по ней и снимок.
    pub fn of_osm(xml: &str) -> Option<Self> {
        let tag = &xml[xml.find("<bounds")?..];
        let tag = &tag[..tag.find('>')?];
        let num = |key: &str| -> Option<f64> {
            let at = tag.find(&format!("{key}=\""))? + key.len() + 2;
            tag[at..].split('"').next()?.parse().ok()
        };
        Some(Self { south: num("minlat")?, west: num("minlon")?, north: num("maxlat")?, east: num("maxlon")? })
    }

    fn mid_lat(&self) -> f64 {
        (self.south + self.north) / 2.0
    }

    fn width_m(&self) -> f64 {
        (self.east - self.west) * 111_320.0 * self.mid_lat().to_radians().cos()
    }

    fn height_m(&self) -> f64 {
        (self.north - self.south) * 110_574.0
    }
}

/// Выгрузка с рамкой `frame` вместо своей (`<bounds>`): импорт кладёт карту по ней.
pub fn with_bounds(xml: &str, frame: &Frame) -> String {
    let tag = format!(r#"<bounds minlat="{:.7}" minlon="{:.7}" maxlat="{:.7}" maxlon="{:.7}"/>"#, frame.south, frame.west, frame.north, frame.east);
    match xml.find("<bounds").and_then(|a| xml[a..].find('>').map(|b| (a, a + b + 1))) {
        Some((a, b)) => format!("{}{tag}{}", &xml[..a], &xml[b..]),
        None => match xml.find("<osm").and_then(|a| xml[a..].find('>').map(|b| a + b + 1)) {
            Some(at) => format!("{}\n {tag}{}", &xml[..at], &xml[at..]),
            None => xml.to_owned(),
        },
    }
}

/// Широта и долгота из того, что вставил человек: «59.46956, 25.65897», ссылка
/// OpenStreetMap (`#map=18/59.46/25.65`, `mlat=…&mlon=…`) или Google Карт
/// (`@59.46,25.65,17z`, `!3d59.46!4d25.65`).
pub fn parse_place(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    let nums = |s: &str| -> Vec<f64> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in s.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_digit() || c == '.' || (c == '-' && cur.is_empty()) {
                cur.push(c);
            } else {
                if let Ok(v) = cur.parse::<f64>() {
                    out.push(v);
                }
                cur.clear();
                if c == '-' {
                    cur.push(c);
                }
            }
        }
        out
    };
    let after = |key: &str| -> Option<f64> { nums(&text[text.find(key)? + key.len()..]).first().copied() };
    let pair = if let (Some(a), Some(b)) = (after("!3d"), after("!4d")) {
        Some((a, b))
    } else if let (Some(a), Some(b)) = (after("mlat="), after("mlon=")) {
        Some((a, b))
    } else if let Some(at) = text.find("#map=") {
        let n = nums(&text[at + 5..]);
        (n.len() >= 3).then(|| (n[1], n[2]))
    } else if let Some(at) = text.find('@') {
        let n = nums(&text[at + 1..]);
        (n.len() >= 2).then(|| (n[0], n[1]))
    } else {
        let n = nums(text);
        (n.len() >= 2).then(|| (n[0], n[1]))
    };
    pair.filter(|(lat, lon)| lat.abs() <= 85.0 && lon.abs() <= 180.0)
}

// ---------- растры ----------

/// Поле чисел по пикселям снимка.
#[derive(Clone)]
struct Grid {
    w: usize,
    h: usize,
    d: Vec<f32>,
}

impl Grid {
    fn new(w: usize, h: usize, v: f32) -> Self {
        Self { w, h, d: vec![v; w * h] }
    }

    fn map(&self, f: impl Fn(f32) -> f32) -> Self {
        Self { w: self.w, h: self.h, d: self.d.iter().map(|&v| f(v)).collect() }
    }
}

/// Индекс за краем — отражением, как `mode="reflect"` в scipy (d c b a | a b c d).
fn reflect(i: isize, n: usize) -> usize {
    let n = n as isize;
    let mut i = i;
    loop {
        if i < 0 {
            i = -i - 1;
        } else if i >= n {
            i = 2 * n - i - 1;
        } else {
            return i as usize;
        }
    }
}

/// Гауссово размытие, `sigma` — в пикселях; ядро до 4σ (как у scipy).
fn gauss(g: &Grid, sigma: f32) -> Grid {
    let r = (4.0 * sigma + 0.5) as isize;
    let mut k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    let pass = |src: &[f32], w: usize, h: usize, along_x: bool| -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let o = j as isize - r;
                    let v = if along_x { src[y * w + reflect(x as isize + o, w)] } else { src[reflect(y as isize + o, h) * w + x] };
                    acc += kv * v;
                }
                out[y * w + x] = acc;
            }
        }
        out
    };
    let a = pass(&g.d, g.w, g.h, true);
    Grid { w: g.w, h: g.h, d: pass(&a, g.w, g.h, false) }
}

/// Размытие как в cover.py (Pillow на восьми битах): вход и выход — ступенями 1/255.
fn blur8(g: &Grid, sigma: f32) -> Grid {
    let q = g.map(|v| (v.clamp(0.0, 1.0) * 255.0).floor() / 255.0);
    gauss(&q, sigma).map(|v| (v * 255.0).round() / 255.0)
}

/// Среднее по квадрату `size`×`size` (`uniform_filter` scipy, отражение за краем).
fn uniform(g: &Grid, size: usize) -> Grid {
    let lo = (size / 2) as isize;
    let pass = |src: &[f32], w: usize, h: usize, along_x: bool| -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for o in -lo..(size as isize - lo) {
                    acc += if along_x { src[y * w + reflect(x as isize + o, w)] } else { src[reflect(y as isize + o, h) * w + x] };
                }
                out[y * w + x] = acc / size as f32;
            }
        }
        out
    };
    let a = pass(&g.d, g.w, g.h, true);
    Grid { w: g.w, h: g.h, d: pass(&a, g.w, g.h, false) }
}

/// Наибольшее (`max = true`) или наименьшее в квадрате `size` — фильтры Pillow
/// MaxFilter/MinFilter по маске; за краем — край.
fn rank(m: &[bool], w: usize, h: usize, size: usize, max: bool) -> Vec<bool> {
    let r = (size / 2) as isize;
    let pass = |src: &[bool], along_x: bool| -> Vec<bool> {
        let mut out = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = !max;
                for o in -r..=r {
                    let v = if along_x { src[y * w + (x as isize + o).clamp(0, w as isize - 1) as usize] } else { src[(y as isize + o).clamp(0, h as isize - 1) as usize * w + x] };
                    if max { acc |= v } else { acc &= v }
                }
                out[y * w + x] = acc;
            }
        }
        out
    };
    let a = pass(m, true);
    pass(&a, false)
}

/// Евклидово расстояние каждой точки маски до ближайшей точки вне её
/// (`distance_transform_edt`), в пикселях. Фельценшвальб — Хуттенлохер.
fn edt(m: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f64 = 1e20;
    fn line(f: &[f64]) -> Vec<f64> {
        let n = f.len();
        let mut d = vec![0.0; n];
        if n == 0 {
            return d;
        }
        let (mut v, mut z) = (vec![0usize; n], vec![0.0f64; n + 1]);
        let mut k = 0usize;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        let cross = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * q as f64 - 2.0 * p as f64);
        for q in 1..n {
            let mut s = cross(q, v[k]);
            while s <= z[k] {
                k -= 1;
                s = cross(q, v[k]);
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f64::INFINITY;
        }
        k = 0;
        for (q, dq) in d.iter_mut().enumerate() {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *dq = (q as f64 - p as f64).powi(2) + f[p];
        }
        d
    }
    let mut g: Vec<f64> = m.iter().map(|&b| if b { FAR } else { 0.0 }).collect();
    for x in 0..w {
        let col: Vec<f64> = (0..h).map(|y| g[y * w + x]).collect();
        for (y, v) in line(&col).into_iter().enumerate() {
            g[y * w + x] = v;
        }
    }
    for y in 0..h {
        let row = line(&g[y * w..(y + 1) * w]);
        g[y * w..(y + 1) * w].copy_from_slice(&row);
    }
    g.into_iter().map(|v| if v >= FAR / 2.0 { ((w * w + h * h) as f64).sqrt() as f32 } else { v.sqrt() as f32 }).collect()
}

/// Процентиль с линейной интерполяцией (как у numpy).
fn percentile(vals: &[f32], q: f32) -> f32 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut v = vals.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let pos = q / 100.0 * (v.len() - 1) as f32;
    let (i, t) = (pos.floor() as usize, pos - pos.floor());
    if i + 1 < v.len() { v[i] * (1.0 - t) + v[i + 1] * t } else { v[i] }
}

/// Оттенок (градусы), насыщенность и яркость 0..1 — как `convert("HSV")` в Pillow,
/// с его ступенями 1/255.
fn hsv(img: &RgbImage) -> (Grid, Grid, Grid) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (mut hh, mut ss, mut vv) = (Grid::new(w, h, 0.0), Grid::new(w, h, 0.0), Grid::new(w, h, 0.0));
    for (k, p) in img.pixels().enumerate() {
        let [r, g, b] = p.0;
        let (maxc, minc) = (r.max(g).max(b), r.min(g).min(b));
        vv.d[k] = maxc as f32 / 255.0;
        if maxc == minc {
            continue;
        }
        let cr = (maxc - minc) as f32;
        let s = cr / maxc as f32;
        let (rc, gc, bc) = ((maxc - r) as f32 / cr, (maxc - g) as f32 / cr, (maxc - b) as f32 / cr);
        let hue = if r == maxc {
            bc - gc
        } else if g == maxc {
            2.0 + rc - bc
        } else {
            4.0 + gc - rc
        };
        let hue = (hue / 6.0 + 1.0).rem_euclid(1.0);
        hh.d[k] = ((hue * 255.0) as i32).clamp(0, 255) as f32 * 360.0 / 255.0;
        ss.d[k] = ((s * 255.0) as i32).clamp(0, 255) as f32 / 255.0;
    }
    (hh, ss, vv)
}

// ---------- паспорт снимка ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Biome {
    Green,
    Dry,
    Barren,
    Ice,
    Bog,
}

/// Биом снимка (photo.py): доли растительности, сухого, серого, снега и льда на
/// суше; болото — по мокрым землям Copernicus (`waw`, коды 3 и 4).
fn biome(img: &RgbImage, waw: Option<&[u8]>, snow_ok: bool) -> Biome {
    let (h, s, v) = hsv(img);
    let (vb, sb) = (gauss(&v, 1.5), gauss(&s, 1.5));
    let gv3 = gauss(&v, 3.0);
    let rough = Grid { w: v.w, h: v.h, d: v.d.iter().zip(&gv3.d).map(|(a, b)| ((a - b).abs() * 4.0).clamp(0.0, 1.0)).collect() };
    let texture = gauss(&rough, 6.0);
    let v8 = gauss(&v, 8.0);
    let n = v.d.len();
    let water: Vec<bool> = (0..n).map(|k| (v8.d[k] < 0.13 && texture.d[k] < 0.12) || (h.d[k] > 165.0 && h.d[k] < 250.0 && s.d[k] > 0.3) || (h.d[k] > 150.0 && h.d[k] < 250.0 && s.d[k] > 0.75)).collect();
    let land = water.iter().filter(|w| !**w).count().max(1) as f32;
    let share = |f: &dyn Fn(usize) -> bool| (0..n).filter(|&k| !water[k] && f(k)).count() as f32 / land;
    let green = share(&|k| sb.d[k] >= 0.25 && (60.0..=150.0).contains(&h.d[k]));
    let straw = share(&|k| sb.d[k] >= 0.25 && h.d[k] < 62.0);
    let grey = share(&|k| sb.d[k] < 0.2);
    let snow_at = |k: usize| vb.d[k] > 0.78 && sb.d[k] < 0.12;
    let snow = share(&snow_at);
    let ice = share(&|k| vb.d[k] > 0.55 && sb.d[k] < 0.16 && (80.0..=230.0).contains(&h.d[k]) && !snow_at(k));
    let mut out = if ice + snow > 0.2 && snow_ok {
        Biome::Ice
    } else if grey > 0.5 && green < 0.2 {
        Biome::Barren
    } else if straw > 0.3 && green < 0.45 {
        Biome::Dry
    } else {
        Biome::Green
    };
    if let Some(waw) = waw {
        let known: Vec<u8> = waw.iter().copied().filter(|&c| c < 254).collect();
        if known.len() * 2 > waw.len() {
            let wet = known.iter().filter(|c| matches!(c, 3 | 4)).count() as f32 / known.len() as f32;
            if out != Biome::Ice && wet > 0.25 {
                out = Biome::Bog;
            }
        }
    }
    out
}

// ---------- классы подсказки ----------

const GRASS: u8 = 0;
const TREES: u8 = 1;
const BROWN: u8 = 2;
const ROCK: u8 = 3;
const WATER: u8 = 4;
const DRY: u8 = 5;
const EARTH: u8 = 6;
const SNOW: u8 = 7;
const ICE: u8 = 8;
const BOG: u8 = 9;

/// Классы по пикселю (cover.py): кроны, трава, бурое, сухое, камень, вода; вне
/// зелёного биома — земля, снег и лёд, на болоте — мох.
fn classify(img: &RgbImage, biome: Biome, forest: Option<&[bool]>) -> Vec<u8> {
    let green_biome = biome == Biome::Green;
    let (h, s, v) = hsv(img);
    let (w, hgt) = (v.w, v.h);
    let n = w * hgt;
    let (vb, vbig) = (blur8(&v, 1.5), blur8(&v, 8.0));
    let b3 = blur8(&v, 3.0);
    let rough = Grid { w, h: hgt, d: v.d.iter().zip(&b3.d).map(|(a, b)| ((a - b).abs() * 4.0).clamp(0.0, 1.0)).collect() };
    let texture = blur8(&rough, 6.0);
    let fine = blur8(&rough, 3.0);
    let mut water: Vec<bool> = (0..n).map(|k| (vbig.d[k] < 0.13 && texture.d[k] < 0.12) || (h.d[k] > 165.0 && h.d[k] < 250.0 && s.d[k] > 0.3) || (h.d[k] > 150.0 && h.d[k] < 250.0 && s.d[k] > 0.75)).collect();
    // Ельник в тени тёмен и гладок, как вода: где Copernicus видит лес — не вода.
    if let Some(f) = forest {
        for k in 0..n {
            water[k] &= !f[k];
        }
    }
    let land: Vec<f32> = (0..n).filter(|&k| !water[k]).map(|k| v.d[k]).collect();
    let land = if land.is_empty() { v.d.clone() } else { land };
    let med = percentile(&land, 50.0);
    let sb = blur8(&s, 3.0);
    let vb10 = blur8(&vb, 10.0);
    let dark: Vec<bool> = (0..n)
        .map(|k| {
            let lit = h.d[k] > 105.0 && h.d[k] < 135.0 && s.d[k] > 0.35 && fine.d[k] > 0.11 && vb.d[k] > med * 0.6;
            let mut shade = vb.d[k] < med * 0.72;
            if !green_biome {
                shade &= (62.0..=170.0).contains(&h.d[k]) && sb.d[k] >= 0.15;
                shade &= vb.d[k] < vb10.d[k] * 0.85;
                if biome == Biome::Ice {
                    shade &= sb.d[k] >= 0.25;
                }
            }
            (shade || lit) && !water[k]
        })
        .collect();
    // Закрытие сливает тень и свет одной кроны, открытие снимает тонкое (ограды с тенью).
    let closed = rank(&rank(&dark, w, hgt, 5, true), w, hgt, 5, false);
    let mut trees = rank(&rank(&closed, w, hgt, 9, false), w, hgt, 9, true);
    for k in 0..n {
        trees[k] &= !water[k];
    }
    let p75 = percentile(&land, 75.0);
    let mut cls = vec![if green_biome { GRASS } else if biome == Biome::Ice { ROCK } else { EARTH }; n];
    for k in 0..n {
        let warm = h.d[k] < 62.0 && s.d[k] > 0.28 && vb.d[k] > 0.12 && !trees[k] && !water[k];
        let dry = warm && vb.d[k] > med * 1.05;
        let brown = warm && !dry;
        let rock = if green_biome {
            ((s.d[k] < 0.2 && vb.d[k] > p75) || (sb.d[k] < 0.27 && texture.d[k] > 0.13 && vb.d[k] > med * 0.85)) && !water[k]
        } else {
            sb.d[k] < 0.2 && (h.d[k] >= 62.0 || sb.d[k] < 0.1) && texture.d[k] > 0.13 && !water[k]
        };
        if !green_biome && sb.d[k] >= 0.25 && (60.0..=150.0).contains(&h.d[k]) {
            cls[k] = GRASS;
        }
        if brown {
            cls[k] = BROWN;
        }
        if dry {
            cls[k] = DRY;
        }
        if rock {
            cls[k] = ROCK;
        }
        if biome == Biome::Ice {
            let snow = vb.d[k] > 0.78 && sb.d[k] < 0.12 && !water[k];
            let ice = vb.d[k] > 0.55 && sb.d[k] < 0.16 && (80.0..=230.0).contains(&h.d[k]) && texture.d[k] < 0.2 && !snow && !water[k];
            if ice {
                cls[k] = ICE;
            }
            if snow {
                cls[k] = SNOW;
            }
        }
        if biome == Biome::Bog {
            // Мох; кочки (темнее середины) — бурое, выгоревший мох — сухое, сфагнум — трава.
            cls[k] = BOG;
            if vb.d[k] < med * 0.85 {
                cls[k] = BROWN;
            }
            if vb.d[k] > med * 1.12 {
                cls[k] = DRY;
            }
            if sb.d[k] >= 0.2 && (60.0..=150.0).contains(&h.d[k]) {
                cls[k] = GRASS;
            }
        }
        if trees[k] {
            cls[k] = TREES;
        }
        if water[k] {
            cls[k] = WATER;
        }
    }
    cls
}

// ---------- кроны и порода ----------

const CANOPY: (f32, f32) = (4.0 * CELL_M, 6.0 * CELL_M);
const LONE: f32 = 7.0 * CELL_M;
const GROW: f32 = 1.2;
const OVERLAP: f32 = 0.75;
const MIN: f32 = 0.9 * CELL_M;
const BUSH: f32 = 1.3 * CELL_M;
const CONIFER_CANOPY: (f32, f32) = (1.4 * CELL_M, 2.2 * CELL_M);
const CONIFER_LONE: f32 = 2.8 * CELL_M;
const DENSE_CANOPY: (f32, f32) = (2.4 * CELL_M, 3.2 * CELL_M);
const DENSE_LONE: f32 = 3.5 * CELL_M;

/// Доля хвойных в лесу NFI по его типу (trees.py).
fn nfi_conifer(kind: &str) -> f32 {
    match kind {
        "Conifer" => 0.9,
        "Mixed mainly conifer" => 0.65,
        "Mixed mainly broadleaved" => 0.35,
        "Broadleaved" | "Coppice" => 0.0,
        "Young trees" | "Felled" | "Ground prep" => 0.4,
        "Coppice with standards" => 0.05,
        "Shrub" => 0.1,
        _ => 0.2,
    }
}

/// Простой воспроизводимый случайный ряд (xorshift) для разброса крон полога.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Кроны по одной (trees.py `find`): центр — самая глубокая точка пятна, радиус —
/// глубина ×1,2 с пределом полога; [(x, y, r в метрах)].
fn find(trees: &[bool], w: usize, h: usize, mpp: f32, needles: Option<&[f32]>, dense: Option<&[bool]>) -> Vec<(usize, usize, f32)> {
    let depth: Vec<f32> = edt(trees, w, h).into_iter().map(|d| d * mpp).collect();
    let mut cand: Vec<usize> = (0..w * h).filter(|&k| trees[k] && depth[k] >= MIN).collect();
    cand.sort_by(|&a, &b| depth[b].total_cmp(&depth[a]).then(a.cmp(&b)));
    let mut rng = Rng(31);
    let size = 8.0 * CELL_M / mpp;
    let reach = (2.0 * LONE / mpp / size).ceil() as i64;
    let mut out: Vec<(usize, usize, f32)> = Vec::new();
    let mut bucket: std::collections::HashMap<(i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for k in cand {
        let (x, y, inner) = (k % w, k / w, depth[k]);
        let (bx, by) = ((x as f32 / size) as i64, (y as f32 / size) as i64);
        let near: Vec<usize> = (-reach..=reach).flat_map(|dx| (-reach..=reach).map(move |dy| (bx + dx, by + dy))).filter_map(|key| bucket.get(&key)).flatten().copied().collect();
        let dist: Vec<f32> = near.iter().map(|&j| ((out[j].0 as f32 - x as f32).hypot(out[j].1 as f32 - y as f32)) * mpp).collect();
        if near.iter().zip(&dist).any(|(&j, &d)| d < out[j].2) {
            continue;
        }
        let ring = (1.6 * inner + 0.5 * CELL_M) / mpp;
        let around = (0..16)
            .filter(|i| {
                let a = *i as f32 * std::f32::consts::PI / 8.0;
                let (px, py) = ((x as f32 + ring * a.cos()) as i64, (y as f32 + ring * a.sin()) as i64);
                px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h && trees[py as usize * w + px as usize]
            })
            .count();
        let kn = needles.map_or(0.0, |n| n[k]);
        let thick = dense.is_some_and(|d| d[k]);
        let (broad_lone, broad) = if thick { (DENSE_LONE, DENSE_CANOPY) } else { (LONE, CANOPY) };
        let lone = broad_lone + (CONIFER_LONE - broad_lone) * kn;
        let (lo, hi) = (broad.0 + (CONIFER_CANOPY.0 - broad.0) * kn, broad.1 + (CONIFER_CANOPY.1 - broad.1) * kn);
        let cap = if inner * GROW <= lone && around < 12 { lone } else { lo + (hi - lo) * rng.next() };
        let r = (inner * GROW).min(cap);
        if near.iter().zip(&dist).any(|(&j, &d)| d < (OVERLAP * (r + out[j].2)).max(out[j].2 + 0.5 * r)) {
            continue;
        }
        bucket.entry((bx, by)).or_default().push(out.len());
        out.push((x, y, r));
    }
    out
}

/// Плавный шум места, ячейка ~25 м (trees.py `clumps`): хвойные куртинами.
fn clumps(x: f32, y: f32, mpp: f32) -> f32 {
    let cell = 25.0 / mpp;
    let (gx, gy) = (x / cell, y / cell);
    let (x0, y0) = (gx.floor() as i64, gy.floor() as i64);
    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let hsh = |i: i64, j: i64| -> f32 {
        let mut v = (i.wrapping_mul(374761393).wrapping_add(j.wrapping_mul(668265263)).wrapping_add(5 * 2246822519)) & 0xFFFF_FFFF;
        v = ((v ^ (v >> 13)).wrapping_mul(1274126177)) & 0xFFFF_FFFF;
        ((v ^ (v >> 16)) & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0
    };
    let top = hsh(x0, y0) * (1.0 - sx) + hsh(x0 + 1, y0) * sx;
    let bottom = hsh(x0, y0 + 1) * (1.0 - sx) + hsh(x0 + 1, y0 + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

fn zscore(a: &[f32]) -> Vec<f32> {
    let med = percentile(a, 50.0);
    let spread = percentile(a, 75.0) - percentile(a, 25.0);
    a.iter().map(|v| (v - med) / (spread / 1.35 + 1e-9)).collect()
}

/// Леса NFI в пикселях снимка: номер леса в каждом пикселе (0 — вне) и типы.
struct Nfi {
    ids: Vec<u32>,
    types: Vec<String>,
}

/// Кроны с породой (trees.py `run`): размер, место, порода по NFI (Англия) или
/// Copernicus (вне Англии), очередь на хвойную — тёмные и холодные среди явной
/// зелени и куртинами.
fn crowns(img: &RgbImage, cls: &[u8], frame: &Frame, dlt: Option<&[u8]>, nfi: Option<&Nfi>, lc: Option<&[u8]>) -> Vec<Crown> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let n = w * h;
    let mpp = (frame.width_m() / w as f64) as f32;
    let trees: Vec<bool> = cls.iter().map(|&c| c == TREES).collect();
    let (mut needles, mut dense) = (None, None);
    // Вне Европы густой полог — где покров Esri видит лес.
    if let (None, None, Some(lc)) = (dlt, nfi, lc) {
        let win = ((20.0 / mpp).round() as usize).max(3);
        let forest = uniform(&Grid { w, h, d: lc.iter().map(|&c| (c == LC_TREES) as u8 as f32).collect() }, win);
        dense = Some((0..n).map(|k| forest.d[k] > 0.5).collect::<Vec<bool>>());
    }
    if let (Some(dlt), None) = (dlt, nfi) {
        let win = ((20.0 / mpp).round() as usize).max(3);
        let forest = uniform(&Grid { w, h, d: dlt.iter().map(|&c| matches!(c, 1 | 2) as u8 as f32).collect() }, win);
        let conif = uniform(&Grid { w, h, d: dlt.iter().map(|&c| (c == 2) as u8 as f32).collect() }, win);
        needles = Some((0..n).map(|k| if forest.d[k] > 0.2 { conif.d[k] / forest.d[k].max(1e-6) } else { 0.0 }).collect::<Vec<f32>>());
        dense = Some((0..n).map(|k| forest.d[k] > 0.5).collect::<Vec<bool>>());
    }
    let found = find(&trees, w, h, mpp, needles.as_deref(), dense.as_deref());
    // Признаки кроны (trees.py `features`): что нужно породе.
    let (hh, _, v) = hsv(img);
    let exg: Vec<f32> = img.pixels().map(|p| (2.0 * p.0[1] as f32 - p.0[0] as f32 - p.0[2] as f32) / 255.0).collect();
    let sm = gauss(&Grid { w, h, d: v.d.iter().zip(&gauss(&v, 3.0).d).map(|(a, b)| (a - b).abs()).collect() }, 6.0);
    let med_pool: Vec<f32> = (0..n).filter(|&k| !(v.d[k] < 0.13 && sm.d[k] < 0.03) && !trees[k]).map(|k| v.d[k]).collect();
    let med = if med_pool.is_empty() { percentile(&v.d, 50.0) } else { percentile(&med_pool, 50.0) };
    struct Feat {
        h: f32,
        light_top: f32,
        exg_top: f32,
    }
    let feats: Vec<Feat> = found
        .iter()
        .map(|&(x, y, r)| {
            let rp = (0.8 * r / mpp).max(1.5);
            let mut px = Vec::new();
            for yy in ((y as f32 - rp).max(0.0) as usize)..((y as f32 + rp + 1.0) as usize).min(h) {
                for xx in ((x as f32 - rp).max(0.0) as usize)..((x as f32 + rp + 1.0) as usize).min(w) {
                    if ((xx as f32 - x as f32).powi(2) + (yy as f32 - y as f32).powi(2)) <= rp * rp {
                        px.push(yy * w + xx);
                    }
                }
            }
            let vv: Vec<f32> = px.iter().map(|&k| v.d[k]).collect();
            let p75 = percentile(&vv, 75.0);
            let bright: Vec<f32> = px.iter().filter(|&&k| v.d[k] > p75).map(|&k| exg[k]).collect();
            let all_exg = px.iter().map(|&k| exg[k]).sum::<f32>() / px.len().max(1) as f32;
            Feat {
                h: px.iter().map(|&k| hh.d[k]).sum::<f32>() / px.len().max(1) as f32,
                light_top: percentile(&vv, 90.0) / med.max(1e-6),
                exg_top: if bright.is_empty() { all_exg } else { bright.iter().sum::<f32>() / bright.len() as f32 },
            }
        })
        .collect();
    // Порода (trees.py `species`).
    let grass: Vec<f32> = (0..n).filter(|&k| cls[k] == GRASS).map(|k| exg[k]).collect();
    let grass_exg = if grass.is_empty() { 0.1 } else { percentile(&grass, 50.0) };
    let big: Vec<f32> = found.iter().zip(&feats).filter(|((_, _, r), _)| *r >= BUSH).map(|(_, f)| f.exg_top).collect();
    let green = if big.is_empty() { 0.0 } else { percentile(&big, 50.0) };
    let trusted = green >= 1.1 * grass_exg;
    let valid: Vec<usize> = (0..found.len()).filter(|&i| found[i].2 >= BUSH && (70.0..=160.0).contains(&feats[i].h) && feats[i].exg_top >= 0.8 * grass_exg).collect();
    let mut score = vec![0.0f32; found.len()];
    if trusted && valid.len() >= 10 {
        let a = zscore(&valid.iter().map(|&i| feats[i].light_top).collect::<Vec<_>>());
        let b = zscore(&valid.iter().map(|&i| feats[i].h).collect::<Vec<_>>());
        for (j, &i) in valid.iter().enumerate() {
            score[i] = (-a[j] + 0.5 * b[j]).clamp(-3.0, 3.0) / 3.0;
        }
    }
    let queue: Vec<f32> = found.iter().zip(&score).map(|(&(x, y, _), s)| clumps(x as f32, y as f32, mpp) + 0.5 * s).collect();
    let mut order: Vec<usize> = (0..found.len()).collect();
    order.sort_by(|&a, &b| queue[b].total_cmp(&queue[a]));
    let mut rank = vec![0.0f32; found.len()];
    for (pos, &i) in order.iter().enumerate() {
        rank[i] = pos as f32 / (found.len().max(2) - 1) as f32;
    }
    let mut out: Vec<(String, String)> = found.iter().map(|&(_, _, r)| ((if r < BUSH { "bush" } else { "broad" }).to_owned(), "none".to_owned())).collect();
    if let (Some(dlt), None) = (dlt, nfi) {
        let reach = ((10.0 / mpp).round() as usize).max(1);
        for (i, &(x, y, r)) in found.iter().enumerate() {
            let (mut forest, mut conif, mut all) = (0usize, 0usize, 0usize);
            for yy in y.saturating_sub(reach)..(y + reach + 1).min(h) {
                for xx in x.saturating_sub(reach)..(x + reach + 1).min(w) {
                    let c = dlt[yy * w + xx];
                    all += 1;
                    forest += matches!(c, 1 | 2) as usize;
                    conif += (c == 2) as usize;
                }
            }
            if (forest as f32) < 0.2 * all as f32 {
                continue;
            }
            let share = conif as f32 / forest as f32;
            // Малая крона в ельнике — молодая ель, а не куст.
            if r < BUSH && share < 0.6 {
                continue;
            }
            let kind = if rank[i] < share { "conifer" } else if r < BUSH { "bush" } else { "broad" };
            out[i] = (kind.to_owned(), format!("dlt:{share:.2}"));
        }
    }
    if let Some(nfi) = nfi {
        let mut groups: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
        for (i, &(x, y, r)) in found.iter().enumerate() {
            let id = nfi.ids[y * w + x];
            if r >= BUSH && id > 0 {
                groups.entry(id).or_default().push(i);
            }
        }
        for (id, mut members) in groups {
            let kind = &nfi.types[id as usize];
            let share = nfi_conifer(kind);
            members.sort_by(|&a, &b| queue[b].total_cmp(&queue[a]));
            let take = (share * members.len() as f32).round() as usize;
            for (k, &i) in members.iter().enumerate() {
                out[i] = ((if k < take { "conifer" } else { "broad" }).to_owned(), format!("nfi:{kind}"));
            }
        }
    }
    found
        .iter()
        .zip(out)
        .enumerate()
        .map(|(i, (&(x, y, r), (kind, src)))| Crown::new((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32, r, kind, src, rank[i]))
        .collect()
}

// ---------- загрузка ----------

/// Снимок мельче 0,3 м ужимается до 0,3: правила мерены на 0,35–0,48 м.
const REF_MPP: f64 = 0.3;
/// Грубее этого снимок растягивается до [`COARSE_TO`]: размытия разбора заданы
/// в точках, и на z17 (1,2 м) протока в 6 м тонула в соседнем лесу.
const COARSE_MPP: f64 = 0.6;
const COARSE_TO: f64 = 0.5;
/// Крупный масштаб — только боевым участкам (сторона до 200 м).
const BATTLE_M: f64 = 200.0;
pub(crate) const TILE_URL: &str = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile";

pub(crate) fn mpp_at(lat: f64, z: u32) -> f64 {
    156_543.033_92 * lat.to_radians().cos() / 2f64.powi(z as i32)
}

pub(crate) fn tile_px(lat: f64, lon: f64, z: u32) -> (f64, f64) {
    let n = 2f64.powi(z as i32) * 256.0;
    let s = lat.to_radians().sin();
    ((lon + 180.0) / 360.0 * n, (0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * n)
}

/// Плитка-заглушка Esri «Map data not yet available»: почти вся ровно серая 204.
pub(crate) fn blank(img: &RgbImage) -> bool {
    let small = image::imageops::resize(img, 64, 64, image::imageops::FilterType::Triangle);
    let grey = small.pixels().filter(|p| p.0.iter().all(|&c| (c as i32 - 204).abs() <= 4)).count();
    grey as f32 / (64.0 * 64.0) > 0.6
}

/// Что программа знает о месте, пока собирает его, — для окна.
pub enum Msg {
    Step(String),
    Done(Box<Result<Place, String>>),
}

/// Собранное место: выгрузка OSM, докачанные контуры и подсказка со снимка.
pub struct Place {
    pub xml: String,
    pub extra: Vec<String>,
    pub cover: Cover,
    pub folder: PathBuf,
    pub summary: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod net {
    use std::time::Duration;

    pub type Agent = ureq::Agent;

    pub fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(10))).timeout_global(Some(Duration::from_secs(90))).build().into()
    }

    pub fn get(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
        let ua = format!("Kartograf/{} (terrain import)", env!("CARGO_PKG_VERSION"));
        let mut r = agent.get(url).header("User-Agent", &ua).call().map_err(|e| format!("{url}: {e}"))?;
        r.body_mut().with_config().limit(200 << 20).read_to_vec().map_err(|e| format!("{url}: {e}"))
    }
}

/// В модуле сеть — браузерная: `kartografFetch` в `foundry/scripts/main.js` берёт
/// заранее скачанное ([`urls`]), а нет — качает синхронно.
#[cfg(target_arch = "wasm32")]
mod net {
    pub struct Agent;

    pub fn agent() -> Agent {
        Agent
    }

    pub fn get(_: &Agent, url: &str) -> Result<Vec<u8>, String> {
        crate::plat::net_fetch(url).map_err(|e| format!("{url}: {e}"))
    }
}

/// Файл из папки места, а нет — из сети (`agent`), и сразу в папку. Без сети —
/// только то, что уже лежит. В модуле папок нет (`std::fs` там отвечает
/// ошибкой) — всегда сеть.
fn cached(agent: Option<&net::Agent>, path: &Path, url: &str) -> Result<Vec<u8>, String> {
    if let Ok(b) = std::fs::read(path) {
        return Ok(b);
    }
    let Some(agent) = agent else { return Err(format!("нет {}", path.display())) };
    let b = net::get(agent, url)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, &b);
    Ok(b)
}

fn tile_url(z: u32, tx: i64, ty: i64) -> String {
    format!("{TILE_URL}/{z}/{ty}/{tx}")
}

fn tile(agent: Option<&net::Agent>, cache: &Path, z: u32, tx: i64, ty: i64) -> Result<RgbImage, String> {
    let b = cached(agent, &cache.join(format!("{z}_{tx}_{ty}.jpg")), &tile_url(z, tx, ty))?;
    image::load_from_memory(&b).map(|i| i.to_rgb8()).map_err(|e| format!("плитка {z}/{ty}/{tx}: {e}"))
}

/// Плитки рамки на масштабе `z`.
fn span(frame: &Frame, z: u32) -> (f64, f64, f64, f64) {
    let (x0, y0) = tile_px(frame.north, frame.west, z);
    let (x1, y1) = tile_px(frame.south, frame.east, z);
    (x0, y0, x1, y1)
}

/// Все плитки рамки на масштабе `z`.
fn tiles_of(frame: &Frame, z: u32) -> Vec<(i64, i64)> {
    let (x0, y0, x1, y1) = span(frame, z);
    ((y0 / 256.0) as i64..=(y1 / 256.0) as i64).flat_map(|ty| ((x0 / 256.0) as i64..=(x1 / 256.0) as i64).map(move |tx| (tx, ty))).collect()
}

/// Мельче этого снимок не ищется: z12 — 38 м на точку, дальше рамка — пара точек.
const ZOOM_LOW: u32 = 12;

/// Масштабы, из которых [`zoom`] выбирает, крупные первыми. Крупнее 18 — только
/// боевым участкам и не больше 64 плиток. Мельче 18 — где Esri на крупных отдаёт
/// заглушку «нет снимка»: в Амазонии снимок есть только с z17, и заглушку разбор
/// принимал за снег.
fn zooms(frame: &Frame) -> Vec<u32> {
    let battle = frame.width_m().max(frame.height_m()) <= BATTLE_M;
    (ZOOM_LOW..=20).rev().filter(|&z| z <= 18 || (battle && tiles_of(frame, z).len() <= 64)).collect()
}

/// Самый крупный масштаб, где у рамки есть снимок (все плитки — не заглушки).
fn zoom(agent: Option<&net::Agent>, cache: &Path, frame: &Frame) -> Result<u32, String> {
    for z in zooms(frame) {
        let (x0, y0, x1, y1) = span(frame, z);
        // Без сети и без плитки этого масштаба в кеше — масштаб пропускается.
        let Ok(mid) = tile(agent, cache, z, ((x0 + x1) / 2.0 / 256.0) as i64, ((y0 + y1) / 2.0 / 256.0) as i64) else { continue };
        if blank(&mid) {
            continue;
        }
        let mut ok = true;
        for (tx, ty) in tiles_of(frame, z) {
            if blank(&tile(agent, cache, z, tx, ty)?) {
                ok = false;
                break;
            }
        }
        if ok {
            return Ok(z);
        }
    }
    Ok(18)
}

/// Снимок по рамке: самый крупный масштаб, ужатый до 0,3 м, если он мельче.
fn photo(agent: Option<&net::Agent>, cache: &Path, frame: &Frame) -> Result<(RgbImage, u32), String> {
    let z = zoom(agent, cache, frame)?;
    let (x0, y0, x1, y1) = span(frame, z);
    let (tx0, ty0, tx1, ty1) = ((x0 / 256.0) as i64, (y0 / 256.0) as i64, (x1 / 256.0) as i64, (y1 / 256.0) as i64);
    let mut canvas = RgbImage::new(((tx1 - tx0 + 1) * 256) as u32, ((ty1 - ty0 + 1) * 256) as u32);
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            image::imageops::replace(&mut canvas, &tile(agent, cache, z, tx, ty)?, (tx - tx0) * 256, (ty - ty0) * 256);
        }
    }
    let (bx, by) = ((x0 - tx0 as f64 * 256.0) as u32, (y0 - ty0 as f64 * 256.0) as u32);
    let (bw, bh) = (((x1 - tx0 as f64 * 256.0) as u32).saturating_sub(bx).max(8), ((y1 - ty0 as f64 * 256.0) as u32).saturating_sub(by).max(8));
    let mut img = image::imageops::crop_imm(&canvas, bx, by, bw, bh).to_image();
    let m = mpp_at(frame.mid_lat(), z);
    let k = if m < REF_MPP {
        m / REF_MPP
    } else if m > COARSE_MPP {
        m / COARSE_TO
    } else {
        1.0
    };
    if k != 1.0 {
        let filter = if k < 1.0 { image::imageops::FilterType::Lanczos3 } else { image::imageops::FilterType::Triangle };
        img = image::imageops::resize(&img, ((img.width() as f64 * k).round() as u32).max(1), ((img.height() as f64 * k).round() as u32).max(1), filter);
    }
    Ok((img, z))
}

/// Слой Copernicus HRL 2018 (eu.py) по рамке, растянутый до `w`×`h` ближайшим
/// соседом: `dlt` — порода (1 лиственное, 2 хвойное), `waw` — вода и мокрые земли.
/// Пустые плитки TIFF (над лесом в Альпах) — «нет данных» (255).
fn copernicus_size(frame: &Frame) -> (usize, usize) {
    (((frame.width_m() / 2.0).round() as usize).max(4), ((frame.height_m() / 2.0).round() as usize).max(4))
}

fn copernicus_url(frame: &Frame, layer: &str) -> String {
    let service = if layer == "dlt" { "HRL_DominantLeafType2018" } else { "HRL_WaterWetness_2018" };
    let (sw, sh) = copernicus_size(frame);
    format!(
        "https://image.discomap.eea.europa.eu/arcgis/rest/services/GioLandPublic/{service}/ImageServer/exportImage?bbox={},{},{},{}&bboxSR=4326&imageSR=4326&size={sw},{sh}&format=tiff&pixelType=U8&compression=None&interpolation=RSP_NearestNeighbor&f=image",
        frame.west, frame.south, frame.east, frame.north
    )
}

fn copernicus(agent: Option<&net::Agent>, file: &Path, frame: &Frame, layer: &str, w: usize, h: usize) -> Result<Vec<u8>, String> {
    let (sw, sh) = copernicus_size(frame);
    let b = cached(agent, file, &copernicus_url(frame, layer))?;
    let src = image::load_from_memory_with_format(&b, image::ImageFormat::Tiff).map(|i| i.to_luma8());
    let src = match src {
        Ok(img) => img,
        Err(_) => image::GrayImage::from_pixel(sw as u32, sh as u32, image::Luma([255])),
    };
    Ok(nearest(&src, w, h))
}

/// Слой, растянутый до `w`×`h` ближайшим соседом.
fn nearest(src: &image::GrayImage, w: usize, h: usize) -> Vec<u8> {
    let (iw, ih) = (src.width() as usize, src.height() as usize);
    (0..w * h)
        .map(|k| {
            let (x, y) = (k % w, k / w);
            let (sx, sy) = (((x as f32 + 0.5) * iw as f32 / w as f32) as usize, ((y as f32 + 0.5) * ih as f32 / h as f32) as usize);
            src.get_pixel(sx.min(iw - 1) as u32, sy.min(ih - 1) as u32).0[0]
        })
        .collect()
}

/// Земной покров Esri (Sentinel-2, 10 м, весь мир): 1 — вода, 2 — деревья,
/// 4 — затопленная зелень, 5 — поля, 7 — застройка, 8 — голая земля, 9 — снег и
/// лёд, 10 — облака, 11 — луга и кусты; 0 — нет данных.
const LC_TREES: u8 = 2;
const LC_SNOW: u8 = 9;

fn landcover_url(frame: &Frame) -> String {
    let (sw, sh) = copernicus_size(frame);
    format!(
        "https://ic.imagery1.arcgis.com/arcgis/rest/services/Sentinel2_10m_LandCover/ImageServer/exportImage?bbox={},{},{},{}&bboxSR=4326&imageSR=4326&size={sw},{sh}&format=tiff&pixelType=U8&interpolation=RSP_NearestNeighbor&f=image",
        frame.west, frame.south, frame.east, frame.north
    )
}

/// Покров по рамке в пикселях снимка `w`×`h`; нет данных у большей части — `None`.
fn landcover(agent: Option<&net::Agent>, file: &Path, frame: &Frame, w: usize, h: usize) -> Option<Vec<u8>> {
    let b = cached(agent, file, &landcover_url(frame)).ok()?;
    let src = image::load_from_memory_with_format(&b, image::ImageFormat::Tiff).ok()?.to_luma8();
    let lc = nearest(&src, w, h);
    (lc.iter().filter(|&&c| c != 0).count() * 2 > lc.len()).then_some(lc)
}

/// Вода, а не тень, — темнее этого (яркость 0..1) и серее [`GREY_WATER`]: озеро
/// в Амазонии — 0,07–0,09 и насыщенность 0,34; тень в пологе у Манауса бывает и
/// темнее (0,03), но густо-зелёная (0,65–0,9).
const DARK_WATER: f32 = 0.12;
const GREY_WATER: f32 = 0.5;

/// Лес по покрову, кроме того, что похоже на воду ([`dark_water`]) и вдвое
/// темнее самого леса: озеро у протоки в Амазонии — вода, хоть покров в 10 м
/// его не видит.
fn cover_forest(img: &RgbImage, lc: &[u8]) -> Vec<bool> {
    let (vs, water) = dark_water(img);
    let canopy: Vec<f32> = (0..lc.len()).filter(|&k| lc[k] == LC_TREES).map(|k| vs[k]).collect();
    let med = if canopy.is_empty() { 0.0 } else { percentile(&canopy, 50.0) };
    (0..lc.len()).map(|k| lc[k] == LC_TREES && !(water[k] && vs[k] < 0.5 * med)).collect()
}

/// Яркость снимка, чуть размытая (σ 1,5 точки), и где она похожа на тёмную воду:
/// темна и сера.
fn dark_water(img: &RgbImage) -> (Vec<f32>, Vec<bool>) {
    let (_, s, v) = hsv(img);
    let (vs, ss) = (gauss(&v, 1.5).d, gauss(&s, 1.5).d);
    let water = vs.iter().zip(&ss).map(|(&v, &s)| v < DARK_WATER && s < GREY_WATER).collect();
    (vs, water)
}

/// LC_BUILT — застройка: тень дома там не вода.
const LC_BUILT: u8 = 7;

/// Вода снимка, что ложится и без OSM: класс «вода», тёмная и серая
/// ([`dark_water`]) и не на застройке по покрову.
fn wild_water(img: &RgbImage, cls: &[u8], lc: &[u8]) -> Vec<bool> {
    let (_, water) = dark_water(img);
    (0..cls.len()).map(|k| cls[k] == WATER && water[k] && lc[k] != LC_BUILT).collect()
}

/// Где покров видит лес, а снимок кроны почти не различает (грубый снимок,
/// ровный полог джунглей), — лес сплошь. Иначе джунгли у Манауса выходили лугом
/// с двумя десятками деревьев: кроной считается лишь тень темнее 0,72 середины.
fn fill_canopy(cls: &mut [u8], lc: &[u8]) {
    let forest: Vec<usize> = (0..cls.len()).filter(|&k| lc[k] == LC_TREES).collect();
    if forest.len() * 10 < cls.len() {
        return;
    }
    let seen = forest.iter().filter(|&&k| cls[k] == TREES).count();
    if seen * 10 >= forest.len() * 4 {
        return;
    }
    for k in forest {
        if cls[k] != WATER {
            cls[k] = TREES;
        }
    }
}

/// Англия — грубая рамка: NFI есть только там.
fn in_england(frame: &Frame) -> bool {
    (49.8..=55.9).contains(&frame.mid_lat()) && (-6.5..=1.9).contains(&((frame.west + frame.east) / 2.0))
}

/// Леса NFI England 2022 по рамке (nfi.py), в пикселях снимка `w`×`h`.
fn nfi_url(frame: &Frame) -> String {
    format!(
        "https://services2.arcgis.com/mHXjwgl3OARRqqD4/arcgis/rest/services/National_Forest_Inventory_England_2022/FeatureServer/0/query?where=1%3D1&geometry={},{},{},{}&geometryType=esriGeometryEnvelope&inSR=4326&spatialRel=esriSpatialRelIntersects&outFields=IFT_IOA,CATEGORY&outSR=4326&f=geojson",
        frame.west, frame.south, frame.east, frame.north
    )
}

fn nfi(agent: Option<&net::Agent>, file: &Path, frame: &Frame, w: usize, h: usize) -> Result<Option<Nfi>, String> {
    if !in_england(frame) {
        return Ok(None);
    }
    let b = cached(agent, file, &nfi_url(frame))?;
    let json: serde_json::Value = serde_json::from_slice(&b).map_err(|e| format!("NFI: {e}"))?;
    let merc = |lat: f64| {
        let s = lat.to_radians().sin();
        ((1.0 + s) / (1.0 - s)).ln() / 2.0
    };
    let px = |lon: f64, lat: f64| -> [f64; 2] { [(lon - frame.west) / (frame.east - frame.west) * w as f64, (merc(frame.north) - merc(lat)) / (merc(frame.north) - merc(frame.south)) * h as f64] };
    let mut ids = vec![0u32; w * h];
    let mut types = vec!["вне".to_owned()];
    let fill = |ids: &mut Vec<u32>, ring: &[[f64; 2]], value: u32| {
        // Строки по центрам пикселей, отрезки по чётности пересечений.
        for y in 0..h {
            let cy = y as f64 + 0.5;
            let mut xs: Vec<f64> = Vec::new();
            for i in 0..ring.len() {
                let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                if (a[1] <= cy) != (b[1] <= cy) {
                    xs.push(a[0] + (cy - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(|a, b| a.total_cmp(b));
            for pair in xs.chunks(2) {
                if let [x0, x1] = pair {
                    for x in (x0 - 0.5).ceil().max(0.0) as usize..((x1 - 0.5).floor() + 1.0).clamp(0.0, w as f64) as usize {
                        ids[y * w + x] = value;
                    }
                }
            }
        }
    };
    for f in json["features"].as_array().into_iter().flatten() {
        types.push(f["properties"]["IFT_IOA"].as_str().unwrap_or("").to_owned());
        let id = types.len() as u32 - 1;
        let g = &f["geometry"];
        let polys: Vec<&serde_json::Value> = match g["type"].as_str() {
            Some("Polygon") => vec![&g["coordinates"]],
            Some("MultiPolygon") => g["coordinates"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
            _ => Vec::new(),
        };
        for poly in polys {
            let rings: Vec<Vec<[f64; 2]>> = poly.as_array().into_iter().flatten().map(|r| r.as_array().into_iter().flatten().filter_map(|c| Some(px(c[0].as_f64()?, c[1].as_f64()?))).collect()).collect();
            for (k, ring) in rings.iter().enumerate() {
                fill(&mut ids, ring, if k == 0 { id } else { 0 });
            }
        }
    }
    Ok(Some(Nfi { ids, types }))
}

/// Где лежит скачанное для одного места.
struct Files {
    tiles: PathBuf,
    dlt: PathBuf,
    waw: PathBuf,
    nfi: PathBuf,
    lc: PathBuf,
}

/// Подсказка со снимка по рамке: снимок, слои, биом, классы, кроны.
fn analyse(agent: Option<&net::Agent>, frame: &Frame, files: &Files, step: &dyn Fn(&str)) -> Result<(Cover, String), String> {
    step("photo");
    let (img, z) = photo(agent, &files.tiles, frame)?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    step("layers");
    let nfi = nfi(agent, &files.nfi, frame, w, h).unwrap_or(None);
    let dlt = if nfi.is_none() { copernicus(agent, &files.dlt, frame, "dlt", w, h).ok() } else { None };
    // Мокрые земли — в родном размере слоя, как в photo.py.
    let (lw, lh) = copernicus_size(frame);
    let waw = if in_england(frame) { None } else { copernicus(agent, &files.waw, frame, "waw", lw, lh).ok() };
    // Copernicus — только Европа: за её пределами весь «нет данных» (255).
    let dlt = dlt.filter(|d| d.iter().filter(|&&c| c < 254).count() * 2 > d.len());
    let waw = waw.filter(|d| d.iter().filter(|&&c| c < 254).count() * 2 > d.len());
    // Вне Европы — покров Esri: лес и снег по нему. В Европе свои слои точнее, а
    // ледник под мореной (Мер-де-Глас) покров считает голой землёй.
    let lc = if dlt.is_none() && nfi.is_none() { landcover(agent, &files.lc, frame, w, h) } else { None };
    step("analysis");
    let snow_ok = lc.as_ref().is_none_or(|l| l.iter().filter(|&&c| c == LC_SNOW).count() * 10 > l.len());
    let biome = biome(&img, waw.as_deref(), snow_ok);
    let forest: Option<Vec<bool>> = match (&dlt, &lc) {
        (Some(d), _) => Some(d.iter().map(|&c| matches!(c, 1 | 2)).collect()),
        (None, Some(l)) => Some(cover_forest(&img, l)),
        _ => None,
    };
    let mut cls = classify(&img, biome, forest.as_deref());
    if let Some(l) = &lc {
        fill_canopy(&mut cls, l);
    }
    let crowns = crowns(&img, &cls, frame, dlt.as_deref(), nfi.as_ref(), lc.as_deref());
    let summary = format!("{biome:?}, снимок z{z}, {w}×{h}, крон {}{}", crowns.len(), if lc.is_some() { ", покров Esri" } else { "" });
    let wild = lc.as_ref().map(|l| wild_water(&img, &cls, l));
    let cover = Cover::from_classes(w, h, cls, crowns).with_photo(img);
    Ok((if let Some(m) = wild { cover.with_wild_water(m) } else { cover }, summary))
}

/// Выгрузка OSM по рамке — с запасом вокруг: API отдаёт только линии с точкой
/// внутри запрошенного, и мост через реку у Болтона, чьи концы за рамкой в 60 м,
/// в карту не попадал. Рамка карты — прежняя (`<bounds>` заменяется).
fn osm_url(want: &Frame) -> String {
    let pad = (want.width_m().max(want.height_m()) * 0.25).max(30.0);
    let (plat, plon) = (pad / 110_574.0, pad / (111_320.0 * want.mid_lat().to_radians().cos()));
    format!("https://api.openstreetmap.org/api/0.6/map?bbox={:.7},{:.7},{:.7},{:.7}", want.west - plon, want.south - plat, want.east + plon, want.north + plat)
}

/// Что [`build`] заведомо скачает для места: выгрузку OSM, плитки снимка всех
/// масштабов, из которых он выбирает, слои лесов и мокрых земель. Модуль качает
/// это фоном заранее: сама сборка в браузере идёт одним куском, и сеть в ней
/// вешала бы окно. Докачку неполных контуров заранее не узнать — её сборка
/// тянет сама.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn urls(lat: f64, lon: f64, cells: [u32; 2]) -> Vec<String> {
    let want = Frame::around(lat, lon, cells);
    // Та рамка, что выйдет из `<bounds>` выгрузки, — до седьмого знака, иначе
    // адреса слоёв не совпадут.
    let frame = Frame::of_osm(&with_bounds("<osm>", &want)).unwrap_or(want);
    let mut out = vec![osm_url(&want)];
    // Мельче 18 нужны редко (где крупнее — заглушка); их сборка докачает сама.
    for z in zooms(&frame).into_iter().filter(|&z| z >= 18) {
        out.extend(tiles_of(&frame, z).into_iter().map(|(tx, ty)| tile_url(z, tx, ty)));
    }
    if in_england(&frame) {
        out.push(nfi_url(&frame));
    } else {
        out.push(copernicus_url(&frame, "dlt"));
        out.push(copernicus_url(&frame, "waw"));
        out.push(landcover_url(&frame));
    }
    out
}

/// Собрать место по координатам: всё скачанное — в папку места `cache/<широта>_<долгота>_<W>x<H>`
/// (повторно сеть не трогается), плитки снимка — в общий `cache/sat_tiles`. В
/// модуле папок нет: скачанное держит в памяти `main.js`.
pub fn build(lat: f64, lon: f64, cells: [u32; 2], cache: &Path, step: &dyn Fn(&str)) -> Result<Place, String> {
    let agent = net::agent();
    let folder = cache.join(format!("{lat:.5}_{lon:.5}_{}x{}", cells[0], cells[1]));
    #[cfg(not(target_arch = "wasm32"))]
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let want = Frame::around(lat, lon, cells);
    step("osm");
    let full = String::from_utf8(cached(Some(&agent), &folder.join("place_full.osm"), &osm_url(&want))?).map_err(|e| format!("OSM: {e}"))?;
    let xml = with_bounds(&full, &want);
    let _ = std::fs::write(folder.join("place.osm"), &xml);
    let frame = Frame::of_osm(&xml).unwrap_or(want);
    let extra: Vec<String> = crate::procgen::osm::incomplete(&xml)
        .into_iter()
        .filter_map(|id| cached(Some(&agent), &folder.join(format!("relation_{id}.osm")), &format!("https://api.openstreetmap.org/api/0.6/relation/{id}/full")).ok())
        .filter_map(|b| String::from_utf8(b).ok())
        .collect();
    let files = Files { tiles: cache.join("sat_tiles"), dlt: folder.join("dlt.tif"), waw: folder.join("waw.tif"), nfi: folder.join("nfi.geojson"), lc: folder.join("landcover.tif") };
    let (cover, summary) = analyse(Some(&agent), &frame, &files, step)?;
    let _ = cover.save_files(&folder);
    Ok(Place { xml, extra, cover, folder, summary })
}

/// `--analyse участок.osm…`: анализ снимка программой по уже скачанному (имена —
/// как у скриптов `tools/osm`), без сети, и сверка с их `.cover.png` и `.crowns.json`.
#[cfg(not(target_arch = "wasm32"))]
pub fn compare(osms: &[PathBuf]) -> i32 {
    for osm in osms {
        let stem = osm.with_extension("");
        let name = stem.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dir = osm.parent().unwrap_or(Path::new("."));
        let Some(frame) = std::fs::read_to_string(osm).ok().and_then(|x| Frame::of_osm(&x)) else {
            println!("{name}: нет рамки");
            continue;
        };
        let with = |ext: &str| PathBuf::from(format!("{}.{ext}", stem.display()));
        let files = Files { tiles: dir.join("sat_tiles"), dlt: with("dlt.tif"), waw: with("waw.tif"), nfi: dir.join("nfi").join(format!("{name}.geojson")), lc: with("landcover.tif") };
        let t = std::time::Instant::now();
        let (cover, summary) = match analyse(None, &frame, &files, &|_| {}) {
            Ok(x) => x,
            Err(e) => {
                println!("{name}: {e}");
                continue;
            }
        };
        let secs = t.elapsed().as_secs_f64();
        let out = dir.join("rust").join(&name);
        let _ = std::fs::create_dir_all(&out);
        let _ = cover.save_files(&out);
        let py = std::fs::read(with("cover.png")).ok().and_then(|b| Cover::from_image(&b).ok());
        let agree = py.map(|p| cover.agreement(&p)).map_or("—".to_owned(), |a| format!("{:.1} %", a * 100.0));
        let kinds = |list: &[(String, String)]| ["broad", "conifer", "bush"].map(|k| list.iter().filter(|c| c.0 == k).count());
        let ours = kinds(&cover.crown_kinds());
        let theirs = std::fs::read_to_string(with("crowns.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|j| j["crowns"].as_array().map(|a| a.iter().map(|c| (c["kind"].as_str().unwrap_or("").to_owned(), String::new())).collect::<Vec<_>>()).unwrap_or_default())
            .map(|l| kinds(&l));
        println!("{name}: {summary}, {secs:.1} с; классы совпали {agree}; кроны л/х/к {ours:?} против {theirs:?}; {}", cover.class_shares());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Модуль качает заранее ровно то, что спросит сборка: та же выгрузка, те же
    /// слои по рамке из `<bounds>` (до седьмого знака), плитки выбранного масштаба.
    #[test]
    fn prefetch_list_matches_what_build_asks_for() {
        for (lat, lon, cells) in [(39.4735, -0.37893, [40, 30]), (53.98, -1.89, [60, 40]), (46.0, 7.0, [256, 256])] {
            let list = urls(lat, lon, cells);
            let want = Frame::around(lat, lon, cells);
            assert_eq!(list[0], osm_url(&want));
            // Как в `build`: выгрузка с чужой рамкой, у которой `<bounds>` заменён.
            let xml = with_bounds(r#"<osm version="0.6"><bounds minlat="1" minlon="2" maxlat="3" maxlon="4"/></osm>"#, &want);
            let frame = Frame::of_osm(&xml).unwrap();
            if in_england(&frame) {
                assert!(list.contains(&nfi_url(&frame)));
            } else {
                assert!(list.contains(&copernicus_url(&frame, "dlt")) && list.contains(&copernicus_url(&frame, "waw")), "слои не совпали");
            }
            for z in zooms(&frame).into_iter().filter(|&z| z >= 18) {
                assert!(tiles_of(&frame, z).iter().all(|&(x, y)| list.contains(&tile_url(z, x, y))), "плитки z{z}");
            }
        }
        // Большой участок — с z18 и мельче; малый — с z20, вниз до z12.
        let big = zooms(&Frame::around(46.0, 7.0, [256, 256]));
        assert_eq!((big[0], *big.last().unwrap()), (18, ZOOM_LOW));
        assert_eq!(zooms(&Frame::around(46.0, 7.0, [40, 30]))[0], 20);
    }

    #[test]
    fn coordinates_come_from_numbers_and_map_links() {
        let near = |a: Option<(f64, f64)>, b: (f64, f64)| a.is_some_and(|a| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6);
        assert!(near(parse_place("59.46956, 25.65897"), (59.46956, 25.65897)));
        assert!(near(parse_place("46.55465 8.00555"), (46.55465, 8.00555)));
        assert!(near(parse_place("https://www.openstreetmap.org/#map=18/48.56370/8.21962"), (48.5637, 8.21962)));
        assert!(near(parse_place("https://www.openstreetmap.org/?mlat=51.16&mlon=-0.71#map=16/51.16/-0.71"), (51.16, -0.71)));
        assert!(near(parse_place("https://www.google.com/maps/@37.0417,-2.4149,17z"), (37.0417, -2.4149)));
        assert!(near(parse_place("https://www.google.com/maps/place/X/@1,2,3z/data=!3d39.4717!4d-5.865"), (39.4717, -5.865)));
        assert_eq!(parse_place("просто текст"), None);
        assert_eq!(parse_place("95, 10"), None);
    }

    #[test]
    fn frame_of_forty_by_thirty_cells_is_sixty_one_by_forty_six_metres() {
        let f = Frame::around(59.46956, 25.65897, [40, 30]);
        assert!((f.width_m() - 60.96).abs() < 0.01 && (f.height_m() - 45.72).abs() < 0.01, "{} × {}", f.width_m(), f.height_m());
        let xml = r#"<osm><bounds minlat="59.4693530" minlon="25.6584310" maxlat="59.4697670" maxlon="25.6595090"/></osm>"#;
        assert_eq!(Frame::of_osm(xml), Some(Frame { south: 59.469353, west: 25.658431, north: 59.469767, east: 25.659509 }));
    }

    #[test]
    fn padded_download_keeps_the_map_frame() {
        let f = Frame { south: 1.0, west: 2.0, north: 1.5, east: 2.5 };
        let xml = r#"<osm version="0.6"><bounds minlat="0.9" minlon="1.9" maxlat="1.6" maxlon="2.6"/><node id="1" lat="0.95" lon="1.95"/></osm>"#;
        let out = with_bounds(xml, &f);
        assert_eq!(Frame::of_osm(&out), Some(f));
        assert!(out.contains(r#"<node id="1""#), "точки за рамкой остаются: по ним идут линии через неё");
    }

    #[test]
    fn distance_transform_matches_brute_force() {
        let (w, h) = (9, 7);
        let m: Vec<bool> = (0..w * h).map(|k| (k % w) % 7 != 0 && (k / w) % 5 != 0).collect();
        let d = edt(&m, w, h);
        for k in 0..w * h {
            let want = if m[k] {
                (0..w * h).filter(|&j| !m[j]).map(|j| (((k % w) as f32 - (j % w) as f32).powi(2) + ((k / w) as f32 - (j / w) as f32).powi(2)).sqrt()).fold(f32::INFINITY, f32::min)
            } else {
                0.0
            };
            assert!((d[k] - want).abs() < 1e-4, "{k}: {} против {want}", d[k]);
        }
    }

    #[test]
    fn one_round_dark_blot_on_grass_is_one_tree_of_its_size() {
        // Луг с тёмным кругом в 6 м: одна крона радиусом около 3 м.
        let (w, h) = (120, 100);
        let img = RgbImage::from_fn(w, h, |x, y| {
            let d = ((x as f32 - 60.0).powi(2) + (y as f32 - 50.0).powi(2)).sqrt();
            if d < 10.0 { image::Rgb([30, 60, 25]) } else { image::Rgb([110, 150, 80]) }
        });
        let frame = Frame { south: 0.0, west: 0.0, north: h as f64 * 0.3 / 110_574.0, east: w as f64 * 0.3 / 111_320.0 };
        let cls = classify(&img, Biome::Green, None);
        assert_eq!(cls[50 * w as usize + 60], TREES);
        assert_eq!(cls[10 * w as usize + 10], GRASS);
        let c = crowns(&img, &cls, &frame, None, None, None);
        assert_eq!(c.len(), 1, "{c:?}");
    }

    /// Где на крупных масштабах заглушка Esri «нет снимка» (ровно серая 204),
    /// снимок берётся с первого масштаба, где он есть, — тут z17.
    #[test]
    fn placeholder_tiles_send_the_photo_to_a_smaller_zoom() {
        let dir = std::env::temp_dir().join(format!("kartograf-zoom-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let frame = Frame::around(-2.12725, -51.44072, [40, 30]);
        for z in zooms(&frame) {
            let colour = if z >= 18 { [204, 204, 204] } else { [40, 70, 35] };
            for (tx, ty) in tiles_of(&frame, z) {
                RgbImage::from_pixel(256, 256, image::Rgb(colour)).save(dir.join(format!("{z}_{tx}_{ty}.jpg"))).unwrap();
            }
        }
        let z = zoom(None, &dir, &frame);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(z, Ok(17));
    }

    /// Серо-белое на снимке — снег, только если покров не против: заглушка и
    /// облака в джунглях снегом не становятся.
    #[test]
    fn bright_grey_is_snow_only_when_cover_allows() {
        let img = RgbImage::from_pixel(60, 40, image::Rgb([212, 214, 216]));
        assert_eq!(biome(&img, None, true), Biome::Ice);
        assert_ne!(biome(&img, None, false), Biome::Ice);
    }

    /// Лес по покрову, а снимок кроны почти не видит — лес сплошь; видит — как
    /// на снимке. Вода остаётся водой.
    #[test]
    fn cover_forest_fills_canopy_the_photo_cannot_see() {
        let lc = vec![LC_TREES; 100];
        let mut blind: Vec<u8> = (0..100).map(|k| if k < 5 { TREES } else if k == 99 { WATER } else { GRASS }).collect();
        fill_canopy(&mut blind, &lc);
        assert_eq!(blind.iter().filter(|&&c| c == TREES).count(), 99);
        assert_eq!(blind[99], WATER);
        let mut sharp: Vec<u8> = (0..100).map(|k| if k % 2 == 0 { TREES } else { GRASS }).collect();
        let before = sharp.clone();
        fill_canopy(&mut sharp, &lc);
        assert_eq!(sharp, before);
    }
}
