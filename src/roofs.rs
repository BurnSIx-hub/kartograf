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

/// Кольцо для скатов: уступ стены до полутора метров между двумя стенами одного
/// хода, каждая хотя бы вдвое длиннее уступа, — в точку. У короткой стороны
/// уступа своя грань, и она шла узкой полосой через весь скат (двор Средних
/// рядов, уступ 1,3 м). Край крыши остаётся по [`simplify_ring`]: по этому
/// кольцу стена с пилястрами выглядывала бы из-под свеса.
pub fn iron_jogs(ring: &[[f32; 2]]) -> Vec<[f32; 2]> {
    const JOG: f32 = 1.0;
    let mut out = ring.to_vec();
    loop {
        let n = out.len();
        if n < 5 {
            break;
        }
        let dir = |a: [f32; 2], b: [f32; 2]| {
            let l = dist(a, b).max(1e-6);
            [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
        };
        let jog = (0..n).find(|&i| {
            let (p, a, b, q) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n], out[(i + 2) % n]);
            let (u, w) = (dir(p, a), dir(b, q));
            let wall = (2.0 * dist(a, b)).max(1.0);
            dist(a, b) < JOG && dist(p, a) >= wall && dist(b, q) >= wall && u[0] * w[0] + u[1] * w[1] > 0.97
        });
        let Some(i) = jog else { break };
        let (a, b) = (out[i], out[(i + 1) % n]);
        out[i] = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        out.remove((i + 1) % n);
    }
    out
}

/// Покрытие кровли без прозрачности — для [`render_outline`], что читает только
/// цвет. У черепицы и шифера FA внизу плитки полоса в 60 точек без единой
/// непрозрачной: край ряда в Dungeondraft ложится на следующий. Повтором плитки
/// она давала белую черту поперёк ската и вдоль ендовы. Пустые края — прочь,
/// полупрозрачное (зубчатый край ряда) — поверх тёмной щели.
pub fn solid_cover(img: image::RgbaImage) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let row = |y: u32| (0..w).map(|x| img.get_pixel(x, y).0[3] as u32).sum::<u32>() >= 128 * w;
    let col = |x: u32| (0..h).map(|y| img.get_pixel(x, y).0[3] as u32).sum::<u32>() >= 128 * h;
    let (Some(y0), Some(y1)) = ((0..h).find(|&y| row(y)), (0..h).rev().find(|&y| row(y))) else { return img };
    let (Some(x0), Some(x1)) = ((0..w).find(|&x| col(x)), (0..w).rev().find(|&x| col(x))) else { return img };
    let mut out = image::imageops::crop_imm(&img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image();
    if out.pixels().all(|p| p.0[3] == 255) {
        return out;
    }
    // Щель — срединный цвет непрозрачного втрое темнее.
    let mut seen: Vec<[u8; 3]> = out.pixels().filter(|p| p.0[3] == 255).map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    let gap: [f32; 3] = std::array::from_fn(|k| {
        seen.sort_unstable_by_key(|c| c[k]);
        seen.get(seen.len() / 2).map_or(0.0, |c| c[k] as f32 / 3.0)
    });
    for p in out.pixels_mut() {
        let a = p.0[3] as f32 / 255.0;
        for k in 0..3 {
            p.0[k] = (p.0[k] as f32 * a + gap[k] * (1.0 - a)).round() as u8;
        }
        p.0[3] = 255;
    }
    out
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
pub const OUTLINE_TOL: f32 = 0.35;

/// Картинка крыши по контуру ([`OutlineRoof`]): `ppc` точек на клетку, только в
/// пределах `clip` (левый верхний и правый нижний углы карты, в клетках): дом,
/// уходящий за рамку, иначе давал картинку в 10 000 точек. Куски — по очереди,
/// поверх друг друга; покрытие куска — `tex` по его ключу (нет — кусок пропущен).
/// `near` — что вокруг бросает на крышу тень ([`Casters::near`]): высшие соседи и кроны.
/// Возвращает картинку и её левый верхний угол в клетках.
pub fn render_outline<'t>(roof: &OutlineRoof, tex: &dyn Fn(&str) -> Option<&'t image::RgbaImage>, ppc: f32, light: [f32; 2], clip: ([f32; 2], [f32; 2]), near: &Near) -> (image::RgbaImage, [f32; 2]) {
    let (lo, hi) = roof.bounds();
    let (lo, hi) = ([lo[0].max(clip.0[0]), lo[1].max(clip.0[1])], [hi[0].min(clip.1[0]), hi[1].min(clip.1[1])]);
    let origin = [(lo[0] * ppc).floor() / ppc, (lo[1] * ppc).floor() / ppc];
    if hi[0] <= lo[0] || hi[1] <= lo[1] {
        return (image::RgbaImage::new(1, 1), origin);
    }
    let (w, h) = (((hi[0] - origin[0]) * ppc).ceil() as u32 + 1, ((hi[1] - origin[1]) * ppc).ceil() as u32 + 1);
    let mut img = image::RgbaImage::new(w, h);
    // Кольца без лишних узлов — раз на кусок: по ним и стыки, и скаты.
    let rings: Vec<Vec<Vec<[f32; 2]>>> = roof.pieces.iter().map(|k| k.rings.iter().map(|r| simplify_ring(r, OUTLINE_TOL)).filter(|r| r.len() >= 3).collect()).collect();
    // Скаты — от колец без мелких уступов, край — по упрощённым.
    let slopes: Vec<Vec<Vec<[f32; 2]>>> = rings.iter().map(|rs| rs.iter().map(|r| iron_jogs(r)).collect()).collect();
    // Высота видимого куска в каждой точке и чей он — для теней.
    let mut relief = Relief { top: vec![f32::MIN; (w * h) as usize], owner: vec![u32::MAX; (w * h) as usize] };
    for (i, piece) in roof.pieces.iter().enumerate() {
        if let Some(t) = tex(&piece.paint) {
            let open = open_edges(&rings, &roof.pieces, i);
            let slope = (&slopes[i][..], open_edges(&slopes, &roof.pieces, i));
            paint_piece(&mut img, origin, ppc, piece, &rings[i], &open, (slope.0, &slope.1), t, light, (&mut relief, i as u32));
        }
    }
    let mut dark = cast_shadows(&relief, w as i32, h as i32, ppc, light);
    // Дом без высоты в OSM — как у тени на земле: верх ската над 6 м.
    let base = if roof.pieces.first().is_some_and(|p| p.height <= 0.0) { DEFAULT_TOP } else { 0.0 };
    cast_near(&mut dark, &relief, (w as usize, h as usize), origin, ppc, light, near, base);
    for (k, p) in img.pixels_mut().enumerate() {
        let f = dark[k];
        if f > 0.0 {
            for c in 0..3 {
                p.0[c] = (p.0[c] as f32 * (1.0 - f)).round() as u8;
            }
        }
    }
    (img, origin)
}

/// Дом без высоты в OSM — 6 м в клетках, как у его крыши и тени на земле.
pub const DEFAULT_TOP: f32 = 6.0 / crate::procgen::osm::CELL_M;
/// Сетка теней соседей и крон на крыше — точек на клетку: тени мягкие, а
/// картинка крыши в выгрузке — 140 точек на клетку.
const NEAR_RES: f32 = 8.0;
/// Тень соседа длиннее не считается, в клетках, как у тени на земле.
const NEAR_REACH: f32 = 40.0;

/// Что бросает тень на крышу снаружи: высшие соседи (кольца и верх в клетках) и
/// кроны деревьев.
#[derive(Default, Debug, Clone, PartialEq)]
pub struct Near {
    pub houses: Vec<(Vec<Vec<[f32; 2]>>, f32)>,
    pub crowns: Vec<Crown>,
}

/// Крона: середина, поперечник в клетках, голая ли.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crown {
    pub pos: [f32; 2],
    pub d: f32,
    pub bare: bool,
}

/// Верх куска для тени на соседей: плоская — свой верх, скатная — середина
/// между свесом и коньком (конёк узкий); без высоты — [`DEFAULT_TOP`].
fn caster_top(p: &RoofPiece) -> f32 {
    if p.height <= 0.0 {
        return DEFAULT_TOP;
    }
    match p.form {
        RoofForm::Flat => p.height,
        _ => (p.height + p.eaves.min(p.height)) / 2.0,
    }
}

/// Кусок крыши, бросающий тень: чей, рамка, верх, кольца.
type Caster<'a> = (usize, [f32; 2], [f32; 2], f32, &'a [Vec<[f32; 2]>]);

/// Всё, что бросает тень на крыши, с рамками — раз на кадр, а не на каждую крышу.
pub struct Casters<'a> {
    houses: Vec<Caster<'a>>,
    crowns: Vec<Crown>,
}

impl<'a> Casters<'a> {
    pub fn new(roofs: &'a [OutlineRoof], props: &[map::Prop]) -> Self {
        let mut houses = Vec::new();
        for (j, r) in roofs.iter().enumerate() {
            for p in &r.pieces {
                let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
                for q in p.rings.iter().flatten() {
                    a = [a[0].min(q[0]), a[1].min(q[1])];
                    b = [b[0].max(q[0]), b[1].max(q[1])];
                }
                if a[0] <= b[0] {
                    houses.push((j, a, b, caster_top(p), &p.rings[..]));
                }
            }
        }
        let crowns = props
            .iter()
            .filter(|p| p.is_canopy() && p.casts_shadow())
            .filter_map(|p| {
                let d = p.size[0].max(p.size[1]) * p.scale;
                (d > 0.0).then(|| Crown { pos: p.pos, d, bare: map::is_bare_tree(&p.asset) })
            })
            .collect();
        Self { houses, crowns }
    }

    /// Что из этого дотягивается тенью до рамки крыши `k` ([`OutlineRoof::bounds`]).
    pub fn near(&self, k: usize, (lo, hi): ([f32; 2], [f32; 2])) -> Near {
        let reaches = |a: [f32; 2], b: [f32; 2], reach: f32| b[0] >= lo[0] - reach && a[0] <= hi[0] + reach && b[1] >= lo[1] - reach && a[1] <= hi[1] + reach;
        Near {
            houses: self.houses.iter().filter(|h| h.0 != k && reaches(h.1, h.2, (h.3 / SUN_RISE).min(NEAR_REACH))).map(|h| (h.4.to_vec(), h.3)).collect(),
            crowns: self.crowns.iter().copied().filter(|c| reaches(c.pos, c.pos, c.d * TREE_HEIGHT / SUN_RISE + c.d)).collect(),
        }
    }
}

/// Тени соседей и крон на крыше — в долю затемнения `dark` точек картинки (`w ×
/// h`, левый верхний угол `origin`), по наибольшей с тенями ярусов. Соседи — на
/// сетке [`NEAR_RES`] с полем к солнцу: проход [`sun_rays`] по их верху, тень —
/// где луч выше ската. Крона — шар на высоте поперечника: на скате высоты `z`
/// её тень сдвинута от солнца на `(высота кроны − z)`; ниже ската кроны тени нет.
/// `base` — подъём крыши без высоты в OSM.
#[allow(clippy::too_many_arguments)]
fn cast_near(dark: &mut [f32], r: &Relief, (w, h): (usize, usize), origin: [f32; 2], ppc: f32, light: [f32; 2], near: &Near, base: f32) {
    if near.houses.is_empty() && near.crowns.is_empty() {
        return;
    }
    let size = [w as f32 / ppc, h as f32 / ppc];
    let tallest = near.houses.iter().map(|x| x.1).fold(0.0f32, f32::max);
    let reach = (tallest / SUN_RISE).min(NEAR_REACH).ceil();
    // Поле — только со стороны солнца: оттуда приходит тень.
    let pad = |s: f32| if s < -1e-4 { reach } else { 0.0 };
    let g0 = [origin[0] - pad(light[0]), origin[1] - pad(light[1])];
    let g1 = [origin[0] + size[0] + pad(-light[0]), origin[1] + size[1] + pad(-light[1])];
    let (gw, gh) = (((g1[0] - g0[0]) * NEAR_RES).ceil() as usize + 1, ((g1[1] - g0[1]) * NEAR_RES).ceil() as usize + 1);
    let at = |i: usize, j: usize| [g0[0] + (i as f32 + 0.5) / NEAR_RES, g0[1] + (j as f32 + 0.5) / NEAR_RES];
    // Верх соседей на сетке.
    let mut top = vec![0.0f32; gw * gh];
    for (rings, z) in &near.houses {
        let (mut a, mut b) = ([f32::MAX; 2], [f32::MIN; 2]);
        for q in rings.iter().flatten() {
            a = [a[0].min(q[0]), a[1].min(q[1])];
            b = [b[0].max(q[0]), b[1].max(q[1])];
        }
        let span = |lo: f32, hi: f32, o: f32, n: usize| ((((lo - o) * NEAR_RES).floor().max(0.0)) as usize, (((hi - o) * NEAR_RES).ceil().max(0.0) as usize).min(n));
        let ((i0, i1), (j0, j1)) = (span(a[0], b[0], g0[0], gw), span(a[1], b[1], g0[1], gh));
        for j in j0..j1 {
            for i in i0..i1 {
                if top[j * gw + i] < *z && inside_rings(rings, at(i, j)) {
                    top[j * gw + i] = *z;
                }
            }
        }
    }
    let ray = if near.houses.is_empty() { Vec::new() } else { sun_rays(&top, gw, gh, light, SUN_RISE / NEAR_RES) };
    let dir = [-light[0], -light[1]];
    let crowns: Vec<(Crown, f32, f32, f32)> = near
        .crowns
        .iter()
        .map(|c| {
            let rad = c.d * 0.42;
            (*c, c.d * TREE_HEIGHT, rad, c.d * 0.16)
        })
        .collect();
    // Сетка — в точку картинки: билинейно для луча, ближайшая для крон.
    for y in 0..h {
        for x in 0..w {
            let k = y * w + x;
            if r.top[k] <= f32::MIN {
                continue;
            }
            let z = r.top[k] + base;
            let p = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
            let mut f = dark[k];
            if !ray.is_empty() {
                let (u, v) = ((p[0] - g0[0]) * NEAR_RES - 0.5, (p[1] - g0[1]) * NEAR_RES - 0.5);
                let (iu, iv) = (u.floor().clamp(0.0, (gw - 1) as f32) as usize, v.floor().clamp(0.0, (gh - 1) as f32) as usize);
                let (iu1, iv1) = ((iu + 1).min(gw - 1), (iv + 1).min(gh - 1));
                let (tu, tv) = ((u - iu as f32).clamp(0.0, 1.0), (v - iv as f32).clamp(0.0, 1.0));
                let g = |i: usize, j: usize| ray[j * gw + i];
                let up = (g(iu, iv) * (1.0 - tu) + g(iu1, iv) * tu) * (1.0 - tv) + (g(iu, iv1) * (1.0 - tu) + g(iu1, iv1) * tu) * tv;
                if up > z {
                    f = f.max((1.0 - CAST_SHADE) * ((up - z) / CAST_SOFT).min(1.0));
                }
            }
            for &(c, hc, rad, soft) in &crowns {
                if hc <= z {
                    continue;
                }
                let s = (hc - z) / SUN_RISE;
                let q = [c.pos[0] + dir[0] * s, c.pos[1] + dir[1] * s];
                let e = dist(p, q);
                if e < rad + soft {
                    let shade = if c.bare { TREE_BARE } else { TREE_SHADE };
                    f = f.max(shade * ((rad + soft - e) / (2.0 * soft)).clamp(0.0, 1.0));
                }
            }
            dark[k] = f;
        }
    }
}

/// Высота солнца: клеток высоты на клетку тени (45°).
pub const SUN_RISE: f32 = 1.0;
/// Свет в полной тени высшего куска и полутень по высоте, в клетках.
const CAST_SHADE: f32 = 0.55;
const CAST_SOFT: f32 = 0.12;

/// Крыша по высотам: верх видимого куска в точке картинки (клетки) и его номер.
struct Relief {
    top: Vec<f32>,
    owner: Vec<u32>,
}

/// Тени высших кусков на низшие той же крыши: ступени Мавзолея, ярусы башни,
/// барабаны собора. Без них ступени сливались в плоский узор. Проход от солнца:
/// в каждой точке — высота луча, пришедшего от соседней точки к солнцу, минус
/// спуск на шаг ([`SUN_RISE`]); луч выше крыши — тень. На своём же куске тени нет:
/// скат уже темнит свет. На землю тень не ложится — картинка только крыша.
/// Возвращает долю затемнения каждой точки картинки `w × h`.
fn cast_shadows(r: &Relief, w: i32, h: i32, ppc: f32, light: [f32; 2]) -> Vec<f32> {
    let mut dark = vec![0.0f32; (w.max(0) * h.max(0)) as usize];
    let m = light[0].abs().max(light[1].abs());
    if m < 1e-4 || w < 2 || h < 2 {
        return dark;
    }
    // Шаг к солнцу: по одной оси ровно точка, по другой — доля.
    let step = [light[0] / m, light[1] / m];
    let drop = step[0].hypot(step[1]) / ppc * SUN_RISE;
    let xs: Vec<i32> = if step[0] < 0.0 { (0..w).collect() } else { (0..w).rev().collect() };
    let ys: Vec<i32> = if step[1] < 0.0 { (0..h).collect() } else { (0..h).rev().collect() };
    let mut ray = vec![f32::MIN; (w * h) as usize];
    let mut from = vec![u32::MAX; (w * h) as usize];
    for &y in &ys {
        for &x in &xs {
            let k = (y * w + x) as usize;
            // Точка к солнцу — между двумя уже пройденными соседями.
            let (ux, uy) = (x as f32 + step[0], y as f32 + step[1]);
            let (fx, fy) = (ux.floor(), uy.floor());
            let (tx, ty) = (ux - fx, uy - fy);
            let at = |xx: i32, yy: i32| if xx < 0 || yy < 0 || xx >= w || yy >= h { (f32::MIN, u32::MAX) } else { (ray[(yy * w + xx) as usize], from[(yy * w + xx) as usize]) };
            let ((a, ao), (b, bo), t) = if tx.abs() < 1e-6 { (at(fx as i32, fy as i32), at(fx as i32, fy as i32 + 1), ty) } else { (at(fx as i32, fy as i32), at(fx as i32 + 1, fy as i32), tx) };
            let (up, up_from) = match (a > f32::MIN, b > f32::MIN) {
                (true, true) => (a * (1.0 - t) + b * t, if t < 0.5 { ao } else { bo }),
                (true, false) => (a, ao),
                (false, true) => (b, bo),
                (false, false) => (f32::MIN, u32::MAX),
            };
            let cast = if up > f32::MIN { up - drop } else { f32::MIN };
            let here = r.top[k];
            if here > f32::MIN && cast > here && up_from != r.owner[k] {
                dark[k] = (1.0 - CAST_SHADE) * ((cast - here) / CAST_SOFT).min(1.0);
            }
            if cast > here {
                (ray[k], from[k]) = (cast, up_from);
            } else {
                (ray[k], from[k]) = (here, r.owner[k]);
            }
        }
    }
    dark
}

/// Высота солнечного луча над каждой точкой сетки `w × h`: большее из верха в
/// точке и луча от соседа к солнцу минус спуск на шаг. `drop` — спуск луча на
/// точку пути; за краем сетки — земля. Луч выше верха точки — точка в тени,
/// и тем глубже, чем выше луч. Тот же проход, что у теней ярусов
/// ([`cast_shadows`]), только без хозяев кусков: для теней на земле.
pub fn sun_rays(top: &[f32], w: usize, h: usize, light: [f32; 2], drop: f32) -> Vec<f32> {
    let mut ray = top.to_vec();
    let m = light[0].abs().max(light[1].abs());
    if m < 1e-4 || w < 2 || h < 2 || top.len() != w * h {
        return ray;
    }
    let step = [light[0] / m, light[1] / m];
    let fall = step[0].hypot(step[1]) * drop;
    let (wi, hi) = (w as i32, h as i32);
    let xs: Vec<i32> = if step[0] < 0.0 { (0..wi).collect() } else { (0..wi).rev().collect() };
    let ys: Vec<i32> = if step[1] < 0.0 { (0..hi).collect() } else { (0..hi).rev().collect() };
    for &y in &ys {
        for &x in &xs {
            let k = (y * wi + x) as usize;
            let (ux, uy) = (x as f32 + step[0], y as f32 + step[1]);
            let (fx, fy) = (ux.floor(), uy.floor());
            let (tx, ty) = (ux - fx, uy - fy);
            let at = |xx: i32, yy: i32| if xx < 0 || yy < 0 || xx >= wi || yy >= hi { 0.0 } else { ray[(yy * wi + xx) as usize] };
            let up = if tx.abs() < 1e-6 { at(fx as i32, fy as i32) * (1.0 - ty) + at(fx as i32, fy as i32 + 1) * ty } else { at(fx as i32, fy as i32) * (1.0 - tx) + at(fx as i32 + 1, fy as i32) * tx };
            ray[k] = ray[k].max(up - fall);
        }
    }
    ray
}

/// Тень кроны: доля чёрного (листва пропускает свет); у голого дерева — ветки.
const TREE_SHADE: f32 = 0.32;
const TREE_BARE: f32 = 0.12;
/// Середина кроны над землёй — в поперечниках кроны: дерево в 10 м с кроной в 6.
const TREE_HEIGHT: f32 = 1.0;

/// Тени крон на земле (маска карты, темнота 0..255): тень кроны — круг чуть
/// меньше кроны, сдвинутый от солнца на высоту её середины ([`TREE_HEIGHT`],
/// солнце — [`SUN_RISE`], как у домов), и от дерева к ней — тень ствола.
/// Под крышами (своими и OSM) тени нет: пол там под крышей, а дерево у стены
/// клало тень в комнату. Где уже лежит нарисованная тень (дома, кисть), крона
/// темнит лишь до своей силы, а не поверх: тень в тени не гуще.
pub fn tree_shade(m: &map::Map) -> Option<crate::mask::Mask> {
    let sh = &m.shadows;
    if !sh.on {
        return None;
    }
    let res = crate::mask::RES as f32;
    let (w, h) = (m.rooms.w, m.rooms.h);
    let light = sh.light();
    let dir = [-light[0], -light[1]];
    let mut acc = vec![0f32; w * h];
    let mut any = false;
    // Кисть по точкам маски в рамке [lo, hi] клеток: сила в точке — max.
    let mut paint = |lo: [f32; 2], hi: [f32; 2], f: &dyn Fn([f32; 2]) -> f32| {
        let span = |a: f32, b: f32, n: usize| (((a * res - 0.5).floor().max(0.0)) as usize, ((b * res + 0.5).ceil().max(0.0) as usize).min(n));
        let ((i0, i1), (j0, j1)) = (span(lo[0], hi[0], w), span(lo[1], hi[1], h));
        for j in j0..j1 {
            for i in i0..i1 {
                let v = f([(i as f32 + 0.5) / res, (j as f32 + 0.5) / res]);
                if v > acc[j * w + i] {
                    acc[j * w + i] = v;
                    any = true;
                }
            }
        }
    };
    for p in m.props.iter().filter(|p| p.is_canopy() && p.casts_shadow()) {
        let d = p.size[0].max(p.size[1]) * p.scale;
        if d <= 0.0 {
            continue;
        }
        let dark = if map::is_bare_tree(&p.asset) { TREE_BARE } else { TREE_SHADE };
        let reach = d * TREE_HEIGHT / SUN_RISE;
        let c = [p.pos[0] + dir[0] * reach, p.pos[1] + dir[1] * reach];
        let (r, soft) = (d * 0.42, d * 0.16);
        let o = r + soft;
        paint([c[0] - o, c[1] - o], [c[0] + o, c[1] + o], &|q| dark * ((o - dist(q, c)) / (2.0 * soft)).clamp(0.0, 1.0));
        // Ствол: полоска от дерева до тени кроны, к кроне тает в неё.
        let half = (d * 0.04).max(0.08);
        let (a, b) = (p.pos, [c[0] - dir[0] * r * 0.5, c[1] - dir[1] * r * 0.5]);
        let (lo, hi) = ([a[0].min(b[0]) - half - 0.3, a[1].min(b[1]) - half - 0.3], [a[0].max(b[0]) + half + 0.3, a[1].max(b[1]) + half + 0.3]);
        paint(lo, hi, &|q| {
            let (ab, aq) = ([b[0] - a[0], b[1] - a[1]], [q[0] - a[0], q[1] - a[1]]);
            let t = ((ab[0] * aq[0] + ab[1] * aq[1]) / (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6)).clamp(0.0, 1.0);
            let e = dist(q, [a[0] + ab[0] * t, a[1] + ab[1] * t]);
            dark * 1.2 * ((half + 0.06 - e) / 0.12).clamp(0.0, 1.0)
        });
    }
    if !any {
        return None;
    }
    // Под крышами — ни тени: прямоугольные крыши и крыши OSM по контуру дома.
    let mut roofed = vec![false; w * h];
    let mut cover = |lo: [f32; 2], hi: [f32; 2], inside: &dyn Fn([f32; 2]) -> bool| {
        let span = |a: f32, b: f32, n: usize| ((a * res).floor().max(0.0) as usize, ((b * res).ceil().max(0.0) as usize).min(n));
        let ((i0, i1), (j0, j1)) = (span(lo[0], hi[0], w), span(lo[1], hi[1], h));
        for j in j0..j1 {
            for i in i0..i1 {
                if acc[j * w + i] > 0.0 && inside([(i as f32 + 0.5) / res, (j as f32 + 0.5) / res]) {
                    roofed[j * w + i] = true;
                }
            }
        }
    };
    for r in &m.roofs {
        let (lo, hi) = ([r.a[0].min(r.b[0]), r.a[1].min(r.b[1])], [r.a[0].max(r.b[0]), r.a[1].max(r.b[1])]);
        cover(lo, hi, &|q| r.contains(q));
    }
    for r in &m.outline_roofs {
        let Some(base) = r.pieces.first() else { continue };
        let (lo, hi) = r.bounds();
        cover(lo, hi, &|q| inside_rings(&base.rings, q));
    }
    let data = acc
        .iter()
        .zip(&roofed)
        .enumerate()
        .map(|(k, (&t, &under))| {
            if under || t <= 0.0 {
                return 0;
            }
            let s = m.shade.as_ref().map_or(0.0, |s| s.data.get(k).copied().unwrap_or(0) as f32 / 255.0);
            let t = t.min(0.95);
            // Итог как max(тень, крона): поверх нарисованной — лишь недостающее.
            let add = if t <= s { 0.0 } else { (t - s) / (1.0 - s) };
            (add * 255.0).round() as u8
        })
        .collect();
    Some(crate::mask::Mask { w, h, data })
}

/// Стороны колец по порядку, без нулевых.
fn edges_of(rings: &[Vec<[f32; 2]>]) -> Vec<([f32; 2], [f32; 2])> {
    rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).filter(|(a, b)| dist(*a, *b) > 1e-4).collect()
}

/// Контур крыши для Foundry: кольца всего дома (первого куска), в клетках, и
/// двор ли кольцо — лежит ли оно внутри остальных по чётности. По нему «Wild
/// Canvas» видит крышу с улицы целиком и не прячет значок двери в наружной стене:
/// прямоугольник тайла у повёрнутого дома захватывает и улицу, и чужие дворы.
pub fn outline(roof: &OutlineRoof) -> Vec<(Vec<[f32; 2]>, bool)> {
    let Some(base) = roof.pieces.first() else { return Vec::new() };
    let rings: Vec<&Vec<[f32; 2]>> = base.rings.iter().filter(|r| r.len() >= 3).collect();
    rings
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let others: Vec<Vec<[f32; 2]>> = rings.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, o)| (*o).clone()).collect();
            ((*r).clone(), inside_rings(&others, r[0]))
        })
        .collect()
}

/// Внутри ли колец точка (по чётности: двор — снаружи).
fn inside_rings(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    let mut c = false;
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
                c = !c;
            }
        }
    }
    c
}

/// Стороны куска `i` (как [`edges_of`]): край — `true`; `false` — стык, за ним
/// другой кусок с верхом не ниже низа нашего ската ([`RoofPiece::eaves`]). Грани
/// вальмы в OSM — отдельные части, и тёмная кромка по каждой резала крышу Сената
/// на полосы.
fn open_edges(rings: &[Vec<Vec<[f32; 2]>>], pieces: &[RoofPiece], i: usize) -> Vec<bool> {
    /// Насколько отступить от середины стороны наружу, в клетках.
    const STEP: f32 = 0.2;
    /// Соседний кусок ниже на столько (клетки, ~0,5 м) — уже уступ.
    const SAME: f32 = 0.3;
    let boxes: Vec<([f32; 2], [f32; 2])> = rings
        .iter()
        .map(|rs| {
            rs.iter().flatten().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]))
        })
        .collect();
    let h = pieces[i].eaves;
    edges_of(&rings[i])
        .iter()
        .map(|&(a, b)| {
            let l = dist(a, b);
            let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let n = [(b[1] - a[1]) / l * STEP, -(b[0] - a[0]) / l * STEP];
            let p = [m[0] + n[0], m[1] + n[1]];
            let out = if inside_rings(&rings[i], p) { [m[0] - n[0], m[1] - n[1]] } else { p };
            !rings.iter().enumerate().any(|(j, r)| {
                let (lo, hi) = boxes[j];
                j != i && pieces[j].height >= h - SAME && out[0] >= lo[0] && out[1] >= lo[1] && out[0] <= hi[0] && out[1] <= hi[1] && inside_rings(r, out)
            })
        })
        .collect()
}

/// Площадь кольца, клетки².
fn ring_area(r: &[[f32; 2]]) -> f32 {
    (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum::<f32>().abs() / 2.0
}

/// Конёк двускатной: точка на нём, направление и полуширина ската — по
/// описанному прямоугольнику наименьшей площади (одна его сторона лежит на
/// стороне контура). Конёк вдоль длинной стороны, `across` — поперёк. Контур,
/// далёкий от прямоугольника (Г-образный дом), — `None`: такой — вальмой.
fn gable_ridge(rings: &[Vec<[f32; 2]>], across: bool) -> Option<([f32; 2], [f32; 2], f32)> {
    let r = rings.first()?;
    let mut best: Option<(f32, [f32; 2], [f32; 2], [f32; 2])> = None;
    for k in 0..r.len() {
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        let l = dist(a, b);
        if l < 1e-4 {
            continue;
        }
        let ax = [(b[0] - a[0]) / l, (b[1] - a[1]) / l];
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in r {
            let (s, t) = (p[0] * ax[0] + p[1] * ax[1], -p[0] * ax[1] + p[1] * ax[0]);
            lo = [lo[0].min(s), lo[1].min(t)];
            hi = [hi[0].max(s), hi[1].max(t)];
        }
        let area = (hi[0] - lo[0]) * (hi[1] - lo[1]);
        if best.as_ref().is_none_or(|q| area < q.0) {
            best = Some((area, ax, lo, hi));
        }
    }
    let (area, ax, lo, hi) = best?;
    if area <= 0.0 || ring_area(r) < 0.8 * area {
        return None;
    }
    let nn = [-ax[1], ax[0]];
    let mid = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let o = [ax[0] * mid[0] + nn[0] * mid[1], ax[1] * mid[0] + nn[1] * mid[1]];
    let (len, wid) = (hi[0] - lo[0], hi[1] - lo[1]);
    // Вдоль оси `ax` — `len`, поперёк — `wid`.
    let ridge_on_ax = (len >= wid) != across;
    Some(if ridge_on_ax { (o, ax, wid / 2.0) } else { (o, nn, len / 2.0) })
}

/// Кусок крыши поверх того, что уже в картинке. `rings` — его кольца без лишних
/// узлов, `open` — какие стороны край ([`open_edges`]). Скатный — каждая точка
/// сходит к ближайшему краю: ряды покрытия вдоль линии этой стороны, свет — по
/// тому, куда она смотрит; где два края одинаково близко — гребень. Односкатный
/// — одной плоскостью, двускатный и свод — от конька. Плоский — покрытие вдоль
/// длинной стороны, по краю парапет. Двор — дыра, к нему крыша тоже скатывается.
/// Высоту своей поверхности кусок пишет в `relief` под номером `me`.
#[allow(clippy::too_many_arguments)]
fn paint_piece(img: &mut image::RgbaImage, origin: [f32; 2], ppc: f32, piece: &RoofPiece, rings: &[Vec<[f32; 2]>], open: &[bool], (slope_rings, slope_open): (&[Vec<[f32; 2]>], &[bool]), tex: &image::RgbaImage, light: [f32; 2], (relief, me): (&mut Relief, u32)) {
    /// Сторона квадрата точек, для которого стороны контура отбираются разом.
    const BLOCK: u32 = 16;
    let edges = edges_of(rings);
    if edges.is_empty() {
        return;
    }
    // Свес — только по краю: к стыку с соседним куском ската нет. Кусок весь в
    // стыках (посреди такого же) — тогда ко всем сторонам.
    let slope_edges = edges_of(slope_rings);
    let eaves: Vec<([f32; 2], [f32; 2])> = match slope_edges.iter().zip(slope_open).filter(|(_, o)| **o).map(|(e, _)| *e).collect::<Vec<_>>() {
        e if e.is_empty() => slope_edges.clone(),
        e => e,
    };
    // Вогнутые углы между свесами: соседний свес у начала и у конца стороны. За
    // таким углом точка ближе всего к самой вершине, и скаты шли дугой вокруг
    // неё; у настоящей крыши там ендова — прямая по биссектрисе угла.
    let turn: Vec<[Option<usize>; 2]> = eaves
        .iter()
        .map(|&(a, b)| {
            let reflex = |prev: [f32; 2], v: [f32; 2], next: [f32; 2]| {
                let (u, w) = ([prev[0] - v[0], prev[1] - v[1]], [next[0] - v[0], next[1] - v[1]]);
                let (lu, lw) = (u[0].hypot(u[1]), w[0].hypot(w[1]));
                let s = [u[0] / lu + w[0] / lw, u[1] / lu + w[1] / lw];
                let ls = s[0].hypot(s[1]);
                // Между сторонами внутри дома — угол выпуклый; снаружи — вогнутый.
                ls > 0.05 && !inside_rings(slope_rings, [v[0] + s[0] / ls * 0.05, v[1] + s[1] / ls * 0.05])
            };
            let prev = eaves.iter().position(|e| e.1 == a).filter(|&j| reflex(eaves[j].0, a, b));
            let next = eaves.iter().position(|e| e.0 == b).filter(|&j| reflex(a, b, eaves[j].1));
            [prev, next]
        })
        .collect();
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
    // Двускатная без прямоугольника и односкатная без стороны — вальмой.
    let ridge = matches!(piece.form, RoofForm::Gabled | RoofForm::Round).then(|| gable_ridge(rings, piece.across)).flatten();
    let shed = piece.down.filter(|d| d[0].hypot(d[1]) > 1e-3).map(|d| {
        let l = d[0].hypot(d[1]);
        [d[0] / l, d[1] / l]
    });
    let form = match piece.form {
        RoofForm::Gabled | RoofForm::Round if ridge.is_none() => RoofForm::Pitched,
        RoofForm::Shed if shed.is_none() => RoofForm::Pitched,
        f => f,
    };
    // Купол: середина и радиус по узлам внешнего кольца.
    let dome = rings.first().filter(|r| !r.is_empty()).map(|r| {
        let c = r.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0] / r.len() as f32, s[1] + p[1] / r.len() as f32]);
        (c, r.iter().map(|p| dist(*p, c)).fold(0.0f32, f32::max).max(1e-3))
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
    let cap = match form {
        RoofForm::Pitched => SLOPE_MAX,
        _ => f32::MAX,
    };
    // Высота поверхности: от низа ската `ze` до верха `zt`. Вальме и шатру —
    // по расстоянию до края; путь до макушки — радиус вписанного круга (2S/P).
    let (zt, ze) = (piece.height, piece.eaves.min(piece.height));
    let run = rings.first().map_or(1.0, |r| {
        let per: f32 = (0..r.len()).map(|k| dist(r[k], r[(k + 1) % r.len()])).sum();
        (2.0 * ring_area(r) / per.max(1e-4)).max(0.1)
    });
    let run = if form == RoofForm::Pitched { run.min(SLOPE_MAX) } else { run };
    let span = shed.and_then(|dn| rings.first().map(|r| r.iter().map(|p| p[0] * dn[0] + p[1] * dn[1]).fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)))));
    // Две ближайшие к `c` стороны из `set`: расстояния.
    let two = |set: &[([f32; 2], [f32; 2])], c: [f32; 2]| {
        let (mut c1, mut c2) = (f32::MAX, f32::MAX);
        for e in set {
            let d = segment_distance(c, e.0, e.1);
            if d < c1 {
                c2 = c1;
                c1 = d;
            } else if d < c2 {
                c2 = d;
            }
        }
        (c1, c2)
    };
    for by in (y0..y1).step_by(BLOCK as usize) {
        for bx in (x0..x1).step_by(BLOCK as usize) {
            // Кандидаты квадрата: ближе к его середине, чем вторая по близости
            // сторона плюс поперечник, — дальше ни одна точка квадрата не дотянется.
            let c = [origin[0] + (bx as f32 + BLOCK as f32 / 2.0) / ppc, origin[1] + (by as f32 + BLOCK as f32 / 2.0) / ppc];
            let (a1, _) = two(&edges, c);
            let (_, e2) = two(&eaves, c);
            let near: Vec<(&([f32; 2], [f32; 2]), bool)> = edges.iter().zip(open.iter().copied()).filter(|(e, _)| segment_distance(c, e.0, e.1) <= a1 + 2.0 * half).collect();
            let reach = e2.min(f32::MAX / 2.0) + 2.0 * half;
            let cand: Vec<usize> = (0..eaves.len()).filter(|&k| segment_distance(c, eaves[k].0, eaves[k].1) <= reach).collect();
            for y in by..(by + BLOCK).min(y1) {
                let xs = &crossings[(y - y0) as usize];
                for x in bx..(bx + BLOCK).min(x1) {
                    let p = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
                    let inner = xs.partition_point(|&v| v < p[0]) % 2 == 1;
                    // Край картинки куска — по любой стороне. У края — сглаженный;
                    // у стыка — точка целиком того куска, в котором её середина:
                    // полупрозрачные точки двух соседей давали светлый шов.
                    let (d0, edge) = near.iter().map(|(e, o)| (segment_distance(p, e.0, e.1), *o)).fold((f32::MAX, true), |a, b| if b.0 < a.0 { b } else { a });
                    let cov = match (edge, inner) {
                        (false, true) => 1.0,
                        (false, false) => 0.0,
                        (true, true) => (d0 * ppc + 0.5).min(1.0),
                        (true, false) => (0.5 - d0 * ppc).max(0.0),
                    };
                    if cov <= 0.0 {
                        continue;
                    }
                    // Два ближайших края: расстояние, точка на стороне, сама сторона.
                    // `g1` — у ближайшего за вогнутым углом: насколько он дальше
                    // соседнего свеса; около нуля — ендова.
                    let (mut d1, mut d2, mut q1, mut q2, mut e1, mut g1) = (f32::MAX, f32::MAX, p, p, eaves[0], f32::MAX);
                    for &k in &cand {
                        let (a, b) = eaves[k];
                        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                        let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy);
                        let mut q = [a[0] + dx * t.clamp(0.0, 1.0), a[1] + dy * t.clamp(0.0, 1.0)];
                        let mut d = dist(p, q);
                        let mut g = f32::MAX;
                        // За вогнутым углом — по прямой стороны, если до неё дальше,
                        // чем до прямой соседа: грань доходит до биссектрисы угла.
                        // Ближе — точка грани соседа (он тоже в кандидатах), а эта
                        // сторона не в счёт: её расстоянием до вершины вдоль
                        // продолжения соседа шёл ложный гребень.
                        let beyond = if t < 0.0 { turn[k][0] } else if t > 1.0 { turn[k][1] } else { None };
                        if let Some(j) = beyond {
                            let line = |(a, b): ([f32; 2], [f32; 2])| ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() / dist(a, b);
                            let (l, lj) = (line((a, b)), line(eaves[j]));
                            if l < lj {
                                continue;
                            }
                            (d, q, g) = (l, [a[0] + dx * t, a[1] + dy * t], l - lj);
                        }
                        if d < d1 {
                            (d2, q2) = (d1, q1);
                            (d1, q1, e1, g1) = (d, q, (a, b), g);
                        } else if d < d2 {
                            (d2, q2) = (d, q);
                        }
                    }
                    let flat_here = form == RoofForm::Flat || (form == RoofForm::Pitched && d1 > cap);
                    let mut c = if flat_here {
                        let (u, v) = ((p[0] * axis[0] + p[1] * axis[1]) / tile[0], (-p[0] * axis[1] + p[1] * axis[0]) / tile[1]);
                        let mut c = bilinear(tex, u, v).map(|k| k * roof_shade([0.0, 1.0], light, true));
                        if form == RoofForm::Flat {
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
                    } else if let (RoofForm::Dome, Some((c0, rad))) = (form, dome) {
                        // Сверху купол — круг: у макушки почти плоско и светлее,
                        // к низу круче; швы покрытия сходятся к макушке.
                        let d = [p[0] - c0[0], p[1] - c0[1]];
                        let r = d[0].hypot(d[1]);
                        let t = (r / rad).clamp(0.0, 1.0);
                        let down = if r > 1e-4 { [d[0] / r, d[1] / r] } else { [0.0, 1.0] };
                        let k = t.powf(1.5);
                        let shade = 0.7 + 0.3 * (down[0] * light[0] + down[1] * light[1]) * k + 0.18 * (1.0 - k);
                        let seams = (std::f32::consts::TAU * rad / tile[0]).round().max(1.0);
                        let u = (d[1].atan2(d[0]) / std::f32::consts::TAU).rem_euclid(1.0) * seams;
                        bilinear(tex, u, -(rad - r) / tile[1]).map(|k| k * shade)
                    } else if let (RoofForm::Shed, Some(dn)) = (form, shed) {
                        // Одна плоскость: ряды поперёк уклона, низ рисунка — к свесу.
                        let (u, v) = ((-p[0] * dn[1] + p[1] * dn[0]) / tile[0], (p[0] * dn[0] + p[1] * dn[1]) / tile[1]);
                        bilinear(tex, u, v).map(|k| k * roof_shade(dn, light, false))
                    } else if let Some((o, ax, half)) = ridge {
                        // От конька к свесу: скат по ту сторону, где точка.
                        let n = [-ax[1], ax[0]];
                        let s = (p[0] - o[0]) * n[0] + (p[1] - o[1]) * n[1];
                        let down = if s >= 0.0 { n } else { [-n[0], -n[1]] };
                        let along = (p[0] - o[0]) * ax[0] + (p[1] - o[1]) * ax[1];
                        let from_eave = (half - s.abs()).max(0.0);
                        let shade = if form == RoofForm::Round {
                            // Свод: у конька почти плоско, к низу круче.
                            let k = (s.abs() / half.max(1e-3)).clamp(0.0, 1.0).sqrt();
                            0.7 + 0.3 * (down[0] * light[0] + down[1] * light[1]) * k + 0.18 * (1.0 - k)
                        } else {
                            roof_shade(down, light, false)
                        };
                        let mut c = bilinear(tex, along / tile[0], -from_eave / tile[1]).map(|k| k * shade);
                        if form == RoofForm::Gabled && s.abs() < CAP / 2.0 {
                            c = bilinear(tex, along / tile[0] * 2.0, s.abs() / tile[1]).map(|k| k * CAP_SHADE);
                            if (CAP / 2.0 - s.abs()) * ppc < 1.0 {
                                c = c.map(|k| k * 0.45);
                            }
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
                        // вершины: там обе стороны сходятся в одну точку); ендова —
                        // где за вогнутым углом сходятся две грани.
                        if (d2 - d1 < CAP / 2.0 && d1 > CAP && dist(q1, q2) > CAP) || (g1 < CAP / 2.0 && d1 > CAP / 2.0) {
                            c = bilinear(tex, along / tile[0] * 2.0, across / tile[1]).map(|k| k * CAP_SHADE);
                            if (CAP / 2.0 - (d2 - d1).min(g1)) * ppc < 1.0 {
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
                    if cov >= 0.5 {
                        let z = match (form, shed, ridge, dome, span) {
                            _ if flat_here => zt,
                            (RoofForm::Shed, Some(dn), _, _, Some((lo, hi))) => zt - (zt - ze) * ((p[0] * dn[0] + p[1] * dn[1] - lo) / (hi - lo).max(1e-4)).clamp(0.0, 1.0),
                            (RoofForm::Gabled | RoofForm::Round, _, Some((o, ax, half)), _, _) => {
                                let s = (p[0] - o[0]) * -ax[1] + (p[1] - o[1]) * ax[0];
                                ze + (zt - ze) * (1.0 - s.abs() / half.max(1e-4)).clamp(0.0, 1.0)
                            }
                            (RoofForm::Dome, _, _, Some((c0, rad)), _) => {
                                let t = (dist(p, c0) / rad).min(1.0);
                                ze + (zt - ze) * (1.0 - t * t).sqrt()
                            }
                            _ => ze + (zt - ze) * (d1 / run).min(1.0),
                        };
                        let k = (y * w + x) as usize;
                        (relief.top[k], relief.owner[k]) = (z, me);
                    }
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
        OutlineRoof { pieces: vec![RoofPiece { rings: vec![outer, yard], form, paint: "grey".into(), height: 0.0, eaves: 0.0, down: None, across: false }] }
    }

    #[test]
    fn taller_neighbours_and_crowns_shade_the_roof() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let sq = |a: f32, b: f32| vec![vec![[a, a], [b, a], [b, b], [a, b]]];
        let roof = OutlineRoof { pieces: vec![RoofPiece { rings: sq(0.0, 6.0), form: RoofForm::Flat, paint: "grey".into(), height: 2.0, eaves: 2.0, down: None, across: false }] };
        let light = [-0.6, -0.8];
        let draw = |near: &Near| render_outline(&roof, &tex, 10.0, light, ([-10.0, -10.0], [50.0, 50.0]), near);
        let (plain, o) = draw(&Near::default());
        let luma = |img: &image::RgbaImage, x: f32, y: f32| img.get_pixel(((x - o[0]) * 10.0) as u32, ((y - o[1]) * 10.0) as u32).0[0];
        let tall = Near { houses: vec![(sq(-6.0, -1.0), 10.0)], crowns: Vec::new() };
        let (img, _) = draw(&tall);
        assert!(luma(&img, 0.5, 0.5) < luma(&plain, 0.5, 0.5) - 30, "высокий сосед не затенил угол");
        assert_eq!(luma(&img, 5.5, 5.5), luma(&plain, 5.5, 5.5), "тень соседа дальше его высоты");
        let low = Near { houses: vec![(sq(-6.0, -1.0), 1.5)], crowns: Vec::new() };
        assert_eq!(draw(&low).0, plain, "сосед ниже крыши её затенил");
        let tree = Near { houses: Vec::new(), crowns: vec![Crown { pos: [-3.0, 1.0], d: 6.0, bare: false }] };
        let (img, _) = draw(&tree);
        assert!(luma(&img, 1.0, 4.2) < luma(&plain, 1.0, 4.2) - 20, "крона не затенила крышу");
        let bush = Near { houses: Vec::new(), crowns: vec![Crown { pos: [-3.0, 1.0], d: 1.5, bare: false }] };
        assert_eq!(draw(&bush).0, plain, "низкая крона легла на крышу");
    }

    #[test]
    fn tree_shadow_lies_beside_the_crown_and_not_under_roofs() {
        let mut m = map::Map::new(30, 30);
        let tree = |pos: [f32; 2]| map::Prop { asset: crate::library::TREE.into(), pos, size: [4.0, 4.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
        m.props.push(tree([6.0, 6.0]));
        m.props.push(tree([18.0, 6.0]));
        // Дом с плоской крышей там, куда легла бы тень второго дерева.
        let wall = vec![vec![[19.0, 7.0], [26.0, 7.0], [26.0, 14.0], [19.0, 14.0]]];
        m.outline_roofs.push(OutlineRoof { pieces: vec![RoofPiece { rings: wall, form: RoofForm::Flat, paint: "grey".into(), height: 3.0, eaves: 3.0, down: None, across: false }] });
        let s = tree_shade(&m).expect("тени крон нет");
        let res = crate::mask::RES as f32;
        let at = |x: f32, y: f32| s.data[(y * res) as usize * s.w + (x * res) as usize];
        let dir = { let l = m.shadows.light(); [-l[0], -l[1]] };
        let c = [6.0 + dir[0] * 4.0, 6.0 + dir[1] * 4.0];
        assert!(at(c[0], c[1]) as f32 >= TREE_SHADE * 255.0 - 2.0, "тень кроны не там: {}", at(c[0], c[1]));
        assert_eq!(at(6.0 - dir[0] * 3.0, 6.0 - dir[1] * 3.0), 0, "тень к солнцу");
        assert_eq!(at(18.0 + dir[0] * 4.0, 6.0 + dir[1] * 4.0), 0, "тень под крышей");
        // Нарисованная тень гуще кроны — крона её не темнит.
        let mut shaded = m.clone();
        shaded.shade = Some(crate::mask::Mask { data: vec![200; m.rooms.data.len()], ..m.rooms.clone() });
        assert_eq!(tree_shade(&shaded).map_or(0, |s| s.data.iter().copied().max().unwrap_or(0)), 0);
        m.shadows.on = false;
        assert!(tree_shade(&m).is_none());
    }

    #[test]
    fn sun_rays_fall_off_the_tower_by_its_height() {
        // Столб высотой 4 точки в (2, 2), свет слева сверху по диагонали: луч
        // спускается на √2 за шаг и гаснет к третьему шагу.
        let (w, h) = (10, 10);
        let mut top = vec![0.0; w * h];
        top[2 * w + 2] = 4.0;
        let ray = sun_rays(&top, w, h, [-0.6, -0.6], 1.0);
        assert!((ray[3 * w + 3] - (4.0 - 2f32.sqrt())).abs() < 1e-4);
        assert!(ray[5 * w + 5] <= 0.0 && ray[4 * w + 4] > 0.0, "луч дошёл не туда");
        assert_eq!(ray[w + 1], 0.0, "со стороны солнца тени нет");
        assert_eq!(ray[2 * w + 2], 4.0);
    }

    #[test]
    fn outline_for_foundry_marks_the_yard_as_a_hole() {
        let rings = outline(&yard_house(RoofForm::Flat));
        assert_eq!(rings.iter().map(|r| r.1).collect::<Vec<_>>(), [false, true]);
        assert_eq!(rings[0].0[2], [8.0, 8.0]);
        let mut two = yard_house(RoofForm::Flat);
        two.pieces[0].rings.push(vec![[10.0, 0.0], [12.0, 0.0], [12.0, 2.0]]);
        assert_eq!(outline(&two).iter().map(|r| r.1).collect::<Vec<_>>(), [false, true, false], "второй корпус — не двор");
        assert!(outline(&OutlineRoof { pieces: Vec::new() }).is_empty());
    }

    #[test]
    fn outline_roof_slopes_to_every_side_and_leaves_the_yard_open() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let (img, origin) = render_outline(&yard_house(RoofForm::Pitched), &tex, 10.0, [0.6, 0.8], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - origin[0]) * 10.0) as u32, ((y - origin[1]) * 10.0) as u32).0;
        assert_eq!(at(4.0, 4.0)[3], 0, "двор закрыт");
        assert_eq!(at(1.5, 4.0)[3], 255, "скат дырявый");
        // Свет сверху-слева (0,6; 0,8 — откуда): скаты на юг и на север — разной яркости.
        assert_ne!(at(4.0, 7.0)[0], at(4.0, 1.0)[0], "скаты одного света");
        // Вальма: по диагонали от угла — гребень темнее ската рядом.
        assert!(at(1.0, 1.0)[0] < at(1.5, 0.6)[0], "гребня нет: {:?} {:?}", at(1.0, 1.0), at(1.5, 0.6));
    }

    /// Кусок-прямоугольник `lo`..`hi` ровного серого покрытия.
    fn slab(lo: [f32; 2], hi: [f32; 2], form: RoofForm, height: f32, eaves: f32, down: Option<[f32; 2]>) -> RoofPiece {
        RoofPiece { rings: vec![vec![lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]]], form, paint: "grey".into(), height, eaves, down, across: false }
    }

    #[test]
    fn shed_is_one_plane_facing_its_direction() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let light = [0.0, -1.0];
        let shade = |down: [f32; 2]| {
            let roof = OutlineRoof { pieces: vec![slab([0.0, 0.0], [6.0, 6.0], RoofForm::Shed, 5.0, 2.0, Some(down))] };
            let (img, o) = render_outline(&roof, &tex, 10.0, light, ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
            let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 10.0) as u32, ((y - o[1]) * 10.0) as u32).0[0];
            // Одна плоскость: у краёв и посередине — одного света, гребня нет.
            assert_eq!(at(1.0, 1.0), at(5.0, 5.0));
            assert_eq!(at(1.0, 5.0), at(3.0, 3.0));
            at(3.0, 3.0)
        };
        assert!(shade([0.0, -1.0]) > shade([0.0, 1.0]), "скат к свету не светлее");
    }

    #[test]
    fn gabled_slopes_only_to_the_long_sides() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let roof = OutlineRoof { pieces: vec![slab([0.0, 0.0], [10.0, 4.0], RoofForm::Gabled, 8.0, 5.0, None)] };
        let (img, o) = render_outline(&roof, &tex, 10.0, [0.6, 0.8], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 10.0) as u32, ((y - o[1]) * 10.0) as u32).0[0];
        assert_ne!(at(5.0, 1.0), at(5.0, 3.0), "скаты одного света");
        // Торец — щипец: у западного края тот же северный скат, а не вальма.
        assert_eq!(at(0.6, 1.0), at(5.0, 1.0));
        assert_eq!(at(9.4, 3.0), at(5.0, 3.0));
    }

    #[test]
    fn dome_is_shaded_smoothly_not_in_facets() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let ring: Vec<[f32; 2]> = (0..16).map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            [5.0 + 4.0 * a.cos(), 5.0 + 4.0 * a.sin()]
        }).collect();
        let roof = OutlineRoof { pieces: vec![RoofPiece { rings: vec![ring], form: RoofForm::Dome, paint: "grey".into(), height: 30.0, eaves: 26.0, down: None, across: false }] };
        let (img, o) = render_outline(&roof, &tex, 20.0, [0.6, -0.8], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 20.0) as u32, ((y - o[1]) * 20.0) as u32).0[0] as i32;
        // Свет с северо-востока: северный бок светлее южного, а по обе стороны
        // от линии к северному узлу (у шатра там ребро) скачка нет.
        assert!(at(5.0, 2.0) > at(5.0, 8.0));
        assert!((at(4.9, 2.0) - at(5.1, 2.0)).abs() <= 3, "{} {}", at(4.9, 2.0), at(5.1, 2.0));
    }

    #[test]
    fn higher_step_casts_a_shadow_on_the_lower() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        // Ступень Мавзолея: площадка 10×10 высотой 1 клетка, на ней 4×4 высотой 3.
        let roof = OutlineRoof {
            pieces: vec![slab([0.0, 0.0], [10.0, 10.0], RoofForm::Flat, 1.0, 1.0, None), slab([3.0, 3.0], [7.0, 7.0], RoofForm::Flat, 3.0, 3.0, None)],
        };
        // Солнце с северо-запада: тень ложится на юго-восток от верхней ступени.
        let (img, o) = render_outline(&roof, &tex, 10.0, [-0.6, -0.8], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 10.0) as u32, ((y - o[1]) * 10.0) as u32).0[0];
        let (lit, shade) = (at(1.5, 5.0), at(5.5, 8.0));
        assert!(shade < lit - 30, "у ступени нет тени: {shade} против {lit}");
        // Дальше длины тени (2 клетки высоты — 2 клетки) снова светло.
        assert_eq!(at(5.5, 9.5), lit);
        // Сама верхняя ступень не затенена.
        assert!(at(5.0, 5.0) >= lit - 2);
    }

    /// Уступ стены в полметра между длинными стенами — в точку, даже когда
    /// упрощение его оставило (кольцо начинается на уступе); уступ в клетку — нет.
    /// Край крыши при этом — по стене, с уступом.
    #[test]
    fn half_metre_jog_in_a_long_wall_is_ironed_out() {
        let ring = vec![[5.0, 4.0], [5.0, 4.4], [0.0, 4.4], [0.0, 0.0], [10.0, 0.0], [10.0, 4.0]];
        let s = iron_jogs(&simplify_ring(&ring, OUTLINE_TOL));
        assert_eq!(s.len(), 5, "{s:?}");
        assert!((0..s.len()).all(|k| dist(s[k], s[(k + 1) % s.len()]) > 1.0), "{s:?}");
        let step = vec![[5.0, 4.0], [5.0, 5.5], [0.0, 5.5], [0.0, 0.0], [10.0, 0.0], [10.0, 4.0]];
        assert_eq!(iron_jogs(&simplify_ring(&step, OUTLINE_TOL)).len(), 6);
        // Над уступом — крыша до самой стены.
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let paint = |_: &str| Some(&grey);
        let piece = RoofPiece { rings: vec![ring], form: RoofForm::Pitched, paint: "grey".into(), height: 6.0, eaves: 3.0, down: None, across: false };
        let (img, o) = render_outline(&OutlineRoof { pieces: vec![piece] }, &paint, 20.0, [0.0, -1.0], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let alpha = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 20.0) as u32, ((y - o[1]) * 20.0) as u32).0[3];
        assert_eq!(alpha(2.0, 4.3), 255, "край крыши срезал уступ");
        assert_eq!(alpha(7.0, 4.3), 0, "крыша за стеной");
    }

    /// Шифер FA: внизу плитки полоса без единой непрозрачной точки, над ней —
    /// зубчатый край ряда. Без неё — ни белого, ни прозрачного.
    #[test]
    fn fa_roof_band_is_cut_and_never_white() {
        let img = image::RgbaImage::from_fn(8, 12, |_, y| match y {
            0..8 => image::Rgba([90, 90, 90, 255]),
            8 => image::Rgba([90, 90, 90, 200]),
            _ => image::Rgba([255, 255, 255, 0]),
        });
        let out = solid_cover(img);
        assert_eq!(out.dimensions(), (8, 9));
        assert!(out.pixels().all(|p| p.0[3] == 255 && p.0[0] <= 90), "белое или прозрачное в покрытии");
    }

    /// Г-образный дом: за вогнутым углом (4, 4) скаты сходятся прямой ендовой по
    /// биссектрисе, а не дугой вокруг вершины, и край плитки (свес) не тянется
    /// продолжением стороны внутрь крыши.
    #[test]
    fn concave_corner_has_a_straight_valley_not_an_arc() {
        // Плитка в 4 клетки, низ — «свес» красный.
        let tex = image::RgbaImage::from_fn(16, 1024, |_, y| if y >= 960 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([160, 160, 160, 255]) });
        let paint = |_: &str| Some(&tex);
        let l = RoofPiece { rings: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [4.0, 4.0], [4.0, 10.0], [0.0, 10.0]]], form: RoofForm::Pitched, paint: "grey".into(), height: 6.0, eaves: 3.0, down: None, across: false };
        let (img, o) = render_outline(&OutlineRoof { pieces: vec![l] }, &paint, 20.0, [0.0, -1.0], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 20.0) as u32, ((y - o[1]) * 20.0) as u32).0;
        for (x, y) in [(2.5, 3.95), (1.5, 3.95), (3.95, 2.5), (3.95, 1.5)] {
            assert!(at(x, y)[1] > 0, "свес тянется за угол в ({x}, {y}): {:?}", at(x, y));
        }
        // По ту сторону биссектрисы — грань нижней стороны, по эту — правой: дугой
        // вокруг вершины весь угол был бы одной гранью. На самой биссектрисе — ендова.
        let (low, right) = (at(6.0, 3.0)[1], at(3.0, 6.0)[1]);
        assert_ne!(low, right);
        for (x, y) in [(3.4, 3.0), (3.6, 2.8), (3.5, 2.6)] {
            assert_eq!(at(x, y)[1], low, "({x}, {y}) не на грани нижней стороны");
            assert_eq!(at(y, x)[1], right, "({y}, {x}) не на грани правой стороны");
        }
        for k in [3.0, 3.3, 3.6] {
            assert!(at(k, k)[1] != low && at(k, k)[1] != right, "нет ендовы в ({k}, {k})");
        }
    }

    #[test]
    fn facets_of_one_hip_meet_without_a_dark_seam() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        // Две грани вальмы встык по x = 5: верх 25 и 23 м, низ обеих — 20 м.
        let pair = |second_eaves: f32| {
            let roof = OutlineRoof {
                pieces: vec![
                    slab([0.0, 0.0], [5.0, 6.0], RoofForm::Shed, 25.0, 20.0, Some([-1.0, 0.0])),
                    slab([5.0, 0.0], [10.0, 6.0], RoofForm::Shed, 23.0, second_eaves, Some([-1.0, 0.0])),
                ],
            };
            let (img, o) = render_outline(&roof, &tex, 10.0, [0.0, -1.0], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
            let at = |x: f32, y: f32| img.get_pixel(((x - o[0]) * 10.0) as u32, ((y - o[1]) * 10.0) as u32).0;
            (at(5.05, 3.0), at(6.5, 3.0))
        };
        let (seam, inside) = pair(20.0);
        assert_eq!(seam, inside, "на стыке граней шов");
        // Вторая выше первой целиком (низ 30 м) — уступ, по краю тёмная кромка.
        let (step, inside) = pair(30.0);
        assert!(step[0] < inside[0], "у уступа нет кромки: {step:?} {inside:?}");
    }

    #[test]
    fn flat_outline_roof_has_a_parapet_and_stays_in_the_map() {
        let grey = image::RgbaImage::from_pixel(16, 16, image::Rgba([160, 160, 160, 255]));
        let tex = |_: &str| Some(&grey);
        let (img, origin) = render_outline(&yard_house(RoofForm::Flat), &tex, 10.0, [0.6, 0.8], ([-5.0, -5.0], [50.0, 50.0]), &Near::default());
        let at = |x: f32, y: f32| img.get_pixel(((x - origin[0]) * 10.0) as u32, ((y - origin[1]) * 10.0) as u32).0;
        assert!(at(0.15, 4.0)[0] > at(1.5, 4.0)[0], "парапета нет");
        // Дом за краем карты — картинка только до края.
        let (cut, o) = render_outline(&yard_house(RoofForm::Flat), &tex, 10.0, [0.6, 0.8], ([0.0, 0.0], [4.0, 4.0]), &Near::default());
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
