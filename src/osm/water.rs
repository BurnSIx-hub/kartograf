//! Пол площадей, вода (с отмелью и водой снимка), море по линии берега.

use super::*;

/// Что за пол у площадного объекта: текстура и порядок слоя (выше — поверх).
/// У леса своего пола нет: подстилка ложится пятнами под деревьями (`plant`).
/// Глубокая вода ложится с мелким краем (`WATER_EDGE`). Пляж без покрытия вдали
/// от моря (`sea` — в выгрузке есть берег моря) — галька: у реки песка почти не бывает.
pub(super) fn ground(t: &Tags, sea: bool) -> Option<(&'static str, u8)> {
    let (landuse, leisure, natural, amenity) = (get(t, "landuse"), get(t, "leisure"), get(t, "natural"), get(t, "amenity"));
    if matches!(leisure, "swimming_pool") || matches!(amenity, "fountain") {
        return Some(("water_shallow", 4));
    }
    if natural == "water" || matches!(landuse, "basin" | "reservoir") || get(t, "waterway") == "riverbank" {
        return Some(("water_deep", 4));
    }
    if natural == "wetland" {
        let bog = matches!(get(t, "wetland"), "bog" | "string_bog" | "palsa_bog");
        return Some((if bog { "bog_moss" } else { "swamp" }, 3));
    }
    // Ледник — лёд; морена на нём — по снимку (камень).
    if natural == "glacier" {
        return Some(("ice_glacier", 3));
    }
    // Пляж и скала — поверх воды: в OSM пляж лежит по обе стороны линии берега
    // (половина — приливная полоса), причал начинается от его внешнего края, и
    // море, закрывшее эту половину, оставляло причал висеть в воде.
    if matches!(natural, "bare_rock" | "scree" | "stone") {
        return Some((rock_kind(t).0, 5));
    }
    let surface = get(t, "surface");
    if natural == "shingle" || (natural == "beach" && (matches!(surface, "gravel" | "pebblestone" | "shingle" | "stone") || (surface.is_empty() && !sea))) {
        return Some(("shingle", 5));
    }
    if matches!(natural, "sand" | "beach" | "dune") {
        return Some(("sand", 5));
    }
    if matches!(landuse, "grass" | "meadow" | "village_green" | "recreation_ground" | "cemetery" | "allotments" | "flowerbed" | "orchard")
        || matches!(leisure, "park" | "garden" | "pitch" | "playground" | "common" | "dog_park")
        || natural == "grassland"
        || amenity == "grave_yard"
    {
        return Some(("meadow", 2));
    }
    if matches!(natural, "heath" | "scrub") || landuse == "farmland" {
        return Some(("rough_grass", 2));
    }
    if landuse == "farmyard" {
        return Some(("dirt", 2));
    }
    if landuse == "railway" {
        return Some(("gravel", 1));
    }
    if plaza(t) {
        // Мощение площади — по её `surface`: Красная площадь — брусчатка (`sett`),
        // а не плитка, по которой пешеходные линии ложились тёмными полосами.
        let hw = match get(t, "area:highway") {
            "" => "pedestrian",
            hw => hw,
        };
        return Some((paving(t, hw, false), 5));
    }
    if get(t, "man_made") == "pier" {
        return Some(("wood_planks", 6));
    }
    None
}

/// Порода голого камня по тегам `rock`, `geological`, `surface`: слот пола и тон
/// камней FA (`Boulder_Stone_Earthy`, `_Slate`…). Прежний `rocky_ground` брал любую
/// породу на карту, вплоть до красной. Без тега — светлая: на снимке освещённая
/// скала почти всегда светлее травы (мел Лулворта, известняк Малхема), а
/// «землистая» `C` на карте вышла тёмно-серой.
pub(super) fn rock_kind(t: &Tags) -> (&'static str, &'static str) {
    let v = format!("{};{};{}", get(t, "rock"), get(t, "geological"), get(t, "surface")).to_lowercase();
    let has = |w: &[&str]| w.iter().any(|w| v.contains(w));
    if has(&["chalk", "limestone", "marl", "dolomite", "marble"]) {
        ("rock_chalk", "earthy")
    } else if has(&["red_sandstone", "redrock", "red sandstone"]) {
        ("rock_redrock", "redrock")
    } else if has(&["sandstone", "gritstone"]) {
        ("rock_sandstone", "sandstone")
    } else if has(&["slate", "shale", "schist", "mudstone", "granite", "gneiss"]) {
        ("rock_slate", "slate")
    } else if has(&["basalt", "volcanic", "lava", "tuff", "obsidian", "andesite"]) {
        ("rock_volcanic", "volcanic")
    } else if has(&["earth", "conglomerate", "clay"]) {
        ("rock_earthy", "earthy")
    } else {
        ("rock_chalk", "earthy")
    }
}

/// Сколько клеток у берега глубокого водоёма мелководье.
pub(super) const WATER_EDGE: f32 = 2.0;

/// За сколько клеток от мелководья море набирает полную глубину.
pub(super) const SEA_SHELF: f32 = 24.0;

/// Шаг камней-переходов в клетках: человеку — шаг, фигурке — клетка.
pub(super) const STEPPING: f32 = 0.9;

/// Самое малое пятно воды снимка, что ложится водой, клеток²: меньше — тень
/// или лужа.
pub(super) const PHOTO_POND: f32 = 6.0;

/// Вода снимка по сетке маски, 0..1: доля воды в круге в три четверти клетки,
/// пятна меньше [`PHOTO_POND`] — нет.
pub(super) fn photo_water(m: &Map, hint: &Hint) -> Vec<f32> {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let res = mask::RES as f32;
    let mut v: Vec<f32> = (0..mw * mh)
        .map(|i| {
            let p = [((i % mw) as f32 + 0.5) / res, ((i / mw) as f32 + 0.5) / res];
            let (u, v, r) = (p[0] / hint.span[0], p[1] / hint.span[1], 0.75);
            ((hint.cover.wild_share(u, v, r / hint.span[0], r / hint.span[1]) - 0.35) / 0.3).clamp(0.0, 1.0)
        })
        .collect();
    // Пятна по связности (4 соседа) над половиной; малые — вон.
    let mut seen = vec![false; mw * mh];
    for s in 0..mw * mh {
        if seen[s] || v[s] < 0.5 {
            continue;
        }
        let mut blob = vec![s];
        seen[s] = true;
        let mut k = 0;
        while k < blob.len() {
            let i = blob[k];
            k += 1;
            let (x, y) = (i % mw, i / mw);
            for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                if nx < mw && ny < mh && !seen[ny * mw + nx] && v[ny * mw + nx] >= 0.5 {
                    seen[ny * mw + nx] = true;
                    blob.push(ny * mw + nx);
                }
            }
        }
        if (blob.len() as f32) < PHOTO_POND * res * res {
            for i in blob {
                v[i] = 0.0;
            }
        }
    }
    // Края у выкинутых пятен (ниже половины) — тоже вон: без ядра это рябь.
    let keep: Vec<bool> = (0..mw * mh).map(|i| v[i] >= 0.5).collect();
    let near = |i: usize| {
        let (x, y) = ((i % mw) as i64, (i / mw) as i64);
        (-2..=2).any(|dy| (-2..=2).any(|dx| {
            let (nx, ny) = (x + dx, y + dy);
            nx >= 0 && ny >= 0 && (nx as usize) < mw && (ny as usize) < mh && keep[ny as usize * mw + nx as usize]
        }))
    };
    (0..mw * mh).map(|i| if v[i] > 0.0 && (keep[i] || near(i)) { v[i] } else { 0.0 }).collect()
}

/// Озёра и реки одним слоем: мелководье по всей воде, глубина — дальше
/// `WATER_EDGE` от берега общей воды. Река из нескольких кусков в OSM иначе
/// получала мели поперёк русла на стыках; край карты берегом не считается —
/// вода уходит за него.
pub(super) fn water(m: &mut Map, floors: &mut Floors, parts: &[&Area], bog: bool, extra: &[f32]) {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    // Сумма, а не наибольшее: на общем крае двух кусков каждый даёт половину,
    // и по наибольшему вдоль шва оставалась полоса суши в полводы.
    let mut union = vec![0.0f32; mw * mh];
    for a in parts {
        for (i, v) in coverage(&m.rooms, &a.rings, 0.0) {
            union[i] = (union[i] + v).min(1.0);
        }
    }
    // Вода снимка, которой нет в OSM, — в ту же воду: общий берег и глубина.
    for (u, &v) in union.iter_mut().zip(extra) {
        *u = u.max(v);
    }
    let weights: Vec<(usize, f32)> = union.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| (i, *v)).collect();
    if weights.is_empty() {
        return;
    }
    // Вода FA полупрозрачна: под морем — песчаное дно, иначе мелководье у берега
    // бурое, как трава под водой, а на снимке там бирюза.
    let mut shelf = vec![false; mw * mh];
    let seas: Vec<&&Area> = parts.iter().filter(|a| get(a.tags, "water") == "sea").collect();
    if !seas.is_empty() {
        let sand = floors.get(m, "sand");
        for a in seas {
            let cov = coverage(&m.rooms, &a.rings, 0.0);
            for &(i, v) in &cov {
                shelf[i] |= v >= 0.5;
            }
            m.paint(Layer::Floor, sand, &cov);
        }
    }
    let shallow = floors.get(m, if bog { "bog_water_edge" } else { "water_shallow" });
    // У бочага берег — торфяной обрыв: мелкой полосы почти нет.
    let edge = if bog { 0.6 } else { WATER_EDGE };
    m.paint(Layer::Floor, shallow, &weights);
    // Расстояние до суши по сетке маски, фаской 3–4 (3 — шаг по прямой).
    let far = u32::MAX / 2;
    let mut dist: Vec<u32> = union.iter().map(|&v| if v >= 0.5 { far } else { 0 }).collect();
    let step = |dist: &mut Vec<u32>, i: usize, j: usize, di: isize, dj: isize, cost: u32| {
        let (ni, nj) = (i as isize + di, j as isize + dj);
        if ni >= 0 && nj >= 0 && (ni as usize) < mw && (nj as usize) < mh {
            let n = dist[nj as usize * mw + ni as usize] + cost;
            if n < dist[j * mw + i] {
                dist[j * mw + i] = n;
            }
        }
    };
    for j in 0..mh {
        for i in 0..mw {
            if dist[j * mw + i] > 0 {
                for (di, dj, c) in [(-1, 0, 3), (0, -1, 3), (-1, -1, 4), (1, -1, 4)] {
                    step(&mut dist, i, j, di, dj, c);
                }
            }
        }
    }
    for j in (0..mh).rev() {
        for i in (0..mw).rev() {
            if dist[j * mw + i] > 0 {
                for (di, dj, c) in [(1, 0, 3), (0, 1, 3), (1, 1, 4), (-1, 1, 4)] {
                    step(&mut dist, i, j, di, dj, c);
                }
            }
        }
    }
    let res = mask::RES as f32;
    let aa = 0.5 / res;
    // У моря отмель широкая и глубина набирается плавно — на снимке бирюза у
    // берега переходит в синеву десятками метров; у озера и реки край резкий.
    let deep: Vec<(usize, f32)> = dist
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 0)
        .map(|(i, &d)| {
            let cells = d.min(far) as f32 / 3.0 / res;
            let v = if shelf[i] { ((cells - edge) / SEA_SHELF).clamp(0.0, 1.0).powf(0.8) } else { 0.5 + (cells - edge) / (2.0 * aa) };
            (i, v.clamp(0.0, 1.0))
        })
        .filter(|(_, v)| *v > 0.0)
        .collect();
    // Глубина моря — бирюзой потемнее, озёр и рек — синей.
    let (sea, inland): (Vec<(usize, f32)>, Vec<(usize, f32)>) = deep.into_iter().partition(|(i, _)| shelf[*i]);
    for (slot, part) in [("sea_deep", sea), (if bog { "bog_water" } else { "water_deep" }, inland)] {
        if !part.is_empty() {
            let t = floors.get(m, slot);
            m.paint(Layer::Floor, t, &part);
        }
    }
}

/// Запас рамки моря за краем карты в клетках: изгиб природного контура рамку
/// внутрь карты не заводит.
pub(super) const SEA_FRAME: f32 = 3.0;

/// Море — кольца в клетках. Площадью моря в OSM нет: есть линия берега
/// `natural=coastline`, суша слева по ходу, вода справа (на карте, где y вниз, —
/// слева по формуле). Линия режется рамкой карты с запасом, а море замыкается
/// вдоль рамки по часовой стрелке на экране до следующего входа линии в рамку.
/// Острова целиком в рамке — дыры (кольца чётно-нечётно). Конец линии внутри
/// рамки — выгрузка обрезала путь — продлевается по ходу до рамки. Линии в
/// рамке нет — море или суша по стороне ближайшего отрезка от середины карты.
pub(super) fn sea_rings(osm: &Osm, proj: &Proj, w: f32, h: f32) -> Vec<Vec<[f32; 2]>> {
    let coast: Vec<&Way> = osm.ways.values().filter(|w| get(&w.tags, "natural") == "coastline" && w.nodes.len() >= 2).collect();
    if coast.is_empty() {
        return Vec::new();
    }
    // Цепочки: конец пути — начало следующего; сначала от голов, потом кольца.
    let starts: HashMap<i64, usize> = coast.iter().enumerate().map(|(k, w)| (w.nodes[0], k)).collect();
    let ends: BTreeSet<i64> = coast.iter().filter_map(|w| w.nodes.last().copied()).collect();
    let mut used = vec![false; coast.len()];
    let mut chains: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    let heads: Vec<usize> = (0..coast.len()).filter(|&k| !ends.contains(&coast[k].nodes[0])).chain(0..coast.len()).collect();
    for k0 in heads {
        if used[k0] {
            continue;
        }
        used[k0] = true;
        let mut ids = coast[k0].nodes.clone();
        let closed = loop {
            let end = ids[ids.len() - 1];
            if end == ids[0] {
                break true;
            }
            match starts.get(&end) {
                Some(&k) if !used[k] => {
                    used[k] = true;
                    ids.extend_from_slice(&coast[k].nodes[1..]);
                }
                _ => break false,
            }
        };
        let mut pts: Vec<[f32; 2]> = ids.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
        pts.dedup();
        if closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        if pts.len() >= 2 {
            chains.push((pts, closed));
        }
    }
    let m = SEA_FRAME;
    let (x0, y0, x1, y1) = (-m, -m, w + m, h + m);
    let in_frame = |p: [f32; 2]| p[0] > x0 && p[0] < x1 && p[1] > y0 && p[1] < y1;
    let lerp = |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
    // Отрезок против рамки (Лян — Барски): доли входа и выхода.
    let clip = |a: [f32; 2], b: [f32; 2]| -> Option<(f32, f32)> {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, q) in [(-dx, a[0] - x0), (dx, x1 - a[0]), (-dy, a[1] - y0), (dy, y1 - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    return None;
                }
                continue;
            }
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
        (t0 < t1).then_some((t0, t1))
    };
    // Луч из точки внутри рамки до рамки.
    let ray = |p: [f32; 2], d: [f32; 2]| {
        let mut t = f32::INFINITY;
        for (c, lo, hi) in [(0, x0, x1), (1, y0, y1)] {
            if d[c] > 1e-9 {
                t = t.min((hi - p[c]) / d[c]);
            } else if d[c] < -1e-9 {
                t = t.min((lo - p[c]) / d[c]);
            }
        }
        if t.is_finite() { [p[0] + d[0] * t, p[1] + d[1] * t] } else { [x0, p[1]] }
    };
    let mut pieces: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut islands: Vec<Vec<[f32; 2]>> = Vec::new();
    for (pts, closed) in &chains {
        let pts = if *closed {
            // Кольцо — с точки за рамкой, чтобы кусок в рамке не рвался на старте.
            match pts.iter().position(|&p| !in_frame(p)) {
                None => {
                    islands.push(pts.clone());
                    continue;
                }
                Some(k) => pts[k..].iter().chain(&pts[..=k]).copied().collect::<Vec<_>>(),
            }
        } else {
            pts.clone()
        };
        let mut cur: Option<Vec<[f32; 2]>> = in_frame(pts[0]).then(|| vec![ray(pts[0], [pts[0][0] - pts[1][0], pts[0][1] - pts[1][1]]), pts[0]]);
        for s in pts.windows(2) {
            let Some((t0, t1)) = clip(s[0], s[1]) else { continue };
            let c = cur.get_or_insert_with(|| vec![lerp(s[0], s[1], t0)]);
            if t1 < 1.0 {
                c.push(lerp(s[0], s[1], t1));
                pieces.extend(cur.take());
            } else {
                c.push(s[1]);
            }
        }
        if let Some(mut c) = cur {
            let n = c.len();
            c.push(ray(c[n - 1], [c[n - 1][0] - c[n - 2][0], c[n - 1][1] - c[n - 2][1]]));
            pieces.push(c);
        }
    }
    // Место на рамке: путь по часовой стрелке от левого верхнего угла.
    let (fw, fh) = (x1 - x0, y1 - y0);
    let per = 2.0 * (fw + fh);
    let at = |p: [f32; 2]| {
        let d = [(p[1] - y0).abs(), (x1 - p[0]).abs(), (y1 - p[1]).abs(), (p[0] - x0).abs()];
        match (0..4).min_by(|&i, &j| d[i].total_cmp(&d[j])).unwrap_or(0) {
            0 => (p[0] - x0).clamp(0.0, fw),
            1 => fw + (p[1] - y0).clamp(0.0, fh),
            2 => fw + fh + (x1 - p[0]).clamp(0.0, fw),
            _ => 2.0 * fw + fh + (y1 - p[1]).clamp(0.0, fh),
        }
    };
    let corners = [(fw, [x1, y0]), (fw + fh, [x1, y1]), (2.0 * fw + fh, [x0, y1]), (per, [x0, y0])];
    let mut rings = Vec::new();
    if !pieces.is_empty() {
        let entries: Vec<f32> = pieces.iter().map(|p| at(p[0])).collect();
        let exits: Vec<f32> = pieces.iter().map(|p| at(p[p.len() - 1])).collect();
        let mut done = vec![false; pieces.len()];
        for first in 0..pieces.len() {
            if done[first] {
                continue;
            }
            let mut ring = Vec::new();
            let mut k = first;
            for _ in 0..pieces.len() {
                done[k] = true;
                ring.extend_from_slice(&pieces[k]);
                let from = exits[k];
                let ahead = |i: usize| (entries[i] - from).rem_euclid(per);
                let next = (0..pieces.len()).min_by(|&i, &j| ahead(i).total_cmp(&ahead(j))).unwrap_or(first);
                // Углы рамки между выходом и следующим входом.
                let to = from + ahead(next);
                for lap in [0.0, per] {
                    for &(s, c) in &corners {
                        if s + lap > from && s + lap < to {
                            ring.push(c);
                        }
                    }
                }
                if next == first || done[next] {
                    break;
                }
                k = next;
            }
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
    } else {
        // Ни один кусок не входит в рамку: вся карта — море, если середина со
        // стороны воды (острова — дыры), иначе суша.
        let c = [w / 2.0, h / 2.0];
        let (mut best, mut side) = (f32::INFINITY, 0.0f32);
        for (pts, closed) in &chains {
            let n = pts.len();
            for k in 0..if *closed { n } else { n - 1 } {
                let (a, b) = (pts[k], pts[(k + 1) % n]);
                let d = segment_distance(c, a, b);
                let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                let len = length(a, b).max(1e-6);
                // У общей вершины двух отрезков решает тот, от чьей прямой точка дальше.
                if d < best - 1e-4 || (d < best + 1e-4 && (cross / len).abs() > side.abs()) {
                    best = best.min(d);
                    side = cross / len;
                }
            }
        }
        if side > 0.0 {
            rings.push(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
        }
    }
    if !islands.is_empty() && rings.is_empty() {
        rings.push(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
    }
    rings.extend(islands);
    rings
}
