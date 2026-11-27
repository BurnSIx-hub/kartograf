//! Створки дверей 3D-карты — модели Blender (`tools/blender/doors.py` → `assets/doors3d`):
//! деревянная (доски, поперечины и раскос сзади, железные полосы петель, кольцо), металлическая
//! (лист в раме с заклёпками, ручка-рычаг) и каменная (плита с резной рамой, кольцо).
//!
//! Модель — створка в клетках: петли на x = 0, другой край на x = 1, низ на y = 0, верх на
//! y = 1,4 (дверь 7 футов), толщина по z. Тело — серая запечённая фактура, её красит цвет породы
//! или камня из имени рисунка двери (`Door_Wood_Walnut…`, `Door_Stone_Sandstone…`); железо — своим
//! цветом. Створка растягивается по проёму ([`leaf_part`]).
//!
//! В сцене створка — один кусок с одной картинкой: фактура тела, под ней полоса цвета железа.
//! 3D Canvas читает метки узла (нетвёрдая, не загораживает обзор) только у узла из одного куска —
//! из двух открытая створка стала бы для фишки стеной.

use crate::model3d::{self, Model3d};
use crate::scene3d::{Look, Material, Mesh};
use std::sync::OnceLock;

/// Ширина и высота створки в модели (клетки).
const W: f32 = 1.0;
const H: f32 = 1.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Wood,
    Metal,
    Stone,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Wood => "wood",
            Kind::Metal => "metal",
            Kind::Stone => "stone",
        }
    }

    fn of(name: &str) -> Option<Kind> {
        [Kind::Wood, Kind::Metal, Kind::Stone].into_iter().find(|k| k.name() == name)
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Kind::Wood => include_bytes!("../assets/doors3d/door_wood.glb"),
            Kind::Metal => include_bytes!("../assets/doors3d/door_metal.glb"),
            Kind::Stone => include_bytes!("../assets/doors3d/door_stone.glb"),
        }
    }
}

/// Вид створки: модель и цвет её тела (светлый конец фактуры).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DoorLook {
    pub kind: Kind,
    pub tint: [u8; 3],
}

/// Вид створки по ключу рисунка двери: дерево, металл, камень и их цвет по имени (FA:
/// `Door_<материал>_<порода или камень>_…`, встроенные `builtin:door_wood`, `builtin:door_metal`).
pub fn look_of(art: Option<&str>) -> DoorLook {
    let low = art.unwrap_or_default().replace('\\', "/").to_lowercase();
    let name = low.rsplit(['/', '|', ':']).next().unwrap_or(&low);
    let has = |w: &str| name.contains(w);
    let kind = if has("stone") {
        Kind::Stone
    } else if has("metal") && !has("wood") || has("portcullis") {
        Kind::Metal
    } else {
        Kind::Wood
    };
    let tint = match kind {
        Kind::Wood => [("ashen", [172, 162, 148]), ("light", [204, 164, 114]), ("walnut", [132, 90, 58]), ("red", [166, 88, 62]), ("dark", [116, 80, 54])].into_iter().find(|(w, _)| has(w)).map_or([156, 112, 72], |(_, c)| c),
        Kind::Metal => [("rust", [156, 104, 72]), ("black", [84, 86, 92]), ("brass", [192, 152, 84]), ("polish", [206, 206, 212])].into_iter().find(|(w, _)| has(w)).map_or([164, 168, 174], |(_, c)| c),
        Kind::Stone => [("redrock", [180, 108, 86]), ("sandstone", [216, 188, 142]), ("slate", [124, 130, 138]), ("volcanic", [92, 86, 88]), ("earthy", [164, 144, 118])].into_iter().find(|(w, _)| has(w)).map_or([156, 152, 146], |(_, c)| c),
    };
    DoorLook { kind, tint }
}

/// Модель створки (один раз на вид).
fn model(kind: Kind) -> Option<&'static Model3d> {
    static MODELS: [OnceLock<Option<Model3d>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    MODELS[kind as usize].get_or_init(|| model3d::read_gltf(kind.bytes(), None, 1.0).ok()).as_ref()
}

/// Полоса цветов под фактурой тела в картинке створки (точки).
const STRIP: u32 = 16;

/// Створка сцены: сетка в осях створки, картинка (ключ для сцены и размер) и материал.
pub struct LeafPart {
    pub mesh: Mesh,
    pub picture: (String, [u32; 2]),
    pub material: Material,
}

/// Тело модели (её картинка) и цвета прочих частей — по порядку частей.
fn body_and_colors(m: &Model3d) -> Option<(usize, Vec<[u8; 3]>)> {
    let body = m.parts.iter().find_map(|p| match p.material.look {
        Look::Picture(i) => Some(i),
        Look::Color(_) => None,
    })?;
    let mut colors = Vec::new();
    for p in &m.parts {
        if let Look::Color(c) = p.material.look
            && !colors.contains(&c)
        {
            colors.push(c);
        }
    }
    Some((body, colors))
}

/// Картинка для ключа из [`leaf_part`]: фактура тела модели, окрашенная в цвет, и под ней полоса
/// цветов железа (не окрашенных), по столбцу на цвет.
pub fn picture(key: &str) -> Option<image::RgbaImage> {
    let mut it = key.strip_prefix("door3d:")?.split(':');
    let (kind, tint) = (Kind::of(it.next()?)?, it.next()?);
    let c = u32::from_str_radix(tint, 16).ok()?;
    let tint = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    let m = model(kind)?;
    let (body, colors) = body_and_colors(m)?;
    let src = m.images.get(body)?;
    let (w, h) = src.dimensions();
    let mut img = image::RgbaImage::new(w, h + STRIP);
    for (x, y, p) in src.enumerate_pixels() {
        let c = [0, 1, 2].map(|k| (p[k] as u32 * tint[k] as u32 / 255) as u8);
        img.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
    }
    let n = colors.len().max(1) as u32;
    for y in h..h + STRIP {
        for x in 0..w {
            let c = colors.get((x * n / w) as usize).copied().unwrap_or([0; 3]);
            img.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], 255]));
        }
    }
    Some(img)
}

/// Створка длиной `len` и высотой `tall` (клетки) от петель вдоль x: модель, растянутая по
/// проёму (с зазором у косяков и у верха, как у прежней доски), одним куском. Нет модели — `None`.
pub fn leaf_part(look: DoorLook, len: f32, tall: f32) -> Option<LeafPart> {
    let m = model(look.kind)?;
    let (body, colors) = body_and_colors(m)?;
    let (w, h) = m.images.get(body)?.dimensions();
    let size = [w, h + STRIP];
    // Доля высоты картинки под фактурой тела; середина полосы цвета `i` в картинке.
    let vk = h as f32 / (h + STRIP) as f32;
    let n = colors.len().max(1) as f32;
    let spot = |i: usize| [(i as f32 + 0.5) / n, (h as f32 + STRIP as f32 / 2.0) / (h + STRIP) as f32];
    let (sx, sy) = (((len - 0.04) / W).max(0.01), ((tall - 0.02) / H).max(0.01));
    let mut mesh = Mesh::default();
    let mut material = None;
    for p in &m.parts {
        let base = mesh.v.len() as u32;
        for v in &p.mesh.v {
            let mut v = *v;
            v.p = [0.02 + v.p[0] * sx, v.p[1] * sy, v.p[2]];
            // Нормаль при растяжении — обратным растяжением.
            let nn = [v.n[0] / sx, v.n[1] / sy, v.n[2]];
            let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt().max(1e-9);
            v.n = [nn[0] / l, nn[1] / l, nn[2] / l];
            v.uv = match p.material.look {
                Look::Picture(_) => [v.uv[0], v.uv[1] * vk],
                Look::Color(c) => spot(colors.iter().position(|&q| q == c).unwrap_or(0)),
            };
            mesh.v.push(v);
        }
        mesh.idx.extend(p.mesh.idx.iter().map(|i| i + base));
        if let Look::Picture(_) = p.material.look {
            material = Some(p.material.clone());
        }
    }
    let material = Material { repeat: false, ..material? };
    Some(LeafPart { mesh, picture: (format!("door3d:{}:{:02x}{:02x}{:02x}", look.kind.name(), look.tint[0], look.tint[1], look.tint[2]), size), material })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_pick_the_kind_and_colour_of_the_leaf() {
        assert_eq!(look_of(Some("builtin:door_wood")).kind, Kind::Wood);
        assert_eq!(look_of(None).kind, Kind::Wood);
        assert_eq!(look_of(Some("builtin:door_metal")).kind, Kind::Metal);
        let fa = |n: &str| look_of(Some(&format!("pack:E:\\D&D\\FA_Objects_A.dungeondraft_pack|textures/portals/{n}")));
        assert_eq!(fa("Door_Wood_Walnut_A_1x1.webp"), DoorLook { kind: Kind::Wood, tint: [132, 90, 58] });
        assert_eq!(fa("Door_Wood_Ashen_J_Rusty_1x1.webp").kind, Kind::Wood, "ржавое — железо деревянной двери");
        assert_eq!(fa("Door_Stone_Sandstone_F_Rusty_1x1.webp"), DoorLook { kind: Kind::Stone, tint: [216, 188, 142] });
        assert_eq!(fa("Door_Metal_Gray_B_1x1.webp").kind, Kind::Metal);
        assert_ne!(fa("Door_Wood_Red_A_1x1.webp").tint, fa("Door_Wood_Dark_A_1x1.webp").tint);
    }

    #[test]
    fn each_model_is_a_leaf_with_a_tinted_body_and_iron() {
        for kind in [Kind::Wood, Kind::Metal, Kind::Stone] {
            let m = model(kind).unwrap_or_else(|| panic!("{kind:?}: модель не читается"));
            let [lo, hi] = m.bounds().unwrap();
            assert!(lo[0] > -0.06 && lo[0] < 0.02 && (hi[0] - W).abs() < 0.06, "{kind:?}: по x от петель до края: {lo:?} {hi:?}");
            assert!(lo[1].abs() < 0.01 && (hi[1] - H).abs() < 0.02, "{kind:?}: по высоте 0..1,4: {lo:?} {hi:?}");
            assert!(hi[2] - lo[2] < 0.2, "{kind:?}: тонкая: {lo:?} {hi:?}");
            let look = DoorLook { kind, tint: [100, 50, 25] };
            let part = leaf_part(look, 2.0, 1.2).unwrap();
            let (key, size) = part.picture.clone();
            let img = picture(&key).unwrap();
            assert_eq!([img.width(), img.height()], size);
            let body = img.height() - STRIP;
            assert!((0..body).all(|y| img.get_pixel(7, y)[0] <= 100 && img.get_pixel(7, y)[2] <= 25), "тело окрашено не светлее цвета");
            let iron = img.get_pixel(3, body + STRIP / 2);
            assert!(iron[0] < 90 && iron[0].abs_diff(iron[2]) < 12, "железо — своим цветом, тёмным и серым: {iron:?}");
            // Вершины железа — в его полосе картинки, тела — выше неё.
            let split = body as f32 / size[1] as f32;
            assert!(part.mesh.v.iter().any(|v| v.uv[1] > split) && part.mesh.v.iter().any(|v| v.uv[1] < split));
            let (plo, phi) = part.mesh.v.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), v| ([0, 1, 2].map(|k| l[k].min(v.p[k])), [0, 1, 2].map(|k| h[k].max(v.p[k]))));
            assert!((phi[0] - (hi[0] - W) * 1.96 - 1.98).abs() < 0.05 && phi[1] < 1.2, "растянута по проёму 2 × 1,2: {plo:?} {phi:?}");
            assert!(part.mesh.idx.iter().all(|&i| (i as usize) < part.mesh.v.len()));
            assert!(part.mesh.v.iter().all(|v| ((v.n[0] * v.n[0] + v.n[1] * v.n[1] + v.n[2] * v.n[2]) - 1.0).abs() < 1e-3));
        }
    }
}
