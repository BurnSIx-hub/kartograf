//! Подземелье или здание WMO целиком — картой Картографа: пол снимком сверху (как в игре,
//! потолки и крыши срезаны над полом), стены для Foundry — срезом геометрии на уровне груди,
//! предметы — объектами на своих местах. Север игры (+X) — вверх карты, запад (+Y) — влево.

use super::blp::Rgba;
use super::wmo::{self, Group};
use super::adt;
use super::{Client, blp};
use crate::map::{Glow, GlowTune, Map, Paint, Prop, Wall, WallStyle};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Options {
    /// Точек на клетку в картинках пола (меньше для огромных: см. `max_pixels`).
    pub ppc: f32,
    pub cells_per_yard: f32,
    /// Срез над полом, ярды: выше — потолки, крыши, балки; их не видно.
    pub cut: f32,
    /// Высота среза стен над полом, ярды.
    pub wall_at: f32,
    /// Сторона куска пола в клетках.
    pub tile: u32,
    pub supersample: u32,
    /// Не больше стольких точек картинок пола на этаж: огромное рисуется реже.
    pub max_pixels: f64,
    /// Поля вокруг, клеток.
    pub margin: f32,
    /// Вариант обстановки к всегда стоящим предметам: имя набора (`Set_A`) или номер; нет —
    /// первый вариант, если он есть.
    pub set: Option<String>,
    /// Стены — срезом геометрии на высоте груди (прежний способ), а не по краю пола.
    pub cut_walls: bool,
    /// Карта инстанса: местность только в этой рамке мира (ярды `[x0, y0, x1, y1]`); нет — у
    /// зданий карты с запасом.
    pub area: Option<[f32; 4]>,
    /// Здания, которых нет в файлах карты (их ставит сервер: корабль в бухте Мёртвых копей):
    /// путь, точка мира начала модели (X — север, Y — запад, Z) и поворот расстановки, градусы.
    pub extra: Vec<(String, [f32; 3], f32)>,
}

impl Default for Options {
    fn default() -> Self {
        Self { ppc: 128.0, cells_per_yard: 0.6, cut: 3.0, wall_at: 1.2, tile: 16, supersample: 2, max_pixels: 1.6e8, margin: 2.0, set: None, cut_walls: false, area: None, extra: Vec::new() }
    }
}

/// Здание целиком: группы, текстуры материалов, расстановка предметов.
pub struct Scene {
    pub name: String,
    pub groups: Vec<Group>,
    pub materials: Vec<wmo::Material>,
    /// Текстуры по одному разу и номер текстуры у каждого материала.
    pub textures: Vec<Rgba>,
    /// Пути текстур в клиенте, по номерам `textures`.
    pub tex_names: Vec<String>,
    pub mat_tex: Vec<Option<usize>>,
    pub doodads: Vec<wmo::Doodad>,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub missing: Vec<String>,
    /// Наборы предметов здания и какие взяты.
    pub sets: Vec<String>,
    pub sets_used: Vec<String>,
    /// Флаги корня (`MOHD`): как читать вид жидкости групп.
    pub root_flags: u16,
    /// Текстуры глади игры: вода, лава, слизь (первый кадр анимации); нет — ровный цвет.
    pub liquid_tex: [Option<Rgba>; 3],
    /// Здания карты (`load_map`): путь и рамка в мире `[x0, y0, x1, y1]`.
    pub placed: Vec<(String, [f32; 4])>,
}

/// Текстуры глади в клиенте: озёрная вода, лава, слизь.
const LIQUID_TEX: [&str; 3] = ["XTextures\\river\\lake_a.1.blp", "XTextures\\lava\\lava.1.blp", "XTextures\\slime\\slime.1.blp"];

/// Путь карты инстанса для `build`: `World\\Maps\\<карта>\\<карта>.wdt`.
pub fn map_path(map: &str) -> String {
    format!("World\\Maps\\{map}\\{map}.wdt")
}

/// Путь — карта инстанса (WDT), а не здание.
pub fn is_map(path: &str) -> bool {
    path.to_lowercase().ends_with(".wdt")
}

/// Номер текстуры в сцене (каждая читается раз); нет файла — `None` и строка в `missing`.
fn scene_texture(client: &Client, scene: &mut Scene, cache: &mut HashMap<String, Option<usize>>, path: &str) -> Option<usize> {
    let key = path.to_lowercase();
    if let Some(&i) = cache.get(&key) {
        return i;
    }
    let i = match client.read(path).and_then(|b| blp::decode(&b)) {
        Ok(img) => {
            scene.textures.push(img);
            scene.tex_names.push(path.to_owned());
            Some(scene.textures.len() - 1)
        }
        Err(e) => {
            scene.missing.push(format!("{path}: {e}"));
            None
        }
    };
    cache.insert(key, i);
    i
}

/// Вода повёрнутого здания — сеткой мира (у `Liquid` клетки по осям): клетки и высоты
/// берутся из сетки здания обратным поворотом. Вид — код клетки (0 вода, 2 магма, 3 слизь).
fn world_liquid(l: &wmo::Liquid, p: &adt::Place, code: u8) -> wmo::Liquid {
    let st = wmo::LIQUID_STEP;
    let (w, h) = (l.tiles[0] as f32 * st, l.tiles[1] as f32 * st);
    let corners = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]].map(|[a, b]| p.to_world([l.corner[0] + a, l.corner[1] + b, 0.0]));
    let x0 = corners.iter().map(|c| c[0]).fold(f32::MAX, f32::min);
    let y0 = corners.iter().map(|c| c[1]).fold(f32::MAX, f32::min);
    let x1 = corners.iter().map(|c| c[0]).fold(f32::MIN, f32::max);
    let y1 = corners.iter().map(|c| c[1]).fold(f32::MIN, f32::max);
    let (nx, ny) = ((((x1 - x0) / st).round() as usize).max(1), (((y1 - y0) / st).round() as usize).max(1));
    // Точка мира → доли сетки здания.
    let grid = |x: f32, y: f32| {
        let m = p.to_model([x, y]);
        [(m[0] - l.corner[0]) / st, (m[1] - l.corner[1]) / st]
    };
    // Высота узла мира — с ближайшего узла здания, который угол видимой клетки: у пустых клеток
    // узлы без высоты (0 вместо глади на −6,5 у каналов Штормграда), а узел мира у края воды
    // при повороте ближе к ним — гладь вставала косым парусом над мостовой. Угол видимой клетки
    // среди четырёх соседних узлов есть всегда: угол клетки мира не дальше 0,71 клетки от её
    // середины, а середина лежит в видимой клетке здания.
    let wet = |ix: usize, iy: usize| {
        (ix.saturating_sub(1)..=ix.min(l.tiles[0].saturating_sub(1))).any(|i| (iy.saturating_sub(1)..=iy.min(l.tiles[1].saturating_sub(1))).any(|j| l.shown.get(j * l.tiles[0] + i).copied().unwrap_or(false)))
    };
    let hgt = |fx: f32, fy: f32| {
        let clamp = |v: f32, n: usize| (v.max(0.0) as usize).min(n.saturating_sub(1));
        let near = [(fx.floor(), fy.floor()), (fx.ceil(), fy.floor()), (fx.floor(), fy.ceil()), (fx.ceil(), fy.ceil())]
            .into_iter()
            .map(|(a, b)| (clamp(a, l.verts[0]), clamp(b, l.verts[1])))
            .min_by(|&(a, b), &(c, d)| {
                let key = |i: usize, j: usize| (!wet(i, j), (i as f32 - fx).powi(2) + (j as f32 - fy).powi(2));
                let (ka, kb) = (key(a, b), key(c, d));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            });
        near.and_then(|(ix, iy)| l.heights.get(iy * l.verts[0] + ix).copied()).unwrap_or(l.corner[2])
    };
    let heights = (0..(nx + 1) * (ny + 1))
        .map(|k| {
            let g = grid(x0 + (k % (nx + 1)) as f32 * st, y0 + (k / (nx + 1)) as f32 * st);
            hgt(g[0], g[1]) + p.pos[1]
        })
        .collect();
    let shown = (0..nx * ny)
        .map(|k| {
            let g = grid(x0 + ((k % nx) as f32 + 0.5) * st, y0 + ((k / nx) as f32 + 0.5) * st);
            let (i, j) = (g[0].floor(), g[1].floor());
            i >= 0.0 && j >= 0.0 && (i as usize) < l.tiles[0] && (j as usize) < l.tiles[1] && l.shown[j as usize * l.tiles[0] + i as usize]
        })
        .collect();
    wmo::Liquid { corner: [x0, y0, l.corner[2] + p.pos[1]], verts: [nx + 1, ny + 1], tiles: [nx, ny], heights, shown, tile_kind: code }
}

/// Кватернион поворота вокруг вертикали.
fn yaw_quat(a: f32) -> [f32; 4] {
    let (s, c) = (a / 2.0).sin_cos();
    [0.0, 0.0, s, c]
}

/// Нормали вершин по треугольникам (у местности своих нормалей не берём).
fn vertex_normals(verts: &[[f32; 3]], tris: &[[u32; 3]]) -> Vec<[f32; 3]> {
    let mut n = vec![[0.0f32; 3]; verts.len()];
    for t in tris {
        let (nn, _) = tri_normal(verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]);
        for &i in t {
            for q in 0..3 {
                n[i as usize][q] += nn[q];
            }
        }
    }
    n.into_iter()
        .map(|v| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
            [v[0] / l, v[1] / l, v[2] / l]
        })
        .collect()
}

/// Карта инстанса целиком: здания на своих местах (с водой и обстановкой), местность в рамке
/// (`area` или здания карты с запасом 40 ярдов; по высоте — рядом со зданиями, иначе склон над
/// копями стал бы этажом), предметы местности. Карта-здание (WDT с флагом 1) — как `load`.
pub fn load_map(client: &Client, path: &str, set: Option<&str>, area: Option<[f32; 4]>, extra: &[(String, [f32; 3], f32)]) -> Result<Scene, String> {
    let wdt = adt::parse_wdt(&client.read(path)?)?;
    if wdt.flags & 1 != 0 {
        let wmo = wdt.wmo.ok_or("WDT: карта-здание без имени здания")?;
        return load(client, &wmo, set);
    }
    let stem = path.rsplit_once('.').map_or(path, |(s, _)| s);
    let name = stem.rsplit(['\\', '/']).next().unwrap_or(stem).to_owned();
    let big = wdt.flags & 0x4 != 0;
    // С рамкой — только файлы, что её задевают: у материка их сотни. Здание числится в каждом
    // файле, который задевает, — его не потерять.
    let adts: Vec<adt::Adt> = wdt
        .tiles
        .iter()
        .filter(|&&(x, y)| area.is_none_or(|r| adt::tile_touches(x, y, r)))
        .filter_map(|&(x, y)| client.read(&format!("{stem}_{x}_{y}.adt")).ok())
        .filter_map(|b| adt::parse_adt(&b, big).ok())
        .collect();
    if adts.is_empty() {
        return Err(format!("{path}: нет файлов местности"));
    }
    let liquid_tex = LIQUID_TEX.map(|p| client.read(p).and_then(|b| blp::decode(&b)).ok());
    let mut scene = Scene { name, groups: Vec::new(), materials: Vec::new(), textures: Vec::new(), tex_names: Vec::new(), mat_tex: Vec::new(), doodads: Vec::new(), lo: [0.0; 3], hi: [0.0; 3], missing: Vec::new(), sets: Vec::new(), sets_used: Vec::new(), root_flags: 0, liquid_tex, placed: Vec::new() };
    let mut cache: HashMap<String, Option<usize>> = HashMap::new();
    // Здания — по разу (одно и то же стоит в соседних файлах).
    let mut seen = std::collections::HashSet::new();
    let mut wlo = [f32::MAX; 3];
    let mut whi = [f32::MIN; 3];
    let mut placed: Vec<(String, adt::Place)> = Vec::new();
    // Поставленные по запросу — последние `extra.len()` в списке.
    // Заданная рамка: здания вне её не берутся (на карте Тёмного Клыка — фермы и склеп поодаль).
    let outside = |p: &adt::Place| area.zip(p.bounds).is_some_and(|(r, b)| b[2] < r[0] || b[0] > r[2] || b[3] < r[1] || b[1] > r[3]);
    for a in &adts {
        for p in &a.objects {
            if seen.insert(p.unique)
                && !outside(p)
                && let Some(w) = a.wmos.get(p.name)
            {
                placed.push((w.clone(), p.clone()));
            }
        }
    }
    for (k, (w, at, turn)) in extra.iter().enumerate() {
        let pos = [adt::ORIGIN - at[1], at[2], adt::ORIGIN - at[0]];
        placed.push((w.clone(), adt::Place { name: 0, unique: u32::MAX - k as u32, pos, rot: [0.0, *turn, 0.0], scale: 1.0, set: 0, bounds: None }));
    }
    {
        for (wpath, p) in &placed {
            let (wpath, p) = (wpath.as_str(), p);
            let asked = p.unique > u32::MAX - extra.len() as u32 - 1 && !extra.is_empty();
            let root = match client.read(wpath).and_then(|b| wmo::parse_root(&b)) {
                Ok(r) => r,
                Err(e) => {
                    scene.missing.push(format!("{wpath}: {e}"));
                    continue;
                }
            };
            let base = scene.materials.len();
            for m in &root.materials {
                let t = scene_texture(client, &mut scene, &mut cache, &m.texture);
                scene.mat_tex.push(t);
            }
            scene.materials.extend(root.materials.iter().cloned());
            let mut own = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for i in 0..root.group_count {
                let Ok(mut g) = client.read(&wmo::group_path(wpath, i)).and_then(|b| wmo::parse_group(&b)) else { continue };
                for v in &mut g.verts {
                    *v = p.to_world(*v);
                    for k in 0..3 {
                        wlo[k] = wlo[k].min(v[k]);
                        whi[k] = whi[k].max(v[k]);
                    }
                    own = [own[0].min(v[0]), own[1].min(v[1]), own[2].max(v[0]), own[3].max(v[1])];
                }
                for n in &mut g.normals {
                    *n = p.turn(*n);
                }
                g.mat_base = base;
                if asked {
                    g.flags |= DECK;
                }
                if let Some(l) = g.liquid.take() {
                    let code = match wmo::liquid_kind(root.flags, g.group_liquid, l.tile_kind) {
                        wmo::LiquidKind::Water => 0,
                        wmo::LiquidKind::Magma => 2,
                        wmo::LiquidKind::Slime => 3,
                    };
                    g.liquid = Some(world_liquid(&l, p, code));
                }
                scene.groups.push(g);
            }
            if own[0] <= own[2] {
                scene.placed.push((wpath.to_owned(), own));
            }
            // Обстановка: всегда стоящий набор и набор этого места.
            let chosen: Vec<std::ops::Range<usize>> = if root.sets.is_empty() {
                std::iter::once(0..root.doodads.len()).collect()
            } else {
                root.sets.iter().enumerate().filter(|(k, _)| *k == 0 || *k == p.set as usize).map(|(_, s)| s.first..(s.first + s.count).min(root.doodads.len())).collect()
            };
            for r in chosen {
                for d in &root.doodads[r] {
                    scene.doodads.push(wmo::Doodad { model: d.model.clone(), pos: p.to_world(d.pos), rot: yaw_quat(p.yaw() + wmo::yaw(d.rot)), scale: d.scale * p.scale });
                }
            }
        }
    }
    let rect = area.or_else(|| (wlo[0] <= whi[0]).then(|| [wlo[0] - 40.0, wlo[1] - 40.0, whi[0] + 40.0, whi[1] + 40.0]));
    let zr = (wlo[2] <= whi[2]).then(|| [wlo[2] - 25.0, whi[2] + 25.0]);
    let inside = |x0: f32, y0: f32, x1: f32, y1: f32| rect.is_none_or(|r| x1 >= r[0] && x0 <= r[2] && y1 >= r[1] && y0 <= r[3]);
    let mut seen_m = std::collections::HashSet::new();
    for a in &adts {
        let tex: Vec<Option<usize>> = a.textures.iter().map(|t| scene_texture(client, &mut scene, &mut cache, t)).collect();
        for ch in &a.chunks {
            if !inside(ch.pos[0] - adt::CHUNK, ch.pos[1] - adt::CHUNK, ch.pos[0], ch.pos[1]) {
                continue;
            }
            let verts: Vec<[f32; 3]> = (0..145).map(|k| ch.vertex(k)).collect();
            let (zlo, zhi) = verts.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v[2]), b.max(v[2])));
            if zr.is_some_and(|z| zhi < z[0] || zlo > z[1]) {
                continue;
            }
            let tris = ch.triangles();
            if tris.is_empty() && ch.liquid.is_none() {
                continue;
            }
            let normals = vertex_normals(&verts, &tris);
            let n = tris.len();
            let paint = adt::Paint { layers: ch.layers.iter().map(|l| (tex.get(l.tex).copied().flatten(), l.alpha.clone())).collect(), shadow: ch.shadow.clone() };
            scene.groups.push(Group {
                flags: 0,
                verts,
                normals,
                uvs: (0..145).map(adt::Chunk::uv).collect(),
                colors: ch.colors.clone(),
                tris,
                tri_flags: vec![0x08; n],
                tri_mat: vec![0; n],
                liquid: ch.liquid.clone(),
                group_liquid: 0,
                mat_base: 0,
                terrain: Some(Box::new(paint)),
            });
        }
        for p in &a.doodads {
            let w = p.to_world([0.0; 3]);
            if !seen_m.insert(p.unique) || !inside(w[0], w[1], w[0], w[1]) || zr.is_some_and(|z| w[2] < z[0] || w[2] > z[1]) {
                continue;
            }
            let Some(model) = a.models.get(p.name) else { continue };
            scene.doodads.push(wmo::Doodad { model: model.clone(), pos: w, rot: yaw_quat(p.yaw()), scale: p.scale });
        }
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for g in &scene.groups {
        for (t, tri) in g.tris.iter().enumerate() {
            if !g.collides(t) && !g.renders(t) {
                continue;
            }
            for &i in tri {
                let v = g.verts[i as usize];
                for k in 0..3 {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return Err(format!("{path}: в рамке нет ни местности, ни зданий"));
    }
    scene.lo = lo;
    scene.hi = hi;
    Ok(scene)
}

pub fn load(client: &Client, path: &str, set: Option<&str>) -> Result<Scene, String> {
    let root = wmo::parse_root(&client.read(path)?)?;
    let mut groups = Vec::new();
    for i in 0..root.group_count {
        groups.push(wmo::parse_group(&client.read(&wmo::group_path(path, i))?)?);
    }
    let mut missing = Vec::new();
    let mut cache: HashMap<String, Option<usize>> = HashMap::new();
    let mut textures: Vec<Rgba> = Vec::new();
    let mut tex_names: Vec<String> = Vec::new();
    let mut mat_tex = Vec::new();
    for m in &root.materials {
        let key = m.texture.to_lowercase();
        let idx = *cache.entry(key).or_insert_with(|| match client.read(&m.texture).and_then(|b| blp::decode(&b)) {
            Ok(img) => {
                textures.push(img);
                tex_names.push(m.texture.clone());
                Some(textures.len() - 1)
            }
            Err(e) => {
                missing.push(format!("{}: {e}", m.texture));
                None
            }
        });
        mat_tex.push(idx);
    }
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for g in &groups {
        for (t, tri) in g.tris.iter().enumerate() {
            if !g.collides(t) && !g.renders(t) {
                continue;
            }
            for &i in tri {
                let p = g.verts[i as usize];
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return Err("в здании нет треугольников".into());
    }
    // Всегда стоящие (набор 0) и один вариант: названный или первый.
    let extra = match set {
        Some(want) => Some(want.parse::<usize>().ok().filter(|&i| i > 0 && i < root.sets.len()).or_else(|| root.sets.iter().position(|s| s.name.eq_ignore_ascii_case(want))).ok_or_else(|| format!("нет набора «{want}»: есть {:?}", root.sets.iter().map(|s| &s.name).collect::<Vec<_>>()))?),
        None => (root.sets.len() > 1).then_some(1),
    };
    let chosen: Vec<usize> = std::iter::once(0).chain(extra).filter(|&i| i < root.sets.len()).collect();
    let ranges: Vec<std::ops::Range<usize>> = if root.sets.is_empty() { std::iter::once(0..root.doodads.len()).collect() } else { chosen.iter().map(|&i| root.sets[i].first..(root.sets[i].first + root.sets[i].count).min(root.doodads.len())).collect() };
    let sets = root.sets.iter().map(|s| s.name.clone()).collect();
    let sets_used = chosen.iter().map(|&i| root.sets[i].name.clone()).collect();
    let doodads = root.doodads.into_iter().enumerate().filter(|(i, _)| ranges.iter().any(|r| r.contains(i))).map(|(_, d)| d).collect();
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path).trim_end_matches(".wmo").trim_end_matches(".WMO").to_owned();
    let liquid_tex = LIQUID_TEX.map(|p| client.read(p).and_then(|b| blp::decode(&b)).ok());
    Ok(Scene { name, groups, materials: root.materials, textures, tex_names, mat_tex, doodads, lo, hi, missing, sets, sets_used, root_flags: root.flags, liquid_tex, placed: Vec::new() })
}

fn tri_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> ([f32; 3], f32) {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l < 1e-9 { ([0.0, 0.0, 0.0], 0.0) } else { ([n[0] / l, n[1] / l, n[2] / l], l / 2.0) }
}

impl Scene {
    fn tri(&self, g: usize, t: usize) -> [[f32; 3]; 3] {
        let gr = &self.groups[g];
        gr.tris[t].map(|i| gr.verts[i as usize])
    }
}

/// Проходимые поверхности по сетке 0,5 ярда: в каждой клетке — высоты всех полов над ней
/// (под балконом — пол и балкон; дно под водой — высотой глади). `walk` — до этой поверхности можно дойти пешком от главной
/// части здания: соседние клетки, перепад не больше ступеньки. Кровля галерей, капители,
/// верхушки стен смотрят вверх, но с пола на них не попасть — это не пол. Отдельный кусок
/// над другим полом, над которым ничего нет (скаты крыши), — тоже не пол, даже большой.
pub struct Surfaces {
    pub x0: f32,
    pub y0: f32,
    pub step: f32,
    pub w: usize,
    pub h: usize,
    /// Начало высот клетки в `z` (длина w·h + 1).
    start: Vec<usize>,
    z: Vec<f32>,
    /// Поверхность наклонная (круче 25°): скат крыши, склон, пандус.
    steep: Vec<bool>,
    /// Поверхность — гладь жидкости (или дно под ней, поднятое к глади).
    wet: Vec<bool>,
    /// Поверхность здания, поставленного по запросу (`DECK`).
    deck: Vec<bool>,
    walk: Vec<bool>,
}

/// В воду или лаву с берега спрыгивают и выбираются обратно: перепад до глади до 6 ярдов —
/// проход, а не обрыв.
const JUMP_IN: f32 = 6.0;

/// Флаг группы (свой, вне флагов `MOGP`): здание поставлено по запросу (корабль, которого нет в
/// файлах карты) — его полы всегда пол: палуба над трюмом не крыша, отдельный от берега корабль
/// не отбрасывается как мелкий кусок.
pub const DECK: u32 = 0x4000_0000;

/// Перепад, который фишка проходит шагом (ступенька, порог), ярды.
const STEP_UP: f32 = 1.2;

/// Поверхность для сети проходов: держит фишку и не круче 60° — пандусы столкновений над
/// ступенями бывают круче 45°, а без них площадка на лестнице отрывалась от двора.
fn passable(scene: &Scene, g: usize, t: usize) -> bool {
    let [a, b, c] = scene.tri(g, t);
    scene.groups[g].collides(t) && tri_normal(a, b, c).0[2] > 0.5
}

impl Surfaces {
    pub fn new(scene: &Scene) -> Self {
        let step = 0.5;
        let (x0, y0) = (scene.lo[0] - 2.0, scene.lo[1] - 2.0);
        let w = ((scene.hi[0] + 2.0 - x0) / step).ceil() as usize + 1;
        let h = ((scene.hi[1] + 2.0 - y0) / step).ceil() as usize + 1;
        let mut cells: Vec<Vec<(f32, bool)>> = vec![Vec::new(); w * h];
        // Высоты полов поставленных по запросу зданий по клеткам (их мало — по месту).
        let mut decks: HashMap<usize, Vec<f32>> = HashMap::new();
        // Самое высокое, что есть над клеткой: под потолком пол «накрыт», крыша — нет.
        let mut top = vec![f32::MIN; w * h];
        let liquid = LiquidMap::new(scene);
        // Под гладью нарисовано дно (смотрит вверх, ниже глади): у лавы оно без столкновений, и
        // без этого лавовое озеро было бы дырой в полу, обнесённой стенами.
        let lq_at = |k: usize| liquid.at(x0 + ((k % w) as f32 + 0.5) * step, y0 + ((k / w) as f32 + 0.5) * step);
        let mut bed = vec![false; w * h];
        for g in 0..scene.groups.len() {
            for t in 0..scene.groups[g].tris.len() {
                let gr = &scene.groups[g];
                if !gr.collides(t) && !gr.renders(t) {
                    continue;
                }
                let floor = passable(scene, g, t);
                let is_deck = scene.groups[g].flags & DECK != 0;
                let p = scene.tri(g, t);
                let steep = tri_normal(p[0], p[1], p[2]).0[2] < 0.9;
                let up = tri_normal(p[0], p[1], p[2]).0[2] > 0.3;
                // Треугольник в клетках сетки: по центрам клеток, мелкий — хотя бы клетку середины.
                let q = p.map(|v| [(v[0] - x0) / step, (v[1] - y0) / step, v[2]]);
                let mid = [(q[0][0] + q[1][0] + q[2][0]) / 3.0, (q[0][1] + q[1][1] + q[2][1]) / 3.0, (q[0][2] + q[1][2] + q[2][2]) / 3.0];
                let mut put = |i: usize, j: usize, v: f32| {
                    if i < w && j < h {
                        let k = j * w + i;
                        top[k] = top[k].max(v);
                        if floor {
                            cells[k].push((v, steep));
                            if is_deck {
                                decks.entry(k).or_default().push(v);
                            }
                        }
                        if up && !bed[k] && v < lq_at(k) + 0.5 {
                            bed[k] = true;
                        }
                    }
                };
                let area = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[1][1] - q[0][1]) * (q[2][0] - q[0][0]);
                if area.abs() < 1e-6 {
                    put(mid[0] as usize, mid[1] as usize, mid[2]);
                    continue;
                }
                let i0 = q.iter().map(|v| v[0]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
                let i1 = (q.iter().map(|v| v[0]).fold(f32::MIN, f32::max).ceil() as usize).min(w);
                let j0 = q.iter().map(|v| v[1]).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
                let j1 = (q.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil() as usize).min(h);
                let mut any = false;
                for j in j0..j1 {
                    for i in i0..i1 {
                        let (px, py) = (i as f32 + 0.5, j as f32 + 0.5);
                        let w0 = ((q[1][0] - px) * (q[2][1] - py) - (q[1][1] - py) * (q[2][0] - px)) / area;
                        let w1 = ((q[2][0] - px) * (q[0][1] - py) - (q[2][1] - py) * (q[0][0] - px)) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                            continue;
                        }
                        any = true;
                        put(i, j, w0 * q[0][2] + w1 * q[1][2] + w2 * q[2][2]);
                    }
                }
                if !any {
                    put(mid[0] as usize, mid[1] as usize, mid[2]);
                }
            }
        }
        // Дно под водой (глубже ярда, и до глади нет другого пола) — на высоте глади: фишка там
        // плывёт. Иначе мостки над затопленным залом стоят «над другим полом» и уходят на второй
        // этаж, а пол под ними — на первый. Над дном без столкновений (лава) пол — сама гладь.
        for (k, c) in cells.iter_mut().enumerate() {
            let lq = lq_at(k);
            if lq == f32::MIN {
                continue;
            }
            if bed[k] && !c.iter().any(|e| e.0 < lq + 0.5) {
                c.push((lq, false));
                continue;
            }
            if c.is_empty() {
                continue;
            }
            let zs: Vec<f32> = c.iter().map(|e| e.0).collect();
            for e in c.iter_mut() {
                if lq > e.0 + 1.0 && !zs.iter().any(|&o| o > e.0 + 0.3 && o < lq) {
                    *e = (lq, false);
                }
            }
        }
        let mut start = Vec::with_capacity(w * h + 1);
        let (mut z, mut steep) = (Vec::new(), Vec::new());
        for c in &mut cells {
            c.sort_by(|a, b| a.0.total_cmp(&b.0));
            start.push(z.len());
            for &(v, st) in c.iter() {
                if z.len() == *start.last().unwrap() || v - *z.last().unwrap() > 0.3 {
                    z.push(v);
                    steep.push(st);
                } else if !st {
                    // Ровное в той же высоте (площадка у ската) — не скат.
                    *steep.last_mut().unwrap() = false;
                }
            }
        }
        start.push(z.len());
        let wet = (0..w * h).flat_map(|k| (start[k]..start[k + 1]).map(move |a| (k, a))).map(|(k, a)| (z[a] - lq_at(k)).abs() < 0.01).collect();
        let deck = (0..w * h).flat_map(|k| (start[k]..start[k + 1]).map(move |a| (k, a))).map(|(k, a)| decks.get(&k).is_some_and(|d| d.iter().any(|&v| (v - z[a]).abs() < 0.31))).collect();
        let mut s = Self { x0, y0, step, w, h, start, walk: vec![false; z.len()], z, steep, wet, deck };
        s.find_network(&top);
        s
    }

    /// Границы проходимого пола в ярдах: `[x0, y0, x1, y1]`; пола нет — `None`.
    pub fn walk_bounds(&self) -> Option<[f32; 4]> {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for k in 0..self.w * self.h {
            if self.cell(k).any(|a| self.walk[a]) {
                let (x, y) = (self.x0 + (k % self.w) as f32 * self.step, self.y0 + (k / self.w) as f32 * self.step);
                b = [b[0].min(x), b[1].min(y), b[2].max(x + self.step), b[3].max(y + self.step)];
            }
        }
        (b[0] <= b[2]).then_some(b)
    }

    fn cell(&self, k: usize) -> std::ops::Range<usize> {
        self.start[k]..self.start[k + 1]
    }

    /// Связные куски поверхностей; проходимые — самый большой и те, что не меньше его четверти
    /// (крыло, куда ведёт дверь с порогом выше ступеньки). Над другим полом и открыто небу
    /// (`top` — самое высокое над клеткой): наклонное — скат крыши или склон над домом, не пол
    /// даже рядом с полом; кусок, который так лежит больше чем наполовину, — крыша: он не
    /// проходим и не главный.
    fn find_network(&mut self, top: &[f32]) {
        let n = self.z.len();
        let mut comp = vec![usize::MAX; n];
        let mut sizes = Vec::new();
        let cell_of: Vec<usize> = (0..self.w * self.h).flat_map(|k| self.cell(k).map(move |_| k)).collect();
        // Над водой открыто небу — палуба, пирс, мостки, а не крыша: крыша стоит над сухим полом.
        let open_over = |a: usize| top[cell_of[a]] <= self.z[a] + 2.0 && self.cell(cell_of[a]).any(|b| self.z[b] < self.z[a] - 2.0 && !self.wet[b]);
        let skip: Vec<bool> = (0..n).map(|a| self.steep[a] && open_over(a)).collect();
        for s0 in 0..n {
            if comp[s0] != usize::MAX || skip[s0] {
                continue;
            }
            let id = sizes.len();
            let mut size = 0;
            let mut stack = vec![s0];
            comp[s0] = id;
            while let Some(a) = stack.pop() {
                size += 1;
                let k = cell_of[a];
                let (i, j) = (k % self.w, k / self.w);
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ni, nj) = (i as i32 + di, j as i32 + dj);
                    if ni < 0 || nj < 0 || ni as usize >= self.w || nj as usize >= self.h {
                        continue;
                    }
                    for b in self.cell(nj as usize * self.w + ni as usize) {
                        let dz = self.z[b] - self.z[a];
                        let reach = dz.abs() <= STEP_UP || (self.wet[a] || self.wet[b]) && dz.abs() <= JUMP_IN;
                        if comp[b] == usize::MAX && !skip[b] && reach {
                            comp[b] = id;
                            stack.push(b);
                        }
                    }
                }
            }
            sizes.push(size);
        }
        // Крыши: открыто небу и под ним другой пол.
        let mut over = vec![0usize; sizes.len()];
        for (a, &c) in comp.iter().enumerate() {
            if c != usize::MAX && open_over(a) {
                over[c] += 1;
            }
        }
        let mut has_deck = vec![false; sizes.len()];
        for (a, &c) in comp.iter().enumerate() {
            if c != usize::MAX && self.deck[a] {
                has_deck[c] = true;
            }
        }
        let roof: Vec<bool> = (0..sizes.len()).map(|c| !has_deck[c] && over[c] * 2 > sizes[c]).collect();
        let big = (0..sizes.len()).filter(|&c| !roof[c]).map(|c| sizes[c]).max().or_else(|| sizes.iter().copied().max()).unwrap_or(0);
        for (w, &c) in self.walk.iter_mut().zip(&comp) {
            *w = c != usize::MAX && !roof[c] && (sizes[c] * 4 >= big || has_deck[c]);
        }
    }
}

/// Сколько квадратных ярдов проходимого пола нужно отдельному этажу.
const MIN_LEVEL_AREA: f32 = 300.0;

/// Этажи. Полосы высот проходимого пола, разделённые пустотой не меньше 3 ярдов, — кандидаты;
/// соседние склеиваются в один этаж, если их полы не стоят друг над другом (двор и собор на
/// лестницах разной высоты — один этаж; подвал под домом, дом и верхний этаж — три). Полоса,
/// где пола меньше `MIN_LEVEL_AREA`, достаётся этажу ниже. Пол этажа в точке — нижний
/// проходимый пол его полосы; пустоты рядом (стены, щели) берут высоту ближайшего пола: у
/// самого большого этажа — до 16 ярдов, у прочих — до 2.
pub fn level_floors(scene: &Scene, surf: &Surfaces) -> Vec<FloorMap> {
    let (w, h, step) = (surf.w, surf.h, surf.step);
    let z0 = scene.lo[2].floor();
    let n = (scene.hi[2] - z0).ceil() as usize + 2;
    let bin = |z: f32| ((z - z0).max(0.0) as usize).min(n - 1);
    let mut area = vec![0f32; n];
    for (a, &z) in surf.z.iter().enumerate() {
        if surf.walk[a] {
            area[bin(z)] += step * step;
        }
    }
    // Пики — высоты, где много пола (сглажено по трём ярдам, не меньше 8 % самого большого,
    // друг от друга не ближе 3 ярдов); граница этажей — самый пустой ярд между соседними пиками:
    // лестница между этажами делится пополам, а не склеивает их.
    let smooth: Vec<f32> = (0..n).map(|i| (i.saturating_sub(1)..=(i + 1).min(n - 1)).map(|j| area[j]).sum::<f32>()).collect();
    let max = smooth.iter().cloned().fold(0.0, f32::max);
    let mut peaks: Vec<usize> = (0..n).filter(|&i| smooth[i] >= max * 0.08 && (i == 0 || smooth[i] >= smooth[i - 1]) && (i + 1 == n || smooth[i] > smooth[i + 1])).collect();
    let mut k = 1;
    while k < peaks.len() {
        if peaks[k] - peaks[k - 1] < 3 {
            let drop = if smooth[peaks[k]] > smooth[peaks[k - 1]] { k - 1 } else { k };
            peaks.remove(drop);
        } else {
            k += 1;
        }
    }
    let mut bands: Vec<[usize; 2]> = Vec::new();
    let mut from = 0;
    for k in 0..peaks.len() {
        let to = match peaks.get(k + 1) {
            Some(&next) => (peaks[k]..next).min_by(|&a, &b| area[a].total_cmp(&area[b])).unwrap_or(peaks[k]),
            None => n - 1,
        };
        bands.push([from, to]);
        from = to + 1;
    }
    if bands.is_empty() {
        bands.push([0, n - 1]);
    }
    // Клетки, где у полосы есть пол.
    let cover = |b: [usize; 2]| -> Vec<bool> { (0..w * h).map(|k| surf.cell(k).any(|a| surf.walk[a] && (b[0]..=b[1]).contains(&bin(surf.z[a])))).collect() };
    let mut merged: Vec<[usize; 2]> = vec![bands[0]];
    for &b in &bands[1..] {
        let last = *merged.last().unwrap();
        let (ca, cb) = (cover(last), cover(b));
        let both = ca.iter().zip(&cb).filter(|(x, y)| **x && **y).count();
        let (na, nb) = (ca.iter().filter(|x| **x).count(), cb.iter().filter(|x| **x).count());
        let small = nb as f32 * step * step < MIN_LEVEL_AREA;
        if small || (both as f32) < 0.1 * na.min(nb).max(1) as f32 {
            merged.last_mut().unwrap()[1] = b[1];
        } else {
            merged.push(b);
        }
    }
    let areas: Vec<usize> = merged.iter().map(|&b| cover(b).iter().filter(|x| **x).count()).collect();
    let main = areas.iter().enumerate().max_by_key(|(_, a)| **a).map_or(0, |(i, _)| i);
    merged
        .iter()
        .enumerate()
        .map(|(lv, &b)| {
            // Нижний проходимый пол полосы; сухой — раньше глади: тропа над лавой и мостки над
            // водой видны, гладь под ними — нет.
            let z = (0..w * h)
                .map(|k| {
                    let mut band = surf.cell(k).filter(|&a| surf.walk[a] && (b[0]..=b[1]).contains(&bin(surf.z[a])));
                    let first = band.clone().next();
                    band.find(|&a| !surf.wet[a]).or(first).map_or(f32::INFINITY, |a| surf.z[a])
                })
                .collect();
            let mut f = FloorMap { x0: surf.x0, y0: surf.y0, step, w, h, z, over: Vec::new() };
            f.fill(if lv == main { 16.0 } else { 2.0 });
            f.over = (0..w * h).map(|k| if f.z[k].is_finite() { surf.cell(k).map(|a| surf.z[a]).find(|&v| v > f.z[k] + 2.0).unwrap_or(f32::INFINITY) } else { f32::INFINITY }).collect();
            f
        })
        .collect()
}

/// Где пол этажа `lv` шагом (не выше `STEP_UP`) переходит в пол этажа `lv + 1`: пандус или
/// лестница, разрезанные границей этажей. Куски стыка — рамками в ярдах `[x0, y0, x1, y1]`.
pub fn level_links(surf: &Surfaces, floors: &[FloorMap]) -> Vec<(usize, [f32; 4])> {
    let (w, h) = (surf.w, surf.h);
    // Свой пол этажа в клетке: проходимый пол как раз этой высоты (не дорисованный соседним).
    let own = |lv: usize, k: usize| -> Option<f32> {
        let z = floors[lv].z[k];
        (z.is_finite() && surf.cell(k).any(|a| surf.walk[a] && (surf.z[a] - z).abs() < 0.01)).then_some(z)
    };
    let mut out = Vec::new();
    for lv in 0..floors.len().saturating_sub(1) {
        let mut mark = vec![false; w * h];
        for k in 0..w * h {
            let Some(za) = own(lv, k) else { continue };
            let (i, j) = (k % w, k / w);
            for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1), (0, 0)] {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if ni < 0 || nj < 0 || ni as usize >= w || nj as usize >= h {
                    continue;
                }
                let n = nj as usize * w + ni as usize;
                if own(lv + 1, n).is_some_and(|zb| (zb - za).abs() <= STEP_UP && zb != za) {
                    mark[k] = true;
                    mark[n] = true;
                }
            }
        }
        // Куски стыка — рамками: клетки ближе полутора ярдов — один кусок (лестница рвётся на
        // ступенях), кусок меньше восьми клеток (двух квадратных ярдов) — шум у кромки воды.
        const NEAR: i32 = 3;
        let mut seen = vec![false; w * h];
        for s0 in 0..w * h {
            if !mark[s0] || seen[s0] {
                continue;
            }
            seen[s0] = true;
            let mut stack = vec![s0];
            let mut b = [usize::MAX, usize::MAX, 0, 0];
            let mut count = 0;
            while let Some(k) = stack.pop() {
                let (i, j) = (k % w, k / w);
                b = [b[0].min(i), b[1].min(j), b[2].max(i), b[3].max(j)];
                count += 1;
                for dj in -NEAR..=NEAR {
                    for di in -NEAR..=NEAR {
                        let (ni, nj) = (i as i32 + di, j as i32 + dj);
                        if ni < 0 || nj < 0 || ni as usize >= w || nj as usize >= h {
                            continue;
                        }
                        let n = nj as usize * w + ni as usize;
                        if mark[n] && !seen[n] {
                            seen[n] = true;
                            stack.push(n);
                        }
                    }
                }
            }
            if count < 8 {
                continue;
            }
            let x = |i: usize| surf.x0 + i as f32 * surf.step;
            let y = |j: usize| surf.y0 + j as f32 * surf.step;
            out.push((lv, [x(b[0]), y(b[1]), x(b[2] + 1), y(b[3] + 1)]));
        }
    }
    out
}

/// Стены по краю проходимого пола этажа: где кончается пол, туда фишке нельзя. Срез на высоте
/// груди у неровной скалы рвётся на черты с просветами (пол под стеной то выше, то ниже, и
/// соседние куски среза не сходятся) — край пола сплошной. Колонны и камни в зале — кольца;
/// вода и лава — пол (дно под гладью лежит на высоте глади); пандус к соседнему этажу — не край.
pub fn floor_edges(surf: &Surfaces, floors: &[FloorMap], lv: usize, frame: Frame, opt: &Options) -> Vec<Vec<[f32; 2]>> {
    let (w, h) = (surf.w, surf.h);
    let own = |l: usize, k: usize| -> Option<f32> {
        let z = floors[l].z[k];
        (z.is_finite() && surf.cell(k).any(|a| surf.walk[a] && (surf.z[a] - z).abs() < 0.01)).then_some(z)
    };
    let mut inside: Vec<bool> = (0..w * h).map(|k| own(lv, k).is_some()).collect();
    for nb in [lv.checked_sub(1), Some(lv + 1)].into_iter().flatten().filter(|&l| l < floors.len()) {
        for (k, cell) in inside.iter_mut().enumerate() {
            let Some(zb) = own(nb, k) else { continue };
            let (i, j) = (k % w, k / w);
            let near = [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)].iter().any(|&(di, dj)| {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                ni >= 0 && nj >= 0 && (ni as usize) < w && (nj as usize) < h && own(lv, nj as usize * w + ni as usize).is_some_and(|za| (za - zb).abs() <= STEP_UP)
            });
            if near {
                *cell = true;
            }
        }
    }
    let c = opt.cells_per_yard;
    chain(&mask_edges(surf, &inside, None, frame), 0.01, 0.35 * c, 1.5 * c)
}

/// Ярус 3D-сцены — свой уровень Foundry: стены по краю его пола и (у верхних ярусов) зоны,
/// войдя в которые фишка переходит на этот уровень.
pub struct Storey {
    pub walls: Vec<Vec<[f32; 2]>>,
    pub zones: Vec<Zone>,
    /// Пол яруса по сетке поверхностей (нижний его пол в клетке; `INFINITY` — нет) — для его
    /// тайла в 3D: `storey_floor` делает из него карту пола.
    pub floor: Vec<f32>,
}

/// Зона яруса: обводы в клетках карты и высоты мира в ярдах — фишка между `low` и `high`
/// стоит на полу этого яруса, а не на улице под ним и не на этаже над ним.
pub struct Zone {
    pub rings: Vec<Vec<[f32; 2]>>,
    pub low: f32,
    pub high: f32,
}

/// Ярусы для 3D-сцены, где этажи не разделены по высоте (город на склоне). Ярус пола — сколько
/// сухих полов под ним стопкой в 2,5–10 ярдах: улица и залы — 0, второй этаж и мост над улицей
/// — 1, этаж над ним — 2. Пол глубоко под другим (выше него больше чем на 25 ярдов ещё пол:
/// земля под городом) — ничей. Из пола яруса — связные куски, по которым ходят шагом (в воду —
/// прыжком): у земли — не меньше четверти самого большого (земля в пустотах за домами — мелкие
/// островки), у верхних — не меньше 6 кв. ярдов; мелкий кусок, к которому с другого яруса можно
/// шагнуть (край балкона над пустотой, обрывок моста), — часть того яруса. Стены яруса — край
/// его пола, кроме мест, где шагом переходят на другой ярус (верх лестницы, конец моста): там
/// фишка проходит и меняет уровень. Петли и «шипы» меньше 2,25 ярда (дырка в мостовой под
/// фонарём, столбик) не стены. В клетках карты, по рамке `frame`; берётся только то, что в
/// карте размером `size` клеток (с запасом в клетку: край пола за картой отрежется по рамке).
pub fn storeys(surf: &Surfaces, frame: Frame, cells_per_yard: f32, size: [f32; 2]) -> Vec<Storey> {
    const UPPER: (f32, f32) = (2.5, 10.0);
    const DEEP: f32 = 25.0;
    const MIN_UPPER: f32 = 6.0;
    const JOIN_MAX: f32 = 30.0;
    let (w, h, n) = (surf.w, surf.h, surf.z.len());
    let cell_of: Vec<usize> = (0..w * h).flat_map(|k| surf.cell(k).map(move |_| k)).collect();
    let usable: Vec<bool> = (0..n).map(|a| surf.walk[a] && !surf.cell(cell_of[a]).any(|b| surf.walk[b] && surf.z[b] > surf.z[a] + DEEP)).collect();
    // Ярус — снизу вверх по клетке (высоты в клетке по возрастанию): над ближайшим сухим полом.
    let mut layer = vec![usize::MAX; n];
    for k in 0..w * h {
        for a in surf.cell(k) {
            if !usable[a] {
                continue;
            }
            let under = surf.cell(k).filter(|&b| !surf.wet[b] && surf.walk[b] && surf.z[b] >= surf.z[a] - UPPER.1 && surf.z[b] <= surf.z[a] - UPPER.0).max_by(|&b, &c| surf.z[b].total_cmp(&surf.z[c]));
            layer[a] = under.map_or(0, |b| if layer[b] == usize::MAX { 1 } else { layer[b] + 1 });
        }
    }
    let reach = |a: usize, b: usize| step_reach(surf, a, b);
    let in_map: Vec<bool> = (0..w * h)
        .map(|k| {
            let m = frame.to_map(surf.x0 + ((k % w) as f32 + 0.5) * surf.step, surf.y0 + ((k / w) as f32 + 0.5) * surf.step);
            m[0] >= -1.0 && m[1] >= -1.0 && m[0] <= size[0] + 1.0 && m[1] <= size[1] + 1.0
        })
        .collect();
    let neighbours = |k: usize| {
        let (i, j) = ((k % w) as i64, (k / w) as i64);
        [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)].into_iter().filter_map(move |(di, dj)| {
            let (ni, nj) = (i + di, j + dj);
            (ni >= 0 && nj >= 0 && (ni as usize) < w && (nj as usize) < h).then_some(nj as usize * w + ni as usize)
        })
    };
    // Связные куски одного яруса.
    let mut comp = vec![usize::MAX; n];
    let mut sizes: Vec<usize> = Vec::new();
    let mut comp_layer: Vec<usize> = Vec::new();
    for s0 in 0..n {
        if !usable[s0] || comp[s0] != usize::MAX {
            continue;
        }
        let id = sizes.len();
        let (mut size, mut stack) = (0, vec![s0]);
        comp[s0] = id;
        while let Some(a) = stack.pop() {
            size += 1;
            for nk in neighbours(cell_of[a]) {
                for b in surf.cell(nk) {
                    if usable[b] && comp[b] == usize::MAX && layer[b] == layer[s0] && reach(a, b) {
                        comp[b] = id;
                        stack.push(b);
                    }
                }
            }
        }
        sizes.push(size);
        comp_layer.push(layer[s0]);
    }
    let big = (0..sizes.len()).filter(|&c| comp_layer[c] == 0).map(|c| sizes[c]).max().unwrap_or(0);
    let area = surf.step * surf.step;
    let mut kept: Vec<bool> = (0..sizes.len()).map(|c| if comp_layer[c] == 0 { sizes[c] * 4 >= big } else { sizes[c] as f32 * area >= MIN_UPPER }).collect();
    // Мелкие (до 30 кв. ярдов) куски, к которым шагают с принятого куска другого яруса, — в тот
    // ярус (волной). Крупный отброшенный кусок (земля под городом) так и остаётся ничьим.
    let tiny: Vec<bool> = sizes.iter().map(|&s| (s as f32 * area) < JOIN_MAX).collect();
    // Чей кусок: приклеенный — того, к кому приклеен (у них одна зона).
    let mut root: Vec<usize> = (0..sizes.len()).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for a in 0..n {
            let ca = comp[a];
            if ca == usize::MAX || !kept[ca] {
                continue;
            }
            for nk in neighbours(cell_of[a]).chain([cell_of[a]]) {
                for b in surf.cell(nk) {
                    let cb = comp[b];
                    if cb != usize::MAX && !kept[cb] && tiny[cb] && reach(a, b) {
                        kept[cb] = true;
                        comp_layer[cb] = comp_layer[ca];
                        root[cb] = root[ca];
                        changed = true;
                    }
                }
            }
        }
    }
    let layer_of = |a: usize| (comp[a] != usize::MAX && kept[comp[a]]).then(|| comp_layer[comp[a]]);
    let top = (0..n).filter(|&a| in_map[cell_of[a]]).filter_map(layer_of).max().unwrap_or(0);
    let c = cells_per_yard;
    let small = |l: &Vec<[f32; 2]>| {
        let (lo, hi) = l.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
        (hi[0] - lo[0]).max(hi[1] - lo[1]) < 2.25 * c
    };
    (0..=top)
        .map(|lv| {
            let inside: Vec<bool> = (0..w * h).map(|k| in_map[k] && surf.cell(k).any(|a| layer_of(a) == Some(lv))).collect();
            // Проход на другой ярус: в клетке вне яруса пол другого яруса, на который шагают с
            // пола этого в соседней клетке.
            let open: Vec<bool> = (0..w * h)
                .map(|k| !inside[k] && surf.cell(k).any(|b| layer_of(b).is_some_and(|l| l != lv) && neighbours(k).any(|nk| inside[nk] && surf.cell(nk).any(|a| layer_of(a) == Some(lv) && reach(a, b)))))
                .collect();
            let walls = chain(&mask_edges(surf, &inside, Some(&open), frame), 0.01, 0.35 * c, 1.5 * c).into_iter().filter(|l| !small(l)).collect();
            let own = |a: usize| (comp[a] != usize::MAX && kept[comp[a]]).then(|| (root[comp[a]], comp_layer[comp[a]]));
            let zones = if lv == 0 { Vec::new() } else { storey_zones(surf, frame, &own, &in_map, lv, c, size) };
            let floor = (0..w * h).map(|k| if in_map[k] { surf.cell(k).filter(|&a| layer_of(a) == Some(lv)).map(|a| surf.z[a]).fold(f32::INFINITY, f32::min) } else { f32::INFINITY }).collect();
            Storey { walls, zones, floor }
        })
        .collect()
}

/// Шагом или прыжком в воду с поверхности `a` на `b` в соседней клетке.
fn step_reach(surf: &Surfaces, a: usize, b: usize) -> bool {
    let dz = (surf.z[b] - surf.z[a]).abs();
    dz <= STEP_UP || (surf.wet[a] || surf.wet[b]) && dz <= JUMP_IN
}

/// Зоны верхнего яруса: по зоне на кусок — клетки куска и ещё клетка вокруг (фишку у стены
/// останавливает стена, а не край зоны), низ на 0,75 ярда под его полом (сухой пол под ярусом —
/// не выше чем на 2,5 ниже), верх на 1,4 над ним (следующий ярус — не ниже чем на 2,5). Если в
/// клетках куска на этой высоте есть чужой пол (у лестницы на второй этаж верх выше пола зала
/// под ней), кусок режется на полосы высоты по ярду. Только в рамке карты `size` (клетки).
fn storey_zones(surf: &Surfaces, frame: Frame, own: &dyn Fn(usize) -> Option<(usize, usize)>, in_map: &[bool], lv: usize, c: f32, size: [f32; 2]) -> Vec<Zone> {
    const BAND: f32 = 1.0;
    let (w, h) = (surf.w, surf.h);
    // Кусок яруса → (клетка, поверхность).
    let mut parts: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for k in (0..w * h).filter(|&k| in_map[k]) {
        for a in surf.cell(k) {
            if let Some((ca, l)) = own(a) {
                if l == lv {
                    parts.entry(ca).or_default().push((k, a));
                }
            }
        }
    }
    let mut ids: Vec<usize> = parts.keys().copied().collect();
    ids.sort_unstable();
    let mut out = Vec::new();
    for ca in ids {
        let cells = &parts[&ca];
        let zmin = cells.iter().map(|&(_, a)| surf.z[a]).fold(f32::MAX, f32::min);
        let zmax = cells.iter().map(|&(_, a)| surf.z[a]).fold(f32::MIN, f32::max);
        let foreign = cells.iter().any(|&(k, a)| surf.cell(k).any(|b| own(b).is_none_or(|(cb, _)| cb != ca) && surf.walk[b] && surf.z[b] >= zmin - 0.75 && surf.z[b] <= zmax + 1.4 && !step_reach(surf, a, b)));
        let band = |z: f32| if foreign { ((z - zmin) / BAND).floor() as i32 } else { 0 };
        let mut groups: Vec<(i32, Vec<bool>, f32, f32)> = Vec::new();
        for &(k, a) in cells {
            let b = band(surf.z[a]);
            let g = match groups.iter().position(|g| g.0 == b) {
                Some(i) => i,
                None => {
                    groups.push((b, vec![false; w * h], f32::MAX, f32::MIN));
                    groups.len() - 1
                }
            };
            let g = &mut groups[g];
            g.1[k] = true;
            g.2 = g.2.min(surf.z[a]);
            g.3 = g.3.max(surf.z[a]);
        }
        groups.sort_by_key(|g| g.0);
        for (_, mask, lo, hi) in groups {
            let grown: Vec<bool> = (0..w * h)
                .map(|k| {
                    let (i, j) = ((k % w) as i64, (k / w) as i64);
                    (-1..=1i64).any(|dj| {
                        (-1..=1i64).any(|di| {
                            let (ni, nj) = (i + di, j + dj);
                            ni >= 0 && nj >= 0 && (ni as usize) < w && (nj as usize) < h && mask[nj as usize * w + ni as usize]
                        })
                    })
                })
                .collect();
            let rings: Vec<Vec<[f32; 2]>> = chain(&mask_edges(surf, &grown, None, frame), 0.01, 0.35 * c, 0.0)
                .into_iter()
                .map(|mut r| {
                    if r.len() > 2 && r.first() == r.last() {
                        r.pop();
                    }
                    r.iter().map(|p| [p[0].clamp(0.0, size[0]), p[1].clamp(0.0, size[1])]).collect::<Vec<_>>()
                })
                .filter(|r: &Vec<[f32; 2]>| {
                    let twice: f32 = (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum();
                    r.len() >= 3 && twice.abs() / 2.0 >= 0.1
                })
                .collect();
            if !rings.is_empty() {
                out.push(Zone { rings, low: lo - 0.75, high: hi + 1.4 });
            }
        }
    }
    out
}

/// Край области клеток сетки поверхностей (`inside`) отрезками в клетках карты: квадраты по
/// четырём серединам клеток, край — между серединами сторон.
fn mask_edges(surf: &Surfaces, inside: &[bool], open: Option<&[bool]>, frame: Frame) -> Vec<([f32; 2], [f32; 2])> {
    let (w, h) = (surf.w, surf.h);
    let at = |i: i64, j: i64| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && inside[j as usize * w + i as usize];
    let pt = |i: f32, j: f32| frame.to_map(surf.x0 + (i + 0.5) * surf.step, surf.y0 + (j + 0.5) * surf.step);
    let mut segs: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for j in -1..h as i64 {
        for i in -1..w as i64 {
            let code = at(i, j) as u8 | (at(i + 1, j) as u8) << 1 | (at(i + 1, j + 1) as u8) << 2 | (at(i, j + 1) as u8) << 3;
            if code == 0 || code == 15 {
                continue;
            }
            // Квадрат у прохода на другой ярус — без края.
            let passage = |i: i64, j: i64| i >= 0 && j >= 0 && (i as usize) < w && (j as usize) < h && open.is_some_and(|o| o[j as usize * w + i as usize]);
            if passage(i, j) || passage(i + 1, j) || passage(i, j + 1) || passage(i + 1, j + 1) {
                continue;
            }
            let (fi, fj) = (i as f32, j as f32);
            let (top, right, bottom, left) = (pt(fi + 0.5, fj), pt(fi + 1.0, fj + 0.5), pt(fi + 0.5, fj + 1.0), pt(fi, fj + 0.5));
            match code {
                1 | 14 => segs.push((left, top)),
                2 | 13 => segs.push((top, right)),
                3 | 12 => segs.push((left, right)),
                4 | 11 => segs.push((right, bottom)),
                6 | 9 => segs.push((top, bottom)),
                7 | 8 => segs.push((left, bottom)),
                5 => segs.extend([(left, top), (right, bottom)]),
                10 => segs.extend([(top, right), (left, bottom)]),
                _ => {}
            }
        }
    }
    segs
}

/// Высота пола этажа по сетке (`INFINITY` — пола нет).
pub struct FloorMap {
    pub x0: f32,
    pub y0: f32,
    pub step: f32,
    pub w: usize,
    pub h: usize,
    pub z: Vec<f32>,
    /// Следующий пол над полом этажа (`INFINITY` — нет): вода глубже среза рисуется до него —
    /// дно озера ниже глади на 5–10 ярдов, а срез — 3 ярда над дном.
    pub over: Vec<f32>,
}

/// Карта пола верхнего яруса (`z` — по сетке поверхностей, как `Storey::floor`): пустоты рядом
/// с полом (стены) — высотой ближайшего пола до 2 ярдов, над полом — следующий пол.
pub fn storey_floor(surf: &Surfaces, z: Vec<f32>) -> FloorMap {
    let (w, h) = (surf.w, surf.h);
    let mut f = FloorMap { x0: surf.x0, y0: surf.y0, step: surf.step, w, h, z, over: Vec::new() };
    f.fill(2.0);
    f.over = (0..w * h).map(|k| if f.z[k].is_finite() { surf.cell(k).map(|a| surf.z[a]).find(|&v| v > f.z[k] + 2.0).unwrap_or(f32::INFINITY) } else { f32::INFINITY }).collect();
    f
}

impl FloorMap {
    pub fn at(&self, x: f32, y: f32) -> Option<f32> {
        let i = ((x - self.x0) / self.step).floor();
        let j = ((y - self.y0) / self.step).floor();
        if i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h {
            return None;
        }
        let z = self.z[j as usize * self.w + i as usize];
        z.is_finite().then_some(z)
    }

    /// Следующий пол над полом этажа в точке.
    pub fn over_at(&self, x: f32, y: f32) -> f32 {
        let i = ((x - self.x0) / self.step).floor();
        let j = ((y - self.y0) / self.step).floor();
        if i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h {
            return f32::INFINITY;
        }
        self.over.get(j as usize * self.w + i as usize).copied().unwrap_or(f32::INFINITY)
    }

    /// Есть ли пол хоть где-то в прямоугольнике (ярды).
    pub fn any_in(&self, xa: f32, xb: f32, ya: f32, yb: f32) -> bool {
        let ci = |v: f32, o: f32, n: usize| (((v - o) / self.step).floor().max(0.0) as usize).min(n.saturating_sub(1));
        let (i0, i1, j0, j1) = (ci(xa, self.x0, self.w), ci(xb, self.x0, self.w), ci(ya, self.y0, self.h), ci(yb, self.y0, self.h));
        (j0..=j1).any(|j| (i0..=i1).any(|i| self.z[j * self.w + i].is_finite()))
    }

    /// Пустые клетки рядом с полом — высотой ближайшего пола (волной по соседям), до `reach` ярдов.
    fn fill(&mut self, reach: f32) {
        let (w, h) = (self.w, self.h);
        let steps = (reach / self.step).ceil() as usize;
        let mut front: Vec<usize> = (0..w * h).filter(|&k| self.z[k].is_finite()).collect();
        for _ in 0..steps {
            let mut next = Vec::new();
            let snapshot = self.z.clone();
            for &k in &front {
                let (i, j) = (k % w, k / w);
                for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ni, nj) = (i as i32 + di, j as i32 + dj);
                    if ni < 0 || nj < 0 || ni as usize >= w || nj as usize >= h {
                        continue;
                    }
                    let nk = nj as usize * w + ni as usize;
                    if !self.z[nk].is_finite() {
                        self.z[nk] = snapshot[k];
                        next.push(nk);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            front = next;
        }
    }
}

/// Перевод координат игры в клетки карты: север (+X) — вверх, запад (+Y) — влево.
#[derive(Clone, Copy)]
pub struct Frame {
    pub hi_x: f32,
    pub hi_y: f32,
    pub cpy: f32,
    pub margin: f32,
}

impl Frame {
    pub fn to_map(self, x: f32, y: f32) -> [f32; 2] {
        [(self.hi_y - y) * self.cpy + self.margin, (self.hi_x - x) * self.cpy + self.margin]
    }
    pub fn to_world(self, u: f32, v: f32) -> [f32; 2] {
        [self.hi_x - (v - self.margin) / self.cpy, self.hi_y - (u - self.margin) / self.cpy]
    }
}

/// Корзины треугольников по 8 ярдов: какие могут попасть в кусок.
struct Bins {
    x0: f32,
    y0: f32,
    size: f32,
    w: usize,
    h: usize,
    cells: Vec<Vec<(u32, u32)>>,
}

impl Bins {
    fn new(scene: &Scene) -> Self {
        let size = 8.0;
        let (x0, y0) = (scene.lo[0], scene.lo[1]);
        let w = ((scene.hi[0] - x0) / size).ceil() as usize + 1;
        let h = ((scene.hi[1] - y0) / size).ceil() as usize + 1;
        let mut cells = vec![Vec::new(); w * h];
        for (g, gr) in scene.groups.iter().enumerate() {
            for t in 0..gr.tris.len() {
                if !gr.renders(t) {
                    continue;
                }
                let p = scene.tri(g, t);
                let (a0, a1) = (p.iter().map(|v| v[0]).fold(f32::MAX, f32::min), p.iter().map(|v| v[0]).fold(f32::MIN, f32::max));
                let (b0, b1) = (p.iter().map(|v| v[1]).fold(f32::MAX, f32::min), p.iter().map(|v| v[1]).fold(f32::MIN, f32::max));
                for j in ((b0 - y0) / size) as usize..=(((b1 - y0) / size) as usize).min(h - 1) {
                    for i in ((a0 - x0) / size) as usize..=(((a1 - x0) / size) as usize).min(w - 1) {
                        cells[j * w + i].push((g as u32, t as u32));
                    }
                }
            }
        }
        Self { x0, y0, size, w, h, cells }
    }

    fn in_rect(&self, xa: f32, xb: f32, ya: f32, yb: f32) -> Vec<(u32, u32)> {
        let ci = |v: f32, o: f32, n: usize| (((v - o) / self.size).floor().max(0.0) as usize).min(n - 1);
        let mut out = Vec::new();
        for j in ci(ya, self.y0, self.h)..=ci(yb, self.y0, self.h) {
            for i in ci(xa, self.x0, self.w)..=ci(xb, self.x0, self.w) {
                out.extend_from_slice(&self.cells[j * self.w + i]);
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

const LIGHT: [f32; 3] = [0.42, 0.42, 0.80];

/// Цвет местности в точке куска (`u`, `v` — доли куска): слои текстур (повтор — 8 на кусок) по
/// маскам с плавным переходом между точками маски, в тени — темнее.
fn terrain_color(scene: &Scene, p: &adt::Paint, u: f32, v: f32) -> [f32; 4] {
    let mut c = [0.45, 0.42, 0.36, 1.0];
    // Маска 64×64 — по серединам точек, между ними — плавно.
    let (fx, fy) = ((u * 64.0 - 0.5).clamp(0.0, 63.0), (v * 64.0 - 0.5).clamp(0.0, 63.0));
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(63), (y0 + 1).min(63));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |m: &[u8]| {
        let g = |x: usize, y: usize| m.get(y * 64 + x).copied().unwrap_or(0) as f32 / 255.0;
        (g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx) * (1.0 - ty) + (g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx) * ty
    };
    for (k, (tex, alpha)) in p.layers.iter().enumerate() {
        let Some(t) = tex.and_then(|i| scene.textures.get(i)) else { continue };
        let s = t.sample(u * 8.0, v * 8.0, [true, true]);
        let a = if k == 0 { 1.0 } else { alpha.as_deref().map_or(0.0, at) };
        for q in 0..3 {
            c[q] = c[q] * (1.0 - a) + s[q] * a;
        }
    }
    if let Some(sh) = p.shadow.as_deref() {
        let dark = 1.0 - 0.3 * at(sh.iter().map(|&b| b * 255).collect::<Vec<u8>>().as_slice());
        for q in c.iter_mut().take(3) {
            *q *= dark;
        }
    }
    c
}

/// Текстура местности, которая видна в точке куска: верхний слой с маской не меньше половины.
fn terrain_layer(p: &adt::Paint, u: f32, v: f32) -> Option<usize> {
    let (mx, my) = (((u * 64.0) as usize).min(63), ((v * 64.0) as usize).min(63));
    p.layers.iter().enumerate().rev().find(|(k, (_, a))| *k == 0 || a.as_deref().is_some_and(|m| m.get(my * 64 + mx).copied().unwrap_or(0) >= 128)).and_then(|(_, (t, _))| *t)
}

/// Номер материала цветом точки: младший байт — красный, старший — зелёный.
fn id_color(id: usize) -> [f32; 4] {
    [(id % 256) as f32 / 255.0, (id / 256 % 256) as f32 / 255.0, 0.0, 1.0]
}

/// Номера карты материалов: жидкости, нет текстуры; текстуры — с `MAT_TEX`.
pub const MAT_WATER: usize = 1;
pub const MAT_MAGMA: usize = 2;
pub const MAT_SLIME: usize = 3;
pub const MAT_NONE: usize = 15;
pub const MAT_TEX: usize = 16;

/// Кусок пола `[u0, v0, u1, v1]` (клетки карты): всё, что ниже среза над полом в каждой точке;
/// `low` — сколько ярдов ниже пола ещё рисовать (у верхних этажей сквозь проёмы виден нижний
/// этаж самой карты, а не снимок). Пусто — `None`. `mat_ids` — вместо цвета номер материала
/// (`id_color`): чей пол видно в точке.
#[allow(clippy::too_many_arguments)]
fn render_tile(scene: &Scene, bins: &Bins, floor: &FloorMap, liquid: &LiquidMap, frame: Frame, rect: [f32; 4], ppc: f32, opt: &Options, low: Option<f32>, mat_ids: bool) -> Option<image::RgbaImage> {
    let ss = if mat_ids { 1 } else { opt.supersample.max(1) };
    let (w, h) = (((rect[2] - rect[0]) * ppc).round() as u32, ((rect[3] - rect[1]) * ppc).round() as u32);
    let (bw, bh) = (w * ss, h * ss);
    let k = ppc * ss as f32; // точек на клетку
    let corners = [frame.to_world(rect[0], rect[1]), frame.to_world(rect[2], rect[3])];
    let (xa, xb) = (corners[1][0].min(corners[0][0]), corners[1][0].max(corners[0][0]));
    let (ya, yb) = (corners[1][1].min(corners[0][1]), corners[1][1].max(corners[0][1]));
    if !floor.any_in(xa, xb, ya, yb) {
        return None;
    }
    let list = bins.in_rect(xa, xb, ya, yb);
    if list.is_empty() {
        return None;
    }
    let mut color = vec![[0f32; 4]; (bw * bh) as usize];
    let mut depth = vec![f32::MIN; (bw * bh) as usize];
    // Где уже лежит гладь: на шве двух групп вода одной высоты не темнеет вдвое.
    let mut wet = vec![false; (bw * bh) as usize];
    // Окно высот точки: [пол − low, пол + срез]; пола нет — точку не рисуем.
    // В воде срез — от глади: дно чаши под скалой шире глади, срез от дна открыл бы его, и вода
    // легла бы за стену пещеры.
    let window = |x: f32, y: f32| {
        floor.at(x, y).map(|f| {
            let lq = liquid.at(x, y);
            // Пол этажа здесь — гладь: дно под ней — этого этажа, как бы глубоко ни лежало.
            let lo = if lq >= f - 0.5 { f - 60.0 } else { low.map_or(f32::MIN, |l| f - l) };
            (lo, f.max(lq) + opt.cut)
        })
    };
    let light = {
        let l = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
        [LIGHT[0] / l, LIGHT[1] / l, LIGHT[2] / l]
    };
    let mut any = false;
    for (g, t) in list {
        let (g, t) = (g as usize, t as usize);
        let gr = &scene.groups[g];
        let terrain = gr.terrain.as_deref();
        let mat = gr.mat_base + gr.tri_mat[t] as usize;
        let m = if terrain.is_some() { None } else { scene.materials.get(mat) };
        let blend = m.map_or(0, |m| m.blend);
        if blend > 2 {
            continue;
        }
        let ids = gr.tris[t].map(|i| i as usize);
        let p = ids.map(|i| gr.verts[i]);
        let s = p.map(|v| {
            let [u, vv] = frame.to_map(v[0], v[1]);
            [(u - rect[0]) * k, (vv - rect[1]) * k]
        });
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let x0 = s.iter().map(|v| v[0]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let x1 = (s.iter().map(|v| v[0]).fold(f32::MIN, f32::max).ceil() as u32).min(bw);
        let y0 = s.iter().map(|v| v[1]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
        let y1 = (s.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil() as u32).min(bh);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let uv = ids.map(|i| gr.uvs.get(i).copied().unwrap_or([0.0, 0.0]));
        let vc = gr.colors.as_ref().map(|c| ids.map(|i| c.get(i).copied().unwrap_or([0.5, 0.5, 0.5])));
        let nrm = ids.map(|i| gr.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]));
        let tex = if terrain.is_some() { None } else { scene.mat_tex.get(mat).copied().flatten().map(|i| &scene.textures[i]) };
        let unlit = m.is_some_and(|m| m.flags & 1 != 0);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[1][1] - py) * (s[2][0] - px)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[2][1] - py) * (s[0][0] - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                    continue;
                }
                let i = (y * bw + x) as usize;
                let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                if z <= depth[i] {
                    continue;
                }
                let wx = w0 * p[0][0] + w1 * p[1][0] + w2 * p[2][0];
                let wy = w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1];
                let Some((lo, hi)) = window(wx, wy) else { continue };
                if z > hi || z < lo {
                    continue;
                }
                let u = w0 * uv[0][0] + w1 * uv[1][0] + w2 * uv[2][0];
                let v = w0 * uv[0][1] + w1 * uv[1][1] + w2 * uv[2][1];
                let mut c = match terrain {
                    Some(p) => terrain_color(scene, p, u, v),
                    None => tex.map_or([0.55, 0.55, 0.55, 1.0], |t| t.sample(u, v, [true, true])),
                };
                if blend >= 1 && c[3] < 0.5 {
                    continue;
                }
                if mat_ids {
                    let tex_id = match terrain {
                        Some(p) => terrain_layer(p, u, v),
                        None => scene.mat_tex.get(mat).copied().flatten(),
                    };
                    color[i] = id_color(tex_id.map_or(MAT_NONE, |t| MAT_TEX + t));
                    depth[i] = z;
                    any = true;
                    continue;
                }
                let shade: [f32; 3] = match (&vc, unlit) {
                    // Местность: свет по нормали, цвета вершин (`MCCV`) как есть.
                    _ if terrain.is_some() => {
                        let n = [0, 1, 2].map(|q| w0 * nrm[0][q] + w1 * nrm[1][q] + w2 * nrm[2][q]);
                        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                        let d = ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / l).max(0.0);
                        let tint = vc.as_ref().map_or([1.0; 3], |vc| [0, 1, 2].map(|q| (w0 * vc[0][q] + w1 * vc[1][q] + w2 * vc[2][q]).clamp(0.3, 2.0)));
                        [0, 1, 2].map(|q| (0.5 + 0.6 * d) * tint[q])
                    }
                    (_, true) => [1.0; 3],
                    // Запечённый свет игры: цвет вершины, вдвое (так он кладётся в игре).
                    (Some(vc), false) => [0, 1, 2].map(|q| ((w0 * vc[0][q] + w1 * vc[1][q] + w2 * vc[2][q]) * 2.0).clamp(0.25, 1.6)),
                    (None, false) => {
                        let n = [0, 1, 2].map(|q| w0 * nrm[0][q] + w1 * nrm[1][q] + w2 * nrm[2][q]);
                        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
                        let d = ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / l).max(0.0);
                        [0.55 + 0.55 * d; 3]
                    }
                };
                c = [(c[0] * shade[0]).min(1.0), (c[1] * shade[1]).min(1.0), (c[2] * shade[2]).min(1.0), 1.0];
                color[i] = c;
                depth[i] = z;
                any = true;
            }
        }
    }
    // Жидкость групп (`MLIQ`): гладь поверх дна, только там, где дно нарисовано — клетки воды
    // квадратные и выходят за стены пещеры. Гладь бывает выше среза (глубокое озеро): она
    // рисуется до следующего пола над полом этажа; чем глубже, тем темнее.
    let water_top = |x: f32, y: f32| floor.at(x, y).map(|f| (floor.over_at(x, y) - 0.3).min(f + 40.0));
    for gr in &scene.groups {
        let Some(l) = &gr.liquid else { continue };
        let kind = wmo::liquid_kind(scene.root_flags, gr.group_liquid, l.tile_kind);
        let (tint, alpha, tex) = match kind {
            wmo::LiquidKind::Water => ([0.16, 0.31, 0.40], 0.72, scene.liquid_tex[0].as_ref()),
            wmo::LiquidKind::Magma => ([0.95, 0.38, 0.08], 0.85, scene.liquid_tex[1].as_ref()),
            wmo::LiquidKind::Slime => ([0.32, 0.62, 0.16], 0.78, scene.liquid_tex[2].as_ref()),
        };
        let st = wmo::LIQUID_STEP;
        for ty in 0..l.tiles[1] {
            for tx in 0..l.tiles[0] {
                if !l.shown.get(ty * l.tiles[0] + tx).copied().unwrap_or(false) {
                    continue;
                }
                let hgt = |ix: usize, iy: usize| l.heights.get(iy * l.verts[0] + ix).copied().unwrap_or(l.corner[2]);
                let corner = |ix: usize, iy: usize| [l.corner[0] + ix as f32 * st, l.corner[1] + iy as f32 * st, hgt(ix, iy)];
                let q = [corner(tx, ty), corner(tx + 1, ty), corner(tx + 1, ty + 1), corner(tx, ty + 1)];
                for p in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    let s = p.map(|v| {
                        let [u, vv] = frame.to_map(v[0], v[1]);
                        [(u - rect[0]) * k, (vv - rect[1]) * k]
                    });
                    let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
                    if area.abs() < 1e-9 {
                        continue;
                    }
                    let x0 = s.iter().map(|v| v[0]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
                    let x1 = (s.iter().map(|v| v[0]).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(bw);
                    let y0 = s.iter().map(|v| v[1]).fold(f32::MAX, f32::min).floor().max(0.0) as u32;
                    let y1 = (s.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil().max(0.0) as u32).min(bh);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[1][1] - py) * (s[2][0] - px)) / area;
                            let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[2][1] - py) * (s[0][0] - px)) / area;
                            let w2 = 1.0 - w0 - w1;
                            if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                                continue;
                            }
                            let i = (y * bw + x) as usize;
                            let z = w0 * p[0][2] + w1 * p[1][2] + w2 * p[2][2];
                            let (wx, wy) = (w0 * p[0][0] + w1 * p[1][0] + w2 * p[2][0], w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1]);
                            let (Some((lo, _)), Some(top)) = (window(wx, wy), water_top(wx, wy)) else { continue };
                            let d = color[i];
                            if z > top || z < lo || z + 0.05 < depth[i] || d[3] <= 0.0 || (wet[i] && z < depth[i] + 0.5) {
                                continue;
                            }
                            if mat_ids {
                                color[i] = id_color(match kind {
                                    wmo::LiquidKind::Water => MAT_WATER,
                                    wmo::LiquidKind::Magma => MAT_MAGMA,
                                    wmo::LiquidKind::Slime => MAT_SLIME,
                                });
                                depth[i] = z;
                                wet[i] = true;
                                any = true;
                                continue;
                            }
                            // Глубина над нарисованным дном: мель прозрачнее, омут темнее.
                            let deep = ((z - depth[i]) / 8.0).clamp(0.0, 1.0);
                            let a = (alpha + (0.93 - alpha) * deep).min(0.93);
                            // Рисунок глади — текстура игры. Вода —
                            // рябью в своём цвете, лава и слизь — как есть, ярче.
                            let base = match tex {
                                Some(t) => {
                                    // Повтор рисунка — три клетки жидкости: мельче на карте читается крапом.
                                    let c = t.sample(wx / (st * 3.0), wy / (st * 3.0), [true, true]);
                                    match kind {
                                        wmo::LiquidKind::Water => [0, 1, 2].map(|q| tint[q] * (0.55 + 0.9 * c[q])),
                                        _ => [0, 1, 2].map(|q| (c[q] * 1.25).min(1.0)),
                                    }
                                }
                                None => tint,
                            };
                            let a = if tex.is_some() && kind != wmo::LiquidKind::Water { a.max(0.95) } else { a };
                            let t = base.map(|c| c * (1.0 - 0.35 * deep));
                            color[i] = [t[0] * a + d[0] * (1.0 - a), t[1] * a + d[1] * (1.0 - a), t[2] * a + d[2] * (1.0 - a), 1.0];
                            depth[i] = z;
                            wet[i] = true;
                            any = true;
                        }
                    }
                }
            }
        }
    }
    if !any {
        return None;
    }
    let mut img = image::RgbaImage::new(w, h);
    let n = (ss * ss) as f32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0f32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let c = color[((y * ss + dy) * bw + x * ss + dx) as usize];
                    for q in 0..3 {
                        acc[q] += c[q] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let a = acc[3] / n;
            if a <= 0.0 {
                continue;
            }
            let px = [0, 1, 2].map(|q| ((acc[q] / acc[3]).clamp(0.0, 1.0) * 255.0).round() as u8);
            img.put_pixel(x, y, image::Rgba([px[0], px[1], px[2], (a * 255.0).round() as u8]));
        }
    }
    Some(img)
}

/// Самая высокая гладь жидкости над точкой, по сетке 1 ярд (`MIN` — жидкости нет).
pub struct LiquidMap {
    x0: f32,
    y0: f32,
    w: usize,
    h: usize,
    z: Vec<f32>,
}

impl LiquidMap {
    pub fn new(scene: &Scene) -> Self {
        let (x0, y0) = (scene.lo[0] - 8.0, scene.lo[1] - 8.0);
        let w = (scene.hi[0] + 8.0 - x0).ceil() as usize + 1;
        let h = (scene.hi[1] + 8.0 - y0).ceil() as usize + 1;
        let mut z = vec![f32::MIN; w * h];
        let st = wmo::LIQUID_STEP;
        for gr in &scene.groups {
            let Some(l) = &gr.liquid else { continue };
            for ty in 0..l.tiles[1] {
                for tx in 0..l.tiles[0] {
                    if !l.shown.get(ty * l.tiles[0] + tx).copied().unwrap_or(false) {
                        continue;
                    }
                    let hgt = |ix: usize, iy: usize| l.heights.get(iy * l.verts[0] + ix).copied().unwrap_or(l.corner[2]);
                    let top = hgt(tx, ty).max(hgt(tx + 1, ty)).max(hgt(tx, ty + 1)).max(hgt(tx + 1, ty + 1));
                    let (ax, ay) = (l.corner[0] + tx as f32 * st, l.corner[1] + ty as f32 * st);
                    for j in ((ay - y0).floor().max(0.0) as usize)..((ay + st - y0).ceil() as usize).min(h) {
                        for i in ((ax - x0).floor().max(0.0) as usize)..((ax + st - x0).ceil() as usize).min(w) {
                            z[j * w + i] = z[j * w + i].max(top);
                        }
                    }
                }
            }
        }
        Self { x0, y0, w, h, z }
    }

    pub fn at(&self, x: f32, y: f32) -> f32 {
        let (i, j) = ((x - self.x0).floor(), (y - self.y0).floor());
        if i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h {
            return f32::MIN;
        }
        self.z[j as usize * self.w + i as usize]
    }
}

/// Стены: срез держащих фишку треугольников на высоте «пол + `wall_at`» (в воде — «гладь +
/// `wall_at`»), куски сшиты в ломаные и упрощены. В клетках карты.
pub fn walls(scene: &Scene, floor: &FloorMap, liquid: &LiquidMap, frame: Frame, opt: &Options) -> Vec<Vec<[f32; 2]>> {
    let mut segs: Vec<([f32; 2], [f32; 2])> = Vec::new();
    for g in 0..scene.groups.len() {
        let gr = &scene.groups[g];
        for t in 0..gr.tris.len() {
            if !gr.collides(t) {
                continue;
            }
            let p = scene.tri(g, t);
            if tri_normal(p[0], p[1], p[2]).0[2].abs() > 0.7 {
                continue;
            }
            let cx = (p[0][0] + p[1][0] + p[2][0]) / 3.0;
            let cy = (p[0][1] + p[1][1] + p[2][1]) / 3.0;
            // Где пол под водой, фишка плывёт по глади: срез — над гладью. Стена пещеры над водой
            // остаётся стеной, борт чаши под гладью — нет (в озеро можно войти, через него видно).
            let cut_at = |x: f32, y: f32| floor.at(x, y).map(|f| f.max(liquid.at(x, y)) + opt.wall_at);
            let Some(hgt) = cut_at(cx, cy) else { continue };
            let mut pts: Vec<[f32; 2]> = Vec::new();
            for e in 0..3 {
                let (a, b) = (p[e], p[(e + 1) % 3]);
                if (a[2] - hgt) * (b[2] - hgt) < 0.0 {
                    let k = (hgt - a[2]) / (b[2] - a[2]);
                    pts.push([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]);
                }
            }
            if pts.len() == 2 {
                let mid = [(pts[0][0] + pts[1][0]) / 2.0, (pts[0][1] + pts[1][1]) / 2.0];
                // Срез — у пола своего места: у края уступа пол под серединой куска иной.
                if cut_at(mid[0], mid[1]).is_some_and(|c| (c - hgt).abs() < 1.0) {
                    segs.push((frame.to_map(pts[0][0], pts[0][1]), frame.to_map(pts[1][0], pts[1][1])));
                }
            }
        }
    }
    // Кусок короче полутора ярдов — бугор пола или камешек, а не стена: на карте он лишняя черта.
    chain(&segs, 0.05 * opt.cells_per_yard, 0.12 * opt.cells_per_yard, 1.5 * opt.cells_per_yard)
}

/// Ломаная в прямоугольнике карты `[0, w] × [0, h]`: что за краем — отрезано, ломаная
/// рвётся на куски (Лиан — Барски по отрезкам).
pub fn clip_line(line: &[[f32; 2]], w: f32, h: f32) -> Vec<Vec<[f32; 2]>> {
    let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    for s in line.windows(2) {
        let (a, d) = (s[0], [s[1][0] - s[0][0], s[1][1] - s[0][1]]);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut keep = true;
        for (p, q) in [(-d[0], a[0]), (d[0], w - a[0]), (-d[1], a[1]), (d[1], h - a[1])] {
            if p == 0.0 {
                keep &= q >= 0.0;
            } else if p < 0.0 {
                t0 = t0.max(q / p);
            } else {
                t1 = t1.min(q / p);
            }
        }
        if !keep || t0 > t1 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        let (pa, pb) = ([a[0] + d[0] * t0, a[1] + d[1] * t0], [a[0] + d[0] * t1, a[1] + d[1] * t1]);
        if cur.last().is_none_or(|l| (l[0] - pa[0]).abs() > 1e-4 || (l[1] - pa[1]).abs() > 1e-4) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur = vec![pa];
        }
        cur.push(pb);
        if t1 < 1.0 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

/// Отрезки в ломаные: концы сшиваются с точностью `snap`, ломаная упрощается с допуском
/// `tol`, короче `min_len` — выбрасывается.
pub fn chain(segs: &[([f32; 2], [f32; 2])], snap: f32, tol: f32, min_len: f32) -> Vec<Vec<[f32; 2]>> {
    let key = |p: [f32; 2]| ((p[0] / snap).round() as i64, (p[1] / snap).round() as i64);
    let mut at: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        if key(s.0) == key(s.1) {
            continue;
        }
        at.entry(key(s.0)).or_default().push(i);
        at.entry(key(s.1)).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut out = Vec::new();
    let degree = |k: &(i64, i64)| at.get(k).map_or(0, Vec::len);
    // Сначала от концов (степень ≠ 2), потом замкнутые.
    let mut starts: Vec<usize> = (0..segs.len()).filter(|&i| key(segs[i].0) != key(segs[i].1)).collect();
    starts.sort_by_key(|&i| usize::from(degree(&key(segs[i].0)) == 2 && degree(&key(segs[i].1)) == 2));
    for s0 in starts {
        if used[s0] {
            continue;
        }
        used[s0] = true;
        let (mut a, mut b) = (segs[s0].0, segs[s0].1);
        if degree(&key(a)) == 2 && degree(&key(b)) != 2 {
            std::mem::swap(&mut a, &mut b);
        }
        let mut line = vec![a, b];
        loop {
            let end = *line.last().unwrap();
            let next = at.get(&key(end)).and_then(|l| l.iter().copied().find(|&i| !used[i]));
            let Some(i) = next else { break };
            used[i] = true;
            let s = segs[i];
            line.push(if key(s.0) == key(end) { s.1 } else { s.0 });
        }
        let len: f32 = line.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum();
        if len >= min_len {
            out.push(simplify(&line, tol));
        }
    }
    out
}

/// Дуглас — Пейкер.
fn simplify(pts: &[[f32; 2]], tol: f32) -> Vec<[f32; 2]> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = (dx * dx + dy * dy).sqrt();
    let dist = |p: [f32; 2]| if len < 1e-6 { ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt() } else { ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len };
    let (mut best, mut far) = (0, 0.0);
    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let d = dist(*p);
        if d > far {
            far = d;
            best = i;
        }
    }
    if far <= tol {
        return vec![a, b];
    }
    let mut left = simplify(&pts[..=best], tol);
    left.pop();
    left.extend(simplify(&pts[best..], tol));
    left
}

/// Что получилось.
pub struct Built {
    pub map: Map,
    pub levels: usize,
    pub tiles: usize,
    pub walls: usize,
    pub objects: usize,
    pub ppc: f32,
    pub notes: Vec<String>,
    /// Наборы предметов здания и взятые.
    pub sets: Vec<String>,
    pub sets_used: Vec<String>,
    /// Папка картинок пола: окно забывает прежние картинки из неё — файлы переписаны.
    pub tiles_dir: PathBuf,
}

/// Папка пола здания: своя у каждого масштаба и варианта обстановки, иначе карта того же
/// здания в другом масштабе показала бы чужие куски.
pub fn variant_dir(name: &str, opt: &Options) -> String {
    let mut s = super::words_name(name);
    let ypc = 1.0 / opt.cells_per_yard;
    if (opt.cells_per_yard - Options::default().cells_per_yard).abs() > 1e-4 {
        s.push_str(&format!("_{}yd", ((ypc * 100.0).round() / 100.0).to_string().replace('.', "_")));
    }
    if let Some(set) = &opt.set {
        s.push('_');
        s.push_str(&super::words_name(set));
    }
    s
}

/// Рамка карты здания и её размер в клетках: по проходимому полу с запасом 4 ярда (скалы снаружи
/// и дальние куски карту не раздувают), в границах геометрии. Одна на снимок пола и на 3D-модель
/// (`gltf`): по ней они ложатся клетка в клетку.
pub fn map_frame(scene: &Scene, surf: &Surfaces, cells_per_yard: f32, margin: f32, area: Option<[f32; 4]>) -> Result<(Frame, f32, f32), String> {
    const PAD: f32 = 4.0;
    let b = surf.walk_bounds().ok_or("в здании нет пола, по которому можно ходить")?;
    // Рамка мира, если задана, режет и здание: квартал большого города, а не весь город.
    let b = match area {
        Some(r) => [b[0].max(r[0] + PAD), b[1].max(r[1] + PAD), b[2].min(r[2] - PAD), b[3].min(r[3] - PAD)],
        None => b,
    };
    if b[0] >= b[2] || b[1] >= b[3] {
        return Err("в рамке area нет пола, по которому можно ходить".into());
    }
    let (lo_x, lo_y) = ((b[0] - PAD).max(scene.lo[0]), (b[1] - PAD).max(scene.lo[1]));
    let (hi_x, hi_y) = ((b[2] + PAD).min(scene.hi[0]), (b[3] + PAD).min(scene.hi[1]));
    let frame = Frame { hi_x, hi_y, cpy: cells_per_yard, margin };
    let (wc, hc) = (((hi_y - lo_y) * cells_per_yard + 2.0 * margin).ceil(), ((hi_x - lo_x) * cells_per_yard + 2.0 * margin).ceil());
    Ok((frame, wc, hc))
}

/// Собрать карту из WMO: картинки пола — в `out_tiles/<имя>/`, предметы — на полку (`out_wow`).
pub fn build(client: &Client, path: &str, out_tiles: &Path, out_wow: &Path, opt: &Options) -> Result<Built, String> {
    let scene = if is_map(path) { load_map(client, path, opt.set.as_deref(), opt.area, &opt.extra)? } else { load(client, path, opt.set.as_deref())? };
    let mut notes: Vec<String> = scene.missing.iter().take(5).cloned().collect();
    let surf = Surfaces::new(&scene);
    let floors = level_floors(&scene, &surf);
    let (frame, wc, hc) = map_frame(&scene, &surf, opt.cells_per_yard, opt.margin, opt.area)?;
    if wc > crate::map::MAX_SIDE as f32 || hc > crate::map::MAX_SIDE as f32 {
        return Err(format!("здание {wc}×{hc} клеток — больше {} по стороне", crate::map::MAX_SIDE));
    }
    let ppc = opt.ppc.min((opt.max_pixels / (wc as f64 * hc as f64)).sqrt() as f32).max(24.0).floor();
    if ppc < opt.ppc {
        notes.push(format!("пол реже: {ppc} точек на клетку (здание {wc}×{hc} клеток)"));
    }
    let bins = Bins::new(&scene);
    let liquid = LiquidMap::new(&scene);
    let dir = out_tiles.join(variant_dir(&scene.name, opt));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);

    // Пол кусками, в несколько потоков: каждый берёт следующий кусок из общего списка.
    let step = opt.tile as f32;
    let mut jobs: Vec<(usize, usize, usize, [f32; 4])> = Vec::new();
    for lv in 0..floors.len() {
        let (mut v, mut tj) = (0.0f32, 0);
        while v < hc {
            let (mut u, mut ti) = (0.0f32, 0);
            while u < wc {
                jobs.push((lv, tj, ti, [u, v, (u + step).min(wc), (v + step).min(hc)]));
                u += step;
                ti += 1;
            }
            v += step;
            tj += 1;
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: Vec<Result<Vec<(usize, PathBuf)>, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads.min(jobs.len().max(1)))
            .map(|_| {
                s.spawn(|| -> Result<Vec<(usize, PathBuf)>, String> {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(lv, tj, ti, rect)) = jobs.get(i) else { break };
                        let low = (lv > 0).then_some(1.0);
                        if let Some(img) = render_tile(&scene, &bins, &floors[lv], &liquid, frame, rect, ppc, opt, low, false) {
                            let file = dir.join(format!("Floor_L{lv}_{tj:02}_{ti:02}.webp"));
                            let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height()).encode(90.0);
                            std::fs::write(&file, &*enc).map_err(|e| format!("{}: {e}", file.display()))?;
                            out.push((i, file));
                        }
                    }
                    Ok(out)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("сбой при снимке пола".into()))).collect()
    });
    let mut pieces: Vec<(usize, PathBuf)> = Vec::new();
    for r in done {
        pieces.extend(r?);
    }
    pieces.sort_by_key(|(i, _)| *i);
    let base = Map::new(wc as u32, hc as u32);
    let mut maps: Vec<Map> = (0..floors.len()).map(|lv| if lv == 0 { base.clone() } else { base.new_level() }).collect();
    let tiles = pieces.len();
    for (i, file) in pieces {
        let (lv, _, _, rect) = jobs[i];
        // Предмет на слое пола, без тени.
        maps[lv].props.push(Prop {
            asset: file.to_string_lossy().into_owned(),
            pos: [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0],
            size: [rect[2] - rect[0], rect[3] - rect[1]],
            rot: 0.0,
            scale: 1.0,
            shadow: Some(false),
            light: None,
            canopy: None,
            layer: Some(100),
            tint: None,
            open: None,
            glow: None,
            sunk: None,
            flip: false,
            inspect: None,
        });
    }
    // Карта материалов каждого этажа (4 точки на клетку) и подписи номеров — по ним пол можно
    // переложить своими текстурами (дерево, камень, вода).
    for (lv, floor) in floors.iter().enumerate() {
        let low = (lv > 0).then_some(1.0);
        if let Some(img) = render_tile(&scene, &bins, floor, &liquid, frame, [0.0, 0.0, wc, hc], 4.0, opt, low, true) {
            let file = dir.join(format!("Materials_L{lv}.png"));
            img.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        }
    }
    let mut legend = serde_json::Map::new();
    for (id, name) in [(MAT_WATER, "water"), (MAT_MAGMA, "magma"), (MAT_SLIME, "slime"), (MAT_NONE, "none")] {
        legend.insert(id.to_string(), name.into());
    }
    for (i, name) in scene.tex_names.iter().enumerate() {
        legend.insert((MAT_TEX + i).to_string(), name.clone().into());
    }
    let file = dir.join("materials.json");
    std::fs::write(&file, serde_json::to_string_pretty(&legend).unwrap_or_default()).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut wall_count = 0;
    for (lv, floor) in floors.iter().enumerate() {
        let lines = if opt.cut_walls { walls(&scene, floor, &liquid, frame, opt) } else { floor_edges(&surf, &floors, lv, frame, opt) };
        // Рамка `area` режет здание: стены за краем карты (весь город вокруг квартала) — нет.
        for line in lines.iter().flat_map(|l| clip_line(l, wc, hc)) {
            maps[lv].walls.push(Wall { points: line, closed: false, bends: Vec::new(), low: false, style: None });
            wall_count += 1;
        }
        maps[lv].wall_style = WallStyle { paint: Paint::Color([38, 34, 30]), width: 0.12 };
    }

    // Предметы — на этаж, над чьим полом стоят не ниже пола и не выше среза (люстры у потолка —
    // нет), сверху вниз; за краем карты — нет.
    let placed: Vec<(usize, &wmo::Doodad, String)> = scene
        .doodads
        .iter()
        .filter_map(|d| {
            let wet = liquid.at(d.pos[0], d.pos[1]);
            let lv = (0..floors.len()).rev().find(|&lv| floors[lv].at(d.pos[0], d.pos[1]).is_some_and(|f| (d.pos[2] >= f - 0.3 || (wet >= f - 0.5 && d.pos[2] >= f - 60.0)) && d.pos[2] <= f.max(wet) + opt.cut + 0.5))?;
            let p = frame.to_map(d.pos[0], d.pos[1]);
            (p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= wc && p[1] <= hc).then(|| (lv, d, client.spelled(&super::m2::m2_path(&d.model))))
        })
        .collect();
    // Каждая модель — один раз на полку, в несколько потоков.
    let mut models: Vec<&str> = placed.iter().map(|(_, _, m)| m.as_str()).collect();
    models.sort_unstable_by_key(|m| m.to_lowercase());
    models.dedup_by_key(|m| m.to_lowercase());
    let sprite_opt = super::render::Options::default();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let imported: Vec<(String, Result<super::Imported, String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads.min(models.len().max(1)))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&m) = models.get(i) else { break };
                        out.push((m.to_lowercase(), super::import(client, m, out_wow, &sprite_opt)));
                    }
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    });
    let mut sprites: HashMap<String, super::Imported> = HashMap::new();
    let mut failed: Vec<String> = Vec::new();
    for (key, r) in imported {
        match r {
            Ok(sp) => {
                sprites.insert(key, sp);
            }
            Err(e) => failed.push(format!("{}: {e}", models.iter().find(|m| m.to_lowercase() == key).unwrap_or(&key.as_str()))),
        }
    }
    failed.sort();
    notes.extend(failed);
    let mut objects = 0;
    // Картинки предметов — в клетках полки (5/3 ярда); у карты в другом масштабе — во столько же раз.
    let k = opt.cells_per_yard / sprite_opt.cells_per_yard;
    // Огоньки без тела — свет на пустой картинке.
    let glow_pic = out_wow.join("Light_Source_Wow_1x1.png");
    let mut lights = 0;
    for (lv, d, model) in placed {
        let Some(sp) = sprites.get(&model.to_lowercase()) else {
            let Some((kind, color, radius)) = light_only(&model) else { continue };
            if lights == 0 && !glow_pic.exists() {
                image::RgbaImage::new(4, 4).save(&glow_pic).map_err(|e| format!("{}: {e}", glow_pic.display()))?;
            }
            maps[lv].props.push(Prop {
                asset: glow_pic.to_string_lossy().into_owned(),
                pos: frame.to_map(d.pos[0], d.pos[1]),
                size: [0.5, 0.5],
                rot: 0.0,
                scale: 1.0,
                shadow: Some(false),
                light: Some(kind),
                canopy: None,
                layer: Some(100),
                tint: None,
                open: None,
                glow: Some(GlowTune { radius: Some(radius * d.scale.clamp(0.7, 2.0)), color, animation: (kind == Glow::Arcane).then(|| "pulse".to_owned()), speed: (kind == Glow::Arcane).then_some(2), intensity: None }),
                sunk: None,
                flip: false,
                inspect: None,
            });
            lights += 1;
            continue;
        };
        let rot_deg = (180.0 - wmo::yaw(d.rot).to_degrees()).rem_euclid(360.0);
        let (s, c) = rot_deg.to_radians().sin_cos();
        let o = [sp.origin[0] * d.scale * k, sp.origin[1] * d.scale * k];
        let p = frame.to_map(d.pos[0], d.pos[1]);
        let pos = [p[0] - (o[0] * c - o[1] * s), p[1] - (o[0] * s + o[1] * c)];
        maps[lv].props.push(Prop {
            asset: sp.file.to_string_lossy().into_owned(),
            pos,
            size: sp.cells,
            rot: rot_deg,
            scale: (d.scale * k).clamp(0.05, 20.0),
            shadow: None,
            light: None,
            canopy: None,
            layer: None,
            tint: underwater_tint(liquid.at(d.pos[0], d.pos[1]) - d.pos[2]),
            open: None,
            glow: None,
            sunk: None,
            flip: false,
            inspect: None,
        });
        objects += 1;
    }
    // Переходы между этажами — невидимая «лестница» (имя делает её переходом) на обоих этажах,
    // не меньше клетки.
    let links = level_links(&surf, &floors);
    if !links.is_empty() {
        let link = dir.join("Stairs_Level_Link_1x1.png");
        image::RgbaImage::new(4, 4).save(&link).map_err(|e| format!("{}: {e}", link.display()))?;
        for (lv, r) in &links {
            let (a, b) = (frame.to_map(r[0], r[1]), frame.to_map(r[2], r[3]));
            let size = [(a[0] - b[0]).abs().max(1.0), (a[1] - b[1]).abs().max(1.0)];
            let pos = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            if pos[0] < 0.0 || pos[1] < 0.0 || pos[0] > wc || pos[1] > hc {
                continue;
            }
            for l in [*lv, lv + 1] {
                maps[l].props.push(Prop {
                    asset: link.to_string_lossy().into_owned(),
                    pos,
                    size,
                    rot: 0.0,
                    scale: 1.0,
                    shadow: Some(false),
                    light: None,
                    canopy: None,
                    layer: Some(100),
                    tint: None,
                    open: None,
                    glow: None,
                    sunk: None,
                    flip: false,
                    inspect: None,
                });
            }
        }
        notes.insert(0, format!("переходов между этажами: {}", links.len()));
    }
    if lights > 0 {
        notes.insert(0, format!("огоньков без тела — светом: {lights}"));
        notes.retain(|n| !(n.contains("нечего рисовать") && n.split(':').next().is_some_and(|m| light_only(m).is_some())));
    }
    notes.truncate(12);
    let levels = maps.len();
    let mut map = maps.remove(0);
    map.levels = maps;
    map.name = scene.name.clone();
    Ok(Built { map, levels, tiles, walls: wall_count, objects, ppc, notes, sets: scene.sets.clone(), sets_used: scene.sets_used.clone(), tiles_dir: dir })
}

/// Модель без тела, которая в игре светит (блуждающий огонёк, светящийся шар, пламя): вид света,
/// цвет и радиус в клетках по словам имени и папке `Lights`. Не свет — `None`.
pub fn light_only(model: &str) -> Option<(Glow, Option<[u8; 3]>, f32)> {
    let stem = model.rsplit(['\\', '/']).next().unwrap_or(model);
    let stem = stem.split('.').next().unwrap_or(stem);
    let words: Vec<String> = super::words_name(stem).to_lowercase().split('_').map(str::to_owned).collect();
    let has = |ws: &[&str]| ws.iter().any(|w| words.iter().any(|x| x == w));
    let in_lights = model.to_lowercase().contains("\\lights\\");
    // Радиус тусклого света в клетках настольной карты: огонёк освещает свой закуток.
    let radius = if has(&["small", "tiny"]) { 3.0 } else if has(&["large", "big", "huge"]) { 6.0 } else { 4.0 };
    if let Some(kind) = crate::map::glow_by_name(&words.join("_")) {
        return Some((kind, None, radius));
    }
    if has(&["fire", "flame", "flames", "ember", "embers"]) {
        return Some((Glow::Brazier, None, radius));
    }
    if !(in_lights || has(&["wisp", "wisps", "glow", "orb", "light", "lights"])) {
        return None;
    }
    let color = [
        ("green", [90, 225, 130]),
        ("purple", [175, 95, 235]),
        ("violet", [175, 95, 235]),
        ("blue", [90, 145, 245]),
        ("red", [235, 80, 60]),
        ("yellow", [240, 215, 110]),
        ("gold", [240, 200, 90]),
        ("orange", [245, 150, 60]),
        ("pink", [240, 120, 200]),
        ("white", [235, 240, 255]),
    ]
    .iter()
    .find(|(w, _)| has(&[w]))
    .map_or([150, 205, 255], |c| c.1);
    Some((Glow::Arcane, Some(color), radius))
}

/// Подкраска предмета на дне: под водой он в её цвете, тем гуще, чем глубже (иначе камни со
/// дна лежат на глади светлыми пятнами). Не под водой — `None`.
pub fn underwater_tint(depth: f32) -> Option<[u8; 3]> {
    if !(0.3..200.0).contains(&depth) {
        return None;
    }
    let t = (depth / 5.0).clamp(0.35, 0.85);
    let water = [60.0, 115.0, 140.0];
    Some(water.map(|c| (255.0 * (1.0 - t) + c * t).round() as u8))
}

/// Подходит ли путь для `build` (корень WMO).
pub fn is_location(path: &str) -> bool {
    wmo::is_root(path)
}

/// Папка картинок пола по умолчанию: данные программы `wow_maps`.
pub fn tiles_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("wow_maps")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_storeys_get_their_own_walls_and_zone() {
        // Улица 30×30 на 0 с домом-дырой 13…17; над её частью — галерея на 5 под потолком на 9
        // (второй этаж: под ним в 5 ярдах сухой пол). Стены земли — край улицы и дыра, без
        // галереи; у галереи — свой ярус: стены по её краю и зона на высоте её пола.
        let s = scene_of(&[
            [0.0, 0.0, 13.0, 30.0, 0.0, 0.0],
            [17.0, 0.0, 30.0, 30.0, 0.0, 0.0],
            [13.0, 0.0, 17.0, 13.0, 0.0, 0.0],
            [13.0, 17.0, 17.0, 30.0, 0.0, 0.0],
            [2.0, 2.0, 12.0, 28.0, 5.0, 5.0],
            [1.0, 1.0, 13.0, 29.0, 9.0, 9.0],
        ]);
        let surf = Surfaces::new(&s);
        let k = ((15.0 - surf.y0) / surf.step) as usize * surf.w + ((7.0 - surf.x0) / surf.step) as usize;
        assert!(surf.cell(k).any(|a| surf.walk[a] && (surf.z[a] - 5.0).abs() < 0.1), "галерея — проходимый пол");
        let frame = Frame { hi_x: 34.0, hi_y: 34.0, cpy: 1.0, margin: 0.0 };
        let st = storeys(&surf, frame, 1.0, [34.0, 34.0]);
        assert_eq!(st.len(), 2, "земля и галерея");
        let lines = &st[0].walls;
        assert_eq!(lines.len(), 2, "край улицы и дом: {lines:?}");
        assert!(st[0].zones.is_empty());
        // Галерея (мир x 2…12, y 2…28) — на карте u = 34 − y, v = 34 − x: внутри неё стен нет.
        assert!(lines.iter().flatten().all(|p| !(p[0] > 7.0 && p[0] < 31.0 && p[1] > 23.0 && p[1] < 31.0)), "стена у галереи: {lines:?}");
        // Стены яруса 1 — обвод галереи (u 6…32, v 22…32).
        let up = &st[1].walls;
        assert!(!up.is_empty());
        assert!(up.iter().flatten().all(|p| p[0] > 5.0 && p[0] < 33.0 && p[1] > 21.0 && p[1] < 33.0), "стены галереи: {up:?}");
        let len: f32 = up.iter().flat_map(|l| l.windows(2)).map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt()).sum();
        assert!((len - 72.0).abs() < 6.0, "обвод 26×10: {len}");
        // Зона — одна, от чуть ниже пола галереи до чуть выше, ниже потолка и выше улицы.
        assert_eq!(st[1].zones.len(), 1);
        let z = &st[1].zones[0];
        assert!(z.low > 0.5 && z.low < 5.0 && z.high > 5.0 && z.high < 9.0, "зона {}…{}", z.low, z.high);
    }

    #[test]
    fn a_bridge_opens_onto_its_terraces() {
        // Две террасы на 5 (мир x 0…10 и 20…30), между ними улица на 0, над улицей мост на 5
        // (y 12…18). Мост — ярус 1: стены вдоль его бортов, а у концов, где шагают на террасы,
        // — нет (там фишка меняет уровень).
        let s = scene_of(&[
            [0.0, 0.0, 10.0, 30.0, 5.0, 5.0],
            [20.0, 0.0, 30.0, 30.0, 5.0, 5.0],
            [10.0, 0.0, 20.0, 30.0, 0.0, 0.0],
            [10.0, 12.0, 20.0, 18.0, 5.0, 5.0],
        ]);
        let surf = Surfaces::new(&s);
        let frame = Frame { hi_x: 34.0, hi_y: 34.0, cpy: 1.0, margin: 0.0 };
        let st = storeys(&surf, frame, 1.0, [34.0, 34.0]);
        assert_eq!(st.len(), 2, "земля и мост");
        // На карте u = 34 − y, v = 34 − x: мост — u 16…22, v 14…24.
        let up = &st[1].walls;
        assert!(up.iter().flatten().all(|p| p[0] > 15.0 && p[0] < 23.0 && p[1] > 13.0 && p[1] < 25.0), "стены моста: {up:?}");
        let along: f32 = up.iter().flat_map(|l| l.windows(2)).map(|s| (s[1][1] - s[0][1]).abs()).sum();
        let across: f32 = up.iter().flat_map(|l| l.windows(2)).map(|s| (s[1][0] - s[0][0]).abs()).sum();
        assert!(along > 15.0, "борта моста: {up:?}");
        assert!(across < 1.5, "поперёк моста у концов стен нет: {up:?}");
        assert_eq!(st[1].zones.len(), 1);
        let z = &st[1].zones[0];
        assert!(z.low > 1.0 && z.low < 5.0 && z.high > 5.0, "зона {}…{}", z.low, z.high);
        // Зона — над мостом, а не над улицей у моста: обвод не дальше клетки от моста.
        assert!(z.rings.iter().flatten().all(|p| p[0] > 15.0 && p[0] < 23.0), "зона {:?}", z.rings);
    }

    #[test]
    fn turned_water_stays_flat_at_its_shore() {
        // Пруд 6×6 клеток посреди сетки 12×12; узлы вне пруда — 0 (у Штормграда так у каналов),
        // гладь — на −6,48. Здание повёрнуто на 30°: углы клеток мира у берега ближе к пустым узлам.
        let n = 12usize;
        let pond = |i: usize, j: usize| (3..9).contains(&i) && (3..9).contains(&j);
        let heights = (0..(n + 1) * (n + 1))
            .map(|k| {
                let (i, j) = (k % (n + 1), k / (n + 1));
                let touches = (i.saturating_sub(1)..=i.min(n - 1)).any(|a| (j.saturating_sub(1)..=j.min(n - 1)).any(|b| pond(a, b)));
                if touches { -6.48 } else { 0.0 }
            })
            .collect();
        let shown = (0..n * n).map(|k| pond(k % n, k / n)).collect();
        let l = wmo::Liquid { corner: [0.0, 0.0, -6.48], verts: [n + 1, n + 1], tiles: [n, n], heights, shown, tile_kind: 0 };
        let p = adt::Place { name: 0, unique: 0, pos: [17000.0, 40.0, 17000.0], rot: [0.0, 30.0, 0.0], scale: 1.0, set: 0, bounds: None };
        let w = world_liquid(&l, &p, 0);
        let mut seen = 0;
        for k in 0..w.tiles[0] * w.tiles[1] {
            if !w.shown[k] {
                continue;
            }
            seen += 1;
            let (i, j) = (k % w.tiles[0], k / w.tiles[0]);
            for (a, b) in [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)] {
                let z = w.heights[b * w.verts[0] + a];
                assert!((z - (40.0 - 6.48)).abs() < 1e-3, "клетка {i},{j}: угол {a},{b} на {z}");
            }
        }
        assert!(seen > 20, "видимых клеток {seen}");
    }

    #[test]
    fn walls_are_clipped_to_the_map() {
        // Ломаная заходит в карту 10×10 слева, идёт внутри, уходит вверх и возвращается.
        let line = [[-5.0, 5.0], [5.0, 5.0], [5.0, -5.0], [8.0, -5.0], [8.0, 5.0]];
        let parts = clip_line(&line, 10.0, 10.0);
        assert_eq!(parts, vec![vec![[0.0, 5.0], [5.0, 5.0], [5.0, 0.0]], vec![[8.0, 0.0], [8.0, 5.0]]]);
        // Целиком снаружи — ничего; целиком внутри — как есть.
        assert!(clip_line(&[[20.0, 1.0], [30.0, 1.0]], 10.0, 10.0).is_empty());
        assert_eq!(clip_line(&[[1.0, 1.0], [2.0, 2.0], [3.0, 1.0]], 10.0, 10.0), vec![vec![[1.0, 1.0], [2.0, 2.0], [3.0, 1.0]]]);
    }

    #[test]
    fn segments_chain_into_simple_lines() {
        // Квадрат из восьми кусков и короткий обрывок.
        let s = |a: [f32; 2], b: [f32; 2]| (a, b);
        let segs = vec![
            s([0.0, 0.0], [1.0, 0.0]),
            s([1.0, 0.0], [2.0, 0.0]),
            s([2.0, 0.0], [2.0, 1.0]),
            s([2.0, 2.0], [2.0, 1.0]),
            s([2.0, 2.0], [1.0, 2.0]),
            s([1.0, 2.0], [0.0, 2.0]),
            s([0.0, 2.0], [0.0, 1.0]),
            s([0.0, 1.0], [0.0, 0.0]),
            s([5.0, 5.0], [5.05, 5.0]),
        ];
        let lines = chain(&segs, 0.01, 0.05, 0.5);
        assert_eq!(lines.len(), 1);
        // Замкнутый обход: углы, без точек на серединах сторон.
        assert_eq!(lines[0].len(), 5, "{:?}", lines[0]);
    }

    /// Сцена из прямоугольников-столкновений: (x0, y0, x1, y1, z у x0, z у x1).
    fn scene_of(quads: &[[f32; 6]]) -> Scene {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for q in quads {
            let b = verts.len() as u32;
            verts.extend([[q[0], q[1], q[4]], [q[2], q[1], q[5]], [q[2], q[3], q[5]], [q[0], q[3], q[4]]]);
            tris.extend([[b, b + 1, b + 2], [b, b + 2, b + 3]]);
        }
        let n = tris.len();
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &verts {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        let group = Group { flags: 0, verts, normals: Vec::new(), uvs: Vec::new(), colors: None, tris, tri_flags: vec![0x08; n], tri_mat: vec![0xFF; n], liquid: None, group_liquid: 0, mat_base: 0, terrain: None };
        Scene { name: "t".into(), groups: vec![group], materials: Vec::new(), textures: Vec::new(), tex_names: Vec::new(), mat_tex: Vec::new(), doodads: Vec::new(), lo, hi, missing: Vec::new(), sets: Vec::new(), sets_used: Vec::new(), root_flags: 0, liquid_tex: [None, None, None], placed: Vec::new() }
    }

    #[test]
    fn roofs_over_floors_are_not_levels_but_open_slopes_are_floor() {
        // Дом 10×10 с полом на 0 и двускатной крышей 3 → 6 → 3 (31°); к свесу крыши примыкает
        // большая открытая площадка на 3 (скат с ней связан шагом); к дому ведёт открытый
        // склон 0 → 3 без пола под ним (вход в пещеру) и площадка на 3 за ним.
        let s = scene_of(&[
            [0.0, 0.0, 10.0, 10.0, 0.0, 0.0],
            [-1.0, -1.0, 5.0, 11.0, 3.0, 6.0],
            [5.0, -1.0, 11.0, 11.0, 6.0, 3.0],
            [-16.0, 0.0, -1.0, 10.0, 3.0, 3.0],
            [-10.0, 20.0, 0.0, 26.0, 0.0, 3.0],
            [0.0, 20.0, 10.0, 26.0, 3.0, 3.0],
        ]);
        let surf = Surfaces::new(&s);
        let walk_at = |x: f32, y: f32, z: f32| {
            let k = ((y - surf.y0) / surf.step) as usize * surf.w + ((x - surf.x0) / surf.step) as usize;
            surf.cell(k).any(|a| surf.walk[a] && (surf.z[a] - z).abs() < 0.5)
        };
        assert!(walk_at(2.0, 5.0, 0.0), "пол дома");
        assert!(!walk_at(2.0, 5.0, 4.2), "скат крыши — не пол");
        assert!(!walk_at(8.0, 5.0, 4.2), "и другой скат");
        assert!(walk_at(-8.0, 5.0, 3.0), "площадка у свеса — пол");
        assert!(walk_at(-5.0, 23.0, 1.5), "открытый склон без пола под ним — пол");
        assert_eq!(level_floors(&s, &surf).len(), 1);
    }

    #[test]
    fn the_material_map_names_what_lies_under_each_point() {
        // Две площадки 10×10 на 0: левая — текстура 0, правая — 1; над правой — вода.
        let mut s = scene_of(&[[0.0, 0.0, 10.0, 10.0, 0.0, 0.0], [0.0, 10.0, 10.0, 20.0, 0.0, 0.0]]);
        s.groups[0].tri_mat = vec![0, 0, 1, 1];
        s.materials = vec![wmo::Material { flags: 0, blend: 0, texture: "a.blp".into() }, wmo::Material { flags: 0, blend: 0, texture: "b.blp".into() }];
        s.mat_tex = vec![Some(0), Some(1)];
        s.textures = vec![Rgba { w: 1, h: 1, px: vec![[200, 0, 0, 255]] }, Rgba { w: 1, h: 1, px: vec![[0, 200, 0, 255]] }];
        s.tex_names = vec!["a.blp".into(), "b.blp".into()];
        let surf = Surfaces::new(&s);
        let floors = level_floors(&s, &surf);
        let bins = Bins::new(&s);
        let frame = Frame { hi_x: 10.0, hi_y: 20.0, cpy: 0.6, margin: 0.0 };
        let id_at = |img: &image::RgbaImage, x: f32, y: f32| {
            let [u, v] = frame.to_map(x, y);
            let p = img.get_pixel((u * 4.0) as u32, (v * 4.0) as u32);
            (p[3] == 255).then_some(p[0] as usize + 256 * p[1] as usize)
        };
        let img = render_tile(&s, &bins, &floors[0], &LiquidMap::new(&s), frame, [0.0, 0.0, 12.0, 6.0], 4.0, &Options::default(), None, true).unwrap();
        assert_eq!(id_at(&img, 5.0, 5.0), Some(MAT_TEX), "площадка с текстурой 0");
        assert_eq!(id_at(&img, 5.0, 15.0), Some(MAT_TEX + 1), "площадка с текстурой 1");
        s.groups[0].liquid = Some(wmo::Liquid { corner: [0.0, 0.0, 0.5], verts: [4, 4], tiles: [3, 3], heights: vec![0.5; 16], shown: vec![true; 9], tile_kind: 0 });
        let img = render_tile(&s, &Bins::new(&s), &floors[0], &LiquidMap::new(&s), frame, [0.0, 0.0, 12.0, 6.0], 4.0, &Options::default(), None, true).unwrap();
        assert_eq!(id_at(&img, 5.0, 5.0), Some(MAT_WATER), "под водой — вода");
        assert_eq!(id_at(&img, 5.0, 15.0), Some(MAT_TEX + 1), "рядом — своя текстура");
    }

    #[test]
    fn water_is_drawn_only_over_drawn_ground_and_lava_is_not_blue() {
        // Дно 10×10 на 0, над ним и далеко за ним — жидкость на 0,5 (клетки по 4,17 ярда).
        let mut s = scene_of(&[[0.0, 0.0, 10.0, 10.0, 0.0, 0.0]]);
        s.groups[0].tri_mat = vec![0; 2];
        s.materials = vec![wmo::Material { flags: 0, blend: 0, texture: String::new() }];
        s.mat_tex = vec![None];
        let (tiles, step) = ([6usize, 6usize], wmo::LIQUID_STEP);
        let liquid = |kind: u8| wmo::Liquid { corner: [-6.0, -6.0, 0.5], verts: [7, 7], tiles, heights: vec![0.5; 49], shown: vec![true; 36], tile_kind: kind };
        s.groups[0].liquid = Some(liquid(0));
        let surf = Surfaces::new(&s);
        let floors = level_floors(&s, &surf);
        let bins = Bins::new(&s);
        let opt = Options { supersample: 1, ..Default::default() };
        let frame = Frame { hi_x: 10.0 + step, hi_y: 10.0 + step, cpy: 0.6, margin: 0.0 };
        let rect = [0.0, 0.0, 12.0, 12.0];
        let img = render_tile(&s, &bins, &floors[0], &LiquidMap::new(&s), frame, rect, 8.0, &opt, None, false).unwrap();
        // Клетка карты над дном — вода; рядом, где дна нет (но клетка воды есть), — пусто.
        let at = |x: f32, y: f32| {
            let [u, v] = frame.to_map(x, y);
            *img.get_pixel((u * 8.0) as u32, (v * 8.0) as u32)
        };
        let water = at(5.0, 5.0);
        assert!(water[3] == 255 && water[2] > water[0], "вода над дном: {water:?}");
        assert_eq!(at(12.0, 5.0)[3], 0, "за дном воды нет");
        // Та же жидкость лавой (клетка 2 без флага корня) — не синяя.
        s.groups[0].liquid = Some(liquid(2));
        let img = render_tile(&s, &bins, &floors[0], &LiquidMap::new(&s), frame, rect, 8.0, &opt, None, false).unwrap();
        let [u, v] = frame.to_map(5.0, 5.0);
        let lava = img.get_pixel((u * 8.0) as u32, (v * 8.0) as u32);
        assert!(lava[0] > lava[2] + 80, "лава: {lava:?}");
    }

    #[test]
    fn deep_water_is_drawn_up_to_its_surface_and_the_basin_side_is_no_wall() {
        // Дно 10×10 на 0, крутой борт чаши у x = 10…10,5 (0 → 6 ярдов) и стена пещеры у
        // x = −0,5…0 (12 → 0): она выше глади.
        let mut s = scene_of(&[[0.0, 0.0, 10.0, 10.0, 0.0, 0.0], [10.0, 0.0, 10.5, 10.0, 0.0, 6.0], [-0.5, 0.0, 0.0, 10.0, 12.0, 0.0]]);
        s.groups[0].tri_mat = vec![0; 6];
        s.materials = vec![wmo::Material { flags: 0, blend: 0, texture: String::new() }];
        s.mat_tex = vec![None];
        let step = wmo::LIQUID_STEP;
        let pool = |z: f32| wmo::Liquid { corner: [-2.0, -2.0, z], verts: [5, 5], tiles: [4, 4], heights: vec![z; 25], shown: vec![true; 16], tile_kind: 0 };
        let surf = Surfaces::new(&s);
        let floors = level_floors(&s, &surf);
        let opt = Options { supersample: 1, ..Default::default() };
        let frame = Frame { hi_x: 10.0 + step, hi_y: 10.0 + step, cpy: 0.6, margin: 0.0 };
        let px = |s: &Scene| {
            let img = render_tile(s, &Bins::new(s), &floors[0], &LiquidMap::new(s), frame, [0.0, 0.0, 12.0, 12.0], 8.0, &opt, None, false).unwrap();
            let [u, v] = frame.to_map(5.0, 5.0);
            *img.get_pixel((u * 8.0) as u32, (v * 8.0) as u32)
        };
        s.groups[0].liquid = Some(pool(0.5));
        let shallow = px(&s);
        // Гладь на 7 — выше среза (дно + 3): озеро не сухое, а темнее мели.
        s.groups[0].liquid = Some(pool(7.0));
        let deep = px(&s);
        assert!(deep[2] > deep[0], "глубокая вода синяя: {deep:?}");
        let sum = |c: image::Rgba<u8>| c[0] as u32 + c[1] as u32 + c[2] as u32;
        assert!(sum(deep) + 20 < sum(shallow), "омут темнее мели: {deep:?} {shallow:?}");
        // Борт чаши под гладью — не стена, стена пещеры над водой — стена; без воды — обе.
        let side = |ls: &[Vec<[f32; 2]>], far: bool| ls.iter().flatten().any(|p| (frame.to_world(p[0], p[1])[0] > 5.0) == far);
        let wet = walls(&s, &floors[0], &LiquidMap::new(&s), frame, &opt);
        assert!(!side(&wet, true) && side(&wet, false), "в воде: борт чаши — не стена, стена пещеры — стена: {wet:?}");
        // Вторая группа с той же гладью внахлёст — тон тот же (шов не темнее).
        let mut two = scene_of(&[[0.0, 0.0, 10.0, 10.0, 0.0, 0.0]]);
        two.groups[0].tri_mat = vec![0; 2];
        two.materials = s.materials.clone();
        two.mat_tex = vec![None];
        two.groups[0].liquid = Some(pool(7.0));
        let one = px(&two);
        let mut g2 = two.groups[0].clone();
        g2.tris.clear();
        g2.tri_flags.clear();
        g2.tri_mat.clear();
        two.groups.push(g2);
        assert_eq!(px(&two), one, "шов двух групп воды");
        s.groups[0].liquid = None;
        let dry = walls(&s, &floors[0], &LiquidMap::new(&s), frame, &opt);
        assert!(side(&dry, true) && side(&dry, false), "без воды — обе стены: {dry:?}");
    }

    #[test]
    fn walls_follow_the_edge_of_the_floor_whole_with_pillars_as_rings() {
        // Зал 20×20 (z 0) с колонной 2×2 посередине (верх на 6, без столкновения сверху — не пол),
        // свод на 12.
        let mut s = scene_of(&[[0.0, 0.0, 9.0, 20.0, 0.0, 0.0], [11.0, 0.0, 20.0, 20.0, 0.0, 0.0], [9.0, 0.0, 11.0, 9.0, 0.0, 0.0], [9.0, 11.0, 11.0, 20.0, 0.0, 0.0], [-1.0, -1.0, 21.0, 21.0, 12.0, 12.0]]);
        s.groups[0].tri_mat = vec![0xFF; 10];
        s.groups[0].tri_flags = vec![0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0, 0];
        let surf = Surfaces::new(&s);
        let floors = level_floors(&s, &surf);
        let opt = Options::default();
        let frame = Frame { hi_x: 22.0, hi_y: 22.0, cpy: 0.6, margin: 0.0 };
        let lines = floor_edges(&surf, &floors, 0, frame, &opt);
        // Два кольца: край зала и колонна; оба замкнуты (первая точка = последней).
        assert_eq!(lines.len(), 2, "{lines:?}");
        for l in &lines {
            let (a, b) = (l[0], l[l.len() - 1]);
            assert!((a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3, "кольцо замкнуто: {l:?}");
        }
    }

    #[test]
    fn lava_over_a_bed_without_collision_is_floor_not_a_hole() {
        // Пол 20×20 на 0, посередине (x 7…13) вместо пола — дно без столкновений, над ним лава:
        // вровень с полом и на 2,5 ярда ниже (в неё спрыгивают). Свод на 12.
        for (bed, lava) in [(-2.0f32, -0.3f32), (-4.0, -2.5)] {
            let mut s = scene_of(&[[0.0, 0.0, 7.0, 20.0, 0.0, 0.0], [13.0, 0.0, 20.0, 20.0, 0.0, 0.0], [7.0, 0.0, 13.0, 20.0, bed, bed], [-1.0, -1.0, 21.0, 21.0, 12.0, 12.0]]);
            s.groups[0].tri_mat = vec![0, 0, 0, 0, 0, 0, 0xFF, 0xFF];
            s.groups[0].tri_flags = vec![0x08, 0x08, 0x08, 0x08, 0, 0, 0, 0];
            s.materials = vec![wmo::Material { flags: 0, blend: 0, texture: String::new() }];
            s.mat_tex = vec![None];
            s.groups[0].liquid = Some(wmo::Liquid { corner: [6.0, -1.0, lava], verts: [3, 7], tiles: [2, 6], heights: vec![lava; 21], shown: vec![true; 12], tile_kind: 2 });
            let surf = Surfaces::new(&s);
            let floors = level_floors(&s, &surf);
            assert_eq!(floors[0].at(10.0, 10.0), Some(lava), "над лавой пол — гладь");
            let frame = Frame { hi_x: 22.0, hi_y: 22.0, cpy: 0.6, margin: 0.0 };
            let lines = floor_edges(&surf, &floors, 0, frame, &Options::default());
            assert_eq!(lines.len(), 1, "лава на {lava}: одно кольцо по краю зала, лава — не дыра: {lines:?}");
        }
    }

    #[test]
    fn a_path_over_lava_is_the_floor_and_the_lava_beside_it_too() {
        // Зал 20×20: дно без столкновений на −6, лава на −4 по всему залу, тропа x 8…12 на 0
        // поперёк; к лаве с тропы спрыгивают. Свод на 12.
        let mut s = scene_of(&[[0.0, 0.0, 20.0, 20.0, -6.0, -6.0], [8.0, 0.0, 12.0, 20.0, 0.0, 0.0], [-1.0, -1.0, 21.0, 21.0, 12.0, 12.0]]);
        s.groups[0].tri_mat = vec![0, 0, 0, 0, 0xFF, 0xFF];
        s.groups[0].tri_flags = vec![0, 0, 0x08, 0x08, 0, 0];
        s.groups[0].liquid = Some(wmo::Liquid { corner: [-1.0, -1.0, -4.0], verts: [7, 7], tiles: [6, 6], heights: vec![-4.0; 49], shown: vec![true; 36], tile_kind: 2 });
        let floors = level_floors(&s, &Surfaces::new(&s));
        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0].at(10.0, 10.0), Some(0.0), "на тропе пол — тропа");
        assert_eq!(floors[0].at(3.0, 10.0), Some(-4.0), "рядом — лава");
    }

    #[test]
    fn a_ramp_between_storeys_becomes_a_level_link() {
        // Зал A (x 0…30, z 0), галерея B над его половиной (x 15…40, z 6), пандус x 40…50
        // (6 → 0) вниз к площадке C (x 50…60, z 0); свод на 20 — видимый, без столкновений.
        let mut s = scene_of(&[
            [0.0, 0.0, 30.0, 30.0, 0.0, 0.0],
            [15.0, 0.0, 40.0, 30.0, 6.0, 6.0],
            [40.0, 0.0, 50.0, 30.0, 6.0, 0.0],
            [50.0, 0.0, 60.0, 30.0, 0.0, 0.0],
            [-1.0, -1.0, 61.0, 31.0, 20.0, 20.0],
        ]);
        s.groups[0].tri_mat = vec![0xFF; 10];
        s.groups[0].tri_flags = vec![0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0, 0];
        let surf = Surfaces::new(&s);
        let floors = level_floors(&s, &surf);
        assert_eq!(floors.len(), 2, "зал и галерея над ним — два этажа");
        let links = level_links(&surf, &floors);
        assert_eq!(links.len(), 1, "один переход — на пандусе: {links:?}");
        let r = links[0].1;
        assert!(r[0] > 39.0 && r[2] < 51.0, "переход на пандусе: {r:?}");
    }

    #[test]
    fn an_asked_for_ship_deck_over_its_hold_is_floor() {
        // Берег 60×60 на 0; поодаль корабль: трюм 20×20 на 1 и палуба над ним на 6, открыта небу.
        let s0 = scene_of(&[[0.0, 0.0, 60.0, 60.0, 0.0, 0.0], [80.0, 0.0, 100.0, 20.0, 1.0, 1.0], [80.0, 0.0, 100.0, 20.0, 6.0, 6.0]]);
        let plain = level_floors(&s0, &Surfaces::new(&s0));
        assert!(plain.iter().all(|f| f.at(90.0, 10.0) != Some(6.0)), "без метки палуба — крыша");
        let mut s = s0;
        s.groups[0].flags |= DECK;
        let floors = level_floors(&s, &Surfaces::new(&s));
        assert!(floors.iter().any(|f| f.at(90.0, 10.0) == Some(6.0)), "с меткой палуба — пол");
    }

    #[test]
    fn a_deck_over_open_water_is_floor_not_a_roof() {
        // Бухта под открытым небом: дно на −6, вода на 0 по всей бухте, палуба 10×10 на 3 над
        // водой (как у корабля), берег на 0.
        let mut s = scene_of(&[[0.0, 0.0, 30.0, 30.0, -6.0, -6.0], [10.0, 10.0, 20.0, 20.0, 3.0, 3.0], [30.0, 0.0, 40.0, 30.0, 0.0, 0.0]]);
        s.groups[0].liquid = Some(wmo::Liquid { corner: [-1.0, -1.0, 0.0], verts: [9, 9], tiles: [8, 8], heights: vec![0.0; 81], shown: vec![true; 64], tile_kind: 0 });
        let floors = level_floors(&s, &Surfaces::new(&s));
        assert!(floors.iter().any(|f| f.at(15.0, 15.0) == Some(3.0)), "палуба — пол: {:?}", floors.iter().map(|f| f.at(15.0, 15.0)).collect::<Vec<_>>());
    }

    #[test]
    fn walkway_over_a_flooded_hall_is_not_a_second_storey() {
        // Затопленный зал 30×30 (дно на 0, гладь на 6), мостки 20×20 на 7 над ним, свод на 20
        // (видимый, без столкновений: под ним мостки — не крыша).
        let mut s = scene_of(&[[0.0, 0.0, 30.0, 30.0, 0.0, 0.0], [5.0, 5.0, 25.0, 25.0, 7.0, 7.0], [-1.0, -1.0, 31.0, 31.0, 20.0, 20.0]]);
        s.groups[0].tri_mat = vec![0xFF, 0xFF, 0xFF, 0xFF, 0, 0];
        s.groups[0].tri_flags = vec![0x08, 0x08, 0x08, 0x08, 0, 0];
        let dry = level_floors(&s, &Surfaces::new(&s)).len();
        s.groups[0].liquid = Some(wmo::Liquid { corner: [-2.0, -2.0, 6.0], verts: [9, 9], tiles: [8, 8], heights: vec![6.0; 81], shown: vec![true; 64], tile_kind: 0 });
        let wet = level_floors(&s, &Surfaces::new(&s));
        assert_eq!((dry, wet.len()), (2, 1), "сухой зал с галереей — два этажа, затопленный с мостками — один");
        // Пол этажа в воде — гладь, на мостках — мостки.
        assert_eq!(wet[0].at(2.0, 2.0), Some(6.0));
        assert_eq!(wet[0].at(15.0, 15.0), Some(7.0));
    }

    #[test]
    fn rock_standing_out_of_deep_water_stays_rock() {
        // Дно 10×10 на 0, гладь на 7, над краем дна (x 8…10) — скала, верх на 8 (выше глади,
        // ниже «гладь + срез»): срез от дна (3) открыл бы под ней воду.
        let mut s = scene_of(&[[0.0, 0.0, 10.0, 10.0, 0.0, 0.0], [8.0, 0.0, 10.0, 10.0, 8.0, 8.0]]);
        s.groups[0].tri_mat = vec![0; 4];
        s.groups[0].tri_flags = vec![0x08, 0x08, 0, 0];
        s.materials = vec![wmo::Material { flags: 0, blend: 0, texture: String::new() }];
        s.mat_tex = vec![None];
        s.groups[0].liquid = Some(wmo::Liquid { corner: [-2.0, -2.0, 7.0], verts: [5, 5], tiles: [4, 4], heights: vec![7.0; 25], shown: vec![true; 16], tile_kind: 0 });
        let floors = level_floors(&s, &Surfaces::new(&s));
        let opt = Options { supersample: 1, ..Default::default() };
        let step = wmo::LIQUID_STEP;
        let frame = Frame { hi_x: 10.0 + step, hi_y: 10.0 + step, cpy: 0.6, margin: 0.0 };
        let img = render_tile(&s, &Bins::new(&s), &floors[0], &LiquidMap::new(&s), frame, [0.0, 0.0, 12.0, 12.0], 8.0, &opt, None, false).unwrap();
        let at = |x: f32, y: f32| {
            let [u, v] = frame.to_map(x, y);
            *img.get_pixel((u * 8.0) as u32, (v * 8.0) as u32)
        };
        let (rock, water) = (at(9.0, 5.0), at(4.0, 5.0));
        assert!(water[2] > water[0] + 20, "в чаше вода: {water:?}");
        assert!(rock[2] < rock[0] + 10, "скала над водой — не вода: {rock:?}");
    }

    #[test]
    fn things_on_the_bottom_take_the_colour_of_the_water() {
        assert_eq!(underwater_tint(-1.0), None, "над водой");
        assert_eq!(underwater_tint(0.1), None, "у самой глади");
        let (a, b) = (underwater_tint(1.0).unwrap(), underwater_tint(8.0).unwrap());
        assert!(a[0] > b[0] && b[2] > b[0], "глубже — гуще и синее: {a:?} {b:?}");
    }

    #[test]
    fn bodiless_glows_become_lights_by_their_names() {
        let (kind, color, r) = light_only("World\\KALIMDOR\\BLACKFATHOM\\PASSIVEDOODADS\\Lights\\BFD_WispSmallGreen.m2").unwrap();
        assert_eq!((kind, r), (Glow::Arcane, 3.0));
        let c = color.unwrap();
        assert!(c[1] > c[0] && c[1] > c[2], "зелёный: {c:?}");
        assert_eq!(light_only("World\\Generic\\Human\\Passive Doodads\\Torches\\GeneralTorch01.m2").map(|l| l.0), Some(Glow::Brazier));
        assert!(light_only("World\\Generic\\ActiveDoodads\\InstancePortal\\InstancePortal.m2").is_none());
        assert!(light_only("World\\GENERIC\\HUMAN\\PASSIVE DOODADS\\Steam\\DeadMineSteam01.m2").is_none());
    }

    #[test]
    fn each_scale_and_set_has_its_own_floor_folder() {
        let d = Options::default();
        assert_eq!(variant_dir("Blackfathom_instance", &d), "Blackfathom_instance");
        let three = Options { cells_per_yard: 1.0 / 3.0, ..Default::default() };
        assert_eq!(variant_dir("Blackfathom_instance", &three), "Blackfathom_instance_3yd");
        let set = Options { cells_per_yard: 1.0 / 3.5, set: Some("Set_A".into()), ..Default::default() };
        assert_eq!(variant_dir("GoldshireInn", &set), "Goldshire_Inn_3_5yd_Set_A");
    }

    #[test]
    fn frame_north_is_up_and_round_trips() {
        let f = Frame { hi_x: 100.0, hi_y: 50.0, cpy: 0.6, margin: 2.0 };
        // Самая северная и западная точка — левый верхний угол (с полями).
        assert_eq!(f.to_map(100.0, 50.0), [2.0, 2.0]);
        // Южнее — ниже, восточнее (меньше Y) — правее.
        let p = f.to_map(90.0, 40.0);
        assert!(p[1] > 2.0 && p[0] > 2.0);
        let w = f.to_world(p[0], p[1]);
        assert!((w[0] - 90.0).abs() < 1e-4 && (w[1] - 40.0).abs() < 1e-4);
    }

    /// Настоящий клиент: `WOW_CLIENT=… WOW_OUT=… WMO=… cargo test wow_location_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn wow_location_real() {
        let (Ok(dir), Ok(out)) = (std::env::var("WOW_CLIENT"), std::env::var("WOW_OUT")) else { return };
        let path = std::env::var("WMO").unwrap_or_else(|_| "World\\wmo\\Dungeon\\LD_ScarletMonestary\\Monestary_Cathedral.wmo".into());
        let client = Client::open(Path::new(&dir)).unwrap();
        let t = std::time::Instant::now();
        // EXTRA="путь;x;y;z;поворот" — здание, которое ставит сервер.
        let extra: Vec<(String, [f32; 3], f32)> = std::env::var("EXTRA")
            .ok()
            .and_then(|e| {
                let v: Vec<&str> = e.split(';').collect();
                let n = |i: usize| v.get(i).and_then(|s| s.trim().parse::<f32>().ok());
                Some((v.first()?.to_string(), [n(1)?, n(2)?, n(3)?], n(4)?))
            })
            .into_iter()
            .collect();
        // AREA="x0,y0,x1,y1" — рамка мира для карты инстанса.
        let area = std::env::var("AREA").ok().and_then(|a| {
            let v: Vec<f32> = a.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
        });
        let scene = if is_map(&path) { load_map(&client, &path, None, area, &extra).unwrap() } else { load(&client, &path, None).unwrap() };
        println!("загружено {:?}: групп {}, предметов {}, рамка {:?}–{:?}; нет: {:?}", t.elapsed(), scene.groups.len(), scene.doodads.len(), scene.lo, scene.hi, scene.missing.iter().take(5).collect::<Vec<_>>());
        println!("этажей {}", level_floors(&scene, &Surfaces::new(&scene)).len());
        if let Ok(probe) = std::env::var("PROBE") {
            let surf = Surfaces::new(&scene);
            let floors = level_floors(&scene, &surf);
            for pt in probe.split(';') {
                let v: Vec<f32> = pt.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if v.len() < 2 {
                    continue;
                }
                let (i, j) = (((v[0] - surf.x0) / surf.step) as usize, ((v[1] - surf.y0) / surf.step) as usize);
                if i >= surf.w || j >= surf.h {
                    continue;
                }
                let k = j * surf.w + i;
                let cell: Vec<(f32, bool)> = surf.cell(k).map(|a| (surf.z[a], surf.walk[a])).collect();
                println!("точка {pt}: поверхности {cell:?}, полы этажей {:?}", floors.iter().map(|f| f.at(v[0], v[1])).collect::<Vec<_>>());
            }
            // Верхняя поверхность клетки: зелёная — проходимая, красная — нет; ярче — выше.
            let mut img = image::RgbImage::new(surf.w as u32, surf.h as u32);
            for k in 0..surf.w * surf.h {
                if let Some(a) = surf.cell(k).last() {
                    let t = ((surf.z[a] - scene.lo[2]) / 40.0).clamp(0.15, 1.0);
                    let c = if surf.walk[a] { [0, (255.0 * t) as u8, 0] } else { [(255.0 * t) as u8, 0, 0] };
                    img.put_pixel((k % surf.w) as u32, (k / surf.w) as u32, image::Rgb(c));
                }
            }
            img.save(Path::new(&out).join("walk.png")).unwrap();
            // Чей пол: ярко-зелёный — свой проходимый пол этажа, тёмно-зелёный — дорисованный
            // соседним (`fill`), красный — проходимый пол есть, но этажу не достался.
            for (lv, f) in floors.iter().enumerate() {
                let mut img = image::RgbImage::new(surf.w as u32, surf.h as u32);
                for k in 0..surf.w * surf.h {
                    let own = f.z[k].is_finite() && surf.cell(k).any(|a| surf.walk[a] && (surf.z[a] - f.z[k]).abs() < 0.01);
                    let walk = surf.cell(k).any(|a| surf.walk[a]);
                    let c = match (own, f.z[k].is_finite(), walk) {
                        (true, _, _) => [60, 230, 60],
                        (false, true, _) => [20, 90, 20],
                        (false, false, true) => [200, 30, 30],
                        _ => [0, 0, 0],
                    };
                    img.put_pixel((k % surf.w) as u32, (k / surf.w) as u32, image::Rgb(c));
                }
                img.save(Path::new(&out).join(format!("level_{lv}.png"))).unwrap();
            }
        }
        let opt = Options { ppc: std::env::var("PPC").ok().and_then(|v| v.parse().ok()).unwrap_or(48.0), set: std::env::var("SET").ok().filter(|s| !s.is_empty()), area, extra: extra.clone(), ..Default::default() };
        let out = Path::new(&out);
        let built = build(&client, &path, &out.join("tiles"), &out.join("wow"), &opt).unwrap();
        println!(
            "карта {}×{}, этажей {}, кусков {}, стен {}, предметов {}, {} точек/клетку, {:?}; заметки {:?}",
            built.map.width, built.map.height, built.levels, built.tiles, built.walls, built.objects, built.ppc, t.elapsed(), built.notes
        );
        std::fs::write(out.join(format!("{}.karta", scene.name)), built.map.to_json()).unwrap();
    }

    /// Материалы и жидкости здания: `WOW_CLIENT=… WMO=… cargo test wow_wmo_materials -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn wow_wmo_materials() {
        let (Ok(dir), Ok(path)) = (std::env::var("WOW_CLIENT"), std::env::var("WMO")) else { return };
        let client = Client::open(Path::new(&dir)).unwrap();
        let root = wmo::parse_root(&client.read(&path).unwrap()).unwrap();
        println!("флаги корня {:#x}", root.flags);
        for (i, m) in root.materials.iter().enumerate() {
            println!("материал {i}: смешение {}, флаги {:#x}, {}", m.blend, m.flags, m.texture);
        }
        let scene = load(&client, &path, None).unwrap();
        for (g, gr) in scene.groups.iter().enumerate() {
            let mut used: Vec<u8> = gr.tri_mat.iter().copied().filter(|&m| m != 0xFF).collect();
            used.sort_unstable();
            used.dedup();
            let liq = gr.liquid.as_ref().map(|l| (l.tile_kind, l.tiles));
            println!("группа {g}: флаги {:#x}, жидкость {} {:?}, материалы {:?}", gr.flags, gr.group_liquid, liq, used);
        }
    }

    /// Прогон по многим зданиям: `WOW_CLIENT=… WOW_OUT=… WMO_LIST=файл cargo test wow_location_sweep
    /// -- --ignored --nocapture`. В файле — пути корней WMO по строке; без `WMO_LIST` — список всех
    /// корней клиента (`roots.tsv`: путь, групп, предметов). Итог — `sweep.jsonl` и `.karta` каждого.
    #[test]
    #[ignore]
    fn wow_location_sweep() {
        let (Ok(dir), Ok(out)) = (std::env::var("WOW_CLIENT"), std::env::var("WOW_OUT")) else { return };
        let out = Path::new(&out);
        std::fs::create_dir_all(out).unwrap();
        let client = Client::open(Path::new(&dir)).unwrap();
        let Ok(list) = std::env::var("WMO_LIST") else {
            let mut rows = Vec::new();
            for p in client.list("wmo").into_iter().filter(|p| is_location(p)) {
                let row = match client.read(p).and_then(|b| wmo::parse_root(&b)) {
                    Ok(r) => format!("{p}\t{}\t{}", r.group_count, r.doodads.len()),
                    Err(e) => format!("{p}\t-1\t-1\t{e}"),
                };
                rows.push(row);
            }
            std::fs::write(out.join("roots.tsv"), rows.join("\n")).unwrap();
            println!("корней {}", rows.len());
            return;
        };
        let ppc: f32 = std::env::var("PPC").ok().and_then(|v| v.parse().ok()).unwrap_or(24.0);
        let karta = out.join("karta");
        std::fs::create_dir_all(&karta).unwrap();
        let mut log = std::fs::OpenOptions::new().create(true).append(true).open(out.join("sweep.jsonl")).unwrap();
        for path in std::fs::read_to_string(&list).unwrap().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let t = std::time::Instant::now();
            let total = load(&client, path, None).map(|s| (s.doodads.len(), s.groups.len(), s.groups.iter().filter(|g| g.flags & 0x2000 != 0).count(), s.hi[2] - s.lo[2]));
            let opt = Options { ppc, ..Default::default() };
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(&client, path, &out.join("tiles"), &out.join("wow"), &opt)));
            let secs = t.elapsed().as_secs_f32();
            let row = match res {
                Err(p) => serde_json::json!({"path": path, "panic": p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())), "secs": secs}),
                Ok(Err(e)) => serde_json::json!({"path": path, "error": e, "secs": secs}),
                Ok(Ok(b)) => {
                    let lv: Vec<serde_json::Value> = std::iter::once(&b.map)
                        .chain(b.map.levels.iter())
                        .map(|m| {
                            let tiles: Vec<&Prop> = m.props.iter().filter(|p| p.layer == Some(100)).collect();
                            serde_json::json!({"tiles": tiles.len(), "tile_cells": tiles.iter().map(|p| p.size[0] * p.size[1]).sum::<f32>(), "walls": m.walls.len(), "wall_len": m.walls.iter().map(|w| w.points.windows(2).map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt()).sum::<f32>()).sum::<f32>(), "objects": m.props.len() - tiles.len()})
                        })
                        .collect();
                    let name = crate::wow::words_name(path.rsplit(['\\', '/']).next().unwrap_or(path).trim_end_matches(".wmo"));
                    std::fs::write(karta.join(format!("{name}.karta")), b.map.to_json()).unwrap();
                    let (doodads, groups, indoor, tall) = total.clone().unwrap_or((0, 0, 0, 0.0));
                    serde_json::json!({"path": path, "name": name, "secs": secs, "size": [b.map.width, b.map.height], "ppc": b.ppc, "levels": lv, "objects": b.objects, "doodads": doodads, "groups": groups, "indoor_groups": indoor, "height": tall, "notes": b.notes, "sets": b.sets, "sets_used": b.sets_used})
                }
            };
            use std::io::Write as _;
            writeln!(log, "{row}").unwrap();
            println!("{:.1} с {path}: {}", secs, row.get("error").or(row.get("panic")).map_or_else(|| format!("этажей {}", row["levels"].as_array().map_or(0, Vec::len)), |e| e.to_string()));
        }
    }
}
