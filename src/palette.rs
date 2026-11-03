//! Чем строить: встроенными ассетами или своими из библиотеки (папка и паки).
//!
//! Генераторы просят ассеты по слотам — встроенным именам: `chair`, `grass_green`,
//! `wall_timber`, `roof_clay_red`, `door_wood`. У каждого слота список слов, по
//! которым в библиотеке ищется замена целыми словами имени и папки: слот `bed`
//! найдёт `Bed_Wooden_A_1x2`, но не `Bedroll`. Нашлось несколько — каждый раз
//! берётся случайный, повторимо по зерну. Не нашлось — встроенный.
//!
//! Размер своего объекта в клетках — по имени (`_2x2`), иначе по картинке
//! (256 точек на клетку). Косяки дверей всегда встроенные: они подгоняются под
//! ширину проёма.

use crate::generate::Rng;
use crate::library::{self, Asset, Kind, PX_PER_CELL};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// Слот, слова-альтернативы (каждая — все слова должны быть) и слова-исключения.
type Slot = (&'static str, &'static [&'static [&'static str]], &'static [&'static str]);

const FLOORS: &[Slot] = &[
    ("flagstones", &[&["flagstone"], &["flagstones"], &["плиты"]], &["dungeon", "moss"]),
    ("wood_planks", &[&["wood", "planks"], &["wooden", "planks"], &["planks"], &["wood", "floor"], &["доски"]], &["wall"]),
    ("dirt", &[&["dirt"], &["earth"], &["soil"], &["земля"]], &["road", "path"]),
    ("grass_green", &[&["grass"], &["трава"]], &["dry", "dead", "yellow", "frost", "snow", "burnt"]),
    ("stone_tiles", &[&["tiles"], &["tile"], &["плитка"]], &["roof", "mosaic"]),
    ("grass_dry", &[&["grass", "dry"], &["grass", "dead"], &["grass", "yellow"], &["сухая", "трава"]], &[]),
    ("grass_frost", &[&["grass", "frost"], &["frost"], &["иней"]], &[]),
    ("snow", &[&["snow"], &["снег"]], &["roof"]),
    ("water_shallow", &[&["water", "shallow"], &["shallow"], &["water"], &["мелководье"], &["вода"]], &["deep", "fall", "lily"]),
    ("water_deep", &[&["water", "deep"], &["deep"], &["глубина"]], &["fall"]),
    ("cave_floor", &[&["cave"], &["пещера"]], &["wall"]),
    ("gravel", &[&["gravel"], &["pebbles"], &["гравий"]], &[]),
    ("ice", &[&["ice"], &["лёд"], &["лед"]], &["cream"]),
    ("sand", &[&["sand"], &["песок"]], &["stone", "sandstone"]),
    ("mud", &[&["mud"], &["грязь"]], &[]),
    ("swamp", &[&["swamp"], &["bog"], &["marsh"], &["болото"]], &[]),
    ("forest_floor", &[&["forest", "floor"], &["leaves"], &["подстилка"]], &[]),
    ("moss", &[&["moss"], &["мох"]], &["stone", "flagstone", "wall"]),
    ("rocky_ground", &[&["rock"], &["rocky"], &["stone", "ground"], &["камни"]], &["wall", "lava"]),
    ("ash", &[&["ash"], &["пепел"]], &["tree"]),
    ("lava", &[&["lava"], &["magma"], &["лава"]], &[]),
    ("dungeon_flagstones", &[&["dungeon"], &["flagstone", "dark"], &["подземелье"]], &["moss"]),
    ("dungeon_flagstones_moss", &[&["flagstone", "moss"], &["dungeon", "moss"], &["мох", "плиты"]], &[]),
    ("cobblestone", &[&["cobblestone"], &["cobble"], &["cobbles"], &["брусчатка"]], &["wall"]),
    ("brick_floor", &[&["brick"], &["bricks"], &["кирпич"]], &["wall"]),
    ("sandstone", &[&["sandstone"], &["песчаник"]], &["wall"]),
    ("marble", &[&["marble"], &["мрамор"]], &["wall"]),
];

const OBJECTS: &[Slot] = &[
    ("chair", &[&["chair"], &["стул"]], &["arm", "armchair"]),
    ("stool", &[&["stool"], &["табурет"]], &[]),
    ("table", &[&["table"], &["стол"]], &["round", "side", "nightstand", "end", "круглый"]),
    ("table_round", &[&["table", "round"], &["стол", "круглый"]], &[]),
    ("desk", &[&["desk"], &["writing", "table"]], &[]),
    ("sofa", &[&["sofa"], &["couch"], &["диван"]], &[]),
    ("armchair", &[&["armchair"], &["chair", "arm"], &["кресло"]], &[]),
    ("bench", &[&["bench"], &["скамья"]], &["work", "workbench"]),
    ("bed", &[&["bed"], &["кровать"]], &["double", "large", "king", "bunk", "flower", "двуспальная"]),
    ("bed_double", &[&["bed", "double"], &["bed", "large"], &["bed", "king"], &["кровать", "двуспальная"]], &[]),
    ("wardrobe", &[&["wardrobe"], &["closet"], &["armoire"], &["шкаф"]], &["book"]),
    ("bookshelf", &[&["bookshelf"], &["bookcase"], &["book", "shelf"], &["shelf"], &["books"]], &[]),
    ("chest", &[&["chest"], &["сундук"]], &["open", "empty", "drawers", "открытый", "пустой"]),
    ("chest_open", &[&["chest", "open"], &["сундук", "открытый"]], &["empty"]),
    ("chest_empty", &[&["chest", "empty"], &["сундук", "пустой"]], &[]),
    ("barrel", &[&["barrel"], &["бочка"]], &["broken"]),
    ("crate", &[&["crate"], &["box"], &["ящик"]], &["broken", "flower", "planter"]),
    ("sacks", &[&["sack"], &["sacks"], &["bag"], &["мешки"], &["мешок"]], &["sleeping"]),
    ("rug", &[&["rug"], &["carpet"], &["ковёр"], &["ковер"]], &["round", "круглый"]),
    ("rug_round", &[&["rug", "round"], &["carpet", "round"], &["ковёр", "круглый"]], &[]),
    ("fireplace", &[&["fireplace"], &["hearth"], &["камин"]], &[]),
    ("cauldron", &[&["cauldron"], &["pot", "cooking"], &["котёл"], &["котел"]], &["plant"]),
    ("anvil", &[&["anvil"], &["наковальня"]], &[]),
    ("campfire", &[&["campfire"], &["bonfire"], &["костёр"], &["костер"]], &[]),
    ("brazier", &[&["brazier"], &["жаровня"]], &[]),
    ("weapon_rack", &[&["weapon", "rack"], &["sword", "rack"], &["стойка", "мечами"]], &["spear", "axe", "mace", "bow"]),
    ("weapon_rack_spears", &[&["spear", "rack"], &["spears", "rack"], &["копьями"]], &[]),
    ("weapon_rack_axes", &[&["axe", "rack"], &["axes", "rack"], &["топорами"]], &[]),
    ("weapon_rack_maces", &[&["mace", "rack"], &["maces", "rack"], &["булавами"]], &[]),
    ("weapon_rack_bows", &[&["bow", "rack"], &["bows", "rack"], &["луками"]], &[]),
    ("weapon_rack_mixed", &[&["weapon", "rack"], &["weapons", "rack"]], &[]),
    ("altar", &[&["altar"], &["алтарь"]], &[]),
    ("statue", &[&["statue"], &["статуя"]], &["broken"]),
    ("pillar", &[&["pillar"], &["column"], &["колонна"]], &["broken", "ruined"]),
    ("rubble", &[&["rubble"], &["debris"], &["обломки"]], &["wall"]),
    ("boulder", &[&["boulder"], &["rock"], &["валун"]], &["wall", "ground", "floor", "small"]),
    ("well", &[&["well"], &["колодец"]], &[]),
    ("plant_pot", &[&["plant", "pot"], &["potted"], &["plant"], &["растение"]], &["wall"]),
    ("bush", &[&["bush"], &["shrub"], &["куст"]], &["berry", "dead"]),
    ("tree", &[&["tree"], &["oak"], &["pine"], &["дерево"]], &["dead", "stump", "fallen", "log", "palm"]),
    ("fountain", &[&["fountain"], &["фонтан"]], &[]),
    ("market_stall", &[&["stall"], &["market"], &["лавка"]], &[]),
    ("market_stall_blue", &[&["stall", "blue"], &["market", "blue"]], &[]),
    ("market_stall_ochre", &[&["stall", "yellow"], &["stall", "ochre"], &["market", "yellow"]], &[]),
    ("cart", &[&["cart"], &["wagon"], &["телега"]], &["broken"]),
    ("street_lamp", &[&["lamp"], &["lantern"], &["streetlight"], &["фонарь"]], &["table", "oil", "hanging"]),
    ("notice_board", &[&["notice", "board"], &["board"], &["sign"], &["доска"]], &["chess", "cutting", "game"]),
    ("stocks", &[&["stocks"], &["pillory"], &["колодки"]], &[]),
    ("planter", &[&["planter"], &["flower", "box"], &["flowers"], &["цветы"]], &[]),
    ("bar_counter", &[&["bar", "counter"], &["counter"], &["bar"], &["стойка"]], &["weapon", "оружие"]),
    ("bridge", &[&["bridge"], &["мост"]], &[]),
    ("stairs", &[&["stairs"], &["staircase"], &["stair"], &["лестница"]], &["spiral"]),
    ("stump", &[&["stump"], &["пень"]], &[]),
];

const WALLS: &[Slot] = &[
    ("wall_stone_blocks", &[&["stone", "block"], &["stone", "blocks"], &["stone"], &["камень"]], &["field", "rubble", "ruin", "ruined", "cave", "rough"]),
    ("wall_sandstone", &[&["sandstone"], &["песчаник"]], &[]),
    ("wall_brick", &[&["brick"], &["bricks"], &["кирпич"]], &[]),
    ("wall_fieldstone", &[&["fieldstone"], &["field", "stone"], &["rubble"], &["rough", "stone"], &["бут"]], &["ruin", "ruined"]),
    ("wall_ruined", &[&["ruined"], &["ruin"], &["руины"]], &["block", "blocks"]),
    ("wall_ruined_blocks", &[&["ruined", "block"], &["ruined", "blocks"], &["ruin", "block"]], &[]),
    ("wall_wood", &[&["wood"], &["wooden"], &["plank"], &["planks"], &["log"], &["дерево"], &["доски"]], &["timber", "palisade"]),
    ("wall_palisade", &[&["palisade"], &["частокол"]], &[]),
    ("wall_metal", &[&["metal"], &["iron"], &["steel"], &["металл"]], &[]),
    ("wall_thatch", &[&["thatch"], &["straw"], &["солома"]], &[]),
    ("wall_timber", &[&["timber"], &["halftimber"], &["half", "timber"], &["tudor"], &["фахверк"]], &[]),
    ("wall_hedge", &[&["hedge"], &["изгородь"]], &[]),
    ("wall_cave", &[&["cave"], &["rock"], &["скала"]], &[]),
    ("wall_dwarven", &[&["dwarven"], &["dwarf"], &["гномья"]], &[]),
    ("wall_drow", &[&["drow"], &["obsidian"], &["обсидиан"]], &[]),
];

const PORTALS: &[Slot] = &[
    ("door_wood", &[&["door"], &["дверь"]], &["frame", "double", "window", "arch", "gate", "secret", "trap", "trapdoor", "косяк"]),
    ("door_wood_iron", &[&["door", "iron"], &["door", "reinforced"], &["дверь", "окованная"]], &["frame", "double"]),
    ("door_metal", &[&["door", "metal"], &["door", "steel"], &["дверь", "железная"]], &["frame", "double"]),
    ("window_stone", &[&["window"], &["окно"]], &["frame", "glass", "stained", "broken"]),
    ("window_wood", &[&["window", "wood"], &["window", "wooden"], &["window"]], &["frame", "glass", "stained", "broken"]),
];

const ROOFS: &[Slot] = &[
    ("roof_clay_red", &[&["clay", "red"], &["tile", "red"], &["tiles", "red"], &["terracotta"], &["clay"], &["черепица"]], &[]),
    ("roof_clay_brown", &[&["clay", "brown"], &["tile", "brown"], &["clay"], &["черепица"]], &[]),
    ("roof_clay_green", &[&["clay", "green"], &["tile", "green"], &["clay"], &["черепица"]], &[]),
    ("roof_slate_blue", &[&["slate", "blue"], &["slate"], &["сланец"]], &[]),
    ("roof_slate_gray", &[&["slate", "gray"], &["slate", "grey"], &["slate"], &["сланец"]], &[]),
    ("roof_shingle", &[&["shingle"], &["shingles"], &["wood"], &["дранка"]], &[]),
    ("roof_thatch", &[&["thatch"], &["straw"], &["солома"]], &[]),
    ("roof_metal", &[&["corrugated"], &["metal"], &["профнастил"]], &["copper"]),
    ("roof_copper", &[&["copper"], &["медь"]], &[]),
];

/// Выбранный под слот ассет: ключ и размер в клетках. `own` — из библиотеки.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub key: String,
    pub size: [f32; 2],
    pub own: bool,
}

/// Чем мерить картинку, когда размер не в имени: ширина и высота в точках.
pub type Measure = Box<dyn Fn(&str) -> Option<[u32; 2]>>;

pub struct Palette {
    /// Слот → ключи и имена подходящих ассетов, по алфавиту ключей.
    slots: BTreeMap<&'static str, Vec<(String, String)>>,
    measure: Measure,
    sizes: RefCell<HashMap<String, [f32; 2]>>,
    rng: RefCell<Rng>,
}

fn table_of(kind: Kind) -> &'static [Slot] {
    match kind {
        Kind::Floor => FLOORS,
        Kind::Object => OBJECTS,
        Kind::Wall => WALLS,
        Kind::Door => PORTALS,
        Kind::Roof => ROOFS,
    }
}

impl Palette {
    /// Всё встроенное.
    pub fn builtin() -> Self {
        Self { slots: BTreeMap::new(), measure: Box::new(|_| None), sizes: RefCell::default(), rng: RefCell::new(Rng::new("")) }
    }

    /// Замены из библиотеки: только не встроенные ассеты. Слот крыши без своих
    /// совпадений берёт любую крышу из библиотеки.
    pub fn from_library(assets: &[Asset], measure: Measure) -> Self {
        let mut slots: BTreeMap<&'static str, Vec<(String, String)>> = BTreeMap::new();
        let mut roofs = Vec::new();
        for a in assets.iter().filter(|a| !a.is_builtin()) {
            let words = a.words();
            let has = |w: &str| words.contains(&w);
            for (slot, alternatives, none) in table_of(a.kind) {
                if alternatives.iter().any(|alt| alt.iter().all(|w| has(w))) && !none.iter().any(|w| has(w)) {
                    slots.entry(slot).or_default().push((a.key.clone(), a.name.clone()));
                }
            }
            if a.kind == Kind::Roof {
                roofs.push((a.key.clone(), a.name.clone()));
            }
        }
        if !roofs.is_empty() {
            for (slot, _, _) in ROOFS {
                slots.entry(slot).or_insert_with(|| roofs.clone());
            }
        }
        for v in slots.values_mut() {
            v.sort();
            v.dedup();
        }
        Self { slots, measure, sizes: RefCell::default(), rng: RefCell::new(Rng::new("")) }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Случайность выбора вариантов: то же зерно — те же варианты.
    pub fn seed(&self, seed: &str) {
        *self.rng.borrow_mut() = Rng::new(&format!("{seed}/assets"));
    }

    /// Встроенный ассет слота — когда библиотека не в деле.
    pub fn builtin_pick(slot: &str) -> Pick {
        let key = format!("{}{slot}", library::BUILTIN);
        Pick { size: library::builtin_size(&key).unwrap_or([1.0, 1.0]), key, own: false }
    }

    /// Ассет под слот: свой, если нашёлся, иначе встроенный.
    pub fn pick(&self, slot: &str) -> Pick {
        let Some(found) = self.slots.get(slot).filter(|v| !v.is_empty()) else { return Self::builtin_pick(slot) };
        let i = if found.len() > 1 { self.rng.borrow_mut().int(0, found.len() as i32 - 1) as usize } else { 0 };
        let (key, name) = &found[i];
        let size = *self.sizes.borrow_mut().entry(key.clone()).or_insert_with(|| {
            library::size_from_name(name)
                .filter(|s| s[0] > 0.0 && s[1] > 0.0)
                .or_else(|| (self.measure)(key).map(|px| [(px[0] as f32 / PX_PER_CELL).max(0.25), (px[1] as f32 / PX_PER_CELL).max(0.25)]))
                .unwrap_or([1.0, 1.0])
        });
        Pick { key: key.clone(), size, own: true }
    }

    /// Ключ — один из ассетов слота (свой или встроенный).
    pub fn fits(&self, slot: &str, key: &str) -> bool {
        key == format!("{}{slot}", library::BUILTIN) || self.slots.get(slot).is_some_and(|v| v.iter().any(|(k, _)| k == key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str, kind: Kind) -> Asset {
        Asset::new(format!("pack:fa.dungeondraft_pack|textures/x/{path}"), &format!("FA/{path}"), kind)
    }

    #[test]
    fn whole_words_match_and_variants_rotate() {
        let assets = vec![
            asset("Furniture/Bed_Wooden_A_1x2.webp", Kind::Object),
            asset("Furniture/Bedroll_A_1x2.webp", Kind::Object),
            asset("Furniture/Bed_Double_B_2x2.webp", Kind::Object),
            asset("Nature/Oak_Tree_A_3x3.webp", Kind::Object),
            asset("Nature/Oak_Tree_B_4x4.webp", Kind::Object),
            asset("Terrain/Grass_Green_A.webp", Kind::Floor),
            asset("Terrain/Grass_Dry_B.webp", Kind::Floor),
            asset("Walls/Wall_Timber_A.webp", Kind::Wall),
            asset("Patterns/Roof_Slate_Blue.webp", Kind::Roof),
            Asset::new(format!("{}chair", library::BUILTIN), "Встроенные/Стул", Kind::Object),
        ];
        let p = Palette::from_library(&assets, Box::new(|_| Some([512, 768])));
        p.seed("x");
        let bed = p.pick("bed");
        assert!(bed.own && bed.key.contains("Bed_Wooden") && bed.size == [1.0, 2.0], "{bed:?}");
        assert!(p.pick("bed_double").key.contains("Bed_Double"));
        // Встроенный стул не считается своим: слот без замены — встроенный.
        assert_eq!(p.pick("chair"), Pick { key: "builtin:chair".into(), size: [1.0, 1.0], own: false });
        assert_eq!(p.pick("grass_green").key, assets[5].key);
        assert_eq!(p.pick("grass_dry").key, assets[6].key);
        assert_eq!(p.pick("wall_timber").key, assets[7].key);
        // Любая крыша закрывает все слоты крыш.
        assert_eq!(p.pick("roof_thatch").key, assets[8].key);
        let picks: std::collections::HashSet<String> = (0..20).map(|_| p.pick("tree").key).collect();
        assert_eq!(picks.len(), 2);
        assert!(p.fits("tree", &assets[3].key) && p.fits("tree", "builtin:tree") && !p.fits("tree", &assets[0].key));
        // Повторимость по зерну.
        let seq = |s: &str| {
            p.seed(s);
            (0..6).map(|_| p.pick("tree").key).collect::<Vec<_>>()
        };
        assert_eq!(seq("а"), seq("а"));
        assert!(Palette::builtin().is_empty());
    }

    #[test]
    fn size_comes_from_the_picture_when_the_name_is_silent() {
        let assets = vec![asset("Furniture/Chair.webp", Kind::Object)];
        let p = Palette::from_library(&assets, Box::new(|_| Some([512, 256])));
        assert_eq!(p.pick("chair").size, [2.0, 1.0]);
    }
}
