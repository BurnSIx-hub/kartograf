//! Разбор выгрузки OSM: узлы, линии, отношения, площади, проекция в клетки.

use super::*;

pub(super) type Tags = BTreeMap<String, String>;

pub(super) struct Node {
    pub(super) lat: f64,
    pub(super) lon: f64,
    pub(super) tags: Tags,
}

pub(super) struct Way {
    pub(super) nodes: Vec<i64>,
    pub(super) tags: Tags,
}

pub(super) struct Relation {
    /// Пути-участники: номер и роль (`outer`, `inner`).
    pub(super) ways: Vec<i64>,
    pub(super) tags: Tags,
}

pub(super) struct Osm {
    /// Юг, запад, север, восток.
    pub(super) bounds: [f64; 4],
    pub(super) nodes: HashMap<i64, Node>,
    pub(super) ways: BTreeMap<i64, Way>,
    pub(super) relations: BTreeMap<i64, Relation>,
}

pub(super) fn tags_of(n: roxmltree::Node) -> Tags {
    n.children().filter(|c| c.has_tag_name("tag")).filter_map(|c| Some((c.attribute("k")?.to_owned(), c.attribute("v")?.to_owned()))).collect()
}

pub(super) fn number(n: roxmltree::Node, attr: &str) -> Option<f64> {
    n.attribute(attr)?.parse().ok()
}

pub(super) fn ident(n: roxmltree::Node) -> Option<i64> {
    n.attribute("id")?.parse().ok()
}

pub(super) fn parse(xml: &str) -> Result<Osm, String> {
    let mut osm = Osm { bounds: [f64::NAN; 4], nodes: HashMap::new(), ways: BTreeMap::new(), relations: BTreeMap::new() };
    let bounds = parse_into(&mut osm, xml)?;
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

/// Природный мультиполигон, неполный в выгрузке: «Экспорт» openstreetmap.org
/// отдаёт только пути, что задевают участок, и большой лес, озеро или осыпь
/// не замыкаются — импорт их выбрасывал (осыпь Уэст-Уотер, луг Лулворта).
/// Номера таких отношений — докачать целиком (`relation/{id}/full`).
pub fn incomplete(xml: &str) -> Vec<i64> {
    let Ok(osm) = parse(xml) else { return Vec::new() };
    osm.relations
        .iter()
        .filter(|(_, r)| get(&r.tags, "type") == "multipolygon" && !unseen(&r.tags) && (ground(&r.tags, false).is_some() || growth(&r.tags).is_some()))
        .filter(|(_, r)| r.ways.iter().any(|w| !osm.ways.contains_key(w)))
        .map(|(id, _)| *id)
        .take(MAX_FETCH)
        .collect()
}

/// Сколько неполных контуров докачивать за один импорт.
pub const MAX_FETCH: usize = 25;

/// Узлы, пути и отношения из докачанной выгрузки — к своим; рамка — своя.
pub(super) fn parse_into(osm: &mut Osm, xml: &str) -> Result<Option<[Option<f64>; 4]>, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("не читается как OSM: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "osm" {
        return Err("это не выгрузка OpenStreetMap (.osm)".into());
    }
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
    Ok(bounds)
}

pub(super) fn get<'a>(t: &'a Tags, k: &str) -> &'a str {
    t.get(k).map_or("", String::as_str)
}

/// Под землёй, в тоннеле или внутри здания: на карте сверху этого не видно.
pub(super) fn unseen(t: &Tags) -> bool {
    let below = |k: &str| get(t, k).split(';').filter_map(|v| v.trim().parse::<f32>().ok()).any(|v| v < 0.0);
    !matches!(get(t, "tunnel"), "" | "no") || below("layer") || below("level") || matches!(get(t, "location"), "underground" | "indoor") || !matches!(get(t, "indoor"), "" | "no")
}

/// Этаж линии для порядка рисования: мост поверх того, что под ним.
pub(super) fn layer(t: &Tags) -> f32 {
    let l = get(t, "layer").trim().parse::<f32>().unwrap_or(0.0);
    if matches!(get(t, "bridge"), "" | "no") { l } else { l.max(1.0) }
}

/// Ширина из тега `width` в метрах, если она правдоподобна.
pub(super) fn width_tag(t: &Tags) -> Option<f32> {
    get(t, "width").trim().trim_end_matches('m').trim().parse::<f32>().ok().filter(|w| (0.3..80.0).contains(w))
}

/// Широта и долгота — в клетки карты: равнопромежуточная проекция у середины
/// участка; на паре километров ошибка меньше клетки.
pub(super) struct Proj {
    pub(super) north: f64,
    pub(super) west: f64,
    pub(super) kx: f64,
    pub(super) ky: f64,
}

impl Proj {
    pub(super) fn at(&self, lat: f64, lon: f64) -> [f32; 2] {
        [((lon - self.west) * self.kx) as f32, ((self.north - lat) * self.ky) as f32]
    }
}

/// Замкнутые кольца из кусков: пути мультиполигона цепляются концами.
/// Незамкнувшееся (часть участников за краем выгрузки) отбрасывается.
pub(super) fn assemble(parts: Vec<Vec<i64>>) -> Vec<Vec<i64>> {
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
pub(super) struct Area<'a> {
    pub(super) rings: Vec<Vec<[f32; 2]>>,
    pub(super) nodes: Vec<Vec<i64>>,
    pub(super) tags: &'a Tags,
}

pub(super) fn areas<'a>(osm: &'a Osm, proj: &Proj) -> Vec<Area<'a>> {
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
