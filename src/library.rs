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
pub const BUILTIN_OBJECTS: [(&str, &str, [f32; 2], &[u8]); 34] = [
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
    ("bookshelf", "Книжный шкаф", [2.0, 1.0], include_bytes!("../assets/builtin/objects/bookshelf.webp")),
    ("chest", "Сундук", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chest.webp")),
    ("chest_open", "Открытый сундук", [1.0, 1.0], include_bytes!("../assets/builtin/objects/chest_open.webp")),
    ("barrel", "Бочка", [1.0, 1.0], include_bytes!("../assets/builtin/objects/barrel.webp")),
    ("crate", "Ящик", [1.0, 1.0], include_bytes!("../assets/builtin/objects/crate.webp")),
    ("sacks", "Мешки", [1.0, 1.0], include_bytes!("../assets/builtin/objects/sacks.webp")),
    ("rug", "Ковёр", [3.0, 2.0], include_bytes!("../assets/builtin/objects/rug.webp")),
    ("rug_round", "Круглый ковёр", [2.0, 2.0], include_bytes!("../assets/builtin/objects/rug_round.webp")),
    ("fireplace", "Камин", [2.0, 1.0], include_bytes!("../assets/builtin/objects/fireplace.webp")),
    ("cauldron", "Котёл", [1.0, 1.0], include_bytes!("../assets/builtin/objects/cauldron.webp")),
    ("anvil", "Наковальня", [1.0, 1.0], include_bytes!("../assets/builtin/objects/anvil.webp")),
    ("campfire", "Костёр", [1.0, 1.0], include_bytes!("../assets/builtin/objects/campfire.webp")),
    ("brazier", "Жаровня", [1.0, 1.0], include_bytes!("../assets/builtin/objects/brazier.webp")),
    ("weapon_rack", "Стойка с оружием", [1.0, 1.0], include_bytes!("../assets/builtin/objects/weapon_rack.webp")),
    ("altar", "Алтарь", [1.0, 1.0], include_bytes!("../assets/builtin/objects/altar.webp")),
    ("statue", "Статуя", [1.0, 1.0], include_bytes!("../assets/builtin/objects/statue.webp")),
    ("pillar", "Колонна", [1.0, 1.0], include_bytes!("../assets/builtin/objects/pillar.webp")),
    ("rubble", "Обломки", [1.0, 1.0], include_bytes!("../assets/builtin/objects/rubble.webp")),
    ("boulder", "Валун", [1.0, 1.0], include_bytes!("../assets/builtin/objects/boulder.webp")),
    ("well", "Колодец", [2.0, 2.0], include_bytes!("../assets/builtin/objects/well.webp")),
    ("plant_pot", "Растение в горшке", [1.0, 1.0], include_bytes!("../assets/builtin/objects/plant_pot.webp")),
    ("bush", "Куст", [1.0, 1.0], include_bytes!("../assets/builtin/objects/bush.webp")),
    ("tree", "Дерево", [3.0, 3.0], include_bytes!("../assets/builtin/objects/tree.webp")),
];

/// Стены, зашитые в программу: имя, подпись, толщина в клетках, байты. Картинка —
/// полоса гребня стены, бесшовная по длине, высотой «толщина × 256» с обводкой
/// по краям, как в паках FA (`tools/walls.py`).
pub const BUILTIN_WALLS: [(&str, &str, f32, &[u8]); 15] = [
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
/// Больше миниатюр в памяти не держим: давно не показанные выбрасываются.
const THUMB_BUDGET: usize = 3000;
const WORKERS: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Floor,
    Wall,
    Door,
    Object,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Floor, Kind::Wall, Kind::Door, Kind::Object];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Floor => "Пол",
            Kind::Wall => "Стены",
            Kind::Door => "Двери",
            Kind::Object => "Объекты",
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
}

impl Asset {
    fn new(key: String, inner: &str, kind: Kind) -> Self {
        let (folder, file) = inner.rsplit_once(['/', '\\']).unwrap_or(("", inner));
        let name = file.rsplit_once('.').map_or(file, |(n, _)| n).to_owned();
        let search = format!("{} {}", name, folder).to_lowercase().replace('_', " ");
        Self { key, name, folder: folder.to_owned(), kind, search }
    }

    pub fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|w| self.search.contains(w.as_str()))
    }
}

/// Размер в клетках по имени ассета FA: `Oak_Tree_A_3x3.webp` → 3×3.
pub fn size_from_name(name: &str) -> Option<[f32; 2]> {
    let tail = name.rsplit('_').next()?;
    let (w, h) = tail.split_once(['x', 'X'])?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

/// Вид ассета по папке. Для паков — по разделу Dungeondraft, для своих папок —
/// по словам в пути; всё прочее считается полом.
fn kind_of(inner: &str, from_pack: bool) -> Option<Kind> {
    let lower = inner.to_lowercase();
    if from_pack {
        let section = lower.strip_prefix("textures/")?.split('/').next()?;
        return match section {
            "terrain" | "patterns" => Some(Kind::Floor),
            "walls" => Some(Kind::Wall),
            "portals" => Some(Kind::Door),
            "objects" => Some(Kind::Object),
            _ => None,
        };
    }
    let parts: Vec<&str> = lower.split(['/', '\\']).collect();
    let has = |names: &[&str]| parts.iter().any(|p| names.contains(p));
    Some(if has(&["objects", "props", "объекты", "предметы"]) {
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
type Sources = Arc<RwLock<HashMap<String, (PathBuf, u64, u64)>>>;

fn read_source(key: &str, sources: &Sources) -> Result<Vec<u8>, String> {
    if key.starts_with(BUILTIN) {
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
}

impl Textures {
    /// Байты картинки как есть — для выгрузки, где файл нужен целиком.
    pub fn bytes(&self, key: &str) -> Result<Vec<u8>, String> {
        read_source(key, &self.sources)
    }

    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let sources: Sources = Arc::default();
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
                    let shadow = job.key.strip_prefix(SHADOW).and_then(|rest| rest.split_once(':'));
                    let result = if let Some((blur, inner)) = shadow {
                        let blur = blur.parse::<f32>().unwrap_or(3.0) / 10.0;
                        read_source(inner, &sources).and_then(|bytes| shadow_image(&bytes, blur))
                    } else {
                        match job.key.strip_prefix(VARY) {
                        Some(inner) => read_source(inner, &sources).and_then(|bytes| decode_rgba(&bytes, VARY_SIDE)).map(|(rgba, size)| {
                            let big = vary(&rgba, inner);
                            (to_color_image(&big), [size[0] * VARY_TIMES, size[1] * VARY_TIMES])
                        }),
                        None => read_source(&job.key, &sources).and_then(|bytes| decode(&bytes, if job.thumb { THUMB_SIDE } else { FULL_SIDE })),
                        }
                    };
                    if tx.send(Done { key: job.key, thumb: job.thumb, result }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self { slots: HashMap::new(), jobs, done, sources, frame: 0 }
    }

    /// Раз в кадр: забрать готовые картинки и выбросить лишние миниатюры.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
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
        std::thread::spawn(move || {
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
        let builtin = textures.chain(objects).chain(walls).chain(portals);
        self.assets = builtin.chain(self.baked.iter().cloned()).chain(self.folder.iter().cloned()).chain(self.from_packs.iter().cloned()).collect();
    }

    /// Номера ассетов нужного вида, подходящих под строку поиска.
    pub fn filter(&self, kind: Kind, query: &str) -> Vec<usize> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
        self.assets.iter().enumerate().filter(|(_, a)| a.kind == kind && a.matches(&words)).map(|(i, _)| i).collect()
    }
}

#[cfg(test)]
mod tests {
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
