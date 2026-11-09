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
use crate::map::segment_distance;
use std::collections::{BTreeSet, HashMap};

/// Метров в клетке D&D: 5 футов.
pub const CELL_M: f32 = 1.524;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Метров в клетке.
    pub cell_m: f32,
    /// Наибольшая сторона карты в клетках: участок крупнее ужимается.
    pub max_cells: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { cell_m: CELL_M, max_cells: 256 }
    }
}

type Tags = BTreeMap<String, String>;

struct Node {
    lat: f64,
    lon: f64,
    tags: Tags,
}

struct Way {
    nodes: Vec<i64>,
    tags: Tags,
}

struct Relation {
    /// Пути-участники: номер и роль (`outer`, `inner`).
    ways: Vec<i64>,
    tags: Tags,
}

struct Osm {
    /// Юг, запад, север, восток.
    bounds: [f64; 4],
    nodes: HashMap<i64, Node>,
    ways: BTreeMap<i64, Way>,
    relations: BTreeMap<i64, Relation>,
}

fn tags_of(n: roxmltree::Node) -> Tags {
    n.children().filter(|c| c.has_tag_name("tag")).filter_map(|c| Some((c.attribute("k")?.to_owned(), c.attribute("v")?.to_owned()))).collect()
}

fn number(n: roxmltree::Node, attr: &str) -> Option<f64> {
    n.attribute(attr)?.parse().ok()
}

fn ident(n: roxmltree::Node) -> Option<i64> {
    n.attribute("id")?.parse().ok()
}

fn parse(xml: &str) -> Result<Osm, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("не читается как OSM: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "osm" {
        return Err("это не выгрузка OpenStreetMap (.osm)".into());
    }
    let mut osm = Osm { bounds: [f64::NAN; 4], nodes: HashMap::new(), ways: BTreeMap::new(), relations: BTreeMap::new() };
    let mut bounds = None;
    for n in root.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "bounds" => bounds = Some([number(n, "minlat"), number(n, "minlon"), number(n, "maxlat"), number(n, "maxlon")]),
            "node" => {
                if let (Some(id), Some(lat), Some(lon)) = (ident(n), number(n, "lat"), number(n, "lon")) {
                    osm.nodes.insert(id, Node { lat, lon, tags: tags_of(n) });
                }
            }
            "way" => {
                if let Some(id) = ident(n) {
                    let nodes = n.children().filter(|c| c.has_tag_name("nd")).filter_map(|c| c.attribute("ref")?.parse().ok()).collect();
                    osm.ways.insert(id, Way { nodes, tags: tags_of(n) });
                }
            }
            "relation" => {
                if let Some(id) = ident(n) {
                    let ways = n
                        .children()
                        .filter(|c| c.has_tag_name("member") && c.attribute("type") == Some("way") && matches!(c.attribute("role"), Some("outer" | "inner" | "")))
                        .filter_map(|c| c.attribute("ref")?.parse().ok())
                        .collect();
                    osm.relations.insert(id, Relation { ways, tags: tags_of(n) });
                }
            }
            _ => {}
        }
    }
    osm.bounds = match bounds {
        Some([Some(s), Some(w), Some(n), Some(e)]) if n > s && e > w => [s, w, n, e],
        _ => {
            if osm.nodes.is_empty() {
                return Err("в выгрузке нет ни одной точки".into());
            }
            let lat = osm.nodes.values().map(|n| n.lat);
            let lon = osm.nodes.values().map(|n| n.lon);
            [lat.clone().fold(f64::MAX, f64::min), lon.clone().fold(f64::MAX, f64::min), lat.fold(f64::MIN, f64::max), lon.fold(f64::MIN, f64::max)]
        }
    };
    Ok(osm)
}

fn get<'a>(t: &'a Tags, k: &str) -> &'a str {
    t.get(k).map_or("", String::as_str)
}

/// Под землёй, в тоннеле или внутри здания: на карте сверху этого не видно.
fn unseen(t: &Tags) -> bool {
    let below = |k: &str| get(t, k).split(';').filter_map(|v| v.trim().parse::<f32>().ok()).any(|v| v < 0.0);
    !matches!(get(t, "tunnel"), "" | "no") || below("layer") || below("level") || matches!(get(t, "location"), "underground" | "indoor") || !matches!(get(t, "indoor"), "" | "no")
}

/// Этаж линии для порядка рисования: мост поверх того, что под ним.
fn layer(t: &Tags) -> f32 {
    let l = get(t, "layer").trim().parse::<f32>().unwrap_or(0.0);
    if matches!(get(t, "bridge"), "" | "no") { l } else { l.max(1.0) }
}

/// Ширина из тега `width` в метрах, если она правдоподобна.
fn width_tag(t: &Tags) -> Option<f32> {
    get(t, "width").trim().trim_end_matches('m').trim().parse::<f32>().ok().filter(|w| (0.3..80.0).contains(w))
}

/// Широта и долгота — в клетки карты: равнопромежуточная проекция у середины
/// участка; на паре километров ошибка меньше клетки.
struct Proj {
    north: f64,
    west: f64,
    kx: f64,
    ky: f64,
}

impl Proj {
    fn at(&self, lat: f64, lon: f64) -> [f32; 2] {
        [((lon - self.west) * self.kx) as f32, ((self.north - lat) * self.ky) as f32]
    }
}

/// Замкнутые кольца из кусков: пути мультиполигона цепляются концами.
/// Незамкнувшееся (часть участников за краем выгрузки) отбрасывается.
fn assemble(parts: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
    let mut out = Vec::new();
    let mut open: Vec<Vec<i64>> = Vec::new();
    for w in parts {
        if w.len() >= 4 && w.first() == w.last() {
            out.push(w);
        } else if w.len() >= 2 {
            open.push(w);
        }
    }
    while let Some(mut cur) = open.pop() {
        loop {
            if cur.len() >= 4 && cur.first() == cur.last() {
                out.push(cur);
                break;
            }
            let end = cur[cur.len() - 1];
            let Some(k) = open.iter().position(|w| w[0] == end || w[w.len() - 1] == end) else { break };
            let mut w = open.swap_remove(k);
            if w[0] != end {
                w.reverse();
            }
            cur.extend_from_slice(&w[1..]);
        }
    }
    out
}

/// Площадной объект: кольца в клетках (без повтора первой точки) и узлы колец.
struct Area<'a> {
    rings: Vec<Vec<[f32; 2]>>,
    nodes: Vec<Vec<i64>>,
    tags: &'a Tags,
}

fn areas<'a>(osm: &'a Osm, proj: &Proj) -> Vec<Area<'a>> {
    // Кольца точек и узлов идут парами по номеру: по узлам ищутся общие точки соседей.
    let to_area = |rings: Vec<Vec<i64>>, tags: &'a Tags| {
        let (mut pts, mut ids) = (Vec::new(), Vec::new());
        for r in rings {
            let p: Vec<[f32; 2]> = r[..r.len() - 1].iter().filter_map(|id| osm.nodes.get(id)).map(|n| proj.at(n.lat, n.lon)).collect();
            if p.len() >= 3 {
                pts.push(p);
                ids.push(r);
            }
        }
        (!pts.is_empty()).then_some(Area { rings: pts, nodes: ids, tags })
    };
    let mut out = Vec::new();
    for w in osm.ways.values() {
        if w.nodes.len() >= 4 && w.nodes.first() == w.nodes.last() && !w.tags.is_empty() && !unseen(&w.tags) {
            out.extend(to_area(vec![w.nodes.clone()], &w.tags));
        }
    }
    for r in osm.relations.values() {
        if get(&r.tags, "type") != "multipolygon" || unseen(&r.tags) {
            continue;
        }
        let parts = r.ways.iter().filter_map(|id| osm.ways.get(id)).map(|w| w.nodes.clone()).collect();
        out.extend(to_area(assemble(parts), &r.tags));
    }
    out
}

/// Что за пол у площадного объекта: текстура и порядок слоя (выше — поверх).
/// У леса своего пола нет: подстилка ложится пятнами под деревьями (`plant`).
/// Глубокая вода ложится с мелким краем (`WATER_EDGE`). Пляж без покрытия вдали
/// от моря (`sea` — в выгрузке есть берег моря) — галька: у реки песка почти не бывает.
fn ground(t: &Tags, sea: bool) -> Option<(&'static str, u8)> {
    let (landuse, leisure, natural, amenity) = (get(t, "landuse"), get(t, "leisure"), get(t, "natural"), get(t, "amenity"));
    if matches!(leisure, "swimming_pool") || matches!(amenity, "fountain") {
        return Some(("water_shallow", 4));
    }
    if natural == "water" || matches!(landuse, "basin" | "reservoir") || get(t, "waterway") == "riverbank" {
        return Some(("water_deep", 4));
    }
    if natural == "wetland" {
        return Some(("swamp", 3));
    }
    if matches!(natural, "bare_rock" | "scree" | "stone") {
        return Some(("rocky_ground", 3));
    }
    let surface = get(t, "surface");
    if natural == "shingle" || (natural == "beach" && (matches!(surface, "gravel" | "pebblestone" | "shingle" | "stone") || (surface.is_empty() && !sea))) {
        return Some(("shingle", 3));
    }
    if matches!(natural, "sand" | "beach" | "dune") {
        return Some(("sand", 3));
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
    if get(t, "railway") == "platform" || (get(t, "highway") == "pedestrian" && get(t, "area") == "yes") || get(t, "place") == "square" || t.contains_key("area:highway") {
        return Some(("stone_tiles", 5));
    }
    None
}

/// Сколько клеток у берега глубокого водоёма мелководье.
const WATER_EDGE: f32 = 2.0;

/// Шаг камней-переходов в клетках: человеку — шаг, фигурке — клетка.
const STEPPING: f32 = 0.9;

/// Озёра и реки одним слоем: мелководье по всей воде, глубина — дальше
/// `WATER_EDGE` от берега общей воды. Река из нескольких кусков в OSM иначе
/// получала мели поперёк русла на стыках; край карты берегом не считается —
/// вода уходит за него.
fn water(m: &mut Map, floors: &mut Floors, parts: &[&Area]) {
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    // Сумма, а не наибольшее: на общем крае двух кусков каждый даёт половину,
    // и по наибольшему вдоль шва оставалась полоса суши в полводы.
    let mut union = vec![0.0f32; mw * mh];
    for a in parts {
        for (i, v) in coverage(&m.rooms, &a.rings, 0.0) {
            union[i] = (union[i] + v).min(1.0);
        }
    }
    let weights: Vec<(usize, f32)> = union.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| (i, *v)).collect();
    if weights.is_empty() {
        return;
    }
    let shallow = floors.get(m, "water_shallow");
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
    let deep: Vec<(usize, f32)> = dist
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 0)
        .map(|(i, &d)| (i, (0.5 + ((d.min(far) as f32 / 3.0 / res) - WATER_EDGE) / (2.0 * aa)).clamp(0.0, 1.0)))
        .filter(|(_, v)| *v > 0.0)
        .collect();
    if !deep.is_empty() {
        let t = floors.get(m, "water_deep");
        m.paint(Layer::Floor, t, &deep);
    }
}

/// Что растёт на площадном объекте: слот, шаг между посадками в клетках, масштаб
/// (от и до) и доля мест, где правда посажено (вереск — редкие кусты).
struct Growth {
    slots: &'static [&'static str],
    spacing: f32,
    scale: (f32, f32),
    share: f64,
    /// Подстилка под кронами — у леса.
    litter: bool,
    /// Старшинство: контуры в OSM перекрываются (вереск накрывает и лес), и
    /// младший не сажает там, где лежит старший. Лес — 0.
    rank: u8,
}

fn growth(t: &Tags) -> Option<Growth> {
    let (landuse, natural) = (get(t, "landuse"), get(t, "natural"));
    if natural == "wood" || landuse == "forest" {
        // Хвойный — зелёные сосны, лиственный — летние, смешанный — вперемешку.
        let slots: &[&str] = match get(t, "leaf_type") {
            "needleleaved" => &["pine"],
            "broadleaved" => &["tree_green"],
            _ => &["tree_green", "tree_green", "pine"],
        };
        return Some(Growth { slots, spacing: 3.4, scale: (1.0, 1.6), share: 1.0, litter: true, rank: 0 });
    }
    if landuse == "orchard" {
        return Some(Growth { slots: &["tree_green"], spacing: 4.5, scale: (0.7, 0.9), share: 1.0, litter: false, rank: 1 });
    }
    if natural == "scrub" {
        return Some(Growth { slots: &["bush_summer", "bush_summer", "bush_summer", "tree_green"], spacing: 2.2, scale: (0.7, 1.2), share: 0.8, litter: false, rank: 2 });
    }
    if natural == "wetland" {
        return Some(Growth { slots: &["cattail", "cattail", "bush_summer"], spacing: 2.0, scale: (0.7, 1.1), share: 0.4, litter: false, rank: 3 });
    }
    if natural == "heath" {
        // Пустошь сверху — папоротник и вереск куртинами, кусты и редкие молодые деревья;
        // голая трава с камнями была пустой против снимка. Папоротник — только из паков.
        return Some(Growth { slots: &["fern", "fern", "fern", "bush_summer", "bush_summer", "tree_green", "pine", "boulder"], spacing: 2.6, scale: (0.6, 1.0), share: 0.5, litter: false, rank: 4 });
    }
    None
}

/// Рамка колец, обрезанная по карте; пустая — контур весь за краем.
fn clipped_box(rings: &[Vec<[f32; 2]>], w: f32, h: f32) -> Option<([f32; 2], [f32; 2])> {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in rings.iter().flatten() {
        for c in 0..2 {
            lo[c] = lo[c].min(p[c]);
            hi[c] = hi[c].max(p[c]);
        }
    }
    let (lo, hi) = ([lo[0].max(0.0), lo[1].max(0.0)], [hi[0].min(w), hi[1].min(h)]);
    (hi[0] > lo[0] && hi[1] > lo[1]).then_some((lo, hi))
}

/// Внутри колец (чётно-нечётно).
fn inside(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> bool {
    let mut odd = false;
    for r in rings {
        for k in 0..r.len() {
            let (a, b) = (r[k], r[(k + 1) % r.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                odd = !odd;
            }
        }
    }
    odd
}


/// Вид дома: пол, слот стены, деревянные ли двери.
fn building_look(kind: &str, t: &Tags) -> (&'static str, &'static str, bool) {
    let floor = match kind {
        "house" | "residential" | "apartments" | "terrace" | "detached" | "semidetached_house" | "hotel" | "dormitory" | "hut" | "cabin" | "shed" => "wood_planks",
        "church" | "cathedral" | "chapel" => "marble",
        "industrial" | "warehouse" | "garage" | "garages" => "flagstones",
        _ => "stone_tiles",
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

/// Дом ли это и какой: `building=*`, кроме навесов и недостроенного.
fn building(t: &Tags) -> Option<&str> {
    let b = get(t, "building");
    (!b.is_empty() && !matches!(b, "no" | "roof" | "construction" | "proposed" | "demolished" | "ruins")).then_some(b)
}

/// Чем выложена дорога: по тегу `surface`, без него — по виду дороги и месту
/// (в городе тротуар плиткой, за городом тропа — земля). Проезжая мощёная —
/// булыжник: асфальта в D&D нет. Утоптанный гравий — светлый, как на снимке;
/// травяная тропа — примятая бурая трава, а не земля.
fn paving(t: &Tags, hw: &str, rural: bool) -> &'static str {
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
fn natural_surface(tex: &str) -> bool {
    matches!(tex, "dirt" | "gravel" | "path_gravel" | "rough_grass" | "mud" | "sand")
}

/// Улица: ширина в метрах, текстура, порядок (выше — поверх).
fn road(t: &Tags, rural: bool) -> Option<(f32, &'static str, u8)> {
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
fn rail(t: &Tags) -> Option<(f32, bool)> {
    let ballast = match get(t, "railway") {
        "rail" | "light_rail" | "narrow_gauge" | "subway" | "disused" | "preserved" | "funicular" => true,
        "tram" => false,
        _ => return None,
    };
    let gauge = get(t, "gauge").split(';').next().and_then(|g| g.trim().parse::<f32>().ok()).filter(|g| (500.0..2000.0).contains(g)).unwrap_or(1435.0);
    Some((gauge / 1000.0, ballast))
}

/// Ручей, канал, река: ширина в метрах и глубокая ли вода.
fn stream(t: &Tags) -> Option<(f32, bool)> {
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
fn offset(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
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
fn near_map(pts: &[[f32; 2]], w: f32, h: f32, margin: f32) -> Vec<Vec<[f32; 2]>> {
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
fn coverage(mk: &Mask, rings: &[Vec<[f32; 2]>], inset: f32) -> Vec<(usize, f32)> {
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
fn noise(p: [f32; 2], scale: f32, seed: u32) -> f32 {
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
fn warp(p: [f32; 2]) -> [f32; 2] {
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
fn natural_outline(r: &[[f32; 2]], pinned: &[bool]) -> Vec<[f32; 2]> {
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
fn dense(line: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
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
fn centre(mk: &Mask, weights: &[(usize, f32)]) -> ([f32; 2], f32) {
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

fn length(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

/// Поворот подписи вдоль отрезка, чтобы читалась слева направо.
fn upright(a: [f32; 2], b: [f32; 2]) -> f32 {
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
fn longest(pts: &[[f32; 2]]) -> Option<([f32; 2], [f32; 2])> {
    pts.windows(2).map(|w| (w[0], w[1])).max_by(|x, y| length(x.0, x.1).total_cmp(&length(y.0, y.1)))
}

/// Пол по слоту — один вариант на карту: с паками FA каждая улица иначе брала бы
/// свою брусчатку.
#[derive(Default)]
struct Floors(BTreeMap<&'static str, usize>);

impl Floors {
    fn get(&mut self, m: &mut Map, slot: &'static str) -> usize {
        *self.0.entry(slot).or_insert_with(|| {
            let k = mat(m, slot);
            // Трава, земля и вода лежат полями на сотни клеток: без перемешивания
            // плитки (как у встроенных) по лугу идёт узор повтора.
            if matches!(slot, "grass_green" | "grass_dry" | "meadow" | "rough_grass" | "dirt" | "gravel" | "shingle" | "path_gravel" | "sand" | "mud" | "forest_floor" | "swamp" | "rocky_ground" | "water_shallow" | "water_deep") {
                m.materials[k].vary = true;
            }
            k
        })
    }
}

/// Уличный предмет по точке OSM: фонарь, скамья, фонтан, памятник.
fn furniture(t: &Tags) -> Option<&'static str> {
    if get(t, "highway") == "street_lamp" {
        return Some("street_lamp");
    }
    match get(t, "amenity") {
        "bench" => return Some("bench"),
        "fountain" => return Some("fountain"),
        _ => {}
    }
    let statue = matches!(get(t, "artwork_type"), "statue" | "sculpture") || matches!(get(t, "memorial"), "statue" | "war_memorial" | "obelisk");
    (statue && (get(t, "tourism") == "artwork" || !get(t, "historic").is_empty())).then_some("statue")
}

/// Ограда по линии OSM: слот стены, низкая ли (видно поверх), толщина в клетках.
/// За городом стена — сухая кладка из бута по пояс, в городе — высокая из блоков.
fn barrier(t: &Tags, rural: bool) -> Option<(&'static str, bool, f32)> {
    match get(t, "barrier") {
        "wall" | "retaining_wall" if rural => Some(("wall_fieldstone", true, 0.35)),
        "wall" | "city_wall" | "retaining_wall" => Some(("wall_stone_blocks", false, 0.3)),
        "fence" => Some(("wall_wood", true, 0.15)),
        "railing" | "guard_rail" | "handrail" => Some(("wall_metal", true, 0.12)),
        "hedge" => Some(("wall_hedge", false, 0.45)),
        _ if get(t, "natural") == "cliff" => Some(("wall_cave", false, 0.6)),
        _ => None,
    }
}

/// Проход в ограде и его ширина в клетках: ворота, калитка, перелаз.
fn gate(t: &Tags) -> Option<f32> {
    match get(t, "barrier") {
        "gate" | "lift_gate" | "swing_gate" | "sliding_gate" | "hampshire_gate" | "cattle_grid" => Some(2.0),
        "stile" | "kissing_gate" | "entrance" | "turnstile" | "bump_gate" => Some(1.0),
        _ => None,
    }
}

/// Ограда кусками: у калиток и ворот — разрыв в их ширину.
fn split_at_gates(pts: &[[f32; 2]], gaps: &[Option<f32>]) -> Vec<Vec<[f32; 2]>> {
    let toward = |p: [f32; 2], q: [f32; 2], d: f32| {
        let l = length(p, q);
        (l > d).then(|| [p[0] + (q[0] - p[0]) * d / l, p[1] + (q[1] - p[1]) * d / l])
    };
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    for (k, &p) in pts.iter().enumerate() {
        let Some(gap) = gaps[k] else {
            cur.push(p);
            continue;
        };
        if k > 0
            && let Some(q) = toward(p, pts[k - 1], gap / 2.0)
        {
            cur.push(q);
        }
        if cur.len() >= 2 {
            out.push(std::mem::take(&mut cur));
        }
        cur.clear();
        if k + 1 < pts.len()
            && let Some(q) = toward(p, pts[k + 1], gap / 2.0)
        {
            cur.push(q);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Где сажать нельзя: дома, дороги, вода, ограды. Сетка маски (`RES` точек на клетку).
struct Blocked {
    w: usize,
    h: usize,
    cells: Vec<bool>,
}

impl Blocked {
    fn new(m: &Map) -> Self {
        let cells = m.rooms.data.iter().map(|&v| v >= 128).collect();
        Self { w: m.rooms.w, h: m.rooms.h, cells }
    }

    fn disk(&mut self, c: [f32; 2], r: f32) {
        let res = mask::RES as f32;
        let (i0, i1) = (((c[0] - r) * res).floor().max(0.0) as usize, (((c[0] + r) * res).ceil().max(0.0) as usize).min(self.w));
        let (j0, j1) = (((c[1] - r) * res).floor().max(0.0) as usize, (((c[1] + r) * res).ceil().max(0.0) as usize).min(self.h));
        for j in j0..j1 {
            for i in i0..i1 {
                let p = mask::sample_pos(i as isize, j as isize);
                if (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r {
                    self.cells[j * self.w + i] = true;
                }
            }
        }
    }

    /// Полоса вдоль ломаной.
    fn line(&mut self, pts: &[[f32; 2]], r: f32) {
        for s in pts.windows(2) {
            let n = (length(s[0], s[1]) / (r * 0.5).max(0.25)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let f = k as f32 / n as f32;
                self.disk([s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f], r);
            }
        }
    }

    fn at(&self, p: [f32; 2]) -> bool {
        let res = mask::RES as f32;
        let (i, j) = ((p[0] * res).floor(), (p[1] * res).floor());
        i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h || self.cells[j as usize * self.w + i as usize]
    }
}

/// Сколько всего деревьев и кустов сажать на карту: больше — Foundry тяжело
/// (у каждой кроны свой тайл).
const MAX_PLANTS: usize = 3000;

/// Посаженное, разложенное по корзинам в клетку: не ближе шага к соседу.
#[derive(Default)]
struct Spacing(HashMap<(i32, i32), Vec<[f32; 2]>>);

impl Spacing {
    fn free(&self, q: [f32; 2], d: f32) -> bool {
        let (bx, by, r) = (q[0].floor() as i32, q[1].floor() as i32, d.ceil() as i32);
        (-r..=r).all(|dx| (-r..=r).all(|dy| self.0.get(&(bx + dx, by + dy)).is_none_or(|v| v.iter().all(|p| length(*p, q) >= d))))
    }

    fn add(&mut self, q: [f32; 2]) {
        self.0.entry((q[0].floor() as i32, q[1].floor() as i32)).or_default().push(q);
    }
}

/// Дом на карте: что про него нужно после заливки всех домов.
struct House {
    rings: Vec<Vec<[f32; 2]>>,
    /// Входы из OSM, в клетках.
    entrances: Vec<[f32; 2]>,
    style: WallStyle,
    wood: bool,
    /// Большой дом с широкими дверьми: вокзал, храм, магазин.
    grand: bool,
}

/// Линия, которая ляжет дорогой: этаж, порядок и сами дороги снизу вверх.
type Lane = (f32, u8, Vec<Path>);

pub fn import(xml: &str, opt: Options) -> Result<(Map, Report), String> {
    let osm = parse(xml)?;
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
    let (w, h) = (((wm / cell - 0.001).ceil() as u32).max(8), ((hm / cell - 0.001).ceil() as u32).max(8));
    let proj = Proj { north, west, kx: mx / cell as f64, ky: my / cell as f64 };
    let m_to_cells = |metres: f32| metres / cell;
    let mut m = Map::new(w, h);
    let mut areas = areas(&osm, &proj);
    let sea = osm.ways.values().any(|w| get(&w.tags, "natural") == "coastline");
    let ground = |t: &Tags| ground(t, sea);
    // Природные контуры — с изгибом; дома, мощёное и пути остаются по линейке.
    // Узлы, общие для нескольких природных контуров, не срезаются.
    let natural: Vec<bool> = areas.iter().map(|a| building(a.tags).is_none() && (growth(a.tags).is_some() || matches!(ground(a.tags), Some((t, _)) if !matches!(t, "stone_tiles" | "gravel" | "dirt")))).collect();
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
    // Город или природа — по доле застройки: от этого зависят пол под всем,
    // дорожки без тега покрытия и стены-ограды.
    // Застройка — только та, что на карте: в выгрузке дома и за её краем.
    let cells_of = |a: &Area| coverage(&m.rooms, &a.rings, 0.0).iter().map(|c| c.1).sum::<f32>() / (mask::RES * mask::RES) as f32;
    let built: f32 = areas.iter().filter(|a| building(a.tags).is_some()).map(cells_of).sum();
    let rural = built < 0.06 * (w * h) as f32;
    m.ambience = if rural { Season::Summer.ambience() } else { "town-1" }.into();
    m.wall_style = wall(if rural { "wall_fieldstone" } else { "wall_brick" });

    // Под всем — мостовая или трава пятнами, поверх — земля, трава, вода и платформы.
    let mut floors = Floors::default();
    let base = floors.get(&mut m, if rural { "meadow" } else { "flagstones" });
    let all = mask::cells(m.rooms.w, m.rooms.h, (0, 0), (w as i32 - 1, h as i32 - 1));
    m.paint(Layer::Floor, base, &all);
    let mut grounds: Vec<(u8, usize)> = areas.iter().enumerate().filter_map(|(k, a)| ground(a.tags).map(|(_, o)| (o, k))).collect();
    grounds.sort();
    let waters: Vec<usize> = grounds.iter().map(|g| g.1).filter(|&k| matches!(ground(areas[k].tags), Some(("water_deep", _)))).collect();
    let mut water_done = false;
    for (_, k) in grounds {
        let (tex, _) = ground(areas[k].tags).unwrap_or(("flagstones", 0));
        if tex == "water_deep" {
            if !water_done {
                water(&mut m, &mut floors, &waters.iter().map(|&k| &areas[k]).collect::<Vec<_>>());
                water_done = true;
            }
            continue;
        }
        let weights = coverage(&m.rooms, &areas[k].rings, 0.0);
        if !weights.is_empty() {
            let t = floors.get(&mut m, tex);
            m.paint(Layer::Floor, t, &weights);
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
        let (on_meadow, on_rough) = (only(&m, meadow), only(&m, rough));
        let res = mask::RES as f32;
        let (mw, mh) = (m.rooms.w, m.rooms.h);
        let (mut to_rough, mut to_meadow) = (Vec::new(), Vec::new());
        for j in 0..mh {
            for i in 0..mw {
                let p = [(i as f32 + 0.5) / res, (j as f32 + 0.5) / res];
                let cell = (p[1] as usize).min(h as usize - 1) * w as usize + (p[0] as usize).min(w as usize - 1);
                let k = j * mw + i;
                if on_meadow[cell] {
                    let n = noise(p, 13.0, 21) * 0.7 + noise(p, 4.5, 22) * 0.3;
                    let v = ((n - 0.2) / 0.3).clamp(0.0, 1.0) * 0.65;
                    if v > 0.0 {
                        to_rough.push((k, v));
                    }
                } else if on_rough[cell] {
                    let n = noise(p, 9.0, 23) * 0.7 + noise(p, 3.5, 24) * 0.3;
                    let v = ((n - 0.15) / 0.3).clamp(0.0, 1.0) * 0.6;
                    if v > 0.0 {
                        to_meadow.push((k, v));
                    }
                }
            }
        }
        m.paint(Layer::Floor, rough, &to_rough);
        m.paint(Layer::Floor, meadow, &to_meadow);
    }

    // Дома: комнаты по контуру.
    let mut houses: Vec<House> = Vec::new();
    let mut names: BTreeMap<String, ([f32; 2], f32)> = BTreeMap::new();
    for a in &areas {
        let Some(kind) = building(a.tags) else { continue };
        let weights = coverage(&m.rooms, &a.rings, 0.0);
        if weights.is_empty() {
            continue;
        }
        let (floor, slot, wood) = building_look(kind, a.tags);
        let f = floors.get(&mut m, floor);
        m.paint(Layer::Rooms, f, &weights);
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
        houses.push(House { rings: a.rings.clone(), entrances, style: wall(slot), wood, grand });
    }
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
    let rails_mat = m.material_index(Material { name: "Рельсы".into(), paint: Paint::Color([84, 82, 80]), vary: false, live: Live::Off }).unwrap_or(0);
    let deck_base = m.material_index(Material { name: "Настил".into(), paint: Paint::Color([70, 50, 34]), vary: false, live: Live::Off }).unwrap_or(0);
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
                let tex = if bridge && order == 3 { "wood_planks" } else { tex };
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
                let mut deck = path(material, width, soft, rough);
                deck.along = bridge && order == 3;
                // Доски FA полупрозрачны (между ними щели) — под настилом сплошная
                // подложка цвета дерева, иначе сквозь мост видно реку.
                let under = (bridge && order == 3).then(|| path(deck_base, width, 0.0, 0.0));
                lanes.push((lay, order, under.into_iter().chain([deck]).collect()));
                if bridge {
                    let style = WallStyle { width: 0.15, ..wall(if order == 3 { "wall_wood" } else { "wall_stone_blocks" }) };
                    for side in [-width / 2.0, width / 2.0] {
                        m.walls.push(Wall { points: offset(&line, side), closed: false, bends: Vec::new(), low: true, style: Some(style.clone()) });
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

    // Лес, сад, кустарник, вереск, болото: отдельных деревьев в OSM у них нет —
    // сажаем дротиками, мимо домов, дорог, воды, оград и соседей.
    let mut blocked = Blocked::new(&m);
    for p in &m.paths {
        blocked.line(&p.curve(), p.width / 2.0 + 0.6);
    }
    for wl in &m.walls {
        blocked.line(&wl.points, 0.8);
    }
    for a in &areas {
        if matches!(ground(a.tags), Some(("water_deep" | "water_shallow", _))) {
            for (k, v) in coverage(&m.rooms, &a.rings, 0.0) {
                if v > 0.5 {
                    blocked.cells[k] = true;
                }
            }
        }
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
            // сетки крон не бывает. Пустошь и кустарник — куртинами.
            let glade = g.litter && noise(q, 9.0, 8) > 0.45;
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
            } else {
                (g.share * (1.0 + 0.8 * noise(q, 10.0, 9) as f64)).clamp(0.0, 1.0)
            };
            if !rng.chance(share) {
                continue;
            }
            let slot = if glade {
                if rng.chance(0.7) { "fern" } else { "bush_summer" }
            } else {
                g.slots[rng.int(0, g.slots.len() as i32 - 1) as usize]
            };
            if missing(slot) {
                continue;
            }
            let scale = g.scale.0 + rng.random() as f32 * (g.scale.1 - g.scale.0);
            drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
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
            let copse = noise(q, 16.0, 11).max(0.0);
            if !rng.chance((0.06 + 0.5 * copse * copse) as f64) {
                continue;
            }
            let tree = rng.chance(0.45);
            let slot = if tree { "tree_green" } else { "bush_summer" };
            if missing(slot) {
                continue;
            }
            let scale = if tree { 1.0 + rng.random() as f32 * 0.5 } else { 0.7 + rng.random() as f32 * 0.5 };
            drop(&mut m, slot, q, rng.int(0, 359) as f32, scale);
            planted += 1;
        }
    }

    // Кувшинки — купами у берега стоячей воды, где мелко, как в заводях на снимке.
    // Только из паков: встроенных нет. На реке их сносит течением.
    if !missing("lilypad") {
        let mut rng = Rng::new("osm/lily");
        for a in areas.iter().filter(|a| get(a.tags, "natural") == "water" && !matches!(get(a.tags, "water"), "river" | "canal" | "stream" | "lock" | "wastewater" | "fountain")) {
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
    for (id, p) in rocks {
        if m.contains(p[0].floor() as i32, p[1].floor() as i32) {
            let mut rng = Rng::new(&format!("osm/rock/{id}"));
            let scale = 1.0 + rng.random() as f32 * 0.6;
            drop(&mut m, "boulder", p, rng.int(0, 359) as f32, scale);
        }
    }

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
        if !outdoors(&m, p) {
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
mod tests {
    use super::*;

    /// Два дома стена к стене, улица с именем, рельсы, дерево, вход и подземная
    /// платформа, которой на карте быть не должно. Около 60×45 м.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5303" lon="-0.1248"/>
 <node id="2" lat="51.5303" lon="-0.1245"/>
 <node id="3" lat="51.5302" lon="-0.1245"/>
 <node id="4" lat="51.5302" lon="-0.1248"/>
 <node id="5" lat="51.5303" lon="-0.1242"/>
 <node id="6" lat="51.5302" lon="-0.1242"/>
 <node id="7" lat="51.5302" lon="-0.12465"><tag k="entrance" v="main"/></node>
 <node id="10" lat="51.53013" lon="-0.1251"/>
 <node id="11" lat="51.53013" lon="-0.1240"/>
 <node id="20" lat="51.53005" lon="-0.1251"/>
 <node id="21" lat="51.53005" lon="-0.1240"/>
 <node id="30" lat="51.53035" lon="-0.12435"><tag k="natural" v="tree"/></node>
 <node id="40" lat="51.53030" lon="-0.1250"/>
 <node id="41" lat="51.53030" lon="-0.1241"/>
 <node id="50" lat="51.53009" lon="-0.1247"><tag k="highway" v="street_lamp"/></node>
 <node id="51" lat="51.53009" lon="-0.1244"><tag k="amenity" v="bench"/></node>
 <node id="60" lat="51.53037" lon="-0.1250"/>
 <node id="61" lat="51.53037" lon="-0.1246"/>
 <way id="500"><nd ref="60"/><nd ref="61"/><tag k="barrier" v="fence"/></way>
 <node id="70" lat="51.53033" lon="-0.1250"/>
 <node id="71" lat="51.53033" lon="-0.1241"/>
 <way id="600"><nd ref="70"/><nd ref="71"/><tag k="highway" v="footway"/></way>
 <way id="100"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="7"/><nd ref="4"/><nd ref="1"/><tag k="building" v="house"/></way>
 <way id="101"><nd ref="2"/><nd ref="5"/><nd ref="6"/><nd ref="3"/><nd ref="2"/><tag k="building" v="retail"/><tag k="name" v="Lantern Shop"/></way>
 <way id="200"><nd ref="10"/><nd ref="11"/><tag k="highway" v="residential"/><tag k="name" v="Crowfall Lane"/></way>
 <way id="300"><nd ref="20"/><nd ref="21"/><tag k="railway" v="rail"/></way>
 <way id="400"><nd ref="40"/><nd ref="41"/><tag k="railway" v="platform"/><tag k="level" v="-1"/></way>
</osm>"#;

    fn sample() -> (Map, Report) {
        import(SAMPLE, Options::default()).expect("выгрузка читается")
    }

    #[test]
    fn houses_become_rooms_with_shared_wall_and_door_at_entrance() {
        let (m, r) = sample();
        assert_eq!(r.count, 2, "{:?}", r.warnings);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // 0,0009° долготы на этой широте — около 62 м, 0,0004° широты — около 44 м.
        assert!((38..=44).contains(&m.width) && (27..=31).contains(&m.height), "{}×{}", m.width, m.height);
        assert!(m.rooms.area() > 100.0, "дома не залиты: {}", m.rooms.area());
        // Между домами — общая стена, наружу — контур; забор — отдельно, низкий.
        let high: Vec<&Wall> = m.walls.iter().filter(|w| !w.low).collect();
        assert_eq!(high.len(), 1, "{:?}", m.walls);
        assert!(high[0].style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(k) if k.contains("wall_brick"))));
        assert!(m.walls.iter().any(|w| w.low && w.style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(k) if k.contains("wall_wood")))), "забора нет");
        assert!(!m.room_edges().is_empty());
        // Дверь у отмеченного входа, на стене, а не в воздухе.
        let entrance = Proj { north: 51.5304, west: -0.1250, kx: 111_320.0 * 51.5302f64.to_radians().cos() / CELL_M as f64, ky: 110_574.0 / CELL_M as f64 }.at(51.5302, -0.12465);
        let door = m.portals.iter().find(|p| p.kind.is_door()).expect("дверь");
        let mid = [(door.a[0] + door.b[0]) / 2.0, (door.a[1] + door.b[1]) / 2.0];
        assert!(length(mid, entrance) < 1.0, "дверь {mid:?}, вход {entrance:?}");
        let cut = crate::map::cut_portals(&m.room_edges(), &m.portals);
        assert!(cut.len() > m.room_edges().len(), "дверь не прорезала стену");
        // У лавки входа нет: дверь — к улице на юге, а не к дорожке позади.
        let doors: Vec<[f32; 2]> = m.portals.iter().filter(|p| p.kind.is_door()).map(|p| [(p.a[0] + p.b[0]) / 2.0, (p.a[1] + p.b[1]) / 2.0]).collect();
        assert_eq!(doors.len(), 2, "{doors:?}");
        let shop = doors.iter().find(|d| length(**d, entrance) > 2.0).expect("дверь лавки");
        assert!(shop[1] > 12.0, "дверь лавки смотрит не на улицу: {shop:?}");
    }

    #[test]
    fn streets_rails_trees_and_labels() {
        let (m, _) = sample();
        let tex = |p: &Path| match &m.materials[p.material].paint {
            Paint::Texture(k) => k.clone(),
            Paint::Color(_) => "color".into(),
        };
        assert!(m.paths.iter().any(|p| tex(p).contains("cobblestone") && (p.width - 6.0 / CELL_M).abs() < 0.01), "улицы нет");
        assert!(m.paths.iter().any(|p| tex(p).contains("gravel")), "насыпи нет");
        assert_eq!(m.paths.iter().filter(|p| tex(p) == "color").count(), 2, "рельсов должно быть два");
        // Подземная платформа (около 7,3 клетки от верха) не нарисована; плитка — только дорожка (около 5,1).
        let tiles: Vec<&Path> = m.paths.iter().filter(|p| tex(p).contains("stone_tiles")).collect();
        assert_eq!(tiles.len(), 1, "подземная платформа на карте");
        assert!(tiles[0].points.iter().all(|q| (q[1] - 5.1).abs() < 0.3), "{:?}", tiles[0].points);
        let count = |word: &str| m.props.iter().filter(|q| q.asset.contains(word)).count();
        assert_eq!((count("builtin:tree"), count("street_lamp"), count("bench")), (1, 1, 1), "{:?}", m.props);
        let bench = m.props.iter().find(|q| q.asset.contains("bench")).unwrap();
        assert!(bench.rot.abs() < 1.0 || (bench.rot - 180.0).abs() < 1.0, "скамья вдоль улицы: {}", bench.rot);
        assert!(m.labels.iter().any(|l| l.text == "Crowfall Lane" && l.gm && l.rot.abs() < 1.0));
    }

    /// Природа: лес слева, озеро справа, гравийная тропа через лес, каменная
    /// ограда с калиткой, камень. Домов нет.
    const RURAL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5300" lon="-0.1250"/>
 <node id="2" lat="51.5300" lon="-0.1246"/>
 <node id="3" lat="51.5304" lon="-0.1246"/>
 <node id="4" lat="51.5304" lon="-0.1250"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="wood"/><tag k="leaf_type" v="broadleaved"/></way>
 <node id="11" lat="51.53005" lon="-0.12445"/>
 <node id="12" lat="51.53005" lon="-0.1242"/>
 <node id="13" lat="51.53025" lon="-0.1242"/>
 <node id="14" lat="51.53025" lon="-0.12445"/>
 <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="14"/><nd ref="11"/><tag k="natural" v="water"/><tag k="name" v="Tarn"/></way>
 <node id="21" lat="51.5302" lon="-0.1251"/>
 <node id="22" lat="51.5302" lon="-0.1245"/>
 <way id="30"><nd ref="21"/><nd ref="22"/><tag k="highway" v="path"/><tag k="surface" v="gravel"/></way>
 <node id="31" lat="51.53035" lon="-0.1245"/>
 <node id="32" lat="51.53035" lon="-0.1243"><tag k="barrier" v="gate"/></node>
 <node id="33" lat="51.53035" lon="-0.1241"/>
 <way id="40"><nd ref="31"/><nd ref="32"/><nd ref="33"/><tag k="barrier" v="wall"/></way>
 <node id="50" lat="51.53032" lon="-0.12425"><tag k="natural" v="stone"/></node>
</osm>"#;

    #[test]
    fn countryside_grass_forest_lake_path_wall() {
        let (m, r) = import(RURAL, Options::default()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(m.ambience, "forest-summer");
        let key = |x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        // Под всем трава (здесь нет ни леса, ни воды, ни тропы).
        assert!(key(38, 3).contains("grass"), "{}", key(38, 3));
        // Озеро: середина глубокая, у края (около x = 25, край гуляет) мелко.
        assert!(key(30, 18).contains("water_deep"), "{}", key(30, 18));
        assert!((23..=28).any(|x| key(x, 18).contains("water_shallow")), "{:?}", (23..=28).map(|x| key(x, 18)).collect::<Vec<_>>());
        // Деревья в лесу (опушка около 18,2 клетки, гуляет на 0,7) и не на тропе (y ≈ 14,5).
        // На ничьей земле справа — разве что одинокие, и не в озере (25–36 × 11–25).
        let trees: Vec<&Prop> = m.props.iter().filter(|q| q.asset.contains("builtin:tree")).collect();
        let (wood, lone): (Vec<&Prop>, Vec<&Prop>) = trees.iter().partition(|q| q.pos[0] < 19.0);
        assert!(wood.len() >= 15, "деревьев {}", wood.len());
        assert!(wood.iter().all(|q| (q.pos[1] - 14.5).abs() > 1.0), "{:?}", wood.iter().map(|q| q.pos).collect::<Vec<_>>());
        assert!(lone.len() <= 4 && lone.iter().all(|q| !((24.5..37.0).contains(&q.pos[0]) && (10.5..26.0).contains(&q.pos[1]))), "{:?}", lone.iter().map(|q| q.pos).collect::<Vec<_>>());
        assert!(m.terrain.iter().any(|c| matches!(&m.materials[c.material as usize].paint, Paint::Texture(t) if t.contains("forest_floor"))), "подстилки нет");
        // Тропа гравийная, край мягкий.
        let path = m.paths.iter().find(|p| matches!(&m.materials[p.material].paint, Paint::Texture(t) if t.contains("gravel"))).expect("тропа");
        assert!(path.soft >= 0.25 && path.soft < 0.45, "гравий ровнее земляной тропы: {}", path.soft);
        // Ограда — сухая кладка, низкая, разорвана у калитки.
        assert_eq!(m.walls.len(), 2, "{:?}", m.walls);
        assert!(m.walls.iter().all(|w| w.low && w.style.as_ref().is_some_and(|s| matches!(&s.paint, Paint::Texture(t) if t.contains("fieldstone")))));
        let gap = length(m.walls[0].points[1], m.walls[1].points[0]);
        assert!((gap - 2.0).abs() < 0.05, "проход {gap}");
        assert!(m.props.iter().any(|q| q.asset.contains("boulder")), "камня нет");
        assert!(m.labels.iter().any(|l| l.text == "Tarn"));
    }

    /// Кладбище слева, речной пляж без покрытия справа; моря в выгрузке нет.
    const CHURCHYARD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.5301" lon="-0.1249"/>
 <node id="2" lat="51.5301" lon="-0.1245"/>
 <node id="3" lat="51.5303" lon="-0.1245"/>
 <node id="4" lat="51.5303" lon="-0.1249"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="amenity" v="grave_yard"/></way>
 <node id="11" lat="51.5301" lon="-0.1244"/>
 <node id="12" lat="51.5301" lon="-0.1242"/>
 <node id="13" lat="51.5303" lon="-0.1242"/>
 <node id="14" lat="51.5303" lon="-0.1244"/>
 <way id="20"><nd ref="11"/><nd ref="12"/><nd ref="13"/><nd ref="14"/><nd ref="11"/><tag k="natural" v="beach"/></way>
</osm>"#;

    #[test]
    fn churchyard_has_graves_in_rows_and_river_beach_is_shingle() {
        let (m, _) = import(CHURCHYARD, Options::default()).unwrap();
        // Кладбище — около 4,5–22,7 × 7,3–21,8 клетки (край гуляет на 0,7).
        let yard = |p: [f32; 2]| (3.5..23.7).contains(&p[0]) && (6.3..22.8).contains(&p[1]);
        // Без надгробий в паках могила — камень у изголовья.
        let stones: Vec<[f32; 2]> = m.props.iter().filter(|q| q.asset.contains("boulder")).map(|q| q.pos).collect();
        assert!(stones.len() >= 8 && stones.iter().all(|&p| yard(p)), "{stones:?}");
        // На кладбище не растёт случайное: оно не ничья земля.
        assert!(m.props.iter().filter(|q| q.asset.contains("tree") || q.asset.contains("bush")).all(|q| !yard(q.pos)));
        let floor = match m.floor_at(31, 14) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        assert!(floor.contains("gravel"), "речной пляж — галька: {floor}");
    }

    /// Река из двух кусков с общим краем уходит за края карты; через неё пешеходный мост.
    const RIVER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osm version="0.6">
 <bounds minlat="51.5300" minlon="-0.1250" maxlat="51.5304" maxlon="-0.1241"/>
 <node id="1" lat="51.53010" lon="-0.1252"/>
 <node id="2" lat="51.53010" lon="-0.12455"/>
 <node id="3" lat="51.53030" lon="-0.12455"/>
 <node id="4" lat="51.53030" lon="-0.1252"/>
 <node id="5" lat="51.53010" lon="-0.1239"/>
 <node id="6" lat="51.53030" lon="-0.1239"/>
 <way id="10"><nd ref="1"/><nd ref="2"/><nd ref="3"/><nd ref="4"/><nd ref="1"/><tag k="natural" v="water"/><tag k="water" v="river"/></way>
 <way id="11"><nd ref="2"/><nd ref="5"/><nd ref="6"/><nd ref="3"/><nd ref="2"/><tag k="natural" v="water"/><tag k="water" v="river"/></way>
 <node id="20" lat="51.5304" lon="-0.1247"/>
 <node id="21" lat="51.5300" lon="-0.1247"/>
 <way id="30"><nd ref="20"/><nd ref="21"/><tag k="highway" v="footway"/><tag k="bridge" v="yes"/><tag k="layer" v="1"/></way>
 <node id="40" lat="51.5304" lon="-0.1243"/>
 <node id="41" lat="51.5300" lon="-0.1243"/>
 <way id="50"><nd ref="40"/><nd ref="41"/><tag k="highway" v="footway"/><tag k="ford" v="stepping_stones"/></way>
</osm>"#;

    #[test]
    fn river_in_parts_has_no_shoal_at_the_join_and_bridge_has_deck_and_rails() {
        let (m, _) = import(RIVER, Options::default()).unwrap();
        let key = |x: i32, y: i32| match m.floor_at(x, y) {
            0 => String::new(),
            k => match &m.materials[k as usize - 1].paint {
                Paint::Texture(t) => t.clone(),
                Paint::Color(_) => "color".into(),
            },
        };
        // Середина русла (y ≈ 14,5): у стыка кусков (x ≈ 20,4) и у краёв карты — глубоко.
        for x in [0, 19, 20, 21, 40] {
            assert!(key(x, 14).contains("water_deep"), "x = {x}: {}", key(x, 14));
        }
        // У берега (около y = 8) — мелко.
        assert!((7..=10).any(|y| key(20, y).contains("water_shallow")), "{:?}", (7..=10).map(|y| key(20, y)).collect::<Vec<_>>());
        // Вдоль шва кусков (x = 20) от берега до берега — вода, без полосы суши
        // и без выемок на берегах.
        for y in 9..=19 {
            assert!(key(20, y).contains("water"), "шов, y = {y}: {}", key(20, y));
        }
        let deck = m.paths.iter().find(|p| matches!(&m.materials[p.material].paint, Paint::Texture(t) if t.contains("wood_planks"))).expect("настил");
        assert!(deck.along);
        // Две точки OSM — дорога растаяла бы целиком: точки не реже двух клеток.
        assert!(deck.points.windows(2).all(|s| length(s[0], s[1]) <= 2.01), "{:?}", deck.points);
        assert_eq!(m.walls.iter().filter(|w| w.low).count(), 2, "перила");
        // Камни-переходы — цепочка камней, а не полоса тропы через воду; полос две:
        // подложка моста и настил над ней.
        assert_eq!(m.paths.len(), 2, "лишняя полоса");
        assert!(matches!(m.materials[m.paths[0].material].paint, Paint::Color(_)), "подложка не снизу");
        let stones = m.props.iter().filter(|q| q.asset.contains("boulder")).count();
        assert!((25..=40).contains(&stones), "камней {stones}");
    }

    #[test]
    fn same_file_same_map() {
        assert_eq!(sample().0.to_json(), sample().0.to_json());
    }

    #[test]
    fn big_area_is_scaled_down_with_warning() {
        let (m, r) = import(SAMPLE, Options { max_cells: 20, ..Options::default() }).unwrap();
        assert!(m.width <= 20 && m.height <= 20);
        assert!(r.warnings.iter().any(|w| w.contains("клетка")), "{:?}", r.warnings);
    }

    #[test]
    fn not_osm_is_an_error() {
        assert!(import("<html></html>", Options::default()).is_err());
        assert!(import("не xml", Options::default()).is_err());
    }

    #[test]
    fn multipolygon_rings_join_and_courtyard_stays_open() {
        let r = assemble(vec![vec![1, 2, 3], vec![3, 4, 1]]);
        assert!(r.len() == 1 && r[0].len() == 5 && r[0].first() == r[0].last(), "{r:?}");
        assert_eq!(assemble(vec![vec![1, 2, 3], vec![1, 4, 3]]).len(), 1);
        assert!(assemble(vec![vec![1, 2, 3]]).is_empty(), "незамкнутое отбрасывается");
        let mk = Mask::for_cells(10, 10);
        let outer = vec![[1.0, 1.0], [9.0, 1.0], [9.0, 9.0], [1.0, 9.0]];
        let inner = vec![[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]];
        let cov = coverage(&mk, &[outer, inner], 0.0);
        let at = |x: f32, y: f32| {
            let k = (y * mask::RES as f32) as usize * mk.w + (x * mask::RES as f32) as usize;
            cov.iter().find(|c| c.0 == k).map_or(0.0, |c| c.1)
        };
        assert_eq!(at(2.0, 2.0), 1.0);
        assert_eq!(at(5.0, 5.0), 0.0, "двор внутри дома залит");
        assert_eq!(at(0.5, 0.5), 0.0);
    }

    /// Настоящая выгрузка: `KARTOGRAF_OSM=путь.osm cargo test osm_file -- --ignored`.
    /// Рядом с файлом кладёт `.karta` и грубый снимок `_preview.png`.
    #[test]
    #[ignore]
    fn osm_file() {
        let Ok(path) = std::env::var("KARTOGRAF_OSM") else { return };
        let xml = std::fs::read_to_string(&path).unwrap();
        let t = std::time::Instant::now();
        let (m, r) = import(&xml, Options::default()).unwrap();
        println!("{}×{} клеток, домов {}, дорог {}, стен {}, дверей {}, объектов {}, подписей {}, {:?}", m.width, m.height, r.count, m.paths.len(), m.walls.len(), m.portals.len(), m.props.len(), m.labels.len(), t.elapsed());
        println!("предупреждения: {:?}", r.warnings);
        let stem = std::path::Path::new(&path).with_extension("");
        std::fs::write(stem.with_extension("karta"), m.to_json()).unwrap();
        preview(&m, 4).save(format!("{}_preview.png", stem.display())).unwrap();
    }

    /// Что подбор берёт в слоты из всех паков настроек (полы тоже):
    /// `SLOTS=meadow,shingle cargo test fa_slots -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fa_slots() {
        let Ok(slots) = std::env::var("SLOTS") else { return };
        let settings = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Kartograf/settings.json");
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let mut assets = Vec::new();
        for p in s["packs"].as_array().unwrap() {
            if let Ok(a) = crate::library::pack_assets(std::path::Path::new(p.as_str().unwrap())) {
                assets.extend(a);
            }
        }
        let pal = Palette::from_library(&assets, Box::new(crate::palette::fake_measure));
        for slot in slots.split(',') {
            let keys = pal.keys(slot);
            println!("== {slot}: {}", keys.len());
            for k in keys.iter().take(30) {
                let name = assets.iter().find(|a| &a.key == k).map_or("?", |a| a.name.as_str());
                println!("   {} | {name}", k.rsplit('/').next().unwrap_or(k));
            }
        }
    }

    /// Паки из настроек программы: `FIND=rail,track cargo test find_fa -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn find_fa() {
        let Ok(words) = std::env::var("FIND") else { return };
        let settings = std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("Kartograf/settings.json");
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        let mut assets = Vec::new();
        for p in s["packs"].as_array().unwrap() {
            if let Ok(a) = crate::library::pack_assets(std::path::Path::new(p.as_str().unwrap())) {
                assets.extend(a);
            }
        }
        println!("ассетов: {}", assets.len());
        // `DUMP=папка` — найденные текстуры пола (`terrain/`) уменьшенными картинками, по 20 на слово.
        if let Ok(dir) = std::env::var("DUMP") {
            for w in words.split(',') {
                let mut n = 0;
                for p in s["packs"].as_array().unwrap() {
                    let Ok(pack) = crate::pck::read_index(std::path::Path::new(p.as_str().unwrap())) else { continue };
                    for e in pack.entries.iter().filter(|e| (e.path.contains("terrain/") || e.path.contains("patterns/")) && e.path.to_lowercase().contains(w)) {
                        if n >= 120 {
                            break;
                        }
                        let Ok(img) = crate::pck::read_bytes(&pack.file, e.offset, e.size).and_then(|b| image::load_from_memory(&b).map_err(|x| x.to_string())) else { continue };
                        let name = e.path.rsplit('/').next().unwrap_or("x").replace(".webp", ".png");
                        img.thumbnail(256, 256).save(std::path::Path::new(&dir).join(name)).unwrap();
                        n += 1;
                    }
                }
            }
        }
        for w in words.split(',') {
            // `FLOORS=1` — только текстуры пола, без объектов.
            let floors_only = std::env::var("FLOORS").is_ok();
            let mut hits: Vec<String> = assets
                .iter()
                .filter(|a| a.key.to_lowercase().contains(w) && (!floors_only || a.kind == crate::library::Kind::Floor))
                .map(|a| a.key.rsplit('|').next().unwrap_or(&a.key).to_owned())
                .collect();
            hits.sort();
            hits.dedup_by(|a, b| a.rsplit('/').next().unwrap().split('_').take(3).eq(b.rsplit('/').next().unwrap().split('_').take(3)));
            println!("== {w}: {}", hits.len());
            for h in hits.iter().take(30) {
                println!("   {h}");
            }
        }
    }

    /// Грубый снимок для глаз: цвет пола по имени текстуры, дороги, стены, двери, деревья.
    fn preview(m: &Map, ppc: u32) -> image::RgbImage {
        let color = |mat: u8| -> [u8; 3] {
            let key = match &m.materials[mat as usize].paint {
                Paint::Texture(k) => k.as_str(),
                Paint::Color(c) => return *c,
            };
            let table: [(&str, [u8; 3]); 16] = [
                ("cobblestone", [104, 98, 92]),
                ("stone_tiles", [178, 172, 160]),
                ("flagstones", [148, 142, 132]),
                ("grass_dry", [150, 150, 84]),
                ("grass", [96, 138, 72]),
                ("water_deep", [40, 74, 124]),
                ("water", [82, 128, 168]),
                ("gravel", [150, 140, 124]),
                ("wood_planks", [168, 128, 88]),
                ("marble", [220, 218, 212]),
                ("dirt", [130, 100, 70]),
                ("forest", [70, 88, 50]),
                ("swamp", [70, 96, 80]),
                ("rocky", [120, 116, 112]),
                ("sand", [214, 196, 140]),
                ("mud", [98, 80, 60]),
            ];
            table.iter().find(|(k, _)| key.contains(k)).map_or([255, 0, 255], |t| t.1)
        };
        let (w, h) = (m.width * ppc, m.height * ppc);
        let mut img = image::RgbImage::new(w, h);
        let res = mask::RES as f32;
        let top = |coats: &[crate::map::Coat], i: isize, j: isize| coats.iter().rev().find(|c| c.mask.get(i, j) >= 128).map(|c| c.material);
        for y in 0..h {
            for x in 0..w {
                let (i, j) = (((x as f32 + 0.5) / ppc as f32 * res) as isize, ((y as f32 + 0.5) / ppc as f32 * res) as isize);
                let c = if m.rooms.get(i, j) >= 128 { top(&m.room_floor, i, j) } else { top(&m.terrain, i, j) };
                img.put_pixel(x, y, image::Rgb(c.map_or([0, 0, 0], color)));
            }
        }
        let dot = |img: &mut image::RgbImage, p: [f32; 2], r: f32, c: [u8; 3]| {
            let (cx, cy, rp) = (p[0] * ppc as f32, p[1] * ppc as f32, (r * ppc as f32).max(0.5));
            for y in (cy - rp).floor() as i32..=(cy + rp).ceil() as i32 {
                for x in (cx - rp).floor() as i32..=(cx + rp).ceil() as i32 {
                    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)) <= rp * rp {
                        img.put_pixel(x as u32, y as u32, image::Rgb(c));
                    }
                }
            }
        };
        let line = |img: &mut image::RgbImage, a: [f32; 2], b: [f32; 2], r: f32, c: [u8; 3]| {
            let steps = ((length(a, b) * ppc as f32) as usize).max(1);
            for s in 0..=steps {
                let f = s as f32 / steps as f32;
                dot(img, [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f], r, c);
            }
        };
        for p in &m.paths {
            let c = color(p.material as u8);
            for s in p.curve().windows(2) {
                line(&mut img, s[0], s[1], p.width / 2.0, c);
            }
        }
        for s in crate::map::cut_portals(&m.room_edges(), &m.portals) {
            line(&mut img, s.0, s.1, 0.15, [40, 30, 26]);
        }
        for wl in &m.walls {
            let c = if wl.low { [90, 80, 70] } else { [40, 30, 26] };
            for s in wl.points.windows(2) {
                line(&mut img, s[0], s[1], 0.15, c);
            }
        }
        for p in &m.portals {
            line(&mut img, p.a, p.b, 0.18, [210, 40, 30]);
        }
        // Кроны полупрозрачными кругами: видно и густоту, и что под ними.
        for q in &m.props {
            let (r, c) = if q.asset.contains("tree") {
                (1.4 * q.scale, [34, 80, 30])
            } else if q.asset.contains("bush") {
                (0.5 * q.scale, [120, 170, 70])
            } else if q.asset.contains("boulder") {
                (0.5 * q.scale, [90, 90, 96])
            } else {
                (0.4, [240, 200, 40])
            };
            dot(&mut img, q.pos, r * 0.35, c);
            let (cx, cy, rp) = (q.pos[0] * ppc as f32, q.pos[1] * ppc as f32, r * ppc as f32);
            for y in (cy - rp).floor() as i32..=(cy + rp).ceil() as i32 {
                for x in (cx - rp).floor() as i32..=(cx + rp).ceil() as i32 {
                    let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                    if x >= 0 && y >= 0 && (x as u32) < w && (y as u32) < h && (d - rp).abs() < 0.7 {
                        img.put_pixel(x as u32, y as u32, image::Rgb(c));
                    }
                }
            }
        }
        img
    }
}
