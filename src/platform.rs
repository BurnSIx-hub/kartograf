//! Помост: площадка выше пола — сцена, кафедра, помост реактора, ступени зала.
//!
//! Высоту на карте сверху показывают так, как на рисованных боевых картах: площадка
//! светлее пола, по её краю светлое ребро, наружу от края — боковая стенка (чем выше
//! помост, тем она шире: что выше, то ближе к глазу) и тень у подножия. Лестница —
//! ступени от края наружу, кверху шире. Стенка и тень идут только по свободным краям:
//! край вдоль стены комнаты их не даёт.
//!
//! Роспись по желанию: перила по свободным краям и по бокам лестниц (столбики с шагом
//! до 1,6 клетки), огоньки (полоски по кромке, на ступенях и на столбиках перил),
//! решётки у края — полоса с прорезями вдоль контура.
//!
//! Здесь — данные и геометрия; рисует `app::draw_platform`.

use serde::{Deserialize, Serialize};

/// Глубина ступени лестницы, клетки.
pub const TREAD: f32 = 0.4;
/// Перила стоят внутри площадки на столько от края, клетки.
pub const RAIL_IN: f32 = 0.12;
/// Наибольший шаг столбиков перил, клетки.
pub const POST_STEP: f32 = 1.6;
/// Решётка у края: от края внутрь — от и до, клетки.
pub const GRATE_IN: (f32, f32) = (0.3, 0.68);

/// Решётка у края помоста: где (точка у края — берётся ближайшая на контуре) и длина
/// вдоль края, клетки.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grate {
    pub at: [f32; 2],
    pub length: f32,
}

/// Лестница на помост: где примыкает (точка у края — берётся ближайшая на контуре),
/// ширина наверху и число ступеней. Пандус — та же лестница гладким скатом: длина та же,
/// что у ступеней, на карте — без рёбер ступеней, в 3D — наклонная плоскость.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stair {
    pub at: [f32; 2],
    pub width: f32,
    pub steps: u8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ramp: bool,
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
    /// Перила по свободным краям и по бокам лестниц.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rail: bool,
    /// Огоньки и их цвет: полоски по кромке, на ступенях, на столбиках перил.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lights: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grates: Vec<Grate>,
    /// Открытые края — номера рёбер чистого контура (`tidy`), ребро `k` идёт от точки `k`
    /// к `k + 1`. У открытого края нет ни стенки, ни кромки, ни тени, ни перил: помост
    /// там сходит в пол той же высоты или край просто не нужен.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<usize>,
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

/// Огоньки по умолчанию — холодный голубой.
pub fn cyan() -> [u8; 3] {
    [110, 228, 255]
}

impl Platform {
    pub fn new(points: Vec<[f32; 2]>, height: f32) -> Self {
        Self { points, height, color: light(), strength: tenth(), side: steel(), stairs: Vec::new(), rail: false, lights: None, grates: Vec::new(), open: Vec::new() }
    }

    /// Края со стенкой: из свободных (`free`, не вдоль стен комнат) убраны открытые.
    pub fn sides(&self, mut free: Vec<bool>) -> Vec<bool> {
        for &k in &self.open {
            if let Some(f) = free.get_mut(k) {
                *f = false;
            }
        }
        free
    }

    /// Открыть край `k` или вернуть ему стенку.
    pub fn set_open(&mut self, k: usize, open: bool) {
        self.open.retain(|&x| x != k);
        if open {
            self.open.push(k);
            self.open.sort_unstable();
        }
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

    /// Сдвинуть помост целиком — с лестницами и решётками.
    pub fn shift(&mut self, d: [f32; 2]) {
        let mv = |q: &mut [f32; 2]| *q = [q[0] + d[0], q[1] + d[1]];
        self.points.iter_mut().for_each(mv);
        self.stairs.iter_mut().for_each(|s| mv(&mut s.at));
        self.grates.iter_mut().for_each(|g| mv(&mut g.at));
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
    ellipse(c, r, r)
}

/// Овал с полуосями `rx`, `ry` вдоль осей карты — так же многоугольником.
pub fn ellipse(c: [f32; 2], rx: f32, ry: f32) -> Vec<[f32; 2]> {
    // Длина эллипса по Рамануджану; у круга — ровно 2πr.
    let per = std::f32::consts::PI * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
    let n = ((per / 0.33).ceil() as usize).clamp(16, 96);
    (0..n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            [c[0] + rx * a.cos(), c[1] + ry * a.sin()]
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

/// Лестница на месте: точка у края `o`, нормаль наружу, касательная, полуширина наверху
/// и внизу, длина марша.
#[derive(Clone, Copy, Debug)]
pub struct StairGeo {
    pub o: [f32; 2],
    pub nrm: [f32; 2],
    pub tan: [f32; 2],
    pub w1: f32,
    pub w0: f32,
    pub run: f32,
}

impl StairGeo {
    /// Полуширина на расстоянии `s` от края наружу (выше края — как наверху).
    pub fn half(&self, s: f32) -> f32 {
        self.w1 + (self.w0 - self.w1) * (s.max(0.0) / self.run)
    }

    /// Точка: `s` — от края помоста наружу, `u` — поперёк, в долях полуширины.
    pub fn pt(&self, s: f32, u: f32) -> [f32; 2] {
        let h = self.half(s);
        [self.o[0] + self.nrm[0] * s + self.tan[0] * u * h, self.o[1] + self.nrm[1] * s + self.tan[1] * u * h]
    }

    /// Точка: `s` наружу, `x` поперёк — в клетках от оси.
    pub fn at(&self, s: f32, x: f32) -> [f32; 2] {
        [self.o[0] + self.nrm[0] * s + self.tan[0] * x, self.o[1] + self.nrm[1] * s + self.tan[1] * x]
    }

    /// Контур марша: верх у края, низ у пола.
    pub fn outline(&self) -> [[f32; 2]; 4] {
        [self.pt(0.0, -1.0), self.pt(0.0, 1.0), self.pt(self.run, 1.0), self.pt(self.run, -1.0)]
    }

    /// Точка `q` в осях лестницы: (наружу от края, поперёк).
    fn local(&self, q: [f32; 2]) -> (f32, f32) {
        let d = [q[0] - self.o[0], q[1] - self.o[1]];
        (d[0] * self.nrm[0] + d[1] * self.nrm[1], d[0] * self.tan[0] + d[1] * self.tan[1])
    }
}

pub fn stair_geo(p: &[[f32; 2]], st: &Stair) -> Option<StairGeo> {
    let (o, nrm, _) = stair_frame(p, st.at)?;
    let steps = st.steps.clamp(1, 16) as f32;
    let w1 = st.width.clamp(0.4, 12.0) / 2.0;
    Some(StairGeo { o, nrm, tan: [-nrm[1], nrm[0]], w1, w0: w1 * (1.0 - 0.06 * steps).max(0.6), run: steps * TREAD })
}

/// Куски ломаной вне прямоугольника лестницы (от `s0` до `s1` наружу, поперёк — до `w`).
fn outside_of(line: &[[f32; 2]], g: &StairGeo, s0: f32, s1: f32, w: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let lerp = |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    for k in 0..line.len().saturating_sub(1) {
        let (a, b) = (line[k], line[k + 1]);
        let ((sa, ua), (sb, ub)) = (g.local(a), g.local(b));
        // Лян — Барски: часть отрезка внутри прямоугольника — от t0 до t1.
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (pp, qq) in [(-(sb - sa), sa - s0), (sb - sa, s1 - sa), (-(ub - ua), ua + w), (ub - ua, w - ua)] {
            if pp.abs() < 1e-9 {
                if qq < 0.0 {
                    t0 = 2.0;
                }
            } else {
                let r = qq / pp;
                if pp < 0.0 { t0 = t0.max(r) } else { t1 = t1.min(r) }
            }
        }
        if t0 < t1 {
            if t0 > 0.0 {
                if cur.is_empty() {
                    cur.push(a);
                }
                cur.push(lerp(a, b, t0));
            }
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            if t1 < 1.0 {
                cur = vec![lerp(a, b, t1), b];
            }
        } else {
            if cur.is_empty() {
                cur.push(a);
            }
            cur.push(b);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Перила: ломаные по свободным краям (на `RAIL_IN` внутри площадки), разомкнутые у каждой
/// лестницы, и по бокам лестниц до нижней ступени. Второе — замкнута ли ломаная.
pub fn rails(p: &[[f32; 2]], free: &[bool], stairs: &[Stair]) -> Vec<(Vec<[f32; 2]>, bool)> {
    let n = p.len();
    if n < 3 || free.len() != n {
        return Vec::new();
    }
    let ring = offset(p, free, -RAIL_IN);
    let mut runs: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    if free.iter().all(|&f| f) {
        let mut l = ring.clone();
        l.push(ring[0]);
        runs.push((l, true));
    } else if let Some(start) = (0..n).find(|&i| !free[i]) {
        let mut cur: Vec<[f32; 2]> = Vec::new();
        for k in 1..=n {
            let i = (start + k) % n;
            if free[i] {
                if cur.is_empty() {
                    cur.push(ring[i]);
                }
                cur.push(ring[(i + 1) % n]);
            } else if !cur.is_empty() {
                runs.push((std::mem::take(&mut cur), false));
            }
        }
        if !cur.is_empty() {
            runs.push((cur, false));
        }
    }
    let geos: Vec<StairGeo> = stairs.iter().filter_map(|s| stair_geo(p, s)).collect();
    for g in &geos {
        let w = (g.w1 - RAIL_IN).max(0.1);
        let mut next = Vec::new();
        for (line, closed) in runs {
            let mut pieces = outside_of(&line, g, -0.6, g.run, w);
            let whole = pieces.len() == 1 && pieces[0].len() == line.len();
            if closed && whole {
                next.push((line, true));
                continue;
            }
            // Замкнутое кольцо, разрезанное лестницей: конец и начало — один кусок.
            if closed && pieces.len() >= 2 {
                let (first, last) = (pieces[0][0], *pieces[pieces.len() - 1].last().unwrap());
                let same = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5;
                if same(first, line[0]) && same(last, line[line.len() - 1]) {
                    let head = pieces.remove(0);
                    pieces.last_mut().unwrap().extend(head.into_iter().skip(1));
                }
            }
            next.extend(pieces.into_iter().map(|l| (l, false)));
        }
        runs = next;
    }
    // Концы перил у проходов: от них идут перила лестниц (на кривом крае конец перил
    // помоста лежит не там, где верх лестницы, — иначе вышло бы два столбика рядом).
    let ends: Vec<[f32; 2]> = runs.iter().filter(|r| !r.1).flat_map(|r| [r.0[0], r.0[r.0.len() - 1]]).collect();
    for g in &geos {
        for side in [-1.0f32, 1.0] {
            let k = 6;
            let mut line: Vec<[f32; 2]> = (0..=k)
                .map(|j| {
                    let s = -RAIL_IN + (g.run - 0.08 + RAIL_IN) * j as f32 / k as f32;
                    g.at(s, side * (g.half(s) - RAIL_IN))
                })
                .collect();
            let top = line[0];
            if let Some(&e) = ends.iter().filter(|e| (e[0] - top[0]).hypot(e[1] - top[1]) < 0.8).min_by(|a, b| (a[0] - top[0]).hypot(a[1] - top[1]).total_cmp(&(b[0] - top[0]).hypot(b[1] - top[1]))) {
                line[0] = e;
            }
            runs.push((line, false));
        }
    }
    runs
}

/// Столбики перил по ломаной: на концах и через равные шаги не длиннее `POST_STEP`.
pub fn posts(line: &[[f32; 2]], closed: bool) -> Vec<[f32; 2]> {
    let lens: Vec<f32> = line.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).collect();
    let total: f32 = lens.iter().sum();
    if line.is_empty() {
        return Vec::new();
    }
    if total < 0.25 {
        return vec![line[0]];
    }
    let k = ((total / POST_STEP).ceil() as usize).max(1);
    let last = if closed { k - 1 } else { k };
    (0..=last)
        .map(|j| {
            let mut s = total * j as f32 / k as f32;
            for (i, &l) in lens.iter().enumerate() {
                if s <= l || i == lens.len() - 1 {
                    let t = (s / l.max(1e-6)).clamp(0.0, 1.0);
                    return [line[i][0] + (line[i + 1][0] - line[i][0]) * t, line[i][1] + (line[i + 1][1] - line[i][1]) * t];
                }
                s -= l;
            }
            line[0]
        })
        .collect()
}

/// Решётка у края: пары точек (ближе к краю, дальше от края) вдоль контура с шагом около
/// `step`, по длине решётки с серединой у ближайшей к `at` точки контура.
pub fn grate_strip(p: &[[f32; 2]], g: &Grate, step: f32) -> Vec<([f32; 2], [f32; 2])> {
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let all = vec![true; n];
    let (a, b) = (offset(p, &all, -GRATE_IN.0), offset(p, &all, -GRATE_IN.1));
    let lens: Vec<f32> = (0..n).map(|i| (p[(i + 1) % n][0] - p[i][0]).hypot(p[(i + 1) % n][1] - p[i][1])).collect();
    let total: f32 = lens.iter().sum();
    let mut start = vec![0.0f32; n];
    for i in 1..n {
        start[i] = start[i - 1] + lens[i - 1];
    }
    // Где на контуре решётка: путь вдоль края до ближайшей к `at` точки.
    let mid = (0..n)
        .map(|i| {
            let (q0, q1) = (p[i], p[(i + 1) % n]);
            let (dx, dy) = (q1[0] - q0[0], q1[1] - q0[1]);
            let t = (((g.at[0] - q0[0]) * dx + (g.at[1] - q0[1]) * dy) / (dx * dx + dy * dy).max(1e-9)).clamp(0.0, 1.0);
            ((q0[0] + dx * t - g.at[0]).hypot(q0[1] + dy * t - g.at[1]), start[i] + lens[i] * t)
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map_or(0.0, |(_, s)| s);
    let len = g.length.clamp(0.3, total * 0.9);
    let k = ((len / step.max(0.02)).ceil() as usize).max(1);
    (0..=k)
        .map(|j| {
            let s = (mid - len / 2.0 + len * j as f32 / k as f32).rem_euclid(total);
            let i = (0..n).rev().find(|&i| start[i] <= s).unwrap_or(0);
            let t = ((s - start[i]) / lens[i].max(1e-6)).clamp(0.0, 1.0);
            let j1 = (i + 1) % n;
            let e = [p[j1][0] - p[i][0], p[j1][1] - p[i][1]];
            let ee = (e[0] * e[0] + e[1] * e[1]).max(1e-9);
            // Точка сдвинутого контура против той же точки края: на прямом крае полоса — ровный
            // прямоугольник, у угла её точки сходятся к вершине сдвинутого контура.
            let inner = |r: &[[f32; 2]]| {
                let proj = |q: [f32; 2]| ((q[0] - p[i][0]) * e[0] + (q[1] - p[i][1]) * e[1]) / ee;
                let (ta, tb) = (proj(r[i]), proj(r[j1]));
                let u = if (tb - ta).abs() < 1e-6 { 0.0 } else { ((t - ta) / (tb - ta)).clamp(0.0, 1.0) };
                [r[i][0] + (r[j1][0] - r[i][0]) * u, r[i][1] + (r[j1][1] - r[i][1]) * u]
            };
            (inner(&a), inner(&b))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f32; 2]; 4] = [[1.0, 1.0], [3.0, 1.0], [3.0, 3.0], [1.0, 3.0]];

    #[test]
    fn open_edges_lose_their_side() {
        let mut pl = Platform::new(SQUARE.to_vec(), 0.3);
        assert_eq!(pl.sides(vec![true; 4]), vec![true; 4]);
        pl.set_open(2, true);
        pl.set_open(2, true);
        assert_eq!(pl.open, vec![2], "дважды — один раз");
        assert_eq!(pl.sides(vec![true, false, true, true]), vec![true, false, false, true], "у стены комнаты стенки и так нет");
        pl.set_open(2, false);
        assert!(pl.open.is_empty());
        assert!(!serde_json::to_string(&pl).unwrap().contains("open"), "пустой список не пишется в файл");
        let old: Platform = serde_json::from_str(r#"{"points": [[1, 1], [3, 1], [3, 3]], "height": 0.3}"#).unwrap();
        assert!(old.open.is_empty(), "старые карты открываются");
    }

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

    #[test]
    fn rail_opens_at_the_stairs_and_runs_down_their_sides() {
        let big = [[0.0, 0.0], [6.0, 0.0], [6.0, 4.0], [0.0, 4.0]];
        // Без лестниц и стен — одно замкнутое кольцо внутри площадки.
        let ring = rails(&big, &[true; 4], &[]);
        assert_eq!(ring.len(), 1);
        assert!(ring[0].1 && near(ring[0].0[0], [RAIL_IN, RAIL_IN]), "{:?}", ring[0].0[0]);
        // Лестница посреди нижнего края: кольцо размыкается в одну ломаную, плюс два бока лестницы.
        let st = Stair { at: [3.0, 4.2], width: 2.0, steps: 3, ramp: false };
        let runs = rails(&big, &[true; 4], std::slice::from_ref(&st));
        assert_eq!(runs.len(), 3, "{runs:?}");
        assert!(runs.iter().all(|r| !r.1));
        // Ни одна точка перил на площадке не лежит в проходе лестницы.
        let g = stair_geo(&big, &st).unwrap();
        for (line, _) in &runs[..1] {
            for q in line {
                let (s, u) = g.local(*q);
                assert!(!(s > -0.5 && u.abs() < g.w1 - RAIL_IN - 1e-3), "точка {q:?} в проходе");
            }
        }
        // Концы ломаной — у краёв прохода.
        let l = &runs[0].0;
        let ends = [g.local(l[0]), g.local(*l.last().unwrap())];
        assert!(ends.iter().all(|e| (e.1.abs() - (g.w1 - RAIL_IN)).abs() < 1e-3), "{ends:?}");
        // Край у стены (правый) перил не даёт: ломаная начинается и кончается у стены.
        let walled = rails(&big, &[true, false, true, true], &[]);
        assert_eq!(walled.len(), 1);
        assert!(!walled[0].1);
        assert!((walled[0].0[0][0] - 6.0).abs() < 1e-4, "начало у стены: {:?}", walled[0].0[0]);
        // Столбики: на концах и не реже чем через POST_STEP.
        let posts = posts(&[[0.0, 0.0], [5.0, 0.0]], false);
        assert_eq!(posts.len(), 5);
        assert!(near(posts[0], [0.0, 0.0]) && near(posts[4], [5.0, 0.0]));
    }

    #[test]
    fn grate_follows_the_edge_inside_the_platform() {
        let big = [[0.0, 0.0], [6.0, 0.0], [6.0, 4.0], [0.0, 4.0]];
        let strip = grate_strip(&big, &Grate { at: [3.0, -0.2], length: 2.0 }, 0.5);
        assert_eq!(strip.len(), 5);
        // Полоса вдоль верхнего края, внутри площадки, ровно от x 2 до 4: прорези поперёк края.
        assert!(near(strip[0].0, [2.0, GRATE_IN.0]) && near(strip[0].1, [2.0, GRATE_IN.1]), "{strip:?}");
        assert!(near(strip[4].0, [4.0, GRATE_IN.0]) && near(strip[4].1, [4.0, GRATE_IN.1]), "{strip:?}");
        // У угла решётка заворачивает за него, а не вылезает наружу.
        let corner = grate_strip(&big, &Grate { at: [6.0, 0.0], length: 2.0 }, 0.25);
        assert!(corner.iter().all(|(a, b)| contains(&big, *a) && contains(&big, *b)), "{corner:?}");
        // Овал вписан в свою рамку.
        assert!(ellipse([0.0, 0.0], 3.0, 1.0).iter().all(|q| q[1].abs() <= 1.0 + 1e-4 && q[0].abs() <= 3.0 + 1e-4));
    }
}
