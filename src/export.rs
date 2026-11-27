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
    /// Из них — лестницы маршем ([`crate::ground3d::flight_by_name`]): в 3D Canvas по ним идут
    /// ногами, переход у них помечен для модуля.
    pub stairs: Vec<[f32; 4]>,
}

/// Прорезать в стенах раскладки проходы `gaps` — как пустые проёмы на карте (отрезок на
/// прямой стены).
pub fn open_walls(lay: &mut Layout, gaps: &[Segment]) {
    if gaps.is_empty() {
        return;
    }
    let portals: Vec<map::Portal> = gaps.iter().map(|&(a, b)| map::Portal { a, b, kind: PortalKind::Opening, asset: None, frame: None }).collect();
    let segs: Vec<Segment> = lay.walls.iter().flat_map(|l| l.windows(2).map(|w| (w[0], w[1]))).collect();
    lay.walls = mask::join(map::cut_portals(&segs, &portals))
        .into_iter()
        .map(|line| {
            let closed = line.len() > 3 && line[0] == line[line.len() - 1];
            simplify(&line, closed, 0.03)
        })
        .filter(|l| l.len() >= 2)
        .collect();
}

/// Стоячая вода в «Wild Canvas»: зыбь, блики на дне, искры и скорость — доли обычных.
const STILL_WATER: (f64, f64, f64, f64) = (0.15, 0.65, 0.3, 0.5);

pub struct Zone {
    pub effect: &'static str,
    pub loops: Vec<(Vec<[f32; 2]>, bool)>,
    /// У области «внутри» (`effect` = "place") — ключ её звука.
    pub place: String,
    /// Доля мелководья у воды, 0..1: по ней блики на дне. Гавань глубокая —
    /// дна не видно; у пруда с мелкими берегами блики как были.
    pub shallow: f32,
    /// Доля мутной воды, 0..1 ([`crate::map::Material::murky`]): больше половины —
    /// у области нет пены у берега, бликов на дне и рыб.
    pub murky: f32,
    /// Доля воды без рыб, 0..1 ([`crate::map::Material::fishless`]): больше половины —
    /// у области рыб нет («Без рыбы»), блики и пена как у обычной воды.
    pub fishless: f32,
    /// Доля стоячей воды, 0..1 ([`crate::map::Material::still`]): больше половины и без
    /// течения — у области нет зыби и пены, свет на дне слабый и медленный.
    pub still: f32,
    /// Своё течение участка воды ([`crate::map::Flow`]): скорость и куда. Нет — как у
    /// всей карты.
    pub flow: Option<(f32, f32)>,
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

/// Сухой верх предмета, стоящего в воде: всё ниже ватерлинии (доля `sunk` от низа картинки)
/// прозрачно, по самой линии край смягчён на пару точек. Низ остаётся в картинке карты.
pub fn dry_part(img: &image::RgbaImage, sunk: f32) -> image::RgbaImage {
    let mut out = img.clone();
    if sunk <= 0.0 {
        return out;
    }
    let line = (1.0 - sunk.clamp(0.0, 1.0)) * img.height() as f32;
    for (_, y, p) in out.enumerate_pixels_mut() {
        let keep = ((line - y as f32 - 0.5) / 2.0 + 0.5).clamp(0.0, 1.0);
        p[3] = (p[3] as f32 * keep).round() as u8;
    }
    out
}

/// Насколько сцена притемнена в полумраке (`darknessLevel` Foundry): общий свет остаётся,
/// фишка видит всё в поле зрения, лампы выделяются. Подобрано на станции с полусотней ламп:
/// при 0,6 мелочь на полу читается с трудом, при 0,3 лампы теряются.
const DIM_DARKNESS: f64 = 0.5;

/// Свет сцены Foundry: включён ли общий свет и насколько сцена притемнена. Выбор карты
/// (`scene_light`) — прежде догадки по звуку места.
pub fn scene_light(map: &Map) -> (bool, f64) {
    match map.scene_light.as_str() {
        "day" => (true, 0.0),
        "dim" => (true, DIM_DARKNESS),
        "dark" => (false, 0.0),
        _ => (open_sky(map), 0.0),
    }
}

/// Флаги сцены: формат Картографа и звук места для «Wild Canvas».
fn scene_flags(map: &Map) -> Value {
    let mut flags = json!({"kartograf": {"format": 1}});
    // Насколько карта выросла влево и вверх: модуль сдвигает по этому фишки старой сцены.
    // `light` — свет сцены, выбранный на карте (пусто — по звуку места): модуль при обновлении
    // сцены берёт новый свет, только если этот выбор с прошлой выгрузки сменился, — иначе
    // остаётся то, что Мастер настроил в Foundry.
    flags[OWN_MODULE] = json!({"origin": map.origin, "light": map.scene_light});
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
fn zones(map: &Map, room_edges: &[Segment], seen: &dyn Fn(&str) -> Option<[f32; 4]>) -> Vec<Zone> {
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
    // Сколько воды глубокой и сколько всего — по закрашенному.
    let (mut deep, mut wet, mut murk, mut cold, mut calm) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for (m, mat) in map.materials.iter().enumerate() {
        let Some(effect) = mat.live().effect() else { continue };
        let mask = map.material_mask(m);
        if effect == "water" {
            let area: u64 = mask.sum();
            wet += area;
            if mat.deep() {
                deep += area;
            }
            if mat.murky() {
                murk += area;
            }
            if mat.fishless() {
                cold += area;
            }
            if mat.still() {
                calm += area;
            }
        }
        match masks.iter_mut().find(|(e, _)| *e == effect) {
            Some((_, all)) => all.add_sat(&mask),
            None => masks.push((effect, mask)),
        }
    }
    let shallow = if wet == 0 { 1.0 } else { 1.0 - deep as f32 / wet as f32 };
    let murky = if wet == 0 { 0.0 } else { murk as f32 / wet as f32 };
    let fishless = if wet == 0 { 0.0 } else { cold as f32 / wet as f32 };
    let still = if wet == 0 { 0.0 } else { calm as f32 / wet as f32 };
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
                            mask.set(i, j, 0);
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
                        mask.set(i, j, 0);
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
                        mask.set(i, j, 0);
                    }
                }
            }
        }
        // Валун посреди воды — препятствие: под ним дыра по видимой части картинки.
        // На течении «Wild Canvas» даёт у такой дыры бурун с носа и след за кормой —
        // вода обтекает камень.
        if effect == "water" {
            let wet: Vec<&map::Prop> = map.props.iter().filter(|p| map::rock_by_name(&p.asset) && mask.get((p.pos[0] * res) as isize, (p.pos[1] * res) as isize) > 0).collect();
            for p in wet {
                let v = seen(&p.asset).unwrap_or([-0.4, -0.4, 0.4, 0.4]);
                let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
                // Эллипс чуть внутри видимой рамки: край камня неровный.
                let (cx, cy) = ((v[0] + v[2]) / 2.0 * w, (v[1] + v[3]) / 2.0 * h);
                let (rx, ry) = ((v[2] - v[0]) / 2.0 * w * 0.8, (v[3] - v[1]) / 2.0 * h * 0.8);
                let (s, c) = p.rot.to_radians().sin_cos();
                let r = rx.max(ry);
                let o = [p.pos[0] + cx * c - cy * s, p.pos[1] + cx * s + cy * c];
                for j in (((o[1] - r) * res).floor().max(0.0) as usize)..(((o[1] + r) * res).ceil().max(0.0) as usize).min(mask.h) {
                    for i in (((o[0] - r) * res).floor().max(0.0) as usize)..(((o[0] + r) * res).ceil().max(0.0) as usize).min(mask.w) {
                        let (dx, dy) = ((i as f32 + 0.5) / res - o[0], (j as f32 + 0.5) / res - o[1]);
                        // В оси картинки: поворот назад.
                        let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
                        if (lx / rx).powi(2) + (ly / ry).powi(2) <= 1.0 {
                            mask.set(i, j, 0);
                        }
                    }
                }
            }
        }
        // Участки течения — своими областями: канал бежит, а гавань рядом стоит.
        // Шов между ними прятать под мост или ворота шлюза: у края области пена.
        let mut parts: Vec<(mask::Mask, Option<(f32, f32)>)> = Vec::new();
        if effect == "water" {
            for f in map.flows.iter().filter(|f| f.points.len() >= 3) {
                let mut ring = f.points.clone();
                ring.push(ring[0]);
                let mut part = mask.clone();
                let mut any = false;
                mask.for_each_nonzero(|i, j, _| {
                    if inside([(i as f32 + 0.5) / res, (j as f32 + 0.5) / res], &ring) {
                        any = true;
                    } else {
                        part.set(i, j, 0);
                    }
                });
                if any {
                    part.for_each_nonzero(|i, j, _| mask.set(i, j, 0));
                    parts.push((part, Some((f.current.clamp(0.0, 3.0), f.drift.rem_euclid(360.0)))));
                }
            }
        }
        parts.push((mask, None));
        for (mask, flow) in parts {
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
            out.push(Zone { effect, loops: tagged, place: String::new(), shallow, murky, fishless, still, flow });
        }
    }
    out
}

/// Вырез под мостом в клетках: по видимой рамке картинки (доли холста от середины),
/// а без неё — чуть внутрь холста. Куски моста нарисованы до самых краёв холста,
/// и вырезы соседних кусков смыкаются.
fn bridge_cut(p: &map::Prop, visible: Option<[f32; 4]>) -> Vec<[f32; 2]> {
    let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
    let [l, t, r, b] = visible.unwrap_or([-0.45, -0.42, 0.45, 0.42]);
    let (c, s) = (p.rot.to_radians().cos(), p.rot.to_radians().sin());
    let corner = |x: f32, y: f32| [p.pos[0] + x * w * c - y * h * s, p.pos[1] + x * w * s + y * h * c];
    vec![corner(l, t), corner(r, t), corner(r, b), corner(l, b), corner(l, t)]
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
    /// Рисунок окна на карте (подоконник): по нему вид окна в 3D.
    pub sill: Option<String>,
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
    pub color: String,
    /// Анимация Foundry: «torch», «flame» и т. п.; пусто — горит ровно.
    pub animation: String,
    /// Плотность цвета, способ окраски, мягкость края и высветление ([`crate::map::Glow::look`]).
    pub look: (f32, u8, f32, f32),
    /// Скорость и сила анимации ([`crate::map::Glow::tempo`]).
    pub tempo: (u8, u8),
}

/// Огни предметов этажа по порядку: встроенный по ключу, чужой по имени файла, либо выбранный
/// рукой; поверх вида света — свой свет предмета (радиус, цвет, анимация).
pub fn prop_lights(map: &Map) -> Vec<Light> {
    map.props
        .iter()
        .filter_map(|p| {
            let s = p.shine()?;
            Some(Light { at: p.pos, bright: s.bright, dim: s.dim, color: s.color, animation: s.animation, look: s.look, tempo: s.tempo })
        })
        .collect()
}

/// Стены карты для VTT: края комнат и нарисованные, без проёмов, сшитые в
/// ломаные и упрощённые — Foundry медленнеет от тысяч мелких отрезков.
pub fn layout(map: &Map, room_edges: &[Segment]) -> Layout {
    layout_seen(map, room_edges, &|_| None)
}

/// То же, но вырез под мостом — по нарисованному: `seen` даёт видимую рамку
/// картинки (`library::visible_box`, доли холста от середины). Без неё — 90 % ширины
/// и 84 % длины холста, а у моста из кусков на стыках тогда проходила бы вода.
pub fn layout_seen(map: &Map, room_edges: &[Segment], seen: &dyn Fn(&str) -> Option<[f32; 4]>) -> Layout {
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
            sill: if p.kind == PortalKind::Window { p.asset.clone() } else { None },
        })
        .collect();
    let lights = prop_lights(map);
    let mut zones = zones(map, room_edges, seen);
    // Костёр под открытым небом — маленькая область: пламя и дым рисует модуль.
    for p in map.props.iter().filter(|p| p.asset.strip_prefix(crate::library::BUILTIN) == Some("campfire")) {
        let r = 0.45 * p.size[0] * p.scale;
        let round: Vec<[f32; 2]> = (0..=12).map(|i| {
            let a = i as f32 / 12.0 * std::f32::consts::TAU;
            [p.pos[0] + r * a.cos(), p.pos[1] + r * a.sin()]
        }).collect();
        zones.push(Zone { effect: "fire", loops: vec![(round, false)], place: String::new(), shallow: 1.0, murky: 0.0, fishless: 0.0, still: 0.0, flow: None });
    }
    // Встроенный фонтан: эффект «Wild Canvas» по его трём чашам — струя, перелив, круги, журчание.
    for p in map.props.iter().filter(|p| p.asset.strip_prefix(crate::library::BUILTIN) == Some("fountain")) {
        let r = 0.36 * p.size[0] * p.scale;
        let round: Vec<[f32; 2]> = (0..=24).map(|i| {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            [p.pos[0] + r * a.cos(), p.pos[1] + r * a.sin()]
        }).collect();
        zones.push(Zone { effect: "fountain", loops: vec![(round, false)], place: String::new(), shallow: 1.0, murky: 0.0, fishless: 0.0, still: 0.0, flow: None });
    }
    // Мост нарисован в фоне — под ним вырез, чтобы вода и рыба не шли поверх настила.
    for p in map.props.iter().filter(|p| map::bridge_by_name(&p.asset)) {
        zones.push(Zone { effect: "cut", loops: vec![(bridge_cut(p, seen(&p.asset)), false)], place: String::new(), shallow: 1.0, murky: 0.0, fishless: 0.0, still: 0.0, flow: None });
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
            zones.push(Zone { effect: "cut", loops: vec![(rect, false)], place: String::new(), shallow: 1.0, murky: 0.0, fishless: 0.0, still: 0.0, flow: None });
        }
    }
    // Комнаты — область со звуком внутри: в таверне слышна таверна, во дворе — лес.
    if !map.indoor.is_empty() && !map.rooms.is_empty() {
        let loops: Vec<Vec<[f32; 2]>> = mask::join(mask::contour(&map.rooms)).into_iter().map(|l| simplify(&l, true, 0.05)).filter(|l| l.len() >= 4).collect();
        let tagged: Vec<_> = loops.iter().map(|l| (l.clone(), loops.iter().filter(|o| !std::ptr::eq(*o, l) && inside(l[0], o)).count() % 2 == 1)).collect();
        if !tagged.is_empty() {
            zones.push(Zone { effect: "place", loops: tagged, place: map.indoor.clone(), shallow: 1.0, murky: 0.0, fishless: 0.0, still: 0.0, flow: None });
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
    let frame = |p: &map::Prop| {
        let (s, c) = p.rot.to_radians().sin_cos();
        let (hw, hh) = (p.size[0] * p.scale / 2.0, p.size[1] * p.scale / 2.0);
        let (ex, ey) = (hw * c.abs() + hh * s.abs(), hw * s.abs() + hh * c.abs());
        [p.pos[0] - ex, p.pos[1] - ey, p.pos[0] + ex, p.pos[1] + ey]
    };
    let passages = map.props.iter().filter(|p| map::passage_by_name(&p.asset)).map(frame).collect();
    let stairs = map.props.iter().filter(|p| crate::ground3d::flight_by_name(&p.asset)).map(frame).collect();
    Layout { walls, low, cliffs, portals, lights, zones, tents, passages, stairs }
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

/// Отрезки, обрезанные по прямоугольнику `r` (клетки) и сдвинутые на `shift` (Лианг —
/// Барски). Стены комнат района — из всей карты: своя маска района обрывается по краю, и
/// по её контуру у края встали бы стены, которых на карте нет.
pub fn clip_segments(segs: &[Segment], r: [f32; 4], shift: [f32; 2]) -> Vec<Segment> {
    let mut out = Vec::new();
    for s in segs {
        let (p, q) = (s.0, s.1);
        let d = [q[0] - p[0], q[1] - p[1]];
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut keep = true;
        for (num, den) in [(p[0] - r[0], -d[0]), (r[2] - p[0], d[0]), (p[1] - r[1], -d[1]), (r[3] - p[1], d[1])] {
            if den.abs() < 1e-9 {
                if num < 0.0 {
                    keep = false;
                    break;
                }
            } else {
                let t = num / den;
                if den < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if !keep || t0 > t1 || (t1 - t0) * (d[0].abs() + d[1].abs()) < 1e-6 {
            continue;
        }
        let at = |t: f32| [p[0] + d[0] * t + shift[0], p[1] + d[1] * t + shift[1]];
        out.push((at(t0), at(t1)));
    }
    out
}

/// Ломаные, обрезанные по прямоугольнику `r`: выход за край рвёт ломаную на куски.
fn clip_lines(lines: &[Vec<[f32; 2]>], r: [f32; 4]) -> Vec<Vec<[f32; 2]>> {
    let mut out = Vec::new();
    for line in lines {
        let mut cur: Vec<[f32; 2]> = Vec::new();
        for w in line.windows(2) {
            match clip_segments(&[(w[0], w[1])], r, [0.0, 0.0]).first() {
                Some(&(a, b)) if cur.last().is_some_and(|l| (l[0] - a[0]).abs() < 1e-4 && (l[1] - a[1]).abs() < 1e-4) => cur.push(b),
                Some(&(a, b)) => {
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur = vec![a, b];
                }
                None => {
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
            }
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    out
}

impl Layout {
    /// Раскладка района — в его пределах `w`×`h` клеток. Обрезанная карта берёт стену,
    /// дверь и палатку, если они задевают край, целиком: без обрезки стена тянулась бы за
    /// сцену на клетки, а дверь за краем висела бы значком над пустотой. Дверь остаётся,
    /// если её середина в районе.
    pub fn clip_to(&mut self, w: f32, h: f32) {
        let r = [0.0, 0.0, w, h];
        self.walls = clip_lines(&self.walls, r);
        self.low = clip_lines(&self.low, r);
        self.cliffs = clip_lines(&self.cliffs, r);
        self.tents = clip_segments(&self.tents, r, [0.0, 0.0]);
        const EPS: f32 = 1e-3;
        self.portals.retain(|p| {
            let m = [(p.seg.0[0] + p.seg.1[0]) / 2.0, (p.seg.0[1] + p.seg.1[1]) / 2.0];
            (-EPS..=w + EPS).contains(&m[0]) && (-EPS..=h + EPS).contains(&m[1])
        });
    }
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

/// WebP без потерь: карты нормалей и ORM — в сжатии с потерями их каналы перемешиваются.
pub fn webp_lossless(img: &image::RgbaImage) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = image::codecs::webp::WebPEncoder::new_lossless(&mut out).encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8);
    out
}

/// WebP картинки карты или `None`, если libwebp не справился. У Красной площади
/// (16 128 точек по стороне, миллион блоков 16×16) переполнялся первый раздел
/// кадра (`VP8_ENC_ERROR_PARTITION0_OVERFLOW`), и `encode` ронял программу. Влезает
/// с одним сегментом и `partition_limit` 100: 73 МБ за 14 с против 99 МБ JPEG
/// (с четырьмя сегментами — то же переполнение). Большую сразу так, без пробы. Фон
/// King's Cross (16 000 × 16 256) не влезает и так, ни при каком `method`: около
/// миллиона блоков — предел формата, ему JPEG.
#[cfg(not(target_arch = "wasm32"))]
pub fn try_webp(img: &image::RgbaImage, quality: f32) -> Option<Vec<u8>> {
    let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height());
    let config = |big: bool| {
        let mut c = webp::WebPConfig::new().ok()?;
        c.quality = quality;
        // Прозрачность — своим потоком рядом с цветом и без предсказания: она без
        // потерь, и предсказание у слоёв стен (сплошь 0 и 255) давало тот же размер
        // в полтора раза дольше — 33,5 с против 21 у King's Cross.
        c.thread_level = 1;
        c.alpha_compression = 1;
        c.alpha_filtering = 0;
        if big {
            c.segments = 1;
            c.partition_limit = 100;
        }
        Some(c)
    };
    if img.width().max(img.height()) <= 8192
        && let Ok(m) = enc.encode_advanced(&config(false)?)
    {
        return Some(m.to_vec());
    }
    enc.encode_advanced(&config(true)?).ok().map(|m| m.to_vec())
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
/// Концы нити `[x0, y0, x1, y1]` (доли картинки от середины) у картинки, отражённой слева направо.
pub fn mirror_ends(e: [f32; 4]) -> [f32; 4] {
    [-e[0], e[1], -e[2], e[3]]
}

pub fn canopy_tile(src: String, center: [f32; 2], size: [f32; 2], rot: f32, px: u32) -> RoofTile {
    let (w, h) = ((size[0] * px as f32).round() as i64, (size[1] * px as f32).round() as i64);
    let (x, y) = ((center[0] * px as f32).round() as i64 - w / 2, (center[1] * px as f32).round() as i64 - h / 2);
    RoofTile { src, rect: [x, y, w, h], rotation: rot, fade: true, leaves: true, wind: 1.0, ..Default::default() }
}

/// Путь вагонетки для «Wild Canvas»: точки в точках сцены (первая — депо, где
/// она скрыта), на сколько точек предмет отстаёт от головы сцепки и груз ли он
/// (руда в вагонетке): груз виден только по пути туда — у конца его выгружают.
#[derive(Clone, Debug, PartialEq)]
pub struct Rail {
    pub path: Vec<[f32; 2]>,
    pub lag: f32,
    pub cargo: bool,
}

/// На сколько градусов и в какую сторону поворачивать кран с осью `pivot` (доли
/// картинки, стрела — вверх по ней): стрела не должна заходить на дома — комнаты
/// и крыши. Пробуем обе стороны шагом 3° до 45° и берём ту, где свободнее; плюс —
/// по часовой. Свободно меньше 10° или стрела уже над домом — не поворачивать.
pub fn slew_arc(map: &Map, p: &map::Prop, pivot: [f32; 2]) -> Option<f32> {
    let (w, h) = (p.size[0] * p.scale, p.size[1] * p.scale);
    let (s, c) = p.rot.to_radians().sin_cos();
    let local = [(pivot[0] - 0.5) * w, (pivot[1] - 0.5) * h];
    let o = [p.pos[0] + local[0] * c - local[1] * s, p.pos[1] + local[0] * s + local[1] * c];
    // От края поворотной платформы до конца стрелы.
    let (from, reach) = (0.45 * w, pivot[1] * h * 0.97);
    let res = mask::RES as f32;
    let house = |q: [f32; 2]| {
        map.rooms.get((q[0] * res).floor() as isize, (q[1] * res).floor() as isize) > 0 || map.roofs.iter().any(|r| r.contains(q)) || map.outline_roofs.iter().any(|r| r.contains(q))
    };
    let free = |a: f32| {
        let (ts, tc) = (p.rot + a).to_radians().sin_cos();
        let (dir, side) = ([ts, -tc], [tc, ts]);
        let n = ((reach - from) / 0.25).ceil().max(1.0) as i32;
        (0..=n).all(|i| {
            let r = from + (reach - from) * i as f32 / n as f32;
            [-0.4f32, 0.0, 0.4].iter().all(|d| !house([o[0] + dir[0] * r + side[0] * d, o[1] + dir[1] * r + side[1] * d]))
        })
    };
    if !free(0.0) {
        return None;
    }
    let side = |sign: f32| {
        let mut best = 0.0;
        let mut a = 3.0;
        while a <= 45.0 + 1e-3 && free(sign * a) {
            best = a;
            a += 3.0;
        }
        best
    };
    let (cw, ccw) = (side(1.0), side(-1.0));
    let (arc, sign) = if cw >= ccw { (cw, 1.0) } else { (ccw, -1.0) };
    (arc >= 10.0).then_some(sign * arc)
}

/// Едет ли предмет по пути вагонеток: вагонетка, вагон или груз в них
/// ([`map::rides_by_name`]) на линии пути (не дальше 0,4 клетки). Всё, что едет по
/// одному пути, — сцепка: голова дальше всех от депо, `lag` — отставание от неё
/// вдоль пути. Точки — в точках сцены (`px` на клетку).
pub fn rail_ride(map: &Map, p: &map::Prop, px: u32) -> Option<Rail> {
    if !map::rides_by_name(&p.asset) {
        return None;
    }
    // Где на пути точка: длина вдоль пути до ближайшего места, если рядом.
    let along = |route: &[[f32; 2]], q: [f32; 2]| -> Option<f32> {
        let (mut acc, mut best) = (0.0f32, None::<(f32, f32)>);
        for w in route.windows(2) {
            let d = [w[1][0] - w[0][0], w[1][1] - w[0][1]];
            let len = d[0].hypot(d[1]);
            if len < 1e-6 {
                continue;
            }
            let u = (((q[0] - w[0][0]) * d[0] + (q[1] - w[0][1]) * d[1]) / (len * len)).clamp(0.0, 1.0);
            let off = (w[0][0] + d[0] * u - q[0]).hypot(w[0][1] + d[1] * u - q[1]);
            if off <= 0.4 && best.is_none_or(|(b, _)| off < b) {
                best = Some((off, acc + u * len));
            }
            acc += len;
        }
        best.map(|b| b.1)
    };
    let k = px as f32;
    map.rails.iter().filter(|r| r.len() >= 2).find_map(|route| {
        let s = along(route, p.pos)?;
        let head = map.props.iter().filter(|q| map::rides_by_name(&q.asset)).filter_map(|q| along(route, q.pos)).fold(s, f32::max);
        Some(Rail { path: route.iter().map(|v| [v[0] * k, v[1] * k]).collect(), lag: (head - s) * k, cargo: crate::library::label(&p.asset).to_lowercase().contains("_fill") })
    })
}

/// Крыша, уже лежащая картинкой рядом со сценой: путь и место в пикселях сцены.
#[derive(Default)]
pub struct RoofTile {
    pub src: String,
    pub rect: [i64; 4],
    /// Поворот в градусах вокруг середины.
    pub rotation: f32,
    /// Картинка отражена слева направо (`texture.scaleX` −1): одна картинка на ассет.
    pub flip: bool,
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
    /// Раскладка своего механизма (JSON из `live.json` папки ассетов): уходит в пометку плитки
    /// рядом с `machine` — «Wild Canvas» оживляет по ней и ассет, которого не знает по имени.
    pub layout: Option<String>,
    /// Сливы на крыше (вшиты в картинку): середина в точках сцены и поворот —
    /// «Wild Canvas» пускает по ним воду в дождь.
    pub scuppers: Vec<[f32; 3]>,
    /// Дымоходы на крыше (вшиты в картинку): середина и сторона в точках сцены —
    /// «Wild Canvas» пускает над ними дым, который гаснет вместе с крышей.
    pub chimneys: Vec<[f32; 3]>,
    /// Предмет карты своим тайлом: на полу под токенами, закреплён; `above` —
    /// над крышами (слой «над крышами»).
    pub item: bool,
    pub above: bool,
    /// Парит («Wild Canvas»): дирижабль плывёт, покачивается, под ним тень.
    pub hover: bool,
    /// Лежит на воде («Wild Canvas» слегка покачивает): лист бумаги, поплавок.
    pub afloat: bool,
    /// Гамак («Wild Canvas» качает): концы-крюки в долях картинки от середины.
    pub swing: Option<[f32; 4]>,
    /// Поворотный кран («Wild Canvas» поворачивает): ось в долях картинки и на
    /// сколько градусов (знак — сторона, плюс — по часовой).
    pub slew: Option<[f32; 3]>,
    /// Едет по пути вагонеток («Wild Canvas»).
    pub rail: Option<Rail>,
    /// Подкраска картинки (умножением).
    pub tint: Option<[u8; 3]>,
    /// Тень от солнца рисует «Wild Canvas» (в картинке её нет): параметры — у сцены.
    pub shadow: bool,
    /// Куст: на земле под токенами, не гаснет — он по колено, а не над головой.
    pub ground: bool,
    /// Край крыши в точках сцены, двор — `true`: по нему встаёт область-поверхность
    /// Foundry (см. [`surface_behavior`]), а тайл гаснет не целиком над токеном, а
    /// внутри открытой поверхности (режим SURFACE). Пусто — тайл гаснет по-старому.
    pub surface: Vec<(Vec<[f32; 2]>, bool)>,
    /// Контур крыши OSM в точках сцены, двор — `true` ([`crate::roofs::outline`]).
    /// В тайл он уходит долями тайла: Мастер подвинет или растянет тайл — контур
    /// поедет с ним. Без контура «Wild Canvas» берёт весь прямоугольник тайла.
    pub outline: Vec<(Vec<[f32; 2]>, bool)>,
    /// 3D-модель предмета (`.glb` рядом со сценой) и его высота в клетках: в 3D-виде (3D Canvas)
    /// тайл — эта модель, растянутая в его рамку; в 2D — прежняя картинка.
    pub model: Option<(String, f32)>,
    /// Предмет стоит на другом (подсвечник на столе): на столько футов над полом этажа.
    pub rest: f32,
    /// Модель крыши (`model`): её низ над полом этажа, клетки. Тайл крыши остаётся на верху
    /// этажа (в 2D он над токенами и створками дверей), модель 3D Canvas сдвигает вниз, на стены.
    pub model_base: Option<f32>,
    /// В 3D-виде не виден (лестница над маршем этажа ниже: там сам марш), в 2D — как есть.
    pub hide3d: bool,
}

/// Сколько потоков брать на картинку: у программы — по ядрам (не больше восьми), в
/// браузере потоков нет.
pub fn threads() -> usize {
    if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8) }
}

/// Строки картинки полосами по потокам: `f(первая строка полосы, её точки)`; `row` —
/// байт в строке.
pub fn par_bands(buf: &mut [u8], row: usize, f: impl Fn(usize, &mut [u8]) + Sync) {
    let rows = buf.len() / row.max(1);
    let n = threads();
    if n <= 1 || rows < 64 {
        f(0, buf);
        return;
    }
    let band = rows.div_ceil(n);
    std::thread::scope(|s| {
        for (i, chunk) in buf.chunks_mut(band * row).enumerate() {
            let f = &f;
            s.spawn(move || f(i * band, chunk));
        }
    });
}

/// `f` над каждым из `items` потоками, ответы — по порядку.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = threads().min(items.len());
    if n <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let jobs: Vec<_> = (0..n)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
    });
    out.sort_by_key(|o| o.0);
    out.into_iter().map(|o| o.1).collect()
}

/// Впечатать предмет в полосу картинки шириной `width` (для Universal VTT, где тайлов нет):
/// строки `rows`, полоса — точки RGB этих строк; середина и размер в точках картинки,
/// поворот по часовой, подкраска умножением. Полосами картинку делят потоки.
#[allow(clippy::too_many_arguments)]
pub fn stamp_rows(dst: &mut [u8], width: u32, rows: std::ops::Range<u32>, src: &image::RgbaImage, center: [f32; 2], size: [f32; 2], rot: f32, tint: Option<[u8; 3]>) {
    let (s, c) = rot.to_radians().sin_cos();
    let r = (size[0].hypot(size[1]) / 2.0).ceil() + 1.0;
    let (x0, y0) = ((center[0] - r).floor().max(0.0) as u32, ((center[1] - r).floor().max(0.0) as u32).max(rows.start));
    let (x1, y1) = (((center[0] + r).ceil() as u32).min(width), ((center[1] + r).ceil().max(0.0) as u32).min(rows.end));
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
            let at = (((y - rows.start) * width + x) * 3) as usize;
            let d = &mut dst[at..at + 3];
            for k in 0..3 {
                d[k] = (px[k] * t[k] * a + d[k] as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Прозрачный слой из двух снимков одного и того же — на чёрном и на белом:
/// прозрачность точки — по разнице снимков, цвет — с чёрного. Так стены с их
/// мягкой тенью снимаются с холста отдельно от пола.
pub fn layer_from_pair(black: &image::RgbImage, white: &image::RgbImage) -> image::RgbaImage {
    let w = black.width() as usize;
    let mut out = image::RgbaImage::new(black.width(), black.height());
    let (bs, ws) = (black.as_raw(), white.as_raw());
    par_bands(&mut out, w * 4, |y0, band| {
        let n = band.len() / 4;
        let (b, wh) = (&bs[y0 * w * 3..(y0 * w + n) * 3], &ws[y0 * w * 3..(y0 * w + n) * 3]);
        for ((o, b), wv) in band.chunks_exact_mut(4).zip(b.chunks_exact(3)).zip(wh.chunks_exact(3)) {
            let d = (0..3).map(|k| wv[k] as i32 - b[k] as i32).sum::<i32>() as f32 / 3.0;
            let a = (255.0 - d).clamp(0.0, 255.0);
            // Единица-другая — шум округления снимка, а не тень.
            if a < 2.5 {
                continue;
            }
            let c = |k: usize| (b[k] as f32 * 255.0 / a).round().clamp(0.0, 255.0) as u8;
            o.copy_from_slice(&[c(0), c(1), c(2), a.round() as u8]);
        }
    });
    out
}

/// Фон верхнего этажа из двух снимков — на чёрной и на белой пустоте: где на этаже
/// ничего не нарисовано, фон прозрачен, и в Foundry сквозь него виден этаж ниже.
/// `None` — пустоты нет: фон сплошной, как у первого этажа.
pub fn floor_from_pair(black: &image::RgbImage, white: &image::RgbImage) -> Option<image::RgbaImage> {
    let mut out = image::RgbaImage::new(black.width(), black.height());
    let mut holes = false;
    for ((o, b), w) in out.pixels_mut().zip(black.pixels()).zip(white.pixels()) {
        let d = (0..3).map(|k| w[k] as i32 - b[k] as i32).sum::<i32>() as f32 / 3.0;
        let a = (255.0 - d).clamp(0.0, 255.0);
        // Почти непрозрачное — шум округления снимка: пол сплошной, цвет как есть.
        if a >= 250.0 {
            *o = image::Rgba([b[0], b[1], b[2], 255]);
            continue;
        }
        holes = true;
        if a < 2.5 {
            continue;
        }
        let c = |k: usize| (b[k] as f32 * 255.0 / a).round().clamp(0.0, 255.0) as u8;
        *o = image::Rgba([c(0), c(1), c(2), a.round() as u8]);
    }
    holes.then_some(out)
}

/// Слой на сплошном цвете: картинка этажа для Universal VTT, где прозрачности нет.
pub fn over_color(layer: &image::RgbaImage, color: [u8; 3]) -> image::RgbImage {
    let mut out = image::RgbImage::new(layer.width(), layer.height());
    for (o, p) in out.pixels_mut().zip(layer.pixels()) {
        let a = p[3] as f32 / 255.0;
        *o = image::Rgb([0, 1, 2].map(|k| (p[k] as f32 * a + color[k] as f32 * (1.0 - a)).round() as u8));
    }
    out
}

/// Слой поверх картинки того же размера: пустота и дыры верхнего этажа показывают
/// этаж ниже (в Universal VTT уровней нет — каждый этаж одной картинкой). Другой
/// размер — серая пустота, как прежде.
pub fn over_image(layer: &image::RgbaImage, under: &image::RgbImage) -> image::RgbImage {
    if layer.dimensions() != under.dimensions() {
        return over_color(layer, [38, 38, 38]);
    }
    let mut out = under.clone();
    for (o, p) in out.pixels_mut().zip(layer.pixels()) {
        let a = p[3] as f32 / 255.0;
        *o = image::Rgb([0, 1, 2].map(|k| (p[k] as f32 * a + o[k] as f32 * (1.0 - a)).round() as u8));
    }
    out
}

/// Прозрачный слой во всю сцену — файлом: WebP, а не вышел — PNG. Расширение — вторым.
pub fn layer_bytes(img: &image::RgbaImage, quality: f32) -> (Vec<u8>, &'static str) {
    if let Some(bytes) = try_webp(img, quality) {
        return (bytes, "webp");
    }
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    (out.into_inner(), "png")
}

/// Где у этажа сплошной пол: сетка по фону с прозрачностью, шаг — в точках сцены.
/// Порог тот же, что у ядра (`alphaThreshold` 0,75).
pub struct Solid {
    step: u32,
    w: u32,
    h: u32,
    cells: Vec<bool>,
}

impl Solid {
    pub fn of(img: &image::RgbaImage) -> Self {
        let step = 8;
        let (w, h) = (img.width().div_ceil(step), img.height().div_ceil(step));
        let at = |x: u32, y: u32| img.get_pixel((x * step + step / 2).min(img.width() - 1), (y * step + step / 2).min(img.height() - 1))[3] >= 191;
        let cells = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| at(x, y)).collect();
        Self { step, w, h, cells }
    }

    /// Контуры сплошного пола в точках сцены, дыра — `true`. Мелочь — щели решётки
    /// люка, шов по кромке борта — отбрасывается: дыра меньше клетки `px`, островок
    /// меньше её четверти.
    pub fn loops(&self, px: u32) -> Vec<(Vec<[f32; 2]>, bool)> {
        let mask = mask::Mask::from_vec(self.w as usize, self.h as usize, &self.cells.iter().map(|&c| if c { 255 } else { 0 }).collect::<Vec<u8>>());
        // Одна «клетка» контура маски — `RES` отсчётов сетки.
        let k = (mask::RES as u32 * self.step) as f32;
        let loops: Vec<Vec<[f32; 2]>> = mask::join(mask::contour(&mask)).into_iter().map(|l| simplify(&l, true, 0.2)).filter(|l| l.len() >= 4).collect();
        let area = |l: &[[f32; 2]]| (0..l.len() - 1).map(|i| l[i][0] * l[i + 1][1] - l[i + 1][0] * l[i][1]).sum::<f32>().abs() / 2.0 * k * k;
        let cell = (px * px) as f32;
        loops
            .iter()
            .filter_map(|l| {
                let hole = loops.iter().filter(|o| !std::ptr::eq(*o, l) && inside(l[0], o)).count() % 2 == 1;
                (area(l) >= if hole { cell } else { cell / 4.0 }).then(|| (l[..l.len() - 1].iter().map(|v| [v[0] * k, v[1] * k]).collect(), hole))
            })
            .collect()
    }

    /// Доля прямоугольника `[x, y, ширина, высота]` (точки сцены) над сплошным полом.
    pub fn share(&self, rect: [i64; 4]) -> f32 {
        let s = self.step as i64;
        let (ax, ay) = (rect[0].div_euclid(s), rect[1].div_euclid(s));
        let (bx, by) = ((rect[0] + rect[2] - 1).div_euclid(s) + 1, (rect[1] + rect[3] - 1).div_euclid(s) + 1);
        let all = ((bx - ax) * (by - ay)).max(1);
        let mut hit = 0;
        for y in ay.max(0)..by.min(self.h as i64) {
            for x in ax.max(0)..bx.min(self.w as i64) {
                hit += self.cells[(y * self.w as i64 + x) as usize] as i64;
            }
        }
        hit as f32 / all as f32
    }
}

/// Слои этажа сверх фона.
#[derive(Default)]
pub struct LevelLayers {
    /// Слой стен (с их тенью, окнами, косяками и слоем «над стенами») картинкой во
    /// всю сцену — `foreground` уровня: ядро держит его на верхе этажа, над токенами
    /// и под тайлами крыш, а створки дверей рисует прямо под ним.
    pub foreground: Option<String>,
    /// У фона этажа есть пустота (он с прозрачностью), сквозь неё видны этажи ниже:
    /// где его пол сплошной. Пусто — фон без дыр.
    pub solid: Option<Solid>,
}

/// Рамка нарисованного в слое: x, y, ширина, высота; слой пуст — `None`.
pub fn opaque_box(img: &image::RgbaImage, min_alpha: u8) -> Option<[u32; 4]> {
    let (mut lo, mut hi) = ([u32::MAX; 2], [0u32; 2]);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] >= min_alpha {
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
        }
    }
    (lo[0] <= hi[0]).then(|| [lo[0], lo[1], hi[0] - lo[0] + 1, hi[1] - lo[1] + 1])
}

/// Рамка куска этажа: где нарисовано хоть в одном из его слоёв (фон, стены), с запасом
/// `pad` точек и в границах картинки `size`. Верхний этаж — стена по гребню, обсерватория
/// над башней — занимает малую долю сцены, а слой во всю сцену держит в видеопамяти
/// сотни мегабайт у каждого игрока. Рамка одна на фон и стены: уровень Foundry 14
/// кладёт их по общим `textures`. Пусто во всех слоях — `None`.
pub fn level_piece_box(boxes: &[Option<[u32; 4]>], size: [u32; 2], pad: u32) -> Option<[u32; 4]> {
    let (mut lo, mut hi) = ([u32::MAX; 2], [0u32; 2]);
    for b in boxes.iter().flatten() {
        lo = [lo[0].min(b[0]), lo[1].min(b[1])];
        hi = [hi[0].max(b[0] + b[2]), hi[1].max(b[1] + b[3])];
    }
    if lo[0] >= hi[0] || lo[1] >= hi[1] {
        return None;
    }
    let (x0, y0) = (lo[0].saturating_sub(pad), lo[1].saturating_sub(pad));
    let (x1, y1) = ((hi[0] + pad).min(size[0]), (hi[1] + pad).min(size[1]));
    Some([x0, y0, x1 - x0, y1 - y0])
}

/// `textures` уровня для куска `piece` (x, y, ширина, высота в точках картинки размером
/// `image`) в сцене размером `scene` точек. Ядро ставит слой уровня от середины сцены и
/// растягивает его на всю сцену (`fit: fill`), потом умножает на `scaleX`/`scaleY`; с
/// якорем в левом верхнем углу и сдвигом от середины кусок встаёт на своё место в
/// натуральную величину.
pub fn level_piece_textures(scene: [u32; 2], image: [u32; 2], piece: [u32; 4]) -> Value {
    let (kx, ky) = (scene[0] as f64 / image[0].max(1) as f64, scene[1] as f64 / image[1].max(1) as f64);
    json!({
        "anchorX": 0, "anchorY": 0,
        "offsetX": (piece[0] as f64 * kx - scene[0] as f64 / 2.0).round() as i64,
        "offsetY": (piece[1] as f64 * ky - scene[1] as f64 / 2.0).round() as i64,
        "fit": "fill",
        "scaleX": piece[2] as f64 / image[0].max(1) as f64,
        "scaleY": piece[3] as f64 / image[1].max(1) as f64,
        "rotation": 0
    })
}

/// Слой поверх картинки: левый верхний угол слоя — в точке `at`.
pub fn stamp_layer(dst: &mut image::RgbImage, src: &image::RgbaImage, at: [u32; 2]) {
    for (x, y, p) in src.enumerate_pixels() {
        let (dx, dy) = (x + at[0], y + at[1]);
        if p[3] == 0 || dx >= dst.width() || dy >= dst.height() {
            continue;
        }
        let a = p[3] as f32 / 255.0;
        let d = dst.get_pixel_mut(dx, dy);
        for k in 0..3 {
            d[k] = (p[k] as f32 * a + d[k] as f32 * (1.0 - a)).round() as u8;
        }
    }
}

/// Предмет вместе со своей тенью — одной картинкой для тайла: Мастер сдвинет или
/// уберёт тайл в Foundry — тень уйдёт с ним, а не останется пятном на полу.
/// `cells` — размер предмета в клетках, `rot` — его поворот: тень в картинке
/// развёрнута навстречу, тайл повернётся вместе с ней. Тень та же, что в
/// редакторе: размытый силуэт, сдвинутый от света, слоями ([`map::Shadows::layers`]).
/// Возвращает картинку с полями под тень и во сколько раз она шире и выше.
pub fn with_shadow(img: &image::RgbaImage, cells: [f32; 2], rot: f32, sh: &map::Shadows) -> (image::RgbaImage, [f32; 2]) {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let ppc = [w as f32 / cells[0].max(1e-3), h as f32 / cells[1].max(1e-3)];
    let (s, c) = (-rot).to_radians().sin_cos();
    // Слой: сдвиг в осях картинки (клетки), размытие (клетки), плотность.
    let layers: Vec<([f32; 2], f32, f32)> = sh
        .layers()
        .iter()
        .map(|l| {
            let o = l.offset();
            ([o[0] * c - o[1] * s, o[0] * s + o[1] * c], (l.blur * 10.0).round().clamp(1.0, 10.0) / 10.0, l.opacity)
        })
        .collect();
    let blur = layers.iter().map(|l| l.1).fold(0.0, f32::max);
    let reach = |k: usize| layers.iter().map(|l| l.0[k].abs()).fold(0.0, f32::max) + 1.6 * blur;
    let (px, py) = ((reach(0) * ppc[0]).ceil() as u32, (reach(1) * ppc[1]).ceil() as u32);
    let (ow, oh) = (w + 2 * px, h + 2 * py);
    // Тень мягкая: считается в 32 точки на клетку.
    let k = [(32.0 / ppc[0]).min(1.0), (32.0 / ppc[1]).min(1.0)];
    let (gw, gh) = ((ow as f32 * k[0]).ceil() as usize + 1, (oh as f32 * k[1]).ceil() as usize + 1);
    let mut light = vec![1.0f32; gw * gh];
    let mut a = vec![0.0f32; gw * gh];
    let mut tmp = vec![0.0f32; gw * gh];
    for (o, b, opacity) in &layers {
        for j in 0..gh {
            for i in 0..gw {
                let (x, y) = ((i as f32 + 0.5) / k[0] - px as f32 - o[0] * ppc[0], (j as f32 + 0.5) / k[1] - py as f32 - o[1] * ppc[1]);
                a[j * gw + i] = if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 { 0.0 } else { img.get_pixel(x as u32, y as u32).0[3] as f32 / 255.0 };
            }
        }
        // Три прохода ящика — почти по Гауссу, как у тени в редакторе.
        let r = [((b * ppc[0] * k[0]) / 2.0).round().max(1.0) as i64, ((b * ppc[1] * k[1]) / 2.0).round().max(1.0) as i64];
        for _ in 0..3 {
            for j in 0..gh {
                for i in 0..gw as i64 {
                    let (lo, hi) = ((i - r[0]).max(0), (i + r[0]).min(gw as i64 - 1));
                    tmp[j * gw + i as usize] = (lo..=hi).map(|q| a[j * gw + q as usize]).sum::<f32>() / (2 * r[0] + 1) as f32;
                }
            }
            for j in 0..gh as i64 {
                let (lo, hi) = ((j - r[1]).max(0), (j + r[1]).min(gh as i64 - 1));
                for i in 0..gw {
                    a[j as usize * gw + i] = (lo..=hi).map(|q| tmp[q as usize * gw + i]).sum::<f32>() / (2 * r[1] + 1) as f32;
                }
            }
        }
        for (t, v) in light.iter_mut().zip(&a) {
            *t *= 1.0 - (v * opacity).clamp(0.0, 1.0);
        }
    }
    let mut out = image::RgbaImage::new(ow, oh);
    for y in 0..oh {
        for x in 0..ow {
            let (fx, fy) = (((x as f32 + 0.5) * k[0] - 0.5).max(0.0), ((y as f32 + 0.5) * k[1] - 0.5).max(0.0));
            let (ix, iy) = ((fx as usize).min(gw - 2), (fy as usize).min(gh - 2));
            let (tx, ty) = ((fx - ix as f32).min(1.0), (fy - iy as f32).min(1.0));
            let g = |i: usize, j: usize| light[j * gw + i];
            let lit = (g(ix, iy) * (1.0 - tx) + g(ix + 1, iy) * tx) * (1.0 - ty) + (g(ix, iy + 1) * (1.0 - tx) + g(ix + 1, iy + 1) * tx) * ty;
            let shade = 1.0 - lit;
            let p = if x >= px && y >= py && x < px + w && y < py + h { img.get_pixel(x - px, y - py).0 } else { [0; 4] };
            let pa = p[3] as f32 / 255.0;
            let oa = pa + shade * (1.0 - pa);
            if oa <= 0.002 {
                continue;
            }
            // Тень чёрная: цвет — только от предмета.
            let col = |v: u8| (v as f32 * pa / oa).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, image::Rgba([col(p[0]), col(p[1]), col(p[2]), (oa * 255.0).round() as u8]));
        }
    }
    (out, [ow as f32 / w as f32, oh as f32 / h as f32])
}

/// Поведение области «поверхность» (ядро Foundry 14): сквозь неё по вертикали не
/// проходят свет, движение и взгляд. С `occlusion` она ещё и открывается — над
/// фишкой под ней, под мышью и по Alt («подсветить объекты») — и прячет внутри себя
/// тайлы с режимом SURFACE на своей высоте и выше: так крыша уходит и без фишки в доме.
fn surface_behavior(n: usize, occlusion: bool) -> Value {
    json!({"_id": doc_id('s', n), "type": "defineSurface", "system": {"placement": "bottom", "light": true, "move": true, "sight": true, "sound": false, "occlusion": occlusion}})
}

/// Кольца в точках сцены — фигурами области.
fn polygons(rings: &[(Vec<[f32; 2]>, bool)]) -> Vec<Value> {
    rings.iter().filter(|(r, _)| r.len() >= 3).map(|(r, hole)| json!({"type": "polygon", "points": r.iter().flat_map(|v| [v[0].round() as i64, v[1].round() as i64]).collect::<Vec<_>>(), "hole": hole})).collect()
}

/// Низ и верх этажа `k`, футы. Один этаж — уровень Foundry по умолчанию, 0 и 20;
/// у нескольких — 10·k и 10·k + 9 (зачем зазор — см. [`foundry_layered`]).
fn level_span(k: usize, multi: bool) -> (i64, i64) {
    if multi { (k as i64 * 10, k as i64 * 10 + 9) } else { (0, 20) }
}

/// Id уровня: первый — уровень по умолчанию Foundry, остальные свои, 16 знаков.
pub fn level_id(k: usize) -> String {
    if k == 0 { LEVEL_ID.to_owned() } else { format!("kartografLevel{k:02}") }
}

/// Сцена с этажами без слоёв стен и без пустоты в фоне (см. [`foundry_layered`]).
#[cfg(test)]
pub fn foundry_levels(map: &Map, name: &str, levels: &[SceneLevel], px: u32, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    foundry_layered(map, name, levels, &[], px, leaves)
}

/// Сцена с этажами: каждый — уровень Foundry 14 со своей картинкой; стены и огни
/// этажа видны только на нём. Один этаж — как раньше, без высот и привязки к
/// уровню. `layers` — слои этажей по порядку: стены уходят в `foreground` уровня;
/// у верхнего этажа с пустотой в фоне видны этажи ниже (`visibility.levels`), а их
/// тайлы входят и в него — ядро показывает только тайлы просматриваемого уровня.
///
/// Этаж `k` занимает высоты от 10·k до 10·k + 9: между его верхом и полом
/// следующего — фут зазора. Ядро сортирует по высоте, потом по слою (картинки
/// уровней ниже тайлов): крыша нижнего этажа на его верхе лежит выше его стен и
/// створок, но ниже фона верхнего — пол второго этажа сам закрывает всё, что под ним.
pub fn foundry_layered(map: &Map, name: &str, levels: &[SceneLevel], layers: &[LevelLayers], px: u32, leaves: &std::collections::HashMap<String, LeafFile>) -> Value {
    let multi = levels.len() > 1;
    let solid = |k: usize| layers.get(k).and_then(|l| l.solid.as_ref());
    let open = |k: usize| solid(k).is_some();
    // Что видно сквозь пустоту этажа: этажи ниже, до первого сплошного включительно.
    let sees: Vec<Vec<usize>> = (0..levels.len())
        .map(|k| {
            let mut out = Vec::new();
            if open(k) {
                for j in (0..k).rev() {
                    out.push(j);
                    if !open(j) {
                        break;
                    }
                }
            }
            out
        })
        .collect();
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
            // Огонь стоит на полу своего этажа: с нуля он светил бы из-под перекрытия.
            if multi {
                l["elevation"] = json!(level_span(k, true).0);
            }
        }
        // Крыша — тайл выше токенов этажа: гаснет, когда под ней токен,
        // и не пускает свет и дождь. Высота выше дверей: ниже 20 они просвечивают сквозь крышу.
        // `x`, `y` — середина, режим 1 — затухание всего тайла. Без `_stats` сцены (см. ниже)
        // миграции сдвинули бы тайл на полразмера и обнулили `occlusion.modes`.
        // Высоты — от границ этажа: низ `b`, верх `t` (один этаж — 0 и 20, как у
        // уровня Foundry по умолчанию). Предметы и кусты — на `b`, под токенами;
        // крыши и кроны — на `t`: над токенами этажа, над его стенами (`foreground`
        // уровня) и створками дверей — ядро рисует их на верхе этажа слоем ниже тайлов.
        let (b, t) = level_span(k, multi);
        // Над крышами: у одного этажа с запасом, у нескольких — в зазоре под полом следующего.
        let over = if multi { json!(t as f64 + 0.5) } else { json!(t + 5) };
        for r in &lv.roofs {
            let [x, y, w, h] = r.rect;
            let mut tile = json!({
                "_id": doc_id('t', tiles.len()), "texture": {"src": r.src, "anchorX": 0.5, "anchorY": 0.5}, "x": x + w / 2, "y": y + h / 2, "width": w, "height": h,
                "elevation": if r.low { (b + 10).min(t) } else { t }, "sort": tiles.len(), "rotation": r.rotation,
                "occlusion": {"modes": if r.fade { json!([1]) } else { json!([]) }, "alpha": 0},
                "restrictions": {"light": r.shelter, "weather": r.shelter}
            });
            if r.flip {
                tile["texture"]["scaleX"] = json!(-1);
            }
            // Предметы карты, паутина и сундук лежат под токенами и закреплены.
            if r.item || r.living.is_some() || r.web.is_some() || r.chest.is_some() {
                tile["locked"] = json!(true);
                if let Some([cr, cg, cb]) = r.tint {
                    tile["texture"]["tint"] = json!(format!("#{cr:02x}{cg:02x}{cb:02x}"));
                }
            }
            if r.item && r.living.is_none() && r.web.is_none() && r.chest.is_none() {
                // Над крышами — выше крыши этажа, иначе на полу (или на том, на чём стоит).
                tile["elevation"] = if r.above { over.clone() } else if r.rest > 0.0 { json!(b as f64 + (r.rest as f64 * 10.0).round() / 10.0) } else { json!(b) };
                tile["occlusion"]["modes"] = json!([]);
                if r.hover {
                    tile["flags"] = json!({LIVE_MODULE: {"hover": 1}});
                }
                if r.afloat {
                    tile["flags"][LIVE_MODULE]["afloat"] = json!(1);
                }
                if let Some(e) = r.swing {
                    tile["flags"][LIVE_MODULE]["swing"] = json!(e.map(|v| (v as f64 * 1000.0).round() / 1000.0));
                }
                if let Some([ax, ay, arc]) = r.slew {
                    tile["flags"][LIVE_MODULE]["slew"] = json!([(ax as f64 * 1000.0).round() / 1000.0, (ay as f64 * 1000.0).round() / 1000.0, arc.round() as i64]);
                }
                if let Some(rail) = &r.rail {
                    let path: Vec<[i64; 2]> = rail.path.iter().map(|v| [v[0].round() as i64, v[1].round() as i64]).collect();
                    tile["flags"][LIVE_MODULE]["rail"] = json!({"path": path, "lag": rail.lag.round() as i64, "cargo": rail.cargo});
                }
                if r.shadow {
                    tile["flags"][LIVE_MODULE]["shadow"] = json!(1);
                }
            } else if let Some((key, value)) = &r.living {
                // Живой предмет под токенами: колонка играет сразу. Дирижабль над
                // крышами так и остаётся над ними — винты крутятся там.
                tile["elevation"] = if r.above { over.clone() } else { json!(b) };
                tile["occlusion"]["modes"] = json!([]);
                tile["flags"] = json!({LIVE_MODULE: {*key: value}});
                if let Some(layout) = r.layout.as_deref().and_then(|l| serde_json::from_str::<Value>(l).ok()) {
                    tile["flags"][LIVE_MODULE]["layout"] = layout;
                }
                if r.hover {
                    tile["flags"][LIVE_MODULE]["hover"] = json!(1);
                }
            } else if r.web.is_some() || r.chest.is_some() {
                tile["elevation"] = json!(b);
                tile["occlusion"]["modes"] = json!([]);
                tile["flags"] = match &r.chest {
                    Some((open, empty)) => json!({OWN_MODULE: {"chest": {"open": open, "empty": empty}}}),
                    None => match r.web.flatten() {
                        Some(e) => json!({OWN_MODULE: {"web": e.map(|v| (v as f64 * 1000.0).round() / 1000.0)}}),
                        None => json!({OWN_MODULE: {"web": true}}),
                    },
                };
                if r.shadow {
                    tile["flags"][LIVE_MODULE]["shadow"] = json!(1);
                }
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
            if r.ground {
                tile["elevation"] = json!(b);
                tile["occlusion"]["modes"] = json!([]);
            }
            if let Some((src, tall)) = &r.model {
                // В 3D-виде — модель в рамке тайла. Нетвёрдая: фишки ходят мимо мебели, как в 2D,
                // а не взбираются на неё.
                tile["flags"][CANVAS3D] = json!({"model3d": src, "fillType": "stretch", "depth": (tall * px as f32).round().max(1.0), "collision": false});
                // Крыша: тайл — на верху этажа, модель — ниже, на стенах. Сдвиг 3D Canvas — в точках сцены.
                if let Some(base) = r.model_base {
                    let elevation = tile["elevation"].as_f64().unwrap_or(t as f64);
                    let ft = b as f64 + base as f64 * 5.0 - elevation;
                    tile["flags"][CANVAS3D]["offset"] = json!({"x": 0, "y": (ft / 5.0 * px as f64).round() + 0.0, "z": 0});
                }
            }
            if r.hide3d {
                tile["flags"][CANVAS3D]["transparency"] = json!(0);
            }
            if !r.scuppers.is_empty() {
                // Сливы крыши: середина и поворот; размер — клетка сцены.
                let s: Vec<Value> = r.scuppers.iter().map(|v| json!([v[0].round() as i64, v[1].round() as i64, v[2].round() as i64])).collect();
                tile["flags"][LIVE_MODULE]["scuppers"] = json!(s);
                tile["flags"][LIVE_MODULE]["scupperSize"] = json!(px);
            }
            if !r.chimneys.is_empty() {
                let c: Vec<Value> = r.chimneys.iter().map(|v| json!([v[0].round() as i64, v[1].round() as i64, v[2].round() as i64])).collect();
                tile["flags"][LIVE_MODULE]["chimneys"] = json!(c);
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
            // Крыша — поверхность: область по её краю на высоте тайла. Тайл с режимом
            // SURFACE прячется внутри открытой поверхности — над фишкой в доме, под мышью
            // и по Alt; сквозь крышу не пролететь, не светит и не видно.
            if !r.surface.is_empty() {
                tile["occlusion"]["modes"] = json!([2]);
                let high = tile["elevation"].clone();
                regions.push(tag(json!({
                    "_id": doc_id('r', regions.len()), "name": "Крыша", "color": "#8a6a44", "shapes": polygons(&r.surface),
                    "elevation": {"bottom": high, "top": high}, "behaviors": [surface_behavior(regions.len(), true)]
                })));
            }
            // Тайл входит и в этажи выше, сквозь пустоту которых виден его этаж: ядро
            // показывает тайлы только просматриваемого уровня. Он весь ниже их пола —
            // что под полом, то им и закрыто. Крыша, что больше чем наполовину под
            // полом этажа выше, — «крыша» первого этажа под вторым, нужная лишь при
            // взгляде снизу: сверху её нет, иначе из-под пола торчал бы её свес.
            if multi {
                let hidden = |up: usize| r.shelter && solid(up).is_some_and(|s| s.share(r.rect) > 0.5);
                let on: Vec<String> = std::iter::once(k).chain((k + 1..levels.len()).filter(|&up| sees[up].contains(&k) && !hidden(up))).map(level_id).collect();
                tile["levels"] = json!(on);
            }
            tiles.push(tile);
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
            // Глубокая вода — гавань, море: блики на дне гаснут по доле глубины.
            if z.effect == "water" && z.shallow < 0.95 {
                region["flags"][LIVE_MODULE]["caustic"] = json!(((z.shallow.max(0.15) as f64) * 100.0).round() / 100.0);
            }
            // Корабль на ходу: вода уходит мимо борта.
            // Участок со своим течением — его скорость (и стоячая вода, если 0).
            let (current, drift) = z.flow.unwrap_or((map.current, map.drift));
            // Мутная вода (болото, тёмный пруд): ни бликов на дне, ни рыб, а стоячая — и
            // без пены у берега. Бегущая и мутная пенится у камня и за кормой.
            if z.effect == "water" && z.murky >= 0.5 {
                if current <= 0.0 {
                    region["flags"][LIVE_MODULE]["foam"] = json!(0);
                }
                region["flags"][LIVE_MODULE]["caustic"] = json!(0);
                region["flags"][LIVE_MODULE]["fish"] = json!(false);
            }
            // Криожидкость, охладитель, кислота: рыб нет, вид «Без рыбы» (номер 3 в Wild Canvas).
            if z.effect == "water" && z.fishless >= 0.5 {
                region["flags"][LIVE_MODULE]["fish"] = json!(false);
                region["flags"][LIVE_MODULE]["fishKind"] = json!(3);
            }
            // Стоячая вода (разлив в коридоре, лужа): без зыби и пены у стен, свет на дне
            // слабый и медленный — зеркало, по которому еле ходят блики. С течением — как обычная.
            if z.effect == "water" && z.still >= 0.5 && current <= 0.0 {
                let flags = &mut region["flags"][LIVE_MODULE];
                let caustic = flags["caustic"].as_f64().unwrap_or(1.0);
                flags["ripple"] = json!(STILL_WATER.0);
                flags["foam"] = json!(0);
                flags["caustic"] = json!((caustic * STILL_WATER.1 * 100.0).round() / 100.0);
                flags["glint"] = json!(STILL_WATER.2);
                flags["speed"] = json!(STILL_WATER.3);
                // Стоячая вода не журчит: звук области «вода» в «Wild Canvas» — ручей.
                flags["sound"] = json!("off");
            }
            if z.effect == "water" && current > 0.0 {
                region["flags"][LIVE_MODULE]["current"] = json!((current as f64 * 10.0).round() / 10.0);
                region["flags"][LIVE_MODULE]["drift"] = json!(drift.rem_euclid(360.0).round() as i64);
            }
            regions.push(tag(region));
        }
        let mut doc = json!({"_id": level_id(k), "name": lv.name, "background": {"src": lv.image_src, "color": "#999999"}});
        // Верх этажа — явно и у одного этажа: у уровня без него верх бесконечный, и
        // `foreground` со стенами лёг бы выше крыш.
        let (bottom, top) = level_span(k, multi);
        doc["elevation"] = json!({"bottom": bottom, "top": top});
        if multi {
            doc["sort"] = json!(k as i64 * 100);
        }
        // Стены — родным слоем уровня: на верхе этажа, над токенами, под крышами.
        if let Some(src) = layers.get(k).and_then(|l| l.foreground.as_ref()) {
            doc["foreground"] = json!({"src": src, "alphaThreshold": 0.75});
        }
        // Фон с пустотой: сплошное — где прозрачность не ниже порога; этажи ниже видны.
        if !sees[k].is_empty() {
            doc["background"]["alphaThreshold"] = json!(0.75);
            let mut below = sees[k].clone();
            below.sort_unstable();
            doc["visibility"] = json!({"levels": below.into_iter().map(level_id).collect::<Vec<_>>()});
        }
        docs.push(doc);
    }
    // Перекрытия: пол этажа выше — поверхность. Нижнему этажу она потолок на его
    // верхе, верхнему — пол на его низе; у каждого уровня своя область: открывшись
    // на верхнем из-за фишки внизу, общая вырезала бы его пол. Лестнице поверхность
    // не мешает: смена уровня в ядре её не проверяет.
    for k in 1..levels.len() {
        let rings = match solid(k) {
            Some(s) => s.loops(px),
            None => {
                let (w, h) = ((map.width * px) as f32, (map.height * px) as f32);
                vec![(vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]], false)]
            }
        };
        if rings.is_empty() {
            continue;
        }
        for (level, high, name, occlusion) in [(k - 1, level_span(k - 1, true).1, format!("Потолок: {}", levels[k].name), true), (k, level_span(k, true).0, format!("Пол: {}", levels[k].name), false)] {
            regions.push(json!({
                "_id": doc_id('r', regions.len()), "name": name, "color": "#7a6a5a", "shapes": polygons(&rings),
                "elevation": {"bottom": high, "top": high}, "levels": [level_id(level)], "behaviors": [surface_behavior(regions.len(), occlusion)]
            }));
        }
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
                let mut region = json!({
                    "_id": doc_id('r', regions.len()), "name": format!("{} ↔ {}", levels[k].name, levels[k + 1].name), "color": "#c9a24a",
                    "shapes": [{"type": "polygon", "points": pts, "hole": false}],
                    "elevation": {"bottom": null, "top": null},
                    "levels": [level_id(k), level_id(k + 1)],
                    "behaviors": [{"_id": doc_id('c', regions.len()), "type": "changeLevel", "system": {"movementActions": []}}]
                });
                // Лестница маршем: в 3D Canvas по ней идут ногами — модуль не спрашивает «Move Up?»,
                // а меняет уровень, когда фишка дошла до пола этажа выше.
                if levels[k].layout.stairs.contains(a) {
                    region["flags"] = json!({"burn-kartograf": {"stairs": true}});
                }
                regions.push(region);
            }
        }
    }
    let (global_light, darkness) = scene_light(map);
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
        // Пещера и подземелье — во тьме, светят их огни. Свет сцены, выбранный на карте,
        // главнее: станция в полумраке видна вся, а не пятнами у ламп.
        "environment": {"darknessLevel": darkness, "globalLight": {"enabled": global_light}},
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
            "config": {"bright": l.bright * 5.0, "dim": l.dim * 5.0, "color": l.color,
                       "alpha": (l.look.0 as f64 * 100.0).round() / 100.0, "coloration": l.look.1, "attenuation": (l.look.2 as f64 * 100.0).round() / 100.0, "luminosity": (l.look.3 as f64 * 100.0).round() / 100.0,
                       // Пустая анимация — свет горит ровно (светильник станции).
                       "animation": {"type": if l.animation.is_empty() { Value::Null } else { json!(l.animation) }, "speed": l.tempo.0, "intensity": l.tempo.1}},
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

/// Путь картинки из общей папки ассетов: от корня данных Foundry, если выгрузка в них;
/// иначе `../assets/<файл>` — от папки карты (модуль Картографа разворачивает его от
/// папки, где лежит .json сцены).
pub fn asset_src(image: &std::path::Path) -> String {
    let s = foundry_src(image);
    if s.contains('/') { s } else { format!("../assets/{s}") }
}

/// Общая папка ассетов: `kartograf/assets` (в данных Foundry и в модуле — у ближайшей
/// папки `kartograf` по пути), иначе `assets` рядом с папкой выгрузки.
pub fn shared_dir(dir: &std::path::Path) -> std::path::PathBuf {
    if let Some(k) = dir.ancestors().find(|a| a.file_name().is_some_and(|n| n == "kartograf")) {
        return k.join("assets");
    }
    dir.parent().map_or_else(|| dir.join("assets"), |p| p.join("assets"))
}

/// Имя файла латиницей: Foundry кодирует кириллицу в имени файла длинными `%D0%9A…`.
/// Русские буквы — транслитом, остальное, кроме букв, цифр и `-`, — `_`.
pub fn ascii_name(s: &str) -> String {
    let lat = |c: char| -> Option<&'static str> {
        Some(match c {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo", 'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y",
            'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f",
            'х' => "kh", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ъ' => "", 'ы' => "y", 'ь' => "", 'э' => "e", 'ю' => "yu", 'я' => "ya",
            'і' => "i", 'ї' => "yi", 'є' => "ye", 'ґ' => "g",
            _ => return None,
        })
    };
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
            continue;
        }
        let low = ch.to_lowercase().next().unwrap_or(ch);
        match lat(low) {
            Some(t) if low != ch => {
                let mut cs = t.chars();
                if let Some(f) = cs.next() {
                    out.push(f.to_ascii_uppercase());
                    out.extend(cs);
                }
            }
            Some(t) => out.push_str(t),
            None => out.push('_'),
        }
    }
    let mut tidy = String::new();
    for ch in out.chars() {
        if !(ch == '_' && (tidy.is_empty() || tidy.ends_with('_'))) {
            tidy.push(ch);
        }
    }
    let tidy = tidy.trim_end_matches('_').to_owned();
    if tidy.is_empty() { "map".to_owned() } else { tidy }
}

/// Имя картинки ассета в общей папке — по самому ассету, латиницей: имя файла без
/// расширения; у перекрашенного (`tint:rrggbb:…`) — `_c` и тон.
pub fn asset_stem(key: &str) -> String {
    let mut rest = key;
    let mut tones = String::new();
    while let Some(r) = rest.strip_prefix(crate::library::TINT) {
        let (hex, tail) = r.split_once(':').unwrap_or((r, ""));
        tones.push_str(hex);
        rest = tail;
    }
    let file = rest.rsplit(['/', '\\', ':']).next().unwrap_or(rest);
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    let mut name = ascii_name(stem);
    if !tones.is_empty() {
        name.push_str("_c");
        name.push_str(&ascii_name(&tones));
    }
    name
}

/// Солнце для теней «Wild Canvas» — в пометку сцены: куда (градусы по часовой от
/// «вправо»), на сколько клеток, размытие в клетках, плотность, затухание.
pub fn shadow_flags(sh: &map::Shadows) -> Value {
    let r = |v: f32| (v as f64 * 1000.0).round() / 1000.0;
    json!({"angle": r(sh.angle), "dist": r(sh.dist), "blur": r(sh.blur), "opacity": r(sh.opacity), "fade": r(sh.fade)})
}

/// Модуль 3D-вида Foundry.
pub const CANVAS3D: &str = "levels-3d-preview";

/// Тайл 3D Canvas с 3D-моделью карты (`crate::scene3d`): рамка тайла — рамка модели в точках
/// сцены (3D Canvas растягивает модель по тайлу), низ — высота низа модели в футах; тайл на
/// всех `levels` этажах сцены. В 3D-виде 3D Canvas не рисует свои стены, двери и фон: их
/// заменяет модель, а значки дверей остаются — двери открывают как обычно.
pub fn model_tile(scene: &mut Value, src: &str, bounds: [[f32; 3]; 2], px: u32, levels: usize) {
    let [lo, hi] = bounds;
    let p = px as f64;
    let pts = |cells: f32| (cells as f64 * p).round();
    // `x`/`y` — середина рамки (якорь тайла 0,5, как у тайлов предметов и крыш): там и начало
    // модели. Рамка тайла в Foundry тогда совпадает с моделью в 3D Canvas.
    let tile = json!({
        "_id": doc_id('m', 0),
        "x": pts((lo[0] + hi[0]) / 2.0), "y": pts((lo[2] + hi[2]) / 2.0),
        "width": pts(hi[0] - lo[0]), "height": pts(hi[2] - lo[2]),
        "elevation": (lo[1] as f64 * 5.0 * 100.0).round() / 100.0,
        "levels": (0..levels.max(1)).map(level_id).collect::<Vec<_>>(),
        "flags": {CANVAS3D: {"model3d": src, "fillType": "stretch", "depth": pts(hi[1] - lo[1]), "collision": true}},
    });
    match scene["tiles"].as_array_mut() {
        Some(t) => t.push(tile),
        None => scene["tiles"] = json!([tile]),
    }
    for (k, v) in [("showSceneWalls", false), ("showSceneDoors", false), ("renderBackground", false)] {
        scene["flags"][CANVAS3D][k] = v.into();
    }
}

/// Метка содержимого файла: 8 знаков хэша его байтов (FNV-1a).
pub fn content_tag(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (h >> 32) as u32 ^ h as u32)
}

/// Дописать к путям картинок в сцене `?v=` и метку содержимого: всякая строка,
/// равная пути из `tags`, — в фоне, тайле, пометке сундука. Файлы выгрузки
/// названы по порядку (`_предмет_7`): убрали предмет — номера съехали, а Foundry
/// держит картинки по пути и показал бы под старым именем прежнюю: на месте
/// парусов — марс. С меткой новая картинка — новый путь; файл на диске тот же.
pub fn tag_sources(v: &mut Value, tags: &std::collections::HashMap<String, String>) {
    match v {
        Value::String(s) => {
            // Модель — без метки: 3D Canvas выбирает загрузчик по тому, что после последней
            // точки, и `x.glb?v=…` для него уже не `.glb`.
            let model = [".glb", ".gltf"].iter().any(|e| s.to_ascii_lowercase().ends_with(e));
            if let Some(t) = tags.get(s.as_str())
                && !model
            {
                *s = format!("{s}?v={t}");
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| tag_sources(x, tags)),
        Value::Object(o) => o.values_mut().for_each(|x| tag_sources(x, tags)),
        _ => {}
    }
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
    fn file_names_are_latin_and_named_by_the_asset() {
        assert_eq!(ascii_name("Криогеника"), "Kriogenika");
        assert_eq!(ascii_name("Воздушная гавань"), "Vozdushnaya_gavan");
        assert_eq!(ascii_name("Иерихон_2"), "Ierikhon_2");
        assert_eq!(ascii_name("Щит и Ёж!"), "Shchit_i_Yozh");
        assert_eq!(ascii_name("***"), "map");
        assert_eq!(asset_stem("pack:E:\\D&D\\FA\\Barrel_Wood_Dark_A_1x1.webp"), "Barrel_Wood_Dark_A_1x1");
        assert_eq!(asset_stem("C:/Карты/ассеты/objects/scifi/Cryo_Reactor_Core_A_5x5.webp"), "Cryo_Reactor_Core_A_5x5");
        assert_eq!(asset_stem("builtin:bar_counter"), "bar_counter");
        assert_eq!(asset_stem("tint:ec8859:builtin:bar_counter"), "bar_counter_cec8859");
        assert_eq!(asset_stem("objects/Барная стойка.png"), "Barnaya_stoyka");
        // Общая папка: у папки `kartograf` по пути, иначе рядом с папкой выгрузки.
        let p = std::path::Path::new;
        assert_eq!(shared_dir(p("D:/Foundry/Data/kartograf/cryo")), p("D:/Foundry/Data/kartograf/assets"));
        assert_eq!(shared_dir(p("D:/Foundry/Data/kartograf")), p("D:/Foundry/Data/kartograf/assets"));
        assert_eq!(shared_dir(p("kartograf/Kriogenika/1763")), p("kartograf/assets"));
        assert_eq!(shared_dir(p("C:/Maps/Pulsar/foundry")), p("C:/Maps/Pulsar/assets"));
        assert_eq!(asset_src(p("C:/nowhere-near-foundry/assets/X.webp")), "../assets/X.webp");
    }

    #[test]
    fn mirrored_strand_ends_swap_sides() {
        // Гамак по диагонали: левый верхний крюк становится правым верхним.
        assert_eq!(mirror_ends([-0.437, -0.42, 0.43, 0.41]), [0.437, -0.42, -0.43, 0.41]);
    }

    #[test]
    fn shadow_of_an_item_is_a_flag_not_a_picture() {
        let mut t = canopy_tile("kartograf/assets/Chair.webp".into(), [2.0, 2.0], [1.0, 1.0], 0.0, 100);
        (t.item, t.fade, t.leaves, t.shadow) = (true, false, false, true);
        let mut m = Map::new(8, 8);
        m.shadows.on = true;
        let lay = layout(&m, &m.room_edges());
        let lv = SceneLevel { name: "x".into(), image_src: "x.webp".into(), layout: &lay, roofs: vec![t] };
        let scene = foundry_layered(&m, "x", &[lv], &[LevelLayers { foreground: None, solid: None }], 100, &Default::default());
        let tile = &scene["tiles"][0];
        assert_eq!(tile["flags"][LIVE_MODULE]["shadow"], 1);
        assert_eq!((tile["width"].as_i64(), tile["height"].as_i64()), (Some(100), Some(100)), "тайл — ровно предмет, без поля под тень");
        let f = shadow_flags(&m.shadows);
        assert!(f["angle"].as_f64().is_some() && f["dist"].as_f64().is_some() && f["opacity"].as_f64().is_some());
    }

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
    fn changed_picture_gets_new_path() {
        // Тот же файл `_предмет_1`, другая картинка — другой путь в сцене; та же — тот же.
        let (sail, nest) = (content_tag(b"sail"), content_tag(b"crows nest"));
        assert_ne!(sail, nest);
        assert_eq!(sail, content_tag(b"sail"));
        assert_eq!(sail.len(), 8);
        let mut scene = json!({"name": "x", "background": {"src": "k/x_1.webp"}, "tiles": [{"texture": {"src": "k/x_1.webp"}, "flags": {"m": {"chest": ["k/x_2.webp", "k/other.webp"]}}}, {"flags": {CANVAS3D: {"model3d": "k/x_3d.glb"}}}]});
        let tags = std::collections::HashMap::from([("k/x_1.webp".to_owned(), sail.clone()), ("k/x_2.webp".to_owned(), nest.clone()), ("k/x_3d.glb".to_owned(), sail.clone())]);
        tag_sources(&mut scene, &tags);
        assert_eq!(scene["tiles"][1]["flags"][CANVAS3D]["model3d"], "k/x_3d.glb", "модель — без метки: 3D Canvas смотрит расширение");
        let want = format!("k/x_1.webp?v={sail}");
        assert_eq!(scene["background"]["src"], want.as_str());
        assert_eq!(scene["tiles"][0]["texture"]["src"], want.as_str());
        assert_eq!(scene["tiles"][0]["flags"]["m"]["chest"], json!([format!("k/x_2.webp?v={nest}"), "k/other.webp"]));
        // Foundry пропускает путь с хвостом запроса: `(\.webp)(\?.*)?$`.
        assert!(want.ends_with(&format!(".webp?v={sail}")));
    }

    #[test]
    fn hatch_over_ladder_links_two_levels() {
        // Люк на палубе над трапом в трюме — переход между уровнями; трап в стороне — нет.
        let hold = Map::new(10, 10);
        let mut deck = hold.new_level();
        let mut hold = hold;
        let prop = |name: &str, x: f32, y: f32| crate::map::Prop { asset: format!("objects/{name}"), pos: [x, y], size: [1.0, 2.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
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
    fn harbour_water_hides_the_bottom() {
        // Гавань — глубина: блики на дне почти гаснут; пруд — мелководье: как были;
        // пополам — вполсилы. Область воды одна: шва с пеной между ними нет.
        let caustic = |deep_cols: i32| {
            let mut m = Map::new(10, 10);
            let shallow = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
            let deep = m.materials.iter().position(|x| x.name == "Глубина").unwrap();
            assert!(m.materials[deep].deep() && !m.materials[shallow].deep());
            m.paint(crate::map::Layer::Floor, shallow, &crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (9, 9)));
            if deep_cols > 0 {
                m.paint(crate::map::Layer::Floor, deep, &crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (deep_cols - 1, 9)));
            }
            m.prune();
            let lay = layout(&m, &[]);
            let scene = foundry_levels(&m, "Вода", &[SceneLevel { name: "Вода".into(), image_src: "w.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
            let water: Vec<&Value> = scene["regions"].as_array().unwrap().iter().filter(|r| r["flags"][LIVE_MODULE]["effect"] == "water").collect();
            assert_eq!(water.len(), 1);
            water[0]["flags"][LIVE_MODULE]["caustic"].as_f64()
        };
        assert_eq!(caustic(0), None);
        assert_eq!(caustic(10), Some(0.15));
        assert!((caustic(5).unwrap() - 0.5).abs() < 0.05);
    }

    #[test]
    fn murky_water_has_no_surf_bottom_or_fish() {
        // Мутная вода (у FA — Water_Opaque…): у берега нет белой пены, дна и рыб не видно.
        // Чистая вода рядом — как была.
        let flags = |name: &str| {
            let mut m = Map::new(10, 10);
            let w = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
            m.materials[w].name = name.into();
            m.paint(crate::map::Layer::Floor, w, &crate::mask::cells(m.rooms.w, m.rooms.h, (2, 2), (7, 7)));
            m.prune();
            let lay = layout(&m, &[]);
            let scene = foundry_levels(&m, "Топь", &[SceneLevel { name: "Топь".into(), image_src: "w.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
            scene["regions"].as_array().unwrap().iter().find(|r| r["flags"][LIVE_MODULE]["effect"] == "water").unwrap()["flags"][LIVE_MODULE].clone()
        };
        let murky = flags("Мутная вода");
        assert_eq!((murky["foam"].as_f64(), murky["caustic"].as_f64(), murky["fish"].as_bool()), (Some(0.0), Some(0.0), Some(false)));
        let clear = flags("Мелководье");
        assert_eq!((clear.get("foam"), clear["fish"].as_bool()), (None, Some(true)));
        // Кислота: рыб нет, вид «Без рыбы», а блики на дне и пена как у чистой воды.
        let acid = flags("Кислота");
        assert_eq!((acid["fish"].as_bool(), acid["fishKind"].as_i64(), acid.get("foam"), acid.get("caustic"), acid.get("ripple")), (Some(false), Some(3), None, None, None));
        // Криожидкость — разлив на полу: без рыб и стоячая — ни зыби, ни пены у стен,
        // блики на дне слабые и медленные.
        let cryo = flags("Криожидкость");
        assert_eq!((cryo["fish"].as_bool(), cryo["fishKind"].as_i64()), (Some(false), Some(3)));
        assert_eq!((cryo["foam"].as_f64(), cryo["ripple"].as_f64(), cryo["caustic"].as_f64(), cryo["glint"].as_f64(), cryo["speed"].as_f64()), (Some(0.0), Some(0.15), Some(0.65), Some(0.3), Some(0.5)));
        // Стоячая жидкость молчит; у кислоты с её рябью звук воды остаётся.
        assert_eq!((cryo["sound"].as_str(), acid.get("sound")), (Some("off"), None));
        // Лужа чистой воды стоит так же, но рыбы в ней по имени не запрещены.
        let pool = flags("Лужа");
        assert_eq!((pool["foam"].as_f64(), pool["ripple"].as_f64(), pool.get("fishKind")), (Some(0.0), Some(0.15), None));
    }

    #[test]
    fn bridge_pieces_cut_water_end_to_end() {
        use crate::map::Prop;
        let mut m = Map::new(12, 14);
        let piece = |y: f32, rot: f32| Prop { asset: "objects/bridges/Bridge_Boardwalk_Straight_A_3x4.webp".into(), pos: [6.0, y], size: [3.0, 4.0], rot, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        m.props.push(piece(4.0, 0.0));
        m.props.push(piece(8.0, 180.0));
        // Кусок нарисован во всю длину холста, по ширине — настил со сваями.
        let seen = |_: &str| Some([-0.45, -0.5, 0.45, 0.5]);
        let lay = layout_seen(&m, &[], &seen);
        let cuts: Vec<_> = lay.zones.iter().filter(|z| z.effect == "cut").collect();
        assert_eq!(cuts.len(), 2);
        let ys = |z: &Zone| {
            let v: Vec<f32> = z.loops[0].0.iter().map(|q| q[1]).collect();
            (v.iter().cloned().fold(f32::MAX, f32::min), v.iter().cloned().fold(f32::MIN, f32::max))
        };
        let (a, b) = (ys(cuts[0]), ys(cuts[1]));
        assert!((a.0 - 2.0).abs() < 1e-4 && (b.1 - 10.0).abs() < 1e-4, "{a:?} {b:?}");
        assert!((a.1 - b.0).abs() < 1e-4, "стык без щели: {a:?} {b:?}");
        // Без картинки — как раньше, чуть внутрь холста.
        let old = layout(&m, &[]);
        let c0 = old.zones.iter().find(|z| z.effect == "cut").unwrap();
        assert!(ys(c0).0 > 2.0);
    }

    #[test]
    fn neon_floor_gets_region_and_cuts_under_furniture() {
        use crate::map::{Live, Material, Paint, Prop};
        let mut m = Map::new(10, 8);
        let key = "pack:cp|textures/terrain/cyberpunk/floors/Floor_Alloy_Grid_A.webp";
        let floor = m.material_index(Material { name: "Floor_Alloy_Grid_A".into(), paint: Paint::Texture(key.into()), vary: false, live: Live::Auto, rot: 0.0 }).unwrap();
        assert_eq!(m.materials[floor].live(), Live::Neon);
        let all = crate::mask::cells(m.rooms.w, m.rooms.h, (1, 1), (8, 6));
        m.paint(crate::map::Layer::Rooms, floor, &all);
        let prop = |asset: &str, pos: [f32; 2]| Prop { asset: asset.into(), pos, size: [2.0, 1.0], rot: 90.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
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
    fn a_prop_with_a_model_is_that_model_in_3d_and_its_picture_in_2d() {
        let map = Map::new(8, 6);
        let lay = layout(&map, &map.room_edges());
        let chair = RoofTile { item: true, model: Some(("k/chair_ab12cd34.glb".into(), 0.8)), ..canopy_tile("k/chair.webp".into(), [2.0, 2.0], [1.0, 1.0], 30.0, 100) };
        let one = [SceneLevel { name: "Дом".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: vec![chair] }];
        let walls = [LevelLayers { foreground: None, solid: None }];
        let t = &foundry_layered(&map, "Дом", &one, &walls, 100, &Default::default())["tiles"][0];
        assert_eq!(t["texture"]["src"], "k/chair.webp", "в 2D — картинка");
        assert_eq!((t["rotation"].as_f64(), t["elevation"].as_i64()), (Some(30.0), Some(0)), "на полу и повёрнут, как картинка");
        assert_eq!(t["flags"][CANVAS3D], json!({"model3d": "k/chair_ab12cd34.glb", "fillType": "stretch", "depth": 80.0, "collision": false}));
    }

    #[test]
    fn walls_go_above_furniture_and_tokens_and_bushes_stay_on_the_ground() {
        // Один этаж: низ 0, верх 20 (уровень Foundry по умолчанию). Предмет — на полу
        // под токенами, куст — на земле, крыша — на верхе этажа; стены — не тайлом, а
        // слоем `foreground` уровня: ядро держит его на верхе этажа под тайлами крыш.
        let map = Map::new(8, 6);
        let lay = layout(&map, &map.room_edges());
        let item = RoofTile { item: true, ..canopy_tile("k/chair.webp".into(), [2.0, 2.0], [1.0, 1.0], 0.0, 100) };
        let bush = RoofTile { ground: true, fade: false, wind: 0.35, ..canopy_tile("k/bush.webp".into(), [5.0, 2.0], [2.0, 2.0], 0.0, 100) };
        let eaves = vec![(vec![[100.0, 100.0], [500.0, 100.0], [500.0, 400.0], [100.0, 400.0]], false)];
        let roof = RoofTile { src: "k/roof.webp".into(), rect: [100, 100, 400, 300], shelter: true, fade: true, wind: 1.0, surface: eaves, ..Default::default() };
        let one = [SceneLevel { name: "Дом".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: vec![item, bush, roof] }];
        let walls = [LevelLayers { foreground: Some("k/walls.webp".into()), solid: None }];
        let scene = foundry_layered(&map, "Дом", &one, &walls, 100, &Default::default());
        let t = scene["tiles"].as_array().unwrap();
        assert_eq!(t.len(), 3, "стены — не тайл");
        assert_eq!((t[0]["elevation"].as_i64(), t[0]["occlusion"]["modes"].as_array().map(Vec::len)), (Some(0), Some(0)), "предмет — на полу");
        assert_eq!((t[1]["elevation"].as_i64(), t[1]["occlusion"]["modes"].as_array().map(Vec::len)), (Some(0), Some(0)), "куст — на земле и не гаснет");
        assert!(t[1]["flags"][LIVE_MODULE]["wind"].as_f64().is_some(), "куст колышется");
        assert_eq!((t[2]["elevation"].as_i64(), t[2]["occlusion"]["modes"].clone()), (Some(20), json!([2])), "крыша — на верхе этажа, прячется поверхностью");
        // Над крышей — область-поверхность на её высоте: открывается над фишкой, под
        // мышью и по Alt, не пускает свет, движение и взгляд.
        let regions = scene["regions"].as_array().unwrap();
        assert_eq!(regions.len(), 1, "один этаж: перекрытий нет, поверхность только у крыши");
        let s = &regions[0];
        assert_eq!((s["elevation"].clone(), s["shapes"][0]["points"].clone()), (json!({"bottom": 20, "top": 20}), json!([100, 100, 500, 100, 500, 400, 100, 400])));
        assert_eq!(s["behaviors"][0]["type"], "defineSurface");
        assert_eq!(s["behaviors"][0]["system"], json!({"placement": "bottom", "light": true, "move": true, "sight": true, "sound": false, "occlusion": true}));
        assert!(s.get("levels").is_none());
        let lv = &scene["levels"][0];
        assert_eq!((lv["foreground"]["src"].as_str(), lv["foreground"]["alphaThreshold"].as_f64()), (Some("k/walls.webp"), Some(0.75)));
        assert!(lv.get("visibility").is_none() && lv["background"].get("alphaThreshold").is_none(), "один этаж — фон сплошной");
        assert_eq!(lv["elevation"], json!({"bottom": 0, "top": 20}), "верх задан: стены — под крышами, а не над всем");
        // Кусок верхнего этажа: рамка по фону и стенам с запасом, в границах картинки;
        // кусок встаёт на место — левый верхний угол сдвинут от середины сцены, ширина в долях.
        let b = level_piece_box(&[Some([100, 200, 50, 40]), None, Some([140, 180, 30, 10])], [1000, 600], 4);
        assert_eq!(b, Some([96, 176, 78, 68]));
        assert_eq!(level_piece_box(&[None, None], [10, 10], 4), None);
        assert_eq!(level_piece_box(&[Some([0, 0, 10, 10])], [10, 10], 4), Some([0, 0, 10, 10]), "запас не выходит за картинку");
        let t = level_piece_textures([1000, 600], [1000, 600], [96, 176, 78, 68]);
        assert_eq!((t["anchorX"].as_i64(), t["offsetX"].as_i64(), t["offsetY"].as_i64(), t["fit"].as_str()), (Some(0), Some(-404), Some(-124), Some("fill")));
        assert!((t["scaleX"].as_f64().unwrap() - 0.078).abs() < 1e-9 && (t["scaleY"].as_f64().unwrap() - 68.0 / 600.0).abs() < 1e-9);
        // Картинка мельче сцены (выгрузка в меньшем масштабе): сдвиг — в точках сцены.
        let t = level_piece_textures([2000, 1200], [1000, 600], [100, 60, 100, 60]);
        assert_eq!((t["offsetX"].as_i64(), t["offsetY"].as_i64()), (Some(-800), Some(-480)));
        // Без слоя стен (выгрузка без тайлов) `foreground` нет.
        assert!(foundry_levels(&map, "Дом", &one, 100, &Default::default())["levels"][0].get("foreground").is_none());
        // Два этажа: у каждого свои границы — предметы на низе, крыши на верхе, стены у каждого свои.
        let two = [
            SceneLevel { name: "Низ".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: Vec::new() },
            SceneLevel { name: "Верх".into(), image_src: "k/b.webp".into(), layout: &lay, roofs: vec![RoofTile { item: true, ..canopy_tile("k/chair.webp".into(), [2.0, 2.0], [1.0, 1.0], 0.0, 100) }] },
        ];
        let walls = [LevelLayers { foreground: Some("k/w1.webp".into()), solid: None }, LevelLayers { foreground: Some("k/w2.webp".into()), solid: None }];
        let scene = foundry_layered(&map, "Дом", &two, &walls, 100, &Default::default());
        let t = scene["tiles"].as_array().unwrap();
        assert_eq!(t[0]["elevation"].as_i64(), Some(10));
        assert_eq!(t[0]["levels"], json!([scene["levels"][1]["_id"]]));
        assert_eq!(scene["levels"][1]["elevation"], json!({"bottom": 10, "top": 19}));
        // Перекрытие — двумя областями: потолок нижнего этажа (на его верхе, открывается)
        // и пол верхнего (на его низе, не открывается). Фон верхнего без пустоты — во всю сцену.
        let r = scene["regions"].as_array().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0]["levels"].clone(), r[0]["elevation"].clone(), r[0]["behaviors"][0]["system"]["occlusion"].clone()), (json!([scene["levels"][0]["_id"]]), json!({"bottom": 9, "top": 9}), json!(true)));
        assert_eq!((r[1]["levels"].clone(), r[1]["elevation"].clone(), r[1]["behaviors"][0]["system"]["occlusion"].clone()), (json!([scene["levels"][1]["_id"]]), json!({"bottom": 10, "top": 10}), json!(false)));
        assert_eq!(r[1]["shapes"][0]["points"], json!([0, 0, 800, 0, 800, 600, 0, 600]));
        assert_eq!(r[1]["behaviors"][0]["system"]["move"], true);
        let ids: std::collections::HashSet<&str> = r.iter().flat_map(|x| [x["_id"].as_str().unwrap(), x["behaviors"][0]["_id"].as_str().unwrap()]).collect();
        assert_eq!(ids.len(), 4, "id областей и поведений не повторяются");
        assert_eq!([scene["levels"][0]["foreground"]["src"].as_str(), scene["levels"][1]["foreground"]["src"].as_str()], [Some("k/w1.webp"), Some("k/w2.webp")]);
    }

    #[test]
    fn the_floor_below_shows_through_the_void_of_an_upper_floor() {
        // Второй этаж 8×6 клеток по 100 точек: пол — правая половина, слева пустота.
        let map = Map::new(8, 6);
        let lay = layout(&map, &map.room_edges());
        let mut bg = image::RgbaImage::new(800, 600);
        for (x, _, p) in bg.enumerate_pixels_mut() {
            *p = image::Rgba([90, 70, 50, if x >= 400 { 255 } else { 0 }]);
        }
        let floor = || Some(Solid::of(&bg));
        let solid = Solid::of(&bg);
        assert!(solid.share([400, 0, 400, 600]) > 0.99 && solid.share([0, 0, 400, 600]) < 0.01);
        assert!((solid.share([300, 100, 200, 200]) - 0.5).abs() < 0.03);
        assert!(solid.share([-500, -500, 100, 100]) == 0.0, "за краем сцены пола нет");
        // Контур пола — правая половина сцены одним кольцом, с точностью до шага сетки.
        let rings = solid.loops(100);
        assert_eq!(rings.len(), 1);
        let (lo, hi) = rings[0].0.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), v| ([lo[0].min(v[0]), lo[1].min(v[1])], [hi[0].max(v[0]), hi[1].max(v[1])]));
        assert!((lo[0] - 400.0).abs() <= 8.0 && lo[1].abs() <= 8.0 && (hi[0] - 800.0).abs() <= 8.0 && (hi[1] - 600.0).abs() <= 8.0 && !rings[0].1, "{lo:?} {hi:?}");
        // Щель решётки (10×150 точек) — не дыра в перекрытии; люк 150×150 — дыра.
        let mut holed = bg.clone();
        for (x, y, p) in holed.enumerate_pixels_mut() {
            if ((500..510).contains(&x) && (100..250).contains(&y)) || ((600..750).contains(&x) && (300..450).contains(&y)) {
                p[3] = 0;
            }
        }
        let rings = Solid::of(&holed).loops(100);
        assert_eq!(rings.iter().filter(|r| r.1).count(), 1, "{rings:?}");
        // Внизу: стул, крыша крыла (на треть под полом второго), крона и дирижабль над
        // крышами; наверху — стул.
        let chair = |x: f32| RoofTile { item: true, ..canopy_tile("k/chair.webp".into(), [x, 3.0], [1.0, 1.0], 0.0, 100) };
        let roof = |x: i64| RoofTile { src: "k/roof.webp".into(), rect: [x, 100, 300, 300], shelter: true, fade: true, wind: 1.0, ..Default::default() };
        let tree = canopy_tile("k/tree.webp".into(), [5.5, 3.0], [3.0, 3.0], 0.0, 100);
        let ship = RoofTile { item: true, above: true, ..canopy_tile("k/ship.webp".into(), [4.0, 3.0], [2.0, 4.0], 0.0, 100) };
        let levels = [
            SceneLevel { name: "Низ".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: vec![chair(1.5), roof(200), tree, ship] },
            SceneLevel { name: "Верх".into(), image_src: "k/b.webp".into(), layout: &lay, roofs: vec![chair(6.0)] },
        ];
        let layers = [LevelLayers::default(), LevelLayers { foreground: None, solid: floor() }];
        let scene = foundry_layered(&map, "Дом", &levels, &layers, 100, &Default::default());
        let (lo, up) = (scene["levels"][0]["_id"].clone(), scene["levels"][1]["_id"].clone());
        // Верхний уровень: фон с порогом прозрачности, нижний виден сквозь пустоту.
        assert_eq!(scene["levels"][1]["visibility"]["levels"], json!([lo]));
        assert_eq!(scene["levels"][1]["background"]["alphaThreshold"].as_f64(), Some(0.75));
        assert!(scene["levels"][0].get("visibility").is_none(), "снизу верхний этаж не виден");
        // Между верхом нижнего этажа и полом верхнего — зазор: всё нижнее, вместе с
        // крышей на его верхе и тем, что над крышами, лежит ниже фона верхнего этажа.
        assert_eq!((&scene["levels"][0]["elevation"], &scene["levels"][1]["elevation"]), (&json!({"bottom": 0, "top": 9}), &json!({"bottom": 10, "top": 19})));
        let t = scene["tiles"].as_array().unwrap();
        let high: Vec<f64> = t.iter().map(|x| x["elevation"].as_f64().unwrap()).collect();
        assert_eq!(high, vec![0.0, 9.0, 9.0, 9.5, 10.0]);
        // Тайлы нижнего этажа — в обоих уровнях: ядро показывает только тайлы
        // просматриваемого; тайл верхнего — только на нём.
        let both = json!([lo, up]);
        assert!(t[..4].iter().all(|x| x["levels"] == both), "{t:#?}");
        assert_eq!(t[4]["levels"], json!([up]));
        // «Крыша» первого этажа под полом второго сверху не показывается: она только
        // для взгляда снизу. Крона там же остаётся — пол закрывает её сам.
        let under = [
            SceneLevel { name: "Низ".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: vec![roof(450)] },
            SceneLevel { name: "Верх".into(), image_src: "k/b.webp".into(), layout: &lay, roofs: Vec::new() },
        ];
        assert_eq!(foundry_layered(&map, "Дом", &under, &layers, 100, &Default::default())["tiles"][0]["levels"], json!([lo]));
        // Фон верхнего этажа без пустоты: ничего не меняется.
        let plain = foundry_layered(&map, "Дом", &levels, &[], 100, &Default::default());
        assert!(plain["levels"][1].get("visibility").is_none() && plain["levels"][1]["background"].get("alphaThreshold").is_none());
        assert_eq!(plain["tiles"][0]["levels"], json!([lo]));
        // Три этажа: сквозь третий видны второй и — сквозь его пустоту — первый;
        // сплошной второй закрыл бы первый.
        let three = [
            SceneLevel { name: "1".into(), image_src: "k/a.webp".into(), layout: &lay, roofs: vec![chair(1.5)] },
            SceneLevel { name: "2".into(), image_src: "k/b.webp".into(), layout: &lay, roofs: Vec::new() },
            SceneLevel { name: "3".into(), image_src: "k/c.webp".into(), layout: &lay, roofs: Vec::new() },
        ];
        let open = || LevelLayers { foreground: None, solid: floor() };
        let all = foundry_layered(&map, "Дом", &three, &[LevelLayers::default(), open(), open()], 100, &Default::default());
        let ids: Vec<Value> = all["levels"].as_array().unwrap().iter().map(|l| l["_id"].clone()).collect();
        assert_eq!(all["levels"][2]["visibility"]["levels"], json!([ids[0], ids[1]]));
        assert_eq!(all["tiles"][0]["levels"], json!(ids));
        let shut = foundry_layered(&map, "Дом", &three, &[LevelLayers::default(), LevelLayers::default(), open()], 100, &Default::default());
        assert_eq!(shut["levels"][2]["visibility"]["levels"], json!([ids[1]]));
        assert_eq!(shut["tiles"][0]["levels"], json!([ids[0]]));
    }

    #[test]
    fn an_upper_floor_keeps_only_what_is_drawn_on_it() {
        // Два снимка этажа: пол (снимки равны), мягкий край в полсилы, пустота.
        let mut black = image::RgbImage::new(3, 1);
        let mut white = image::RgbImage::from_pixel(3, 1, image::Rgb([255, 255, 255]));
        for img in [&mut black, &mut white] {
            img.put_pixel(0, 0, image::Rgb([120, 90, 60]));
        }
        black.put_pixel(1, 0, image::Rgb([60, 45, 30]));
        white.put_pixel(1, 0, image::Rgb([188, 173, 158]));
        let bg = floor_from_pair(&black, &white).expect("пустота есть");
        assert_eq!(bg.get_pixel(0, 0).0, [120, 90, 60, 255]);
        let edge = bg.get_pixel(1, 0).0;
        assert!((edge[3] as i32 - 127).abs() <= 1 && (edge[0] as i32 - 120).abs() <= 2, "{edge:?}");
        assert_eq!(bg.get_pixel(2, 0).0[3], 0);
        // Шум снимка в единицу-другую дыр не делает: этаж, закрашенный целиком, — сплошной.
        let mut noisy = black.clone();
        noisy.put_pixel(2, 0, image::Rgb([40, 40, 40]));
        let mut same = noisy.clone();
        same.put_pixel(0, 0, image::Rgb([122, 92, 62]));
        assert!(floor_from_pair(&noisy, &same).is_none());
        // Для Universal VTT в пустоте — картинка этажа ниже; у самого нижнего — прежняя серая.
        let flat = over_color(&bg, [38, 38, 38]);
        assert_eq!((flat.get_pixel(0, 0).0, flat.get_pixel(2, 0).0), ([120, 90, 60], [38, 38, 38]));
        let below = image::RgbImage::from_pixel(3, 1, image::Rgb([10, 200, 30]));
        let seen = over_image(&bg, &below);
        assert_eq!((seen.get_pixel(0, 0).0, seen.get_pixel(2, 0).0), ([120, 90, 60], [10, 200, 30]), "сквозь пустоту — этаж ниже");
        let half = seen.get_pixel(1, 0).0;
        assert!((half[1] as i32 - 145).abs() <= 3, "край пополам: {half:?}");
        assert_eq!(over_image(&bg, &image::RgbImage::new(5, 1)).get_pixel(2, 0).0, [38, 38, 38], "чужой размер — серая пустота");
    }

    #[test]
    fn a_layer_comes_off_the_canvas_from_shots_on_black_and_white() {
        // Точка стены непрозрачна (снимки равны), тень — чёрная в полсилы, пустое — разница во весь белый.
        let mut black = image::RgbImage::new(4, 1);
        let mut white = image::RgbImage::from_pixel(4, 1, image::Rgb([255, 255, 255]));
        black.put_pixel(1, 0, image::Rgb([120, 90, 60]));
        white.put_pixel(1, 0, image::Rgb([120, 90, 60]));
        white.put_pixel(2, 0, image::Rgb([128, 128, 128]));
        // Полупрозрачный рыжий край стены (плотность 0,5): на чёрном вдвое темнее.
        black.put_pixel(3, 0, image::Rgb([100, 50, 0]));
        white.put_pixel(3, 0, image::Rgb([228, 178, 128]));
        let layer = layer_from_pair(&black, &white);
        assert_eq!(layer.get_pixel(0, 0).0[3], 0);
        assert_eq!(layer.get_pixel(1, 0).0, [120, 90, 60, 255]);
        assert_eq!(layer.get_pixel(2, 0).0, [0, 0, 0, 127]);
        let edge = layer.get_pixel(3, 0).0;
        assert!((edge[0] as i32 - 200).abs() <= 2 && (edge[1] as i32 - 100).abs() <= 2 && (edge[3] as i32 - 127).abs() <= 1, "{edge:?}");
        assert_eq!(opaque_box(&layer, 8), Some([1, 0, 3, 1]));
        assert_eq!(opaque_box(&image::RgbaImage::new(3, 3), 8), None);
        // Слой обратно на пол: непрозрачное заменяет, тень темнит вдвое.
        let mut floor = image::RgbImage::from_pixel(6, 1, image::Rgb([200, 200, 200]));
        stamp_layer(&mut floor, &layer, [1, 0]);
        assert_eq!(floor.get_pixel(1, 0).0, [200, 200, 200]);
        assert_eq!(floor.get_pixel(2, 0).0, [120, 90, 60]);
        assert!((floor.get_pixel(3, 0).0[0] as i32 - 100).abs() <= 2);
    }

    #[test]
    fn an_item_tile_carries_its_own_shadow() {
        // Ящик 1×1 (64 точки), тень вправо-вниз на 0,2 клетки: картинка шире, тень —
        // по ту сторону, куда падает; у повёрнутого на 180° предмета — по другую.
        let pic = image::RgbaImage::from_pixel(64, 64, image::Rgba([180, 120, 60, 255]));
        let sh = crate::map::Shadows { on: true, angle: 45.0, dist: 0.2, blur: 0.2, opacity: 0.6, fade: 0.0, ..Map::new(2, 2).shadows };
        let (out, grow) = with_shadow(&pic, [1.0, 1.0], 0.0, &sh);
        assert!(grow[0] > 1.2 && grow[1] > 1.2 && grow[0] < 2.5, "{grow:?}");
        let pad = (out.width() - 64) / 2;
        // Сам предмет не тронут и стоит посередине.
        assert_eq!(out.get_pixel(pad + 32, pad + 32).0, [180, 120, 60, 255]);
        // Тень: чёрная, полупрозрачная, справа-снизу плотнее, чем слева-сверху.
        let at = |dx: i32, dy: i32| out.get_pixel((pad as i32 + 32 + dx) as u32, (pad as i32 + 32 + dy) as u32).0;
        let (far, near) = (at(38, 38), at(-38, -38));
        assert!(far[3] > 40 && far[0] < 30, "тени справа-снизу нет: {far:?}");
        assert!(near[3] < far[3] / 3, "тень легла и против света: {near:?} и {far:?}");
        // Предмет повёрнут на 180°: тайл повернётся в Foundry, поэтому в картинке тень с другой стороны.
        let (turned, _) = with_shadow(&pic, [1.0, 1.0], 180.0, &sh);
        let tat = |dx: i32, dy: i32| turned.get_pixel((pad as i32 + 32 + dx) as u32, (pad as i32 + 32 + dy) as u32).0;
        assert!(tat(-38, -38)[3] > 40 && tat(38, 38)[3] < tat(-38, -38)[3] / 3);
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
        let mut hammock = canopy_tile("k/h.webp".into(), [4.0, 1.0], [2.0, 2.0], 45.0, 100);
        (hammock.item, hammock.swing) = (true, Some([-0.4371, -0.42, 0.43, 0.41]));
        let mut paper = canopy_tile("k/p.webp".into(), [5.0, 3.0], [1.0, 1.0], 0.0, 100);
        (paper.item, paper.afloat) = (true, true);
        // Свой механизм: имя и раскладка из `live.json` — в пометке плитки.
        let mut fan = canopy_tile("k/f.webp".into(), [1.0, 3.0], [1.0, 1.0], 0.0, 100);
        (fan.item, fan.living, fan.layout) = (true, Some(("machine", "Vent_Fan_Wall_B".to_owned())), Some(r#"{"fans":[[0.5,0.5,0.4,2.0]]}"#.to_owned()));
        assert!(crate::map::swing_by_name("objects/Hammock_Wood_Dark_Blue_A_2x2.webp") && !crate::map::swing_by_name("x/Rope_A.webp"));
        let levels = [SceneLevel { name: "Город".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![ship, balloon, hammock, paper, fan] }];
        let scene = foundry_levels(&a, "Город", &levels, 100, &Default::default());
        let t = &scene["tiles"][0];
        assert_eq!(t["elevation"], 25, "над крышами, а не на полу, как колонка");
        assert_eq!(t["flags"][LIVE_MODULE]["machine"], "Airship_Gnome_Brass_A");
        assert_eq!(t["flags"][LIVE_MODULE]["hover"], 1);
        let b = &scene["tiles"][1];
        assert_eq!((b["elevation"].as_i64(), b["flags"][LIVE_MODULE]["hover"].as_i64()), (Some(25), Some(1)));
        // Гамак качается: концы-крюки в пометке, на полу под токенами.
        let h = &scene["tiles"][2];
        assert_eq!(h["flags"][LIVE_MODULE]["swing"], json!([-0.437, -0.42, 0.43, 0.41]));
        assert_eq!(h["elevation"], 0);
        // Бумага на воде покачивается; гамак — нет.
        assert_eq!(scene["tiles"][3]["flags"][LIVE_MODULE]["afloat"], 1);
        assert!(h["flags"][LIVE_MODULE].get("afloat").is_none());
        // Свой вентилятор: имя и раскладка в пометке; у дирижабля раскладки нет — её знает модуль.
        let f = &scene["tiles"][4]["flags"][LIVE_MODULE];
        assert_eq!((f["machine"].as_str(), &f["layout"]), (Some("Vent_Fan_Wall_B"), &json!({"fans": [[0.5, 0.5, 0.4, 2.0]]})));
        assert!(t["flags"][LIVE_MODULE].get("layout").is_none());
    }

    #[test]
    fn dry_part_keeps_what_is_above_the_waterline() {
        let img = image::RgbaImage::from_pixel(4, 10, image::Rgba([200, 100, 50, 255]));
        let dry = dry_part(&img, 0.6);
        // Ватерлиния — на 0,4 высоты: строки 0–2 целы, строка 4 и ниже прозрачны.
        assert_eq!((dry.get_pixel(0, 0)[3], dry.get_pixel(3, 2)[3]), (255, 255));
        assert_eq!((dry.get_pixel(0, 5)[3], dry.get_pixel(3, 9)[3]), (0, 0));
        assert!(dry.get_pixel(0, 3)[3] > 0 && dry.get_pixel(0, 3)[3] < 255, "край смягчён");
        assert_eq!(dry.get_pixel(1, 1).0[..3], [200, 100, 50], "цвет не тронут");
        assert_eq!(dry_part(&img, 0.0), img);
    }

    #[test]
    fn alarm_light_blinks_slowly() {
        use crate::map::Prop;
        let mut m = Map::new(6, 6);
        let prop = |asset: &str, at: [f32; 2]| Prop { asset: asset.into(), pos: at, size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        m.props.push(prop("set:scifi/objects/Alarm_Light_Red_A_1x1.webp", [2.0, 2.0]));
        m.props.push(prop("pack:fa|textures/objects/Lantern_Metal_A_1x1.webp", [4.0, 4.0]));
        let lay = layout(&m, &[]);
        let scene = foundry_levels(&m, "Станция", &[SceneLevel { name: "Станция".into(), image_src: "s.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        let (alarm, lamp) = (&scene["lights"][0]["config"]["animation"], &scene["lights"][1]["config"]["animation"]);
        assert_eq!((alarm["type"].as_str(), alarm["speed"].as_i64(), alarm["intensity"].as_i64()), (Some("revolving"), Some(7), Some(8)));
        assert_eq!(scene["lights"][0]["config"]["coloration"], 2);
        assert_eq!((lamp["type"].as_str(), lamp["speed"].as_i64()), (Some("torch"), Some(3)));
        // Свой свет одной лампы: медленнее, шире и другого цвета — вид «тревога» у остальных прежний.
        m.props[0].glow = Some(crate::map::GlowTune { radius: Some(3.0), color: Some([255, 160, 0]), animation: Some("pulse".into()), speed: Some(2), intensity: Some(4) });
        let lay = layout(&m, &[]);
        let scene = foundry_levels(&m, "Станция", &[SceneLevel { name: "Станция".into(), image_src: "s.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        let own = &scene["lights"][0]["config"];
        assert_eq!((own["animation"]["type"].as_str(), own["animation"]["speed"].as_i64(), own["animation"]["intensity"].as_i64()), (Some("pulse"), Some(2), Some(4)));
        assert_eq!((own["color"].as_str(), own["dim"].as_f64()), (Some("#ffa000"), Some(15.0)));
        m.props[0].glow = Some(crate::map::GlowTune { animation: Some(String::new()), ..Default::default() });
        let lay = layout(&m, &[]);
        let scene = foundry_levels(&m, "Станция", &[SceneLevel { name: "Станция".into(), image_src: "s.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        assert!(scene["lights"][0]["config"]["animation"]["type"].is_null(), "ровный свет");
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
            SceneLevel { name: "Низ".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![RoofTile { src: "k/r.png".into(), rect: [50, 50, 300, 200], rotation: 0.0, shelter: true, fade: true, leaves: false, low: false, wind: 1.0, rope: false, web: None, chest: None, living: None, scuppers: vec![[100.0, 50.0, 0.0]], chimneys: vec![[120.0, 80.0, 40.0]], ..Default::default() }] },
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
        // Крыша — тайл над токенами первого этажа, гаснет над токеном. Высота — верх
        // своего этажа (9): ниже пола второго (10), а не +20 от низа.
        let t = &scene["tiles"][0];
        assert_eq!((t["x"].as_i64(), t["width"].as_i64(), t["elevation"].as_i64()), (Some(200), Some(300), Some(9)));
        // Крона второго этажа — на его верхе.
        assert_eq!(scene["tiles"][1]["elevation"], 19);
        // Тайл без контура поверхности гаснет по-старому, над токеном.
        assert_eq!(scene["tiles"][0]["occlusion"]["modes"], json!([1]));
        assert_eq!(t["occlusion"]["modes"][0], 1);
        assert_eq!(scene["_stats"]["coreVersion"], "14.355");
        assert_eq!(t["texture"]["anchorX"].as_f64(), Some(0.5));
        assert_eq!(t["levels"][0], lv[0]["_id"]);
        // Крону качает ветер «Wild Canvas», крышу — нет; у крыши лишь сливы для дождя.
        assert!(t["flags"][LIVE_MODULE].get("wind").is_none());
        assert_eq!(t["flags"][LIVE_MODULE]["scuppers"][0], json!([100, 50, 0]));
        // Дымоходы — рядом со сливами, метка не затирает их.
        assert_eq!(t["flags"][LIVE_MODULE]["chimneys"][0], json!([120, 80, 40]));
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
        // Свет сцены — отдельно от звука места: пещера в полумраке, станция днём, лес во тьме.
        let env = |m: &Map| foundry_levels(m, "Дом", &levels, 100, &Default::default())["environment"].clone();
        autumn.scene_light = "dim".into();
        assert_eq!((env(&autumn)["globalLight"]["enabled"].as_bool(), env(&autumn)["darknessLevel"].as_f64()), (Some(true), Some(0.5)));
        autumn.scene_light = "day".into();
        assert_eq!((env(&autumn)["globalLight"]["enabled"].as_bool(), env(&autumn)["darknessLevel"].as_f64()), (Some(true), Some(0.0)));
        autumn.ambience = "forest-autumn".into();
        autumn.scene_light = "dark".into();
        assert_eq!(env(&autumn)["globalLight"]["enabled"], false, "лес ночью — по выбору карты");
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][OWN_MODULE]["light"], "dark");
        autumn.scene_light.clear();
        autumn.ambience = "station-1".into();
        assert_eq!(env(&autumn)["globalLight"]["enabled"], false, "станция без выбора — во тьме, как подземелье");
        assert_eq!(foundry_levels(&autumn, "Дом", &levels, 100, &Default::default())["flags"][LIVE_MODULE]["ambience"], "station-1");
        autumn.ambience = "cave-1".into();
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
        assert_eq!(flags[OWN_MODULE]["origin"], json!([0, 0]));
        let mut grown = m.clone();
        grown.grow(3, 18, 0, 0);
        assert_eq!(scene_flags(&grown)[OWN_MODULE]["origin"], json!([3, 18]));
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
    fn segments_are_cut_by_the_district_and_moved_into_it() {
        let segs = vec![([0.0, 5.0], [20.0, 5.0]), ([12.0, 0.0], [12.0, 3.0]), ([30.0, 30.0], [40.0, 30.0]), ([10.0, 10.0], [14.0, 14.0])];
        let cut = clip_segments(&segs, [10.0, 4.0, 15.0, 12.0], [-10.0, -4.0]);
        assert_eq!(cut.len(), 2, "вне района — не берётся: {cut:?}");
        assert_eq!(cut[0], ([0.0, 1.0], [5.0, 1.0]), "через район — по краю");
        let ((ax, ay), (bx, by)) = ((cut[1].0[0], cut[1].0[1]), (cut[1].1[0], cut[1].1[1]));
        assert!((ax - 0.0).abs() < 1e-5 && (ay - 6.0).abs() < 1e-5 && (bx - 2.0).abs() < 1e-5 && (by - 8.0).abs() < 1e-5, "косой — до нижнего края: {cut:?}");
    }

    #[test]
    fn a_district_layout_keeps_only_what_is_inside() {
        let door = |a: [f32; 2], b: [f32; 2]| PortalOut { seg: (a, b), kind: PortalKind::Door, leaf: None, sill: None };
        let mut lay = Layout {
            tents: vec![([-2.0, 3.0], [2.0, 3.0])],
            // Через левый край, вдоль края, вне района и ломаная, дважды выходящая за верх.
            walls: vec![vec![[-6.0, 9.0], [2.0, 9.0]], vec![[0.0, 2.0], [0.0, 5.0]], vec![[-1.0, 2.0], [0.0, 2.0]], vec![[2.0, 1.0], [2.0, -1.0], [4.0, -1.0], [4.0, 1.0], [6.0, 1.0]]],
            low: vec![vec![[9.0, 20.0], [9.0, 24.0]]],
            cliffs: Vec::new(),
            // За верхним краем, за левым, на краю и через край серединой внутри.
            portals: vec![door([15.0, -1.0], [15.0, 0.0]), door([-2.0, 21.0], [0.0, 21.0]), door([26.0, 4.0], [26.0, 5.0]), door([-0.4, 7.0], [0.6, 7.0])],
            lights: Vec::new(),
            zones: Vec::new(),
            passages: Vec::new(),
            stairs: Vec::new(),
        };
        lay.clip_to(26.0, 22.0);
        assert_eq!(lay.walls, vec![vec![[0.0, 9.0], [2.0, 9.0]], vec![[0.0, 2.0], [0.0, 5.0]], vec![[2.0, 1.0], [2.0, 0.0]], vec![[4.0, 0.0], [4.0, 1.0], [6.0, 1.0]]]);
        assert_eq!(lay.low, vec![vec![[9.0, 20.0], [9.0, 22.0]]]);
        assert_eq!(lay.tents, vec![([0.0, 3.0], [2.0, 3.0])], "полог — тем же направлением");
        let kept: Vec<_> = lay.portals.iter().map(|p| p.seg).collect();
        assert_eq!(kept, vec![([26.0, 4.0], [26.0, 5.0]), ([-0.4, 7.0], [0.6, 7.0])]);
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
        m.props.push(Prop { asset: format!("{}campfire", crate::library::BUILTIN), pos: [3.0, 2.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false });
        // Чужая жаровня светит по имени, выключенный рукой костёр — нет, бочка с огнём по выбору — да.
        let other = |asset: &str, light| Prop { asset: asset.into(), pos: [5.0, 5.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
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
        let p = crate::map::Prop { asset: "x".into(), pos: [10.0, 10.0], size: [4.0, 2.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: Some(2), glow: None, sunk: None, flip: false };
        let walls = tent_walls(&p, Some(&hull));
        assert!(!walls.is_empty() && walls.len() < hull.len());
        for (a, b) in &walls {
            // Внутренность слева (y вниз), и ни одно ребро не обращено вниз, ко входу.
            let cross = (b[0] - a[0]) * (10.0 - a[1]) - (b[1] - a[1]) * (10.0 - a[0]);
            assert!(cross < 0.0);
            assert!((a[1] + b[1]) / 2.0 < 10.0 + 0.5 || (a[0] - b[0]).abs() < 0.2, "{a:?} {b:?}");
        }
    }

    fn thing(asset: &str, pos: [f32; 2], size: [f32; 2], rot: f32) -> crate::map::Prop {
        crate::map::Prop { asset: asset.into(), pos, size, rot, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false }
    }

    #[test]
    fn crane_slews_away_from_houses_and_flies_over_all() {
        let key = "objects/steampunk/Crane_Steam_Gnome_Brass_A_4x9.webp";
        let pivot = crate::map::slew_by_name(key).expect("кран поворотный");
        assert_eq!(crate::map::layer_by_name(key), crate::map::ABOVE_ROOFS, "кран над всем");
        assert_eq!(crate::map::layer_by_name("x/Crane_Wood_Dark_A_2x4.webp"), crate::map::ABOVE_ROOFS);
        assert!(crate::map::slew_by_name("x/Crane_Wood_Dark_A_2x4.webp").is_none(), "у деревянной стрелы оси нет");
        // Стрела вверх; справа сверху — дом: крутить можно только против часовой.
        let mut m = Map::new(30, 30);
        let crane = thing(key, [10.0, 15.0], [4.0, 9.0], 0.0);
        assert_eq!(slew_arc(&m, &crane, pivot), Some(45.0), "кругом пусто — по часовой");
        let house = crate::mask::cells(m.rooms.w, m.rooms.h, (12, 6), (18, 13));
        m.paint(crate::map::Layer::Rooms, 3, &house);
        assert_eq!(slew_arc(&m, &crane, pivot), Some(-45.0));
        // Дом и слева — поворот там, где свободнее, но меньше 10° не крутится.
        let left = crate::mask::cells(m.rooms.w, m.rooms.h, (4, 6), (9, 13));
        m.paint(crate::map::Layer::Rooms, 3, &left);
        assert_eq!(slew_arc(&m, &crane, pivot), None);
        // Пометка в сцене: ось и угол.
        let la = layout(&m, &[]);
        let mut t = canopy_tile("k/c.webp".into(), [10.0, 15.0], [4.0, 9.0], 0.0, 100);
        (t.item, t.above, t.slew) = (true, true, Some([pivot[0], pivot[1], -45.0]));
        let scene = foundry_levels(&m, "Порт", &[SceneLevel { name: "Порт".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![t] }], 100, &Default::default());
        let tile = &scene["tiles"][0];
        assert_eq!(tile["flags"][LIVE_MODULE]["slew"], json!([0.504, 0.779, -45]));
        assert_eq!(tile["elevation"], 25, "над крышами");
    }

    #[test]
    fn carts_on_a_rail_ride_as_one_train() {
        let mut m = Map::new(20, 24);
        m.rails = vec![vec![[5.0, 1.0], [5.0, 20.0], [9.0, 20.0]]];
        let head = thing("x/MineCart_Metal_Gray_B1_1x2.webp", [5.0, 10.0], [1.0, 2.0], 0.0);
        let ore = thing("x/Ore_Minecart_Fill_Black_C1_1x2.webp", [5.05, 10.0], [1.0, 2.0], 0.0);
        let tail = thing("x/MineCart_Metal_Gray_C1_1x2.webp", [5.0, 7.8], [1.0, 2.0], 0.0);
        let track = thing("x/Track_Dark_Straight_D1_1x5.webp", [5.0, 14.0], [1.0, 5.0], 0.0);
        let aside = thing("x/Barrel_A1_1x1.webp", [6.0, 10.0], [1.0, 1.0], 0.0);
        // Брус крепи над входом стоит прямо на пути, но не едет и не сдвигает голову.
        let beam = thing("x/Fence_Wood_Dark_B_Beam_B1_1x1.webp", [5.0, 15.0], [1.0, 1.0], 90.0);
        m.props = vec![head.clone(), ore.clone(), tail.clone(), track.clone(), aside.clone(), beam.clone()];
        let r = rail_ride(&m, &head, 100).expect("вагонетка едет");
        assert_eq!(r.path, vec![[500.0, 100.0], [500.0, 2000.0], [900.0, 2000.0]]);
        assert_eq!((r.lag, r.cargo), (0.0, false), "голова сцепки");
        let o = rail_ride(&m, &ore, 100).unwrap();
        assert!(o.cargo && o.lag.abs() < 1e-3, "руда едет в голове и выгружается");
        assert!((rail_ride(&m, &tail, 100).unwrap().lag - 220.0).abs() < 1e-2, "хвост отстаёт на 2,2 клетки");
        assert!(rail_ride(&m, &track, 100).is_none() && rail_ride(&m, &aside, 100).is_none(), "рельсы и то, что рядом, стоят");
        assert!(rail_ride(&m, &beam, 100).is_none(), "крепь на пути стоит");
        // Пометка в сцене.
        let la = layout(&m, &[]);
        let mut t = canopy_tile("k/c.webp".into(), [5.0, 7.8], [1.0, 2.0], 0.0, 100);
        (t.item, t.rail) = (true, rail_ride(&m, &tail, 100));
        let scene = foundry_levels(&m, "Рудник", &[SceneLevel { name: "Рудник".into(), image_src: "k/a.jpg".into(), layout: &la, roofs: vec![t] }], 100, &Default::default());
        assert_eq!(scene["tiles"][0]["flags"][LIVE_MODULE]["rail"], json!({"path": [[500, 100], [500, 2000], [900, 2000]], "lag": 220, "cargo": false}));
        assert_eq!(scene["tiles"][0]["elevation"], 0, "по земле, под фишками");
        // Путь сдвигается вместе с картой.
        m.grow(2, 1, 0, 0);
        assert_eq!(m.rails[0][0], [7.0, 2.0]);
    }

    #[test]
    fn canal_runs_while_harbour_stands_and_water_flows_round_a_rock() {
        let mut m = Map::new(12, 12);
        let water = m.materials.iter().position(|x| x.name == "Мелководье").unwrap();
        // Тёмная непрозрачная вода — мутная: блики и рыбы гаснут, а пена у бегущей остаётся.
        m.materials[water].name = "Мутная вода".into();
        assert!(m.materials[water].murky());
        let all = crate::mask::cells(m.rooms.w, m.rooms.h, (0, 0), (12, 12));
        m.paint(crate::map::Layer::Floor, water, &all);
        m.prune();
        // Канал слева бежит вниз, справа — стоячая гавань.
        m.flows = vec![crate::map::Flow { points: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 12.0], [0.0, 12.0]], current: 0.8, drift: 90.0 }];
        // Валун в канале — дыра в воде; на берегу — нет.
        m.props = vec![thing("x/Boulder_Stone_Earthy_A3_3x3.webp", [2.5, 4.0], [3.0, 3.0], 0.0)];
        m.props[0].scale = 0.5;
        assert!(crate::map::rock_by_name(&m.props[0].asset) && !crate::map::rock_by_name("x/Barrel_A1_1x1.webp"));
        let lay = layout(&m, &[]);
        let water_zones: Vec<&Zone> = lay.zones.iter().filter(|z| z.effect == "water").collect();
        assert_eq!(water_zones.len(), 2);
        let canal = water_zones.iter().find(|z| z.flow.is_some()).expect("канал");
        assert_eq!(canal.flow, Some((0.8, 90.0)));
        assert_eq!(canal.loops.iter().filter(|l| l.1).count(), 1, "дыра под валуном");
        let harbour = water_zones.iter().find(|z| z.flow.is_none()).unwrap();
        assert_eq!(harbour.loops.iter().filter(|l| l.1).count(), 0);
        let scene = foundry_levels(&m, "Канал", &[SceneLevel { name: "Канал".into(), image_src: "p.jpg".into(), layout: &lay, roofs: Vec::new() }], 100, &Default::default());
        let regions = scene["regions"].as_array().unwrap();
        let flowing: Vec<&Value> = regions.iter().filter(|r| r["flags"][LIVE_MODULE]["current"].is_number()).collect();
        assert_eq!(flowing.len(), 1, "течёт только канал");
        assert_eq!((flowing[0]["flags"][LIVE_MODULE]["current"].as_f64(), flowing[0]["flags"][LIVE_MODULE]["drift"].as_i64()), (Some(0.8), Some(90)));
        assert!(flowing[0]["flags"][LIVE_MODULE].get("foam").is_none(), "бегущая мутная вода пенится у камня");
        let still = regions.iter().find(|r| r["flags"][LIVE_MODULE]["effect"] == "water" && !r["flags"][LIVE_MODULE]["current"].is_number()).unwrap();
        assert_eq!((still["flags"][LIVE_MODULE]["foam"].as_i64(), still["flags"][LIVE_MODULE]["fish"].as_bool()), (Some(0), Some(false)));
        m.grow(1, 0, 0, 0);
        assert_eq!(m.flows[0].points[0], [1.0, 0.0]);
    }

    #[test]
    fn the_3d_model_is_a_tile_over_its_frame_on_every_storey() {
        let mut scene = json!({"tiles": [], "flags": {}});
        model_tile(&mut scene, "kartograf/x/x_3d.glb", [[-0.125, 0.0, -0.125], [20.125, 4.0, 10.0]], 100, 2);
        let t = &scene["tiles"][0];
        assert_eq!((t["x"].as_f64(), t["y"].as_f64(), t["width"].as_f64(), t["height"].as_f64()), (Some(1000.0), Some(494.0), Some(2025.0), Some(1013.0)));
        assert_eq!(t["elevation"], 0.0);
        assert_eq!(t["levels"], json!([LEVEL_ID, level_id(1)]));
        let f = &t["flags"][CANVAS3D];
        assert_eq!((f["model3d"].as_str(), f["fillType"].as_str(), f["depth"].as_f64()), (Some("kartograf/x/x_3d.glb"), Some("stretch"), Some(400.0)));
        assert_eq!(scene["flags"][CANVAS3D]["showSceneWalls"], false);
        assert_eq!(scene["flags"][CANVAS3D]["renderBackground"], false);
        assert!(t.get("texture").is_none(), "середина рамки — якорь по умолчанию, 0,5");
    }
}
