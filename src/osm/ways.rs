//! Дороги, рельсы, ручьи и геометрия линий и контуров.

use super::*;

/// Чем выложена дорога: по тегу `surface`, без него — по виду дороги и месту
/// (в городе тротуар плиткой, за городом тропа — земля). Проезжая мощёная —
/// булыжник: асфальта в D&D нет. Утоптанный гравий — светлый, как на снимке;
/// травяная тропа — примятая бурая трава, а не земля.
pub(super) fn paving(t: &Tags, hw: &str, rural: bool) -> &'static str {
    let car = !matches!(hw, "pedestrian" | "footway" | "path" | "steps" | "cycleway" | "bridleway" | "track");
    match get(t, "surface") {
        "asphalt" | "paved" | "concrete" | "concrete:plates" | "paving_stones" | "chipseal" => {
            if car {
                "street"
            } else {
                "stone_tiles"
            }
        }
        "sett" | "cobblestone" | "unhewn_cobblestone" => "street",
        "gravel" | "fine_gravel" | "compacted" => "path_gravel",
        "pebblestone" | "rock" => "gravel",
        "grass" | "grass_paver" => "rough_grass",
        "dirt" | "ground" | "earth" | "unpaved" | "woodchips" => "dirt",
        "mud" => "mud",
        "sand" => "sand",
        _ if car => "street",
        _ if rural || matches!(hw, "track" | "path" | "bridleway") => "dirt",
        _ => "stone_tiles",
    }
}

/// Не мощёное: край у такой дорожки мягкий и неровный.
pub(super) fn natural_surface(tex: &str) -> bool {
    matches!(tex, "dirt" | "gravel" | "path_gravel" | "rough_grass" | "mud" | "sand")
}

/// Улица: ширина в метрах, текстура, порядок (выше — поверх).
pub(super) fn road(t: &Tags, rural: bool) -> Option<(f32, &'static str, u8)> {
    if get(t, "area") == "yes" {
        return None;
    }
    let hw = get(t, "highway");
    let w = match hw {
        "motorway" | "trunk" => 12.0,
        "primary" => 10.0,
        "secondary" => 9.0,
        "tertiary" => 8.0,
        "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" | "tertiary_link" => 7.0,
        "unclassified" | "residential" => 6.0,
        "living_street" | "pedestrian" => 5.0,
        "service" => 4.0,
        "track" => 3.0,
        "footway" | "steps" | "cycleway" | "bridleway" => 2.0,
        "path" => 1.5,
        _ => return None,
    };
    let order = if matches!(hw, "pedestrian" | "footway" | "path" | "steps" | "cycleway" | "bridleway" | "track") { 3 } else { 4 };
    Some((width_tag(t).unwrap_or(w), paving(t, hw, rural), order))
}

/// Рельсовый путь: колея в метрах и есть ли насыпь (у трамвая рельсы в мостовой).
pub(super) fn rail(t: &Tags) -> Option<(f32, bool)> {
    let ballast = match get(t, "railway") {
        "rail" | "light_rail" | "narrow_gauge" | "subway" | "disused" | "preserved" | "funicular" => true,
        "tram" => false,
        _ => return None,
    };
    let gauge = get(t, "gauge").split(';').next().and_then(|g| g.trim().parse::<f32>().ok()).filter(|g| (500.0..2000.0).contains(g)).unwrap_or(1435.0);
    Some((gauge / 1000.0, ballast))
}

/// Ручей, канал, река: ширина в метрах и глубокая ли вода.
pub(super) fn stream(t: &Tags) -> Option<(f32, bool)> {
    let (w, deep) = match get(t, "waterway") {
        "river" => (20.0, true),
        "canal" => (12.0, true),
        "stream" => (3.0, false),
        "ditch" | "drain" => (1.5, false),
        _ => return None,
    };
    Some((width_tag(t).unwrap_or(w), deep))
}

/// Ломаная, сдвинутая вбок на `d` клеток (плюс — влево по ходу).
pub(super) fn offset(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    (0..n)
        .map(|k| {
            let (a, b) = (pts[k.saturating_sub(1)], pts[(k + 1).min(n - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            [pts[k][0] + dy / len * d, pts[k][1] - dx / len * d]
        })
        .collect()
}

/// Куски ломаной у карты: точки дальше `margin` клеток от края выбрасываются,
/// кроме соседей тех, что внутри, — линия уходит за край, а не обрывается на нём.
pub(super) fn near_map(pts: &[[f32; 2]], w: f32, h: f32, margin: f32) -> Vec<Vec<[f32; 2]>> {
    let inside = |p: [f32; 2]| p[0] >= -margin && p[1] >= -margin && p[0] <= w + margin && p[1] <= h + margin;
    let keep: Vec<bool> = (0..pts.len()).map(|k| inside(pts[k]) || (k > 0 && inside(pts[k - 1])) || (k + 1 < pts.len() && inside(pts[k + 1]))).collect();
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur = Vec::new();
    for (k, &p) in pts.iter().enumerate() {
        if keep[k] {
            if cur.last() != Some(&p) {
                cur.push(p);
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.retain(|l| l.len() >= 2);
    out
}

/// Сила точек маски внутри колец (чётно-нечётно — двор внутри дома остаётся
/// двором) с краем на шаг маски, как у овальной кисти: контур ляжет ровно по
/// линии, и стена под углом выйдет прямой. Только то, что на карте. `inset` —
/// на сколько клеток край отступает внутрь (середина озера без мелководья).
///
/// Внутри или нет — построчно по пересечениям строки с рёбрами; расстояние до
/// края — только до рёбер из соседних корзин в клетку: у озера и вереска рёбер
/// тысячи, а каждую точку со всеми не сравнить.
pub(super) fn coverage(mk: &Mask, rings: &[Vec<[f32; 2]>], inset: f32) -> Vec<(usize, f32)> {
    let res = mask::RES as f32;
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in rings.iter().flatten() {
        for c in 0..2 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let span = |lo: f32, hi: f32, n: usize| (((lo * res).floor() as isize - 1).clamp(0, n as isize) as usize, ((hi * res).ceil() as isize + 1).clamp(0, n as isize) as usize);
    let (i0, i1) = span(lo[0], hi[0], mk.w);
    let (j0, j1) = span(lo[1], hi[1], mk.h);
    let edges: Vec<([f32; 2], [f32; 2])> = rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).collect();
    let mut buckets: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (e, &(a, b)) in edges.iter().enumerate() {
        let n = (length(a, b) / 0.5).ceil().max(1.0) as usize;
        for s in 0..=n {
            let f = s as f32 / n as f32;
            let key = ((a[0] + (b[0] - a[0]) * f).floor() as i32, (a[1] + (b[1] - a[1]) * f).floor() as i32);
            let v = buckets.entry(key).or_default();
            if v.last() != Some(&e) {
                v.push(e);
            }
        }
    }
    let reach = (inset + 1.0).ceil() as i32;
    let aa = 0.5 / res;
    let mut out = Vec::new();
    let mut xs: Vec<f32> = Vec::new();
    for j in j0..j1 {
        let y = (j as f32 + 0.5) / res;
        xs.clear();
        xs.extend(edges.iter().filter(|(a, b)| (a[1] > y) != (b[1] > y)).map(|(a, b)| a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1])));
        xs.sort_by(f32::total_cmp);
        let mut left = 0;
        for i in i0..i1 {
            let x = (i as f32 + 0.5) / res;
            while left < xs.len() && xs[left] < x {
                left += 1;
            }
            let (cx, cy) = (x.floor() as i32, y.floor() as i32);
            let mut d = f32::INFINITY;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    for &e in buckets.get(&(cx + dx, cy + dy)).map_or(&[][..], |v| v.as_slice()) {
                        d = d.min(segment_distance([x, y], edges[e].0, edges[e].1));
                    }
                }
            }
            let s = if left % 2 == 1 { d - inset } else { -d - inset };
            let v = (0.5 + s / (2.0 * aa)).clamp(0.0, 1.0);
            if v > 0.0 {
                out.push((j * mk.w + i, v));
            }
        }
    }
    out
}

/// Плавный шум места от −1 до 1: ячейка в `scale` клеток, `seed` — какое из полей.
/// Зависит только от места, а не от порядка обхода.
pub(super) fn noise(p: [f32; 2], scale: f32, seed: u32) -> f32 {
    let hash = |x: i32, y: i32| {
        let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263) ^ seed.wrapping_mul(2_246_822_519);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
    };
    let (gx, gy) = (p[0] / scale, p[1] / scale);
    let (x0, y0) = (gx.floor() as i32, gy.floor() as i32);
    let (fx, fy) = (gx - x0 as f32, gy - y0 as f32);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(x0, y0) * (1.0 - sx) + hash(x0 + 1, y0) * sx;
    let bottom = hash(x0, y0 + 1) * (1.0 - sx) + hash(x0 + 1, y0 + 1) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Сдвиг природного контура в точке: плавное поле шума, около полуклетки. Поле
/// зависит только от места — общая граница леса и вереска сдвигается одинаково.
pub(super) fn warp(p: [f32; 2]) -> [f32; 2] {
    const SCALE: f32 = 5.0;
    const AMP: f32 = 0.7;
    [noise(p, SCALE, 1) * AMP, noise(p, SCALE, 2) * AMP]
}

/// Природный контур: рёбра дробятся на куски до двух клеток, точки сдвигаются
/// полем `warp`, углы срезаются дважды не дальше полутора клеток — берег и
/// опушка без прямых по линейке. Дробление от концов ребра одинаково в обе
/// стороны, поэтому общая граница соседей остаётся общей. Точки, где сходятся
/// несколько контуров (`pinned`), не срезаются: иначе у стыка двух кусков реки
/// каждый скругляет свой угол и на берегу остаётся выемка суши.
pub(super) fn natural_outline(r: &[[f32; 2]], pinned: &[bool]) -> Vec<[f32; 2]> {
    let mut pts = Vec::new();
    let mut pins = Vec::new();
    for k in 0..r.len() {
        let (a, b) = (r[k], r[(k + 1) % r.len()]);
        let n = (length(a, b) / 2.0).ceil().max(1.0) as usize;
        for s in 0..n {
            let f = s as f32 / n as f32;
            let p = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
            let d = warp(p);
            pts.push([p[0] + d[0], p[1] + d[1]]);
            pins.push(s == 0 && pinned.get(k).copied().unwrap_or(false));
        }
    }
    for _ in 0..2 {
        let n = pts.len();
        let (mut next, mut next_pins) = (Vec::with_capacity(n * 2), Vec::with_capacity(n * 2));
        for k in 0..n {
            let (u, v, w) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
            if pins[k] {
                next.push(v);
                next_pins.push(true);
                continue;
            }
            let toward = |q: [f32; 2]| {
                let t = (1.5 / length(v, q).max(1e-6)).min(0.25);
                [v[0] + (q[0] - v[0]) * t, v[1] + (q[1] - v[1]) * t]
            };
            next.extend([toward(u), toward(w)]);
            next_pins.extend([false, false]);
        }
        (pts, pins) = (next, next_pins);
    }
    pts
}

/// Линия с точками не реже `step` клеток. Дорога в Картографе тает к концам на
/// ширину края, и прямая из двух точек OSM таяла целиком — её не было видно.
pub(super) fn dense(line: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(line.len());
    for s in line.windows(2) {
        let n = (length(s[0], s[1]) / step).ceil().max(1.0) as usize;
        out.extend((0..n).map(|q| {
            let f = q as f32 / n as f32;
            [s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]
        }));
    }
    out.extend(line.last().copied());
    out
}

/// Середина взвешенных точек маски и их площадь в клетках.
pub(super) fn centre(mk: &Mask, weights: &[(usize, f32)]) -> ([f32; 2], f32) {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for &(k, v) in weights {
        let p = mask::sample_pos((k % mk.w) as isize, (k / mk.w) as isize);
        sx += p[0] * v;
        sy += p[1] * v;
        sw += v;
    }
    let cells = sw / (mask::RES * mask::RES) as f32;
    ([sx / sw.max(1e-6), sy / sw.max(1e-6)], cells)
}

pub(super) fn length(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Поворот подписи вдоль отрезка, чтобы читалась слева направо.
pub(super) fn upright(a: [f32; 2], b: [f32; 2]) -> f32 {
    let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees();
    if deg > 90.0 {
        deg - 180.0
    } else if deg <= -90.0 {
        deg + 180.0
    } else {
        deg
    }
}

/// Самый длинный отрезок ломаной.
pub(super) fn longest(pts: &[[f32; 2]]) -> Option<([f32; 2], [f32; 2])> {
    pts.windows(2).map(|w| (w[0], w[1])).max_by(|x, y| length(x.0, x.1).total_cmp(&length(y.0, y.1)))
}
