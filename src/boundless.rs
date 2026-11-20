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

/// Сторона маски крыш всего мира: Boundless растягивает её на мир и по ней гасит свет
/// под крышами издалека. Своей у него — по прямоугольникам домов, у Картографа крыши любой формы.
pub const ROOF_MASK: u32 = 2048;

/// Маска крыш мира: белое там, где крыша, по прозрачности её картинки.
pub struct RoofMask {
    img: image::RgbaImage,
    scale: f64,
    any: bool,
}

impl RoofMask {
    pub fn new(world: &World) -> Self {
        Self { img: image::RgbaImage::new(ROOF_MASK, ROOF_MASK), scale: ROOF_MASK as f64 / world.side as f64, any: false }
    }

    /// Крыша куска: середина — в точках куска, `offset` — начало куска в мире.
    pub fn add(&mut self, roof: &BakedRoof, offset: [f64; 2]) {
        let s = self.scale;
        let (w, h) = ((roof.size[0] * s).round().max(1.0) as u32, (roof.size[1] * s).round().max(1.0) as u32);
        let pic = image::imageops::resize(&roof.image, w, h, image::imageops::FilterType::Triangle);
        let (x0, y0) = (((roof.center[0] + offset[0]) * s - w as f64 / 2.0).round() as i64, ((roof.center[1] + offset[1]) * s - h as f64 / 2.0).round() as i64);
        for (x, y, p) in pic.enumerate_pixels() {
            let (gx, gy) = (x0 + x as i64, y0 + y as i64);
            if gx < 0 || gy < 0 || gx >= ROOF_MASK as i64 || gy >= ROOF_MASK as i64 || p[3] == 0 {
                continue;
            }
            let q = self.img.get_pixel_mut(gx as u32, gy as u32);
            *q = image::Rgba([255, 255, 255, q[3].max(p[3])]);
            self.any = true;
        }
    }

    /// Картинка маски; крыш нет — `None`.
    pub fn bytes(&self) -> Option<Vec<u8>> {
        self.any.then(|| webp(&self.img))
    }
}

/// Наибольшая сторона куска мира, что снимается за раз, в точках: кусок — обычный
/// район выгрузки, со всеми слоями в памяти (около гигабайта).
pub const CHUNK_PX: u32 = 8192;

/// Размеры мира: сторона (плитка × 2^maxZoomZ, карта — в левом верхнем углу), плитка
/// (целое число клеток), верхний уровень пирамиды, сетка секторов и уровень кусков: на
/// `chunk_z` кусок — одна плитка, выше он режется сам, ниже — плитки собираются с диска.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct World {
    pub side: u32,
    pub tile: u32,
    pub max_z: u32,
    pub sim: u32,
    pub sector: u32,
    pub cell_px: u32,
    pub chunk_z: u32,
}

impl World {
    /// Мир под картинку `w`×`h` точек при `px` точках на клетку. Сектор — около 25 клеток.
    pub fn new(w: u32, h: u32, px: u32) -> Self {
        let need = w.max(h).max(1);
        let px = px.max(1);
        // Плитка — около 1/32 стороны: тогда верхний уровень — z5, и крыши открываются
        // вблизи («Иерихон_2», 6000 точек: плитка 512 дала бы z4 — крыши запечены
        // навсегда). Целым числом клеток — тогда куски мира кончаются на границе клеток.
        let target = (need / 32).clamp(128, 1024);
        let tile = ((target as f64 / px as f64).round() as u32).max(1) * px;
        // Маленькая карта (меньше ~40 клеток) уровня z5 не набрала бы: дома Boundless ставит с z5,
        // мебель под крышами — тоже, и её не было бы совсем. Такой мир достраивается пустыми плитками.
        let mut max_z = 0;
        while tile << max_z < need || max_z < ROOF_Z {
            max_z += 1;
        }
        let side = tile << max_z;
        let sim = side.div_ceil(px * 25).max(1);
        let mut c = 0;
        while c < max_z && tile << (c + 1) <= CHUNK_PX {
            c += 1;
        }
        Self { side, tile, max_z, sim, sector: side.div_ceil(sim), cell_px: px, chunk_z: max_z - c }
    }

    /// Сторона куска в точках.
    pub fn chunk_px(&self) -> u32 {
        self.tile << (self.max_z - self.chunk_z)
    }

    /// Сторона куска в клетках.
    pub fn chunk_cells(&self) -> u32 {
        self.chunk_px() / self.cell_px
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
            let mut ov = json!({"cx": x, "cy": y, "w": w, "h": h, "rot": rot, "roof": src});
            if t["flags"].is_object() {
                ov["flags"] = t["flags"].clone();
            }
            overlays.insert(id.to_string(), ov);
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
        // Флаги Wild Canvas: крона качается, гамак ходит, дирижабль парит.
        if t["flags"].is_object() {
            tile["flags"] = t["flags"].clone();
        }
        // Подкрашенный предмет: цвет Boundless кладёт сам. Отражения он не знает — отражённое
        // Картограф для него кладёт своей картинкой (`app::exporting`).
        if let Some(tint) = t["texture"]["tint"].as_str() {
            tile["tint"] = json!(tint);
        }
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
///
/// Многоэтажная (`scene["levels"]` — уровни Foundry 14 из выгрузки): у каждого уровня свой
/// манифест в `flags.boundless.manifestUrl`, картинок уровня нет — их даёт пирамида.
///
/// `live` — области (вода, звуки мест, переходы между этажами) и флаги сцены обычной
/// выгрузки всей карты: Wild Canvas оживляет их и в сцене Boundless.
pub fn scene(scene: &Value, world: &World, manifests: &[String], live: &Value) -> Value {
    let mut out = json!({
        "name": format!("{} (Boundless)", scene["name"].as_str().unwrap_or("Картограф")),
        "width": world.side, "height": world.side, "padding": 0,
        "grid": scene["grid"], "tokenVision": true,
        "environment": scene["environment"],
        "backgroundColor": "#000000",
        "flags": {"boundless": {"enabled": true, "manifestUrl": manifests.first()}}
    });
    if let Some(flags) = live["flags"].as_object() {
        for (k, v) in flags.iter().filter(|(k, _)| k.as_str() != "boundless") {
            out["flags"][k] = v.clone();
        }
    }
    if let Some(regions) = live["regions"].as_array().filter(|r| !r.is_empty()) {
        out["regions"] = json!(regions);
    }
    let levels = scene["levels"].as_array().filter(|l| l.len() > 1 && l.len() == manifests.len());
    if let Some(levels) = levels {
        let docs: Vec<Value> = levels
            .iter()
            .zip(manifests)
            .map(|(lv, url)| {
                let mut doc = json!({"_id": lv["_id"], "name": lv["name"], "elevation": lv["elevation"], "background": {"color": "#000000"}, "flags": {"boundless": {"manifestUrl": url}}});
                if !lv["visibility"].is_null() {
                    doc["visibility"] = lv["visibility"].clone();
                }
                doc
            })
            .collect();
        out["levels"] = json!(docs);
        if !scene["initialLevel"].is_null() {
            out["initialLevel"] = scene["initialLevel"].clone();
        }
    }
    out
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

fn webp(img: &image::RgbaImage) -> Vec<u8> {
    crate::export::layer_bytes(img, 88.0).0
}

/// Пустая плитка: там, где карты нет (мир — квадрат 2^z плиток), модуль всё равно её спросит.
pub fn empty_tile(world: &World) -> Vec<u8> {
    webp(&image::RgbaImage::new(world.tile, world.tile))
}

/// Плитки куска мира: уровни от `max_z` до `chunk_z`. `plain` — кусок (`chunk_px` ×
/// `chunk_px`, лишнее за картой прозрачно) без крыш, `t0` — его левая верхняя плитка на
/// `max_z`. Уровни ниже [`ROOF_Z`] — с крышами. Если кусок кончается выше них
/// (`chunk_z >= ROOF_Z`), его плитка `chunk_z` с крышами — вторым: из неё соберутся
/// нижние уровни.
pub fn chunk_tiles(plain: image::RgbaImage, roofs: &[BakedRoof], world: &World, t0: [u32; 2]) -> (Vec<(String, Vec<u8>)>, Option<Vec<u8>>) {
    let mut roofed = plain.clone();
    for roof in roofs {
        let (w, h) = (roof.size[0].round().max(1.0) as u32, roof.size[1].round().max(1.0) as u32);
        let pic = if roof.image.dimensions() == (w, h) { roof.image.clone() } else { image::imageops::resize(&roof.image, w, h, image::imageops::FilterType::Triangle) };
        image::imageops::overlay(&mut roofed, &pic, (roof.center[0] - w as f64 / 2.0).round() as i64, (roof.center[1] - h as f64 / 2.0).round() as i64);
    }
    let mut out = Vec::new();
    let (mut a, mut b) = (plain, roofed);
    for z in (world.chunk_z..=world.max_z).rev() {
        let img = if z < ROOF_Z { &b } else { &a };
        let m = 1u32 << (z - world.chunk_z);
        let shift = world.max_z - z;
        let cells: Vec<(u32, u32)> = (0..m).flat_map(|y| (0..m).map(move |x| (x, y))).collect();
        let t = world.tile;
        let bytes = crate::export::par_map(&cells, |&(x, y)| Some(webp(&image::imageops::crop_imm(img, x * t, y * t, t, t).to_image())));
        for ((x, y), bytes) in cells.into_iter().zip(bytes) {
            out.push((format!("{z}/{}_{}.webp", (t0[0] >> shift) + x, (t0[1] >> shift) + y), bytes.unwrap_or_default()));
        }
        if z > world.chunk_z {
            if z > ROOF_Z {
                a = half(&a);
            }
            b = half(&b);
        }
    }
    let low = (world.chunk_z >= ROOF_Z).then(|| webp(&b));
    (out, low)
}

/// Уровни ниже `chunk_z` — из плиток уровня выше на диске, по четыре в одну. `pyr` —
/// пирамида, `roofed` — плитки с крышами тех уровней, что в пирамиде без них. Нет
/// плитки — прозрачная.
#[cfg(not(target_arch = "wasm32"))]
pub fn merge_down(pyr: &std::path::Path, roofed: &std::path::Path, world: &World) -> Result<(), String> {
    let t = world.tile;
    let load = |dir: &std::path::Path, z: u32, x: u32, y: u32| -> image::RgbaImage {
        std::fs::read(dir.join(z.to_string()).join(format!("{x}_{y}.webp"))).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8()).unwrap_or_else(|| image::RgbaImage::new(t, t))
    };
    let merge = |from: &std::path::Path, to: &std::path::Path, z: u32| -> Result<(), String> {
        let n = 1u32 << z;
        let dir = to.join(z.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let cells: Vec<(u32, u32)> = (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).collect();
        let done = crate::export::par_map(&cells, |&(x, y)| {
            let mut big = image::RgbaImage::new(2 * t, 2 * t);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                image::imageops::replace(&mut big, &load(from, z + 1, 2 * x + dx, 2 * y + dy), (dx * t) as i64, (dy * t) as i64);
            }
            std::fs::write(dir.join(format!("{x}_{y}.webp")), webp(&half(&big))).ok()
        });
        if done.iter().any(Option::is_none) { Err(format!("{}: не записалось", dir.display())) } else { Ok(()) }
    };
    for z in (0..world.chunk_z).rev() {
        if z >= ROOF_Z {
            merge(pyr, pyr, z)?;
            merge(roofed, roofed, z)?;
        } else {
            merge(if z + 1 >= ROOF_Z { roofed } else { pyr }, pyr, z)?;
        }
    }
    Ok(())
}

/// Точки сцены во флагах Wild Canvas у тайла — дымоходы, сливы, путь по рельсам — со
/// сдвигом куска в мир. Рамка крыши (`outline`) и концы гамака — доли тайла, их не трогать.
/// Только `get_mut`: `v["ключ"]` на запись вставляет пустое значение, и у каждого тайла
/// появлялись `chimneys: null`, `rail: {path: null}`, `scuppers: null`.
fn shift_live(v: &mut Value, offset: [f64; 2]) {
    let Some(live) = v.get_mut("flags").and_then(|f| f.get_mut(crate::export::LIVE_MODULE)).and_then(Value::as_object_mut) else {
        return;
    };
    let mv = |p: &mut Value| {
        if let Some(a) = p.as_array_mut().filter(|a| a.len() >= 2) {
            a[0] = json!(a[0].as_f64().unwrap_or(0.0) + offset[0]);
            a[1] = json!(a[1].as_f64().unwrap_or(0.0) + offset[1]);
        }
    };
    for key in ["chimneys", "scuppers"] {
        for p in live.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
            mv(p);
        }
    }
    for p in live.get_mut("rail").and_then(|r| r.get_mut("path")).and_then(Value::as_array_mut).into_iter().flatten() {
        mv(p);
    }
}

/// Данные сцен кусков в одну: сдвиг куска в мир и только своё — середина вещи в
/// `own` = `[x0, y0, x1, y1)` (у шва двух кусков стена, лампа, крыша — одна). Крыши — все
/// свои: у Картографа крыша — тайл-поверхность ([`is_roof`]).
#[derive(Default)]
pub struct Gather {
    walls: Vec<Value>,
    lights: Vec<Value>,
    sounds: Vec<Value>,
    tiles: Vec<Value>,
}

impl Gather {
    /// `level` — id этажа: берётся то, у чего он первым в `levels` (тайл виден и с этажей
    /// выше, но лежит на своём); у карты в один этаж `levels` нет — `None`.
    pub fn add(&mut self, scene: &Value, offset: [f64; 2], own: [f64; 4], level: Option<&str>) {
        let on_level = |v: &Value| level.is_none_or(|id| v["levels"].get(0).and_then(Value::as_str).is_none_or(|first| first == id));
        let mine = |x: f64, y: f64| x >= own[0] && x < own[2] && y >= own[1] && y < own[3];
        let shift = |v: &mut Value, kx: &str, ky: &str| {
            if let Some(x) = v[kx].as_f64() {
                v[kx] = json!(x + offset[0]);
            }
            if let Some(y) = v[ky].as_f64() {
                v[ky] = json!(y + offset[1]);
            }
        };
        for w in scene["walls"].as_array().into_iter().flatten().filter(|v| on_level(v)) {
            let Some(c) = w["c"].as_array().filter(|c| c.len() == 4) else { continue };
            let c: Vec<f64> = c.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
            let c = [c[0] + offset[0], c[1] + offset[1], c[2] + offset[0], c[3] + offset[1]];
            if !mine((c[0] + c[2]) / 2.0, (c[1] + c[3]) / 2.0) {
                continue;
            }
            let mut w = w.clone();
            w["c"] = json!(c);
            self.walls.push(w);
        }
        for (list, out) in [(&scene["lights"], &mut self.lights), (&scene["sounds"], &mut self.sounds), (&scene["tiles"], &mut self.tiles)] {
            for v in list.as_array().into_iter().flatten().filter(|v| on_level(v)) {
                let mut v = v.clone();
                shift(&mut v, "x", "y");
                shift_live(&mut v, offset);
                if mine(v["x"].as_f64().unwrap_or(f64::NAN), v["y"].as_f64().unwrap_or(f64::NAN)) {
                    out.push(v);
                }
            }
        }
    }

    /// Собранное — сценой для [`manifest`].
    pub fn scene(&self) -> Value {
        json!({"walls": self.walls, "lights": self.lights, "sounds": self.sounds, "tiles": self.tiles})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_is_a_whole_number_of_tiles_and_reaches_the_roof_level() {
        // King's Cross: 16 000 × 16 256 при 64 на клетку — плитка 8 клеток, кусок 128.
        let w = World::new(16000, 16256, 64);
        assert_eq!((w.tile, w.max_z, w.side, w.chunk_z, w.chunk_px(), w.chunk_cells()), (512, 5, 16384, 1, 8192, 128));
        assert!(w.max_z >= ROOF_Z, "крыши должны открываться");
        // «Иерихон_2»: 6000 × 4400 при 100 — плитка 2 клетки, мир одним куском.
        let j = World::new(6000, 4400, 100);
        assert_eq!((j.tile, j.max_z, j.side, j.chunk_z, j.chunk_cells()), (200, 5, 6400, 0, 64));
        // 1000×1000 при 100 — плитка 10 клеток, кусок 80 клеток (8000 точек), куски — z4.
        let big = World::new(100_000, 100_000, 100);
        assert_eq!((big.tile, big.max_z, big.side, big.chunk_z, big.chunk_cells()), (1000, 7, 128_000, 4, 80));
        assert!(big.sector * big.sim >= big.side);
        // 16×12 клеток: плитка в клетку, мир 32 клетки — z5 есть, лишнее — пустые плитки.
        let small = World::new(1600, 1200, 100);
        assert_eq!((small.tile, small.max_z, small.side, small.chunk_z), (100, ROOF_Z, 3200, 0));
    }

    #[test]
    fn walls_are_cut_by_sectors_and_tiles_move_to_their_corner() {
        let world = World { side: 1000, tile: 500, max_z: 1, sim: 2, sector: 500, cell_px: 100, chunk_z: 0 };
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
                {"x": 780, "y": 790, "width": 50, "height": 50, "texture": {"src": "k/table.webp", "tint": "#a0c0ff"}, "occlusion": {"modes": [1], "alpha": 0}}
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
        assert_eq!(m["tiles"][1]["tint"], "#a0c0ff", "цвет подкрашенного предмета");
        assert!(m["tiles"][0].get("tint").is_none());
    }

    #[test]
    fn roofs_are_baked_below_the_roof_level_only() {
        // Мир одним куском, 6 уровней по 4 точки: z0..z4 — с крышей, z5 — без.
        let world = World { side: 128, tile: 4, max_z: 5, sim: 1, sector: 128, cell_px: 4, chunk_z: 0 };
        let plain = image::RgbaImage::from_pixel(128, 128, image::Rgba([0, 0, 255, 255]));
        let roof = BakedRoof { image: image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])), center: [4.0, 4.0], size: [8.0, 8.0] };
        let (files, low) = chunk_tiles(plain, &[roof], &world, [0, 0]);
        assert!(low.is_none(), "кусок дошёл до z0 сам");
        assert_eq!(files.len(), (0..=5).map(|z| 1usize << (2 * z)).sum::<usize>());
        let px = |name: &str| {
            let b = &files.iter().find(|f| f.0 == format!("{name}.webp")).unwrap().1;
            image::load_from_memory(b).unwrap().to_rgba8().get_pixel(0, 0).0
        };
        assert!(px("5/0_0")[2] > 200 && px("5/0_0")[0] < 40, "z5 — без крыши: {:?}", px("5/0_0"));
        assert!(px("4/0_0")[0] > 200, "z4 — с крышей: {:?}", px("4/0_0"));
        assert!(px("0/0_0")[2] > 200, "z0 — весь мир, крыша мелкая");
    }

    #[test]
    fn low_levels_are_merged_from_chunks_with_roofs_from_their_own_chain() {
        // Плитка 8 точек (у меньшей WebP мешает цвет соседей в блоке 2×2), z0..z6, куски —
        // z5 (по 2×2 плитки z6): 32×32 куска. Крыша — в первом куске: на z5 её нет, на z4 и ниже есть.
        let world = World { side: 512, tile: 8, max_z: 6, sim: 1, sector: 512, cell_px: 8, chunk_z: 5 };
        let dir = std::env::temp_dir().join(format!("kartograf_bl_{}", std::process::id()));
        let (pyr, roofed) = (dir.join("pyr"), dir.join("roofed"));
        for cy in 0..32u32 {
            for cx in 0..32u32 {
                let plain = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 255, 255]));
                let roofs = if (cx, cy) == (0, 0) { vec![BakedRoof { image: image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255])), center: [8.0, 8.0], size: [16.0, 16.0] }] } else { Vec::new() };
                let (files, low) = chunk_tiles(plain, &roofs, &world, [cx * 2, cy * 2]);
                for (rel, b) in files {
                    let p = pyr.join(rel);
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(p, b).unwrap();
                }
                let p = roofed.join("5").join(format!("{cx}_{cy}.webp"));
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, low.expect("кусок выше уровня крыш отдаёт свою плитку с крышей")).unwrap();
            }
        }
        merge_down(&pyr, &roofed, &world).unwrap();
        let px = |z: u32, x: u32, y: u32| image::load_from_memory(&std::fs::read(pyr.join(z.to_string()).join(format!("{x}_{y}.webp"))).unwrap()).unwrap().to_rgba8().get_pixel(0, 0).0;
        assert!(px(5, 0, 0)[2] > 200 && px(5, 0, 0)[0] < 40, "z5 — без крыши");
        assert!(px(4, 0, 0)[0] > 100, "z4 — из кусков с крышей: {:?}", px(4, 0, 0));
        assert!(px(4, 1, 0)[2] > 200, "соседняя плитка z4 — без крыши");
        assert_eq!((0..=6).filter(|z| pyr.join(z.to_string()).join("0_0.webp").exists()).count(), 7, "все уровни на месте");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_roof_mask_follows_the_roof_picture_in_world_coordinates() {
        let world = World { side: 4096, tile: 512, max_z: 3, sim: 1, sector: 4096, cell_px: 64, chunk_z: 0 };
        let mut mask = RoofMask::new(&world);
        assert!(mask.bytes().is_none(), "крыш нет — маски нет");
        // Крыша 400×400 в куске со сдвигом 2048: в маске (половина мира) — 200×200 у середины.
        let mut pic = image::RgbaImage::new(400, 400);
        for (x, _, p) in pic.enumerate_pixels_mut() {
            *p = image::Rgba([90, 60, 40, if x < 200 { 255 } else { 0 }]);
        }
        mask.add(&BakedRoof { image: pic, center: [200.0, 200.0], size: [400.0, 400.0] }, [2048.0, 2048.0]);
        let img = image::load_from_memory(&mask.bytes().unwrap()).unwrap().to_rgba8();
        assert!(img.get_pixel(1030, 1100)[3] > 200, "левая половина крыши — в маске");
        assert!(img.get_pixel(1170, 1100)[3] < 30, "прозрачная половина — нет");
        assert!(img.get_pixel(500, 500)[3] < 30);
    }

    #[test]
    fn floors_get_their_own_things_and_manifests() {
        let scene = json!({
            "name": "Дом",
            "walls": [{"c": [0, 0, 100, 0], "levels": ["L0"]}, {"c": [0, 50, 100, 50], "levels": ["L1"]}],
            "lights": [{"x": 10, "y": 10, "levels": ["L1"], "config": {}}],
            "tiles": [{"x": 20, "y": 20, "width": 10, "height": 10, "levels": ["L0", "L1"], "texture": {"src": "k/t.webp"}}],
            "levels": [{"_id": "L0", "name": "Низ", "elevation": {"bottom": 0, "top": 9}, "background": {"src": "a.webp"}},
                       {"_id": "L1", "name": "Верх", "elevation": {"bottom": 10, "top": 19}, "visibility": {"levels": ["L0"]}, "background": {"src": "b.webp"}}],
            "initialLevel": "L1"
        });
        let own = [0.0, 0.0, f64::INFINITY, f64::INFINITY];
        let (mut low, mut up) = (Gather::default(), Gather::default());
        low.add(&scene, [0.0, 0.0], own, Some("L0"));
        up.add(&scene, [0.0, 0.0], own, Some("L1"));
        let (l, u) = (low.scene(), up.scene());
        assert_eq!((l["walls"].as_array().unwrap().len(), u["walls"].as_array().unwrap().len()), (1, 1));
        assert_eq!((l["lights"].as_array().unwrap().len(), u["lights"].as_array().unwrap().len()), (0, 1));
        assert_eq!((l["tiles"].as_array().unwrap().len(), u["tiles"].as_array().unwrap().len()), (1, 0), "тайл лежит на своём этаже, хоть виден и сверху");
        let world = World::new(1000, 1000, 100);
        let live = json!({"flags": {"burn-living-map": {"ambience": "town-1"}}, "regions": [{"name": "Вода"}]});
        let s = super::scene(&scene, &world, &["k/a.json".into(), "k/b.json".into()], &live);
        assert_eq!((s["flags"]["burn-living-map"]["ambience"].as_str(), s["regions"][0]["name"].as_str()), (Some("town-1"), Some("Вода")), "живое — в сцене");
        assert_eq!(s["flags"]["boundless"]["manifestUrl"], "k/a.json");
        assert_eq!(s["levels"][1]["flags"]["boundless"]["manifestUrl"], "k/b.json");
        assert_eq!(s["levels"][1]["visibility"]["levels"], json!(["L0"]), "сверху виден низ");
        assert!(s["levels"][0]["background"].get("src").is_none(), "картинки этажа нет — её даёт пирамида");
        assert_eq!(s["initialLevel"], "L1");
        let one = super::scene(&json!({"name": "X"}), &world, &["k/a.json".into()], &Value::Null);
        assert!(one.get("levels").is_none() && one["flags"]["boundless"]["manifestUrl"] == "k/a.json");
    }

    #[test]
    fn live_points_of_a_chunk_move_into_the_world_and_reach_the_manifest() {
        let scene = json!({"tiles": [
            {"x": 50, "y": 50, "width": 100, "height": 100, "occlusion": {"modes": [2]}, "texture": {"src": "k/r.webp"},
             "flags": {"burn-living-map": {"chimneys": [[60, 40, 30]], "outline": [{"points": [0.1, 0.2], "hole": false}]}}},
            {"x": 300, "y": 300, "width": 50, "height": 50, "occlusion": {"modes": [1]}, "texture": {"src": "k/tree.webp"},
             "flags": {"burn-living-map": {"wind": 1, "rail": {"path": [[10, 20], [30, 40]], "lag": 0}}}}
        ]});
        let mut g = Gather::default();
        g.add(&scene, [1000.0, 2000.0], [0.0, 0.0, f64::INFINITY, f64::INFINITY], None);
        let world = World { side: 4096, tile: 512, max_z: 3, sim: 1, sector: 4096, cell_px: 64, chunk_z: 0 };
        let m = manifest(&g.scene(), &world, "k/p");
        let ov = &m["overlays"]["0"];
        assert_eq!(ov["flags"]["burn-living-map"]["chimneys"], json!([[1060.0, 2040.0, 30]]), "труба — в мире");
        assert_eq!(ov["flags"]["burn-living-map"]["outline"][0]["points"], json!([0.1, 0.2]), "рамка — доли, не сдвигается");
        let tile = &m["tiles"][0];
        assert_eq!(tile["flags"]["burn-living-map"]["wind"], 1);
        assert_eq!(tile["flags"]["burn-living-map"]["rail"]["path"], json!([[1010.0, 2020.0], [1030.0, 2040.0]]));
        let keys = |v: &Value| v["flags"]["burn-living-map"].as_object().map(|o| o.keys().cloned().collect::<Vec<_>>());
        assert_eq!(keys(tile), Some(vec!["rail".to_owned(), "wind".to_owned()]), "пустых пометок не прибавилось");
        assert_eq!(keys(ov), Some(vec!["chimneys".to_owned(), "outline".to_owned()]));
    }
}
