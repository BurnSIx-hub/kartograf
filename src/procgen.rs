//! Готовые карты из зерна: подземелье, пещера, лес, городской квартал, дом и таверна.
//! Всё строится встроенными ассетами; одно зерно с теми же настройками и размером карты
//! даёт одну и ту же карту.

use crate::generate::{self, Kind, Options, Rng, Room};
use crate::library;
use crate::map::{self, Layer, Live, Map, Material, Paint, Path, Portal, PortalKind, Prop, Roof, RoofShape, Segment, Shadows, Wall, WallStyle};
use crate::mask::{self, Brush, Mask, Shape};

include!("prop_bounds.rs");

/// Мебель в домах крупнее настоящей: на столе играют фигурки, а не меряют комнаты.
const FURNITURE: f32 = 1.35;
/// Сколько от оси стены до предмета: половина её толщины.
const WALL_GAP: f32 = 0.2;

fn key(name: &str) -> String {
    format!("{}{name}", library::BUILTIN)
}

fn mat(m: &mut Map, name: &str) -> usize {
    let k = key(name);
    m.material_index(Material { name: library::label(&k), paint: Paint::Texture(k), vary: false, live: Live::Auto }).unwrap_or(0)
}

fn wall(name: &str) -> WallStyle {
    let k = key(name);
    let width = library::builtin_wall_width(&k).unwrap_or(0.35);
    WallStyle { paint: Paint::Texture(k), width }
}

fn size(name: &str) -> [f32; 2] {
    library::builtin_size(&key(name)).unwrap_or([1.0, 1.0])
}

/// Видимый предмет от центра с учётом поворота на четверти: лево, верх, право, низ.
fn bounds(name: &str, rot: f32, scale: f32) -> [f32; 4] {
    let s = size(name);
    let b = BOUNDS.iter().find(|t| t.0 == name).map_or([-s[0] / 2.0, -s[1] / 2.0, s[0] / 2.0, s[1] / 2.0], |t| t.1);
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

    fn free(&self, x: i32, y: i32, w: i32, h: i32) -> bool {
        x >= 0 && y >= 0 && x + w <= self.w && y + h <= self.h && (y..y + h).all(|j| (x..x + w).all(|i| !self.used[(j * self.w + i) as usize]))
    }
}

/// Объект без проверки места: `pos` — центр холста.
fn drop(m: &mut Map, name: &str, pos: [f32; 2], rot: f32, scale: f32) {
    m.props.push(Prop { asset: key(name), pos, size: size(name), rot, scale, shadow: None });
}

/// Объект в свободные клетки, `x`, `y` — левый верхний угол занятого места.
fn put(m: &mut Map, sp: &mut Spots, name: &str, x: i32, y: i32, rot: f32, scale: f32) -> bool {
    let [l, t, r, u] = bounds(name, rot, scale);
    let (fw, fh) = (cells_of(r - l), cells_of(u - t));
    if !sp.free(x, y, fw, fh) {
        return false;
    }
    sp.block(x, y, fw, fh);
    drop(m, name, [x as f32 + fw as f32 / 2.0 - (l + r) / 2.0, y as f32 + fh as f32 / 2.0 - (t + u) / 2.0], rot, scale);
    true
}

/// Спиной вплотную к случайной стене комнаты; `corner` — ещё и боком к соседней.
fn by_wall(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str, scale: f32, corner: bool) -> bool {
    for _ in 0..40 {
        let side = rng.int(0, 3);
        let rot = side as f32 * 90.0;
        let [left, top, right, low] = bounds(name, rot, scale);
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
        drop(m, name, pos, rot, scale);
        return true;
    }
    false
}

fn inside(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str, scale: f32) -> bool {
    let [left, top, right, low] = bounds(name, 0.0, scale);
    let (fw, fh) = (cells_of(right - left), cells_of(low - top));
    if fw > r.w || fh > r.h {
        return false;
    }
    (0..16).any(|_| {
        let (x, y) = (rng.int(r.x, r.x + r.w - fw), rng.int(r.y, r.y + r.h - fh));
        put(m, sp, name, x, y, 0.0, scale)
    })
}

fn pick<'a>(rng: &mut Rng, of: &[&'a str]) -> &'a str {
    of[rng.int(0, of.len() as i32 - 1) as usize]
}

/// Стол с местами вокруг. Занимает 3×4 клетки от `x`, `y`.
fn dining(m: &mut Map, sp: &mut Spots, rng: &mut Rng, x: i32, y: i32) -> bool {
    if !sp.free(x, y, 3, 4) {
        return false;
    }
    let seat = pick(rng, &["stool", "chair"]);
    if rng.chance(0.3) {
        put(m, sp, "table_round", x, y + 1, 0.0, 1.1);
        for (i, j, rot) in [(x + 1, y, 0.0), (x + 1, y + 3, 180.0), (x + 2, y + 1, 90.0)] {
            put(m, sp, seat, i, j, rot, FURNITURE);
        }
    } else {
        put(m, sp, "table", x, y + 1, 0.0, FURNITURE);
        for (i, j, rot) in [(x, y, 0.0), (x + 2, y, 0.0), (x, y + 3, 180.0), (x + 2, y + 3, 180.0)] {
            if rng.chance(0.85) {
                put(m, sp, seat, i, j, rot, FURNITURE);
            }
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Entrance,
    Exit,
    Treasure,
    Shrine,
    Barracks,
    Storage,
    Library,
    Armoury,
    Ruin,
    Kitchen,
    Bedroom,
    Living,
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
            for _ in 0..(r.w * r.h / 4).clamp(3, 12) {
                let n = pick(rng, &["crate", "barrel", "sacks", "crate"]);
                by_wall(m, sp, rng, r, n, f, false);
            }
        }
        Role::Library => {
            for _ in 0..(r.w + r.h) / 3 {
                by_wall(m, sp, rng, r, "bookshelf", 1.0, false);
            }
            let (x, y) = (r.x + r.w / 2 - 1, r.y + r.h / 2);
            if put(m, sp, "desk", x, y, 0.0, f) {
                put(m, sp, "chair", x + 1, y - 1, 0.0, f);
            }
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
            for n in ["barrel", "sacks", "crate", "barrel"] {
                by_wall(m, sp, rng, r, n, f, true);
            }
            put(m, sp, "table", r.x + r.w / 2 - 1, r.y + r.h / 2, 0.0, f);
        }
        Role::Bedroom => {
            let bed = pick(rng, &["bed", "bed_double"]);
            by_wall(m, sp, rng, r, bed, f, true);
            by_wall(m, sp, rng, r, "chest", f, false);
            by_wall(m, sp, rng, r, "wardrobe", f, true);
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
        Role::Living => {
            by_wall(m, sp, rng, r, "fireplace", 1.0, false);
            by_wall(m, sp, rng, r, "bookshelf", 1.0, false);
            by_wall(m, sp, rng, r, "sofa", f, false);
            by_wall(m, sp, rng, r, "armchair", f, true);
            by_wall(m, sp, rng, r, "plant_pot", f, true);
            if r.w >= 6 && r.h >= 6 {
                put(m, sp, "rug", r.x + r.w / 2 - 2, r.y + r.h / 2 - 1, 0.0, 1.0);
            }
        }
    }
}

fn portal(a: [f32; 2], b: [f32; 2], kind: PortalKind, wood: bool) -> Portal {
    let style = if wood { "wood" } else { "stone" };
    let len = ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()).round() as i32;
    let asset = if kind == PortalKind::Window { format!("window_{style}") } else { "door_wood".to_owned() };
    let frame = (kind != PortalKind::Window).then(|| key(&format!("frame_{style}_{}", len.clamp(1, 3))));
    Portal { a, b, kind, asset: Some(key(&asset)), frame }
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
fn dark_outside(m: &mut Map) {
    let dark: Vec<(usize, f32)> = m.rooms.data.iter().enumerate().filter(|(_, v)| **v < 128).map(|(i, _)| (i, 1.0)).collect();
    m.shade_paint(&dark, 0.8);
}

fn dungeon(m: &mut Map, opt: &Options, material: usize, rng: &mut Rng) -> usize {
    let (mut cells, rooms) = generate::dungeon(m.width, m.height, opt);
    let (w, h) = (m.width as i32, m.height as i32);
    ground(m, "rocky_ground");
    if rooms.is_empty() {
        return 0;
    }
    // Коридор в клетку при толстых стенах — щель: ширим до двух.
    let was = cells.clone();
    let in_room = |x: i32, y: i32| rooms.iter().any(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h);
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            if was[(y * w + x) as usize] && !in_room(x, y) {
                for (i, j) in [(1, 0), (0, 1), (1, 1)] {
                    cells[((y + j) * w + x + i) as usize] = true;
                }
            }
        }
    }
    // Вход — ворота на краю карты: ближайшая к краю комната получает ход наружу.
    let reach = |r: &Room| [r.y, r.x, h - r.y - r.h, w - r.x - r.w];
    let first = (0..rooms.len()).min_by_key(|&i| *reach(&rooms[i]).iter().min().unwrap_or(&0)).unwrap_or(0);
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
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    dark_outside(m);
    m.portals.push(portal(gate.0, gate.1, PortalKind::DoubleDoor, false));
    if opt.fill {
        furnish_dungeon(m, &cells, &rooms, first, rng);
    }
    rooms.len()
}

/// Двери на входах коридоров, объекты по назначению комнат.
fn furnish_dungeon(m: &mut Map, cells: &[bool], rooms: &[Room], first: usize, rng: &mut Rng) {
    let (w, h) = (m.width as i32, m.height as i32);
    let mut sp = Spots::new(m);
    let in_room = |x: i32, y: i32| rooms.iter().any(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h);
    let corridor = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && cells[(y * w + x) as usize] && !in_room(x, y);
    for r in rooms {
        // Вход коридора — ровно две клетки у стены комнаты; шире — это уже не проём.
        let mut doors = Vec::new();
        for x in r.x..r.x + r.w {
            for (y, out, inner) in [(r.y, r.y - 1, r.y), (r.y + r.h, r.y + r.h, r.y + r.h - 1)] {
                if corridor(x, out) && !corridor(x - 1, out) {
                    let pair = corridor(x + 1, out) && !corridor(x + 2, out);
                    doors.push((pair, [x as f32, y as f32], [x as f32 + 2.0, y as f32], x, inner));
                }
            }
        }
        for y in r.y..r.y + r.h {
            for (x, out, inner) in [(r.x, r.x - 1, r.x), (r.x + r.w, r.x + r.w, r.x + r.w - 1)] {
                if corridor(out, y) && !corridor(out, y - 1) {
                    let pair = corridor(out, y + 1) && !corridor(out, y + 2);
                    doors.push((pair, [x as f32, y as f32], [x as f32, y as f32 + 2.0], inner, y));
                }
            }
        }
        for (pair, a, b, x, y) in doors {
            sp.block(x - 1, y - 1, 4, 4);
            if pair && rng.chance(0.6) {
                m.portals.push(portal(a, b, PortalKind::DoubleDoor, false));
            }
        }
    }
    // Чем дальше от входа, тем важнее комната: в самой дальней лестница вниз, перед ней клад.
    let (ox, oy) = rooms[first].center();
    let mut order: Vec<usize> = (0..rooms.len()).collect();
    order.sort_by_key(|&i| {
        let (x, y) = rooms[i].center();
        (x - ox).abs() + (y - oy).abs()
    });
    let last = order.len() - 1;
    for (k, &i) in order.iter().enumerate() {
        let role = if k == 0 {
            Role::Entrance
        } else if k == last {
            Role::Exit
        } else if k + 1 == last {
            Role::Treasure
        } else {
            [Role::Shrine, Role::Barracks, Role::Storage, Role::Library, Role::Armoury, Role::Ruin, Role::Storage, Role::Ruin][rng.int(0, 7) as usize]
        };
        furnish(m, &mut sp, rng, rooms[i], role);
    }
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

fn cave(m: &mut Map, opt: &Options, rng: &mut Rng) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    let at = |x: i32, y: i32| (y * w + x) as usize;
    let mut open = Vec::new();
    // Клеточный автомат: стены собираются в массивы, пустоты — в залы. Берём самую большую пустоту.
    for _ in 0..30 {
        let fill = 0.5 + opt.density as f64 * 0.1;
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
    ground(m, "rocky_ground");
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
    let floor = mat(m, "cave_floor");
    let spots: Vec<_> = open.iter().map(|&i| (cell_pos(i), 0.95)).collect();
    let weights = blobs(m, &spots, 0.8, 1.0);
    m.paint(Layer::Rooms, floor, &weights);
    m.wall_style = wall("wall_cave");
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    dark_outside(m);
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
    for (name, count, radius, flow) in [("gravel", 8, 2.2, 0.8), ("moss", 7, 1.8, 0.7), ("rocky_ground", 6, 2.4, 0.6)] {
        let g = mat(m, name);
        let mut spots = Vec::new();
        for _ in 0..count {
            spots.push((cell_pos(open[rng.int(0, open.len() as i32 - 1) as usize]), radius));
        }
        let weights = inner(m, blobs(m, &spots, 0.2, flow));
        m.paint(Layer::Rooms, g, &weights);
    }
    if !deep.is_empty() {
        let (shallow, depth) = (mat(m, "water_shallow"), mat(m, "water_deep"));
        for _ in 0..rng.int(1, 2) {
            let c = cell_pos(deep[rng.int(0, deep.len() as i32 - 1) as usize]);
            let mut lobes = Vec::new();
            for _ in 0..4 {
                let (dx, dy) = (rng.random() as f32 * 2.0 - 1.0, rng.random() as f32 * 2.0 - 1.0);
                lobes.push(([c[0] + dx, c[1] + dy], 1.1 + rng.random() as f32 * 0.6));
            }
            let weights = inner(m, blobs(m, &lobes, 0.6, 1.0));
            m.paint(Layer::Rooms, shallow, &weights);
            let weights = inner(m, blobs(m, &[(c, 0.9)], 0.3, 0.9));
            m.paint(Layer::Rooms, depth, &weights);
            sp.block(c[0] as i32 - 3, c[1] as i32 - 3, 7, 7);
        }
    }
    let scatter = (open.len() as f32 * (0.03 + opt.density * 0.05)) as i32;
    for _ in 0..scatter {
        let i = open[rng.int(0, open.len() as i32 - 1) as usize] as i32;
        let n = pick(rng, &["boulder", "boulder", "rubble"]);
        put(m, &mut sp, n, i % w, i / w, 0.0, 0.7 + rng.random() as f32 * 0.5);
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

fn forest(m: &mut Map, opt: &Options, rng: &mut Rng) -> usize {
    let (w, h) = (m.width as f32, m.height as f32);
    ground(m, "grass_green");
    patches(m, rng, "grass_dry", (w * h / 60.0) as i32, (2.0, 4.5), 0.6);
    let (dirt, water, gravel) = (mat(m, "dirt"), mat(m, "water_shallow"), mat(m, "gravel"));
    let glade = 2.5 + w.min(h) * 0.08;

    let (left, right) = (h * (0.25 + rng.random() as f32 * 0.5), h * (0.25 + rng.random() as f32 * 0.5));
    let line = road(rng, [-1.0, left], [w + 1.0, right], h * 0.14);
    let trail = Path { points: line, material: dirt, width: 2.0, soft: 0.5, rough: 0.25, along: false };
    let side = rng.chance(0.5);
    let x = if side { w * 0.75 } else { w * 0.25 };
    let (top, low) = (x + (rng.random() as f32 - 0.5) * w * 0.15, x + (rng.random() as f32 - 0.5) * w * 0.15);
    let river = road(rng, [top, -1.0], [low, h + 1.0], w * 0.09);
    let bank = Path { points: river.clone(), material: gravel, width: 3.2, soft: 0.6, rough: 0.35, along: false };
    let stream = Path { points: river, material: water, width: 2.0, soft: 0.45, rough: 0.3, along: false };

    // Лагерь в стороне от дороги и ручья, к нему идёт своя тропинка.
    let mut camp = [w / 2.0, h / 2.0];
    let mut best = f32::MIN;
    for _ in 0..200 {
        let p = [w * (0.15 + rng.random() as f32 * 0.7), h * (0.15 + rng.random() as f32 * 0.7)];
        let room = trail.distance(p).min(bank.distance(p));
        let score = -(room - glade - 2.0).abs();
        if score > best {
            best = score;
            camp = p;
        }
    }
    let line = trail.curve();
    let join = line.iter().copied().min_by(|a, b| (a[0] - camp[0]).hypot(a[1] - camp[1]).total_cmp(&(b[0] - camp[0]).hypot(b[1] - camp[1]))).unwrap_or(camp);
    let side_path = Path { points: road(rng, join, camp, 0.8), material: dirt, width: 1.1, soft: 0.6, rough: 0.3, along: false };
    let floor = mat(m, "forest_floor");
    let bare = blobs(m, &[(camp, glade * 0.55)], 0.2, 0.6);
    m.paint(Layer::Floor, dirt, &bare);
    m.paths = vec![bank.clone(), stream.clone(), trail.clone(), side_path.clone()];
    let bridge = crossing(&trail, &stream);

    // Деревья дротиками: новое ставится, только если не лезет на тропу, ручей и соседей.
    let spacing = 3.4 - opt.density * 1.5;
    let mut trees: Vec<[f32; 2]> = Vec::new();
    let mut tries = (w * h * 3.0) as i32;
    while tries > 0 {
        tries -= 1;
        let p = [rng.random() as f32 * w, rng.random() as f32 * h];
        let far = |c: [f32; 2], d: f32| (c[0] - p[0]).hypot(c[1] - p[1]) >= d;
        if !far(camp, glade + 1.0) || trail.distance(p) < 2.2 || side_path.distance(p) < 1.6 || bank.distance(p) < 2.6 || !trees.iter().all(|&t| far(t, spacing)) {
            continue;
        }
        trees.push(p);
    }
    let under: Vec<_> = trees.iter().map(|&t| (t, 2.4)).collect();
    let shade = blobs(m, &under, 0.15, 0.8);
    m.paint(Layer::Floor, floor, &shade);
    // Сначала низкое, деревья последними: кроны накрывают камни и кусты, а не наоборот.
    let brush = (w * h * (0.02 + opt.density * 0.03)) as i32;
    for _ in 0..brush {
        let p = [rng.random() as f32 * w, rng.random() as f32 * h];
        let clear_of_tree = trees.iter().all(|t| (t[0] - p[0]).hypot(t[1] - p[1]) > 1.4);
        if clear_of_tree && trail.distance(p) > 1.5 && side_path.distance(p) > 1.2 && bank.distance(p) > 2.0 && (p[0] - camp[0]).hypot(p[1] - camp[1]) > glade * 0.7 {
            let n = pick(rng, &["bush", "bush", "bush", "boulder"]);
            drop(m, n, p, rng.int(0, 3) as f32 * 90.0, 0.6 + rng.random() as f32 * 0.6);
        }
    }
    drop(m, "campfire", camp, 0.0, 1.0);
    for (dx, dy, n) in [(-1.8, 0.6, "sacks"), (1.7, -0.8, "crate"), (0.4, 1.9, "boulder"), (-1.2, -1.6, "barrel")] {
        drop(m, n, [camp[0] + dx, camp[1] + dy], 0.0, 0.9);
    }
    if let Some((at, angle)) = bridge {
        drop(m, "bridge", at, angle, 1.0);
    }
    for &p in &trees {
        let (rot, scale) = (rng.int(0, 3) as f32 * 90.0, 0.8 + rng.random() as f32 * 0.5);
        drop(m, "tree", p, rot, scale);
    }
    trees.len()
}

/// Дом с крышей: комната, стены, дверь и окна на стороне `door` (0 — север, 1 — восток, 2 — юг, 3 — запад).
fn house(m: &mut Map, rng: &mut Rng, sp: &mut Spots, r: Room, door: i32) {
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
    let (st, roof, wood) = LOOKS[rng.int(0, LOOKS.len() as i32 - 1) as usize];
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

fn town(m: &mut Map, opt: &Options, rng: &mut Rng) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    ground(m, "dirt");
    patches(m, rng, "grass_green", w * h / 90, (1.2, 3.0), 0.75);
    patches(m, rng, "mud", w * h / 160, (0.8, 2.0), 0.6);
    patches(m, rng, "gravel", w * h / 200, (0.8, 1.6), 0.5);
    // Кварталы: прямоугольник делится прямыми улицами разной ширины.
    let mut todo = vec![Room { x: 0, y: 0, w, h }];
    let mut blocks = Vec::new();
    let mut streets: Vec<Room> = Vec::new();
    let top = 12 + ((1.0 - opt.density) * 8.0) as i32;
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
        let paving = mat(m, "flagstones");
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
        let s = size(centre)[0] as i32;
        let (ox, oy) = (rng.int(-1, 1), rng.int(-1, 1));
        put(m, &mut sp, centre, cx - s / 2 + ox, cy - s / 2 + oy, 0.0, 1.0);
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
    }
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
                    drop(m, "tree", c, rng.int(0, 3) as f32 * 90.0, 0.8);
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
                    house(m, rng, &mut sp, r, door);
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
            inside(m, &mut sp, rng, b, n, 0.9);
        }
    }
    let edges = m.room_edges();
    let looks = std::mem::take(&mut m.edge_looks);
    m.sync_edge_looks(&edges);
    m.edge_looks.extend(looks);
    count
}

fn interior(m: &mut Map, opt: &Options, rng: &mut Rng, tavern: bool) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    ground(m, "grass_green");
    patches(m, rng, "grass_dry", w * h / 60, (2.0, 4.0), 0.6);
    // Здание не растёт вместе с картой: дом на 40 клеток — это уже не дом.
    let (max_w, max_h) = if tavern { (28, 22) } else { (18, 14) };
    let (bw, bh) = (((w as f32 * 0.8) as i32).min(max_w).clamp(6, (w - 2).max(6)), ((h as f32 * 0.7) as i32).min(max_h).clamp(6, (h - 4).max(6)));
    let building = Room { x: (w - bw) / 2, y: ((h - bh) / 2 - 1).max(0), w: bw, h: bh };
    let planks = mat(m, "wood_planks");
    m.paint_rect(Layer::Rooms, (building.x, building.y), (building.x + bw - 1, building.y + bh - 1), Some(planks));
    let outer = if tavern { wall("wall_timber") } else { wall(pick(rng, &["wall_timber", "wall_fieldstone", "wall_brick"])) };
    let wood = !matches!(&outer.paint, Paint::Texture(k) if k.ends_with("fieldstone"));
    m.wall_style = outer.clone();
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    let inner = WallStyle { width: outer.width * 0.8, ..outer };

    // Дом режется перегородками; в каждой дверь, новые перегородки в двери не упираются.
    let area = 30 + ((1.0 - opt.density) * 36.0) as i32;
    let mut todo = vec![building];
    let mut rooms: Vec<Room> = Vec::new();
    let mut doors: Vec<[f32; 2]> = Vec::new();
    while let Some(r) = todo.pop() {
        // В таверне зал во всю высоту дома: первая перегородка отрезает меньшую долю и зал больше не делится.
        let first = rooms.is_empty() && todo.is_empty() && tavern;
        let tall = r.h > r.w && !first;
        let long = if tall { r.h } else { r.w };
        if long < 9 || r.w * r.h <= area + rng.int(0, 14) {
            rooms.push(r);
            continue;
        }
        let (lo, hi) = if first { (long * 3 / 5, long * 7 / 10) } else { (4, long - 4) };
        let free: Vec<i32> = (lo..=hi)
            .filter(|c| {
                let line = if tall { r.y + c } else { r.x + c } as f32;
                let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, (r.x + r.w) as f32, (r.y + r.h) as f32);
                let here = |d: &&[f32; 2]| d[0] >= x0 - 0.6 && d[0] <= x1 + 0.6 && d[1] >= y0 - 0.6 && d[1] <= y1 + 0.6;
                !doors.iter().filter(here).any(|d| (if tall { d[1] } else { d[0] } - line).abs() < 2.1)
            })
            .collect();
        if free.is_empty() {
            rooms.push(r);
            continue;
        }
        let cut = free[rng.int(0, free.len() as i32 - 1) as usize];
        let (a, b, p, q) = if tall {
            let y = (r.y + cut) as f32;
            ([r.x as f32, y], [(r.x + r.w) as f32, y], Room { h: cut, ..r }, Room { y: r.y + cut, h: r.h - cut, ..r })
        } else {
            let x = (r.x + cut) as f32;
            ([x, r.y as f32], [x, (r.y + r.h) as f32], Room { w: cut, ..r }, Room { x: r.x + cut, w: r.w - cut, ..r })
        };
        m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), style: Some(inner.clone()) });
        // Дверь не прижата к углу: косяк шире проёма и иначе уходит в поперечную стену.
        let span = if tall { r.w } else { r.h };
        let t = rng.int(2, (span - 3).max(2)) as f32;
        let (da, db) = if tall { ([a[0] + t, a[1]], [a[0] + t + 1.0, a[1]]) } else { ([a[0], a[1] + t], [a[0], a[1] + t + 1.0]) };
        m.portals.push(portal(da, db, PortalKind::Door, true));
        doors.push([(da[0] + db[0]) / 2.0, (da[1] + db[1]) / 2.0]);
        if first {
            rooms.push(p);
        } else {
            todo.push(p);
        }
        todo.push(q);
    }

    // Зал — самая большая комната у южной стены: через неё вход.
    let south = building.y + building.h;
    let hall = (0..rooms.len()).filter(|&i| rooms[i].y + rooms[i].h == south).max_by_key(|&i| rooms[i].w * rooms[i].h).unwrap_or(0);
    let mut sp = Spots::new(m);
    for d in &doors {
        sp.block(d[0].floor() as i32 - 1, d[1].floor() as i32 - 1, 3, 3);
    }
    let main = rooms[hall];
    let wide_gate = main.w >= 6;
    let gate = main.x + main.w / 2 - i32::from(wide_gate);
    let gate_len = if wide_gate { 2.0 } else { 1.0 };
    let (ga, gb) = ([gate as f32, south as f32], [gate as f32 + gate_len, south as f32]);
    m.portals.push(portal(ga, gb, if wide_gate { PortalKind::DoubleDoor } else { PortalKind::Door }, wood));
    sp.block(gate - 1, south - 3, 4, 3);
    // Окна на внешних стенах.
    for r in &rooms {
        for (on, a, b) in [
            (r.y == building.y, [r.x as f32 + 2.0, r.y as f32], [r.x as f32 + 4.0, r.y as f32]),
            (r.y + r.h == south && r.x != main.x, [r.x as f32 + 2.0, south as f32], [r.x as f32 + 4.0, south as f32]),
            (r.x == building.x, [r.x as f32, r.y as f32 + 2.0], [r.x as f32, r.y as f32 + 4.0]),
            (r.x + r.w == building.x + building.w, [(r.x + r.w) as f32, r.y as f32 + 2.0], [(r.x + r.w) as f32, r.y as f32 + 4.0]),
        ] {
            let len = if a[0] == b[0] { r.h } else { r.w };
            if on && len >= 6 {
                m.portals.push(portal(a, b, PortalKind::Window, wood));
            }
        }
    }

    if tavern {
        // Стойка отгораживает угол зала: за ней бочки и полки, перед ней табуреты.
        let scale = ((main.w - 3) as f32 / 7.0).clamp(0.6, 1.0);
        let [left, top, right, low] = bounds("bar_counter", 0.0, scale);
        let y = main.y as f32 + 2.2 - top;
        let x = main.x as f32 + 1.6 - left;
        drop(m, "bar_counter", [x, y], 0.0, scale);
        let width = cells_of(right - left);
        sp.block(main.x, main.y, width + 2, 5);
        for i in 0..width / 2 {
            drop(m, pick(rng, &["barrel", "barrel", "crate"]), [main.x as f32 + 1.0 + i as f32 * 2.0, main.y as f32 + 0.75], 0.0, FURNITURE);
        }
        for i in 0..(width + 1) / 2 {
            drop(m, "stool", [main.x as f32 + 2.2 + i as f32 * 2.0, y + low + 0.5], 0.0, FURNITURE);
        }
        by_wall(m, &mut sp, rng, main, "fireplace", 1.0, false);
        let mut ty = main.y + 5;
        while ty + 3 < main.y + main.h - 1 {
            let mut tx = main.x + 1 + rng.int(0, 1);
            while tx + 3 < main.x + main.w {
                if rng.chance(0.85) {
                    dining(m, &mut sp, rng, tx, ty);
                }
                tx += 5;
            }
            ty += 5;
        }
    } else {
        furnish(m, &mut sp, rng, main, Role::Living);
        dining(m, &mut sp, rng, main.x + 1, main.y + main.h - 5);
    }
    let tiles = mat(m, "stone_tiles");
    let mut order = [Role::Kitchen, Role::Storage, Role::Bedroom, Role::Bedroom, Role::Library, Role::Bedroom, Role::Storage].into_iter();
    for (i, r) in rooms.iter().enumerate() {
        if i == hall {
            continue;
        }
        let role = order.next().unwrap_or(Role::Bedroom);
        if role == Role::Kitchen {
            m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(tiles));
        }
        furnish(m, &mut sp, rng, *r, role);
    }

    // Снаружи: дорожка от двери к югу, двор вокруг.
    let dirt = mat(m, "dirt");
    let end = gate as f32 + 1.0 + (rng.random() as f32 - 0.5) * 6.0;
    let path = road(rng, [gate as f32 + 1.0, south as f32 + 0.3], [end, h as f32 + 1.0], 0.7);
    m.paths.push(Path { points: path, material: dirt, width: 2.0, soft: 0.5, rough: 0.3, along: false });
    for _ in 0..(w + h) / 2 {
        let p = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
        let near = p[0] > building.x as f32 - 1.5 && p[0] < (building.x + bw) as f32 + 1.5 && p[1] > building.y as f32 - 1.5 && p[1] < south as f32 + 1.5;
        let lane = (p[0] - gate as f32 - 1.0).abs() < 3.0 && p[1] > south as f32;
        if !near && !lane {
            let n = pick(rng, &["bush", "bush", "bush", "tree"]);
            drop(m, n, p, rng.int(0, 3) as f32 * 90.0, 0.8);
        }
    }
    rooms.len()
}

/// Вся карта заново. Возвращает, сколько получилось главного: комнат, домов, деревьев.
/// `material` — пол подземелья; у остальных видов покрытия свои.
pub fn build(m: &mut Map, opt: &Options, material: usize) -> usize {
    let mut rng = Rng::new(&format!("{}/{:?}", opt.seed, opt.kind));
    clear(m);
    match opt.kind {
        Kind::Dungeon => dungeon(m, opt, material, &mut rng),
        Kind::Cave => cave(m, opt, &mut rng),
        Kind::Forest => forest(m, opt, &mut rng),
        Kind::Town => town(m, opt, &mut rng),
        Kind::House => interior(m, opt, &mut rng, false),
        Kind::Tavern => interior(m, opt, &mut rng, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(kind: Kind, seed: &str) -> Options {
        Options { kind, seed: seed.into(), ..Default::default() }
    }

    #[test]
    fn every_kind_builds_and_repeats() {
        for kind in Kind::ALL {
            for (w, h) in [(40, 30), (24, 24), (12, 10), (80, 20), (60, 60)] {
                for seed in ["а", "б", "в"] {
                    let (mut a, mut b) = (Map::new(w, h), Map::new(w, h));
                    let n = build(&mut a, &opt(kind, seed), 0);
                    build(&mut b, &opt(kind, seed), 0);
                    assert_eq!(a.to_json(), b.to_json(), "{kind:?} {w}×{h}");
                    if (w, h) == (40, 30) {
                        assert!(n > 0 && !a.props.is_empty(), "{kind:?} пуста");
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
            build(&mut c, &opt(Kind::Cave, seed), 0);
            build(&mut d, &opt(Kind::Dungeon, seed), 0);
            assert_eq!(c.portals.iter().filter(|p| p.kind == PortalKind::Opening).count(), 2);
            assert!(d.portals.iter().any(|p| p.kind == PortalKind::DoubleDoor));
            assert!(d.props.iter().any(|p| p.asset.ends_with("stairs")));
        }
    }

    #[test]
    fn forest_keeps_camp_off_the_road_and_trees_on_top() {
        let mut m = Map::new(40, 30);
        build(&mut m, &opt(Kind::Forest, "лагерь"), 0);
        let camp = m.props.iter().find(|p| p.asset.ends_with("campfire")).unwrap();
        assert!(m.paths[2].distance(camp.pos) > 2.0);
        let first_tree = m.props.iter().position(|p| p.asset.ends_with("tree")).unwrap();
        assert!(m.props[first_tree..].iter().all(|p| p.asset.ends_with("tree")));
    }

    #[test]
    fn props_stay_out_of_town_houses() {
        let mut m = Map::new(48, 36);
        build(&mut m, &opt(Kind::Town, "дома"), 0);
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
        build(&mut m, &opt(Kind::House, "мебель"), 0);
        let segs: Vec<Segment> = m.room_edges().into_iter().chain(m.walls.iter().flat_map(|w| w.chords().collect::<Vec<_>>())).collect();
        let bed = m.props.iter().find(|p| p.asset.contains("bed")).expect("кровать");
        let [l, t, r, u] = bounds(bed.asset.trim_start_matches(library::BUILTIN), bed.rot, bed.scale);
        let near = [[bed.pos[0] + l, bed.pos[1]], [bed.pos[0] + r, bed.pos[1]], [bed.pos[0], bed.pos[1] + t], [bed.pos[0], bed.pos[1] + u]];
        let gap = near.iter().flat_map(|p| segs.iter().map(move |s| map::segment_distance(*p, s.0, s.1))).fold(f32::MAX, f32::min);
        assert!(gap < 0.35, "{gap}");
    }
}
