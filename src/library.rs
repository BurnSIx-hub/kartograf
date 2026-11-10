//! Библиотека ассетов: картинки из папки пользователя и из паков Dungeondraft.
//!
//! Ассет называется ключом: для файла это путь к нему, для картинки из пака —
//! `pack:<файл пака>|<путь внутри>`. Ключ и пишется в карту.
//!
//! Картинки грузятся в фоновых потоках: окно не ждёт, пока прочитается и разожмётся
//! webp из пака в четыре гигабайта. Пока картинки нет, на её месте заглушка.

use crate::pck;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};

pub const IMAGE_EXTS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
/// Пикселей на клетку у ассетов Dungeondraft (и Forgotten Adventures).
pub const PX_PER_CELL: f32 = 256.0;
/// С мип-уровнями: отдалённая карта сжимает картинку в разы, и без них
/// мелкий рисунок рябит.
const TEX_OPTIONS: TextureOptions = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR_REPEAT };
const MAX_FILES: usize = 20000;
const THUMB_SIDE: u32 = 128;
const FULL_SIDE: u32 = 2048;
/// Ключ встроенной текстуры: `builtin:<имя>` (см. [`BUILTIN_TEXTURES`]).
pub const BUILTIN: &str = "builtin:";

/// Текстуры, зашитые в программу: новая карта начинается с них, и карта с ними
/// открывается на любом компьютере. Имя, подпись, нужно ли разнообразие, байты.
pub const BUILTIN_TEXTURES: [(&str, &str, bool, &[u8]); 27] = [
    ("flagstones", "Плиты", false, include_bytes!("../assets/builtin/flagstones.webp")),
    ("wood_planks", "Доски", false, include_bytes!("../assets/builtin/wood_planks.webp")),
    ("dirt", "Земля", true, include_bytes!("../assets/builtin/dirt.webp")),
    ("grass_green", "Трава", true, include_bytes!("../assets/builtin/grass_green.webp")),
    ("stone_tiles", "Плитка", false, include_bytes!("../assets/builtin/stone_tiles.webp")),
    ("grass_dry", "Сухая трава", true, include_bytes!("../assets/builtin/grass_dry.webp")),
    ("grass_frost", "Иней", true, include_bytes!("../assets/builtin/grass_frost.webp")),
    ("snow", "Снег", true, include_bytes!("../assets/builtin/snow.webp")),
    ("water_shallow", "Мелководье", true, include_bytes!("../assets/builtin/water_shallow.webp")),
    ("water_deep", "Глубина", true, include_bytes!("../assets/builtin/water_deep.webp")),
    ("cave_floor", "Пол пещеры", true, include_bytes!("../assets/builtin/cave_floor.webp")),
    ("gravel", "Гравий", true, include_bytes!("../assets/builtin/gravel.webp")),
    ("ice", "Лёд", true, include_bytes!("../assets/builtin/ice.webp")),
    ("sand", "Песок", true, include_bytes!("../assets/builtin/sand.webp")),
    ("mud", "Грязь", true, include_bytes!("../assets/builtin/mud.webp")),
    ("swamp", "Болото", true, include_bytes!("../assets/builtin/swamp.webp")),
    ("forest_floor", "Лесная подстилка", true, include_bytes!("../assets/builtin/forest_floor.webp")),
    ("moss", "Мох", true, include_bytes!("../assets/builtin/moss.webp")),
    ("rocky_ground", "Каменистая земля", true, include_bytes!("../assets/builtin/rocky_ground.webp")),
    ("ash", "Пепел", true, include_bytes!("../assets/builtin/ash.webp")),
    ("lava", "Лава", false, include_bytes!("../assets/builtin/lava.webp")),
    ("dungeon_flagstones", "Плиты подземелья", false, include_bytes!("../assets/builtin/dungeon_flagstones.webp")),
    ("dungeon_flagstones_moss", "Плиты со мхом", false, include_bytes!("../assets/builtin/dungeon_flagstones_moss.webp")),
    ("cobblestone", "Брусчатка", false, include_bytes!("../assets/builtin/cobblestone.webp")),
    ("brick_floor", "Кирпич", false, include_bytes!("../assets/builtin/brick_floor.webp")),
    ("sandstone", "Песчаник", false, include_bytes!("../assets/builtin/sandstone.webp")),
    ("marble", "Мрамор", false, include_bytes!("../assets/builtin/marble.webp")),
];

/// Объекты, зашитые в программу: имя, подпись, размер в клетках, байты. Картинка —
/// 256 точек на клетку с прозрачным полем и тенью (`tools/cutout.py`).
pub const BUILTIN_OBJECTS: [(&str, &str, [f32; 2], &[u8]); 54] = [
    ("chair", "Стул", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chair.webp")),
    ("stool", "Табурет", [1.0, 1.0], include_bytes!("../assets/builtin/objects/stool.webp")),
    ("table", "Стол", [2.0, 1.0], include_bytes!("../assets/builtin/objects/table.webp")),
    ("table_round", "Круглый стол", [2.0, 2.0], include_bytes!("../assets/builtin/objects/table_round.webp")),
    ("desk", "Письменный стол", [2.0, 1.0], include_bytes!("../assets/builtin/objects/desk.webp")),
    ("sofa", "Диван", [2.0, 1.0], include_bytes!("../assets/builtin/objects/sofa.webp")),
    ("armchair", "Кресло", [1.0, 1.0], include_bytes!("../assets/builtin/objects/armchair.webp")),
    ("bench", "Скамья", [2.0, 1.0], include_bytes!("../assets/builtin/objects/bench.webp")),
    ("bed", "Кровать", [1.0, 2.0], include_bytes!("../assets/builtin/objects/bed.webp")),
    ("bed_double", "Двуспальная кровать", [2.0, 2.0], include_bytes!("../assets/builtin/objects/bed_double.webp")),
    ("wardrobe", "Шкаф", [2.0, 1.0], include_bytes!("../assets/builtin/objects/wardrobe.webp")),
    ("bookshelf", "Книжный шкаф", [3.0, 1.0], include_bytes!("../assets/builtin/objects/bookshelf.webp")),
    ("chest", "Сундук", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chest.webp")),
    ("chest_open", "Открытый сундук", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chest_open.webp")),
    ("chest_empty", "Пустой сундук", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chest_empty.webp")),
    ("barrel", "Бочка", [1.0, 1.0], include_bytes!("../assets/builtin/objects/barrel.webp")),
    ("crate", "Ящик", [1.0, 1.0], include_bytes!("../assets/builtin/objects/crate.webp")),
    ("sacks", "Мешки", [1.0, 1.0], include_bytes!("../assets/builtin/objects/sacks.webp")),
    ("rug", "Ковёр", [3.0, 2.0], include_bytes!("../assets/builtin/objects/rug.webp")),
    ("rug_round", "Круглый ковёр", [2.0, 2.0], include_bytes!("../assets/builtin/objects/rug_round.webp")),
    ("fireplace", "Камин", [3.0, 3.0], include_bytes!("../assets/builtin/objects/fireplace.webp")),
    ("cauldron", "Котёл", [1.0, 1.0], include_bytes!("../assets/builtin/objects/cauldron.webp")),
    ("anvil", "Наковальня", [1.0, 1.0], include_bytes!("../assets/builtin/objects/anvil.webp")),
    ("campfire", "Костёр", [1.0, 1.0], include_bytes!("../assets/builtin/objects/campfire.webp")),
    ("brazier", "Жаровня", [1.0, 1.0], include_bytes!("../assets/builtin/objects/brazier.webp")),
    ("weapon_rack", "Стойка с мечами", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack.webp")),
    ("weapon_rack_spears", "Стойка с копьями", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack_spears.webp")),
    ("weapon_rack_axes", "Стойка с топорами", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack_axes.webp")),
    ("weapon_rack_maces", "Стойка с булавами", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack_maces.webp")),
    ("weapon_rack_bows", "Стойка с луками", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack_bows.webp")),
    ("weapon_rack_mixed", "Стойка с разным оружием", [2.0, 2.0], include_bytes!("../assets/builtin/objects/weapon_rack_mixed.webp")),
    ("altar", "Алтарь", [1.0, 1.0], include_bytes!("../assets/builtin/objects/altar.webp")),
    ("statue", "Статуя", [1.0, 1.0], include_bytes!("../assets/builtin/objects/statue.webp")),
    ("pillar", "Колонна", [1.0, 1.0], include_bytes!("../assets/builtin/objects/pillar.webp")),
    ("rubble", "Обломки", [1.0, 1.0], include_bytes!("../assets/builtin/objects/rubble.webp")),
    ("boulder", "Валун", [1.0, 1.0], include_bytes!("../assets/builtin/objects/boulder.webp")),
    ("well", "Колодец", [2.0, 2.0], include_bytes!("../assets/builtin/objects/well.webp")),
    ("plant_pot", "Растение в горшке", [1.0, 1.0], include_bytes!("../assets/builtin/objects/plant_pot.webp")),
    ("bush", "Куст", [1.0, 1.0], include_bytes!("../assets/builtin/objects/bush.webp")),
    ("tree", "Дерево", [3.0, 3.0], include_bytes!("../assets/builtin/objects/tree.webp")),
    ("fountain", "Фонтан", [4.0, 4.0], include_bytes!("../assets/builtin/objects/fountain.webp")),
    ("market_stall", "Лавка с красным навесом", [3.0, 3.0], include_bytes!("../assets/builtin/objects/market_stall.webp")),
    ("market_stall_blue", "Лавка с синим навесом", [3.0, 3.0], include_bytes!("../assets/builtin/objects/market_stall_blue.webp")),
    ("market_stall_ochre", "Лавка с жёлтым навесом", [3.0, 3.0], include_bytes!("../assets/builtin/objects/market_stall_ochre.webp")),
    ("cart", "Телега", [2.0, 3.0], include_bytes!("../assets/builtin/objects/cart.webp")),
    ("street_lamp", "Фонарь", [1.0, 1.0], include_bytes!("../assets/builtin/objects/street_lamp.webp")),
    ("notice_board", "Доска объявлений", [3.0, 2.0], include_bytes!("../assets/builtin/objects/notice_board.webp")),
    ("stocks", "Колодки", [2.0, 2.0], include_bytes!("../assets/builtin/objects/stocks.webp")),
    ("planter", "Ящик с цветами", [1.0, 1.0], include_bytes!("../assets/builtin/objects/planter.webp")),
    ("bar_counter", "Барная стойка", [7.0, 2.0], include_bytes!("../assets/builtin/objects/bar_counter.webp")),
    ("bridge", "Мост", [6.0, 4.0], include_bytes!("../assets/builtin/objects/bridge.webp")),
    ("stairs", "Лестница вниз", [3.0, 3.0], include_bytes!("../assets/builtin/objects/stairs.webp")),
    ("stairs_shade", "Тень спуска", [3.0, 1.0], include_bytes!("../assets/builtin/objects/stairs_shade.webp")),
    ("stump", "Пень", [3.0, 3.0], include_bytes!("../assets/builtin/objects/stump.webp")),
];

/// Стены, зашитые в программу: имя, подпись, толщина в клетках, байты. Картинка —
/// полоса гребня стены, бесшовная по длине, высотой «толщина × 256» с обводкой
/// по краям, как в паках FA (`tools/walls.py`).
pub const BUILTIN_WALLS: [(&str, &str, f32, &[u8]); 21] = [
    ("wall_stone_blocks", "Каменные блоки", 0.45, include_bytes!("../assets/builtin/walls/stone_blocks.webp")),
    ("wall_sandstone", "Песчаник", 0.45, include_bytes!("../assets/builtin/walls/sandstone.webp")),
    ("wall_brick", "Кирпич", 0.35, include_bytes!("../assets/builtin/walls/brick.webp")),
    ("wall_fieldstone", "Бутовый камень", 0.4, include_bytes!("../assets/builtin/walls/fieldstone.webp")),
    ("wall_ruined", "Руины из бута", 0.45, include_bytes!("../assets/builtin/walls/ruined.webp")),
    ("wall_ruined_blocks", "Руины из блоков", 0.45, include_bytes!("../assets/builtin/walls/ruined_blocks.webp")),
    ("wall_wood", "Доски", 0.3, include_bytes!("../assets/builtin/walls/wood.webp")),
    ("wall_palisade", "Частокол", 0.4, include_bytes!("../assets/builtin/walls/palisade.webp")),
    ("wall_metal", "Металл", 0.3, include_bytes!("../assets/builtin/walls/metal.webp")),
    ("wall_thatch", "Солома", 0.45, include_bytes!("../assets/builtin/walls/thatch.webp")),
    ("wall_timber", "Фахверк", 0.35, include_bytes!("../assets/builtin/walls/timber.webp")),
    ("wall_hedge", "Живая изгородь", 0.5, include_bytes!("../assets/builtin/walls/hedge.webp")),
    ("wall_cave", "Скала", 0.6, include_bytes!("../assets/builtin/walls/cave.webp")),
    // Из каменных блоков, `tools/races.py`.
    ("wall_dwarven", "Гномья кладка", 0.45, include_bytes!("../assets/builtin/walls/dwarven.webp")),
    ("wall_drow", "Обсидиан дроу", 0.45, include_bytes!("../assets/builtin/walls/drow.webp")),
    // Зубцы парапета из каменных блоков, `tools/crenels.py`: мерлон ~1 клетка, бойница ~0,45.
    ("wall_crenels", "Зубцы", 0.5, include_bytes!("../assets/builtin/walls/crenels.webp")),
    // Полосы рельефа, `tools/cliffs.py`: стенка обрыва, гребень уступа (кромка травой,
    // сухой травой, снегом), осыпь под обрывом.
    ("wall_cliff", "Обрыв", 1.0, include_bytes!("../assets/builtin/walls/cliff.webp")),
    ("wall_ridge", "Гребень", 1.0, include_bytes!("../assets/builtin/walls/ridge.webp")),
    ("wall_ridge_dry", "Гребень, сухая трава", 1.0, include_bytes!("../assets/builtin/walls/ridge_dry.webp")),
    ("wall_ridge_snow", "Гребень, снег", 1.0, include_bytes!("../assets/builtin/walls/ridge_snow.webp")),
    ("wall_under_cliff", "Осыпь", 0.5, include_bytes!("../assets/builtin/walls/under_cliff.webp")),
];

/// Встроенные двери, косяки и окна (`tools/portals.py`): ключ, подпись, размер холста
/// в клетках, байты. Створка — доска поперёк холста 1×1 с петлёй справа; косяк
/// `frame_*_N` — столбы для проёма в N клеток на холсте N+1; окно — подоконник 2×1.
pub const BUILTIN_PORTALS: [(&str, &str, [f32; 2], &[u8]); 11] = [
    ("door_wood", "Дверь деревянная", [1.0, 1.0], include_bytes!("../assets/builtin/portals/door_wood.webp")),
    ("door_wood_iron", "Дверь окованная", [1.0, 1.0], include_bytes!("../assets/builtin/portals/door_wood_iron.webp")),
    ("door_metal", "Дверь железная", [1.0, 1.0], include_bytes!("../assets/builtin/portals/door_metal.webp")),
    ("frame_wood_1", "Косяк деревянный", [2.0, 1.0], include_bytes!("../assets/builtin/portals/frame_wood_1.webp")),
    ("frame_wood_2", "Косяк деревянный, 2 клетки", [3.0, 1.0], include_bytes!("../assets/builtin/portals/frame_wood_2.webp")),
    ("frame_wood_3", "Косяк деревянный, 3 клетки", [4.0, 1.0], include_bytes!("../assets/builtin/portals/frame_wood_3.webp")),
    ("frame_stone_1", "Косяк каменный", [2.0, 1.0], include_bytes!("../assets/builtin/portals/frame_stone_1.webp")),
    ("frame_stone_2", "Косяк каменный, 2 клетки", [3.0, 1.0], include_bytes!("../assets/builtin/portals/frame_stone_2.webp")),
    ("frame_stone_3", "Косяк каменный, 3 клетки", [4.0, 1.0], include_bytes!("../assets/builtin/portals/frame_stone_3.webp")),
    ("window_stone", "Окно, каменный подоконник", [2.0, 1.0], include_bytes!("../assets/builtin/portals/window_stone.webp")),
    ("window_wood", "Окно, деревянный подоконник", [2.0, 1.0], include_bytes!("../assets/builtin/portals/window_wood.webp")),
];

/// Створка по умолчанию — для дверей без своей картинки, в том числе в выгрузке.
pub const DEFAULT_DOOR: &str = "builtin:door_wood";

/// Покрытия крыш (`tools/roofs.py`): бесшовные, 512 px — две клетки; ряды
/// поперёк, низ картинки — к свесу, как у узоров крыш FA.
pub const BUILTIN_ROOFS: [(&str, &str, &[u8]); 9] = [
    ("roof_clay_red", "Черепица красная", include_bytes!("../assets/builtin/roofs/roof_clay_red.webp")),
    ("roof_clay_brown", "Черепица коричневая", include_bytes!("../assets/builtin/roofs/roof_clay_brown.webp")),
    ("roof_clay_green", "Черепица зелёная", include_bytes!("../assets/builtin/roofs/roof_clay_green.webp")),
    ("roof_slate_blue", "Сланец синий", include_bytes!("../assets/builtin/roofs/roof_slate_blue.webp")),
    ("roof_slate_gray", "Сланец серый", include_bytes!("../assets/builtin/roofs/roof_slate_gray.webp")),
    ("roof_shingle", "Дранка", include_bytes!("../assets/builtin/roofs/roof_shingle.webp")),
    ("roof_thatch", "Солома", include_bytes!("../assets/builtin/roofs/roof_thatch.webp")),
    ("roof_metal", "Профнастил", include_bytes!("../assets/builtin/roofs/roof_metal.webp")),
    ("roof_copper", "Медь", include_bytes!("../assets/builtin/roofs/roof_copper.webp")),
];
pub const DEFAULT_ROOF: &str = "builtin:roof_clay_red";

/// Встроенное дерево — единственное, чья крона уходит в тайл сама; и пень под неё.
pub const TREE: &str = "builtin:tree";
pub const STUMP: &str = "builtin:stump";
/// Пень под кроной: доля размера дерева. Ствол, а не вся крона.
pub const STUMP_SHARE: f32 = 0.4;

/// Слова имени файла ассета: `Window_Sill_Wood_Ashen_A1_1x1` → window, sill, wood…
fn key_words(key: &str) -> Vec<String> {
    let file = key.rsplit(['/', '\\', '|']).next().unwrap_or(key);
    let name = file.rsplit_once('.').map_or(file, |(n, _)| n);
    name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Стекло с переплётом к подоконнику FA. Окно FA — три картинки: подоконник в разрезе
/// стены, стойки рамы по краям и само окно (`Window_Wood_Ashen_A_1x1`, среди объектов)
/// поверх. Берётся окно того же материала и цвета; проём от двух клеток — большое
/// (`Window_Large_*_2x1`). Вариант (A, B, C…) — по ключу подоконника, всегда один.
pub fn window_glass(assets: &[Asset], sill: &str, len: f32) -> Option<String> {
    let words = key_words(sill);
    if !words.iter().any(|w| w == "sill") {
        return None;
    }
    let large = len >= 1.5;
    // `Small` — форточка посередине проёма, к ней своя узкая рама.
    const SKIP: [&str; 10] = ["sill", "frame", "curtain", "curtains", "broken", "stained", "wired", "shutter", "shutters", "small"];
    let mut best: Vec<(usize, &str)> = Vec::new();
    for a in assets.iter().filter(|a| a.kind == Kind::Object) {
        let f = key_words(&a.key);
        if f.first().map(String::as_str) != Some("window") || f.iter().any(|w| SKIP.contains(&w.as_str())) || f.iter().any(|w| w == "large") != large {
            continue;
        }
        // Стекло FA — варианты одной буквой (`_E_`); `_A1_`, `_B1_` у малых окон —
        // брусок и доска-ставень, в проёме они лежали поперёк.
        if !large && !f.iter().any(|w| w.len() == 1 && w.chars().all(|c| c.is_ascii_lowercase())) {
            continue;
        }
        let score = f.iter().filter(|w| words.contains(w)).count();
        match best.first() {
            Some((s, _)) if score < *s => {}
            Some((s, _)) if score == *s => best.push((score, &a.key)),
            _ => best = vec![(score, &a.key)],
        }
    }
    if best.is_empty() {
        return None;
    }
    // У FA варианты A–B — с защёлками (смотрятся форточкой), G–N — с переплётами и
    // подставками, O–P — решётка без стекла. Чистое стекло — C–F: его, если есть.
    if !large {
        let plain: Vec<(usize, &str)> = best.iter().copied().filter(|(_, k)| key_words(k).iter().any(|w| ["c", "d", "e", "f"].contains(&w.as_str()))).collect();
        if !plain.is_empty() {
            best = plain;
        }
    }
    best.sort_by(|a, b| a.1.cmp(b.1));
    let h = sill.bytes().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32));
    Some(best[h as usize % best.len()].1.to_owned())
}

/// Встроенный косяк под ширину проёма: у них варианты на 1, 2 и 3 клетки.
/// Чужой косяк остаётся как есть.
pub fn frame_for_width(key: &str, width: f32) -> String {
    match key.strip_prefix(BUILTIN).and_then(|id| id.rsplit_once('_')) {
        Some((style, _)) if style.starts_with("frame_") => format!("{BUILTIN}{style}_{}", (width.round() as i32).clamp(1, 3)),
        _ => key.to_owned(),
    }
}

/// Подпись и байты встроенной картинки — текстуры, объекта или стены.
fn builtin(key: &str) -> Option<(&'static str, &'static [u8])> {
    let id = key.strip_prefix(BUILTIN)?;
    BUILTIN_TEXTURES
        .iter()
        .find(|t| t.0 == id)
        .map(|t| (t.1, t.3))
        .or_else(|| BUILTIN_OBJECTS.iter().find(|t| t.0 == id).map(|t| (t.1, t.3)))
        .or_else(|| BUILTIN_WALLS.iter().find(|t| t.0 == id).map(|t| (t.1, t.3)))
        .or_else(|| BUILTIN_PORTALS.iter().find(|t| t.0 == id).map(|t| (t.1, t.3)))
        .or_else(|| BUILTIN_ROOFS.iter().find(|t| t.0 == id).map(|t| (t.1, t.2)))
}

/// Встроенный ассет с таким ключом есть: текстура, объект, стена, дверь или крыша.
pub fn is_builtin_key(key: &str) -> bool {
    builtin(key).is_some()
}

fn is_cliff_path(lower: &str) -> bool {
    lower.contains("cliff") && !lower.contains("under_cliff")
}

fn is_relief_path(lower: &str) -> bool {
    ["grass_ridge_", "/bank_", "under_cliff_"].iter().any(|w| lower.contains(w))
}

/// Скала FA из паков: стена-дорожка, толщина которой — высота картинки.
pub fn is_cliff(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower.contains("|textures/paths/") && is_cliff_path(&lower)
}

/// Толщина скалы в клетках: у FA 256 точек на клетку, скалы бывают в 1–4 клетки.
pub fn cliff_width(px: [u32; 2]) -> f32 {
    (px[1] as f32 / 256.0).max(0.25)
}

/// Толщина встроенной стены в клетках — под неё нарисована полоса.
pub fn builtin_wall_width(key: &str) -> Option<f32> {
    let id = key.strip_prefix(BUILTIN)?;
    BUILTIN_WALLS.iter().find(|t| t.0 == id).map(|t| t.2)
}

/// Стены из отдельных штук (брёвна частокола): картинка — ряд из стольких
/// кругов, и программа расставляет их вдоль стены, а не тянет полосу.
pub const STAMPED_WALLS: [(&str, u32); 1] = [("wall_palisade", 8)];

pub fn builtin_wall_stamps(key: &str) -> Option<u32> {
    let id = key.strip_prefix(BUILTIN)?;
    STAMPED_WALLS.iter().find(|t| t.0 == id).map(|t| t.1)
}

/// Размер встроенного объекта в клетках — известен и до загрузки картинки.
pub fn builtin_size(key: &str) -> Option<[f32; 2]> {
    let id = key.strip_prefix(BUILTIN)?;
    BUILTIN_OBJECTS.iter().find(|t| t.0 == id).or_else(|| BUILTIN_PORTALS.iter().find(|t| t.0 == id)).map(|t| t.2)
}

/// Ключ с этой приставкой — та же картинка, но с «разнообразием» (см. [`vary`]).
pub const VARY: &str = "vary:";
/// Ключ тени объекта: `shadow:<размытие в десятых клетки>:<ключ картинки>`.
/// Вместо размера картинки слот тени отдаёт [поле, ширина силуэта] в точках —
/// по ним видно, сколько поля под размытие в долях объекта.
pub const SHADOW: &str = "shadow:";
const SHADOW_SIDE: u32 = 256;
/// Во сколько раз плитка с разнообразием больше исходной по каждой стороне.
pub const VARY_TIMES: u32 = 3;
/// Исходник для разнообразия не крупнее этого: большая плитка — втрое больше.
const VARY_SIDE: u32 = 1024;
/// Плитки, сшитые `tools/quilt.py` и разложенные `tools/masonry.py`: уже большие и без
/// повтора, а разнообразие только наложило бы их камни и листья друг на друга.
const QUILTED: [&str; 13] = ["dirt", "wood_planks", "gravel", "forest_floor", "rocky_ground", "flagstones", "dungeon_flagstones", "dungeon_flagstones_moss", "stone_tiles", "sandstone", "marble", "brick_floor", "cobblestone"];
/// Больше миниатюр в памяти не держим: давно не показанные выбрасываются.
const THUMB_BUDGET: usize = 3000;
/// Видеопамять под полные картинки, тени и разнообразный пол. Сверх неё давно не
/// показанные выгружаются (сначала самые старые) — понадобятся, раскроются снова.
/// Без этого каждый новый квартал добавлял картинки, а старые оставались.
const FULL_BUDGET: usize = 1 << 30;
/// Сколько кадров картинку не показывали, прежде чем её можно выгрузить.
const FULL_IDLE: u64 = 300;
const WORKERS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Floor,
    Wall,
    Door,
    Object,
    Roof,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Floor, Kind::Wall, Kind::Door, Kind::Object, Kind::Roof];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Floor => "Пол",
            Kind::Wall => "Стены",
            Kind::Door => "Двери",
            Kind::Object => "Объекты",
            Kind::Roof => "Крыши",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub key: String,
    pub name: String,
    /// Папка внутри библиотеки — для поиска и подсказки.
    pub folder: String,
    pub kind: Kind,
    /// Для поиска: имя и папка в нижнем регистре.
    search: String,
    /// Варианты одного предмета лежат на полке одной миниатюрой.
    pub family: String,
}

impl Asset {
    pub fn new(key: String, inner: &str, kind: Kind) -> Self {
        let (folder, file) = inner.rsplit_once(['/', '\\']).unwrap_or(("", inner));
        let name = file.rsplit_once('.').map_or(file, |(n, _)| n).to_owned();
        let search = format!("{} {}", name, folder).to_lowercase().replace('_', " ");
        let family = family_of(&key, &name, folder);
        Self { key, name, folder: folder.to_owned(), kind, search, family }
    }

    pub fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|w| self.search.contains(w.as_str()))
    }

    pub fn is_builtin(&self) -> bool {
        self.key.starts_with(BUILTIN)
    }

    /// Слова имени и папки по отдельности: `Barrel_Wood_A1_1x1` → barrel, wood, a1, 1x1.
    pub fn words(&self) -> Vec<&str> {
        self.search.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect()
    }
}

/// Ширина и высота картинки в точках без полного раскрытия — по заголовку.
pub fn dimensions(sources: &Sources, key: &str) -> Option<[u32; 2]> {
    let bytes = read_source(key, sources).ok()?;
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    Some([w, h])
}

/// Видимая часть картинки: рамка непрозрачного в долях от середины (-0.5…0.5).
/// У FA у стола вокруг прозрачные поля, и стулья, придвинутые к краю холста,
/// висели в стороне от столешницы.
pub fn visible_box(sources: &Sources, key: &str) -> Option<[f32; 4]> {
    let bytes = read_source(key, sources).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let (mut l, mut t, mut r, mut b) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 40 {
            (l, t, r, b) = (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1));
        }
    }
    (r > l && b > t).then(|| [l as f32 / w as f32 - 0.5, t as f32 / h as f32 - 0.5, r as f32 / w as f32 - 0.5, b as f32 / h as f32 - 0.5])
}

/// Концы нити (паутина): крайние непрозрачные точки вдоль главной оси рисунка,
/// доли холста от середины: `[x0, y0, x1, y1]`.
pub fn strand_ends(sources: &Sources, key: &str) -> Option<[f32; 4]> {
    let bytes = read_source(key, sources).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let step = (w.max(h) / 256).max(1);
    let pts: Vec<(f32, f32)> = img.enumerate_pixels().filter(|(x, y, p)| p[3] > 40 && x % step == 0 && y % step == 0).map(|(x, y, _)| (x as f32 + 0.5, y as f32 + 0.5)).collect();
    if pts.len() < 4 {
        return None;
    }
    let n = pts.len() as f32;
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n);
    let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
    for &(x, y) in &pts {
        let (dx, dy) = (x - mx, y - my);
        sxx += dx * dx;
        syy += dy * dy;
        sxy += dx * dy;
    }
    let a = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (c, s) = (a.cos(), a.sin());
    let proj = |p: &&(f32, f32)| (p.0 - mx) * c + (p.1 - my) * s;
    let lo = pts.iter().min_by(|p, q| proj(p).total_cmp(&proj(q)))?;
    let hi = pts.iter().max_by(|p, q| proj(p).total_cmp(&proj(q)))?;
    let f = |p: &(f32, f32)| [p.0 / w as f32 - 0.5, p.1 / h as f32 - 0.5];
    let (a, b) = (f(lo), f(hi));
    Some([a[0], a[1], b[0], b[1]])
}

/// Средняя непрозрачность картинки, 0..1 (каждая восьмая точка). У FA среди полов
/// есть накладки — редкие доски на прозрачном (`Wood_Ashen_Flooring_H`, ~0,2 при
/// ~0,8 у прочих досок): полом комнаты такая не годится, сквозь неё виден двор.
/// Замер помнится до конца работы программы: файл не меняется, а разбор долгий.
pub fn opacity(sources: &Sources, key: &str) -> Option<f32> {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Option<f32>>>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(a) = seen.lock().ok()?.get(key) {
        return *a;
    }
    let a = measure_opacity(sources, key);
    seen.lock().ok()?.insert(key.to_owned(), a);
    a
}

fn measure_opacity(sources: &Sources, key: &str) -> Option<f32> {
    let bytes = read_source(key, sources).ok()?;
    let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (mut sum, mut n) = (0u64, 0u64);
    for y in (0..img.height()).step_by(8) {
        for x in (0..img.width()).step_by(8) {
            sum += img.get_pixel(x, y)[3] as u64;
            n += 1;
        }
    }
    (n > 0).then(|| sum as f32 / n as f32 / 255.0)
}

/// Встроенные предметы с вариантами: всё, что начинается с этих имён, — одна семья.
const BUILTIN_FAMILIES: [&str; 12] = ["market_stall", "chest", "weapon_rack", "door", "window", "rug", "bed", "table", "roof_clay", "roof_slate", "frame_wood", "frame_stone"];

/// Семья ассета. У встроенных — по таблице. У паков и папок — имя без хвоста, который
/// отличает варианты: размера, номера, буквы (`Barrel_Wood_A2_1x1` → `barrel_wood`).
/// Пусто после чистки — сам по себе.
pub fn family_of(key: &str, name: &str, folder: &str) -> String {
    if let Some(id) = key.strip_prefix(BUILTIN) {
        return BUILTIN_FAMILIES.iter().find(|f| id.starts_with(**f)).map_or_else(|| id.to_owned(), |f| (*f).to_owned());
    }
    let lower = name.to_lowercase();
    let mut words: Vec<&str> = lower.split(['_', '-', ' ']).filter(|w| !w.is_empty()).collect();
    while words.last().is_some_and(|w| is_variant_word(w)) {
        words.pop();
    }
    if words.is_empty() { format!("{folder}/{lower}") } else { format!("{folder}/{}", words.join("_")) }
}

/// `2`, `a`, `a2`, `v3`, `2x1` — хвост варианта, а не часть названия.
fn is_variant_word(w: &str) -> bool {
    let digits = w.trim_start_matches(|c: char| c.is_alphabetic());
    let letters = w.chars().count() - digits.chars().count();
    let number = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    let size = w.split_once('x').is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && (a.to_owned() + b).chars().all(|c| c.is_ascii_digit()));
    size || (letters <= 1 && (number || digits.is_empty()))
}

/// Размер в клетках по имени ассета FA: `Oak_Tree_A_3x3.webp` → 3×3.
pub fn size_from_name(name: &str) -> Option<[f32; 2]> {
    let tail = name.rsplit('_').next()?;
    let (w, h) = tail.split_once(['x', 'X'])?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

/// Ассеты пака без загрузчика: для проверок подбора на настоящих паках.
#[cfg(test)]
pub fn pack_assets(file: &std::path::Path) -> Result<Vec<Asset>, String> {
    let pack = crate::pck::read_index(file)?;
    Ok(pack
        .entries
        .iter()
        .filter_map(|e| {
            let kind = kind_of(&e.path, true)?;
            let inner = e.path.strip_prefix("textures/").unwrap_or(&e.path);
            let inner = inner.split_once('/').map_or(inner, |(_, rest)| rest);
            Some(Asset::new(pack_key(&pack.file, &e.path), &format!("{}/{inner}", pack.name), kind))
        })
        .collect())
}

/// Вид ассета по папке. Для паков — по разделу Dungeondraft, для своих папок —
/// по словам в пути; всё прочее считается полом.
fn kind_of(inner: &str, from_pack: bool) -> Option<Kind> {
    let lower = inner.to_lowercase();
    if from_pack {
        let section = lower.strip_prefix("textures/")?.split('/').next()?;
        // Узоры крыш FA: черепица, сланец, дранка, солома, профнастил.
        if section == "patterns" && (lower.contains("roof") || lower.contains("corrugated")) {
            return Some(Kind::Roof);
        }
        return match section {
            "terrain" | "patterns" => Some(Kind::Floor),
            "walls" => Some(Kind::Wall),
            // Скалы FA — дорожки Dungeondraft (`paths/…Cliff…`): полоса вдоль линии,
            // как стена.
            "paths" if is_cliff_path(&lower) => Some(Kind::Wall),
            // Полосы рельефа FA: гребень уступа, берег, осыпь под обрывом.
            "paths" if is_relief_path(&lower) => Some(Kind::Wall),
            // Каменные ступени FA — тоже дорожка, но ставятся пролётом, как предмет.
            "paths" if lower.contains("/stairs_stone_") || lower.contains("/stairs_railing_stone_") => Some(Kind::Object),
            "portals" => Some(Kind::Door),
            "objects" => Some(Kind::Object),
            _ => None,
        };
    }
    let parts: Vec<&str> = lower.split(['/', '\\']).collect();
    let has = |names: &[&str]| parts.iter().any(|p| names.contains(p));
    Some(if has(&["roofs", "крыши"]) {
        Kind::Roof
    } else if has(&["objects", "props", "объекты", "предметы"]) {
        Kind::Object
    } else if has(&["walls", "стены"]) {
        Kind::Wall
    } else if has(&["doors", "portals", "двери"]) {
        Kind::Door
    } else {
        Kind::Floor
    })
}

/// Картинки в папке и её подпапках, по алфавиту.
pub fn scan(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| IMAGE_EXTS.contains(&x.to_string_lossy().to_lowercase().as_str())) {
                out.push(p);
                if out.len() >= MAX_FILES {
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

pub fn pack_key(pack: &Path, inner: &str) -> String {
    format!("pack:{}|{inner}", pack.display())
}

/// Короткое имя ассета по ключу — для подписей.
pub fn label(key: &str) -> String {
    if let Some(t) = builtin(key) {
        return t.0.to_owned();
    }
    let inner = key.rsplit_once('|').map_or(key, |(_, i)| i);
    let file = inner.rsplit(['/', '\\']).next().unwrap_or(inner);
    file.rsplit_once('.').map_or(file, |(n, _)| n).to_owned()
}

/// Где лежат байты картинки из пака.
pub type Sources = Arc<RwLock<HashMap<String, (PathBuf, u64, u64)>>>;

/// Ключ картинки встроенного набора: `set:cyberpunk/objects/Sofa_Neon_A_2x1.webp`.
pub const SET: &str = "set:";

/// Набор ассета (`cyberpunk`), если он из встроенного набора. Ассеты набора
/// идут только в свой генератор, в обычные карты не попадают.
pub fn set_of(key: &str) -> Option<&str> {
    key.strip_prefix(SET)?.split('/').next()
}

fn read_source(key: &str, sources: &Sources) -> Result<Vec<u8>, String> {
    if let Some(rel) = key.strip_prefix(SET) {
        crate::sets::SET_FILES.iter().find(|f| f.0 == rel).map(|f| f.1.to_vec()).ok_or_else(|| format!("нет картинки набора {key}"))
    } else if key.starts_with(BUILTIN) {
        builtin(key).map(|t| t.1.to_vec()).ok_or_else(|| format!("нет встроенной картинки {key}"))
    } else if key.starts_with("pack:") {
        let src = sources.read().ok().and_then(|s| s.get(key).cloned());
        let (file, offset, size) = src.ok_or("пак с этой картинкой не подключён")?;
        pck::read_bytes(&file, offset, size)
    } else {
        std::fs::read(key).map_err(|e| e.to_string())
    }
}

struct Job {
    key: String,
    thumb: bool,
}

struct Done {
    key: String,
    thumb: bool,
    result: Result<(ColorImage, [u32; 2]), String>,
}

enum Slot {
    Loading,
    Ready { tex: TextureHandle, size: [u32; 2], used: u64 },
    Failed,
}

pub struct Textures {
    slots: HashMap<(String, bool), Slot>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
    pub sources: Sources,
    frame: u64,
    /// В браузере потоков нет: заказы ждут здесь, и кадр разбирает их сам.
    #[cfg(target_arch = "wasm32")]
    queue: Receiver<Job>,
    /// Отложенные кадром заказы: миниатюры вперёд, крупные следом.
    #[cfg(target_arch = "wasm32")]
    backlog: (std::collections::VecDeque<Job>, std::collections::VecDeque<Job>),
}

/// Раскрыть заказанную картинку: тень, разнообразный пол или обычная.
fn load_job(job: &Job, sources: &Sources) -> Result<(ColorImage, [u32; 2]), String> {
    let shadow = job.key.strip_prefix(SHADOW).and_then(|rest| rest.split_once(':'));
    if let Some((blur, inner)) = shadow {
        let blur = blur.parse::<f32>().unwrap_or(3.0) / 10.0;
        read_source(inner, sources).and_then(|bytes| shadow_image(&bytes, blur))
    } else {
        match job.key.strip_prefix(VARY) {
            Some(inner) if inner.strip_prefix(BUILTIN).is_some_and(|id| QUILTED.contains(&id)) => read_source(inner, sources).and_then(|bytes| decode(&bytes, FULL_SIDE)),
            Some(inner) => read_source(inner, sources).and_then(|bytes| decode_rgba(&bytes, VARY_SIDE)).map(|(rgba, size)| {
                let big = vary(&rgba, inner);
                (to_color_image(&big), [size[0] * VARY_TIMES, size[1] * VARY_TIMES])
            }),
            None => read_source(&job.key, sources).and_then(|bytes| decode(&bytes, if job.thumb { THUMB_SIDE } else { FULL_SIDE })),
        }
    }
}

impl Textures {
    /// Байты картинки как есть — для выгрузки, где файл нужен целиком.
    pub fn bytes(&self, key: &str) -> Result<Vec<u8>, String> {
        read_source(key, &self.sources)
    }

    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<Done>();
        let sources: Sources = Arc::default();
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (done_tx, ctx);
            return Self { slots: HashMap::new(), jobs, done, sources, frame: 0, queue: job_rx, backlog: Default::default() };
        }
        #[cfg(not(target_arch = "wasm32"))]
        let job_rx = Arc::new(Mutex::new(job_rx));
        #[cfg(not(target_arch = "wasm32"))]
        for _ in 0..WORKERS {
            let (rx, tx, sources, ctx) = (job_rx.clone(), done_tx.clone(), sources.clone(), ctx.clone());
            std::thread::spawn(move || {
                loop {
                    let job = match rx.lock() {
                        Ok(r) => match r.recv() {
                            Ok(j) => j,
                            Err(_) => return,
                        },
                        Err(_) => return,
                    };
                    let result = load_job(&job, &sources);
                    if tx.send(Done { key: job.key, thumb: job.thumb, result }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        Self { slots: HashMap::new(), jobs, done, sources, frame: 0 }
    }

    /// Раз в кадр: забрать готовые картинки и выбросить лишние миниатюры.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        #[cfg(target_arch = "wasm32")]
        {
            // Картинку из пака раскрывает сам браузер (`createImageBitmap`, вне
            // основного потока и в разы быстрее декодера в wasm) — она придёт позже.
            // Остальное (тени, разнообразный пол, встроенные) — здесь, миниатюры первыми.
            while let Ok(job) = self.queue.try_recv() {
                if web_decode(&job, &self.sources) {
                    continue;
                }
                if job.thumb { self.backlog.0.push_back(job) } else { self.backlog.1.push_back(job) }
            }
            let mut busy = false;
            if let Ok(v) = crate::plat::web::call("kartografTakeDecoded", &[]) {
                for item in js_sys::Array::from(&v).iter() {
                    busy = true;
                    let get = |k: &str| js_sys::Reflect::get(&item, &k.into()).ok();
                    let key = get("key").and_then(|v| v.as_string()).unwrap_or_default();
                    let thumb = get("thumb").and_then(|v| v.as_bool()).unwrap_or(false);
                    let num = |k: &str| get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
                    let slot = match get("data").filter(|d| !d.is_undefined() && !d.is_null()) {
                        Some(d) => {
                            let px = js_sys::Uint8ClampedArray::new(&d).to_vec();
                            let (w, h) = (num("w"), num("h"));
                            if px.len() == w * h * 4 && w > 0 {
                                let image = ColorImage::from_rgba_unmultiplied([w, h], &px);
                                Slot::Ready { tex: ctx.load_texture(&key, image, TEX_OPTIONS), size: [num("ow") as u32, num("oh") as u32], used: self.frame }
                            } else {
                                Slot::Failed
                            }
                        }
                        None => Slot::Failed,
                    };
                    self.slots.insert((key, thumb), slot);
                }
            }
            // Не дольше ~12 мс за кадр, остальное — в следующих.
            let started = crate::plat::Instant::now();
            while started.elapsed().as_millis() < 12 {
                let Some(job) = self.backlog.0.pop_front().or_else(|| self.backlog.1.pop_front()) else { break };
                busy = true;
                let result = load_job(&job, &self.sources);
                self.slots.insert((job.key.clone(), job.thumb), match result {
                    Ok((image, size)) => Slot::Ready { tex: ctx.load_texture(&job.key, image, TEX_OPTIONS), size, used: self.frame },
                    Err(_) => Slot::Failed,
                });
            }
            let waiting = crate::plat::web::call("kartografDecoding", &[]).ok().and_then(|v| v.as_f64()).unwrap_or(0.0) > 0.0;
            if busy || waiting || !self.backlog.0.is_empty() || !self.backlog.1.is_empty() {
                ctx.request_repaint();
            }
        }
        while let Ok(d) = self.done.try_recv() {
            let slot = match d.result {
                Ok((image, size)) => Slot::Ready { tex: ctx.load_texture(&d.key, image, TEX_OPTIONS), size, used: self.frame },
                Err(_) => Slot::Failed,
            };
            self.slots.insert((d.key, d.thumb), slot);
        }
        let thumbs = self.slots.keys().filter(|k| k.1).count();
        if thumbs > THUMB_BUDGET {
            let cutoff = self.frame.saturating_sub(120);
            self.slots.retain(|k, s| !k.1 || !matches!(s, Slot::Ready { used, .. } if *used < cutoff));
        }
        self.evict_full();
    }

    /// Полные картинки сверх [`FULL_BUDGET`]: выгрузить давно не показанные.
    fn evict_full(&mut self) {
        // Мипмапы добавляют треть.
        let bytes = |t: &TextureHandle| t.size()[0] * t.size()[1] * 4 * 4 / 3;
        let mut total = 0;
        let mut idle = Vec::new();
        let cutoff = self.frame.saturating_sub(FULL_IDLE);
        for (k, s) in &self.slots {
            if let (false, Slot::Ready { tex, used, .. }) = (k.1, s) {
                total += bytes(tex);
                if *used < cutoff {
                    idle.push((*used, bytes(tex), k.clone()));
                }
            }
        }
        if total <= FULL_BUDGET {
            return;
        }
        idle.sort_by_key(|i| i.0);
        for (_, b, k) in idle {
            if total <= FULL_BUDGET {
                break;
            }
            self.slots.remove(&k);
            total -= b;
        }
    }

    /// Картинка и её настоящий размер в пикселях. `None` — ещё грузится или не читается.
    pub fn get(&mut self, key: &str, thumb: bool) -> Option<(&TextureHandle, [u32; 2])> {
        let id = (key.to_owned(), thumb);
        if !self.slots.contains_key(&id) {
            self.slots.insert(id.clone(), Slot::Loading);
            let _ = self.jobs.send(Job { key: key.to_owned(), thumb });
            return None;
        }
        match self.slots.get_mut(&id) {
            Some(Slot::Ready { tex, size, used }) => {
                *used = self.frame;
                Some((tex, *size))
            }
            _ => None,
        }
    }

    pub fn failed(&self, key: &str, thumb: bool) -> bool {
        matches!(self.slots.get(&(key.to_owned(), thumb)), Some(Slot::Failed))
    }

    /// Все ли запрошенные картинки уже пришли (для самопроверки).
    pub fn idle(&self) -> bool {
        !self.slots.values().any(|s| matches!(s, Slot::Loading))
    }
}

/// Отдать браузеру картинку из пака: кусок файла он раскроет сам, тень — размоет
/// (`ctx.filter`, то же размытие, что [`shadow_image`]). `false` — не из пака или
/// разнообразный пол: раскрываем здесь.
#[cfg(target_arch = "wasm32")]
fn web_decode(job: &Job, sources: &Sources) -> bool {
    use wasm_bindgen::JsValue;
    let (inner, side, blur) = match job.key.strip_prefix(SHADOW).and_then(|rest| rest.split_once(':')) {
        Some((blur, inner)) => (inner, SHADOW_SIDE, blur.parse::<f32>().unwrap_or(3.0) / 10.0),
        None => (job.key.as_str(), if job.thumb { THUMB_SIDE } else { FULL_SIDE }, 0.0),
    };
    if !inner.starts_with("pack:") {
        return false;
    }
    let Some((file, offset, size)) = sources.read().ok().and_then(|s| s.get(inner).cloned()) else { return false };
    let args = [
        JsValue::from_str(&job.key),
        JsValue::from_bool(job.thumb),
        JsValue::from_str(&file.to_string_lossy()),
        JsValue::from_f64(offset as f64),
        JsValue::from_f64(size as f64),
        JsValue::from_f64(side as f64),
        JsValue::from_f64(blur as f64),
        JsValue::from_f64(PX_PER_CELL as f64),
    ];
    // Браузер без размытия на холсте вернёт `false` — тогда тень считаем сами.
    crate::plat::web::call("kartografDecode", &args).is_ok_and(|v| v.as_bool() != Some(false))
}

fn decode(bytes: &[u8], max_side: u32) -> Result<(ColorImage, [u32; 2]), String> {
    decode_rgba(bytes, max_side).map(|(rgba, size)| (to_color_image(&rgba), size))
}

/// Картинка не крупнее `max_side` и её настоящий размер.
fn decode_rgba(bytes: &[u8], max_side: u32) -> Result<(image::RgbaImage, [u32; 2]), String> {
    let picture = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let size = [picture.width(), picture.height()];
    let picture = if size[0].max(size[1]) > max_side { picture.thumbnail(max_side, max_side) } else { picture };
    Ok((picture.to_rgba8(), size))
}

/// Тень объекта: силуэт по прозрачности, размытый тремя проходами ящика
/// (почти по Гауссу), с полями под размытие. Картинка мелкая — тень мягкая,
/// подробности ей ни к чему.
pub fn shadow_image(bytes: &[u8], blur_cells: f32) -> Result<(ColorImage, [u32; 2]), String> {
    let (small, orig) = decode_rgba(bytes, SHADOW_SIDE)?;
    let (w, h) = (small.width() as usize, small.height() as usize);
    let ppc = w as f32 / (orig[0] as f32 / PX_PER_CELL).max(1e-3);
    let r = ((blur_cells * ppc / 2.0).round() as usize).clamp(1, 64);
    let pad = 3 * r + 1;
    let (pw, ph) = (w + 2 * pad, h + 2 * pad);
    let mut a = vec![0f32; pw * ph];
    for (x, y, p) in small.enumerate_pixels() {
        a[(y as usize + pad) * pw + x as usize + pad] = p.0[3] as f32;
    }
    let mut tmp = vec![0f32; pw * ph];
    for _ in 0..3 {
        box_pass(&a, &mut tmp, pw, ph, r, true);
        box_pass(&tmp, &mut a, pw, ph, r, false);
    }
    let rgba: Vec<u8> = a.iter().flat_map(|&v| [0, 0, 0, v.round().clamp(0.0, 255.0) as u8]).collect();
    Ok((ColorImage::from_rgba_unmultiplied([pw, ph], &rgba), [pad as u32, w as u32]))
}

/// Скользящее среднее по строкам или столбцам, окно 2r+1, за краем — пусто.
fn box_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: usize, rows: bool) {
    let (n, lines) = if rows { (w, h) } else { (h, w) };
    let at = |line: usize, k: usize| if rows { line * w + k } else { k * w + line };
    let norm = 1.0 / (2 * r + 1) as f32;
    for line in 0..lines {
        let mut sum = 0.0;
        for k in 0..=r.min(n - 1) {
            sum += src[at(line, k)];
        }
        for k in 0..n {
            dst[at(line, k)] = sum * norm;
            if k + r + 1 < n {
                sum += src[at(line, k + r + 1)];
            }
            if k >= r {
                sum -= src[at(line, k - r)];
            }
        }
    }
}

fn to_color_image(rgba: &image::RgbaImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

/// «Разнообразие» против заметного повтора плитки. Из бесшовной картинки собирается
/// плитка в [`VARY_TIMES`] раз больше: сетка 6×6 кусков по полплитки, у каждого свой
/// случайный сдвиг и, через раз, поворот на пол-оборота. Переходы между кусками
/// узкие и мягкие, смешивание сохраняет разброс (Heitz — Neyret), чтобы на стыках
/// рисунок не мутнел. Сетка кусков замкнута, поэтому большая плитка тоже бесшовна.
/// Случайность — от ключа ассета: одна картинка всегда раскладывается одинаково.
/// Поворот только на пол-оборота: он не меняет направление линий, и волны снега
/// или ряби не ложатся крест-накрест. Доски и кладку разнообразие всё равно ломает —
/// в переходах швы двоятся; оно для травы, земли, воды, снега.
pub fn vary(src: &image::RgbaImage, seed: &str) -> image::RgbaImage {
    const PATCHES: i64 = 6;
    let (w, h) = (src.width() as i64, src.height() as i64);
    let (cw, ch) = (w * VARY_TIMES as i64, h * VARY_TIMES as i64);
    if w == 0 || h == 0 {
        return src.clone();
    }
    let mut rng = crate::generate::Rng::new(seed);
    let patches: Vec<(i64, i64, bool)> = (0..PATCHES * PATCHES)
        .map(|_| ((rng.random() * w as f64) as i64, (rng.random() * h as f64) as i64, rng.chance(0.5)))
        .collect();
    let mut mean = [0f64; 4];
    for p in src.pixels() {
        for c in 0..4 {
            mean[c] += p[c] as f64;
        }
    }
    let mean = mean.map(|m| (m / (w * h) as f64) as f32);
    // Доля соседнего куска на отрезке между центрами: переход — треть промежутка.
    let share = |t: f32| {
        let t = ((t - 0.5) * 3.0 + 0.5).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let along = |v: i64, step: f32| {
        let u = (v as f32 + 0.5) / step - 0.5;
        let i = u.floor();
        let s = share(u - i);
        [(i as i64, 1.0 - s), (i as i64 + 1, s)]
    };
    let (pw, ph) = (cw as f32 / PATCHES as f32, ch as f32 / PATCHES as f32);
    let mut out = image::RgbaImage::new(cw as u32, ch as u32);
    for y in 0..ch {
        let ys = along(y, ph);
        for x in 0..cw {
            let xs = along(x, pw);
            let (mut sum, mut sq, mut alpha) = ([0f32; 3], 0f32, 0f32);
            for &(j, wy) in &ys {
                for &(i, wx) in &xs {
                    let wgt = wx * wy;
                    if wgt < 1e-4 {
                        continue;
                    }
                    let (ox, oy, turned) = patches[(j.rem_euclid(PATCHES) * PATCHES + i.rem_euclid(PATCHES)) as usize];
                    let (a, b) = if turned { (-x, -y) } else { (x, y) };
                    let p = src.get_pixel((a + ox).rem_euclid(w) as u32, (b + oy).rem_euclid(h) as u32);
                    for c in 0..3 {
                        sum[c] += wgt * (p[c] as f32 - mean[c]);
                    }
                    alpha += wgt * p[3] as f32;
                    sq += wgt * wgt;
                }
            }
            let k = 1.0 / sq.sqrt().max(1e-4);
            let px = [0, 1, 2].map(|c| (mean[c] + sum[c] * k).round().clamp(0.0, 255.0) as u8);
            out.put_pixel(x as u32, y as u32, image::Rgba([px[0], px[1], px[2], alpha.round().clamp(0.0, 255.0) as u8]));
        }
    }
    out
}

/// Все ассеты: из папки и из паков. Оглавления паков читаются в фоне.
pub struct Library {
    pub assets: Vec<Asset>,
    pub packs: Vec<(PathBuf, Result<usize, String>)>,
    pending: usize,
    tx: Sender<(PathBuf, Result<pck::Pack, String>)>,
    rx: Receiver<(PathBuf, Result<pck::Pack, String>)>,
    /// Ассеты из папки — отдельно, чтобы смена папки не трогала паки.
    folder: Vec<Asset>,
    from_packs: Vec<Asset>,
    /// Объединённые из нескольких объектов (`App::bake`) — папка в данных программы.
    baked: Vec<Asset>,
}

impl Default for Library {
    fn default() -> Self {
        let (tx, rx) = channel();
        let mut library = Self { assets: Vec::new(), packs: Vec::new(), pending: 0, tx, rx, folder: Vec::new(), from_packs: Vec::new(), baked: Vec::new() };
        library.rebuild();
        library
    }
}

impl Library {
    pub fn set_folder(&mut self, dir: &Path) {
        self.folder = scan(dir)
            .into_iter()
            .filter_map(|p| {
                let inner = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().into_owned();
                let kind = kind_of(&inner, false)?;
                Some(Asset::new(p.to_string_lossy().into_owned(), &inner, kind))
            })
            .collect();
        self.rebuild();
    }

    /// Папка объединённых объектов: всё в ней — объекты, под папкой «Запечённые».
    pub fn set_baked(&mut self, dir: &Path) {
        self.baked = scan(dir)
            .into_iter()
            .map(|p| {
                let inner = p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().into_owned();
                Asset::new(p.to_string_lossy().into_owned(), &format!("Запечённые/{inner}"), Kind::Object)
            })
            .collect();
        self.rebuild();
    }

    pub fn add_pack(&mut self, file: PathBuf) {
        if self.packs.iter().any(|(f, _)| *f == file) {
            return;
        }
        self.packs.push((file.clone(), Ok(0)));
        self.pending += 1;
        let tx = self.tx.clone();
        crate::plat::spawn(move || {
            let r = pck::read_index(&file);
            let _ = tx.send((file, r));
        });
    }

    pub fn loading(&self) -> bool {
        self.pending > 0
    }

    /// Забрать прочитанные оглавления. `true` — список ассетов изменился.
    pub fn poll(&mut self, textures: &Textures) -> bool {
        let mut changed = false;
        while let Ok((file, result)) = self.rx.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            let status = match result {
                Ok(pack) => {
                    let mut sources = textures.sources.write().unwrap_or_else(|e| e.into_inner());
                    let before = self.from_packs.len();
                    for e in &pack.entries {
                        let Some(kind) = kind_of(&e.path, true) else { continue };
                        let key = pack_key(&pack.file, &e.path);
                        sources.insert(key.clone(), (pack.file.clone(), e.offset, e.size));
                        let inner = e.path.strip_prefix("textures/").unwrap_or(&e.path);
                        let inner = inner.split_once('/').map_or(inner, |(_, rest)| rest);
                        self.from_packs.push(Asset::new(key, &format!("{}/{inner}", pack.name), kind));
                    }
                    Ok(self.from_packs.len() - before)
                }
                Err(e) => Err(e),
            };
            if let Some(slot) = self.packs.iter_mut().find(|(f, _)| *f == file) {
                slot.1 = status;
            }
            changed = true;
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    fn rebuild(&mut self) {
        let textures = BUILTIN_TEXTURES.iter().map(|t| Asset::new(format!("{BUILTIN}{}", t.0), &format!("Встроенные/{}", t.1), Kind::Floor));
        let objects = BUILTIN_OBJECTS.iter().map(|t| Asset::new(format!("{BUILTIN}{}", t.0), &format!("Встроенные/{}", t.1), Kind::Object));
        let walls = BUILTIN_WALLS.iter().map(|t| Asset::new(format!("{BUILTIN}{}", t.0), &format!("Встроенные/{}", t.1), Kind::Wall));
        let portals = BUILTIN_PORTALS.iter().map(|t| Asset::new(format!("{BUILTIN}{}", t.0), &format!("Встроенные/{}", t.1), Kind::Door));
        let roofs = BUILTIN_ROOFS.iter().map(|t| Asset::new(format!("{BUILTIN}{}", t.0), &format!("Встроенные/{}", t.1), Kind::Roof));
        // Встроенные наборы — как своя папка: «Киберпанк/objects/…».
        let sets = crate::sets::SET_FILES.iter().filter_map(|(rel, _)| {
            let kind = kind_of(rel, false)?;
            let shown = rel.strip_prefix("cyberpunk/").map_or_else(|| (*rel).to_owned(), |r| format!("Киберпанк/{r}"));
            Some(Asset::new(format!("{SET}{rel}"), &shown, kind))
        });
        let builtin = textures.chain(objects).chain(walls).chain(portals).chain(roofs).chain(sets);
        self.assets = builtin.chain(self.baked.iter().cloned()).chain(self.folder.iter().cloned()).chain(self.from_packs.iter().cloned()).collect();
    }

    /// Номера ассетов нужного вида, подходящих под строку поиска.
    pub fn filter(&self, kind: Kind, query: &str) -> Vec<usize> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
        self.assets.iter().enumerate().filter(|(_, a)| a.kind == kind && a.matches(&words)).map(|(i, _)| i).collect()
    }

    /// Найденное по семьям: порядок — по первому представителю семьи.
    pub fn group(&self, found: &[usize]) -> Vec<Vec<usize>> {
        let mut at: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for &i in found {
            match at.entry(self.assets[i].family.as_str()) {
                std::collections::hash_map::Entry::Occupied(e) => groups[*e.get()].push(i),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(groups.len());
                    groups.push(vec![i]);
                }
            }
        }
        groups
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn fa_sill_gets_its_glass() {
        let a = |path: &str, kind: Kind| Asset::new(format!("pack:fa.dungeondraft_pack|textures/{path}"), &format!("FA/{path}"), kind);
        let assets = vec![
            a("objects/Structures/Windows/Window_Wood_Ashen_A_1x1.webp", Kind::Object),
            a("objects/Structures/Windows/Window_Wood_Dark_A_1x1.webp", Kind::Object),
            a("objects/Structures/Windows/Large_Windows/Window_Large_Wood_Ashen_A_2x1.webp", Kind::Object),
            a("objects/Structures/Windows/Stained_Windows/Stained_Window_Wood_Ashen_A_1x1.webp", Kind::Object),
            a("objects/Structures/Windows/Window_Sills/Frames/Window_Frame_Wood_Ashen_A_1x1.webp", Kind::Object),
            a("portals/Windows/Window_Sills/Window_Sill_Wood_Ashen_A1_1x1.webp", Kind::Door),
        ];
        let sill = &assets[5].key;
        assert_eq!(window_glass(&assets, sill, 1.0).as_deref(), Some(assets[0].key.as_str()));
        assert_eq!(window_glass(&assets, sill, 2.0).as_deref(), Some(assets[2].key.as_str()));
        assert_eq!(window_glass(&assets, "builtin:window_wood", 1.0), None);
    }
    #[test]
    fn variants_fall_into_one_family() {
        let fam = |name: &str| super::family_of("pack:x", name, "objects");
        assert_eq!(fam("Barrel_Wood_A2_1x1"), fam("Barrel_Wood_B_1x1"));
        assert_eq!(fam("Barrel_Wood_3"), fam("Barrel_Wood_B_1x1"));
        assert_ne!(fam("Barrel_Wood_A"), fam("Barrel_Iron_A"));
        assert_eq!(super::family_of("builtin:chest_empty", "", ""), "chest");
        assert_eq!(super::family_of("builtin:chest", "", ""), "chest");
        assert_eq!(super::family_of("builtin:barrel", "", ""), "barrel");
        let lib = super::Library::default();
        let found = lib.filter(super::Kind::Object, "сундук");
        assert_eq!(lib.group(&found).len(), 1);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn builtin_objects_decode_with_transparency_and_their_size() {
        for (id, _, size, bytes) in super::BUILTIN_OBJECTS {
            let img = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("{id}: {e}")).to_rgba8();
            // Встроенные объекты нарезаны по 512 px на клетку, вдвое крупнее паков:
            // их можно увеличивать без мыла. Размер в клетках у них из таблицы.
            let cells = [img.width() as f32 / 512.0, img.height() as f32 / 512.0];
            assert_eq!(cells, size, "{id}: размер в таблице не совпал с картинкой");
            assert_eq!(img.get_pixel(0, 0)[3], 0, "{id}: угол должен быть прозрачным");
            assert_eq!(super::builtin_size(&format!("{}{id}", super::BUILTIN)), Some(size));
        }
    }

    #[test]
    fn builtin_portals_match_their_size_and_frames_follow_width() {
        for (id, _, size, bytes) in super::BUILTIN_PORTALS {
            let img = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("{id}: {e}")).to_rgba8();
            assert_eq!([img.width() as f32 / 512.0, img.height() as f32 / 512.0], size, "{id}");
            assert_eq!(img.get_pixel(0, 0)[3], 0, "{id}: угол прозрачен");
        }
        let b = super::BUILTIN;
        assert_eq!(super::frame_for_width(&format!("{b}frame_stone_1"), 2.0), format!("{b}frame_stone_2"));
        assert_eq!(super::frame_for_width(&format!("{b}frame_wood_3"), 1.0), format!("{b}frame_wood_1"));
        assert_eq!(super::frame_for_width(&format!("{b}frame_wood_1"), 6.0), format!("{b}frame_wood_3"));
        assert_eq!(super::frame_for_width("pack:x|Door_Frame_Wood_2x1.webp", 2.0), "pack:x|Door_Frame_Wood_2x1.webp");
        assert_eq!(super::builtin_size(&format!("{b}frame_wood_2")), Some([3.0, 1.0]));
    }

    #[test]
    fn builtin_walls_are_strips_of_their_thickness() {
        for (id, _, width, bytes) in super::BUILTIN_WALLS {
            let img = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("{id}: {e}")).to_rgba8();
            assert_eq!(super::builtin_wall_width(&format!("{}{id}", super::BUILTIN)), Some(width));
            if let Some(n) = super::builtin_wall_stamps(&format!("{}{id}", super::BUILTIN)) {
                assert_eq!(img.width(), img.height() * n, "{id}: ряд из {n} квадратов");
                let c = img.height() / 2;
                assert_eq!(img.get_pixel(c, c)[3], 255, "{id}: середина круга непрозрачна");
                assert_eq!(img.get_pixel(0, 0)[3], 0, "{id}: угол квадрата прозрачен");
                continue;
            }
            let px = (width * super::PX_PER_CELL).round() as u32;
            assert_eq!(img.height(), px, "{id}: высота полосы — толщина × 256");
            assert!(img.width() > img.height() * 3, "{id}: полоса должна быть длинной");
            // Гребень и осыпь — полупрозрачные полосы рельефа: тень откоса тает книзу.
            if ["wall_ridge", "wall_ridge_dry", "wall_ridge_snow", "wall_under_cliff"].contains(&id) {
                continue;
            }
            let mid = img.height() / 2;
            assert_eq!(img.get_pixel(0, mid)[3], 255, "{id}: середина полосы непрозрачна");
            assert_eq!(super::builtin_wall_width(&format!("{}{id}", super::BUILTIN)), Some(width));
        }
    }

    #[test]
    fn vary_is_bigger_seamless_and_not_a_plain_repeat() {
        // Бесшовная «шумная» картинка 64×64.
        let src = image::RgbaImage::from_fn(64, 64, |x, y| {
            let v = ((x as f32 * 0.3).sin() * (y as f32 * 0.2).cos() * 60.0 + ((x * 7 + y * 13) % 17) as f32 * 4.0 + 100.0) as u8;
            image::Rgba([v, v / 2, 255 - v, 255])
        });
        let big = vary(&src, "проверка");
        assert_eq!(big.dimensions(), (192, 192));
        let d = |a: &image::Rgba<u8>, b: &image::Rgba<u8>| (a[0] as i32 - b[0] as i32).abs();
        let inner: f32 = (0..191).map(|x| d(big.get_pixel(x, 50), big.get_pixel(x + 1, 50)) as f32).sum::<f32>() / 191.0;
        let seam: f32 = (0..192).map(|y| d(big.get_pixel(0, y), big.get_pixel(191, y)) as f32).sum::<f32>() / 192.0;
        assert!(seam < inner * 2.0, "шов {seam}, внутри {inner}");
        let same = (0..128).filter(|&x| big.get_pixel(x, 40) == big.get_pixel(x + 64, 40)).count();
        assert!(same < 100, "через исходную ширину рисунок не должен повторяться: совпало {same}");
        assert_eq!(vary(&src, "проверка"), big, "раскладка повторима");
    }

    use super::*;

    #[test]
    fn shadow_is_blurred_silhouette_with_margin() {
        // Квадрат 2×2 клетки: непрозрачная середина, прозрачные края.
        let mut img = image::RgbaImage::new(512, 512);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let inside = (128..384).contains(&x) && (128..384).contains(&y);
            *p = image::Rgba([200, 30, 30, if inside { 255 } else { 0 }]);
        }
        let mut bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let (shadow, [pad, w]) = shadow_image(&bytes, 0.3).unwrap();
        assert_eq!(w, 256);
        assert_eq!(shadow.size, [256 + 2 * pad as usize, 256 + 2 * pad as usize]);
        let at = |x: usize, y: usize| shadow.pixels[y * shadow.size[0] + x];
        let c = shadow.size[0] / 2;
        // Середина тёмная и чёрная, угол поля пустой, край силуэта — полутень.
        assert!(at(c, c).a() > 250 && at(c, c).r() == 0);
        assert_eq!(at(0, 0).a(), 0);
        let edge = at(pad as usize + 64, c).a();
        assert!((90..170).contains(&edge), "{edge}");
    }

    #[test]
    fn size_and_kind_from_names() {
        assert_eq!(size_from_name("Oak_Tree_A_3x2"), Some([3.0, 2.0]));
        assert_eq!(size_from_name("Barrel"), None);
        assert_eq!(kind_of("textures/objects/Trees/Oak_2x2.webp", true), Some(Kind::Object));
        assert_eq!(kind_of("textures/portals/Doors/Door_1x1.webp", true), Some(Kind::Door));
        assert_eq!(kind_of("textures/terrain/Grass/Grass_A.webp", true), Some(Kind::Floor));
        assert_eq!(kind_of("textures/lights/Candle.png", true), None);
        // Скалы FA — дорожки, но в библиотеке это стены; тень под скалой и прочие дорожки — нет.
        assert_eq!(kind_of("textures/paths/Natural_Decor_Paths/Cliffs_Paths/Cliff_Stone_Earthy_A1.webp", true), Some(Kind::Wall));
        // Осыпь под обрывом — полоса рельефа, как и гребень.
        assert_eq!(kind_of("textures/paths/Cliff_Paths/Under_Cliff_Paths/Under_Cliff_A1.webp", true), Some(Kind::Wall));
        assert_eq!(kind_of("textures/paths/Natural_Decor_Paths/Cliffs_Paths/Grass_Ridge_Dirt_01_A1.webp", true), Some(Kind::Wall));
        assert_eq!(kind_of("textures/paths/Sand_Paths/Sand_Path_A.webp", true), None);
        assert_eq!(kind_of("textures/paths/Cable_Paths/Cable_Metal_Brass_A_Path.webp", true), None);
        assert!(is_cliff("pack:fa.dungeondraft_pack|textures/paths/Cliff_Paths/Cliff_Ice_A1.webp"));
        assert!(!is_cliff("pack:fa.dungeondraft_pack|textures/walls/Walls_Stone/Wall_Stone_A.webp"));
        assert_eq!(cliff_width([768, 512]), 2.0);
        assert_eq!(kind_of("Мои/Объекты/бочка.png", false), Some(Kind::Object));
        assert_eq!(kind_of("плитка.png", false), Some(Kind::Floor));
    }

    #[test]
    fn search_by_words_in_name_and_folder() {
        let a = Asset::new("k".into(), "FA/Furniture/Tables/Table_Round_Wood_2x2.webp", Kind::Object);
        assert!(a.matches(&["round".into(), "tables".into()]));
        assert!(!a.matches(&["chair".into()]));
        assert_eq!(a.name, "Table_Round_Wood_2x2");
        assert_eq!(label(&pack_key(Path::new("C:/p.dungeondraft_pack"), "textures/objects/X/Bed_2x1.png")), "Bed_2x1");
    }
}
