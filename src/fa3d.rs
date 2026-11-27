//! Модели к рисункам FA по имени: у предмета без своей модели (`.glb` рядом с картинкой) модель
//! подбирается по имени его рисунка — бочка к бочке, стол к столу — из набора CC0 на полке
//! («CC0/Poly Haven»).
//!
//! Подобранная модель встаёт в непрозрачную часть рисунка, а не во всю его рамку: у рисунков FA
//! вокруг предмета бывает пустое поле в половину рамки (табурет, сундук). По горизонтали она
//! растягивается в эту часть, по высоте — меньше: предметы FA крупнее настоящих, и бочка в
//! рост рисунка вышла бы в рост человека. Записывается своей `.glb` (исходная модель со всеми
//! картами под узлом с поворотом и растяжением) с точками без площади в углах рамки рисунка:
//! 3D Canvas растягивает модель тайла по тайлу, как у крыш.

/// Набор на полке, из которого подбираются модели.
pub const PACK: &str = "Poly Haven";
/// Версия правил и укладки: меняется — подобранные модели пишутся заново.
pub const VERSION: u32 = 1;
/// Правило: имя рисунка начинается с одного из `starts`, в пути есть все слова `need` и нет слов
/// `not` (битые, упавшие — у модели этого нет), модели — по порядку предпочтения, берётся та, что
/// по сторонам ближе к рисунку.
/// - `lying` — на боку: бочонок и бочка «Side» лежат, модель — стоячая бочка;
/// - `stretch` — во сколько раз стороны модели могут соотноситься иначе, чем у рисунка (с
///   поворотом на четверть оборота): доску скамьи и ящик растянуть можно, круглую бочку — нет;
/// - `top` — сверху на модель ложится сам рисунок: у полной бочки и ящика с товаром сверху видно,
///   что в них (вино, хлеб), сбоку — модель.
pub struct Rule {
    pub starts: &'static [&'static str],
    pub need: &'static [&'static str],
    pub not: &'static [&'static str],
    pub models: &'static [&'static str],
    pub lying: bool,
    pub stretch: f32,
    pub top: bool,
}

const BROKEN: &[&str] = &["broken", "fallen", "damaged"];
const FILLED: &[&str] = &["/filled"];
const NONE: &[&str] = &[];
const BARRELS: &[&str] = &["wine_barrel_01"];
const DRUMS: &[&str] = &["barrel_01", "barrel_02", "barrel_03"];
const CRATES: &[&str] = &["wooden_crate_01", "wooden_crate_02", "wooden_military_crate"];

pub const RULES: &[Rule] = &[
    Rule { starts: &["barrel_side_wood", "keg_wood"], need: NONE, not: BROKEN, models: BARRELS, lying: true, stretch: 1.35, top: false },
    Rule { starts: &["barrel_wood", "barrel_large_wood"], need: NONE, not: &["broken", "/filled", "_side", "lid", "stand", "torture"], models: BARRELS, lying: false, stretch: 1.3, top: false },
    Rule { starts: &["barrel_wood"], need: FILLED, not: BROKEN, models: BARRELS, lying: false, stretch: 1.3, top: true },
    Rule { starts: &["barrel_metal"], need: NONE, not: &["broken", "/filled"], models: DRUMS, lying: false, stretch: 1.3, top: false },
    Rule { starts: &["barrel_metal"], need: FILLED, not: BROKEN, models: DRUMS, lying: false, stretch: 1.3, top: true },
    Rule { starts: &["crate_wood", "box_crate_wood"], need: NONE, not: &["/filled", "lid", "broken"], models: CRATES, lying: false, stretch: 2.0, top: false },
    Rule { starts: &["crate_wood"], need: FILLED, not: BROKEN, models: CRATES, lying: false, stretch: 2.0, top: true },
    Rule { starts: &["table_rectangle_wood"], need: NONE, not: &["/filled", "arrange", "broken", "fallen"], models: &["wooden_table_02_2", "wooden_table_03", "wooden_table_01", "small_wooden_table_01"], lying: false, stretch: 1.8, top: false },
    Rule { starts: &["table_round_wood"], need: NONE, not: &["/filled", "arrange", "broken", "fallen"], models: &["round_wooden_table_01", "round_wooden_table_02"], lying: false, stretch: 1.3, top: false },
    Rule { starts: &["chair_wood"], need: NONE, not: BROKEN, models: &["wooden_chair_01", "painted_wooden_chair_02"], lying: false, stretch: 1.4, top: false },
    Rule { starts: &["rocking_chair_wood"], need: NONE, not: BROKEN, models: &["rockingchair_01"], lying: false, stretch: 1.4, top: false },
    Rule { starts: &["stool_wood"], need: NONE, not: BROKEN, models: &["wooden_stool_01", "painted_wooden_stool"], lying: false, stretch: 1.4, top: false },
    Rule { starts: &["bookshelf_wood"], need: NONE, not: &["path", "scroll"], models: &["wooden_bookshelf_worn"], lying: false, stretch: 1.6, top: false },
    Rule { starts: &["bucket_wood"], need: NONE, not: BROKEN, models: &["wooden_bucket_02", "wooden_bucket_01"], lying: false, stretch: 1.3, top: false },
    Rule { starts: &["lantern_metal"], need: NONE, not: BROKEN, models: &["lantern_01"], lying: false, stretch: 1.5, top: false },
];

/// Путь рисунка без пака и подкраски, строчными, через `/`: `pack:…|textures/objects/A/B.webp` →
/// `textures/objects/a/b.webp`.
pub fn inner_path(key: &str) -> String {
    key.rsplit('|').next().unwrap_or(key).replace('\\', "/").to_lowercase()
}

/// Правило для рисунка по его ключу.
pub fn rule_for(key: &str) -> Option<&'static Rule> {
    let path = inner_path(key);
    let name = path.rsplit('/').next().unwrap_or(&path);
    RULES.iter().find(|r| r.starts.iter().any(|s| name.starts_with(s)) && r.need.iter().all(|w| path.contains(w)) && !r.not.iter().any(|w| path.contains(w)))
}

/// Модель набора на полке по имени: `<имя>_<ш>x<в>.glb`, имя — без учёта регистра.
pub fn shelf_file(pack: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let prefix = format!("{name}_");
    let mut hits: Vec<std::path::PathBuf> = std::fs::read_dir(pack)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".glb")).is_some_and(|size| !size.contains('_') && crate::library::size_from_name(&format!("x_{size}")).is_some())
        })
        .collect();
    hits.sort();
    hits.into_iter().next()
}

/// Непрозрачная часть картинки — доли рамки: `[x0, y0, x1, y1]`, y — вниз.
pub fn opaque_box(img: &image::RgbaImage) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 40 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then(|| [x0 as f32 / w as f32, y0 as f32 / h as f32, x1 as f32 / w as f32, y1 as f32 / h as f32])
}

/// Как модель встаёт в рисунок: матрица glTF (по столбцам) из пространства модели на полке в
/// пространство предмета, рамка рисунка (углы — точками без площади), насколько стороны модели
/// не те (1 — те же) и высота её верха (клетки).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub matrix: [f32; 16],
    pub frame: [[f32; 3]; 2],
    pub skew: f32,
    pub top: f32,
}

type M3 = [[f32; 3]; 3];

fn mul3(a: &M3, b: &M3) -> M3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, c) in row.iter_mut().enumerate() {
            *c = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Рамка модели `bounds` после поворота `r`.
fn turned(r: &M3, [lo, hi]: [[f32; 3]; 2]) -> [[f32; 3]; 2] {
    let mut out = [[f32::MAX; 3], [f32::MIN; 3]];
    for k in 0..8 {
        let p = [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }];
        for i in 0..3 {
            let v: f32 = (0..3).map(|j| r[i][j] * p[j]).sum();
            out[0][i] = out[0][i].min(v);
            out[1][i] = out[1][i].max(v);
        }
    }
    out
}

/// Уложить модель с рамкой `bounds` (клетки, y — вверх, +z — вниз рисунка) в рисунок рамкой
/// `frame` (клетки) с непрозрачной частью `opaque` (доли рамки). `lying` — положить на бок: верх
/// модели — влево рисунка. Поворот на четверть оборота — если так стороны ближе к рисунку.
pub fn place(bounds: [[f32; 3]; 2], frame: [f32; 2], opaque: [f32; 4], lying: bool) -> Placement {
    const ID: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    // На бок: вокруг z на четверть оборота (верх модели — к −x).
    let lie: M3 = if lying { [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]] } else { ID };
    // Вокруг y на четверть оборота.
    let turn: M3 = [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]];
    let (bw, bd) = (((opaque[2] - opaque[0]) * frame[0]).max(1e-3), ((opaque[3] - opaque[1]) * frame[1]).max(1e-3));
    let (bcx, bcz) = (((opaque[0] + opaque[2]) / 2.0 - 0.5) * frame[0], ((opaque[1] + opaque[3]) / 2.0 - 0.5) * frame[1]);
    let skew_of = |b: &[[f32; 3]; 2]| {
        let (w, d) = ((b[1][0] - b[0][0]).max(1e-3), (b[1][2] - b[0][2]).max(1e-3));
        ((w / d) / (bw / bd)).ln().abs().exp()
    };
    let straight = lie;
    let quarter = mul3(&turn, &lie);
    let (r, b) = [straight, quarter].into_iter().map(|r| (r, turned(&r, bounds))).min_by(|a, b| skew_of(&a.1).total_cmp(&skew_of(&b.1))).unwrap();
    let skew = skew_of(&b);
    let (w, d) = ((b[1][0] - b[0][0]).max(1e-3), (b[1][2] - b[0][2]).max(1e-3));
    let (sx, sz) = (bw / w, bd / d);
    // По высоте: крупнее настоящей — меньше, чем по сторонам (среднее с настоящей высотой).
    let g = (sx * sz).sqrt();
    let sy = if g > 1.0 { g.sqrt() } else { g };
    let s = [sx, sy, sz];
    let m: M3 = [0, 1, 2].map(|i| [0, 1, 2].map(|j| s[i] * r[i][j]));
    let t = [bcx - sx * (b[0][0] + b[1][0]) / 2.0, -sy * b[0][1], bcz - sz * (b[0][2] + b[1][2]) / 2.0];
    let matrix = [m[0][0], m[1][0], m[2][0], 0.0, m[0][1], m[1][1], m[2][1], 0.0, m[0][2], m[1][2], m[2][2], 0.0, t[0], t[1], t[2], 1.0];
    let frame = [[-frame[0] / 2.0, 0.0, -frame[1] / 2.0], [frame[0] / 2.0, 0.0, frame[1] / 2.0]];
    Placement { matrix, frame, skew, top: sy * (b[1][1] - b[0][1]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &[f32; 16], p: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|i| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i])
    }

    #[test]
    fn names_pick_their_rule_and_full_or_broken_ones_stay_pictures() {
        let key = |p: &str| format!("pack:E:\\D&D\\FA_Objects_A_v3.52.dungeondraft_pack|textures/objects/{p}");
        let models = |p: &str| rule_for(&key(p)).map(|r| (r.models[0], r.lying));
        assert_eq!(models("Decor/Storage/Barrels/Barrel_Wood_Dark_A_1x1.webp"), Some(("wine_barrel_01", false)));
        assert_eq!(models("Decor/Storage/Barrels/Keg_Wood_Dark_E_1x1.webp"), Some(("wine_barrel_01", true)));
        assert_eq!(models("Decor/Storage/Barrels/Barrel_Side_Wood_Dark_A_1x1.webp"), Some(("wine_barrel_01", true)));
        let top = |p: &str| rule_for(&key(p)).map(|r| r.top);
        assert_eq!(top("Decor/Storage/Barrels/Filled/Barrel_Wood_Dark_Water_Clear_A1_1x1.webp"), Some(true), "вода сверху — рисунком на модели");
        assert_eq!(top("Decor/Storage/Boxes_and_Crates/Crates/Filled/Crate_Wood_Dark_Breads_A_1x1.webp"), Some(true), "хлеб сверху — рисунком");
        assert_eq!(top("Decor/Storage/Barrels/Barrel_Wood_Dark_A_1x1.webp"), Some(false));
        assert_eq!(models("Decor/Storage/Barrels/Barrel_Metal_Rusty_A_1x1.webp"), Some(("barrel_01", false)));
        assert_eq!(models("Furniture/Seating/Chairs/Broken/Chair_Broken_Wood_Dark_A_1x1.webp"), None);
        assert_eq!(models("Furniture/Seating/Chairs/Fallen/Chair_Fallen_Wood_Dark_A_1x1.webp"), None);
        assert_eq!(models("Furniture/Tables/Rectangle_Tables/Table_Rectangle_Wood_Dark_E_2x1.webp"), Some(("wooden_table_02_2", false)));
        assert_eq!(models("Furniture/Tables/Rectangle_Tables/Filled/Arranged_Tables_with_Chairs/Table_Arrangement_Wood_Ashen_Filled_A4_3x2.webp"), None);
        assert_eq!(models("Decor/Storage/Chests/Chest_Wood_Dark_A_1x1.webp"), None, "сундук открывается — остаётся картинкой");
        assert_eq!(rule_for("tint:96322a:C:\\Ассеты\\Stool_Wood_Dark_A_1x1.png").map(|r| r.models[0]), Some("wooden_stool_01"));
    }

    #[test]
    fn a_model_fills_the_drawn_part_of_the_picture_not_its_frame() {
        // Бочка 0,4 × 0,6 × 0,4 клетки в рисунке 1×1, нарисована в середине на 0,76 клетки.
        let barrel = [[-0.2, 0.0, -0.2], [0.2, 0.6, 0.2]];
        let p = place(barrel, [1.0, 1.0], [0.12, 0.12, 0.88, 0.88], false);
        assert!((p.skew - 1.0).abs() < 1e-4);
        let lo = apply(&p.matrix, barrel[0]);
        let hi = apply(&p.matrix, barrel[1]);
        assert!((lo[0] + 0.38).abs() < 1e-4 && (hi[0] - 0.38).abs() < 1e-4 && (lo[2] + 0.38).abs() < 1e-4, "{lo:?} {hi:?}");
        assert!(lo[1].abs() < 1e-5, "на полу");
        // Растяжение по сторонам 1,9: по высоте — √1,9, бочка не в рост человека.
        assert!((hi[1] - 0.6 * 1.9f32.sqrt()).abs() < 1e-3, "{}", hi[1]);
        assert!((p.top - hi[1]).abs() < 1e-4, "верх — {}", p.top);
        assert_eq!(p.frame, [[-0.5, 0.0, -0.5], [0.5, 0.0, 0.5]]);
    }

    #[test]
    fn a_long_model_turns_to_lie_along_the_picture_and_a_keg_lies_on_its_side() {
        // Ящик длиной по z, рисунок — длинный по x и сдвинут вправо-вниз.
        let crate_z = [[-0.15, 0.0, -0.3], [0.15, 0.3, 0.3]];
        let p = place(crate_z, [2.0, 1.0], [0.5, 0.5, 1.0, 1.0], false);
        assert!(p.skew < 1.01, "{}", p.skew);
        let (a, b) = (apply(&p.matrix, crate_z[0]), apply(&p.matrix, crate_z[1]));
        let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
        let (z0, z1) = (a[2].min(b[2]), a[2].max(b[2]));
        assert!((x0 - 0.0).abs() < 1e-4 && (x1 - 1.0).abs() < 1e-4, "{x0} {x1}");
        assert!((z0 - 0.0).abs() < 1e-4 && (z1 - 0.5).abs() < 1e-4, "{z0} {z1}");
        // Стоячая бочка 0,5 × 0,8 — на бок: в рисунке длиной по x, высота — её ширина.
        let barrel = [[-0.25, 0.0, -0.25], [0.25, 0.8, 0.25]];
        let k = place(barrel, [1.0, 1.0], [0.1, 0.25, 0.9, 0.75], true);
        assert!(k.skew < 1.01, "{}", k.skew);
        let top = apply(&k.matrix, [0.0, 0.8, 0.0]);
        let bottom = apply(&k.matrix, [0.0, 0.0, 0.0]);
        assert!(top[0] < bottom[0] && (top[1] - bottom[1]).abs() < 1e-4, "лежит: {top:?} {bottom:?}");
        let side = apply(&k.matrix, [0.25, 0.4, 0.0]);
        assert!(side[1] > 0.0 && (apply(&k.matrix, [-0.25, 0.4, 0.0])[1]).abs() < 1e-4, "на полу боком");
    }

    #[test]
    fn a_square_picture_takes_a_long_crate_but_not_a_long_barrel() {
        let long = [[-0.4, 0.0, -0.15], [0.4, 0.3, 0.15]];
        let skew = place(long, [1.0, 1.0], [0.05, 0.05, 0.95, 0.95], false).skew;
        let barrel = RULES.iter().find(|r| r.starts.contains(&"barrel_wood")).unwrap();
        let crates = RULES.iter().find(|r| r.starts.contains(&"crate_wood")).unwrap();
        assert!(skew > barrel.stretch && skew > crates.stretch, "{skew}: ящик в 2,7 раза длиннее — и ему не годится");
        let squat = [[-0.3, 0.0, -0.17], [0.3, 0.3, 0.17]];
        let skew = place(squat, [1.0, 1.0], [0.05, 0.05, 0.95, 0.95], false).skew;
        assert!(skew > barrel.stretch && skew <= crates.stretch, "{skew}");
    }

    #[test]
    fn the_opaque_box_skips_the_empty_margin() {
        let mut img = image::RgbaImage::new(100, 50);
        for x in 20..60 {
            for y in 10..40 {
                img.put_pixel(x, y, image::Rgba([1, 2, 3, 255]));
            }
        }
        img.put_pixel(90, 45, image::Rgba([0, 0, 0, 20]));
        assert_eq!(opaque_box(&img), Some([0.2, 0.2, 0.6, 0.8]));
        assert_eq!(opaque_box(&image::RgbaImage::new(4, 4)), None);
    }
}
