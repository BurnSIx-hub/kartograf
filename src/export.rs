//! Выгрузка карты в VTT: сцена Foundry (JSON для «Import Data») и Universal VTT
//! (`.dd2vtt`). Картинку карты снимает окно по плиткам (см. `App::export_*`),
//! здесь — стены, проёмы и свет в координатах картинки.

use crate::map::{self, Map, PortalKind, Segment};
use crate::mask;
use serde_json::{Value, json};

/// Уровень сцены, который Foundry 14 создаёт по умолчанию: с ним «Import Data»
/// заменяет фон, а не заводит второй уровень.
const LEVEL_ID: &str = "defaultLevel0000";

/// Что выгружается кроме картинки.
pub struct Layout {
    /// Стены пологов палаток: отрезок, внутренность палатки слева от него.
    pub tents: Vec<([f32; 2], [f32; 2])>,
    /// Сплошные стены — ломаные в клетках.
    pub walls: Vec<Vec<[f32; 2]>>,
    /// Низкие стены (`Wall::low`): шаг держат, зрение и свет — ограниченно.
    pub low: Vec<Vec<[f32; 2]>>,
    /// Кромки обрывов рельефа: не пройти, а видно, светит и слышно поверх.
    pub cliffs: Vec<Vec<[f32; 2]>>,
    /// Двери и окна — отрезок в клетках; пустые проёмы сюда не попадают.
    pub portals: Vec<PortalOut>,
    /// Свет: середина в клетках, радиус яркого и тусклого в клетках, цвет.
    pub lights: Vec<Light>,
    /// Живые области: эффект и контуры в клетках; `true` — дыра (остров).
    pub zones: Vec<Zone>,
    /// Лестницы и люки: рамки [x0, y0, x1, y1] в клетках. Совпали на соседних
    /// этажах — в Foundry между уровнями переход.
    pub passages: Vec<[f32; 4]>,
}

pub struct Zone {
    pub effect: &'static str,
    pub loops: Vec<(Vec<[f32; 2]>, bool)>,
    /// У области «внутри» (`effect` = "place") — ключ её звука.
    pub place: String,
}

/// Модуль Foundry, который оживляет области.
pub const LIVE_MODULE: &str = "burn-living-map";
/// Свой модуль Foundry: его пометки на тайлах (паутина, сундук).
pub const OWN_MODULE: &str = "burn-kartograf";

/// Под открытым небом: лес, город, местность по координатам — не пещера и не
/// подземелье. Пустой звук места — карта, нарисованная руками: о небе не знаем.
fn open_sky(map: &Map) -> bool {
    map.ambience.starts_with("forest-") || map.ambience.starts_with("town-")
}

/// Флаги сцены: формат Картографа и звук места для «Wild Canvas».
fn scene_flags(map: &Map) -> Value {
    let mut flags = json!({"kartograf": {"format": 1}});
    if !map.ambience.is_empty() {
        flags[LIVE_MODULE] = json!({"ambience": map.ambience});
        // Под открытым небом — тени облаков «Wild Canvas».
        if open_sky(map) {
            flags[LIVE_MODULE]["clouds"] = json!(0.6);
            flags[LIVE_MODULE]["cloudCover"] = json!(0.5);
        }
    }
    // Зерно карты — рыбкам «Wild Canvas»: на каждой карте свои, у всех игроков те же.
    if !map.seed.is_empty() {
        if flags.get(LIVE_MODULE).is_none() {
            flags[LIVE_MODULE] = json!({});
        }
        flags[LIVE_MODULE]["seed"] = json!(map.seed);
    }
    // Поверхности под шагами.
    if let Some(s) = surfaces(map) {
        if flags.get(LIVE_MODULE).is_none() {
            flags[LIVE_MODULE] = json!({});
        }
        flags[LIVE_MODULE]["surfaces"] = s;
    }
    flags
}

/// Поверхности для шагов «Wild Canvas»: по букве на клетку, строки подряд,
/// `.` — не знаем, молчать. Берётся верхний слой в середине клетки: дорога поверх
/// пола комнаты, пол комнаты поверх местности.
pub fn surfaces(map: &Map) -> Option<Value> {
    let (w, h) = (map.width as usize, map.height as usize);
    let res = mask::RES as isize;
    let mut cells = vec!['.'; w * h];
    let mut known = false;
    let at = |m: &mask::Mask, x: usize, y: usize| m.get(x as isize * res + res / 2, y as isize * res + res / 2);
    for y in 0..h {
        for x in 0..w {
            let mut top: Option<usize> = None;
            for c in &map.terrain {
                if at(&c.mask, x, y) > 127 {
                    top = Some(c.material as usize);
                }
            }
            if at(&map.rooms, x, y) > 127 {
                for c in &map.room_floor {
                    if at(&c.mask, x, y) > 127 {
                        top = Some(c.material as usize);
                    }
                }
            }
            if let Some(s) = top.and_then(|m| map.materials.get(m)).and_then(|m| m.surface()) {
                cells[y * w + x] = s;
                known = true;
            }
        }
    }
    // Дороги и реки — поверх: середина клетки в пределах полуширины линии.
    for p in &map.paths {
        let Some(s) = map.materials.get(p.material).and_then(|m| m.surface()) else { continue };
        let line = p.curve();
        let hw = p.width / 2.0;
        for seg in line.windows(2) {
            let (lo, hi) = (seg[0][0].min(seg[1][0]) - hw, seg[0][0].max(seg[1][0]) + hw);
            let (top, bot) = (seg[0][1].min(seg[1][1]) - hw, seg[0][1].max(seg[1][1]) + hw);
            for y in (top.floor().max(0.0) as usize)..(bot.ceil().max(0.0) as usize).min(h) {
                for x in (lo.floor().max(0.0) as usize)..(hi.ceil().max(0.0) as usize).min(w) {
                    if map::segment_distance([x as f32 + 0.5, y as f32 + 0.5], seg[0], seg[1]) <= hw {
                        cells[y * w + x] = s;
                        known = true;
                    }
                }
            }
        }
    }
    known.then(|| json!({"w": w, "h": h, "cells": cells.into_iter().collect::<String>()}))
}

/// Контуры живых материалов: по эффекту — все залитые им места, острова — дырами.
/// Покрытия одного эффекта сливаются в одну маску до обводки: Foundry вычитает
/// дыры области из всех её фигур, и дыра мха под светлым пятном (тоже болото)
/// вырезала бы само пятно.
fn zones(map: &Map, room_edges: &[Segment]) -> Vec<Zone> {
    // Стены твёрдые: под ними живого нет. Без этого пена и рябь воды ложились
    // поверх борта корабля и стен у воды — стены впечатаны в картинку под эффектом.
    // Стена комнаты стоит снаружи края комнаты на всю толщину, свободная — серединой.
    let room_w = map.wall_looks.iter().map(|l| l.width).fold(map.wall_style.width, f32::max);
    let mut solid: Vec<(Segment, f32)> = room_edges.iter().map(|s| (*s, room_w + 0.1)).collect();
    for w in map.walls.iter().filter(|w| !w.is_decor()) {
        let half = w.style.as_ref().map_or(map.wall_style.width, |s| s.width) / 2.0 + 0.1;
        solid.extend(w.segments().map(|s| (s, half)));
    }
    let mut masks: Vec<(&'static str, mask::Mask)> = Vec::new();
    for (m, mat) in map.materials.iter().enumerate() {
        let Some(effect) = mat.live().effect() else { continue };
        let mask = map.material_mask(m);
        match masks.iter_mut().find(|(e, _)| *e == effect) {
            Some((_, all)) => {
                for (a, v) in all.data.iter_mut().zip(&mask.data) {
                    *a = a.saturating_add(*v);
                }
            }
            None => masks.push((effect, mask)),
        }
    }
    let mut out: Vec<Zone> = Vec::new();
    for (effect, mut mask) in masks {
        // Под настилом (дорожка вдоль линии — мостки, мост) живого нет: по тропе
        // через болото Виру раба шаг обычный, вода и рыбы под досками не идут.
        let res = mask::RES as f32;
        for p in map.paths.iter().filter(|p| p.along) {
            let hw = p.width / 2.0;
            for seg in p.points.windows(2) {
                let (lo, hi) = ([seg[0][0].min(seg[1][0]) - hw, seg[0][1].min(seg[1][1]) - hw], [seg[0][0].max(seg[1][0]) + hw, seg[0][1].max(seg[1][1]) + hw]);
                for j in ((lo[1] * res).floor().max(0.0) as usize)..((hi[1] * res).ceil().max(0.0) as usize).min(mask.h) {
                    for i in ((lo[0] * res).floor().max(0.0) as usize)..((hi[0] * res).ceil().max(0.0) as usize).min(mask.w) {
                        if map::segment_distance([(i as f32 + 0.5) / res, (j as f32 + 0.5) / res], seg[0], seg[1]) <= hw {
                            mask.data[j * mask.w + i] = 0;
                        }
                    }
                }
            }
        }
        for ((a, b), hw) in &solid {
            let (lo, hi) = ([a[0].min(b[0]) - hw, a[1].min(b[1]) - hw], [a[0].max(b[0]) + hw, a[1].max(b[1]) + hw]);
            for j in ((lo[1] * res).floor().max(0.0) as usize)..((hi[1] * res).ceil().max(0.0) as usize).min(mask.h) {
                for i in ((lo[0] * res).floor().max(0.0) as usize)..((hi[0] * res).ceil().max(0.0) as usize).min(mask.w) {
                    if map::segment_distance([(i as f32 + 0.5) / res, (j as f32 + 0.5) / res], *a, *b) <= *hw {
                        mask.data[j * mask.w + i] = 0;
                    }
                }
            }
        }
        // Камни-переходы — тоже сухой путь через воду: кружок под каждым, при
        // шаге 0,9 клетки они сливаются в полосу брода.
        for p in map.props.iter().filter(|p| p.asset.to_lowercase().contains("stepping")) {
            let r = 0.6;
            for j in (((p.pos[1] - r) * res).floor().max(0.0) as usize)..(((p.pos[1] + r) * res).ceil().max(0.0) as usize).min(mask.h) {
                for i in (((p.pos[0] - r) * res).floor().max(0.0) as usize)..(((p.pos[0] + r) * res).ceil().max(0.0) as usize).min(mask.w) {
                    let (dx, dy) = ((i as f32 + 0.5) / res - p.pos[0], (j as f32 + 0.5) / res - p.pos[1]);
                    if dx * dx + dy * dy <= r * r {
                        mask.data[j * mask.w + i] = 0;
                    }
                }
            }
        }
        if mask.is_empty() {
            continue;
        }
        let loops: Vec<Vec<[f32; 2]>> = mask::join(mask::contour(&mask)).into_iter().map(|l| simplify(&l, true, 0.05)).filter(|l| l.len() >= 4).collect();
        let tagged = loops
            .iter()
            .map(|l| {
                // Дыра — контур внутри нечётного числа других.
                let depth = loops.iter().filter(|o| !std::ptr::eq(*o, l) && inside(l[0], o)).count();
                (l.clone(), depth % 2 == 1)
            })
            .collect::<Vec<_>>();
        out.push(Zone { effect, loops: tagged, place: String::new() });
    }
    out
}

/// Мост по имени файла: встроенный и библиотечные (`Log_Bridge_…`).
fn is_bridge(key: &str) -> bool {
    let file = key.rsplit(['/', '\\', '|']).next().unwrap_or(key).to_lowercase();
    file.split(|c: char| !c.is_alphanumeric()).any(|w| w == "bridge" || w == "мост")
}

/// Точка внутри замкнутой ломаной (чётность пересечений).
fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    for w in poly.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

pub struct PortalOut {
    pub seg: Segment,
    pub kind: PortalKind,
    /// Картинка створки — у дверей; без своей — встроенная деревянная.
    pub leaf: Option<String>,
}

/// Картинка створки, уже лежащая рядом со сценой: путь для Foundry и ширина в точках.
pub struct LeafFile {
    pub src: String,
    pub width: u32,
}

pub struct Light {
    pub at: [f32; 2],
    pub bright: f32,
    pub dim: f32,
    pub color: &'static str,
    /// Анимация Foundry: «torch», «flame» и т. п.
    pub animation: &'static str,
}

/// Стены карты для VTT: края комнат и нарисованные, без проёмов, сшитые в
/// ломаные и упрощённые — Foundry медленнеет от тысяч мелких отрезков.
pub fn layout(map: &Map, room_edges: &[Segment]) -> Layout {
    let mut segs: Vec<Segment> = room_edges.to_vec();
    // Трубы по полу — рисунок, не стена.
    let mut low_segs: Vec<Segment> = Vec::new();
    for w in map.walls.iter().filter(|w| !w.is_decor()) {
        if w.low {
            low_segs.extend(w.segments());
        } else {
            segs.extend(w.segments());
        }
    }
    let lines = |segs: &[Segment]| -> Vec<Vec<[f32; 2]>> {
        mask::join(map::cut_portals(segs, &map.portals))
            .into_iter()
            .map(|line| {
                let closed = line.len() > 3 && line[0] == line[line.len() - 1];
                simplify(&line, closed, 0.03)
            })
            .filter(|l| l.len() >= 2)
            .collect()
    };
    let walls = lines(&segs);
    let low = lines(&low_segs);
    let portals = map
        .portals
        .iter()
        .filter(|p| p.kind != PortalKind::Opening)
        .map(|p| PortalOut {
            seg: (p.a, p.b),
            kind: p.kind,
            leaf: p.kind.is_door().then(|| p.asset.clone().unwrap_or_else(|| crate::library::DEFAULT_DOOR.into())),
        })
        .collect();
    let lights = map
        .props
        .iter()
        .filter_map(|p| {
            // Огонь: встроенный по ключу, чужой по имени файла, либо выбранный рукой.
            let (bright, dim, color, animation) = p.light()?.params()?;
            Some(Light { at: p.pos, bright: bright * p.scale.sqrt(), dim: dim * p.scale.sqrt(), color, animation })
        })
        .collect();
    let mut zones = zones(map, room_edges);
    // Костёр под открытым небом — маленькая область: пламя и дым рисует модуль.
    for p in map.props.iter().filter(|p| p.asset.strip_prefix(crate::library::BUILTIN) == Some("campfire")) {
        let r = 0.45 * p.size[0] * p.scale;
        let round: Vec<[f32; 2]> = (0..=12).map(|i| {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            [p.pos[0] + r * a.cos(), p.pos[1] + r * a.sin()]
        }).collect();
        zones.push(Zone { effect: "fire", loops: vec![(round, false)], place: String::new() });
    }
    // Встроенный фонтан: эффект «Wild Canvas» по его трём чашам — струя, перелив, круги, журчание.
    for p in map.props.iter().filter(|p| p.asset.strip_prefix(crate::library::BUILTIN) == Some("fountain")) {
        let r = 0.36 * p.size[0] * p.scale;
        let round: Vec<[f32; 2]> = (0..=24).map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            [p.pos[0] + r * a.cos(), p.pos[1] + r * a.sin()]
        }).collect();
        zones.push(Zone { effect: "fountain", loops: vec![(round, false)], place: String::new() });
    }
    // Мост нарисован в фоне — под ним вырез, чтобы вода и рыба не шли поверх настила.
    for p in map.props.iter().filter(|p| is_bridge(&p.asset)) {
        let (hw, hh) = (p.size[0] * p.scale * 0.45, p.size[1] * p.scale * 0.42);
        let (c, s) = (p.rot.to_radians().cos(), p.rot.to_radians().sin());
        let corner = |x: f32, y: f32| [p.pos[0] + x * c - y * s, p.pos[1] + x * s + y * c];
        let rect = vec![corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh), corner(-hw, -hh)];
        zones.push(Zone { effect: "cut", loops: vec![(rect, false)], place: String::new() });
    }
    // Неон бежит по швам пола, а мебель запечена в фон: под каждым предметом на
    // неоновом полу — вырез. Огням и плоскому (люк, решётка, ковёр) не нужен.
    let neon: Vec<mask::Mask> = map.materials.iter().enumerate().filter(|(_, m)| m.live() == map::Live::Neon).map(|(i, _)| map.material_mask(i)).collect();
    if !neon.is_empty() {
        let res = mask::RES as f32;
        let on_neon = |q: [f32; 2]| neon.iter().any(|n| n.get((q[0] * res) as isize, (q[1] * res) as isize) > 0);
        // Врезанное в пол (`Floor_Hatch`, `Floor_Vent_Grate`, `Cables_Floor`) — тоже плоское.
        let inlaid = |k: &str| {
            let f = k.rsplit(['/', '|']).next().unwrap_or(k).to_lowercase();
            f.starts_with("floor_") || f.contains("_floor_")
        };
        for p in map.props.iter().filter(|p| on_neon(p.pos) && p.light().is_none() && !map::flat_by_name(&p.asset) && !map::web_by_name(&p.asset) && !inlaid(&p.asset)) {
            let (hw, hh) = (p.size[0] * p.scale * 0.45, p.size[1] * p.scale * 0.45);
            let (c, s) = (p.rot.to_radians().cos(), p.rot.to_radians().sin());
            let corner = |x: f32, y: f32| [p.pos[0] + x * c - y * s, p.pos[1] + x * s + y * c];
            let rect = vec![corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh), corner(-hw, -hh)];
            zones.push(Zone { effect: "cut", loops: vec![(rect, false)], place: String::new() });
        }
    }
    // Комнаты — область со звуком внутри: в таверне слышна таверна, во дворе — лес.
    if !map.indoor.is_empty() && !map.rooms.is_empty() {
        let loops: Vec<Vec<[f32; 2]>> = mask::join(mask::contour(&map.rooms)).into_iter().map(|l| simplify(&l, true, 0.05)).filter(|l| l.len() >= 4).collect();
        let tagged: Vec<_> = loops.iter().map(|l| (l.clone(), loops.iter().filter(|o| !std::ptr::eq(*o, l) && inside(l[0], o)).count() % 2 == 1)).collect();
        if !tagged.is_empty() {
            zones.push(Zone { effect: "place", loops: tagged, place: map.indoor.clone() });
        }
    }
    // Палатки — по прямоугольнику картинки; выгрузка заменит на контур (`tent_walls`).
    let tents = map.props.iter().filter(|p| map::awning_by_name(&p.asset)).flat_map(|p| tent_walls(p, None)).collect();
    let cliffs = map
        .relief
        .as_ref()
        .map(|r| {
            crate::relief::edges(r)
                .into_iter()
                .filter(|e| e.kind == crate::relief::EdgeKind::Cliff)
                .map(|e| {
                    let mut line = e.points;
                    if e.closed {
                        line.push(line[0]);
                    }
                    simplify(&line, e.closed, 0.05)
                })
                .filter(|l| l.len() >= 2)
                .collect()
        })
        .unwrap_or_default();
    let passages = map
        .props
        .iter()
        .filter(|p| map::passage_by_name(&p.asset))
        .map(|p| {
            let (s, c) = p.rot.to_radians().sin_cos();
            let (hw, hh) = (p.size[0] * p.scale / 2.0, p.size[1] * p.scale / 2.0);
            let (ex, ey) = (hw * c.abs() + hh * s.abs(), hw * s.abs() + hh * c.abs());
            [p.pos[0] - ex, p.pos[1] - ey, p.pos[0] + ex, p.pos[1] + ey]
        })
        .collect();
    Layout { walls, low, cliffs, portals, lights, zones, tents, passages }
}

/// Контур картинки: выпуклая оболочка того, что нарисовано (непрозрачно), в долях
/// картинки 0..1, не больше восьми углов. У палатки — полог вместе с товаром.
pub fn alpha_hull(img: &image::RgbaImage) -> Vec<[f32; 2]> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let step = (w.max(h) / 256).max(1) as usize;
    let solid = |x: u32, y: u32| img.get_pixel(x, y)[3] >= 160;
    let mut pts = Vec::new();
    for y in (0..h).step_by(step) {
        let Some(l) = (0..w).find(|&x| solid(x, y)) else { continue };
        let r = (0..w).rev().find(|&x| solid(x, y)).unwrap_or(l);
        for yy in [y as f32, (y as f32 + step as f32).min(h as f32)] {
            pts.push([l as f32 / w as f32, yy / h as f32]);
            pts.push([(r + 1) as f32 / w as f32, yy / h as f32]);
        }
    }
    let mut hull = convex_hull(pts);
    // Лишние углы: срезаем тот, без которого площадь меньше всего меняется.
    let tri = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0;
    while hull.len() > 3 {
        let n = hull.len();
        let (i, area) = (0..n).map(|i| (i, tri(hull[(i + n - 1) % n], hull[i], hull[(i + 1) % n]))).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        if n > 8 || area < 0.002 {
            hull.remove(i);
        } else {
            break;
        }
    }
    hull
}

/// Выпуклая оболочка (монотонная цепь), обход по часовой на экране (y вниз).
fn convex_hull(mut pts: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut out: Vec<[f32; 2]> = Vec::new();
    for pass in 0..2 {
        let start = out.len();
        let iter: Box<dyn Iterator<Item = &[f32; 2]>> = if pass == 0 { Box::new(pts.iter()) } else { Box::new(pts.iter().rev()) };
        for &p in iter {
            while out.len() >= start + 2 && cross(out[out.len() - 2], out[out.len() - 1], p) <= 0.0 {
                out.pop();
            }
            out.push(p);
        }
        out.pop();
    }
    out
}

/// Стены полога палатки: по контуру картинки (`hull`, доли картинки) или, если его
/// нет, по её прямоугольнику чуть внутрь. Рёбра, что смотрят во вход (`open`), —
/// без стен. Каждый отрезок — внутренность палатки слева от него.
pub fn tent_walls(p: &map::Prop, hull: Option<&[[f32; 2]]>) -> Vec<([f32; 2], [f32; 2])> {
    let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
    let local: Vec<[f32; 2]> = match hull {
        Some(hl) if hl.len() >= 3 => hl.iter().map(|v| [(v[0] - 0.5) * w, (v[1] - 0.5) * h]).collect(),
        // Без контура — чуть внутрь: у картинки поля, а верёвки и колья — снаружи полога.
        _ => [[-0.44, -0.44], [0.44, -0.44], [0.44, 0.44], [-0.44, 0.44]].iter().map(|v| [v[0] * w, v[1] * h]).collect(),
    };
    let n = local.len();
    let c = local.iter().fold([0.0, 0.0], |s, v| [s[0] + v[0] / n as f32, s[1] + v[1] / n as f32]);
    let door = [[0.0, -1.0], [1.0, 0.0], [0.0, 1.0], [-1.0, 0.0]][p.open.unwrap_or(2) as usize % 4];
    // Наружная нормаль ребра: перпендикуляр, повёрнутый от середины.
    let facing: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (local[i], local[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let mut nn = [dy / len, -dx / len];
            let mid = [(a[0] + b[0]) / 2.0 - c[0], (a[1] + b[1]) / 2.0 - c[1]];
            if nn[0] * mid[0] + nn[1] * mid[1] < 0.0 {
                nn = [-nn[0], -nn[1]];
            }
            nn[0] * door[0] + nn[1] * door[1]
        })
        .collect();
    let widest = (0..n).max_by(|&a, &b| facing[a].total_cmp(&facing[b])).unwrap_or(0);
    let (s, co) = p.rot.to_radians().sin_cos();
    let world = |v: [f32; 2]| [p.pos[0] + v[0] * co - v[1] * s, p.pos[1] + v[0] * s + v[1] * co];
    let mid = world(c);
    let mut out = Vec::new();
    for i in (0..n).filter(|&i| i != widest && facing[i] <= 0.55) {
        let (a, b) = (world(local[i]), world(local[(i + 1) % n]));
        // Внутренность слева на экране (y вниз): середина палатки по ту сторону, где крест < 0.
        let cross = (b[0] - a[0]) * (mid[1] - a[1]) - (b[1] - a[1]) * (mid[0] - a[0]);
        out.push(if cross < 0.0 { (a, b) } else { (b, a) });
    }
    out
}

/// Предел формата по стороне: карта больше уходит в JPEG.
pub const WEBP_MAX_SIDE: u32 = 16383;

/// Наибольшая сторона картинки сцены. Больше Foundry не покажет: WebGL берёт
/// текстуру до 16 384 точек (столько и у RX 9070 XT), и WebP её не вместит.
pub const SCENE_MAX_SIDE: u32 = WEBP_MAX_SIDE;

/// Точек на клетку, при которых картинка карты `w`×`h` клеток влезает в
/// [`SCENE_MAX_SIDE`]. Красная площадь 256×256 при 256 на клетку — 65 536 точек,
/// около 13 ГБ памяти и больше предела JPEG; при 100 — 25 600, Foundry её не показал бы.
pub fn fit_px(w: u32, h: u32, px: u32) -> u32 {
    px.min(SCENE_MAX_SIDE / w.max(h).max(1)).max(1)
}

/// WebP со сжатием: картинка карты и прозрачные слои весят в разы меньше PNG.
/// Встроенный кодировщик `image` умеет только без потерь, поэтому libwebp.
#[cfg(target_arch = "wasm32")]
pub fn webp_bytes(img: &image::RgbaImage, _quality: f32) -> Vec<u8> {
    // В браузере libwebp нет: без потерь, зато тем же форматом.
    let mut out = Vec::new();
    let _ = image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8);
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub fn webp_bytes(img: &image::RgbaImage, quality: f32) -> Vec<u8> {
    webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height()).encode(quality).to_vec()
}

/// WebP картинки карты или `None`, если libwebp не справился. У Красной площади
/// (16 128 точек по стороне, миллион блоков 16×16) переполнялся первый раздел
/// кадра (`VP8_ENC_ERROR_PARTITION0_OVERFLOW`), и `encode` ронял программу. Влезает
/// с одним сегментом и `partition_limit` 100: 73 МБ за 14 с против 99 МБ JPEG
/// (с четырьмя сегментами — то же переполнение). Большую сразу так, без пробы.
#[cfg(not(target_arch = "wasm32"))]
pub fn try_webp(img: &image::RgbaImage, quality: f32) -> Option<Vec<u8>> {
    let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height());
    if img.width().max(img.height()) <= 8192
        && let Ok(m) = enc.encode_simple(false, quality)
    {
        return Some(m.to_vec());
    }
    let mut config = webp::WebPConfig::new().ok()?;
    config.quality = quality;
    config.alpha_compression = 1;
    config.segments = 1;
    config.partition_limit = 100;
    enc.encode_advanced(&config).ok().map(|m| m.to_vec())
}

#[cfg(target_arch = "wasm32")]
pub fn try_webp(img: &image::RgbaImage, quality: f32) -> Option<Vec<u8>> {
    Some(webp_bytes(img, quality)).filter(|b| !b.is_empty())
}

/// Упрощение ломаной (Дуглас — Пекер): точки, отступающие от прямой меньше `tol`, уходят.
/// У замкнутой концы совпадают и остаются.
pub fn simplify(line: &[[f32; 2]], closed: bool, tol: f32) -> Vec<[f32; 2]> {
    if line.len() <= 2 {
        return line.to_vec();
    }
    if closed {
        // Делим кольцо в самой дальней от начала точке: у двух половин концы не совпадают.
        let far = (1..line.len() - 1)
            .max_by(|&a, &b| dist2(line[0], line[a]).total_cmp(&dist2(line[0], line[b])))
            .unwrap_or(line.len() / 2);
        let mut out = simplify(&line[..=far], false, tol);
        out.pop();
        out.extend(simplify(&line[far..], false, tol));
        return out;
    }
    let mut keep = vec![false; line.len()];
    keep[0] = true;
    keep[line.len() - 1] = true;
    let mut stack = vec![(0, line.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (mut worst, mut at) = (0.0f32, 0);
        for k in a + 1..b {
            let d = map::segment_distance(line[k], line[a], line[b]);
            if d > worst {
                worst = d;
                at = k;
            }
        }
        if worst > tol {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    line.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Идентификатор документа Foundry: 16 букв и цифр, из номера — чтобы выгрузка
/// одной карты давала один и тот же файл.
fn doc_id(kind: char, n: usize) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut h = (n as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (kind as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    let mut s = String::from("krt");
    s.push(kind);
    for _ in 0..12 {
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        s.push(ABC[(h % ABC.len() as u64) as usize] as char);
    }
    s
}

/// Сцена Foundry 14: картинка в уровне по умолчанию, клетка = `px` пикселей,
/// без полей (padding 0), чтобы координаты стен совпадали с картинкой.
/// Двери подвижные: створка — картинка анимации стены (`leaves` по ключу ассета),
/// двойная дверь — `animation.double`, Foundry сам делит проём на две створки.
#[cfg(test)]
pub fn foundry_scene(map: &Map, name: &str, image_src: &str, px: u32, layout: &Layout, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    foundry_levels(map, name, &[SceneLevel { name: name.to_owned(), image_src: image_src.to_owned(), layout, roofs: Vec::new() }], px, leaves)
}

/// Этаж сцены: имя, картинка и что на нём.
pub struct SceneLevel<'a> {
    pub name: String,
    pub image_src: String,
    pub layout: &'a Layout,
    /// Крыши этажа — тайлами над токенами.
    pub roofs: Vec<RoofTile>,
}

/// Крона: крыша без ограничения света, с поворотом объекта.
pub fn canopy_tile(src: String, center: [f32; 2], size: [f32; 2], rot: f32, px: u32) -> RoofTile {
    let (w, h) = ((size[0] * px as f32).round() as i64, (size[1] * px as f32).round() as i64);
    let (x, y) = ((center[0] * px as f32).round() as i64 - w / 2, (center[1] * px as f32).round() as i64 - h / 2);
    RoofTile { src, rect: [x, y, w, h], rotation: rot, fade: true, leaves: true, wind: 1.0, ..Default::default() }
}

/// Крыша, уже лежащая картинкой рядом со сценой: путь и место в пикселях сцены.
#[derive(Default)]
pub struct RoofTile {
    pub src: String,
    pub rect: [i64; 4],
    /// Поворот в градусах вокруг середины.
    pub rotation: f32,
    /// Не пускает свет и дождь: у крыши да, у кроны нет.
    pub shelter: bool,
    /// Гаснет над токеном. Голое дерево — нет: сквозь ветки и так видно.
    pub fade: bool,
    /// С неё летят листья и светлячки сцены: с кроны и куста да, с белья нет.
    pub leaves: bool,
    /// На 10 футах, а не на 20: шатёр палатки.
    pub low: bool,
    /// Сила ветра «Wild Canvas»: 1 — крона, куст лишь колышется.
    pub wind: f32,
    /// Верёвка с бельём: концы стоят, колышется середина.
    pub rope: bool,
    /// Паутина: на полу под токенами, чуть покачивается; внутри — концы нити
    /// (доли холста от середины), если их удалось найти.
    pub web: Option<Option<[f32; 4]>>,
    /// Сундук: картинки открытого с добычей и пустого; модуль сделает его
    /// контейнером Item Piles.
    pub chest: Option<(String, String)>,
    /// Живой предмет «Wild Canvas»: пометка (ключ, значение) — колонка, машины.
    pub living: Option<(&'static str, String)>,
    /// Сливы на крыше (вшиты в картинку): середина в точках сцены и поворот —
    /// «Wild Canvas» пускает по ним воду в дождь.
    pub scuppers: Vec<[f32; 3]>,
    /// Предмет карты своим тайлом: на полу под токенами, закреплён; `above` —
    /// над крышами (слой «над крышами»).
    pub item: bool,
    pub above: bool,
    /// Парит («Wild Canvas»): дирижабль плывёт, покачивается, под ним тень.
    pub hover: bool,
    /// Подкраска картинки (умножением).
    pub tint: Option<[u8; 3]>,
    /// Контур крыши OSM в точках сцены, двор — `true` ([`crate::roofs::outline`]).
    /// В тайл он уходит долями тайла: Мастер подвинет или растянет тайл — контур
    /// поедет с ним. Без контура «Wild Canvas» берёт весь прямоугольник тайла.
    pub outline: Vec<(Vec<[f32; 2]>, bool)>,
}

/// Впечатать предмет в картинку (для Universal VTT, где тайлов нет): середина
/// и размер в точках картинки, поворот по часовой, подкраска умножением.
pub fn stamp(dst: &mut image::RgbImage, src: &image::RgbaImage, center: [f32; 2], size: [f32; 2], rot: f32, tint: Option<[u8; 3]>) {
    let (s, c) = rot.to_radians().sin_cos();
    let r = (size[0].hypot(size[1]) / 2.0).ceil() + 1.0;
    let (x0, y0) = ((center[0] - r).floor().max(0.0) as u32, (center[1] - r).floor().max(0.0) as u32);
    let (x1, y1) = (((center[0] + r).ceil() as u32).min(dst.width()), ((center[1] + r).ceil() as u32).min(dst.height()));
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let t = tint.map_or([1.0; 3], |t| t.map(|v| v as f32 / 255.0));
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - center[0], y as f32 + 0.5 - center[1]);
            let (u, v) = ((c * dx + s * dy) / size[0] + 0.5, (-s * dx + c * dy) / size[1] + 0.5);
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            // Билинейно: картинка предмета обычно крупнее места в фоне.
            let (fx, fy) = (u * sw - 0.5, v * sh - 0.5);
            let (ix, iy) = (fx.floor(), fy.floor());
            let (ax, ay) = (fx - ix, fy - iy);
            let at = |xx: f32, yy: f32| src.get_pixel(xx.clamp(0.0, sw - 1.0) as u32, yy.clamp(0.0, sh - 1.0) as u32).0.map(|k| k as f32);
            let (p, q, r2, w) = (at(ix, iy), at(ix + 1.0, iy), at(ix, iy + 1.0), at(ix + 1.0, iy + 1.0));
            let px: [f32; 4] = std::array::from_fn(|k| (p[k] * (1.0 - ax) + q[k] * ax) * (1.0 - ay) + (r2[k] * (1.0 - ax) + w[k] * ax) * ay);
            let a = px[3] / 255.0;
            if a <= 0.0 {
                continue;
            }
            let d = dst.get_pixel_mut(x, y);
            for k in 0..3 {
                d[k] = (px[k] * t[k] * a + d[k] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Id уровня: первый — уровень по умолчанию Foundry, остальные свои, 16 знаков.
pub fn level_id(k: usize) -> String {
    if k == 0 { LEVEL_ID.to_owned() } else { format!("kartografLevel{k:02}") }
}

/// Сцена с этажами: каждый — уровень Foundry 14 со своей картинкой, высота этажа
/// 10 футов; стены и огни этажа видны только на нём. Один этаж — как раньше,
/// без высот и привязки к уровню.
pub fn foundry_levels(map: &Map, name: &str, levels: &[SceneLevel], px: u32, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    let multi = levels.len() > 1;
    let mut walls = Vec::new();
    let mut lights = Vec::new();
    let mut regions = Vec::new();
    let mut tiles = Vec::new();
    let mut docs = Vec::new();
    for (k, lv) in levels.iter().enumerate() {
        let tag = |mut v: Value| {
            if multi {
                v["levels"] = json!([level_id(k)]);
            }
            v
        };
        let (w0, l0) = (walls.len(), lights.len());
        level_docs(lv.layout, px, leaves, &mut walls, &mut lights);
        for w in &mut walls[w0..] {
            *w = tag(w.take());
        }
        for l in &mut lights[l0..] {
            *l = tag(l.take());
        }
        // Крыша — тайл выше токенов этажа: гаснет, когда под ней токен,
        // и не пускает свет и дождь. Высота выше дверей: ниже 20 они просвечивают сквозь крышу.
        // `x`, `y` — середина, режим 1 — затухание всего тайла. Без `_stats` сцены (см. ниже)
        // миграции сдвинули бы тайл на полразмера и обнулили `occlusion.modes`.
        for r in &lv.roofs {
            let [x, y, w, h] = r.rect;
            let mut tile = json!({
                "_id": doc_id('t', tiles.len()), "texture": {"src": r.src, "anchorX": 0.5, "anchorY": 0.5}, "x": x + w / 2, "y": y + h / 2, "width": w, "height": h,
                "elevation": k as i64 * 10 + if r.low { 10 } else { 20 }, "sort": tiles.len(), "rotation": r.rotation,
                "occlusion": {"modes": if r.fade { json!([1]) } else { json!([]) }, "alpha": 0},
                "restrictions": {"light": r.shelter, "weather": r.shelter}
            });
            // Предметы карты, паутина и сундук лежат под токенами и закреплены.
            if r.item || r.living.is_some() || r.web.is_some() || r.chest.is_some() {
                tile["locked"] = json!(true);
                if let Some([cr, cg, cb]) = r.tint {
                    tile["texture"]["tint"] = json!(format!("#{cr:02x}{cg:02x}{cb:02x}"));
                }
            }
            if r.item && r.living.is_none() && r.web.is_none() && r.chest.is_none() {
                // Над крышами — выше крыши этажа, иначе на полу.
                tile["elevation"] = json!(k as i64 * 10 + if r.above { 25 } else { 0 });
                tile["occlusion"]["modes"] = json!([]);
                if r.hover {
                    tile["flags"] = json!({LIVE_MODULE: {"hover": 1}});
                }
            } else if let Some((key, value)) = &r.living {
                // Живой предмет под токенами: колонка играет сразу. Дирижабль над
                // крышами так и остаётся над ними — винты крутятся там.
                tile["elevation"] = json!(k as i64 * 10 + if r.above { 25 } else { 0 });
                tile["occlusion"]["modes"] = json!([]);
                tile["flags"] = json!({LIVE_MODULE: {*key: value}});
                if r.hover {
                    tile["flags"][LIVE_MODULE]["hover"] = json!(1);
                }
            } else if r.web.is_some() || r.chest.is_some() {
                tile["elevation"] = json!(k as i64 * 10);
                tile["occlusion"]["modes"] = json!([]);
                tile["flags"] = match &r.chest {
                    Some((open, empty)) => json!({OWN_MODULE: {"chest": {"open": open, "empty": empty}}}),
                    None => match r.web.flatten() {
                        Some(e) => json!({OWN_MODULE: {"web": e.map(|v| (v as f64 * 1000.0).round() / 1000.0)}}),
                        None => json!({OWN_MODULE: {"web": true}}),
                    },
                };
            } else if !r.shelter {
                // Крону качает ветер «Wild Canvas»; крыша стоит.
                // Осенью с неё летят листья, в магическом лесу — светлячки.
                let wind = if (r.wind - 1.0).abs() < 1e-3 { json!(1) } else { json!((r.wind as f64 * 100.0).round() / 100.0) };
                tile["flags"] = json!({LIVE_MODULE: {"wind": wind}});
                if r.rope {
                    tile["flags"][LIVE_MODULE]["rope"] = json!(true);
                }
                if !map.particles.is_empty() && r.leaves {
                    tile["flags"][LIVE_MODULE]["particles"] = json!(map.particles);
                }
            }
            if !r.scuppers.is_empty() {
                // Сливы крыши: середина и поворот; размер — клетка сцены.
                let s: Vec<Value> = r.scuppers.iter().map(|v| json!([v[0].round() as i64, v[1].round() as i64, v[2].round() as i64])).collect();
                tile["flags"] = json!({LIVE_MODULE: {"scuppers": s, "scupperSize": px}});
            }
            if !r.outline.is_empty() && w > 0 && h > 0 {
                // Доли тайла от левого верхнего угла, 4 знака: у тайла в 8000 точек — меньше точки.
                let frac = |v: f32, o: i64, s: i64| ((v as f64 - o as f64) / s as f64 * 1e4).round() / 1e4;
                let rings: Vec<Value> = r
                    .outline
                    .iter()
                    .map(|(ring, hole)| json!({"points": ring.iter().flat_map(|v| [frac(v[0], x, w), frac(v[1], y, h)]).collect::<Vec<_>>(), "hole": hole}))
                    .collect();
                tile["flags"][LIVE_MODULE]["outline"] = json!(rings);
            }
            tiles.push(tag(tile));
        }
        let p = px as f32;
        for z in &lv.layout.zones {
            let shapes: Vec<Value> = z
                .loops
                .iter()
                .map(|(l, hole)| {
                    let pts: Vec<i64> = l[..l.len() - 1].iter().flat_map(|v| [(v[0] * p).round() as i64, (v[1] * p).round() as i64]).collect();
                    json!({"type": "polygon", "points": pts, "hole": hole})
                })
                .collect();
            let (title, color) = match z.effect {
                "lava" => ("Лава", "#ff6a1a"),
                "swamp" => ("Болото", "#6a8f3a"),
                "neon" => ("Неон", "#28e0ff"),
                "fire" => ("Огонь", "#ff9a3c"),
                "cut" => ("Вырез", "#8a6a44"),
                "place" => ("Звук внутри", "#c9a24a"),
                _ => ("Вода", "#2a8fd6"),
            };
            // Болото и вода — труднопроходимые: шаг вдвое дороже (поведение ядра
            // Foundry, работает в любой системе). Без него токен шёл по топи и
            // бочагам Виру раба, как по лугу.
            let behaviors = if matches!(z.effect, "swamp" | "water") {
                json!([{"_id": doc_id('b', regions.len()), "type": "modifyMovementCost", "system": {"difficulties": {"walk": 2}}}])
            } else {
                json!([])
            };
            let region = json!({
                "_id": doc_id('r', regions.len()), "name": title, "color": color, "shapes": shapes,
                "elevation": {"bottom": null, "top": null}, "behaviors": behaviors,
                "flags": if z.effect == "place" { json!({LIVE_MODULE: {"place": z.place}}) } else { json!({LIVE_MODULE: {"effect": z.effect, "fish": z.effect == "water"}}) }
            });
            let mut region = region;
            // Корабль на ходу: вода уходит мимо борта.
            if z.effect == "water" && map.current > 0.0 {
                region["flags"][LIVE_MODULE]["current"] = json!((map.current as f64 * 10.0).round() / 10.0);
                region["flags"][LIVE_MODULE]["drift"] = json!(map.drift.rem_euclid(360.0).round() as i64);
            }
            regions.push(tag(region));
        }
        let mut doc = json!({"_id": level_id(k), "name": lv.name, "background": {"src": lv.image_src, "color": "#999999"}});
        if multi {
            doc["elevation"] = json!({"bottom": k as i64 * 10, "top": (k as i64 + 1) * 10});
            doc["sort"] = json!(k as i64 * 100);
        }
        docs.push(doc);
    }
    // Переходы между этажами: лестница или люк в одном месте на соседних уровнях —
    // область на обоих, с поведением ядра «Сменить уровень» (токен вошёл — вопрос, куда).
    for k in 0..levels.len().saturating_sub(1) {
        for a in &levels[k].layout.passages {
            for b in &levels[k + 1].layout.passages {
                if a[0] >= b[2] || b[0] >= a[2] || a[1] >= b[3] || b[1] >= a[3] {
                    continue;
                }
                let r = [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
                let p = px as f32;
                let pts: Vec<i64> = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].iter().flat_map(|v| [(v[0] * p).round() as i64, (v[1] * p).round() as i64]).collect();
                regions.push(json!({
                    "_id": doc_id('r', regions.len()), "name": format!("{} ↔ {}", levels[k].name, levels[k + 1].name), "color": "#c9a24a",
                    "shapes": [{"type": "polygon", "points": pts, "hole": false}],
                    "elevation": {"bottom": null, "top": null},
                    "levels": [level_id(k), level_id(k + 1)],
                    "behaviors": [{"_id": doc_id('c', regions.len()), "type": "changeLevel", "system": {"movementActions": []}}]
                }));
            }
        }
    }
    json!({
        "name": name,
        "width": map.width * px,
        "height": map.height * px,
        "padding": 0,
        "grid": {"type": 1, "size": px, "distance": 5, "units": "ft"},
        // Фон и в уровне (формат 14), и по-старому: JSON без версии Foundry при
        // импорте проходит миграцию со старого формата, и та берёт фон отсюда.
        "background": {"src": levels[0].image_src},
        "levels": docs,
        "initialLevel": LEVEL_ID,
        "tokenVision": true,
        // Под открытым небом — день: без общего света Foundry 14 оставляет токену
        // лишь его собственный обзор (у Рэндала у брода — круг в клетку, вокруг тьма).
        // Пещера и подземелье — во тьме, светят их огни.
        "environment": {"globalLight": {"enabled": open_sky(map)}},
        // Зимний лес — со снегом Foundry.
        "weather": map.weather,
        "walls": walls,
        "lights": lights,
        "regions": regions,
        "tiles": tiles,
        // Версия новее миграций тайла (14.349, 14.355): данные уже в их формате.
        "_stats": {"coreVersion": "14.355"},
        "flags": scene_flags(map)
    })
}

/// Стены, двери, окна и огни одного этажа — в общие списки сцены.
fn level_docs(layout: &Layout, px: u32, leaves: &std::collections::HashMap<String, LeafFile>, walls: &mut Vec<Value>, lights_out: &mut Vec<Value>) {
    let p = px as f32;
    let at = |v: [f32; 2]| [(v[0] * p).round() as i64, (v[1] * p).round() as i64];
    for line in &layout.walls {
        for w in line.windows(2) {
            let (a, b) = (at(w[0]), at(w[1]));
            if a != b {
                walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]]}));
            }
        }
    }
    // Низкая стена: не пройти; видно и светит поверх одной, сквозь две — нет.
    for line in &layout.low {
        for w in line.windows(2) {
            let (a, b) = (at(w[0]), at(w[1]));
            if a != b {
                walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]], "sight": 10, "light": 10, "sound": 10}));
            }
        }
    }
    // Кромка обрыва: шаг держит, а зрению, свету и звуку не мешает.
    for line in &layout.cliffs {
        for w in line.windows(2) {
            let (a, b) = (at(w[0]), at(w[1]));
            if a != b {
                walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]], "sight": 0, "light": 0, "sound": 0}));
            }
        }
    }
    // Полог палатки — две стены на ребре с ограничениями в разные стороны: изнутри
    // наружу — как сквозь ткань (свет, зрение, звук «ограничено»), снаружи внутрь —
    // глухо. Пройти нельзя ни так, ни так. Внутренность — слева от отрезка.
    for &(a0, b0) in &layout.tents {
        let (a, b) = (at(a0), at(b0));
        if a == b {
            continue;
        }
        walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]], "sight": 10, "light": 10, "sound": 10, "dir": 2}));
        walls.push(json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]], "dir": 1}));
    }
    for po in &layout.portals {
        let (a0, b0) = po.seg;
        let (a, b) = (at(a0), at(b0));
        let mut w = json!({"_id": doc_id('w', walls.len()), "c": [a[0], a[1], b[0], b[1]]});
        match po.kind {
            PortalKind::Opening => continue,
            // Дверь закрыта и непрозрачна, пока её не откроют.
            PortalKind::Door | PortalKind::DoubleDoor => {
                w["door"] = json!(1);
                w["doorSound"] = json!(door_sound(po.leaf.as_deref()));
                let double = po.kind == PortalKind::DoubleDoor;
                if let Some(leaf) = po.leaf.as_ref().and_then(|k| leaves.get(k)) {
                    // Foundry растягивает створку на длину, а по толщине масштабирует
                    // как `сетка / textureGridSize`: столько точек картинки на клетку
                    // её длины — тогда створка той же толщины, что в Картографе.
                    let len = ((b0[0] - a0[0]).powi(2) + (b0[1] - a0[1]).powi(2)).sqrt() / if double { 2.0 } else { 1.0 };
                    let tgs = (leaf.width as f32 / len.max(0.1)).round().max(1.0);
                    w["animation"] = json!({"type": "swing", "texture": leaf.src, "double": double, "duration": 750, "direction": 1, "flip": false, "strength": 1.0});
                    w["flags"] = json!({"core": {"textureGridSize": tgs}});
                }
            }
            // Потайная дверь Foundry: игроки видят стену, пока ведущий не откроет.
            PortalKind::Secret => {
                w["door"] = json!(2);
                w["doorSound"] = json!("stoneBasic");
            }
            // Окно: сквозь него видно и светит, но не пройти.
            PortalKind::Window => {
                w["sight"] = json!(0);
                w["light"] = json!(0);
                w["sound"] = json!(10);
            }
        }
        walls.push(w);
    }
    for l in &layout.lights {
        let [x, y] = at(l.at);
        // Радиус в единицах сетки сцены: футы, 5 на клетку.
        lights_out.push(json!({
            "_id": doc_id('l', lights_out.len()), "x": x, "y": y,
            "config": {"bright": l.bright * 5.0, "dim": l.dim * 5.0, "color": l.color, "alpha": 0.35,
                       "animation": {"type": l.animation, "speed": 3, "intensity": 3}},
            // Модуль Картографа ставит у огня кнопку: гасить и зажигать может и игрок.
            "flags": {OWN_MODULE: {"toggle": true}}
        }));
    }
}

/// Звук двери Foundry (`CONFIG.Wall.doorSounds`) по картинке створки: решётка
/// камеры — тюремная, железная — металл, окованная — тяжёлое дерево.
fn door_sound(leaf: Option<&str>) -> &'static str {
    let file = leaf.map(|k| k.rsplit(['/', '|']).next().unwrap_or(k).to_lowercase()).unwrap_or_default();
    if file.starts_with("door_metal_gray_a_") {
        "jail"
    } else if ["metal", "iron", "steel"].iter().any(|w| file.contains(w)) && !file.contains("wood") {
        "metal"
    } else if file.contains("iron") || file.contains("reinforced") || file.contains("heavy") {
        "woodHeavy"
    } else if file.contains("stone") {
        "stoneBasic"
    } else {
        "woodBasic"
    }
}

/// Universal VTT (`.dd2vtt`): картинка внутри файла, всё в клетках.
pub fn dd2vtt(map: &Map, px: u32, image_b64: &str, layout: &Layout) -> Value {
    let pt = |v: [f32; 2]| json!({"x": v[0], "y": v[1]});
    let los: Vec<Value> = layout.walls.iter().map(|l| Value::Array(l.iter().map(|&v| pt(v)).collect())).collect();
    let portals: Vec<Value> = layout
        .portals
        .iter()
        .map(|po| {
            let ((a, b), kind) = (&po.seg, &po.kind);
            let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            json!({
                "position": pt(mid), "bounds": [pt(*a), pt(*b)],
                "rotation": (b[1] - a[1]).atan2(b[0] - a[0]),
                "closed": kind.is_door() || kind.hidden(), "freestanding": false
            })
        })
        .collect();
    let lights: Vec<Value> = layout
        .lights
        .iter()
        .map(|l| json!({"position": pt(l.at), "range": l.dim, "intensity": 1.0, "color": format!("ff{}", &l.color[1..]), "shadows": true}))
        .collect();
    json!({
        "format": 0.3,
        "resolution": {"map_origin": {"x": 0, "y": 0}, "map_size": {"x": map.width, "y": map.height}, "pixels_per_grid": px},
        "line_of_sight": los,
        "objects_line_of_sight": [],
        "portals": portals,
        "environment": {"baked_lighting": false, "ambient_light": "ffffffff"},
        "lights": lights,
        "image": image_b64
    })
}

/// Путь картинки для `background.src`: относительно папки Data Foundry, если
/// выгрузка лежит в ней (Data узнаётся по соседям `worlds` и `modules`),
/// иначе — одно имя файла.
pub fn foundry_src(image: &std::path::Path) -> String {
    // В модуле путь уже от корня данных Foundry.
    if cfg!(target_arch = "wasm32") {
        return image.to_string_lossy().replace('\\', "/");
    }
    let mut dir = image.parent();
    while let Some(d) = dir {
        if d.join("worlds").is_dir() && d.join("modules").is_dir() {
            if let Ok(rel) = image.strip_prefix(d) {
                return rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            }
        }
        dir = d.parent();
    }
    image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Папка Data Foundry этого компьютера — из его настроек, если он стоит.
pub fn foundry_data_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let text = std::fs::read_to_string(std::path::Path::new(&base).join("FoundryVTT/Config/options.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let data = std::path::PathBuf::from(v.get("dataPath")?.as_str()?).join("Data");
    data.is_dir().then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooms_get_their_own_sound_region() {
        let mut m = Map::new(12, 12);
        let rooms = crate::mask::cells(m.rooms.w, m.rooms.h, (2, 2), (7, 6));
        m.paint(crate::map::Layer::Rooms, 0, &rooms);
        assert!(layout(&m, &m.room_edges()).zones.iter().all(|z| z.effect != "place"), "без indoor области нет");
        m.indoor = "tavern".into();
        let lay = layout(&m, &m.room_edges());
        let z = lay.zones.iter().find(|z| z.effect == "place").expect("нет области звука");
        assert_eq!(z.place, "tavern");
        assert_eq!(z.loops.len(), 1);
    }

    #[test]
    fn bog_patches_join_one_region_and_the_boardwalk_is_cut_out() {
        // Мох и светлое пятно на нём — оба болото: одна область без дыры под
        // пятном; мостки поперёк — вырез (по ним шаг обычный).
        let mut m = Map::new(12, 8);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        m.materials[water].live = crate::map::Live::Swamp;
        let straw = 3;
        m.materials[straw].live = crate::map::Live::Swamp;
        m.paint(crate::map::Layer::Floor, water, &crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (11, 7)));
        m.paint(crate::map::Layer::Floor, straw, &crate::mask::cells(m.rooms.w, m.rooms.h, (7, 1), (9, 2)));
        m.paths.push(crate::map::Path { points: vec![[-1.0, 5.0], [13.0, 5.0]], material: 0, width: 1.2, soft: 0.0, rough: 0.0, along: true });
        m.prune();
        let lay = layout(&m, &[]);
        let swamp: Vec<&Zone> = lay.zones.iter().filter(|z| z.effect == "swamp").collect();
        assert_eq!(swamp.len(), 1, "одна область болота");
        let covered = |p: [f32; 2]| {
            let n = swamp[0].loops.iter().filter(|(l, _)| inside(p, l)).count();
            n % 2 == 1
        };
        assert!(covered([8.5, 2.0]), "светлое пятно — тоже болото");
        assert!(covered([3.5, 2.5]), "мох — болото");
        assert!(!covered([6.0, 5.0]), "настил вырезан");
    }

    #[test]
    fn water_becomes_living_region_with_island_hole() {
        let mut m = Map::new(12, 12);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        assert_eq!(m.materials[water].live(), crate::map::Live::Water);
        let all = crate::mask::cells(m.rooms.w, m.rooms.h, (1, 1), (9, 9));
        m.paint(crate::map::Layer::Floor, water, &all);
        let grass = crate::mask::cells(m.rooms.w, m.rooms.h, (4, 4), (6, 6));
        m.paint(crate::map::Layer::Floor, 3, &grass);
        m.prune();
        let lay = layout(&m, &[]);
        assert_eq!(lay.zones.len(), 1);
        let z = &lay.zones[0];
        assert_eq!((z.effect, z.loops.len()), ("water", 2));
        assert_eq!(z.loops.iter().filter(|l| l.1).count(), 1);
        let scene = foundry_levels(&m, "Пруд", &[SceneLevel { name: "Пруд".into(), image_src: "p.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        let r = &scene["regions"][0];
        assert_eq!(r["flags"][LIVE_MODULE]["effect"], "water");
        // Вода — труднопроходимая: шаг вдвое дороже.
        assert_eq!(r["behaviors"][0]["type"], "modifyMovementCost");
        assert_eq!(r["behaviors"][0]["system"]["difficulties"]["walk"], 2);
        assert_eq!(r["shapes"].as_array().unwrap().len(), 2);
        assert_eq!(r["shapes"][0]["type"], "polygon");
    }

    #[test]
    fn hatch_over_ladder_links_two_levels() {
        // Люк на палубе над трапом в трюме — переход между уровнями; трап в стороне — нет.
        let hold = Map::new(10, 10);
        let mut deck = hold.new_level();
        let mut hold = hold;
        let prop = |name: &str, x: f32, y: f32| crate::map::Prop { asset: format!("objects/{name}"), pos: [x, y], size: [1.0, 2.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
        hold.props.push(prop("Ladder_Wood_Dark_A1_1x2.webp", 5.0, 5.0));
        hold.props.push(prop("Ladder_Wood_Dark_A1_1x2.webp", 8.0, 2.0));
        deck.props.push(prop("Trapdoor_Wood_Ashen_A2_Open_1x1.webp", 5.2, 4.6));
        deck.props.push(prop("Barrel_Wood_A1_1x1.webp", 8.0, 2.0));
        assert!(crate::map::passage_by_name("Stairs_Shade") == false && crate::map::passage_by_name("x/Floor_Hatch_A_1x1.webp"));
        let (lh, ld) = (layout(&hold, &[]), layout(&deck, &[]));
        let levels = [
            SceneLevel { name: "Трюм".into(), image_src: "h.jpg".into(), layout: &lh, roofs: Vec::new() },
            SceneLevel { name: "Палуба".into(), image_src: "d.jpg".into(), layout: &ld, roofs: Vec::new() },
        ];
        let scene = foundry_levels(&deck, "Корабль", &levels, 100, &Default::default());
        let links: Vec<&Value> = scene["regions"].as_array().unwrap().iter().filter(|r| r["behaviors"][0]["type"] == "changeLevel").collect();
        assert_eq!(links.len(), 1, "один переход: люк над трапом");
        assert_eq!(links[0]["levels"], json!([level_id(0), level_id(1)]));
        assert_eq!(links[0]["name"], "Трюм ↔ Палуба");
        assert_eq!(links[0]["behaviors"][0]["system"]["movementActions"], json!([]));
    }

    #[test]
    fn no_living_water_under_walls() {
        // Борт корабля стоит над водой: пена не должна ложиться на стену.
        let mut m = Map::new(12, 12);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        m.paint(crate::map::Layer::Floor, water, &crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (11, 11)));
        m.wall_style.width = 0.4;
        m.walls.push(crate::map::Wall { points: vec![[2.0, 6.0], [10.0, 6.0]], closed: false, bends: Vec::new(), low: true, style: None });
        m.prune();
        let lay = layout(&m, &m.room_edges());
        let z = lay.zones.iter().find(|z| z.effect == "water").unwrap();
        let wet = |p: [f32; 2]| z.loops.iter().filter(|(l, _)| inside(p, l)).count() % 2 == 1;
        assert!(!wet([6.0, 6.0]) && !wet([6.0, 6.18]), "под стеной сухо");
        assert!(wet([6.0, 3.0]) && wet([6.0, 6.6]), "рядом со стеной — вода");
    }

    #[test]
    fn ship_underway_makes_water_run_past() {
        let mut m = Map::new(12, 12);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        m.paint(crate::map::Layer::Floor, water, &crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (11, 11)));
        m.paint(crate::map::Layer::Floor, 3, &crate::mask::cells(m.rooms.w, m.rooms.h, (5, 3), (7, 9)));
        m.prune();
        let lay = layout(&m, &[]);
        let level = |lay| [SceneLevel { name: "Море".into(), image_src: "s.jpg".into(), layout: lay, roofs: Vec::new() }];
        let still = foundry_levels(&m, "Море", &level(&lay), 100, &Default::default());
        assert!(still["regions"][0]["flags"][LIVE_MODULE].get("current").is_none(), "стоячая вода без течения");
        (m.current, m.drift) = (1.25, 450.0);
        let back: Map = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!((back.current, back.drift), (1.25, 450.0));
        let sea = foundry_levels(&m, "Море", &level(&lay), 100, &Default::default());
        let f = &sea["regions"][0]["flags"][LIVE_MODULE];
        assert_eq!((f["effect"].as_str(), f["current"].as_f64(), f["drift"].as_i64()), (Some("water"), Some(1.3), Some(90)));
        let old: Map = serde_json::from_str(&serde_json::to_string(&Map::new(2, 2)).unwrap().replace(",\"drift\":90.0", "")).unwrap();
        assert_eq!((old.current, old.drift), (0.0, 90.0), "старые карты: вода стоит, направление вниз");
    }

    #[test]
    fn neon_floor_gets_region_and_cuts_under_furniture() {
        use crate::map::{Live, Material, Paint, Prop};
        let mut m = Map::new(10, 8);
        let key = "pack:cp|textures/terrain/cyberpunk/floors/Floor_Alloy_Grid_A.webp";
        let floor = m.material_index(Material { name: "Floor_Alloy_Grid_A".into(), paint: Paint::Texture(key.into()), vary: false, live: Live::Auto }).unwrap();
        assert_eq!(m.materials[floor].live(), Live::Neon);
        let all = crate::mask::cells(m.rooms.w, m.rooms.h, (1, 1), (8, 6));
        m.paint(crate::map::Layer::Rooms, floor, &all);
        let prop = |asset: &str, pos: [f32; 2]| Prop { asset: asset.into(), pos, size: [2.0, 1.0], rot: 90.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
        m.props.push(prop("pack:cp|textures/objects/Armchair_Neon_A_2x1.webp", [4.0, 4.0]));
        m.props.push(prop("pack:cp|textures/objects/Neon_Lamp_Floor_A_1x1.webp", [6.0, 3.0]));
        m.props.push(prop("pack:cp|textures/objects/Floor_Vent_Grate_A_1x1.webp", [3.0, 3.0]));
        m.props.push(prop("pack:cp|textures/objects/Cables_Floor_A_3x1.webp", [5.0, 5.5]));
        // Вне неонового пола — без выреза.
        m.props.push(prop("pack:cp|textures/objects/Armchair_Neon_A_2x1.webp", [0.5, 0.5]));
        let lay = layout(&m, &[]);
        assert!(lay.zones.iter().any(|z| z.effect == "neon"));
        let cuts: Vec<_> = lay.zones.iter().filter(|z| z.effect == "cut").collect();
        assert_eq!(cuts.len(), 1, "вырез только под креслом на полу");
        let xs: Vec<f32> = cuts[0].loops[0].0.iter().map(|v| v[0]).collect();
        // Повёрнуто на 90°: поперёк по x — клетка, а не две.
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min) < 1.0);
    }

    #[test]
    fn osm_roof_tile_carries_its_outline_in_tile_fractions() {
        let a = Map::new(6, 4);
        let la = layout(&a, &a.room_edges());
        let roof = RoofTile {
            src: "k/r.webp".into(),
            rect: [100, 200, 400, 200],
            shelter: true,
            fade: true,
            outline: vec![(vec![[100.0, 200.0], [500.0, 200.0], [300.0, 400.0]], false), (vec![[250.0, 250.0], [350.0, 250.0], [300.0, 300.0]], true)],
            ..Default::default()
        };
        let plain = RoofTile { src: "k/q.webp".into(), rect: [0, 0, 100, 100], shelter: true, fade: true, ..Default::default() };
        let scene = foundry_levels(&a, "Дом", &[SceneLevel { name: "Дом".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![roof, plain] }], 100, &Default::default());
        let o = &scene["tiles"][0]["flags"][LIVE_MODULE]["outline"];
        assert_eq!(o[0]["points"], json!([0.0, 0.0, 1.0, 0.0, 0.5, 1.0]));
        assert_eq!((o[0]["hole"].as_bool(), o[1]["hole"].as_bool()), (Some(false), Some(true)));
        assert_eq!(o[1]["points"][1].as_f64(), Some(0.25));
        assert!(scene["tiles"][1].get("flags").is_none(), "у своей крыши Картографа контура нет — берётся прямоугольник");
    }

    #[test]
    fn airship_floats_above_roofs_with_spinning_rotors() {
        let a = Map::new(6, 4);
        let la = layout(&a, &a.room_edges());
        let key = "objects/steampunk/Airship_Gnome_Brass_A_5x10.webp";
        assert!(crate::map::hover_by_name(key) && !crate::map::hover_by_name("objects/Barrel_A1_1x1.webp"));
        let mut ship = canopy_tile("k/ship.webp".into(), [3.0, 2.0], [2.0, 4.0], 90.0, 100);
        (ship.item, ship.above, ship.hover, ship.living) = (true, true, true, crate::map::living_by_name(key));
        let mut balloon = canopy_tile("k/b.webp".into(), [1.0, 1.0], [1.0, 1.0], 0.0, 100);
        (balloon.item, balloon.above, balloon.hover) = (true, true, true);
        let levels = [SceneLevel { name: "Город".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![ship, balloon] }];
        let scene = foundry_levels(&a, "Город", &levels, 100, &Default::default());
        let t = &scene["tiles"][0];
        assert_eq!(t["elevation"], 25, "над крышами, а не на полу, как колонка");
        assert_eq!(t["flags"][LIVE_MODULE]["machine"], "Airship_Gnome_Brass_A");
        assert_eq!(t["flags"][LIVE_MODULE]["hover"], 1);
        let b = &scene["tiles"][1];
        assert_eq!((b["elevation"].as_i64(), b["flags"][LIVE_MODULE]["hover"].as_i64()), (Some(25), Some(1)));
    }

    #[test]
    fn big_map_shrinks_to_what_foundry_shows() {
        assert_eq!(fit_px(40, 30, 140), 140, "боевая карта — как просили");
        assert_eq!(fit_px(256, 256, 256), 63);
        assert_eq!(fit_px(100, 300, 100), 54, "по длинной стороне");
        assert!(fit_px(256, 256, 256) * 256 <= SCENE_MAX_SIDE);
        assert_eq!(fit_px(100_000, 1, 256), 1, "не меньше точки");
    }

    #[test]
    fn levels_become_foundry_levels() {
        let mut a = Map::new(6, 4);
        let w = crate::mask::cells(a.rooms.w, a.rooms.h, (1, 1), (3, 2));
        a.paint(crate::map::Layer::Rooms, 0, &w);
        let b = a.new_level();
        let mut b2 = b.clone();
        b2.walls.push(crate::map::Wall { points: vec![[0.0, 0.0], [6.0, 0.0]], closed: false, bends: Vec::new(), low: false, style: None });
        let (la, lb) = (layout(&a, &a.room_edges()), layout(&b2, &b2.room_edges()));
        let levels = [
            SceneLevel { name: "Низ".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![RoofTile { src: "k/r.png".into(), rect: [50, 50, 300, 200], rotation: 0.0, shelter: true, fade: true, leaves: false, low: false, wind: 1.0, rope: false, web: None, chest: None, living: None, scuppers: vec![[100.0, 50.0, 0.0]], ..Default::default() }] },
            SceneLevel { name: "Верх".into(), image_src: "k/b.jpg".into(), layout: &lb, roofs: vec![canopy_tile("k/t.png".into(), [3.0, 2.0], [3.0, 3.0], 90.0, 100)] },
        ];
        let scene = foundry_levels(&a, "Дом", &levels, 100, &Default::default());
        let lv = scene["levels"].as_array().unwrap();
        assert_eq!(lv.len(), 2);
        assert_eq!(lv[1]["_id"].as_str().unwrap().len(), 16);
        assert_eq!(lv[1]["elevation"]["bottom"], 10);
        assert_eq!(lv[1]["background"]["src"], "k/b.jpg");
        let walls = scene["walls"].as_array().unwrap();
        // Стена второго этажа видна только на нём; id не повторяются.
        assert_eq!(walls.last().unwrap()["levels"][0], lv[1]["_id"]);
        assert!(walls.iter().filter(|w| w["levels"][0] == lv[0]["_id"]).count() >= 4);
        let ids: std::collections::HashSet<_> = walls.iter().map(|w| w["_id"].as_str().unwrap()).collect();
        assert_eq!(ids.len(), walls.len());
        // Крыша — тайл над токенами первого этажа, гаснет над токеном.
        let t = &scene["tiles"][0];
        assert_eq!((t["x"].as_i64(), t["width"].as_i64(), t["elevation"].as_i64()), (Some(200), Some(300), Some(20)));
        assert_eq!(t["occlusion"]["modes"][0], 1);
        assert_eq!(scene["_stats"]["coreVersion"], "14.355");
        assert_eq!(t["texture"]["anchorX"].as_f64(), Some(0.5));
        assert_eq!(t["levels"][0], lv[0]["_id"]);
        // Крону качает ветер «Wild Canvas», крышу — нет; у крыши лишь сливы для дождя.
        assert!(t["flags"][LIVE_MODULE].get("wind").is_none());
        assert_eq!(t["flags"][LIVE_MODULE]["scuppers"][0], json!([100, 50, 0]));
        let crown = &scene["tiles"][1];
        assert_eq!(crown["flags"][LIVE_MODULE]["wind"], 1);
        assert!(crown["flags"][LIVE_MODULE].get("particles").is_none());
        assert_eq!(crown["restrictions"]["light"], false);
        let mut autumn = a.clone();
        autumn.particles = "leaves".into();
        let scene = foundry_levels(&autumn, "Дом", &levels, 100, &Default::default());
        assert_eq!(scene["tiles"][1]["flags"][LIVE_MODULE]["particles"], "leaves");
        assert!(scene["tiles"][0]["flags"][LIVE_MODULE].get("particles").is_none());
        assert_eq!(scene["weather"], "");
        autumn.weather = "snow".into();
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["weather"], "snow");
        // Звук места — флагом сцены; без него его нет (поверхности для шагов — есть).
        assert!(scene["flags"][LIVE_MODULE].get("ambience").is_none());
        autumn.ambience = "forest-autumn".into();
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][LIVE_MODULE]["ambience"], "forest-autumn");
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][LIVE_MODULE]["clouds"], 0.6);
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["environment"]["globalLight"]["enabled"], true, "лес — днём");
        autumn.ambience = "cave-1".into();
        assert!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][LIVE_MODULE].get("clouds").is_none());
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["environment"]["globalLight"]["enabled"], false, "пещера — во тьме");
        // Зерно — рыбкам; и без звука места.
        autumn.ambience.clear();
        autumn.seed = "озеро".into();
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][LIVE_MODULE]["seed"], "озеро");
    }
    use crate::map::{Layer, Portal, Prop, Wall};

    #[test]
    fn surfaces_follow_the_top_floor() {
        let mut m = Map::new(4, 2);
        assert!(surfaces(&m).is_none());
        // Трава (3) везде, доски (1) слева, вода (8) в правом нижнем углу.
        m.paint_rect(Layer::Floor, (0, 0), (3, 1), Some(3));
        m.paint_rect(Layer::Floor, (0, 0), (0, 1), Some(1));
        m.paint_rect(Layer::Floor, (3, 1), (3, 1), Some(8));
        let s = surfaces(&m).unwrap();
        assert_eq!((s["w"].as_u64(), s["h"].as_u64()), (Some(4), Some(2)));
        assert_eq!(s["cells"], "orrrorrw");
        let flags = scene_flags(&m);
        assert_eq!(flags[LIVE_MODULE]["surfaces"]["cells"], "orrrorrw");
        // Песчаник — камень, иней — снег, брусчатка — камень, песок — песок.
        let sf = |i: usize| m.materials[i].surface();
        let by = |name: &str| m.materials.iter().position(|x| x.name == name).unwrap();
        assert_eq!(sf(by("Песчаник")), Some('s'));
        assert_eq!(sf(by("Иней")), Some('n'));
        assert_eq!(sf(by("Брусчатка")), Some('s'));
        assert_eq!(sf(by("Песок")), Some('a'));
        assert_eq!(sf(by("Лава")), None);
        assert_eq!(sf(by("Болото")), Some('m'));
    }

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let line = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
        let s = simplify(&line, true, 0.01);
        assert_eq!(s, vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]]);
        let open = simplify(&[[0.0, 0.0], [1.0, 0.001], [2.0, 0.0]], false, 0.01);
        assert_eq!(open.len(), 2);
    }

    #[test]
    fn scene_has_walls_doors_windows_and_fire_light() {
        let mut m = Map::new(8, 6);
        m.paint_rect(Layer::Rooms, (1, 1), (4, 3), Some(0));
        m.walls.push(Wall { points: vec![[6.0, 0.0], [6.0, 6.0]], closed: false, bends: Vec::new(), low: false, style: None });
        m.portals.push(Portal { a: [2.0, 1.0], b: [3.0, 1.0], kind: PortalKind::Door, asset: None, frame: None });
        m.portals.push(Portal { a: [6.0, 2.0], b: [6.0, 3.0], kind: PortalKind::Window, asset: None, frame: None });
        // Двойная дверь в нижней стене и пустой проём в левой.
        m.portals.push(Portal { a: [2.0, 4.0], b: [4.0, 4.0], kind: PortalKind::DoubleDoor, asset: None, frame: None });
        m.portals.push(Portal { a: [1.0, 2.0], b: [1.0, 3.0], kind: PortalKind::Opening, asset: None, frame: None });
        m.props.push(Prop { asset: format!("{}campfire", crate::library::BUILTIN), pos: [3.0, 2.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None });
        // Чужая жаровня светит по имени, выключенный рукой костёр — нет, бочка с огнём по выбору — да.
        let other = |asset: &str, light| Prop { asset: asset.into(), pos: [5.0, 5.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light, canopy: None, layer: None, tint: None, open: None };
        m.props.push(other("pack:fa.dungeondraft_pack|textures/objects/Brazier_Iron_A_1x1.webp", None));
        m.props.push(other("pack:fa.dungeondraft_pack|textures/objects/Campfire_B_1x1.webp", Some(crate::map::Glow::Off)));
        m.props.push(other("pack:fa.dungeondraft_pack|textures/objects/Barrel_Fire_A_1x1.webp", Some(crate::map::Glow::Lamp)));
        let lay = layout(&m, &m.room_edges());
        let mut leaves = std::collections::HashMap::new();
        leaves.insert(crate::library::DEFAULT_DOOR.to_owned(), LeafFile { src: "kartograf/door.webp".into(), width: 512 });
        let scene = foundry_scene(&m, "Проба", "kartograf/proba.jpg", 100, &lay, &leaves);
        assert_eq!((scene["width"].as_u64(), scene["height"].as_u64()), (Some(800), Some(600)));
        assert_eq!(scene["levels"][0]["background"]["src"], "kartograf/proba.jpg");
        assert_eq!(scene["background"]["src"], "kartograf/proba.jpg");
        let walls = scene["walls"].as_array().unwrap();
        // Комната 4×3: дверь сверху, двойная дверь снизу, пустой проём слева режут
        // её на три ломаные — 2 + 2 + 3 отрезка; ручная стена окном разрезана
        // надвое; плюс дверь, двойная и окно. Пустой проём стеной не стал.
        assert_eq!(walls.len(), 7 + 2 + 3, "{walls:#?}");
        let door = walls.iter().find(|w| w["door"] == 1 && w["animation"]["double"] == false).expect("дверь");
        assert_eq!(door["c"], json!([200, 100, 300, 100]));
        assert_eq!(door["animation"]["texture"], "kartograf/door.webp");
        assert_eq!(door["flags"]["core"]["textureGridSize"], 512.0);
        let double = walls.iter().find(|w| w["animation"]["double"] == true).expect("двойная дверь");
        assert_eq!(double["c"], json!([200, 400, 400, 400]));
        // Каждая створка — клетка: картинка в 512 точек на клетку её длины.
        assert_eq!(double["flags"]["core"]["textureGridSize"], 512.0);
        let window = walls.iter().find(|w| w["sight"] == 0).expect("окно");
        assert_eq!(window["c"], json!([600, 200, 600, 300]));
        let ids: std::collections::HashSet<_> = walls.iter().map(|w| w["_id"].as_str().unwrap().to_owned()).collect();
        assert!(ids.len() == walls.len() && ids.iter().all(|i| i.len() == 16));
        let light = &scene["lights"][0];
        assert_eq!((light["x"].as_i64(), light["y"].as_i64(), light["config"]["dim"].as_f64()), (Some(300), Some(200), Some(30.0)));
        assert_eq!(scene["lights"].as_array().unwrap().len(), 3, "костёр, чужая жаровня и бочка по выбору");
        assert_eq!(scene["lights"][1]["config"]["animation"]["type"], "torch");
        assert_eq!(scene["lights"][2]["config"]["color"], "#ffc870");
        let uv = dd2vtt(&m, 100, "", &lay);
        assert_eq!(uv["portals"].as_array().unwrap().len(), 3);
        assert_eq!(uv["resolution"]["pixels_per_grid"], 100);
    }

    #[test]
    fn secret_passage_is_a_foundry_secret_door_but_a_solid_wall_in_the_picture() {
        let mut m = Map::new(8, 6);
        m.paint_rect(Layer::Rooms, (1, 1), (4, 3), Some(0));
        m.portals.push(Portal { a: [2.0, 1.0], b: [3.0, 1.0], kind: PortalKind::Secret, asset: None, frame: None });
        let lay = layout(&m, &m.room_edges());
        let scene = foundry_scene(&m, "Тайна", "t.jpg", 100, &lay, &Default::default());
        let walls = scene["walls"].as_array().unwrap();
        let secret = walls.iter().find(|w| w["door"] == 2).expect("потайная дверь");
        assert_eq!(secret["c"], json!([200, 100, 300, 100]));
        assert!(secret.get("animation").is_none());
        // В сцене стена вокруг хода разрезана, а на холсте (ширина с запасом) — нет.
        assert!(lay.walls.iter().all(|l| l.windows(2).all(|w| !(w[0][1] == 1.0 && w[1][1] == 1.0 && w[0][0] < 2.5 && w[1][0] > 2.5))));
        let drawn = map::cut_portals_wide(&m.room_edges(), &m.portals, 0.1);
        assert!(drawn.iter().any(|(a, b)| a[1] == 1.0 && b[1] == 1.0 && a[0].min(b[0]) <= 2.0 && a[0].max(b[0]) >= 3.0));
        let uv = dd2vtt(&m, 100, "", &lay);
        assert_eq!(uv["portals"][0]["closed"], true);
    }

    #[test]
    fn src_is_relative_to_foundry_data() {
        let root = std::env::temp_dir().join(format!("kartograf-src-{}", std::process::id()));
        for d in ["worlds", "modules", "kartograf/зал"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        assert_eq!(foundry_src(&root.join("kartograf/зал/зал.jpg")), "kartograf/зал/зал.jpg");
        assert_eq!(foundry_src(&std::env::temp_dir().join("x.jpg")), "x.jpg");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn tent_walls_follow_drawing() {
        // Ромб посреди пустой картинки: оболочка внутри него, не по краям картинки.
        let mut img = image::RgbaImage::new(200, 100);
        for y in 0..100u32 {
            for x in 0..200u32 {
                let (dx, dy) = ((x as f32 - 100.0).abs() / 80.0, (y as f32 - 50.0).abs() / 40.0);
                if dx + dy <= 1.0 {
                    img.put_pixel(x, y, image::Rgba([200, 200, 200, 255]));
                }
            }
        }
        let hull = alpha_hull(&img);
        assert!((3..=8).contains(&hull.len()), "{hull:?}");
        assert!(hull.iter().all(|v| v[0] >= 0.09 && v[0] <= 0.91 && v[1] >= 0.09 && v[1] <= 0.91), "{hull:?}");
        let p = crate::map::Prop { asset: "x".into(), pos: [10.0, 10.0], size: [4.0, 2.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: Some(2) };
        let walls = tent_walls(&p, Some(&hull));
        assert!(!walls.is_empty() && walls.len() < hull.len());
        for (a, b) in &walls {
            // Внутренность слева (y вниз), и ни одно ребро не обращено вниз, ко входу.
            let cross = (b[0] - a[0]) * (10.0 - a[1]) - (b[1] - a[1]) * (10.0 - a[0]);
            assert!(cross < 0.0);
            assert!((a[1] + b[1]) / 2.0 < 10.0 + 0.5 || (a[0] - b[0]).abs() < 0.2, "{a:?} {b:?}");
        }
    }
}
