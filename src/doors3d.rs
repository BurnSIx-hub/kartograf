//! Створки дверей 3D-карты — модели Blender (`tools/blender/doors.py` → `assets/doors3d`):
//! деревянная (доски, поперечины и раскос сзади, железные полосы петель, кольцо), металлическая
//! (лист в раме с заклёпками, ручка-рычаг) и каменная (плита с резной рамой, кольцо).
//!
//! Модель — створка в клетках: петли на x = 0, другой край на x = 1, низ на y = 0, верх на
//! y = 1,4 (дверь 7 футов), толщина по z. Створка растягивается по проёму ([`leaf_part`]).
//!
//! Модель — один кусок с одним материалом и тремя картами, запечёнными в Blender: цвет, нормали
//! (волокна, вмятины, сколы, скруглённые рёбра) и ORM (затенение, шероховатость, металл). 3D Canvas
//! читает метки узла (нетвёрдая, не загораживает обзор) только у узла из одного куска — из двух
//! открытая створка стала бы для фишки стеной.
//!
//! Цвет тела — по рисунку двери на карте: порода или камень из имени (`Door_Wood_Walnut…`,
//! `Door_Stone_Sandstone…`) перекрашивает его с сохранением разброса тонов (грязь, износ,
//! волокна). У дерева и камня красится всё, что не металл, — железо остаётся своим; у
//! металлической — вся створка.

use crate::model3d::{self, Model3d};
use crate::scene3d::{Material, Mesh};
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

/// Вид створки: модель и цвет её тела (светлый тон, как на рисунке двери: средний тон тела —
/// [`BODY`] от него).
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

/// Средний тон тела створки — такая доля цвета двери: цвет из имени — светлый тон рисунка.
const BODY: f32 = 0.85;

/// Модель створки, её карты и средний цвет тела в картинке цвета.
struct Door {
    model: Model3d,
    normal: image::RgbaImage,
    orm: image::RgbaImage,
    mean: [f32; 3],
}

/// Красится ли точка картинки цвета (по точке ORM): у дерева и камня — не металл, у металлической
/// двери — всё. Пустое место развёртки (шероховатость 0) — нет.
fn tinted(kind: Kind, orm: &image::Rgba<u8>) -> bool {
    orm[1] > 8 && (kind == Kind::Metal || orm[2] < 40)
}

/// Модель створки (один раз на вид).
fn door(kind: Kind) -> Option<&'static Door> {
    static DOORS: [OnceLock<Option<Door>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    DOORS[kind as usize]
        .get_or_init(|| {
            let model = model3d::read_gltf(kind.bytes(), None, 1.0).ok()?;
            let (normal, orm) = model3d::material_maps(kind.bytes()).ok()?;
            let base = model.images.first()?;
            if base.dimensions() != orm.dimensions() {
                return None;
            }
            let (mut sum, mut n) = ([0.0f64; 3], 0usize);
            for (p, o) in base.pixels().zip(orm.pixels()) {
                if tinted(kind, o) {
                    (0..3).for_each(|k| sum[k] += p[k] as f64);
                    n += 1;
                }
            }
            let mean = sum.map(|v| (v / n.max(1) as f64).max(1.0) as f32);
            Some(Door { model, normal, orm, mean })
        })
        .as_ref()
}

/// Створка сцены: сетка в осях створки, картинки — цвет, нормали, ORM (ключ для сцены и размер)
/// и материал (картинку и карты ему ставит сцена).
pub struct LeafPart {
    pub mesh: Mesh,
    pub pictures: [(String, [u32; 2]); 3],
    pub material: Material,
}

/// Ключ — карта (нормали, ORM): её пишут без потерь.
pub fn is_map(key: &str) -> bool {
    key.starts_with("door3d:") && (key.ends_with(":normal") || key.ends_with(":orm"))
}

/// Картинка для ключа из [`leaf_part`]: цвет створки, перекрашенный в цвет двери, или её карта.
pub fn picture(key: &str) -> Option<image::RgbaImage> {
    let mut it = key.strip_prefix("door3d:")?.split(':');
    let kind = Kind::of(it.next()?)?;
    let d = door(kind)?;
    let tint = match it.next()? {
        "normal" => return Some(d.normal.clone()),
        "orm" => return Some(d.orm.clone()),
        t => u32::from_str_radix(t, 16).ok()?,
    };
    let tint = [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8];
    let k = [0, 1, 2].map(|c| tint[c] as f32 * BODY / d.mean[c]);
    let mut img = d.model.images.first()?.clone();
    for (p, o) in img.pixels_mut().zip(d.orm.pixels()) {
        if tinted(kind, o) {
            for c in 0..3 {
                p[c] = (p[c] as f32 * k[c]).round().min(255.0) as u8;
            }
        }
        p[3] = 255;
    }
    Some(img)
}

/// Створка длиной `len` и высотой `tall` (клетки) от петель вдоль x: модель, растянутая по
/// проёму (с зазором у петель и у верха, у свободного края — `free`), одним куском; `mirror` —
/// отражённая по толщине (вторая створка двойной двери: лицом в ту же сторону, что и первая).
/// Нет модели — `None`.
pub fn leaf_part(look: DoorLook, len: f32, tall: f32, free: f32, mirror: bool) -> Option<LeafPart> {
    let d = door(look.kind)?;
    let size = |img: &image::RgbaImage| [img.width(), img.height()];
    let name = look.kind.name();
    let pictures = [
        (format!("door3d:{name}:{:02x}{:02x}{:02x}", look.tint[0], look.tint[1], look.tint[2]), size(d.model.images.first()?)),
        (format!("door3d:{name}:normal"), size(&d.normal)),
        (format!("door3d:{name}:orm"), size(&d.orm)),
    ];
    let (sx, sy) = (((len - 0.02 - free) / W).max(0.01), ((tall - 0.02) / H).max(0.01));
    let mut mesh = Mesh::default();
    for p in &d.model.parts {
        let base = mesh.v.len() as u32;
        for v in &p.mesh.v {
            let mut v = *v;
            v.p = [0.02 + v.p[0] * sx, v.p[1] * sy, v.p[2]];
            // Нормаль при растяжении — обратным растяжением.
            let nn = [v.n[0] / sx, v.n[1] / sy, v.n[2]];
            let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt().max(1e-9);
            v.n = [nn[0] / l, nn[1] / l, nn[2] / l];
            if mirror {
                v.p[2] = -v.p[2];
                v.n[2] = -v.n[2];
            }
            mesh.v.push(v);
        }
        // Отражение выворачивает обход треугольников — обратно.
        for t in p.mesh.idx.chunks_exact(3) {
            let t = if mirror { [t[0], t[2], t[1]] } else { [t[0], t[1], t[2]] };
            mesh.idx.extend(t.map(|i| i + base));
        }
    }
    let material = Material { repeat: false, alpha: crate::scene3d::Alpha::Opaque, opacity: 1.0, ..d.model.parts.first()?.material.clone() };
    Some(LeafPart { mesh, pictures, material })
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
    fn each_model_is_one_leaf_with_colour_normal_and_orm_maps() {
        for kind in [Kind::Wood, Kind::Metal, Kind::Stone] {
            let d = door(kind).unwrap_or_else(|| panic!("{kind:?}: модель не читается"));
            assert_eq!(d.model.parts.len(), 1, "{kind:?}: один кусок — иначе 3D Canvas не прочтёт метки створки");
            let [lo, hi] = d.model.bounds().unwrap();
            assert!(lo[0] > -0.06 && lo[0] < 0.02 && (hi[0] - W).abs() < 0.06, "{kind:?}: по x от петель до края: {lo:?} {hi:?}");
            assert!(lo[1].abs() < 0.01 && (hi[1] - H).abs() < 0.02, "{kind:?}: по высоте 0..1,4: {lo:?} {hi:?}");
            assert!(hi[2] - lo[2] < 0.2, "{kind:?}: тонкая: {lo:?} {hi:?}");
            assert!(d.model.triangles() < 12_000, "{kind:?}: лёгкая: {}", d.model.triangles());
            // Нормали — в среднем «прямо из поверхности», но с рельефом; металл — у железа.
            let used: Vec<_> = d.normal.pixels().zip(d.orm.pixels()).filter(|(_, o)| o[1] > 8).collect();
            assert!(used.len() > d.normal.pixels().len() / 2, "{kind:?}: развёртка занимает картинку");
            let blue = used.iter().map(|(n, _)| n[2] as f32).sum::<f32>() / used.len() as f32;
            assert!(blue > 200.0, "{kind:?}: нормали в осях поверхности: синий {blue}");
            assert!(used.iter().filter(|(n, _)| n[0].abs_diff(128) > 12 || n[1].abs_diff(128) > 12).count() > used.len() / 20, "{kind:?}: в нормалях есть рельеф");
            assert!(used.iter().any(|(_, o)| o[2] > 200), "{kind:?}: есть металл");
        }
    }

    #[test]
    fn the_body_takes_the_door_colour_and_the_iron_keeps_its_own() {
        for kind in [Kind::Wood, Kind::Stone, Kind::Metal] {
            let d = door(kind).unwrap();
            let look = DoorLook { kind, tint: [200, 100, 50] };
            let part = leaf_part(look, 2.0, 1.2, 0.02, false).unwrap();
            // Отражённая: та же створка с другим лицом, треугольники смотрят наружу как и были.
            let back = leaf_part(look, 2.0, 1.2, 0.02, true).unwrap();
            assert!(part.mesh.v.iter().zip(&back.mesh.v).all(|(a, b)| a.p[2] == -b.p[2] && a.p[0] == b.p[0]));
            let tri = |m: &Mesh, k: usize| {
                let [a, b, c] = [0, 1, 2].map(|j| m.v[m.idx[k * 3 + j] as usize]);
                let (e1, e2) = ([0, 1, 2].map(|q| b.p[q] - a.p[q]), [0, 1, 2].map(|q| c.p[q] - a.p[q]));
                let g = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                g[0] * a.n[0] + g[1] * a.n[1] + g[2] * a.n[2]
            };
            let agree = |m: &Mesh| (0..m.idx.len() / 3).filter(|&k| tri(m, k) > 0.0).count() as f32 / (m.idx.len() / 3) as f32;
            assert!((agree(&part.mesh) - agree(&back.mesh)).abs() < 0.01, "обход как у исходной: {} {}", agree(&part.mesh), agree(&back.mesh));
            let [(key, size), (nk, _), (ok, _)] = part.pictures.clone();
            assert!(!is_map(&key) && is_map(&nk) && is_map(&ok));
            let img = picture(&key).unwrap();
            assert_eq!([img.width(), img.height()], size);
            assert_eq!(picture(&nk).unwrap().dimensions(), d.normal.dimensions());
            // Средний цвет тела — доля BODY цвета двери, разброс тонов сохранён.
            let body: Vec<_> = img.pixels().zip(d.orm.pixels()).filter(|(_, o)| tinted(kind, o)).map(|(p, _)| *p).collect();
            let mean = [0, 1, 2].map(|c| body.iter().map(|p| p[c] as f32).sum::<f32>() / body.len() as f32);
            for c in 0..3 {
                let want = look.tint[c] as f32 * BODY;
                assert!((mean[c] - want).abs() < want * 0.08 + 3.0, "{kind:?}: средний цвет {mean:?} ≈ {want}");
            }
            let spread = body.iter().map(|p| p[0]).max().unwrap() - body.iter().map(|p| p[0]).min().unwrap();
            assert!(spread > 60, "{kind:?}: тона разные: {spread}");
            // Железо деревянной и каменной — своим цветом: тёмное, не рыжее от цвета двери.
            if kind != Kind::Metal {
                let iron: Vec<_> = img.pixels().zip(d.orm.pixels()).filter(|(_, o)| o[1] > 8 && o[2] > 200).map(|(p, _)| *p).collect();
                let r = iron.iter().map(|p| p[0] as f32).sum::<f32>() / iron.len().max(1) as f32;
                assert!(!iron.is_empty() && r < 90.0, "{kind:?}: железо тёмное: {r}");
            }
            let (plo, phi) = part.mesh.v.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(l, h), v| ([0, 1, 2].map(|k| l[k].min(v.p[k])), [0, 1, 2].map(|k| h[k].max(v.p[k]))));
            let [_, hi] = d.model.bounds().unwrap();
            assert!((phi[0] - (hi[0] - W) * 1.96 - 1.98).abs() < 0.05 && phi[1] < 1.2, "растянута по проёму 2 × 1,2: {plo:?} {phi:?}");
            assert!(part.mesh.idx.iter().all(|&i| (i as usize) < part.mesh.v.len()));
            assert!(part.mesh.v.iter().all(|v| ((v.n[0] * v.n[0] + v.n[1] * v.n[1] + v.n[2] * v.n[2]) - 1.0).abs() < 1e-3));
        }
    }
}
