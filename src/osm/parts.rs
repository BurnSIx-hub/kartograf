//! Полы, ограды, обрывы с осыпью, ворота, занятые места, дома.

use super::*;

/// Пол по слоту — один вариант на карту: с паками FA каждая улица иначе брала бы
/// свою брусчатку.
#[derive(Default)]
pub(super) struct Floors(pub(super) BTreeMap<&'static str, usize>);

impl Floors {
    pub(super) fn get(&mut self, m: &mut Map, slot: &'static str) -> usize {
        *self.0.entry(slot).or_insert_with(|| {
            let k = mat(m, slot);
            // Трава, земля и вода лежат полями на сотни клеток: без перемешивания
            // плитки (как у встроенных) по лугу идёт узор повтора.
            if slot.starts_with("rock_") || matches!(slot, "grass_green" | "grass_dry" | "meadow" | "rough_grass" | "dirt" | "gravel" | "shingle" | "path_gravel" | "sand" | "mud" | "forest_floor" | "swamp" | "rocky_ground" | "water_shallow" | "water_deep") {
                m.materials[k].vary = true;
            }
            k
        })
    }

    /// Пол слота в цвете `rgb` (см. [`library::tint_key`]): тот же вариант
    /// картинки, что у [`Floors::get`], перекрашенный.
    pub(super) fn tinted(&mut self, m: &mut Map, slot: &'static str, rgb: [u8; 3]) -> usize {
        let k = self.get(m, slot);
        let base = m.materials[k].clone();
        let paint = match &base.paint {
            Paint::Texture(key) => Paint::Texture(library::tint_key(rgb, key)),
            Paint::Color(c) => Paint::Color([0, 1, 2].map(|i| ((c[i] as u32 + rgb[i] as u32) / 2) as u8)),
        };
        m.material_index(Material { paint, ..base }).unwrap_or(k)
    }
}

/// Стена слота в цвете `rgb`, если он есть.
pub(super) fn wall_tinted(slot: &str, rgb: Option<[u8; 3]>) -> WallStyle {
    let mut style = wall(slot);
    if let (Some(c), Paint::Texture(key)) = (rgb, &style.paint) {
        style.paint = Paint::Texture(library::tint_key(c, key));
    }
    style
}

/// Уличный предмет по точке OSM: фонарь, скамья, фонтан, памятник.
pub(super) fn furniture(t: &Tags) -> Option<&'static str> {
    if get(t, "highway") == "street_lamp" {
        return Some("street_lamp");
    }
    match get(t, "amenity") {
        "bench" => return Some("bench"),
        "fountain" => return Some("fountain"),
        _ => {}
    }
    let statue = matches!(get(t, "artwork_type"), "statue" | "sculpture") || matches!(get(t, "memorial"), "statue" | "war_memorial" | "obelisk");
    (statue && (get(t, "tourism") == "artwork" || !get(t, "historic").is_empty())).then_some("statue")
}

/// Ограда по линии OSM: слот стены, низкая ли (видно поверх), толщина в клетках.
/// За городом стена — сухая кладка из бута по пояс, в городе — высокая из блоков.
pub(super) fn barrier(t: &Tags, rural: bool) -> Option<(&'static str, bool, f32)> {
    match get(t, "barrier") {
        "wall" | "retaining_wall" if rural => Some(("wall_fieldstone", true, 0.35)),
        "wall" | "city_wall" | "retaining_wall" => Some(("wall_stone_blocks", false, 0.3)),
        "fence" => Some(("wall_wood", true, 0.15)),
        "railing" | "guard_rail" | "handrail" => Some(("wall_metal", true, 0.12)),
        "hedge" => Some(("wall_hedge", false, 0.45)),
        // Обрыв — не стена, а рельеф (`cliff_relief`): полоса скалы, тень и стена в Foundry.
        _ => None,
    }
}

/// Ярусов у обрыва без тега `height` и предел: обрыв Малхема в 80 м — полсотни
/// ярусов, а сверху стенка всё равно отвесная, и полоса в десятки клеток съела бы карту.
pub(super) const CLIFF_LEVELS: f32 = 3.0;
pub(super) const CLIFF_MAX: f32 = 6.0;
/// Плато за кромкой обрыва, клетки; дальше земля спадает на `CLIFF_FALL` яруса
/// на клетку — пологим склоном, а не вторым обрывом. Круче (0,35) склон рисовался
/// кольцами гребней, на Лулворте — поперёк пляжа.
pub(super) const CLIFF_SHELF: f32 = 3.0;
pub(super) const CLIFF_FALL: f32 = 0.15;
/// За концом линии обрыв сходит на нет на этом пути, клетки.
pub(super) const CLIFF_END: f32 = 2.0;
/// Низина за концом линии поднимается так полого (ярусов на клетку) — без стенки и гребня.
pub(super) const CLIFF_END_FALL: f32 = 0.25;

/// Высота обрыва в ярусах (ярус — клетка, 1,524 м) по тегу `height`.
pub(super) fn cliff_levels(t: &Tags) -> f32 {
    get(t, "height").trim().trim_end_matches('m').trim().parse::<f32>().ok().filter(|v| *v > 0.0).map_or(CLIFF_LEVELS, |m| (m / CELL_M).round().clamp(1.0, CLIFF_MAX))
}

/// Рельеф по обрывам OSM. Верх обрыва в OSM — слева по ходу линии при севере
/// сверху; на карте, где y вниз, это нормаль `(dy, −dx)` к ходу `(dx, dy)`
/// (линия на восток — верх на севере). Опускается нижняя сторона, а не
/// поднимается верхняя: рельеф темнит ярусы ниже самого верхнего, и узкое
/// поднятое плато затемняло всю карту вокруг. Низ — полоса `CLIFF_SHELF`, дальше
/// плавный подъём обратно; за концами линии — на нет за `CLIFF_END`. Кромки,
/// полосы, тени и стены Foundry дальше строит рельеф сам.
pub(super) fn cliff_relief(lines: &[(Vec<[f32; 2]>, f32)], w: u32, h: u32) -> Option<crate::relief::Relief> {
    if lines.is_empty() {
        return None;
    }
    let mut r = crate::relief::Relief::for_cells(w, h);
    // Настоящая местность: низина не темнеет по высоте, пологий подъём — без гребней.
    r.natural = true;
    let res = mask::RES as f32;
    for (line, levels) in lines {
        // За концом линии стенка сходит на нет за `CLIFF_END`, а низина — полого,
        // `CLIFF_END_FALL` яруса на клетку: при одном коротком спаде торец низины
        // вставал стенкой поперёк пляжа Лулворта и дальше в море.
        let end_len = (levels / CLIFF_END_FALL).max(CLIFF_END);
        let reach = CLIFF_SHELF + levels / CLIFF_FALL + end_len;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in line {
            for c in 0..2 {
                lo[c] = lo[c].min(p[c] - reach);
                hi[c] = hi[c].max(p[c] + reach);
            }
        }
        let i0 = ((lo[0] * res).floor().max(0.0) as usize).min(r.w);
        let i1 = ((hi[0] * res).ceil().max(0.0) as usize).min(r.w);
        let j0 = ((lo[1] * res).floor().max(0.0) as usize).min(r.h);
        let j1 = ((hi[1] * res).ceil().max(0.0) as usize).min(r.h);
        let n = line.len() - 1;
        for j in j0..j1 {
            for i in i0..i1 {
                let p = [(i as f32 + 0.5) / res, (j as f32 + 0.5) / res];
                // Ближайший отрезок; у общей вершины — тот, от чьей прямой точка дальше.
                let (mut best, mut side, mut beyond) = (f32::INFINITY, 0.0f32, 0.0f32);
                for (k, s) in line.windows(2).enumerate() {
                    let (a, b) = (s[0], s[1]);
                    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                    let len = dx.hypot(dy).max(1e-6);
                    let t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (len * len);
                    let q = [a[0] + dx * t.clamp(0.0, 1.0), a[1] + dy * t.clamp(0.0, 1.0)];
                    let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                    let s = ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx) / len;
                    if d < best - 1e-4 || (d < best + 1e-4 && s.abs() > side.abs()) {
                        best = best.min(d);
                        side = s;
                        beyond = if k == 0 && t < 0.0 { -t * len } else if k + 1 == n && t > 1.0 { (t - 1.0) * len } else { 0.0 };
                    }
                }
                let fall = if best <= CLIFF_SHELF { 1.0 } else { (1.0 - (best - CLIFF_SHELF) * CLIFF_FALL / levels).max(0.0) };
                let soft = (1.0 - beyond / end_len).max(0.0);
                let hard = (1.0 - beyond / CLIFF_END).max(0.0);
                let v = if side > 0.0 {
                    // Верх за концом линии опускается вслед за низиной: стенки там нет.
                    if beyond <= 0.0 {
                        continue;
                    }
                    -levels * fall * soft * (1.0 - hard)
                } else {
                    -levels * fall * soft
                };
                let k = j * r.w + i;
                r.data[k] = r.data[k].min(v);
            }
        }
    }
    Some(r)
}

/// Осыпь под обрывом: шаг между кучками вдоль подножия, клетки, и насколько
/// дальше от него ложатся одиночки на ярус перепада.
pub(super) const SCREE_STEP: f32 = 2.6;
pub(super) const SCREE_REACH: f32 = 0.35;
/// Камней осыпи на карту не больше примерно этого: каждый в Foundry — свой тайл.
/// Длинные обрывы гор — реже, а не без конца.
pub(super) const SCREE_MAX: f32 = 500.0;

/// Осыпь под обрывами рельефа: камни породы участка у подножия кучками — глыба у
/// самой стенки, мелочь вокруг неё, — и редкие одиночки дальше; у высокого обрыва
/// гуще и шире, к свободному концу реже. Без них стенка читалась каменной оградой:
/// полоса, тень — и сразу ровная трава. Ровная россыпь мелочи читалась гравием, а
/// каждый камень в Foundry — свой тайл. Мимо домов и воды (`hard`), дорог, оград и
/// чужих стенок.
pub(super) fn cliff_scree(m: &mut Map, hard: &Blocked, tone: &str) {
    let Some(r) = m.relief.clone() else { return };
    if missing("boulder") {
        return;
    }
    let stone = asset_toned("boulder", tone);
    let lines: Vec<(Vec<[f32; 2]>, f32)> = m.paths.iter().map(|p| (p.curve(), p.width / 2.0 + 0.3)).chain(m.walls.iter().map(|wl| (wl.points.clone(), 0.5))).collect();
    let (w, h) = (m.width as f32, m.height as f32);
    let mut placed: Vec<([f32; 2], f32)> = Vec::new();
    let edges: Vec<crate::relief::Edge> = crate::relief::edges(&r).into_iter().filter(|e| e.kind == crate::relief::EdgeKind::Cliff).collect();
    let runs: Vec<Vec<f32>> = edges
        .iter()
        .map(|e| {
            let mut along = vec![0.0f32; e.points.len()];
            for k in 1..e.points.len() {
                along[k] = along[k - 1] + length(e.points[k - 1], e.points[k]);
            }
            along
        })
        .collect();
    // В среднем кучка с одиночкой — около трёх камней на шаг.
    let all: f32 = runs.iter().filter_map(|a| a.last()).sum();
    let budget = (SCREE_MAX / (all / SCREE_STEP * 3.0).max(1.0)).min(1.0);
    for (n, (e, along)) in edges.iter().zip(&runs).enumerate() {
        let mut rng = Rng::new(&format!("osm/scree/{n}"));
        let pts = &e.points;
        let total = along.last().copied().unwrap_or(0.0);
        let mut next = rng.random() as f32 * SCREE_STEP;
        for k in 0..pts.len() {
            if along[k] < next {
                continue;
            }
            next = along[k] + SCREE_STEP * (0.6 + rng.random() as f32 * 0.8);
            // У свободного конца стенка сходит на нет — и осыпь с ней.
            let to_end = [along[k], total - along[k]];
            let taper = (0..2).map(|s| if e.free[s] { (to_end[s] / 3.0).min(1.0) } else { 1.0 }).fold(1.0f32, f32::min);
            let rich = (e.drop[k] / 3.0).clamp(0.5, 1.6) * taper * budget;
            let (p, up) = (pts[k], e.up(k));
            let at = |d: f32, s: f32| [p[0] - up[0] * d - up[1] * s, p[1] - up[1] * d + up[0] * s];
            let mut stones: Vec<([f32; 2], f32)> = Vec::new();
            // Кучка: глыба у самой стенки, мелочь вокруг неё — больше вниз по склону.
            if rng.chance((0.75 * rich).min(0.95) as f64) {
                let d = e.width[k] + 0.25 + rng.random() as f32 * 0.35;
                let big = 0.7 + rng.random() as f32 * 0.5;
                stones.push((at(d, (rng.random() as f32 - 0.5) * 0.6), big));
                for _ in 0..rng.int(2, 4) {
                    let q = at(d + (rng.random() as f32 - 0.15) * big, (rng.random() as f32 - 0.5) * 1.6 * big);
                    stones.push((q, 0.3 + rng.random() as f32 * 0.2));
                }
            }
            // Одиночка подальше; у высокого обрыва дальше.
            if rng.chance((0.5 * rich).min(0.9) as f64) {
                let d = e.width[k] + 0.4 + rng.random() as f32 * (0.6 + SCREE_REACH * e.drop[k]);
                stones.push((at(d, (rng.random() as f32 - 0.5) * 1.5), 0.35 + rng.random() as f32 * 0.3));
            }
            for (q, size) in stones {
                let off = q[0] < 0.3 || q[1] < 0.3 || q[0] > w - 0.3 || q[1] > h - 0.3;
                if off || hard.at(q) || r.steepest(q, 0.25) >= crate::relief::CLIFF {
                    continue;
                }
                if lines.iter().any(|(l, half)| l.windows(2).map(|s| segment_distance(q, s[0], s[1])).fold(f32::INFINITY, f32::min) < *half) {
                    continue;
                }
                if placed.iter().any(|&(o, s)| length(o, q) < (s + size) * 0.34) {
                    continue;
                }
                drop_pick(m, &stone, q, rng.int(0, 359) as f32, size);
                placed.push((q, size));
            }
        }
    }
}

/// Проход в ограде и его ширина в клетках: ворота, калитка, перелаз.
pub(super) fn gate(t: &Tags) -> Option<f32> {
    match get(t, "barrier") {
        "gate" | "lift_gate" | "swing_gate" | "sliding_gate" | "hampshire_gate" | "cattle_grid" => Some(2.0),
        "stile" | "kissing_gate" | "entrance" | "turnstile" | "bump_gate" => Some(1.0),
        _ => None,
    }
}

/// Ограда кусками: у калиток и ворот — разрыв в их ширину.
pub(super) fn split_at_gates(pts: &[[f32; 2]], gaps: &[Option<f32>]) -> Vec<Vec<[f32; 2]>> {
    let toward = |p: [f32; 2], q: [f32; 2], d: f32| {
        let l = length(p, q);
        (l > d).then(|| [p[0] + (q[0] - p[0]) * d / l, p[1] + (q[1] - p[1]) * d / l])
    };
    let mut out = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    for (k, &p) in pts.iter().enumerate() {
        let Some(gap) = gaps[k] else {
            cur.push(p);
            continue;
        };
        if k > 0
            && let Some(q) = toward(p, pts[k - 1], gap / 2.0)
        {
            cur.push(q);
        }
        if cur.len() >= 2 {
            out.push(std::mem::take(&mut cur));
        }
        cur.clear();
        if k + 1 < pts.len()
            && let Some(q) = toward(p, pts[k + 1], gap / 2.0)
        {
            cur.push(q);
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Где сажать нельзя: дома, дороги, вода, ограды. Сетка маски (`RES` точек на клетку).
#[derive(Clone)]
pub(super) struct Blocked {
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) cells: Vec<bool>,
}

impl Blocked {
    pub(super) fn new(m: &Map) -> Self {
        let cells = m.rooms.data.iter().map(|&v| v >= 128).collect();
        Self { w: m.rooms.w, h: m.rooms.h, cells }
    }

    pub(super) fn disk(&mut self, c: [f32; 2], r: f32) {
        let res = mask::RES as f32;
        let (i0, i1) = (((c[0] - r) * res).floor().max(0.0) as usize, (((c[0] + r) * res).ceil().max(0.0) as usize).min(self.w));
        let (j0, j1) = (((c[1] - r) * res).floor().max(0.0) as usize, (((c[1] + r) * res).ceil().max(0.0) as usize).min(self.h));
        for j in j0..j1 {
            for i in i0..i1 {
                let p = mask::sample_pos(i as isize, j as isize);
                if (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r {
                    self.cells[j * self.w + i] = true;
                }
            }
        }
    }

    /// Полоса вдоль ломаной.
    pub(super) fn line(&mut self, pts: &[[f32; 2]], r: f32) {
        for s in pts.windows(2) {
            let n = (length(s[0], s[1]) / (r * 0.5).max(0.25)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let f = k as f32 / n as f32;
                self.disk([s[0][0] + (s[1][0] - s[0][0]) * f, s[0][1] + (s[1][1] - s[0][1]) * f], r);
            }
        }
    }

    pub(super) fn at(&self, p: [f32; 2]) -> bool {
        let res = mask::RES as f32;
        let (i, j) = ((p[0] * res).floor(), (p[1] * res).floor());
        i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h || self.cells[j as usize * self.w + i as usize]
    }
}

/// Сколько всего деревьев и кустов сажать на карту: больше — Foundry тяжело
/// (у каждой кроны свой тайл).
pub(super) const MAX_PLANTS: usize = 3000;

/// Посаженное, разложенное по корзинам в клетку: не ближе шага к соседу.
#[derive(Default)]
pub(super) struct Spacing(pub(super) HashMap<(i32, i32), Vec<[f32; 2]>>);

impl Spacing {
    pub(super) fn free(&self, q: [f32; 2], d: f32) -> bool {
        let (bx, by, r) = (q[0].floor() as i32, q[1].floor() as i32, d.ceil() as i32);
        (-r..=r).all(|dx| (-r..=r).all(|dy| self.0.get(&(bx + dx, by + dy)).is_none_or(|v| v.iter().all(|p| length(*p, q) >= d))))
    }

    pub(super) fn add(&mut self, q: [f32; 2]) {
        self.0.entry((q[0].floor() as i32, q[1].floor() as i32)).or_default().push(q);
    }
}

/// Дом на карте: что про него нужно после заливки всех домов.
pub(super) struct House {
    pub(super) rings: Vec<Vec<[f32; 2]>>,
    /// Входы из OSM, в клетках.
    pub(super) entrances: Vec<[f32; 2]>,
    pub(super) style: WallStyle,
    pub(super) wood: bool,
    /// Большой дом с широкими дверьми: вокзал, храм, магазин.
    pub(super) grand: bool,
    /// Есть ли двери: у крепостной стены — нет.
    pub(super) doors: bool,
}

/// Линия, которая ляжет дорогой: этаж, порядок и сами дороги снизу вверх.
pub(super) type Lane = (f32, u8, Vec<Path>);
