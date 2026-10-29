//! Маска: сила слоя в точках мелкой сетки, `RES` точек на клетку по каждой стороне.
//! Точка `(i, j)` стоит в середине своей подклетки: `((i + 0.5) / RES, (j + 0.5) / RES)`.
//! Так граница между закрашенной клеткой и пустой при пороге 127,5 ложится ровно
//! на линию сетки, а кисть «Клетки» даёт те же стены, что и раньше.
//!
//! Здесь же кисти (что и с какой силой закрашивает мазок) и контур маски,
//! по которому встают стены комнат.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

pub const RES: usize = 4;
/// Порог «внутри комнаты».
const LEVEL: f32 = 127.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Mask {
    /// Маска под карту `cells_w × cells_h` клеток.
    pub fn for_cells(cells_w: u32, cells_h: u32) -> Self {
        let (w, h) = (cells_w as usize * RES, cells_h as usize * RES);
        Self { w, h, data: vec![0; w * h] }
    }

    /// Значение точки; за краем — 0.
    pub fn get(&self, i: isize, j: isize) -> u8 {
        if i < 0 || j < 0 || i as usize >= self.w || j as usize >= self.h { 0 } else { self.data[j as usize * self.w + i as usize] }
    }

    pub fn is_empty(&self) -> bool {
        self.data.iter().all(|&v| v == 0)
    }

    /// Площадь выше порога, в клетках.
    pub fn area(&self) -> f32 {
        self.data.iter().filter(|&&v| v as f32 > LEVEL).count() as f32 / (RES * RES) as f32
    }
}

pub fn sample_pos(i: isize, j: isize) -> [f32; 2] {
    [(i as f32 + 0.5) / RES as f32, (j as f32 + 0.5) / RES as f32]
}

// ----- запись в файл: сжатые байты в base64, а не тысячи чисел -----

#[derive(Serialize, Deserialize)]
struct Packed {
    w: usize,
    h: usize,
    z: String,
}

impl Serialize for Mask {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use std::io::Write;
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = enc.write_all(&self.data);
        let bytes = enc.finish().unwrap_or_default();
        Packed { w: self.w, h: self.h, z: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Mask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        use std::io::Read;
        let p = Packed::deserialize(d)?;
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &p.z).map_err(D::Error::custom)?;
        let mut data = Vec::with_capacity(p.w * p.h);
        flate2::read::DeflateDecoder::new(&bytes[..]).read_to_end(&mut data).map_err(D::Error::custom)?;
        if data.len() != p.w * p.h {
            return Err(D::Error::custom("маска повреждена"));
        }
        Ok(Mask { w: p.w, h: p.h, data })
    }
}

// ----- кисти -----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Целые клетки протяжкой прямоугольника.
    Cells,
    Square,
    Circle,
    /// Россыпь мелких мягких пятен.
    Spray,
}

impl Shape {
    pub const ALL: [Shape; 4] = [Shape::Cells, Shape::Square, Shape::Circle, Shape::Spray];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Cells => "Клетки",
            Shape::Square => "Квадрат",
            Shape::Circle => "Круг",
            Shape::Spray => "Спрей",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    pub shape: Shape,
    /// Половина стороны квадрата или радиус круга, в клетках.
    pub radius: f32,
    /// Доля радиуса с полной силой; дальше — плавный спад к краю.
    pub hardness: f32,
    /// Сколько даёт один отпечаток: 1 — сразу всё.
    pub flow: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Self { shape: Shape::Circle, radius: 1.5, hardness: 0.8, flow: 1.0 }
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Сила по расстоянию от центра: внутри `hard` — 1, у края `r` — 0.
fn falloff(d: f32, r: f32, hardness: f32) -> f32 {
    // Даже у самой жёсткой кисти край размыт на шаг маски: иначе круг ложится
    // лесенкой и стена вокруг него идёт зубцами.
    // Полоса размытия стоит на самом крае, чтобы половина силы приходилась ровно на `r`.
    let aa = 0.5 / RES as f32;
    let hard = (r * hardness.clamp(0.0, 1.0)).min(r - aa).max(0.0);
    let outer = r + aa;
    if d <= hard {
        1.0
    } else if d >= outer {
        0.0
    } else {
        1.0 - smooth((d - hard) / (outer - hard).max(1e-4))
    }
}

/// Отпечаток кисти: точки маски и сила для каждой (0..1).
/// `seed` нужен спрею: каждый отпечаток рассыпается по-своему, но повторимо.
pub fn dab(w: usize, h: usize, brush: &Brush, c: [f32; 2], seed: u32) -> Vec<(usize, f32)> {
    let r = brush.radius.max(0.05);
    let res = RES as f32;
    let (i0, i1) = (((c[0] - r) * res - 1.0).floor().max(0.0) as usize, (((c[0] + r) * res + 1.0).ceil().max(0.0) as usize).min(w));
    let (j0, j1) = (((c[1] - r) * res - 1.0).floor().max(0.0) as usize, (((c[1] + r) * res + 1.0).ceil().max(0.0) as usize).min(h));
    let mut out = Vec::new();
    match brush.shape {
        Shape::Cells | Shape::Square | Shape::Circle => {
            for j in j0..j1 {
                for i in i0..i1 {
                    let p = sample_pos(i as isize, j as isize);
                    let (dx, dy) = ((p[0] - c[0]).abs(), (p[1] - c[1]).abs());
                    let d = if brush.shape == Shape::Circle { (dx * dx + dy * dy).sqrt() } else { dx.max(dy) };
                    let v = falloff(d, r, brush.hardness) * brush.flow;
                    if v > 0.0 {
                        out.push((j * w + i, v));
                    }
                }
            }
        }
        Shape::Spray => {
            // Пятна радиусом в пятую часть кисти, равномерно по кругу.
            let mut rng = crate::generate::Rng::new(&seed.to_string());
            let dots = ((r * r * 6.0) as usize).clamp(3, 60);
            let spot = Brush { shape: Shape::Circle, radius: (r * 0.2).max(0.2), hardness: brush.hardness * 0.5, flow: brush.flow };
            let mut acc: HashMap<usize, f32> = HashMap::new();
            for _ in 0..dots {
                let a = rng.random() as f32 * std::f32::consts::TAU;
                let d = r * (rng.random() as f32).sqrt();
                for (i, v) in dab(w, h, &spot, [c[0] + a.cos() * d, c[1] + a.sin() * d], 0) {
                    let e = acc.entry(i).or_insert(0.0);
                    *e = 1.0 - (1.0 - *e) * (1.0 - v);
                }
            }
            out.extend(acc);
        }
    }
    out
}

/// Целые клетки прямоугольника между двумя углами (включительно).
pub fn cells(w: usize, h: usize, a: (i32, i32), b: (i32, i32)) -> Vec<(usize, f32)> {
    let cw = (w / RES) as i32;
    let ch = (h / RES) as i32;
    let (x0, x1) = (a.0.min(b.0).max(0), a.0.max(b.0).min(cw - 1));
    let (y0, y1) = (a.1.min(b.1).max(0), a.1.max(b.1).min(ch - 1));
    let mut out = Vec::new();
    if x0 > x1 || y0 > y1 {
        return out;
    }
    for j in y0 as usize * RES..(y1 as usize + 1) * RES {
        for i in x0 as usize * RES..(x1 as usize + 1) * RES {
            out.push((j * w + i, 1.0));
        }
    }
    out
}

// ----- контур -----

pub type Segment = ([f32; 2], [f32; 2]);

/// Граница маски на пороге — замкнутые ломаные. Квадраты сетки (marching squares),
/// склейка кусков в ломаные, спрямление: точки на одной прямой убираются, срезанные
/// углы прямоугольников возвращаются острыми, остальные изгибы сглаживаются.
pub fn contour(m: &Mask) -> Vec<Segment> {
    contour_lines(contour_raw(m))
}

/// Куски границы по квадратам сетки, ещё не склеенные.
fn contour_raw(m: &Mask) -> Vec<Segment> {
    let v = |i: isize, j: isize| m.get(i, j) as f32;
    let lerp = |a: [f32; 2], b: [f32; 2], va: f32, vb: f32| {
        let t = if (vb - va).abs() < 1e-6 { 0.5 } else { ((LEVEL - va) / (vb - va)).clamp(0.0, 1.0) };
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    };
    let mut pieces: Vec<Segment> = Vec::new();
    for j in -1..m.h as isize {
        for i in -1..m.w as isize {
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
            let case = (a > LEVEL) as u8 | ((b > LEVEL) as u8) << 1 | ((c > LEVEL) as u8) << 2 | ((d > LEVEL) as u8) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let (pa, pb, pc, pd) = (sample_pos(i, j), sample_pos(i + 1, j), sample_pos(i + 1, j + 1), sample_pos(i, j + 1));
            let top = || lerp(pa, pb, a, b);
            let right = || lerp(pb, pc, b, c);
            let bottom = || lerp(pd, pc, d, c);
            let left = || lerp(pa, pd, a, d);
            let mut add = |p: [f32; 2], q: [f32; 2]| pieces.push((p, q));
            match case {
                1 | 14 => add(left(), top()),
                2 | 13 => add(top(), right()),
                3 | 12 => add(left(), right()),
                4 | 11 => add(right(), bottom()),
                6 | 9 => add(top(), bottom()),
                7 | 8 => add(left(), bottom()),
                5 => {
                    add(left(), top());
                    add(right(), bottom());
                }
                10 => {
                    add(top(), right());
                    add(left(), bottom());
                }
                _ => {}
            }
        }
    }
    pieces
}

fn contour_lines(pieces: Vec<Segment>) -> Vec<Segment> {
    let mut out = Vec::new();
    for line in join(pieces) {
        let line = round_off(&sharpen(&straighten(&simplify(&line, 0.02))));
        let n = line.len();
        let closed = n > 2 && near(line[0], line[n - 1]);
        let pts = if closed { &line[..n - 1] } else { &line[..] };
        let count = if closed { pts.len() } else { pts.len().saturating_sub(1) };
        for k in 0..count {
            out.push((pts[k], pts[(k + 1) % pts.len()]));
        }
    }
    out
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

fn key(p: [f32; 2]) -> (i64, i64) {
    ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64)
}

/// Куски в ломаные. Направление кусков у квадратов сетки не согласовано, поэтому
/// продолжение ищется по любому концу куска.
pub fn join(pieces: Vec<Segment>) -> Vec<Vec<[f32; 2]>> {
    let mut at: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (n, (a, b)) in pieces.iter().enumerate() {
        at.entry(key(*a)).or_default().push(n);
        at.entry(key(*b)).or_default().push(n);
    }
    let mut used = vec![false; pieces.len()];
    // Неиспользованный кусок с концом в точке `p` и его другой конец.
    let next = |p: [f32; 2], used: &[bool]| -> Option<(usize, [f32; 2])> {
        let n = *at.get(&key(p))?.iter().find(|&&n| !used[n])?;
        let (a, b) = pieces[n];
        Some((n, if key(a) == key(p) { b } else { a }))
    };
    let mut lines = Vec::new();
    for s in 0..pieces.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut line = vec![pieces[s].0, pieces[s].1];
        while let Some((n, q)) = next(*line.last().unwrap(), &used) {
            used[n] = true;
            line.push(q);
        }
        // Не замкнулась — дотянуть и в другую сторону.
        if !near(line[0], *line.last().unwrap()) {
            while let Some((n, q)) = next(line[0], &used) {
                used[n] = true;
                line.insert(0, q);
            }
        }
        lines.push(line);
    }
    lines
}

/// Дуглас — Пекер: точки, отходящие от прямой меньше чем на `eps`, выбрасываются.
fn simplify(line: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let closed = near(line[0], line[line.len() - 1]);
    if closed {
        // Замкнутую режем в самой дальней от начала точке, чтобы концы не съелись.
        let far = (1..line.len() - 1)
            .max_by(|&x, &y| {
                let d = |k: usize| (line[k][0] - line[0][0]).powi(2) + (line[k][1] - line[0][1]).powi(2);
                d(x).total_cmp(&d(y))
            })
            .unwrap_or(1);
        let mut a = simplify(&line[..=far], eps);
        let b = simplify(&line[far..], eps);
        a.pop();
        a.extend(b);
        return a;
    }
    let (first, last) = (line[0], line[line.len() - 1]);
    let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (k, dmax) = (1..line.len() - 1).map(|k| (k, ((line[k][0] - first[0]) * dy - (line[k][1] - first[1]) * dx).abs() / len)).fold((0, 0.0), |acc, x| if x.1 > acc.1 { x } else { acc });
    if dmax > eps {
        let mut a = simplify(&line[..=k], eps);
        a.pop();
        a.extend(simplify(&line[k..], eps));
        a
    } else {
        vec![first, last]
    }
}

/// Почти ровный участок вдоль сетки — ровный: у квадратной кисти соседние мазки
/// ложатся мягким краем внахлёст, и край гуляет на сотые доли клетки. Участок не
/// короче клетки, разброс не больше `SPREAD`: у круга радиусом меньше пяти клеток
/// такого плоского места нет.
fn straighten(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    const SPREAD: f32 = 0.03;
    const SPAN: f32 = 1.0;
    let closed = line.len() > 3 && near(line[0], line[line.len() - 1]);
    let mut pts: Vec<[f32; 2]> = if closed { line[..line.len() - 1].to_vec() } else { line.to_vec() };
    let n = pts.len();
    if n < 3 {
        return line.to_vec();
    }
    if closed {
        // Начать после косого отрезка: он не входит ни в один ровный участок,
        // и участок не рвётся на стыке начала и конца.
        let slanted = |k: usize| {
            let (a, b) = (pts[(k + n - 1) % n], pts[k]);
            (a[0] - b[0]).abs() > SPREAD && (a[1] - b[1]).abs() > SPREAD
        };
        let start = (0..n).find(|&k| slanted(k)).unwrap_or(0);
        pts.rotate_left(start);
    }
    for axis in 0..2 {
        let other = 1 - axis;
        let mut i = 0;
        while i + 1 < n {
            let (mut lo, mut hi) = (pts[i][axis], pts[i][axis]);
            let mut j = i;
            while j + 1 < n {
                let v = pts[j + 1][axis];
                if v.max(hi) - v.min(lo) > SPREAD {
                    break;
                }
                lo = lo.min(v);
                hi = hi.max(v);
                j += 1;
            }
            if j > i && (pts[j][other] - pts[i][other]).abs() >= SPAN {
                let mean = (i..=j).map(|k| pts[k][axis]).sum::<f32>() / (j - i + 1) as f32;
                let mean = (mean * 1e4).round() / 1e4;
                for p in &mut pts[i..=j] {
                    p[axis] = mean;
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }
    if closed {
        pts.push(pts[0]);
    }
    pts
}

/// Короткий скос между отрезками вдоль сетки под прямым углом — это срезанный угол
/// прямоугольника: вместо скоса ставим точку пересечения. У клеток скос — один отрезок,
/// у квадратной кисти с мягким краем — дуга из нескольких.
fn sharpen(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let closed = line.len() > 3 && near(line[0], line[line.len() - 1]);
    let mut pts: Vec<[f32; 2]> = if closed { line[..line.len() - 1].to_vec() } else { line.to_vec() };
    // Срез угла у клеток — диагональ в полшага сетки: 0,125·√2 ≈ 0,18 клетки.
    let limit = 0.8 / RES as f32;
    // Скругление угла квадратной кисти — до полуклетки по дуге.
    let arc_limit = 0.6;
    let along = |d: [f32; 2]| d[0].abs() < 1e-5 || d[1].abs() < 1e-5;
    let dist = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let mut k = 0;
    'outer: while pts.len() >= 4 && k < pts.len() {
        let n = pts.len();
        for span in 1..=6usize {
            if span + 2 >= n {
                break;
            }
            if !closed && (k == 0 || k + span + 1 >= n) {
                break;
            }
            let (p0, p1) = (pts[(k + n - 1) % n], pts[k]);
            let (p2, p3) = (pts[(k + span) % n], pts[(k + span + 1) % n]);
            let chain: f32 = (0..span).map(|s| dist(pts[(k + s) % n], pts[(k + s + 1) % n])).sum();
            if chain >= arc_limit {
                break;
            }
            if span == 1 && chain >= limit {
                continue;
            }
            let d1 = [p1[0] - p0[0], p1[1] - p0[1]];
            let d2 = [p3[0] - p2[0], p3[1] - p2[1]];
            let (l1, l2) = ((d1[0] * d1[0] + d1[1] * d1[1]).sqrt(), (d2[0] * d2[0] + d2[1] * d2[1]).sqrt());
            // Только вдоль сетки: у круглой кисти перпендикулярные отрезки — не угол комнаты.
            let square = l1 > 1e-6 && l2 > 1e-6 && along(d1) && along(d2) && ((d1[0] * d2[0] + d1[1] * d2[1]) / (l1 * l2)).abs() < 0.05;
            if !square {
                continue;
            }
            // Пересечение прямых p0-p1 и p2-p3.
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            let t = ((p2[0] - p0[0]) * d2[1] - (p2[1] - p0[1]) * d2[0]) / den;
            let x = [p0[0] + d1[0] * t, p0[1] + d1[1] * t];
            let x = [(x[0] * 1e4).round() / 1e4, (x[1] * 1e4).round() / 1e4];
            pts[k] = x;
            for _ in 0..span {
                let at = (k + 1) % pts.len();
                pts.remove(at);
                if at < k {
                    k -= 1;
                }
            }
            continue 'outer;
        }
        k += 1;
    }
    // После острых углов соседние отрезки могут лечь на одну прямую.
    let mut merged: Vec<[f32; 2]> = Vec::new();
    let n = pts.len();
    for k in 0..n {
        if closed || (k > 0 && k + 1 < n) {
            let (a, b, c) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            if cross.abs() < 1e-5 {
                continue;
            }
        }
        merged.push(pts[k]);
    }
    if closed && !merged.is_empty() {
        merged.push(merged[0]);
    }
    merged
}

/// Плавные изгибы. Ломаная дробится на точки через `STEP` и сглаживается по Таубину
/// (шаг к середине соседей, потом чуть назад — так кривая не съёживается). Углы
/// между отрезками вдоль сетки и концы открытой ломаной стоят на месте, точки на прямой
/// остаются на ней — комнаты кистью «Клетки» не меняются.
fn round_off(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    const STEP: f32 = 0.125;
    const PASSES: usize = 12;
    let n = line.len();
    if n < 3 {
        return line.to_vec();
    }
    let closed = near(line[0], line[n - 1]);
    let pts = if closed { &line[..n - 1] } else { line };
    let m = pts.len();
    let along = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 || (a[1] - b[1]).abs() < 1e-5;
    let hard = |k: usize| {
        if !closed && (k == 0 || k + 1 == m) {
            return true;
        }
        let (a, b, c) = (pts[(k + m - 1) % m], pts[k], pts[(k + 1) % m]);
        along(a, b) && along(b, c)
    };
    let mut dense: Vec<[f32; 2]> = Vec::new();
    let mut fixed: Vec<bool> = Vec::new();
    let edges = if closed { m } else { m - 1 };
    for k in 0..edges {
        let (a, b) = (pts[k], pts[(k + 1) % m]);
        dense.push(a);
        fixed.push(hard(k));
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let parts = (len / STEP).ceil().max(1.0) as usize;
        for p in 1..parts {
            let t = p as f32 / parts as f32;
            dense.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            fixed.push(false);
        }
    }
    if !closed {
        dense.push(pts[m - 1]);
        fixed.push(true);
    }
    let len = dense.len();
    for pass in 0..PASSES * 2 {
        let f = if pass % 2 == 0 { 0.5 } else { -0.53 };
        let prev = dense.clone();
        for k in 0..len {
            if fixed[k] {
                continue;
            }
            let (a, c) = (prev[(k + len - 1) % len], prev[(k + 1) % len]);
            dense[k][0] += f * ((a[0] + c[0]) / 2.0 - prev[k][0]);
            dense[k][1] += f * ((a[1] + c[1]) / 2.0 - prev[k][1]);
        }
    }
    if closed {
        dense.push(dense[0]);
    }
    simplify(&dense, 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(m: &mut Mask, weights: &[(usize, f32)]) {
        for &(i, v) in weights {
            m.data[i] = (m.data[i] as f32 + v * (255.0 - m.data[i] as f32)).round() as u8;
        }
    }

    fn fill_cells(m: &mut Mask, a: (i32, i32), b: (i32, i32)) {
        let w = cells(m.w, m.h, a, b);
        fill(m, &w);
    }

    fn fill_dab(m: &mut Mask, brush: &Brush, c: [f32; 2], seed: u32) {
        let w = dab(m.w, m.h, brush, c, seed);
        fill(m, &w);
    }

    fn length(s: &[Segment]) -> f32 {
        s.iter().map(|(a, b)| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()).sum()
    }

    #[test]
    fn cell_rectangle_has_four_exact_walls() {
        let mut m = Mask::for_cells(20, 12);
        fill_cells(&mut m, (2, 2), (7, 6));
        assert_eq!(m.area(), 30.0);
        let c = contour(&m);
        assert_eq!(c.len(), 4, "{c:?}");
        let mut xs: Vec<f32> = c.iter().flat_map(|(a, b)| [a[0], b[0]]).collect();
        xs.sort_by(f32::total_cmp);
        assert_eq!((xs[0], xs[xs.len() - 1]), (2.0, 8.0));
        assert!((length(&c) - 22.0).abs() < 1e-3);
    }

    #[test]
    fn l_shape_and_map_edge() {
        let mut m = Mask::for_cells(20, 12);
        fill_cells(&mut m, (2, 2), (7, 6));
        fill_cells(&mut m, (5, 5), (10, 8));
        let c = contour(&m);
        assert!((length(&c) - 32.0).abs() < 1e-3, "{}", length(&c));
        assert!(c.iter().all(|(a, b)| a[0] == b[0] || a[1] == b[1]), "все стены прямые: {c:?}");
        fill_cells(&mut m, (0, 0), (0, 0));
        assert!(contour(&m).iter().any(|(a, b)| (a[1] == 0.0 && b[1] == 0.0) && (a[0] - b[0]).abs() == 1.0), "у края карты стена по краю");
    }

    #[test]
    fn circle_has_round_wall() {
        let mut m = Mask::for_cells(20, 20);
        fill_dab(&mut m, &Brush { shape: Shape::Circle, radius: 4.0, hardness: 1.0, flow: 1.0 }, [10.0, 10.0], 0);
        let c = contour(&m);
        let l = length(&c);
        assert!((l - std::f32::consts::TAU * 4.0).abs() < 1.0, "длина {l}");
        assert!(c.len() >= 12 && c.len() < 200, "отрезков {}", c.len());
        assert!((m.area() - std::f32::consts::PI * 16.0).abs() < 2.0);
    }

    #[test]
    fn soft_square_stroke_keeps_sharp_corners() {
        // Как в мазке: точка берёт самую большую силу из отпечатков.
        let mut m = Mask::for_cells(30, 20);
        let b = Brush { shape: Shape::Square, radius: 1.5, hardness: 0.8, flow: 1.0 };
        let mut most = vec![0f32; m.data.len()];
        for x in [22.0, 22.45, 22.9, 23.35, 23.8, 24.0] {
            for (i, v) in dab(m.w, m.h, &b, [x, 14.0], 0) {
                most[i] = most[i].max(v);
            }
        }
        for (d, v) in m.data.iter_mut().zip(most) {
            *d = (v * 255.0).round() as u8;
        }
        let c = contour(&m);
        assert_eq!(c.len(), 4, "{c:?}");
        assert!(c.iter().all(|(a, b)| a[0] == b[0] || a[1] == b[1]), "{c:?}");
    }

    #[test]
    fn freehand_corners_are_rounded() {
        // Два круглых мазка крестом: во внутренних углах раньше торчали шипы.
        let mut m = Mask::for_cells(20, 20);
        let b = Brush { shape: Shape::Circle, radius: 1.5, hardness: 0.8, flow: 1.0 };
        for k in 0..=20 {
            let t = 3.0 + k as f32 * 0.7;
            fill_dab(&mut m, &b, [t, 10.0], 0);
            fill_dab(&mut m, &b, [10.0, t], 0);
        }
        for line in join(contour(&m)) {
            for k in 1..line.len() - 1 {
                let (a, b, c) = (line[k - 1], line[k], line[k + 1]);
                let (d1, d2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
                let cos = (d1[0] * d2[0] + d1[1] * d2[1]) / (d1[0].hypot(d1[1]) * d2[0].hypot(d2[1]));
                assert!(cos > 0.6, "излом {:.0}° в {b:?}", cos.acos().to_degrees());
            }
        }
    }

    #[test]
    fn soft_brush_fades_and_spray_is_repeatable() {
        let (w, h) = (80, 80);
        let soft = Brush { shape: Shape::Circle, radius: 3.0, hardness: 0.0, flow: 1.0 };
        let d = dab(w, h, &soft, [10.0, 10.0], 0);
        let at = |x: f32, y: f32| {
            let i = (y * RES as f32 - 0.5).round() as usize * w + (x * RES as f32 - 0.5).round() as usize;
            d.iter().find(|(k, _)| *k == i).map_or(0.0, |p| p.1)
        };
        assert!(at(10.0, 10.0) > 0.95 && at(11.5, 10.0) < 0.7 && at(11.5, 10.0) > 0.2);
        let spray = Brush { shape: Shape::Spray, ..soft };
        let mut a = dab(w, h, &spray, [10.0, 10.0], 7);
        let mut b = dab(w, h, &spray, [10.0, 10.0], 7);
        a.sort_by_key(|p| p.0);
        b.sort_by_key(|p| p.0);
        assert_eq!(a, b);
        assert!(a.len() < d.len(), "спрей рыхлее круга");
    }

    #[test]
    fn mask_round_trip_is_small() {
        let mut m = Mask::for_cells(100, 100);
        fill_cells(&mut m, (10, 10), (40, 30));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.len() < 5000, "{} байт", json.len());
        assert_eq!(serde_json::from_str::<Mask>(&json).unwrap(), m);
    }
}
