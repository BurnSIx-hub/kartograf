//! Мир для модуля Boundless (SweetyHake, Foundry v14): огромная карта одной сценой,
//! которую модуль подгружает по частям. Формат — по исходникам Boundless 1.1.0
//! (документа нет):
//!
//! - пирамида плиток `{z}/{x}_{y}.webp`: на уровне `z` мир — 2^z × 2^z плиток по
//!   `tilePx` точек; модуль берёт уровень по масштабу (`log2(worldPx·scale/tilePx)`);
//! - манифест уровня: `worldPx`, `tilePx`, `maxZoomZ`, `simGrid` × `simGrid` секторов по
//!   `simSectorPx`; в секторе `"sx_sy"` — стены `w` `[x1, y1, x2, y2, move, sight, light,
//!   sound, dir]`, двери `d` `[x1, y1, x2, y2, door, ds]`, свет `l` `[x, y, dim, bright,
//!   color, alpha, anim]`, звуки `s`, номера тайлов `t` и домов `o`;
//! - дом-оверлей `{cx, cy, w, h, rot, roof}`: крышу модуль кладёт сам с z5 и открывает у
//!   фишки, ниже z5 крыши запечены в плитки;
//! - сцена: размер мира, флаг `boundless` `{enabled, manifestUrl}`.
//!
//! Мир собирается из обычной выгрузки: картинка пола со стенами, файлы крыш и сцена
//! JSON со стенами, светом и тайлами.

use serde_json::{Value, json};

/// С этого уровня пирамиды Boundless кладёт крыши сам (`localZ >= 5`), ниже они
/// запечены в плитки.
pub const ROOF_Z: u32 = 5;

/// Размеры мира: сторона (плитка × 2^maxZoomZ, карта — в левом верхнем углу), плитка,
/// верхний уровень пирамиды, сетка секторов.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct World {
    pub side: u32,
    pub tile: u32,
    pub max_z: u32,
    pub sim: u32,
    pub sector: u32,
}

impl World {
    /// Мир под картинку `w`×`h` точек при `grid` точках на клетку. Плитка у небольших
    /// миров — 512: верхний уровень должен дойти до z5, иначе крыши не откроются никогда.
    /// Сектор — около 25 клеток.
    pub fn new(w: u32, h: u32, grid: u32) -> Self {
        let need = w.max(h).max(1);
        // Около 1/32 стороны: тогда верхний уровень — z5, и крыши открываются вблизи
        // («Иерихон_2», 6000 точек: плитка 512 дала бы z4 — крыши запечены навсегда).
        let tile = (need / 32).max(1).next_power_of_two().clamp(128, 1024);
        let mut max_z = 0;
        while tile << max_z < need {
            max_z += 1;
        }
        let side = tile << max_z;
        let sim = side.div_ceil(grid.max(1) * 25).max(1);
        Self { side, tile, max_z, sim, sector: side.div_ceil(sim) }
    }

    fn sector_of(&self, x: f64, y: f64) -> (u32, u32) {
        let k = |v: f64| ((v.max(0.0) / self.sector as f64) as u32).min(self.sim - 1);
        (k(x), k(y))
    }
}

/// Отрезок, обрезанный по прямоугольнику (Лианг — Барски); ничего не осталось — `None`.
fn clip(p: [f64; 2], q: [f64; 2], r: [f64; 4]) -> Option<([f64; 2], [f64; 2])> {
    let d = [q[0] - p[0], q[1] - p[1]];
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (num, den) in [(p[0] - r[0], -d[0]), (r[2] - p[0], d[0]), (p[1] - r[1], -d[1]), (r[3] - p[1], d[1])] {
        if den.abs() < 1e-12 {
            if num < 0.0 {
                return None;
            }
        } else {
            let t = num / den;
            if den < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
    }
    (t0 < t1 - 1e-9).then(|| ([p[0] + d[0] * t0, p[1] + d[1] * t0], [p[0] + d[0] * t1, p[1] + d[1] * t1]))
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Крыша ли тайл сцены: у Картографа крыша — поверхность (`occlusion.modes` с 2).
pub fn is_roof(tile: &Value) -> bool {
    tile["occlusion"]["modes"].as_array().is_some_and(|m| m.iter().any(|v| v.as_u64() == Some(2)))
}

/// Манифест уровня из сцены JSON обычной выгрузки. `pyramid` — путь к пирамиде от
/// данных Foundry.
pub fn manifest(scene: &Value, world: &World, pyramid: &str) -> Value {
    let mut sectors: std::collections::BTreeMap<(u32, u32), serde_json::Map<String, Value>> = Default::default();
    let mut push = |s: (u32, u32), key: &str, v: Value| {
        sectors.entry(s).or_default().entry(key.to_owned()).or_insert_with(|| json!([])).as_array_mut().unwrap().push(v);
    };
    let num = |v: &Value, d: f64| v.as_f64().unwrap_or(d);
    // Стены — по секторам, длинная режется на куски: модуль берёт стены секторов вокруг фишки.
    for w in scene["walls"].as_array().into_iter().flatten() {
        let c: Vec<f64> = w["c"].as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
        if c.len() != 4 {
            continue;
        }
        let door = w["door"].as_u64().unwrap_or(0);
        if door > 0 {
            let s = world.sector_of((c[0] + c[2]) / 2.0, (c[1] + c[3]) / 2.0);
            push(s, "d", json!([c[0], c[1], c[2], c[3], door, w["ds"].as_u64().unwrap_or(0)]));
            continue;
        }
        let tail = [num(&w["move"], 20.0), num(&w["sight"], 20.0), num(&w["light"], 20.0), num(&w["sound"], 20.0), num(&w["dir"], 0.0)];
        let (a, b) = world.sector_of(c[0].min(c[2]), c[1].min(c[3]));
        let (e, f) = world.sector_of(c[0].max(c[2]), c[1].max(c[3]));
        for sy in b..=f {
            for sx in a..=e {
                let sz = world.sector as f64;
                let rect = [sx as f64 * sz, sy as f64 * sz, (sx + 1) as f64 * sz, (sy + 1) as f64 * sz];
                if let Some((p, q)) = clip([c[0], c[1]], [c[2], c[3]], rect) {
                    let mut row = vec![r1(p[0]), r1(p[1]), r1(q[0]), r1(q[1])];
                    row.extend(tail);
                    push((sx, sy), "w", json!(row));
                }
            }
        }
    }
    for l in scene["lights"].as_array().into_iter().flatten() {
        let c = &l["config"];
        let (x, y) = (num(&l["x"], 0.0), num(&l["y"], 0.0));
        push(world.sector_of(x, y), "l", json!([x, y, num(&c["dim"], 0.0), num(&c["bright"], 0.0), c["color"], num(&c["alpha"], 0.5), c["animation"]["type"].as_str().unwrap_or("")]));
    }
    for s in scene["sounds"].as_array().into_iter().flatten() {
        let (x, y) = (num(&s["x"], 0.0), num(&s["y"], 0.0));
        push(
            world.sector_of(x, y),
            "s",
            json!([x, y, num(&s["radius"], 20.0), s["path"], num(&s["volume"], 0.6), s["walls"].as_bool().unwrap_or(true), s["easing"].as_bool().unwrap_or(true), num(&s["darkness"]["min"], 0.0), num(&s["darkness"]["max"], 1.0)]),
        );
    }
    // Тайлы и крыши. У Foundry 14 x, y тайла — середина; у Boundless тайл без anchor —
    // от левого верхнего угла, дом — от середины.
    let mut tiles = Vec::new();
    let mut overlays = serde_json::Map::new();
    let all: Vec<&Value> = scene["tiles"].as_array().into_iter().flatten().collect();
    // Рамки крыш (Foundry 14: середина и размер): что под крышей, ниже уровня крыш не видно.
    let roof_rects: Vec<[f64; 4]> = all.iter().filter(|t| is_roof(t)).map(|t| [num(&t["x"], 0.0), num(&t["y"], 0.0), num(&t["width"], 0.0), num(&t["height"], 0.0)]).collect();
    let under_roof = |x: f64, y: f64| roof_rects.iter().any(|r| (x - r[0]).abs() <= r[2] / 2.0 && (y - r[1]).abs() <= r[3] / 2.0);
    for t in all {
        let (x, y, w, h) = (num(&t["x"], 0.0), num(&t["y"], 0.0), num(&t["width"], 0.0), num(&t["height"], 0.0));
        let src = t["texture"]["src"].clone();
        let rot = num(&t["rotation"], 0.0);
        if is_roof(t) {
            let id = overlays.len();
            overlays.insert(id.to_string(), json!({"cx": x, "cy": y, "w": w, "h": h, "rot": rot, "roof": src}));
            push(world.sector_of(x, y), "o", json!(id));
            continue;
        }
        let id = tiles.len();
        let mut tile = json!({
            "src": src, "x": x - w / 2.0, "y": y - h / 2.0, "w": w, "h": h, "rot": rot,
            "elevation": num(&t["elevation"], 0.0), "sort": num(&t["sort"], 0.0),
            "occlusionMode": t["occlusion"]["modes"].clone(), "occludedAlpha": num(&t["occlusion"]["alpha"], 0.0),
            "restrictsLight": t["restrictions"]["light"].as_bool().unwrap_or(false), "restrictsWeather": t["restrictions"]["weather"].as_bool().unwrap_or(false)
        });
        // Мебель в доме: ниже уровня крыш крыша запечена в плитки, и тайл лёг бы поверх неё.
        if under_roof(x, y) {
            tile["minZ"] = json!(ROOF_Z);
        }
        tiles.push(tile);
        push(world.sector_of(x, y), "t", json!(id));
    }
    let sectors: serde_json::Map<String, Value> = sectors.into_iter().map(|((x, y), v)| (format!("{x}_{y}"), Value::Object(v))).collect();
    json!({
        "worldPx": world.side, "tilePx": world.tile, "maxZoomZ": world.max_z,
        "simGrid": world.sim, "simSectorPx": world.sector,
        "pyramidBaseUrl": pyramid,
        "bounds": {"x": 0, "y": 0, "w": world.side, "h": world.side},
        "sectors": sectors, "overlays": overlays, "tiles": tiles
    })
}

/// Сцена для Boundless: размер мира, сетка и свет сцены выгрузки, флаг модуля. Стен,
/// света и тайлов в ней нет — их подгружает модуль из манифеста.
pub fn scene(scene: &Value, world: &World, manifest: &str) -> Value {
    json!({
        "name": format!("{} (Boundless)", scene["name"].as_str().unwrap_or("Картограф")),
        "width": world.side, "height": world.side, "padding": 0,
        "grid": scene["grid"], "tokenVision": true,
        "environment": scene["environment"],
        "backgroundColor": "#000000",
        "flags": {"boundless": {"enabled": true, "manifestUrl": manifest}}
    })
}

/// Вдвое меньше: среднее квадрата 2×2.
fn half(img: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = ((img.width() / 2).max(1), (img.height() / 2).max(1));
    let mut out = image::RgbaImage::new(w, h);
    let src = img.as_raw();
    let sw = img.width() as usize;
    let dst: &mut [u8] = &mut out;
    crate::export::par_bands(dst, w as usize * 4, |y0, band| {
        for (row, line) in band.chunks_mut(w as usize * 4).enumerate() {
            let y = (y0 + row) * 2;
            for x in 0..w as usize {
                for c in 0..4 {
                    let at = |yy: usize, xx: usize| src[(yy * sw + xx) * 4 + c] as u32;
                    line[x * 4 + c] = ((at(y, 2 * x) + at(y, 2 * x + 1) + at(y + 1, 2 * x) + at(y + 1, 2 * x + 1) + 2) / 4) as u8;
                }
            }
        }
    });
    out
}

/// Крыша для запекания: картинка, середина и размер в точках мира.
pub struct BakedRoof {
    pub image: image::RgbaImage,
    pub center: [f64; 2],
    pub size: [f64; 2],
}

/// Пирамида: файлы `z/x_y.webp` и их байты. `plain` — мир без крыш (`side`×`side`,
/// карта со стенами в левом верхнем углу); уровни ниже [`ROOF_Z`] — с крышами.
pub fn pyramid(plain: image::RgbaImage, roofs: &[BakedRoof], world: &World) -> Vec<(String, Vec<u8>)> {
    let mut roofed = (world.max_z > 0 && !roofs.is_empty()).then(|| plain.clone());
    if let Some(r) = &mut roofed {
        for roof in roofs {
            let (w, h) = (roof.size[0].round().max(1.0) as u32, roof.size[1].round().max(1.0) as u32);
            let pic = if roof.image.dimensions() == (w, h) { roof.image.clone() } else { image::imageops::resize(&roof.image, w, h, image::imageops::FilterType::Triangle) };
            image::imageops::overlay(r, &pic, (roof.center[0] - w as f64 / 2.0).round() as i64, (roof.center[1] - h as f64 / 2.0).round() as i64);
        }
    }
    let mut out = Vec::new();
    let cut = |img: &image::RgbaImage, z: u32, out: &mut Vec<(String, Vec<u8>)>| {
        let n = 1u32 << z;
        let t = img.width() / n;
        let cells: Vec<(u32, u32)> = (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).collect();
        let bytes = crate::export::par_map(&cells, |&(x, y)| {
            let tile = image::imageops::crop_imm(img, x * t, y * t, t, t).to_image();
            let tile = if t == world.tile { tile } else { image::imageops::resize(&tile, world.tile, world.tile, image::imageops::FilterType::Triangle) };
            Some(crate::export::layer_bytes(&tile, 88.0).0)
        });
        for ((x, y), b) in cells.into_iter().zip(bytes) {
            out.push((format!("{z}/{x}_{y}.webp"), b.unwrap_or_default()));
        }
    };
    let (mut a, mut b) = (plain, roofed);
    for z in (0..=world.max_z).rev() {
        let img = if z < ROOF_Z { b.as_ref().unwrap_or(&a) } else { &a };
        cut(img, z, &mut out);
        if z > 0 {
            if z >= ROOF_Z || b.is_none() {
                a = half(&a);
            }
            if let Some(r) = &b {
                b = Some(half(r));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_is_a_whole_number_of_tiles_and_reaches_the_roof_level() {
        // King's Cross: 16 000 × 16 256 при 64 на клетку.
        let w = World::new(16000, 16256, 64);
        assert_eq!((w.tile, w.max_z, w.side), (512, 5, 16384));
        assert!(w.max_z >= ROOF_Z, "крыши должны открываться");
        // «Иерихон_2»: 6000 × 4400 при 100.
        let j = World::new(6000, 4400, 100);
        assert_eq!((j.tile, j.max_z, j.side), (256, 5, 8192));
        // 1000×1000 при 100 на клетку — плитка 1024.
        let big = World::new(100_000, 100_000, 100);
        assert_eq!((big.tile, big.max_z, big.side), (1024, 7, 131_072));
        assert_eq!(big.sector * big.sim >= big.side, true);
    }

    #[test]
    fn walls_are_cut_by_sectors_and_tiles_move_to_their_corner() {
        let world = World { side: 1000, tile: 500, max_z: 1, sim: 2, sector: 500 };
        let scene = json!({
            "walls": [
                {"c": [100, 100, 900, 100]},
                {"c": [400, 400, 450, 450], "door": 1, "ds": 0},
                {"c": [600, 600, 700, 600], "move": 20, "sight": 0, "light": 0, "sound": 10}
            ],
            "lights": [{"x": 700, "y": 200, "config": {"dim": 30, "bright": 15, "color": "#ffc870", "alpha": 0.35, "animation": {"type": "torch"}}}],
            "tiles": [
                {"x": 250, "y": 750, "width": 100, "height": 50, "rotation": 90, "texture": {"src": "k/tree.webp"}, "occlusion": {"modes": [1], "alpha": 0}},
                {"x": 800, "y": 800, "width": 200, "height": 100, "texture": {"src": "k/roof_1.webp"}, "occlusion": {"modes": [2], "alpha": 0}},
                {"x": 780, "y": 790, "width": 50, "height": 50, "texture": {"src": "k/table.webp"}, "occlusion": {"modes": [1], "alpha": 0}}
            ]
        });
        let m = manifest(&scene, &world, "k/pyramid");
        let s = &m["sectors"];
        assert_eq!(s["0_0"]["w"][0], json!([100.0, 100.0, 500.0, 100.0, 20.0, 20.0, 20.0, 20.0, 0.0]), "стена через два сектора — двумя кусками");
        assert_eq!(s["1_0"]["w"][0], json!([500.0, 100.0, 900.0, 100.0, 20.0, 20.0, 20.0, 20.0, 0.0]));
        assert_eq!(s["0_0"]["d"][0], json!([400.0, 400.0, 450.0, 450.0, 1, 0]));
        assert_eq!(s["1_1"]["w"][0], json!([600.0, 600.0, 700.0, 600.0, 20.0, 0.0, 0.0, 10.0, 0.0]), "низкая стена — со своими свойствами");
        assert_eq!(s["1_0"]["l"][0], json!([700.0, 200.0, 30.0, 15.0, "#ffc870", 0.35, "torch"]));
        assert_eq!(m["tiles"][0]["x"], 200.0);
        assert_eq!(m["tiles"][0]["y"], 725.0);
        assert_eq!(s["0_1"]["t"], json!([0]));
        assert_eq!(m["overlays"]["0"], json!({"cx": 800.0, "cy": 800.0, "w": 200.0, "h": 100.0, "rot": 0.0, "roof": "k/roof_1.webp"}));
        assert_eq!(s["1_1"]["o"], json!([0]));
        assert_eq!(m["tiles"].as_array().unwrap().len(), 2, "крыша — домом, а не тайлом");
        assert_eq!(m["tiles"][1]["minZ"], json!(ROOF_Z), "стол в доме — только с уровня крыш");
        assert!(m["tiles"][0].get("minZ").is_none(), "дерево не под крышей — видно всегда");
    }

    #[test]
    fn roofs_are_baked_below_the_roof_level_only() {
        // Мир 6 уровней по 4 точки: z0..z4 — с крышей, z5 — без.
        let world = World { side: 128, tile: 4, max_z: 5, sim: 1, sector: 128 };
        let plain = image::RgbaImage::from_pixel(128, 128, image::Rgba([0, 0, 255, 255]));
        let roof = BakedRoof { image: image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])), center: [4.0, 4.0], size: [8.0, 8.0] };
        let files = pyramid(plain, &[roof], &world);
        assert_eq!(files.len(), (0..=5).map(|z| 1usize << (2 * z)).sum::<usize>());
        let px = |name: &str| {
            let b = &files.iter().find(|f| f.0 == format!("{name}.webp")).unwrap().1;
            image::load_from_memory(b).unwrap().to_rgba8().get_pixel(0, 0).0
        };
        assert!(px("5/0_0")[2] > 200 && px("5/0_0")[0] < 40, "z5 — без крыши: {:?}", px("5/0_0"));
        assert!(px("4/0_0")[0] > 200, "z4 — с крышей: {:?}", px("4/0_0"));
        assert!(px("0/0_0")[2] > 200, "z0 — весь мир, крыша мелкая");
    }
}
