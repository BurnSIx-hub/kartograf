//! Городская стена с воротами. Устроена, как стены средневековых городов:
//!
//! - Куртина — толстая стена; по её верху ход (2 клетки), снаружи парапет с
//!   зубцами, изнутри низкий край. Высоту видно по тени: стена кладёт её на
//!   улицу у своего подножия, а у самой стены темно со всех сторон.
//! - Башни выступают из стены наружу, чтобы бить вдоль неё, и стоят на
//!   расстоянии выстрела друг от друга (здесь — через 7–10 клеток куртины).
//!   Круглые (посередине стены, наружу и внутрь поровну) под шатром или
//!   квадратные (наружу) под вальмой; одна — открытая площадка с баллистой.
//!   С хода по стене в башню ведёт дверь.
//! - Ворота — проезд между двумя привратными башнями: снаружи решётка, изнутри
//!   створки; по бокам караульни с дверями в проезд и на ход по стене.
//! - Снаружи ров с мостом напротив ворот и дорога в поле; у стены деревьев нет.
//! - Изнутри вдоль стены улица (её держали свободной, чтобы гонять по ней
//!   защитников), к ходу по стене поднимаются лестницы; от ворот в город идёт
//!   главная улица, по ней — городской квартал.
//!
//! На ровной карте всё в одном уровне: ход по стене отделён от земли низкими
//! стенами — в VTT их не пройти, а видно поверх одной (со стены — и поле, и
//! город; с поля в город — нет).

use super::*;
use std::f32::consts::TAU;

/// Ширина хода по стене, улицы вдоль неё изнутри, насколько башни выступают наружу.
const THICK: i32 = 2;
const LANE: i32 = 4;
const OUT: i32 = 3;
/// Ширина квадратной башни и поперечник круглой, привратной башни, проезда ворот.
const TOWER: i32 = 5;
const ROUND: i32 = 6;
const GATE_TOWER: i32 = 4;
const PASS: i32 = 3;
/// Высота стены в клетках (около 6 м) и тени: как у колизея.
const WALL_H: f32 = 4.0;
const SUN: f32 = 1.25;
const SHADE: f32 = 0.5;
const CONTACT: f32 = 0.42;
const CONTACT_REACH: f32 = 0.9;

/// Башня: левый край, полукруглая ли, открытая ли (с баллистой).
struct Tower {
    x: i32,
    round: bool,
    open: bool,
}

impl Tower {
    fn width(&self) -> i32 {
        if self.round { ROUND } else { TOWER }
    }

    /// Занятые клетки: круглая — круг серединой на оси хода, квадратная — от
    /// внутренней грани стены наружу.
    fn rect(&self, wy: i32) -> Room {
        if self.round {
            Room { x: self.x, y: wy + THICK / 2 - ROUND / 2, w: ROUND, h: ROUND }
        } else {
            Room { x: self.x, y: wy - OUT, w: TOWER, h: OUT + THICK }
        }
    }
}

pub(super) fn town_wall(m: &mut Map, p: &Plan, rng: &mut Rng, warn: &mut Vec<String>) -> usize {
    let (w, h) = (m.width as i32, m.height as i32);
    if w < 24 || h < 18 {
        warn.push(format!("городская стена не помещается на карту {w}×{h}: нужно хотя бы 24×18"));
        return 0;
    }
    let season = p.options.season;
    let winter = season == Season::Winter;
    let look = forest_look(season);
    ground(m, look.ground);
    patches(m, rng, look.patches, w * h / 60, (2.0, 4.0), 0.6);

    // Наружная грань стены — по линии `wy`; ход по стене — `wy..wy+THICK`.
    let wy = ((h as f32 * 0.32).round() as i32).clamp(8, 14);
    let y_in = wy + THICK + LANE;
    let h_in = h - y_in;

    // Город за стеной — квартал на своей карте, вклеенный ниже улицы вдоль стены.
    // Его главная улица — поперёк стены: ворота встают на неё.
    let mut street = Room { x: w / 2 - 2, y: y_in, w: 4, h: h_in };
    let mut houses = 0;
    if h_in >= 12 {
        let mut sub = Map::new(w as u32, h_in as u32);
        sub.shadows = m.shadows.clone();
        let mut sub_rng = Rng::new(&format!("{}/town", p.seed));
        let (n, road) = quarter::quarter_in(&mut sub, p, &mut sub_rng, warn, Some(false), true);
        houses = n;
        street = Room { x: road.x, y: y_in, w: road.w, h: h_in };
        paste(m, &sub, y_in);
    }
    let x0 = ((street.x as f32 + street.w as f32 / 2.0 - PASS as f32 / 2.0).round() as i32).clamp(GATE_TOWER + 2, w - PASS - GATE_TOWER - 2);
    let gate = Room { x: x0 - GATE_TOWER, y: wy - OUT, w: PASS + 2 * GATE_TOWER, h: OUT + THICK + 1 };
    let gc = x0 as f32 + PASS as f32 / 2.0;

    // Башни от ворот в обе стороны.
    let round = rng.chance(0.6);
    let tw = if round { ROUND } else { TOWER };
    let mut towers: Vec<Tower> = Vec::new();
    let mut tx = gate.x;
    loop {
        tx -= rng.int(7, 10) + tw;
        if tx < 1 {
            break;
        }
        towers.push(Tower { x: tx, round, open: false });
    }
    let mut tx = gate.x + gate.w;
    loop {
        tx += rng.int(7, 10);
        if tx + tw > w - 1 {
            break;
        }
        towers.push(Tower { x: tx, round, open: false });
        tx += tw;
    }
    towers.sort_by_key(|t| t.x);
    if towers.len() >= 2 {
        let k = rng.int(0, towers.len() as i32 - 1) as usize;
        towers[k].open = true;
    }

    // ----- земля -----
    let dirt = mat(m, "dirt");
    let flags = mat(m, p.style.floor.as_deref().and_then(plan::texture_id).unwrap_or("flagstones"));
    let cobble = mat(m, if winter && !missing("cobblestone_snowy") { "cobblestone_snowy" } else { "cobblestone" });
    // Ход по стене — плиты.
    m.paint_rect(Layer::Floor, (0, wy), (w - 1, wy + THICK - 1), Some(flags));
    // Улица вдоль стены — утоптанная земля, от ворот в город — брусчатка.
    let lane_mat = if winter && !missing("snow_trampled") { mat(m, "snow_trampled") } else { dirt };
    m.paint_rect(Layer::Floor, (0, wy + THICK), (w - 1, y_in - 1), Some(lane_mat));
    let mut worn = Vec::new();
    for _ in 0..(w / 3) {
        worn.push(([rng.random() as f32 * w as f32, (wy + THICK) as f32 + rng.random() as f32 * LANE as f32], 0.5 + rng.random() as f32 * 0.6));
    }
    let gravel = mat(m, "gravel");
    let weights = blobs(m, &worn, 0.1, 0.5);
    m.paint(Layer::Floor, gravel, &weights);
    m.paint_rect(Layer::Floor, (street.x.min(x0), wy + THICK), ((street.x + street.w).max(x0 + PASS) - 1, y_in - 1), Some(cobble));

    // Ров и дорога в поле.
    let foot = (wy - OUT) as f32;
    let moat_y = foot - 1.0 - 2.8;
    let moat = (moat_y >= 3.5).then(|| {
        let line = road(rng, [-1.0, moat_y], [w as f32 + 1.0, moat_y], 0.35);
        let mut s = stream(m, line, 4.0);
        if winter && !missing("ice") {
            let ice = mat(m, "ice");
            s.water.material = ice;
            s.deep.material = ice;
        }
        s
    });
    let road_line = {
        let mut pts = vec![[gc, foot + 0.5], [gc, foot - 1.0]];
        let end = gc + (rng.random() as f32 - 0.5) * w as f32 * 0.3;
        let top = road(rng, [gc, foot - 1.0], [end, -1.5], w as f32 * 0.04);
        pts.extend(top.into_iter().skip(1));
        pts
    };
    let field_road = Path { points: road_line.clone(), material: dirt, width: 3.0, soft: 0.5, rough: 0.25, along: false };
    // У ворот дорога мощёная.
    let paved = Path { points: vec![[gc, foot + 0.5], [gc, foot - 0.5], [gc, foot - 1.5], [gc, foot - 2.5]], material: cobble, width: PASS as f32 + 0.6, soft: 0.7, rough: 0.3, along: false };
    // Под стеной снаружи — полоса щебня у подножия.
    if let Some(s) = &moat {
        m.paths.extend(s.paths());
    }
    m.paths.push(field_road.clone());
    m.paths.push(paved);
    if let Some(s) = &moat {
        bridge(m, &field_road, s);
    }

    // ----- башни и ворота -----
    let style = match p.style.walls.as_deref().and_then(plan::wall_id) {
        Some(st) => wall(st),
        None => wall(pick(rng, &["wall_stone_blocks", "wall_stone_blocks", "wall_fieldstone", "wall_sandstone"])),
    };
    m.wall_style = style.clone();
    // Караульни и башни — плиты, проезд — брусчатка.
    m.paint_rect(Layer::Rooms, (gate.x, gate.y), (gate.x + gate.w - 1, gate.y + gate.h - 1), Some(flags));
    m.paint_rect(Layer::Rooms, (x0, gate.y), (x0 + PASS - 1, gate.y + gate.h - 1), Some(cobble));
    for t in &towers {
        let r = t.rect(wy);
        if t.round {
            m.paint_oval(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(flags));
        } else {
            m.paint_rect(Layer::Rooms, (r.x, r.y), (r.x + r.w - 1, r.y + r.h - 1), Some(flags));
        }
    }
    let edges = m.room_edges();
    let looks = std::mem::take(&mut m.edge_looks);
    m.sync_edge_looks(&edges);
    m.edge_looks.extend(looks);

    let inner = WallStyle { width: style.width * 0.8, ..style.clone() };
    let mut doors: Vec<[f32; 2]> = Vec::new();
    // Проезд от караулен — стенами с дверью в каждую.
    for x in [x0, x0 + PASS] {
        m.walls.push(Wall { points: vec![[x as f32, gate.y as f32], [x as f32, (gate.y + gate.h) as f32]], closed: false, bends: Vec::new(), low: false, style: Some(inner.clone()) });
        let y = (wy - 1) as f32;
        m.portals.push(portal([x as f32, y], [x as f32, y + 1.0], PortalKind::Door, false));
        doors.push([x as f32, y + 0.5]);
    }
    // Снаружи решётка, изнутри створки ворот.
    let mut grille = portal([x0 as f32, gate.y as f32], [(x0 + PASS) as f32, gate.y as f32], PortalKind::DoubleDoor, false);
    if !missing("door_bars") {
        grille.asset = Some(asset("door_bars").key);
    }
    m.portals.push(grille);
    m.portals.push(portal([x0 as f32, (gate.y + gate.h) as f32], [(x0 + PASS) as f32, (gate.y + gate.h) as f32], PortalKind::DoubleDoor, true));
    // Бойницы караулен — в наружной стене.
    for cx in [gate.x as f32 + GATE_TOWER as f32 / 2.0, (gate.x + gate.w) as f32 - GATE_TOWER as f32 / 2.0] {
        let a = [cx - 0.5, gate.y as f32];
        m.portals.push(portal(a, [cx + 0.5, gate.y as f32], PortalKind::Window, false));
    }
    // С хода по стене — двери в башни и караульни.
    let walk = [(wy as f32) + 0.5, (wy as f32) + 1.5];
    let mut blocks: Vec<(i32, i32)> = towers.iter().map(|t| (t.x, t.x + t.width())).collect();
    blocks.push((gate.x, gate.x + gate.w));
    blocks.sort();
    for &(a, b) in &blocks {
        let open = towers.iter().any(|t| t.open && t.x == a);
        for x in [a, b] {
            if x > 0 && x < w {
                if open {
                    m.portals.push(opening([x as f32, wy as f32], [x as f32, (wy + THICK) as f32]));
                } else {
                    m.portals.push(portal([x as f32, walk[0]], [x as f32, walk[1]], PortalKind::Door, true));
                }
                doors.push([x as f32, wy as f32 + 1.0]);
            }
        }
    }
    // Башням — бойница наружу, в самой выступающей точке.
    for t in towers.iter().filter(|t| !t.open) {
        let r = t.rect(wy);
        let cx = r.x as f32 + r.w as f32 / 2.0;
        m.portals.push(portal([cx - 0.5, r.y as f32], [cx + 0.5, r.y as f32], PortalKind::Window, false));
    }

    // Под стеной снаружи — полоса щебня у подножия, по куртине между башнями.
    {
        let mut from = 0;
        let mut runs: Vec<(i32, i32)> = Vec::new();
        for &(a, b) in &blocks {
            if a > from {
                runs.push((from, a));
            }
            from = b;
        }
        if from < w {
            runs.push((from, w));
        }
        for (a, b) in runs {
            let points = (a..=b).map(|x| [x as f32, wy as f32 - 0.45]).collect();
            m.paths.push(Path { points, material: gravel, width: 1.0, soft: 0.7, rough: 0.5, along: false });
        }
    }

    // ----- ход по стене: зубцы снаружи, низкий край изнутри, лестницы -----
    let crenels = WallStyle { paint: Paint::Texture(key("wall_crenels")), width: 0.5 };
    let edge = WallStyle { width: 0.28, ..style.clone() };
    let mut pieces: Vec<(f32, f32)> = Vec::new();
    let mut from = 0;
    for &(a, b) in &blocks {
        if a > from {
            pieces.push((from as f32, a as f32));
        }
        from = b;
    }
    if from < w {
        pieces.push((from as f32, w as f32));
    }
    let mut sp = Spots::new(m);
    // Занятое: всё, что вклеено из квартала, ворота, башни, двери.
    sp.block(0, y_in, w, h_in);
    sp.block(gate.x, gate.y, gate.w, gate.h);
    for t in &towers {
        let r = t.rect(wy);
        sp.block(r.x, r.y, r.w, r.h);
    }
    sp.block(street.x.min(x0), wy + THICK + 1, (street.x + street.w).max(x0 + PASS) - street.x.min(x0), LANE);
    let mut up = false;
    for &(a, b) in &pieces {
        let y_out = wy as f32;
        let y_edge = (wy + THICK) as f32;
        m.walls.push(Wall { points: vec![[a, y_out], [b, y_out]], closed: false, bends: Vec::new(), low: true, style: Some(crenels.clone()) });
        // Лестница с улицы на ход: пролёт вдоль стены, верх — к проёму в краю.
        let len = b - a;
        let mut gap: Option<(f32, f32)> = None;
        if len >= 7.0 {
            let s = (a + (len - 3.0) / 2.0).floor() + if up { 1.0 } else { -1.0 };
            let s = s.clamp(a + 1.0, b - 4.0);
            let si = s as i32;
            if sp.free(si, wy + THICK, 3, 1) {
                stairs(m, s, y_edge, up);
                sp.block(si - 1, wy + THICK, 5, 2);
                let top = if up { s + 2.0 } else { s };
                gap = Some((top, top + 1.0));
                // Перила со стороны улицы и у верхнего конца: снизу вход открыт.
                let rail_y = y_edge + 1.0;
                let unseen = WallStyle { paint: Paint::Color([0, 0, 0]), width: 0.0 };
                m.walls.push(Wall { points: vec![[s, rail_y], [s + 3.0, rail_y]], closed: false, bends: Vec::new(), low: true, style: Some(unseen.clone()) });
                let x_top = if up { s + 3.0 } else { s };
                m.walls.push(Wall { points: vec![[x_top, y_edge], [x_top, rail_y]], closed: false, bends: Vec::new(), low: true, style: Some(unseen) });
                up = !up;
            }
        }
        match gap {
            Some((g0, g1)) => {
                m.walls.push(Wall { points: vec![[a, y_edge], [g0, y_edge]], closed: false, bends: Vec::new(), low: true, style: Some(edge.clone()) });
                m.walls.push(Wall { points: vec![[g1, y_edge], [b, y_edge]], closed: false, bends: Vec::new(), low: true, style: Some(edge.clone()) });
            }
            None => m.walls.push(Wall { points: vec![[a, y_edge], [b, y_edge]], closed: false, bends: Vec::new(), low: true, style: Some(edge.clone()) }),
        }
        // На ходу у дверей башен — бочка или ящики; зимой у зубцов сугробы.
        if len >= 4.0 {
            let near_end = if rng.chance(0.5) { a + 1.2 } else { b - 1.2 };
            if near_end > 0.5 && near_end < w as f32 - 0.5 && rng.chance(0.7) {
                let what = pick(rng, &["barrel", "crate", "barrel"]);
                drop(m, what, [near_end, wy as f32 + 0.6], rng.int(0, 3) as f32 * 90.0, 0.6);
            }
        }
        if winter && !missing("snow_pile") {
            let mut x = a + 0.8 + rng.random() as f32 * 2.0;
            while x < b - 0.8 {
                drop(m, "snow_pile", [x, wy as f32 + 0.45], rng.int(0, 1) as f32 * 180.0, 0.45);
                x += 2.5 + rng.random() as f32 * 3.0;
            }
        }
    }

    // ----- внутри башен и караулен -----
    for t in &towers {
        let r = t.rect(wy);
        if t.open {
            // Открытая площадка: баллиста смотрит в поле (у FA ствол — вверх холста).
            if !missing("ballista") {
                drop(m, "ballista", [r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0 - 0.2], 0.0, 0.9);
            }
            continue;
        }
        // Двери на ход — по бокам у стены: перед ними пусто.
        sp.unblock(r.x, r.y, r.w, r.h);
        sp.block(r.x, wy, 1, THICK);
        sp.block(r.x + r.w - 1, wy, 1, THICK);
        let inside_r = if t.round { Room { x: r.x + 1, y: r.y + 1, w: r.w - 2, h: r.h - 2 } } else { Room { x: r.x + 1, y: r.y, w: r.w - 2, h: r.h } };
        for n in ["weapon_rack_spears", "barrel", "crate"] {
            by_wall(m, &mut sp, rng, inside_r, n, 0.8, false);
        }
        sp.block(r.x, r.y, r.w, r.h);
    }
    for (gx, door_x) in [(gate.x, x0 - 1), (x0 + PASS, x0 + PASS)] {
        let r = Room { x: gx, y: gate.y, w: GATE_TOWER, h: gate.h };
        sp.unblock(r.x, r.y, r.w, r.h);
        sp.block(door_x, wy - 1, 1, 1);
        sp.block(r.x, wy, 1, THICK);
        sp.block(r.x + r.w - 1, wy, 1, THICK);
        // Ворот решётки — цепи у проезда, у наружной стены.
        let chains_x = if door_x < x0 { x0 as f32 - 0.5 } else { (x0 + PASS) as f32 + 0.5 };
        if !missing("chains") {
            drop(m, "chains", [chains_x, gate.y as f32 + 0.7], 90.0, 0.7);
        }
        sp.block(door_x, gate.y, 1, 1);
        put(m, &mut sp, "table", r.x + 1, gate.y + gate.h - 3, 0.0, 0.8);
        for n in ["weapon_rack", "barrel", "brazier"] {
            by_wall(m, &mut sp, rng, r, n, 0.8, false);
        }
        sp.block(r.x, r.y, r.w, r.h);
    }

    // ----- улица вдоль стены и поле -----
    let lane = Room { x: 0, y: wy + THICK, w, h: LANE };
    let stuff = ["cart", "hay", "crate", "barrel", "sacks", "water_barrel", "firewood_pile"];
    for _ in 0..(w / 9).max(2) {
        let n = pick(rng, &stuff);
        by_side(m, &mut sp, rng, lane, n, 0.8, false, &[0]);
    }
    if !missing("notice_board") {
        let nx = if rng.chance(0.5) { gate.x - 2 } else { gate.x + gate.w + 1 };
        put(m, &mut sp, "notice_board", nx, y_in - 1, 0.0, 0.8);
    }
    for x in [gate.x - 1, gate.x + gate.w] {
        put(m, &mut sp, "street_lamp", x, wy + THICK + 1, 0.0, 0.7);
    }
    // Деревья и кусты в поле — подальше от стены, рва и дороги.
    let clear_to = moat.as_ref().map_or(foot - 3.0, |_| moat_y - 3.5);
    let mut planted: Vec<[f32; 2]> = Vec::new();
    let mut tries = w * 6;
    while tries > 0 && clear_to > 1.0 {
        tries -= 1;
        let q = [rng.random() as f32 * w as f32, rng.random() as f32 * clear_to];
        if field_road.distance(q) < 3.0 || planted.iter().any(|o| (o[0] - q[0]).hypot(o[1] - q[1]) < 3.2) {
            continue;
        }
        planted.push(q);
        if rng.chance(0.45) {
            drop(m, season_tree(rng, season), q, rng.random() as f32 * 360.0, 0.8 + rng.random() as f32 * 0.4);
        } else if let Some(b) = season_bush(rng, season) {
            drop(m, b, q, rng.random() as f32 * 360.0, 0.8);
        }
    }
    // Телега у дороги.
    if clear_to > 3.0 && rng.chance(0.7) {
        let t = road_line.len() * 2 / 3;
        if let Some(q) = road_line.get(t) {
            drop(m, "cart", [q[0] + 2.2, q[1]], 0.0, 1.0);
        }
    }

    // ----- крыши -----
    let roof = match p.style.roof.as_deref().and_then(plan::roof_id) {
        Some(r) => r,
        None => pick(rng, &["roof_slate_gray", "roof_slate_gray", "roof_slate_blue", "roof_clay_red", "roof_shingle"]),
    };
    let rk = key(roof);
    for t in towers.iter().filter(|t| !t.open) {
        let shape = if t.round { RoofShape::Tent } else { RoofShape::Hip };
        let r = t.rect(wy);
        m.roofs.push(Roof { a: [r.x as f32, r.y as f32], b: [(r.x + r.w) as f32, (r.y + r.h) as f32], shape, paint: rk.clone(), turn: 0, overhang: 0.3 });
    }
    m.roofs.push(Roof { a: [gate.x as f32, gate.y as f32], b: [(gate.x + gate.w) as f32, (gate.y + gate.h) as f32], shape: RoofShape::Hip, paint: rk.clone(), turn: 0, overhang: 0.3 });

    // ----- тень стены по высотам -----
    shade_wall(m, wy, &towers, gate, y_in);
    let _ = doors;
    towers.len() + 2 + houses
}

/// Пролёт вдоль стены у края хода: `s` — левый конец, `y` — край хода; `up` —
/// поднимается вправо (иначе влево). Тень — у нижнего конца.
fn stairs(m: &mut Map, s: f32, y: f32, up: bool) {
    // Каменный пролёт FA (картинка — ступени вдоль своей высоты, верх холста —
    // нижний конец, как у спуска в крипту); без него — встроенная лестница.
    let flight = !missing("stairs_down");
    let p = if flight { asset("stairs_down") } else { Palette::builtin_pick("stairs") };
    let along_x = p.size[0] >= p.size[1];
    let scale = 3.0 / p.size[0].max(p.size[1]).max(0.5);
    let rot = (if along_x { 0.0 } else { 90.0 }) + if up { 0.0 } else { 180.0 };
    let c = [s + 1.5, y + 0.5];
    drop_pick(m, &p, c, rot, scale);
    if flight {
        // Тень внизу пролёта: лестница ведёт вверх.
        drop_pick(m, &Palette::builtin_pick("stairs_shade"), c, rot, scale);
        if let Some(shade) = m.props.last_mut() {
            shade.layer = Some(crate::map::LAYER_DEFAULT);
        }
        if !missing("stair_rail") {
            drop_pick(m, &asset("stair_rail"), [c[0], c[1] + 0.5 * scale], rot, scale);
        }
    }
}

/// Тень стены, башен и ворот по высотам: как у колизея — луч к солнцу и мягкая
/// тень у подножия со всех сторон.
fn shade_wall(m: &mut Map, wy: i32, towers: &[Tower], gate: Room, y_in: i32) {
    let res = mask::RES as f32;
    let (mw, mh) = (m.rooms.w, m.rooms.h);
    let mut height = vec![0.0f32; mw * mh];
    for j in 0..mh {
        let y = (j as f32 + 0.5) / res;
        if y >= y_in as f32 {
            continue;
        }
        for i in 0..mw {
            let x = (i as f32 + 0.5) / res;
            let k = j * mw + i;
            let band = y >= wy as f32 && y < (wy + THICK) as f32;
            let built = m.rooms.data[k] >= 128 && (y >= gate.y as f32 || towers.iter().any(|t| x >= t.x as f32 && x < (t.x + t.width()) as f32));
            if band || built {
                height[k] = WALL_H;
            }
        }
    }
    let light = m.shadows.light();
    let sample = |x: f32, y: f32| -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f32, j: f32| if i < 0.0 || j < 0.0 || i as usize >= mw || j as usize >= mh { 0.0 } else { height[j as usize * mw + i as usize] };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let low = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + low * fy
    };
    let y_hi = ((y_in as f32 + WALL_H / SUN + 1.0) * res) as usize;
    let mut out: Vec<(usize, f32)> = Vec::new();
    for j in 0..mh.min(y_hi) {
        for i in 0..mw {
            let k = j * mw + i;
            let here = height[k];
            if here >= WALL_H {
                continue;
            }
            let reach = (WALL_H - here) / SUN;
            let mut sun = 0.0f32;
            let mut t = 0.0625;
            while t <= reach + 0.3 {
                let hq = sample(i as f32 + light[0] * t * res, j as f32 + light[1] * t * res);
                sun = sun.max(((hq - here - t * SUN) / 0.3).clamp(0.0, 1.0));
                if sun >= 1.0 {
                    break;
                }
                t += 0.0625;
            }
            let mut near = 0.0f32;
            for d in 0..12 {
                let a = d as f32 / 12.0 * TAU;
                for step in 1..=4 {
                    let r = CONTACT_REACH * step as f32 / 4.0;
                    let hq = sample(i as f32 + a.cos() * r * res, j as f32 + a.sin() * r * res);
                    let rise = ((hq - here) / 0.5).clamp(0.0, 1.0);
                    near = near.max(rise * (1.0 - r / (CONTACT_REACH + 0.05)).powf(1.5));
                }
            }
            let v = 1.0 - (1.0 - sun * SHADE) * (1.0 - near * CONTACT);
            if v > 0.005 {
                out.push((k, v));
            }
        }
    }
    m.shade_paint(&out, 1.0);
}

/// Вклеить карту `s` (той же ширины) в `m` со сдвигом вниз на `dy` клеток:
/// маски строками, остальное — со сдвигом координат; материалы и виды стен —
/// по своим номерам в `m`.
fn paste(m: &mut Map, s: &Map, dy: i32) {
    let off = dy as usize * mask::RES * m.rooms.w;
    let n = s.rooms.data.len().min(m.rooms.data.len() - off);
    let remap: Vec<u8> = s.materials.iter().map(|mt| m.material_index(mt.clone()).unwrap_or(0) as u8).collect();
    let blank = Mask { data: vec![0; m.rooms.data.len()], ..m.rooms.clone() };
    let copy = |dst: &mut Vec<crate::map::Coat>, src: &[crate::map::Coat]| {
        for c in dst.iter_mut() {
            c.mask.data[off..off + n].fill(0);
        }
        for c in src {
            let mat = remap[c.material as usize];
            let k = match dst.iter().position(|d| d.material == mat) {
                Some(k) => k,
                None => {
                    dst.push(crate::map::Coat { material: mat, mask: blank.clone() });
                    dst.len() - 1
                }
            };
            dst[k].mask.data[off..off + n].copy_from_slice(&c.mask.data[..n]);
        }
    };
    copy(&mut m.terrain, &s.terrain);
    copy(&mut m.room_floor, &s.room_floor);
    m.rooms.data[off..off + n].copy_from_slice(&s.rooms.data[..n]);
    if let Some(ss) = &s.shade {
        let sh = m.shade.get_or_insert_with(|| blank.clone());
        sh.data[off..off + n].copy_from_slice(&ss.data[..n]);
    }
    let d = dy as f32;
    let down = |p: [f32; 2]| [p[0], p[1] + d];
    for wl in &s.walls {
        let mut wl = wl.clone();
        wl.points.iter_mut().for_each(|p| *p = down(*p));
        m.walls.push(wl);
    }
    for po in &s.portals {
        m.portals.push(Portal { a: down(po.a), b: down(po.b), ..po.clone() });
    }
    for pr in &s.props {
        m.props.push(Prop { pos: down(pr.pos), ..pr.clone() });
    }
    for pa in &s.paths {
        let mut pa = pa.clone();
        pa.points.iter_mut().for_each(|p| *p = down(*p));
        pa.material = remap[pa.material] as usize;
        m.paths.push(pa);
    }
    for r in &s.roofs {
        m.roofs.push(Roof { a: down(r.a), b: down(r.b), ..r.clone() });
    }
    for l in &s.labels {
        let mut l = l.clone();
        l.pos = down(l.pos);
        m.labels.push(l);
    }
    for (&(kx, ky), &v) in &s.edge_looks {
        let look = &s.wall_looks[v as usize];
        let idx = match m.wall_looks.iter().position(|l| l == look) {
            Some(i) => i,
            None => {
                m.wall_looks.push(look.clone());
                m.wall_looks.len() - 1
            }
        };
        m.edge_looks.insert((kx, ky + dy * 64), idx as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ворота на главной улице квартала, ход по стене отделён низкими стенами,
    /// а в VTT они ограничивают зрение, а не глушат его.
    #[test]
    fn gate_meets_the_street_and_walk_is_low() {
        for (w, h, seed) in [(56, 40, "w1"), (44, 32, "w2"), (30, 20, "w3")] {
            let mut p = Plan::new(Kind::Walls);
            p.seed = seed.into();
            let mut m = Map::new(w, h);
            let r = build(&mut m, &p, 0, crate::palette::Palette::builtin()).unwrap();
            assert!(r.count > 0, "{seed}: {:?}", r.warnings);
            assert!(m.walls.iter().any(|w| w.low), "{seed}: нет низких стен");
            // Решётка и створки ворот — двойные двери на одной вертикали.
            let gates: Vec<&Portal> = m.portals.iter().filter(|p| p.kind == PortalKind::DoubleDoor && p.center()[1] <= (14 + OUT) as f32 + 0.5).collect();
            assert_eq!(gates.len(), 2, "{seed}");
            assert_eq!(gates[0].center()[0], gates[1].center()[0], "{seed}");
            let lay = crate::export::layout(&m, &m.room_edges());
            assert!(!lay.low.is_empty(), "{seed}: низкие стены не дошли до выгрузки");
        }
    }
}
