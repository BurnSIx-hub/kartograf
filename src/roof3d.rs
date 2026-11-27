//! 3D-крыши ([`crate::scene3d::Roof3`]) из крыш карты. Картинка крыши из 2D ложится на скаты
//! сверху: сверху 3D-крыша та же, что на карте.
//!
//! Прямоугольная ([`Roof`]) — точными скатами по [`Roof::geometry`]: скат проходит через верх
//! стен и уходит свесом ниже; щипцы и торцы — отвесными стенками до ската. Крыша по контуру
//! ([`OutlineRoof`]: дома из OSM, крыши руками по точкам) — сеткой высот того же прохода, что
//! рисует её картинку ([`crate::roofs::outline_heights`]), низом на верху стен; по контуру —
//! стенка от верха стен до ската (щипцы двускатных).

use crate::map::{OutlineRoof, Roof, RoofShape, convex_depth};
use crate::scene3d::{Mesh, Picture, Roof3, Vertex, quad, tri};

/// Уклон двускатной и вальмовой: подъём на клетку пути (~39°).
pub const PITCH: f32 = 0.8;
/// Односкатной — положе.
const SHED_PITCH: f32 = 0.4;
/// Шатёр: высота макушки над стенами к полуширине дома по стенам.
const TENT_PITCH: f32 = 1.6;
/// Плоская: верх над стенами и толщина плиты.
const FLAT_RISE: f32 = 0.06;
const FLAT_SLAB: f32 = 0.14;
/// Сетка высот крыши по контуру: точек на клетку.
const OUTLINE_PPC: f32 = 4.0;
/// Стенка по контуру такой крыши — кусками не длиннее, клетки.
const SKIRT_PIECE: f32 = 0.25;

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Точка карты в картинке крыши: `frame` — её левый верхний угол, ширина и высота (клетки).
fn uv(frame: [f32; 4], p: [f32; 2]) -> [f32; 2] {
    [(p[0] - frame[0]) / frame[2].max(1e-6), (p[1] - frame[1]) / frame[3].max(1e-6)]
}

fn centroid(pts: &[[f32; 2]]) -> [f32; 2] {
    let n = pts.len().max(1) as f32;
    let s = pts.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

/// Отвесная стенка над отрезком `a`–`b` от `bottom` до `ha` и `hb`, лицом по `out`.
fn wall(m: &mut Mesh, a: [f32; 2], b: [f32; 2], bottom: f32, (ha, hb): (f32, f32), out: [f32; 2]) {
    if ha.max(hb) - bottom < 1e-3 {
        return;
    }
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    quad(m, [[a[0], bottom, a[1]], [b[0], bottom, b[1]], [b[0], hb, b[1]], [a[0], ha, a[1]]], [out[0], 0.0, out[1]], [[0.0, ha - bottom], [len, hb - bottom], [len, 0.0], [0.0, 0.0]]);
}

/// Нормаль отрезка в сторону от точки `c` (середины крыши).
fn away(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> [f32; 2] {
    let d = sub(b, a);
    let l = d[0].hypot(d[1]).max(1e-9);
    let n = [d[1] / l, -d[0] / l];
    let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    if dot(n, sub(m, c)) < 0.0 { [-n[0], -n[1]] } else { n }
}

/// Прямоугольная крыша на стенах высотой `eave` (верх стен над полом этажа) и толщиной
/// `2 × half`: скат проходит через внешний верхний край стены, а не через её ось — иначе
/// наружная половина стены выходила бы сквозь свес.
pub fn rect(r: &Roof, eave: f32, half: f32, picture: Picture, frame: [f32; 4]) -> Roof3 {
    let outline = r.outline();
    let c = centroid(&outline);
    // Свес от наружной стороны стены.
    let o = r.overhang.max(0.0) - half.max(0.0);
    let (mut cover, mut gables) = (Mesh::default(), Mesh::default());
    let on_edge = |p: [f32; 2]| convex_depth(&outline, p).abs() < 1e-3;
    let p3 = |p: [f32; 2], y: f32| [p[0], y, p[1]];
    if r.shape == RoofShape::Flat {
        let h = eave + FLAT_RISE;
        for [i, j, k] in crate::platform::triangulate(&outline) {
            let (a, b, d) = (outline[i], outline[j], outline[k]);
            tri(&mut cover, [p3(a, h), p3(b, h), p3(d, h)], [0.0, 1.0, 0.0], [uv(frame, a), uv(frame, b), uv(frame, d)]);
        }
        for e in 0..outline.len() {
            let (a, b) = (outline[e], outline[(e + 1) % outline.len()]);
            wall(&mut gables, a, b, h - FLAT_SLAB, (h, h), away(a, b, c));
        }
    } else {
        let (planes, _) = r.geometry();
        // Скат: от верха стен на `D − свес` от конька (или верхнего края) вниз по `down`.
        let ds: Vec<f32> = planes.iter().map(|pl| pl.poly.iter().map(|v| dot(sub(*v, pl.origin), pl.down)).fold(0.0, f32::max)).collect();
        let dmin = ds.iter().copied().fold(f32::MAX, f32::min);
        let pitch: Vec<f32> = (0..planes.len())
            .map(|i| match r.shape {
                RoofShape::Shed => SHED_PITCH,
                // Шатёр: грани разной ширины сходятся в одну макушку — у широкой уклон положе.
                RoofShape::Tent => TENT_PITCH * (dmin - o).max(0.05) / (ds[i] - o).max(0.05),
                _ => PITCH,
            })
            .collect();
        let height = |i: usize, p: [f32; 2]| eave + pitch[i] * ((ds[i] - o).max(0.0) - dot(sub(p, planes[i].origin), planes[i].down));
        let bottom = planes.iter().enumerate().flat_map(|(i, pl)| pl.poly.iter().map(move |&v| (i, v))).map(|(i, v)| height(i, v)).fold(f32::MAX, f32::min);
        for (i, pl) in planes.iter().enumerate() {
            let n = norm3([pitch[i] * pl.down[0], 1.0, pitch[i] * pl.down[1]]);
            for t in 1..pl.poly.len().saturating_sub(1) {
                let (a, b, d) = (pl.poly[0], pl.poly[t], pl.poly[t + 1]);
                tri(&mut cover, [p3(a, height(i, a)), p3(b, height(i, b)), p3(d, height(i, d))], n, [uv(frame, a), uv(frame, b), uv(frame, d)]);
            }
            // Щипцы и торцы: стороны скатов, что лежат на краю крыши, — стенкой до низа свеса.
            for e in 0..pl.poly.len() {
                let (a, b) = (pl.poly[e], pl.poly[(e + 1) % pl.poly.len()]);
                if on_edge(a) && on_edge(b) && on_edge([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]) {
                    wall(&mut gables, a, b, bottom, (height(i, a), height(i, b)), away(a, b, c));
                }
            }
        }
    }
    Roof3 { picture, frame, cover, gables, outline: vec![outline] }
}

/// Крыша по контуру на стенах высотой `eave`: сетка высот её же рисования, низом на стенах.
/// `clip` — рамка карты (клетки). Пустая (весь дом за рамкой) — `None`.
pub fn outline(r: &OutlineRoof, eave: f32, picture: Picture, frame: [f32; 4], clip: ([f32; 2], [f32; 2])) -> Option<Roof3> {
    let (top, w, h, origin) = crate::roofs::outline_heights(r, OUTLINE_PPC, clip);
    let (w, h) = (w as usize, h as usize);
    if w < 2 || h < 2 {
        return None;
    }
    let base = top.iter().copied().filter(|&z| z > f32::MIN).fold(f32::MAX, f32::min);
    if base == f32::MAX {
        return None;
    }
    // Край — на точку дальше: сетка накрывает край крыши, лишнее срезает прозрачность картинки.
    let mut z = top.clone();
    for j in 0..h {
        for i in 0..w {
            if top[j * w + i] > f32::MIN {
                continue;
            }
            let mut best = f32::MAX;
            for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
                let (ii, jj) = (i as isize + di, j as isize + dj);
                if ii >= 0 && jj >= 0 && (ii as usize) < w && (jj as usize) < h {
                    let v = top[jj as usize * w + ii as usize];
                    if v > f32::MIN {
                        best = best.min(v);
                    }
                }
            }
            if best < f32::MAX {
                z[j * w + i] = best;
            }
        }
    }
    let defined = |i: usize, j: usize| z[j * w + i] > f32::MIN;
    let y = |v: f32| eave + (v - base);
    let (fx0, fz0, fx1, fz1) = (frame[0], frame[1], frame[0] + frame[2], frame[1] + frame[3]);
    let at = |i: usize, j: usize| [(origin[0] + (i as f32 + 0.5) / OUTLINE_PPC).clamp(fx0, fx1), (origin[1] + (j as f32 + 0.5) / OUTLINE_PPC).clamp(fz0, fz1)];
    let zh = |i: isize, j: isize, own: f32| {
        if i < 0 || j < 0 || i as usize >= w || j as usize >= h || !defined(i as usize, j as usize) { own } else { z[j as usize * w + i as usize] }
    };
    let mut cover = Mesh::default();
    let mut index = vec![u32::MAX; w * h];
    let step = 1.0 / OUTLINE_PPC;
    for j in 0..h {
        for i in 0..w {
            if !defined(i, j) {
                continue;
            }
            let own = z[j * w + i];
            let (ii, jj) = (i as isize, j as isize);
            let n = norm3([-(zh(ii + 1, jj, own) - zh(ii - 1, jj, own)) / (2.0 * step), 1.0, -(zh(ii, jj + 1, own) - zh(ii, jj - 1, own)) / (2.0 * step)]);
            let p = at(i, j);
            index[j * w + i] = cover.v.len() as u32;
            cover.v.push(Vertex { p: [p[0], y(own), p[1]], n, uv: uv(frame, p) });
        }
    }
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            let (a, b, c, d) = (index[j * w + i], index[j * w + i + 1], index[(j + 1) * w + i + 1], index[(j + 1) * w + i]);
            if [a, b, c, d].contains(&u32::MAX) {
                continue;
            }
            cover.idx.extend([a, d, c, a, c, b]);
        }
    }
    // Стенка по контуру крыши: от верха стен до ската над краем.
    let rings: Vec<Vec<[f32; 2]>> = crate::roofs::outline(r).into_iter().map(|(ring, _)| ring).filter(|r| r.len() >= 3).collect();
    let height_at = |p: [f32; 2]| {
        let (fi, fj) = ((p[0] - origin[0]) * OUTLINE_PPC - 0.5, (p[1] - origin[1]) * OUTLINE_PPC - 0.5);
        let (ci, cj) = (fi.round() as isize, fj.round() as isize);
        let mut best: Option<(f32, f32)> = None;
        for dj in -1..=1 {
            for di in -1..=1 {
                let (i, j) = (ci + di, cj + dj);
                if i < 0 || j < 0 || i as usize >= w || j as usize >= h || !defined(i as usize, j as usize) {
                    continue;
                }
                let d = (i as f32 - fi).hypot(j as f32 - fj);
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, z[j as usize * w + i as usize]));
                }
            }
        }
        best.map_or(eave, |b| y(b.1))
    };
    let mut gables = Mesh::default();
    let all: Vec<[f32; 2]> = rings.iter().flatten().copied().collect();
    let c = centroid(&all);
    for ring in &rings {
        for e in 0..ring.len() {
            let (a, b) = (ring[e], ring[(e + 1) % ring.len()]);
            let n = ((b[0] - a[0]).hypot(b[1] - a[1]) / SKIRT_PIECE).ceil().max(1.0) as usize;
            let out = away(a, b, c);
            for t in 0..n {
                let lerp = |f: f32| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
                let (p, q) = (lerp(t as f32 / n as f32), lerp((t + 1) as f32 / n as f32));
                wall(&mut gables, p, q, eave, (height_at(p), height_at(q)), out);
            }
        }
    }
    Some(Roof3 { picture, frame, cover, gables, outline: rings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic() -> Picture {
        Picture { key: "roof".into(), size: [100, 100], alpha: true }
    }

    fn ys(m: &Mesh) -> (f32, f32) {
        m.v.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v.p[1]), hi.max(v.p[1])))
    }

    #[test]
    fn a_gable_roof_meets_the_wall_tops_and_rises_to_its_ridge() {
        let mut r = Roof::new([0.0, 0.0], [6.0, 4.0], RoofShape::Gable, "x".into());
        r.overhang = 0.25;
        let frame = [-0.25, -0.25, 6.5, 4.5];
        let roof = rect(&r, 2.0, 0.0, pic(), frame);
        let (lo, hi) = ys(&roof.cover);
        // Конёк — над серединой на полширины по стенам × уклон; свес — ниже верха стен.
        assert!((hi - (2.0 + PITCH * 2.0)).abs() < 1e-4, "{hi}");
        assert!((lo - (2.0 - PITCH * 0.25)).abs() < 1e-4, "{lo}");
        // Над стеной (y = 0) скат как раз на верху стен.
        let on_wall = roof.cover.v.iter().filter(|v| v.p[2].abs() < 1e-4).map(|v| v.p[1]).collect::<Vec<_>>();
        assert!(on_wall.is_empty() || on_wall.iter().all(|&y| y >= 2.0 - 1e-4));
        // Щипцы на торцах: от низа свеса до конька.
        let (glo, ghi) = ys(&roof.gables);
        assert!((ghi - hi).abs() < 1e-4 && (glo - lo).abs() < 1e-4);
        assert!(roof.gables.v.iter().all(|v| v.p[0].abs() < 0.26 || (v.p[0] - 6.0).abs() < 0.26), "щипцы только на торцах");
        // Картинка — по рамке: углы крыши в углах картинки.
        assert!(roof.cover.v.iter().all(|v| (0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1])));

        // Стена толщиной 0,25: скат — через её наружный верхний край, не через ось. Иначе
        // наружная половина стены выходит сквозь свес (в 3D Canvas — полосой поверх края ската).
        let thick = rect(&r, 2.0, 0.125, pic(), frame);
        let (tlo, thi) = ys(&thick.cover);
        assert!((thi - (2.0 + PITCH * 2.125)).abs() < 1e-4, "{thi}");
        assert!((tlo - (2.0 - PITCH * 0.125)).abs() < 1e-4, "{tlo}");
        // Высота ската над наружной стороной стены (z = −0,125) — верх стены.
        let n = &thick.cover;
        let at = |z: f32| {
            let t = n.idx.chunks(3).find_map(|t| {
                let (a, b, c) = (n.v[t[0] as usize].p, n.v[t[1] as usize].p, n.v[t[2] as usize].p);
                let (z0, z1) = (a[2].min(b[2]).min(c[2]), a[2].max(b[2]).max(c[2]));
                (z0 <= z && z <= z1 && a[0].min(b[0]).min(c[0]) <= 3.0 && a[0].max(b[0]).max(c[0]) >= 3.0).then_some((a, b, c))
            })?;
            // Скат плоский: высота линейна по z вдоль ската.
            let (p, q) = if (t.0[2] - t.1[2]).abs() > 1e-6 { (t.0, t.1) } else { (t.0, t.2) };
            Some(p[1] + (q[1] - p[1]) * (z - p[2]) / (q[2] - p[2]))
        };
        let face = at(-0.125).unwrap();
        assert!((face - 2.0).abs() < 1e-4, "над наружной стороной стены — {face}");
    }

    #[test]
    fn a_flat_roof_is_a_thin_slab_on_the_walls() {
        let r = Roof::new([0.0, 0.0], [4.0, 4.0], RoofShape::Flat, "x".into());
        let roof = rect(&r, 2.0, 0.0, pic(), [-0.25, -0.25, 4.5, 4.5]);
        let (lo, hi) = ys(&roof.cover);
        assert!((lo - hi).abs() < 1e-6 && (hi - 2.0 - FLAT_RISE).abs() < 1e-6);
        assert!(crate::scene3d::inside(&roof.outline, [2.0, 2.0]) && !crate::scene3d::inside(&roof.outline, [6.0, 2.0]));
    }

    #[test]
    fn a_tent_comes_to_one_apex() {
        let r = Roof::new([0.0, 0.0], [4.0, 3.0], RoofShape::Tent, "x".into());
        let roof = rect(&r, 2.0, 0.0, pic(), [-0.5, -0.5, 5.0, 4.0]);
        let apex: Vec<f32> = roof.cover.v.iter().filter(|v| (v.p[0] - 2.0).abs() < 1e-4 && (v.p[2] - 1.5).abs() < 1e-4).map(|v| v.p[1]).collect();
        assert!(apex.len() >= 8);
        assert!(apex.iter().all(|&y| (y - apex[0]).abs() < 1e-3), "{apex:?}");
    }
}
