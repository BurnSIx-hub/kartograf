//! Помост: площадка выше пола — сцена, кафедра, помост реактора, ступени зала.
//!
//! Высоту на карте сверху показывают так, как на рисованных боевых картах: площадка
//! светлее пола, по её краю светлое ребро, наружу от края — боковая стенка (чем выше
//! помост, тем она шире: что выше, то ближе к глазу) и тень у подножия. Лестница —
//! ступени от края наружу, кверху шире. Стенка и тень идут только по свободным краям:
//! край вдоль стены комнаты их не даёт.
//!
//! Здесь — данные и геометрия; рисует `app::draw_platform`.

use serde::{Deserialize, Serialize};

/// Глубина ступени лестницы, клетки.
pub const TREAD: f32 = 0.4;

/// Лестница на помост: где примыкает (точка у края — берётся ближайшая на контуре),
/// ширина наверху и число ступеней.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stair {
    pub at: [f32; 2],
    pub width: f32,
    pub steps: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Platform {
    /// Контур площадки по порядку, клетки.
    pub points: Vec<[f32; 2]>,
    /// Видимая ширина боковой стенки, клетки: высота помоста на глаз. Ступень — около
    /// 0,08, помост в пять ступеней — 0,4.
    pub height: f32,
    /// Цвет площадки и его сила (0–1): площадка — тот же пол, подкрашенный этим цветом.
    #[serde(default = "light")]
    pub color: [u8; 3],
    #[serde(default = "tenth")]
    pub strength: f32,
    /// Цвет боковой стенки и ступеней.
    #[serde(default = "steel")]
    pub side: [u8; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stairs: Vec<Stair>,
}

pub fn light() -> [u8; 3] {
    [255, 255, 255]
}

pub fn tenth() -> f32 {
    0.1
}

pub fn steel() -> [u8; 3] {
    [98, 106, 116]
}

impl Platform {
    pub fn new(points: Vec<[f32; 2]>, height: f32) -> Self {
        Self { points, height, color: light(), strength: tenth(), side: steel(), stairs: Vec::new() }
    }

    /// Рамка контура: левый верхний и правый нижний углы.
    pub fn bounds(&self) -> ([f32; 2], [f32; 2]) {
        let mut lo = [f32::INFINITY; 2];
        let mut hi = [f32::NEG_INFINITY; 2];
        for p in &self.points {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        (lo, hi)
    }
}

/// Площадь со знаком: у контура, идущего на экране по часовой (ось y вниз), — положительная.
pub fn area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f32>() / 2.0
}

/// Контур без повторов подряд и без замыкающей точки, равной первой.
pub fn tidy(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_none_or(|q| (q[0] - p[0]).abs() > 1e-4 || (q[1] - p[1]).abs() > 1e-4) {
            out.push(p);
        }
    }
    if out.len() > 1 && (out[0][0] - out[out.len() - 1][0]).abs() < 1e-4 && (out[0][1] - out[out.len() - 1][1]).abs() < 1e-4 {
        out.pop();
    }
    out
}

/// Наружные нормали рёбер; ребро `i` идёт от точки `i` к `i + 1`.
pub fn normals(p: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let s = if area(p) < 0.0 { -1.0 } else { 1.0 };
    let n = p.len();
    (0..n)
        .map(|i| {
            let (dx, dy) = (p[(i + 1) % n][0] - p[i][0], p[(i + 1) % n][1] - p[i][1]);
            let l = dx.hypot(dy).max(1e-6);
            [s * dy / l, -s * dx / l]
        })
        .collect()
}

/// Точки контура, сдвинутые наружу на `d` (внутрь — отрицательное). На углу — по
/// биссектрисе, так что полосы соседних рёбер сходятся без щели и нахлёста; у вершины
/// между свободным и несвободным ребром (`free`) — по нормали свободного: полоса
/// кончается у стены прямым срезом.
pub fn offset(p: &[[f32; 2]], free: &[bool], d: f32) -> Vec<[f32; 2]> {
    let nrm = normals(p);
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = ((i + n - 1) % n, i);
            let (n0, n1) = (nrm[a], nrm[b]);
            let dir = match (free[a], free[b]) {
                (true, false) => n0,
                (false, true) => n1,
                _ => {
                    // Острый угол не вытягивается в иглу: не дальше трёх сдвигов.
                    let k = (1.0 + n0[0] * n1[0] + n0[1] * n1[1]).max(0.34);
                    [(n0[0] + n1[0]) / k, (n0[1] + n1[1]) / k]
                }
            };
            [p[i][0] + dir[0] * d, p[i][1] + dir[1] * d]
        })
        .collect()
}

fn cross(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// Контур — треугольниками (отрезанием «ушей»): номера точек. Годится и вогнутый.
pub fn triangulate(p: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let s = if area(p) < 0.0 { -1.0 } else { 1.0 };
    let mut left: Vec<usize> = (0..p.len()).collect();
    let mut out = Vec::new();
    let mut idle = 0;
    let mut at = 0;
    while left.len() > 3 && idle <= left.len() {
        let n = left.len();
        let (a, b, c) = (left[(at + n - 1) % n], left[at % n], left[(at + 1) % n]);
        let convex = cross(p[a], p[b], p[c]) * s > 1e-9;
        let empty = convex
            && left.iter().all(|&q| {
                q == a || q == b || q == c || {
                    let (d0, d1, d2) = (cross(p[a], p[b], p[q]) * s, cross(p[b], p[c], p[q]) * s, cross(p[c], p[a], p[q]) * s);
                    !(d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0)
                }
            });
        if empty {
            out.push([a, b, c]);
            left.remove(at % n);
            idle = 0;
        } else {
            at += 1;
            idle += 1;
        }
        if at >= left.len() {
            at = 0;
        }
    }
    // Что осталось (или контур с самопересечением, где ушей не нашлось) — веером.
    for k in 1..left.len().saturating_sub(1) {
        out.push([left[0], left[k], left[k + 1]]);
    }
    out
}

/// Точка внутри контура.
#[cfg(test)]
pub fn contains(p: &[[f32; 2]], q: [f32; 2]) -> bool {
    let n = p.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        if (a[1] > q[1]) != (b[1] > q[1]) && q[0] < a[0] + (q[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
            inside = !inside;
        }
    }
    inside
}

/// Круг многоугольником: сторона около трети клетки, от 16 до 96 сторон.
pub fn circle(c: [f32; 2], r: f32) -> Vec<[f32; 2]> {
    let n = ((std::f32::consts::TAU * r / 0.33).ceil() as usize).clamp(16, 96);
    (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect()
}

/// Часть контура внутри прямоугольника (помост-круг у стены зала: круг, обрезанный залом).
pub fn within(p: &[[f32; 2]], rect: [f32; 4]) -> Vec<[f32; 2]> {
    let [x0, y0, x1, y1] = rect;
    // Четыре полуплоскости: (ось, граница, знак «внутри»).
    let mut out = p.to_vec();
    for (axis, edge, keep_less) in [(0, x0, false), (0, x1, true), (1, y0, false), (1, y1, true)] {
        let inside = |q: [f32; 2]| if keep_less { q[axis] <= edge } else { q[axis] >= edge };
        let src = std::mem::take(&mut out);
        for i in 0..src.len() {
            let (a, b) = (src[i], src[(i + 1) % src.len()]);
            if inside(a) != inside(b) {
                let t = (edge - a[axis]) / (b[axis] - a[axis]);
                let cut = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                if inside(a) {
                    out.push(a);
                }
                out.push(cut);
            } else if inside(a) {
                out.push(a);
            }
        }
    }
    tidy(&out)
}

/// Прямоугольник без круга, который вырезает из него кусок с одного края или угла
/// (ярус амфитеатра: зал за дугой ступени). `None` — круг не режет рамку ровно в двух местах.
pub fn bitten(rect: [f32; 4], c: [f32; 2], r: f32) -> Option<Vec<[f32; 2]>> {
    let [x0, y0, x1, y1] = rect;
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    let inside = |q: [f32; 2]| (q[0] - c[0]).hypot(q[1] - c[1]) < r;
    // Обход рамки по часовой: точки вне круга и места, где рамка входит в круг и выходит.
    let mut ring: Vec<([f32; 2], u8)> = Vec::new();    // 0 — точка рамки, 1 — вход в круг, 2 — выход
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        if !inside(a) {
            ring.push((a, 0));
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (fx, fy) = (a[0] - c[0], a[1] - c[1]);
        let (qa, qb, qc) = (dx * dx + dy * dy, 2.0 * (fx * dx + fy * dy), fx * fx + fy * fy - r * r);
        let disc = qb * qb - 4.0 * qa * qc;
        if disc > 0.0 {
            let sq = disc.sqrt();
            for t in [(-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa)] {
                if t > 1e-6 && t < 1.0 - 1e-6 {
                    let q = [a[0] + dx * t, a[1] + dy * t];
                    // Чуть дальше по ребру — внутри круга: значит, здесь рамка в круг входит.
                    let ahead = [a[0] + dx * (t + 1e-3), a[1] + dy * (t + 1e-3)];
                    ring.push((q, if inside(ahead) { 1 } else { 2 }));
                }
            }
        }
    }
    if ring.iter().filter(|p| p.1 == 1).count() != 1 || ring.iter().filter(|p| p.1 == 2).count() != 1 {
        return None;
    }
    let enter = ring.iter().position(|p| p.1 == 1)?;
    let exit = ring.iter().position(|p| p.1 == 2)?;
    let (pe, px) = (ring[enter].0, ring[exit].0);
    // От выхода по рамке до входа, затем по дуге внутри рамки обратно к выходу.
    let mut out = Vec::new();
    let mut k = exit;
    loop {
        out.push(ring[k].0);
        if k == enter {
            break;
        }
        k = (k + 1) % ring.len();
    }
    let (a0, a1) = ((pe[1] - c[1]).atan2(pe[0] - c[0]), (px[1] - c[1]).atan2(px[0] - c[0]));
    // Из двух дуг между точками — та, чья середина внутри рамки.
    let tau = std::f32::consts::TAU;
    let sweep = (a1 - a0).rem_euclid(tau);
    let mid = |sw: f32| [c[0] + r * (a0 + sw / 2.0).cos(), c[1] + r * (a0 + sw / 2.0).sin()];
    let in_rect = |q: [f32; 2]| q[0] > x0 && q[0] < x1 && q[1] > y0 && q[1] < y1;
    let sweep = if in_rect(mid(sweep)) { sweep } else { sweep - tau };
    let steps = ((sweep.abs() * r / 0.33).ceil() as usize).clamp(2, 96);
    for i in 1..steps {
        let a = a0 + sweep * i as f32 / steps as f32;
        out.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
    Some(tidy(&out))
}

/// Где лестница примыкает к помосту: ближайшая к `at` точка контура, наружная нормаль
/// ребра и его номер.
pub fn stair_frame(p: &[[f32; 2]], at: [f32; 2]) -> Option<([f32; 2], [f32; 2], usize)> {
    let nrm = normals(p);
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let t = (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / (dx * dx + dy * dy).max(1e-9)).clamp(0.0, 1.0);
            let q = [a[0] + dx * t, a[1] + dy * t];
            ((q[0] - at[0]).hypot(q[1] - at[1]), q, i)
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, q, i)| (q, nrm[i], i))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f32; 2]; 4] = [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]];

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn edges_look_outward_whichever_way_the_outline_runs() {
        assert!(area(&SQUARE) > 0.0);
        assert!(near(normals(&SQUARE)[0], [0.0, -1.0]), "верхнее ребро смотрит вверх");
        let back: Vec<[f32; 2]> = SQUARE.iter().rev().copied().collect();
        assert!(area(&back) < 0.0);
        // Тот же квадрат задом наперёд: ребро 0 идёт от (1, 3) к (3, 3) — оно нижнее.
        assert!(near(normals(&back)[0], [0.0, 1.0]), "нижнее ребро смотрит вниз: {:?}", normals(&back)[0]);
        // Сдвиг наружу: углы квадрата уходят по диагонали, ширина полосы — ровно сдвиг.
        let out = offset(&SQUARE, &[true; 4], 0.5);
        assert!(near(out[0], [0.5, 0.5]) && near(out[2], [3.5, 3.5]), "{out:?}");
        // Правое ребро у стены: полоса верхнего ребра кончается над углом прямым срезом.
        let cut = offset(&SQUARE, &[true, false, true, true], 0.5);
        assert!(near(cut[1], [3.0, 0.5]) && near(cut[2], [3.0, 3.5]), "{cut:?}");
        assert_eq!(tidy(&[[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]).len(), 3);
    }

    #[test]
    fn concave_outline_splits_into_triangles_that_cover_it() {
        // Буква «Г»: вогнутый угол в (2, 2).
        let l = [[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [2.0, 2.0], [2.0, 4.0], [0.0, 4.0]];
        let tris = triangulate(&l);
        assert_eq!(tris.len(), 4);
        let sum: f32 = tris.iter().map(|t| (cross(l[t[0]], l[t[1]], l[t[2]]) / 2.0).abs()).sum();
        assert!((sum - area(&l).abs()).abs() < 1e-4, "площадь треугольников {sum}, контура {}", area(&l));
        // Ни один треугольник не лезет в вырез.
        for t in &tris {
            let c = [(l[t[0]][0] + l[t[1]][0] + l[t[2]][0]) / 3.0, (l[t[0]][1] + l[t[1]][1] + l[t[2]][1]) / 3.0];
            assert!(contains(&l, c), "{c:?}");
        }
        assert!(contains(&l, [1.0, 3.0]) && !contains(&l, [3.0, 3.0]));
        let back: Vec<[f32; 2]> = l.iter().rev().copied().collect();
        assert_eq!(triangulate(&back).len(), 4);
        assert_eq!(triangulate(&circle([5.0, 5.0], 2.0)).len(), circle([5.0, 5.0], 2.0).len() - 2);
    }

    #[test]
    fn circle_by_the_wall_and_a_tier_behind_an_arc() {
        // Помост реактора: круг радиуса 7,7 с центром за стеной зала — в зале от него сегмент.
        let dais = within(&circle([33.2, 17.0], 7.7), [15.6, 12.3, 30.6, 21.7]);
        assert!(dais.iter().all(|p| p[0] >= 15.6 - 1e-4 && p[0] <= 30.6 + 1e-4 && p[1] >= 12.3 - 1e-4 && p[1] <= 21.7 + 1e-4));
        assert!(contains(&dais, [29.0, 17.0]) && !contains(&dais, [20.0, 17.0]));
        // Ярус зала: прямоугольник без круга, выкусившего левый край.
        let tier = bitten([24.0, 3.0, 34.0, 8.0], [18.9, 5.5], 6.07).expect("круг режет левый край в двух местах");
        assert!(contains(&tier, [30.0, 5.5]) && !contains(&tier, [24.5, 5.5]), "{tier:?}");
        let whole = 10.0 * 5.0;
        assert!(area(&tier).abs() < whole && area(&tier).abs() > whole * 0.8, "{}", area(&tier));
        assert!(bitten([24.0, 3.0, 34.0, 8.0], [50.0, 50.0], 2.0).is_none(), "круг в стороне");
        assert!(bitten([24.0, 3.0, 34.0, 8.0], [29.0, 5.5], 1.0).is_none(), "круг целиком внутри");
        // Лестница встаёт к ближайшему ребру и смотрит наружу.
        let (at, n, edge) = stair_frame(&SQUARE, [2.2, 3.4]).unwrap();
        assert!(near(at, [2.2, 3.0]) && near(n, [0.0, 1.0]) && edge == 2, "{at:?} {n:?} {edge}");
    }
}
