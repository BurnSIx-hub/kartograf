//! Модель строго сверху в картинку: ортогональная проекция, буфер глубины, текстуры,
//! свет сверху-слева. Перед модели (+X в WoW) смотрит вниз картинки — как у предметов FA.
//! Ось WoW: X — вперёд, Y — влево, Z — вверх; сверху левый бок фигуры, смотрящей вниз, — справа.

use super::blp::Rgba;
use super::m2::Model;

pub struct Options {
    /// Точек на клетку в готовой картинке.
    pub ppc: f32,
    /// Клеток в ярде WoW: клетка D&D — 5 футов, ярд — 3 фута.
    pub cells_per_yard: f32,
    /// Во сколько раз рисовать крупнее перед уменьшением (сглаживание краёв).
    pub supersample: u32,
    /// Не больше стольких точек по стороне готовой картинки: крупное рисуется реже.
    pub max_px: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { ppc: crate::library::PX_PER_CELL, cells_per_yard: 0.6, supersample: 3, max_px: 4096 }
    }
}

pub struct Sprite {
    pub img: image::RgbaImage,
    /// Размер картинки в клетках.
    pub cells: [f32; 2],
    /// Сколько треугольников нарисовано и сколько кусков пропущено (свечения, наложения).
    pub drawn: usize,
    pub skipped: usize,
    /// Где начало координат модели (точка, которой её ставит игра) — в клетках от середины
    /// картинки, при повороте 0: по ней предмет из расстановки WMO встаёт на своё место.
    pub origin: [f32; 2],
}

/// Направление на свет в осях модели: сверху, с верхнего левого угла картинки (−X, −Y).
const LIGHT: [f32; 3] = [-0.42, -0.42, 0.80];

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

struct Tri {
    p: [[f32; 2]; 3],
    z: [f32; 3],
    uv: [[f32; 2]; 3],
    n: [[f32; 3]; 3],
    tex: Option<usize>,
    blend: u16,
    unlit: bool,
}

pub fn render(model: &Model, textures: &[Option<Rgba>], opt: &Options) -> Result<Sprite, String> {
    // Что рисуем: сплошное, с вырезом и полупрозрачное; свечения и наложения — нет.
    let mut skipped = 0;
    let mut chosen = Vec::new();
    for b in &model.batches {
        let mat = model.materials.get(b.material);
        let blend = mat.map_or(0, |m| m.blend);
        // Свечения и наложения не рисуем; невидимые в покое (плоскости эмиттеров) — тоже.
        if blend > 2 || b.alpha < 0.05 {
            skipped += 1;
            continue;
        }
        chosen.push((b, blend, mat.is_some_and(|m| m.flags & 1 != 0)));
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for (b, _, _) in &chosen {
        for t in &b.triangles {
            for &i in t {
                let p = model.vertices.get(i as usize).ok_or("M2: вершина за краем")?.pos;
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return Err("нечего рисовать: у модели нет сплошных частей".into());
    }
    // Масштаб: точки на ярд в готовой картинке, с уменьшением для крупного.
    let (span_u, span_v) = (hi[1] - lo[1], hi[0] - lo[0]);
    // Стоит ребром (водопад, вертикальная плоскость): сверху — линия, а не предмет.
    if span_u.max(span_v) * opt.cells_per_yard < 0.08 {
        return Err(format!("слишком мелкий: {:.2} × {:.2} клетки", span_u * opt.cells_per_yard, span_v * opt.cells_per_yard));
    }
    if span_u.min(span_v) * opt.cells_per_yard < 0.05 {
        return Err(format!("сверху видно ребро: {:.2} × {:.2} клетки", span_u * opt.cells_per_yard, span_v * opt.cells_per_yard));
    }
    let mut ppy = opt.ppc * opt.cells_per_yard;
    let longest = span_u.max(span_v) * ppy;
    if longest > (opt.max_px - 4) as f32 {
        ppy *= (opt.max_px - 4) as f32 / longest;
    }
    let pad = 2u32;
    let (w, h) = ((span_u * ppy).ceil() as u32 + pad * 2, (span_v * ppy).ceil() as u32 + pad * 2);
    // Сглаживание — пока буфер не больше ~25 млн точек: крупная модель в 4096 точек с 3× —
    // это 150 млн точек и гигабайты памяти.
    let mut ss = opt.supersample.max(1);
    while ss > 1 && (w * ss) as u64 * (h * ss) as u64 > 25_000_000 {
        ss -= 1;
    }
    let (bw, bh) = (w * ss, h * ss);
    let s = ppy * ss as f32;
    let off = (pad * ss) as f32;
    let to_screen = |p: [f32; 3]| [(p[1] - lo[1]) * s + off, (p[0] - lo[0]) * s + off];

    let mut tris = Vec::new();
    for (b, blend, unlit) in &chosen {
        for t in &b.triangles {
            let v = t.map(|i| &model.vertices[i as usize]);
            tris.push(Tri {
                p: v.map(|v| to_screen(v.pos)),
                z: v.map(|v| v.pos[2]),
                uv: v.map(|v| v.uv),
                n: v.map(|v| v.normal),
                tex: b.texture.filter(|&t| textures.get(t).is_some_and(Option::is_some)),
                blend: *blend,
                unlit: *unlit,
            });
        }
    }
    let wrap = |ti: usize| model.textures.get(ti).map_or([true, true], |t| [t.flags & 1 != 0, t.flags & 2 != 0]);
    let zr = (hi[2] - lo[2]).max(1e-3);
    let light = norm(LIGHT);
    let shade = |n: [f32; 3], z: f32, unlit: bool| -> f32 {
        if unlit {
            return 1.0;
        }
        let n = norm(n);
        let lambert = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        // Ниже — темнее: сверху так читается, что выше.
        let height = 0.78 + 0.22 * ((z - lo[2]) / zr).clamp(0.0, 1.0);
        (0.52 + 0.62 * lambert) * height
    };

    let mut color = vec![[0f32; 4]; (bw * bh) as usize];
    let mut depth = vec![f32::MIN; (bw * bh) as usize];
    // Сплошное — первым; полупрозрачное — после, снизу вверх, поверх сплошного по глубине.
    let (solid, mut glass): (Vec<&Tri>, Vec<&Tri>) = tris.iter().partition(|t| t.blend < 2);
    glass.sort_by(|a, b| (a.z.iter().sum::<f32>()).total_cmp(&b.z.iter().sum::<f32>()));
    let mut drawn = 0;
    for (pass, list) in [(0, solid), (1, glass)] {
        for t in list {
            let [a, b, c] = t.p;
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            drawn += 1;
            let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
            let x1 = (a[0].max(b[0]).max(c[0]).ceil() as u32).min(bw);
            let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
            let y1 = (a[1].max(b[1]).max(c[1]).ceil() as u32).min(bh);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                        continue;
                    }
                    let i = (y * bw + x) as usize;
                    let z = w0 * t.z[0] + w1 * t.z[1] + w2 * t.z[2];
                    if z < depth[i] {
                        continue;
                    }
                    let uv = [w0 * t.uv[0][0] + w1 * t.uv[1][0] + w2 * t.uv[2][0], w0 * t.uv[0][1] + w1 * t.uv[1][1] + w2 * t.uv[2][1]];
                    let mut c4 = match t.tex {
                        Some(ti) => textures[ti].as_ref().unwrap().sample(uv[0], uv[1], wrap(ti)),
                        None => [0.6, 0.6, 0.6, 1.0],
                    };
                    match t.blend {
                        0 => c4[3] = 1.0,
                        1 => {
                            if c4[3] < 0.5 {
                                continue;
                            }
                            c4[3] = 1.0;
                        }
                        _ => {}
                    }
                    let n = [0, 1, 2].map(|k| w0 * t.n[0][k] + w1 * t.n[1][k] + w2 * t.n[2][k]);
                    let k = shade(n, z, t.unlit);
                    let rgb = [c4[0] * k, c4[1] * k, c4[2] * k];
                    if pass == 0 {
                        color[i] = [rgb[0].min(1.0), rgb[1].min(1.0), rgb[2].min(1.0), 1.0];
                        depth[i] = z;
                    } else {
                        // Поверх того, что уже есть, с предумноженной альфой.
                        let a = c4[3];
                        let d = color[i];
                        color[i] = [rgb[0].min(1.0) * a + d[0] * (1.0 - a), rgb[1].min(1.0) * a + d[1] * (1.0 - a), rgb[2].min(1.0) * a + d[2] * (1.0 - a), a + d[3] * (1.0 - a)];
                    }
                }
            }
        }
    }
    // Уменьшение: среднее по квадрату ss×ss (цвет предумножен на альфу).
    let mut img = image::RgbaImage::new(w, h);
    let n = (ss * ss) as f32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let c = color[((y * ss + dy) * bw + x * ss + dx) as usize];
                    for k in 0..4 {
                        acc[k] += c[k];
                    }
                }
            }
            let a = acc[3] / n;
            if a <= 0.0 {
                continue;
            }
            let px = [0, 1, 2].map(|k| ((acc[k] / n / a).clamp(0.0, 1.0) * 255.0).round() as u8);
            img.put_pixel(x, y, image::Rgba([px[0], px[1], px[2], (a.clamp(0.0, 1.0) * 255.0).round() as u8]));
        }
    }
    // Клеток на точку: ppy точек — ярд, ярд — cells_per_yard клеток.
    let cell_per_px = opt.cells_per_yard / ppy;
    let origin = [(pad as f32 - lo[1] * ppy - w as f32 / 2.0) * cell_per_px, (pad as f32 - lo[0] * ppy - h as f32 / 2.0) * cell_per_px];
    Ok(Sprite { img, cells: [w as f32 * cell_per_px, h as f32 * cell_per_px], drawn, skipped, origin })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wow::m2;

    #[test]
    fn a_triangle_seen_from_above_lands_where_its_axes_say() {
        let (m, s) = m2::tests::tiny_model();
        let model = m2::parse(&m, Some(&s)).unwrap();
        // Белая текстура 1×1.
        let tex = Rgba { w: 1, h: 1, px: vec![[255, 255, 255, 255]] };
        let opt = Options { ppc: 60.0, cells_per_yard: 0.6, supersample: 2, max_px: 4096 };
        let sp = render(&model, &[Some(tex)], &opt).unwrap();
        // Треугольник (0,0)-(1,0)-(0,1) ярда: 36 точек на ярд, поля по 2.
        assert_eq!((sp.img.width(), sp.img.height()), (40, 40));
        assert!((sp.cells[0] - 40.0 / 60.0).abs() < 1e-4);
        let a = |x: u32, y: u32| sp.img.get_pixel(x, y)[3];
        // Угол прямой — у начала (слева сверху); гипотенуза отрезает правый нижний угол.
        assert_eq!(a(4, 4), 255);
        assert_eq!(a(35, 35), 0);
        // +X модели — вниз картинки: вершина (1,0) у нижнего левого угла.
        assert_eq!(a(4, 30), 255);
        assert_eq!(a(30, 4), 255);
        assert_eq!(sp.drawn, 1);
        // Начало координат модели — в углу прямого угла: 2 точки поля от края, середина — 20.
        assert!((sp.origin[0] + 18.0 / 60.0).abs() < 1e-4 && (sp.origin[1] + 18.0 / 60.0).abs() < 1e-4, "{:?}", sp.origin);
    }
}
