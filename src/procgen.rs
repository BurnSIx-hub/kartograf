//! Готовые карты из зерна: наполнение подземелья, пещера, лес, городской квартал, дом и
//! таверна. Всё строится встроенными ассетами; одно зерно с теми же настройками и размером
//! карты даёт одну и ту же карту.

use crate::generate::{Kind, Options, Rng, Room};
use crate::library;
use crate::map::{Layer, Live, Map, Material, Paint, Path, Portal, PortalKind, Prop, Roof, RoofShape, Shadows, Wall, WallStyle};
use crate::mask::{self, Brush, Shape};

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

/// Объект без проверки места: `pos` — центр.
fn drop(m: &mut Map, name: &str, pos: [f32; 2], rot: f32, scale: f32) {
    m.props.push(Prop { asset: key(name), pos, size: size(name), rot, scale, shadow: None });
}

/// Объект в свободные клетки, `x`, `y` — левый верхний угол занятого места.
fn put(m: &mut Map, sp: &mut Spots, name: &str, x: i32, y: i32, rot: f32, scale: f32) -> bool {
    let s = size(name);
    let turned = (rot / 90.0).round() as i32 % 2 != 0;
    let (fw, fh) = if turned { (s[1], s[0]) } else { (s[0], s[1]) };
    let (fw, fh) = (fw.ceil() as i32, fh.ceil() as i32);
    if !sp.free(x, y, fw, fh) {
        return false;
    }
    sp.block(x, y, fw, fh);
    drop(m, name, [x as f32 + fw as f32 / 2.0, y as f32 + fh as f32 / 2.0], rot, scale);
    true
}

/// Спиной к случайной стене комнаты.
fn by_wall(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str) -> bool {
    let s = size(name);
    for _ in 0..24 {
        let side = rng.int(0, 3);
        let (fw, fh) = if side % 2 == 0 { (s[0], s[1]) } else { (s[1], s[0]) };
        let (fw, fh) = (fw.ceil() as i32, fh.ceil() as i32);
        if fw > r.w || fh > r.h {
            continue;
        }
        let (x, y) = match side {
            0 => (rng.int(r.x, r.x + r.w - fw), r.y),
            1 => (r.x + r.w - fw, rng.int(r.y, r.y + r.h - fh)),
            2 => (rng.int(r.x, r.x + r.w - fw), r.y + r.h - fh),
            _ => (r.x, rng.int(r.y, r.y + r.h - fh)),
        };
        if put(m, sp, name, x, y, side as f32 * 90.0, 1.0) {
            return true;
        }
    }
    false
}

fn inside(m: &mut Map, sp: &mut Spots, rng: &mut Rng, r: Room, name: &str) -> bool {
    let s = size(name);
    let (fw, fh) = (s[0].ceil() as i32, s[1].ceil() as i32);
    if fw > r.w || fh > r.h {
        return false;
    }
    (0..16).any(|_| {
        let (x, y) = (rng.int(r.x, r.x + r.w - fw), rng.int(r.y, r.y + r.h - fh));
        put(m, sp, name, x, y, 0.0, 1.0)
    })
}

fn pick<'a>(rng: &mut Rng, of: &[&'a str]) -> &'a str {
    of[rng.int(0, of.len() as i32 - 1) as usize]
}

/// Стол со стульями по длинным сторонам.
fn dining(m: &mut Map, sp: &mut Spots, rng: &mut Rng, x: i32, y: i32) -> bool {
    if !put(m, sp, "table", x, y, 0.0, 1.0) {
        return false;
    }
    let seat = pick(rng, &["stool", "chair"]);
    for (i, j, rot) in [(x, y - 1, 0.0), (x + 1, y - 1, 0.0), (x, y + 1, 180.0), (x + 1, y + 1, 180.0)] {
        if rng.chance(0.8) {
            put(m, sp, seat, i, j, rot, 0.75);
        }
    }
    true
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Entrance,
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
    let big = r.w.min(r.h) >= 5;
    if big && matches!(role, Role::Shrine | Role::Treasure | Role::Entrance) {
        for (x, y) in [(r.x + 1, r.y + 1), (r.x + r.w - 2, r.y + 1), (r.x + 1, r.y + r.h - 2), (r.x + r.w - 2, r.y + r.h - 2)] {
            put(m, sp, "pillar", x, y, 0.0, 0.9);
        }
    }
    match role {
        Role::Entrance => {
            by_wall(m, sp, rng, r, "brazier");
            by_wall(m, sp, rng, r, "brazier");
            inside(m, sp, rng, r, "rubble");
        }
        Role::Treasure => {
            put(m, sp, "rug_round", r.x + r.w / 2 - 1, r.y + r.h / 2 - 1, 0.0, 1.0);
            by_wall(m, sp, rng, r, "chest");
            by_wall(m, sp, rng, r, "chest_open");
            by_wall(m, sp, rng, r, "statue");
            by_wall(m, sp, rng, r, "brazier");
        }
        Role::Shrine => {
            put(m, sp, "altar", r.x + r.w / 2, r.y + 1, 0.0, 1.0);
            by_wall(m, sp, rng, r, "brazier");
            by_wall(m, sp, rng, r, "brazier");
            by_wall(m, sp, rng, r, "statue");
        }
        Role::Barracks => {
            for _ in 0..(r.w * r.h / 8).clamp(1, 5) {
                by_wall(m, sp, rng, r, "bed");
            }
            by_wall(m, sp, rng, r, "chest_empty");
            dining(m, sp, rng, r.x + r.w / 2 - 1, r.y + r.h / 2);
        }
        Role::Storage => {
            for _ in 0..(r.w * r.h / 4).clamp(2, 10) {
                let n = pick(rng, &["crate", "barrel", "sacks"]);
                by_wall(m, sp, rng, r, n);
            }
        }
        Role::Library => {
            for _ in 0..(r.w + r.h) / 3 {
                by_wall(m, sp, rng, r, "bookshelf");
            }
            let (x, y) = (r.x + r.w / 2 - 1, r.y + r.h / 2);
            if put(m, sp, "desk", x, y, 0.0, 1.0) {
                put(m, sp, "chair", x, y - 1, 0.0, 0.8);
            }
        }
        Role::Armoury => {
            for _ in 0..(r.w + r.h) / 4 {
                let n = pick(rng, &["weapon_rack", "weapon_rack_spears", "weapon_rack_axes", "weapon_rack_maces", "weapon_rack_bows", "weapon_rack_mixed"]);
                by_wall(m, sp, rng, r, n);
            }
            by_wall(m, sp, rng, r, "anvil");
        }
        Role::Ruin => {
            for _ in 0..(r.w * r.h / 5).clamp(1, 7) {
                let n = pick(rng, &["rubble", "rubble", "boulder"]);
                inside(m, sp, rng, r, n);
            }
        }
        Role::Kitchen => {
            by_wall(m, sp, rng, r, "fireplace");
            by_wall(m, sp, rng, r, "cauldron");
            by_wall(m, sp, rng, r, "barrel");
            by_wall(m, sp, rng, r, "sacks");
            by_wall(m, sp, rng, r, "crate");
            put(m, sp, "table", r.x + r.w / 2 - 1, r.y + r.h / 2, 0.0, 1.0);
        }
        Role::Bedroom => {
            let bed = pick(rng, &["bed", "bed_double"]);
            by_wall(m, sp, rng, r, bed);
            by_wall(m, sp, rng, r, "chest");
            by_wall(m, sp, rng, r, "wardrobe");
            if big {
                inside(m, sp, rng, r, "rug_round");
            }
        }
        Role::Living => {
            by_wall(m, sp, rng, r, "fireplace");
            by_wall(m, sp, rng, r, "bookshelf");
            by_wall(m, sp, rng, r, "plant_pot");
            if r.w >= 6 && r.h >= 5 {
                put(m, sp, "rug", r.x + r.w / 2 - 1, r.y + r.h / 2 - 1, 0.0, 1.0);
            }
            by_wall(m, sp, rng, r, "sofa");
            by_wall(m, sp, rng, r, "armchair");
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

/// Двери на входах коридоров, объекты по назначению комнат. Старые объекты и двери убираются.
pub fn furnish_dungeon(m: &mut Map, cells: &[bool], rooms: &[Room], seed: &str) {
    let mut rng = Rng::new(&format!("{seed}/наполнение"));
    m.props.clear();
    m.portals.clear();
    let (w, h) = (m.width as i32, m.height as i32);
    let mut sp = Spots::new(m);
    let in_room = |x: i32, y: i32| rooms.iter().any(|r| x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h);
    let corridor = |x: i32, y: i32| x >= 0 && y >= 0 && x < w && y < h && cells[(y * w + x) as usize] && !in_room(x, y);
    for r in rooms {
        let mut doors = Vec::new();
        for x in r.x..r.x + r.w {
            for (y, out, inner) in [(r.y, r.y - 1, r.y), (r.y + r.h, r.y + r.h, r.y + r.h - 1)] {
                if corridor(x, out) {
                    let lone = !corridor(x - 1, out) && !corridor(x + 1, out);
                    doors.push((lone, [x as f32, y as f32], [x as f32 + 1.0, y as f32], x, inner));
                }
            }
        }
        for y in r.y..r.y + r.h {
            for (x, out, inner) in [(r.x, r.x - 1, r.x), (r.x + r.w, r.x + r.w, r.x + r.w - 1)] {
                if corridor(out, y) {
                    let lone = !corridor(out, y - 1) && !corridor(out, y + 1);
                    doors.push((lone, [x as f32, y as f32], [x as f32, y as f32 + 1.0], inner, y));
                }
            }
        }
        for (lone, a, b, x, y) in doors {
            sp.block(x - 1, y - 1, 3, 3);
            if lone && rng.chance(0.75) {
                m.portals.push(portal(a, b, PortalKind::Door, false));
            }
        }
    }
    let far = rooms.len().saturating_sub(1);
    for (i, r) in rooms.iter().enumerate() {
        let role = if i == 0 {
            Role::Entrance
        } else if i == far {
            Role::Treasure
        } else {
            [Role::Shrine, Role::Barracks, Role::Storage, Role::Library, Role::Armoury, Role::Ruin, Role::Storage, Role::Ruin][rng.int(0, 7) as usize]
        };
        furnish(m, &mut sp, &mut rng, *r, role);
    }
}

/// Карта с чистого листа: палитра остаётся.
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
    let cell_pos = |i: usize| [(i as i32 % w) as f32 + 0.5, (i as i32 / w) as f32 + 0.5];
    ground(m, "rocky_ground");
    let floor = mat(m, "cave_floor");
    let spots: Vec<_> = open.iter().map(|&i| (cell_pos(i), 0.95)).collect();
    let weights = blobs(m, &spots, 0.8, 1.0);
    m.paint(Layer::Rooms, floor, &weights);
    m.wall_style = wall("wall_cave");
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    // За стенами тьма: скала не должна спорить с залами.
    let dark: Vec<(usize, f32)> = m.rooms.data.iter().enumerate().filter(|(_, v)| **v < 128).map(|(i, _)| (i, 1.0)).collect();
    m.shade_paint(&dark, 0.8);

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

/// Плавная кривая через карту от края до края.
fn wander(rng: &mut Rng, a: [f32; 2], b: [f32; 2], sway: f32) -> Vec<[f32; 2]> {
    let n = 6;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let off = if i == 0 || i == n { 0.0 } else { (rng.random() as f32 - 0.5) * sway };
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            [a[0] + dx * t - dy / len * off, a[1] + dy * t + dx / len * off]
        })
        .collect()
}

fn forest(m: &mut Map, opt: &Options, rng: &mut Rng) -> usize {
    let (w, h) = (m.width as f32, m.height as f32);
    ground(m, "grass_green");
    patches(m, rng, "grass_dry", (w * h / 60.0) as i32, (2.0, 4.5), 0.6);
    let (dirt, water, gravel) = (mat(m, "dirt"), mat(m, "water_shallow"), mat(m, "gravel"));
    let centre = [w * (0.35 + rng.random() as f32 * 0.3), h * (0.35 + rng.random() as f32 * 0.3)];
    let glade = 2.5 + w.min(h) * 0.1;

    let trail = {
        let (l, r) = ([-1.0, h * (0.2 + rng.random() as f32 * 0.6)], [w + 1.0, h * (0.2 + rng.random() as f32 * 0.6)]);
        let mut pts = wander(rng, l, centre, h * 0.25);
        pts.extend(wander(rng, centre, r, h * 0.25).into_iter().skip(1));
        Path { points: pts, material: dirt, width: 1.7, soft: 0.5, rough: 0.3, along: false }
    };
    let side = rng.chance(0.5);
    let x = if side { w * 0.8 } else { w * 0.2 };
    let (top, low) = (x + (rng.random() as f32 - 0.5) * w * 0.15, x + (rng.random() as f32 - 0.5) * w * 0.15);
    let river = wander(rng, [top, -1.0], [low, h + 1.0], w * 0.15);
    let bank = Path { points: river.clone(), material: gravel, width: 3.2, soft: 0.6, rough: 0.35, along: false };
    let stream = Path { points: river, material: water, width: 2.0, soft: 0.45, rough: 0.3, along: false };
    let floor = mat(m, "forest_floor");
    let bare = blobs(m, &[(centre, glade * 0.55)], 0.2, 0.6);
    m.paint(Layer::Floor, dirt, &bare);
    m.paths = vec![bank.clone(), stream, trail.clone()];

    // Деревья дротиками: новое ставится, только если не лезет на тропу, ручей и соседей.
    let spacing = 3.4 - opt.density * 1.5;
    let mut trees: Vec<[f32; 2]> = Vec::new();
    let mut tries = (w * h * 3.0) as i32;
    while tries > 0 {
        tries -= 1;
        let p = [rng.random() as f32 * w, rng.random() as f32 * h];
        let far = |c: [f32; 2], d: f32| (c[0] - p[0]).hypot(c[1] - p[1]) >= d;
        if !far(centre, glade + 1.0) || trail.distance(p) < 2.2 || bank.distance(p) < 2.6 || !trees.iter().all(|&t| far(t, spacing)) {
            continue;
        }
        trees.push(p);
    }
    let under: Vec<_> = trees.iter().map(|&t| (t, 2.4)).collect();
    let shade = blobs(m, &under, 0.15, 0.8);
    m.paint(Layer::Floor, floor, &shade);
    for &p in &trees {
        let (rot, scale) = (rng.int(0, 3) as f32 * 90.0, 0.8 + rng.random() as f32 * 0.5);
        drop(m, "tree", p, rot, scale);
    }
    let brush = (w * h * (0.02 + opt.density * 0.03)) as i32;
    for _ in 0..brush {
        let p = [rng.random() as f32 * w, rng.random() as f32 * h];
        if trail.distance(p) > 1.5 && bank.distance(p) > 2.0 && (p[0] - centre[0]).hypot(p[1] - centre[1]) > glade * 0.7 {
            let n = pick(rng, &["bush", "bush", "bush", "boulder"]);
            drop(m, n, p, rng.int(0, 3) as f32 * 90.0, 0.6 + rng.random() as f32 * 0.6);
        }
    }
    drop(m, "campfire", centre, 0.0, 1.0);
    for (dx, dy, n) in [(-1.8, 0.6, "sacks"), (1.7, -0.8, "crate"), (0.4, 1.9, "boulder")] {
        drop(m, n, [centre[0] + dx, centre[1] + dy], 0.0, 0.9);
    }
    trees.len()
}

/// Дом с крышей: комната, стены, дверь и окна на стороне `door` (0 — север, 1 — восток, 2 — юг, 3 — запад).
fn house(m: &mut Map, rng: &mut Rng, sp: &mut Spots, r: Room, door: i32) {
    const LOOKS: [(&str, &str, bool); 7] = [
        ("wall_timber", "roof_clay_red", true),
        ("wall_timber", "roof_thatch", true),
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
    for s in [-2, 2] {
        if rng.chance(0.5) {
            let n = pick(rng, &["planter", "barrel", "crate"]);
            put(m, sp, n, fx + sx * s, fy + sy * s, 0.0, 0.8);
        }
    }
}

fn town(m: &mut Map, opt: &Options, rng: &mut Rng) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    ground(m, "dirt");
    patches(m, rng, "grass_green", w * h / 70, (1.5, 3.5), 0.8);
    let cobble = mat(m, "cobblestone");
    // Кварталы: прямоугольник делится улицами, пока делится.
    let mut todo = vec![Room { x: 0, y: 0, w, h }];
    let mut blocks = Vec::new();
    let top = 12 + ((1.0 - opt.density) * 8.0) as i32;
    while let Some(b) = todo.pop() {
        let wide = b.w >= b.h;
        let long = if wide { b.w } else { b.h };
        if long < top {
            blocks.push(b);
            continue;
        }
        let cut = rng.int(long * 2 / 5, long * 3 / 5);
        let street = 2;
        if wide {
            m.paint_rect(Layer::Floor, (b.x + cut, b.y), (b.x + cut + street - 1, b.y + b.h - 1), Some(cobble));
            todo.push(Room { w: cut, ..b });
            todo.push(Room { x: b.x + cut + street, w: b.w - cut - street, ..b });
        } else {
            m.paint_rect(Layer::Floor, (b.x, b.y + cut), (b.x + b.w - 1, b.y + cut + street - 1), Some(cobble));
            todo.push(Room { h: cut, ..b });
            todo.push(Room { y: b.y + cut + street, h: b.h - cut - street, ..b });
        }
    }
    m.wall_style = wall("wall_timber");
    let mut sp = Spots::new(m);
    // Площадь — квартал ближе всех к середине.
    let middle = blocks
        .iter()
        .copied()
        .filter(|b| b.w >= 6 && b.h >= 6)
        .min_by_key(|b| (b.x + b.w / 2 - w / 2).abs() + (b.y + b.h / 2 - h / 2).abs());
    if let Some(b) = middle {
        let paving = mat(m, "flagstones");
        m.paint_rect(Layer::Floor, (b.x, b.y), (b.x + b.w - 1, b.y + b.h - 1), Some(paving));
        let (cx, cy) = (b.x + b.w / 2, b.y + b.h / 2);
        let centre = if b.w >= 8 && b.h >= 8 { "fountain" } else { "well" };
        let s = size(centre)[0] as i32;
        put(m, &mut sp, centre, cx - s / 2, cy - s / 2, 0.0, 1.0);
        for (x, y, rot) in [(cx - 1, cy - s / 2 - 2, 0.0), (cx - 1, cy + s / 2 + 1, 180.0)] {
            put(m, &mut sp, "bench", x, y, rot, 1.0);
        }
        for _ in 0..(b.w * b.h / 20).clamp(2, 6) {
            let n = pick(rng, &["market_stall", "market_stall_blue", "market_stall_ochre"]);
            by_wall(m, &mut sp, rng, b, n);
        }
        by_wall(m, &mut sp, rng, b, "notice_board");
        for (x, y) in [(b.x, b.y), (b.x + b.w - 1, b.y), (b.x, b.y + b.h - 1), (b.x + b.w - 1, b.y + b.h - 1)] {
            put(m, &mut sp, "street_lamp", x, y, 0.0, 0.7);
        }
        by_wall(m, &mut sp, rng, b, "cart");
    }
    let mut count = 0;
    for b in blocks.iter().copied().filter(|b| Some(*b) != middle) {
        // Дома рядом вдоль длинной стороны, лицом к улице.
        let row = b.w >= b.h;
        let (long, deep) = if row { (b.w, b.h) } else { (b.h, b.w) };
        if long < 5 || deep < 4 {
            continue;
        }
        let two = deep >= 10;
        let depth = if two { (deep / 2 - 1).min(7) } else { (deep - 1).clamp(3, 7) };
        for front in [true, false].into_iter().take(if two { 2 } else { 1 }) {
            let mut t = 1;
            while t + 4 < long {
                let len = rng.int(4, 8).min(long - t - 1);
                if len < 4 {
                    break;
                }
                let d = rng.int((depth - 1).max(3), depth);
                let v = if front { 0 } else { deep - d };
                let r = if row { Room { x: b.x + t, y: b.y + v, w: len, h: d } } else { Room { x: b.x + v, y: b.y + t, w: d, h: len } };
                // Если сторона блока лежит на краю карты, улицы там нет: дверь смотрит внутрь.
                let edge = if row { if front { b.y == 0 } else { b.y + b.h == h } } else if front { b.x == 0 } else { b.x + b.w == w };
                let outward = if row { if front { 0 } else { 2 } } else if front { 3 } else { 1 };
                let door = if edge { (outward + 2) % 4 } else { outward };
                house(m, rng, &mut sp, r, door);
                count += 1;
                t += len + rng.int(1, 2);
            }
        }
        put(m, &mut sp, "street_lamp", b.x, b.y, 0.0, 0.7);
        if rng.chance(0.5) {
            put(m, &mut sp, "street_lamp", b.x + b.w - 1, b.y + b.h - 1, 0.0, 0.7);
        }
        for _ in 0..(b.w * b.h / 40) {
            let n = pick(rng, &["bush", "bush", "crate", "barrel", "planter"]);
            inside(m, &mut sp, rng, b, n);
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
    let share = if tavern { 0.8 } else { 0.6 };
    let (bw, bh) = (((w as f32 * share) as i32).clamp(6, w - 2), ((h as f32 * share) as i32).clamp(6, h - 4));
    let house = Room { x: (w - bw) / 2, y: (h - bh) / 2 - 1, w: bw, h: bh };
    let planks = mat(m, "wood_planks");
    m.paint_rect(Layer::Rooms, (house.x, house.y), (house.x + bw - 1, house.y + bh - 1), Some(planks));
    let outer = if tavern { wall("wall_timber") } else { wall(pick(rng, &["wall_timber", "wall_fieldstone", "wall_brick"])) };
    let wood = !matches!(&outer.paint, Paint::Texture(k) if k.ends_with("fieldstone"));
    m.wall_style = outer.clone();
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);
    let inner = WallStyle { width: outer.width * 0.8, ..outer };

    // Дом режется перегородками; в каждой дверь, новые перегородки в двери не упираются.
    let area = 26 + ((1.0 - opt.density) * 40.0) as i32;
    let mut todo = vec![house];
    let mut rooms: Vec<Room> = Vec::new();
    let mut doors: Vec<[f32; 2]> = Vec::new();
    while let Some(r) = todo.pop() {
        // В таверне зал во всю высоту дома: первая перегородка отрезает меньшую долю и зал больше не делится.
        let first = rooms.is_empty() && todo.is_empty() && tavern;
        let tall = r.h > r.w && !first;
        let long = if tall { r.h } else { r.w };
        if long < 7 || r.w * r.h <= area + rng.int(0, 14) {
            rooms.push(r);
            continue;
        }
        let (lo, hi) = if first { (long * 3 / 5, long * 7 / 10) } else { (3, long - 3) };
        let free: Vec<i32> = (lo..=hi)
            .filter(|c| {
                let line = if tall { r.y + c } else { r.x + c } as f32;
                !doors.iter().any(|d| (if tall { d[1] } else { d[0] } - line).abs() < 1.1)
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
        let span = if tall { r.w } else { r.h };
        let t = rng.int(1, (span - 2).max(1)) as f32;
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
    let south = house.y + house.h;
    let hall = (0..rooms.len()).filter(|&i| rooms[i].y + rooms[i].h == south).max_by_key(|&i| rooms[i].w * rooms[i].h).unwrap_or(0);
    let mut sp = Spots::new(m);
    for d in &doors {
        sp.block(d[0].floor() as i32 - 1, d[1].floor() as i32 - 1, 3, 3);
    }
    let main = rooms[hall];
    let gate = main.x + main.w / 2 - 1;
    let (ga, gb) = ([gate as f32, south as f32], [gate as f32 + 2.0, south as f32]);
    m.portals.push(portal(ga, gb, PortalKind::DoubleDoor, wood));
    sp.block(gate - 1, south - 3, 4, 3);
    // Окна на внешних стенах.
    for r in &rooms {
        for (on, a, b) in [
            (r.y == house.y, [r.x as f32 + 1.0, r.y as f32], [r.x as f32 + 3.0, r.y as f32]),
            (r.y + r.h == south && r.x != main.x, [r.x as f32 + 1.0, south as f32], [r.x as f32 + 3.0, south as f32]),
            (r.x == house.x, [r.x as f32, r.y as f32 + 1.0], [r.x as f32, r.y as f32 + 3.0]),
            (r.x + r.w == house.x + house.w, [(r.x + r.w) as f32, r.y as f32 + 1.0], [(r.x + r.w) as f32, r.y as f32 + 3.0]),
        ] {
            let len = if a[0] == b[0] { r.h } else { r.w };
            if on && len >= 4 {
                m.portals.push(portal(a, b, PortalKind::Window, wood));
            }
        }
    }

    if tavern {
        // Стойка у северной стены зала, за ней бочки; столы рядами.
        for i in 0..(main.w / 3).min(2) {
            put(m, &mut sp, "table", main.x + 1 + i * 2, main.y + 1, 0.0, 1.0);
            put(m, &mut sp, "stool", main.x + 1 + i * 2, main.y + 2, 0.0, 0.75);
            put(m, &mut sp, "stool", main.x + 2 + i * 2, main.y + 2, 0.0, 0.75);
        }
        for i in 0..(main.w / 3).min(2) * 2 {
            let n = pick(rng, &["barrel", "barrel", "crate"]);
            put(m, &mut sp, n, main.x + 1 + i, main.y, 0.0, 0.9);
        }
        by_wall(m, &mut sp, rng, main, "fireplace");
        let mut y = main.y + 4;
        while y + 1 < main.y + main.h - 1 {
            let mut x = main.x + 1;
            while x + 2 < main.x + main.w {
                if rng.chance(0.85) {
                    dining(m, &mut sp, rng, x, y);
                }
                x += 4;
            }
            y += 4;
        }
    } else {
        furnish(m, &mut sp, rng, main, Role::Living);
        dining(m, &mut sp, rng, main.x + 1, main.y + main.h - 3);
    }
    let tiles = mat(m, "stone_tiles");
    let mut order = [Role::Kitchen, Role::Storage, Role::Bedroom, Role::Bedroom, Role::Bedroom, Role::Library].into_iter();
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
    let path = wander(rng, [gate as f32 + 1.0, south as f32 + 0.3], [end, h as f32 + 1.0], 1.5);
    m.paths.push(Path { points: path, material: dirt, width: 2.0, soft: 0.5, rough: 0.3, along: false });
    for _ in 0..(w + h) / 2 {
        let p = [rng.random() as f32 * w as f32, rng.random() as f32 * h as f32];
        let near = p[0] > house.x as f32 - 1.5 && p[0] < (house.x + bw) as f32 + 1.5 && p[1] > house.y as f32 - 1.5 && p[1] < south as f32 + 1.5;
        let road = (p[0] - gate as f32 - 1.0).abs() < 3.0 && p[1] > south as f32;
        if !near && !road {
            let n = pick(rng, &["bush", "bush", "bush", "tree"]);
            drop(m, n, p, rng.int(0, 3) as f32 * 90.0, 0.8);
        }
    }
    rooms.len()
}

/// Вся карта заново, кроме подземелья: оно строится поверх местности (см. `App::generate`).
/// Возвращает, сколько получилось главного: комнат, домов, деревьев, клеток пещеры.
pub fn build(m: &mut Map, opt: &Options) -> usize {
    let mut rng = Rng::new(&format!("{}/{:?}", opt.seed, opt.kind));
    clear(m);
    match opt.kind {
        Kind::Dungeon => 0,
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
        for kind in [Kind::Cave, Kind::Forest, Kind::Town, Kind::House, Kind::Tavern] {
            for (w, h) in [(40, 30), (24, 24), (12, 10), (80, 20)] {
                for seed in ["а", "б", "в"] {
                    let (mut a, mut b) = (Map::new(w, h), Map::new(w, h));
                    let n = build(&mut a, &opt(kind, seed));
                    build(&mut b, &opt(kind, seed));
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
    fn props_stay_out_of_town_houses() {
        let mut m = Map::new(48, 36);
        build(&mut m, &opt(Kind::Town, "дома"));
        assert!(!m.roofs.is_empty());
        for p in m.props.iter().filter(|p| p.asset.ends_with("bench") || p.asset.ends_with("barrel")) {
            assert!(!m.roofs.iter().any(|r| p.pos[0] > r.a[0] && p.pos[0] < r.b[0] && p.pos[1] > r.a[1] && p.pos[1] < r.b[1]), "{p:?}");
        }
    }

    #[test]
    fn dungeon_gets_doors_and_things() {
        let o = Options { rooms: 10, seed: "клад".into(), ..Default::default() };
        let (cells, rooms) = crate::generate::dungeon(50, 40, &o);
        let mut m = Map::new(50, 40);
        furnish_dungeon(&mut m, &cells, &rooms, &o.seed);
        assert!(m.props.iter().any(|p| p.asset.ends_with("chest")));
        assert!(!m.portals.is_empty());
    }
}
