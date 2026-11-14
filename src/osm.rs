//! Местность из OpenStreetMap. Выгрузка `.osm` (на openstreetmap.org — «Экспорт»)
//! становится картой: дома — комнатами по настоящим контурам, улицы, рельсы и
//! каналы — дорогами, парки, вода и платформы — полом, деревья — объектами,
//! названия улиц и больших зданий — подписями для ведущего.
//!
//! - Масштаб D&D: клетка — 5 футов. Участок шире `Options::max_cells` клеток
//!   ужимается — клетка крупнее, об этом предупреждение.
//! - Дом заливается в маску комнат с краем на шаг маски, как у овальной кисти,
//!   поэтому стены встают по контуру под любым углом, а не лесенкой. Соседи
//!   сливаются в одну комнату — общая стена между ними ставится отдельно.
//! - Двери — у входов, отмеченных в OSM (`entrance`), а где их нет — одна, на
//!   стене, ближайшей к улице или дорожке.
//! - Чего сверху не видно, пропускается: тоннели, метро, подземные и внутренние
//!   платформы и переходы (`tunnel`, `layer` и `level` ниже нуля, `indoor`).
//!   Навесы над платформами (`building=roof`) — тоже: под ними должно быть видно рельсы.

use super::*;
use crate::map::{OutlineRoof, RoofForm, RoofPiece, segment_distance};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::{FRAC_PI_4, FRAC_PI_8};

/// Метров в клетке D&D: 5 футов.
pub const CELL_M: f32 = 1.524;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Метров в клетке.
    pub cell_m: f32,
    /// Наибольшая сторона карты в клетках: участок крупнее ужимается.
    pub max_cells: u32,
    /// Досадить рощи на свободную сушу, 0..1: 0 — деревья только по снимку и OSM.
    /// Боевая карта у брода Болтона по снимку вышла голой: берег — трава и галька.
    pub trees: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self { cell_m: CELL_M, max_cells: 256, trees: 0.0 }
    }
}

// Остальное — по темам, в `src/osm/`; здесь — сам импорт.
#[path = "osm/cover.rs"]
mod cover;
pub use cover::*;
#[path = "osm/parse.rs"]
mod parse;
pub use parse::*;
#[path = "osm/water.rs"]
mod water;
use water::*;
#[path = "osm/growth.rs"]
mod growth;
use growth::*;
#[path = "osm/roof_photo.rs"]
mod roof_photo;
use roof_photo::*;
#[path = "osm/buildings.rs"]
mod buildings;
use buildings::*;
#[path = "osm/ways.rs"]
mod ways;
use ways::*;
#[path = "osm/parts.rs"]
mod parts;
use parts::*;
// Свои имена важнее одноимённых из procgen (там тоже есть inside, ground, road,
// stream): пока всё было одним файлом, своё их и так перекрывало.
use growth::inside;
use water::ground;
use ways::{road, stream};

#[cfg(test)]
pub fn import(xml: &str, opt: Options) -> Result<(Map, Report), String> {
    import_cover(xml, &[], None, opt)
}

/// Карта из выгрузки и докачанных к ней контуров (`incomplete`); рамка — у выгрузки.
/// С подсказкой со снимка деревья — где на нём кроны, орляк и сухая трава — где
/// бурое, трава на голом камне — где он зарос.
pub fn import_cover(xml: &str, extra: &[String], cover: Option<&Cover>, opt: Options) -> Result<(Map, Report), String> {
    let mut osm = parse(xml)?;
    for e in extra {
        let _ = parse_into(&mut osm, e);
    }
    let [south, west, north, east] = osm.bounds;
    let mid = ((south + north) / 2.0).to_radians();
    let (mx, my) = (111_320.0 * mid.cos(), 110_574.0);
    let (wm, hm) = (((east - west) * mx) as f32, ((north - south) * my) as f32);
    let mut warnings = Vec::new();
    let mut cell = opt.cell_m.max(0.1);
    if wm.max(hm) / cell > opt.max_cells as f32 {
        cell = wm.max(hm) / opt.max_cells as f32;
        warnings.push(format!("участок {:.0}×{:.0} м больше {} клеток по 5 футов: клетка {:.1} м", wm, hm, opt.max_cells, cell));
    }
    // Природное, что так и не замкнулось (сети не было), — пропадает: сказать.
    let lost: Vec<String> = osm
        .relations
        .values()
        .filter(|r| get(&r.tags, "type") == "multipolygon" && !unseen(&r.tags) && (ground(&r.tags, false).is_some() || growth(&r.tags).is_some()))
        .filter(|r| r.ways.iter().any(|w| !osm.ways.contains_key(w)))
        .map(|r| match get(&r.tags, "name") {
            "" => ["natural", "landuse", "leisure"].iter().map(|k| get(&r.tags, k)).find(|v| !v.is_empty()).unwrap_or("контур").to_owned(),
            n => n.to_owned(),
        })
        .collect();
    if !lost.is_empty() {
        warnings.push(format!("неполные в выгрузке и пропущены: {}", lost.join(", ")));
    }
    // Десятая клетки сверху — округление границ в выгрузке (7 знаков: рамка
    // 40×30 клеток выходила 40,04×30,04 и на карте — 41×31 с полоской).
    let (w, h) = (((wm / cell - 0.1).ceil() as u32).max(8), ((hm / cell - 0.1).ceil() as u32).max(8));
    let proj = Proj { north, west, kx: mx / cell as f64, ky: my / cell as f64 };
    let mut hint = cover.map(|c| Hint { cover: c, span: [wm / cell, hm / cell], lean: None, sun: None });
    let m_to_cells = |metres: f32| metres / cell;
    let mut m = Map::new(w, h);
    let sea_tags: Tags = [("natural", "water"), ("water", "sea")].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
    let mut areas = areas(&osm, &proj);
    let sea = osm.ways.values().any(|w| get(&w.tags, "natural") == "coastline");
    let ground = |t: &Tags| ground(t, sea);
    // Природные контуры — с изгибом; дома, мощёное и пути остаются по линейке.
    // Узлы, общие для нескольких природных контуров, не срезаются.
    let natural: Vec<bool> = areas.iter().map(|a| building(a.tags).is_none() && !plaza(a.tags) && (growth(a.tags).is_some() || matches!(ground(a.tags), Some((t, _)) if !matches!(t, "stone_tiles" | "gravel" | "dirt" | "wood_planks")))).collect();
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for a in areas.iter().zip(&natural).filter(|(_, n)| **n).map(|(a, _)| a) {
        for ring in &a.nodes {
            let mut seen = BTreeSet::new();
            for id in &ring[..ring.len() - 1] {
                if seen.insert(*id) {
                    *uses.entry(*id).or_default() += 1;
                }
            }
        }
    }
    for (a, _) in areas.iter_mut().zip(&natural).filter(|(_, n)| **n) {
        a.rings = a
            .rings
            .iter()
            .enumerate()
            .map(|(k, r)| {
                let pins: Vec<bool> = match a.nodes.get(k) {
                    Some(ids) if ids.len() == r.len() + 1 => ids[..r.len()].iter().map(|id| uses.get(id).copied().unwrap_or(0) > 1).collect(),
                    _ => vec![false; r.len()],
                };
                natural_outline(r, &pins)
            })
            .collect();
    }
    // Море по линии берега — обычной водой, с тем же изгибом берега.
    let sea_rings = sea_rings(&osm, &proj, w as f32, h as f32);
    if !sea_rings.is_empty() {
        let rings = sea_rings.iter().map(|r| natural_outline(r, &vec![false; r.len()])).collect();
        areas.push(Area { rings, nodes: Vec::new(), tags: &sea_tags });
    }
    // Город или природа — по доле застройки: от этого зависят пол под всем,
    // дорожки без тега покрытия и стены-ограды.
    // Застройка — только та, что на карте: в выгрузке дома и за её краем.
    let cells_of = |a: &Area| coverage(&m.rooms, &a.rings, 0.0).iter().map(|c| c.1).sum::<f32>() / (mask::RES * mask::RES) as f32;
    let built: f32 = areas.iter().filter(|a| building(a.tags).is_some()).map(cells_of).sum();
    // Боевая карта с хижиной или хутором (дом-три на рамку в 60 м, без городских
    // улиц и рельсов) — природа, хоть дома и занимают больше 6 % рамки.
    let houses = areas.iter().filter(|a| building(a.tags).is_some() && cells_of(a) > 0.0).count();
    let streets = osm.ways.values().any(|w| {
        matches!(get(&w.tags, "highway"), "residential" | "living_street" | "pedestrian" | "primary" | "secondary" | "tertiary" | "trunk") || !get(&w.tags, "railway").is_empty()
    });
    let rural = built < 0.06 * (w * h) as f32 || (houses <= 3 && !streets);
    m.ambience = if rural { Season::Summer.ambience() } else { "town-1" }.into();
    m.wall_style = wall(if rural { "wall_fieldstone" } else { "wall_brick" });

    // Под всем — мостовая или трава пятнами, поверх — земля, трава, вода и платформы.
    let mut floors = Floors::default();
    let glacier = areas.iter().any(|a| get(a.tags, "natural") == "glacier");
    let base_slot = if rural { hint.as_ref().map_or("meadow", |h| h.cover.base_floor()) } else { "flagstones" };
    // Снег и лёд по снимку: камень — серый сланец (мел и мергель выходили бежевыми),
    // тропа — примятый снег.
    let cold = matches!(base_slot, "ice_glacier" | "snow");
    // Снег и лёд по снимку — зимний звук места, а не летний лес.
    if rural && cold {
        m.ambience = Season::Winter.ambience().into();
    }
    // Верховое болото: бочаги — торфяная вода без кувшинок, мочажины — топь.
    let bog = base_slot == "bog_moss";
    // Сухой юг по снимку: лиственное — каменный дуб, сад — оливы, куст — ракитник,
    // эспарто, розмарин; папоротнику там не расти.
    let arid = matches!(base_slot, "earth_pale" | "straw_grass");
    let base = floors.get(&mut m, base_slot);
    let all = mask::cells(m.rooms.w, m.rooms.h, (0, 0), (w as i32 - 1, h as i32 - 1));
    m.paint(Layer::Floor, base, &all);
    let mut grounds: Vec<(u8, usize)> = areas.iter().enumerate().filter_map(|(k, a)| ground(a.tags).map(|(_, o)| (o, k))).collect();
    grounds.sort();
    let waters: Vec<usize> = grounds.iter().map(|g| g.1).filter(|&k| matches!(ground(areas[k].tags), Some(("water_deep", _)))).collect();
    let wild = hint.as_ref().filter(|h| h.cover.wild.is_some()).map(|h| photo_water(&m, h)).unwrap_or_default();
    let mut water_done = false;
    let mut rock_mats: Vec<usize> = Vec::new();
    let relaid = relay_plazas(&areas, &m, hint.as_ref(), sea, &mut warnings);
    for (_, k) in grounds {
        let (tex, _) = ground(areas[k].tags).unwrap_or(("flagstones", 0));
        let tex = relaid.get(&k).copied().unwrap_or(tex);
        // Ледник, что на снимке белый (фирн Мёнхсйоха), — снег: иначе по краю
        // ледника OSM оставалась ступенчатая полоса серого льда.
        let tex = if cold && tex.starts_with("rock_") {
            "rock_alpine"
        } else if base_slot == "snow" && tex == "ice_glacier" {
            "snow"
        } else {
            tex
        };
        if tex == "water_deep" {
            if !water_done {
                water(&mut m, &mut floors, &waters.iter().map(|&k| &areas[k]).collect::<Vec<_>>(), bog, &wild);
                water_done = true;
            }
            continue;
        }
        let weights = coverage(&m.rooms, &areas[k].rings, 0.0);
        // Парк и сад — травой только там, где она есть на снимке: в контуре парка
        // лежат и мощёные площадки. В Кремле газоном выходила площадь у белой
        // дорожки, а газоны OSM внутри того же парка со снимком совпадали.
        let weights = match &hint {
            Some(h) if park_only(areas[k].tags) => {
                let (mw, res) = (m.rooms.w, mask::RES as f32);
                weights
                    .into_iter()
                    .filter_map(|(i, v)| {
                        let p = [((i % mw) as f32 + 0.5) / res, ((i / mw) as f32 + 0.5) / res];
                        let t = ((h.green(p, 0.8) - 0.3) / 0.4).clamp(0.0, 1.0);
                        (t > 0.0).then_some((i, v * t * t * (3.0 - 2.0 * t)))
                    })
                    .collect()
            }
            _ => weights,
        };
        if !weights.is_empty() {
            // Бассейн и фонтан — на плитке: вода FA полупрозрачна, по траве
            // бассейн выходил тёмной кляксой.
            if tex == "water_shallow" {
                let tiles = floors.get(&mut m, "stone_tiles");
                m.paint(Layer::Floor, tiles, &weights);
            }
            let t = floors.get(&mut m, tex);
            m.paint(Layer::Floor, t, &weights);
            if tex.starts_with("rock_") && !rock_mats.contains(&t) {
                rock_mats.push(t);
            }
        }
    }
    if !water_done && wild.iter().any(|&v| v > 0.0) {
        water(&mut m, &mut floors, &[], bog, &wild);
    }
    // Отмели рек и озёр — по снимку: в OSM галечник у излучины чаще внутри
    // контура реки, и на карте там выходило мелководье по траве, серое, а на
    // снимке — светлая галька. У моря светлое у берега — пена прибоя, не галька.
    if let Some(hint) = &hint {
        let inland: Vec<&Area> = waters.iter().map(|&k| &areas[k]).filter(|a| get(a.tags, "water") != "sea").collect();
        let res = mask::RES as f32;
        let mw = m.rooms.w;
        let bars: Vec<(usize, f32)> = inland
            .iter()
            .flat_map(|a| coverage(&m.rooms, &a.rings, 0.0))
            .filter_map(|(k, v)| {
                let p = [((k % mw) as f32 + 0.5) / res, ((k / mw) as f32 + 0.5) / res];
                let rock = hint.share(p, 0.8, Class::Rock);
                let t = ((rock - 0.4) / 0.3).clamp(0.0, 1.0);
                (t > 0.0).then_some((k, v * t * t * (3.0 - 2.0 * t)))
            })
            .collect();
        if !bars.is_empty() {
            let t = floors.get(&mut m, "shingle");
            m.paint(Layer::Floor, t, &bars);
        }
    }
    if rural {
        // Луг пятнами некошеного и широкими разводами: на снимке поле не бывает
        // ровным ковром одного тона. Только по лугу — пляж, вода и пустошь целы.
        // И обратно: на пустоши — проплешины зелёной травы. Края — по полю шума в
        // две октавы: круглые пятна кистью выглядели наляпанными.
        let (meadow, rough) = (floors.get(&mut m, "meadow"), floors.get(&mut m, "rough_grass"));
        // Клетка годится, если она и соседи — того же пола: пятно не лезет на пляж и в воду.
        let only = |m: &Map, mat: usize| -> Vec<bool> {
            (0..(w * h) as i32)
                .map(|k| {
                    let (x, y) = (k % w as i32, k / w as i32);
                    [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|&(dx, dy)| matches!(m.floor_at(x + dx, y + dy) as usize, f if f == mat + 1 || f == 0))
                })
                .collect()
        };
        // На основе (луг, а вне зелёного биома — земля, сухая трава, снег).
        let on_ice = if glacier && base_slot != "snow" {
            let ice = floors.get(&mut m, "ice_glacier");
            only(&m, ice)
        } else {
            Vec::new()
        };
        let (mut to_earth, mut to_snow, mut to_rock, mut to_ice) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // Голый камень — с травой в трещинах и на уступах: на снимке мел и сланец
        // местами заросли, сплошная серая заливка выглядела асфальтом.
        let on_rock: Vec<bool> = rock_mats.iter().map(|&r| only(&m, r)).fold(vec![false; (w * h) as usize], |acc, v| acc.iter().zip(v).map(|(a, b)| *a || b).collect());
        // Со снимком — по точкам маски, где лежит камень, вплоть до края контура:
        // проверка соседей по клеткам оставляла по краю голую полосу, на карте —
        // белый пунктир по заросшему склону.
        let rock_here: Vec<bool> = match &hint {
            Some(_) => (0..m.rooms.w * m.rooms.h).map(|k| m.terrain.iter().any(|c| rock_mats.contains(&(c.material as usize)) && c.mask.data[k] > 0)).collect(),
            None => Vec::new(),
        };
        let mut to_grass = Vec::new();
        let mut to_dry = Vec::new();
        let res = mask::RES as f32;
        let (mw, mh) = (m.rooms.w, m.rooms.h);
        let (mut to_rough, mut to_meadow) = (Vec::new(), Vec::new());
        // Доля пола в точке маски. Пятна по снимку ложатся по ней, а не по клетке
        // с соседями: у края контура OSM клеточная проверка резала пятно по сетке —
        // ступеньки в клетку на краю снега у ледника Мёнхсйоха. К краю (доля
        // 0,95 → 0,6) пятно слабеет и на соседний пол не лезет.
        let ice_mat = (!on_ice.is_empty()).then(|| floors.get(&mut m, "ice_glacier"));
        let share_of = |mat: usize| -> Vec<f32> {
            match m.terrain.iter().find(|c| c.material as usize == mat) {
                Some(c) => c.mask.data.iter().map(|&v| v as f32 / 255.0).collect(),
                None => vec![0.0; mw * mh],
            }
        };
        let (base_share, rough_share) = (share_of(base), share_of(rough));
        let ice_share = ice_mat.map_or_else(Vec::new, share_of);
        let fade = |x: f32| {
            let t = ((x - 0.6) / 0.35).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        for j in 0..mh {
            for i in 0..mw {
                let p = [(i as f32 + 0.5) / res, (j as f32 + 0.5) / res];
                let cell = (p[1] as usize).min(h as usize - 1) * w as usize + (p[0] as usize).min(w as usize - 1);
                let k = j * mw + i;
                let marks = [to_earth.len(), to_snow.len(), to_rock.len(), to_ice.len(), to_grass.len(), to_dry.len(), to_rough.len(), to_meadow.len()];
                let mut f = 1.0f32;
                // Со снимком пятна — где они на самом деле: бурое на лугу — орляк и
                // сухая трава, зелень на пустоши и на камне — трава. Без снимка — по шуму.
                let seen = |class: Class| hint.as_ref().map(|h| h.share(p, 0.8, class));
                let ramp = |x: f32, a: f32, b: f32| {
                    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                    t * t * (3.0 - 2.0 * t)
                };
                if rock_here.get(k).copied().unwrap_or(on_rock[cell]) {
                    // Пятна не мельче нескольких клеток: в клетку-две они упирались
                    // в шаг маски (4 точки на клетку) и выходили квадратными кляксами.
                    let v = match seen(Class::Rock) {
                        Some(rock) => ramp(1.0 - rock - seen(Class::Water).unwrap_or(0.0), 0.5, 0.85),
                        None => {
                            let n = noise(p, 10.0, 25) * 0.72 + noise(p, 3.8, 26) * 0.2 + noise(p, 1.3, 29) * 0.08;
                            ((n - 0.18) / 0.32).clamp(0.0, 1.0) * 0.85
                        }
                    };
                    if v > 0.0 {
                        to_grass.push((k, v));
                    }
                } else if base_share[k] > 0.6 {
                    f = fade(base_share[k]);
                    // Вне зелёного биома основа — не луг: зелень на ней — луг, серое
                    // без тёплого оттенка — камень.
                    if base_slot != "meadow" {
                        if let Some(grass) = seen(Class::Grass) {
                            let v = ramp(grass, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                to_meadow.push((k, v));
                            }
                        }
                        // Камень — где его почти сплошь: светлые гребни мергеля и
                        // дороги Табернаса при пороге 0,35 ложились пятнами по всей земле.
                        if let Some(rock) = seen(Class::Rock) {
                            let v = ramp(rock, 0.5, 0.85) * 0.85;
                            if v > 0.0 {
                                to_rock.push((k, v));
                            }
                        }
                    }
                    for (class, slot_is_base, out) in [(Class::Earth, base_slot == "earth_pale", &mut to_earth), (Class::Snow, base_slot == "snow", &mut to_snow), (Class::Ice, base_slot == "ice_glacier", &mut to_ice)] {
                        if let Some(x) = seen(class).filter(|_| !slot_is_base) {
                            let v = ramp(x, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                out.push((k, v));
                            }
                        }
                    }
                    // Мелкая октава в клетку — край пятна рваный, как у травы, а не
                    // ломаный по отрезкам сетки шума.
                    // Сухая и скошенная трава — своим ровным полом, не кочками пустоши.
                    if let Some(dry) = seen(Class::Dry).filter(|_| base_slot != "straw_grass") {
                        let v = ramp(dry, 0.3, 0.65) * 0.85;
                        if v > 0.0 {
                            to_dry.push((k, v));
                        }
                    }
                    let v = match seen(Class::Brown) {
                        Some(brown) => ramp(brown, 0.3, 0.65) * 0.8,
                        None => {
                            let n = noise(p, 13.0, 21) * 0.62 + noise(p, 4.5, 22) * 0.26 + noise(p, 1.2, 27) * 0.12;
                            ((n - 0.2) / 0.3).clamp(0.0, 1.0) * 0.65
                        }
                    };
                    if v > 0.0 {
                        to_rough.push((k, v));
                    }
                } else if ice_share.get(k).is_some_and(|&x| x > 0.6) {
                    f = fade(ice_share[k]);
                    // Морена на леднике — камень по снимку; фирн (белое) — снег.
                    if let Some(rock) = seen(Class::Rock) {
                        let v = ramp(rock, 0.35, 0.7) * 0.85;
                        if v > 0.0 {
                            to_rock.push((k, v));
                        }
                    }
                    if let Some(snow) = seen(Class::Snow) {
                        let v = ramp(snow, 0.3, 0.65) * 0.9;
                        if v > 0.0 {
                            to_snow.push((k, v));
                        }
                    }
                } else if rough_share[k] > 0.6 {
                    f = fade(rough_share[k]);
                    // Кустарник в пустыне — по голой земле; снег на пустоши — снег.
                    for (class, out) in [(Class::Earth, &mut to_earth), (Class::Snow, &mut to_snow)] {
                        if let Some(x) = seen(class) {
                            let v = ramp(x, 0.3, 0.65) * 0.85;
                            if v > 0.0 {
                                out.push((k, v));
                            }
                        }
                    }
                    // Пустошь, что на снимке — светлый выгон (пастбища Малхема под тем же тегом).
                    if let Some(dry) = seen(Class::Dry) {
                        let v = ramp(dry, 0.3, 0.65) * 0.85;
                        if v > 0.0 {
                            to_dry.push((k, v));
                        }
                    }
                    let v = match seen(Class::Grass) {
                        Some(grass) => ramp(grass, 0.45, 0.8) * 0.75,
                        None => {
                            let n = noise(p, 9.0, 23) * 0.62 + noise(p, 3.5, 24) * 0.26 + noise(p, 1.2, 28) * 0.12;
                            ((n - 0.15) / 0.3).clamp(0.0, 1.0) * 0.6
                        }
                    };
                    if v > 0.0 {
                        to_meadow.push((k, v));
                    }
                }
                if f < 1.0 {
                    for (out, from) in [&mut to_earth, &mut to_snow, &mut to_rock, &mut to_ice, &mut to_grass, &mut to_dry, &mut to_rough, &mut to_meadow].into_iter().zip(marks) {
                        for e in &mut out[from..] {
                            e.1 *= f;
                        }
                    }
                }
            }
        }
        // На болоте тёмное по снимку — кочки: тёмный мох (бурая трава FA светлее
        // мха и выходила светлыми пятнами).
        let hummock = if bog { floors.get(&mut m, "bog_hummock") } else { rough };
        m.paint(Layer::Floor, hummock, &to_rough);
        m.paint(Layer::Floor, meadow, &to_meadow);
        // Трава на камне, что по снимку зарос, — основа: в пустыне земля, на леднике снег.
        m.paint(Layer::Floor, base, &to_grass);
        if !to_dry.is_empty() {
            // Выгоревший мох болота — светлая сухая трава вполсилы: сплошь мох FA
            // темнее снимка, а без светлых пятен болото — ровный бурый ковёр.
            if bog {
                let straw = floors.get(&mut m, "straw_grass");
                let light: Vec<(usize, f32)> = to_dry.iter().map(|&(k, v)| (k, v * 0.6)).collect();
                m.paint(Layer::Floor, straw, &light);
            } else {
                let dry = floors.get(&mut m, "dry_grass");
                m.paint(Layer::Floor, dry, &to_dry);
            }
        }
        // Голая земля: на сухом пастбище — бурая потрескавшаяся, в пустыне — светлая.
        // Камень и земля на леднике — серая морена (сланец): мел и мергель на нём
        // выходили бежевыми.
        for (slot, cells) in [(if cold { "rock_alpine" } else if base_slot == "straw_grass" { "earth_dry" } else { "earth_pale" }, &to_earth), ("ice_glacier", &to_ice), ("snow", &to_snow), (if cold { "rock_alpine" } else { rock_kind(&Tags::new()).0 }, &to_rock)] {
            if !cells.is_empty() {
                let t = floors.get(&mut m, slot);
                m.paint(Layer::Floor, t, cells);
            }
        }
    }

    // Верховое болото — топь целиком: пятна выгоревшего мха, зелени и кочек тоже
    // болото для «Живой карты» и для шага (вдвое дороже), а не только сам мох.
    if bog {
        for slot in ["bog_moss", "bog_hummock", "straw_grass", "meadow"] {
            if let Some(&i) = floors.0.get(slot) {
                m.materials[i].live = Live::Swamp;
            }
        }
    }

    // Дома: комнаты по контуру.
    let mut houses: Vec<House> = Vec::new();
    let mut ramparts: Vec<(Vec<Vec<[f32; 2]>>, WallStyle)> = Vec::new();
    // Части зданий: площадь, середина, теги — для крыш домов без своих тегов крыши.
    let parts: Vec<(f32, [f32; 2], &Area)> = areas
        .iter()
        .filter(|a| a.tags.contains_key("building:part") && building(a.tags).is_none())
        .filter_map(|a| {
            let r = a.rings.first().filter(|r| !r.is_empty())?;
            let c = r.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
            Some((ring_area(r), [c[0] / r.len() as f32, c[1] / r.len() as f32], a))
        })
        .collect();
    // Наклон и солнце снимка — по всем домам сразу, до крыш. Дом с частями —
    // частями: его контур с высотой самой высокой части лёг бы мимо.
    if let Some(h) = hint.as_ref() {
        let whole = |a: &Area| building(a.tags).is_some() && !parts.iter().any(|p| inside(&a.rings, p.1));
        let tall: Vec<(&[Vec<[f32; 2]>], f32)> = areas
            .iter()
            .filter(|a| whole(a) || (a.tags.contains_key("building:part") && building(a.tags).is_none()))
            .filter(|a| part_height(a.tags) >= 9.0)
            .map(|a| (&a.rings[..], part_height(a.tags) / cell))
            .collect();
        let lean = photo_lean(h, &tall);
        let leant = Hint { cover: h.cover, span: h.span, lean, sun: None };
        let lights: Vec<Light> = areas
            .iter()
            .filter(|a| whole(a) && matches!(roof_form(get(a.tags, "roof:shape"), "", RoofForm::Flat), RoofForm::Pitched | RoofForm::Gabled | RoofForm::Spire))
            .filter_map(|a| roof_light(&leant, &a.rings, &coverage(&m.rooms, &a.rings, 0.0), m.rooms.w, leant.shift(part_height(a.tags).max(6.0) / cell), cell))
            .collect();
        let sun = photo_sun(&lights);
        hint = Some(Hint { sun, ..leant });
    }
    let mut names: BTreeMap<String, ([f32; 2], f32)> = BTreeMap::new();
    for a in &areas {
        let Some(kind) = building(a.tags) else { continue };
        let weights = coverage(&m.rooms, &a.rings, 0.0);
        if weights.is_empty() {
            continue;
        }
        let (floor, slot, wood) = building_look(kind, a.tags);
        // Крепость — кирпич в своём цвете: стена — по цвету верха (тег, а нет —
        // снимок), боевая башня — по цвету стен (сверху у неё крыша). Кремлёвская
        // стена выходила домом с дощатым полом, хотя в OSM она `#ec8859`.
        let wall_top = rampart(a.tags);
        let fort = wall_top || get(a.tags, "tower:type") == "defensive";
        let tag = |k: &str| colour(get(a.tags, k));
        let tint = if wall_top {
            tag("roof:colour").or_else(|| tag("building:colour")).or_else(|| hint.as_ref().and_then(|h| h.colour(&weights, m.rooms.w, h.shift(part_height(a.tags) / cell), false)))
        } else if fort {
            tag("building:colour")
        } else {
            None
        };
        let f = match (fort, tint) {
            (true, Some(c)) => floors.tinted(&mut m, "brick_floor", c),
            (true, None) => floors.get(&mut m, "brick_floor"),
            _ => floors.get(&mut m, floor),
        };
        m.paint(Layer::Rooms, f, &weights);
        if wall_top {
            let teeth = [0, 1, 2].map(|i| (tint.unwrap_or([150, 140, 125])[i] as f32 * 0.72) as u8);
            ramparts.push((a.rings.clone(), WallStyle { width: 0.4, ..wall_tinted(slot, Some(teeth)) }));
        }
        let entrances = a
            .nodes
            .iter()
            .flatten()
            .filter_map(|id| osm.nodes.get(id))
            .filter(|n| !matches!(get(&n.tags, "entrance"), "" | "no") || !get(&n.tags, "door").is_empty())
            .map(|n| proj.at(n.lat, n.lon))
            .collect();
        let grand = matches!(kind, "train_station" | "transportation" | "church" | "cathedral" | "civic" | "public" | "commercial" | "retail" | "hotel" | "government");
        let (c, cells) = centre(&m.rooms, &weights);
        let name = get(a.tags, "name");
        if !name.is_empty() && cells >= 40.0 && names.get(name).is_none_or(|n| n.1 < cells) {
            names.insert(name.to_owned(), (c, cells));
        }
        let style = if fort { wall_tinted(slot, tag("building:colour").or(tint)) } else { wall(slot) };
        // Крыша — у всего, что сверху закрыто: у стены и трибуны её нет.
        if !wall_top && kind != "grandstand" {
            m.outline_roofs.push(outline_roof(a, &parts, hint.as_ref(), &m.rooms, &weights, cell, rural));
        }
        houses.push(House { rings: a.rings.clone(), entrances, style, wood: wood && !fort, grand, doors: !wall_top });
    }
    // Навес (`building=roof`) — крыша без стен и пола: над перроном, заправкой,
    // залом «Зарядья». Под ним земля как есть, крыша гаснет над токеном.
    for a in &areas {
        if get(a.tags, "building") != "roof" {
            continue;
        }
        let weights = coverage(&m.rooms, &a.rings, 0.0);
        if !weights.is_empty() {
            m.outline_roofs.push(outline_roof(a, &parts, hint.as_ref(), &m.rooms, &weights, cell, rural));
        }
    }
    // Зубцы — когда все дома на месте: за краем, где башня, их не ставить.
    for (rings, teeth) in &ramparts {
        battlements(&mut m, rings, teeth);
    }
    house_shadows(&mut m, &areas, cell);
    m.prune();
    // Названия озёр, лесов, пустошей.
    for a in &areas {
        let name = get(a.tags, "name");
        if name.is_empty() || building(a.tags).is_some() || (ground(a.tags).is_none() && growth(a.tags).is_none()) {
            continue;
        }
        let (c, cells) = centre(&m.rooms, &coverage(&m.rooms, &a.rings, 0.0));
        if cells >= 60.0 && names.get(name).is_none_or(|n| n.1 < cells) {
            names.insert(name.to_owned(), (c, cells));
        }
    }

    // Линии снизу вверх: вода, рельсы, тропинки, улицы; мосты — поверх.
    let mut lanes: Vec<Lane> = Vec::new();
    // Проезжие улицы и пешеходные дорожки — порознь: дверь дома ищет сначала улицу.
    let mut streets: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut walks: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut street_names: BTreeMap<String, ([f32; 2], [f32; 2], f32)> = BTreeMap::new();
    let mut stones: Vec<[f32; 2]> = Vec::new();
    let rails_mat = m.material_index(Material { name: "Рельсы".into(), paint: Paint::Color([84, 82, 80]), vary: false, live: Live::Off, rot: 0.0 }).unwrap_or(0);
    let deck_base = m.material_index(Material { name: "Настил".into(), paint: Paint::Color([70, 50, 34]), vary: false, live: Live::Off, rot: 0.0 }).unwrap_or(0);
    // Тропа по пляжу и гальке не видна: на снимке Лулворта следа нет, а на карте
    // береговая тропа ложилась вдоль пляжа тёмной полосой.
    let beaches: Vec<&Area> = areas.iter().filter(|a| matches!(get(a.tags, "natural"), "beach" | "shingle" | "sand")).collect();
    let on_beach = |p: [f32; 2]| beaches.iter().any(|b| inside(&b.rings, p));
    // Пешеходная линия по площади — разметка для маршрутов: по Красной площади
    // они ложились светлыми полосами поперёк сплошной брусчатки.
    let plazas: Vec<&Area> = areas.iter().filter(|a| plaza(a.tags) && building(a.tags).is_none()).collect();
    let on_plaza = |p: [f32; 2]| plazas.iter().any(|b| inside(&b.rings, p));
    for way in osm.ways.values() {
        let t = &way.tags;
        if t.is_empty() || unseen(t) {
            continue;
        }
        let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
        let closed = way.nodes.len() >= 4 && way.nodes.first() == way.nodes.last();
        let lay = layer(t);
        for line in near_map(&pts, w as f32, h as f32, 12.0) {
            let line = dense(&line, 2.0);
            let path = |material: usize, width: f32, soft: f32, rough: f32| Path { points: line.clone(), material, width, soft, rough, along: false };
            if let Some((metres, deep)) = stream(t) {
                let water = floors.get(&mut m, if deep { "water_deep" } else { "water_shallow" });
                lanes.push((lay, 0, vec![path(water, m_to_cells(metres), 0.3, 0.15)]));
            } else if let Some((gauge, ballast)) = rail(t) {
                let half = m_to_cells(gauge / 2.0 + 0.035);
                let mut v = Vec::new();
                if ballast {
                    let gravel = floors.get(&mut m, "gravel");
                    v.push(path(gravel, m_to_cells(gauge + 1.8), 0.25, 0.1));
                }
                for side in [-half, half] {
                    v.push(Path { points: offset(&line, side), material: rails_mat, width: 0.1, soft: 0.0, rough: 0.0, along: false });
                }
                lanes.push((lay, if ballast { 1 } else { 5 }, v));
            } else if get(t, "ford") == "stepping_stones" || get(t, "surface") == "stepping_stones" {
                // Камни-переходы: цепочка камней через воду с ровным шагом по
                // всей длине, полосы дороги нет.
                let mut next = 0.0;
                let mut walked = 0.0;
                for s in line.windows(2) {
                    let l = length(s[0], s[1]);
                    while next <= walked + l {
                        let f = (next - walked) / l.max(1e-6);
                        stones.push([s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]);
                        next += STEPPING;
                    }
                    walked += l;
                }
            } else if let Some((metres, tex, order)) = road(t, rural) {
                // Мост: у пешеходного — дощатый настил вдоль и деревянные перила,
                // у проезжего — своё мощение и каменный парапет.
                let bridge = !matches!(get(t, "bridge"), "" | "no");
                let tex = if bridge && order == 3 {
                    "wood_planks"
                } else if cold && natural_surface(tex) {
                    "snow_trampled"
                } else {
                    tex
                };
                let material = floors.get(&mut m, tex);
                // Гравийная дорожка парка ровнее тропы: на снимке у неё чёткий край.
                let (soft, rough) = if bridge {
                    (0.0, 0.0)
                } else if tex == "path_gravel" {
                    (0.3, 0.12)
                } else if natural_surface(tex) {
                    (0.45, 0.25)
                } else {
                    (0.12, 0.04)
                };
                let width = m_to_cells(metres).max(1.0);
                // Рыхлое покрытие или приливная — по пляжу следа нет (`surface=pebblestone`,
                // `tidal=yes` у береговой тропы Лулворта). Мощёная — видна.
                let loose = matches!(get(t, "surface"), "" | "pebblestone" | "gravel" | "fine_gravel" | "sand" | "shingle" | "ground" | "dirt" | "earth" | "grass" | "rock" | "unpaved" | "compacted")
                    || get(t, "tidal") == "yes";
                // Ступени видны и на площади: это перепад, а не разметка.
                let steps = get(t, "highway") == "steps";
                let hidden = |p: [f32; 2]| (loose && on_beach(p)) || (!steps && on_plaza(p));
                if order == 3 && !bridge && line.iter().any(|&p| hidden(p)) {
                    // Куски тропы вне пляжа и площади — порознь, по ним — ничего.
                    let mut run: Vec<[f32; 2]> = Vec::new();
                    for &p in line.iter().chain(std::iter::once(&[f32::NAN; 2])) {
                        if p[0].is_finite() && !hidden(p) {
                            run.push(p);
                            continue;
                        }
                        if run.len() >= 2 {
                            let points = std::mem::take(&mut run);
                            lanes.push((lay, order, vec![Path { points, material, width, soft, rough, along: false }]));
                        }
                        run.clear();
                    }
                    // Для дверей домов тропа остаётся тропой и там, где её не видно.
                    walks.push(line.clone());
                    continue;
                }
                let mut deck = path(material, width, soft, rough);
                deck.along = bridge && order == 3;
                // Доски FA полупрозрачны (между ними щели) — под настилом сплошная
                // подложка цвета дерева, иначе сквозь мост видно реку.
                let under = (bridge && order == 3).then(|| path(deck_base, width, 0.0, 0.0));
                lanes.push((lay, order, under.into_iter().chain([deck]).collect()));
                // Настил по болоту (`bridge=boardwalk`) лежит на земле, перил у него
                // нет: на Виру раба с тропы нельзя было сойти в болото. Перила моста —
                // только у карты: за краем сцены они тянулись на десятки клеток.
                if bridge && get(t, "bridge") != "boardwalk" {
                    let style = WallStyle { width: 0.15, ..wall(if order == 3 { "wall_wood" } else { "wall_stone_blocks" }) };
                    for side in [-width / 2.0, width / 2.0] {
                        for points in near_map(&offset(&line, side), w as f32, h as f32, 0.5) {
                            m.walls.push(Wall { points, closed: false, bends: Vec::new(), low: true, style: Some(style.clone()) });
                        }
                    }
                }
                (if order >= 4 { &mut streets } else { &mut walks }).push(line.clone());
                let name = get(t, "name");
                if order >= 4
                    && !name.is_empty()
                    && let Some((a, b)) = longest(&line)
                    && street_names.get(name).is_none_or(|s| s.2 < length(a, b))
                {
                    street_names.insert(name.to_owned(), (a, b, m_to_cells(metres)));
                }
            } else if get(t, "man_made") == "pier" && !closed {
                // Причал линией — дощатый настил вдоль, с подложкой, как у мостков.
                let width = m_to_cells(width_tag(t).unwrap_or(3.0)).max(1.0);
                let planks = floors.get(&mut m, "wood_planks");
                let mut deck = path(planks, width, 0.0, 0.0);
                deck.along = true;
                lanes.push((lay.max(1.0), 3, vec![path(deck_base, width, 0.0, 0.0), deck]));
                walks.push(line.clone());
            } else if get(t, "railway") == "platform" && !closed {
                let tiles = floors.get(&mut m, "stone_tiles");
                lanes.push((lay, 2, vec![path(tiles, m_to_cells(width_tag(t).unwrap_or(4.0)), 0.05, 0.0)]));
            }
        }
    }
    lanes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    m.paths.extend(lanes.into_iter().flat_map(|l| l.2));

    walls_and_doors(&mut m, &houses, &streets, &walks);

    // Ограды: стены, заборы, решётки, изгороди, обрывы; у калиток и ворот — проход.
    for way in osm.ways.values() {
        let Some((slot, low, width)) = barrier(&way.tags, rural) else { continue };
        if unseen(&way.tags) {
            continue;
        }
        let nodes: Vec<&Node> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).collect();
        let pts: Vec<[f32; 2]> = nodes.iter().map(|n| proj.at(n.lat, n.lon)).collect();
        let gaps: Vec<Option<f32>> = nodes.iter().map(|n| gate(&n.tags)).collect();
        let style = WallStyle { width, ..wall(slot) };
        for piece in split_at_gates(&pts, &gaps) {
            for line in near_map(&piece, w as f32, h as f32, 0.5) {
                m.walls.push(Wall { points: line, closed: false, bends: Vec::new(), low, style: Some(style.clone()) });
            }
        }
    }

    // Обрывы — рельефом: земля опускается с нижней стороны линии, полосы скалы,
    // тени и стены Foundry строит рельеф.
    let cliffs: Vec<(Vec<[f32; 2]>, f32)> = osm
        .ways
        .values()
        .filter(|way| get(&way.tags, "natural") == "cliff" && !unseen(&way.tags))
        .flat_map(|way| {
            let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
            let levels = cliff_levels(&way.tags);
            near_map(&pts, w as f32, h as f32, 25.0).into_iter().map(move |l| (l, levels))
        })
        .collect();
    if let Some(mut r) = cliff_relief(&cliffs, w, h) {
        relief_strips(&mut r, Season::Summer);
        r.quantize();
        m.relief = Some(r);
    }

    // Лес, сад, кустарник, вереск, болото: отдельных деревьев в OSM у них нет —
    // сажаем дротиками, мимо домов, дорог, воды, оград и соседей.
    let mut blocked = Blocked::new(&m);
    for a in &areas {
        if matches!(ground(a.tags), Some(("water_deep" | "water_shallow", _))) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.5 {
                    blocked.cells[k] = true;
                }
            }
        }
    }
    // Дома и вода: над ними кроны не нависают, над дорогой и оградой — да.
    let hard = blocked.clone();
    // Полоса скалы ложится вниз от кромки, до трёх клеток: на ней не растёт.
    for (l, _) in &cliffs {
        blocked.line(l, 1.0);
        blocked.line(&offset(l, -1.8), 1.8);
    }
    for p in &m.paths {
        blocked.line(&p.curve(), p.width / 2.0 + 0.6);
    }
    for wl in &m.walls {
        blocked.line(&wl.points, 0.8);
    }
    let mut spacing = Spacing::default();
    let mut planted = 0usize;
    let mut litter: Vec<([f32; 2], f32)> = Vec::new();

    // Кладбище: могилы рядами, изголовьем на запад, как у церкви генератора; мимо
    // дорожек, оград и домов. Без надгробий в паках — холмик земли с камнем.
    let tombstones = !missing("tombstone");
    for (k, a) in areas.iter().enumerate().filter(|(_, a)| get(a.tags, "amenity") == "grave_yard" || get(a.tags, "landuse") == "cemetery") {
        let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
        let dirt = floors.get(&mut m, "dirt");
        let mut rng = Rng::new(&format!("osm/graves/{k}"));
        let mut y = lo[1].floor() + 0.5;
        let mut row = 0;
        while y + 1.0 < hi[1] {
            let mut x = lo[0].floor() + 0.5 + (row % 2) as f32 * 1.5;
            while x + 2.0 < hi[0] {
                let q = [x + 1.0, y + 0.5];
                let spots = [[x, y], [x + 2.0, y], [x, y + 1.0], [x + 2.0, y + 1.0], q];
                if spots.iter().all(|&c| !blocked.at(c) && inside(&a.rings, c)) && rng.chance(0.55) {
                    church::grave(&mut m, &mut rng, q, tombstones, dirt, false);
                    blocked.disk(q, 1.2);
                }
                x += 3.0;
            }
            y += 2.5;
            row += 1;
        }
    }

    // Деревья по снимку — первыми, по дереву на крону и размером с неё (см.
    // [`Hint::crowns`]): в лесу и за его краем, на лугу и у реки. В OSM одиноких
    // деревьев и редколесья по склонам почти нет. В городе тёмное на снимке — и
    // тени домов, так что там кроны встают только на зелёном OSM (лес, парк,
    // газон) и не на мощёном. Без них в городском лесу не росло ни дерева: лес со
    // снимком деревьев не сажает, ждёт крон (Тайницкий сад Кремля был газоном).
    let mut planted_crowns: Vec<([f32; 2], f32)> = Vec::new();
    let town_green: Option<Vec<bool>> = (!rural && hint.is_some()).then(|| {
        let mut green = vec![false; m.rooms.w * m.rooms.h];
        let leafy = |t: &Tags| growth(t).is_some() || matches!(ground(t), Some((g, _)) if matches!(g, "meadow" | "rough_grass" | "straw_grass"));
        let hard = |t: &Tags| plaza(t) || building(t).is_some() || matches!(ground(t), Some((g, _)) if matches!(g, "stone_tiles" | "street" | "flagstones"));
        for (on, a) in areas.iter().filter_map(|a| if leafy(a.tags) { Some((true, a)) } else { None }).chain(areas.iter().filter(|a| hard(a.tags)).map(|a| (false, a))) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.5 {
                    green[k] = on;
                }
            }
        }
        green
    });
    if let Some(hint) = hint.as_ref() {
        let mut rng = Rng::new("osm/cover-trees");
        let woods: Vec<&Area> = areas.iter().filter(|a| growth(a.tags).is_some_and(|g| g.litter)).collect();
        let orchards: Vec<&Area> = areas.iter().filter(|a| get(a.tags, "landuse") == "orchard").collect();
        let mut crowns: Vec<Spot> = Vec::new();
        let res = mask::RES as f32;
        for Spot { q, mut r, conifer, queue } in hint.trees(w as usize, h as usize, cell) {
            if blocked.at(q) || hint.share(q, r.min(2.0), Class::Water) > 0.3 {
                continue;
            }
            if let Some(green) = &town_green
                && !green[(q[1] * res) as usize * m.rooms.w + (q[0] * res) as usize]
            {
                continue;
            }
            // Крона нависает над дорогой и оградой (на снимке полог над дорогой
            // смыкается), над домом и водой — не дальше половины радиуса.
            let hits = |r: f32| (0..8).any(|i| hard.at([q[0] + 0.5 * r * (i as f32 * FRAC_PI_4).cos(), q[1] + 0.5 * r * (i as f32 * FRAC_PI_4).sin()]));
            while r >= CROWN_MIN && hits(r) {
                r *= 0.85;
            }
            if r >= CROWN_MIN {
                crowns.push(Spot { q, r, conifer, queue });
            }
        }
        // Мелкие снизу: большое дерево выше и ложится поверх.
        crowns.sort_by(|a, b| a.r.total_cmp(&b.r));
        for Spot { q, r, conifer, queue } in crowns {
            if planted >= MAX_PLANTS {
                break;
            }
            // Порода: по карте лесов, если анализ снимка её знал; иначе в лесу OSM —
            // по его `leaf_type`, долей хвойных из очереди (куртинами, тёмные первыми);
            // за лесом — лиственное. Ели наугад вставали тёмными звёздами в ясеневой
            // роще Малхема и по одной среди дубов Болтона.
            let wood = woods.iter().find(|a| inside(&a.rings, q)).and_then(|a| growth(a.tags));
            // Сад — оливы: в сухом краю всякий, в зелёном — по тегу (`trees=olive_trees`).
            let olive = orchards.iter().any(|a| inside(&a.rings, q) && (arid || get(a.tags, "trees").contains("olive") || get(a.tags, "crop") == "olive"));
            let slot = if olive && r >= CROWN_BUSH { "olivo" } else { crown_slot(r, conifer, wood.as_ref(), queue, arid) };
            if missing(slot) {
                continue;
            }
            drop_crown(&mut m, slot, q, rng.int(0, 359) as f32, r);
            planted += 1;
            planted_crowns.push((q, r));
            spacing.add(q);
            // Под кроной ничего не сажать: мелочь легла бы поверх неё.
            blocked.disk(q, r * 0.95);
            if r >= CROWN_BUSH && wood.is_some() {
                litter.push((q, r));
            }
        }
    }

    let mut sown: Vec<(u8, usize, Growth)> = areas.iter().enumerate().filter_map(|(k, a)| growth(a.tags).map(|g| (g.rank, k, g))).collect();
    sown.sort_by_key(|s| (s.0, s.1));
    for (n, (rank, k, g)) in sown.iter().enumerate() {
        let a = &areas[*k];
        let (k, rank) = (*k, *rank);
        // Старшие контуры, что могут лежать поверх этого.
        let above: Vec<&Area> = sown[..n].iter().filter(|s| s.0 < rank).map(|s| &areas[s.1]).collect();
        let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
        let mut rng = Rng::new(&format!("osm/plant/{k}"));
        let tries = ((hi[0] - lo[0]) * (hi[1] - lo[1]) * 1.5) as usize;
        for _ in 0..tries {
            if planted >= MAX_PLANTS {
                break;
            }
            let q = [lo[0] + rng.random() as f32 * (hi[0] - lo[0]), lo[1] + rng.random() as f32 * (hi[1] - lo[1])];
            if blocked.at(q) {
                continue;
            }
            // Лес местами гуще и реже, с полянами в папоротнике: на снимке ровной
            // сетки крон не бывает. Пустошь и кустарник — куртинами. Со снимком
            // поляна — где на нём просвет.
            let glade = g.litter
                && match &hint {
                    Some(h) => h.share(q, 1.5, Class::Trees) < 0.3,
                    None => noise(q, 9.0, 8) > 0.45,
                };
            let local = if glade {
                2.2
            } else if g.litter {
                g.spacing * (1.0 + 0.3 * noise(q, 14.0, 7))
            } else {
                g.spacing
            };
            if !spacing.free(q, local) || !inside(&a.rings, q) || above.iter().any(|b| inside(&b.rings, q)) {
                continue;
            }
            spacing.add(q);
            let share = if glade {
                0.35
            } else if g.litter {
                g.share
            } else if g.rank >= 3 && g.rank <= 4 {
                // Пустошь и болото — куртинами между открытыми местами: ровный
                // «горох» кустов по всей пустоши против снимка выглядел узором.
                // Со снимком — где на нём бурое (орляк, вереск); деревья уже стоят.
                let c = match &hint {
                    Some(h) => ((h.share(q, 1.2, Class::Brown) - 0.25) / 0.4).clamp(0.0, 1.0),
                    None => ((noise(q, 11.0, 9) + 0.1) / 0.45).clamp(0.0, 1.0),
                };
                (g.share * 1.9 * (c * c * (3.0 - 2.0 * c)) as f64).clamp(0.0, 0.95)
            } else {
                (g.share * (1.0 + 0.8 * noise(q, 10.0, 9) as f64)).clamp(0.0, 1.0)
            };
            if !rng.chance(share) {
                continue;
            }
            let slot = arid_slot(
                if glade {
                    if rng.chance(0.7) { "fern" } else { "bush_summer" }
                } else {
                    g.slots[rng.int(0, g.slots.len() as i32 - 1) as usize]
                },
                arid,
            );
            // Со снимком деревья уже стоят по его кронам: лишние мелкие у большой
            // кроны дробили её.
            if hint.is_some() && (slot.starts_with("tree") || slot == "pine") {
                continue;
            }
            if missing(slot) {
                continue;
            }
            let scale = g.scale.0 + rng.random() as f32 * (g.scale.1 - g.scale.0);
            if slot == "boulder" {
                // Камни одной породы: вперемешку с синими и рудными выходила пёстрая
                // галька. На пустоши — мшистые, на скале и осыпи — по тегу `rock`.
                let tone = if growth_rock(a.tags) { rock_kind(a.tags).1 } else { "mossy" };
                drop_pick(&mut m, &asset_toned("boulder", tone), q, rng.int(0, 359) as f32, scale);
            } else {
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
            }
            // Папоротник ложится в картинку карты, тайла у него нет — в предел не идёт.
            if !slot.starts_with("fern") {
                planted += 1;
            }
            if g.litter && (slot.starts_with("tree") || slot == "pine") {
                litter.push((q, 2.4));
            }
        }
    }

    // Ничья земля за городом — ни луга, ни поля, ни леса в OSM: выгон и неудобья.
    // Одинокие кусты и деревья, местами купами; голый ковёр травы против снимка
    // выглядел пустым. Луг, поле, парк и кладбище не трогаются.
    if rural {
        let mw = m.rooms.w;
        let mut claimed = vec![false; mw * m.rooms.h];
        for a in areas.iter().filter(|a| ground(a.tags).is_some() || growth(a.tags).is_some() || building(a.tags).is_some()) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.0 {
                    claimed[k] = true;
                }
            }
        }
        let res = mask::RES as f32;
        let mut rng = Rng::new("osm/lone");
        for _ in 0..(w * h) as usize / 3 {
            if planted >= MAX_PLANTS {
                break;
            }
            let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
            if blocked.at(q) || claimed[(q[1] * res) as usize * mw + (q[0] * res) as usize] || !spacing.free(q, 4.5) {
                continue;
            }
            spacing.add(q);
            // Со снимком деревья и кусты уже стоят по его пятнам (куст — мелкое
            // пятно); здесь — редкие одиночные. Кусты по краю крон кольцом
            // обсаживали большое дерево мелочью.
            let edge = hint.as_ref().map(|h| h.share(q, 1.5, Class::Trees));
            let chance = match edge {
                Some(_) => 0.01,
                None => {
                    let copse = noise(q, 16.0, 11).max(0.0);
                    0.06 + 0.5 * copse * copse
                }
            };
            if !rng.chance(chance as f64) {
                continue;
            }
            let tree = edge.is_none() && rng.chance(0.45);
            let slot = if tree { "tree_green" } else { arid_slot("bush_summer", arid) };
            if missing(slot) {
                continue;
            }
            let scale = if tree { 1.0 + rng.random() as f32 * 0.5 } else { 0.7 + rng.random() as f32 * 0.5 };
            drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
            planted += 1;
        }
        // Верховое болото: на тёмных кочках (бурое по снимку) — кустарнички и
        // изредка низкая сосенка, на светлом мху — пучки осоки. Голый мох на
        // боевой карте читался ровным ковром, а на снимке он весь в кочках.
        if let Some(hint) = hint.as_ref().filter(|_| bog) {
            let mut rng = Rng::new("osm/bog");
            for _ in 0..(w * h) as usize {
                if planted >= MAX_PLANTS {
                    break;
                }
                let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
                if blocked.at(q) || claimed[(q[1] * res) as usize * mw + (q[0] * res) as usize] || !spacing.free(q, 1.6) {
                    continue;
                }
                let hummock = hint.share(q, 1.0, Class::Brown);
                if hint.share(q, 0.8, Class::Water) > 0.0 || !rng.chance((0.03 + 0.3 * hummock) as f64) {
                    continue;
                }
                let (slot, scale) = if hummock > 0.5 && rng.chance(0.12) {
                    ("fir", 0.55 + rng.random() as f32 * 0.25)
                } else if hummock > 0.3 {
                    ("bush_summer", 0.4 + rng.random() as f32 * 0.25)
                } else {
                    ("tussock", 0.5 + rng.random() as f32 * 0.3)
                };
                if missing(slot) {
                    continue;
                }
                spacing.add(q);
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
                planted += 1;
            }
        }
        // Рощи по желанию (`Options::trees`): куртины деревьев и кустов на свободной
        // суше — не на воде, пляже, камне, в домах и на тропах, не там, где снимок
        // видит воду, камень или снег. Порода — как у крон снимка (доля хвойных среди
        // крон с известной породой), в сухом краю — каменный дуб, на болоте — низкие
        // сосенки; на снегу и льду не растёт ничего.
        if opt.trees > 0.0 && !cold {
            let mut bare = vec![false; mw * m.rooms.h];
            for a in areas.iter() {
                let hard = building(a.tags).is_some()
                    || matches!(ground(a.tags), Some((t, _)) if t.starts_with("water") || t.starts_with("sea") || t.starts_with("rock_") || matches!(t, "sand" | "shingle" | "gravel" | "stone_tiles" | "street" | "flagstones"));
                if hard {
                    for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                        bare[k] |= v > 0.3;
                    }
                }
            }
            let known: Vec<&Crown> = hint.as_ref().and_then(|h| h.cover.crowns.as_ref()).map(|c| c.iter().filter(|c| c.src != "none" && c.kind != "bush").collect()).unwrap_or_default();
            let needles = if known.len() >= 5 { known.iter().filter(|c| c.kind == "conifer").count() as f32 / known.len() as f32 } else { 0.15 };
            let mut rng = Rng::new("osm/grove");
            for _ in 0..(w * h) as usize {
                if planted >= MAX_PLANTS {
                    break;
                }
                let q = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
                if blocked.at(q) || bare[(q[1] * res) as usize * mw + (q[0] * res) as usize] {
                    continue;
                }
                // Галька у берега на снимке — «камень»: на ней не растёт, но лишь
                // где её больше половины; вода, снег и лёд — уже с четверти.
                if let Some(h) = &hint
                    && ([Class::Water, Class::Snow, Class::Ice].iter().any(|&c| h.share(q, 1.0, c) > 0.25) || h.share(q, 1.0, Class::Rock) > 0.5)
                {
                    continue;
                }
                let copse = (noise(q, 8.0, 41) * 0.5 + 0.5).clamp(0.0, 1.0);
                if !rng.chance((opt.trees * (0.03 + 0.5 * copse.powf(2.5) * 2.0)) as f64) {
                    continue;
                }
                // В гуще куртины — деревья, по её краю и вразброс — кусты.
                let tree = rng.chance((0.25 + 0.75 * copse) as f64);
                let (slot, scale, gap) = if !tree {
                    (if bog { "bush_summer" } else { arid_slot("bush_summer", arid) }, 0.6 + rng.random() as f32 * 0.4, 1.6)
                } else if bog {
                    ("fir", 0.55 + rng.random() as f32 * 0.25, 2.0)
                } else if arid {
                    ("encina", 0.9 + rng.random() as f32 * 0.5, 2.8)
                } else if rng.chance(needles as f64) {
                    let r = 1.6 + rng.random() as f32 * 1.0;
                    (conifer_slot(r), 0.8 + rng.random() as f32 * 0.4, 2.4)
                } else {
                    ("tree_green", 1.0 + rng.random() as f32 * 0.5, 2.8)
                };
                if missing(slot) || !spacing.free(q, gap) {
                    continue;
                }
                spacing.add(q);
                drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
                planted += 1;
                if tree && !bog {
                    litter.push((q, 2.4));
                }
            }
        }
    }

    // Кувшинки — купами у берега стоячей воды, где мелко, как в заводях на снимке.
    // Только из паков: встроенных нет. На реке их сносит течением.
    if !missing("lilypad") && !bog {
        let mut rng = Rng::new("osm/lily");
        for a in areas.iter().filter(|a| get(a.tags, "natural") == "water" && !matches!(get(a.tags, "water"), "river" | "canal" | "stream" | "lock" | "wastewater" | "fountain" | "sea" | "bay" | "lagoon")) {
            let Some((lo, hi)) = clipped_box(&a.rings, w as f32, h as f32) else { continue };
            let edge = |p: [f32; 2]| a.rings.iter().flat_map(|r| (0..r.len()).map(move |k| (r[k], r[(k + 1) % r.len()]))).map(|(u, v)| segment_distance(p, u, v)).fold(f32::INFINITY, f32::min);
            let cells = (hi[0] - lo[0]) * (hi[1] - lo[1]);
            let mut beds = (cells / 900.0).clamp(1.0, 10.0) as usize;
            for _ in 0..beds * 40 {
                if beds == 0 {
                    break;
                }
                let c = [lo[0] + rng.random() as f32 * (hi[0] - lo[0]), lo[1] + rng.random() as f32 * (hi[1] - lo[1])];
                if !inside(&a.rings, c) || !(1.0..2.6).contains(&edge(c)) {
                    continue;
                }
                beds -= 1;
                // Купа в 2–3 клетки, листья с клетку: мельче на карте их не видно.
                for _ in 0..rng.int(8, 16) {
                    let p = [c[0] + (rng.random() as f32 - 0.5) * 5.0, c[1] + (rng.random() as f32 - 0.5) * 5.0];
                    if inside(&a.rings, p) && edge(p) > 0.4 && m.contains(p[0].floor() as i32, p[1].floor() as i32) {
                        drop(&mut m, "lilypad", p, rng.int(0, 359) as f32, 0.8 + rng.random() as f32 * 0.5);
                    }
                }
            }
        }
    }
    if !litter.is_empty() {
        let f = floors.get(&mut m, "forest_floor");
        let weights = blobs(&m, &litter, 0.15, 0.8);
        m.paint(Layer::Floor, f, &weights);
    }
    if planted >= MAX_PLANTS {
        warnings.push(format!("деревьев и кустов больше {MAX_PLANTS}: часть леса не засажена"));
    }

    // Камни-переходы — одного вида на всю карту: у FA их пять пород, вперемешку пестро.
    let stone = asset("stepping_stone");
    for (k, &p) in stones.iter().enumerate() {
        if m.contains(p[0].floor() as i32, p[1].floor() as i32) {
            let mut rng = Rng::new(&format!("osm/stepping/{k}"));
            drop_pick(&mut m, &stone, p, rng.int(0, 359) as f32, 0.7);
        }
    }

    // Камни по точкам.
    let mut rocks: Vec<(i64, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| matches!(get(&n.tags, "natural"), "rock" | "stone") && !unseen(&n.tags)).map(|(id, n)| (*id, proj.at(n.lat, n.lon))).collect();
    rocks.sort_by_key(|r| r.0);
    // Порода камней — как у скал на карте.
    let tone = areas.iter().find(|a| growth_rock(a.tags)).map_or("earthy", |a| rock_kind(a.tags).1);
    for (id, p) in rocks {
        if m.contains(p[0].floor() as i32, p[1].floor() as i32) {
            let mut rng = Rng::new(&format!("osm/rock/{id}"));
            let scale = 1.0 + rng.random() as f32 * 0.6;
            drop_pick(&mut m, &asset_toned("boulder", tone), p, rng.int(0, 359) as f32, scale);
        }
    }
    // Осыпь под обрывами — той же породы; тег породы у самого обрыва главнее.
    let cliff_tone = osm
        .ways
        .values()
        .find(|way| get(&way.tags, "natural") == "cliff" && ["rock", "geological", "surface"].iter().any(|k| !get(&way.tags, k).is_empty()))
        .map_or(tone, |way| rock_kind(&way.tags).1);
    cliff_scree(&mut m, &hard, cliff_tone);

    // Деревья: отдельные и рядами вдоль улиц.
    let mut trees: Vec<(i64, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| get(&n.tags, "natural") == "tree" && !unseen(&n.tags)).map(|(id, n)| (*id, proj.at(n.lat, n.lon))).collect();
    for (id, way) in &osm.ways {
        if get(&way.tags, "natural") != "tree_row" {
            continue;
        }
        let pts: Vec<[f32; 2]> = way.nodes.iter().filter_map(|n| osm.nodes.get(n)).map(|n| proj.at(n.lat, n.lon)).collect();
        let step = m_to_cells(8.0);
        for (k, s) in pts.windows(2).enumerate() {
            let n = (length(s[0], s[1]) / step).floor() as usize;
            for q in 0..=n {
                let f = q as f32 / (n.max(1)) as f32;
                trees.push((id * 1000 + (k * 100 + q) as i64, [s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f]));
            }
        }
    }
    trees.sort_by_key(|t| t.0);
    let res = mask::RES as f32;
    let outdoors = |m: &Map, p: [f32; 2]| m.contains(p[0].floor() as i32, p[1].floor() as i32) && m.rooms.get((p[0] * res).floor() as isize, (p[1] * res).floor() as isize) < 128;
    for (id, p) in trees {
        // Дерево OSM в кроне со снимка — то же дерево: второе, мельче, вставало
        // рядом с ним или на нём.
        if !outdoors(&m, p) || planted_crowns.iter().any(|(c, r)| length(*c, p) < r.max(2.0)) {
            continue;
        }
        let mut rng = Rng::new(&format!("osm/tree/{id}"));
        let rot = rng.int(0, 359) as f32;
        let scale = 1.2 + rng.random() as f32 * 0.4;
        drop(&mut m, "tree_green", p, rot, scale);
    }

    // Уличное по точкам: фонари, скамьи (вдоль ближайшей улицы), фонтаны, памятники.
    let mut things: Vec<(i64, &'static str, [f32; 2])> = osm.nodes.iter().filter(|(_, n)| !unseen(&n.tags)).filter_map(|(id, n)| Some((*id, furniture(&n.tags)?, proj.at(n.lat, n.lon)))).collect();
    things.sort_by_key(|t| t.0);
    for (_, slot, p) in things {
        if !outdoors(&m, p) {
            continue;
        }
        let along = streets.iter().chain(&walks).flat_map(|l| l.windows(2)).min_by(|x, y| segment_distance(p, x[0], x[1]).total_cmp(&segment_distance(p, y[0], y[1])));
        let rot = match (slot, along) {
            ("bench", Some(s)) => (s[1][1] - s[0][1]).atan2(s[1][0] - s[0][0]).to_degrees().rem_euclid(180.0),
            _ => 0.0,
        };
        drop(&mut m, slot, p, rot, 1.0);
    }

    // Подписи для ведущего: улицы вдоль, большие здания посередине.
    for (name, (a, b, width)) in street_names {
        let mut l = Label::new(name, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
        l.size = (width * 0.4).clamp(0.6, 2.5);
        l.rot = upright(a, b);
        l.gm = true;
        m.labels.push(l);
    }
    for (name, (c, cells)) in names {
        let mut l = Label::new(name, c);
        l.size = (cells.sqrt() * 0.08).clamp(1.0, 3.0);
        l.gm = true;
        m.labels.push(l);
    }
    m.labels.retain(|l| l.pos[0] >= 0.0 && l.pos[1] >= 0.0 && l.pos[0] < w as f32 && l.pos[1] < h as f32);
    if houses.is_empty() && m.paths.is_empty() {
        warnings.push("в выгрузке нет ни домов, ни улиц".into());
    }
    Ok((m, Report { count: houses.len(), warnings }))
}

/// Как далеко от дома ещё ищется улица для двери; дальше — к дорожке.
const STREET_REACH: f32 = 12.0;

/// Стены домов: вид наружных по дому, общие стены соседей, двери.
fn walls_and_doors(m: &mut Map, houses: &[House], streets: &[Vec<[f32; 2]>], walks: &[Vec<[f32; 2]>]) {
    let res = mask::RES as f32;
    let (w, h) = (m.width as f32, m.height as f32);
    let edges = m.room_edges();
    let rooms = m.rooms.clone();
    let inside = |p: [f32; 2]| rooms.get((p[0] * res).floor() as isize, (p[1] * res).floor() as isize) >= 128;
    let on_border = |s: &Segment| {
        let edge = |v: f32, lim: f32| v.abs() < 0.05 || (v - lim).abs() < 0.05;
        (edge(s.0[0], w) && edge(s.1[0], w) && (s.0[0] - s.1[0]).abs() < 0.05) || (edge(s.0[1], h) && edge(s.1[1], h) && (s.0[1] - s.1[1]).abs() < 0.05)
    };
    let mut shared: BTreeSet<[i32; 4]> = BTreeSet::new();
    for house in houses {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in house.rings.iter().flatten() {
            for c in 0..2 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        let near_ring = |p: [f32; 2]| house.rings.iter().any(|r| (0..r.len()).any(|k| segment_distance(p, r[k], r[(k + 1) % r.len()]) < 0.25));
        let own: Vec<Segment> = edges
            .iter()
            .copied()
            .filter(|s| {
                let mid = [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
                mid[0] >= lo[0] - 0.5 && mid[0] <= hi[0] + 0.5 && mid[1] >= lo[1] - 0.5 && mid[1] <= hi[1] + 0.5 && near_ring(mid)
            })
            .collect();
        m.set_edge_look(&own, &house.style);

        // Общая стена: по обе стороны отрезка контура дома — комнаты.
        for r in &house.rings {
            for k in 0..r.len() {
                let (a, b) = (r[k], r[(k + 1) % r.len()]);
                let len = length(a, b);
                let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                if len < 0.25 || !m.contains(mid[0].floor() as i32, mid[1].floor() as i32) {
                    continue;
                }
                let n = [-(b[1] - a[1]) / len * 0.5, (b[0] - a[0]) / len * 0.5];
                if !(inside([mid[0] + n[0], mid[1] + n[1]]) && inside([mid[0] - n[0], mid[1] - n[1]])) {
                    continue;
                }
                let q = |p: [f32; 2]| [(p[0] * 100.0).round() as i32, (p[1] * 100.0).round() as i32];
                let (qa, qb) = (q(a), q(b));
                let key = if qa <= qb { [qa[0], qa[1], qb[0], qb[1]] } else { [qb[0], qb[1], qa[0], qa[1]] };
                if shared.insert(key) {
                    m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), low: false, style: Some(house.style.clone()) });
                }
            }
        }

        // Двери: у отмеченных входов, иначе одна — к ближайшей улице.
        if !house.doors {
            continue;
        }
        let doorable: Vec<Segment> = own.iter().copied().filter(|s| length(s.0, s.1) >= 1.4 && !on_border(s)).collect();
        if doorable.is_empty() {
            continue;
        }
        let mut spots: Vec<[f32; 2]> = Vec::new();
        for &e in &house.entrances {
            if spots.len() < 6 && spots.iter().all(|s| length(*s, e) >= 2.5) {
                spots.push(e);
            }
        }
        if spots.is_empty() {
            let mid = |s: &Segment| [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
            let dist = |lines: &[Vec<[f32; 2]>], p: [f32; 2]| lines.iter().flat_map(|l| l.windows(2)).map(|s| segment_distance(p, s[0], s[1])).fold(f32::INFINITY, f32::min);
            // Улица рядом — дверь к ней, а не во двор к дорожке; улицы нет — к дорожке.
            let near_street = doorable.iter().any(|s| dist(streets, mid(s)) < STREET_REACH);
            let lines = if near_street || walks.is_empty() { streets } else { walks };
            let best = doorable.iter().min_by(|x, y| dist(lines, mid(x)).total_cmp(&dist(lines, mid(y))).then(length(y.0, y.1).total_cmp(&length(x.0, x.1))));
            if let Some(s) = best {
                spots.push([(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0]);
            }
        }
        for p in spots {
            let Some(&(a, b)) = doorable.iter().min_by(|x, y| segment_distance(p, x.0, x.1).total_cmp(&segment_distance(p, y.0, y.1))) else { continue };
            if segment_distance(p, a, b) > 1.5 {
                continue;
            }
            let len = length(a, b);
            let width = if house.grand && len >= 3.4 { 2.0 } else { 1.0 };
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let t = ((p[0] - a[0]) * dir[0] + (p[1] - a[1]) * dir[1]).clamp(width / 2.0 + 0.2, len - width / 2.0 - 0.2);
            let at = |s: f32| [a[0] + dir[0] * s, a[1] + dir[1] * s];
            let kind = if width >= 2.0 { PortalKind::DoubleDoor } else { PortalKind::Door };
            m.portals.push(portal(at(t - width / 2.0), at(t + width / 2.0), kind, house.wood));
        }
    }
}

#[cfg(test)]
#[path = "osm/tests.rs"]
mod tests;
