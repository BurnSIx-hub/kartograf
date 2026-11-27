//! Окна 3D-карты — модели Blender (`tools/blender/windows.py` → `assets/windows3d`): деревянное
//! (откосы и подоконник из досок, рама из двух створок с фрамугой и переплётами, железные петли) и
//! каменное (откосы из тёсаных блоков, подоконник, крестовое окно — средник с поперечиной, свинцовые
//! переплёты ромбом). И стекло — бесшовная картинка на клетку: цвет с прозрачностью, рябь старого
//! стекла, пятна.
//!
//! Модель рамы — проём в клетках: x от 0 до 1 вдоль стены, y от 0 (подоконник) до 0,8 (верх
//! проёма), по z — стена толщиной 2·[`REF`] вокруг нуля, стекло — плоскость z = 0. Рама
//! растягивается по проёму ([`frame`]): длинный проём — несколько окон в ряд, у каждого края
//! (откосы, рама) — как в модели, середина тянется; по толщине откосы доходят до лица стены,
//! середина (рама, средник) — как есть, подоконник выступает за лицо, как в модели.
//!
//! Рама — один кусок с одним материалом и тремя картами, как створки дверей ([`crate::doors3d`]).
//! Вид и цвет — по рисунку подоконника на карте (`Window_Sill_Wood_Ashen…`, `builtin:window_stone`):
//! тёсаный камень и доски перекрашиваются с сохранением тонов, железо и свинец — своим цветом.

use crate::doors3d::{self, Baked};
use crate::scene3d::{Alpha, Material, Mesh, Vertex};
use std::sync::OnceLock;

/// Ширина и высота проёма в модели (клетки).
const W: f32 = 1.0;
const H: f32 = 0.8;
/// Полутолщина стены в модели: откосы — от −REF до REF.
const REF: f32 = 0.125;
/// По толщине до стольких от середины — как в модели (рама, переплёты); дальше до лица стены —
/// растягивается.
const CORE: f32 = 0.035;
/// По длине у каждого края столько — как в модели (откос и рама); середина тянется.
const EDGE: f32 = 0.075;
/// Ширина одного окна: длинный проём — несколько окон в ряд.
const BAY: f32 = 1.0;
/// Ширина малого окна (`Window_Sill_Small_*`): оно посередине проёма, по бокам — стена.
pub const SMALL: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Wood,
    Stone,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Wood => "wood",
            Kind::Stone => "stone",
        }
    }

    fn of(name: &str) -> Option<Kind> {
        [Kind::Wood, Kind::Stone].into_iter().find(|k| k.name() == name)
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Kind::Wood => include_bytes!("../assets/windows3d/window_wood.glb"),
            Kind::Stone => include_bytes!("../assets/windows3d/window_stone.glb"),
        }
    }
}

const GLASS: &[u8] = include_bytes!("../assets/windows3d/glass.glb");

/// Вид окна: модель, цвет досок или камня (светлый тон, как у створок дверей) и малое ли оно.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowLook {
    pub kind: Kind,
    pub tint: [u8; 3],
    pub small: bool,
}

/// Вид окна по ключу рисунка подоконника: дерево или камень и их цвет по имени — как у дверей
/// ([`doors3d::look_of`]), малое (`Small`). Окно без рисунка — каменное, как встроенное по
/// умолчанию.
pub fn look_of(art: Option<&str>) -> WindowLook {
    let art = art.unwrap_or("builtin:window_stone");
    let d = doors3d::look_of(Some(art));
    let kind = if d.kind == doors3d::Kind::Wood { Kind::Wood } else { Kind::Stone };
    let name = art.rsplit(['/', '\\', '|', ':']).next().unwrap_or(art).to_lowercase();
    WindowLook { kind, tint: d.tint, small: name.split(|c: char| !c.is_alphanumeric()).any(|w| w == "small") }
}

/// Красится ли точка картинки цвета (по точке ORM): доски и камень — да, железо и свинец (металл)
/// и пустое место развёртки — нет.
fn tinted(orm: &image::Rgba<u8>) -> bool {
    orm[1] > 8 && orm[2] < 40
}

/// Рама окна (один раз на вид).
fn window(kind: Kind) -> Option<&'static Baked> {
    static WINDOWS: [OnceLock<Option<Baked>>; 2] = [OnceLock::new(), OnceLock::new()];
    WINDOWS[kind as usize].get_or_init(|| Baked::read(kind.bytes(), tinted)).as_ref()
}

/// Стекло: картинка цвета с прозрачностью и карты.
fn glass() -> Option<&'static Baked> {
    static GLASS_MODEL: OnceLock<Option<Baked>> = OnceLock::new();
    GLASS_MODEL.get_or_init(|| Baked::read(GLASS, |_| false)).as_ref()
}

/// Ключ — карта или стекло (у стекла прозрачность): их пишут без потерь.
pub fn is_map(key: &str) -> bool {
    key.starts_with("window3d:") && (key.ends_with(":normal") || key.ends_with(":orm") || key.starts_with("window3d:glass:"))
}

/// Картинка для ключа из [`frame`] и [`glass_part`]: цвет рамы, перекрашенный в цвет окна, её
/// карты, картинки стекла.
pub fn picture(key: &str) -> Option<image::RgbaImage> {
    let mut it = key.strip_prefix("window3d:")?.split(':');
    let what = it.next()?;
    let w = if what == "glass" { glass()? } else { window(Kind::of(what)?)? };
    match it.next()? {
        "normal" => Some(w.normal.clone()),
        "orm" => Some(w.orm.clone()),
        "base" if what == "glass" => w.model.images.first().cloned(),
        t => {
            let t = u32::from_str_radix(t, 16).ok()?;
            w.recolour([(t >> 16) as u8, (t >> 8) as u8, t as u8], tinted)
        }
    }
}

/// Кусок окна для сцены: сетка в осях проёма, картинки — цвет, нормали, ORM (ключ для сцены и
/// размер) и материал (картинку и карты ему ставит сцена).
pub struct WindowPart {
    pub mesh: Mesh,
    pub pictures: [(String, [u32; 2]); 3],
    pub material: Material,
}

/// Рама окна в проёме длиной `len` и высотой `tall` (клетки) в стене полутолщиной `half`, в осях
/// проёма: x — вдоль от начала, y — вверх от подоконника, z — поперёк стены. Нет модели — `None`.
pub fn frame(look: WindowLook, len: f32, tall: f32, half: f32) -> Option<WindowPart> {
    let w = window(look.kind)?;
    let size = |img: &image::RgbaImage| [img.width(), img.height()];
    let name = look.kind.name();
    let pictures = [
        (format!("window3d:{name}:{:02x}{:02x}{:02x}", look.tint[0], look.tint[1], look.tint[2]), size(w.model.images.first()?)),
        (format!("window3d:{name}:normal"), size(&w.normal)),
        (format!("window3d:{name}:orm"), size(&w.orm)),
    ];
    let bays = (len / BAY).round().max(1.0);
    let bw = len / bays;
    let sy = tall / H;
    // По длине: края — как есть, середина тянется; узкое окно — всё целиком. Второе — растяжение в
    // точке (для нормалей).
    let mid = (bw - 2.0 * EDGE) / (W - 2.0 * EDGE);
    let along = |x: f32| -> (f32, f32) {
        if mid < 0.3 {
            (x * bw / W, bw / W)
        } else if x <= EDGE {
            (x, 1.0)
        } else if x >= W - EDGE {
            (bw - (W - x), 1.0)
        } else {
            (EDGE + (x - EDGE) * mid, mid)
        }
    };
    // По толщине: середина — как есть, откосы — до лица стены, выступ подоконника — за ним, как в
    // модели; в тонкой стене — всё целиком.
    let k = (half - CORE) / (REF - CORE);
    let across = |z: f32| -> (f32, f32) {
        let a = z.abs();
        if half < CORE + 0.005 {
            (z * half / REF, half / REF)
        } else if a <= CORE {
            (z, 1.0)
        } else if a <= REF {
            ((CORE + (a - CORE) * k).copysign(z), k)
        } else {
            ((a + half - REF).copysign(z), 1.0)
        }
    };
    let mut mesh = Mesh::default();
    for bay in 0..bays as usize {
        let x0 = bay as f32 * bw;
        for p in &w.model.parts {
            let base = mesh.v.len() as u32;
            for v in &p.mesh.v {
                let ((x, sx), (z, sz)) = (along(v.p[0]), across(v.p[2]));
                // Нормаль при растяжении — обратным растяжением.
                let n = [v.n[0] / sx, v.n[1] / sy, v.n[2] / sz];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-9);
                mesh.v.push(Vertex { p: [x0 + x, v.p[1] * sy, z], n: n.map(|c| c / l), uv: v.uv });
            }
            mesh.idx.extend(p.mesh.idx.iter().map(|i| i + base));
        }
    }
    let material = Material { repeat: false, alpha: Alpha::Opaque, opacity: 1.0, ..w.model.parts.first()?.material.clone() };
    Some(WindowPart { mesh, pictures, material })
}

/// Стекло в проёме длиной `len` и высотой `tall`: плоскость z = 0 в осях проёма, картинка — на
/// клетку, повторяется; полупрозрачное, с обеих сторон.
pub fn glass_part(len: f32, tall: f32) -> Option<WindowPart> {
    let g = glass()?;
    let size = |img: &image::RgbaImage| [img.width(), img.height()];
    let pictures = [
        ("window3d:glass:base".to_owned(), size(g.model.images.first()?)),
        ("window3d:glass:normal".to_owned(), size(&g.normal)),
        ("window3d:glass:orm".to_owned(), size(&g.orm)),
    ];
    let mut mesh = Mesh::default();
    crate::scene3d::quad(&mut mesh, [[0.0, 0.0, 0.0], [len, 0.0, 0.0], [len, tall, 0.0], [0.0, tall, 0.0]], [0.0, 0.0, 1.0], [[0.0, 0.0], [len, 0.0], [len, -tall], [0.0, -tall]]);
    let material = Material { look: crate::scene3d::Look::Color([255; 3]), alpha: Alpha::Blend, two_sided: true, repeat: true, opacity: 1.0, maps: None };
    Some(WindowPart { mesh, pictures, material })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sill_picks_the_kind_and_colour_of_the_window() {
        assert_eq!(look_of(Some("builtin:window_wood")).kind, Kind::Wood);
        assert_eq!(look_of(Some("builtin:window_stone")).kind, Kind::Stone);
        assert_eq!(look_of(None).kind, Kind::Stone, "без рисунка — как встроенное по умолчанию");
        let fa = |n: &str| look_of(Some(&format!("pack:E:\\D&D\\FA_Objects_A.dungeondraft_pack|textures/portals/Windows/Window_Sills/{n}")));
        assert_eq!(fa("Window_Sill_Wood_Walnut_A1_1x1.webp"), WindowLook { kind: Kind::Wood, tint: [132, 90, 58], small: false });
        assert_eq!(fa("Window_Sill_Stone_Sandstone_A1_1x1.webp"), WindowLook { kind: Kind::Stone, tint: [216, 188, 142], small: false });
        assert_eq!(fa("Window_Sill_Small_Stone_Slate_A1_1x1.webp"), WindowLook { kind: Kind::Stone, tint: [124, 130, 138], small: true });
        assert!(!fa("Window_Sill_Large_Wood_Ashen_A1_2x1.webp").small);
        assert_ne!(fa("Window_Sill_Wood_Ashen_A1_1x1.webp").tint, fa("Window_Sill_Wood_Dark_A1_1x1.webp").tint);
    }

    #[test]
    fn each_frame_is_one_piece_with_colour_normal_and_orm_maps() {
        for kind in [Kind::Wood, Kind::Stone] {
            let w = window(kind).unwrap_or_else(|| panic!("{kind:?}: модель не читается"));
            assert_eq!(w.model.parts.len(), 1, "{kind:?}: один кусок");
            let [lo, hi] = w.model.bounds().unwrap();
            assert!(lo[0].abs() < 0.01 && (hi[0] - W).abs() < 0.01, "{kind:?}: по x во всю ширину проёма: {lo:?} {hi:?}");
            assert!(lo[1].abs() < 0.01 && (hi[1] - H).abs() < 0.01, "{kind:?}: по высоте 0..0,8: {lo:?} {hi:?}");
            assert!(hi[2] > REF && hi[2] < REF + 0.08 && (lo[2] + hi[2]).abs() < 0.01, "{kind:?}: откосы до лица стены, подоконник чуть дальше: {lo:?} {hi:?}");
            assert!(w.model.triangles() < 4_000, "{kind:?}: лёгкое: {}", w.model.triangles());
            let used: Vec<_> = w.normal.pixels().zip(w.orm.pixels()).filter(|(_, o)| o[1] > 8).collect();
            assert!(used.len() > w.normal.pixels().len() / 3, "{kind:?}: развёртка занимает картинку");
            let blue = used.iter().map(|(n, _)| n[2] as f32).sum::<f32>() / used.len() as f32;
            assert!(blue > 200.0, "{kind:?}: нормали в осях поверхности: синий {blue}");
            assert!(used.iter().any(|(_, o)| o[2] > 150), "{kind:?}: есть металл (петли, свинец)");
        }
        let g = glass().expect("стекло читается");
        let base = g.model.images.first().unwrap();
        let alpha = base.pixels().map(|p| p[3] as f32).sum::<f32>() / base.pixels().len() as f32;
        assert!((30.0..120.0).contains(&alpha), "стекло полупрозрачное: {alpha}");
        assert!(g.orm.pixels().map(|p| p[1] as f32).sum::<f32>() / (g.orm.pixels().len() as f32) < 90.0, "стекло гладкое");
    }

    #[test]
    fn a_frame_stretches_to_the_opening_and_the_wall() {
        let look = look_of(Some("builtin:window_wood"));
        let one = frame(look, 1.0, 0.8, REF).unwrap();
        let w = window(Kind::Wood).unwrap();
        assert_eq!(one.mesh.v.len(), w.model.parts[0].mesh.v.len());
        let span = |m: &Mesh, k: usize| m.v.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v.p[k]), h.max(v.p[k])));
        // Два окна в ряд в проёме в две клетки; толстая стена — откосы до её лица.
        let two = frame(look, 2.0, 0.8, 0.2).unwrap();
        assert_eq!(two.mesh.v.len(), 2 * one.mesh.v.len());
        let (x0, x1) = span(&two.mesh, 0);
        assert!(x0.abs() < 0.01 && (x1 - 2.0).abs() < 0.01, "{x0} {x1}");
        let (z0, z1) = span(&two.mesh, 2);
        let (o0, o1) = span(&one.mesh, 2);
        assert!((z1 - (o1 + 0.2 - REF)).abs() < 1e-3 && (z0 - (o0 - 0.2 + REF)).abs() < 1e-3, "подоконник за лицом стены на столько же: {z0} {z1}");
        // Рама посередине толщины — как в модели.
        let core = |m: &Mesh| m.v.iter().filter(|v| v.p[2].abs() <= CORE).count();
        assert_eq!(core(&two.mesh), 2 * core(&one.mesh));
        // Тонкая стена: всё внутри неё и подоконника.
        let thin = frame(look, 0.7, 0.8, 0.03).unwrap();
        let (t0, t1) = span(&thin.mesh, 2);
        assert!(t1 < 0.06 && t0 > -0.06, "{t0} {t1}");
        assert!(thin.mesh.v.iter().all(|v| ((v.n[0] * v.n[0] + v.n[1] * v.n[1] + v.n[2] * v.n[2]) - 1.0).abs() < 1e-3));
        let [(key, size), (nk, _), (ok, _)] = two.pictures;
        assert!(!is_map(&key) && is_map(&nk) && is_map(&ok));
        let img = picture(&key).unwrap();
        assert_eq!([img.width(), img.height()], size);
        // Стекло — во весь проём, его картинки — по ключам, без потерь.
        let g = glass_part(2.0, 0.8).unwrap();
        assert_eq!(span(&g.mesh, 0), (0.0, 2.0));
        assert!(g.pictures.iter().all(|(k, _)| is_map(k) && picture(k).is_some()));
        assert_eq!(picture(&g.pictures[0].0).unwrap().pixels().next().map(|p| p[3] < 200), Some(true), "у стекла своя прозрачность");
    }
}
