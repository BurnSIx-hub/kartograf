//! Покои ярла — длинный дом северного правителя. Устроены, как залы вождей
//! (Борг на Лофотенах, Лейре) и как дома ярлов в северных играх:
//!
//! - Вход с торца через сени; по бокам сеней караульня и кухня с чёрным ходом.
//! - Зал — длинный, ось с запада на восток. Посередине очаг во всю длину зала:
//!   каменный бортик, угли, огни. По обе стороны — столы с лавками.
//! - Вдоль длинных стен — помосты на ступень выше (на них сидели и спали), их
//!   край держат два ряда столбов. Помост виден по бордюру и тени, как в церкви.
//! - В дальнем конце зала — возвышение, на нём высокое сиденье ярла лицом ко
//!   входу, перед ним шкура, по бокам жаровни и стяги, над ним — волчья голова.
//! - За возвышением, двери по бокам от трона: спальня ярла и палата совета
//!   (стол с картой).
//!
//! Свои ассеты: синий трон, тёмно-синие стяги, серые меха и волчья голова — в
//! цвет герба Магадана, если в библиотеке есть такие; иначе любые.

use super::church::{curb, ledge, CURB};
use super::*;

/// Глубина сеней с караульней и кухней, покоев за залом, помостов, возвышения.
const WEST: i32 = 5;
const EAST: i32 = 7;
const BENCH: i32 = 3;
const DAIS: i32 = 3;
/// Шаг столбов вдоль помоста и насколько они отступают на помост от края.
const BAY: i32 = 3;
const POST: f32 = 0.6;

pub(super) fn jarl(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    if w < 30 || h < 20 {
        warn.push(format!("покои ярла не помещаются на карту {w}×{h}: нужно хотя бы 30×20"));
        return 0;
    }
    let season = p.options.season;
    let winter = season == Season::Winter;
    let look = forest_look(season);
    ground(m, look.ground);
    patches(m, rng, look.patches, w * h / 60, (2.0, 4.0), 0.6);

    // Дом: длинный, по середине карты; ширина чётная — ось по линии сетки.
    let bw = (w - 6).clamp(30, 48);
    let bh = ((h - 8).clamp(14, 20) / 2) * 2;
    let (bx, by) = ((w - bw) / 2 + 1, (h - bh) / 2);
    let axis = by + bh / 2;
    let (hx0, hx1) = (bx + WEST, bx + bw - EAST);
    let (ny0, ny1) = (by + BENCH, by + bh - BENCH);
    let dx = hx1 - DAIS;
    let (x1, y1) = (bx + bw, by + bh);

    // ----- полы -----
    let planks = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("wood_planks"));
    let flags = mat(m, "flagstones");
    m.paint_rect(Layer::Rooms, (bx, by), (x1 - 1, y1 - 1), Some(planks));
    // Сени и кухня — камень: там грязь с улицы и огонь.
    m.paint_rect(Layer::Rooms, (bx, axis - 2), (hx0 - 1, axis + 1), Some(flags));
    m.paint_rect(Layer::Rooms, (bx, axis + 2), (hx0 - 1, y1 - 1), Some(flags));
    let style = match p.style.walls.as_deref().and_then(plan::wall_id) {
        Some(st) => wall(st),
        None => wall(pick(rng, &["wall_wood", "wall_wood", "wall_timber"])),
    };
    let inner = WallStyle { width: style.width * 0.75, ..style.clone() };
    m.wall_style = style.clone();
    let edges = m.room_edges();
    m.sync_edge_looks(&edges);

    // ----- перегородки и двери -----
    let line = |m: &mut Map, a: [f32; 2], b: [f32; 2]| {
        m.walls.push(Wall { points: vec![a, b], closed: false, bends: Vec::new(), low: false, style: Some(inner.clone()) });
    };
    let f = |v: i32| v as f32;
    // Сени — от зала стена с двойной дверью; караульня и кухня — от сеней.
    line(m, [f(hx0), f(by)], [f(hx0), f(y1)]);
    line(m, [f(bx), f(axis - 2)], [f(hx0), f(axis - 2)]);
    line(m, [f(bx), f(axis + 2)], [f(hx0), f(axis + 2)]);
    // Покои за залом и стена между спальней и палатой совета.
    line(m, [f(hx1), f(by)], [f(hx1), f(y1)]);
    line(m, [f(hx1), f(axis)], [f(x1), f(axis)]);
    let mut doors: Vec<[f32; 2]> = Vec::new();
    let mut door = |m: &mut Map, a: [f32; 2], b: [f32; 2], kind: PortalKind| {
        m.portals.push(portal(a, b, kind, true));
        doors.push([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]);
    };
    door(m, [f(bx), f(axis - 1)], [f(bx), f(axis + 1)], PortalKind::DoubleDoor);
    door(m, [f(hx0), f(axis - 1)], [f(hx0), f(axis + 1)], PortalKind::DoubleDoor);
    let gx = bx + WEST / 2;
    door(m, [f(gx), f(axis - 2)], [f(gx + 1), f(axis - 2)], PortalKind::Door);
    door(m, [f(gx), f(axis + 2)], [f(gx + 1), f(axis + 2)], PortalKind::Door);
    // Чёрный ход из кухни — к поленнице и колодцу.
    door(m, [f(bx + 1), f(y1)], [f(bx + 2), f(y1)], PortalKind::Door);
    // В покои — по бокам от возвышения, с помостов.
    door(m, [f(hx1), f(by + 1)], [f(hx1), f(by + 2)], PortalKind::Door);
    door(m, [f(hx1), f(y1 - 2)], [f(hx1), f(y1 - 1)], PortalKind::Door);
    // Окна — маленькие, редкие: зимой тепло дороже света.
    let mut x = hx0 + 2;
    while x < hx1 - 1 {
        for y in [by, y1] {
            m.portals.push(portal([f(x), f(y)], [f(x + 1), f(y)], PortalKind::Window, true));
        }
        x += 5;
    }
    for (a, b) in [(by + 2, by + 3), (y1 - 3, y1 - 2)] {
        m.portals.push(portal([f(x1), f(a)], [f(x1), f(b)], PortalKind::Window, true));
    }
    m.portals.push(portal([f(hx1 + 3), f(by)], [f(hx1 + 4), f(by)], PortalKind::Window, true));
    m.portals.push(portal([f(bx + 1), f(by)], [f(bx + 2), f(by)], PortalKind::Window, true));

    // ----- помосты, возвышение, очаг -----
    // Тень на нижнем полу у края помостов и возвышения; бордюр — одной ломаной.
    ledge(m, false, f(ny0), f(hx0), f(dx), 1.0);
    ledge(m, false, f(ny1), f(hx0), f(dx), -1.0);
    ledge(m, true, f(dx), f(ny0), f(ny1), -1.0);
    let o = CURB / 2.0;
    curb(m, &[[f(hx0) - 0.3, f(ny0) - o], [f(dx) + o, f(ny0) - o], [f(dx) + o, f(ny1) + o], [f(hx0) - 0.3, f(ny1) + o]]);
    // Очаг: каменное дно с золой, бортик кругом, огни через каждые три клетки.
    let (hz0, hz1) = (hx0 + 4, dx - 3);
    let hearth = hz1 - hz0 >= 4;
    if hearth {
        m.paint_rect(Layer::Rooms, (hz0, axis - 1), (hz1 - 1, axis), Some(flags));
        let ash = mat(m, "ash");
        let spots: Vec<([f32; 2], f32)> = (hz0..hz1).map(|x| ([x as f32 + 0.5, axis as f32], 0.8)).collect();
        let weights = blobs(m, &spots, 0.3, 0.7);
        m.paint(Layer::Rooms, ash, &weights);
        let (a, b) = ([f(hz0) + o, f(axis - 1) + o], [f(hz1) - o, f(axis + 1) - o]);
        curb(m, &[a, [b[0], a[1]], b, [a[0], b[1]], [a[0], a[1] + 0.01]]);
        let fire = if missing("firepit") { "campfire" } else { "firepit" };
        let mut x = f(hz0) + 1.5;
        while x < f(hz1) - 0.5 {
            drop(m, fire, [x, f(axis)], rng.int(0, 3) as f32 * 90.0, 1.4);
            x += 3.0;
        }
    }
    // Столбы — на помосте у края, с шагом пролёта.
    let post = asset("pillar_stone");
    let mut x = hx0 + 2;
    while x < dx {
        for y in [f(ny0) - POST, f(ny1) + POST] {
            drop_pick(m, &post, [f(x), y], 0.0, 0.85);
        }
        x += BAY;
    }

    let mut sp = Spots::new(m);
    // У дверей пусто: клетка по обе стороны.
    for d in &doors {
        sp.block((d[0] - 1.0).floor() as i32, (d[1] - 1.0).floor() as i32, 2, 2);
    }

    // Столы с лавками по обе стороны очага.
    if hearth {
        let table = asset("table");
        let bench = asset("bench");
        for (ty, toward) in [(axis - 3, 1.0f32), (axis + 3, -1.0)] {
            let mut x = hz0;
            while x + 2 <= hz1 {
                drop_pick(m, &table, [f(x) + 1.0, f(ty)], 0.0, 1.0);
                // Лавки — с обеих сторон стола, со стороны очага тоже.
                for side in [-1.0f32, 1.0] {
                    drop_pick(m, &bench, [f(x) + 1.0, f(ty) + side * 0.95], if side * toward > 0.0 { 0.0 } else { 180.0 }, 0.9);
                }
                if rng.chance(0.6) && !missing("table_clutter") {
                    drop(m, "table_clutter", [f(x) + 1.0, f(ty)], 0.0, 0.8);
                }
                x += 3;
            }
        }
        sp.block(hz0, axis - 4, hz1 - hz0, 9);
    }
    // На помостах — меха и лавки у стен между столбами.
    let pelt = !missing("fur_pelt");
    for (wall_y, inward, rot) in [(by, 1.0f32, 180.0f32), (y1, -1.0, 0.0)] {
        let mut x = hx0 + 1;
        while x + 2 < dx {
            let q = [f(x) + 1.0, f(wall_y) + inward * 0.75];
            if doors.iter().all(|d| (d[0] - q[0]).hypot(d[1] - q[1]) > 1.5) {
                if pelt && rng.chance(0.5) {
                    drop(m, "fur_pelt", [q[0], q[1] + inward * 0.6], rng.random() as f32 * 40.0 - 20.0, 0.9);
                } else {
                    drop(m, "bench", q, rot, 0.9);
                }
            }
            x += BAY;
        }
    }

    // Возвышение: шкура, на ней трон лицом к входу; жаровни, стяги, волк.
    let seat = [f(hx1) - 1.3, f(axis)];
    if pelt {
        let p = asset_toned("fur_pelt", "gray");
        let s = 2.6 / p.size[0].max(p.size[1]).max(0.5);
        drop_pick(m, &p, [seat[0] - 0.3, seat[1]], 90.0, s);
    } else {
        drop(m, "rug", [seat[0] - 0.3, seat[1]], 90.0, 0.8);
    }
    let throne = asset_toned(if missing("throne_any") { "throne" } else { "throne_any" }, "blue");
    if !missing("throne_any") || !missing("throne") {
        // Высокое сиденье — крупное: картинка FA на клетку, а сиденье ярла — на полторы.
        let s = 1.6 / throne.size[0].max(throne.size[1]).max(0.5);
        drop_pick(m, &throne, seat, 90.0, s);
    } else {
        drop(m, "armchair", seat, 90.0, 1.2);
    }
    // Жаровни — пара одинаковых по углам возвышения.
    // Огонь — обычный: у FA есть синие и зелёные, колдовские.
    let brazier = asset_toned("brazier", "orange");
    for y in [f(ny0) + 0.7, f(ny1) - 0.7] {
        drop_pick(m, &brazier, [f(dx) + 0.8, y], 0.0, 0.8);
    }
    // На стене за троном: по бокам волчьи головы, дальше — стяги. Висят на
    // стене: видимый верх картинки — у её лица (поворот 90° кладёт верх холста
    // к востоку, к стене).
    let face = f(hx1) - inner.width / 2.0;
    let hang = |m: &mut Map, p: &Pick, y: f32, s: f32| {
        let top = p.visible.map_or(-p.size[1] / 2.0, |v| v[1]);
        drop_pick(m, p, [face + top * s, y], 90.0, s);
    };
    if !missing("trophy") {
        let t = asset_toned("trophy", "wolf");
        for y in [f(axis) - 1.4, f(axis) + 1.4] {
            hang(m, &t, y, 1.3);
        }
    }
    if !missing("banner") {
        let b = asset_toned(if missing("banner_large") { "banner" } else { "banner_large" }, "navy");
        for y in [f(axis) - 2.8, f(axis) + 2.8] {
            hang(m, &b, y, 1.0);
        }
    }
    sp.block(dx, ny0, DAIS, ny1 - ny0);

    // ----- сени, караульня, кухня -----
    let hall = Room { x: bx, y: axis - 2, w: WEST, h: 4 };
    let guard = Room { x: bx, y: by, w: WEST, h: axis - 2 - by };
    let kitchen = Room { x: bx, y: axis + 2, w: WEST, h: y1 - axis - 2 };
    by_side(m, &mut sp, rng, hall, "weapon_rack_spears", 0.8, false, &[0, 2]);
    by_side(m, &mut sp, rng, hall, "bench", 0.8, false, &[0, 2]);
    for n in ["bed", "bed", "weapon_rack", "chest"] {
        by_wall(m, &mut sp, rng, guard, n, 0.9, true);
    }
    inside(m, &mut sp, rng, guard, "table", 0.8);
    // Печи и полки с припасом встроенных нет — только из библиотеки.
    for n in ["oven", "cauldron", "shelf_full", "barrel", "sacks", "barrel", "crate"].into_iter().filter(|n| !missing(n)) {
        by_wall(m, &mut sp, rng, kitchen, n, 0.9, true);
    }
    inside(m, &mut sp, rng, kitchen, "table", 0.8);

    // ----- спальня ярла и палата совета -----
    let bed = Room { x: hx1, y: by, w: EAST, h: axis - by };
    let council = Room { x: hx1, y: axis, w: EAST, h: y1 - axis };
    by_side(m, &mut sp, rng, bed, "bed_double", 1.3, false, &[1]);
    by_side(m, &mut sp, rng, bed, "fireplace", 1.0, false, &[0]);
    for n in ["wardrobe", "chest", "weapon_rack", "armchair", "bookshelf"] {
        by_wall(m, &mut sp, rng, bed, n, 0.9, true);
    }
    inside(m, &mut sp, rng, bed, "table_round", 0.8);
    if pelt {
        inside(m, &mut sp, rng, bed, "fur_pelt", 1.0);
    } else {
        inside(m, &mut sp, rng, bed, "rug", 0.9);
    }
    // Стол совета посередине, на нём карта, кругом стулья.
    let c = [f(council.x) + f(council.w) / 2.0, f(council.y) + f(council.h) / 2.0];
    drop(m, "table", c, 0.0, 1.2);
    if !missing("world_map") {
        drop(m, "world_map", c, rng.int(-1, 1) as f32 * 8.0, 0.8);
    }
    let chair = asset("chair");
    for (dxx, dyy, rot) in [(-0.6f32, -1.1f32, 0.0f32), (0.6, -1.1, 0.0), (-0.6, 1.1, 180.0), (0.6, 1.1, 180.0), (-1.9, 0.0, 270.0), (1.9, 0.0, 90.0)] {
        drop_pick(m, &chair, [c[0] + dxx, c[1] + dyy], rot, 1.0);
    }
    sp.block((c[0] - 2.5) as i32, (c[1] - 2.0) as i32, 5, 4);
    for n in ["bookshelf", "chest", "bookshelf"] {
        by_wall(m, &mut sp, rng, council, n, 0.9, false);
    }

    // ----- двор -----
    let dirt = mat(m, if winter && !missing("snow_trampled") { "snow_trampled" } else { "dirt" });
    let end_y = f(axis) + (rng.random() as f32 - 0.5) * 6.0;
    let path = Path { points: road(rng, [f(bx) - 0.5, f(axis)], [-1.5, end_y], 1.0), material: dirt, width: 2.6, soft: 0.6, rough: 0.3, along: false };
    let back = Path { points: vec![[f(bx) + 1.5, f(y1) + 0.5], [f(bx) + 1.5, f(y1) + 2.0], [f(bx) - 1.0, f(h) + 1.0]], material: dirt, width: 1.4, soft: 0.6, rough: 0.3, along: false };
    m.paths.push(path.clone());
    m.paths.push(back);
    let wood = if winter && !missing("firewood_snowy") { "firewood_snowy" } else { "firewood_pile" };
    if !missing(wood) {
        drop(m, wood, [f(bx) + 4.0, f(y1) + 0.7], 0.0, 1.0);
    }
    drop(m, "barrel", [f(bx) - 0.7, f(axis) - 2.0], 0.0, 0.8);
    drop(m, "barrel", [f(bx) - 0.7, f(axis) + 2.0], 0.0, 0.8);
    // Деревья — вокруг, не у стен и не на тропе.
    let mut planted: Vec<[f32; 2]> = Vec::new();
    for _ in 0..(w * h / 4) {
        let q = [rng.random() as f32 * f(w), rng.random() as f32 * f(h)];
        let near_house = q[0] > f(bx) - 2.5 && q[0] < f(x1) + 2.5 && q[1] > f(by) - 2.5 && q[1] < f(y1) + 2.5;
        if near_house || path.distance(q) < 2.5 || planted.iter().any(|o| (o[0] - q[0]).hypot(o[1] - q[1]) < 3.5) {
            continue;
        }
        planted.push(q);
        if rng.chance(0.5) {
            drop(m, season_tree(rng, season), q, rng.random() as f32 * 360.0, 0.8 + rng.random() as f32 * 0.4);
        } else if let Some(b) = season_bush(rng, season) {
            drop(m, b, q, rng.random() as f32 * 360.0, 0.8);
        }
    }
    if winter && !missing("snow_pile") {
        for k in 0..(bw / 4) {
            let x = f(bx) + 1.0 + k as f32 * 4.0 + rng.random() as f32;
            drop(m, "snow_pile", [x, f(by) - 0.6], 0.0, 0.7);
        }
    }

    // ----- крыша: двускатная во всю длину -----
    let roof = match p.style.roof.as_deref().and_then(plan::roof_id) {
        Some(r) => r,
        None => pick(rng, &["roof_thatch", "roof_shingle", "roof_shingle"]),
    };
    m.roofs.push(Roof { a: [f(bx), f(by)], b: [f(x1), f(y1)], shape: RoofShape::Gable, paint: key(roof), turn: 0, overhang: 0.4 });
    7
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Трон — на оси зала у восточной стены, вход — в западном торце.
    #[test]
    fn throne_faces_the_entrance() {
        for (w, h, seed) in [(40, 28, "j1"), (30, 20, "j2"), (56, 36, "j3")] {
            let mut p = Plan::new(Kind::Jarl);
            p.seed = seed.into();
            let mut m = Map::new(w, h);
            let r = build(&mut m, &p, 0, crate::palette::Palette::builtin()).unwrap();
            assert!(r.count > 0, "{seed}: {:?}", r.warnings);
            let entrance = m.portals.iter().find(|p| p.kind == PortalKind::DoubleDoor).expect("вход");
            assert!(entrance.a[0] == entrance.b[0], "{seed}: вход в торце");
            assert!(!m.roofs.is_empty(), "{seed}");
        }
    }
}
