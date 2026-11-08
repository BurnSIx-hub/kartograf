//! Крыша в картинку — для выгрузки. На холсте её рисуют сетки egui
//! (`App::draw_roof`), здесь та же геометрия (`Roof::geometry`) считается по
//! точкам: в Foundry крыша уходит отдельным тайлом с прозрачным фоном.

use crate::map::{self, Roof, RoofShape, convex_depth, roof_shade, segment_distance};

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
