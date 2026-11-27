//! Запись 3D-сцены карты (`scene3d`) в `.glb` для 3D Canvas.
//!
//! Каждый узел сцены — свой узел glTF со своей сеткой: three.js кладёт `extras` узла в его
//! `userData`, и по этим меткам 3D Canvas решает, что в модели твёрдое и что закрывает обзор:
//! - стекло: `sight: false` — сквозь окно видно, но не пройти;
//! - створка: `door` и `swing` для модуля `burn-3d-fx`, нетвёрдая и прозрачная для обзора.
//!
//! Метку 3D Canvas читает у узла-сетки, а узел с одним куском three.js и делает сеткой, поэтому
//! у створки и стекла кусок всегда один. Картинки — какими их дала выгрузка: JPEG, PNG или WebP
//! (`EXT_texture_webp`, его three.js понимает).

use crate::scene3d::{Alpha, Look, Part, Scene3d};
use serde_json::{Value, json};

#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let mut v = json!({"buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = t.into();
        }
        self.data.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }
}

/// Цвет sRGB в линейный (так glTF хранит `baseColorFactor`).
fn linear(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    })
}

/// Метки узла: этаж (`storey`, с нуля) у всех и метки 3D Canvas у створок и стекла.
///
/// Створка — не дверь 3D Canvas (`isDoor`): её щелчок в 3D открывал бы створку отдельно от
/// двери Foundry. Её поворачивает модуль `burn-3d-fx` вслед за дверью Foundry по `door`
/// (этаж и концы стены-двери в точках сцены) на `swing` радиан. Ход и обзор держит сама дверь
/// Foundry, поэтому створка нетвёрдая и прозрачная для обзора 3D Canvas. Метки на частях к
/// тому же не дают 3D Canvas запечь тайл в общую сетку — створки остаются подвижными.
fn extras(node: &crate::scene3d::Node) -> Value {
    let mut e = json!({"storey": node.storey});
    match (node.part, node.door.as_ref()) {
        (Part::Door, Some(d)) => {
            for (k, v) in [("door", json!(d.id)), ("swing", json!(d.swing)), ("collision", json!(false)), ("sight", json!(false))] {
                e[k] = v;
            }
        }
        (Part::Glass, _) => e["sight"] = false.into(),
        _ => {}
    }
    e
}

/// `.glb` сцены. `images[i]` — байты и MIME картинки `scene.pictures[i]`.
pub fn write(scene: &Scene3d, images: &[(Vec<u8>, &str)], name: &str) -> Result<Vec<u8>, String> {
    const ARRAY: u32 = 34962;
    const ELEMENTS: u32 = 34963;
    if images.len() != scene.pictures.len() {
        return Err(format!("картинок {} на {} в сцене", images.len(), scene.pictures.len()));
    }
    let mut bin = Bin::default();
    let mut webp = false;
    let mut images_json = Vec::new();
    for (bytes, mime) in images {
        let view = bin.view(bytes, None);
        webp |= *mime == "image/webp";
        images_json.push(json!({"bufferView": view, "mimeType": mime}));
    }
    // Сэмплер: повтор плиткой (стены) или один раз с краем (пол этажа).
    let samplers = json!([
        {"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497},
        {"magFilter": 9729, "minFilter": 9987, "wrapS": 33071, "wrapT": 33071},
    ]);
    let mut textures: Vec<Value> = Vec::new();
    let mut texture_of = std::collections::HashMap::new();
    let mut materials = Vec::new();
    for m in &scene.materials {
        let mut pbr = json!({"metallicFactor": 0.0, "roughnessFactor": 0.9});
        match m.look {
            Look::Picture(p) => {
                let key = (p, m.repeat);
                let t = *texture_of.entry(key).or_insert_with(|| {
                    let sampler = if m.repeat { 0 } else { 1 };
                    let source = if images[p].1 == "image/webp" { json!({"sampler": sampler, "extensions": {"EXT_texture_webp": {"source": p}}}) } else { json!({"sampler": sampler, "source": p}) };
                    textures.push(source);
                    textures.len() - 1
                });
                pbr["baseColorTexture"] = json!({"index": t});
                pbr["baseColorFactor"] = json!([1.0, 1.0, 1.0, m.opacity]);
            }
            Look::Color(c) => {
                let l = linear(c);
                pbr["baseColorFactor"] = json!([l[0], l[1], l[2], m.opacity]);
            }
        }
        let mut mat = json!({"pbrMetallicRoughness": pbr, "doubleSided": m.two_sided});
        match m.alpha {
            Alpha::Opaque => mat["alphaMode"] = "OPAQUE".into(),
            Alpha::Mask => {
                mat["alphaMode"] = "MASK".into();
                mat["alphaCutoff"] = 0.5.into();
            }
            Alpha::Blend => mat["alphaMode"] = "BLEND".into(),
        }
        materials.push(mat);
    }

    let mut accessors: Vec<Value> = Vec::new();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    for node in &scene.nodes {
        let mut prims = Vec::new();
        for (mat, mesh) in &node.prims {
            if mesh.idx.is_empty() {
                continue;
            }
            let n = mesh.v.len();
            let (lo, hi) = mesh.v.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| ([0, 1, 2].map(|k| lo[k].min(v.p[k])), [0, 1, 2].map(|k| hi[k].max(v.p[k]))));
            let mut acc = |bin: &mut Bin, bytes: Vec<u8>, kind: &str, extra: Value| -> usize {
                let view = bin.view(&bytes, Some(ARRAY));
                let mut a = json!({"bufferView": view, "componentType": 5126, "count": n, "type": kind});
                if let Value::Object(e) = extra {
                    for (k, v) in e {
                        a[k] = v;
                    }
                }
                accessors.push(a);
                accessors.len() - 1
            };
            let pos = acc(&mut bin, floats(mesh.v.iter().map(|v| v.p)), "VEC3", json!({"min": lo, "max": hi}));
            let nrm = acc(&mut bin, floats(mesh.v.iter().map(|v| v.n)), "VEC3", json!({}));
            let uv = acc(&mut bin, floats(mesh.v.iter().map(|v| v.uv)), "VEC2", json!({}));
            let idx: Vec<u8> = mesh.idx.iter().flat_map(|i| i.to_le_bytes()).collect();
            let view = bin.view(&idx, Some(ELEMENTS));
            accessors.push(json!({"bufferView": view, "componentType": 5125, "count": mesh.idx.len(), "type": "SCALAR"}));
            prims.push(json!({"attributes": {"POSITION": pos, "NORMAL": nrm, "TEXCOORD_0": uv}, "indices": accessors.len() - 1, "material": mat, "mode": 4}));
        }
        if prims.is_empty() {
            continue;
        }
        meshes.push(json!({"name": node.name, "primitives": prims}));
        let mut n = json!({"name": node.name, "mesh": meshes.len() - 1});
        if node.at != [0.0; 3] {
            n["translation"] = json!(node.at);
        }
        if node.yaw != 0.0 {
            let (s, c) = (node.yaw / 2.0).sin_cos();
            n["rotation"] = json!([0.0, s, 0.0, c]);
        }
        n["extras"] = extras(node);
        nodes.push(n);
    }
    // Модель без анимаций 3D Canvas при загрузке «упрощает»: вписывает сдвиг и поворот узлов
    // в геометрию и склеивает сетки с одинаковым материалом (`applyTransforms`, `mergeMeshes`).
    // Створки слиплись бы в одну сетку без меток, а петля ушла бы в начало модели. Модель
    // с анимацией он оставляет как есть, поэтому здесь неподвижная «анимация» пустого узла:
    // плеер анимаций тайла водит только его.
    let hold = nodes.len();
    nodes.push(json!({"name": "hold"}));
    let times = bin.view(&floats([[0.0f32], [1.0]].into_iter()), None);
    accessors.push(json!({"bufferView": times, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0.0], "max": [1.0]}));
    let places = bin.view(&floats([[0.0f32; 3], [0.0; 3]].into_iter()), None);
    accessors.push(json!({"bufferView": places, "componentType": 5126, "count": 2, "type": "VEC3"}));
    let animation = json!({"name": "hold", "channels": [{"sampler": 0, "target": {"node": hold, "path": "translation"}}], "samplers": [{"input": accessors.len() - 2, "output": accessors.len() - 1, "interpolation": "STEP"}]});
    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    let roots: Vec<usize> = (0..nodes.len()).collect();
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": format!("Kartograf {}", env!("CARGO_PKG_VERSION"))},
        "scene": 0,
        "scenes": [{"name": name, "nodes": roots}],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
        "accessors": accessors,
        "animations": [animation],
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.data.len()}],
    });
    if !textures.is_empty() {
        doc["textures"] = textures.into();
        doc["images"] = images_json.into();
        doc["samplers"] = samplers;
    }
    if webp {
        doc["extensionsUsed"] = json!(["EXT_texture_webp"]);
        doc["extensionsRequired"] = json!(["EXT_texture_webp"]);
    }
    let mut js = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.data.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin.data);
    Ok(out)
}

fn floats<const N: usize>(vals: impl Iterator<Item = [f32; N]>) -> Vec<u8> {
    vals.flat_map(|a| a.into_iter().flat_map(f32::to_le_bytes)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::PortalKind;
    use crate::scene3d::{Opening, Picture, Storey, WallLook, build};

    fn json_of(glb: &[u8]) -> Value {
        assert_eq!(&glb[0..4], b"glTF");
        let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        assert_eq!(&glb[16..20], b"JSON");
        assert_eq!(u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize, glb.len());
        serde_json::from_slice(&glb[20..20 + len]).unwrap()
    }

    fn sample() -> Scene3d {
        let door = Opening { seg: ([2.0, 1.0], [3.0, 1.0]), kind: PortalKind::Door, id: "wDoor".into(), art: None };
        let win = Opening { seg: ([0.5, 3.0], [1.5, 3.0]), kind: PortalKind::Window, id: "wWin".into(), art: None };
        let st = Storey {
            floor: Some(Picture { key: "floor_1".into(), size: [4, 4], alpha: false }),
            walls: vec![vec![[0.0, 1.0], [2.0, 1.0]], vec![[3.0, 1.0], [4.0, 1.0]]],
            low: Vec::new(),
            openings: vec![door, win],
            gaps: Vec::new(),
            look: WallLook { width: 0.25, strip: None, side: None, color: [80, 70, 60] },
            ground: Default::default(),
            roofs: Vec::new(),
            cover: None,
            caps: Vec::new(),
        };
        build([4.0, 4.0], &[st])
    }

    #[test]
    fn doors_and_glass_carry_the_3d_canvas_tags() {
        let s = sample();
        // Картинки — пол и тело створки.
        let glb = write(&s, &vec![(vec![0xFF, 0xD8, 0xFF], "image/jpeg"); s.pictures.len()], "test").unwrap();
        let doc = json_of(&glb);
        let nodes = doc["nodes"].as_array().unwrap();
        let door = nodes.iter().find(|n| n["extras"]["door"] == "wDoor").unwrap();
        assert!(door["extras"].get("isDoor").is_none(), "не дверь 3D Canvas: её водит модуль вслед за дверью Foundry");
        assert_eq!((door["extras"]["collision"].as_bool(), door["extras"]["sight"].as_bool()), (Some(false), Some(false)));
        assert!((door["extras"]["swing"].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
        assert_eq!(door["translation"], json!([2.0, 0.0, 1.0]));
        // У створки один кусок: three.js делает такой узел сеткой, и метка ложится на неё.
        let mesh = &doc["meshes"][door["mesh"].as_u64().unwrap() as usize];
        assert_eq!(mesh["primitives"].as_array().unwrap().len(), 1);
        let glass = nodes.iter().find(|n| n["extras"]["sight"] == false && n["extras"].get("door").is_none()).unwrap();
        assert!(glass["extras"].get("collision").is_none(), "стекло твёрдое — по флагам тайла");
        assert!(glass["name"].as_str().unwrap().starts_with("window"));
        let walls = nodes.iter().find(|n| n["name"] == "walls_1").unwrap();
        assert_eq!(walls["extras"], json!({"storey": 0}), "стены — по флагам тайла");
        assert!(doc.get("extensionsRequired").is_none());
    }

    #[test]
    fn the_floor_picture_is_laid_once_and_webp_is_declared() {
        let s = sample();
        let glb = write(&s, &vec![(b"RIFF....WEBP".to_vec(), "image/webp"); s.pictures.len()], "test").unwrap();
        let doc = json_of(&glb);
        assert_eq!(doc["extensionsRequired"], json!(["EXT_texture_webp"]));
        let tex = &doc["textures"][0];
        assert_eq!(tex["extensions"]["EXT_texture_webp"]["source"], 0);
        assert_eq!(doc["samplers"][tex["sampler"].as_u64().unwrap() as usize]["wrapS"], 33071);
        // Все куски пишутся с нормалями и UV, индексы — u32.
        for m in doc["meshes"].as_array().unwrap() {
            for p in m["primitives"].as_array().unwrap() {
                assert!(p["attributes"].get("NORMAL").is_some() && p["attributes"].get("TEXCOORD_0").is_some());
                assert_eq!(doc["accessors"][p["indices"].as_u64().unwrap() as usize]["componentType"], 5125);
            }
        }
    }

    #[test]
    fn a_still_animation_keeps_3d_canvas_from_merging_the_leaves() {
        let s = sample();
        let doc = json_of(&write(&s, &vec![(vec![0xFF, 0xD8, 0xFF], "image/jpeg"); s.pictures.len()], "test").unwrap());
        let anim = &doc["animations"][0];
        let node = anim["channels"][0]["target"]["node"].as_u64().unwrap() as usize;
        // Водит пустой узел, а не створку и не стены.
        assert_eq!(doc["nodes"][node]["name"], "hold");
        assert!(doc["nodes"][node].get("mesh").is_none());
        let input = &doc["accessors"][anim["samplers"][0]["input"].as_u64().unwrap() as usize];
        assert_eq!((input["min"].clone(), input["max"].clone()), (json!([0.0]), json!([1.0])));
        assert!(doc["scenes"][0]["nodes"].as_array().unwrap().contains(&json!(node)));
    }

    #[test]
    fn pictures_must_match_the_scene() {
        assert!(write(&sample(), &[], "test").is_err());
    }
}
