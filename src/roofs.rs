//! Крыша в картинку — для выгрузки. На холсте её рисуют сетки egui
//! (`App::draw_roof`), здесь та же геометрия (`Roof::geometry`) считается по
//! точкам: в Foundry крыша уходит отдельным тайлом с прозрачным фоном.

use crate::map::{self, OutlineRoof, Roof, RoofForm, RoofPiece, RoofShape, convex_depth, roof_shade, segment_distance};

/// Ширина гребня на коньке и рёбрах, в клетках.
pub const CAP: f32 = 0.14;
/// Тёмная кромка по свесу, в точках экрана или картинки.
pub const RIM_PX: f32 = 1.5;
/// Гребень темнее самого тёмного ската (тот — 0,6).
pub const CAP_SHADE: f32 = 0.55;
/// Парапет плоской крыши: ширина и шаг рисунка вдоль, в клетках.
pub const PARAPET: f32 = 0.25;
pub const PARAPET_TILE: f32 = 2.0;
/// Тень парапета на кровле внутрь, в клетках, и как темно у самого парапета.
pub const PARAPET_SHADOW: f32 = 0.12;
pub const PARAPET_SHADE: f32 = 0.6;

/// Стороны парапета: трапеции с косым срезом 45° в углах (снаружи по краю,
/// внутрь на ширину). Начало, вдоль стороны и внутрь — для покрытия.
/// Сперва вертикальные: горизонтальные ложатся поверх.
pub fn parapet_sides(lo: [f32; 2], hi: [f32; 2], w: f32) -> Vec<([[f32; 2]; 4], [f32; 2], [f32; 2], [f32; 2])> {
    let (il, it, ir, ib) = (lo[0] + w, lo[1] + w, hi[0] - w, hi[1] - w);
    vec![
        ([lo, [lo[0], hi[1]], [il, ib], [il, it]], lo, [0.0, 1.0], [1.0, 0.0]),
        ([[hi[0], lo[1]], hi, [ir, ib], [ir, it]], [hi[0], lo[1]], [0.0, 1.0], [-1.0, 0.0]),
        ([lo, [hi[0], lo[1]], [ir, it], [il, it]], lo, [1.0, 0.0], [0.0, 1.0]),
        ([[lo[0], hi[1]], hi, [ir, ib], [il, ib]], [lo[0], hi[1]], [1.0, 0.0], [0.0, -1.0]),
    ]
}

/// Картинка крыши: `ppc` точек на клетку. Возвращает картинку и её левый
/// верхний угол в клетках.
/// Предмет на крыше: картинка 1×1 клетка, середина и поворот (по часовой).
pub type Fixture<'a> = (&'a image::RgbaImage, [f32; 2], f32);

/// Точка предмета `f` в картинке: доли 0..1 или `None`, если мимо.
fn fixture_uv(f: &Fixture, p: [f32; 2]) -> Option<(f32, f32)> {
    let (s, c) = f.2.to_radians().sin_cos();
    let (dx, dy) = (p[0] - f.1[0], p[1] - f.1[1]);
    let (u, v) = (c * dx + s * dy + 0.5, -s * dx + c * dy + 0.5);
    ((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v)).then_some((u, v))
}

/// Снег на крыше: цвет; сквозь ровный покров чуть видны ряды покрытия.
const SNOW: [f32; 3] = [236.0, 241.0, 248.0];
const SNOW_DEPTH: f32 = 0.86;

/// Гладкий шум на плоскости, 0..1: по узлам сетки — хеш, между ними — плавно.
fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    let h = |i: i32, j: i32| {
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
    let top = h(i, j) * (1.0 - tx) + h(i + 1, j) * tx;
    let low = h(i, j + 1) * (1.0 - tx) + h(i + 1, j + 1) * tx;
    top * (1.0 - ty) + low * ty
}

/// Сколько снега лежит в точке крыши `p` (клетки карты), 0..1. `edge` — сколько
/// клеток до края: у свеса снег тоньше, сквозь него видно покрытие.
pub fn snow_cover(p: [f32; 2], edge: f32) -> f32 {
    // Ровный покров с лёгкой рябью; изредка — проталина, где снег тоньше.
    let ripple = 0.92 + 0.08 * noise2(p[0] * 3.1, p[1] * 3.1, 11);
    let thaw = noise2(p[0] * 0.45 + 3.0, p[1] * 0.45, 7);
    let thaw = if thaw < 0.2 { 0.55 + 0.45 * (thaw / 0.2) } else { 1.0 };
    SNOW_DEPTH * ripple * thaw * (edge / 0.3).clamp(0.45, 1.0)
}

#[allow(clippy::too_many_arguments)]
pub fn render(roof: &Roof, tex: &image::RgbaImage, tile: [f32; 2], ppc: f32, light: [f32; 2], parapet: Option<&image::RgbaImage>, fixtures: &[Fixture], snow: bool) -> (image::RgbaImage, [f32; 2]) {
    let out = roof.outline();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &out {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    // Сливы выступают за край: картинка шире на их вынос.
    let (mut blo, mut bhi) = (lo, hi);
    for f in fixtures {
        blo = [blo[0].min(f.1[0] - 0.71), blo[1].min(f.1[1] - 0.71)];
        bhi = [bhi[0].max(f.1[0] + 0.71), bhi[1].max(f.1[1] + 0.71)];
    }
    // Начало — на целой точке картинки сцены.
    let origin = [(blo[0] * ppc).floor() / ppc, (blo[1] * ppc).floor() / ppc];
    let (w, h) = (((bhi[0] - origin[0]) * ppc).ceil() as u32 + 1, ((bhi[1] - origin[1]) * ppc).ceil() as u32 + 1);
    let mut img = image::RgbaImage::new(w.max(1), h.max(1));
    let (planes, lines) = roof.geometry();
    let flat = roof.shape == RoofShape::Flat;
    let sample_of = |tex: &image::RgbaImage, u: f32, v: f32| -> [f32; 3] {
        let (tw, th) = (tex.width() as f32, tex.height() as f32);
        let (x, y) = ((u * tw).rem_euclid(tw) - 0.5, (v * th).rem_euclid(th) - 0.5);
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (x - fx, y - fy);
        let at = |xx: f32, yy: f32| {
            let q = tex.get_pixel(xx.rem_euclid(tw) as u32 % tex.width(), yy.rem_euclid(th) as u32 % tex.height()).0;
            [q[0] as f32, q[1] as f32, q[2] as f32]
        };
        let (a, b, c, d) = (at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0));
        std::array::from_fn(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
    };
    let sample = |u: f32, v: f32| sample_of(tex, u, v);
    // Предмет с прозрачностью: ближайшая точка картинки.
    let pick = |img: &image::RgbaImage, u: f32, v: f32| -> [f32; 4] {
        let q = img.get_pixel(((u * img.width() as f32) as u32).min(img.width() - 1), ((v * img.height() as f32) as u32).min(img.height() - 1)).0;
        [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32 / 255.0]
    };
    // Поверх крыши — предметы на ней (обычное «поверх» по прозрачности).
    let over = |p: [f32; 2], mut c: [f32; 3], mut a: f32| -> ([f32; 3], f32) {
        for f in fixtures {
            if let Some((u, v)) = fixture_uv(f, p) {
                let q = pick(f.0, u, v);
                let na = q[3] + a * (1.0 - q[3]);
                if na > 0.0 {
                    c = std::array::from_fn(|k| (q[k] * q[3] + c[k] * a * (1.0 - q[3])) / na);
                }
                a = na;
            }
        }
        (c, a)
    };
    let put = |img: &mut image::RgbaImage, x: u32, y: u32, c: [f32; 3], a: f32| {
        if a > 0.0 {
            img.put_pixel(x, y, image::Rgba([c[0].round().clamp(0.0, 255.0) as u8, c[1].round().clamp(0.0, 255.0) as u8, c[2].round().clamp(0.0, 255.0) as u8, (a * 255.0).round().clamp(0.0, 255.0) as u8]));
        }
    };
    for y in 0..img.height() {
        for x in 0..img.width() {
            let p = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
            let depth = convex_depth(&out, p) * ppc;
            let cov = (depth + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                let (c, a) = over(p, [0.0; 3], 0.0);
                put(&mut img, x, y, c, a);
                continue;
            }
            let Some(pl) = planes.iter().max_by(|a, b| a.inside(p).total_cmp(&b.inside(p))) else { continue };
            let d = [p[0] - pl.origin[0], p[1] - pl.origin[1]];
            let (u, v) = ((d[0] * pl.along[0] + d[1] * pl.along[1]) / tile[0], (d[0] * pl.down[0] + d[1] * pl.down[1]) / tile[1]);
            let mut c = sample(u, v).map(|k| k * roof_shade(pl.down, light, flat));
            // Гребни: ряд того же покрытия вдоль линии, темнее, с кромками.
            for &(a, b) in &lines {
                let dist = segment_distance(p, a, b);
                if dist < CAP / 2.0 {
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len;
                    c = sample(t / tile[0] * 2.0, dist / tile[1]).map(|k| k * CAP_SHADE);
                    if (CAP / 2.0 - dist) * ppc < 1.0 {
                        c = c.map(|k| k * 0.45);
                    }
                }
            }
            // Парапет: полоса по краю, в углах косой срез (горизонтальные поверх),
            // от него на кровлю — тень.
            if let Some(pt) = parapet.filter(|_| flat) {
                let (dl, dr, dt, db) = (p[0] - lo[0], hi[0] - p[0], p[1] - lo[1], hi[1] - p[1]);
                let (dh, dv) = (dt.min(db), dl.min(dr));
                let m = dh.min(dv);
                if m < PARAPET {
                    let along = if dh <= dv { p[0] - lo[0] } else { p[1] - lo[1] };
                    c = sample_of(pt, along / PARAPET_TILE, m / PARAPET);
                } else if m < PARAPET + PARAPET_SHADOW {
                    let k = PARAPET_SHADE + (1.0 - PARAPET_SHADE) * (m - PARAPET) / PARAPET_SHADOW;
                    c = c.map(|v| v * k);
                }
            }
            // Зимой — снег пятнами поверх покрытия; скат темнит и его.
            if snow && !flat {
                // На коньке ветер сдувает — снега меньше.
                let ridge = lines.iter().map(|&(a, b)| segment_distance(p, a, b)).fold(f32::MAX, f32::min);
                let k = snow_cover(p, depth / ppc) * if ridge < CAP / 2.0 { 0.55 } else { 1.0 };
                let s = roof_shade(pl.down, light, flat);
                c = std::array::from_fn(|i| c[i] * (1.0 - k) + SNOW[i] * (0.55 + 0.45 * s) * k);
            }
            if depth < RIM_PX {
                c = c.map(|k| k * 0.4);
            }
            let (c, a) = over(p, c, cov);
            put(&mut img, x, y, c, a);
        }
    }
    (img, origin)
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// Кольцо без лишних узлов (Дуглас — Пекер, допуск `tol` клеток). У контура OSM
/// узлы стоят и посреди прямой стены, и каждый дал бы поперёк ската ложный гребень.
/// Круг купола остаётся многоугольником с гранями.
pub fn simplify_ring(ring: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    let mut pts: Vec<[f32; 2]> = ring.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 4 {
        return pts;
    }
    fn chain(p: &[[f32; 2]], tol: f32, out: &mut Vec<[f32; 2]>) {
        let (a, b) = (p[0], p[p.len() - 1]);
        let far = (1..p.len() - 1).map(|i| (i, segment_distance(p[i], a, b))).max_by(|x, y| x.1.total_cmp(&y.1));
        match far {
            Some((i, d)) if d > tol => {
                chain(&p[..=i], tol, out);
                out.pop();
                chain(&p[i..], tol, out);
            }
            _ => out.extend([a, b]),
        }
    }
    // Замкнутое — двумя цепочками: от первого узла до самого дальнего и обратно.
    let far = (1..pts.len()).max_by(|&i, &j| dist(pts[0], pts[i]).total_cmp(&dist(pts[0], pts[j]))).unwrap_or(1);
    let mut out = Vec::new();
    chain(&pts[..=far], tol, &mut out);
    out.pop();
    let mut back = pts[far..].to_vec();
    back.push(pts[0]);
    chain(&back, tol, &mut out);
    out.pop();
    if out.len() < 3 { pts } else { out }
}

/// Покрытие в точке `u`, `v` (доли плитки), с размытием между точками картинки.
fn bilinear(tex: &image::RgbaImage, u: f32, v: f32) -> [f32; 3] {
    let (tw, th) = (tex.width() as f32, tex.height() as f32);
    let (x, y) = ((u * tw).rem_euclid(tw) - 0.5, (v * th).rem_euclid(th) - 0.5);
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);
    let at = |xx: f32, yy: f32| {
        let q = tex.get_pixel(xx.rem_euclid(tw) as u32 % tex.width(), yy.rem_euclid(th) as u32 % tex.height()).0;
        [q[0] as f32, q[1] as f32, q[2] as f32]
    };
    let (a, b, c, d) = (at(fx, fy), at(fx + 1.0, fy), at(fx, fy + 1.0), at(fx + 1.0, fy + 1.0));
    std::array::from_fn(|k| (a[k] * (1.0 - tx) + b[k] * tx) * (1.0 - ty) + (c[k] * (1.0 - tx) + d[k] * tx) * ty)
}

/// Скат широкого дома не длиннее этого, в клетках (6 м): выше — плоский верх. У
/// ГУМа сплошная вальма давала крест коньков через весь квартал.
pub const SLOPE_MAX: f32 = 4.0;
/// Допуск упрощения контура, в клетках: выступы фасада меньше него не режут свес в пилу.
const OUTLINE_TOL: f32 = 0.35;

/// Картинка крыши по контуру ([`OutlineRoof`]): `ppc` точек на клетку, только в
/// пределах `clip` (левый верхний и правый нижний углы карты, в клетках): дом,
/// уходящий за рамку, иначе давал картинку в 10 000 точек. Куски — по очереди,
/// поверх друг друга; покрытие куска — `tex` по его ключу (нет — кусок пропущен).
/// Возвращает картинку и её левый верхний угол в клетках.
pub fn render_outline<'t>(roof: &OutlineRoof, tex: &dyn Fn(&str) -> Option<&'t image::RgbaImage>, ppc: f32, light: [f32; 2], clip: ([f32; 2], [f32; 2])) -> (image::RgbaImage, [f32; 2]) {
    let (lo, hi) = roof.bounds();
    let (lo, hi) = ([lo[0].max(clip.0[0]), lo[1].max(clip.0[1])], [hi[0].min(clip.1[0]), hi[1].min(clip.1[1])]);
    let origin = [(lo[0] * ppc).floor() / ppc, (lo[1] * ppc).floor() / ppc];
    if hi[0] <= lo[0] || hi[1] <= lo[1] {
        return (image::RgbaImage::new(1, 1), origin);
    }
    let (w, h) = (((hi[0] - origin[0]) * ppc).ceil() as u32 + 1, ((hi[1] - origin[1]) * ppc).ceil() as u32 + 1);
    let mut img = image::RgbaImage::new(w, h);
    for piece in &roof.pieces {
        if let Some(t) = tex(&piece.paint) {
            paint_piece(&mut img, origin, ppc, piece, t, light);
        }
    }
    (img, origin)
}

/// Кусок крыши поверх того, что уже в картинке. Скатный — каждая точка сходит к
/// ближайшей стороне: ряды покрытия вдоль линии этой стороны, свет — по тому,
/// куда она смотрит; где две стороны одинаково близко — гребень. Плоский —
/// покрытие вдоль длинной стороны, по краю парапет. Двор — дыра, к нему крыша
/// тоже скатывается.
fn paint_piece(img: &mut image::RgbaImage, origin: [f32; 2], ppc: f32, piece: &RoofPiece, tex: &image::RgbaImage, light: [f32; 2]) {
    /// Сторона квадрата точек, для которого стороны контура отбираются разом.
    const BLOCK: u32 = 16;
    let rings: Vec<Vec<[f32; 2]>> = piece.rings.iter().map(|r| simplify_ring(r, OUTLINE_TOL)).filter(|r| r.len() >= 3).collect();
    let edges: Vec<([f32; 2], [f32; 2])> = rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).filter(|(a, b)| dist(*a, *b) > 1e-4).collect();
    if edges.is_empty() {
        return;
    }
    let tile = tile_cells([tex.width(), tex.height()]);
    let (w, h) = (img.width(), img.height());
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in rings.iter().flatten() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let px = |v: f32, o: f32, n: u32| (((v - o) * ppc).floor().max(0.0) as u32).min(n);
    let (x0, x1) = (px(lo[0] - 0.1, origin[0], w), (px(hi[0] + 0.1, origin[0], w) + 1).min(w));
    let (y0, y1) = (px(lo[1] - 0.1, origin[1], h), (px(hi[1] + 0.1, origin[1], h) + 1).min(h));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    // Плоская: ряды вдоль самой длинной стороны внешнего кольца.
    let axis = rings
        .first()
        .and_then(|r| (0..r.len()).map(|k| (r[k], r[(k + 1) % r.len()])).max_by(|x, y| dist(x.0, x.1).total_cmp(&dist(y.0, y.1))))
        .map_or([1.0, 0.0], |(a, b)| {
            let l = dist(a, b).max(1e-6);
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        });
    // Внутри ли: пересечения строки с контуром — раз на строку, по чётности.
    let crossings: Vec<Vec<f32>> = (y0..y1)
        .map(|y| {
            let py = origin[1] + (y as f32 + 0.5) / ppc;
            let mut xs: Vec<f32> = edges.iter().filter(|(a, b)| (a[1] > py) != (b[1] > py)).map(|(a, b)| (b[0] - a[0]) * (py - a[1]) / (b[1] - a[1]) + a[0]).collect();
            xs.sort_by(|a, b| a.total_cmp(b));
            xs
        })
        .collect();
    let half = BLOCK as f32 * std::f32::consts::FRAC_1_SQRT_2 / ppc;
    let cap = match piece.form {
        RoofForm::Pitched => SLOPE_MAX,
        _ => f32::MAX,
    };
    for by in (y0..y1).step_by(BLOCK as usize) {
        for bx in (x0..x1).step_by(BLOCK as usize) {
            // Кандидаты квадрата: ближе к его середине, чем вторая по близости
            // сторона плюс поперечник, — дальше ни одна точка квадрата не дотянется.
            let c = [origin[0] + (bx as f32 + BLOCK as f32 / 2.0) / ppc, origin[1] + (by as f32 + BLOCK as f32 / 2.0) / ppc];
            let ds: Vec<f32> = edges.iter().map(|e| segment_distance(c, e.0, e.1)).collect();
            let (mut c1, mut c2) = (f32::MAX, f32::MAX);
            for &d in &ds {
                if d < c1 {
                    c2 = c1;
                    c1 = d;
                } else if d < c2 {
                    c2 = d;
                }
            }
            let reach = c2.min(f32::MAX / 2.0) + 2.0 * half;
            let cand: Vec<&([f32; 2], [f32; 2])> = edges.iter().zip(&ds).filter(|(_, d)| **d <= reach).map(|(e, _)| e).collect();
            for y in by..(by + BLOCK).min(y1) {
                let xs = &crossings[(y - y0) as usize];
                for x in bx..(bx + BLOCK).min(x1) {
                    let p = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
                    // Две ближайшие стороны: расстояние, точка на стороне, сама сторона.
                    let (mut d1, mut d2, mut q1, mut q2, mut e1) = (f32::MAX, f32::MAX, p, p, edges[0]);
                    for &&(a, b) in &cand {
                        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                        let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                        let q = [a[0] + dx * t, a[1] + dy * t];
                        let d = dist(p, q);
                        if d < d1 {
                            (d2, q2) = (d1, q1);
                            (d1, q1, e1) = (d, q, (a, b));
                        } else if d < d2 {
                            (d2, q2) = (d, q);
                        }
                    }
                    let inner = xs.partition_point(|&v| v < p[0]) % 2 == 1;
                    let cov = if inner { (d1 * ppc + 0.5).min(1.0) } else { (0.5 - d1 * ppc).max(0.0) };
                    if cov <= 0.0 {
                        continue;
                    }
                    let flat_here = piece.form == RoofForm::Flat || d1 > cap;
                    let mut c = if flat_here {
                        let (u, v) = ((p[0] * axis[0] + p[1] * axis[1]) / tile[0], (-p[0] * axis[1] + p[1] * axis[0]) / tile[1]);
                        let mut c = bilinear(tex, u, v).map(|k| k * roof_shade([0.0, 1.0], light, true));
                        if piece.form == RoofForm::Flat {
                            if d1 < PARAPET {
                                c = c.map(|k| (k * 1.12).min(255.0));
                            } else if d1 < PARAPET + PARAPET_SHADOW {
                                let k = PARAPET_SHADE + (1.0 - PARAPET_SHADE) * (d1 - PARAPET) / PARAPET_SHADOW;
                                c = c.map(|v| v * k);
                            }
                        } else if d1 - cap < CAP / 2.0 {
                            // Перелом ската к плоскому верху.
                            c = c.map(|k| k * CAP_SHADE);
                        }
                        c
                    } else {
                        // Скат — по линии ближайшей стороны: ряды вдоль неё, смотрит
                        // наружу. К точке на стороне — у вогнутого угла ряды
                        // закручивались кругами.
                        let (a, b) = e1;
                        let len = dist(a, b);
                        let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
                        let mut n = [dir[1], -dir[0]];
                        if (q1[0] - p[0]) * n[0] + (q1[1] - p[1]) * n[1] < 0.0 {
                            n = [-n[0], -n[1]];
                        }
                        let along = (p[0] - a[0]) * dir[0] + (p[1] - a[1]) * dir[1];
                        let across = ((p[0] - a[0]) * n[0] + (p[1] - a[1]) * n[1]).abs();
                        let mut c = bilinear(tex, along / tile[0], -across / tile[1]).map(|k| k * roof_shade(n, light, false));
                        // Гребень — где две стороны одинаково близко (не у самой
                        // вершины: там обе стороны сходятся в одну точку).
                        if d2 - d1 < CAP / 2.0 && d1 > CAP && dist(q1, q2) > CAP {
                            c = bilinear(tex, along / tile[0] * 2.0, across / tile[1]).map(|k| k * CAP_SHADE);
                            if (CAP / 2.0 - (d2 - d1)) * ppc < 1.0 {
                                c = c.map(|k| k * 0.45);
                            }
                        }
                        c
                    };
                    if inner && d1 * ppc < RIM_PX {
                        c = c.map(|k| k * 0.4);
                    }
                    // Поверх того, что уже лежит (основная крыша под частью).
                    let under = img.get_pixel(x, y).0;
                    let ua = under[3] as f32 / 255.0;
                    let oa = cov + ua * (1.0 - cov);
                    let mix = |k: usize| ((c[k] * cov + under[k] as f32 * ua * (1.0 - cov)) / oa.max(1e-6)).round().clamp(0.0, 255.0) as u8;
                    img.put_pixel(x, y, image::Rgba([mix(0), mix(1), mix(2), (oa * 255.0).round() as u8]));
                }
            }
        }
    }
}

/// Размер плитки покрытия в клетках: 256 точек картинки на клетку, как у пола.
pub fn tile_cells(px: [u32; 2]) -> [f32; 2] {
    [(px[0] as f32 / crate::library::PX_PER_CELL).max(0.5), (px[1] as f32 / crate::library::PX_PER_CELL).max(0.5)]
}

/// Тень крыши длиннее тени стены: крыша выше.
pub fn shadow_offset(sh: &map::Shadows) -> [f32; 2] {
    let o = sh.offset();
    [o[0] * 2.5, o[1] * 2.5]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_loses_nodes_on_straight_walls_but_keeps_corners() {
        // Квадрат 10×10 с узлами посреди сторон и мелкой ступенькой фасада.
        let ring = vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0], [10.0, 5.0], [10.1, 5.2], [10.0, 5.4], [10.0, 10.0], [5.0, 10.0], [0.0, 10.0], [0.0, 0.0]];
        let s = simplify_ring(&ring, OUTLINE_TOL);
        assert_eq!(s.len(), 4, "{s:?}");
        // Купол — круг из 36 узлов — остаётся многоугольником.
        let circle: Vec<[f32; 2]> = (0..36).map(|k| [5.0 * (k as f32 * 10f32.to_radians()).cos(), 5.0 * (k as f32 * 10f32.to_radians()).sin()]).collect();
        assert!(simplify_ring(&circle, OUTLINE_TOL).len() >= 6);
    }

    /// Квадратный дом 8×8 со двором 2×2 посередине, ровное серое покрытие.
    fn yard_house(form: RoofForm) -> OutlineRoof {
        let outer = vec![[0.0, 0.0], [8.0, 0.0], [8.0, 8.0], [0.0, 8.0]];
        let yard = vec![[3.0, 3.0], [5.0, 3.0], [5.0, 5.0], [3.0, 5.0]];
        OutlineRoof { pieces: vec![RoofPiece { rings: vec![outer, yard], form, paint: "grey".into() }] }
    }

    #[test]
    fn outline_roof_slopes_to_every_side_and_leaves_the_yard_open() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let (img, origin) = render_outline(&yard_house(RoofForm::Pitched), &tex, 10.0, [0.6, 0.8], ([-5.0, -5.0], [50.0, 50.0]));
        let at = |x: f32, y: f32| img.get_pixel(((x - origin[0]) * 10.0) as u32, ((y - origin[1]) * 10.0) as u32).0;
        assert_eq!(at(4.0, 4.0)[3], 0, "двор закрыт");
        assert_eq!(at(1.5, 4.0)[3], 255, "скат дырявый");
        // Свет сверху-слева (0,6; 0,8 — откуда): скаты на юг и на север — разной яркости.
        assert_ne!(at(4.0, 7.0)[0], at(4.0, 1.0)[0], "скаты одного света");
        // Вальма: по диагонали от угла — гребень темнее ската рядом.
        assert!(at(1.0, 1.0)[0] < at(1.5, 0.6)[0], "гребня нет: {:?} {:?}", at(1.0, 1.0), at(1.5, 0.6));
    }

    #[test]
    fn flat_outline_roof_has_a_parapet_and_stays_in_the_map() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let (img, origin) = render_outline(&yard_house(RoofForm::Flat), &tex, 10.0, [0.6, 0.8], ([-5.0, -5.0], [50.0, 50.0]));
        let at = |x: f32, y: f32| img.get_pixel(((x - origin[0]) * 10.0) as u32, ((y - origin[1]) * 10.0) as u32).0;
        assert!(at(0.15, 4.0)[0] > at(1.5, 4.0)[0], "парапета нет");
        // Дом за краем карты — картинка только до края.
        let (cut, o) = render_outline(&yard_house(RoofForm::Flat), &tex, 10.0, [0.6, 0.8], ([0.0, 0.0], [4.0, 4.0]));
        assert!(o[0] >= 0.0 && cut.width() <= 41 && cut.height() <= 41, "{o:?} {}×{}", cut.width(), cut.height());
    }

    #[test]
    fn flat_cyber_roof_gets_steel_parapet() {
        let load = |rel: &str| image::load_from_memory(crate::sets::SET_FILES.iter().find(|f| f.0 == rel).unwrap().1).unwrap().to_rgba8();
        let (tex, pt) = (load("cyberpunk/roofs/Roof_Flat_Membrane_A.webp"), load("cyberpunk/walls/Roof_Parapet_Steel_A.webp"));
        let roof = Roof { overhang: 0.0, ..Roof::new([0.0, 0.0], [6.0, 4.0], RoofShape::Flat, "set:cyberpunk/roofs/Roof_Flat_Membrane_A.webp".into()) };
        assert!(roof.parapet().is_some());
        let tile = tile_cells([tex.width(), tex.height()]);
        let (img, _) = render(&roof, &tex, tile, 64.0, [0.0, -1.0], Some(&pt), &[], false);
        let (plain, _) = render(&roof, &tex, tile, 64.0, [0.0, -1.0], None, &[], false);
        // По краю — отлив, а не мембрана; посередине — та же кровля.
        assert_ne!(img.get_pixel(150, 8).0, plain.get_pixel(150, 8).0);
        assert_eq!(img.get_pixel(192, 128).0, plain.get_pixel(192, 128).0);
        // Сливы на длинных сторонах, воронки посередине; слив выступает наружу.
        let fx = roof.fixtures();
        assert_eq!(fx.iter().filter(|f| f.0.contains("Scupper")).count(), 2);
        assert!(fx.iter().any(|f| f.0.contains("Drain")));
        let imgs: Vec<image::RgbaImage> = fx.iter().map(|f| load(f.0.trim_start_matches("set:"))).collect();
        let items: Vec<Fixture> = fx.iter().zip(&imgs).map(|(f, i)| (i, f.1, f.2)).collect();
        let (full, origin) = render(&roof, &tex, tile, 64.0, [0.0, -1.0], Some(&pt), &items, false);
        assert!(origin[1] < 0.0 && full.height() > img.height());
        if let Ok(out) = std::env::var("KARTOGRAF_PARAPET_PNG") {
            full.save(out).unwrap();
        }
        // Карта с такой крышей — посмотреть на холсте редактора.
        if let Ok(out) = std::env::var("KARTOGRAF_PARAPET_MAP") {
            let mut m = map::Map::new(12, 9);
            let mut r = roof.clone();
            r.a = [3.0, 2.0];
            r.b = [9.0, 6.0];
            m.roofs.push(r);
            std::fs::write(out, m.to_json()).unwrap();
        }
    }

    #[test]
    fn rendered_roof_is_opaque_inside_and_lit_side_brighter() {
        let tex = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 100, 80, 255]));
        let roof = Roof { overhang: 0.0, ..Roof::new([0.0, 0.0], [4.0, 2.0], RoofShape::Gable, String::new()) };
        // Свет сверху: северный скат светлее южного.
        let (img, origin) = render(&roof, &tex, [1.0, 1.0], 50.0, [0.0, -1.0], None, &[], false);
        assert_eq!(origin, [0.0, 0.0]);
        assert!(img.width() >= 200 && img.height() >= 100);
        let (n, s) = (img.get_pixel(100, 20).0, img.get_pixel(100, 80).0);
        assert_eq!((n[3], s[3]), (255, 255));
        assert!(n[0] > s[0], "{n:?} {s:?}");
        // На коньке — гребень темнее светлого ската.
        assert!(img.get_pixel(100, 50).0[0] < n[0]);
        // Вне крыши прозрачно.
        assert_eq!(img.get_pixel(img.width() - 1, img.height() - 1).0[3], 0);
    }
}
