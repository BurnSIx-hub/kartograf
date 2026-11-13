//! Цвет и свет крыш по тегам и снимку: солнце, наклон снимка.

use super::*;

/// Цвет из тега OSM: `#rgb`, `#rrggbb` или ходовое в OSM имя CSS.
pub(super) fn colour(v: &str) -> Option<[u8; 3]> {
    let v = v.trim().to_lowercase();
    if let Some(hex) = v.strip_prefix('#') {
        let d: Vec<u8> = hex.chars().filter_map(|c| c.to_digit(16).map(|d| d as u8)).collect();
        return match (hex.len(), d.len()) {
            (3, 3) => Some([d[0] * 17, d[1] * 17, d[2] * 17]),
            (6, 6) => Some([d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5]]),
            _ => None,
        };
    }
    Some(match v.as_str() {
        "white" => [255, 255, 255],
        "ivory" => [255, 255, 240],
        "beige" => [245, 245, 220],
        "cream" => [255, 253, 208],
        "yellow" => [255, 255, 0],
        "gold" => [255, 215, 0],
        "orange" => [255, 165, 0],
        "tan" => [210, 180, 140],
        "brown" => [165, 42, 42],
        "sienna" => [160, 82, 45],
        "maroon" => [128, 0, 0],
        "darkred" => [139, 0, 0],
        "red" => [255, 0, 0],
        "firebrick" => [178, 34, 34],
        "indianred" => [205, 92, 92],
        "salmon" => [250, 128, 114],
        "darksalmon" => [233, 150, 122],
        "pink" => [255, 192, 203],
        "grey" | "gray" => [128, 128, 128],
        // В CSS «тёмно-серый» светлее серого (169); в OSM им метят тёмную
        // кровлю — Средние торговые ряды выходили серебряными.
        "darkgrey" | "darkgray" => [96, 96, 96],
        "dimgrey" | "dimgray" => [105, 105, 105],
        "slategrey" | "slategray" => [112, 128, 144],
        "darkslategrey" | "darkslategray" => [47, 79, 79],
        "lightgrey" | "lightgray" => [211, 211, 211],
        "silver" => [192, 192, 192],
        "black" => [0, 0, 0],
        "green" => [0, 128, 0],
        "darkgreen" => [0, 100, 0],
        "lightgreen" => [144, 238, 144],
        "olive" => [128, 128, 0],
        "blue" => [0, 0, 255],
        "lightblue" => [173, 216, 230],
        "navy" => [0, 0, 128],
        "teal" => [0, 128, 128],
        "seagreen" => [46, 139, 87],
        "darkseagreen" => [143, 188, 143],
        "forestgreen" => [34, 139, 34],
        "darkolivegreen" => [85, 107, 47],
        "olivedrab" => [107, 142, 35],
        "steelblue" => [70, 130, 180],
        "skyblue" => [135, 206, 235],
        "chocolate" => [210, 105, 30],
        "saddlebrown" => [139, 69, 19],
        "peru" => [205, 133, 63],
        "rosybrown" => [188, 143, 143],
        "burlywood" => [222, 184, 135],
        "khaki" => [240, 230, 140],
        "wheat" => [245, 222, 179],
        "coral" => [255, 127, 80],
        "tomato" => [255, 99, 71],
        "crimson" => [220, 20, 60],
        _ => return None,
    })
}

/// Цвет крыши из тега OSM — приглушённый: там берут имена CSS, и `lightgreen`
/// выходил кислотным, а `black` — дырой без рисунка покрытия. Имя — грубая
/// категория: `green` на снимке — тусклая крашеная жесть, а чистый CSS-зелёный
/// кислотный (дом на Ильинке); его глушим сильнее. Код `#617666` маппер подбирал
/// сам — ему верим больше. Золото куполов и на снимке яркое.
pub(super) fn tag_roof(v: &str) -> Option<[u8; 3]> {
    let c = colour(v)?;
    let v = v.trim().to_ascii_lowercase();
    Some(if v.starts_with('#') || v.contains("gold") { roof_rgb(c) } else { muted(c, 0.3, 0.3) })
}

pub(super) fn roof_rgb(c: [u8; 3]) -> [u8; 3] {
    muted(c, 0.6, 0.12)
}

/// Насыщенность не выше `smax`, светлота от `lmin` до 0,7 (HSL).
pub(super) fn muted(c: [u8; 3], smax: f32, lmin: f32) -> [u8; 3] {
    let [r, g, b] = c.map(|v| v as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((mx + mn) / 2.0, mx - mn);
    let s = if d < 1e-6 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6) };
    let (s2, l2) = (s.min(smax), l.clamp(lmin, 0.7));
    if s2 == s && l2 == l {
        return c;
    }
    let h = if d < 1e-6 {
        0.0
    } else if mx == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    let k = (1.0 - (2.0 * l2 - 1.0).abs()) * s2;
    let x = k * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h as u32 {
        0 => (k, x, 0.0),
        1 => (x, k, 0.0),
        2 => (0.0, k, x),
        3 => (0.0, x, k),
        4 => (x, 0.0, k),
        _ => (k, 0.0, x),
    };
    let m = l2 - k / 2.0;
    [r1, g1, b1].map(|v| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// Яркость цвета, 0..255.
pub(super) fn luma(c: [u8; 3]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

/// Куда смотрит скат по `roof:direction` (градусы от севера по часовой или румб
/// `N`, `NE`…): вниз по скату, в клетках карты (x — восток, y — юг).
pub(super) fn roof_down(v: &str) -> Option<[f32; 2]> {
    const RUMB: [&str; 16] = ["N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW", "NW", "NNW"];
    let v = v.trim();
    let deg = v.parse::<f32>().ok().or_else(|| RUMB.iter().position(|r| r.eq_ignore_ascii_case(v)).map(|i| i as f32 * 22.5))?;
    let a = deg.to_radians();
    Some([a.sin(), -a.cos()])
}

/// Цвет крыши со снимка, если ему можно верить: кусок `area` м² с верхом на
/// `height` м. Снимок не строго сверху — крыша высокого дома сдвинута от
/// контура (шатёр Спасской башни — метров на пятнадцать), и узкий высокий кусок
/// брал бы по контуру землю и тень рядом. Совсем тёмное (яркость меньше 40 —
/// Царская башня под кронами) — тень, а не кровля. Темнее цвета из тега снимок
/// почти всегда: гранит Мавзолея в 2,3 раза, тёмно-зелёная жесть Средних рядов в
/// 2,1 — по отношению тень от кровли не отличить. С найденным наклоном снимка
/// ([`photo_lean`]) цвет берётся там, куда крыша сдвинута; мерка доверия та же:
/// ошибка высоты главки (этажи по 3 м, высота купола на глаз) больше её самой —
/// с мягкой меркой золотая главка у Зарядья по снимку выходила серой.
pub(super) fn photo_roof(hint: Option<&Hint>, samples: &[(usize, f32)], mw: usize, area: f32, height: f32, cell_m: f32, sunlit: bool) -> Option<[u8; 3]> {
    let hint = hint?;
    if area.max(0.0).sqrt() < 0.5 * height {
        return None;
    }
    hint.colour(samples, mw, hint.shift(height / cell_m), sunlit).filter(|&c| luma(c) >= 40.0)
}

/// Наклон снимка, клеток сдвига на клетку высоты (x — восток, y — юг). Снимок
/// снят не строго сверху: крыша дома высотой `h` лежит на нём со сдвигом
/// `наклон × h` от контура (Китай-город — 0,29 м на метр к западу-юго-западу).
/// Наклон свой у каждого снимка, а на рамке карты — один. Ищется перебором: контуры `pieces` (кольца и высота в клетках,
/// от трёх этажей) при верном наклоне ложатся на края крыш, и перепад яркости
/// поперёк контура наибольший. Пик узкий — у дома в 20 клеток шаг наклона 0,1
/// уводит контур на две клетки, — поэтому три прохода: сперва щуп поперёк
/// контура широкий и шаг крупный, потом всё уже. Меньше пяти домов, мало точек
/// или выигрыш против «строго сверху» меньше 20% — не найден: у Красной площади
/// он 11%, пик пологий, и два направления в полсотни градусов врозь дают один
/// счёт (у Китай-города — 53%).
pub(super) fn photo_lean(hint: &Hint, pieces: &[(&[Vec<[f32; 2]>], f32)]) -> Option<[f32; 2]> {
    const STEP: f32 = 0.25;
    // Точки контуров: место, наружу, высота.
    let mut pts: Vec<([f32; 2], [f32; 2], f32)> = Vec::new();
    let mut used = 0;
    for &(rings, h) in pieces {
        let before = pts.len();
        for (a, b, n) in outward_edges(rings) {
            let count = (length(a, b) / STEP).ceil() as usize;
            for i in 0..count {
                let t = (i as f32 + 0.5) / count as f32;
                let p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                // Выгрузка шире рамки: контуры за снимком не в счёт.
                if p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= hint.span[0] && p[1] <= hint.span[1] {
                    pts.push((p, n, h));
                }
            }
        }
        if pts.len() > before {
            used += 1;
        }
    }
    if used < 5 || pts.len() < 400 {
        return None;
    }
    let score = |l: [f32; 2], across: f32| {
        let (mut sum, mut n) = (0.0f64, 0u32);
        for &(p, d, h) in &pts {
            let q = [p[0] + l[0] * h, p[1] + l[1] * h];
            if let (Some(o), Some(i)) = (hint.luma_at([q[0] + d[0] * across, q[1] + d[1] * across]), hint.luma_at([q[0] - d[0] * across, q[1] - d[1] * across])) {
                sum += (o - i).abs() as f64;
                n += 1;
            }
        }
        (n as usize * 2 >= pts.len()).then(|| (sum / n as f64) as f32)
    };
    let best_of = |c: [f32; 2], span: f32, step: f32, across: f32| {
        let k = (span / step).round() as i32;
        let mut best: Option<([f32; 2], f32)> = None;
        for i in -k..=k {
            for j in -k..=k {
                let l = [c[0] + i as f32 * step, c[1] + j as f32 * step];
                if let Some(s) = score(l, across)
                    && best.is_none_or(|b| s > b.1)
                {
                    best = Some((l, s));
                }
            }
        }
        best
    };
    let (coarse, _) = best_of([0.0, 0.0], 1.0, 0.1, 1.5)?;
    let (middle, _) = best_of(coarse, 0.1, 0.025, 0.6)?;
    let (lean, top) = best_of(middle, 0.03, 0.01, 0.3)?;
    let flat = score([0.0, 0.0], 0.3)?;
    (top >= flat * 1.2 && top >= flat + 2.0).then_some(lean)
}

/// Стороны колец (концы и единичная нормаль наружу — от того, что внутри колец).
pub(super) fn outward_edges(rings: &[Vec<[f32; 2]>]) -> Vec<([f32; 2], [f32; 2], [f32; 2])> {
    let mut edges = Vec::new();
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            let len = length(a, b);
            if len < 1e-3 {
                continue;
            }
            let mut n = [(b[1] - a[1]) / len, -(b[0] - a[0]) / len];
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            if inside(rings, [mid[0] + n[0] * 0.05, mid[1] + n[1] * 0.05]) {
                n = [-n[0], -n[1]];
            }
            edges.push((a, b, n));
        }
    }
    edges
}

/// Свет крыши на снимке, будь она вальмой по контуру: яркость точки, под которой
/// скат смотрит в сторону `n` (наружу от ближайшей стороны контура), — примерно
/// `mean + gain·n`. У скатной крыши скат к солнцу светлее, и `gain` смотрит на
/// солнце; у плоской он около нуля (тень парапета даже темнит край к солнцу).
#[derive(Clone, Copy, Debug)]
pub(super) struct Light {
    pub(super) gain: [f32; 2],
    pub(super) mean: f32,
    /// Ковариация `gain` (соседние точки снимка не независимы — она занижена).
    pub(super) cov: [[f32; 2]; 2],
    /// То же по одним перепадам на стыках скатов (конёк, рёбра): пары точек по
    /// разные стороны стыка, `Δяркость ≈ step·Δn`. Раскладка плоской кровли
    /// (светлая полоса фонарей у одного края, тёмная у другого — Ипатьевский
    /// переулок, 9 с1) даёт `gain`, но не перепад на стыке.
    pub(super) step: [f32; 2],
    pub(super) step_cov: [[f32; 2]; 2],
}

/// [`Light`] крыши по контуру `rings` и сэмплам маски; `shift` — сдвиг крыши на
/// снимке, клетки. Кромка — метр от стены — не в счёт: туда при ошибке высоты
/// попадает фасад или земля. Скаты — как у отрисовки: по ближайшей стороне
/// упрощённого контура. Меньше 40 точек — не судить.
pub(super) fn roof_light(hint: &Hint, rings: &[Vec<[f32; 2]>], samples: &[(usize, f32)], mw: usize, shift: [f32; 2], cell_m: f32) -> Option<Light> {
    let rings: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| crate::roofs::iron_jogs(&crate::roofs::simplify_ring(r, crate::roofs::OUTLINE_TOL))).filter(|r| r.len() >= 3).collect();
    let edges = outward_edges(&rings);
    let res = mask::RES as f32;
    let rim = 1.0 / cell_m;
    let (mut xtx, mut xty, mut yy, mut count) = ([[0.0f64; 3]; 3], [0.0f64; 3], 0.0f64, 0usize);
    // Сэмпл маски → скат, куда он смотрит, яркость.
    let mut seen: HashMap<usize, (usize, [f32; 2], f32)> = HashMap::new();
    for &(k, v) in samples {
        if v < 0.5 {
            continue;
        }
        let p = [((k % mw) as f32 + 0.5) / res, ((k / mw) as f32 + 0.5) / res];
        let Some((d, n, e)) = edges.iter().enumerate().map(|(i, e)| (segment_distance(p, e.0, e.1), e.2, i)).min_by(|x, y| x.0.total_cmp(&y.0)) else { continue };
        if d < rim {
            continue;
        }
        let Some(y) = hint.luma_at([p[0] + shift[0], p[1] + shift[1]]) else { continue };
        seen.insert(k, (e, n, y));
        let x = [1.0, n[0] as f64, n[1] as f64];
        for i in 0..3 {
            for j in 0..3 {
                xtx[i][j] += x[i] * x[j];
            }
            xty[i] += x[i] * y as f64;
        }
        yy += (y as f64).powi(2);
        count += 1;
    }
    if count < 40 {
        return None;
    }
    let inv = invert3(xtx)?;
    let beta: [f64; 3] = std::array::from_fn(|i| (0..3).map(|j| inv[i][j] * xty[j]).sum());
    let rss = (yy - (0..3).map(|i| beta[i] * xty[i]).sum::<f64>()).max(0.0);
    let s2 = rss / (count - 3) as f64;
    // Перепады на стыках: пары точек через две клетки (по полтора метра от
    // стыка) — промах наклона на метр их не путает, а через полклетки обе точки
    // ложились на один настоящий скат.
    let gap = 2 * mask::RES;
    let (mut a, mut v, mut bb, mut pairs) = ([[0.0f64; 2]; 2], [0.0f64; 2], 0.0f64, 0usize);
    for (&k, &(e, n, y)) in &seen {
        for q in [k + gap, k + gap * mw] {
            let Some(&(f, m, z)) = seen.get(&q) else { continue };
            if f == e || (q == k + gap && q / mw != k / mw) {
                continue;
            }
            let (dn, db) = ([(n[0] - m[0]) as f64, (n[1] - m[1]) as f64], (y - z) as f64);
            for i in 0..2 {
                for j in 0..2 {
                    a[i][j] += dn[i] * dn[j];
                }
                v[i] += dn[i] * db;
            }
            bb += db * db;
            pairs += 1;
        }
    }
    let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let (step, step_cov) = if pairs >= 10 && det.abs() > 1e-6 {
        let ai = [[a[1][1] / det, -a[0][1] / det], [-a[1][0] / det, a[0][0] / det]];
        let g = [ai[0][0] * v[0] + ai[0][1] * v[1], ai[1][0] * v[0] + ai[1][1] * v[1]];
        let s2 = ((bb - g[0] * v[0] - g[1] * v[1]) / (pairs - 2) as f64).max(0.0);
        ([g[0] as f32, g[1] as f32], ai.map(|r| r.map(|x| (x * s2) as f32)))
    } else {
        ([0.0, 0.0], [[f32::INFINITY, 0.0], [0.0, f32::INFINITY]])
    };
    Some(Light {
        gain: [beta[1] as f32, beta[2] as f32],
        mean: (xty[0] / count as f64) as f32,
        cov: [[(s2 * inv[1][1]) as f32, (s2 * inv[1][2]) as f32], [(s2 * inv[2][1]) as f32, (s2 * inv[2][2]) as f32]],
        step,
        step_cov,
    })
}

/// Обратная 3×3; вырожденная (все скаты в одну сторону) — нет.
pub(super) fn invert3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let c = |i: usize, j: usize| {
        let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
        let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    (det.abs() > 1e-9 * m[0][0].powi(3).max(1.0)).then(|| std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det)))
}

/// Откуда солнце на снимке: куда в среднем смотрит `gain` ([`Light`]) крыш, что в
/// OSM скатные. Меньше трёх таких крыш или они светятся вразнобой — не понять.
pub(super) fn photo_sun(lights: &[Light]) -> Option<[f32; 2]> {
    let units: Vec<[f32; 2]> = lights
        .iter()
        .filter_map(|l| {
            let g = (l.gain[0].powi(2) + l.gain[1].powi(2)).sqrt();
            (g > 0.0).then(|| [l.gain[0] / g, l.gain[1] / g])
        })
        .collect();
    if units.len() < 3 {
        return None;
    }
    let sum = units.iter().fold([0.0, 0.0], |s, u| [s[0] + u[0], s[1] + u[1]]);
    let r = (sum[0].powi(2) + sum[1].powi(2)).sqrt();
    (r >= 0.4 * units.len() as f32).then(|| [sum[0] / r, sum[1] / r])
}

/// Видно ли на снимке, что крыша скатная ([`Light`] при солнце `sun`): скат к
/// солнцу светлее противоположного не меньше чем на 8% средней яркости, и
/// перепад есть на самих стыках скатов — не меньше 6%; оба — втрое больше своей
/// ошибки. Обратное не верно: светлая кровля на снимке засвечена, скаты вдоль
/// солнца светятся одинаково — и без перепада крыша бывает скатной (половина
/// скатных крыш Китай-города перепада не дала).
pub(super) fn pitched_by_light(l: &Light, sun: [f32; 2]) -> bool {
    let mean = l.mean.max(1.0);
    let along = |g: [f32; 2], c: [[f32; 2]; 2]| {
        let var = sun[0] * sun[0] * c[0][0] + 2.0 * sun[0] * sun[1] * c[0][1] + sun[1] * sun[1] * c[1][1];
        ((g[0] * sun[0] + g[1] * sun[1]) / mean, var.max(0.0).sqrt() / mean)
    };
    let (k, se) = along(l.gain, l.cov);
    let (s, sse) = along(l.step, l.step_cov);
    k >= 0.08 && s >= 0.06 && k >= 3.0 * se && s >= 3.0 * sse
}
