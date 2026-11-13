//! Дома: вид и высота крыши, тени домов, крыши по контуру, стены-крепости.

use super::*;

/// Год постройки по `start_date` (`1966`, `1966-05-01`, `~1966`, `1960s`); век
/// (`C19`) и прочее — нет.
pub(super) fn built(t: &Tags) -> Option<i32> {
    let digits: String = get(t, "start_date").chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() == 4 { digits.parse().ok() } else { None }
}

/// Низ ската, м ([`RoofPiece::eaves`]): верх `height` без высоты крыши
/// (`roof:height`, нет — до `min_height`, нет — 3 м); у плоской — верх.
pub(super) fn eaves_of(t: &Tags, form: RoofForm, height: f32) -> f32 {
    if form == RoofForm::Flat {
        return height;
    }
    let num = |k: &str| get(t, k).split_whitespace().next().and_then(|v| v.parse::<f32>().ok());
    let roof = num("roof:height").or_else(|| num("min_height").filter(|&m| m < height).map(|m| height - m)).unwrap_or(3.0);
    (height - roof).max(0.0)
}

/// Площадь кольца, клетки².
pub(super) fn ring_area(r: &[[f32; 2]]) -> f32 {
    (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum::<f32>().abs() / 2.0
}

/// Вид крыши по `roof:shape` (и плоские покрытия по `roof:material`); пусто — `default`.
/// `round` — свод вдоль дома (пассажи ГУМа), а не купол.
pub(super) fn roof_form(shape: &str, material: &str, default: RoofForm) -> RoofForm {
    match shape {
        "flat" => RoofForm::Flat,
        "" if matches!(material, "tar_paper" | "concrete" | "asphalt" | "gravel") => RoofForm::Flat,
        "" => default,
        "pyramidal" | "tent" => RoofForm::Spire,
        "dome" | "onion" | "spherical" | "cone" | "conical" => RoofForm::Dome,
        "skillion" | "lean_to" => RoofForm::Shed,
        "gabled" | "gambrel" | "saltbox" => RoofForm::Gabled,
        "round" | "barrel" => RoofForm::Round,
        // Пила — ряды односкатных; сверху читается теми же полосами.
        "sawtooth" => RoofForm::Rows,
        _ => RoofForm::Pitched,
    }
}

/// Зал от этой площади, м², крыт рядами скатов ([`hall`]).
pub(super) const HALL_MIN_M2: f32 = 800.0;

/// Рынок, вокзал, склад, цех, ангар: большой пролёт крыт не одной вальмой, а
/// рядами скатов. Вальмой с плоским верхом Центральный рынок Валенсии выходил
/// бурым полем с каймой, а на снимке у него полосы коньков.
pub(super) fn hall(t: &Tags) -> bool {
    matches!(get(t, "building"), "marketplace" | "train_station" | "transportation" | "warehouse" | "industrial" | "factory" | "manufacture" | "hangar")
        || get(t, "amenity") == "marketplace"
        || get(t, "railway") == "station"
        || get(t, "public_transport") == "station"
}

/// Покрытие крыши по `roof:material`; без него — в городе жесть, за городом черепица.
pub(super) fn roof_slot(material: &str, rural: bool) -> &'static str {
    match material {
        "roof_tiles" | "tile" | "tiles" | "clay" | "ceramic" => "roof_clay_red",
        "slate" => "roof_slate_gray",
        "copper" => "roof_copper",
        "thatch" | "reed" | "straw" => "roof_thatch",
        "wood" | "wooden" | "shingle" | "shingles" => "roof_shingle",
        "stone" | "marble" | "granite" | "sandstone" => "flagstones",
        "metal" | "tin" | "steel" | "zinc" | "metal_sheet" | "glass" | "concrete" | "tar_paper" | "asphalt" => "roof_metal",
        _ if rural => "roof_clay_red",
        _ => "roof_metal",
    }
}

/// Высота части, м: `height`, нет — этажи по 3 м.
pub(super) fn part_height(t: &Tags) -> f32 {
    let num = |k: &str| get(t, k).split_whitespace().next().and_then(|v| v.parse::<f32>().ok());
    num("height").or_else(|| num("building:levels").map(|l| l * 3.0)).unwrap_or(0.0)
}

/// Насколько темна тень дома на земле: доля чёрного.
pub(super) const HOUSE_SHADE: f32 = 0.42;
/// На сколько клеток высоты луча над землёй тень тает к концу.
pub(super) const HOUSE_SHADE_SOFT: f32 = 0.6;
/// Тень дальше этого не считается, клеток: башня в 100 м легла бы на полкарты.
pub(super) const HOUSE_SHADE_REACH: f32 = 40.0;

/// Тело, что бросает тень: кольца в клетках, высота в клетках, навес ли.
pub(super) type Solid<'a> = (&'a [Vec<[f32; 2]>], f32, bool);

/// Тени домов на земле — в нарисованную тень карты ([`Map::shade`]): Мастер
/// сотрёт лишнее той же кистью. Солнце — то же, что у теней стен и ярусов крыш:
/// с той стороны, что [`crate::map::Shadows::light`], на высоте
/// [`crate::roofs::SUN_RISE`]; длина тени — по высоте дома и его частей (нет
/// высоты — 6 м, как у крыши). Дома за краем карты тоже бросают тень на неё.
/// В доме тени нет — там пол под крышей, а под навесом — тень целиком.
pub(super) fn house_shadows(m: &mut Map, areas: &[Area], cell_m: f32) {
    if !m.shadows.on || cell_m <= 0.0 {
        return;
    }
    let solids: Vec<Solid> = areas
        .iter()
        .filter_map(|a| {
            let canopy = get(a.tags, "building") == "roof";
            let part = !matches!(get(a.tags, "building:part"), "" | "no");
            if !(canopy || part || building(a.tags).is_some()) {
                return None;
            }
            let h = match part_height(a.tags) {
                h if h > 0.0 => h,
                _ if canopy => 4.0,
                _ => 6.0,
            };
            Some((&a.rings[..], h / cell_m, canopy))
        })
        .collect();
    let shade = shadow_mask(&solids, m.rooms.w, m.rooms.h, m.shadows.light());
    if shade.iter().all(|&v| v == 0) {
        return;
    }
    let template = &m.rooms;
    let out = m.shade.get_or_insert_with(|| Mask { data: vec![0; template.data.len()], ..template.clone() });
    for (d, s) in out.data.iter_mut().zip(shade) {
        *d = (*d).max(s);
    }
}

/// Тень тел (кольца в клетках, высота в клетках, навес ли) на маске карты
/// `w × h` точек: темнота 0..255. Высоты — на сетке с полями за краем карты на
/// длину самой длинной тени, луч от солнца — [`crate::roofs::sun_rays`].
pub(super) fn shadow_mask(solids: &[Solid], w: usize, h: usize, light: [f32; 2]) -> Vec<u8> {
    let res = mask::RES as f32;
    let tallest = solids.iter().map(|s| s.1).fold(0.0f32, f32::max);
    let pad = (tallest / crate::roofs::SUN_RISE).min(HOUSE_SHADE_REACH).ceil() as usize + 1;
    let (pw, ph) = (w + 2 * pad * mask::RES, h + 2 * pad * mask::RES);
    let grid = Mask { w: pw, h: ph, data: Vec::new() };
    let mut top = vec![0.0f32; pw * ph];
    let mut indoor = vec![false; pw * ph];
    let mut under = vec![false; pw * ph];
    for &(rings, height, canopy) in solids {
        let shifted: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| r.iter().map(|p| [p[0] + pad as f32, p[1] + pad as f32]).collect()).collect();
        for (i, v) in coverage(&grid, &shifted, 0.0) {
            if v < 0.5 {
                continue;
            }
            top[i] = top[i].max(height);
            if canopy {
                under[i] = true;
            } else {
                indoor[i] = true;
            }
        }
    }
    let ray = crate::roofs::sun_rays(&top, pw, ph, light, crate::roofs::SUN_RISE / res);
    let o = pad * mask::RES;
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let k = (y + o) * pw + x + o;
            if indoor[k] {
                continue;
            }
            let dark = if under[k] { 1.0 } else { ((ray[k] - top[k]) / HOUSE_SHADE_SOFT).clamp(0.0, 1.0) };
            out[y * w + x] = (dark * HOUSE_SHADE * 255.0).round() as u8;
        }
    }
    // Край тени — мягче на точку, кроме самих домов.
    let mut soft = out.clone();
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let k = (y + o) * pw + x + o;
            if indoor[k] {
                continue;
            }
            let sum: u32 = [-1i32, 0, 1].iter().flat_map(|&dy| [-1i32, 0, 1].map(move |dx| (dx, dy))).map(|(dx, dy)| out[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize] as u32).sum();
            soft[y * w + x] = (sum / 9) as u8;
        }
    }
    soft
}

/// Крыша по контуру дома. Основа — весь дом: вид и покрытие из его тегов, нет —
/// из самой большой части; цвет — со снимка, если ему можно верить
/// ([`photo_roof`]), иначе из тега. Без тега вида — скатами; башня — шатром,
/// низкая площадка (Лобное место), стройка, навес — плоские; в городе плоские
/// и белая кровля, будка, дом от трёх этажей с 1960-х, если скаты не видны на
/// снимке ([`pitched_by_light`]). Поверх — части (`building:part`) снизу вверх по высоте, каждая своим
/// видом (без вида — плоская, как в OSM, а верх башни — шатром) и цветом.
#[allow(clippy::too_many_arguments)]
pub(super) fn outline_roof(a: &Area, parts: &[(f32, [f32; 2], &Area)], hint: Option<&Hint>, rooms: &Mask, weights: &[(usize, f32)], cell_m: f32, rural: bool) -> OutlineRoof {
    let mw = rooms.w;
    let m2 = cell_m * cell_m;
    let mut mine: Vec<&(f32, [f32; 2], &Area)> = parts.iter().filter(|p| inside(&a.rings, p.1)).collect();
    let outer = a.rings.first().map_or(0.0, |r| ring_area(r));
    // Теги основы — у части, что и есть тело дома (не меньше половины): купол
    // церкви вида основе не даёт.
    let main = mine.iter().filter(|p| p.0 >= outer * 0.5).max_by(|x, y| x.0.total_cmp(&y.0)).map(|p| p.2.tags);
    let tag = |k: &str| match get(a.tags, k) {
        "" => main.map_or("", |t| get(t, k)),
        v => v,
    };
    let paint = |slot: &str, rgb: Option<[u8; 3]>| {
        let key = asset(slot).key;
        rgb.map_or_else(|| key.clone(), |c| library::tint_key(c, &key))
    };
    let tower = get(a.tags, "man_made") == "tower" || !get(a.tags, "tower:type").is_empty();
    let own_h = part_height(a.tags);
    // Высота для доверия снимку: без тегов — два этажа.
    let seen_h = if own_h > 0.0 { own_h } else { 6.0 };
    let base_osm = tag_roof(tag("roof:colour"));
    let base_photo = photo_roof(hint, weights, mw, outer * m2, seen_h, cell_m, false);
    let base_rgb = base_photo.or(base_osm);
    let base_material = tag("roof:material");
    let bare = get(a.tags, "building:levels").is_empty() && get(a.tags, "height").is_empty();
    let platform = own_h > 0.0 && own_h <= 4.0 && get(a.tags, "building:levels").is_empty();
    // Светлая серая кровля на снимке — плоская: скатные — жесть, черепица, медь,
    // а светлое серое — мембрана и бетон (будки в Кремле).
    // Без тени: низкую постройку во дворе на Варварке крыло затеняло на 40%, и
    // срединный цвет из светло-серого становился тёмным.
    let pale = [base_photo, base_photo.and_then(|_| photo_roof(hint, weights, mw, outer * m2, seen_h, cell_m, true))]
        .into_iter()
        .flatten()
        .any(|c| c.iter().max().unwrap_or(&0) - c.iter().min().unwrap_or(&0) < 30 && luma(c) > 135.0);
    // Скаты видны на снимке — скатная, что бы ни подсказывали цвет и год.
    let lit = match hint {
        Some(h) if mine.is_empty() && !tower && !platform && tag("roof:shape").is_empty() => h.sun.is_some_and(|sun| roof_light(h, &a.rings, weights, mw, h.shift(seen_h / cell_m), cell_m).is_some_and(|l| pitched_by_light(&l, sun))),
        _ => false,
    };
    // Дом от трёх этажей с 1960-х — советский модернизм и позже, с плоской
    // кровлей (Ипатьевский переулок, 9 с1 — 1966). Церковь — нет: часовню
    // строят по старому образцу и теперь.
    let sacred = matches!(get(a.tags, "building"), "church" | "chapel" | "cathedral" | "temple" | "mosque" | "synagogue" | "shrine" | "monastery") || get(a.tags, "amenity") == "place_of_worship";
    let modern = !sacred && own_h >= 9.0 && built(a.tags).is_some_and(|y| y >= 1960);
    // Большой зал — рядами скатов, если снимок не говорит о плоской мембране, а
    // год — о модернизме.
    let big_hall = hall(a.tags) && outer * m2 >= HALL_MIN_M2 && !pale && !modern;
    let default = if tower {
        RoofForm::Spire
    } else if platform || matches!(get(a.tags, "building"), "construction" | "roof") {
        RoofForm::Flat
    } else if big_hall {
        RoofForm::Rows
    } else if lit {
        RoofForm::Pitched
    } else if !rural && (pale || modern || (bare && outer * m2 < 60.0)) {
        RoofForm::Flat
    } else {
        RoofForm::Pitched
    };
    // Площадка без покрытия в тегах — каменная.
    let base_slot = if platform && base_material.is_empty() { "flagstones" } else { roof_slot(base_material, rural) };
    let base_form = roof_form(tag("roof:shape"), base_material, default);
    let mut pieces = vec![RoofPiece {
        rings: a.rings.clone(),
        form: base_form,
        paint: paint(base_slot, base_rgb),
        height: own_h,
        eaves: eaves_of(a.tags, base_form, own_h),
        down: (base_form == RoofForm::Shed).then(|| roof_down(tag("roof:direction"))).flatten(),
        across: tag("roof:orientation") == "across",
    }];
    mine.sort_by(|x, y| part_height(x.2.tags).total_cmp(&part_height(y.2.tags)));
    // Цвет частей — группами по тегу `roof:colour` (грани вальмы без тега — одной
    // группой): со снимка по всей группе, если ему можно верить, иначе из тега,
    // иначе цвет дома. По одной грани снимок дал бы её свет, а не цвет кровли.
    // Верить ли — по самому крупному куску группы, где куски — части с общими
    // узлами: грани одной вальмы Сената — одна кровля, а шесть главок «Красного
    // звона» врозь — шесть мелких, и по сумме снимок отдавал им бирюзу кровли
    // под ними.
    let group = |t: &Tags| match get(t, "roof:colour").trim() {
        "" if roof_form(get(t, "roof:shape"), "", RoofForm::Flat) == RoofForm::Shed => "~shed".to_owned(),
        "" => String::new(),
        c => c.to_lowercase(),
    };
    let height_of = |t: &Tags| match part_height(t) {
        h if h > 0.0 => h,
        _ => own_h,
    };
    let samples: Vec<Vec<(usize, f32)>> = mine.iter().map(|p| coverage(rooms, &p.2.rings, 0.0)).collect();
    let mut groups: HashMap<String, (Vec<(usize, f32)>, Vec<usize>, f32)> = HashMap::new();
    for (i, (p, s)) in mine.iter().zip(&samples).enumerate() {
        let k = group(p.2.tags);
        if !k.is_empty() {
            let g = groups.entry(k).or_default();
            g.0.extend_from_slice(s);
            g.1.push(i);
            g.2 = g.2.max(height_of(p.2.tags));
        }
    }
    // Самый крупный кусок группы, м²: части, связанные общими узлами, — вместе.
    let largest = |members: &[usize]| {
        let nodes: Vec<HashSet<i64>> = members.iter().map(|&i| mine[i].2.nodes.iter().flatten().copied().collect()).collect();
        let mut root: Vec<usize> = (0..members.len()).collect();
        fn find(root: &mut [usize], i: usize) -> usize {
            if root[i] != i {
                root[i] = find(root, root[i]);
            }
            root[i]
        }
        for a in 0..members.len() {
            for b in a + 1..members.len() {
                if !nodes[a].is_disjoint(&nodes[b]) {
                    let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                    root[ra] = rb;
                }
            }
        }
        let mut area: HashMap<usize, f32> = HashMap::new();
        for (k, &i) in members.iter().enumerate() {
            *area.entry(find(&mut root, k)).or_default() += mine[i].0 * m2;
        }
        area.into_values().fold(0.0f32, f32::max)
    };
    let seen = |h: f32| if h > 0.0 { h } else { seen_h };
    let group_rgb: HashMap<String, Option<[u8; 3]>> = groups.into_iter().map(|(k, (s, members, h))| (k, photo_roof(hint, &s, mw, largest(&members), seen(h), cell_m, false))).collect();
    let top = mine.len().checked_sub(1);
    for (i, (p, s)) in mine.iter().zip(&samples).enumerate() {
        let t = p.2.tags;
        let material = match get(t, "roof:material") {
            "" => base_material,
            m => m,
        };
        let h = height_of(t);
        let photo = match group(t) {
            k if k.is_empty() => photo_roof(hint, s, mw, p.0 * m2, seen(h), cell_m, false),
            k => group_rgb.get(&k).copied().flatten(),
        };
        let rgb = photo.or_else(|| tag_roof(get(t, "roof:colour"))).or(base_rgb);
        let mut form = roof_form(get(t, "roof:shape"), material, RoofForm::Flat);
        // Верх башни без вида — шатром: у Сенатской башни вид в OSM не задан.
        if tower && Some(i) == top && get(t, "roof:shape").is_empty() {
            form = RoofForm::Spire;
        }
        let slot = if get(t, "roof:material").is_empty() { base_slot } else { roof_slot(material, rural) };
        pieces.push(RoofPiece {
            rings: p.2.rings.clone(),
            form,
            paint: paint(slot, rgb),
            height: h,
            eaves: eaves_of(t, form, h),
            down: (form == RoofForm::Shed).then(|| roof_down(get(t, "roof:direction"))).flatten(),
            across: get(t, "roof:orientation") == "across",
        });
    }
    // Основа дома с частями видна только в щелях между ними, а её `height` — верх
    // всего дома (у собора 67 м): из щели она клала бы тень на весь собор. Пусть
    // будет не выше низа самой низкой части.
    if let Some(low) = pieces.iter().skip(1).map(|p| p.eaves).reduce(f32::min) {
        pieces[0].height = pieces[0].height.min(low);
        pieces[0].eaves = pieces[0].eaves.min(pieces[0].height);
    }
    // Высоты — в клетки, как всё на карте: по ним тени.
    for p in &mut pieces {
        p.height /= cell_m;
        p.eaves /= cell_m;
    }
    OutlineRoof { pieces }
}

/// Крепостная стена контуром (`building=wall`, `historic=citywalls`): сверху её
/// видно целиком — боевой ход с зубцами, а не дом с дощатым полом и дверью.
/// Башня на линии стен (`historic=citywalls` с `tower:type`) — дом с крышей.
pub(super) fn rampart(t: &Tags) -> bool {
    get(t, "building") == "wall" || (matches!(get(t, "historic"), "citywalls" | "city_wall") && get(t, "tower:type").is_empty() && get(t, "man_made") != "tower")
}

/// Зубцы по краю боевого хода: низкие стенки 1,3 × 0,4 клетки с шагом 2,2 у самой
/// кромки — через них не пройти, но видно. Заливкой пола они расплывались в
/// пятна: в клетке четыре точки маски, а зубец — полторы. Край, за которым
/// другой дом (башня, пристройка), — без зубцов.
pub(super) fn battlements(m: &mut Map, rings: &[Vec<[f32; 2]>], style: &WallStyle) {
    const STEP: f32 = 2.2;
    let res = mask::RES as f32;
    let inset = 0.12 + style.width / 2.0;
    let mut teeth: Vec<Wall> = Vec::new();
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            let len = length(a, b);
            if len < 1.6 {
                continue;
            }
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let mut n = [-dir[1], dir[0]];
            if !inside(rings, [mid[0] + n[0] * 0.3, mid[1] + n[1] * 0.3]) {
                n = [-n[0], -n[1]];
            }
            let out = [mid[0] - n[0] * 0.6, mid[1] - n[1] * 0.6];
            if m.rooms.get((out[0] * res).floor() as isize, (out[1] * res).floor() as isize) >= 128 {
                continue;
            }
            let count = ((len - 0.4) / STEP).floor().max(1.0) as usize;
            let start = (len - (count as f32 - 1.0) * STEP) / 2.0;
            for i in 0..count {
                let s = start + i as f32 * STEP;
                let at = |t: f32| [a[0] + dir[0] * t + n[0] * inset, a[1] + dir[1] * t + n[1] * inset];
                let (p, q) = (at(s - 0.65), at(s + 0.65));
                if m.contains(p[0].floor() as i32, p[1].floor() as i32) && m.contains(q[0].floor() as i32, q[1].floor() as i32) {
                    teeth.push(Wall { points: vec![p, q], closed: false, bends: Vec::new(), low: true, style: Some(style.clone()) });
                }
            }
        }
    }
    m.walls.extend(teeth);
}

/// Вид дома: пол, слот стены, деревянные ли двери.
pub(super) fn building_look(kind: &str, t: &Tags) -> (&'static str, &'static str, bool) {
    // Пол дома не должен совпадать с мостовой: у ГУМа и Мавзолея он был той же
    // серой плиткой, что и Красная площадь, и дом читался лишь по тонкой стене.
    let floor = match kind {
        "church" | "cathedral" | "chapel" | "mosque" | "temple" | "synagogue" | "shrine" | "monastery" | "mausoleum" => "temple_floor",
        "industrial" | "warehouse" | "garage" | "garages" | "hangar" | "tower" | "castle" | "fortress" | "bunker" | "train_station" | "transportation" => "brick_floor",
        _ => "wood_planks",
    };
    let wall = match get(t, "building:material") {
        "stone" | "sandstone" | "limestone" | "granite" => "wall_stone_blocks",
        "wood" | "timber_framing" => "wall_timber",
        "metal" | "steel" | "glass" => "wall_metal",
        _ => match kind {
            "church" | "cathedral" | "chapel" => "wall_stone_blocks",
            "hut" | "cabin" | "shed" => "wall_wood",
            _ => "wall_brick",
        },
    };
    (floor, wall, matches!(wall, "wall_wood" | "wall_timber") || floor == "wood_planks")
}

/// Дом ли это и какой: `building=*`, кроме навесов, задуманного и снесённого.
/// Стройка — дом: внутренний корпус Средних торговых рядов в OSM стройка, а на
/// снимке под белой крышей.
pub(super) fn building(t: &Tags) -> Option<&str> {
    let b = get(t, "building");
    (!b.is_empty() && !matches!(b, "no" | "roof" | "proposed" | "demolished" | "ruins")).then_some(b)
}
