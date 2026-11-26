//! Модель предмета в 3D — в «пространстве предмета»: единица — клетка, y — вверх, середина рамки
//! вида сверху — в нуле по x и z, низ — у нуля по y, перед — +z (вниз картинки на полке, как у
//! предметов FA и вида сверху WoW). Так предмет встаёт на своё место карты одним сдвигом в
//! середину картинки, поворотом и растяжением в её размер.
//!
//! Откуда: модель WoW (M2 с текстурами клиента) или файл glTF (`.glb`, `.gltf`) — свободные
//! наборы CC0 и свои же записанные. Куда: в окно 3D, в `.glb` рядом с картинкой предмета на полке
//! (по нему выгрузка даёт тайлу предмета модель 3D Canvas) и в вид сверху для полки.

use crate::scene3d::{Alpha, Look, Material, Mesh, Vertex};
use serde_json::Value;
use std::path::Path;

/// Клеток в метре: клетка — 5 футов (1,524 м), glTF пишут в метрах.
pub const PER_METER: f32 = 1.0 / 1.524;
/// Клеток в ярде WoW — как у вида сверху на полке.
pub const PER_YARD: f32 = 0.6;
/// Картинка модели в памяти — не больше стольких точек по стороне.
const MAX_IMAGE: u32 = 1024;

#[derive(Clone)]
pub struct Part3 {
    pub mesh: Mesh,
    /// `Look::Picture` — номер в [`Model3d::images`].
    pub material: Material,
}

#[derive(Clone, Default)]
pub struct Model3d {
    pub parts: Vec<Part3>,
    pub images: Vec<image::RgbaImage>,
}

impl Model3d {
    pub fn bounds(&self) -> Option<[[f32; 3]; 2]> {
        let mut it = self.parts.iter().flat_map(|p| p.mesh.v.iter().map(|v| v.p)).peekable();
        it.peek()?;
        Some(it.fold([[f32::MAX; 3], [f32::MIN; 3]], |[lo, hi], p| [[lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])]]))
    }

    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.idx.len() / 3).sum()
    }

    /// Середину рамки сверху — в ноль по x и z. Ушедшее под пол больше чем на четверть высоты
    /// (начало модели посередине) — поставить на пол; немного ниже нуля (ножки в полу) — как есть.
    pub fn normalize(&mut self) {
        let Some([lo, hi]) = self.bounds() else { return };
        let (cx, cz) = ((lo[0] + hi[0]) / 2.0, (lo[2] + hi[2]) / 2.0);
        let lift = if -lo[1] > 0.25 * (hi[1] - lo[1]) { -lo[1] } else { 0.0 };
        for v in self.parts.iter_mut().flat_map(|p| p.mesh.v.iter_mut()) {
            v.p = [v.p[0] - cx, v.p[1] + lift, v.p[2] - cz];
        }
    }

    /// `.glb` модели: один узел, куски по материалам, картинки PNG.
    pub fn write_glb(&self, name: &str) -> Result<Vec<u8>, String> {
        use crate::scene3d::{Node, Part, Picture, Scene3d};
        let prims = self.parts.iter().enumerate().filter(|(_, p)| !p.mesh.idx.is_empty()).map(|(i, p)| (i, p.mesh.clone())).collect();
        let scene = Scene3d {
            nodes: vec![Node { name: name.to_owned(), part: Part::Wall, storey: 0, at: [0.0; 3], yaw: 0.0, prims, door: None }],
            materials: self.parts.iter().map(|p| p.material.clone()).collect(),
            pictures: self.images.iter().enumerate().map(|(i, img)| Picture { key: format!("{name}#{i}"), size: [img.width(), img.height()], alpha: img.pixels().any(|p| p[3] < 250) }).collect(),
        };
        let mut images = Vec::new();
        for img in &self.images {
            let mut out = Vec::new();
            image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8).map_err(|e| format!("картинка модели: {e}"))?;
            images.push((out, "image/png"));
        }
        crate::glb::write(&scene, &images, name)
    }
}

use image::ImageEncoder;

/// sRGB-байт из линейного (glTF хранит цвет линейным).
fn srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

type M4 = [f32; 16];
const ID4: M4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

fn mul4(a: &M4, b: &M4) -> M4 {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

/// Матрица узла: `matrix` или сдвиг · поворот · масштаб.
fn local(node: &Value) -> M4 {
    let f = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
    if let Some(m) = node["matrix"].as_array().filter(|m| m.len() == 16) {
        let mut out = [0.0; 16];
        for (o, v) in out.iter_mut().zip(m) {
            *o = f(v);
        }
        return out;
    }
    let get = |k: &str, d: &[f32]| node[k].as_array().map_or_else(|| d.to_vec(), |a| a.iter().map(f).collect());
    let (t, r, s) = (get("translation", &[0.0; 3]), get("rotation", &[0.0, 0.0, 0.0, 1.0]), get("scale", &[1.0; 3]));
    let (x, y, z, w) = (r[0], r[1], r[2], r[3]);
    let rot = [
        1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w), 0.0,
        2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w), 0.0,
        2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y), 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    let mut m = rot;
    for c in 0..3 {
        for r in 0..3 {
            m[c * 4 + r] *= s[c];
        }
    }
    m[12] = t[0];
    m[13] = t[1];
    m[14] = t[2];
    m
}

fn apply(m: &M4, p: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

/// Нормаль через присоединённую матрицу верхнего угла 3×3 (верна и при неравном масштабе) и
/// знак определителя: отрицательный — узел отражён, обход треугольников меняется.
fn normal_matrix(m: &M4) -> ([[f32; 3]; 3], f32) {
    let a = |c: usize, r: usize| m[c * 4 + r];
    let col = |c: usize| [a(c, 0), a(c, 1), a(c, 2)];
    let cross = |u: [f32; 3], v: [f32; 3]| [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let (c0, c1, c2) = (col(0), col(1), col(2));
    let det = c0[0] * (c1[1] * c2[2] - c1[2] * c2[1]) - c1[0] * (c0[1] * c2[2] - c0[2] * c2[1]) + c2[0] * (c0[1] * c1[2] - c0[2] * c1[1]);
    ([cross(c1, c2), cross(c2, c0), cross(c0, c1)], det)
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-12 { [0.0, 1.0, 0.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

struct Doc {
    j: Value,
    bufs: Vec<Vec<u8>>,
}

fn data_uri(u: &str) -> Option<Result<Vec<u8>, String>> {
    let rest = u.strip_prefix("data:")?;
    let (_, b64) = rest.split_once(";base64,")?;
    use base64::Engine;
    Some(base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| format!("glTF: base64: {e}")))
}

fn load_uri(u: &str, dir: Option<&Path>) -> Result<Vec<u8>, String> {
    if let Some(d) = data_uri(u) {
        return d;
    }
    let dir = dir.ok_or("glTF: внешний файл без папки")?;
    let name = u.replace("%20", " ");
    let path = dir.join(&name);
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

impl Doc {
    fn view(&self, i: usize) -> Result<(&[u8], usize), String> {
        let v = &self.j["bufferViews"][i];
        let b = self.bufs.get(v["buffer"].as_u64().unwrap_or(0) as usize).ok_or("glTF: нет буфера")?;
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
        let data = b.get(off..off + len).ok_or("glTF: вид за краем буфера")?;
        Ok((data, v["byteStride"].as_u64().unwrap_or(0) as usize))
    }

    /// Значения доступа подряд (целые с `normalized` — в 0..1) и сколько их на элемент.
    fn floats(&self, i: usize) -> Result<(Vec<f32>, usize), String> {
        let a = &self.j["accessors"][i];
        let comps = match a["type"].as_str().unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            t => return Err(format!("glTF: вид доступа {t}")),
        };
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let ct = a["componentType"].as_u64().unwrap_or(5126);
        let size = match ct {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return Err(format!("glTF: тип {ct}")),
        };
        let Some(vi) = a["bufferView"].as_u64() else { return Ok((vec![0.0; count * comps], comps)) };
        let (data, stride) = self.view(vi as usize)?;
        let stride = if stride == 0 { size * comps } else { stride };
        let off = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let norm = a["normalized"].as_bool().unwrap_or(false);
        let mut out = Vec::with_capacity(count * comps);
        for e in 0..count {
            for c in 0..comps {
                let at = off + e * stride + c * size;
                let b = data.get(at..at + size).ok_or("glTF: доступ за краем")?;
                let v = match ct {
                    5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    5121 => b[0] as f32 / if norm { 255.0 } else { 1.0 },
                    5120 => (b[0] as i8) as f32 / if norm { 127.0 } else { 1.0 },
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f32 / if norm { 65535.0 } else { 1.0 },
                    5122 => i16::from_le_bytes([b[0], b[1]]) as f32 / if norm { 32767.0 } else { 1.0 },
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                };
                out.push(v);
            }
        }
        Ok((out, comps))
    }

    fn indices(&self, i: usize) -> Result<Vec<u32>, String> {
        let a = &self.j["accessors"][i];
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let ct = a["componentType"].as_u64().unwrap_or(5125);
        let size = match ct {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return Err(format!("glTF: номера типа {ct}")),
        };
        let vi = a["bufferView"].as_u64().ok_or("glTF: номера без вида")? as usize;
        let (data, stride) = self.view(vi)?;
        let stride = if stride == 0 { size } else { stride };
        let off = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        (0..count)
            .map(|e| {
                let at = off + e * stride;
                let b = data.get(at..at + size).ok_or("glTF: номер за краем")?;
                Ok(match size {
                    1 => b[0] as u32,
                    2 => u16::from_le_bytes([b[0], b[1]]) as u32,
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                })
            })
            .collect()
    }

    fn image(&self, i: usize, dir: Option<&Path>) -> Result<image::RgbaImage, String> {
        let im = &self.j["images"][i];
        let bytes = match (im["bufferView"].as_u64(), im["uri"].as_str()) {
            (Some(v), _) => self.view(v as usize)?.0.to_vec(),
            (None, Some(u)) => load_uri(u, dir)?,
            _ => return Err("glTF: картинка без данных".into()),
        };
        let img = image::load_from_memory(&bytes).map_err(|e| format!("glTF: картинка {i}: {e}"))?.to_rgba8();
        let side = img.width().max(img.height());
        Ok(if side > MAX_IMAGE {
            let k = MAX_IMAGE as f32 / side as f32;
            image::imageops::resize(&img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle)
        } else {
            img
        })
    }
}

/// Прочитать glTF: `.glb` целиком или текст `.gltf` (внешние буферы и картинки — из `dir`).
/// `scale` — клеток в единице файла: у наборов — [`PER_METER`], у своих записанных — 1.
/// Модель не сдвигается: начало файла остаётся началом ([`Model3d::normalize`] — отдельно).
pub fn read_gltf(bytes: &[u8], dir: Option<&Path>, scale: f32) -> Result<Model3d, String> {
    let (json, bin) = if bytes.len() >= 12 && &bytes[0..4] == b"glTF" {
        let mut at = 12;
        let (mut json, mut bin) = (None, None);
        while at + 8 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let kind = &bytes[at + 4..at + 8];
            let body = bytes.get(at + 8..at + 8 + len).ok_or("glb: кусок за краем")?;
            match kind {
                b"JSON" => json = Some(body),
                b"BIN\0" => bin = Some(body.to_vec()),
                _ => {}
            }
            at += 8 + len;
        }
        (json.ok_or("glb: нет JSON")?, bin)
    } else {
        (bytes, None)
    };
    let j: Value = serde_json::from_slice(json).map_err(|e| format!("glTF: {e}"))?;
    let mut bufs = Vec::new();
    for (i, b) in j["buffers"].as_array().into_iter().flatten().enumerate() {
        bufs.push(match b["uri"].as_str() {
            Some(u) => load_uri(u, dir)?,
            None if i == 0 => bin.clone().unwrap_or_default(),
            None => Vec::new(),
        });
    }
    let doc = Doc { j, bufs };
    let j = &doc.j;

    // Узлы сцены с их матрицами (обход в глубину; петли и безумная глубина — отсекаются).
    let scene = j["scene"].as_u64().unwrap_or(0) as usize;
    let roots: Vec<usize> = match j["scenes"][scene]["nodes"].as_array() {
        Some(r) => r.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect(),
        None => (0..j["nodes"].as_array().map_or(0, |n| n.len())).collect(),
    };
    let mut placed: Vec<(usize, M4)> = Vec::new();
    let mut stack: Vec<(usize, M4, usize)> = roots.into_iter().map(|n| (n, ID4, 0)).collect();
    while let Some((n, parent, depth)) = stack.pop() {
        if depth > 64 {
            continue;
        }
        let node = &j["nodes"][n];
        let m = mul4(&parent, &local(node));
        if node["mesh"].as_u64().is_some() {
            placed.push((n, m));
        }
        for c in node["children"].as_array().into_iter().flatten().filter_map(|v| v.as_u64()) {
            stack.push((c as usize, m, depth + 1));
        }
    }

    let mut model = Model3d::default();
    // Картинка (номер glTF, оттенок) → номер картинки модели.
    let mut pictures: std::collections::HashMap<(usize, [u8; 3]), usize> = std::collections::HashMap::new();
    for (n, m) in placed {
        let mesh = &j["meshes"][j["nodes"][n]["mesh"].as_u64().unwrap_or(0) as usize];
        let (nm, det) = normal_matrix(&m);
        for prim in mesh["primitives"].as_array().into_iter().flatten() {
            if prim["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let attrs = &prim["attributes"];
            let Some(pi) = attrs["POSITION"].as_u64() else { continue };
            let (pos, _) = doc.floats(pi as usize)?;
            let count = pos.len() / 3;
            let normals = match attrs["NORMAL"].as_u64() {
                Some(i) => Some(doc.floats(i as usize)?.0),
                None => None,
            };
            let mat_j = prim["material"].as_u64().map(|i| &j["materials"][i as usize]);
            let pbr = mat_j.map(|mj| &mj["pbrMetallicRoughness"]);
            let tex_info = pbr.map(|p| &p["baseColorTexture"]).filter(|t| t["index"].as_u64().is_some());
            let set = tex_info.and_then(|t| t["texCoord"].as_u64()).unwrap_or(0);
            let uvs = match attrs[format!("TEXCOORD_{set}")].as_u64() {
                Some(i) => Some(doc.floats(i as usize)?.0),
                None => None,
            };
            let factor: Vec<f32> = pbr.and_then(|p| p["baseColorFactor"].as_array()).map_or_else(|| vec![1.0; 4], |a| a.iter().map(|v| v.as_f64().unwrap_or(1.0) as f32).collect());
            let factor = [0, 1, 2, 3].map(|k| factor.get(k).copied().unwrap_or(1.0));
            let mut idx = match prim["indices"].as_u64() {
                Some(i) => doc.indices(i as usize)?,
                None => (0..count as u32).collect(),
            };
            idx.truncate(idx.len() / 3 * 3);
            if idx.iter().any(|&i| i as usize >= count) {
                return Err("glTF: номер вершины за краем".into());
            }
            if det < 0.0 {
                for t in idx.chunks_mut(3) {
                    t.swap(1, 2);
                }
            }
            let mut v: Vec<Vertex> = (0..count)
                .map(|k| {
                    let p = apply(&m, [pos[k * 3], pos[k * 3 + 1], pos[k * 3 + 2]]).map(|c| c * scale);
                    let n = normals.as_ref().map_or([0.0, 1.0, 0.0], |ns| {
                        let n0 = [ns[k * 3], ns[k * 3 + 1], ns[k * 3 + 2]];
                        unit([0, 1, 2].map(|r| nm[0][r] * n0[0] + nm[1][r] * n0[1] + nm[2][r] * n0[2]))
                    });
                    let uv = uvs.as_ref().map_or([0.0, 0.0], |u| [u[k * 2], u[k * 2 + 1]]);
                    Vertex { p, n, uv }
                })
                .collect();
            // Без нормалей — по граням.
            if normals.is_none() {
                let mut acc = vec![[0.0f32; 3]; count];
                for t in idx.chunks(3) {
                    let (a, b, c) = (v[t[0] as usize].p, v[t[1] as usize].p, v[t[2] as usize].p);
                    let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                    let g = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                    for &i in t {
                        for q in 0..3 {
                            acc[i as usize][q] += g[q];
                        }
                    }
                }
                for (vv, n) in v.iter_mut().zip(acc) {
                    vv.n = unit(n);
                }
            }
            // Цвет: картинка (с оттенком множителя), иначе множитель с цветом вершин.
            let tint = [srgb(factor[0]), srgb(factor[1]), srgb(factor[2])];
            let source = tex_info.and_then(|t| t["index"].as_u64()).map(|ti| &j["textures"][ti as usize]).and_then(|t| t["source"].as_u64().or_else(|| t["extensions"]["EXT_texture_webp"]["source"].as_u64()));
            let look = match source {
                Some(si) => {
                    let key = (si as usize, tint);
                    match pictures.get(&key) {
                        Some(&i) => Some(Look::Picture(i)),
                        None => match doc.image(si as usize, dir) {
                            Ok(mut img) => {
                                if tint != [255; 3] {
                                    for p in img.pixels_mut() {
                                        for c in 0..3 {
                                            p[c] = (p[c] as u32 * tint[c] as u32 / 255) as u8;
                                        }
                                    }
                                }
                                model.images.push(img);
                                pictures.insert(key, model.images.len() - 1);
                                Some(Look::Picture(model.images.len() - 1))
                            }
                            Err(_) => None,
                        },
                    }
                }
                None => None,
            };
            let look = look.unwrap_or_else(|| {
                let vc = attrs["COLOR_0"].as_u64().and_then(|i| doc.floats(i as usize).ok()).filter(|(c, n)| *n >= 3 && !c.is_empty());
                let avg = vc.map_or([1.0; 3], |(c, n)| {
                    let k = (c.len() / n) as f32;
                    [0, 1, 2].map(|q| c.iter().skip(q).step_by(n).sum::<f32>() / k)
                });
                Look::Color([0, 1, 2].map(|q| srgb(factor[q] * avg[q])))
            });
            let alpha = match mat_j.and_then(|mj| mj["alphaMode"].as_str()) {
                Some("MASK") => Alpha::Mask,
                Some("BLEND") => Alpha::Blend,
                _ => Alpha::Opaque,
            };
            let two_sided = mat_j.is_some_and(|mj| mj["doubleSided"].as_bool().unwrap_or(false));
            let opacity = if alpha == Alpha::Blend { factor[3] } else { 1.0 };
            model.parts.push(Part3 { mesh: Mesh { v: std::mem::take(&mut v), idx }, material: Material { look, alpha, two_sided, repeat: true, opacity } });
        }
    }
    if model.parts.is_empty() {
        return Err("glTF: в сцене нет сеток".into());
    }
    Ok(model)
}

/// Прочитать файл модели: `.glb` или `.gltf` рядом со своими буферами.
pub fn read_file(path: &Path, scale: f32) -> Result<Model3d, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    read_gltf(&bytes, path.parent(), scale)
}

#[cfg(not(target_arch = "wasm32"))]
mod wow_side {
    use super::*;
    use crate::wow::{blp, m2, render};

    /// Модель WoW — в пространство предмета: ось X WoW (перед) — вниз картинки (+z), Y (влево) —
    /// вправо (+x), Z — вверх; ярды — в клетки. Берётся то же, что рисует вид сверху
    /// ([`render::render`]): сплошное, с вырезом и полупрозрачное, без свечений; середина — та же.
    pub fn from_m2(model: &m2::Model, textures: &[Option<blp::Rgba>]) -> Result<Model3d, String> {
        let chosen: Vec<&m2::Batch> = model.batches.iter().filter(|b| model.materials.get(b.material).map_or(0, |m| m.blend) <= 2 && b.alpha >= 0.05).collect();
        let at = |i: u32| model.vertices.get(i as usize).ok_or("M2: вершина за краем");
        // Обход треугольников против нормалей вершин (такие модели в WoW есть) — развернуть.
        let mut votes = 0i64;
        for b in &chosen {
            for t in &b.triangles {
                let (a, b2, c) = (at(t[0])?, at(t[1])?, at(t[2])?);
                let (e1, e2) = ([b2.pos[0] - a.pos[0], b2.pos[1] - a.pos[1], b2.pos[2] - a.pos[2]], [c.pos[0] - a.pos[0], c.pos[1] - a.pos[1], c.pos[2] - a.pos[2]]);
                let g = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                let n = [0, 1, 2].map(|k| a.normal[k] + b2.normal[k] + c.normal[k]);
                let d = g[0] * n[0] + g[1] * n[1] + g[2] * n[2];
                votes += d.signum() as i64;
            }
        }
        let flip = votes < 0;
        let mut out = Model3d::default();
        let mut pics: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        for b in chosen {
            let mat = model.materials.get(b.material);
            let blend = mat.map_or(0, |m| m.blend);
            let tex = b.texture.and_then(|ti| textures.get(ti).and_then(|t| t.as_ref()).map(|t| (ti, t)));
            let look = match tex {
                Some((ti, t)) => {
                    let i = *pics.entry(ti).or_insert_with(|| {
                        let raw: Vec<u8> = t.px.iter().flatten().copied().collect();
                        out.images.push(image::RgbaImage::from_raw(t.w, t.h, raw).unwrap_or_else(|| image::RgbaImage::new(1, 1)));
                        out.images.len() - 1
                    });
                    Look::Picture(i)
                }
                None => Look::Color([150, 150, 150]),
            };
            let alpha = match blend {
                0 => Alpha::Opaque,
                1 => Alpha::Mask,
                _ => Alpha::Blend,
            };
            let material = Material { look, alpha, two_sided: mat.is_some_and(|m| m.flags & 0x4 != 0), repeat: true, opacity: if blend == 2 { b.alpha.clamp(0.0, 1.0) } else { 1.0 } };
            let mut mesh = Mesh::default();
            let mut remap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            for t in &b.triangles {
                let t = if flip { [t[0], t[2], t[1]] } else { *t };
                for i in t {
                    let k = match remap.get(&i) {
                        Some(&k) => k,
                        None => {
                            let v = at(i)?;
                            let p = [v.pos[1], v.pos[2], v.pos[0]].map(|c| c * PER_YARD);
                            let n = [v.normal[1], v.normal[2], v.normal[0]];
                            mesh.v.push(Vertex { p, n: if flip { n.map(|c| -c) } else { n }, uv: v.uv });
                            remap.insert(i, mesh.v.len() as u32 - 1);
                            mesh.v.len() as u32 - 1
                        }
                    };
                    mesh.idx.push(k);
                }
            }
            if !mesh.idx.is_empty() {
                out.parts.push(Part3 { mesh, material });
            }
        }
        if out.parts.is_empty() {
            return Err("нечего показать: у модели нет сплошных частей".into());
        }
        out.normalize();
        Ok(out)
    }

    /// Вид сверху для полки — тем же рисованием, что у предметов WoW: картинка и размер в клетках.
    pub fn top_view(model: &Model3d, ppc: f32) -> Result<(image::RgbaImage, [f32; 2]), String> {
        let mut m = m2::Model { vertices: Vec::new(), textures: Vec::new(), materials: Vec::new(), batches: Vec::new() };
        let mut textures = Vec::new();
        for img in &model.images {
            textures.push(Some(blp::Rgba { w: img.width(), h: img.height(), px: img.pixels().map(|p| p.0).collect() }));
            m.textures.push(m2::Texture { kind: 0, flags: 3, file: String::new() });
        }
        for part in &model.parts {
            let tex = match part.material.look {
                Look::Picture(i) => Some(i),
                Look::Color(c) => {
                    textures.push(Some(blp::Rgba { w: 1, h: 1, px: vec![[c[0], c[1], c[2], 255]] }));
                    m.textures.push(m2::Texture { kind: 0, flags: 3, file: String::new() });
                    Some(textures.len() - 1)
                }
            };
            let blend = match part.material.alpha {
                Alpha::Opaque => 0,
                Alpha::Mask => 1,
                Alpha::Blend => 2,
            };
            m.materials.push(m2::Material { flags: if part.material.two_sided { 0x4 } else { 0 }, blend });
            let base = m.vertices.len() as u32;
            for v in &part.mesh.v {
                // Обратно в оси WoW: X — вперёд (+z), Y — +x, Z — вверх; клетки — в ярды.
                m.vertices.push(m2::Vertex { pos: [v.p[2], v.p[0], v.p[1]].map(|c| c / PER_YARD), normal: [v.n[2], v.n[0], v.n[1]], uv: v.uv });
            }
            let triangles = part.mesh.idx.chunks(3).filter(|t| t.len() == 3).map(|t| [t[0] + base, t[1] + base, t[2] + base]).collect();
            m.batches.push(m2::Batch { triangles, texture: tex, material: m.materials.len() - 1, alpha: part.material.opacity, flow: None });
        }
        let opt = render::Options { ppc, cells_per_yard: PER_YARD, ..Default::default() };
        let s = render::render(&m, &textures, &opt)?;
        Ok((s.img, s.cells))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use wow_side::{from_m2, top_view};

#[cfg(test)]
mod tests {
    use super::*;

    /// Ящик 2×1×0,5 клетки с картинкой 2×2 на верху и цветом на боку.
    fn crate_box() -> Model3d {
        let mut top = Mesh::default();
        let v = |p: [f32; 3], uv: [f32; 2]| Vertex { p, n: [0.0, 1.0, 0.0], uv };
        top.v = vec![v([-1.0, 0.5, -0.5], [0.0, 0.0]), v([1.0, 0.5, -0.5], [1.0, 0.0]), v([1.0, 0.5, 0.5], [1.0, 1.0]), v([-1.0, 0.5, 0.5], [0.0, 1.0])];
        top.idx = vec![0, 2, 1, 0, 3, 2];
        let mut side = Mesh::default();
        let s = |p: [f32; 3]| Vertex { p, n: [0.0, 0.0, 1.0], uv: [0.0, 0.0] };
        side.v = vec![s([-1.0, 0.0, 0.5]), s([1.0, 0.0, 0.5]), s([1.0, 0.5, 0.5]), s([-1.0, 0.5, 0.5])];
        side.idx = vec![0, 1, 2, 0, 2, 3];
        let img = image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([(x * 200) as u8, (y * 200) as u8, 50, 255]));
        Model3d {
            parts: vec![
                Part3 { mesh: top, material: Material { look: Look::Picture(0), alpha: Alpha::Opaque, two_sided: false, repeat: true, opacity: 1.0 } },
                Part3 { mesh: side, material: Material { look: Look::Color([200, 40, 40]), alpha: Alpha::Opaque, two_sided: true, repeat: true, opacity: 1.0 } },
            ],
            images: vec![img],
        }
    }

    #[test]
    fn a_written_model_reads_back_the_same() {
        let m = crate_box();
        let bytes = m.write_glb("ящик").unwrap();
        let back = read_gltf(&bytes, None, 1.0).unwrap();
        assert_eq!(back.parts.len(), 2);
        assert_eq!(back.triangles(), 4);
        assert_eq!(back.bounds(), m.bounds());
        assert_eq!(back.images.len(), 1);
        assert_eq!(back.images[0].get_pixel(1, 1), m.images[0].get_pixel(1, 1));
        assert_eq!(back.parts[1].material.look, Look::Color([200, 40, 40]), "цвет — через линейный и обратно");
        assert!(back.parts[1].material.two_sided);
    }

    #[test]
    fn metres_become_cells_and_a_centred_model_stands_on_the_floor() {
        let mut m = crate_box();
        for v in m.parts.iter_mut().flat_map(|p| p.mesh.v.iter_mut()) {
            v.p = [v.p[0] + 3.0, v.p[1] - 0.25, v.p[2] + 1.0];
        }
        let bytes = m.write_glb("x").unwrap();
        let mut back = read_gltf(&bytes, None, PER_METER).unwrap();
        let [lo, hi] = back.bounds().unwrap();
        assert!((hi[0] - lo[0] - 2.0 * PER_METER).abs() < 1e-4);
        back.normalize();
        let [lo, hi] = back.bounds().unwrap();
        assert!(lo[0].abs() - hi[0].abs() < 1e-5 && (lo[2] + hi[2]).abs() < 1e-5, "середина в нуле");
        assert!(lo[1].abs() < 1e-5, "половина под полом — поставлен на пол: {lo:?}");
    }

    #[test]
    fn node_transforms_and_mirroring_are_applied() {
        // Узел-родитель сдвигает на (10, 0, 0), ребёнок отражён по x: обход треугольника меняется.
        let m = crate_box();
        let bytes = m.write_glb("x").unwrap();
        let (json_len, _) = (u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize, 0);
        let mut j: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
        j["nodes"][0]["scale"] = serde_json::json!([-1.0, 1.0, 1.0]);
        let n = j["nodes"].as_array().unwrap().len();
        j["nodes"].as_array_mut().unwrap().push(serde_json::json!({"translation": [10.0, 0.0, 0.0], "children": [0]}));
        j["scenes"][0]["nodes"] = serde_json::json!([n]);
        let mut text = serde_json::to_vec(&j).unwrap();
        while text.len() % 4 != 0 {
            text.push(b' ');
        }
        let rest = &bytes[20 + json_len..];
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + text.len() + rest.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&text);
        out.extend_from_slice(rest);
        let back = read_gltf(&out, None, 1.0).unwrap();
        let [lo, hi] = back.bounds().unwrap();
        assert!((lo[0] - 9.0).abs() < 1e-4 && (hi[0] - 11.0).abs() < 1e-4, "{lo:?} {hi:?}");
        // Верх смотрит вверх и после отражения: обход развёрнут вместе с ним.
        let top = &back.parts[0].mesh;
        let t = &top.idx[0..3];
        let (a, b, c) = (top.v[t[0] as usize].p, top.v[t[1] as usize].p, top.v[t[2] as usize].p);
        let g = (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]);
        assert!(g > 0.0, "грань смотрит вверх");
    }

    #[test]
    fn the_top_view_is_as_wide_as_the_model() {
        let m = crate_box();
        let (img, cells) = top_view(&m, 50.0).unwrap();
        // С полями по две точки с края: у 50 точек на клетку — плюс 0,1.
        assert!((cells[0] - 2.0).abs() < 0.15 && (cells[1] - 1.0).abs() < 0.15, "{cells:?}");
        // Верх — картинка: левый верхний угол её тёмный (x = 0, y = 0).
        let px = img.get_pixel(img.width() / 4, img.height() / 4);
        assert!(px[3] == 255);
    }

    #[test]
    fn a_wow_model_lands_front_down_the_picture_and_centred() {
        let (bytes, skin) = crate::wow::m2::tests::tiny_model();
        let model = crate::wow::m2::parse(&bytes, Some(&skin)).unwrap();
        let tex = crate::wow::blp::Rgba { w: 1, h: 1, px: vec![[255, 255, 255, 255]] };
        let m = from_m2(&model, &[Some(tex)]).unwrap();
        let [lo, hi] = m.bounds().unwrap();
        // Треугольник (0,0)-(1,0)-(0,1) ярда: по 0,6 клетки вдоль x и z, середина в нуле.
        assert!((hi[0] - lo[0] - 0.6).abs() < 1e-4 && (hi[2] - lo[2] - 0.6).abs() < 1e-4);
        assert!((lo[0] + hi[0]).abs() < 1e-5 && (lo[2] + hi[2]).abs() < 1e-5);
        // Вершина (1, 0) WoW — вперёд, вниз картинки: у неё z наибольший.
        let far = m.parts[0].mesh.v.iter().max_by(|a, b| a.p[2].total_cmp(&b.p[2])).unwrap();
        assert!((far.p[0] - lo[0]).abs() < 1e-5, "передняя вершина — слева, как на виде сверху");
    }
}
