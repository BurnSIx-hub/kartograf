//! Готовые карты по плану (`plan.rs`): подземелье, пещера, лес, городской квартал,
//! дом и таверна. План говорит, что и зачем, а здесь это превращается в пол, стены,
//! двери, мебель, крыши и свет встроенными ассетами. Случайная карта идёт тем же
//! путём: окно генератора пишет случайный план, строитель его исполняет.
//!
//! Один план с тем же зерном даёт одну и ту же карту: вся случайность — из `Rng`,
//! порядок обхода везде постоянный.

use crate::generate::{self, Corridor, Kind, Rng, Room};
use crate::library;
use crate::map::{Label, Layer, Live, Map, Material, Paint, Path, Portal, PortalKind, Prop, Roof, RoofShape, Segment, Shadows, Wall, WallStyle};
use crate::mask::{self, Brush, Mask, Shape};
use crate::palette::{Palette, Pick};
use crate::plan::{self, Assets, CaveKind, LinkKind, Mood, Plan, River, Role, Season, Size, SizeName, OUTSIDE};
use std::cell::RefCell;
use std::collections::BTreeMap;

thread_local! {
    /// Подбор ассетов на время постройки: помощники просят ассет по слоту и не
    /// таскают лишний параметр. Ставится в `build`, снимается по его концу.
    static PALETTE: RefCell<Option<Palette>> = const { RefCell::new(None) };
}

/// Снимает подбор, когда постройка кончилась — и при ошибке тоже.
struct PaletteGuard;

impl Drop for PaletteGuard {
    fn drop(&mut self) {
        PALETTE.with(|p| *p.borrow_mut() = None);
    }
}

/// Ассет под слот: свой из библиотеки или встроенный.
fn asset(slot: &str) -> Pick {
    PALETTE.with(|p| match p.borrow().as_ref() {
        Some(pal) => pal.pick(slot),
        None => Palette::builtin_pick(slot),
    })
}

/// Ключ объекта — ассет этого слота (свой или встроенный).
fn fits(slot: &str, asset: &str) -> bool {
    PALETTE.with(|p| match p.borrow().as_ref() {
        Some(pal) => pal.fits(slot, asset),
        None => asset == key(slot),
    })
}

include!("prop_bounds.rs");

#[path = "arena.rs"]
mod arena;

/// Сколько клеток у края карты отдаётся реке у дома.
const RIVER_ROOM: i32 = 9;

/// Мебель в домах крупнее настоящей: на столе играют фигурки, а не меряют комнаты.
const FURNITURE: f32 = 1.35;
/// Сколько от оси стены до предмета: половина её толщины.
const WALL_GAP: f32 = 0.2;

/// Что вышло: сколько главного (комнат, домов, деревьев) и что из плана не удалось.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub count: usize,
    pub warnings: Vec<String>,
}

/// Ключ встроенного ассета — для косяков, которые всегда встроенные.
fn key(name: &str) -> String {
    format!("{}{name}", library::BUILTIN)
}

fn mat(m: &mut Map, name: &str) -> usize {
    let k = asset(name).key;
    m.material_index(Material { name: library::label(&k), paint: Paint::Texture(k), vary: false, live: Live::Auto }).unwrap_or(0)
}

/// Стена слота: картинка своя или встроенная, толщина — встроенной полосы.
fn wall(name: &str) -> WallStyle {
    let pick = asset(name);
    // Скала FA толщиной в свою картинку (`size` — точки / 256), прочие свои — 0,35.
    let width = library::builtin_wall_width(&pick.key).unwrap_or(if library::is_cliff(&pick.key) { pick.size[1] } else { 0.35 });
    WallStyle { paint: Paint::Texture(pick.key), width }
}

/// Видимый предмет от центра с учётом поворота на четверти: лево, верх, право, низ.
/// У встроенных — по таблице, у своих — весь холст.
fn bounds(p: &Pick, slot: &str, rot: f32, scale: f32) -> [f32; 4] {
    let s = p.size;
    let whole = [-s[0] / 2.0, -s[1] / 2.0, s[0] / 2.0, s[1] / 2.0];
    let b = if p.own { p.visible.unwrap_or(whole) } else { BOUNDS.iter().find(|t| t.0 == slot).map_or(whole, |t| t.1) };
    let [l, t, r, u] = b.map(|v| v * scale);
    match (rot / 90.0).round() as i32 % 4 {
        0 => [l, t, r, u],
        1 => [-u, l, -t, r],
        2 => [-r, -u, -l, -t],
        _ => [t, -r, u, -l],
    }
}

fn cells_of(len: f32) -> i32 {
    ((len - 0.02).ceil() as i32).max(1)
}

/// Занятые клетки: объекты не налезают друг на друга и не перекрывают проходы.
struct Spots {
    w: i32,
    h: i32,
    used: Vec<bool>,
}

impl Spots {
    fn new(m: &Map) -> Self {
        let (w, h) = (m.width as i32, m.height as i32);
        Self { w, h, used: vec![false; (w * h) as usize] }
    }

    fn block(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for j in y.max(0)..(y + h).min(self.h) {
            for i in x.max(0)..(x + w).min(self.w) {
                self.used[(j * self.w + i) as usize] = true;
            }
        }
    }

    /// Вернуть место: только что занятое и не пригодившееся.
    fn unblock(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for j in y.max(0)..(y + h).min(self.h) {
            for i in x.max(0)..(x + w).min(self.w) {
                self.used[(j * self.w + i) as usize] = false;
            }
        }
    }

    fn free(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        x >= 0 && y >= 0 && x + w <= self.w && y + h <= self.h && (y..y + h).all(|j| (x..x + w).all(|i| !self.used[(j * self.w + i) as usize]))
    }
}

/// Объект без проверки места: `pos` — центр холста.
fn drop(m: &mut Map, name: &str, pos: [f32; 2], rot: f32, scale: f32) {
    let pick = asset(name);
    // Голые деревья FA по 8–11 клеток — втрое больше кроны: без ужатия ветви
    // накрывают весь зимний лес.
    let scale = if name == "tree_bare" && pick.own { scale * (5.0 / pick.size[0].max(pick.size[1])).min(1.0) } else { scale };
    drop_pick(m, &pick, pos, rot, scale);
    let Some(p) = m.props.last_mut() else { return };
    // Слот дерева — дерево (пни, бревна и пальмы не берутся): своё тоже уходит
    // в Foundry кроной над токенами, как встроенное.
    if name.starts_with("tree") && !crate::map::canopy_by_name(&p.asset) {
        p.canopy = Some(true);
    }
    // У встроенного дерева и куста одна картинка на все сезоны — подкрашиваем.
    if !pick.own {
        p.tint = season_tint(name);
    }
}

/// Подкраска встроенного под время года: умножение, поэтому только темнее и в цвет.
fn season_tint(slot: &str) -> Option<[u8; 3]> {
    match slot.split_once('_').map(|(_, s)| s) {
        Some("autumn") => Some([255, 150, 70]),
        Some("spring") => Some([215, 255, 190]),
        Some("winter" | "bare") => Some([120, 165, 185]),
        Some("magic") => Some([130, 160, 255]),
        _ => None,
    }
}

/// Лес по времени года: земля, пятна на ней, подстилка под деревьями, кусты.
struct ForestLook {
    ground: &'static str,
    patches: &'static str,
    under: &'static str,
    bush: &'static str,
    /// Доля голых деревьев: зимой больше половины, ранней весной немного.
    bare: f64,
    /// Сколько кустов против летнего: зимой их почти нет.
    brush: f32,
}

fn forest_look(season: Season) -> ForestLook {
    match season {
        Season::Summer => ForestLook { ground: "grass_green", patches: "grass_dry", under: "forest_floor", bush: "bush_summer", bare: 0.0, brush: 1.0 },
        Season::Spring => ForestLook { ground: "grass_green", patches: "mud", under: "moss", bush: "bush_spring", bare: 0.15, brush: 1.2 },
        Season::Autumn => ForestLook { ground: "grass_dry", patches: "forest_floor", under: "forest_floor", bush: "bush_autumn", bare: 0.1, brush: 1.0 },
        Season::Winter => ForestLook { ground: "snow", patches: "grass_frost", under: "grass_frost", bush: "bush_summer", bare: 0.55, brush: 0.3 },
        Season::Magic => ForestLook { ground: "moss", patches: "grass_green", under: "forest_floor", bush: "bush_magic", bare: 0.0, brush: 1.3 },
    }
}

/// Пещера по виду: пол, стены, пятна на полу (слот, сколько, радиус, растёкость),
/// водоём (мелкое и середина, сколько) и что стоит по пещере.
struct CaveLook {
    outside: &'static str,
    floor: &'static str,
    wall: &'static str,
    patches: [(&'static str, i32, f32, f32); 3],
    pool: Option<(&'static str, &'static str)>,
    pools: [i32; 2],
    /// Камни и прочее; повторы — чаще. Своего нет (грибов во встроенных) — пропуск.
    scatter: &'static [&'static str],
    /// Что растёт у воды.
    shore: Option<&'static str>,
    /// Размер водоёма против каменной пещеры.
    pool_size: f32,
    /// Сколько всего стоит против каменной.
    clutter: f32,
}

fn cave_look(kind: CaveKind) -> CaveLook {
    match kind {
        CaveKind::Stone => CaveLook {
            outside: "rocky_ground",
            floor: "cave_slate",
            wall: "wall_cave_stone",
            patches: [("gravel", 8, 2.2, 0.8), ("moss", 7, 1.8, 0.7), ("rocky_ground", 6, 2.4, 0.6)],
            pool: Some(("water_shallow", "water_deep")),
            pools: [1, 2],
            scatter: &["boulder", "boulder", "rubble"],
            shore: None,
            pool_size: 1.0,
            clutter: 1.0,
        },
        CaveKind::Ice => CaveLook {
            outside: "snow",
            floor: "ice",
            wall: "wall_cave_ice",
            patches: [("snow", 9, 2.4, 0.8), ("gravel", 4, 1.8, 0.7), ("cave_slate", 5, 2.2, 0.6)],
            // Подо льдом тёмная вода: полынья.
            pool: Some(("ice", "water_deep")),
            pools: [1, 2],
            scatter: &["stalagmite_ice", "stalagmite_ice", "boulder"],
            shore: None,
            pool_size: 1.3,
            clutter: 1.2,
        },
        CaveKind::Sand => CaveLook {
            outside: "sand",
            floor: "cave_sandstone",
            wall: "wall_cave_sand",
            patches: [("sand", 10, 2.6, 0.8), ("gravel", 6, 2.0, 0.7), ("rocky_ground", 4, 2.0, 0.6)],
            pool: None,
            pools: [0, 0],
            scatter: &["boulder", "rubble", "rubble"],
            shore: None,
            pool_size: 1.0,
            clutter: 1.0,
        },
        CaveKind::Swamp => CaveLook {
            outside: "mud",
            floor: "cave_earthy",
            wall: "wall_cave_moss",
            patches: [("mud", 10, 2.4, 0.8), ("moss", 9, 2.0, 0.7), ("swamp", 4, 1.6, 0.6)],
            pool: Some(("swamp_wet", "water_shallow")),
            pools: [2, 4],
            scatter: &["mushroom", "boulder", "cattail"],
            shore: Some("cattail"),
            pool_size: 1.7,
            clutter: 1.4,
        },
        CaveKind::Volcanic => CaveLook {
            outside: "ash",
            floor: "cave_volcanic",
            wall: "wall_cave_volcanic",
            patches: [("ash", 9, 2.4, 0.8), ("rocky_ground", 6, 2.2, 0.7), ("gravel", 4, 1.8, 0.6)],
            pool: Some(("lava", "lava")),
            pools: [1, 3],
            scatter: &["boulder", "boulder", "rubble"],
            shore: None,
            pool_size: 1.3,
            clutter: 1.0,
        },
        CaveKind::Fungal => CaveLook {
            outside: "rocky_ground",
            floor: "cave_slate",
            wall: "wall_cave_purple",
            patches: [("moss", 10, 2.4, 0.8), ("cave_earthy", 6, 2.2, 0.7), ("gravel", 4, 1.8, 0.6)],
            pool: Some(("water_shallow", "water_deep")),
            pools: [1, 2],
            scatter: &["mushroom_glow", "mushroom_glow", "crystal", "mushroom", "boulder"],
            shore: Some("mushroom_glow"),
            pool_size: 1.2,
            clutter: 2.2,
        },
    }
}

/// Слот, которому нет встроенной картинки и не нашлось своей: грибы, кристаллы.
fn missing(slot: &str) -> bool {
    let p = asset(slot);
    !p.own && !library::is_builtin_key(&p.key)
}

/// Дерево у построек по времени года: зимой больше половины голых.
fn season_tree(rng: &mut Rng, season: Season) -> &'static str {
    let look = forest_look(season);
    if look.bare > 0.0 && rng.chance(look.bare) { "tree_bare" } else { tree_slot(season) }
}

/// Куст у построек по времени года; зимой их почти нет — тогда ничего.
fn season_bush(rng: &mut Rng, season: Season) -> Option<&'static str> {
    let look = forest_look(season);
    (look.brush >= 1.0 || rng.chance(look.brush as f64)).then_some(look.bush)
}

/// Трава в квартале: летом и весной зелёная, осенью сухая, зимой снег, в магическом — мох.
fn town_grass(season: Season) -> (&'static str, &'static str) {
    match season {
        Season::Summer | Season::Spring => ("grass_green", "mud"),
        Season::Autumn => ("grass_dry", "mud"),
        Season::Winter => ("snow", "grass_frost"),
        Season::Magic => ("moss", "grass_green"),
    }
}

fn tree_slot(season: Season) -> &'static str {
    match season {
        Season::Summer => "tree_summer",
        Season::Spring => "tree_spring",
        Season::Autumn => "tree_autumn",
        Season::Winter => "tree_winter",
        Season::Magic => "tree_magic",
    }
}

fn drop_pick(m: &mut Map, p: &Pick, pos: [f32; 2], rot: f32, scale: f32) {
    m.props.push(Prop { asset: p.key.clone(), pos, size: p.size, rot, scale, shadow: None, light: None, canopy: None, layer: None, tint: None });
}

/// Объект в свободные клетки, `x`, `y` — левый верхний угол занятого места.
fn put(m: &mut Map, sp: &mut Spots, name: &str, x: i32, y: i32, rot: f32, scale: f32) -> bool {
    put_pick(m, sp, &asset(name), name, x, y, rot, scale)
}

#[allow(clippy::too_many_arguments)]
fn put_pick(m: &mut Map, sp: &mut Spots, p: &Pick, slot: &str, x: i32, y: i32, rot: f32, scale: f32) -> bool {
    let [l, t, r, u] = bounds(p, slot, rot, scale);
    let (fw, fh) = (cells_of(r - l), cells_of(u - t));
    if !sp.free(x, y, fw, fh) {
        return false;
    }
    sp.block(x, y, fw, fh);
    drop_pick(m, p, [x as f32 + fw as f32 / 2.0 - (l + r) / 2.0, y as f32 + fh as f32 / 2.0 - (t + u) / 2.0], rot, scale);
    true
}

/// Спиной вплотную к случайной стене комнаты; `corner` — ещё и боком к соседней.
fn by_wall(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str, scale: f32, corner: bool) -> bool {
    let p = asset(name);
    for _ in 0..40 {
        let side = rng.int(0, 3);
        let rot = (side as f32 * 90.0 + p.turn) % 360.0;
        let [left, top, right, low] = bounds(&p, name, rot, scale);
        let (fw, fh) = (cells_of(right - left), cells_of(low - top));
        if fw > r.w || fh > r.h {
            continue;
        }
        let (lo_x, hi_x, lo_y, hi_y) = (r.x, r.x + r.w - fw, r.y, r.y + r.h - fh);
        let pick_axis = |rng: &mut Rng, lo: i32, hi: i32| if corner { if rng.chance(0.5) { lo } else { hi } } else { rng.int(lo, hi) };
        let (x, y) = match side {
            0 => (pick_axis(rng, lo_x, hi_x), lo_y),
            1 => (hi_x, pick_axis(rng, lo_y, hi_y)),
            2 => (pick_axis(rng, lo_x, hi_x), hi_y),
            _ => (lo_x, pick_axis(rng, lo_y, hi_y)),
        };
        if !sp.free(x, y, fw, fh) {
            continue;
        }
        sp.block(x, y, fw, fh);
        let mut pos = [x as f32 + fw as f32 / 2.0 - (left + right) / 2.0, y as f32 + fh as f32 / 2.0 - (top + low) / 2.0];
        // Предмет дотягивается до стены: клетки шире его самого.
        if x == lo_x && (side == 3 || corner) {
            pos[0] = r.x as f32 + WALL_GAP - left;
        }
        if x == hi_x && (side == 1 || corner) {
            pos[0] = (r.x + r.w) as f32 - WALL_GAP - right;
        }
        if y == lo_y && (side == 0 || corner) {
            pos[1] = r.y as f32 + WALL_GAP - top;
        }
        if y == hi_y && (side == 2 || corner) {
            pos[1] = (r.y + r.h) as f32 - WALL_GAP - low;
        }
        drop_pick(m, &p, pos, rot, scale);
        return true;
    }
    false
}

fn inside(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str, scale: f32) -> bool {
    let p = asset(name);
    let [left, top, right, low] = bounds(&p, name, 0.0, scale);
    let (fw, fh) = (cells_of(right - left), cells_of(low - top));
    if fw > r.w || fh > r.h {
        return false;
    }
    let done = (0..16).any(|_| {
        let (x, y) = (rng.int(r.x, r.x + r.w - fw), rng.int(r.y, r.y + r.h - fh));
        put_pick(m, sp, &p, name, x, y, 0.0, scale)
    });
    if done
        && !p.own
        && let Some(q) = m.props.last_mut()
    {
        q.tint = season_tint(name);
    }
    done
}

fn pick<'a>(rng: &mut Rng, of: &[&'a str]) -> &'a str {
    of[rng.int(0, of.len() as i32 - 1) as usize]
}

/// Стол с местами вокруг. Занимает 3×4 клетки от `x`, `y`. Сиденья придвинуты
/// вплотную: по клеткам они отъезжали на полклетки. Стол кладётся после них, так что
/// чуть задвинутое сиденье уходит под столешницу, а не лежит на ней.
/// Место под обеденный стол со стульями и шаг между ними: у готовых наборов FA
/// (обычно 2×2) — 2×2 и проход в клетку, у своих стола со стульями — 3×4.
fn dining_cell() -> (i32, i32) {
    if missing("dining_set") { (3, 4) } else { (2, 2) }
}

fn dining(m: &mut Map, sp: &mut Spots, rng: &mut Rng, x: i32, y: i32) -> bool {
    let (cw, ch) = dining_cell();
    if !sp.free(x, y, cw, ch) {
        return false;
    }
    sp.block(x, y, cw, ch);
    // Готовый набор FA — стол, стулья, посуда разом; лишь бы влез в своё место.
    if !missing("dining_set") {
        let c = [x as f32 + cw as f32 / 2.0, y as f32 + ch as f32 / 2.0];
        for _ in 0..6 {
            let set = asset("dining_set");
            for rot in [0.0f32, 90.0] {
                let [l, t, r, u] = bounds(&set, "dining_set", rot, 1.0);
                if r - l <= cw as f32 + 0.25 && u - t <= ch as f32 + 0.25 && (r - l).min(u - t) >= 1.5 {
                    drop_pick(m, &set, [c[0] - (l + r) / 2.0, c[1] - (t + u) / 2.0], rot, 1.0);
                    return true;
                }
            }
        }
        sp.unblock(x, y, cw, ch);
        return false;
    }
    let seat = pick(rng, &["stool", "chair", "chair"]);
    // Один выбор на весь стол: из библиотеки каждый вызов берёт новый вариант, а мерить
    // надо то, что ставится.
    let chair = asset(seat);
    let c = [x as f32 + 1.5, y as f32 + 2.0];
    let [sl, st, sr, su] = bounds(&chair, seat, 0.0, FURNITURE);
    let depth = (su - st).min(sr - sl);
    // От края стола до середины сиденья: под столешницей только самый край.
    let out = depth * 0.5 - depth * 0.12;
    if rng.chance(0.3) {
        let table = asset("table_round");
        let [l, _, r, _] = bounds(&table, "table_round", 0.0, 1.1);
        let rad = (r - l) / 2.0 + out;
        for (k, rot) in [0.0f32, 90.0, 180.0, 270.0].into_iter().enumerate() {
            if k > 0 && !rng.chance(0.85) {
                continue;
            }
            let a = (rot - 90.0).to_radians();
            drop_pick(m, &chair, [c[0] + rad * a.cos(), c[1] + rad * a.sin()], rot, FURNITURE);
        }
        drop_pick(m, &table, c, 0.0, 1.1);
    } else {
        let table = asset("table");
        let [l, t, r, u] = bounds(&table, "table", 0.0, FURNITURE);
        let along = (r - l) * 0.25;
        for (side, rot) in [(-1.0f32, 0.0), (1.0, 180.0)] {
            let edge = if side < 0.0 { t } else { u };
            for dx in [-along, along] {
                if rng.chance(0.85) {
                    drop_pick(m, &chair, [c[0] + dx, c[1] + edge + side * out], rot, FURNITURE);
                }
            }
        }
        drop_pick(m, &table, c, 0.0, FURNITURE);
        // Утварь на столешницу: кружки, миски, хлеб — если влезает в стол.
        if !missing("table_clutter") && rng.chance(0.75) {
            let stuff = asset("table_clutter");
            for rot in [0.0f32, 90.0] {
                let [sl, stp, sr, su] = bounds(&stuff, "table_clutter", rot, 1.0);
                if sr - sl <= (r - l) * 1.02 && su - stp <= (u - t) * 1.02 {
                    drop_pick(m, &stuff, [c[0] - (sl + sr) / 2.0, c[1] - (stp + su) / 2.0], rot, 1.0);
                    break;
                }
            }
        }
    }
    true
}

/// Стол без стульев (рабочий, кухонный) с утварью на нём; `false` — не встал.
fn dining_free_table(m: &mut Map, sp: &mut Spots, rng: &mut Rng, x: i32, y: i32) -> bool {
    let table = asset("table");
    let [l, t, r, u] = bounds(&table, "table", 0.0, FURNITURE);
    let (fw, fh) = (cells_of(r - l), cells_of(u - t));
    if !sp.free(x, y, fw, fh) {
        return false;
    }
    sp.block(x, y, fw, fh);
    let c = [x as f32 + fw as f32 / 2.0, y as f32 + fh as f32 / 2.0];
    drop_pick(m, &table, [c[0] - (l + r) / 2.0, c[1] - (t + u) / 2.0], 0.0, FURNITURE);
    if !missing("table_clutter") && rng.chance(0.85) {
        let stuff = asset("table_clutter");
        for rot in [0.0f32, 90.0] {
            let [sl, st, sr, su] = bounds(&stuff, "table_clutter", rot, 1.0);
            if sr - sl <= (r - l) * 1.02 && su - st <= (u - t) * 1.02 {
                drop_pick(m, &stuff, [c[0] - (sl + sr) / 2.0, c[1] - (st + su) / 2.0], rot, 1.0);
                break;
            }
        }
    }
    true
}

/// Угол у камина: камин спиной к стене, перед ним ковёр, два кресла к огню.
/// `sides` — у каких стен можно (0 — север, 1 — восток, 2 — юг, 3 — запад).
/// Не нашлось места — `false`, тогда камин ставят как придётся.
fn hearth(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, sides: &[i32]) -> bool {
    let fire = asset("fireplace");
    let chair = asset("armchair");
    let rug = if rng.chance(0.5) { "rug_round" } else { "rug" };
    let carpet = asset(rug);
    let mut order: Vec<i32> = sides.to_vec();
    shuffle_i32(&mut order, rng);
    for side in order {
        let len = if side % 2 == 0 { r.w } else { r.h };
        let rot = (side as f32 * 90.0 + fire.turn) % 360.0;
        let [fl, ft, fr, fu] = bounds(&fire, "fireplace", rot, 1.0);
        let (along, depth) = if side % 2 == 0 { (fr - fl, fu - ft) } else { (fu - ft, fr - fl) };
        // Полоса у стены: камин, ковёр, кресла — глубиной до 4,5 клетки, шириной 4.
        let zone_w = (along.ceil() as i32).max(4);
        let zone_d = ((depth + 3.4).ceil() as i32).min(match side % 2 { 0 => r.h - 3, _ => r.w - 3 });
        if zone_w + 2 > len || zone_d < 3 {
            continue;
        }
        // Из стенных координат (вдоль стены u, от стены v) — в клетки карты.
        let to_world = |u: f32, v: f32| -> [f32; 2] {
            match side {
                0 => [r.x as f32 + u, r.y as f32 + v],
                1 => [(r.x + r.w) as f32 - v, r.y as f32 + u],
                2 => [r.x as f32 + u, (r.y + r.h) as f32 - v],
                _ => [r.x as f32 + v, r.y as f32 + u],
            }
        };
        let cell_rect = |u0: i32, v0: i32, du: i32, dv: i32| -> (i32, i32, i32, i32) {
            match side {
                0 => (r.x + u0, r.y + v0, du, dv),
                1 => (r.x + r.w - v0 - dv, r.y + u0, dv, du),
                2 => (r.x + u0, r.y + r.h - v0 - dv, du, dv),
                _ => (r.x + v0, r.y + u0, dv, du),
            }
        };
        // Середина стены, потом сдвиги — где свободно.
        let mid = (len - zone_w) / 2;
        let mut tries: Vec<i32> = vec![mid];
        for d in 1..=(len - zone_w - 1).max(0) {
            tries.push(mid - d);
            tries.push(mid + d);
        }
        for u0 in tries.into_iter().filter(|&u| u >= 1 && u + zone_w < len) {
            let (x, y, w, h) = cell_rect(u0, 0, zone_w, zone_d);
            if !sp.free(x, y, w, h) {
                continue;
            }
            sp.block(x, y, w, h);
            let uc = u0 as f32 + zone_w as f32 / 2.0;
            let c = to_world(uc, WALL_GAP + depth / 2.0);
            drop_pick(m, &fire, [c[0] - (fl + fr) / 2.0, c[1] - (ft + fu) / 2.0], rot, 1.0);
            let rug_rot = side as f32 * 90.0;
            let [rl, rt, rr, ru] = bounds(&carpet, rug, rug_rot, 1.0);
            let rug_depth = if side % 2 == 0 { ru - rt } else { rr - rl };
            let rc = to_world(uc, WALL_GAP + depth + 0.2 + rug_depth.min(2.4) / 2.0);
            drop_pick(m, &carpet, [rc[0] - (rl + rr) / 2.0, rc[1] - (rt + ru) / 2.0], rug_rot, 1.0);
            // Кресла лицом к огню: спинкой от стены.
            let crot = (side as f32 * 90.0 + 180.0 + chair.turn) % 360.0;
            let [cl, ct, cr, cu] = bounds(&chair, "armchair", crot, FURNITURE);
            for du in [-1.25f32, 1.25] {
                let cc = to_world(uc + du, WALL_GAP + depth + 1.9);
                drop_pick(m, &chair, [cc[0] - (cl + cr) / 2.0, cc[1] - (ct + cu) / 2.0], crot, FURNITURE);
            }
            return true;
        }
    }
    false
}

fn shuffle_i32(v: &mut [i32], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.int(0, i as i32) as usize;
        v.swap(i, j);
    }
}

/// Вдоль стен зала: скамьи, бочки, полки с добром, цветы — не на проходах и не у дверей.
fn dress_walls(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room) {
    let mut names: Vec<&str> = vec!["bench", "barrel", "plant_pot", "barrel", "crate", "bench"];
    if !missing("shelf_full") {
        names.push("shelf_full");
    }
    if !missing("storage_pile") {
        names.push("storage_pile");
        names.push("storage_pile");
    }
    for _ in 0..(r.w + r.h) / 4 {
        let n = pick(rng, &names);
        let scale = if ["shelf_full", "storage_pile"].contains(&n) { 1.0 } else { FURNITURE };
        by_wall(m, sp, rng, r, n, scale, false);
    }
}

/// Двор у дома: что нужно хозяйству, вокруг точки `c`; чего нет в паках — без того.
fn yard(m: &mut Map, sp: &mut Spots, rng: &mut Rng, names: &[&str], c: [f32; 2], avoid: &[Path]) {
    for &id in names {
        if missing(id) {
            continue;
        }
        let p = asset(id);
        let rot = rng.int(0, 3) as f32 * 90.0;
        let [l, t, r, u] = bounds(&p, id, rot, 1.0);
        let (fw, fh) = (cells_of(r - l), cells_of(u - t));
        for _ in 0..40 {
            let a = rng.random() as f32 * std::f32::consts::TAU;
            let d = 0.5 + rng.random() as f32 * 3.5;
            let (x, y) = ((c[0] + a.cos() * d - fw as f32 / 2.0).floor() as i32, (c[1] + a.sin() * d - fh as f32 / 2.0).floor() as i32);
            if x < 0 || y < 0 || x + fw > m.width as i32 || y + fh > m.height as i32 {
                continue;
            }
            let centre = [x as f32 + fw as f32 / 2.0, y as f32 + fh as f32 / 2.0];
            if avoid.iter().any(|p| p.distance(centre) < 1.6) {
                continue;
            }
            if put_pick(m, sp, &p, id, x, y, rot, 1.0) {
                break;
            }
        }
    }
}

/// Проход: клетки от `a` до `b` уголком — сперва прямо по `y` (от двери вглубь),
/// потом вдоль `x`; на них мебель не встанет.
fn walkway(sp: &mut Spots, a: [f32; 2], b: [f32; 2], inside: Room) {
    let (ax, ay) = (a[0].floor() as i32, a[1].floor() as i32);
    let (bx, by) = (b[0].floor() as i32, b[1].floor() as i32);
    let keep = |x: i32, y: i32| x >= inside.x && y >= inside.y && x < inside.x + inside.w && y < inside.y + inside.h;
    for y in ay.min(by)..=ay.max(by) {
        if keep(ax, y) {
            sp.block(ax, y, 1, 1);
        }
    }
    for x in ax.min(bx)..=ax.max(bx) {
        if keep(x, by) {
            sp.block(x, by, 1, 1);
        }
    }
}

fn furnish(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, role: Role) {
    let big = r.w.min(r.h) >= 6;
    let f = FURNITURE;
    if big && matches!(role, Role::Shrine | Role::Treasure | Role::Entrance) {
        for (x, y) in [(r.x + 1, r.y + 1), (r.x + r.w - 2, r.y + 1), (r.x + 1, r.y + r.h - 2), (r.x + r.w - 2, r.y + r.h - 2)] {
            put(m, sp, "pillar", x, y, 0.0, 1.0);
        }
    }
    match role {
        Role::Auto | Role::Empty => {}
        Role::Entrance => {
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
            inside(m, sp, rng, r, "rubble", 1.0);
        }
        Role::Exit => {
            // Лестница нужна всегда: в тесной комнате она меньше и стоит посередине.
            if !by_wall(m, sp, rng, r, "stairs", 1.0, false) && !by_wall(m, sp, rng, r, "stairs", 0.7, false) {
                drop(m, "stairs", [r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0], 0.0, 0.6);
            }
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
        }
        Role::Treasure => {
            by_wall(m, sp, rng, r, "chest", f, true);
            by_wall(m, sp, rng, r, "chest_open", f, false);
            by_wall(m, sp, rng, r, "statue", f, false);
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
        }
        Role::Shrine => {
            by_wall(m, sp, rng, r, "altar", f, false);
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
            by_wall(m, sp, rng, r, "brazier", 1.0, false);
            by_wall(m, sp, rng, r, "statue", f, true);
        }
        Role::Barracks => {
            for _ in 0..(r.w * r.h / 9).clamp(1, 5) {
                by_wall(m, sp, rng, r, "bed", f, false);
            }
            by_wall(m, sp, rng, r, "chest_empty", f, true);
        }
        Role::Storage => {
            // Груды FA — уже сложенные ящики и бочки; между ними россыпью свои.
            let piles = !missing("storage_pile");
            for _ in 0..(r.w * r.h / 4).clamp(3, 12) {
                if piles && rng.chance(0.5) {
                    by_wall(m, sp, rng, r, "storage_pile", 1.0, false);
                } else {
                    let n = pick(rng, &["crate", "barrel", "sacks", "crate"]);
                    by_wall(m, sp, rng, r, n, f, false);
                }
            }
        }
        Role::Library => {
            // В комнате неровной формы стены заняты проходами: тогда полки встают внутри.
            let mut shelves = 0;
            for _ in 0..(r.w + r.h) / 3 {
                shelves += by_wall(m, sp, rng, r, "bookshelf", 1.0, false) as i32;
            }
            for _ in shelves..2 {
                inside(m, sp, rng, r, "bookshelf", 1.0);
            }
            let (x, y) = (r.x + r.w / 2 - 1, r.y + r.h / 2);
            if put(m, sp, "desk", x, y, 0.0, f) {
                put(m, sp, "chair", x + 1, y - 1, 0.0, f);
            } else if inside(m, sp, rng, r, "desk", f) {
                inside(m, sp, rng, r, "chair", f);
            }
            by_wall(m, sp, rng, r, "chest", f, true);
        }
        Role::Armoury => {
            for _ in 0..(r.w + r.h) / 4 {
                let n = pick(rng, &["weapon_rack", "weapon_rack_spears", "weapon_rack_axes", "weapon_rack_maces", "weapon_rack_bows", "weapon_rack_mixed"]);
                by_wall(m, sp, rng, r, n, 1.0, false);
            }
            by_wall(m, sp, rng, r, "anvil", f, false);
        }
        Role::Ruin => {
            for _ in 0..(r.w * r.h / 6).clamp(1, 6) {
                let n = pick(rng, &["rubble", "rubble", "boulder"]);
                by_wall(m, sp, rng, r, n, 1.0, false);
            }
        }
        Role::Kitchen => {
            by_wall(m, sp, rng, r, "fireplace", 1.0, false);
            by_wall(m, sp, rng, r, "cauldron", f, false);
            if !missing("shelf_full") {
                by_wall(m, sp, rng, r, "shelf_full", 1.0, false);
            }
            if !missing("storage_pile") {
                by_wall(m, sp, rng, r, "storage_pile", 1.0, true);
            }
            for n in ["barrel", "sacks", "crate", "barrel"] {
                by_wall(m, sp, rng, r, n, f, true);
            }
            // Рабочий стол посередине — с утварью, если она есть.
            if !dining_free_table(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2) {
                put(m, sp, "table", r.x + r.w / 2 - 1, r.y + r.h / 2, 0.0, f);
            }
        }
        Role::Bedroom => {
            let bed = pick(rng, &["bed", "bed_double"]);
            by_wall(m, sp, rng, r, bed, f, true);
            by_wall(m, sp, rng, r, "chest", f, false);
            by_wall(m, sp, rng, r, "wardrobe", f, true);
            if !missing("shelf_full") && rng.chance(0.5) {
                by_wall(m, sp, rng, r, "shelf_full", 1.0, false);
            }
            if r.w * r.h >= 30 {
                by_wall(m, sp, rng, r, "desk", f, false);
                by_wall(m, sp, rng, r, "bed", f, true);
            }
            if r.w * r.h >= 48 {
                by_wall(m, sp, rng, r, "bed", f, true);
                by_wall(m, sp, rng, r, "chest_empty", f, false);
                put(m, sp, "rug_round", r.x + r.w / 2 - 1, r.y + r.h / 2 - 1, 0.0, 1.3);
            }
        }
        Role::Hall | Role::Living => {
            // Угол у камина — у любой стены, кроме фасада (там вход).
            let cosy = hearth(m, sp, rng, r, &[0, 1, 3]) || hearth(m, sp, rng, r, &[2]);
            if !cosy {
                by_wall(m, sp, rng, r, "fireplace", 1.0, false);
            }
            let shelf = if !missing("shelf_full") && rng.chance(0.6) { "shelf_full" } else { "bookshelf" };
            by_wall(m, sp, rng, r, shelf, 1.0, false);
            by_wall(m, sp, rng, r, "sofa", f, false);
            if !cosy {
                by_wall(m, sp, rng, r, "armchair", f, true);
            }
            by_wall(m, sp, rng, r, "plant_pot", f, true);
            if !missing("storage_pile") && rng.chance(0.5) {
                by_wall(m, sp, rng, r, "storage_pile", 1.0, true);
            }
            if !cosy && r.w >= 6 && r.h >= 6 {
                put(m, sp, "rug", r.x + r.w / 2 - 2, r.y + r.h / 2 - 1, 0.0, 1.0);
            }
        }
    }
}

/// Обязательные объекты комнаты из плана: чего в ней ещё нет — ставится у стены
/// или внутри; что не влезло, уходит в предупреждения.
fn wanted(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, names: &[String], who: &str, warn: &mut Vec<String>) {
    for name in names {
        let Some(id) = plan::object_id(name) else { continue };
        let here = |p: &Prop| fits(id, &p.asset) && p.pos[0] >= r.x as f32 && p.pos[0] <= (r.x + r.w) as f32 && p.pos[1] >= r.y as f32 && p.pos[1] <= (r.y + r.h) as f32;
        if m.props.iter().any(here) {
            continue;
        }
        let s = Palette::builtin_pick(id).size;
        let scale = if s[0].max(s[1]) >= 3.0 { 1.0 } else { FURNITURE };
        if !by_wall(m, sp, rng, r, id, scale, false) && !inside(m, sp, rng, r, id, scale) && !inside(m, sp, rng, r, id, 1.0) {
            warn.push(format!("{who}: объект «{}» не поместился", library::label(&key(id))));
        }
    }
}

/// Проём на стене с нужными створкой и косяком. Пустой проём остаётся с косяком,
/// тайный ход — ничем не выдаёт себя.
fn portal(a: [f32; 2], b: [f32; 2], kind: PortalKind, wood: bool) -> Portal {
    let style = if wood { "wood" } else { "stone" };
    let len = ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()).round() as i32;
    let asset = match kind {
        PortalKind::Window => Some(asset(&format!("window_{style}")).key),
        PortalKind::Door | PortalKind::DoubleDoor => Some(asset("door_wood").key),
        PortalKind::Opening | PortalKind::Secret => None,
    };
    let frame = match kind {
        PortalKind::Window => asset.as_deref().and_then(|k| PALETTE.with(|p| p.borrow().as_ref().and_then(|pal| pal.window_frame(k)))),
        PortalKind::Secret => None,
        _ => Some(key(&format!("frame_{style}_{}", len.clamp(1, 3)))),
    };
    Portal { a, b, kind, asset, frame }
}

fn opening(a: [f32; 2], b: [f32; 2]) -> Portal {
    Portal { a, b, kind: PortalKind::Opening, asset: None, frame: None }
}

/// Вид подземелья по умолчанию: ставится при открытии окна, потом его можно поменять.
pub fn dungeon_look(m: &mut Map) -> usize {
    m.wall_style = wall("wall_stone_blocks");
    mat(m, "dungeon_flagstones")
}

/// Вокруг помещений темно: скала не спорит с комнатами.
fn dark_outside(m: &mut Map, level: f32) {
    let dark: Vec<(usize, f32)> = m.rooms.data.iter().enumerate().filter(|(_, v)| **v < 128).map(|(i, _)| (i, 1.0)).collect();
    m.shade_paint(&dark, level);
}

/// Подпись комнаты из плана — посередине неё.
fn label(m: &mut Map, spec: &plan::RoomSpec, r: Room) {
    if !spec.name.trim().is_empty() {
        let mut l = Label::new(spec.name.trim().to_owned(), [r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0]);
        l.size = 0.5;
        // Название комнаты — подсказка ведущему: в выгрузку для игроков оно не идёт.
        l.gm = true;
        m.labels.push(l);
    }
}

/// Пределы ширины и высоты комнаты подземелья по плану.
fn dungeon_room_size(sz: Size, lo: i32, hi: i32) -> ([i32; 2], [i32; 2]) {
    let third = (hi - lo) / 3;
    match sz {
        Size::Cells([w, h]) => ([w as i32, h as i32], [w as i32, h as i32]),
        Size::Named(SizeName::Small) => ([lo, lo], [lo + third, lo + third]),
        Size::Named(SizeName::Medium) => ([lo, lo], [hi, hi]),
        Size::Named(SizeName::Large) => ([hi - third, hi - third], [hi, hi]),
    }
}

fn dungeon(m: &mut Map, p: &Plan, material: usize, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    let [lo, hi] = p.options.room_size;
    let (lo, hi) = (lo.max(2) as i32, hi.max(lo.max(2)) as i32);
    let sizes: Vec<_> = p.rooms.iter().map(|r| dungeon_room_size(r.size, lo, hi)).collect();
    let links: Vec<(usize, usize)> = p.inner_links().filter_map(|l| Some((p.room_index(&l.from)?, p.room_index(&l.to)?))).collect();
    let (mut cells, rooms, corridors) = generate::dungeon_with(m.width, m.height, &sizes, (!p.links.is_empty()).then_some(&links[..]), p.options.loops, &mut Rng::new(&p.seed));
    let material = p.style.floor.as_deref().and_then(plan::texture_id).map_or(material, |t| mat(m, t));
    if let Some(st) = p.style.walls.as_deref().and_then(plan::wall_id) {
        m.wall_style = wall(st);
    }
    ground(m, "rocky_ground");
    // Безликая комната без связей — пожелание о числе комнат, а не требование.
    for spec in p.rooms[rooms.len()..].iter().filter(|r| r.role != Role::Auto || !r.name.is_empty() || !r.objects.is_empty() || p.links.iter().any(|l| l.touches(&r.id))) {
        warn.push(format!("комната «{}» не поместилась", spec.id));
    }
    if rooms.is_empty() {
        return 0;
    }
    // Коридор в клетку при толстых стенах — щель: ширим до двух.
    let was = cells.clone();
    let in_room = |x: i32, y: i32| rooms.iter().any(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h);
    let mut wide: Vec<Vec<bool>> = corridors.iter().map(|_| vec![false; cells.len()]).collect();
    for (c, w2) in corridors.iter().zip(&mut wide) {
        for &i in &c.cells {
            w2[i] = true;
        }
    }
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            if was[(y * w + x) as usize] && !in_room(x, y) {
                for (i, j) in [(1, 0), (0, 1), (1, 1)] {
                    cells[((y + j) * w + x + i) as usize] = true;
                }
            }
        }
    }
    for (c, w2) in corridors.iter().zip(&mut wide) {
        for &i in &c.cells {
            let (x, y) = (i as i32 % w, i as i32 / w);
            if in_room(x, y) {
                continue;
            }
            for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
                if x + dx < w && y + dy < h {
                    w2[((y + dy) * w + x + dx) as usize] = true;
                }
            }
        }
    }
    // Вход — ворота на краю карты: комната входа по плану, иначе ближайшая к краю.
    let reach = |r: &Room| [r.y, r.x, h - r.y - r.h, w - r.x - r.w];
    let first = p.entrance().filter(|&i| i < rooms.len() && (p.rooms[i].role == Role::Entrance || p.links.iter().any(|l| l.touches(OUTSIDE)))).unwrap_or_else(|| (0..rooms.len()).min_by_key(|&i| *reach(&rooms[i]).iter().min().unwrap_or(&0)).unwrap_or(0));
    let r = rooms[first];
    let side = (0..4).min_by_key(|&s| reach(&r)[s]).unwrap_or(0);
    let (cx, cy) = r.center();
    let mut set = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            cells[(y * w + x) as usize] = true;
        }
    };
    let gate = match side {
        0 => {
            (0..r.y).for_each(|y| [cx, cx + 1].iter().for_each(|&x| set(x, y)));
            ([cx as f32, 0.0], [cx as f32 + 2.0, 0.0])
        }
        1 => {
            (0..r.x).for_each(|x| [cy, cy + 1].iter().for_each(|&y| set(x, y)));
            ([0.0, cy as f32], [0.0, cy as f32 + 2.0])
        }
        2 => {
            (r.y + r.h..h).for_each(|y| [cx, cx + 1].iter().for_each(|&x| set(x, y)));
            ([cx as f32, h as f32], [cx as f32 + 2.0, h as f32])
        }
        _ => {
            (r.x + r.w..w).for_each(|x| [cy, cy + 1].iter().for_each(|&y| set(x, y)));
            ([w as f32, cy as f32], [w as f32, cy as f32 + 2.0])
        }
    };
    let (mw, mh, cw) = (m.rooms.w, m.rooms.h, w as usize);
    let weights: Vec<(usize, f32)> = cells
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .flat_map(|(k, _)| {
            let c = ((k % cw) as i32, (k / cw) as i32);
            mask::cells(mw, mh, c, c)
        })
        .collect();
    m.paint(Layer::Rooms, material, &weights);
    for (spec, r) in p.rooms.iter().zip(&rooms) {
        if let Some(t) = spec.floor.as_deref().and_then(plan::texture_id) {
            let t = mat(m, t);
            m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(t));
        }
    }
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    dark_outside(m, if p.mood == Mood::Gloomy { 0.95 } else { 0.8 });
    let gate_kind = p.links.iter().find(|l| l.touches(OUTSIDE)).map_or(LinkKind::DoubleDoor, |l| l.kind);
    match gate_kind {
        LinkKind::Opening => m.portals.push(opening(gate.0, gate.1)),
        LinkKind::Secret => m.portals.push(portal(gate.0, gate.1, PortalKind::Secret, false)),
        LinkKind::Door => m.portals.push(portal(gate.0, gate.1, PortalKind::Door, false)),
        LinkKind::DoubleDoor | LinkKind::Bridge => m.portals.push(portal(gate.0, gate.1, PortalKind::DoubleDoor, false)),
    }
    for (spec, r) in p.rooms.iter().zip(&rooms) {
        label(m, spec, *r);
    }
    if p.options.fill {
        furnish_dungeon(m, &cells, &rooms, first, rng, p, &corridors, &wide, warn);
    }
    rooms.len()
}

/// Двери на входах коридоров, объекты по назначению комнат.
#[allow(clippy::too_many_arguments)]
fn furnish_dungeon(m: &mut Map, cells: &[bool], rooms: &[Room], first: usize, rng: &mut Rng, p: &Plan, corridors: &[Corridor], wide: &[Vec<bool>], warn: &mut Vec<String>) {
    let (w, h) = (m.width as i32, m.height as i32);
    let mut sp = Spots::new(m);
    let in_room = |x: i32, y: i32| rooms.iter().any(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h);
    let corridor = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && cells[(y * w + x) as usize] && !in_room(x, y);
    // Чей коридор выходит в эту клетку у комнаты `k` — и какой связью он задан.
    let link_at = |k: usize, x: i32, y: i32| -> Option<LinkKind> {
        let i = (y * w + x) as usize;
        let c = corridors.iter().zip(wide).find(|(c, w2)| (c.a == k || c.b == k) && w2[i]).map(|(c, _)| c)?;
        let (a, b) = (&p.rooms[c.a].id, &p.rooms[c.b].id);
        p.links.iter().find(|l| (l.from == *a && l.to == *b) || (l.from == *b && l.to == *a)).map(|l| l.kind)
    };
    for (k, r) in rooms.iter().enumerate() {
        // Вход коридора — ровно две клетки у стены комнаты; шире — это уже не проём.
        let mut doors = Vec::new();
        for x in r.x..r.x + r.w {
            for (y, out, inner) in [(r.y, r.y - 1, r.y), (r.y + r.h, r.y + r.h, r.y + r.h - 1)] {
                if corridor(x, out) && !corridor(x - 1, out) {
                    let pair = corridor(x + 1, out) && !corridor(x + 2, out);
                    doors.push((pair, [x as f32, y as f32], [x as f32 + 2.0, y as f32], x, inner, link_at(k, x, out)));
                }
            }
        }
        for y in r.y..r.y + r.h {
            for (x, out, inner) in [(r.x, r.x - 1, r.x), (r.x + r.w, r.x + r.w, r.x + r.w - 1)] {
                if corridor(out, y) && !corridor(out, y - 1) {
                    let pair = corridor(out, y + 1) && !corridor(out, y + 2);
                    doors.push((pair, [x as f32, y as f32], [x as f32, y as f32 + 2.0], inner, y, link_at(k, out, y)));
                }
            }
        }
        for (pair, a, b, x, y, link) in doors {
            sp.block(x - 1, y - 1, 4, 4);
            let one = [if a[0] == b[0] { a[0] } else { a[0] + 1.0 }, if a[1] == b[1] { a[1] } else { a[1] + 1.0 }];
            match link {
                None => {
                    if pair && rng.chance(0.6) {
                        m.portals.push(portal(a, b, PortalKind::DoubleDoor, false));
                    }
                }
                Some(LinkKind::Opening) => {}
                Some(LinkKind::Secret) => m.portals.push(portal(a, if pair { b } else { one }, PortalKind::Secret, false)),
                Some(LinkKind::DoubleDoor) if pair => m.portals.push(portal(a, b, PortalKind::DoubleDoor, false)),
                Some(LinkKind::Door) if pair => m.portals.push(portal(a, b, PortalKind::DoubleDoor, false)),
                Some(_) => m.portals.push(portal(a, one, PortalKind::Door, false)),
            }
        }
    }
    // Чем дальше от входа, тем важнее комната: в самой дальней лестница вниз, перед ней клад.
    // Роли из плана главнее.
    let (ox, oy) = rooms[first].center();
    let mut order: Vec<usize> = (0..rooms.len()).collect();
    order.sort_by_key(|&i| {
        let (x, y) = rooms[i].center();
        (x - ox).abs() + (y - oy).abs()
    });
    let last = order.len() - 1;
    let has_exit = p.rooms.iter().any(|r| r.role == Role::Exit);
    for (k, &i) in order.iter().enumerate() {
        let given = p.rooms[i].role;
        let role = if given != Role::Auto {
            given
        } else if k == 0 {
            Role::Entrance
        } else if k == last && !has_exit {
            Role::Exit
        } else if k + 1 == last && !has_exit {
            Role::Treasure
        } else {
            [Role::Shrine, Role::Barracks, Role::Storage, Role::Library, Role::Armoury, Role::Ruin, Role::Storage, Role::Ruin][rng.int(0, 7) as usize]
        };
        furnish(m, &mut sp, rng, rooms[i], role);
        if p.mood == Mood::Abandoned {
            furnish(m, &mut sp, rng, rooms[i], Role::Ruin);
        }
        wanted(m, &mut sp, rng, rooms[i], &p.rooms[i].objects, &format!("комната «{}»", p.rooms[i].id), warn);
    }
    wanted(m, &mut sp, rng, rooms[first], &p.objects, "подземелье", warn);
}

/// Карта с чистого листа: палитра и вид стен остаются.
fn clear(m: &mut Map) {
    let fresh = Map::new(m.width, m.height);
    m.terrain = fresh.terrain;
    m.rooms = fresh.rooms;
    m.room_floor = fresh.room_floor;
    m.walls.clear();
    m.wall_looks.clear();
    m.edge_looks.clear();
    m.portals.clear();
    m.props.clear();
    m.paths.clear();
    m.labels.clear();
    m.roofs.clear();
    m.shade = None;
    m.shadows = Shadows { on: true, angle: 45.0, dist: 0.7, blur: 0.3, opacity: 0.6, fade: 0.7 };
}

/// Разом все круги одним мазком: `Map::paint` копирует маску, по кругу на вызов дорого.
fn blobs(m: &Map, spots: &[([f32; 2], f32)], hardness: f32, flow: f32) -> Vec<(usize, f32)> {
    let (w, h) = (m.rooms.w, m.rooms.h);
    let mut all = vec![0.0f32; w * h];
    for &(c, radius) in spots {
        let b = Brush { shape: Shape::Circle, radius, hardness, flow };
        for (i, v) in mask::dab(w, h, &b, c, 0) {
            all[i] = 1.0 - (1.0 - all[i]) * (1.0 - v);
        }
    }
    all.iter().enumerate().filter(|(_, v)| **v > 0.0).map(|(i, v)| (i, *v)).collect()
}

fn ground(m: &mut Map, name: &str) {
    let g = mat(m, name);
    let (w, h) = (m.width as i32, m.height as i32);
    m.paint_rect(Layer::Floor, (0, 0), (w - 1, h - 1), Some(g));
}

fn patches(m: &mut Map, rng: &mut Rng, name: &str, count: i32, r: (f32, f32), flow: f32) {
    let g = mat(m, name);
    let (w, h) = (m.width as f32, m.height as f32);
    let mut spots = Vec::new();
    for _ in 0..count {
        let c = [rng.random() as f32 * w, rng.random() as f32 * h];
        spots.push((c, r.0 + rng.random() as f32 * (r.1 - r.0)));
    }
    let weights = blobs(m, &spots, 0.2, flow);
    m.paint(Layer::Floor, g, &weights);
}

/// Объекты плана вокруг точки: по кругу, не ближе `keep` к тропам, на свободное место.
fn around(m: &mut Map, sp: &mut Spots, rng: &mut Rng, names: &[String], c: [f32; 2], who: &str, avoid: &[Path], keep: f32, warn: &mut Vec<String>) {
    for name in names {
        let Some(id) = plan::object_id(name) else { continue };
        let p = asset(id);
        let [l, t, r, u] = bounds(&p, id, 0.0, 1.0);
        let (fw, fh) = (cells_of(r - l), cells_of(u - t));
        let mut done = false;
        for k in 0..90 {
            let a = rng.random() as f32 * std::f32::consts::TAU;
            // Тесно у точки — ищем дальше: каждые 30 попыток кольцо шире.
            let d = 1.5 + rng.random() as f32 * 4.0 + (k / 30) as f32 * 2.5;
            let (x, y) = ((c[0] + a.cos() * d - fw as f32 / 2.0).floor() as i32, (c[1] + a.sin() * d - fh as f32 / 2.0).floor() as i32);
            let centre = [x as f32 + fw as f32 / 2.0, y as f32 + fh as f32 / 2.0];
            if avoid.iter().any(|p| p.distance(centre) < keep + (p.width / 2.0 - 1.0).max(0.0)) {
                continue;
            }
            if put_pick(m, sp, &p, id, x, y, 0.0, 1.0) {
                done = true;
                break;
            }
        }
        if !done {
            warn.push(format!("{who}: объект «{}» не поместился", library::label(&key(id))));
        }
    }
}

fn cave(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let density = p.options.density;
    let (w, h) = (m.width as i32, m.height as i32);
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let mut open = Vec::new();
    // Клеточный автомат: стены собираются в массивы, пустоты — в залы. Берём самую большую пустоту.
    for _ in 0..30 {
        let fill = 0.5 + density as f64 * 0.1;
        let mut c: Vec<bool> = (0..w * h).map(|i| i % w >= 2 && i % w < w - 2 && i / w >= 2 && i / w < h - 2 && rng.chance(fill)).collect();
        for _ in 0..5 {
            let was = c.clone();
            for y in 2..h - 2 {
                for x in 2..w - 2 {
                    let n = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (i, j))).filter(|&(i, j)| was[at(x + i, y + j)]).count();
                    c[at(x, y)] = n >= 5;
                }
            }
        }
        let mut seen = vec![false; c.len()];
        let mut best: Vec<usize> = Vec::new();
        for s in 0..c.len() {
            if !c[s] || seen[s] {
                continue;
            }
            let (mut stack, mut part) = (vec![s], Vec::new());
            seen[s] = true;
            while let Some(i) = stack.pop() {
                part.push(i);
                for n in [i + 1, i.wrapping_sub(1), i + w as usize, i.wrapping_sub(w as usize)] {
                    if n < c.len() && c[n] && !seen[n] {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
            if part.len() > best.len() {
                best = part;
            }
        }
        if best.len() as i32 * 4 >= w * h || open.is_empty() {
            open = best.clone();
        }
        if best.len() as i32 * 3 >= w * h {
            break;
        }
    }
    let mut cells = vec![false; (w * h) as usize];
    for &i in &open {
        cells[i] = true;
    }
    let look = cave_look(p.options.cave);
    ground(m, look.outside);
    if open.is_empty() {
        return 0;
    }

    // Вход и выход: ход шириной в три клетки до края карты на двух противоположных сторонах.
    let along_x = w >= h;
    let mut gates = Vec::new();
    for near in [true, false] {
        let key_of = |i: usize| if along_x { i as i32 % w } else { i as i32 / w };
        let end = if near { open.iter().copied().min_by_key(|&i| key_of(i)) } else { open.iter().copied().max_by_key(|&i| key_of(i)) };
        let Some(end) = end else { continue };
        let (ex, ey) = (end as i32 % w, end as i32 / w);
        let (mid, from, to) = if along_x { (ey.clamp(1, h - 3), ex, if near { 0 } else { w - 1 }) } else { (ex.clamp(1, w - 3), ey, if near { 0 } else { h - 1 }) };
        for t in from.min(to)..=from.max(to) {
            for s in -1..=1 {
                let i = if along_x { at(t, mid + s) } else { at(mid + s, t) };
                cells[i] = true;
            }
        }
        let far = if near { 0.0 } else if along_x { w as f32 } else { h as f32 };
        let (a, b) = (mid as f32 - 1.0, mid as f32 + 2.0);
        gates.push(if along_x { opening([far, a], [far, b]) } else { opening([a, far], [b, far]) });
    }
    let open: Vec<usize> = (0..cells.len()).filter(|&i| cells[i]).collect();
    let cell_pos = |i: usize| [(i as i32 % w) as f32 + 0.5, (i as i32 / w) as f32 + 0.5];
    let floor = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or(look.floor));
    let spots: Vec<_> = open.iter().map(|&i| (cell_pos(i), 0.95)).collect();
    let weights = blobs(m, &spots, 0.8, 1.0);
    m.paint(Layer::Rooms, floor, &weights);
    m.wall_style = wall(p.style.walls.as_deref().and_then(plan::wall_id).unwrap_or(look.wall));
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    dark_outside(m, if p.mood == Mood::Gloomy { 0.95 } else { 0.8 });
    m.portals = gates;

    // Внутренние клетки: от них до стены хотя бы две клетки.
    let deep: Vec<usize> = open
        .iter()
        .copied()
        .filter(|&i| {
            let (x, y) = (i as i32 % w, i as i32 / w);
            (-2..=2).all(|j| (-2..=2).all(|k| x + k >= 0 && y + j >= 0 && x + k < w && y + j < h && cells[at(x + k, y + j)]))
        })
        .collect();
    let mut sp = Spots::new(m);
    for i in 0..cells.len() {
        if !cells[i] {
            sp.used[i] = true;
        }
    }
    let inner = |m: &Map, weights: Vec<(usize, f32)>| -> Vec<(usize, f32)> { weights.into_iter().filter(|(i, _)| m.rooms.data[*i] > 200).collect() };
    for (name, count, radius, flow) in look.patches {
        let g = mat(m, name);
        let mut spots = Vec::new();
        for _ in 0..count {
            spots.push((cell_pos(open[rng.int(0, open.len() as i32 - 1) as usize]), radius));
        }
        let weights = inner(m, blobs(m, &spots, 0.2, flow));
        m.paint(Layer::Rooms, g, &weights);
    }
    let mut shores = Vec::new();
    if !deep.is_empty()
        && let Some((shallow, depth)) = look.pool
    {
        let (shallow, depth) = (mat(m, shallow), mat(m, depth));
        for _ in 0..rng.int(look.pools[0], look.pools[1]) {
            let c = cell_pos(deep[rng.int(0, deep.len() as i32 - 1) as usize]);
            let mut lobes = Vec::new();
            for _ in 0..4 {
                let (dx, dy) = (rng.random() as f32 * 2.0 - 1.0, rng.random() as f32 * 2.0 - 1.0);
                lobes.push(([c[0] + dx * look.pool_size, c[1] + dy * look.pool_size], (1.1 + rng.random() as f32 * 0.6) * look.pool_size));
            }
            let weights = inner(m, blobs(m, &lobes, 0.6, 1.0));
            m.paint(Layer::Rooms, shallow, &weights);
            let weights = inner(m, blobs(m, &[(c, 0.9 * look.pool_size)], 0.3, 0.9));
            m.paint(Layer::Rooms, depth, &weights);
            let half = (3.0 * look.pool_size).round() as i32;
            sp.block(c[0] as i32 - half, c[1] as i32 - half, half * 2 + 1, half * 2 + 1);
            shores.push(c);
        }
    }
    // У воды — рогоз, у грибной — грибы: кольцом по берегу, за пределами водоёма.
    if let Some(s) = look.shore.filter(|s| !missing(s)) {
        for c in &shores {
            for k in 0..rng.int(4, 7) {
                let a = k as f32 * 1.1 + rng.random() as f32 * 0.8;
                let r = 3.6 * look.pool_size + rng.random() as f32 * 0.9;
                let (x, y) = ((c[0] + a.cos() * r) as i32, (c[1] + a.sin() * r) as i32);
                if x >= 0 && y >= 0 && x < w && y < h && cells[at(x, y)] {
                    put(m, &mut sp, s, x, y, rng.int(0, 3) as f32 * 90.0, 0.8 + rng.random() as f32 * 0.4);
                }
            }
        }
    }
    let scatter = (open.len() as f32 * (0.03 + density * 0.05) * look.clutter) as i32;
    let things: Vec<&str> = look.scatter.iter().copied().filter(|s| !missing(s)).collect();
    let things = if things.is_empty() { vec!["boulder"] } else { things };
    for _ in 0..scatter {
        let i = open[rng.int(0, open.len() as i32 - 1) as usize] as i32;
        let n = pick(rng, &things);
        put(m, &mut sp, n, i % w, i / w, 0.0, 0.7 + rng.random() as f32 * 0.5);
        // Грибы, кристаллы, рогоз и ледяные сталагмиты растут кучками.
        if n != "boulder" && n != "rubble" {
            for _ in 0..rng.int(1, 3) {
                let (x, y) = (i % w + rng.int(-1, 1), i / w + rng.int(-1, 1));
                if x >= 0 && y >= 0 && x < w && y < h && cells[at(x, y)] {
                    put(m, &mut sp, n, x, y, rng.int(0, 3) as f32 * 90.0, 0.6 + rng.random() as f32 * 0.4);
                }
            }
        }
    }
    // Стоянка в глубине и клад там, куда дальше всего идти.
    if !deep.is_empty() {
        let c = deep[rng.int(0, deep.len() as i32 - 1) as usize] as i32;
        let (x, y) = (c % w, c / w);
        if put(m, &mut sp, "campfire", x, y, 0.0, 1.0) {
            for (i, j, n) in [(-1, -1, "sacks"), (1, -1, "crate"), (0, 1, "barrel")] {
                put(m, &mut sp, n, x + i, y + j, 0.0, 0.9);
            }
        }
        let far = open.iter().copied().max_by_key(|&i| ((i as i32 % w) - x).abs() + ((i as i32 / w) - y).abs());
        if let Some(f) = far {
            put(m, &mut sp, "chest", f as i32 % w, f as i32 / w, 0.0, 1.0);
        }
        around(m, &mut sp, rng, &p.objects, [x as f32 + 0.5, y as f32 + 0.5], "пещера", &[], 0.0, warn);
    } else if !p.objects.is_empty() {
        warn.push("пещера: негде поставить объекты, нет глубины".into());
    }
    open.len()
}

/// Спокойная кривая: одна-две широкие дуги и немного неровности, без зигзага.
fn road(rng: &mut Rng, a: [f32; 2], b: [f32; 2], sway: f32) -> Vec<[f32; 2]> {
    let n = 5;
    let phase = rng.random() as f32 * std::f32::consts::TAU;
    let waves = 0.6 + rng.random() as f32 * 0.5;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy).max(1.0);
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let off = ((t * waves * std::f32::consts::TAU + phase).sin() - phase.sin() * (1.0 - t) - (waves * std::f32::consts::TAU + phase).sin() * t) * sway + (rng.random() as f32 - 0.5) * sway * 0.2 * ((t * std::f32::consts::PI).sin());
            [a[0] + dx * t - dy / len * off, a[1] + dy * t + dx / len * off]
        })
        .collect()
}

/// Где линии сходятся ближе всего и куда там идёт первая, в градусах.
fn crossing(a: &Path, b: &Path) -> Option<([f32; 2], f32)> {
    let line = a.curve();
    let i = (1..line.len().saturating_sub(1)).min_by(|&i, &j| b.distance(line[i]).total_cmp(&b.distance(line[j])))?;
    (b.distance(line[i]) < 1.0).then(|| {
        let (p, q) = (line[i.saturating_sub(3)], line[(i + 3).min(line.len() - 1)]);
        (line[i], (q[1] - p[1]).atan2(q[0] - p[0]).to_degrees())
    })
}

/// Река с галечным берегом и глубиной посередине: три полосы по одной линии.
/// `width` — ширина воды в клетках. Река от четырёх клеток: уже — рыба «Живой
/// карты» (около клетки длиной) в ней не помещается, а глубины не остаётся вовсе.
struct Stream {
    bank: Path,
    water: Path,
    deep: Path,
}

impl Stream {
    fn paths(&self) -> [Path; 3] {
        [self.bank.clone(), self.water.clone(), self.deep.clone()]
    }

    /// Ближе `d` клеток к краю берега.
    fn near(&self, p: [f32; 2], d: f32) -> bool {
        self.bank.distance(p) < self.bank.width / 2.0 + d
    }
}

fn stream(m: &mut Map, line: Vec<[f32; 2]>, width: f32) -> Stream {
    let (water, deep, gravel) = (mat(m, "water_shallow"), mat(m, "water_deep"), mat(m, "gravel"));
    let bank = Path { points: line.clone(), material: gravel, width: width + 1.6, soft: 0.6, rough: 0.35, along: false };
    let deep = Path { points: line.clone(), material: deep, width: (width * 0.45).max(1.2), soft: 0.8, rough: 0.35, along: false };
    let water = Path { points: line, material: water, width, soft: 0.45, rough: 0.3, along: false };
    Stream { bank, water, deep }
}

/// Мост через реку там, где `path` её пересекает: поперёк течения, серединой на
/// стрежне, концами на сухом за галькой. Картинка тянется по длине под ширину
/// реки (до полутора раз), а сверх того — растёт целиком.
fn bridge(m: &mut Map, path: &Path, river: &Stream) -> bool {
    let Some((_, along)) = crossing(path, &river.water) else { return false };
    let line = river.water.curve();
    let near = |q: [f32; 2]| path.distance(q);
    let Some(i) = (1..line.len().saturating_sub(1)).min_by(|&a, &b| near(line[a]).total_cmp(&near(line[b]))) else { return false };
    let (p, q) = (line[i.saturating_sub(2)], line[(i + 2).min(line.len() - 1)]);
    let flow = (q[1] - p[1]).atan2(q[0] - p[0]).to_degrees();
    // Из двух поперечных — ближняя к ходу тропы.
    let mut rot = flow + 90.0;
    let diff = ((rot - along).rem_euclid(360.0) + 180.0).rem_euclid(360.0) - 180.0;
    if diff.abs() > 90.0 {
        rot += 180.0;
    }
    let pick = asset("bridge");
    let [l, t, r, u] = bounds(&pick, "bridge", 0.0, 1.0);
    // Длинная сторона картинки — вдоль хода: у встроенного и у FA мост лежит по x.
    let lengthwise = r - l >= u - t;
    let long = if lengthwise { r - l } else { u - t };
    if !lengthwise {
        rot -= 90.0;
    }
    let span = river.bank.width + 1.4;
    let f = span / long.max(0.5);
    let scale = if f > 1.5 { f / 1.5 } else if f < 0.75 { f / 0.75 } else { 1.0 };
    let stretch = f / scale;
    drop_pick(m, &pick, line[i], rot.rem_euclid(360.0), scale);
    if let Some(pr) = m.props.last_mut() {
        pr.size[if lengthwise { 0 } else { 1 }] *= stretch;
    }
    true
}

fn forest(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let density = p.options.density;
    let look = forest_look(p.options.season);
    let (w, h) = (m.width as f32, m.height as f32);
    ground(m, look.ground);
    patches(m, rng, look.patches, (w * h / 60.0) as i32, (2.0, 4.5), 0.6);
    let dirt = mat(m, "dirt");
    let glade = 2.5 + w.min(h) * 0.08;

    let (left, right) = (h * (0.25 + rng.random() as f32 * 0.5), h * (0.25 + rng.random() as f32 * 0.5));
    let line = road(rng, [-1.0, left], [w + 1.0, right], h * 0.14);
    let trail = Path { points: line, material: dirt, width: 2.0, soft: 0.5, rough: 0.25, along: false };
    // Ручей с севера на юг; план выбирает берег или убирает ручей вовсе.
    let river = match p.river {
        River::None => None,
        River::East => Some(true),
        River::West => Some(false),
        _ => Some(rng.chance(0.5)),
    };
    let water_paths = river.map(|side| {
        let x = if side { w * 0.75 } else { w * 0.25 };
        let (top, low) = (x + (rng.random() as f32 - 0.5) * w * 0.15, x + (rng.random() as f32 - 0.5) * w * 0.15);
        let width = (w * 0.12).clamp(4.0, 5.5);
        let line = road(rng, [top, -1.0], [low, h + 1.0], w * 0.09);
        stream(m, line, width)
    });
    // Расстояния считаются от края гальки.
    let near_water = |p: [f32; 2], d: f32| water_paths.as_ref().is_some_and(|s| s.near(p, d - 1.6));

    // Лагерь в стороне от дороги и ручья, к нему идёт своя тропинка.
    let mut camp = [w / 2.0, h / 2.0];
    let mut best = f32::MIN;
    for _ in 0..200 {
        let q = [w * (0.15 + rng.random() as f32 * 0.7), h * (0.15 + rng.random() as f32 * 0.7)];
        let room = water_paths.as_ref().map_or(trail.distance(q), |s| trail.distance(q).min(s.bank.distance(q) - s.bank.width / 2.0 + 1.6));
        let score = -(room - glade - 2.0).abs();
        if score > best {
            best = score;
            camp = q;
        }
    }
    let line = trail.curve();
    let join = line.iter().copied().min_by(|a, b| (a[0] - camp[0]).hypot(a[1] - camp[1]).total_cmp(&(b[0] - camp[0]).hypot(b[1] - camp[1]))).unwrap_or(camp);
    let side_path = Path { points: road(rng, join, camp, 0.8), material: dirt, width: 1.1, soft: 0.6, rough: 0.3, along: false };
    let floor = mat(m, look.under);
    let bare = blobs(m, &[(camp, glade * 0.55)], 0.2, 0.6);
    m.paint(Layer::Floor, dirt, &bare);
    m.paths = match &water_paths {
        Some(s) => s.paths().into_iter().chain([trail.clone(), side_path.clone()]).collect(),
        None => vec![trail.clone(), side_path.clone()],
    };

    // Деревья дротиками: новое ставится, только если не лезет на тропу, ручей и соседей.
    let spacing = 3.4 - density * 1.5;
    let mut trees: Vec<[f32; 2]> = Vec::new();
    let mut tries = (w * h * 3.0) as i32;
    while tries > 0 {
        tries -= 1;
        let q = [rng.random() as f32 * w, rng.random() as f32 * h];
        let far = |c: [f32; 2], d: f32| (c[0] - q[0]).hypot(c[1] - q[1]) >= d;
        if !far(camp, glade + 1.0) || trail.distance(q) < 2.2 || side_path.distance(q) < 1.6 || near_water(q, 2.6) || !trees.iter().all(|&t| far(t, spacing)) {
            continue;
        }
        trees.push(q);
    }
    let under: Vec<_> = trees.iter().map(|&t| (t, 2.4)).collect();
    let shade = blobs(m, &under, 0.15, 0.8);
    m.paint(Layer::Floor, floor, &shade);
    // Сначала низкое, деревья последними: кроны накрывают камни и кусты, а не наоборот.
    let brush = (w * h * (0.02 + density * 0.03) * look.brush) as i32;
    for _ in 0..brush {
        let q = [rng.random() as f32 * w, rng.random() as f32 * h];
        let clear_of_tree = trees.iter().all(|t| (t[0] - q[0]).hypot(t[1] - q[1]) > 1.4);
        if clear_of_tree && trail.distance(q) > 1.5 && side_path.distance(q) > 1.2 && !near_water(q, 2.0) && (q[0] - camp[0]).hypot(q[1] - camp[1]) > glade * 0.7 {
            let n = pick(rng, &[look.bush, look.bush, look.bush, "boulder"]);
            drop(m, n, q, rng.int(0, 3) as f32 * 90.0, 0.6 + rng.random() as f32 * 0.6);
        }
    }
    drop(m, "campfire", camp, 0.0, 1.0);
    for (dx, dy, n) in [(-1.8, 0.6, "sacks"), (1.7, -0.8, "crate"), (0.4, 1.9, "boulder"), (-1.2, -1.6, "barrel")] {
        drop(m, n, [camp[0] + dx, camp[1] + dy], 0.0, 0.9);
    }
    if let Some(s) = &water_paths {
        bridge(m, &trail, s);
    }
    if !p.objects.is_empty() {
        let mut sp = Spots::new(m);
        sp.block(camp[0] as i32 - 2, camp[1] as i32 - 2, 5, 5);
        let avoid: Vec<Path> = m.paths.clone();
        around(m, &mut sp, rng, &p.objects, camp, "лес", &avoid, 1.8, warn);
    }
    if p.mood == Mood::Abandoned {
        abandon(m, rng, &[]);
    }
    for &q in &trees {
        let (rot, scale) = (rng.int(0, 3) as f32 * 90.0, 0.8 + rng.random() as f32 * 0.5);
        let slot = if rng.chance(look.bare) { "tree_bare" } else { tree_slot(p.options.season) };
        drop(m, slot, q, rot, scale);
    }
    trees.len()
}

/// Дом с крышей: комната, стены, дверь и окна на стороне `door` (0 — север, 1 — восток, 2 — юг, 3 — запад).
/// `look` — стены и крыша по плану; без него — случайные из набора.
fn house(m: &mut Map, rng: &mut Rng, sp: &mut Spots, r: Room, door: i32, look: Option<(&str, &str)>) {
    const LOOKS: [(&str, &str, bool); 8] = [
        ("wall_timber", "roof_clay_red", true),
        ("wall_timber", "roof_thatch", true),
        ("wall_wood", "roof_thatch", true),
        ("wall_brick", "roof_clay_brown", true),
        ("wall_stone_blocks", "roof_slate_blue", false),
        ("wall_stone_blocks", "roof_slate_gray", false),
        ("wall_fieldstone", "roof_shingle", false),
        ("wall_wood", "roof_shingle", true),
    ];
    let (st, roof, wood) = match look {
        Some((st, roof)) => (st, roof, !["stone", "brick", "cave", "dwarven", "drow", "metal"].iter().any(|s| st.contains(s))),
        None => LOOKS[rng.int(0, LOOKS.len() as i32 - 1) as usize],
    };
    let planks = mat(m, "wood_planks");
    m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(planks));
    let (ax, ay, bx, by) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
    sp.block(r.x, r.y, r.w, r.h);
    let own: Vec<_> = m.room_edges().into_iter().filter(|s| [s.0, s.1].iter().all(|p| p[0] >= ax && p[0] <= bx && p[1] >= ay && p[1] <= by)).collect();
    m.set_edge_look(&own, &wall(st));
    m.roofs.push(Roof { a: [ax, ay], b: [bx, by], shape: RoofShape::Gable, paint: key(roof), turn: u8::from(r.h > r.w), overhang: 0.3 });

    let along = if door % 2 == 0 { r.w } else { r.h };
    let at = rng.int(1, (along - 2).max(1));
    let edge = |t: i32, len: i32| -> ([f32; 2], [f32; 2]) {
        let t = t as f32;
        let e = t + len as f32;
        match door {
            0 => ([ax + t, ay], [ax + e, ay]),
            1 => ([bx, ay + t], [bx, ay + e]),
            2 => ([ax + t, by], [ax + e, by]),
            _ => ([ax, ay + t], [ax, ay + e]),
        }
    };
    let (a, b) = edge(at, 1);
    m.portals.push(portal(a, b, PortalKind::Door, wood));
    for t in [at - 3, at + 2] {
        if t >= 1 && t + 2 <= along - 1 {
            let (a, b) = edge(t, 2);
            m.portals.push(portal(a, b, PortalKind::Window, wood));
        }
    }
    // Перед дверью свободно; рядом с домом — бочки и цветы.
    let (dx, dy): (i32, i32) = [(0, -1), (1, 0), (0, 1), (-1, 0)][door as usize];
    let front = [(a[0] + b[0]) / 2.0 + dx as f32 * 0.5, (a[1] + b[1]) / 2.0 + dy as f32 * 0.5];
    let (fx, fy) = (front[0].floor() as i32, front[1].floor() as i32);
    sp.block(fx, fy, 1, 1);
    let (sx, sy) = (dy.abs(), dx.abs());
    for s in [-2, 2, 3] {
        if rng.chance(0.45) {
            let n = pick(rng, &["planter", "barrel", "crate", "sacks"]);
            put(m, sp, n, fx + sx * s, fy + sy * s, 0.0, 0.8 + rng.random() as f32 * 0.3);
        }
    }
    // Задний двор за низким забором: не у каждого дома.
    let (bdx, bdy) = (-dx, -dy);
    let d = rng.int(2, 3);
    if rng.chance(0.4) {
        let (x0, y0, x1, y1) = if bdy != 0 {
            let y = if bdy > 0 { r.y + r.h } else { r.y - d };
            (r.x, y, r.w, d)
        } else {
            let x = if bdx > 0 { r.x + r.w } else { r.x - d };
            (x, r.y, d, r.h)
        };
        if sp.free(x0, y0, x1, y1) {
            let pts: Vec<[f32; 2]> = if bdy != 0 {
                let (near, far) = if bdy > 0 { (by, by + d as f32) } else { (ay, ay - d as f32) };
                vec![[ax, near], [ax, far], [bx, far], [bx, near]]
            } else {
                let (near, far) = if bdx > 0 { (bx, bx + d as f32) } else { (ax, ax - d as f32) };
                vec![[near, ay], [far, ay], [far, by], [near, by]]
            };
            if pts.iter().all(|p| p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= m.width as f32 && p[1] <= m.height as f32) {
                let style = WallStyle { width: 0.18, ..wall(if rng.chance(0.5) { "wall_wood" } else { "wall_hedge" }) };
                m.walls.push(Wall { points: pts, closed: false, bends: Vec::new(), style: Some(style) });
                sp.block(x0, y0, x1, y1);
            }
        }
    }
}

/// Бордюр: стена-полоса вдоль контура мощёного; прямые стороны идут прямо, углы чуть скруглены.
/// По краю карты бордюра нет.
fn curbs(m: &mut Map, street: &Mask) {
    let stone = mat(m, "wall_fieldstone");
    let (w, h) = (m.width as f32, m.height as f32);
    let along_edge = |a: [f32; 2], b: [f32; 2]| {
        (a[0] < 0.1 && b[0] < 0.1) || (a[1] < 0.1 && b[1] < 0.1) || (a[0] > w - 0.1 && b[0] > w - 0.1) || (a[1] > h - 0.1 && b[1] > h - 0.1)
    };
    for line in mask::join(mask::contour(street)) {
        let line = crate::export::simplify(&line, true, 0.05);
        let mut run: Vec<[f32; 2]> = Vec::new();
        for pair in line.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if along_edge(a, b) {
                push_curb(m, &mut run, stone);
                continue;
            }
            // Опорные точки у самых концов держат кривую на прямой; скругление остаётся в углу.
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let at = |t: f32| [a[0] + (b[0] - a[0]) * t / len, a[1] + (b[1] - a[1]) * t / len];
            let mut t = 0.25f32.min(len / 3.0);
            run.push(at(t));
            while t + 1.0 < len - 0.25 {
                t += 1.0;
                run.push(at(t));
            }
            run.push(at((len - 0.25).max(len * 2.0 / 3.0)));
        }
        push_curb(m, &mut run, stone);
    }
}

fn push_curb(m: &mut Map, run: &mut Vec<[f32; 2]>, material: usize) {
    if run.len() >= 2 {
        m.paths.push(Path { points: std::mem::take(run), material, width: 0.22, soft: 0.0, rough: 0.0, along: true });
    }
    run.clear();
}

fn town(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let density = p.options.density;
    let (w, h) = (m.width as i32, m.height as i32);
    let season = p.options.season;
    let (grass, wet) = town_grass(season);
    ground(m, "dirt");
    // Зимой снег лежит шире травы: пятна крупнее и чаще.
    let (count, size) = if season == Season::Winter { (w * h / 50, (2.0, 4.5)) } else { (w * h / 90, (1.2, 3.0)) };
    patches(m, rng, grass, count, size, 0.75);
    patches(m, rng, wet, w * h / 160, (0.8, 2.0), 0.6);
    patches(m, rng, "gravel", w * h / 200, (0.8, 1.6), 0.5);
    // Кварталы: прямоугольник делится прямыми улицами разной ширины.
    let mut todo = vec![Room { x: 0, y: 0, w, h }];
    let mut blocks = Vec::new();
    let mut streets: Vec<Room> = Vec::new();
    let top = 12 + ((1.0 - density) * 8.0) as i32;
    while let Some(b) = todo.pop() {
        let wide = b.w >= b.h;
        let long = if wide { b.w } else { b.h };
        if long < top + rng.int(-1, 4) {
            blocks.push(b);
            continue;
        }
        let cut = rng.int(long * 3 / 10, long * 7 / 10);
        let street = rng.int(2, 3);
        streets.push(if wide { Room { x: b.x + cut, w: street, ..b } } else { Room { y: b.y + cut, h: street, ..b } });
        if wide {
            todo.push(Room { w: cut, ..b });
            todo.push(Room { x: b.x + cut + street, w: b.w - cut - street, ..b });
        } else {
            todo.push(Room { h: cut, ..b });
            todo.push(Room { y: b.y + cut + street, h: b.h - cut - street, ..b });
        }
    }
    m.wall_style = wall("wall_timber");
    let mut sp = Spots::new(m);
    // Площадь — квартал ближе всех к середине, замощённый целиком.
    let middle = blocks
        .iter()
        .copied()
        .filter(|b| b.w >= 6 && b.h >= 6)
        .min_by_key(|b| (b.x + b.w / 2 - w / 2).abs() + (b.y + b.h / 2 - h / 2).abs());
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut paved = Mask { w: mw, h: mh, data: vec![0; mw * mh] };
    for r in streets.iter().chain(middle.iter()) {
        for (i, _) in mask::cells(mw, mh, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1)) {
            paved.data[i] = 255;
        }
    }
    let cobble = mat(m, "cobblestone");
    let weights: Vec<(usize, f32)> = paved.data.iter().enumerate().filter(|(_, v)| **v > 0).map(|(i, _)| (i, 1.0)).collect();
    m.paint(Layer::Floor, cobble, &weights);
    if let Some(b) = middle {
        let paving = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("flagstones"));
        m.paint_rect(Layer::Floor, (b.x, b.y), (b.x + b.w - 1, b.y + b.h - 1), Some(paving));
    }
    // Улица грязная: земля наползает с краёв, местами брусчатка стёрта.
    let dirt = mat(m, "dirt");
    let mut worn = Vec::new();
    for _ in 0..streets.iter().map(|r| r.w * r.h).sum::<i32>() / 12 {
        let r = streets[rng.int(0, streets.len() as i32 - 1) as usize];
        worn.push(([r.x as f32 + rng.random() as f32 * r.w as f32, r.y as f32 + rng.random() as f32 * r.h as f32], 0.6 + rng.random() as f32 * 0.9));
    }
    let weights = blobs(m, &worn, 0.1, 0.65);
    m.paint(Layer::Floor, dirt, &weights);
    curbs(m, &paved);

    if let Some(b) = middle {
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
        let centre = if b.w >= 9 && b.h >= 9 { "fountain" } else { "well" };
        let c = asset(centre);
        let s = c.size[0] as i32;
        let (ox, oy) = (rng.int(-1, 1), rng.int(-1, 1));
        put_pick(m, &mut sp, &c, centre, cx - s / 2 + ox, cy - s / 2 + oy, 0.0, 1.0);
        for _ in 0..(b.w * b.h / 24).clamp(2, 5) {
            let n = pick(rng, &["market_stall", "market_stall_blue", "market_stall_ochre"]);
            by_wall(m, &mut sp, rng, b, n, 1.0, false);
        }
        by_wall(m, &mut sp, rng, b, "notice_board", 1.0, false);
        by_wall(m, &mut sp, rng, b, "cart", 1.0, false);
        for _ in 0..3 {
            let n = pick(rng, &["crate", "barrel", "sacks", "bench", "planter"]);
            inside(m, &mut sp, rng, b, n, 1.0);
        }
        for (x, y) in [(b.x + 1, b.y + 1), (b.x + b.w - 2, b.y + b.h - 2)] {
            put(m, &mut sp, "street_lamp", x, y, 0.0, 0.7);
        }
        wanted(m, &mut sp, rng, b, &p.objects, "площадь", warn);
    } else if !p.objects.is_empty() {
        warn.push("квартал: площади не вышло, объектам негде встать".into());
    }
    let look = match (p.style.walls.as_deref().and_then(plan::wall_id), p.style.roof.as_deref().and_then(plan::roof_id)) {
        (None, None) => None,
        (st, roof) => Some((st.unwrap_or("wall_timber"), roof.unwrap_or("roof_clay_red"))),
    };
    let mut count = 0;
    for b in blocks.iter().copied().filter(|b| Some(*b) != middle) {
        // Дома вдоль длинной стороны, лицом к улице; отступы и пустыри у всех разные.
        let row = b.w >= b.h;
        let (long, deep) = if row { (b.w, b.h) } else { (b.h, b.w) };
        if long < 6 || deep < 5 {
            continue;
        }
        let two = deep >= 12;
        let depth = if two { (deep / 2 - 1).min(7) } else { (deep - 2).clamp(3, 7) };
        for front in [true, false].into_iter().take(if two { 2 } else { 1 }) {
            let mut t = rng.int(1, 2);
            while t + 4 < long {
                let len = rng.int(4, 9).min(long - t - 1);
                if len < 4 {
                    break;
                }
                if rng.chance(0.14) {
                    // Пустырь или сад вместо дома.
                    let c = if row { [(b.x + t) as f32 + len as f32 / 2.0, b.y as f32 + deep as f32 / 2.0] } else { [b.x as f32 + deep as f32 / 2.0, (b.y + t) as f32 + len as f32 / 2.0] };
                    let tree = season_tree(rng, season);
                    drop(m, tree, c, rng.int(0, 3) as f32 * 90.0, 0.8);
                    t += len + 1;
                    continue;
                }
                let d = rng.int((depth - 2).max(3), depth);
                let set = rng.int(1, 2).min(deep - d - 1);
                let v = if front { set } else { deep - d - set };
                let r = if row { Room { x: b.x + t, y: b.y + v, w: len, h: d } } else { Room { x: b.x + v, y: b.y + t, w: d, h: len } };
                // Если сторона блока лежит на краю карты, улицы там нет: дверь смотрит внутрь.
                let edge = if row { if front { b.y == 0 } else { b.y + b.h == h } } else if front { b.x == 0 } else { b.x + b.w == w };
                let outward = if row { if front { 0 } else { 2 } } else if front { 3 } else { 1 };
                let door = if edge { (outward + 2) % 4 } else { outward };
                // Ряды одного квартала могут сойтись: дом на занятое место не встаёт.
                if sp.free(r.x - 1, r.y - 1, r.w + 2, r.h + 2) {
                    house(m, rng, &mut sp, r, door, look);
                    count += 1;
                }
                t += len + rng.int(1, 3);
            }
        }
        if rng.chance(0.6) {
            put(m, &mut sp, "street_lamp", b.x + rng.int(0, 1), b.y + rng.int(0, 1), 0.0, 0.7);
        }
        if rng.chance(0.4) {
            put(m, &mut sp, "street_lamp", b.x + b.w - 1, b.y + b.h - 1, 0.0, 0.7);
        }
        for _ in 0..(b.w * b.h / 25) {
            let n = pick(rng, &["bush", "bush", "crate", "barrel", "planter", "rubble", "sacks"]);
            let n = if n == "bush" { season_bush(rng, season) } else { Some(n) };
            if let Some(n) = n {
                inside(m, &mut sp, rng, b, n, 0.9);
            }
        }
    }
    if p.mood == Mood::Abandoned {
        let houses: Vec<Room> = m.roofs.iter().map(|r| Room { x: r.a[0] as i32, y: r.a[1] as i32, w: (r.b[0] - r.a[0]) as i32, h: (r.b[1] - r.a[1]) as i32 }).collect();
        abandon(m, rng, &houses);
    }
    let edges = m.room_edges();
    let looks = std::mem::take(&mut m.edge_looks);
    m.sync_edge_looks(&edges);
    m.edge_looks.extend(looks);
    count
}

// ----- дом и таверна по плану -----

/// Вес комнаты при делении дома: во сколько раз она больше кладовой.
fn room_weight(spec: &plan::RoomSpec, tavern: bool) -> f32 {
    let base = match spec.role {
        Role::Hall | Role::Living => {
            if tavern {
                6.0
            } else {
                4.0
            }
        }
        Role::Barracks => 3.0,
        Role::Kitchen | Role::Bedroom | Role::Library | Role::Shrine | Role::Armoury | Role::Ruin | Role::Entrance | Role::Auto => 2.0,
        Role::Treasure | Role::Exit | Role::Empty => 1.5,
        Role::Storage => 1.2,
    };
    base * spec.size.weight()
}

/// Обход комнат от входа по связям: соседи по плану идут рядом и в раскладке
/// оказываются рядом. Кто не связан — в конце, по порядку плана.
fn bfs_order(n: usize, start: usize, graph: &BTreeMap<usize, Vec<usize>>) -> Vec<usize> {
    let mut seen = vec![false; n];
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([start]);
    seen[start] = true;
    while let Some(i) = queue.pop_front() {
        out.push(i);
        if let Some(ns) = graph.get(&i) {
            for &j in ns {
                if !seen[j] {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
    }
    out.extend((0..n).filter(|&i| !seen[i]));
    out
}

fn shuffle(v: &mut [usize], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = rng.int(0, i as i32) as usize;
        v.swap(i, j);
    }
}

/// Деление прямоугольника дома между комнатами по весам: группа комнат делится
/// надвое поперёк длинной стороны, каждая половина — дальше. Первые в списке
/// ложатся слева и сверху. Меньше четырёх клеток по стороне не бывает; кому не
/// хватило места, остаётся без комнаты.
fn slice(r: Room, items: &[usize], w: &[f32], out: &mut [Option<Room>], rng: &mut Rng) {
    const MIN: i32 = 4;
    if items.len() == 1 {
        out[items[0]] = Some(r);
        return;
    }
    let total: f32 = items.iter().map(|&i| w[i]).sum::<f32>().max(1e-3);
    let (mut k, mut acc, mut best) = (1, 0.0f32, f32::MAX);
    for c in 1..items.len() {
        acc += w[items[c - 1]];
        let d = (acc / total - 0.5).abs();
        if d < best {
            best = d;
            k = c;
        }
    }
    let f = items[..k].iter().map(|&i| w[i]).sum::<f32>() / total;
    let mut vertical = r.w >= r.h;
    if (if vertical { r.w } else { r.h }) < 2 * MIN {
        vertical = !vertical;
    }
    let long = if vertical { r.w } else { r.h };
    if long < 2 * MIN {
        out[items[0]] = Some(r);
        return;
    }
    let cut = ((long as f32 * f).round() as i32 + rng.int(-1, 1)).clamp(MIN, long - MIN);
    let (p, q) = if vertical { (Room { w: cut, ..r }, Room { x: r.x + cut, w: r.w - cut, ..r }) } else { (Room { h: cut, ..r }, Room { y: r.y + cut, h: r.h - cut, ..r }) };
    slice(p, &items[..k], w, out, rng);
    slice(q, &items[k..], w, out, rng);
}

/// Общая стена двух комнат: отрезок по узлам сетки, концы по возрастанию. Касание углом не считается.
fn shared(a: Room, b: Room) -> Option<Segment> {
    if a.x + a.w == b.x || b.x + b.w == a.x {
        let x = if a.x + a.w == b.x { b.x } else { a.x } as f32;
        let (y0, y1) = (a.y.max(b.y), (a.y + a.h).min(b.y + b.h));
        if y1 - y0 >= 1 {
            return Some(([x, y0 as f32], [x, y1 as f32]));
        }
    }
    if a.y + a.h == b.y || b.y + b.h == a.y {
        let y = if a.y + a.h == b.y { b.y } else { a.y } as f32;
        let (x0, x1) = (a.x.max(b.x), (a.x + a.w).min(b.x + b.w));
        if x1 - x0 >= 1 {
            return Some(([x0 as f32, y], [x1 as f32, y]));
        }
    }
    None
}

fn seg_len(s: &Segment) -> i32 {
    ((s.1[0] - s.0[0]).abs() + (s.1[1] - s.0[1]).abs()).round() as i32
}

/// Насколько раскладка отвечает плану: сошедшиеся связи, размеры, вход у южной стены.
fn layout_score(p: &Plan, rooms: &[Option<Room>], building: Room, start: usize) -> f32 {
    let mut s = 0.0;
    for l in p.inner_links() {
        if let (Some(a), Some(b)) = (p.room_index(&l.from), p.room_index(&l.to))
            && let (Some(ra), Some(rb)) = (rooms[a], rooms[b])
            && let Some(seg) = shared(ra, rb)
        {
            let need = if l.kind == LinkKind::DoubleDoor { 2 } else { 1 };
            if seg_len(&seg) >= need {
                s += 10.0;
            }
        }
    }
    for (i, spec) in p.rooms.iter().enumerate() {
        if let (Size::Cells([cw, ch]), Some(r)) = (spec.size, rooms[i]) {
            s -= ((r.w * r.h) as f32 - (cw * ch) as f32).abs() / (cw * ch) as f32 * 3.0;
        }
    }
    if let Some(r) = rooms[start]
        && r.y + r.h == building.y + building.h
    {
        s += 2.0;
    }
    s - rooms.iter().filter(|r| r.is_none()).count() as f32 * 50.0
}

/// Место для двери на стене `seg` шириной `width` клеток: не у самого угла и не там,
/// где в стену упирается поперечная (`corners` — такие точки вдоль стены), и не на
/// уже занятом куске. Без хорошего места — посередине.
fn door_spot(seg: Segment, width: i32, corners: &[i32], taken: &[(i32, i32)], rng: &mut Rng) -> Option<(i32, i32)> {
    let vertical = seg.0[0] == seg.1[0];
    let (s, e) = if vertical { (seg.0[1] as i32, seg.1[1] as i32) } else { (seg.0[0] as i32, seg.1[0] as i32) };
    if e - s < width {
        return None;
    }
    let free = |t: i32| !taken.iter().any(|&(a, b)| t < b + 1 && t + width > a - 1);
    let good: Vec<i32> = (s + 1..=e - width - 1).filter(|&t| free(t) && !corners.iter().any(|&c| t - 1 < c && c < t + width + 1)).collect();
    if !good.is_empty() {
        let t = good[rng.int(0, good.len() as i32 - 1) as usize];
        return Some((t, t + width));
    }
    let t = s + (e - s - width) / 2;
    free(t).then_some((t, t + width))
}

/// Проём на стене между `from` и `to` вдоль неё.
fn along(seg: Segment, from: i32, to: i32) -> ([f32; 2], [f32; 2]) {
    if seg.0[0] == seg.1[0] { ([seg.0[0], from as f32], [seg.0[0], to as f32]) } else { ([from as f32, seg.0[1]], [to as f32, seg.0[1]]) }
}

/// Запустение: обломки и валуны там, где нет комнат.
fn abandon(m: &mut Map, rng: &mut Rng, rooms: &[Room]) {
    let (w, h) = (m.width as f32, m.height as f32);
    for _ in 0..((w * h) / 70.0) as i32 {
        let q = [rng.random() as f32 * w, rng.random() as f32 * h];
        let inside = rooms.iter().any(|r| q[0] >= r.x as f32 - 1.0 && q[0] <= (r.x + r.w) as f32 + 1.0 && q[1] >= r.y as f32 - 1.0 && q[1] <= (r.y + r.h) as f32 + 1.0);
        if !inside && m.paths.iter().all(|p| p.distance(q) > 1.0) {
            let n = pick(rng, &["rubble", "rubble", "boulder"]);
            drop(m, n, q, rng.int(0, 3) as f32 * 90.0, 0.6 + rng.random() as f32 * 0.5);
        }
    }
}

/// Желаемый размер комнаты по назначению, в клетках: ширина вдоль фасада и глубина.
/// Зал таверны считается от рядов столов (стол со стульями — 3×4 клетки, между
/// рядами проход), плюс полоса стойки у задней стены. Подсобки — сколько нужно делу.
fn target(spec: &plan::RoomSpec, tavern: bool) -> (i32, i32) {
    if let Size::Cells([w, h]) = spec.size {
        return (w as i32, h as i32);
    }
    let k = match spec.size {
        Size::Named(SizeName::Small) => 0,
        Size::Named(SizeName::Large) => 2,
        _ => 1,
    };
    let t: [(i32, i32); 3] = match spec.role {
        // Глубина: полоса стойки (4) + ряды столов по 5 + проход у входа.
        Role::Hall if tavern => [(9, 10), (13, 11), (17, 15)],
        Role::Hall | Role::Living => [(6, 5), (7, 6), (9, 7)],
        Role::Kitchen => [(4, 4), (5, 5), (6, 5)],
        Role::Storage => [(3, 3), (4, 4), (5, 4)],
        Role::Bedroom => [(4, 4), (4, 5), (5, 6)],
        Role::Barracks => [(5, 5), (6, 6), (8, 6)],
        Role::Treasure | Role::Exit | Role::Empty => [(3, 3), (4, 3), (5, 4)],
        _ => [(4, 4), (5, 5), (6, 6)],
    };
    t[k]
}

/// Раскладка «от зала»: зал — лицом на юг, у его задней стены ряд подсобок (кухня,
/// кладовые — ближе к стойке), что не влезло — крыло сбоку, от фасада вглубь.
/// Дом выходит буквой Г, П или прямоугольником — смотря сколько комнат и какие.
/// Координаты от угла (0, 0); вторым — рамка дома, третьим — корпуса (зал, задний
/// ряд, крыло): у каждого своя крыша. Не вышло — `None`.
type Layout = (Vec<Option<Room>>, Room, Vec<Room>);

fn wings(p: &Plan, tavern: bool, hall: usize, graph: &BTreeMap<usize, Vec<usize>>, rng: &mut Rng, scale: f32, shuffle_some: bool) -> Option<Layout> {
    let n = p.rooms.len();
    let dims: Vec<(i32, i32)> = p
        .rooms
        .iter()
        .map(|s| {
            let (w, h) = target(s, tavern);
            let k = if matches!(s.size, Size::Cells(_)) { 1.0 } else { scale };
            (((w as f32 * k).round() as i32).max(3), ((h as f32 * k).round() as i32).max(3))
        })
        .collect();
    let (hw, hh) = dims[hall];
    // Порядок: от зала по связям; кухня и кладовые — первыми, они за стойкой.
    let service = |i: usize| matches!(p.rooms[i].role, Role::Kitchen | Role::Storage);
    let linked = |a: usize, b: usize| graph.get(&a).is_some_and(|v| v.contains(&b));
    let mut order: Vec<usize> = bfs_order(n, hall, graph).into_iter().filter(|&i| i != hall).collect();
    order.sort_by_key(|&i| (!linked(hall, i) && !service(i), !service(i)));
    if shuffle_some && order.len() > 1 {
        for _ in 0..2 {
            let a = rng.int(0, order.len() as i32 - 2) as usize;
            order.swap(a, a + 1);
        }
    }
    // Задний ряд — во всю ширину зала или короче (тогда дом — буквой Г).
    let mut back: Vec<usize> = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    let mut len = 0;
    for &i in &order {
        if len + dims[i].0 <= hw + 1 && (back.is_empty() || !rng.chance(0.12)) {
            back.push(i);
            len += dims[i].0;
        } else {
            rest.push(i);
        }
    }
    // Рядом с кухней — то, что с ней связано: кладовая у кухни, а не через зал.
    if let Some(k) = back.iter().position(|&i| p.rooms[i].role == Role::Kitchen) {
        let kitchen = back[k];
        let mut row = vec![kitchen];
        for &i in &back {
            if i != kitchen && linked(kitchen, i) {
                row.push(i);
            }
        }
        for &i in &back {
            if !row.contains(&i) {
                row.push(i);
            }
        }
        back = row;
    }
    let depth = back.iter().map(|&i| dims[i].1).max().unwrap_or(0).min(hh.max(3));
    // Ряд вплотную к залу: короче зала — прижат к левому или правому краю.
    let fill = len >= hw - 2;
    let row_len = if fill { hw } else { len };
    let row_left = !rng.chance(0.5);
    // Крыло: ширина по самой широкой комнате, длина — вдоль зала и заднего ряда.
    let wing_w = rest.iter().map(|&i| dims[i].0).max().unwrap_or(0).clamp(0, 7);
    let column = depth + hh;
    let mut wing_hs: Vec<i32> = rest.iter().map(|&i| dims[i].1).collect();
    let sum: i32 = wing_hs.iter().sum();
    if sum > column {
        let k = column as f32 / sum as f32;
        for h in wing_hs.iter_mut() {
            *h = ((*h as f32 * k).floor() as i32).max(3);
        }
        let mut over = wing_hs.iter().sum::<i32>() - column;
        for h in wing_hs.iter_mut().rev() {
            while over > 0 && *h > 3 {
                *h -= 1;
                over -= 1;
            }
        }
        if over > 0 {
            return None;
        }
    }
    // Крыло со стороны, где задний ряд короче, — дом собраннее.
    let wing_left = if fill { rng.chance(0.5) } else { !row_left };
    let ox = if wing_left && !rest.is_empty() { wing_w } else { 0 };
    let hall_room = Room { x: ox, y: depth, w: hw, h: hh };
    let mut out = vec![None; n];
    out[hall] = Some(hall_room);
    // Задний ряд: ширины по желаемым, растянуты до ряда.
    if !back.is_empty() {
        let total: i32 = back.iter().map(|&i| dims[i].0).sum();
        let mut x = if row_left { ox } else { ox + hw - row_len };
        let mut left = row_len;
        for (k, &i) in back.iter().enumerate() {
            let w = if k + 1 == back.len() { left } else { ((dims[i].0 * row_len) as f32 / total as f32).round() as i32 };
            if w < 3 {
                return None;
            }
            out[i] = Some(Room { x, y: 0, w, h: depth });
            x += w;
            left -= w;
        }
    }
    // Крыло: снизу (у фасада) — связанные с залом, выше — остальные.
    if !rest.is_empty() {
        let mut idx: Vec<usize> = (0..rest.len()).collect();
        idx.sort_by_key(|&k| !linked(hall, rest[k]));
        let x = if wing_left { 0 } else { ox + hw };
        let mut y = depth + hh;
        for k in idx {
            let h = wing_hs[k];
            y -= h;
            out[rest[k]] = Some(Room { x, y, w: wing_w, h });
        }
    }
    let rooms: Vec<Room> = out.iter().flatten().copied().collect();
    let (x0, y0) = (rooms.iter().map(|r| r.x).min()?, rooms.iter().map(|r| r.y).min()?);
    let (x1, y1) = (rooms.iter().map(|r| r.x + r.w).max()?, rooms.iter().map(|r| r.y + r.h).max()?);
    let span = |ids: &[usize]| -> Option<Room> {
        let rs: Vec<Room> = ids.iter().filter_map(|&i| out[i]).collect();
        let (a, b) = (rs.iter().map(|r| r.x).min()?, rs.iter().map(|r| r.y).min()?);
        let (c, d) = (rs.iter().map(|r| r.x + r.w).max()?, rs.iter().map(|r| r.y + r.h).max()?);
        Some(Room { x: a, y: b, w: c - a, h: d - b })
    };
    let mut blocks = vec![hall_room];
    blocks.extend(span(&back));
    blocks.extend(span(&rest));
    Some((out, Room { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, blocks))
}

/// Внешние куски стен комнаты среди комнат дома: сторона (0 — север, 1 — восток,
/// 2 — юг, 3 — запад) и отрезок. Где к стене прилегает соседняя комната — не внешняя.
fn outer_pieces(r: Room, all: &[Room]) -> Vec<(i32, Segment)> {
    let mut out = Vec::new();
    for side in 0..4 {
        let (line, s, e) = match side {
            0 => (r.y, r.x, r.x + r.w),
            1 => (r.x + r.w, r.y, r.y + r.h),
            2 => (r.y + r.h, r.x, r.x + r.w),
            _ => (r.x, r.y, r.y + r.h),
        };
        let mut covered: Vec<(i32, i32)> = all
            .iter()
            .filter(|o| **o != r)
            .filter_map(|o| match side {
                0 if o.y + o.h == line => Some((o.x, o.x + o.w)),
                1 if o.x == line => Some((o.y, o.y + o.h)),
                2 if o.y == line => Some((o.x, o.x + o.w)),
                3 if o.x + o.w == line => Some((o.y, o.y + o.h)),
                _ => None,
            })
            .collect();
        covered.sort();
        let mut t = s;
        let mut push = |a: i32, b: i32| {
            if b > a {
                let (a, b) = (a as f32, b as f32);
                let l = line as f32;
                out.push((side, if side % 2 == 0 { ([a, l], [b, l]) } else { ([l, a], [l, b]) }));
            }
        };
        for (cs, ce) in covered {
            if ce <= t || cs >= e {
                continue;
            }
            push(t, cs.max(t));
            t = t.max(ce);
        }
        push(t, e);
    }
    out
}

fn interior(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let tavern = p.kind == Kind::Tavern;
    let (w, h) = (m.width as i32, m.height as i32);
    let season = p.options.season;
    let look = forest_look(season);
    ground(m, look.ground);
    patches(m, rng, look.patches, w * h / 60, (2.0, 4.0), 0.6);
    let planks = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("wood_planks"));
    let outer = match p.style.walls.as_deref().and_then(plan::wall_id) {
        Some(st) => wall(st),
        None if tavern => wall("wall_timber"),
        None => wall(pick(rng, &["wall_timber", "wall_fieldstone", "wall_brick"])),
    };
    let wood = !matches!(&outer.paint, Paint::Texture(k) if k.ends_with("fieldstone"));
    let inner = WallStyle { width: outer.width * 0.8, ..outer.clone() };

    // Раскладка «от зала» (`wings`): размеры по назначению, несколько попыток,
    // лучшая — где сошлось больше связей; не влезло — комнаты помельче. Совсем не
    // вышло — прежняя нарезка прямоугольника.
    let n = p.rooms.len();
    let graph = plan::adjacency(p);
    let start = p.entrance().unwrap_or(0);
    let hall_i = p.rooms.iter().position(|s| s.role.is_hall()).unwrap_or(start);
    // По краям — двор: дорожка, деревья, объекты плана.
    // Река по плану просит полосу у своего края: вода в пять клеток, галька и суша.
    let river_at = p.river.side();
    let (room_w, room_h) = match river_at {
        Some(s) if s % 2 == 1 => (w - 4 - RIVER_ROOM, h - 5),
        Some(_) => (w - 4, h - 5 - RIVER_ROOM),
        None => (w - 4, h - 5),
    };
    let mut best: Option<(f32, Layout)> = None;
    for scale in [1.0f32, 0.85, 0.72, 0.6, 0.5] {
        for attempt in 0..24 {
            let Some(layout) = wings(p, tavern, hall_i, &graph, rng, scale, attempt > 0) else { continue };
            let (rooms, bb) = (&layout.0, layout.1);
            if bb.w > room_w || bb.h > room_h {
                continue;
            }
            let used: i32 = rooms.iter().flatten().map(|r| r.w * r.h).sum();
            let score = layout_score(p, rooms, bb, start) - (bb.w * bb.h - used) as f32 / (bb.w * bb.h) as f32 * 4.0;
            if best.as_ref().is_none_or(|b| score > b.0 + 1e-6) {
                best = Some((score, layout));
            }
        }
        if best.is_some() {
            break;
        }
    }
    let (laid, building, blocks) = match best {
        Some((_, (rooms, bb, blocks))) => {
            let (mut dx, mut dy) = ((w - bb.w) / 2, ((h - bb.h) / 2 - 1).max(1));
            // Дом — подальше от реки: середина оставшейся части карты.
            match river_at {
                Some(0) => dy = RIVER_ROOM + ((h - RIVER_ROOM - bb.h) / 2).max(0),
                Some(1) => dx = ((w - RIVER_ROOM - bb.w) / 2).max(1),
                Some(2) => dy = ((h - RIVER_ROOM - bb.h) / 2).max(1),
                Some(3) => dx = RIVER_ROOM + ((w - RIVER_ROOM - bb.w) / 2).max(0),
                _ => {}
            }
            let shift = |r: Room| Room { x: r.x + dx, y: r.y + dy, ..r };
            let moved = rooms.iter().map(|r| r.map(shift)).collect::<Vec<_>>();
            (moved, shift(bb), blocks.into_iter().map(shift).collect::<Vec<_>>())
        }
        None => {
            let (bw, bh) = plan::building_size(p.kind, m.width, m.height);
            let building = Room { x: (w - bw) / 2, y: ((h - bh) / 2 - 1).max(0), w: bw, h: bh };
            let weights: Vec<f32> = p.rooms.iter().map(|s| room_weight(s, tavern)).collect();
            let order = bfs_order(n, start, &graph);
            let mut best: Option<(f32, Vec<Option<Room>>)> = None;
            for attempt in 0..24 {
                let mut ord = order.clone();
                if attempt > 0 {
                    shuffle(&mut ord, rng);
                }
                let mut rooms = vec![None; n];
                slice(building, &ord, &weights, &mut rooms, rng);
                let score = layout_score(p, &rooms, building, start);
                if best.as_ref().is_none_or(|b| score > b.0 + 1e-6) {
                    best = Some((score, rooms));
                }
            }
            (best.map(|b| b.1).unwrap_or_default(), building, vec![building])
        }
    };
    let (bw, bh) = (building.w, building.h);
    for (spec, r) in p.rooms.iter().zip(&laid) {
        if r.is_none() {
            warn.push(format!("комната «{}» не поместилась в дом {bw}×{bh}", spec.id));
        }
    }
    let placed: Vec<(usize, Room)> = laid.iter().enumerate().filter_map(|(i, r)| r.map(|r| (i, r))).collect();
    if placed.is_empty() {
        return 0;
    }
    let all: Vec<Room> = placed.iter().map(|(_, r)| *r).collect();
    for r in &all {
        m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(planks));
    }
    m.wall_style = outer.clone();
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    // Крыша — над каждым корпусом своя, конёк вдоль длинной стороны; в Foundry она
    // ляжет тайлом над токенами и спрячет комнаты, пока туда не войдут.
    let roof = match p.style.roof.as_deref().and_then(plan::roof_id) {
        Some(r) => r,
        None => match &outer.paint {
            Paint::Texture(k) if k.ends_with("fieldstone") || k.ends_with("stone_blocks") => pick(rng, &["roof_slate_gray", "roof_slate_blue", "roof_shingle"]),
            Paint::Texture(k) if k.ends_with("brick") => pick(rng, &["roof_clay_brown", "roof_clay_red"]),
            _ => pick(rng, &["roof_clay_red", "roof_thatch", "roof_shingle", "roof_clay_brown"]),
        },
    };
    // Крыши — всегда встроенные: они нарисованы под скаты и конёк, библиотечные — нет.
    let roof_key = key(roof);
    for b in blocks.iter().filter(|b| b.w >= 2 && b.h >= 2) {
        // Почти квадратный большой зал — шатром на четыре ската, остальное — два ската.
        let squareish = b.w.min(b.h) * 5 >= b.w.max(b.h) * 4 && b.w.min(b.h) >= 8;
        let shape = if squareish { RoofShape::Hip } else { RoofShape::Gable };
        m.roofs.push(Roof { a: [b.x as f32, b.y as f32], b: [(b.x + b.w) as f32, (b.y + b.h) as f32], shape, paint: roof_key.clone(), turn: u8::from(b.h > b.w), overhang: 0.3 });
    }

    // Перегородки: общие стены соседей, слитые в прямые линии.
    let mut lines: BTreeMap<(bool, i32), Vec<(i32, i32)>> = BTreeMap::new();
    let mut corners: BTreeMap<(bool, i32), Vec<i32>> = BTreeMap::new();
    for &(_, r) in &placed {
        for (vertical, c, along_c) in [(true, r.x, r.y), (true, r.x + r.w, r.y), (false, r.y, r.x), (false, r.y + r.h, r.x)] {
            let ends = if vertical { [along_c, along_c + r.h] } else { [along_c, along_c + r.w] };
            corners.entry((vertical, c)).or_default().extend(ends);
        }
    }
    for (k, &(_, a)) in placed.iter().enumerate() {
        for &(_, b) in &placed[k + 1..] {
            if let Some(seg) = shared(a, b) {
                let vertical = seg.0[0] == seg.1[0];
                let (c, s, e) = if vertical { (seg.0[0] as i32, seg.0[1] as i32, seg.1[1] as i32) } else { (seg.0[1] as i32, seg.0[0] as i32, seg.1[0] as i32) };
                lines.entry((vertical, c)).or_default().push((s, e));
            }
        }
    }
    for (&(vertical, c), iv) in &mut lines {
        iv.sort();
        let mut merged: Vec<(i32, i32)> = Vec::new();
        for &(s, e) in iv.iter() {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        for (s, e) in merged {
            let (a, b) = if vertical { ([c as f32, s as f32], [c as f32, e as f32]) } else { ([s as f32, c as f32], [e as f32, c as f32]) };
            m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(inner.clone()) });
        }
    }

    // Двери по связям; занятые куски стен — чтобы двери не легли друг на друга.
    let mut taken: BTreeMap<(bool, i32), Vec<(i32, i32)>> = BTreeMap::new();
    let mut door_at: Vec<[f32; 2]> = Vec::new();
    // Дверь между комнатами: кто с кем уже соединён.
    let mut joined: Vec<(usize, usize)> = Vec::new();
    let room_of = |i: usize| laid[i];
    let place_door = |m: &mut Map, rng: &mut Rng, a: usize, b: usize, kind: LinkKind, taken: &mut BTreeMap<(bool, i32), Vec<(i32, i32)>>, door_at: &mut Vec<[f32; 2]>| -> Result<(), String> {
        let (Some(ra), Some(rb)) = (room_of(a), room_of(b)) else { return Err("комната не поместилась".into()) };
        let Some(seg) = shared(ra, rb) else { return Err("комнаты не смежны".into()) };
        let vertical = seg.0[0] == seg.1[0];
        let line = (vertical, if vertical { seg.0[0] } else { seg.0[1] } as i32);
        let width = match kind {
            LinkKind::DoubleDoor | LinkKind::Opening if seg_len(&seg) >= 4 => 2,
            _ => 1,
        };
        let (no_corners, no_taken) = (Vec::new(), Vec::new());
        let (s, e) = door_spot(seg, width, corners.get(&line).unwrap_or(&no_corners), taken.get(&line).unwrap_or(&no_taken), rng).ok_or("на общей стене нет места")?;
        taken.entry(line).or_default().push((s, e));
        let (da, db) = along(seg, s, e);
        let pk = match kind {
            LinkKind::Door => PortalKind::Door,
            LinkKind::DoubleDoor if width == 2 => PortalKind::DoubleDoor,
            LinkKind::DoubleDoor | LinkKind::Bridge => PortalKind::Door,
            LinkKind::Opening => PortalKind::Opening,
            LinkKind::Secret => PortalKind::Secret,
        };
        m.portals.push(portal(da, db, pk, true));
        door_at.push([(da[0] + db[0]) / 2.0, (da[1] + db[1]) / 2.0]);
        Ok(())
    };
    for l in p.inner_links() {
        let (Some(a), Some(b)) = (p.room_index(&l.from), p.room_index(&l.to)) else { continue };
        if l.kind == LinkKind::Bridge {
            warn.push(format!("связь «{}» — «{}»: мост внутри дома — обычная дверь", l.from, l.to));
        }
        match place_door(m, rng, a, b, l.kind, &mut taken, &mut door_at) {
            Ok(()) => joined.push((a, b)),
            Err(why) => warn.push(format!("связь «{}» — «{}» ({}) не поставлена: {why}", l.from, l.to, l.kind.label())),
        }
    }
    // Проходимость: до каждой комнаты должно быть как дойти. Куда дверей не хватило,
    // они добавляются по смежности — от уже достижимых комнат.
    let mut reach = vec![false; n];
    reach[start] = room_of(start).is_some();
    if !reach[start] && let Some(&(i, _)) = placed.first() {
        reach[i] = true;
    }
    loop {
        let mut grew = false;
        for &(a, b) in &joined {
            if reach[a] != reach[b] {
                reach[a] = true;
                reach[b] = true;
                grew = true;
            }
        }
        if grew {
            continue;
        }
        let Some(&(i, ri)) = placed.iter().find(|(i, _)| !reach[*i]) else { break };
        // Сосед среди достижимых с самой длинной общей стеной.
        let near = placed.iter().filter(|(j, rj)| reach[*j] && shared(ri, *rj).is_some()).max_by_key(|(_, rj)| shared(ri, *rj).map_or(0, |s| seg_len(&s))).map(|(j, _)| *j);
        let Some(j) = near else {
            warn.push(format!("комната «{}» отрезана от остальных", p.rooms[i].id));
            reach[i] = true;
            continue;
        };
        match place_door(m, rng, i, j, LinkKind::Door, &mut taken, &mut door_at) {
            Ok(()) => {
                if p.inner_links().next().is_some() {
                    warn.push(format!("комната «{}»: добавлена дверь в «{}», иначе не пройти", p.rooms[i].id, p.rooms[j].id));
                }
                joined.push((i, j));
            }
            Err(why) => {
                warn.push(format!("комната «{}»: не удалось поставить дверь в «{}»: {why}", p.rooms[i].id, p.rooms[j].id));
                reach[i] = true;
            }
        }
    }

    // Входы: связи с улицей; без них — зал (или первая комната) с юга.
    let mut outs: Vec<(usize, Option<LinkKind>)> = p.links.iter().filter(|l| l.touches(OUTSIDE)).filter_map(|l| Some((p.room_index(l.other(OUTSIDE))?, Some(l.kind)))).collect();
    if outs.is_empty() {
        outs.push((start, None));
    }
    let mut sp = Spots::new(m);
    for d in &door_at {
        sp.block(d[0].floor() as i32 - 1, d[1].floor() as i32 - 1, 3, 3);
    }
    let river_side = p.river.side();
    let mut entrance: Option<(i32, [f32; 2], [f32; 2])> = None;
    let mut outer_doors: Vec<(usize, i32)> = Vec::new();
    for (ri, kind) in outs {
        let Some(r) = room_of(ri) else { continue };
        let mut sides = outer_pieces(r, &all);
        if sides.is_empty() {
            warn.push(format!("комната «{}» не у внешней стены: вход через неё невозможен", p.rooms[ri].id));
            continue;
        }
        // Юг — лицо дома; мост просит сторону реки.
        let rank = |side: i32| if kind == Some(LinkKind::Bridge) && river_side == Some(side) { -1 } else { [3, 2, 0, 1][side as usize] };
        sides.sort_by_key(|(s, seg)| (rank(*s), -seg_len(seg)));
        let (side, seg) = sides[0];
        let vertical = side % 2 == 1;
        let line = (vertical, if vertical { seg.0[0] } else { seg.0[1] } as i32);
        let wide = seg_len(&seg) >= 6;
        let (width, pk) = match kind {
            None | Some(LinkKind::DoubleDoor) | Some(LinkKind::Bridge) if wide => (2, PortalKind::DoubleDoor),
            None | Some(LinkKind::DoubleDoor) | Some(LinkKind::Bridge) | Some(LinkKind::Door) => (1, PortalKind::Door),
            Some(LinkKind::Opening) => (if wide { 2 } else { 1 }, PortalKind::Opening),
            Some(LinkKind::Secret) => (1, PortalKind::Secret),
        };
        if kind == Some(LinkKind::Bridge) && river_side != Some(side) {
            warn.push(format!("комната «{}»: для моста нужна река с той же стороны, что вход", p.rooms[ri].id));
        }
        let (no_corners, no_taken) = (Vec::new(), Vec::new());
        let Some((s, e)) = door_spot(seg, width, corners.get(&line).unwrap_or(&no_corners), taken.get(&line).unwrap_or(&no_taken), rng) else {
            warn.push(format!("комната «{}»: на внешней стене нет места для входа", p.rooms[ri].id));
            continue;
        };
        taken.entry(line).or_default().push((s, e));
        let (ga, gb) = along(seg, s, e);
        m.portals.push(portal(ga, gb, pk, wood));
        outer_doors.push((ri, side));
        // Перед дверью внутри свободно.
        let (dx, dy): (i32, i32) = [(0, -1), (1, 0), (0, 1), (-1, 0)][side as usize];
        let mid = [(ga[0] + gb[0]) / 2.0, (ga[1] + gb[1]) / 2.0];
        let (ix, iy) = ((mid[0] - dx as f32 * 1.5).floor() as i32, (mid[1] - dy as f32 * 1.5).floor() as i32);
        sp.block(ix - 1, iy - 1, 3, 3);
        if entrance.is_none() {
            entrance = Some((side, ga, gb));
        }
    }
    // Задняя дверь кухни — во двор, не на фасад; у неё хозяйство.
    let mut back_yard: Option<[f32; 2]> = None;
    if let Some(&(ki, kr)) = placed.iter().find(|(i, _)| p.rooms[*i].role == Role::Kitchen)
        && !outer_doors.iter().any(|(i, _)| *i == ki)
    {
        let mut sides: Vec<(i32, Segment)> = outer_pieces(kr, &all).into_iter().filter(|(s, seg)| *s != 2 && seg_len(seg) >= 3).collect();
        sides.sort_by_key(|(s, seg)| (*s != 0, -seg_len(seg)));
        if let Some(&(side, seg)) = sides.first() {
            let vertical = side % 2 == 1;
            let line = (vertical, if vertical { seg.0[0] } else { seg.0[1] } as i32);
            let (no_corners, no_taken) = (Vec::new(), Vec::new());
            if let Some((s, e)) = door_spot(seg, 1, corners.get(&line).unwrap_or(&no_corners), taken.get(&line).unwrap_or(&no_taken), rng) {
                taken.entry(line).or_default().push((s, e));
                let (ga, gb) = along(seg, s, e);
                m.portals.push(portal(ga, gb, PortalKind::Door, wood));
                outer_doors.push((ki, side));
                let (dx, dy): (f32, f32) = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)][side as usize];
                let mid = [(ga[0] + gb[0]) / 2.0, (ga[1] + gb[1]) / 2.0];
                sp.block((mid[0] - dx * 1.5).floor() as i32 - 1, (mid[1] - dy * 1.5).floor() as i32 - 1, 3, 3);
                sp.block((mid[0] + dx * 0.5).floor() as i32 - 1, (mid[1] + dy * 0.5).floor() as i32 - 1, 3, 3);
                back_yard = Some([mid[0] + dx * 3.0, mid[1] + dy * 3.0]);
            }
        }
    }
    // Окна на внешних стенах длиной от шести клеток, кроме стены с входом.
    for &(i, _) in &placed {
        let Some(r) = room_of(i) else { continue };
        for (side, seg) in outer_pieces(r, &all) {
            if seg_len(&seg) < 5 || outer_doors.contains(&(i, side)) {
                continue;
            }
            let s = if side % 2 == 1 { seg.0[1] } else { seg.0[0] } as i32;
            let (a, b) = along(seg, s + 2, s + 4);
            m.portals.push(portal(a, b, PortalKind::Window, wood));
        }
    }

    // Мебель: зал с баром или гостиной, остальные по роли; плановые объекты и подписи.
    let hall = placed.iter().find(|(i, _)| p.rooms[*i].role.is_hall()).or_else(|| placed.iter().max_by_key(|(_, r)| r.w * r.h)).map(|(i, _)| *i).unwrap_or(start);
    let tiles = mat(m, "stone_tiles");
    let mut auto_roles = [Role::Kitchen, Role::Storage, Role::Bedroom, Role::Bedroom, Role::Library, Role::Bedroom, Role::Storage].into_iter();
    let mut bar_done = false;
    for &(i, r) in &placed {
        let spec = &p.rooms[i];
        let role = if i == hall {
            if tavern { Role::Hall } else { Role::Living }
        } else if spec.role == Role::Auto {
            auto_roles.next().unwrap_or(Role::Bedroom)
        } else {
            spec.role
        };
        match spec.floor.as_deref().and_then(plan::texture_id) {
            Some(t) => {
                let t = mat(m, t);
                m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(t));
            }
            None if role == Role::Kitchen => {
                m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(tiles));
            }
            None => {}
        }
        if role == Role::Hall && tavern && !bar_done {
            bar_done = true;
            let main = r;
            // Стойка отгораживает угол зала: за ней бочки и полки, перед ней табуреты.
            let scale = ((main.w - 3) as f32 / 7.0).clamp(0.6, 1.0);
            let bar = asset("bar_counter");
            let [left, top, right, low] = bounds(&bar, "bar_counter", 0.0, scale);
            let y = main.y as f32 + 2.2 - top;
            let width = cells_of(right - left);
            // Дверь кухни в задней стене зала — за стойкой: хозяин выходит из кухни
            // прямо к прилавку. Нет такой двери — стойка в левом углу, как раньше.
            let back_door = door_at.iter().filter(|d| (d[1] - main.y as f32).abs() < 0.01 && d[0] > main.x as f32 && d[0] < (main.x + main.w) as f32).map(|d| d[0]).next();
            let x0 = match back_door {
                Some(dx) => (dx - 1.0).clamp(main.x as f32 + 1.0, (main.x + main.w - width - 1).max(main.x + 1) as f32),
                None => main.x as f32 + 1.0,
            };
            let x = x0 + 0.6 - left;
            drop_pick(m, &bar, [x, y], 0.0, scale);
            let bx = x0.floor() as i32;
            sp.block(bx - 1, main.y, width + 2, 4);
            // За стойкой: полки и бочки вдоль стены, кроме места у двери.
            for k in 0..width / 2 {
                let px = x0 + k as f32 * 2.0 + 0.4;
                if back_door.is_some_and(|d| (px - d).abs() < 1.3) {
                    continue;
                }
                let n = if !missing("shelf_full") && k % 2 == 1 { "shelf_full" } else { pick(rng, &["barrel", "barrel", "crate"]) };
                drop(m, n, [px, main.y as f32 + 0.75], 0.0, if n == "shelf_full" { 1.0 } else { FURNITURE });
            }
            for k in 0..(width + 1) / 2 {
                drop(m, "stool", [x0 + 1.2 + k as f32 * 2.0, y + low + 0.5], 0.0, FURNITURE);
            }
            // Главный проход: от входа прямо к стойке — столы встанут по обе стороны.
            // Двери зала и так свободны (у каждой занят квадрат 3×3).
            let bar_front = [x0 + width as f32 / 2.0, y + low + 1.6];
            let mut aisle: Option<i32> = None;
            if let Some((side, ga, gb)) = entrance {
                let (dx, dy): (f32, f32) = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)][side as usize];
                let gate = [(ga[0] + gb[0]) / 2.0 - dx * 0.5, (ga[1] + gb[1]) / 2.0 - dy * 0.5];
                walkway(&mut sp, gate, bar_front, main);
                if side % 2 == 0 {
                    aisle = Some(gate[0].floor() as i32);
                }
                // Лестница наверх — в переднем углу у входа, со стороны без стойки.
                let stairs = asset("stairs");
                let [sl, st, sr, su] = bounds(&stairs, "stairs", 0.0, 1.0);
                let (sw, sh) = (cells_of(sr - sl), cells_of(su - st));
                let left_bar = x0 + (width as f32) / 2.0 < main.x as f32 + main.w as f32 / 2.0;
                let sx = if left_bar { main.x + main.w - sw } else { main.x };
                if sh + 5 < main.h {
                    put_pick(m, &mut sp, &stairs, "stairs", sx, main.y + main.h - sh, 0.0, 1.0);
                }
            }
            // Камин с креслами — у боковой стены: задняя за стойкой, в передней вход.
            if !hearth(m, &mut sp, rng, main, &[1, 3]) && !hearth(m, &mut sp, rng, main, &[0, 2]) {
                by_wall(m, &mut sp, rng, main, "fireplace", 1.0, false);
            }
            // Ряды столов — сразу за проходом перед стойкой.
            let mut ty = (bar_front[1].floor() as i32 + 1).max(main.y + 4);
            let (cw, ch) = dining_cell();
            while ty + ch <= main.y + main.h {
                // Столы через клетку прохода; главный проход — в такой промежуток.
                let off = match aisle {
                    Some(ax) => (ax - main.x - 1 - cw).rem_euclid(cw + 1),
                    None => rng.int(0, 1),
                };
                let mut tx = main.x + 1 + off.min(main.w - cw - 1).max(0);
                while tx + cw < main.x + main.w {
                    if rng.chance(0.92) {
                        dining(m, &mut sp, rng, tx, ty);
                    }
                    tx += cw + 1;
                }
                ty += ch + 1;
            }
            // Вдоль стен — скамьи, бочки, полки: большой зал без них пустой.
            dress_walls(m, &mut sp, rng, main);
        } else if role.is_hall() {
            furnish(m, &mut sp, rng, r, Role::Living);
            // Обеденный стол — от середины комнаты к краям, где первым найдётся место.
            let (cw, ch) = dining_cell();
            let (cx, cy) = (r.x + (r.w - cw) / 2, r.y + (r.h - ch) / 2);
            let mut spots: Vec<(i32, i32)> = (r.y..=r.y + r.h - ch).flat_map(|y| (r.x..=r.x + r.w - cw).map(move |x| (x, y))).collect();
            spots.sort_by_key(|&(x, y)| (x - cx).abs() + (y - cy).abs());
            for (x, y) in spots {
                if dining(m, &mut sp, rng, x, y) {
                    break;
                }
            }
        } else {
            furnish(m, &mut sp, rng, r, role);
        }
        if p.mood == Mood::Abandoned {
            furnish(m, &mut sp, rng, r, Role::Ruin);
        }
        wanted(m, &mut sp, rng, r, &spec.objects, &format!("комната «{}»", spec.id), warn);
        label(m, spec, r);
    }

    // Снаружи: река по плану, дорожка от входа к краю карты, мост, двор вокруг.
    let dirt = mat(m, "dirt");
    let (wf, hf) = (w as f32, h as f32);
    let water_paths = river_side.and_then(|side| {
        let (near, far) = match side {
            0 => (0.0, building.y as f32),
            1 => ((building.x + building.w) as f32, wf),
            2 => ((building.y + building.h) as f32, hf),
            _ => (0.0, building.x as f32),
        };
        if far - near < 6.0 {
            warn.push(format!("для реки с {} не хватает места: нужно шесть клеток между домом и краем", ["севера", "востока", "юга", "запада"][side as usize]));
            return None;
        }
        // Вода — до пяти клеток, но с клеткой суши у дома и у края карты.
        let width = (far - near - 3.6).clamp(2.4, 5.0);
        let c = (near + far) / 2.0 + (rng.random() as f32 - 0.5) * (far - near - width - 3.6).max(0.0);
        let line = if side % 2 == 0 { road(rng, [-1.0, c], [wf + 1.0, c], 1.0) } else { road(rng, [c, -1.0], [c, hf + 1.0], 1.0) };
        Some(stream(m, line, width))
    });
    if let Some(s) = &water_paths {
        m.paths.extend(s.paths());
    }
    let mut lane: Option<Path> = None;
    if let Some((side, ga, gb)) = entrance {
        let (dx, dy): (f32, f32) = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)][side as usize];
        let mid = [(ga[0] + gb[0]) / 2.0, (ga[1] + gb[1]) / 2.0];
        let from = [mid[0] + dx * 0.3, mid[1] + dy * 0.3];
        // Через реку тропа идёт прямо от двери: мост ровно продолжает её, а не стоит наискосок.
        let jitter = if water_paths.is_some() { 0.0 } else { (rng.random() as f32 - 0.5) * 6.0 };
        let to = match side {
            0 => [mid[0] + jitter, -1.0],
            1 => [wf + 1.0, mid[1] + jitter],
            2 => [mid[0] + jitter, hf + 1.0],
            _ => [-1.0, mid[1] + jitter],
        };
        // Прямая — частыми точками: мост ищется по ближайшей к воде точке линии.
        let straight = |n: usize| (0..=n).map(|k| {
            let t = k as f32 / n as f32;
            [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]
        }).collect::<Vec<_>>();
        let points = if water_paths.is_some() { straight(((to[0] - from[0]).hypot(to[1] - from[1]) as usize).max(2)) } else { road(rng, from, to, 0.7) };
        let path = Path { points, material: dirt, width: 2.0, soft: 0.5, rough: 0.3, along: false };
        if let Some(s) = &water_paths {
            bridge(m, &path, s);
        }
        m.paths.push(path.clone());
        lane = Some(path);
    }
    let avoid: Vec<Path> = m.paths.clone();
    for _ in 0..(w + h) / 2 {
        let q = [rng.random() as f32 * wf, rng.random() as f32 * hf];
        let near = q[0] > building.x as f32 - 1.5 && q[0] < (building.x + bw) as f32 + 1.5 && q[1] > building.y as f32 - 1.5 && q[1] < (building.y + bh) as f32 + 1.5;
        let on_lane = lane.as_ref().is_some_and(|l| l.distance(q) < 2.0);
        let on_water = water_paths.as_ref().is_some_and(|s| s.near(q, 0.8));
        if !near && !on_lane && !on_water {
            let n = match pick(rng, &["bush", "bush", "bush", "tree"]) {
                "tree" => Some(season_tree(rng, season)),
                _ => season_bush(rng, season),
            };
            if let Some(n) = n {
                drop(m, n, q, rng.int(0, 3) as f32 * 90.0, 0.8);
            }
        }
    }
    // Хозяйство у задней двери: у таверны — конюшенный угол, у дома — дрова.
    for r in &all {
        sp.block(r.x - 1, r.y - 1, r.w + 2, r.h + 2);
    }
    if let Some(c) = back_yard {
        let names: &[&str] = if tavern { &["cart", "hay", "hay", "trough", "firewood_pile", "barrel", "crate"] } else { &["firewood_pile", "barrel", "crate", "planter"] };
        yard(m, &mut sp, rng, names, c, &avoid);
    }
    if !p.objects.is_empty() {
        sp.block(building.x - 1, building.y - 1, bw + 2, bh + 2);
        let c = match entrance {
            Some((side, ga, gb)) => {
                let (dx, dy): (f32, f32) = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)][side as usize];
                [(ga[0] + gb[0]) / 2.0 + dx * 3.0, (ga[1] + gb[1]) / 2.0 + dy * 3.0]
            }
            None => [building.x as f32 + bw as f32 / 2.0, (building.y + bh) as f32 + 3.0],
        };
        around(m, &mut sp, rng, &p.objects, c, "двор", &avoid, 1.6, warn);
    }
    if p.mood == Mood::Abandoned {
        abandon(m, rng, &all);
    }
    placed.len()
}

/// Настроение, общее для всех видов: мрачно — темнее и тени гуще.
fn mood(m: &mut Map, p: &Plan) {
    if p.mood == Mood::Gloomy {
        m.shadows.opacity = 0.75;
        m.shadows.dist = 0.9;
        let all: Vec<(usize, f32)> = (0..m.rooms.data.len()).map(|i| (i, 1.0)).collect();
        m.shade_paint(&all, 0.3);
    }
}

/// Вся карта заново по плану. Ошибка — план не проходит проверку или не для этой
/// карты; что не удалось из задуманного — в предупреждениях отчёта.
/// `material` — пол подземелья из окна генератора; план со своим полом его перекрывает.
/// `palette` — свои ассеты из библиотеки; в деле только когда план просит
/// `assets: library`, иначе всё встроенное.
pub fn build(m: &mut Map, p: &Plan, material: usize, palette: Palette) -> Result<Report, String> {
    p.validate([m.width, m.height])?;
    if let Some(size) = p.size
        && size != [m.width, m.height]
    {
        return Err(format!("план рассчитан на карту {}×{}, а эта {}×{}", size[0], size[1], m.width, m.height));
    }
    let mut warnings = p.unused();
    let _guard = PaletteGuard;
    if p.assets == Assets::Library {
        if palette.is_empty() {
            warnings.push("в библиотеке нет своих ассетов: построено встроенными".into());
        } else {
            palette.seed(&p.seed);
            PALETTE.with(|slot| *slot.borrow_mut() = Some(palette));
        }
    }
    let mut rng = Rng::new(&format!("{}/{:?}", p.seed, p.kind));
    clear(m);
    let count = match p.kind {
        Kind::Dungeon => dungeon(m, p, material, &mut rng, &mut warnings),
        Kind::Cave => cave(m, p, &mut rng, &mut warnings),
        Kind::Forest => forest(m, p, &mut rng, &mut warnings),
        Kind::Town => town(m, p, &mut rng, &mut warnings),
        Kind::House | Kind::Tavern => interior(m, p, &mut rng, &mut warnings),
        Kind::Arena => arena::arena(m, p, &mut rng, &mut warnings),
    };
    mood(m, p);
    Ok(Report { count, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Options;
    use crate::map;
    use crate::plan::{Link, RoomSpec};
    use std::collections::VecDeque;

    /// Библиотека как у паков FA: по паре вариантов на слот, часть слотов без замены.
    fn fake_library() -> Palette {
        use crate::library::{Asset, Kind as K};
        let a = |path: &str, kind: K| Asset::new(format!("pack:fa.dungeondraft_pack|textures/x/{path}"), &format!("FA/{path}"), kind);
        let assets = vec![
            a("Furniture/Chair_Wooden_A_1x1.webp", K::Object),
            a("Furniture/Chair_Wooden_B_1x1.webp", K::Object),
            a("Furniture/Table_Long_A_2x1.webp", K::Object),
            a("Furniture/Bed_Single_A_1x2.webp", K::Object),
            a("Furniture/Bed_Double_A_2x2.webp", K::Object),
            a("Nature/Oak_Tree_A_3x3.webp", K::Object),
            a("Nature/Pine_Tree_B_3x3.webp", K::Object),
            a("Terrain/Grass_Green_A.webp", K::Floor),
            a("Terrain/Wood_Planks_A.webp", K::Floor),
            a("Walls/Wall_Timber_A.webp", K::Wall),
            a("Portals/Door_Wooden_A_1x1.webp", K::Door),
        ];
        Palette::from_library(&assets, Box::new(|_| Some([256, 256])))
    }

    #[test]
    fn library_assets_replace_builtin_slots_and_repeat() {
        let mut p = tavern_plan();
        p.assets = Assets::Library;
        let mut m = Map::new(40, 30);
        let r = build(&mut m, &p, 0, fake_library()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let count = |word: &str| m.props.iter().filter(|q| q.asset.contains(word)).count();
        assert!(count("Chair_Wooden") > 0 && count("builtin:chair") == 0, "стулья не из библиотеки");
        assert!(count("builtin:barrel") > 0, "у бочки замены нет — встроенная");
        assert!(m.materials.iter().any(|t| matches!(&t.paint, Paint::Texture(k) if k.contains("Wood_Planks"))));
        assert!(matches!(&m.wall_style.paint, Paint::Texture(k) if k.contains("Wall_Timber")));
        assert!(m.portals.iter().any(|q| q.asset.as_deref().is_some_and(|k| k.contains("Door_Wooden"))));
        // Косяки всегда встроенные, окна без замены — тоже.
        assert!(m.portals.iter().filter_map(|q| q.frame.as_deref()).all(|f| f.starts_with("builtin:")));
        assert!(m.portals.iter().filter(|q| q.kind == PortalKind::Window).all(|q| q.asset.as_deref().is_some_and(|k| k.starts_with("builtin:"))));
        // Размер своего объекта — из имени.
        let bed = m.props.iter().find(|q| q.asset.contains("Bed_")).expect("кровать");
        assert_eq!(bed.size, if bed.asset.contains("Double") { [2.0, 2.0] } else { [1.0, 2.0] });
        let mut again = Map::new(40, 30);
        build(&mut again, &p, 0, fake_library()).unwrap();
        assert_eq!(m.to_json(), again.to_json());
        // Лес: деревья чередуются между вариантами.
        let mut f = Plan::random(&opt(Kind::Forest, "лес"), [40, 30]);
        f.assets = Assets::Library;
        let mut fm = Map::new(40, 30);
        build(&mut fm, &f, 0, fake_library()).unwrap();
        assert!(fm.props.iter().any(|q| q.asset.contains("Oak_Tree")) && fm.props.iter().any(|q| q.asset.contains("Pine_Tree")));
        // Просили библиотеку, а она пуста — предупреждение и всё встроенное.
        let mut e = Map::new(40, 30);
        let r = build(&mut e, &p, 0, Palette::builtin()).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("нет своих ассетов")), "{:?}", r.warnings);
        assert!(e.props.iter().all(|q| q.asset.starts_with("builtin:")));
    }

    fn opt(kind: Kind, seed: &str) -> Options {
        Options { kind, seed: seed.into(), ..Default::default() }
    }

    fn random(m: &mut Map, kind: Kind, seed: &str) -> Report {
        let plan = Plan::random(&opt(kind, seed), [m.width, m.height]);
        build(m, &plan, 0, Palette::builtin()).unwrap()
    }

    /// Комнаты, в которые можно попасть из `start` через двери на общих стенах.
    fn reachable(m: &Map, rooms: &[Room], start: usize) -> Vec<bool> {
        let mut seen = vec![false; rooms.len()];
        seen[start] = true;
        let mut queue = VecDeque::from([start]);
        while let Some(i) = queue.pop_front() {
            for j in 0..rooms.len() {
                if seen[j] {
                    continue;
                }
                let Some(seg) = shared(rooms[i], rooms[j]) else { continue };
                let door = m.portals.iter().any(|p| p.kind != PortalKind::Window && map::segment_distance(p.center(), seg.0, seg.1) < 0.01);
                if door {
                    seen[j] = true;
                    queue.push_back(j);
                }
            }
        }
        seen
    }

    fn tavern_plan() -> Plan {
        let mut p = Plan::new(Kind::Tavern);
        p.seed = "ясень".into();
        p.rooms = vec![
            RoomSpec { size: Size::Named(SizeName::Large), name: "Зал".into(), objects: vec!["камин".into()], ..RoomSpec::new("hall", Role::Hall) },
            RoomSpec::new("kitchen", Role::Kitchen),
            RoomSpec { name: "Подвал".into(), ..RoomSpec::new("cellar", Role::Storage) },
            RoomSpec::new("pantry", Role::Storage),
            RoomSpec::new("room", Role::Bedroom),
        ];
        p.links = vec![
            Link::new("hall", OUTSIDE, LinkKind::DoubleDoor),
            Link::new("hall", "kitchen", LinkKind::Door),
            Link::new("kitchen", "pantry", LinkKind::Door),
            Link::new("hall", "cellar", LinkKind::Door),
            Link::new("cellar", "pantry", LinkKind::Secret),
            Link::new("hall", "room", LinkKind::Door),
        ];
        p
    }

    #[test]
    fn every_kind_builds_and_repeats() {
        for kind in Kind::ALL {
            for (w, h) in [(40, 30), (24, 24), (12, 10), (80, 20), (60, 60)] {
                for seed in ["а", "б", "в"] {
                    let (mut a, mut b) = (Map::new(w, h), Map::new(w, h));
                    let r = random(&mut a, kind, seed);
                    random(&mut b, kind, seed);
                    assert_eq!(a.to_json(), b.to_json(), "{kind:?} {w}×{h}");
                    if (w, h) == (40, 30) {
                        assert!(r.count > 0 && !a.props.is_empty(), "{kind:?} пуста");
                        assert!(r.warnings.is_empty(), "{kind:?}: {:?}", r.warnings);
                    }
                    for p in &a.props {
                        assert!(p.pos[0].is_finite() && p.pos[1].is_finite());
                    }
                }
            }
        }
    }

    #[test]
    fn caves_and_dungeons_have_a_way_out() {
        for seed in ["а", "б", "в", "г"] {
            let (mut c, mut d) = (Map::new(40, 30), Map::new(40, 30));
            random(&mut c, Kind::Cave, seed);
            random(&mut d, Kind::Dungeon, seed);
            assert_eq!(c.portals.iter().filter(|p| p.kind == PortalKind::Opening).count(), 2);
            assert!(d.portals.iter().any(|p| p.kind == PortalKind::DoubleDoor));
            assert!(d.props.iter().any(|p| p.asset.ends_with("stairs")));
        }
    }

    #[test]
    fn every_cave_kind_builds_and_keeps_its_look() {
        for kind in CaveKind::ALL {
            let mut p = Plan::random(&opt(Kind::Cave, "кв"), [40, 30]);
            p.options.cave = kind;
            let mut m = Map::new(40, 30);
            let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
            assert!(r.count > 0, "{kind:?}");
            assert_eq!(m.portals.iter().filter(|q| q.kind == PortalKind::Opening).count(), 2, "{kind:?}");
            // Во встроенных грибов и рогоза нет — их и не ставит.
            assert!(m.props.iter().all(|q| crate::library::builtin_size(&q.asset).is_some()), "{kind:?}");
            let has = |name: &str| m.room_floor.iter().chain(&m.terrain).any(|c| matches!(&m.materials[c.material as usize].paint, Paint::Texture(k) if k.ends_with(&format!(":{name}"))));
            match kind {
                CaveKind::Ice => assert!(has("ice") && has("snow")),
                CaveKind::Sand => assert!(has("sand") && !has("water_shallow")),
                CaveKind::Swamp => assert!(has("swamp") && has("mud")),
                CaveKind::Volcanic => assert!(has("lava") && has("ash")),
                _ => {}
            }
        }
        let json = Plan::random(&opt(Kind::Cave, "x"), [30, 20]).to_json();
        assert!(!json.replace(' ', "").contains("\"cave\":"), "каменная по умолчанию не пишется");
        let mut p = Plan::random(&opt(Kind::Cave, "x"), [30, 20]);
        p.options.cave = CaveKind::Ice;
        let json = p.to_json();
        assert!(json.replace(' ', "").contains("\"cave\":\"ice\""), "{json}");
        assert_eq!(Plan::from_json(&json.replace("\"ice\"", "\"ледяная\"")).unwrap().options.cave, CaveKind::Ice);
    }

    #[test]
    fn yards_and_streets_follow_the_season() {
        for kind in [Kind::House, Kind::Tavern, Kind::Town] {
            let mut p = Plan::random(&opt(kind, "двор"), [40, 30]);
            p.options.season = Season::Winter;
            let mut m = Map::new(40, 30);
            build(&mut m, &p, 0, Palette::builtin()).unwrap();
            let has = |name: &str| m.terrain.iter().any(|c| matches!(&m.materials[c.material as usize].paint, Paint::Texture(k) if k.ends_with(&format!(":{name}"))));
            assert!(has("snow"), "{kind:?}: снега нет");
            assert!(!has("grass_green"), "{kind:?}: зелёная трава зимой");
            // Встроенные деревья зимой подкрашены холодным; кусты, как в лесу, вечнозелёные.
            let plants: Vec<_> = m.props.iter().filter(|q| q.asset.ends_with("tree")).collect();
            assert!(plants.iter().all(|q| q.tint == Some([120, 165, 185])), "{kind:?}");
        }
    }

    #[test]
    fn builtin_forest_follows_the_season() {
        let mut p = Plan::random(&opt(Kind::Forest, "зима"), [40, 30]);
        p.options.season = Season::Winter;
        let mut m = Map::new(40, 30);
        build(&mut m, &p, 0, Palette::builtin()).unwrap();
        assert!(m.materials.iter().any(|t| matches!(&t.paint, Paint::Texture(k) if k.ends_with("snow"))));
        let trees: Vec<_> = m.props.iter().filter(|q| q.asset == "builtin:tree").collect();
        assert!(!trees.is_empty() && trees.iter().all(|q| q.tint.is_some() && q.is_canopy()));
        // Лето — как было: без подкраски.
        let mut m = Map::new(40, 30);
        build(&mut m, &Plan::random(&opt(Kind::Forest, "лето"), [40, 30]), 0, Palette::builtin()).unwrap();
        assert!(m.props.iter().all(|q| q.tint.is_none()));
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"season\":\"winter\""));
        assert_eq!(Plan::from_json(&json.replace("winter", "зима")).unwrap().options.season, Season::Winter);
    }

    #[test]
    fn pack_trees_from_the_generator_are_canopies() {
        use crate::library::{Asset, Kind as K};
        let assets = vec![Asset::new("pack:fa.dungeondraft_pack|textures/x/Trees/Tree_Oak_A_6x6.webp".into(), "FA/Trees/Tree_Oak_A_6x6.webp", K::Object)];
        let mut p = Plan::random(&opt(Kind::Forest, "кроны"), [40, 30]);
        p.assets = Assets::Library;
        let mut m = Map::new(40, 30);
        build(&mut m, &p, 0, Palette::from_library(&assets, Box::new(|_| None))).unwrap();
        let trees: Vec<_> = m.props.iter().filter(|q| q.asset.contains("Tree_Oak")).collect();
        assert!(!trees.is_empty() && trees.iter().all(|q| q.is_canopy()));
    }

    #[test]
    fn forest_keeps_camp_off_the_road_and_trees_on_top() {
        let mut m = Map::new(40, 30);
        random(&mut m, Kind::Forest, "лагерь");
        let camp = m.props.iter().find(|p| p.asset.ends_with("campfire")).unwrap();
        assert!(m.paths[2].distance(camp.pos) > 2.0);
        let first_tree = m.props.iter().position(|p| p.asset.ends_with("tree")).unwrap();
        assert!(m.props[first_tree..].iter().all(|p| p.asset.ends_with("tree")));
        // Без реки — ни моста, ни берега.
        let mut p = Plan::random(&opt(Kind::Forest, "сухо"), [40, 30]);
        p.river = River::None;
        let mut dry = Map::new(40, 30);
        build(&mut dry, &p, 0, Palette::builtin()).unwrap();
        assert_eq!(dry.paths.len(), 2);
        assert!(!dry.props.iter().any(|p| p.asset.ends_with("bridge")));
    }

    #[test]
    fn props_stay_out_of_town_houses() {
        let mut m = Map::new(48, 36);
        random(&mut m, Kind::Town, "дома");
        assert!(!m.roofs.is_empty());
        assert!(m.paths.iter().any(|p| p.along), "нет бордюра");
        for (i, a) in m.roofs.iter().enumerate() {
            for b in &m.roofs[i + 1..] {
                assert!(a.b[0] <= b.a[0] || b.b[0] <= a.a[0] || a.b[1] <= b.a[1] || b.b[1] <= a.a[1], "дома наехали");
            }
        }
        for p in m.props.iter().filter(|p| p.asset.ends_with("bench") || p.asset.ends_with("barrel")) {
            assert!(!m.roofs.iter().any(|r| p.pos[0] > r.a[0] && p.pos[0] < r.b[0] && p.pos[1] > r.a[1] && p.pos[1] < r.b[1]), "{p:?}");
        }
    }

    #[test]
    fn furniture_touches_the_wall() {
        let mut m = Map::new(30, 24);
        random(&mut m, Kind::House, "мебель");
        let segs: Vec<Segment> = m.room_edges().into_iter().chain(m.walls.iter().flat_map(|w| w.chords().collect::<Vec<_>>())).collect();
        let bed = m.props.iter().find(|p| p.asset.contains("bed")).expect("кровать");
        let slot = bed.asset.trim_start_matches(library::BUILTIN);
        let [l, t, r, u] = bounds(&Palette::builtin_pick(slot), slot, bed.rot, bed.scale);
        let near = [[bed.pos[0] + l, bed.pos[1]], [bed.pos[0] + r, bed.pos[1]], [bed.pos[0], bed.pos[1] + t], [bed.pos[0], bed.pos[1] + u]];
        let gap = near.iter().flat_map(|p| segs.iter().map(move |s| map::segment_distance(*p, s.0, s.1))).fold(f32::MAX, f32::min);
        assert!(gap < 0.35, "{gap}");
    }

    #[test]
    fn planned_tavern_has_its_rooms_links_and_secret_passage() {
        let p = tavern_plan();
        let mut m = Map::new(40, 30);
        let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
        assert_eq!(r.count, 5, "{:?}", r.warnings);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // Комнаты восстанавливаются по перегородкам: тайный ход — между подвалом и кладовой.
        let secret = m.portals.iter().filter(|q| q.kind == PortalKind::Secret).count();
        assert_eq!(secret, 1);
        assert!(m.portals.iter().any(|q| q.kind == PortalKind::DoubleDoor), "нет парадного входа");
        assert!(m.labels.iter().any(|l| l.text == "Подвал") && m.labels.iter().any(|l| l.text == "Зал"));
        assert!(m.props.iter().filter(|q| q.asset.ends_with("fireplace")).count() >= 1);
        assert!(m.props.iter().any(|q| q.asset.ends_with("bar_counter")));
        // Крыша над каждым корпусом и вся дома целиком под крышами.
        assert!(!m.roofs.is_empty());
        let covered = |x: f32, y: f32| m.roofs.iter().any(|r| x >= r.a[0] && x <= r.b[0] && y >= r.a[1] && y <= r.b[1]);
        for l in &m.labels {
            assert!(covered(l.pos[0], l.pos[1]), "«{}» без крыши", l.text);
        }
        // Тот же план — та же карта; другое зерно — другая.
        let mut again = Map::new(40, 30);
        build(&mut again, &p, 0, Palette::builtin()).unwrap();
        assert_eq!(m.to_json(), again.to_json());
        let mut other = Map::new(40, 30);
        build(&mut other, &Plan { seed: "дуб".into(), ..p.clone() }, 0, Palette::builtin()).unwrap();
        assert_ne!(m.to_json(), other.to_json());
    }

    #[test]
    fn planned_rooms_are_all_reachable() {
        for (seed, links) in [("а", true), ("б", true), ("в", false), ("г", false)] {
            let mut p = tavern_plan();
            p.seed = seed.into();
            if !links {
                p.links.clear();
            }
            let mut m = Map::new(40, 30);
            let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
            assert_eq!(r.count, 5, "{seed}: {:?}", r.warnings);
            let building = footprint(&m);
            // Комнаты — по перегородкам и внешним стенам: восстановим их из стен.
            let rooms = rooms_from_walls(&m, building);
            assert_eq!(rooms.len(), 5, "{seed}: {rooms:?}");
            let hall = rooms.iter().position(|r| m.labels.iter().any(|l| l.text == "Зал" && l.pos[0] > r.x as f32 && l.pos[0] < (r.x + r.w) as f32 && l.pos[1] > r.y as f32 && l.pos[1] < (r.y + r.h) as f32)).unwrap();
            let seen = reachable(&m, &rooms, hall);
            assert!(seen.iter().all(|&s| s), "{seed}: недостижимы {seen:?}");
            // Вход — на внешней стене дома: по одну сторону дом, по другую двор.
            assert!(m.portals.iter().any(|q| matches!(q.kind, PortalKind::Door | PortalKind::DoubleDoor) && on_outer_wall(&m, q)), "{seed}: нет входа снаружи");
        }
    }

    /// Клетка — в доме (маска комнат).
    fn in_house(m: &Map, x: i32, y: i32) -> bool {
        let r = mask::RES as isize;
        m.rooms.get(x as isize * r + r / 2, y as isize * r + r / 2) > 127
    }

    /// Рамка дома по маске комнат.
    fn footprint(m: &Map) -> Room {
        let cells: Vec<(i32, i32)> = (0..m.height as i32).flat_map(|y| (0..m.width as i32).map(move |x| (x, y))).filter(|&(x, y)| in_house(m, x, y)).collect();
        let (x0, y0) = (cells.iter().map(|c| c.0).min().unwrap(), cells.iter().map(|c| c.1).min().unwrap());
        let (x1, y1) = (cells.iter().map(|c| c.0).max().unwrap(), cells.iter().map(|c| c.1).max().unwrap());
        Room { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 }
    }

    /// Проём на стене между домом и двором: по одну сторону дом, по другую нет.
    fn on_outer_wall(m: &Map, q: &Portal) -> bool {
        let vertical = (q.a[0] - q.b[0]).abs() < 0.01;
        // Середина первой клетки проёма.
        let (x, y) = if vertical { (q.a[0].round() as i32, q.a[1].min(q.b[1]).floor() as i32) } else { (q.a[0].min(q.b[0]).floor() as i32, q.a[1].round() as i32) };
        if vertical { in_house(m, x - 1, y) != in_house(m, x, y) } else { in_house(m, x, y - 1) != in_house(m, x, y) }
    }

    /// Комнаты дома по перегородкам: клетки, разделённые стенами, — разные комнаты.
    fn rooms_from_walls(m: &Map, b: Room) -> Vec<Room> {
        let blocked = |x: i32, y: i32, nx: i32, ny: i32| -> bool {
            let (a, c) = if nx != x { let xx = x.max(nx) as f32; ([xx, y as f32], [xx, y as f32 + 1.0]) } else { let yy = y.max(ny) as f32; ([x as f32, yy], [x as f32 + 1.0, yy]) };
            let mid = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0];
            m.walls.iter().any(|w| w.distance(mid) < 0.01)
        };
        let mut owner = vec![usize::MAX; (b.w * b.h) as usize];
        let mut rooms = Vec::new();
        for s in 0..owner.len() {
            if owner[s] != usize::MAX || !in_house(m, b.x + s as i32 % b.w, b.y + s as i32 / b.w) {
                continue;
            }
            let id = rooms.len();
            let mut stack = vec![s];
            owner[s] = id;
            let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
            while let Some(i) = stack.pop() {
                let (x, y) = (b.x + i as i32 % b.w, b.y + i as i32 / b.w);
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < b.x || ny < b.y || nx >= b.x + b.w || ny >= b.y + b.h || !in_house(m, nx, ny) || blocked(x, y, nx, ny) {
                        continue;
                    }
                    let n = ((ny - b.y) * b.w + nx - b.x) as usize;
                    if owner[n] == usize::MAX {
                        owner[n] = id;
                        stack.push(n);
                    }
                }
            }
            rooms.push(Room { x: lo.0, y: lo.1, w: hi.0 - lo.0 + 1, h: hi.1 - lo.1 + 1 });
        }
        rooms
    }

    #[test]
    fn planned_dungeon_follows_roles_and_links() {
        let mut p = Plan::new(Kind::Dungeon);
        p.seed = "культ".into();
        p.rooms = vec![
            RoomSpec::new("gate", Role::Entrance),
            RoomSpec { size: Size::Named(SizeName::Large), name: "Храм".into(), ..RoomSpec::new("temple", Role::Shrine) },
            RoomSpec::new("vault", Role::Treasure),
            RoomSpec { size: Size::Cells([4, 4]), ..RoomSpec::new("stairs", Role::Exit) },
        ];
        p.links = vec![
            Link::new("gate", OUTSIDE, LinkKind::DoubleDoor),
            Link::new("gate", "temple", LinkKind::Door),
            Link::new("temple", "vault", LinkKind::Secret),
            Link::new("temple", "stairs", LinkKind::Opening),
        ];
        let mut m = Map::new(40, 30);
        let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
        assert_eq!(r.count, 4, "{:?}", r.warnings);
        assert!(m.portals.iter().any(|q| q.kind == PortalKind::Secret), "{:?}", m.portals);
        assert!(m.props.iter().any(|q| q.asset.ends_with("altar")) && m.props.iter().any(|q| q.asset.ends_with("stairs")));
        assert!(m.labels.iter().any(|l| l.text == "Храм"));
        // Ровно одна лестница: выход задан планом, дальняя комната её не дублирует.
        assert_eq!(m.props.iter().filter(|q| q.asset.ends_with("stairs")).count(), 1);
    }

    #[test]
    fn river_and_mood_change_the_yard() {
        let mut p = Plan::random(&opt(Kind::House, "река"), [36, 30]);
        p.river = River::South;
        p.links = vec![Link::new("hall", OUTSIDE, LinkKind::Bridge)];
        p.objects = vec!["колодец".into(), "телега".into()];
        p.mood = Mood::Gloomy;
        let mut m = Map::new(36, 30);
        let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(m.props.iter().any(|q| q.asset.ends_with("bridge")), "нет моста");
        // Река не уже четырёх клеток, мост длиннее берега с галькой и лежит поперёк.
        let bank = m.paths.iter().map(|q| q.width).fold(0.0, f32::max);
        assert!(bank >= 4.0 + 1.6, "река узкая: берег {bank}");
        let br = m.props.iter().find(|q| q.asset.ends_with("bridge")).unwrap();
        assert!(br.size[0] * br.scale * 0.85 >= bank, "мост короче реки: {br:?}");
        assert!((br.rot.rem_euclid(180.0) - 90.0).abs() < 15.0, "мост не поперёк: {}", br.rot);
        assert!(m.props.iter().any(|q| q.asset.ends_with("well")) && m.props.iter().any(|q| q.asset.ends_with("cart")));
        assert!(m.shade.is_some() && m.shadows.opacity > 0.7);
        // Объекты не в доме.
        for q in m.props.iter().filter(|q| q.asset.ends_with("well") || q.asset.ends_with("cart")) {
            assert!(!in_house(&m, q.pos[0].floor() as i32, q.pos[1].floor() as i32), "{q:?}");
        }
    }

    #[test]
    fn unused_fields_warn_and_bad_plan_fails() {
        let mut p = Plan::random(&opt(Kind::Cave, "x"), [30, 20]);
        p.rooms.push(RoomSpec::new("a", Role::Auto));
        let mut m = Map::new(30, 20);
        let r = build(&mut m, &p, 0, Palette::builtin()).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("комнаты не используются")), "{:?}", r.warnings);
        p.size = Some([10, 10]);
        assert!(build(&mut m, &p, 0, Palette::builtin()).unwrap_err().contains("рассчитан на карту"));
        let mut bad = Plan::new(Kind::House);
        bad.rooms = vec![RoomSpec::new("a", Role::Auto)];
        bad.links = vec![Link::new("a", "b", LinkKind::Door)];
        assert!(build(&mut m, &bad, 0, Palette::builtin()).unwrap_err().contains("«b» нет"));
    }

    #[test]
    fn example_plans_build_clean() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plans");
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        files.sort();
        assert!(files.len() >= 3, "мало примеров в {}", dir.display());
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let p = Plan::from_json(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let [w, h] = p.map_size([40, 30]);
            p.validate([w, h]).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            let mut m = Map::new(w, h);
            let material = dungeon_look(&mut m);
            let r = build(&mut m, &p, material, Palette::builtin()).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
            assert!(r.warnings.is_empty(), "{}: {:?}", f.display(), r.warnings);
            assert!(r.count > 0 && !m.props.is_empty(), "{}", f.display());
            // Карта из плана сохраняется и открывается.
            let again = Map::from_json(&m.to_json()).unwrap();
            assert_eq!(again, m);
        }
    }
}

