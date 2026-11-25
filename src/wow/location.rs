//! Подземелье или здание WMO целиком — картой Картографа: пол снимком сверху (как в игре,
//! потолки и крыши срезаны над полом), стены для Foundry — срезом геометрии на уровне груди,
//! предметы — объектами на своих местах. Север игры (+X) — вверх карты, запад (+Y) — влево.

use super::blp::Rgba;
use super::wmo::{self, Group};
use super::{Client, blp};
use crate::map::{Map, Paint, Prop, Wall, WallStyle};
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
}

impl Default for Options {
    fn default() -> Self {
        Self { ppc: 128.0, cells_per_yard: 0.6, cut: 3.0, wall_at: 1.2, tile: 16, supersample: 2, max_pixels: 1.6e8, margin: 2.0, set: None }
    }
}

/// Здание целиком: группы, текстуры материалов, расстановка предметов.
pub struct Scene {
    pub name: String,
    pub groups: Vec<Group>,
    pub materials: Vec<wmo::Material>,
    /// Текстуры по одному разу и номер текстуры у каждого материала.
    pub textures: Vec<Rgba>,
    pub mat_tex: Vec<Option<usize>>,
    pub doodads: Vec<wmo::Doodad>,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub missing: Vec<String>,
    /// Наборы предметов здания и какие взяты.
    pub sets: Vec<String>,
    pub sets_used: Vec<String>,
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
    let mut mat_tex = Vec::new();
    for m in &root.materials {
        let key = m.texture.to_lowercase();
        let idx = *cache.entry(key).or_insert_with(|| match client.read(&m.texture).and_then(|b| blp::decode(&b)) {
            Ok(img) => {
                textures.push(img);
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
    Ok(Scene { name, groups, materials: root.materials, textures, mat_tex, doodads, lo, hi, missing, sets, sets_used })
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
/// (под балконом — пол и балкон). `walk` — до этой поверхности можно дойти пешком от главной
/// части здания: соседние клетки, перепад не больше ступеньки. Кровля галерей, капители,
/// верхушки стен смотрят вверх, но с пола на них не попасть — это не пол.
pub struct Surfaces {
    pub x0: f32,
    pub y0: f32,
    pub step: f32,
    pub w: usize,
    pub h: usize,
    /// Начало высот клетки в `z` (длина w·h + 1).
    start: Vec<usize>,
    z: Vec<f32>,
    walk: Vec<bool>,
}

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
        let mut cells: Vec<Vec<f32>> = vec![Vec::new(); w * h];
        // У дома есть внутренние группы — полы только из них: скаты крыши снаружи пологие и
        // большие, а по ним не ходят. У подземелья все группы внутренние.
        let indoor = scene.groups.iter().any(|g| g.flags & 0x2000 != 0);
        for g in 0..scene.groups.len() {
            if indoor && scene.groups[g].flags & 0x2000 == 0 {
                continue;
            }
            for t in 0..scene.groups[g].tris.len() {
                if !passable(scene, g, t) {
                    continue;
                }
                let p = scene.tri(g, t);
                // Треугольник в клетках сетки: по центрам клеток, мелкий — хотя бы клетку середины.
                let q = p.map(|v| [(v[0] - x0) / step, (v[1] - y0) / step, v[2]]);
                let mid = [(q[0][0] + q[1][0] + q[2][0]) / 3.0, (q[0][1] + q[1][1] + q[2][1]) / 3.0, (q[0][2] + q[1][2] + q[2][2]) / 3.0];
                let mut put = |i: usize, j: usize, v: f32| {
                    if i < w && j < h {
                        cells[j * w + i].push(v);
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
        let mut start = Vec::with_capacity(w * h + 1);
        let mut z = Vec::new();
        for c in &mut cells {
            c.sort_by(f32::total_cmp);
            start.push(z.len());
            for &v in c.iter() {
                if z.len() == *start.last().unwrap() || v - *z.last().unwrap() > 0.3 {
                    z.push(v);
                }
            }
        }
        start.push(z.len());
        let mut s = Self { x0, y0, step, w, h, start, walk: vec![false; z.len()], z };
        s.find_network();
        s
    }

    fn cell(&self, k: usize) -> std::ops::Range<usize> {
        self.start[k]..self.start[k + 1]
    }

    /// Связные куски поверхностей; проходимые — самый большой и те, что не меньше его четверти
    /// (крыло, куда ведёт дверь с порогом выше ступеньки).
    fn find_network(&mut self) {
        let n = self.z.len();
        let mut comp = vec![usize::MAX; n];
        let mut sizes = Vec::new();
        let cell_of: Vec<usize> = (0..self.w * self.h).flat_map(|k| self.cell(k).map(move |_| k)).collect();
        for s0 in 0..n {
            if comp[s0] != usize::MAX {
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
                        if comp[b] == usize::MAX && (self.z[b] - self.z[a]).abs() <= STEP_UP {
                            comp[b] = id;
                            stack.push(b);
                        }
                    }
                }
            }
            sizes.push(size);
        }
        let big = sizes.iter().copied().max().unwrap_or(0);
        for a in 0..n {
            self.walk[a] = comp[a] != usize::MAX && sizes[comp[a]] * 4 >= big;
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
            let z = (0..w * h).map(|k| surf.cell(k).filter(|&a| surf.walk[a] && (b[0]..=b[1]).contains(&bin(surf.z[a]))).map(|a| surf.z[a]).next().unwrap_or(f32::INFINITY)).collect();
            let mut f = FloorMap { x0: surf.x0, y0: surf.y0, step, w, h, z };
            f.fill(if lv == main { 16.0 } else { 2.0 });
            f
        })
        .collect()
}

/// Высота пола этажа по сетке (`INFINITY` — пола нет).
pub struct FloorMap {
    pub x0: f32,
    pub y0: f32,
    pub step: f32,
    pub w: usize,
    pub h: usize,
    pub z: Vec<f32>,
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

/// Кусок пола `[u0, v0, u1, v1]` (клетки карты): всё, что ниже среза над полом в каждой точке;
/// `low` — сколько ярдов ниже пола ещё рисовать (у верхних этажей сквозь проёмы виден нижний
/// этаж самой карты, а не снимок). Пусто — `None`.
#[allow(clippy::too_many_arguments)]
fn render_tile(scene: &Scene, bins: &Bins, floor: &FloorMap, frame: Frame, rect: [f32; 4], ppc: f32, opt: &Options, low: Option<f32>) -> Option<image::RgbaImage> {
    let ss = opt.supersample.max(1);
    let (w, h) = (((rect[2] - rect[0]) * ppc).round() as u32, ((rect[3] - rect[1]) * ppc).round() as u32);
    let (bw, bh) = (w * ss, h * ss);
    let k = ppc * ss as f32; // точек на клетку
    let corners = [frame.to_world(rect[0], rect[1]), frame.to_world(rect[2], rect[3])];
    let (xa, xb) = (corners[1][0].min(corners[0][0]), corners[1][0].max(corners[0][0]));
    let (ya, yb) = (corners[1][1].min(corners[0][1]), corners[1][1].max(corners[0][1]));
    let list = bins.in_rect(xa, xb, ya, yb);
    if list.is_empty() {
        return None;
    }
    let mut color = vec![[0f32; 4]; (bw * bh) as usize];
    let mut depth = vec![f32::MIN; (bw * bh) as usize];
    // Окно высот точки: [пол − low, пол + срез]; пола нет — точку не рисуем.
    let window = |x: f32, y: f32| floor.at(x, y).map(|f| (low.map_or(f32::MIN, |l| f - l), f + opt.cut));
    let light = {
        let l = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
        [LIGHT[0] / l, LIGHT[1] / l, LIGHT[2] / l]
    };
    let mut any = false;
    for (g, t) in list {
        let (g, t) = (g as usize, t as usize);
        let gr = &scene.groups[g];
        let mat = gr.tri_mat[t] as usize;
        let m = scene.materials.get(mat);
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
        let tex = scene.mat_tex.get(mat).copied().flatten().map(|i| &scene.textures[i]);
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
                let mut c = tex.map_or([0.55, 0.55, 0.55, 1.0], |t| t.sample(u, v, [true, true]));
                if blend >= 1 && c[3] < 0.5 {
                    continue;
                }
                let shade: [f32; 3] = match (&vc, unlit) {
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
    // Вода групп (`MLIQ`): полупрозрачная гладь поверх дна, ниже среза.
    const WATER: [f32; 3] = [0.16, 0.31, 0.40];
    for gr in &scene.groups {
        let Some(l) = &gr.liquid else { continue };
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
                            let Some((lo, hi)) = window(w0 * p[0][0] + w1 * p[1][0] + w2 * p[2][0], w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1]) else { continue };
                            if z > hi || z < lo || z + 0.05 < depth[i] {
                                continue;
                            }
                            let d = color[i];
                            let a = if d[3] > 0.0 { 0.72 } else { 1.0 };
                            color[i] = [WATER[0] * a + d[0] * (1.0 - a), WATER[1] * a + d[1] * (1.0 - a), WATER[2] * a + d[2] * (1.0 - a), 1.0];
                            depth[i] = z;
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

/// Стены: срез держащих фишку треугольников на высоте «пол + `wall_at`», куски сшиты в
/// ломаные и упрощены. В клетках карты.
pub fn walls(scene: &Scene, floor: &FloorMap, frame: Frame, opt: &Options) -> Vec<Vec<[f32; 2]>> {
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
            let Some(f) = floor.at(cx, cy) else { continue };
            let hgt = f + opt.wall_at;
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
                if floor.at(mid[0], mid[1]).is_some_and(|fm| (fm + opt.wall_at - hgt).abs() < 1.0) {
                    segs.push((frame.to_map(pts[0][0], pts[0][1]), frame.to_map(pts[1][0], pts[1][1])));
                }
            }
        }
    }
    chain(&segs, 0.05 * opt.cells_per_yard, 0.12 * opt.cells_per_yard, 0.6 * opt.cells_per_yard)
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
}

/// Собрать карту из WMO: картинки пола — в `out_tiles/<имя>/`, предметы — на полку (`out_wow`).
pub fn build(client: &Client, path: &str, out_tiles: &Path, out_wow: &Path, opt: &Options) -> Result<Built, String> {
    let scene = load(client, path, opt.set.as_deref())?;
    let mut notes: Vec<String> = scene.missing.iter().take(5).cloned().collect();
    let frame = Frame { hi_x: scene.hi[0], hi_y: scene.hi[1], cpy: opt.cells_per_yard, margin: opt.margin };
    let (wc, hc) = (((scene.hi[1] - scene.lo[1]) * opt.cells_per_yard + 2.0 * opt.margin).ceil(), ((scene.hi[0] - scene.lo[0]) * opt.cells_per_yard + 2.0 * opt.margin).ceil());
    if wc > crate::map::MAX_SIDE as f32 || hc > crate::map::MAX_SIDE as f32 {
        return Err(format!("здание {wc}×{hc} клеток — больше {} по стороне", crate::map::MAX_SIDE));
    }
    let ppc = opt.ppc.min((opt.max_pixels / (wc as f64 * hc as f64)).sqrt() as f32).max(24.0).floor();
    if ppc < opt.ppc {
        notes.push(format!("пол реже: {ppc} точек на клетку (здание {wc}×{hc} клеток)"));
    }
    let surf = Surfaces::new(&scene);
    let floors = level_floors(&scene, &surf);
    let bins = Bins::new(&scene);
    let dir = out_tiles.join(super::words_name(&scene.name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Предметы: каждая модель — один раз на полку.
    let mut sprites: HashMap<String, Option<super::Imported>> = HashMap::new();
    let sprite_opt = super::render::Options::default();
    let mut maps: Vec<Map> = Vec::new();
    let (mut tiles, mut wall_count, mut objects) = (0, 0, 0);
    let base = Map::new(wc as u32, hc as u32);
    for (lv, floor) in floors.iter().enumerate() {
        let mut m = if lv == 0 { base.clone() } else { base.new_level() };
        // Пол кусками: предметы на слое пола, без тени.
        let step = opt.tile as f32;
        let (mut v, mut tj) = (0.0f32, 0);
        while v < hc {
            let (mut u, mut ti) = (0.0f32, 0);
            while u < wc {
                let rect = [u, v, (u + step).min(wc), (v + step).min(hc)];
                let low = (lv > 0).then_some(1.0);
                if let Some(img) = render_tile(&scene, &bins, floor, frame, rect, ppc, opt, low) {
                    let file = dir.join(format!("Floor_L{lv}_{tj:02}_{ti:02}.webp"));
                    let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height()).encode(90.0);
                    std::fs::write(&file, &*enc).map_err(|e| format!("{}: {e}", file.display()))?;
                    m.props.push(Prop {
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
                    });
                    tiles += 1;
                }
                u += step;
                ti += 1;
            }
            v += step;
            tj += 1;
        }
        for line in walls(&scene, floor, frame, opt) {
            m.walls.push(Wall { points: line, closed: false, bends: Vec::new(), low: false, style: None });
            wall_count += 1;
        }
        m.wall_style = WallStyle { paint: Paint::Color([38, 34, 30]), width: 0.12 };
        maps.push(m);
    }
    // Предметы — на этаж, где их пол; висящие выше среза (люстры у потолка) — нет.
    for d in &scene.doodads {
        // Этаж — тот, над чьим полом предмет стоит не выше среза (сверху вниз).
        let Some(lv) = (0..floors.len()).rev().find(|&lv| floors[lv].at(d.pos[0], d.pos[1]).is_some_and(|f| d.pos[2] >= f - 0.3 && d.pos[2] <= f + opt.cut + 0.5)) else { continue };
        let model = super::m2::m2_path(&d.model);
        let entry = sprites.entry(model.to_lowercase()).or_insert_with(|| match super::import(client, &model, out_wow, &sprite_opt) {
            Ok(r) => Some(r),
            Err(e) => {
                notes.push(format!("{model}: {e}"));
                None
            }
        });
        let Some(sp) = entry else { continue };
        let rot_deg = (180.0 - wmo::yaw(d.rot).to_degrees()).rem_euclid(360.0);
        let (s, c) = rot_deg.to_radians().sin_cos();
        let o = [sp.origin[0] * d.scale, sp.origin[1] * d.scale];
        let p = frame.to_map(d.pos[0], d.pos[1]);
        let pos = [p[0] - (o[0] * c - o[1] * s), p[1] - (o[0] * s + o[1] * c)];
        maps[lv].props.push(Prop {
            asset: sp.file.to_string_lossy().into_owned(),
            pos,
            size: sp.cells,
            rot: rot_deg,
            scale: d.scale.clamp(0.05, 20.0),
            shadow: None,
            light: None,
            canopy: None,
            layer: None,
            tint: None,
            open: None,
            glow: None,
            sunk: None,
            flip: false,
        });
        objects += 1;
    }
    notes.truncate(12);
    let levels = maps.len();
    let mut map = maps.remove(0);
    map.levels = maps;
    map.name = scene.name.clone();
    Ok(Built { map, levels, tiles, walls: wall_count, objects, ppc, notes, sets: scene.sets.clone(), sets_used: scene.sets_used.clone() })
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
        let scene = load(&client, &path, None).unwrap();
        println!("загружено {:?}: групп {}, предметов {}, рамка {:?}–{:?}", t.elapsed(), scene.groups.len(), scene.doodads.len(), scene.lo, scene.hi);
        println!("этажей {}", level_floors(&scene, &Surfaces::new(&scene)).len());
        if let Ok(probe) = std::env::var("PROBE") {
            let surf = Surfaces::new(&scene);
            let floors = level_floors(&scene, &surf);
            for pt in probe.split(';') {
                let v: Vec<f32> = pt.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                let (i, j) = (((v[0] - surf.x0) / surf.step) as usize, ((v[1] - surf.y0) / surf.step) as usize);
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
        }
        let opt = Options { ppc: std::env::var("PPC").ok().and_then(|v| v.parse().ok()).unwrap_or(48.0), set: std::env::var("SET").ok().filter(|s| !s.is_empty()), ..Default::default() };
        let out = Path::new(&out);
        let built = build(&client, &path, &out.join("tiles"), &out.join("wow"), &opt).unwrap();
        println!(
            "карта {}×{}, этажей {}, кусков {}, стен {}, предметов {}, {} точек/клетку, {:?}; заметки {:?}",
            built.map.width, built.map.height, built.levels, built.tiles, built.walls, built.objects, built.ppc, t.elapsed(), built.notes
        );
        std::fs::write(out.join(format!("{}.karta", scene.name)), built.map.to_json()).unwrap();
    }
}
