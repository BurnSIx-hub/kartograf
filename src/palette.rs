//! Чем строить: встроенными ассетами или своими из библиотеки (папка и паки).
//!
//! Генераторы просят ассеты по слотам — встроенным именам: `chair`, `grass_green`,
//! `wall_timber`, `roof_clay_red`, `door_wood`. У каждого слота список слов, по
//! которым в библиотеке ищется замена. Первое слово — то, что предмет есть: у FA
//! оно первым и стоит в имени файла (`Table_Rectangle_…`, `Fancy_Chair_…`), поэтому
//! `Coffee_Table`, `Barricade` и всё из папки «Tables» столом не считаются. Остальные
//! слова — где угодно в имени, исключения — в имени и в папке. Слот `bed` найдёт
//! `Bed_Wooden_A_1x2`, но не `Bedroll`. Нашлось несколько — каждый раз берётся
//! случайный, повторимо по зерну. Не нашлось — встроенный.
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
    ("wood_planks", &[&["wood", "planks"], &["wood", "flooring"], &["wooden", "planks"], &["planks"], &["wood", "floor"], &["доски"]], &["wall"]),
    ("dirt", &[&["dirt"], &["earth"], &["soil"], &["земля"]], &["road", "path"]),
    ("grass_green", &[&["grass"], &["трава"]], &["dry", "dead", "yellow", "frost", "snow", "burnt"]),
    ("stone_tiles", &[&["tiles"], &["tile"], &["плитка"]], &["roof", "mosaic"]),
    ("grass_dry", &[&["grass", "dry"], &["grass", "dead"], &["grass", "yellow"], &["сухая", "трава"]], &[]),
    // У FA иней — `Frosted_Grass`, `Grassy_Snow`; снег с песком (`Snow_Sand`) выглядит
    // булыжником — не берётся.
    ("grass_frost", &[&["grass", "frost"], &["frost"], &["frosted", "grass"], &["grassy", "snow"], &["иней"]], &[]),
    ("snow", &[&["snow"], &["снег"]], &["roof", "sand", "disturbed"]),
    ("water_shallow", &[&["water", "shallow"], &["shallow"], &["water"], &["мелководье"], &["вода"]], &["deep", "fall", "lily"]),
    ("water_deep", &[&["water", "deep"], &["deep"], &["глубина"]], &["fall"]),
    ("cave_floor", &[&["cave"], &["пещера"]], &["wall"]),
    // Полы пещер FA по породе: `Cave_Floor_02_Slate`, `_01_Earthy`, `_03_Sandstone`, `_05_Volcanic`.
    ("cave_slate", &[&["cave", "slate"]], &["wall"]),
    ("cave_earthy", &[&["cave", "earthy"]], &["wall"]),
    ("cave_sandstone", &[&["cave", "sandstone"]], &["wall"]),
    ("cave_volcanic", &[&["cave", "volcanic"]], &["wall"]),
    ("gravel", &[&["gravel"], &["pebbles"], &["гравий"]], &[]),
    ("ice", &[&["ice"], &["лёд"], &["лед"]], &["cream"]),
    ("sand", &[&["sand"], &["песок"]], &["stone", "sandstone"]),
    // У FA грязь — `Wet_Mud_*`: первое слово «wet».
    ("mud", &[&["mud"], &["wet", "mud"], &["грязь"]], &[]),
    ("swamp", &[&["swamp"], &["bog"], &["marsh"], &["болото"]], &[]),
    ("swamp_wet", &[&["marsh", "wet"], &["swamp", "wet"], &["топь"]], &[]),
    ("forest_floor", &[&["forest", "floor"], &["leaves"], &["подстилка"]], &[]),
    ("moss", &[&["moss"], &["мох"]], &["stone", "flagstone", "wall"]),
    ("rocky_ground", &[&["rock"], &["rocky"], &["stone", "ground"], &["камни"]], &["wall", "lava"]),
    ("ash", &[&["ash"], &["пепел"]], &["tree"]),
    ("lava", &[&["lava"], &["magma"], &["лава"]], &[]),
    ("dungeon_flagstones", &[&["dungeon"], &["flagstone", "dark"], &["подземелье"]], &["moss"]),
    // Пол подземелья по виду — только камень, в тёмных тонах (сланец, вулканический);
    // у храма мрамор и узорная плитка, у руин треснувшая и мшистая.
    // Киберпанк: металлические плиты; сетка — под живой неон.
    ("cy_floor", &[&["floor", "alloy", "grid"], &["floor", "alloy", "panels"], &["floor", "alloy", "dark"]], &[]),
    ("crypt_floor", &[&["large", "flagstone", "slate"], &["large", "flagstone", "volcanic"], &["flat", "stone", "floor", "slate"], &["flat", "stone", "floor", "volcanic"], &["stone", "square", "tiles", "slate"]], &["snowy", "dirt", "cracked", "rotated"]),
    ("prison_floor", &[&["rock", "tiles", "slate"], &["rock", "tiles", "volcanic"], &["cobblestone", "square", "slate"], &["cobblestone", "slate"], &["cobblestone", "volcanic"]], &["snowy", "mossy", "rotated", "dirt"]),
    ("temple_floor", &[&["marble", "tiles"], &["stone", "patterned", "tiles", "sandstone"], &["stone", "hexagonal", "tiles", "sandstone"], &["stone", "patterned", "tiles", "slate"]], &["cracked"]),
    ("ruin_floor", &[&["stone", "tiles", "cracked", "slate"], &["stone", "tiles", "cracked", "earthy"], &["rock", "tiles", "mossy"]], &["snowy"]),
    ("dungeon_flagstones_moss", &[&["flagstone", "moss"], &["dungeon", "moss"], &["мох", "плиты"]], &[]),
    ("cobblestone", &[&["cobblestone"], &["cobble"], &["cobbles"], &["брусчатка"]], &["wall"]),
    ("brick_floor", &[&["brick"], &["bricks"], &["кирпич"]], &["wall"]),
    ("sandstone", &[&["sandstone"], &["песчаник"]], &["wall"]),
    ("marble", &[&["marble"], &["мрамор"]], &["wall"]),
    // Колизей: тёплый песок арены (без серого и белого), крупный — пятнами;
    // ряды — каменные блоки, проходы — большие плиты, подиум — белый мрамор.
    ("arena_sand", &[&["sand", "02"], &["sand", "04"], &["sand", "05"]], &[]),
    ("arena_sand_rough", &[&["desert", "pebble", "sand", "yellow"], &["desert", "clumped", "sand", "yellow"]], &[]),
    ("arena_tiers_sand", &[&["stone", "tiles", "sandstone"]], &["cracked", "hexagonal", "diagonal", "square", "patterned"]),
    ("arena_tiers_slate", &[&["stone", "tiles", "slate"], &["stone", "tiles", "earthy"]], &["cracked", "hexagonal", "diagonal", "square", "patterned"]),
    ("arena_walk_sand", &[&["flagstone", "large", "sandstone"]], &[]),
    ("arena_walk_slate", &[&["flagstone", "large", "slate"]], &[]),
    ("arena_marble", &[&["marble", "tiles", "white"]], &["cracked"]),
];

/// Полы комнат: сквозь них не должно быть видно двора (см. [`Palette::solid`]).
const INDOOR_FLOORS: &[&str] = &["wood_planks", "flagstones", "stone_tiles", "dungeon_flagstones", "dungeon_flagstones_moss", "cobblestone", "brick_floor", "sandstone", "marble", "crypt_floor", "prison_floor", "temple_floor", "ruin_floor"];

const OBJECTS: &[Slot] = &[
    ("chair", &[&["chair"], &["стул"]], &["arm", "armchair"]),
    ("stool", &[&["stool"], &["табурет"]], &[]),
    ("table", &[&["table"], &["стол"]], &["round", "side", "nightstand", "end", "runner", "leg", "cover", "cloth", "knife", "misc", "tray", "triangle", "halfcircle", "poligon", "круглый"]),
    ("table_round", &[&["table", "round"], &["стол", "круглый"]], &[]),
    ("desk", &[&["desk"], &["writing", "table"]], &[]),
    ("sofa", &[&["sofa"], &["couch"], &["диван"]], &[]),
    ("armchair", &[&["armchair"], &["chair", "arm"], &["кресло"]], &[]),
    // Простая деревянная: обитые кожей, школьные, пристенные и прочие — не то.
    ("bench", &[&["bench"], &["скамья"]], &["work", "workbench", "fancy", "ceremorph", "piano", "charpai", "school", "wall", "base", "stone", "metal", "snowy", "frosty"]),
    // Каменная — для площади и улицы.
    ("bench_stone", &[&["bench", "stone"]], &["wall", "base", "damaged", "fallen"]),
    ("bed", &[&["bed"], &["кровать"]], &["double", "large", "king", "bunk", "flower", "двуспальная"]),
    ("bed_double", &[&["bed", "double"], &["bed", "large"], &["bed", "king"], &["bed", "arranged"], &["кровать", "двуспальная"]], &[]),
    ("wardrobe", &[&["wardrobe"], &["closet"], &["armoire"], &["шкаф"]], &["book"]),
    ("bookshelf", &[&["bookshelf"], &["bookcase"], &["book", "shelf"], &["shelf"], &["books"]], &[]),
    ("chest", &[&["chest"], &["сундук"]], &["open", "empty", "drawers", "открытый", "пустой"]),
    // Киберпанк — только свой набор (имена из него).
    ("cy_bar", &[&["bar", "counter", "neon"]], &[]),
    ("cy_sofa", &[&["sofa", "neon"]], &[]),
    ("cy_arcade", &[&["arcade", "cabinet"]], &[]),
    ("cy_server", &[&["server", "rack"]], &[]),
    ("cy_terminal", &[&["terminal", "console"]], &[]),
    ("cy_holo_table", &[&["holo", "table"]], &[]),
    ("cy_holo", &[&["holo", "projector"]], &[]),
    ("cy_cryo", &[&["cryo", "pod"]], &[]),
    ("cy_med_bed", &[&["medical", "bed"]], &[]),
    ("cy_surgery", &[&["surgery", "chair", "cyber"]], &[]),
    ("cy_workbench", &[&["workbench", "robotics"]], &[]),
    ("cy_hovercar", &[&["hover", "car"]], &[]),
    ("cy_moto", &[&["motorcycle", "neon"]], &[]),
    ("cy_lockers", &[&["lockers", "row"]], &[]),
    ("cy_desk", &[&["desk", "glass"]], &[]),
    ("cy_chair", &[&["chair", "office", "neon"]], &[]),
    ("cy_vending", &[&["vending", "machine", "neon"]], &[]),
    ("cy_noodle", &[&["noodle", "stall"]], &[]),
    ("cy_crate", &[&["crate", "tech"]], &[]),
    ("cy_trash", &[&["trash", "bin", "tech"]], &[]),
    ("cy_planter", &[&["planter", "hydroponic"]], &[]),
    ("cy_reactor", &[&["reactor", "core"]], &[]),
    ("cy_turret", &[&["turret", "security"]], &[]),
    ("cy_drone", &[&["drone", "broken"]], &[]),
    ("cy_android", &[&["android", "wreck"]], &[]),
    ("cy_lamp", &[&["neon", "lamp"]], &[]),
    ("cy_hatch", &[&["floor", "hatch"]], &[]),
    ("cy_vent", &[&["floor", "vent"]], &[]),
    ("cy_cables", &[&["cables", "floor"]], &[]),
    ("cy_partition", &[&["glass", "partition"]], &[]),
    // Сундук с сокровищем FA: у него есть открытый с монетами и пустой — в Foundry
    // он открывается (Item Piles).
    ("treasure_chest", &[&["treasure", "chest"]], &["coins", "empty", "false", "bottom", "open"]),
    ("chest_open", &[&["chest", "open"], &["сундук", "открытый"]], &["empty"]),
    ("chest_empty", &[&["chest", "empty"], &["сундук", "пустой"]], &[]),
    // Бочки с надписью «TNT» — не для таверны и не для улицы.
    ("barrel", &[&["barrel"], &["бочка"]], &["broken", "tnt"]),
    ("crate", &[&["crate"], &["ящик"]], &["broken", "flower", "planter"]),
    ("sacks", &[&["sack"], &["sacks"], &["bag"], &["мешки"], &["мешок"]], &["sleeping"]),
    ("rug", &[&["rug"], &["carpet"], &["ковёр"], &["ковер"]], &["round", "круглый"]),
    ("rug_round", &[&["rug", "round"], &["carpet", "round"], &["ковёр", "круглый"]], &[]),
    ("fireplace", &[&["fireplace"], &["hearth"], &["камин"]], &["accessories", "holder", "tool", "tools", "chimneyless"]),
    ("cauldron", &[&["cauldron"], &["pot", "cooking"], &["котёл"], &["котел"]], &["plant"]),
    // Подставка и сапожная наковальня — не то: на подставке FA одно бревно, рисованное мелко.
    ("anvil", &[&["anvil"], &["наковальня"]], &["stand", "small", "cobbler"]),
    // Подземелье. Встроенных нет: без библиотеки ставится запасное.
    // Закрытые крышкой, целые; открытая коробка без крышки читается как ящик.
    ("sarcophagus", &[&["sarcophagus", "lid"]], &["steps", "base", "dwarven", "double", "broken", "side"]),
    ("coffin", &[&["coffin"]], &["lid"]),
    // Груды костей 2×2; скелеты и черепа FA в разном масштабе — не берутся.
    ("bones", &[&["bones", "arranged"]], &["beast", "bloody", "tarrasque"]),
    // Самостоятельная статуя-фигура: без крыльев (это насадки), бюстов, статуэток и шаров.
    ("statue_figure", &[&["statue", "hooded", "figure"], &["statue", "standing"], &["statue", "death"], &["statue", "sitting", "lion"]], &["wings", "bust", "pointing", "fallen", "broken"]),
    ("cobweb", &[&["cobweb"]], &[]),
    ("candles", &[&["candles", "arranged"]], &[]),
    ("chains", &[&["hanging", "chain"]], &["sign", "pet"]),
    ("cage", &[&["cage", "metal"]], &["bird", "door", "top", "bed", "ceremorph", "small", "chain"]),
    ("pillar_stone", &[&["pillar", "stone"], &["pillar", "solid"], &["pillar", "marble"]], &["broken", "wood", "lamp", "railing", "torment", "drow", "dwarven", "palisade", "awning", "overlay", "metal", "brickwood", "stonewood", "blue", "green", "purple", "red", "white", "yellow"]),
    ("pillar_broken", &[&["pillar", "broken"]], &[]),
    ("campfire", &[&["campfire"], &["bonfire"], &["костёр"], &["костер"]], &[]),
    // Волшебная «повелевающая элементалями» и упавшие — не для обстановки.
    ("brazier", &[&["brazier"], &["жаровня"]], &["commanding", "elementals", "fallen"]),
    ("weapon_rack", &[&["weapon", "rack"], &["sword", "rack"], &["стойка", "мечами"]], &["spear", "axe", "mace", "bow"]),
    ("weapon_rack_spears", &[&["spear", "rack"], &["spears", "rack"], &["копьями"]], &[]),
    ("weapon_rack_axes", &[&["axe", "rack"], &["axes", "rack"], &["топорами"]], &[]),
    ("weapon_rack_maces", &[&["mace", "rack"], &["maces", "rack"], &["булавами"]], &[]),
    ("weapon_rack_bows", &[&["bow", "rack"], &["bows", "rack"], &["луками"]], &[]),
    ("weapon_rack_mixed", &[&["weapon", "rack"], &["weapons", "rack"]], &[]),
    ("altar", &[&["altar"], &["алтарь"]], &[]),
    ("statue", &[&["statue"], &["статуя"]], &["broken"]),
    ("pillar", &[&["pillar"], &["column"], &["колонна"]], &["broken", "ruined"]),
    ("rubble", &[&["rubble", "pile"], &["rubble"], &["debris"], &["обломки"]], &["wall"]),
    ("boulder", &[&["boulder"], &["rock"], &["валун"]], &["wall", "ground", "floor", "small"]),
    ("well", &[&["well"], &["колодец"]], &[]),
    ("plant_pot", &[&["plant", "pot"], &["potted"], &["plant"], &["растение"]], &["wall"]),
    ("bush", &[&["bush"], &["shrub"], &["куст"]], &["berry", "dead"]),
    ("bush_summer", &[&["bush", "green"], &["bush", "multicolor1"]], &["berry", "dead"]),
    ("bush_spring", &[&["berry", "bush", "green"], &["bush", "green"]], &["dead"]),
    ("bush_autumn", &[&["bush", "red"], &["bush", "yellow"]], &["berry", "dead"]),
    ("bush_magic", &[&["bush", "multicolor2"], &["bush", "multicolor3"]], &["berry", "dead"]),
    ("tree", &[&["tree"], &["oak"], &["pine"], &["дерево"]], &["dead", "stump", "fallen", "log", "palm"]),
    // Времена года по цветам крон FA: Multicolor1 — оливковая зелень, 2 и 3 — бирюза с сиренью.
    ("tree_summer", &[&["tree", "green"], &["tree", "multicolor1"], &["pine", "green"]], &["dead", "stump", "fallen", "log", "palm", "shadow"]),
    ("tree_spring", &[&["tree", "green"], &["pine", "green"]], &["dead", "stump", "fallen", "log", "palm", "shadow"]),
    ("tree_autumn", &[&["tree", "red"], &["tree", "yellow"], &["pine", "orange"]], &["dead", "stump", "fallen", "log", "palm", "shadow"]),
    ("tree_winter", &[&["pine", "green"], &["pine", "blue"]], &["dead", "stump", "fallen", "log", "palm", "shadow"]),
    ("tree_magic", &[&["tree", "multicolor2"], &["tree", "multicolor3"], &["pine", "blue"]], &["dead", "stump", "fallen", "log", "palm", "shadow"]),
    ("tree_bare", &[&["bare", "tree"]], &["shadow", "branch", "fallen"]),
    // Фонтаны FA — части под сборку: половинки «Cut», настенные, струи. Целого нет,
    // поэтому фонтан всегда встроенный.
    ("fountain", &[&["fountain"], &["фонтан"]], &["cut", "wall", "arched", "feature", "spill", "spout", "water", "statue", "empty"]),
    // Торговое место FA — навес с товаром под ним одной картинкой.
    ("market_stall", &[&["stall"], &["market"], &["лавка"], &["tarps", "arranged"]], &["overlay"]),
    // Бочка с чистой водой во двор (у FA колодец сверху — тёмная вода с мусором).
    ("water_barrel", &[&["barrel", "water", "clear"]], &["rusty"]),
    // Прилавок лавки: витрина с товаром; без паков — стойка.
    ("shop_counter", &[&["display", "case", "filled"]], &["broken", "top"]),
    ("market_stall_blue", &[&["stall", "blue"], &["market", "blue"]], &[]),
    ("market_stall_ochre", &[&["stall", "yellow"], &["stall", "ochre"], &["market", "yellow"]], &[]),
    ("cart", &[&["cart"], &["wagon"], &["телега"]], &["broken"]),
    // Только уличный столб: у FA «Lamp» — и настольные, и ручные фонари, и бра.
    // Только фонарный столб: «Lamp_Street» FA сверху — металлическая перекладина,
    // на улице читается как кран; ручные лампы и бра — тем более.
    ("street_lamp", &[&["lamp", "pillar"], &["lamp", "post"], &["streetlight"], &["фонарь"]], &["table", "oil", "hanging", "wall", "light", "double"]),
    ("notice_board", &[&["notice", "board"], &["board"], &["sign"], &["доска"]], &["chess", "cutting", "game"]),
    ("stocks", &[&["stocks"], &["pillory"], &["колодки"]], &[]),
    ("planter", &[&["planter"], &["flower", "box"], &["flowers"], &["цветы"]], &[]),
    ("bar_counter", &[&["bar", "counter"], &["counter"], &["bar"], &["стойка"]], &["weapon", "оружие"]),
    // Готовые сценки FA: стол со стульями и едой, утварь на столешницу, полки с
    // добром, груды ящиков и бочек. Встроенных нет — без паков их не ставят.
    ("dining_set", &[&["table", "arrangement", "filled"], &["table", "round", "arrangement", "filled"]], &["fancy"]),
    ("table_clutter", &[&["table", "arrangement"]], &["filled", "wood", "round"]),
    ("shelf_full", &[&["shelf", "filled"], &["shelf", "arrangement"], &["cupboard", "arrangement"]], &["corner"]),
    ("storage_pile", &[&["storage", "arrangement"]], &[]),
    ("hay", &[&["hay", "bale"]], &["bundle"]),
    ("firewood_pile", &[&["firewood", "pile"], &["firewood", "log", "pile"]], &[]),
    ("trough", &[&["trough", "wood"]], &["wine"]),
    // Квартал: горн кузни, печь пекарни, нужник во дворе, бельё на верёвке.
    ("forge", &[&["forge", "lit"], &["forge"]], &["chimney", "tool", "accessor"]),
    ("oven", &[&["oven", "brick"], &["oven"]], &["shovel"]),
    ("privy", &[&["toilet", "wood"]], &["lid"]),
    ("clothes_line", &[&["clothes", "line"]], &["colorable"]),
    // Колизей. Своих нет у встроенных — без библиотеки этого просто не будет.
    ("throne", &[&["throne", "leather", "red"]], &[]),
    ("arena_brazier", &[&["brazier", "orange", "lit"], &["brazier", "red", "lit"]], &[]),
    ("arena_statue", &[&["statue", "sitting", "lion"]], &["volcanic"]),
    ("arena_rug", &[&["rug", "red"]], &["round", "circle", "oval", "runner", "1x1", "1x2", "2x1"]),
    ("arena_obelisk", &[&["statue", "sandstone", "obelisk"], &["statue", "marble", "white", "obelisk"]], &[]),
    ("arena_barricade", &[&["barricade", "spike"]], &["bilateral"]),
    ("arena_grate", &[&["torment", "floor", "grate", "circle", "3x3"]], &["damaged", "bloody", "rusty"]),
    ("arena_trapdoor", &[&["torment", "floor", "grate", "square", "2x2"]], &["bloody", "rusty"]),
    ("arena_cage", &[&["cage", "metal", "gray", "2x2"]], &["broken", "fallen", "top", "chain"]),
    ("arena_bones", &[&["beast", "bone", "white", "pile"]], &[]),
    ("arena_blood", &[&["blood", "splatter"], &["blood", "puddle"]], &[]),
    ("arena_weapon", &[&["shield", "metal"], &["broken", "spear"], &["trident"], &["shortsword"], &["broken", "shield", "wood"]], &["bloody"]),
    // Цельный бревенчатый мост FA (`Log_Bridge_*_11x4`): без концов, пролётов,
    // верёвочных, подъёмных и мостков в доску.
    ("bridge", &[&["bridge"], &["log", "bridge"], &["мост"]], &["straight", "rope", "draw", "track", "plank"]),
    // Накладки FA (`Stairs_Overlay`) прозрачны — лестницы на карте не видно.
    ("stairs", &[&["stairs"], &["staircase"], &["stair"], &["лестница"]], &["spiral", "overlay", "railing", "rolling", "library", "pillar", "path"]),
    // Каменные ступени FA — «дорожка» в разделе paths: пролёт 3×1 в тон камня.
    ("stairs_down", &[&["stairs", "stone", "path"]], &["railing"]),
    // Перила к ним: тонкая полоса той же длины.
    ("stair_rail", &[&["stairs", "railing", "stone", "path"]], &[]),
    ("stump", &[&["stump"], &["пень"]], &[]),
    ("stalagmite_ice", &[&["arctic", "stalagmite", "ice"], &["stalagmite", "ice"], &["сосулька"]], &[]),
    ("crystal", &[&["underdark", "crystal"], &["crystal", "rock"], &["кристалл"]], &[]),
    // Грибы у болота — бурые и белые; светящиеся Подземья — для грибной пещеры.
    ("mushroom", &[&["mushroom", "brown"], &["mushroom", "tan"], &["mushroom", "white"], &["гриб"]], &["stalk"]),
    ("mushroom_glow", &[&["underdark", "mushroom"]], &["stalk"]),
    ("cattail", &[&["cattail", "brown"], &["cattail", "yellow"], &["рогоз"]], &[]),
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
    // Скалы FA (`Cliff_Stone_*`): земляные, ледяные, песчаные — не пещера; верх скалы — часть.
    ("wall_cave", &[&["cliff", "stone"], &["cave"], &["rock"], &["скала"]], &["top"]),
    // Скалы под вид пещеры: каменная — сланец и бурый камень, болотная — мшистый и
    // земляной, грибная — лиловый.
    ("wall_cave_stone", &[&["cliff", "stone", "slate"], &["cliff", "stone", "earthy"]], &["top", "sand", "desert"]),
    ("wall_cave_ice", &[&["cliff", "ice"], &["arctic", "ice", "cliff"]], &["top"]),
    ("wall_cave_sand", &[&["cliff", "stone", "sandstone"], &["cliff", "sandy"]], &["top"]),
    ("wall_cave_moss", &[&["cliff", "stone", "mossy"], &["cliff", "dirt"]], &["top"]),
    ("wall_cave_volcanic", &[&["cliff", "stone", "volcanic"]], &["top", "sand", "desert"]),
    ("wall_cave_purple", &[&["cliff", "stone", "purple"]], &["top"]),
    ("cy_wall", &[&["alloy"]], &[]),
    ("wall_dwarven", &[&["dwarven"], &["dwarf"], &["гномья"]], &[]),
    ("wall_drow", &[&["drow"], &["obsidian"], &["обсидиан"]], &[]),
];

const PORTALS: &[Slot] = &[
    // `Door_Large` — двустворчатая на две клетки: в одну створку она растягивалась.
    ("door_wood", &[&["door"], &["дверь"]], &["frame", "double", "window", "arch", "gate", "secret", "trap", "trapdoor", "large", "косяк"]),
    ("door_wood_iron", &[&["door", "iron"], &["door", "reinforced"], &["дверь", "окованная"]], &["frame", "double"]),
    ("door_metal", &[&["door", "metal"], &["door", "steel"], &["дверь", "железная"]], &["frame", "double"]),
    // Решётка камеры: у FA это тонкая серая дверь `Door_Metal_Gray_A`.
    ("door_bars", &[&["door", "metal", "gray", "a"]], &["frame", "double"]),
    // Железные косяки FA — для киберпанка.
    ("frame_door_metal", &[&["door", "frame", "metal"]], &["double"]),
    ("frame_double_metal", &[&["door", "frame", "double", "metal"]], &[]),
    // Каменные косяки FA в тон стены: на одну клетку и двойной на две.
    ("frame_door_stone", &[&["door", "frame", "stone"]], &["double"]),
    ("frame_double_stone", &[&["door", "frame", "double", "stone"]], &[]),
    // Малые подоконники FA нарисованы торчком, часть — на чёрном фоне.
    ("window_stone", &[&["window", "stone"], &["window"], &["окно"]], &["frame", "glass", "stained", "broken", "small"]),
    ("window_wood", &[&["window", "wood"], &["window", "wooden"], &["window"]], &["frame", "glass", "stained", "broken", "small"]),
];

const ROOFS: &[Slot] = &[
    ("roof_clay_red", &[&["clay", "red"], &["tile", "red"], &["tiles", "red"], &["terracotta"], &["clay"], &["черепица"]], &[]),
    ("roof_clay_brown", &[&["clay", "brown"], &["tile", "brown"], &["clay"], &["черепица"]], &[]),
    ("roof_clay_green", &[&["clay", "green"], &["tile", "green"], &["clay"], &["черепица"]], &[]),
    ("roof_slate_blue", &[&["slate", "blue"], &["slate"], &["сланец"]], &[]),
    ("roof_slate_gray", &[&["slate", "gray"], &["slate", "grey"], &["slate"], &["сланец"]], &[]),
    ("roof_shingle", &[&["shingle"], &["shingles"], &["wood"], &["дранка"]], &[]),
    ("roof_thatch", &[&["thatch"], &["straw"], &["hay"], &["солома"]], &[]),
    ("roof_metal", &[&["corrugated"], &["metal"], &["профнастил"]], &["copper"]),
    ("roof_copper", &[&["copper"], &["медь"]], &[]),
];

/// Слова в начале имени, что не говорят, что за предмет: `Fancy_Chair` — стул,
/// `Wall_Brick` — кирпич, `Double_Bed` — кровать (двуспальность ловит исключение).
const MODIFIERS: &[&str] = &["fancy", "single", "double", "small", "large", "big", "old", "wall", "walls", "roof", "texture", "fa", "arranged"];

/// Чего генератор не берёт никогда: серые заготовки под перекраску и накладки.
const NEVER: &[&str] = &["colorable", "overlay", "path"];

/// Части и обломки предметов у FA: каркас кровати, дверца шкафа, ножка стола, конец
/// моста. Сами по себе они не мебель, а целые наборы (`Arrangement`) — неведомого размера.
const PARTS: &[&str] = &[
    "frame", "headboard", "base", "lid", "insert", "sticker", "side", "end", "corner", "branch", "brackets", "support", "decoration",
    "winch", "patch", "shadow", "piece", "pieces", "part", "top", "leg", "cloth", "runner", "cover", "door", "drawer", "broken", "fallen",
    "arrangement", "filled", "spit", "pile", "flap", "connector", "ending", "blanket", "blankets", "mattress", "pillow", "pillows", "ladder", "headboards", "frames",
];

/// Куда смотрит спина своего предмета, если не вверх, как у встроенных: камины FA
/// нарисованы задней стенкой (дымоход, глухая сторона топки) влево, деревянной
/// полкой и устьем топки вправо; у полукруглого горна плоская сторона внизу.
/// Поворот по часовой, который ставит спину наверх.
fn turn_of(slot: &str, name: &str) -> f32 {
    let file = name.rsplit('/').next().unwrap_or(name);
    // Стойки FA для оружия нарисованы стоя: доской влево, крюками вправо.
    let standing = library::size_from_name(file).is_some_and(|s| s[1] > s[0]);
    if slot.starts_with("weapon_rack") && file.starts_with("Weapon_Rack") && standing {
        90.0
    } else if slot == "forge" && file.contains("SemiCircle") {
        180.0
    } else if slot == "fireplace" && file.starts_with("Fireplace_") {
        90.0
    } else if (slot == "stairs_down" || slot == "stair_rail") && file.starts_with("Stairs_") {
        // Пролёт нарисован лёжа: ступени поперёк длинной стороны.
        90.0
    } else {
        0.0
    }
}

/// Размер из имени близок к встроенному — только у столов и сидений: под них место
/// размечено заранее (стол со стульями — 3×4 клетки), и стол 5×2 налезает на соседей,
/// а столик 1×1 оставляет стулья в воздухе. Прочее ставится по своему размеру.
fn fits_size(slot: &str, name: &str) -> bool {
    let beds = ["bed", "bed_double"].contains(&slot);
    if !beds && !["table", "table_round", "chair", "stool", "bar_counter"].contains(&slot) {
        return true;
    }
    let (Some(own), Some(base)) = (library::size_from_name(name), library::builtin_size(&format!("{}{slot}", library::BUILTIN))) else { return true };
    let (big, small) = (own[0].max(own[1]), own[0].min(own[1]));
    let (bb, bs) = (base[0].max(base[1]), base[0].min(base[1]));
    // Застеленная кровать FA одна на оба слота: одиночную от двуспальной отличает ширина.
    big <= bb + 0.5 && small <= bs + 0.5 && big >= bb * 0.75 && (!beds || small >= bs * 0.75)
}

/// Из найденного под слот брать только с этим словом, если такие есть: у FA
/// `Single_Bed_*` — голый матрас под одеяло, застеленные — `Arranged_Bed_*`.
/// Окна FA: в деревянной стене — деревянный подоконник, в каменной — каменный.
const PREFER: &[(&str, &str)] = &[("bed", "arranged"), ("bed_double", "arranged"), ("window_wood", "wood"), ("window_stone", "stone")];

/// Близкие слоты: чем заменить, когда своего под слот нет.
const NEAR: &[(&str, &str)] = &[
    ("tree_summer", "tree"),
    ("tree_spring", "tree"),
    ("tree_autumn", "tree"),
    ("tree_winter", "tree"),
    ("tree_magic", "tree"),
    ("tree_bare", "tree_winter"),
    ("bush_summer", "bush"),
    ("bush_spring", "bush"),
    ("bush_autumn", "bush"),
    ("bush_magic", "bush"),
    ("grass_dry", "grass_green"),
    ("grass_frost", "grass_green"),
    ("moss", "grass_green"),
    ("forest_floor", "grass_green"),
    ("mud", "dirt"),
    ("swamp_wet", "swamp"),
    ("ash", "dirt"),
    ("water_deep", "water_shallow"),
    ("stone_tiles", "flagstones"),
    ("dungeon_flagstones", "flagstones"),
    ("crypt_floor", "dungeon_flagstones"),
    ("prison_floor", "flagstones"),
    ("temple_floor", "marble"),
    ("ruin_floor", "dungeon_flagstones_moss"),
    ("pillar_stone", "pillar"),
    ("statue_figure", "statue"),
    ("pillar_broken", "rubble"),
    ("dungeon_flagstones_moss", "flagstones"),
    ("rug_round", "rug"),
    ("wall_timber", "wall_wood"),
    ("wall_thatch", "wall_wood"),
    ("wall_palisade", "wall_wood"),
    ("wall_fieldstone", "wall_stone_blocks"),
    ("wall_sandstone", "wall_stone_blocks"),
    ("wall_ruined", "wall_stone_blocks"),
    ("wall_ruined_blocks", "wall_stone_blocks"),
    ("wall_cave", "wall_stone_blocks"),
    ("wall_cave_stone", "wall_cave"),
    ("wall_cave_ice", "wall_cave"),
    ("wall_cave_sand", "wall_cave"),
    ("wall_cave_moss", "wall_cave"),
    ("wall_cave_volcanic", "wall_cave"),
    ("wall_cave_purple", "wall_cave"),
    ("cave_slate", "cave_floor"),
    ("cave_earthy", "cave_floor"),
    ("cave_sandstone", "sand"),
    ("cave_volcanic", "ash"),
    ("wall_hedge", "wall_wood"),
    ("door_wood_iron", "door_wood"),
    ("weapon_rack_spears", "weapon_rack"),
    ("weapon_rack_axes", "weapon_rack"),
    ("weapon_rack_maces", "weapon_rack"),
    ("weapon_rack_bows", "weapon_rack"),
    ("market_stall_blue", "market_stall"),
    ("market_stall_ochre", "market_stall"),
    ("sandstone", "sand"),
    ("chest_open", "chest"),
    ("treasure_chest", "chest"),
    ("cy_bar", "bar_counter"),
    ("cy_desk", "table"),
    ("cy_chair", "chair"),
    ("cy_crate", "crate"),
    ("cy_floor", "flagstones"),
    ("chest_empty", "chest"),
    ("arena_sand", "sand"),
    ("arena_sand_rough", "sand"),
    ("arena_tiers_sand", "sandstone"),
    ("arena_tiers_slate", "flagstones"),
    ("arena_rug", "rug"),
    ("arena_obelisk", "statue"),
    ("arena_walk_sand", "flagstones"),
    ("arena_walk_slate", "flagstones"),
    ("arena_marble", "marble"),
    ("arena_brazier", "brazier"),
    ("arena_statue", "statue"),
];

/// Слова имени файла, строчными.
fn file_words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// Выбранный под слот ассет: ключ и размер в клетках. `own` — из библиотеки.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub key: String,
    pub size: [f32; 2],
    pub own: bool,
    /// Видимая рамка своего объекта в клетках от середины, без масштаба.
    pub visible: Option<[f32; 4]>,
    /// Доворот по часовой, чтобы спина предмета смотрела вверх (см. [`turn_of`]).
    pub turn: f32,
}

/// Чем мерить картинку, когда размер не в имени: ширина и высота в точках.
pub type Measure = Box<dyn Fn(&str) -> Option<[u32; 2]>>;
/// Чем мерить непрозрачность текстуры, 0..1.
pub type Opacity = Box<dyn Fn(&str) -> Option<f32>>;

pub struct Palette {
    /// Слот → ключи и имена подходящих ассетов, по алфавиту ключей.
    slots: BTreeMap<&'static str, Vec<(String, String)>>,
    measure: Measure,
    sizes: RefCell<HashMap<String, [f32; 2]>>,
    /// Чем найти видимую часть картинки (доли от середины); нет — весь холст.
    visible: Option<Box<dyn Fn(&str) -> Option<[f32; 4]>>>,
    boxes: RefCell<HashMap<String, Option<[f32; 4]>>>,
    /// Концы нити по картинке (паутина), доли холста от середины.
    ends: Option<Box<dyn Fn(&str) -> Option<[f32; 4]>>>,
    /// Чем мерить непрозрачность текстуры пола; нет — не мерить.
    opacity: Option<Opacity>,
    alphas: RefCell<HashMap<String, Option<f32>>>,
    rng: RefCell<Rng>,
    /// Рамы окон FA (`Window_Frame_*`): ставятся поверх подоконника.
    frames: Vec<(String, String)>,
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
        Self { slots: BTreeMap::new(), measure: Box::new(|_| None), sizes: RefCell::default(), visible: None, boxes: RefCell::default(), ends: None, opacity: None, alphas: RefCell::default(), rng: RefCell::new(Rng::new("")), frames: Vec::new() }
    }

    /// Замены из библиотеки: только не встроенные ассеты. Слот крыши без своих
    /// совпадений берёт любую крышу из библиотеки.
    pub fn from_library(assets: &[Asset], measure: Measure) -> Self {
        let mut slots: BTreeMap<&'static str, Vec<(String, String)>> = BTreeMap::new();
        let mut roofs = Vec::new();
        let mut frames = Vec::new();
        for a in assets.iter().filter(|a| !a.is_builtin()) {
            let words = a.words();
            let file = file_words(&a.name);
            if a.kind == Kind::Door && file.iter().any(|w| w == "window") && file.iter().any(|w| w == "frame") {
                frames.push((a.key.clone(), a.name.clone()));
            }
            let head = file.iter().find(|w| !MODIFIERS.contains(&w.as_str())).map_or("", |w| w.as_str());
            let in_file = |w: &str| file.iter().any(|f| f == w);
            for (slot, alternatives, none) in table_of(a.kind) {
                let named = alternatives.iter().any(|alt| alt[0] == head && alt[1..].iter().all(|w| in_file(w)));
                let part = a.kind == Kind::Object && PARTS.iter().any(|w| in_file(w)) && !alternatives.iter().any(|alt| alt.iter().any(|w| PARTS.contains(w)));
                if named && !part && !NEVER.iter().any(|w| in_file(w) && !(["stairs_down", "stair_rail"].contains(slot) && *w == "path")) && !none.iter().any(|w| in_file(w) || words.contains(w)) && fits_size(slot, &a.name) {
                    slots.entry(slot).or_default().push((a.key.clone(), a.name.clone()));
                }
            }
            if a.kind == Kind::Roof && !NEVER.iter().any(|w| file.iter().any(|f| f == w)) {
                roofs.push((a.key.clone(), a.name.clone()));
            }
        }
        if !roofs.is_empty() {
            for (slot, _, _) in ROOFS {
                slots.entry(slot).or_insert_with(|| roofs.clone());
            }
        }
        for (slot, word) in PREFER {
            if let Some(v) = slots.get_mut(slot)
                && v.iter().any(|(_, name)| file_words(name).iter().any(|w| w == word))
            {
                v.retain(|(_, name)| file_words(name).iter().any(|w| w == word));
            }
        }
        for v in slots.values_mut() {
            v.sort();
            v.dedup();
        }
        frames.sort();
        Self { slots, measure, sizes: RefCell::default(), visible: None, boxes: RefCell::default(), ends: None, opacity: None, alphas: RefCell::default(), rng: RefCell::new(Rng::new("")), frames }
    }

    /// Рама под подоконник FA: того же размера (`1x1`, `2x1`, «large»), ближе всех
    /// по материалу и цвету. У FA окно — подоконник и рама, без рамы стоек нет.
    pub fn window_frame(&self, sill: &str) -> Option<String> {
        let file = sill.rsplit(['/', '|']).next().unwrap_or(sill);
        let name = file.rsplit_once('.').map_or(file, |(n, _)| n);
        let words = file_words(name);
        if !words.iter().any(|w| w == "sill") {
            return None;
        }
        let size = words.last()?;
        let large = words.iter().any(|w| w == "large");
        self.frames
            .iter()
            .filter_map(|(key, n)| {
                let f = file_words(n);
                // Узкая рама `Small` — к форточке; наше стекло во весь проём, рама к нему обычная.
                let small = f.iter().any(|w| w == "small");
                (f.last() == Some(size) && f.iter().any(|w| w == "large") == large).then(|| (f.iter().filter(|w| words.contains(w)).count() as i32 - if small { 10 } else { 0 }, key))
            })
            .max_by_key(|(score, key)| (*score, std::cmp::Reverse(*key)))
            .map(|(_, key)| key.clone())
    }

    pub fn with_opacity(mut self, f: Opacity) -> Self {
        self.opacity = Some(f);
        self
    }

    fn alpha(&self, key: &str) -> Option<f32> {
        let f = self.opacity.as_ref()?;
        *self.alphas.borrow_mut().entry(key.to_owned()).or_insert_with(|| f(key))
    }

    /// Пол комнаты, а не накладка. Полы FA непрозрачны от 0,30 (брусчатка, кирпич)
    /// до 0,9, накладки — редкие доски `Flooring_H`, `Shack_Floor_*_B1/B2` — до 0,26.
    /// Трава и земля FA тоже около 0,3 (так они смешиваются), поэтому мерится только
    /// пол комнат. Не смогли измерить — годится.
    fn solid(&self, key: &str) -> bool {
        self.alpha(key).is_none_or(|a| a >= 0.28)
    }

    pub fn with_visible(mut self, f: Box<dyn Fn(&str) -> Option<[f32; 4]>>) -> Self {
        self.visible = Some(f);
        self
    }

    pub fn with_ends(mut self, f: Box<dyn Fn(&str) -> Option<[f32; 4]>>) -> Self {
        self.ends = Some(f);
        self
    }

    /// Концы нити своего ассета, доли холста от середины.
    pub fn ends(&self, key: &str) -> Option<[f32; 4]> {
        self.ends.as_ref().and_then(|f| f(key))
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
        // Встроенного под слот нет (дерево осени) — встроенный у близкого слота.
        // Проверка по всем встроенным, а не по размеру объекта: иначе у мха, грязи,
        // глубокой воды и скалы брался «близкий» — трава, земля, мелководье, блоки.
        if !library::is_builtin_key(&key)
            && let Some(near) = NEAR.iter().find(|n| n.0 == slot)
        {
            return Self::builtin_pick(near.1);
        }
        Pick { size: library::builtin_size(&key).unwrap_or([1.0, 1.0]), key, own: false, visible: None, turn: 0.0 }
    }

    /// Ассет под слот: свой, если нашёлся; нет — свой у близкого слота (сухая трава —
    /// просто трава: встроенное пятно на чужой траве режет глаз); иначе встроенный.
    pub fn pick(&self, slot: &str) -> Pick {
        self.pick_toned(slot, "")
    }

    /// Как [`Palette::pick`], но из своих — только те, в имени которых есть слово
    /// `tone` (камень: `slate`, `sandstone`…): так подземелье держит один камень.
    /// Таких нет — из всех.
    pub fn pick_toned(&self, slot: &str, tone: &str) -> Pick {
        let near = NEAR.iter().find(|n| n.0 == slot).map(|n| n.1);
        let indoor = INDOOR_FLOORS.contains(&slot);
        let own = |s: &str| self.slots.get(s).filter(|v| !v.is_empty());
        let far = near.and_then(|n| NEAR.iter().find(|f| f.0 == n)).map(|f| f.1);
        let Some(all) = own(slot).or_else(|| near.and_then(own)).or_else(|| far.and_then(own)) else { return Self::builtin_pick(slot) };
        let tone = tone.to_lowercase();
        // У части FA красный камень зовётся `Red`, а не `Redrock` (ступени).
        let alias = if tone == "redrock" { "red".to_string() } else { tone.clone() };
        let toned: Vec<(String, String)> = if tone.is_empty() { Vec::new() } else { all.iter().filter(|(_, n)| { let w = file_words(n); w.contains(&tone) || w.contains(&alias) }).cloned().collect() };
        let found = if toned.is_empty() { all } else { &toned };
        let mut i = if found.len() > 1 { self.rng.borrow_mut().int(0, found.len() as i32 - 1) as usize } else { 0 };
        // Выпала накладка — следующий по списку: мерить все варианты долго (секунды).
        if indoor && let Some(k) = (0..found.len()).map(|step| (i + step) % found.len()).find(|&k| self.solid(&found[k].0)) {
            i = k;
        }
        let (key, name) = &found[i];
        let size = *self.sizes.borrow_mut().entry(key.clone()).or_insert_with(|| {
            library::size_from_name(name)
                .filter(|s| s[0] > 0.0 && s[1] > 0.0)
                .or_else(|| (self.measure)(key).map(|px| [(px[0] as f32 / PX_PER_CELL).max(0.25), (px[1] as f32 / PX_PER_CELL).max(0.25)]))
                .unwrap_or([1.0, 1.0])
        });
        let visible = *self.boxes.borrow_mut().entry(key.clone()).or_insert_with(|| self.visible.as_ref().and_then(|f| f(key)));
        let visible = visible.map(|[l, t, r, b]| [l * size[0], t * size[1], r * size[0], b * size[1]]);
        Pick { key: key.clone(), size, own: true, visible, turn: turn_of(slot, name) }
    }

    /// Все свои ассеты слота, по алфавиту ключей; пусто — своих нет.
    pub fn keys(&self, slot: &str) -> Vec<String> {
        self.slots.get(slot).map(|v| v.iter().map(|(k, _)| k.clone()).collect()).unwrap_or_default()
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
            asset("Nature/Tree_Oak_A_3x3.webp", Kind::Object),
            asset("Nature/Oak_Tree_B_3x3.webp", Kind::Object),
            asset("Tables/Coffee_Table_A_2x1.webp", Kind::Object),
            asset("Tables/Fancy_Chair_Red_A_1x1.webp", Kind::Object),
            asset("Terrain/Grass_Green_A.webp", Kind::Floor),
            asset("Terrain/Grass_Dry_B.webp", Kind::Floor),
            asset("Walls/Wall_Timber_A.webp", Kind::Wall),
            asset("Patterns/Roof_Slate_Blue.webp", Kind::Roof),
            asset("Wood_Terrain/Wood_Ashen_Flooring_A.webp", Kind::Floor),
            Asset::new(format!("{}chair", library::BUILTIN), "Встроенные/Стул", Kind::Object),
        ];
        let p = Palette::from_library(&assets, Box::new(|_| Some([512, 768])));
        p.seed("x");
        let bed = p.pick("bed");
        assert!(bed.own && bed.key.contains("Bed_Wooden") && bed.size == [1.0, 2.0], "{bed:?}");
        assert!(p.pick("bed_double").key.contains("Bed_Double"));
        // Стул — «Fancy_Chair», а не встроенный; кофейный столик — не стол.
        assert_eq!(p.pick("chair").key, assets[6].key);
        assert_eq!(p.pick("table"), Pick { key: "builtin:table".into(), size: [2.0, 1.0], own: false, visible: None, turn: 0.0 });
        assert_eq!(p.pick("grass_green").key, assets[7].key);
        assert_eq!(p.pick("grass_dry").key, assets[8].key);
        assert_eq!(p.pick("wall_timber").key, assets[9].key);
        assert_eq!(p.pick("wood_planks").key, assets[11].key);
        // Любая крыша закрывает все слоты крыш.
        assert_eq!(p.pick("roof_thatch").key, assets[10].key);
        let picks: std::collections::HashSet<String> = (0..20).map(|_| p.pick("tree").key).collect();
        assert_eq!(picks.len(), 2);
        assert!(p.fits("tree", &assets[3].key) && p.fits("tree", "builtin:tree") && !p.fits("tree", &assets[0].key));
        // Дуб без слова «tree» первым — тоже дерево: слот знает «oak».
        assert!(p.fits("tree", &assets[4].key));
        // Повторимость по зерну.
        let seq = |s: &str| {
            p.seed(s);
            (0..6).map(|_| p.pick("tree").key).collect::<Vec<_>>()
        };
        assert_eq!(seq("а"), seq("а"));
        assert!(Palette::builtin().is_empty());
    }

    /// Что подберётся из настоящих паков FA: `cargo test --release fa_picks -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn fa_picks() {
        let dir = std::path::Path::new(r"E:\D&D\FA_Objects_A_v3.51");
        let mut assets = Vec::new();
        for f in std::fs::read_dir(dir).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == "dungeondraft_pack")) {
            assets.extend(library::pack_assets(&f.path()).unwrap_or_default());
        }
        let p = Palette::from_library(&assets, Box::new(|_| None));
        p.seed("проба");
        for table in [FLOORS, OBJECTS, WALLS, PORTALS, ROOFS] {
            for (slot, _, _) in table {
                let n = p.slots.get(slot).map_or(0, Vec::len);
                if n == 0 && !p.pick(slot).own {
                    println!("{slot:24}     -  ВСТРОЕННЫЙ");
                    continue;
                }
                let names: Vec<String> = (0..3).map(|_| p.pick(slot)).filter(|x| x.own).map(|x| library::label(&x.key)).collect();
                println!("{slot:24} {n:5}  {}", names.join(" | "));
            }
        }
    }

    #[test]
    fn floor_overlays_are_not_floors() {
        let assets = vec![
            asset("Wood/Wood_Ashen_Flooring_A.webp", Kind::Floor),
            asset("Wood/Wood_Ashen_Flooring_B.webp", Kind::Floor),
            asset("Wood/Wood_Ashen_Flooring_H.webp", Kind::Floor),
            asset("Grass/Grass_F.webp", Kind::Floor),
            asset("Grass/Grass_G.webp", Kind::Floor),
        ];
        let alpha = |k: &str| Some(if k.contains("_H") { 0.2 } else if k.contains("Grass_F") { 0.32 } else if k.contains("Grass") { 0.9 } else { 0.83 });
        let p = Palette::from_library(&assets, Box::new(|_| None)).with_opacity(Box::new(alpha));
        assert!((0..20).all(|_| !p.pick("wood_planks").key.contains("_H")));
        // Трава — не пол комнаты: прозрачная трава FA остаётся в деле.
        assert!((0..40).any(|_| p.pick("grass_green").key.contains("Grass_F")));
    }

    #[test]
    fn fa_quirks_are_handled() {
        let assets = vec![
            asset("Beds/Single_Bed_Purple_Blanket_E_1x2.webp", Kind::Object),
            asset("Beds/Arranged_Bed_Wood_Ashen_A1_1x2.webp", Kind::Object),
            asset("Beds/Single_Bed_Ashen_A_1x2.webp", Kind::Object),
            asset("Beds/Arranged_Bed_Wood_Ashen_A6_2x2.webp", Kind::Object),
            asset("Fireplaces/Fireplace_Rounded_Chimney_Stone_Earthy_A_2x2.webp", Kind::Object),
            asset("Fireplaces/Fireplace_Chimneyless_Stone_Slate_B1_1x2.webp", Kind::Object),
            asset("Windows/Window_Sill_Small_Stone_Slate_A1_1x1.webp", Kind::Door),
            asset("Windows/Window_Sill_Stone_Slate_A1_1x1.webp", Kind::Door),
        ];
        let p = Palette::from_library(&assets, Box::new(|_| None));
        // Одеяло — накладка, голый матрас — основа: берётся застеленная кровать.
        assert!((0..10).all(|_| p.pick("bed").key == assets[1].key));
        assert_eq!(p.pick("bed_double").key, assets[3].key);
        let fire = p.pick("fireplace");
        assert_eq!((fire.key.as_str(), fire.turn), (assets[4].key.as_str(), 90.0));
        // Малые подоконники FA стоят торчком и бывают без прозрачности.
        assert_eq!(p.pick("window_stone").key, assets[7].key);
    }

    #[test]
    fn fa_window_gets_frame_of_its_size_and_material() {
        let assets = vec![
            asset("Windows/Window_Sill_Stone_Slate_A1_1x1.webp", Kind::Door),
            asset("Windows/Window_Sill_Large_Wood_Light_A1_2x1.webp", Kind::Door),
            asset("Windows/Window_Frames/Window_Frame_Wood_Light_A_1x1.webp", Kind::Door),
            asset("Windows/Window_Frames/Window_Frame_Stone_Slate_A_1x1.webp", Kind::Door),
            asset("Windows/Window_Frames/Window_Frame_Large_Wood_Light_A_2x1.webp", Kind::Door),
        ];
        let p = Palette::from_library(&assets, Box::new(|_| None));
        assert_eq!(p.window_frame(&assets[0].key).as_deref(), Some(assets[3].key.as_str()));
        assert_eq!(p.window_frame(&assets[1].key).as_deref(), Some(assets[4].key.as_str()));
        // Рама — не окно, и встроенному окну своя рама не нужна.
        assert!(!p.fits("window_wood", &assets[2].key));
        assert_eq!(p.window_frame("builtin:window_wood"), None);
    }

    #[test]
    fn forest_seasons_pick_fa_colors() {
        let assets = vec![
            asset("Trees/Tree_Tops/Tree_Green_A1_6x6.webp", Kind::Object),
            asset("Trees/Tree_Tops/Tree_Red_A1_6x6.webp", Kind::Object),
            asset("Trees/Tree_Tops/Tree_Multicolor3_A1_6x6.webp", Kind::Object),
            asset("Trees/Pine_Trees/Pine_Blue_A1_4x4.webp", Kind::Object),
            asset("Trees/Bare_Trees/Bare_Tree_Dark_A1_8x9.webp", Kind::Object),
            asset("Trees/Bare_Trees_Shadows/Bare_Tree_Shadow_Dark_A1_8x9.webp", Kind::Object),
        ];
        let p = Palette::from_library(&assets, Box::new(|_| None));
        assert_eq!(p.keys("tree_summer"), vec![assets[0].key.clone()]);
        assert_eq!(p.keys("tree_autumn"), vec![assets[1].key.clone()]);
        assert_eq!(p.keys("tree_bare"), vec![assets[4].key.clone()]);
        assert!(p.keys("tree_magic").contains(&assets[2].key) && p.keys("tree_winter") == vec![assets[3].key.clone()]);
        // Весной своих нет — летние; встроенного осеннего нет — встроенное дерево.
        assert_eq!(p.pick("tree_spring").key, assets[0].key);
        assert_eq!(Palette::builtin_pick("tree_autumn").key, "builtin:tree");
        assert_eq!(Palette::builtin_pick("tree_bare").key, "builtin:tree");
    }

    #[test]
    fn stumps_for_canopies_come_from_the_library() {
        let assets = vec![asset("Trees/Stumps/Stump_Ashen_A1_4x4.webp", Kind::Object), asset("Logs/Fir_Log_Dark_Stump_A2_3x2.webp", Kind::Object)];
        let p = Palette::from_library(&assets, Box::new(|_| None));
        // Бревно с пнём сбоку — не пень сверху.
        assert_eq!(p.keys("stump"), vec![assets[0].key.clone()]);
        assert!(Palette::builtin().keys("stump").is_empty());
    }

    #[test]
    fn size_comes_from_the_picture_when_the_name_is_silent() {
        let assets = vec![asset("Furniture/Chair.webp", Kind::Object)];
        let p = Palette::from_library(&assets, Box::new(|_| Some([512, 256])));
        assert_eq!(p.pick("chair").size, [2.0, 1.0]);
    }
}
