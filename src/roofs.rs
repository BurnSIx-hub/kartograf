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

/// Картинка крыши: `ppc` точек на клетку. Возвращает картинку и её левый
/// верхний угол в клетках.
pub fn render(roof: &Roof, tex: &image::RgbaImage, tile: [f32; 2], ppc: f32, light: [f32; 2]) -> (image::RgbaImage, [f32; 2]) {
    let out = roof.outline();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &out {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    // Начало — на целой точке картинки сцены.
    let origin = [(lo[0] * ppc).floor() / ppc, (lo[1] * ppc).floor() / ppc];
    let (w, h) = (((hi[0] - origin[0]) * ppc).ceil() as u32 + 1, ((hi[1] - origin[1]) * ppc).ceil() as u32 + 1);
    let mut img = image::RgbaImage::new(w.max(1), h.max(1));
    let (planes, lines) = roof.geometry();
    let flat = roof.shape == RoofShape::Flat;
    let sample = |u: f32, v: f32| -> [f32; 3] {
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
    for y in 0..img.height() {
        for x in 0..img.width() {
            let p = [origin[0] + (x as f32 + 0.5) / ppc, origin[1] + (y as f32 + 0.5) / ppc];
            let depth = convex_depth(&out, p) * ppc;
            let cov = (depth + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
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
            if depth < RIM_PX {
                c = c.map(|k| k * 0.4);
            }
            img.put_pixel(x, y, image::Rgba([c[0].round().clamp(0.0, 255.0) as u8, c[1].round().clamp(0.0, 255.0) as u8, c[2].round().clamp(0.0, 255.0) as u8, (cov * 255.0).round() as u8]));
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
    fn rendered_roof_is_opaque_inside_and_lit_side_brighter() {
        let tex = image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 100, 80, 255]));
        let roof = Roof { overhang: 0.0, ..Roof::new([0.0, 0.0], [4.0, 2.0], RoofShape::Gable, String::new()) };
        // Свет сверху: северный скат светлее южного.
        let (img, origin) = render(&roof, &tex, [1.0, 1.0], 50.0, [0.0, -1.0]);
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
