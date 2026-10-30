//! Случайное подземелье: комнаты и коридоры между ними. Как в Map Forge, одно и то же
//! зерно с теми же настройками даёт одну и ту же карту.

/// mulberry32, тот же, что в Map Forge.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: &str) -> Self {
        let mut h: u32 = 1779033703 ^ seed.encode_utf16().count() as u32;
        for c in seed.encode_utf16() {
            h = (h ^ c as u32).wrapping_mul(3432918353);
            h = h.rotate_left(13);
        }
        h = (h ^ (h >> 16)).wrapping_mul(2246822507);
        h = (h ^ (h >> 13)).wrapping_mul(3266489909);
        Self(h ^ (h >> 16))
    }

    /// [0, 1)
    pub fn random(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B79F5);
        let a = self.0;
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }

    /// [min, max] включительно.
    pub fn int(&mut self, min: i32, max: i32) -> i32 {
        min + (self.random() * (max - min + 1) as f64).floor() as i32
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.random() < p
    }
}

/// Что строить. Подземелье ложится поверх местности, остальное заменяет карту целиком.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dungeon,
    Cave,
    Forest,
    Town,
    House,
    Tavern,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Dungeon, Kind::Cave, Kind::Forest, Kind::Town, Kind::House, Kind::Tavern];

    pub fn label(self) -> &'static str {
        match self {
            Kind::Dungeon => "Случайное подземелье",
            Kind::Cave => "Пещера",
            Kind::Forest => "Лес",
            Kind::Town => "Городской квартал",
            Kind::House => "Дом",
            Kind::Tavern => "Таверна",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub kind: Kind,
    /// Подземелье: расставить двери и объекты по комнатам.
    pub fill: bool,
    /// Насколько плотно: больше деревьев, домов, комнат, камней.
    pub density: f32,
    pub rooms: u32,
    pub min_size: u32,
    pub max_size: u32,
    /// Доля лишних коридоров, дающих кольца, а не одно дерево.
    pub loops: f32,
    pub seed: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { kind: Kind::Dungeon, fill: true, density: 0.5, rooms: 8, min_size: 3, max_size: 7, loops: 0.2, seed: String::new() }
    }
}

pub fn new_seed() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{:x}", (t as u64) % 0xFFFFFF)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Room {
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// Пересекается с другой, включая зазор в клетку между ними.
    fn touches(&self, o: &Room) -> bool {
        self.x - 1 < o.x + o.w && o.x - 1 < self.x + self.w && self.y - 1 < o.y + o.h && o.y - 1 < self.y + self.h
    }
}

/// Маска клеток подземелья (`true` — пол) размером `width * height`.
pub fn dungeon(width: u32, height: u32, opt: &Options) -> (Vec<bool>, Vec<Room>) {
    let (w, h) = (width as i32, height as i32);
    let mut rng = Rng::new(&opt.seed);
    let (lo, hi) = (opt.min_size.max(2) as i32, opt.max_size.max(opt.min_size.max(2)) as i32);
    let mut rooms: Vec<Room> = Vec::new();
    for _ in 0..opt.rooms * 40 {
        if rooms.len() as u32 >= opt.rooms {
            break;
        }
        let (rw, rh) = (rng.int(lo, hi), rng.int(lo, hi));
        if rw + 2 > w || rh + 2 > h {
            continue;
        }
        let r = Room { x: rng.int(1, w - rw - 1), y: rng.int(1, h - rh - 1), w: rw, h: rh };
        if !rooms.iter().any(|o| o.touches(&r)) {
            rooms.push(r);
        }
    }

    let mut cells = vec![false; (width * height) as usize];
    let mut set = |x: i32, y: i32| {
        if x >= 0 && y >= 0 && x < w && y < h {
            cells[(y * w + x) as usize] = true;
        }
    };
    for r in &rooms {
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                set(x, y);
            }
        }
    }

    // Каждая комната соединяется с ближайшей из уже соединённых: так связно всё.
    let mut links: Vec<(usize, usize)> = Vec::new();
    for i in 1..rooms.len() {
        let (cx, cy) = rooms[i].center();
        let near = (0..i).min_by_key(|&j| {
            let (x, y) = rooms[j].center();
            (x - cx).abs() + (y - cy).abs()
        });
        if let Some(j) = near {
            links.push((i, j));
        }
        if i >= 2 && rng.chance(opt.loops as f64) {
            let j = rng.int(0, i as i32 - 1) as usize;
            if !links.contains(&(i, j)) {
                links.push((i, j));
            }
        }
    }
    for (i, j) in links {
        let (a, b) = (rooms[i].center(), rooms[j].center());
        let horizontal_first = rng.chance(0.5);
        let corner = if horizontal_first { (b.0, a.1) } else { (a.0, b.1) };
        for (p, q) in [(a, corner), (corner, b)] {
            for x in p.0.min(q.0)..=p.0.max(q.0) {
                for y in p.1.min(q.1)..=p.1.max(q.1) {
                    set(x, y);
                }
            }
        }
    }
    (cells, rooms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_dungeon() {
        let opt = Options { seed: "пещера".into(), ..Default::default() };
        assert_eq!(dungeon(40, 30, &opt), dungeon(40, 30, &opt));
        let other = Options { seed: "другая".into(), ..opt.clone() };
        assert_ne!(dungeon(40, 30, &opt).0, dungeon(40, 30, &other).0);
    }

    #[test]
    fn rng_matches_map_forge() {
        // Числа, которые даёт rng.mjs из Map Forge для того же зерна (снято в node).
        let mut r = Rng::new("проверка");
        assert_eq!(r.random(), 0.3776834295131266);
        assert_eq!(r.random(), 0.40034142951481044);
    }

    #[test]
    fn all_rooms_are_connected() {
        let opt = Options { rooms: 10, seed: "связь".into(), ..Default::default() };
        let (cells, rooms) = dungeon(50, 40, &opt);
        assert!(rooms.len() >= 5);
        // Заливка от первой комнаты должна дойти до всех.
        let (w, h) = (50i32, 40i32);
        let mut seen = vec![false; cells.len()];
        let start = (rooms[0].y * w + rooms[0].x) as usize;
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            if seen[i] || !cells[i] {
                continue;
            }
            seen[i] = true;
            let (x, y) = (i as i32 % w, i as i32 / w);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && nx < w && ny < h {
                    stack.push((ny * w + nx) as usize);
                }
            }
        }
        for r in &rooms {
            assert!(seen[(r.y * w + r.x) as usize], "комната {r:?} отрезана");
        }
    }
}
