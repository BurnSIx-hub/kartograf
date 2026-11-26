//! Здание или карта WoW — 3D-моделью glTF (`.glb`) для 3D-вида Foundry (3D Canvas): геометрия и
//! текстуры как в игре, запечённый свет — цветом вершин, предметы на своих местах; потолки и
//! крыши срезаны над полом этажа, как у снимка пола (вид сверху под углом — «кукольный домик»).
//! Местность карты (ADT) — слоями, запечёнными в картинку на 4×4 куска; вода, лава и слизь —
//! гладью с рисунком игры. Текущий рисунок (водопад — предмет со сдвигом текстуры, гладь — медленным
//! дрейфом) — `extras.flow` материала, текстур в секунду: его двигает скрипт тайла 3D Canvas, а у
//! модели в `.json` — `flow: true`. Карта материка — только с рамкой `area`: из большого города можно
//! взять квартал. Пол верхних ярусов (мост над улицей, второй этаж над залом того же этажа по
//! высоте) — своими тайлами `_S1.glb`…, тоже срезанными над полом: в верхе (`_top`) их не видно
//! и фишки на них не встают. Рядом — `<имя>_walls.json`: стены Foundry по ярусам (каждый — свой
//! уровень Foundry) и зоны верхних ярусов, где фишка переходит на их уровень; в точках сцены по
//! 100 на клетку, высоты зон — в клетках над началом модели.
//!
//! Оси модели — клетки карты Картографа: X — вправо (восток), Z — вниз карты (юг), Y — вверх.
//! Начало — левый верхний угол той же рамки, что у `location::build` (`map_frame`), высота 0 —
//! самый низкий пол этажа 0. Единица — клетка: модель ложится на карту клетка в клетку.

use super::blp::Rgba;
use super::location::{self, FloorMap, Frame, LiquidMap, Scene, Surfaces};
use super::{Client, adt, m2, wmo};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Options {
    pub cells_per_yard: f32,
    pub margin: f32,
    /// Срез над полом этажа, ярды: выше — потолки, крыши, верх стен.
    pub cut: f32,
    /// Только свет игры (`KHR_materials_unlit`): картинка как в клиенте, свет сцены её не меняет.
    /// Нет — материалы обычные, их освещает сцена (факелы Foundry), запечённый свет остаётся тенью.
    pub unlit: bool,
    /// Срезанный верх (потолки, крыши, верх стен) — отдельной моделью `<имя>_top.glb`: снаружи
    /// здание целое, на время игры внутри верх прячут.
    pub top: bool,
    pub set: Option<String>,
    /// Рамка мира у карты `[x0, y0, x1, y1]` (X — север, Y — запад, ярды).
    pub area: Option<[f32; 4]>,
    pub extra: Vec<(String, [f32; 3], f32)>,
    /// Точек текстуры местности на кусок ADT (33⅓ ярда).
    pub ground: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { cells_per_yard: 0.6, margin: 2.0, cut: 4.0, unlit: true, top: true, set: None, area: None, extra: Vec::new(), ground: 256 }
    }
}

pub struct Exported {
    /// Файл на этаж: этаж 0 — под именем, что просили, выше — `_L1`, `_L2`…
    pub files: Vec<PathBuf>,
    /// Размер карты в клетках (рамка `map_frame`).
    pub cells: [f32; 2],
    /// Рамка каждой модели в клетках (оси модели): низ-лево-верх и верх-право-низ.
    pub boxes: Vec<[[f32; 3]; 2]>,
    pub triangles: usize,
    pub doodads: usize,
    pub textures: usize,
    pub notes: Vec<String>,
}

#[derive(Clone, Copy)]
struct V {
    p: [f32; 3],
    n: [f32; 3],
    uv: [f32; 2],
    c: [f32; 3],
}

fn mix(a: &V, b: &V, t: f32) -> V {
    let l = |x: f32, y: f32| x + (y - x) * t;
    V { p: [0, 1, 2].map(|k| l(a.p[k], b.p[k])), n: [0, 1, 2].map(|k| l(a.n[k], b.n[k])), uv: [l(a.uv[0], b.uv[0]), l(a.uv[1], b.uv[1])], c: [0, 1, 2].map(|k| l(a.c[k], b.c[k])) }
}

/// Часть многоугольника, где `side(v) >= 0` (отсечение одной плоскостью, Сазерленд — Ходжмен).
fn keep(poly: &[V], side: impl Fn(&V) -> f32) -> Vec<V> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
        let (da, db) = (side(a), side(b));
        if da >= 0.0 {
            out.push(*a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            out.push(mix(a, b, da / (da - db)));
        }
    }
    out
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l < 1e-6 { [0.0, 0.0, 1.0] } else { [v[0] / l, v[1] / l, v[2] / l] }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Поворот кватернионом `x, y, z, w`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let t = cross(u, v).map(|c| 2.0 * c);
    let c = cross(u, t);
    [v[0] + q[3] * t[0] + c[0], v[1] + q[3] * t[1] + c[1], v[2] + q[3] * t[2] + c[2]]
}

/// Обход треугольников: glTF ждёт лицо против часовой стрелки. Обход сверяется с нормалями
/// вершин — если большинство треугольников смотрит против своих нормалей, обход обратный.
fn flipped(tris: impl Iterator<Item = [[f32; 3]; 3]>, normals: impl Iterator<Item = [f32; 3]>) -> bool {
    let mut score = 0.0f64;
    for (p, n) in tris.zip(normals) {
        let g = cross([p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]], [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]]);
        score += dot(g, n).signum() as f64;
    }
    score < 0.0
}

/// Свет сверху с северо-запада — как у снимка пола (`location`), в осях игры.
const LIGHT: [f32; 3] = [0.42, 0.42, 0.80];

fn lambert(n: [f32; 3]) -> f32 {
    dot(norm(n), norm(LIGHT)).max(0.0)
}

/// Цвет из «экранного» (как его кладёт игра поверх текстуры) в линейный: glTF умножает
/// линейные цвета, а игра — значения sRGB.
fn linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| v.max(0.0).powf(2.2))
}

/// Запечённый свет здания (`MOCV`, вдвое — как кладёт игра) около точки: предметы внутри
/// здания так же темны, как стены вокруг них. Корзины по 3 ярда.
struct Ambient {
    cells: HashMap<(i32, i32, i32), ([f32; 3], u32)>,
}

impl Ambient {
    const SIZE: f32 = 3.0;
    fn new(scene: &Scene) -> Self {
        let mut cells: HashMap<(i32, i32, i32), ([f32; 3], u32)> = HashMap::new();
        for g in &scene.groups {
            let Some(colors) = &g.colors else { continue };
            if g.terrain.is_some() {
                continue;
            }
            for (v, c) in g.verts.iter().zip(colors) {
                let k = ((v[0] / Self::SIZE).floor() as i32, (v[1] / Self::SIZE).floor() as i32, (v[2] / Self::SIZE).floor() as i32);
                let e = cells.entry(k).or_insert(([0.0; 3], 0));
                for (s, v) in e.0.iter_mut().zip(c) {
                    *s += (v * 2.0).clamp(0.25, 1.6);
                }
                e.1 += 1;
            }
        }
        Self { cells }
    }
    fn at(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        let k = ((p[0] / Self::SIZE).floor() as i32, (p[1] / Self::SIZE).floor() as i32, (p[2] / Self::SIZE).floor() as i32);
        let (mut s, mut n) = ([0.0f32; 3], 0u32);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some((c, m)) = self.cells.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        for q in 0..3 {
                            s[q] += c[q];
                        }
                        n += m;
                    }
                }
            }
        }
        (n > 0).then(|| s.map(|v| v / n as f32))
    }
}

/// Картинки текстур: каждая кодируется раз; сплошные — JPEG, с прозрачностью — PNG.
#[derive(Default)]
struct Images {
    keys: HashMap<String, usize>,
    list: Vec<(Vec<u8>, &'static str)>,
}

impl Images {
    fn add(&mut self, key: &str, img: &Rgba) -> Result<usize, String> {
        self.add_with(key, img, 92)
    }
    fn add_with(&mut self, key: &str, img: &Rgba, quality: u8) -> Result<usize, String> {
        if let Some(&i) = self.keys.get(key) {
            return Ok(i);
        }
        let mut bytes = Vec::new();
        let mime = if img.px.iter().all(|p| p[3] == 255) {
            let rgb: Vec<u8> = img.px.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
            let buf = image::RgbImage::from_raw(img.w, img.h, rgb).ok_or("текстура: размер не сходится")?;
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality).encode_image(&buf).map_err(|e| format!("{key}: {e}"))?;
            "image/jpeg"
        } else {
            let buf = image::RgbaImage::from_raw(img.w, img.h, img.px.concat()).ok_or("текстура: размер не сходится")?;
            buf.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).map_err(|e| format!("{key}: {e}"))?;
            "image/png"
        };
        self.list.push((bytes, mime));
        self.keys.insert(key.to_owned(), self.list.len() - 1);
        Ok(self.list.len() - 1)
    }
}

/// Материал glTF: текстура, как кладётся (0 — сплошной, 1 — вырез по альфе, 2 — полупрозрачный),
/// двусторонний ли, повтор по U/V, непрозрачность.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Mat {
    image: Option<usize>,
    mode: u8,
    two_sided: bool,
    wrap: [bool; 2],
    alpha: u8,
    /// Крышка среза: тёмная, без текстуры (цвет стен снимка пола).
    cap: bool,
    /// Течение рисунка в тысячных долях текстуры за секунду по U и V (`flow_key`); в glb — `extras.flow`
    /// материала, рисунок по ней сдвигает скрипт тайла 3D Canvas.
    flow: [i16; 2],
}

/// Течение рисунка — ключом материала: тысячные доли текстуры за секунду.
fn flow_key(f: [f32; 2]) -> [i16; 2] {
    f.map(|c| (c * 1000.0).round().clamp(i16::MIN as f32, i16::MAX as f32) as i16)
}

/// Течение глади, текстур за секунду: в игре вода, лава и слизь — покадровая анимация на месте,
/// glTF её не умеет; медленный дрейф рисунка (вода — четверть ярда в секунду) оживляет гладь.
const LIQUID_FLOW: [[f32; 2]; 3] = [[0.02, 0.01], [0.008, 0.004], [0.01, 0.005]];

#[derive(Default)]
struct Prim {
    v: Vec<V>,
    idx: Vec<u32>,
    shared: HashMap<u64, u32>,
}

impl Prim {
    fn vert(&mut self, key: Option<u64>, v: V) -> u32 {
        if let Some(k) = key
            && let Some(&i) = self.shared.get(&k)
        {
            return i;
        }
        let i = self.v.len() as u32;
        self.v.push(v);
        if let Some(k) = key {
            self.shared.insert(k, i);
        }
        i
    }
}

/// Сборка одного этажа: куски по материалам.
struct Level<'a> {
    floor: &'a FloorMap,
    liquid: &'a LiquidMap,
    frame: Frame,
    cells: [f32; 2],
    z0: f32,
    cut: f32,
    /// Ниже пола этажа на столько ярдов ещё рисовать (у верхних этажей — 1; у нижнего — всё).
    low: Option<f32>,
    mats: HashMap<Mat, usize>,
    prims: Vec<(Mat, Prim)>,
    triangles: usize,
    /// Рамка того, что легло, в осях модели (клетки).
    lo: [f32; 3],
    hi: [f32; 3],
    segs: Vec<Seg>,
    caps_count: usize,
    /// Верх здания (`Some` — полы всех этажей и ярусов с их `low`): то, что выше среза верхнего
    /// этажа над точкой, и то, что стоит вдали от пола (наружные стены, свесы крыш), — отдельным
    /// тайлом, его прячут на время игры.
    all: Option<&'a [(&'a FloorMap, Option<f32>)]>,
}

/// Верх здания берёт и то, что за рамкой карты на столько ярдов: свесы крыш, контрфорсы.
const TOP_PAD: f32 = 12.0;

/// Верхние этажи рисуют и на ярд ниже своего пола (толща перекрытия).
const UPPER_LOW: f32 = 1.0;

/// Пол яруса (по сетке поверхностей: угол и шаг `grid`, `w` клеток в ряд), которого не взял
/// ни один этаж по высоте: где пол яруса уже в окне этажа (второй этаж — своя полоса высот),
/// он лежит в тайле этажа, и тайл яруса его не дублирует. Чего нет — `INFINITY`.
fn storey_only(floor: &[f32], grid: [f32; 3], w: usize, floors: &[FloorMap], liquid: impl Fn(f32, f32) -> f32, cut: f32) -> Vec<f32> {
    floor
        .iter()
        .enumerate()
        .map(|(k, &z)| {
            let (x, y) = (grid[0] + ((k % w) as f32 + 0.5) * grid[2], grid[1] + ((k / w) as f32 + 0.5) * grid[2]);
            let lq = liquid(x, y);
            let taken = floors.iter().enumerate().any(|(lv, f)| {
                f.at(x, y).is_some_and(|fz| {
                    let (b, t) = level_window(fz, lq, cut, (lv > 0).then_some(UPPER_LOW));
                    z >= b && z <= t
                })
            });
            if z.is_finite() && !taken { z } else { f32::INFINITY }
        })
        .collect()
}

/// Промежутки высот, которых не взял ни один этаж (`taken` — что взяли, в любом порядке).
fn untaken(mut taken: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    taken.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut free = Vec::new();
    let mut from = f32::MIN;
    for (b, t) in taken {
        if b > from {
            free.push((from, b));
        }
        from = from.max(t);
    }
    free.push((from, f32::MAX));
    free
}

/// Что этаж берёт в точке, ярды: от `low` ниже пола (нет — всё ниже) до среза над полом или
/// гладью. Пол этажа здесь — гладь: дно под ней — этого этажа, как бы глубоко ни лежало.
fn level_window(floor: f32, liquid: f32, cut: f32, low: Option<f32>) -> (f32, f32) {
    let bottom = if liquid >= floor - 0.5 { floor - 60.0 } else { low.map_or(f32::MIN, |l| floor - l) };
    (bottom, floor.max(liquid) + cut)
}

impl Level<'_> {
    /// Положить треугольник игры (точки в ярдах мира): за краем карты и над пустотой — нет,
    /// выше среза над полом — срезается. `keys` — номера вершин для общих вершин целых треугольников.
    fn put(&mut self, mat: Mat, tri: [V; 3], keys: Option<[u64; 3]>) {
        let c = [(tri[0].p[0] + tri[1].p[0] + tri[2].p[0]) / 3.0, (tri[0].p[1] + tri[1].p[1] + tri[2].p[1]) / 3.0];
        if let Some(all) = self.all {
            let m = self.frame.to_map(c[0], c[1]);
            let pad = TOP_PAD * self.frame.cpy;
            if m[0] < -pad || m[1] < -pad || m[0] > self.cells[0] + pad || m[1] > self.cells[1] + pad {
                return;
            }
            // Верху — всё, что не взял ни один этаж: выше среза верхнего, между срезом нижнего
            // этажа и полом верхнего, ниже пола (крутой склон под площадкой), за краем карты.
            let inside = m[0] >= 0.0 && m[1] >= 0.0 && m[0] <= self.cells[0] && m[1] <= self.cells[1];
            let lq = self.liquid.at(c[0], c[1]);
            let taken: Vec<(f32, f32)> = if inside {
                all.iter().filter_map(|&(f, low)| f.at(c[0], c[1]).map(|f| level_window(f, lq, self.cut, low))).collect()
            } else {
                Vec::new()
            };
            let mut pieces = Vec::new();
            for (lo, hi) in untaken(taken) {
                if tri.iter().all(|v| v.p[2] >= lo && v.p[2] <= hi) {
                    pieces.push((tri, keys));
                    continue;
                }
                let mut poly = tri.to_vec();
                if lo > f32::MIN {
                    poly = keep(&poly, |v| v.p[2] - lo);
                }
                if hi < f32::MAX {
                    poly = keep(&poly, |v| hi - v.p[2]);
                }
                pieces.extend((1..poly.len().saturating_sub(1)).map(|i| ([poly[0], poly[i], poly[i + 1]], None)));
            }
            self.push(mat, pieces);
            return;
        }
        let Some(f) = self.floor.at(c[0], c[1]) else { return };
        let m = self.frame.to_map(c[0], c[1]);
        if m[0] < 0.0 || m[1] < 0.0 || m[0] > self.cells[0] || m[1] > self.cells[1] {
            return;
        }
        let lq = self.liquid.at(c[0], c[1]);
        let (bottom, top) = level_window(f, lq, self.cut, self.low);
        let whole = tri.iter().all(|v| v.p[2] <= top && v.p[2] >= bottom);
        let pieces: Vec<([V; 3], Option<[u64; 3]>)> = if whole {
            vec![(tri, keys)]
        } else {
            // Где грань пересекает срез — отрезок для крышки (`caps`): по нему видно толщу стены.
            if mat.mode == 0 {
                let p = tri.map(|v| v.p);
                let cuts: Vec<[f32; 2]> = (0..3)
                    .filter_map(|i| {
                        let (a, b) = if p[i] <= p[(i + 1) % 3] { (p[i], p[(i + 1) % 3]) } else { (p[(i + 1) % 3], p[i]) };
                        ((a[2] - top) * (b[2] - top) < 0.0).then(|| {
                            let t = (top - a[2]) / (b[2] - a[2]);
                            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
                        })
                    })
                    .collect();
                let g = cross([p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]], [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]]);
                let (h, l) = ((g[0] * g[0] + g[1] * g[1]).sqrt(), (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt());
                if cuts.len() == 2 && h > 0.3 * l {
                    self.segs.push(Seg { a: cuts[0], b: cuts[1], n: [g[0] / h, g[1] / h], top });
                }
            }
            let poly = keep(&keep(&tri, |v| top - v.p[2]), |v| v.p[2] - bottom);
            (1..poly.len().saturating_sub(1)).map(|i| ([poly[0], poly[i], poly[i + 1]], None)).collect()
        };
        self.push(mat, pieces);
    }

    /// Положить готовые куски (точки в ярдах мира) в материал, переводя в оси модели.
    fn push(&mut self, mat: Mat, pieces: Vec<([V; 3], Option<[u64; 3]>)>) {
        if pieces.is_empty() {
            return;
        }
        let slot = *self.mats.entry(mat).or_insert_with(|| {
            self.prims.push((mat, Prim::default()));
            self.prims.len() - 1
        });
        let (frame, z0) = (self.frame, self.z0);
        let out = |v: &V| -> V {
            let [u, w] = frame.to_map(v.p[0], v.p[1]);
            let n = norm(v.n);
            V { p: [u, (v.p[2] - z0) * frame.cpy, w], n: [-n[1], n[2], -n[0]], uv: v.uv, c: v.c }
        };
        for (t, k) in pieces {
            let prim = &mut self.prims[slot].1;
            let ids: Vec<u32> = (0..3).map(|i| prim.vert(k.map(|k| k[i]), out(&t[i]))).collect();
            for &i in &ids {
                let p = prim.v[i as usize].p;
                self.lo = [0, 1, 2].map(|k| self.lo[k].min(p[k]));
                self.hi = [0, 1, 2].map(|k| self.hi[k].max(p[k]));
            }
            prim.idx.extend(ids);
            self.triangles += 1;
        }
    }

    /// Крышки среза. Стены и колонны игры — пустые коробки: срезанные над полом, сверху они
    /// показывают пустоту. Срез закрывается тёмным, как стены на снимке пола: по рядам через
    /// 0,2 ярда ищется толща — вход (грань смотрит навстречу ходу) и выход (грань смотрит по
    /// ходу). Толще 4 ярдов — не стена: у одиночной грани пары нет, и заливать до следующей
    /// стены нельзя. Ряды идут по X и по Y: стена вдоль хода рядом не пересекается.
    fn caps(&mut self) {
        const STEP: f32 = 0.2;
        const THICK: f32 = 4.0;
        if self.segs.is_empty() {
            return;
        }
        let mut quads: Vec<([f32; 2], [f32; 2], f32)> = Vec::new();
        for axis in 0..2 {
            let other = 1 - axis;
            let lo = self.segs.iter().map(|s| s.a[other].min(s.b[other])).fold(f32::MAX, f32::min);
            let hi = self.segs.iter().map(|s| s.a[other].max(s.b[other])).fold(f32::MIN, f32::max);
            let rows = ((hi - lo) / STEP).ceil() as usize + 1;
            let mut by_row: Vec<Vec<(f32, i8, f32)>> = vec![Vec::new(); rows];
            for s in &self.segs {
                if s.n[axis].abs() < 0.3 {
                    continue;
                }
                let (o0, o1) = (s.a[other].min(s.b[other]), s.a[other].max(s.b[other]));
                let r0 = ((o0 - lo) / STEP).ceil().max(0.0) as usize;
                for (r, row) in by_row.iter_mut().enumerate().skip(r0) {
                    let y = lo + r as f32 * STEP;
                    if y >= o1 {
                        break;
                    }
                    let t = (y - s.a[other]) / (s.b[other] - s.a[other]);
                    row.push((s.a[axis] + t * (s.b[axis] - s.a[axis]), if s.n[axis] < 0.0 { 1 } else { -1 }, s.top));
                }
            }
            // Толща ряда — отрезки; одинаковые в соседних рядах сливаются в прямоугольник.
            let mut open: Vec<(f32, f32, f32, usize, usize)> = Vec::new();
            let close = |o: (f32, f32, f32, usize, usize), quads: &mut Vec<([f32; 2], [f32; 2], f32)>| {
                let (b0, b1) = (lo + o.3 as f32 * STEP - STEP / 2.0, lo + o.4 as f32 * STEP + STEP / 2.0);
                let pt = |a: f32, b: f32| if axis == 0 { [a, b] } else { [b, a] };
                quads.push((pt(o.0, b0), pt(o.1, b1), o.2));
            };
            for (r, row) in by_row.iter_mut().enumerate() {
                row.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut runs: Vec<(f32, f32, f32)> = Vec::new();
                let (mut depth, mut start) = (0, (0.0f32, 0.0f32));
                for &(x, d, top) in row.iter() {
                    if d > 0 {
                        if depth == 0 {
                            start = (x, top);
                        }
                        depth += 1;
                    } else if depth > 0 {
                        depth -= 1;
                        if depth == 0 && x - start.0 > 0.02 && x - start.0 <= THICK {
                            runs.push((start.0, x, (start.1 + top) / 2.0));
                        }
                    }
                }
                let mut used = vec![false; runs.len()];
                let mut next = Vec::with_capacity(open.len());
                for o in open.drain(..) {
                    match runs.iter().enumerate().position(|(k, u)| !used[k] && (u.0 - o.0).abs() < 0.05 && (u.1 - o.1).abs() < 0.05 && (u.2 - o.2).abs() < 0.05) {
                        Some(k) => {
                            used[k] = true;
                            next.push((o.0, o.1, o.2, o.3, r));
                        }
                        None => close(o, &mut quads),
                    }
                }
                for (k, u) in runs.into_iter().enumerate() {
                    if !used[k] {
                        next.push((u.0, u.1, u.2, r, r));
                    }
                }
                open = next;
            }
            for o in open.drain(..) {
                close(o, &mut quads);
            }
        }
        let mat = Mat { image: None, mode: 0, two_sided: false, wrap: [true, true], alpha: 255, cap: true, flow: [0, 0] };
        let v = |x: f32, y: f32, z: f32| V { p: [x, y, z + 0.02], n: [0.0, 0.0, 1.0], uv: [0.0, 0.0], c: [1.0; 3] };
        let pieces: Vec<([V; 3], Option<[u64; 3]>)> = quads
            .iter()
            .flat_map(|&(a, b, z)| {
                // Сверху против часовой стрелки (в осях игры X — север, Y — запад: вид сверху зеркален).
                let q = [v(a[0], a[1], z), v(a[0], b[1], z), v(b[0], b[1], z), v(b[0], a[1], z)];
                let up = cross([q[1].p[0] - q[0].p[0], q[1].p[1] - q[0].p[1], 0.0], [q[2].p[0] - q[0].p[0], q[2].p[1] - q[0].p[1], 0.0])[2] > 0.0;
                let q = if up { q } else { [q[0], q[3], q[2], q[1]] };
                [([q[0], q[1], q[2]], None), ([q[0], q[2], q[3]], None)]
            })
            .collect();
        self.caps_count = quads.len();
        self.push(mat, pieces);
    }
}

/// Цвет жидкости без текстуры: вода (её толща), лава, слизь.
const LIQUID_TINT: [[f32; 3]; 3] = [[0.18, 0.40, 0.48], [0.95, 0.38, 0.08], [0.32, 0.62, 0.16]];

/// Гребни ряби на воде: светлая бирюза, как блики в игре.
const WATER_CREST: [f32; 3] = [0.70, 0.86, 0.90];

/// Непрозрачность воды: дно мелкого бассейна видно, рябь читается.
const WATER_ALPHA: u8 = 150;

/// Рисунок глади: вода — рябь игры между цветом толщи и гребня, лава и слизь — как есть, ярче
/// (как у снимка). Рябь игры почти чёрная и бледная (`lake_a`: яркость 0–0,25, в игре её кладёт
/// шейдер поверх цвета воды): её размах растягивается на весь — от 2-го до 98-го процентиля.
fn liquid_texture(t: &Rgba, kind: wmo::LiquidKind) -> Rgba {
    let luma = |p: &[u8; 4]| (p[0] as f32 * 0.3 + p[1] as f32 * 0.59 + p[2] as f32 * 0.11) / 255.0;
    let mut sorted: Vec<f32> = t.px.iter().map(luma).collect();
    sorted.sort_by(f32::total_cmp);
    let at = |q: f32| sorted.get(((sorted.len().saturating_sub(1)) as f32 * q) as usize).copied().unwrap_or(0.0);
    let (lo, hi) = (at(0.02), at(0.98));
    let span = (hi - lo).max(1e-3);
    let px = t
        .px
        .iter()
        .map(|p| {
            let o = match kind {
                wmo::LiquidKind::Water => {
                    let w = ((luma(p) - lo) / span).clamp(0.0, 1.0).powf(1.3);
                    [0, 1, 2].map(|q| LIQUID_TINT[0][q] + (WATER_CREST[q] - LIQUID_TINT[0][q]) * w)
                }
                _ => [p[0], p[1], p[2]].map(|v| (v as f32 / 255.0 * 1.25).min(1.0)),
            };
            let o = o.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
            [o[0], o[1], o[2], 255]
        })
        .collect();
    Rgba { w: t.w, h: t.h, px }
}

/// Вода по имени текстуры (водопад, река, ручей), а не лава или слизь.
fn watery(file: &str) -> bool {
    let f = file.to_lowercase();
    ["water", "fall", "river", "stream"].iter().any(|w| f.contains(w)) && !["lava", "magma", "slime"].iter().any(|w| f.contains(w))
}

/// Кусков местности в стороне картинки: 4×4 куска (133⅓ ярда) — одна текстура.
const BLOCK: usize = 4;

/// Местность, запечённая в картинки: блок 4×4 куска — одна, `r` точек на кусок. Соседние куски
/// в картинке — соседи и на земле, так что фильтр текстуры не даёт швов внутри блока.
struct Ground {
    blocks: Vec<((i32, i32), Rgba)>,
    chunks: Vec<GroundChunk>,
}

/// Кусок местности: группа сцены, блок (строка, столбец), место в блоке (строка, столбец).
#[derive(Clone, Copy)]
struct GroundChunk {
    group: usize,
    block: (i32, i32),
    at: (usize, usize),
}

impl Ground {
    fn bake(scene: &Scene, r: usize) -> Self {
        // Номер куска в сетке мира: строки — к югу, столбцы — к востоку (угол куска — `verts[0]`).
        let b = BLOCK as i32;
        let mut chunks = Vec::new();
        for (gi, g) in scene.groups.iter().enumerate() {
            if g.terrain.is_none() || g.tris.is_empty() {
                continue;
            }
            let Some(p) = g.verts.first() else { continue };
            let (row, col) = (((adt::ORIGIN - p[0]) / adt::CHUNK).round() as i32, ((adt::ORIGIN - p[1]) / adt::CHUNK).round() as i32);
            chunks.push(GroundChunk { group: gi, block: (row.div_euclid(b), col.div_euclid(b)), at: (row.rem_euclid(b) as usize, col.rem_euclid(b) as usize) });
        }
        // Текстуры слоёв — заранее уменьшенные до шага картинки (8 повторов на кусок): иначе
        // точка картинки выхватывала бы одну точку текстуры и рябила.
        let small_side = (r / 8).max(2) as u32;
        let mut small: HashMap<usize, Rgba> = HashMap::new();
        for c in &chunks {
            let Some(paint) = scene.groups[c.group].terrain.as_deref() else { continue };
            for &(t, _) in &paint.layers {
                if let Some(t) = t
                    && !small.contains_key(&t)
                    && let Some(src) = scene.textures.get(t)
                    && let Some(img) = image::RgbaImage::from_raw(src.w, src.h, src.px.concat())
                {
                    let s = image::imageops::resize(&img, small_side, small_side, image::imageops::FilterType::Triangle);
                    small.insert(t, Rgba { w: small_side, h: small_side, px: s.pixels().map(|p| p.0).collect() });
                }
            }
        }
        let mut keys: Vec<(i32, i32)> = chunks.iter().map(|c| c.block).collect();
        keys.sort_unstable();
        keys.dedup();
        let side = BLOCK * r;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, keys.len().max(1));
        let per = keys.len().div_ceil(threads).max(1);
        let blocks: Vec<((i32, i32), Rgba)> = std::thread::scope(|sc| {
            let jobs: Vec<_> = keys
                .chunks(per)
                .map(|part| {
                    let (chunks, small) = (&chunks, &small);
                    sc.spawn(move || {
                        part.iter()
                            .map(|&key| {
                                let mut px = vec![[115u8, 107, 92, 255]; side * side];
                                for c in chunks.iter().filter(|c| c.block == key) {
                                    if let Some(paint) = scene.groups[c.group].terrain.as_deref() {
                                        bake_chunk(paint, small, r, &mut px[c.at.0 * r * side + c.at.1 * r..], side);
                                    }
                                }
                                (key, Rgba { w: side as u32, h: side as u32, px })
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
        });
        Self { blocks, chunks }
    }
}

/// Кусок местности в картинку (`out` — с угла куска, строка — `stride` точек): слои по маскам
/// с плавным переходом между точками маски, в тени — темнее (как снимок местности 2D).
fn bake_chunk(p: &adt::Paint, small: &HashMap<usize, Rgba>, r: usize, out: &mut [[u8; 4]], stride: usize) {
    let shadow: Option<Vec<u8>> = p.shadow.as_ref().map(|s| s.iter().map(|&b| b.min(1) * 255).collect());
    for b in 0..r {
        for a in 0..r {
            let (u, v) = ((a as f32 + 0.5) / r as f32, (b as f32 + 0.5) / r as f32);
            // Маска 64×64 — по серединам точек, между ними — плавно.
            let (fx, fy) = ((u * 64.0 - 0.5).clamp(0.0, 63.0), (v * 64.0 - 0.5).clamp(0.0, 63.0));
            let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(63), (y0 + 1).min(63));
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let at = |m: &[u8]| {
                let g = |x: usize, y: usize| m.get(y * 64 + x).copied().unwrap_or(0) as f32 / 255.0;
                (g(x0, y0) * (1.0 - tx) + g(x1, y0) * tx) * (1.0 - ty) + (g(x0, y1) * (1.0 - tx) + g(x1, y1) * tx) * ty
            };
            let mut c = [0.45f32, 0.42, 0.36];
            for (k, (tex, alpha)) in p.layers.iter().enumerate() {
                let Some(t) = tex.and_then(|i| small.get(&i)) else { continue };
                let s = t.sample(u * 8.0, v * 8.0, [true, true]);
                let w = if k == 0 { 1.0 } else { alpha.as_deref().map_or(0.0, at) };
                for q in 0..3 {
                    c[q] = c[q] * (1.0 - w) + s[q] * w;
                }
            }
            if let Some(sh) = &shadow {
                let dark = 1.0 - 0.3 * at(sh);
                c = c.map(|q| q * dark);
            }
            if let Some(px) = out.get_mut(b * stride + a) {
                let q = c.map(|q| (q * 255.0).round().clamp(0.0, 255.0) as u8);
                *px = [q[0], q[1], q[2], 255];
            }
        }
    }
}

/// Где грань пересекает срез: отрезок (ярды мира), куда смотрит грань (по горизонтали), высота среза.
struct Seg {
    a: [f32; 2],
    b: [f32; 2],
    n: [f32; 2],
    top: f32,
}

/// Выгрузить здание (`.wmo`) или карту инстанса (`.wdt`) в `out` (`.glb`), этаж на файл.
pub fn export(client: &Client, path: &str, out: &Path, opt: &Options) -> Result<Exported, String> {
    let scene = if location::is_map(path) { location::load_map(client, path, opt.set.as_deref(), opt.area, &opt.extra)? } else { location::load(client, path, opt.set.as_deref())? };
    let mut notes: Vec<String> = scene.missing.iter().take(5).cloned().collect();
    // Здания карты и их рамки в мире — по ним выбирают `area`.
    for (w, b) in &scene.placed {
        notes.push(format!("здание {w}: area={:.0},{:.0},{:.0},{:.0}", b[0], b[1], b[2], b[3]));
    }
    let surf = Surfaces::new(&scene);
    let floors = location::level_floors(&scene, &surf);
    let (frame, wc, hc) = location::map_frame(&scene, &surf, opt.cells_per_yard, opt.margin, opt.area)?;
    let liquid = LiquidMap::new(&scene);
    let ambient = Ambient::new(&scene);
    let z0 = floors.first().map_or(scene.lo[2], |f| f.z.iter().copied().filter(|z| z.is_finite()).fold(f32::MAX, f32::min));
    let mut images = Images::default();
    // Ярусы для 3D-сцены (земля, мосты над улицей и вторые этажи): стены и зоны — рядом в
    // `_walls.json`, полы верхних ярусов — свои тайлы `_S1.glb`…: в верхе их не видно и фишки на
    // них не встают. Пол яруса, который уже лежит в окне этажа по высоте (второй этаж — своя
    // полоса высот), ярусу не нужен: тайлы не дублируют друг друга.
    let storeys = location::storeys(&surf, frame, opt.cells_per_yard, [wc, hc]);
    let storey_floors: Vec<FloorMap> = storeys
        .iter()
        .skip(1)
        .map(|s| location::storey_floor(&surf, storey_only(&s.floor, [surf.x0, surf.y0, surf.step], surf.w, &floors, |x, y| liquid.at(x, y), opt.cut)))
        .filter(|f| f.z.iter().any(|z| z.is_finite()))
        .collect();
    let all: Vec<(&FloorMap, Option<f32>)> = floors.iter().enumerate().map(|(lv, f)| (f, (lv > 0).then_some(UPPER_LOW))).chain(storey_floors.iter().map(|f| (f, Some(UPPER_LOW)))).collect();
    let level = |floor, low, all| Level { floor, liquid: &liquid, frame, cells: [wc, hc], z0, cut: opt.cut, low, mats: HashMap::new(), prims: Vec::new(), triangles: 0, lo: [f32::MAX; 3], hi: [f32::MIN; 3], segs: Vec::new(), caps_count: 0, all };
    let mut levels: Vec<Level> = floors.iter().enumerate().map(|(lv, floor)| level(floor, (lv > 0).then_some(UPPER_LOW), None)).collect();
    let mut part_names: Vec<String> = (0..floors.len()).map(|lv| format!("L{lv}")).collect();
    for (k, f) in storey_floors.iter().enumerate() {
        levels.push(level(f, Some(UPPER_LOW), None));
        part_names.push(format!("S{}", k + 1));
    }
    // Верх — последним: в него идёт всё, что срезано у этажей и ярусов и что стоит вдали от пола.
    if opt.top
        && let Some(last) = floors.last()
    {
        levels.push(level(last, None, Some(&all)));
        part_names.push("top".into());
    }
    let near = |p: [f32; 3]| {
        let m = frame.to_map(p[0], p[1]);
        let pad = if opt.top { TOP_PAD * frame.cpy } else { 0.0 };
        m[0] >= -pad && m[1] >= -pad && m[0] <= wc + pad && m[1] <= hc + pad
    };

    // Здание: треугольники с материалом; свечения (наложения) glTF не умеет — пропускаются.
    let (mut glows, mut terrain) = (0usize, 0usize);
    for (gi, g) in scene.groups.iter().enumerate() {
        if g.terrain.is_some() {
            terrain += 1;
            continue;
        }
        let drawn: Vec<usize> = (0..g.tris.len()).filter(|&t| g.renders(t)).collect();
        let flip = flipped(drawn.iter().map(|&t| g.tris[t].map(|i| g.verts[i as usize])), drawn.iter().map(|&t| g.tris[t].iter().fold([0.0; 3], |s, &i| {
            let n = g.normals.get(i as usize).copied().unwrap_or([0.0; 3]);
            [s[0] + n[0], s[1] + n[1], s[2] + n[2]]
        })));
        for t in drawn {
            let mi = g.mat_base + g.tri_mat[t] as usize;
            let Some(m) = scene.materials.get(mi) else { continue };
            if m.blend > 2 {
                glows += 1;
                continue;
            }
            let image = match scene.mat_tex.get(mi).copied().flatten() {
                Some(i) => Some(images.add(&format!("wmo:{}", scene.tex_names[i].to_lowercase()), &scene.textures[i])?),
                None => None,
            };
            let mat = Mat { image, mode: m.blend as u8, two_sided: m.flags & 0x4 != 0, wrap: [m.flags & 0x40 == 0, m.flags & 0x80 == 0], alpha: 255, cap: false, flow: [0, 0] };
            let unlit_mat = m.flags & 1 != 0;
            let mut ids = g.tris[t];
            if flip {
                ids.swap(1, 2);
            }
            let tri = ids.map(|i| {
                let i = i as usize;
                let n = g.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                let c = if unlit_mat {
                    [1.0; 3]
                } else {
                    match &g.colors {
                        Some(c) => c.get(i).copied().unwrap_or([0.5; 3]).map(|v| (v * 2.0).clamp(0.25, 1.6)),
                        None if opt.unlit => [0.55 + 0.55 * lambert(n); 3],
                        None => [1.0; 3],
                    }
                };
                V { p: g.verts[i], n, uv: g.uvs.get(i).copied().unwrap_or([0.0, 0.0]), c: linear(c) }
            });
            let keys = ids.map(|i| ((gi as u64) << 32) | i as u64);
            for l in &mut levels {
                l.put(mat, tri, Some(keys));
            }
        }
    }
    if glows > 0 {
        notes.push(format!("свечений без glTF: {glows} треуг."));
    }
    if terrain > 0 {
        let ground = Ground::bake(&scene, opt.ground as usize);
        let mut block_img: HashMap<(i32, i32), usize> = HashMap::new();
        for (key, img) in &ground.blocks {
            block_img.insert(*key, images.add_with(&format!("ground:{}:{}", key.0, key.1), img, 85)?);
        }
        for &GroundChunk { group: gi, block, at: sub } in &ground.chunks {
            let g = &scene.groups[gi];
            let mat = Mat { image: block_img.get(&block).copied(), mode: 0, two_sided: false, wrap: [false, false], alpha: 255, cap: false, flow: [0, 0] };
            let nrm = |i: u32| g.normals.get(i as usize).copied().unwrap_or([0.0, 0.0, 1.0]);
            let flip = flipped(g.tris.iter().map(|t| t.map(|i| g.verts[i as usize])), g.tris.iter().map(|t| t.iter().fold([0.0; 3], |s, &i| {
                let n = nrm(i);
                [s[0] + n[0], s[1] + n[1], s[2] + n[2]]
            })));
            for t in &g.tris {
                let mut ids = *t;
                if flip {
                    ids.swap(1, 2);
                }
                let tri = ids.map(|i| {
                    let n = nrm(i);
                    let uv = g.uvs.get(i as usize).copied().unwrap_or([0.0, 0.0]);
                    // Цвета вершин (`MCCV`, 1.0 — как есть) и свет по нормали — как у снимка местности.
                    let tint = g.colors.as_ref().and_then(|c| c.get(i as usize)).copied().unwrap_or([1.0; 3]).map(|q| q.clamp(0.3, 2.0));
                    let shade = if opt.unlit { 0.5 + 0.6 * lambert(n) } else { 1.0 };
                    V { p: g.verts[i as usize], n, uv: [(sub.1 as f32 + uv[0]) / BLOCK as f32, (sub.0 as f32 + uv[1]) / BLOCK as f32], c: linear(tint.map(|q| (q * shade).min(1.6))) }
                });
                let keys = ids.map(|i| ((gi as u64) << 32) | i as u64);
                for l in &mut levels {
                    l.put(mat, tri, Some(keys));
                }
            }
        }
        notes.push(format!("местность: кусков {}, картинок {} по {} точек", ground.chunks.len(), ground.blocks.len(), BLOCK * opt.ground as usize));
    }

    // Жидкость (`MLIQ` зданий, `MCLQ`/`MH2O` местности): гладь клетками с рисунком игры; вода —
    // полупрозрачная (видно дно), лава и слизь — сплошные. Одна и та же клетка из здания и из
    // местности кладётся раз.
    let mut laid: HashMap<(i32, i32), Vec<f32>> = HashMap::new();
    let mut liquid_img: [Option<Option<usize>>; 3] = [None; 3];
    let mut liquid_tiles = 0usize;
    let st = wmo::LIQUID_STEP;
    for g in &scene.groups {
        let Some(l) = &g.liquid else { continue };
        let kind = wmo::liquid_kind(scene.root_flags, g.group_liquid, l.tile_kind);
        let k = match kind {
            wmo::LiquidKind::Water => 0,
            wmo::LiquidKind::Magma => 1,
            wmo::LiquidKind::Slime => 2,
        };
        if liquid_img[k].is_none() {
            liquid_img[k] = Some(match &scene.liquid_tex[k] {
                Some(t) => Some(images.add(&format!("liquid:{k}"), &liquid_texture(t, kind))?),
                None => None,
            });
        }
        let image = liquid_img[k].flatten();
        let water = k == 0;
        let mat = Mat { image, mode: if water { 2 } else { 0 }, two_sided: water, wrap: [true, true], alpha: if water { WATER_ALPHA } else { 255 }, cap: false, flow: flow_key(LIQUID_FLOW[k]) };
        let c = if image.is_some() { [1.0; 3] } else { linear(LIQUID_TINT[k]) };
        let hgt = |ix: usize, iy: usize| l.heights.get(iy * l.verts[0] + ix).copied().unwrap_or(l.corner[2]);
        for ty in 0..l.tiles[1] {
            for tx in 0..l.tiles[0] {
                if !l.shown.get(ty * l.tiles[0] + tx).copied().unwrap_or(false) {
                    continue;
                }
                let corner = |ix: usize, iy: usize| [l.corner[0] + ix as f32 * st, l.corner[1] + iy as f32 * st, hgt(ix, iy)];
                let q = [corner(tx, ty), corner(tx + 1, ty), corner(tx + 1, ty + 1), corner(tx, ty + 1)];
                let mid = [(q[0][0] + q[2][0]) / 2.0, (q[0][1] + q[2][1]) / 2.0, (q[0][2] + q[1][2] + q[2][2] + q[3][2]) / 4.0];
                let cell = ((mid[0] / st).floor() as i32, (mid[1] / st).floor() as i32);
                let seen = laid.entry(cell).or_default();
                if seen.iter().any(|&z| (z - mid[2]).abs() < 0.5) || !near(mid) {
                    continue;
                }
                seen.push(mid[2]);
                liquid_tiles += 1;
                // Повтор рисунка — три клетки жидкости, как на снимке.
                let v = |p: [f32; 3]| V { p, n: [0.0, 0.0, 1.0], uv: [(p[0] - scene.lo[0]) / (st * 3.0), (p[1] - scene.lo[1]) / (st * 3.0)], c };
                let up = cross([q[1][0] - q[0][0], q[1][1] - q[0][1], 0.0], [q[2][0] - q[0][0], q[2][1] - q[0][1], 0.0])[2] > 0.0;
                let q = if up { q } else { [q[0], q[3], q[2], q[1]] };
                for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    let tri = tri.map(v);
                    for l in &mut levels {
                        l.put(mat, tri, None);
                    }
                }
            }
        }
    }
    if liquid_tiles > 0 {
        notes.push(format!("жидкость: клеток {liquid_tiles}"));
    }

    // Предметы: каждая модель читается раз, ставится на место поворотом и масштабом расстановки.
    let mut models: HashMap<String, Option<super::Loaded>> = HashMap::new();
    let (mut placed, mut lights) = (0usize, 0usize);
    let mut failed: Vec<String> = Vec::new();
    for (di, d) in scene.doodads.iter().enumerate() {
        let name = client.spelled(&m2::m2_path(&d.model));
        if !near(d.pos) || (!opt.top && levels.iter().all(|l| l.floor.at(d.pos[0], d.pos[1]).is_none())) {
            continue;
        }
        let key = name.to_lowercase();
        if !models.contains_key(&key) {
            let loaded = match super::load(client, &name) {
                Ok(m) => Some(m),
                Err(e) => {
                    if location::light_only(&name).is_none() {
                        failed.push(format!("{name}: {e}"));
                    }
                    None
                }
            };
            models.insert(key.clone(), loaded);
        }
        let Some((model, textures, _)) = models.get(&key).and_then(|m| m.as_ref()) else {
            if location::light_only(&name).is_some() {
                lights += 1;
            }
            continue;
        };
        let amb = ambient.at(d.pos);
        let mut any = false;
        let batches: Vec<&m2::Batch> = model.batches.iter().filter(|b| model.materials.get(b.material).map_or(0, |m| m.blend) <= 2 && b.alpha >= 0.05).collect();
        let flip = flipped(
            batches.iter().flat_map(|b| b.triangles.iter()).map(|t| t.map(|i| model.vertices.get(i as usize).map_or([0.0; 3], |v| v.pos))),
            batches.iter().flat_map(|b| b.triangles.iter()).map(|t| t.iter().fold([0.0; 3], |s, &i| {
                let n = model.vertices.get(i as usize).map_or([0.0; 3], |v| v.normal);
                [s[0] + n[0], s[1] + n[1], s[2] + n[2]]
            })),
        );
        for b in batches {
            let m = model.materials.get(b.material);
            let blend = m.map_or(0, |m| m.blend) as u8;
            let tex = b.texture.filter(|&ti| textures.get(ti).is_some_and(Option::is_some));
            let image = match tex {
                Some(ti) => {
                    let file = &model.textures[ti].file;
                    let k = if file.is_empty() { format!("m2:{key}#{ti}") } else { file.to_lowercase() };
                    let img = textures[ti].as_ref().unwrap();
                    // Текущая вода предмета (водопад, ручей): картинка игры тёмная и бледная —
                    // рисунок кладётся в цвета воды, как гладь.
                    if b.flow.is_some() && watery(&k) {
                        Some(images.add(&format!("{k}#water"), &liquid_texture(img, wmo::LiquidKind::Water))?)
                    } else {
                        Some(images.add(&k, img)?)
                    }
                }
                None => None,
            };
            let wrap = tex.and_then(|ti| model.textures.get(ti)).map_or([true, true], |t| [t.flags & 1 != 0, t.flags & 2 != 0]);
            let alpha = if blend == 2 { (b.alpha.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 255 };
            let mat = Mat { image, mode: blend, two_sided: m.is_some_and(|m| m.flags & 0x4 != 0), wrap, alpha, cap: false, flow: b.flow.map_or([0, 0], flow_key) };
            let unlit_mat = m.is_some_and(|m| m.flags & 1 != 0);
            for t in &b.triangles {
                let mut ids = *t;
                if flip {
                    ids.swap(1, 2);
                }
                let Some(vs) = ids.iter().map(|&i| model.vertices.get(i as usize)).collect::<Option<Vec<_>>>() else { continue };
                let tri: [V; 3] = std::array::from_fn(|k| {
                    let v = vs[k];
                    let p = rotate(d.rot, v.pos.map(|c| c * d.scale));
                    let n = rotate(d.rot, v.normal);
                    let base = amb.unwrap_or([1.0; 3]);
                    let c = if unlit_mat {
                        [1.0; 3]
                    } else if opt.unlit {
                        let l = 0.62 + 0.5 * lambert(n);
                        base.map(|q| (q * l).clamp(0.2, 1.6))
                    } else {
                        base
                    };
                    V { p: [d.pos[0] + p[0], d.pos[1] + p[1], d.pos[2] + p[2]], n, uv: v.uv, c: linear(c) }
                });
                let keys = ids.map(|i| (1u64 << 63) | ((di as u64) << 32) | i as u64);
                let before: usize = levels.iter().map(|l| l.triangles).sum();
                for l in &mut levels {
                    l.put(mat, tri, Some(keys));
                }
                any |= levels.iter().map(|l| l.triangles).sum::<usize>() > before;
            }
        }
        if any {
            placed += 1;
        }
    }
    if lights > 0 {
        notes.push(format!("огоньков без тела (свет, не геометрия): {lights}"));
    }
    failed.sort();
    failed.dedup();
    notes.extend(failed.into_iter().take(8));
    for l in &mut levels {
        l.caps();
    }
    let caps: usize = levels.iter().map(|l| l.caps_count).sum();
    if caps > 0 {
        notes.insert(0, format!("крышек среза: {caps}"));
    }

    let mut files = Vec::new();
    let mut boxes = Vec::new();
    let mut parts = Vec::new();
    let mut flowing = Vec::new();
    let mut triangles = 0;
    let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "model".into());
    let dir = out.parent().map(Path::to_path_buf).unwrap_or_default();
    if !dir.as_os_str().is_empty() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    for (l, part) in levels.iter().zip(&part_names) {
        if l.triangles == 0 {
            continue;
        }
        let part = part.clone();
        let file = if part == "L0" { out.to_path_buf() } else { dir.join(format!("{stem}_{part}.glb")) };
        write_glb(&file, &l.prims, &images, opt.unlit, &format!("{}_{part}", scene.name))?;
        triangles += l.triangles;
        boxes.push([l.lo, l.hi]);
        parts.push(part);
        files.push(file);
        flowing.push(l.prims.iter().any(|(m, p)| m.flow != [0, 0] && m.image.is_some() && !p.idx.is_empty()));
    }
    if files.is_empty() {
        return Err("нечего выгружать: ни одного треугольника над полом".into());
    }
    // Рядом — рамка: размер карты в клетках, масштаб, срез и рамка каждой модели в клетках (её
    // тайл в Foundry — в точке 0, 0 и размером с рамку: 3D Canvas тянет рамку модели до тайла).
    let info = json!({
        "name": scene.name,
        "source": path,
        "cells": [wc, hc],
        "yards_per_cell": 1.0 / opt.cells_per_yard,
        "cut_yards": opt.cut,
        "unlit": opt.unlit,
        // Рамка карты в мире (X — север, Y — запад, ярды): клетка (u, v) — точка
        // X = north − (v − margin) / cells_per_yard, Y = west − (u − margin) / cells_per_yard.
        "world": {"north": frame.hi_x, "west": frame.hi_y, "cells_per_yard": frame.cpy, "margin": frame.margin},
        // `flow` — в модели есть текущий рисунок (водопад, вода, лава): тайлу нужен скрипт течения.
        "models": files.iter().zip(&boxes).zip(&parts).zip(&flowing).map(|(((f, b), p), &flow)| json!({"part": p, "file": f.file_name().map(|n| n.to_string_lossy().into_owned()), "min": b[0], "max": b[1], "flow": flow})).collect::<Vec<_>>(),
    });
    // Стены и зоны 3D-сцены Foundry — в точках сцены по 100 на клетку (тайлы моделей там же в
    // 0, 0), по ярусам: земля, мосты над улицей и вторые этажи — свои уровни Foundry. Высоты
    // зон — в клетках над началом моделей: у тайла это его высота плюс клетки × размер клетки.
    let px = |p: [f32; 2]| [(p[0] * 100.0).round() as i64, (p[1] * 100.0).round() as i64];
    let to_walls = |lines: &[Vec<[f32; 2]>]| -> Vec<Value> {
        lines
            .iter()
            .flat_map(|l| location::clip_line(l, wc, hc))
            .flat_map(|l| {
                l.windows(2)
                    .map(|s| {
                        let (a, b) = (px(s[0]), px(s[1]));
                        json!({ "c": [a[0], a[1], b[0], b[1]] })
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let up = |z: f32| ((z - z0) * frame.cpy * 100.0).round() / 100.0;
    let storeys_json: Vec<Value> = storeys
        .iter()
        .enumerate()
        .map(|(lv, s)| {
            let zones: Vec<Value> = s
                .zones
                .iter()
                .map(|z| {
                    let rings: Vec<Vec<i64>> = z.rings.iter().map(|r| r.iter().flat_map(|p| px([p[0].clamp(0.0, wc), p[1].clamp(0.0, hc)])).collect()).collect();
                    json!({"rings": rings, "bottom": up(z.low), "top": up(z.high)})
                })
                .collect();
            json!({"storey": lv, "walls": to_walls(&s.walls), "zones": zones})
        })
        .collect();
    let ground = storeys.first().map(|s| to_walls(&s.walls)).unwrap_or_default();
    let walls_file = dir.join(format!("{stem}_walls.json"));
    std::fs::write(&walls_file, serde_json::to_string(&json!({"px_per_cell": 100, "walls": ground, "storeys": storeys_json})).unwrap_or_default()).map_err(|e| format!("{}: {e}", walls_file.display()))?;
    let upper: Vec<String> = storeys_json
        .iter()
        .skip(1)
        .map(|s| format!("ярус {}: стен {}, зон {}", s["storey"], s["walls"].as_array().map_or(0, Vec::len), s["zones"].as_array().map_or(0, Vec::len)))
        .collect();
    let upper = if upper.is_empty() { String::new() } else { format!("; {}", upper.join("; ")) };
    notes.push(format!("стен для 3D-сцены: на земле {}{upper} (`{stem}_walls.json`)", ground.len()));
    let info_file = dir.join(format!("{stem}.json"));
    std::fs::write(&info_file, serde_json::to_string_pretty(&info).unwrap_or_default()).map_err(|e| format!("{}: {e}", info_file.display()))?;
    Ok(Exported { files, cells: [wc, hc], boxes, triangles, doodads: placed, textures: images.list.len(), notes })
}

/// Двоичный буфер glTF: куски по 4 байта и их описания (`bufferViews`).
#[derive(Default)]
struct Bin {
    data: Vec<u8>,
    views: Vec<Value>,
}

impl Bin {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.data.len().is_multiple_of(4) {
            self.data.push(0);
        }
        let mut v = json!({"buffer": 0, "byteOffset": self.data.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = t.into();
        }
        self.data.extend_from_slice(bytes);
        self.views.push(v);
        self.views.len() - 1
    }
}

fn floats<const N: usize>(vals: impl Iterator<Item = [f32; N]>) -> Vec<u8> {
    vals.flat_map(|a| a.into_iter().flat_map(f32::to_le_bytes)).collect()
}

fn write_glb(file: &Path, prims: &[(Mat, Prim)], images: &Images, unlit: bool, name: &str) -> Result<(), String> {
    const ARRAY: u32 = 34962;
    const ELEMENTS: u32 = 34963;
    let mut bin = Bin::default();
    let mut accessors: Vec<Value> = Vec::new();
    // Картинки — только те, что нужны этому этажу.
    let mut image_ix: HashMap<usize, usize> = HashMap::new();
    let mut images_json: Vec<Value> = Vec::new();
    let mut samplers: Vec<Value> = Vec::new();
    let mut sampler_ix: HashMap<[bool; 2], usize> = HashMap::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut texture_ix: HashMap<(usize, [bool; 2]), usize> = HashMap::new();
    let mut materials: Vec<Value> = Vec::new();
    let mut primitives: Vec<Value> = Vec::new();
    for (mat, prim) in prims {
        if prim.idx.is_empty() {
            continue;
        }
        let tex = match mat.image {
            Some(img) => {
                let src = *image_ix.entry(img).or_insert_with(|| {
                    let (bytes, mime) = &images.list[img];
                    let view = bin.view(bytes, None);
                    images_json.push(json!({"bufferView": view, "mimeType": mime}));
                    images_json.len() - 1
                });
                let smp = *sampler_ix.entry(mat.wrap).or_insert_with(|| {
                    let w = |r: bool| if r { 10497 } else { 33071 };
                    samplers.push(json!({"magFilter": 9729, "minFilter": 9987, "wrapS": w(mat.wrap[0]), "wrapT": w(mat.wrap[1])}));
                    samplers.len() - 1
                });
                Some(*texture_ix.entry((src, mat.wrap)).or_insert_with(|| {
                    textures.push(json!({"sampler": smp, "source": src}));
                    textures.len() - 1
                }))
            }
            None => None,
        };
        let mut pbr = json!({"baseColorFactor": [1.0, 1.0, 1.0, mat.alpha as f32 / 255.0], "metallicFactor": 0.0, "roughnessFactor": 1.0});
        if let Some(t) = tex {
            pbr["baseColorTexture"] = json!({"index": t});
        } else if mat.cap {
            // Цвет стен снимка пола (`WallStyle` [38, 34, 30]) — в линейном.
            pbr["baseColorFactor"] = json!(linear([38.0 / 255.0, 34.0 / 255.0, 30.0 / 255.0]).into_iter().chain([1.0]).collect::<Vec<f32>>());
        } else {
            pbr["baseColorFactor"] = json!([0.55, 0.55, 0.55, mat.alpha as f32 / 255.0]);
        }
        let mode = ["OPAQUE", "MASK", "BLEND"][mat.mode.min(2) as usize];
        let mut m = json!({
            "pbrMetallicRoughness": pbr,
            "alphaMode": mode,
            "doubleSided": mat.two_sided || mat.mode == 1,
        });
        if mat.mode == 1 {
            m["alphaCutoff"] = 0.5.into();
        }
        if unlit {
            m["extensions"] = json!({"KHR_materials_unlit": {}});
        }
        // Three.js кладёт `extras` материала в `userData`: оттуда течение берёт скрипт тайла.
        if mat.flow != [0, 0] {
            m["extras"] = json!({"flow": mat.flow.map(|c| c as f64 / 1000.0)});
        }
        materials.push(m);

        let (lo, hi) = prim.v.iter().fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), v| ([0, 1, 2].map(|k| lo[k].min(v.p[k])), [0, 1, 2].map(|k| hi[k].max(v.p[k]))));
        let n = prim.v.len();
        let mut acc = |bin: &mut Bin, bytes: Vec<u8>, kind: &str, extra: Value| -> usize {
            let view = bin.view(&bytes, Some(ARRAY));
            let mut a = json!({"bufferView": view, "componentType": 5126, "count": n, "type": kind});
            if let Value::Object(e) = extra {
                for (k, v) in e {
                    a[k] = v;
                }
            }
            accessors.push(a);
            accessors.len() - 1
        };
        let pos = acc(&mut bin, floats(prim.v.iter().map(|v| v.p)), "VEC3", json!({"min": lo, "max": hi}));
        let nrm = acc(&mut bin, floats(prim.v.iter().map(|v| v.n)), "VEC3", json!({}));
        let col = acc(&mut bin, floats(prim.v.iter().map(|v| v.c)), "VEC3", json!({}));
        let mut attributes = json!({"POSITION": pos, "NORMAL": nrm, "COLOR_0": col});
        if tex.is_some() {
            attributes["TEXCOORD_0"] = acc(&mut bin, floats(prim.v.iter().map(|v| v.uv)), "VEC2", json!({})).into();
        }
        let idx: Vec<u8> = prim.idx.iter().flat_map(|i| i.to_le_bytes()).collect();
        let view = bin.view(&idx, Some(ELEMENTS));
        accessors.push(json!({"bufferView": view, "componentType": 5125, "count": prim.idx.len(), "type": "SCALAR"}));
        primitives.push(json!({"attributes": attributes, "indices": accessors.len() - 1, "material": materials.len() - 1, "mode": 4}));
    }
    while !bin.data.len().is_multiple_of(4) {
        bin.data.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": format!("Kartograf {}", env!("CARGO_PKG_VERSION"))},
        "scene": 0,
        "scenes": [{"name": name, "nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": primitives}],
        "materials": materials,
        "accessors": accessors,
        "bufferViews": bin.views,
        "buffers": [{"byteLength": bin.data.len()}],
    });
    if !textures.is_empty() {
        doc["textures"] = textures.into();
        doc["images"] = images_json.into();
        doc["samplers"] = samplers.into();
    }
    if unlit {
        doc["extensionsUsed"] = json!(["KHR_materials_unlit"]);
    }
    let mut js = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.data.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin.data);
    std::fs::write(file, out).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(z: f32) -> V {
        V { p: [0.0, 0.0, z], n: [0.0, 0.0, 1.0], uv: [0.0, 0.0], c: [1.0; 3] }
    }

    fn solid(c: [u8; 4]) -> Rgba {
        Rgba { w: 2, h: 2, px: vec![c; 4] }
    }

    #[test]
    fn ground_layers_follow_their_masks() {
        // Нижний слой — красный, верхний — синий по маске: левая половина куска (столбцы маски
        // 0–31, это `u`, к востоку) — синяя, правая — красная.
        let mask: Vec<u8> = (0..64 * 64).map(|k| if k % 64 < 32 { 255 } else { 0 }).collect();
        let paint = adt::Paint { layers: vec![(Some(0), None), (Some(1), Some(mask))], shadow: None };
        let small: HashMap<usize, Rgba> = [(0, solid([200, 0, 0, 255])), (1, solid([0, 0, 200, 255]))].into_iter().collect();
        let r = 16;
        let mut px = vec![[0u8; 4]; r * r];
        bake_chunk(&paint, &small, r, &mut px, r);
        assert_eq!(px[8 * r + 2], [0, 0, 200, 255], "слева — верхний слой");
        assert_eq!(px[8 * r + 13], [200, 0, 0, 255], "справа — нижний слой");
    }

    #[test]
    fn the_top_takes_what_no_storey_took() {
        // Пол первого этажа на 0, второго — на 10, срез 4: этажи берут ниже 4 и 9…14, верх —
        // 4…9 (стена между этажами) и выше 14.
        let taken = vec![level_window(10.0, f32::MIN, 4.0, Some(UPPER_LOW)), level_window(0.0, f32::MIN, 4.0, None)];
        assert_eq!(untaken(taken), vec![(4.0, 9.0), (14.0, f32::MAX)]);
        // Только верхний этаж в точке (крутой склон под площадкой): ниже его — верху.
        assert_eq!(untaken(vec![(9.0, 14.0)]), vec![(f32::MIN, 9.0), (14.0, f32::MAX)]);
        // Пола нет — всё верху.
        assert_eq!(untaken(Vec::new()), vec![(f32::MIN, f32::MAX)]);
    }

    #[test]
    fn a_storey_tile_skips_floor_a_level_already_holds() {
        // Этаж: пол на 0 во всех четырёх клетках, срез 4. Ярус: пол на 3 (в окне этажа — уже в
        // его тайле), на 6 (выше среза — тайлу яруса), пусто, на 6.
        let floors = vec![FloorMap { x0: 0.0, y0: 0.0, step: 1.0, w: 2, h: 2, z: vec![0.0; 4], over: vec![f32::INFINITY; 4] }];
        let got = storey_only(&[3.0, 6.0, f32::INFINITY, 6.0], [0.0, 0.0, 1.0], 2, &floors, |_, _| f32::MIN, 4.0);
        assert_eq!(got, vec![f32::INFINITY, 6.0, f32::INFINITY, 6.0]);
        // Вода выше пола этажа поднимает его окно: пол яруса на 6 над гладью на 3 — уже в окне.
        let wet = storey_only(&[6.0], [0.0, 0.0, 1.0], 1, &floors[..1], |_, _| 3.0, 4.0);
        assert_eq!(wet, vec![f32::INFINITY]);
    }

    #[test]
    fn water_is_tinted_and_lava_brightened() {
        let w = liquid_texture(&solid([128, 128, 128, 255]), wmo::LiquidKind::Water);
        assert!(w.px[0][2] > w.px[0][0], "вода синее, чем красная: {:?}", w.px[0]);
        // Тёмная бледная рябь игры растягивается: впадина — цвет толщи, гребень — светлый.
        let ripple = Rgba { w: 60, h: 1, px: (0..60u8).map(|v| [v, v, v, 255]).collect() };
        let w = liquid_texture(&ripple, wmo::LiquidKind::Water);
        let (dark, light) = (w.px[0], w.px[59]);
        assert!(dark[1] < 110 && light[1] > 200, "рябь на весь размах: {dark:?} … {light:?}");
        assert!(watery("fountains/monasteryfalls.blp"));
        assert!(!watery("generic/lavafall01.blp"), "лавопад — не вода");
        let l = liquid_texture(&solid([100, 50, 0, 255]), wmo::LiquidKind::Magma);
        assert_eq!(l.px[0], [125, 63, 0, 255]);
    }

    #[test]
    fn a_triangle_above_the_cut_loses_its_top() {
        let mut tri = [v(0.0), v(0.0), v(10.0)];
        tri[1].p[0] = 1.0;
        tri[2].p[1] = 1.0;
        let poly = keep(&tri, |q| 4.0 - q.p[2]);
        // Две точки ниже среза и две на срезе — четырёхугольник, весь не выше 4.
        assert_eq!(poly.len(), 4);
        assert!(poly.iter().all(|q| q.p[2] <= 4.0 + 1e-5));
        assert!(keep(&tri, |q| q.p[2] - 20.0).is_empty(), "целиком ниже — пусто");
    }

    #[test]
    fn quaternions_turn_like_the_placement() {
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let r = rotate([0.0, 0.0, h, h], [1.0, 0.0, 0.0]);
        assert!((r[0]).abs() < 1e-5 && (r[1] - 1.0).abs() < 1e-5, "{r:?}");
    }

    #[test]
    fn winding_is_checked_against_normals() {
        let up = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        assert!(!flipped(std::iter::once(up), std::iter::once([0.0, 0.0, 1.0])));
        assert!(flipped(std::iter::once(up), std::iter::once([0.0, 0.0, -1.0])));
    }

    #[test]
    fn glb_has_a_valid_header_and_aligned_chunks() {
        let dir = std::env::temp_dir().join(format!("kartograf_glb_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut prim = Prim::default();
        for z in [0.0, 1.0, 2.0] {
            let i = prim.vert(None, v(z));
            prim.idx.push(i);
        }
        let mat = Mat { image: None, mode: 0, two_sided: false, wrap: [true, true], alpha: 255, cap: false, flow: [0, 0] };
        let file = dir.join("t.glb");
        write_glb(&file, &[(mat, prim)], &Images::default(), true, "t").unwrap();
        let b = std::fs::read(&file).unwrap();
        assert_eq!(&b[..4], b"glTF");
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()) as usize, b.len());
        let js_len = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        assert_eq!(js_len % 4, 0);
        let doc: Value = serde_json::from_slice(&b[20..20 + js_len]).unwrap();
        assert_eq!(doc["accessors"][0]["count"], 3);
        assert_eq!(doc["materials"][0]["extensions"]["KHR_materials_unlit"], json!({}));
        assert!(doc["materials"][0].get("extras").is_none(), "стоячий рисунок — без течения");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flowing_material_carries_its_speed_for_the_tile_script() {
        let dir = std::env::temp_dir().join(format!("kartograf_flow_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut prim = Prim::default();
        for z in [0.0, 1.0, 2.0] {
            let i = prim.vert(None, v(z));
            prim.idx.push(i);
        }
        // Водопад: полтекстуры в секунду по U — как у MonasteryFalls.
        let mat = Mat { image: None, mode: 0, two_sided: true, wrap: [true, false], alpha: 255, cap: false, flow: flow_key([0.5, 0.0]) };
        let file = dir.join("f.glb");
        write_glb(&file, &[(mat, prim)], &Images::default(), true, "f").unwrap();
        let b = std::fs::read(&file).unwrap();
        let js_len = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let doc: Value = serde_json::from_slice(&b[20..20 + js_len]).unwrap();
        assert_eq!(doc["materials"][0]["extras"]["flow"], json!([0.5, 0.0]));
        assert_eq!(flow_key(LIQUID_FLOW[0]), [20, 10], "вода дрейфует медленно");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
