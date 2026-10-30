//! Выгрузка карты в VTT: сцена Foundry (JSON для «Import Data») и Universal VTT
//! (`.dd2vtt`). Картинку карты снимает окно по плиткам (см. `App::export_*`),
//! здесь — стены, проёмы и свет в координатах картинки.

use crate::map::{self, Map, PortalKind, Segment};
use crate::mask;
use serde_json::{Value, json};

/// Уровень сцены, который Foundry 14 создаёт по умолчанию: с ним «Import Data»
/// заменяет фон, а не заводит второй уровень.
const LEVEL_ID: &str = "defaultLevel0000";

/// Что выгружается кроме картинки.
pub struct Layout {
    /// Сплошные стены — ломаные в клетках.
    pub walls: Vec<Vec<[f32; 2]>>,
    /// Двери и окна — отрезок в клетках; пустые проёмы сюда не попадают.
    pub portals: Vec<PortalOut>,
    /// Свет: середина в клетках, радиус яркого и тусклого в клетках, цвет.
    pub lights: Vec<Light>,
    /// Живые области: эффект и контуры в клетках; `true` — дыра (остров).
    pub zones: Vec<Zone>,
}

pub struct Zone {
    pub effect: &'static str,
    pub loops: Vec<(Vec<[f32; 2]>, bool)>,
}

/// Модуль Foundry, который оживляет области.
pub const LIVE_MODULE: &str = "burn-living-map";

/// Контуры живых материалов: по эффекту — все залитые им места, острова — дырами.
fn zones(map: &Map) -> Vec<Zone> {
    let mut out: Vec<Zone> = Vec::new();
    for (m, mat) in map.materials.iter().enumerate() {
        let Some(effect) = mat.live().effect() else { continue };
        let mask = map.material_mask(m);
        if mask.is_empty() {
            continue;
        }
        let loops: Vec<Vec<[f32; 2]>> = mask::join(mask::contour(&mask)).into_iter().map(|l| simplify(&l, true, 0.05)).filter(|l| l.len() >= 4).collect();
        let tagged = loops
            .iter()
            .map(|l| {
                // Дыра — контур внутри нечётного числа других.
                let depth = loops.iter().filter(|o| !std::ptr::eq(*o, l) && inside(l[0], o)).count();
                (l.clone(), depth % 2 == 1)
            })
            .collect::<Vec<_>>();
        match out.iter_mut().find(|z| z.effect == effect) {
            Some(z) => z.loops.extend(tagged),
            None => out.push(Zone { effect, loops: tagged }),
        }
    }
    out
}

/// Точка внутри замкнутой ломаной (чётность пересечений).
fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    for w in poly.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

pub struct PortalOut {
    pub seg: Segment,
    pub kind: PortalKind,
    /// Картинка створки — у дверей; без своей — встроенная деревянная.
    pub leaf: Option<String>,
}

/// Картинка створки, уже лежащая рядом со сценой: путь для Foundry и ширина в точках.
pub struct LeafFile {
    pub src: String,
    pub width: u32,
}

pub struct Light {
    pub at: [f32; 2],
    pub bright: f32,
    pub dim: f32,
    pub color: &'static str,
    /// Анимация Foundry: «torch», «flame» и т. п.
    pub animation: &'static str,
}

/// Огонь среди встроенных объектов: что светит и как.
fn light_of(asset: &str) -> Option<(f32, f32, &'static str, &'static str)> {
    let id = asset.strip_prefix(crate::library::BUILTIN)?;
    match id {
        "campfire" => Some((3.0, 6.0, "#ff9a3c", "flame")),
        "fireplace" => Some((2.0, 5.0, "#ff8a2c", "flame")),
        "brazier" => Some((2.0, 4.0, "#ffa040", "torch")),
        _ => None,
    }
}

/// Стены карты для VTT: края комнат и нарисованные, без проёмов, сшитые в
/// ломаные и упрощённые — Foundry медленнеет от тысяч мелких отрезков.
pub fn layout(map: &Map, room_edges: &[Segment]) -> Layout {
    let mut segs: Vec<Segment> = room_edges.to_vec();
    for w in &map.walls {
        segs.extend(w.segments());
    }
    let cut = map::cut_portals(&segs, &map.portals);
    let walls = mask::join(cut)
        .into_iter()
        .map(|line| {
            let closed = line.len() > 3 && line[0] == line[line.len() - 1];
            simplify(&line, closed, 0.03)
        })
        .filter(|l| l.len() >= 2)
        .collect();
    let portals = map
        .portals
        .iter()
        .filter(|p| p.kind != PortalKind::Opening)
        .map(|p| PortalOut {
            seg: (p.a, p.b),
            kind: p.kind,
            leaf: p.kind.is_door().then(|| p.asset.clone().unwrap_or_else(|| crate::library::DEFAULT_DOOR.into())),
        })
        .collect();
    let lights = map
        .props
        .iter()
        .filter_map(|p| {
            let (bright, dim, color, animation) = light_of(&p.asset)?;
            Some(Light { at: p.pos, bright: bright * p.scale.sqrt(), dim: dim * p.scale.sqrt(), color, animation })
        })
        .collect();
    Layout { walls, portals, lights, zones: zones(map) }
}

/// Упрощение ломаной (Дуглас — Пекер): точки, отступающие от прямой меньше `tol`, уходят.
/// У замкнутой концы совпадают и остаются.
pub fn simplify(line: &[[f32; 2]], closed: bool, tol: f32) -> Vec<[f32; 2]> {
    if line.len() <= 2 {
        return line.to_vec();
    }
    if closed {
        // Делим кольцо в самой дальней от начала точке: у двух половин концы не совпадают.
        let far = (1..line.len() - 1)
            .max_by(|&a, &b| dist2(line[0], line[a]).total_cmp(&dist2(line[0], line[b])))
            .unwrap_or(line.len() / 2);
        let mut out = simplify(&line[..=far], false, tol);
        out.pop();
        out.extend(simplify(&line[far..], false, tol));
        return out;
    }
    let mut keep = vec![false; line.len()];
    keep[0] = true;
    keep[line.len() - 1] = true;
    let mut stack = vec![(0, line.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut worst, mut at) = (0.0f32, 0);
        for k in a + 1..b {
            let d = map::segment_distance(line[k], line[a], line[b]);
            if d > worst {
                worst = d;
                at = k;
            }
        }
        if worst > tol {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    line.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Идентификатор документа Foundry: 16 букв и цифр, из номера — чтобы выгрузка
/// одной карты давала один и тот же файл.
fn doc_id(kind: char, n: usize) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut h = (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (kind as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    let mut s = String::from("krt");
    s.push(kind);
    for _ in 0..12 {
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        s.push(ABC[(h % ABC.len() as u64) as usize] as char);
    }
    s
}

/// Сцена Foundry 14: картинка в уровне по умолчанию, клетка = `px` пикселей,
/// без полей (padding 0), чтобы координаты стен совпадали с картинкой.
/// Двери подвижные: створка — картинка анимации стены (`leaves` по ключу ассета),
/// двойная дверь — `animation.double`, Foundry сам делит проём на две створки.
#[cfg(test)]
pub fn foundry_scene(map: &Map, name: &str, image_src: &str, px: u32, layout: &Layout, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    foundry_levels(map, name, &[SceneLevel { name: name.to_owned(), image_src: image_src.to_owned(), layout, roofs: Vec::new() }], px, leaves)
}

/// Этаж сцены: имя, картинка и что на нём.
pub struct SceneLevel<'a> {
    pub name: String,
    pub image_src: String,
    pub layout: &'a Layout,
    /// Крыши этажа — тайлами над токенами.
    pub roofs: Vec<RoofTile>,
}

/// Крыша, уже лежащая картинкой рядом со сценой: путь и место в пикселях сцены.
pub struct RoofTile {
    pub src: String,
    pub rect: [i64; 4],
}

/// Id уровня: первый — уровень по умолчанию Foundry, остальные свои, 16 знаков.
pub fn level_id(k: usize) -> String {
    if k == 0 { LEVEL_ID.to_owned() } else { format!("kartografLevel{k:02}") }
}

/// Сцена с этажами: каждый — уровень Foundry 14 со своей картинкой, высота этажа
/// 10 футов; стены и огни этажа видны только на нём. Один этаж — как раньше,
/// без высот и привязки к уровню.
pub fn foundry_levels(map: &Map, name: &str, levels: &[SceneLevel], px: u32, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    let multi = levels.len() > 1;
    let mut walls = Vec::new();
    let mut lights = Vec::new();
    let mut regions = Vec::new();
    let mut tiles = Vec::new();
    let mut docs = Vec::new();
    for (k, lv) in levels.iter().enumerate() {
        let tag = |mut v: Value| {
            if multi {
                v["levels"] = json!([level_id(k)]);
            }
            v
        };
        let (w0, l0) = (walls.len(), lights.len());
        level_docs(lv.layout, px, leaves, &mut walls, &mut lights);
        for w in &mut walls[w0..] {
            *w = tag(w.take());
        }
        for l in &mut lights[l0..] {
            *l = tag(l.take());
        }
        // Крыша — тайл выше токенов этажа: гаснет, когда под ней токен,
        // и не пускает свет и дождь. `x`, `y` тайла — точка привязки картинки
        // (по умолчанию её середина), поэтому привязка — в левый верхний угол.
        for r in &lv.roofs {
            let [x, y, w, h] = r.rect;
            tiles.push(tag(json!({
                "_id": doc_id('t', tiles.len()), "texture": {"src": r.src, "anchorX": 0, "anchorY": 0}, "x": x, "y": y, "width": w, "height": h,
                "elevation": k as i64 * 10 + 9, "sort": tiles.len(),
                "occlusion": {"modes": [1], "alpha": 0},
                "restrictions": {"light": true, "weather": true}
            })));
        }
        let p = px as f32;
        for z in &lv.layout.zones {
            let shapes: Vec<Value> = z
                .loops
                .iter()
                .map(|(l, hole)| {
                    let pts: Vec<i64> = l[..l.len() - 1].iter().flat_map(|v| [(v[0] * p).round() as i64, (v[1] * p).round() as i64]).collect();
                    json!({"type": "polygon", "points": pts, "hole": hole})
                })
                .collect();
            let (title, color) = match z.effect {
                "lava" => ("Лава", "#ff6a1a"),
                "swamp" => ("Болото", "#6a8f3a"),
                _ => ("Вода", "#2a8fd6"),
            };
            let region = json!({
                "_id": doc_id('r', regions.len()), "name": title, "color": color, "shapes": shapes,
                "elevation": {"bottom": null, "top": null},
                "flags": {LIVE_MODULE: {"effect": z.effect, "fish": z.effect == "water"}}
            });
            regions.push(tag(region));
        }
        let mut doc = json!({"_id": level_id(k), "name": lv.name, "background": {"src": lv.image_src, "color": "#999999"}});
        if multi {
            doc["elevation"] = json!({"bottom": k as i64 * 10, "top": (k as i64 + 1) * 10});
            doc["sort"] = json!(k as i64 * 100);
        }
        docs.push(doc);
    }
    json!({
        "name": name,
        "width": map.width * px,
        "height": map.height * px,
        "padding": 0,
        "grid": {"type": 1, "size": px, "distance": 5, "units": "ft"},
        // Фон и в уровне (формат 14), и по-старому: JSON без версии Foundry при
        // импорте проходит миграцию со старого формата, и та берёт фон отсюда.
        "background": {"src": levels[0].image_src},
        "levels": docs,
        "initialLevel": LEVEL_ID,
        "tokenVision": true,
        "walls": walls,
        "lights": lights,
        "regions": regions,
        "tiles": tiles,
        "flags": {"kartograf": {"format": 1}}
    })
}

/// Стены, двери, окна и огни одного этажа — в общие списки сцены.
fn level_docs(layout: &Layout, px: u32, leaves: &std::collections::HashMap<String, LeafFile>, walls: &mut Vec<Value>, lights_out: &mut Vec<Value>) {
    let p = px as f32;
    let at = |v: [f32; 2]| [(v[0] * p).round() as i64, (v[1] * p).round() as i64];
    for line in &layout.walls {
        for w in line.windows(2) {
            let (a, b) = (at(w[0]), at(w[1]));
            if a != b {
                walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]]}));
            }
        }
    }
    for po in &layout.portals {
        let (a0, b0) = po.seg;
        let (a, b) = (at(a0), at(b0));
        let mut w = json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]]});
        match po.kind {
            PortalKind::Opening => continue,
            // Дверь закрыта и непрозрачна, пока её не откроют.
            PortalKind::Door | PortalKind::DoubleDoor => {
                w["door"] = json!(1);
                let double = po.kind == PortalKind::DoubleDoor;
                if let Some(leaf) = po.leaf.as_ref().and_then(|k| leaves.get(k)) {
                    // Foundry растягивает створку на длину, а по толщине масштабирует
                    // как `сетка / textureGridSize`: столько точек картинки на клетку
                    // её длины — тогда створка той же толщины, что в Картографе.
                    let len = ((b0[0] - a0[0]).powi(2) + (b0[1] - a0[1]).powi(2)).sqrt() / if double { 2.0 } else { 1.0 };
                    let tgs = (leaf.width as f32 / len.max(0.1)).round().max(1.0);
                    w["animation"] = json!({"type": "swing", "texture": leaf.src, "double": double, "duration": 750, "direction": 1, "flip": false, "strength": 1.0});
                    w["flags"] = json!({"core": {"textureGridSize": tgs}});
                }
            }
            // Окно: сквозь него видно и светит, но не пройти.
            PortalKind::Window => {
                w["sight"] = json!(0);
                w["light"] = json!(0);
                w["sound"] = json!(10);
            }
        }
        walls.push(w);
    }
    for l in &layout.lights {
        let [x, y] = at(l.at);
        // Радиус в единицах сетки сцены: футы, 5 на клетку.
        lights_out.push(json!({
            "_id": doc_id('l', lights_out.len()), "x": x, "y": y,
            "config": {"bright": l.bright * 5.0, "dim": l.dim * 5.0, "color": l.color, "alpha": 0.35,
                       "animation": {"type": l.animation, "speed": 3, "intensity": 3}}
        }));
    }
}

/// Universal VTT (`.dd2vtt`): картинка внутри файла, всё в клетках.
pub fn dd2vtt(map: &Map, px: u32, image_b64: &str, layout: &Layout) -> Value {
    let pt = |v: [f32; 2]| json!({"x": v[0], "y": v[1]});
    let los: Vec<Value> = layout.walls.iter().map(|l| Value::Array(l.iter().map(|&v| pt(v)).collect())).collect();
    let portals: Vec<Value> = layout
        .portals
        .iter()
        .map(|po| {
            let ((a, b), kind) = (&po.seg, &po.kind);
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            json!({
                "position": pt(mid), "bounds": [pt(*a), pt(*b)],
                "rotation": (b[1] - a[1]).atan2(b[0] - a[0]),
                "closed": kind.is_door(), "freestanding": false
            })
        })
        .collect();
    let lights: Vec<Value> = layout
        .lights
        .iter()
        .map(|l| json!({"position": pt(l.at), "range": l.dim, "intensity": 1.0, "color": format!("ff{}", &l.color[1..]), "shadows": true}))
        .collect();
    json!({
        "format": 0.3,
        "resolution": {"map_origin": {"x": 0, "y": 0}, "map_size": {"x": map.width, "y": map.height}, "pixels_per_grid": px},
        "line_of_sight": los,
        "objects_line_of_sight": [],
        "portals": portals,
        "environment": {"baked_lighting": false, "ambient_light": "ffffffff"},
        "lights": lights,
        "image": image_b64
    })
}

/// Путь картинки для `background.src`: относительно папки Data Foundry, если
/// выгрузка лежит в ней (Data узнаётся по соседям `worlds` и `modules`),
/// иначе — одно имя файла.
pub fn foundry_src(image: &std::path::Path) -> String {
    let mut dir = image.parent();
    while let Some(d) = dir {
        if d.join("worlds").is_dir() && d.join("modules").is_dir() {
            if let Ok(rel) = image.strip_prefix(d) {
                return rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            }
        }
        dir = d.parent();
    }
    image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Папка Data Foundry этого компьютера — из его настроек, если он стоит.
pub fn foundry_data_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let text = std::fs::read_to_string(std::path::Path::new(&base).join("FoundryVTT/Config/options.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let data = std::path::PathBuf::from(v.get("dataPath")?.as_str()?).join("Data");
    data.is_dir().then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_becomes_living_region_with_island_hole() {
        let mut m = Map::new(12, 12);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        assert_eq!(m.materials[water].live(), crate::map::Live::Water);
        let all = crate::mask::cells(m.rooms.w, m.rooms.h, (1, 1), (9, 9));
        m.paint(crate::map::Layer::Floor, water, &all);
        let grass = crate::mask::cells(m.rooms.w, m.rooms.h, (4, 4), (6, 6));
        m.paint(crate::map::Layer::Floor, 3, &grass);
        m.prune();
        let lay = layout(&m, &[]);
        assert_eq!(lay.zones.len(), 1);
        let z = &lay.zones[0];
        assert_eq!((z.effect, z.loops.len()), ("water", 2));
        assert_eq!(z.loops.iter().filter(|l| l.1).count(), 1);
        let scene = foundry_levels(&m, "Пруд", &[SceneLevel { name: "Пруд".into(), image_src: "p.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        let r = &scene["regions"][0];
        assert_eq!(r["flags"][LIVE_MODULE]["effect"], "water");
        assert_eq!(r["shapes"].as_array().unwrap().len(), 2);
        assert_eq!(r["shapes"][0]["type"], "polygon");
    }

    #[test]
    fn levels_become_foundry_levels() {
        let mut a = Map::new(6, 4);
        let w = crate::mask::cells(a.rooms.w, a.rooms.h, (1, 1), (3, 2));
        a.paint(crate::map::Layer::Rooms, 0, &w);
        let b = a.new_level();
        let mut b2 = b.clone();
        b2.walls.push(crate::map::Wall { points: vec![[0.0, 0.0], [6.0, 0.0]], closed: false, bends: Vec::new(), style: None });
        let (la, lb) = (layout(&a, &a.room_edges()), layout(&b2, &b2.room_edges()));
        let levels = [
            SceneLevel { name: "Низ".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![RoofTile { src: "k/r.png".into(), rect: [50, 50, 300, 200] }] },
            SceneLevel { name: "Верх".into(), image_src: "k/b.jpg".into(), layout: &lb, roofs: Vec::new() },
        ];
        let scene = foundry_levels(&a, "Дом", &levels, 100, &Default::default());
        let lv = scene["levels"].as_array().unwrap();
        assert_eq!(lv.len(), 2);
        assert_eq!(lv[1]["_id"].as_str().unwrap().len(), 16);
        assert_eq!(lv[1]["elevation"]["bottom"], 10);
        assert_eq!(lv[1]["background"]["src"], "k/b.jpg");
        let walls = scene["walls"].as_array().unwrap();
        // Стена второго этажа видна только на нём; id не повторяются.
        assert_eq!(walls.last().unwrap()["levels"][0], lv[1]["_id"]);
        assert!(walls.iter().filter(|w| w["levels"][0] == lv[0]["_id"]).count() >= 4);
        let ids: std::collections::HashSet<_> = walls.iter().map(|w| w["_id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), walls.len());
        // Крыша — тайл над токенами первого этажа, гаснет над токеном.
        let t = &scene["tiles"][0];
        assert_eq!((t["x"].as_i64(), t["width"].as_i64(), t["elevation"].as_i64()), (Some(50), Some(300), Some(9)));
        assert_eq!(t["occlusion"]["modes"][0], 1);
        assert_eq!((t["texture"]["anchorX"].as_i64(), t["texture"]["anchorY"].as_i64()), (Some(0), Some(0)));
        assert_eq!(t["levels"][0], lv[0]["_id"]);
    }
    use crate::map::{Layer, Portal, Prop, Wall};

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let line = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
        let s = simplify(&line, true, 0.01);
        assert_eq!(s, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]]);
        let open = simplify(&[[0.0, 0.0], [1.0, 0.001], [2.0, 0.0]], false, 0.01);
        assert_eq!(open.len(), 2);
    }

    #[test]
    fn scene_has_walls_doors_windows_and_fire_light() {
        let mut m = Map::new(8, 6);
        m.paint_rect(Layer::Rooms, (1, 1), (4, 3), Some(0));
        m.walls.push(Wall { points: vec![[6.0, 0.0], [6.0, 6.0]], closed: false, bends: Vec::new(), style: None });
        m.portals.push(Portal { a: [2.0, 1.0], b: [3.0, 1.0], kind: PortalKind::Door, asset: None, frame: None });
        m.portals.push(Portal { a: [6.0, 2.0], b: [6.0, 3.0], kind: PortalKind::Window, asset: None, frame: None });
        // Двойная дверь в нижней стене и пустой проём в левой.
        m.portals.push(Portal { a: [2.0, 4.0], b: [4.0, 4.0], kind: PortalKind::DoubleDoor, asset: None, frame: None });
        m.portals.push(Portal { a: [1.0, 2.0], b: [1.0, 3.0], kind: PortalKind::Opening, asset: None, frame: None });
        m.props.push(Prop { asset: format!("{}campfire", crate::library::BUILTIN), pos: [3.0, 2.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None });
        let lay = layout(&m, &m.room_edges());
        let mut leaves = std::collections::HashMap::new();
        leaves.insert(crate::library::DEFAULT_DOOR.to_owned(), LeafFile { src: "kartograf/door.webp".into(), width: 512 });
        let scene = foundry_scene(&m, "Проба", "kartograf/proba.jpg", 100, &lay, &leaves);
        assert_eq!((scene["width"].as_u64(), scene["height"].as_u64()), (Some(800), Some(600)));
        assert_eq!(scene["levels"][0]["background"]["src"], "kartograf/proba.jpg");
        assert_eq!(scene["background"]["src"], "kartograf/proba.jpg");
        let walls = scene["walls"].as_array().unwrap();
        // Комната 4×3: дверь сверху, двойная дверь снизу, пустой проём слева режут
        // её на три ломаные — 2 + 2 + 3 отрезка; ручная стена окном разрезана
        // надвое; плюс дверь, двойная и окно. Пустой проём стеной не стал.
        assert_eq!(walls.len(), 7 + 2 + 3, "{walls:#?}");
        let door = walls.iter().find(|w| w["door"] == 1 && w["animation"]["double"] == false).expect("дверь");
        assert_eq!(door["c"], json!([200, 100, 300, 100]));
        assert_eq!(door["animation"]["texture"], "kartograf/door.webp");
        assert_eq!(door["flags"]["core"]["textureGridSize"], 512.0);
        let double = walls.iter().find(|w| w["animation"]["double"] == true).expect("двойная дверь");
        assert_eq!(double["c"], json!([200, 400, 400, 400]));
        // Каждая створка — клетка: картинка в 512 точек на клетку её длины.
        assert_eq!(double["flags"]["core"]["textureGridSize"], 512.0);
        let window = walls.iter().find(|w| w["sight"] == 0).expect("окно");
        assert_eq!(window["c"], json!([600, 200, 600, 300]));
        let ids: std::collections::HashSet<_> = walls.iter().map(|w| w["_id"].as_str().unwrap().to_owned()).collect();
        assert!(ids.len() == walls.len() && ids.iter().all(|i| i.len() == 16));
        let light = &scene["lights"][0];
        assert_eq!((light["x"].as_i64(), light["y"].as_i64(), light["config"]["dim"].as_f64()), (Some(300), Some(200), Some(30.0)));
        let uv = dd2vtt(&m, 100, "", &lay);
        assert_eq!(uv["portals"].as_array().unwrap().len(), 3);
        assert_eq!(uv["resolution"]["pixels_per_grid"], 100);
    }

    #[test]
    fn src_is_relative_to_foundry_data() {
        let root = std::env::temp_dir().join(format!("kartograf-src-{}", std::process::id()));
        for d in ["worlds", "modules", "kartograf/зал"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        assert_eq!(foundry_src(&root.join("kartograf/зал/зал.jpg")), "kartograf/зал/зал.jpg");
        assert_eq!(foundry_src(&std::env::temp_dir().join("x.jpg")), "x.jpg");
        let _ = std::fs::remove_dir_all(root);
    }
}
