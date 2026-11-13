//! Рельеф: высота земли в точках мелкой сетки масок (`mask::RES` на клетку).
//! Высота — в клетках: клетка — 5 футов, ярус — тоже 5 футов.
//!
//! Рисуется рельеф так, как на профессиональных боевых картах (FA, 2-Minute
//! Tabletop, Czepeku): не плавной светотенью, а ярусами. Земля делится на ярусы
//! по 5 футов; внутри яруса она ровная, а чем ярус ниже верхнего, тем он темнее.
//! Между ярусами — кромка ([`edges`]): где склон круче [`CLIFF`] — обрыв, полоса
//! скалы вниз от кромки шириной по перепаду, под ней осыпь; где положе — гребень,
//! узкая полоса уступа. Ярус выше отбрасывает тень на нижний. В Foundry кромка
//! обрыва — стена: не пройти, но видно поверх.

use crate::mask::{self, Mask, RES};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Склон круче этого (подъём в клетках на клетку пути, ~56°) — обрыв.
pub const CLIFF: f32 = 1.5;
/// Солнце: подъём луча на клетку пути. 1 — 45°, тень длиной в высоту.
const SUN: f32 = 0.75;
/// Темнота тени яруса.
const CAST: f32 = 0.45;
/// Дальше этого тень не ищется, клеток.
const REACH: f32 = 16.0;
/// Насколько темнее каждый ярус ниже верхнего и предел.
const DEPTH: f32 = 0.12;
const DEPTH_MAX: f32 = 0.45;
/// У рельефа настоящей местности склон положе этого (ярусов на клетку) — без гребня.
const NATURAL_RIDGE: f32 = 0.3;
/// Ширина полосы гребня, клетки.
pub const RIDGE_WIDTH: f32 = 1.0;
/// Ширина осыпи под обрывом, клетки.
pub const UNDER_WIDTH: f32 = 0.8;
/// На каком пути до свободного конца обрыв сходит на нет; гребень — быстрее.
const TAPER: f32 = 1.8;
/// На каком пути до свободного конца полоса обрыва гаснет; гребня — быстрее.
const END_FADE: f32 = 1.6;
/// Насколько вокруг спрятанного гребня ступень затемнения сглажена, клетки.
const HIDE: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Relief {
    pub w: usize,
    pub h: usize,
    /// Высота точек, клетки; строками, как у маски.
    pub data: Vec<f32>,
    /// Полосы кромок: стенка обрыва, гребень, осыпь — ключи картинок; пусто — встроенные.
    pub face: String,
    pub ridge: String,
    pub under: String,
    /// Рельеф настоящей местности (из OSM): ярусы не темнеют по высоте и пологие
    /// склоны идут без гребней. На снимках низина не темнее склона — темнеет только
    /// тень от обрыва, а гребни на пологом подъёме ложились кольцами горизонталей.
    pub natural: bool,
}

impl Relief {
    /// Тот же рельеф на поле шире (как [`Mask::padded`]): новые поля — на нуле.
    pub fn padded(&self, cells_w: u32, cells_h: u32, dx: u32, dy: u32) -> Self {
        let (w, h) = (cells_w as usize * RES, cells_h as usize * RES);
        let (ox, oy) = (dx as usize * RES, dy as usize * RES);
        let mut data = vec![0.0; w * h];
        for j in 0..self.h.min(h.saturating_sub(oy)) {
            let n = self.w.min(w.saturating_sub(ox));
            let at = (j + oy) * w + ox;
            data[at..at + n].copy_from_slice(&self.data[j * self.w..j * self.w + n]);
        }
        Self { w, h, data, ..self.clone() }
    }

    pub fn for_cells(cells_w: u32, cells_h: u32) -> Self {
        let (w, h) = (cells_w as usize * RES, cells_h as usize * RES);
        Self { w, h, data: vec![0.0; w * h], face: String::new(), ridge: String::new(), under: String::new(), natural: false }
    }

    pub fn face_key(&self) -> String {
        if self.face.is_empty() { format!("{}wall_cliff", crate::library::BUILTIN) } else { self.face.clone() }
    }

    pub fn ridge_key(&self) -> String {
        if self.ridge.is_empty() { format!("{}wall_ridge", crate::library::BUILTIN) } else { self.ridge.clone() }
    }

    pub fn under_key(&self) -> String {
        if self.under.is_empty() { format!("{}wall_under_cliff", crate::library::BUILTIN) } else { self.under.clone() }
    }

    /// Высота точки; за краем — крайней точки: земля за картой продолжается.
    pub fn get(&self, i: isize, j: isize) -> f32 {
        let i = i.clamp(0, self.w as isize - 1) as usize;
        let j = j.clamp(0, self.h as isize - 1) as usize;
        self.data[j * self.w + i]
    }

    /// Высоты — к шагу, которым они пишутся в файл: открытая карта равна сохранённой.
    pub fn quantize(&mut self) {
        self.data.iter_mut().for_each(|v| *v = (*v * QUANT).round() / QUANT);
    }

    pub fn is_flat(&self) -> bool {
        let lo = self.data.iter().copied().fold(f32::INFINITY, f32::min);
        self.data.iter().all(|v| (v - lo).abs() < 0.02)
    }

    /// Высота в точке карты (клетки); между точками сетки — по прямой.
    pub fn at(&self, p: [f32; 2]) -> f32 {
        let (x, y) = (p[0] * RES as f32 - 0.5, p[1] * RES as f32 - 0.5);
        let (i, j) = (x.floor(), y.floor());
        let (tx, ty) = (x - i, y - j);
        let (i, j) = (i as isize, j as isize);
        let top = self.get(i, j) * (1.0 - tx) + self.get(i + 1, j) * tx;
        let low = self.get(i, j + 1) * (1.0 - tx) + self.get(i + 1, j + 1) * tx;
        top * (1.0 - ty) + low * ty
    }

    /// Ярус точки карты: 0 — от −2,5 до 2,5 фута и т. д.
    pub fn level_at(&self, p: [f32; 2]) -> i32 {
        level(self.at(p))
    }

    /// Подъём по осям в точке сетки, клеток на клетку пути.
    fn grad(&self, i: isize, j: isize) -> [f32; 2] {
        let r = RES as f32 / 2.0;
        [(self.get(i + 1, j) - self.get(i - 1, j)) * r, (self.get(i, j + 1) - self.get(i, j - 1)) * r]
    }

    pub fn slope(&self, i: isize, j: isize) -> f32 {
        let g = self.grad(i, j);
        g[0].hypot(g[1])
    }

    /// Уклон в точке карты — по ближайшей точке сетки.
    pub fn slope_at(&self, p: [f32; 2]) -> f32 {
        self.slope((p[0] * RES as f32 - 0.5).round() as isize, (p[1] * RES as f32 - 0.5).round() as isize)
    }

    /// Самый крутой склон в круге радиуса `r` клеток вокруг точки карты.
    pub fn steepest(&self, p: [f32; 2], r: f32) -> f32 {
        let res = RES as f32;
        let (ci, cj) = ((p[0] * res - 0.5).round() as isize, (p[1] * res - 0.5).round() as isize);
        let n = (r * res).ceil() as isize;
        let mut best = 0.0f32;
        for j in cj - n..=cj + n {
            for i in ci - n..=ci + n {
                if i < 0 || j < 0 || i >= self.w as isize || j >= self.h as isize {
                    continue;
                }
                if ((i - ci).pow(2) + (j - cj).pow(2)) as f32 <= (r * res).powi(2) + 0.5 {
                    best = best.max(self.slope(i, j));
                }
            }
        }
        best
    }

    /// Сгладить точки по весам: каждая тянется к среднему соседей.
    pub fn smooth(&mut self, weights: &[(usize, f32)]) {
        let old = self.clone();
        for &(k, v) in weights {
            let (i, j) = ((k % self.w) as isize, (k / self.w) as isize);
            let mut sum = 0.0;
            for dj in -1..=1 {
                for di in -1..=1 {
                    sum += old.get(i + di, j + dj);
                }
            }
            self.data[k] += (sum / 9.0 - self.data[k]) * v.clamp(0.0, 1.0) * 0.6;
        }
    }
}

/// Ярус по высоте в клетках.
pub fn level(h: f32) -> i32 {
    (h + 0.5).floor() as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Обрыв: полоса скалы, в Foundry — стена.
    Cliff,
    /// Гребень: уступ в ярус по пологому склону, пройти можно.
    Ridge,
}

/// Кромка между ярусами: ломаная по верхнему краю, высокий ярус — слева по ходу
/// (по нормали `[-dy, dx]` к направлению `[dx, dy]`), полоса ложится вправо, вниз.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub kind: EdgeKind,
    pub points: Vec<[f32; 2]>,
    /// Ширина полосы у каждой точки, клетки.
    pub width: Vec<f32>,
    /// Перепад у каждой точки, клетки.
    pub drop: Vec<f32>,
    pub closed: bool,
    /// Свободные концы — начало и конец: не у края карты, кромка там сходит на нет.
    pub free: [bool; 2],
}

impl Edge {
    /// Нормаль к высокому ярусу у точки `k`.
    pub fn up(&self, k: usize) -> [f32; 2] {
        normal_at(&self.points, k, self.closed)
    }

    /// Нормаль, сглаженная по `span` соседям в каждую сторону: поперечины
    /// широкой полосы на изгибе не перехлёстывают друг друга — на полупрозрачной
    /// полосе перехлёст виден тёмной чёрточкой.
    pub fn smooth_up(&self, k: usize, span: usize) -> [f32; 2] {
        let n = self.points.len() as isize;
        let mut s = [0.0f32; 2];
        for d in -(span as isize)..=span as isize {
            let i = k as isize + d;
            let i = if self.closed { i.rem_euclid(n) } else if (0..n).contains(&i) { i } else { continue };
            let u = self.up(i as usize);
            let w = 1.0 - d.abs() as f32 / (span as f32 + 1.0);
            s = [s[0] + u[0] * w, s[1] + u[1] * w];
        }
        let l = s[0].hypot(s[1]);
        if l < 1e-4 { self.up(k) } else { [s[0] / l, s[1] / l] }
    }
}

/// Нормаль «влево по ходу» у вершины — среднее соседних отрезков.
pub fn normal_at(pts: &[[f32; 2]], k: usize, closed: bool) -> [f32; 2] {
    let n = pts.len();
    let dir = |a: [f32; 2], b: [f32; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-6);
        [dx / l, dy / l]
    };
    let prev = if k > 0 { Some(dir(pts[k - 1], pts[k])) } else if closed { Some(dir(pts[n - 2], pts[0])) } else { None };
    let next = if k + 1 < n { Some(dir(pts[k], pts[k + 1])) } else if closed { Some(dir(pts[0], pts[1])) } else { None };
    let d = match (prev, next) {
        (Some(a), Some(b)) => {
            let s = [a[0] + b[0], a[1] + b[1]];
            let l = s[0].hypot(s[1]);
            if l < 1e-4 { a } else { [s[0] / l, s[1] / l] }
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => [1.0, 0.0],
    };
    [-d[1], d[0]]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Граница яруса `k` (высота `k − 0,5`) как маска: порог маски ровно на ней.
fn level_mask(r: &Relief, k: i32) -> Mask {
    let at = k as f32 - 0.5;
    Mask { w: r.w, h: r.h, data: r.data.iter().map(|&h| byte(0.5 + (h - at) * 4.0)).collect() }
}

/// Точки ломаной не реже чем через `step` клеток.
fn densify(line: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(line.len() * 2);
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / step).ceil().max(1.0) as usize;
        for s in 0..n {
            let t = s as f32 / n as f32;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    if let Some(&l) = line.last() {
        out.push(l);
    }
    out
}

/// Кромки между ярусами: обрывы и гребни. По краю карты кромок нет.
pub fn edges(r: &Relief) -> Vec<Edge> {
    edges_and_bare(r).0
}

/// Кромки и голые точки границ ярусов внутри карты — те, где своей полосы нет
/// (середина стенки, нижняя ступень, обрезок): их закрывает стенка выше.
fn edges_and_bare(r: &Relief) -> (Vec<Edge>, Vec<[f32; 2]>) {
    let (mw, mh) = (r.w as f32 / RES as f32, r.h as f32 / RES as f32);
    let inside = |p: [f32; 2]| p[0] > 0.1 && p[1] > 0.1 && p[0] < mw - 0.1 && p[1] < mh - 0.1;
    let lo = level(r.data.iter().copied().fold(f32::INFINITY, f32::min));
    let hi = level(r.data.iter().copied().fold(f32::NEG_INFINITY, f32::max));
    let mut out = Vec::new();
    let mut bare = Vec::new();
    for k in lo + 1..=hi {
        let bound = k as f32 - 0.5;
        for line in mask::join(mask::contour(&level_mask(r, k))) {
            let closed0 = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let mut pts = densify(&line, 0.25);
            if pts.len() < 3 {
                continue;
            }
            // Высокий ярус — слева по ходу. Голосуют точки внутри карты: по её краю
            // контур идёт там, где высоты по обе стороны равны.
            let votes: i32 = (0..pts.len())
                .filter(|&i| inside(pts[i]))
                .map(|i| {
                    let n = normal_at(&pts, i, closed0);
                    let (a, b) = ([pts[i][0] + n[0] * 0.3, pts[i][1] + n[1] * 0.3], [pts[i][0] - n[0] * 0.3, pts[i][1] - n[1] * 0.3]);
                    let d = r.at(a) - r.at(b);
                    if d > 0.05 { 1 } else if d < -0.05 { -1 } else { 0 }
                })
                .sum();
            if votes < 0 {
                pts.reverse();
            }
            // Что у каждой точки: обрыв (только верхняя кромка), гребень или ничего.
            let n = pts.len();
            let mut kind: Vec<Option<EdgeKind>> = Vec::with_capacity(n);
            let mut width = Vec::with_capacity(n);
            let mut drop = Vec::with_capacity(n);
            for i in 0..n {
                let p = pts[i];
                let up = normal_at(&pts, i, closed0);
                if !inside(p) {
                    kind.push(None);
                    width.push(0.0);
                    drop.push(0.0);
                    continue;
                }
                let along = |t: f32| [p[0] + up[0] * t, p[1] + up[1] * t];
                if r.steepest(p, 0.35) >= CLIFF {
                    // Выше рядом ещё крутая граница — это не верх стенки, а её середина
                    // или нижняя ступень: стенка рисуется одна, от верхней кромки.
                    let next = (1..=9).map(|s| s as f32 * 0.1).find(|&t| r.at(along(t)) > bound + 1.0);
                    if next.is_some_and(|t| r.steepest(along(t), 0.3) >= CLIFF * 0.8) {
                        kind.push(None);
                        width.push(0.0);
                        drop.push(0.0);
                        continue;
                    }
                    // Подножие — где уклон снова пологий хотя бы на 0,7 клетки: ступени
                    // через узкую полку — одна стенка.
                    let (mut t, mut last) = (0.25f32, 0.25f32);
                    while t < 4.0 {
                        if r.slope_at(along(-t)) >= CLIFF * 0.5 {
                            last = t;
                        } else if t - last > 0.7 {
                            break;
                        }
                        t += 0.1;
                    }
                    let foot = last + 0.1;
                    let d = (r.at(along(0.3)) - r.at(along(-foot - 0.2))).max(0.5);
                    kind.push(Some(EdgeKind::Cliff));
                    width.push(foot.max(0.35 + 0.35 * d).clamp(0.5, 3.0));
                    drop.push(d);
                } else if r.natural && r.steepest(p, 0.5) < NATURAL_RIDGE {
                    // Пологий склон настоящей местности — без гребня.
                    kind.push(None);
                    width.push(0.0);
                    drop.push(0.0);
                } else {
                    kind.push(Some(EdgeKind::Ridge));
                    width.push(RIDGE_WIDTH);
                    drop.push(1.0);
                }
            }
            // Участки одного вида подряд — отдельные кромки. У замкнутой ломаной
            // начало — там, где вид меняется, иначе участок через начало разрезан надвое.
            let whole = closed0 && kind.iter().all(|k| *k == kind[0]) && kind[0].is_some();
            if closed0 && !whole
                && let Some(s) = (1..n).find(|&i| kind[i] != kind[i - 1])
            {
                pts.rotate_left(s);
                kind.rotate_left(s);
                width.rotate_left(s);
                drop.rotate_left(s);
            }
            let mut i = 0;
            while i < n {
                let Some(kd) = kind[i] else {
                    if inside(pts[i]) {
                        bare.push(pts[i]);
                    }
                    i += 1;
                    continue;
                };
                let mut j = i;
                while j + 1 < n && kind[j + 1] == Some(kd) {
                    j += 1;
                }
                if j - i >= 2 {
                    // У края карты обрыв уходит за край, а не сходит на нет. Соседи
                    // у замкнутого контура — по кругу; открытый кончается у края.
                    let border = |q: Option<usize>| q.is_none_or(|q| !inside(pts[q]));
                    let prev = if i > 0 { Some(i - 1) } else if closed0 { Some(n - 1) } else { None };
                    let next = if j + 1 < n { Some(j + 1) } else if closed0 { Some(0) } else { None };
                    let free = [!whole && !border(prev), !whole && !border(next)];
                    let mut e = Edge { kind: kd, points: pts[i..=j].to_vec(), width: width[i..=j].to_vec(), drop: drop[i..=j].to_vec(), closed: whole, free };
                    soften(&mut e);
                    out.push(e);
                } else {
                    bare.extend_from_slice(&pts[i..=j]);
                }
                i = j + 1;
            }
        }
    }
    (out, bare)
}

/// Ширина вдоль кромки плавно, а к свободному концу кромка сходит на нет, как
/// концевые куски скал FA: стенка ниже и ниже и уходит в склон.
fn soften(e: &mut Edge) {
    let n = e.width.len();
    let src = e.width.clone();
    for i in 0..n {
        let (a, b) = (i.saturating_sub(3), (i + 3).min(n - 1));
        e.width[i] = src[a..=b].iter().sum::<f32>() / (b - a + 1) as f32;
    }
    let (from_start, run) = runs(&e.points);
    let reach = if e.kind == EdgeKind::Cliff { TAPER } else { TAPER * 0.6 }.min(run * 0.4).max(1e-3);
    for i in 0..n {
        let d = end_distance(e, &from_start, run, i);
        // Стенка ниже к концу, но не в точку (сжатые пласты — торец бревна):
        // дальше она гаснет и растворяется в склоне.
        let t = 1.0 - (d / reach).min(1.0);
        e.width[i] *= 1.0 - 0.8 * t * t;
    }
}

/// Путь от начала до каждой точки и длина всей ломаной.
fn runs(pts: &[[f32; 2]]) -> (Vec<f32>, f32) {
    let mut run = 0.0;
    let mut out = vec![0.0; pts.len()];
    for i in 1..pts.len() {
        run += (pts[i][0] - pts[i - 1][0]).hypot(pts[i][1] - pts[i - 1][1]);
        out[i] = run;
    }
    (out, run)
}

/// Путь от точки `i` до ближайшего свободного конца; бесконечность, если их нет.
fn end_distance(e: &Edge, from_start: &[f32], run: f32, i: usize) -> f32 {
    let a = if e.free[0] { from_start[i] } else { f32::INFINITY };
    let b = if e.free[1] { run - from_start[i] } else { f32::INFINITY };
    a.min(b)
}

/// Что рисуется из рельефа: затемнение ярусов, их тени и кромки.
pub struct Look {
    /// Ярус ниже — темнее; плюс тень от яруса выше. Чёрным поверх пола.
    pub dark: Mask,
    pub edges: Vec<Edge>,
    /// Прозрачность полосы у каждой точки кромки, 1 — видна вся (гаснет под
    /// дорогой и у свободного конца). Низ полосы гаснет раньше кромки.
    pub fade: Vec<Vec<f32>>,
}

/// Затемнение и тени при солнце, от которого тени падают под углом `angle`
/// (градусы по часовой от «вправо», как у `Shadows`). `roads` — линии дорог и их
/// полуширины: под дорогой съезд, там ярусы не ступенями, а как есть.
pub fn look(r: &Relief, angle: f32, roads: &[(Vec<[f32; 2]>, f32)]) -> Look {
    let (w, h) = (r.w, r.h);
    // Насколько ступени ярусов сглажены: на дороге 1, к клетке от обочины — ноль.
    let mut on_road = vec![0.0f32; w * h];
    let step = 1.0 / RES as f32;
    for (line, half) in roads {
        for s in line.windows(2) {
            let reach = half + 1.0;
            let (x0, x1) = (s[0][0].min(s[1][0]) - reach, s[0][0].max(s[1][0]) + reach);
            let (y0, y1) = (s[0][1].min(s[1][1]) - reach, s[0][1].max(s[1][1]) + reach);
            let (i0, i1) = (((x0 / step).floor().max(0.0)) as usize, ((x1 / step).ceil() as usize).min(w));
            let (j0, j1) = (((y0 / step).floor().max(0.0)) as usize, ((y1 / step).ceil() as usize).min(h));
            for j in j0..j1 {
                for i in i0..i1 {
                    let gap = crate::map::segment_distance(mask::sample_pos(i as isize, j as isize), s[0], s[1]) - half;
                    let v = 1.0 - smoothstep(0.0, 1.0, gap);
                    let k = j * w + i;
                    on_road[k] = on_road[k].max(v);
                }
            }
        }
    }
    // Где полосы нет или она гаснет (гребень под дорогой, обрубок, конец обрыва,
    // голая граница), ступени тоже нет: иначе край затемнения торчит голой линией.
    let (edges, bare) = edges_and_bare(r);
    let fade = fades(&edges, roads);
    let hidden = edges.iter().zip(&fade).flat_map(|(e, f)| e.points.iter().copied().zip(f.iter().map(|a| 1.0 - a))).chain(bare.iter().map(|&p| (p, 1.0)));
    for (p, a) in hidden {
        if a < 0.01 {
            continue;
        }
        let (i0, i1) = ((((p[0] - HIDE) / step).floor().max(0.0)) as usize, (((p[0] + HIDE) / step).ceil() as usize).min(w));
        let (j0, j1) = ((((p[1] - HIDE) / step).floor().max(0.0)) as usize, (((p[1] + HIDE) / step).ceil() as usize).min(h));
        for j in j0..j1 {
            for i in i0..i1 {
                let q = mask::sample_pos(i as isize, j as isize);
                let v = a * (1.0 - smoothstep(HIDE * 0.4, HIDE, (q[0] - p[0]).hypot(q[1] - p[1])));
                let k = j * w + i;
                on_road[k] = on_road[k].max(v);
            }
        }
    }
    // Тени — от ярусов, а не от плавных склонов: ступень даёт тень, как на картах.
    // Под дорогой — от плавной высоты: съезд не режется тёмной полосой.
    let steps = Relief { data: r.data.iter().zip(&on_road).map(|(&v, &t)| level(v) as f32 + (v - level(v) as f32) * t).collect(), ..r.clone() };
    let top = steps.data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let a = angle.to_radians();
    let to_sun = [-a.cos(), -a.sin()];
    let (mw, mh) = (w as f32 * step, h as f32 * step);
    let mut cast = vec![0.0f32; w * h];
    for j in 0..h {
        for i in 0..w {
            let k = j * w + i;
            let h0 = steps.data[k];
            let p0 = mask::sample_pos(i as isize, j as isize);
            let mut t = step;
            while t < REACH {
                let ray = h0 + SUN * t;
                if ray > top {
                    break;
                }
                let p = [p0[0] + to_sun[0] * t, p0[1] + to_sun[1] * t];
                if p[0] < 0.0 || p[1] < 0.0 || p[0] > mw || p[1] > mh {
                    break;
                }
                if steps.at(p) > ray + 0.05 {
                    cast[k] = 1.0;
                    break;
                }
                t += step;
            }
        }
    }
    // Край тени мягче: два прохода размытия три на три.
    for _ in 0..2 {
        let old = cast.clone();
        for j in 0..h {
            for i in 0..w {
                let mut sum = 0.0;
                for dj in -1..=1isize {
                    for di in -1..=1isize {
                        let (x, y) = ((i as isize + di).clamp(0, w as isize - 1) as usize, (j as isize + dj).clamp(0, h as isize - 1) as usize);
                        sum += old[y * w + x];
                    }
                }
                cast[j * w + i] = sum / 9.0;
            }
        }
    }
    let mut dark = Mask { w, h, data: vec![0; w * h] };
    for k in 0..w * h {
        let depth = if r.natural { 0.0 } else { ((top - steps.data[k]) * DEPTH).min(DEPTH_MAX) };
        // Тень и глубина складываются как два слоя полупрозрачного чёрного.
        let v = 1.0 - (1.0 - depth) * (1.0 - cast[k] * CAST);
        dark.data[k] = byte(v);
    }
    Look { dark, edges, fade }
}

/// Прозрачность полос по точкам кромок. У свободного конца полоса гаснет:
/// сузившийся край не срезан ровно. Гребень под дорогой и мостом гаснет тоже:
/// тропа идёт через уступ поверх него.
fn fades(edges: &[Edge], roads: &[(Vec<[f32; 2]>, f32)]) -> Vec<Vec<f32>> {
    edges
        .iter()
        .map(|e| {
            let n = e.points.len();
            let (from_start, run) = runs(&e.points);
            let reach = if e.kind == EdgeKind::Cliff { END_FADE } else { END_FADE * 0.6 }.min(run * 0.3);
            let mut f: Vec<f32> = (0..n).map(|i| smoothstep(0.0, reach, end_distance(e, &from_start, run, i))).collect();
            if e.kind != EdgeKind::Ridge {
                return f;
            }
            for (k, p) in e.points.iter().enumerate() {
                let gap = roads.iter().map(|(l, half)| l.windows(2).map(|s| crate::map::segment_distance(*p, s[0], s[1])).fold(f32::INFINITY, f32::min) - half).fold(f32::INFINITY, f32::min);
                f[k] *= ((gap + 0.1) / 0.5).clamp(0.0, 1.0);
            }
            // Видимый кусок гребня короче двух с половиной клеток (хвост у съезда,
            // у конца обрыва, между дорогами) лежит на земле бревном, а не уступом.
            // Такие куски не рисуются.
            let mut k = 0;
            while k < n {
                if f[k] <= 0.05 {
                    k += 1;
                    continue;
                }
                let start = k;
                let mut len = 0.0;
                while k + 1 < n && f[k + 1] > 0.05 {
                    len += (e.points[k + 1][0] - e.points[k][0]).hypot(e.points[k + 1][1] - e.points[k][1]);
                    k += 1;
                }
                if len < 2.5 && !(e.closed && start == 0 && k + 1 == n) {
                    f[start..=k].iter_mut().for_each(|v| *v = 0.0);
                }
                k += 1;
            }
            f
        })
        .collect()
}

/// Гладкий шум 0..1: хеш в узлах сетки, между ними — плавно.
pub fn noise(x: f32, y: f32, seed: u32) -> f32 {
    let hash = |i: i32, j: i32| {
        let mut v = (i as u32).wrapping_mul(0x9E37_79B1) ^ (j as u32).wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
        v ^= v >> 15;
        v = v.wrapping_mul(0x2C1B_3C6D);
        v ^= v >> 12;
        (v & 0xFFFF) as f32 / 65535.0
    };
    let (i, j) = (x.floor(), y.floor());
    let (tx, ty) = (x - i, y - j);
    let (tx, ty) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (i, j) = (i as i32, j as i32);
    let top = hash(i, j) * (1.0 - tx) + hash(i + 1, j) * tx;
    let low = hash(i, j + 1) * (1.0 - tx) + hash(i + 1, j + 1) * tx;
    top * (1.0 - ty) + low * ty
}

/// Шум в несколько октав, 0..1; `scale` — размер самой крупной волны, клеток.
pub fn fbm(p: [f32; 2], scale: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut f, mut norm) = (0.0, 1.0, 1.0 / scale, 0.0);
    for o in 0..4u32 {
        sum += noise(p[0] * f, p[1] * f, seed.wrapping_add(o * 7919)) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

// ----- запись в файл: высоты шагом 1/64 клетки, сжатые, в base64 -----

const QUANT: f32 = 64.0;

#[derive(Serialize, Deserialize)]
struct Packed {
    w: usize,
    h: usize,
    z: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    face: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ridge: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    under: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    natural: bool,
}

impl Serialize for Relief {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use std::io::Write;
        let bytes: Vec<u8> = self.data.iter().flat_map(|v| ((v * QUANT).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16).to_le_bytes()).collect();
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        let _ = enc.write_all(&bytes);
        let z = enc.finish().unwrap_or_default();
        Packed { w: self.w, h: self.h, z: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, z), face: self.face.clone(), ridge: self.ridge.clone(), under: self.under.clone(), natural: self.natural }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Relief {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        use std::io::Read;
        let p = Packed::deserialize(d)?;
        let z = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &p.z).map_err(D::Error::custom)?;
        let mut bytes = Vec::with_capacity(p.w * p.h * 2);
        flate2::read::DeflateDecoder::new(&z[..]).read_to_end(&mut bytes).map_err(D::Error::custom)?;
        if bytes.len() != p.w * p.h * 2 {
            return Err(D::Error::custom("рельеф повреждён"));
        }
        let data = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / QUANT).collect();
        Ok(Relief { w: p.w, h: p.h, data, face: p.face, ridge: p.ridge, under: p.under, natural: p.natural })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Плато высотой 3 клетки (15 футов) с отвесным краем посреди карты 10×10.
    fn mesa() -> Relief {
        let mut r = Relief::for_cells(10, 10);
        for j in 0..r.h {
            for i in 0..r.w {
                if i >= 5 * RES {
                    r.data[j * r.w + i] = 3.0;
                }
            }
        }
        r
    }

    #[test]
    fn file_keeps_heights_and_strips() {
        let mut r = mesa();
        r.data[7] = -1.25;
        r.face = "pack:fa|textures/paths/Cliff_Stone_Earthy_A1.webp".into();
        let text = serde_json::to_string(&r).unwrap();
        let back: Relief = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn sheer_edge_is_one_cliff_along_the_top() {
        let e = edges(&mesa());
        let cliffs: Vec<&Edge> = e.iter().filter(|e| e.kind == EdgeKind::Cliff).collect();
        // Три границы ярусов в одной стенке — одна кромка, по верху.
        assert_eq!(cliffs.len(), 1, "{e:?}");
        let c = cliffs[0];
        assert!(c.points.iter().all(|p| (p[0] - 5.0).abs() < 0.3), "{:?}", c.points);
        // Высокий ярус — слева по ходу, полоса — вниз, к западу.
        let up = c.up(c.points.len() / 2);
        assert!(up[0] > 0.9, "{up:?}");
        let mid = c.width[c.width.len() / 2];
        let drop = c.drop[c.drop.len() / 2];
        assert!((mid - (0.35 + 0.35 * drop)).abs() < 0.2 && drop > 2.5, "ширина {mid}, перепад {drop}");
    }

    #[test]
    fn gentle_slope_steps_into_ridges() {
        let mut r = Relief::for_cells(16, 8);
        for j in 0..r.h {
            for i in 0..r.w {
                r.data[j * r.w + i] = i as f32 / RES as f32 * 0.25;
            }
        }
        let e = edges(&r);
        // Подъём почти на 4 клетки по 0,25: четыре уступа, все — гребни.
        assert_eq!(e.len(), 4, "{e:?}");
        assert!(e.iter().all(|e| e.kind == EdgeKind::Ridge));
    }

    #[test]
    fn lower_level_is_darker_and_shadowed() {
        // Тени вправо, солнце слева: у подножия плато справа тени нет, слева — тень от обрыва при тени влево.
        let l = look(&mesa(), 180.0, &[]);
        let at = |x: f32| l.dark.get((x * RES as f32) as isize, 20);
        assert!(at(4.0) > at(1.0) + 40, "тень обрыва к западу: {} против {}", at(4.0), at(1.0));
        assert!(at(1.0) > 40, "нижний ярус темнее: {}", at(1.0));
        assert_eq!(at(8.0), 0, "верхний ярус не темнеет");
    }

    #[test]
    fn natural_relief_darkens_only_by_shadow() {
        // Настоящая местность: нижний ярус вдали от обрыва не темнее верхнего; тень обрыва есть.
        let mut r = mesa();
        r.natural = true;
        let l = look(&r, 180.0, &[]);
        let at = |x: f32| l.dark.get((x * RES as f32) as isize, 20);
        assert_eq!(at(0.5), 0, "низина не темнеет по высоте");
        assert!(at(4.0) > 40, "тень обрыва осталась: {}", at(4.0));
        let back: Relief = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(back.natural, "флаг сохраняется в файле");
    }

    #[test]
    fn road_ramp_has_no_dark_band() {
        // Пологий подъём на ярус вдоль дороги: без дороги затемнение ступенькой, на дороге — плавно.
        let mut r = Relief::for_cells(12, 6);
        for j in 0..r.h {
            for i in 0..r.w {
                r.data[j * r.w + i] = ((i as f32 / RES as f32 - 3.0) / 6.0).clamp(0.0, 1.0);
            }
        }
        let road = vec![(vec![[0.0, 3.0], [12.0, 3.0]], 1.0)];
        let jump = |l: &Look| (1..r.w).map(|i| (l.dark.get(i as isize, 12) as i32 - l.dark.get(i as isize - 1, 12) as i32).abs()).max().unwrap();
        assert!(jump(&look(&r, 0.0, &[])) >= 20, "без дороги — ступень");
        let smooth = jump(&look(&r, 0.0, &road));
        assert!(smooth < 6, "на дороге скачок {smooth}");
    }

    #[test]
    fn free_cliff_end_sinks_and_fades() {
        // Плато на востоке: на севере край отвесный, к югу склон всё положе.
        // Обрыв уходит за северный край карты, а к югу сходит на нет.
        let mut r = Relief::for_cells(12, 12);
        for j in 0..r.h {
            for i in 0..r.w {
                let (x, y) = (i as f32 / RES as f32, j as f32 / RES as f32);
                let s = 0.4 + 3.6 * (1.0 - smoothstep(4.0, 8.0, y));
                r.data[j * r.w + i] = ((x - 6.0) * s + 1.5).clamp(0.0, 3.0);
            }
        }
        let l = look(&r, 45.0, &[]);
        let (c, f) = l.edges.iter().zip(&l.fade).filter(|(e, _)| e.kind == EdgeKind::Cliff).max_by_key(|(e, _)| e.points.len()).expect("обрыва нет");
        assert_eq!(c.free.iter().filter(|&&v| v).count(), 1, "свободен один конец: {:?}", c.free);
        let n = c.points.len();
        let (tip, edge) = if c.free[0] { (0, n - 1) } else { (n - 1, 0) };
        let widest = c.width.iter().copied().fold(0.0, f32::max);
        assert!(c.width[tip] < 0.35 * widest, "у конца {} при ширине {widest}", c.width[tip]);
        assert!(f[tip] < 0.05, "конец не погас: {}", f[tip]);
        assert!(f[edge] > 0.99, "у края карты гаснуть нечему: {}", f[edge]);
    }
}
