//! Прямой скелет многоугольника (straight skeleton) — плоские грани скатов крыши.
//!
//! Стороны контура разом сдвигаются внутрь с одной скоростью; углы идут по
//! биссектрисам и чертят рёбра крыши: выпуклый угол — вальмовое ребро или конёк,
//! вогнутый — ендову. Полоса, которую замела сторона, — её скат: плоскость от
//! свеса вверх. «Каждая точка сходит к ближайшей стороне» у невыпуклого дома
//! давало круглые конусы вокруг вогнутых углов и плывущие гребни.
//!
//! Сторона с весом 0 стоит на месте — это щипец (торец двускатной) или стык с
//! соседним куском: ската к ней нет, скаты соседних сторон доходят до неё.
//!
//! Считается пошагово от события к событию: сторона стянулась в точку (ребро
//! дошло до ребра) или вогнутый угол упёрся в сторону напротив (контур делится
//! надвое или сливается с двором). Одновременные события (у прямоугольных домов
//! они все такие) разбираются по одному с допуском.

type P = [f64; 2];

/// Сторона контура: от `a` к `b`; `w` — 1 у свеса, 0 у щипца и стыка.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Side {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub w: f32,
}

/// Кусок ската — выпуклый (трапеция или треугольник) — и номер его стороны по
/// порядку во всех кольцах.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub side: usize,
    pub poly: Vec<[f32; 2]>,
}

/// Ребро крыши между скатами двух сторон: `valley` — ендова, иначе конёк или
/// вальмовое ребро.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arc {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub valley: bool,
    pub sides: [usize; 2],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub faces: Vec<Face>,
    pub arcs: Vec<Arc>,
}

const EPS: f64 = 1e-7;
const NONE: usize = usize::MAX;

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Прямая стороны: точка, направление, нормаль внутрь дома (слева от хода), вес.
struct Edge {
    p: P,
    u: P,
    n: P,
    w: f64,
    id: usize,
}

/// Угол фронта между сторонами `l` (приходит) и `r` (уходит).
struct Vert {
    l: usize,
    r: usize,
    p0: P,
    t0: f64,
    vel: P,
    prev: usize,
    next: usize,
    alive: bool,
    reflex: bool,
    /// Стороны легли друг на друга навстречу: игла нулевой ширины — конёк.
    spike: bool,
    /// С какого времени полоса стороны `r` (до следующего угла) ещё не записана.
    emit: f64,
}

struct Front {
    e: Vec<Edge>,
    v: Vec<Vert>,
    t: f64,
    faces: Vec<(usize, [P; 4])>,
    arcs: Vec<(P, P, bool, [usize; 2])>,
    failed: bool,
}

enum Kin {
    Move(P),
    /// Стороны на одной прямой, но с разной скоростью: между ними растёт уступ.
    Step,
    Spike,
}

impl Front {
    fn pos(&self, k: usize) -> P {
        let v = &self.v[k];
        add(v.p0, mul(v.vel, self.t - v.t0))
    }

    fn pos_at(&self, k: usize, t: f64) -> P {
        let v = &self.v[k];
        add(v.p0, mul(v.vel, t - v.t0))
    }

    fn kin(&self, l: usize, r: usize) -> Kin {
        let (a, b) = (&self.e[l], &self.e[r]);
        let det = cross(a.n, b.n);
        if det.abs() > 1e-9 {
            return Kin::Move([(a.w * b.n[1] - b.w * a.n[1]) / det, (a.n[0] * b.w - b.n[0] * a.w) / det]);
        }
        if dot(a.n, b.n) > 0.0 {
            if (a.w - b.w).abs() < 1e-9 { Kin::Move(mul(a.n, a.w)) } else { Kin::Step }
        } else {
            Kin::Spike
        }
    }

    fn push(&mut self, l: usize, r: usize, p: P, vel: P, spike: bool) -> usize {
        let reflex = cross(self.e[l].u, self.e[r].u) < -1e-9;
        self.v.push(Vert { l, r, p0: p, t0: self.t, vel, prev: NONE, next: NONE, alive: true, reflex, spike, emit: self.t });
        self.v.len() - 1
    }

    fn link(&mut self, a: usize, b: usize) {
        self.v[a].next = b;
        self.v[b].prev = a;
    }

    /// Новый угол между сторонами `l` и `r` в точке `p`: один, или два с уступом
    /// между ними. Возвращает первый и последний по ходу.
    fn spawn(&mut self, l: usize, r: usize, p: P) -> (usize, usize) {
        match self.kin(l, r) {
            Kin::Move(vel) => {
                let k = self.push(l, r, p, vel, false);
                (k, k)
            }
            Kin::Spike => {
                let k = self.push(l, r, p, [0.0, 0.0], true);
                (k, k)
            }
            Kin::Step => {
                // Уступ — сторона без скорости поперёк прямой: от стоящей к ушедшей.
                let n = self.e[l].n;
                let d = if self.e[r].w > self.e[l].w { n } else { mul(n, -1.0) };
                self.e.push(Edge { p, u: d, n: [-d[1], d[0]], w: 0.0, id: NONE });
                let s = self.e.len() - 1;
                let (va, vb) = (mul(n, self.e[l].w), mul(n, self.e[r].w));
                let a = self.push(l, s, p, va, false);
                let b = self.push(s, r, p, vb, false);
                self.link(a, b);
                (a, b)
            }
        }
    }

    /// Полоса, что замела сторона от угла `k` до следующего с прошлой записи.
    fn emit(&mut self, k: usize) {
        let n = self.v[k].next;
        let from = self.v[k].emit;
        self.v[k].emit = self.t;
        let e = &self.e[self.v[k].r];
        if e.id == NONE || e.w <= 0.0 || self.t - from < 1e-12 || n == NONE {
            return;
        }
        // Следующий угол мог родиться позже записи — тогда от его рождения.
        let from = from.max(self.v[n].t0).max(self.v[k].t0);
        let q = [self.pos_at(k, from), self.pos_at(n, from), self.pos(n), self.pos(k)];
        let area: f64 = (0..4).map(|i| cross(q[i], q[(i + 1) % 4])).sum::<f64>().abs() / 2.0;
        if area > 1e-12 {
            self.faces.push((e.id, q));
        }
    }

    /// Угол кончился в точке `p`: его путь — ребро крыши.
    fn kill(&mut self, k: usize, p: P) {
        let v = &self.v[k];
        let (l, r) = (&self.e[v.l], &self.e[v.r]);
        if v.alive && l.id != NONE && r.id != NONE && l.w > 0.0 && r.w > 0.0 && dist(v.p0, p) > 1e-6 && dot(l.n, r.n) < 0.9999 {
            self.arcs.push((v.p0, p, v.reflex, [l.id, r.id]));
        }
        self.v[k].alive = false;
    }

    /// Конёк между двумя сторонами, что сошлись навстречу.
    fn ridge(&mut self, a: P, b: P, l: usize, r: usize) {
        let (el, er) = (&self.e[l], &self.e[r]);
        if el.id != NONE && er.id != NONE && el.w > 0.0 && er.w > 0.0 && dist(a, b) > 1e-6 {
            self.arcs.push((a, b, false, [el.id, er.id]));
        }
    }

    /// Сторона между `k` и следующим стянулась в точку: два угла — в один.
    fn merge(&mut self, k: usize) {
        let n = self.v[k].next;
        if n == k {
            let p = self.pos(k);
            self.kill(k, p);
            return;
        }
        let (pk, nn) = (self.v[k].prev, self.v[n].next);
        let (a, b) = (self.pos(k), self.pos(n));
        let p = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        self.emit(pk);
        self.emit(k);
        self.emit(n);
        let (l, r) = (self.v[k].l, self.v[n].r);
        self.kill(k, p);
        self.kill(n, p);
        if pk == n {
            return;
        }
        let (f, last) = self.spawn(l, r, p);
        self.link(pk, f);
        self.link(last, nn);
    }

    /// Петля из двух углов: две стороны легли друг на друга — конёк между углами.
    fn two_loop(&mut self, a: usize) {
        let b = self.v[a].next;
        self.emit(a);
        self.emit(b);
        let (pa, pb) = (self.pos(a), self.pos(b));
        self.ridge(pa, pb, self.v[a].r, self.v[b].r);
        self.kill(a, pa);
        self.kill(b, pb);
    }

    /// Вогнутый угол `x` упёрся в сторону от `k` до следующего: фронт делится.
    fn pinch(&mut self, x: usize, k: usize) {
        let n = self.v[k].next;
        let (px, nx) = (self.v[x].prev, self.v[x].next);
        let e = self.v[k].r;
        let p = self.pos(x);
        self.emit(px);
        self.emit(x);
        self.emit(k);
        let (l, r) = (self.v[x].l, self.v[x].r);
        self.kill(x, p);
        let (f1, l1) = self.spawn(l, e, p);
        self.link(px, f1);
        self.link(l1, n);
        let (f2, l2) = self.spawn(e, r, p);
        self.link(k, f2);
        self.link(l2, nx);
    }

    /// Игла: стороны угла `m` лежат друг на друге навстречу. Она складывается в
    /// конёк до ближнего из соседних углов, дальше идёт угол с его стороной.
    fn spike(&mut self, m: usize) {
        let (a, c) = (self.v[m].prev, self.v[m].next);
        if a == c {
            self.two_loop(m);
            return;
        }
        let (p, pa, pc) = (self.pos(m), self.pos(a), self.pos(c));
        self.emit(a);
        self.emit(m);
        let (l, r) = (self.v[m].l, self.v[m].r);
        if dist(p, pc) <= dist(p, pa) {
            self.emit(c);
            self.ridge(p, pc, l, r);
            let (cn, cr) = (self.v[c].next, self.v[c].r);
            self.kill(m, p);
            self.kill(c, pc);
            let (f, last) = self.spawn(l, cr, pc);
            self.link(a, f);
            self.link(last, cn);
        } else {
            let (ap, al) = (self.v[a].prev, self.v[a].l);
            self.emit(ap);
            self.ridge(p, pa, l, r);
            self.kill(m, p);
            self.kill(a, pa);
            let (f, last) = self.spawn(al, r, pa);
            self.link(ap, f);
            self.link(last, c);
        }
    }

    fn alive(&self) -> Vec<usize> {
        (0..self.v.len()).filter(|&k| self.v[k].alive).collect()
    }

    /// Подходит ли вогнутый угол `x` к стороне от `k`: расстояние до её прямой и
    /// скорость сближения (минус — сближаются).
    fn gap(&self, x: usize, k: usize, t: f64) -> (f64, f64) {
        let e = &self.e[self.v[k].r];
        (dot(e.n, sub(self.pos_at(x, t), e.p)) - e.w * t, dot(e.n, self.v[x].vel) - e.w)
    }

    /// Лежит ли угол `x` в момент `t` в пределах стороны от `k` до следующего.
    fn within(&self, x: usize, k: usize, t: f64) -> bool {
        let n = self.v[k].next;
        let u = self.e[self.v[k].r].u;
        let (a, b, p) = (self.pos_at(k, t), self.pos_at(n, t), self.pos_at(x, t));
        let (s, len) = (dot(sub(p, a), u), dot(sub(b, a), u));
        s > -EPS && s < len + EPS
    }

    /// Всё, что случилось в этот момент, — по одному, пока фронт не станет чистым.
    fn settle(&mut self) {
        for _ in 0..self.v.len() * 8 + 64 {
            let alive = self.alive();
            if let Some(&k) = alive.iter().find(|&&k| self.v[k].spike) {
                self.spike(k);
                continue;
            }
            if let Some(&k) = alive.iter().find(|&&k| self.v[k].next == k) {
                let p = self.pos(k);
                self.kill(k, p);
                continue;
            }
            if let Some(&k) = alive.iter().find(|&&k| self.v[self.v[k].next].next == k) {
                self.two_loop(k);
                continue;
            }
            // Сторона нулевой длины, которая не растёт.
            let zero = alive.iter().copied().find(|&k| {
                let n = self.v[k].next;
                let u = self.e[self.v[k].r].u;
                dist(self.pos(k), self.pos(n)) < EPS && dot(sub(self.v[n].vel, self.v[k].vel), u) <= 1e-9
            });
            if let Some(k) = zero {
                self.merge(k);
                continue;
            }
            let mut hit = None;
            'find: for &x in alive.iter().filter(|&&x| self.v[x].reflex) {
                for &k in &alive {
                    if k == x || k == self.v[x].prev {
                        continue;
                    }
                    let (f, df) = self.gap(x, k, self.t);
                    if df < -1e-9 && f.abs() < EPS && self.within(x, k, self.t) {
                        hit = Some((x, k));
                        break 'find;
                    }
                }
            }
            match hit {
                Some((x, k)) => self.pinch(x, k),
                None => return,
            }
        }
        self.failed = true;
    }

    /// Ближайшее событие после текущего момента: время и что случится.
    fn next_event(&self) -> Option<(f64, Option<(usize, usize)>, usize)> {
        let alive = self.alive();
        let mut best: Option<(f64, Option<(usize, usize)>, usize)> = None;
        let mut take = |t: f64, split: Option<(usize, usize)>, k: usize| {
            if best.is_none_or(|b| t < b.0) {
                best = Some((t, split, k));
            }
        };
        for &k in &alive {
            let n = self.v[k].next;
            let u = self.e[self.v[k].r].u;
            let len = dot(sub(self.pos(n), self.pos(k)), u);
            let dl = dot(sub(self.v[n].vel, self.v[k].vel), u);
            if dl < -1e-12 {
                take(self.t + len.max(0.0) / -dl, None, k);
            }
        }
        for &x in alive.iter().filter(|&&x| self.v[x].reflex) {
            for &k in &alive {
                if k == x || k == self.v[x].prev {
                    continue;
                }
                let (f, df) = self.gap(x, k, self.t);
                if df < -1e-12 && f > -EPS {
                    let te = self.t + f.max(0.0) / -df;
                    if self.within(x, k, te) {
                        take(te, Some((x, k)), k);
                    }
                }
            }
        }
        best
    }
}

/// Внутри ли колец точка (по чётности: двор — снаружи).
fn inside(rings: &[Vec<(P, P)>], p: P) -> bool {
    let mut c = false;
    for r in rings {
        for &(a, b) in r {
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                c = !c;
            }
        }
    }
    c
}

/// Скаты и рёбра крыши по кольцам (первое — дом, остальные — дворы). Номер
/// стороны — по порядку во всех кольцах подряд. `None` — контур вырожден
/// (самопересечение, игла) или скаты не покрыли дом: тогда рисуют по-старому.
pub fn skeleton(rings: &[Vec<Side>]) -> Option<Skeleton> {
    let f64p = |p: [f32; 2]| [p[0] as f64, p[1] as f64];
    let mut id = 0;
    // Кольца: (от, до, вес, номер) без нулевых сторон.
    let mut loops: Vec<Vec<(P, P, f64, usize)>> = Vec::new();
    for ring in rings {
        let mut out = Vec::new();
        for s in ring {
            let (a, b) = (f64p(s.a), f64p(s.b));
            if dist(a, b) > 1e-6 {
                out.push((a, b, s.w as f64, id));
            }
            id += 1;
        }
        if out.len() >= 3 {
            loops.push(out);
        }
    }
    if loops.is_empty() || loops.iter().map(Vec::len).sum::<usize>() > 400 {
        return None;
    }
    let plain: Vec<Vec<(P, P)>> = loops.iter().map(|l| l.iter().map(|s| (s.0, s.1)).collect()).collect();
    // Дом — слева от хода каждой стороны: кольцо, что идёт иначе, разворачивается.
    for ring in &mut loops {
        let votes: i32 = ring
            .iter()
            .map(|&(a, b, ..)| {
                let l = dist(a, b);
                let n = [-(b[1] - a[1]) / l, (b[0] - a[0]) / l];
                let m = [(a[0] + b[0]) / 2.0 + n[0] * 1e-4, (a[1] + b[1]) / 2.0 + n[1] * 1e-4];
                if inside(&plain, m) { 1 } else { -1 }
            })
            .sum();
        if votes < 0 {
            ring.reverse();
            for s in ring.iter_mut() {
                (s.0, s.1) = (s.1, s.0);
            }
        }
    }
    // Соседние стороны на одной прямой с одним весом — одна сторона.
    for ring in &mut loops {
        let mut k = 0;
        while ring.len() > 3 && k < ring.len() {
            let n = (k + 1) % ring.len();
            let (a, b) = (ring[k], ring[n]);
            let (u, w) = (sub(a.1, a.0), sub(b.1, b.0));
            if cross(u, w).abs() < 1e-9 * dist(a.0, a.1) * dist(b.0, b.1) && dot(u, w) > 0.0 && (a.2 - b.2).abs() < 1e-9 {
                ring[k].1 = b.1;
                ring.remove(n);
                if n < k {
                    k -= 1;
                }
            } else {
                k += 1;
            }
        }
    }
    let area: f64 = loops.iter().map(|r| r.iter().map(|s| cross(s.0, s.1)).sum::<f64>() / 2.0).sum();
    if area <= 1e-6 {
        return None;
    }
    let mut f = Front { e: Vec::new(), v: Vec::new(), t: 0.0, faces: Vec::new(), arcs: Vec::new(), failed: false };
    for ring in &loops {
        let base = f.e.len();
        for &(a, b, w, id) in ring {
            let l = dist(a, b);
            let u = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
            f.e.push(Edge { p: a, u, n: [-u[1], u[0]], w, id });
        }
        let n = ring.len();
        let (mut first, mut last) = (NONE, NONE);
        for k in 0..n {
            let (a, b) = f.spawn(base + (k + n - 1) % n, base + k, ring[k].0);
            if first == NONE {
                first = a;
            } else {
                f.link(last, a);
            }
            last = b;
        }
        f.link(last, first);
    }
    f.settle();
    let limit = f.v.len() * 12 + 200;
    for _ in 0..limit {
        if f.failed {
            return None;
        }
        let Some((te, split, k)) = f.next_event() else { break };
        f.t = te.max(f.t);
        match split {
            Some((x, e)) => f.pinch(x, e),
            None => f.merge(k),
        }
        f.settle();
        if f.alive().is_empty() {
            break;
        }
    }
    if f.failed || !f.alive().is_empty() {
        return None;
    }
    let quad_area = |q: &[P; 4]| (0..4).map(|i| cross(q[i], q[(i + 1) % 4])).sum::<f64>().abs() / 2.0;
    let covered: f64 = f.faces.iter().map(|(_, q)| quad_area(q)).sum();
    if (covered - area).abs() > 2e-3 * area + 1e-4 {
        return None;
    }
    let f32p = |p: P| [p[0] as f32, p[1] as f32];
    Some(Skeleton {
        faces: f.faces.iter().map(|(side, q)| Face { side: *side, poly: q.iter().map(|&p| f32p(p)).collect() }).collect(),
        arcs: f.arcs.iter().map(|&(a, b, valley, sides)| Arc { a: f32p(a), b: f32p(b), valley, sides }).collect(),
    })
}

/// Стороны кольца подряд с одним весом.
#[cfg(test)]
pub fn sides_of(ring: &[[f32; 2]], w: f32) -> Vec<Side> {
    (0..ring.len()).map(|k| Side { a: ring[k], b: ring[(k + 1) % ring.len()], w }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(f: &Face) -> f32 {
        let n = f.poly.len();
        (0..n).map(|i| f.poly[i][0] * f.poly[(i + 1) % n][1] - f.poly[(i + 1) % n][0] * f.poly[i][1]).sum::<f32>().abs() / 2.0
    }

    /// Сторона, чей скат накрывает точку, и высота там (расстояние до её прямой).
    fn at(sk: &Skeleton, sides: &[Side], p: [f32; 2]) -> Option<(usize, f32)> {
        let inside = |poly: &[[f32; 2]]| {
            let n = poly.len();
            let s: Vec<f32> = (0..n).map(|i| (poly[(i + 1) % n][0] - poly[i][0]) * (p[1] - poly[i][1]) - (poly[(i + 1) % n][1] - poly[i][1]) * (p[0] - poly[i][0])).collect();
            s.iter().all(|v| *v >= -1e-4) || s.iter().all(|v| *v <= 1e-4)
        };
        let f = sk.faces.iter().find(|f| inside(&f.poly))?;
        let s = sides[f.side];
        let l = (s.b[0] - s.a[0]).hypot(s.b[1] - s.a[1]);
        Some((f.side, ((s.b[0] - s.a[0]) * (p[1] - s.a[1]) - (s.b[1] - s.a[1]) * (p[0] - s.a[0])).abs() / l))
    }

    #[test]
    fn rectangle_is_a_hip_roof_with_a_ridge() {
        let sides = sides_of(&[[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], 1.0);
        let sk = skeleton(&[sides.clone()]).unwrap();
        assert!((sk.faces.iter().map(area).sum::<f32>() - 60.0).abs() < 1e-3);
        // Конёк от (3, 3) до (7, 3) и четыре вальмовых ребра из углов.
        let ridge = sk.arcs.iter().find(|a| (a.a[1] - 3.0).abs() < 1e-4 && (a.b[1] - 3.0).abs() < 1e-4 && (a.a[0] - a.b[0]).abs() > 3.9).expect("конька нет");
        assert!(!ridge.valley);
        assert_eq!(sk.arcs.iter().filter(|a| !a.valley).count(), 5, "{:?}", sk.arcs);
        assert_eq!(at(&sk, &sides, [5.0, 1.0]), Some((0, 1.0)));
        assert_eq!(at(&sk, &sides, [9.0, 3.0]).map(|x| x.0), Some(1));
        assert_eq!(at(&sk, &sides, [5.0, 5.5]).map(|x| x.0), Some(2));
    }

    #[test]
    fn gable_ends_have_no_slope() {
        // Торцы (короткие стороны) — щипцы: два ската до самых торцов, конёк во всю длину.
        let mut sides = sides_of(&[[0.0, 0.0], [10.0, 0.0], [10.0, 6.0], [0.0, 6.0]], 1.0);
        sides[1].w = 0.0;
        sides[3].w = 0.0;
        let sk = skeleton(&[sides.clone()]).unwrap();
        assert!(sk.faces.iter().all(|f| f.side == 0 || f.side == 2));
        assert_eq!(at(&sk, &sides, [0.2, 2.9]), Some((0, 2.9)));
        assert_eq!(at(&sk, &sides, [9.9, 3.5]).map(|x| x.0), Some(2));
        let ridge: Vec<&Arc> = sk.arcs.iter().collect();
        assert_eq!(ridge.len(), 1, "{ridge:?}");
        assert!((ridge[0].a[0] - ridge[0].b[0]).abs() > 9.99 && (ridge[0].a[1] - 3.0).abs() < 1e-4);
    }

    #[test]
    fn l_shape_gets_a_straight_valley_from_the_inner_corner() {
        // Г-образный дом: полка 10×4 и крыло 4×10. Из вогнутого угла (4, 4) ендова
        // идёт по биссектрисе к (2, 2), а не дугой; у старого счёта вокруг угла был конус.
        let ring = [[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [4.0, 4.0], [4.0, 10.0], [0.0, 10.0]];
        let sides = sides_of(&ring, 1.0);
        let sk = skeleton(&[sides.clone()]).unwrap();
        assert!((sk.faces.iter().map(area).sum::<f32>() - 64.0).abs() < 1e-2);
        let valley: Vec<&Arc> = sk.arcs.iter().filter(|a| a.valley).collect();
        assert_eq!(valley.len(), 1, "{:?}", sk.arcs);
        let (a, b) = (valley[0].a, valley[0].b);
        assert!((a[0] - 4.0).abs() < 1e-4 && (a[1] - 4.0).abs() < 1e-4 && (b[0] - 2.0).abs() < 1e-4 && (b[1] - 2.0).abs() < 1e-4, "{a:?} {b:?}");
        // У самого вогнутого угла скат — плоскость своей стороны, а не конус к вершине.
        let (side, z) = at(&sk, &sides, [3.5, 3.9]).unwrap();
        assert_eq!(side, 3, "точка над полкой у угла сходит к стене крыла");
        assert!((z - 0.5).abs() < 1e-4, "высота {z}");
        // Коньки полки и крыла на высоте 2.
        assert!((at(&sk, &sides, [7.0, 2.0]).unwrap().1 - 2.0).abs() < 1e-3);
        assert!((at(&sk, &sides, [2.0, 7.0]).unwrap().1 - 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_courtyard_gets_slopes_too() {
        // Дом 8×8 со двором 2×2: скаты наружу и во двор, конёк кольцом посередине.
        let outer = sides_of(&[[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]], 1.0);
        let yard = sides_of(&[[3.0, 3.0], [5.0, 3.0], [5.0, 5.0], [3.0, 5.0]], 1.0);
        let all: Vec<Side> = outer.iter().chain(&yard).copied().collect();
        let sk = skeleton(&[outer, yard]).unwrap();
        assert!((sk.faces.iter().map(area).sum::<f32>() - 60.0).abs() < 1e-2);
        assert_eq!(at(&sk, &all, [4.0, 1.0]), Some((0, 1.0)));
        // У двора — к стене двора (сторона 4 — верхняя), высота растёт от двора.
        let (side, z) = at(&sk, &all, [4.0, 2.5]).unwrap();
        assert_eq!(side, 4);
        assert!((z - 0.5).abs() < 1e-4);
        // Углы двора вогнуты: от них ендовы.
        assert_eq!(sk.arcs.iter().filter(|a| a.valley).count(), 4, "{:?}", sk.arcs);
    }

    #[test]
    fn a_stepped_and_chamfered_house_is_covered_exactly() {
        // Стенд из отчёта: Г-образный корпус, уступ стены 0,5, срез угла 45°.
        let ring = [[17.0, 3.0], [22.0, 3.0], [22.0, 3.5], [27.0, 3.5], [29.0, 5.5], [29.0, 9.0], [23.0, 9.0], [23.0, 17.0], [17.0, 17.0]];
        let n = ring.len();
        let want: f32 = (0..n).map(|i| ring[i][0] * ring[(i + 1) % n][1] - ring[(i + 1) % n][0] * ring[i][1]).sum::<f32>().abs() / 2.0;
        let sk = skeleton(&[sides_of(&ring, 1.0)]).unwrap();
        assert!((sk.faces.iter().map(area).sum::<f32>() - want).abs() < 0.02, "площадь скатов");
        // Рёбра — прямые отрезки под 45° или вдоль стен; ендовы — от вогнутых углов.
        assert!(sk.arcs.iter().filter(|a| a.valley).count() >= 2);
        assert!(sk.arcs.iter().all(|a| (a.a[0] - a.b[0]).hypot(a.a[1] - a.b[1]) > 1e-3));
    }

    #[test]
    fn a_wall_half_joined_to_a_neighbour_keeps_a_slope_on_the_free_half() {
        // Сторона снизу: левая половина — стык с соседом (вес 0), правая — свес.
        let sides = vec![
            Side { a: [0.0, 0.0], b: [8.0, 0.0], w: 1.0 },
            Side { a: [8.0, 0.0], b: [8.0, 6.0], w: 1.0 },
            Side { a: [8.0, 6.0], b: [4.0, 6.0], w: 1.0 },
            Side { a: [4.0, 6.0], b: [0.0, 6.0], w: 0.0 },
            Side { a: [0.0, 6.0], b: [0.0, 0.0], w: 1.0 },
        ];
        let sk = skeleton(&[sides.clone()]).unwrap();
        assert!((sk.faces.iter().map(area).sum::<f32>() - 48.0).abs() < 1e-2);
        assert!(sk.faces.iter().all(|f| f.side != 3), "к стыку ската нет");
        assert_eq!(at(&sk, &sides, [6.0, 5.5]), Some((2, 0.5)));
    }
}
