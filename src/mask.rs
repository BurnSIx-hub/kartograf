//! Маска: сила слоя в точках мелкой сетки, `RES` точек на клетку по каждой стороне.
//! Точка `(i, j)` стоит в середине своей подклетки: `((i + 0.5) / RES, (j + 0.5) / RES)`.
//! Так граница между закрашенной клеткой и пустой при пороге 127,5 ложится ровно
//! на линию сетки, а кисть «Клетки» даёт те же стены, что и раньше.
//!
//! Здесь же кисти (что и с какой силой закрашивает мазок) и контур маски,
//! по которому встают стены комнат.

use std::f32::consts::SQRT_2;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const RES: usize = 4;
/// Порог «внутри комнаты».
const LEVEL: f32 = 127.5;

/// Сторона куска маски в точках: 32 клетки. Степень двойки — номер куска сдвигом.
pub const CHUNK: usize = 32 * RES;
/// Сторона плитки отпечатков ([`Mask::chunk_tiles`]) в точках: 2 клетки.
pub const PRINT_TILE: usize = 2 * RES;
const SHIFT: u32 = CHUNK.trailing_zeros();
const AREA: usize = CHUNK * CHUNK;

/// Кусок маски: весь одной силы (пустой, залитый) — без памяти под точки; иначе точки.
/// Точки — за `Arc`: копия маски (шаг отмены) делит с ней куски, а правка копирует
/// только свой кусок.
#[derive(Clone)]
enum Chunk {
    Fill(u8),
    Data(Arc<Vec<u8>>),
}

/// Отпечатки содержимого: у каждой правки новый, у копии — тот же.
static STAMPS: AtomicU64 = AtomicU64::new(1);

/// Маска кусками по [`CHUNK`] точек. Карта 1000×1000 клеток — 4000×4000 точек на слой;
/// сплошным массивом это 16 МБ на слой, на каждом шаге отмены. Кусками — память только
/// под то, где сила разная; пустое и залитое целиком места не занимают.
pub struct Mask {
    pub w: usize,
    pub h: usize,
    nx: usize,
    chunks: Vec<Chunk>,
    /// Куски, которые правили после [`Mask::compact`], — по биту на кусок.
    touched: Vec<u64>,
    /// Отпечаток содержимого; 0 — правили, новый выдаст [`Mask::stamp`].
    stamp: AtomicU64,
    /// Отпечатки кусков, как `stamp`, но по куску: отрисовка перестраивает только те
    /// куски земли, чей отпечаток (или соседей) сменился.
    gens: Vec<AtomicU64>,
}

impl Clone for Mask {
    fn clone(&self) -> Self {
        // Отпечаток — до копии: иначе копия и оригинал получили бы разные, хотя одинаковы.
        let gens = (0..self.chunks.len()).map(|c| AtomicU64::new(self.chunk_gen(c))).collect();
        Self { w: self.w, h: self.h, nx: self.nx, chunks: self.chunks.clone(), touched: self.touched.clone(), stamp: AtomicU64::new(self.stamp()), gens }
    }
}

impl std::fmt::Debug for Mask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.chunks.iter().filter(|c| matches!(c, Chunk::Data(_))).count();
        write!(f, "Mask({}×{}, кусков с точками {data} из {})", self.w, self.h, self.chunks.len())
    }
}

impl PartialEq for Mask {
    fn eq(&self, other: &Self) -> bool {
        if (self.w, self.h) != (other.w, other.h) {
            return false;
        }
        (0..self.chunks.len()).all(|c| match (&self.chunks[c], &other.chunks[c]) {
            (Chunk::Fill(a), Chunk::Fill(b)) => a == b,
            (Chunk::Data(a), Chunk::Data(b)) if Arc::ptr_eq(a, b) => true,
            _ => self.chunk_rows(c).all(|(j, i0, i1)| (i0..i1).all(|i| self.get_u(i, j) == other.get_u(i, j))),
        })
    }
}

impl Mask {
    /// Маска `w × h` точек одной силы `v`.
    pub fn filled(w: usize, h: usize, v: u8) -> Self {
        let nx = w.div_ceil(CHUNK);
        let ny = h.div_ceil(CHUNK);
        Self { w, h, nx, chunks: vec![Chunk::Fill(v); nx * ny], touched: vec![0; (nx * ny).div_ceil(64)], stamp: AtomicU64::new(0), gens: (0..nx * ny).map(|_| AtomicU64::new(0)).collect() }
    }

    /// Пустая маска `w × h` точек.
    pub fn new(w: usize, h: usize) -> Self {
        Self::filled(w, h, 0)
    }

    /// Маска под карту `cells_w × cells_h` клеток.
    pub fn for_cells(cells_w: u32, cells_h: u32) -> Self {
        Self::new(cells_w as usize * RES, cells_h as usize * RES)
    }

    /// Пустая маска того же размера.
    pub fn blank_like(&self) -> Self {
        Self::new(self.w, self.h)
    }

    /// Маска из точек подряд, строками (как прежний сплошной массив).
    pub fn from_vec(w: usize, h: usize, data: &[u8]) -> Self {
        let mut m = Self::new(w, h);
        for j in 0..h {
            for i in 0..w {
                let v = data.get(j * w + i).copied().unwrap_or(0);
                if v != 0 {
                    m.set(i, j, v);
                }
            }
        }
        m.compact();
        m
    }

    /// Точки подряд, строками.
    #[cfg(test)]
    pub fn to_vec(&self) -> Vec<u8> {
        self.values().collect()
    }

    /// Сколько точек.
    pub fn len(&self) -> usize {
        self.w * self.h
    }

    #[inline]
    fn locate(&self, i: usize, j: usize) -> (usize, usize) {
        ((j >> SHIFT) * self.nx + (i >> SHIFT), ((j & (CHUNK - 1)) << SHIFT) | (i & (CHUNK - 1)))
    }

    /// Значение точки внутри маски.
    #[inline]
    fn get_u(&self, i: usize, j: usize) -> u8 {
        let (c, o) = self.locate(i, j);
        match &self.chunks[c] {
            Chunk::Fill(v) => *v,
            Chunk::Data(d) => d[o],
        }
    }

    /// Значение точки; за краем — 0.
    #[inline]
    pub fn get(&self, i: isize, j: isize) -> u8 {
        if i < 0 || j < 0 || i as usize >= self.w || j as usize >= self.h { 0 } else { self.get_u(i as usize, j as usize) }
    }

    /// Значение точки по номеру строками (`j * w + i`), как в прежнем массиве.
    #[inline]
    pub fn at(&self, k: usize) -> u8 {
        if k >= self.len() { 0 } else { self.get_u(k % self.w, k / self.w) }
    }

    /// Поставить силу точки; за краем — ничего.
    #[inline]
    pub fn set(&mut self, i: usize, j: usize, v: u8) {
        if i >= self.w || j >= self.h {
            return;
        }
        let (c, o) = self.locate(i, j);
        match &mut self.chunks[c] {
            Chunk::Fill(x) if *x == v => return,
            Chunk::Fill(x) => {
                let mut d = vec![*x; AREA];
                d[o] = v;
                self.chunks[c] = Chunk::Data(Arc::new(d));
            }
            Chunk::Data(d) => {
                if d[o] == v {
                    return;
                }
                Arc::make_mut(d)[o] = v;
            }
        }
        self.touched[c >> 6] |= 1 << (c & 63);
        self.stamp.store(0, Ordering::Relaxed);
        self.gens[c].store(0, Ordering::Relaxed);
    }

    /// Поставить силу точки по номеру строками.
    #[inline]
    pub fn put(&mut self, k: usize, v: u8) {
        if k < self.len() {
            self.set(k % self.w, k / self.w, v);
        }
    }

    /// Точки подряд, строками.
    pub fn values(&self) -> impl Iterator<Item = u8> + '_ {
        (0..self.h).flat_map(move |j| (0..self.w).map(move |i| self.get_u(i, j)))
    }

    /// Строки куска `c` в пределах маски: `(j, i0, i1)`.
    fn chunk_rows(&self, c: usize) -> impl Iterator<Item = (usize, usize, usize)> {
        let (cx, cy) = (c % self.nx, c / self.nx);
        let (i0, j0) = (cx * CHUNK, cy * CHUNK);
        let (i1, j1) = ((i0 + CHUNK).min(self.w), (j0 + CHUNK).min(self.h));
        (j0..j1).map(move |j| (j, i0, i1))
    }

    /// Сила куска, в котором точка `(i, j)`, если он весь одной силы.
    pub fn uniform_at(&self, i: usize, j: usize) -> Option<u8> {
        if i >= self.w || j >= self.h {
            return Some(0);
        }
        match &self.chunks[self.locate(i, j).0] {
            Chunk::Fill(v) => Some(*v),
            Chunk::Data(_) => None,
        }
    }

    /// Куски, ставшие одной силы, — снова без памяти под точки. Смотрит только те, что
    /// правили после прошлого раза: на большой карте это доли процента кусков.
    pub fn compact(&mut self) {
        let touched = std::mem::replace(&mut self.touched, vec![0; self.chunks.len().div_ceil(64)]);
        for (word, bits) in touched.into_iter().enumerate() {
            let mut bits = bits;
            while bits != 0 {
                let c = word * 64 + bits.trailing_zeros() as usize;
                bits &= bits - 1;
                self.compact_one(c);
            }
        }
    }

    fn compact_one(&mut self, c: usize) {
        let Chunk::Data(d) = &self.chunks[c] else { return };
        let (cx, cy) = (c % self.nx, c / self.nx);
        let (ew, eh) = ((self.w - cx * CHUNK).min(CHUNK), (self.h - cy * CHUNK).min(CHUNK));
        let first = d[0];
        if (0..eh).all(|r| d[r * CHUNK..r * CHUNK + ew].iter().all(|&v| v == first)) {
            self.chunks[c] = Chunk::Fill(first);
        }
    }

    /// Отпечаток содержимого: тот же — содержимое то же (копия или без правок).
    pub fn stamp(&self) -> u64 {
        let s = self.stamp.load(Ordering::Relaxed);
        if s != 0 {
            return s;
        }
        let fresh = STAMPS.fetch_add(1, Ordering::Relaxed);
        match self.stamp.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => fresh,
            Err(now) => now,
        }
    }

    /// Каждая точка с силой больше нуля: `(i, j, сила)`, кусками; пустые куски пропускаются.
    pub fn for_each_nonzero(&self, mut f: impl FnMut(usize, usize, u8)) {
        for c in 0..self.chunks.len() {
            match &self.chunks[c] {
                Chunk::Fill(0) => {}
                Chunk::Fill(v) => {
                    for (j, i0, i1) in self.chunk_rows(c) {
                        for i in i0..i1 {
                            f(i, j, *v);
                        }
                    }
                }
                Chunk::Data(d) => {
                    let (x0, y0) = ((c % self.nx) * CHUNK, (c / self.nx) * CHUNK);
                    for (j, i0, i1) in self.chunk_rows(c) {
                        let row = &d[(j - y0) * CHUNK..];
                        for i in i0..i1 {
                            let v = row[i - x0];
                            if v != 0 {
                                f(i, j, v);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Сумма сил всех точек.
    pub fn sum(&self) -> u64 {
        let mut s = 0u64;
        self.for_each_nonzero(|_, _, v| s += v as u64);
        s
    }

    /// Сложить с другой маской того же размера (с насыщением на 255).
    pub fn add_sat(&mut self, other: &Mask) {
        other.for_each_nonzero(|i, j, v| {
            let a = self.get_u(i, j);
            self.set(i, j, a.saturating_add(v));
        });
    }

    /// Кусков по горизонтали и по вертикали.
    pub fn chunks_wh(&self) -> (usize, usize) {
        (self.nx, self.chunks.len() / self.nx.max(1))
    }

    /// Отпечаток куска `c`: тот же — содержимое куска то же. У куска одной силы он по
    /// самой силе: одинаковые залитые куски разных слоёв и шагов отмены совпадают.
    pub fn chunk_gen(&self, c: usize) -> u64 {
        match self.chunks.get(c) {
            None => 0,
            Some(Chunk::Fill(v)) => (1 << 63) | *v as u64,
            Some(Chunk::Data(_)) => {
                let g = self.gens[c].load(Ordering::Relaxed);
                if g != 0 {
                    return g;
                }
                let fresh = STAMPS.fetch_add(1, Ordering::Relaxed);
                match self.gens[c].compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => fresh,
                    Err(now) => now,
                }
            }
        }
    }

    /// Отпечатки плиток куска `c` ([`PRINT_TILE`]×[`PRINT_TILE`] точек) по строкам,
    /// одинаковые от запуска к запуску (кэш снимков выгрузки лежит на диске): плитка одной
    /// силы — по силе, где бы она ни лежала, иначе по точкам. Так кэш видит, где маска
    /// поменялась, с точностью до двух клеток, а не до куска в 32.
    pub fn chunk_tiles(&self, c: usize) -> Vec<u64> {
        use std::hash::{Hash, Hasher};
        const N: usize = CHUNK / PRINT_TILE;
        let flat = |v: u8| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (1u8, v).hash(&mut h);
            h.finish()
        };
        match self.chunks.get(c) {
            None => vec![0; N * N],
            Some(Chunk::Fill(v)) => vec![flat(*v); N * N],
            Some(Chunk::Data(d)) => (0..N * N)
                .map(|t| {
                    let (tx, ty) = (t % N, t / N);
                    let rows = (0..PRINT_TILE).map(|y| (ty * PRINT_TILE + y) * CHUNK + tx * PRINT_TILE);
                    let first = d[ty * PRINT_TILE * CHUNK + tx * PRINT_TILE];
                    if rows.clone().all(|at| d[at..at + PRINT_TILE].iter().all(|&v| v == first)) {
                        return flat(first);
                    }
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    2u8.hash(&mut h);
                    for at in rows {
                        h.write(&d[at..at + PRINT_TILE]);
                    }
                    h.finish()
                })
                .collect(),
        }
    }

    /// Отпечаток пустой плитки ([`Mask::chunk_tiles`]).
    pub fn empty_tile() -> u64 {
        Self::for_cells(1, 1).chunk_tiles(0)[0]
    }

    /// Сколько кусков держат свои точки (для замеров памяти).
    pub fn data_chunks(&self) -> usize {
        self.chunks.iter().filter(|c| matches!(c, Chunk::Data(_))).count()
    }

    /// Сколько байт занимают точки кусков (копии для отмены делят их с картой).
    pub fn bytes(&self) -> usize {
        self.data_chunks() * AREA
    }

    pub fn is_empty(&self) -> bool {
        (0..self.chunks.len()).all(|c| match &self.chunks[c] {
            Chunk::Fill(v) => *v == 0,
            Chunk::Data(_) => self.chunk_rows(c).all(|(j, i0, i1)| (i0..i1).all(|i| self.get_u(i, j) == 0)),
        })
    }

    /// Та же маска на холсте `cells_w × cells_h` клеток, сдвинутая на `dx`, `dy`
    /// клеток; новые поля пустые. Холст не меньше прежнего со сдвигом.
    pub fn padded(&self, cells_w: u32, cells_h: u32, dx: u32, dy: u32) -> Self {
        let mut out = Self::for_cells(cells_w, cells_h);
        let (ox, oy) = (dx as usize * RES, dy as usize * RES);
        for j in 0..self.h.min(out.h.saturating_sub(oy)) {
            for i in 0..self.w.min(out.w.saturating_sub(ox)) {
                if self.uniform_at(i, j) == Some(0) {
                    continue;
                }
                out.set(i + ox, j + oy, self.get_u(i, j));
            }
        }
        out.compact();
        out
    }

    /// Кусок маски: `cells_w`×`cells_h` клеток от клетки `x0, y0` (район карты отдельной
    /// картой). За краем исходной — пусто.
    pub fn cropped(&self, cells_w: u32, cells_h: u32, x0: u32, y0: u32) -> Self {
        let mut out = Self::for_cells(cells_w, cells_h);
        let (ox, oy) = (x0 as usize * RES, y0 as usize * RES);
        for j in 0..out.h.min(self.h.saturating_sub(oy)) {
            for i in 0..out.w.min(self.w.saturating_sub(ox)) {
                if self.uniform_at(i + ox, j + oy) == Some(0) {
                    continue;
                }
                out.set(i, j, self.get_u(i + ox, j + oy));
            }
        }
        out.compact();
        out
    }

    /// Площадь выше порога, в клетках.
    pub fn area(&self) -> f32 {
        let mut n = 0usize;
        for c in 0..self.chunks.len() {
            match &self.chunks[c] {
                Chunk::Fill(v) if *v as f32 > LEVEL => n += self.chunk_rows(c).map(|(_, i0, i1)| i1 - i0).sum::<usize>(),
                Chunk::Fill(_) => {}
                Chunk::Data(_) => n += self.chunk_rows(c).map(|(j, i0, i1)| (i0..i1).filter(|&i| self.get_u(i, j) as f32 > LEVEL).count()).sum::<usize>(),
            }
        }
        n as f32 / (RES * RES) as f32
    }
}

pub fn sample_pos(i: isize, j: isize) -> [f32; 2] {
    [(i as f32 + 0.5) / RES as f32, (j as f32 + 0.5) / RES as f32]
}

// ----- запись в файл: сжатые байты в base64, а не тысячи чисел -----
//
// Формат 3 (`t`): куски по порядку, у каждого метка — 0 и сила (весь одной силы) или 1 и
// его точки в пределах маски строками; всё вместе сжато. Пустая карта 1000×1000 — пара
// сотен байт, и запись не проходит по 16 миллионам точек каждого слоя.
// Формат 2 (`z`): все точки подряд строками, сжатые, — читается по-прежнему.

#[derive(Serialize)]
struct Packed {
    w: usize,
    h: usize,
    t: String,
}

#[derive(Deserialize)]
struct Read {
    w: usize,
    h: usize,
    #[serde(default)]
    z: Option<String>,
    #[serde(default)]
    t: Option<String>,
}

impl Serialize for Mask {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use std::io::Write;
        let mut m = self.clone();
        m.compact();
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        let mut row = Vec::with_capacity(CHUNK);
        for c in 0..m.chunks.len() {
            match &m.chunks[c] {
                Chunk::Fill(v) => {
                    let _ = enc.write_all(&[0, *v]);
                }
                Chunk::Data(_) => {
                    let _ = enc.write_all(&[1]);
                    for (j, i0, i1) in m.chunk_rows(c) {
                        row.clear();
                        row.extend((i0..i1).map(|i| m.get_u(i, j)));
                        let _ = enc.write_all(&row);
                    }
                }
            }
        }
        let bytes = enc.finish().unwrap_or_default();
        Packed { w: self.w, h: self.h, t: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Mask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        use std::io::Read as _;
        let p = Read::deserialize(d)?;
        let unpack = |text: &str| -> Result<Vec<u8>, D::Error> {
            let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, text).map_err(D::Error::custom)?;
            let mut data = Vec::new();
            flate2::read::DeflateDecoder::new(&bytes[..]).read_to_end(&mut data).map_err(D::Error::custom)?;
            Ok(data)
        };
        let broken = || D::Error::custom("маска повреждена");
        match (&p.t, &p.z) {
            (Some(t), _) => {
                let data = unpack(t)?;
                let mut m = Mask::new(p.w, p.h);
                let mut at = 0;
                for c in 0..m.chunks.len() {
                    match data.get(at) {
                        Some(0) => {
                            m.chunks[c] = Chunk::Fill(*data.get(at + 1).ok_or_else(broken)?);
                            at += 2;
                        }
                        Some(1) => {
                            at += 1;
                            let rows: Vec<(usize, usize, usize)> = m.chunk_rows(c).collect();
                            let mut d = vec![0u8; AREA];
                            for (r, &(_, i0, i1)) in rows.iter().enumerate() {
                                let n = i1 - i0;
                                d[r * CHUNK..r * CHUNK + n].copy_from_slice(data.get(at..at + n).ok_or_else(broken)?);
                                at += n;
                            }
                            m.chunks[c] = Chunk::Data(Arc::new(d));
                        }
                        _ => return Err(broken()),
                    }
                }
                if at != data.len() {
                    return Err(broken());
                }
                Ok(m)
            }
            (None, Some(z)) => {
                let data = unpack(z)?;
                if data.len() != p.w * p.h {
                    return Err(broken());
                }
                Ok(Mask::from_vec(p.w, p.h, &data))
            }
            (None, None) => Err(D::Error::custom("у маски нет точек")),
        }
    }
}

// ----- кисти -----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Целые клетки протяжкой прямоугольника.
    Cells,
    /// Овал, вписанный в протянутый прямоугольник клеток: круглая комната одним жестом.
    Oval,
    Square,
    Circle,
    /// Россыпь мелких мягких пятен.
    Spray,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Cells, Shape::Oval, Shape::Square, Shape::Circle, Shape::Spray];

    /// Ставится протяжкой рамки, а не мазком.
    pub fn drags(self) -> bool {
        matches!(self, Shape::Cells | Shape::Oval)
    }

    pub fn label(self) -> &'static str {
        match self {
            Shape::Cells => "Клетки",
            Shape::Oval => "Овал",
            Shape::Square => "Квадрат",
            Shape::Circle => "Круг",
            Shape::Spray => "Спрей",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Brush {
    pub shape: Shape,
    /// Половина стороны квадрата или радиус круга, в клетках.
    pub radius: f32,
    /// Доля радиуса с полной силой; дальше — плавный спад к краю.
    pub hardness: f32,
    /// Сколько даёт один отпечаток: 1 — сразу всё.
    pub flow: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Self { shape: Shape::Circle, radius: 1.5, hardness: 0.8, flow: 1.0 }
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Затухание фигуры к краю: сила спадает с расстоянием от края внутрь. На `fade` = 1
/// спад идёт от самой глубокой точки, на 0 фигура остаётся как была.
pub fn soften(w: usize, h: usize, weights: &mut [(usize, f32)], fade: f32) {
    if fade <= 0.0 || weights.is_empty() {
        return;
    }
    let inside = |i: usize| (i % w, i / w);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for &(i, _) in weights.iter() {
        let (x, y) = inside(i);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    // Расстояние до края по двум проходам: прямой шаг 1, косой √2. Поле шире фигуры на
    // точку: снаружи ноль. С `beyond` фигура, что упирается в край карты, за ним
    // продолжается: рамка карты — не край фигуры (берег, нарисованный за край, не
    // тает у рамки и не оставляет под собой полоску воды).
    let (fw, fh) = (x1 - x0 + 3, y1 - y0 + 3);
    let field = |beyond: bool| {
        let mut dist = vec![0f32; fw * fh];
        for &(i, v) in weights.iter() {
            if v >= 0.5 {
                let (x, y) = inside(i);
                dist[(y - y0 + 1) * fw + x - x0 + 1] = f32::MAX;
            }
        }
        if beyond {
            for y in 0..fh {
                if x0 == 0 {
                    dist[y * fw] = dist[y * fw + 1];
                }
                if x1 + 1 == w {
                    dist[y * fw + fw - 1] = dist[y * fw + fw - 2];
                }
            }
            for x in 0..fw {
                if y0 == 0 {
                    dist[x] = dist[fw + x];
                }
                if y1 + 1 == h {
                    dist[(fh - 1) * fw + x] = dist[(fh - 2) * fw + x];
                }
            }
        }
        for pass in 0..2 {
            for k in 0..fw * fh {
                let k = if pass == 0 { k } else { fw * fh - 1 - k };
                let (x, y) = (k % fw, k / fw);
                if x == 0 || y == 0 || x == fw - 1 || y == fh - 1 || dist[k] == 0.0 {
                    continue;
                }
                let around = if pass == 0 { [(k - 1, 1.0), (k - fw, 1.0), (k - fw - 1, SQRT_2), (k - fw + 1, SQRT_2)] } else { [(k + 1, 1.0), (k + fw, 1.0), (k + fw + 1, SQRT_2), (k + fw - 1, SQRT_2)] };
                dist[k] = around.iter().map(|&(n, cost)| dist[n] + cost).fold(dist[k], f32::min);
            }
        }
        dist
    };
    // Ширина спада — по самой фигуре, как и раньше; рамка карты её не меняет.
    let plain = field(false);
    let deepest = plain.iter().copied().filter(|d| *d < f32::MAX).fold(0.0, f32::max);
    let touches = x0 == 0 || y0 == 0 || x1 + 1 == w || y1 + 1 == h;
    let dist = if touches { field(true) } else { plain };
    let reach = (deepest * fade).max(1.0);
    for (i, v) in weights.iter_mut() {
        let (x, y) = inside(*i);
        let d = dist[(y - y0 + 1) * fw + x - x0 + 1];
        *v *= smooth(d / reach);
    }
}

/// Сила по расстоянию от центра: внутри `hard` — 1, у края `r` — 0.
fn falloff(d: f32, r: f32, hardness: f32) -> f32 {
    // Даже у самой жёсткой кисти край размыт на шаг маски: иначе круг ложится
    // лесенкой и стена вокруг него идёт зубцами.
    // Полоса размытия стоит на самом крае, чтобы половина силы приходилась ровно на `r`.
    let aa = 0.5 / RES as f32;
    let hard = (r * hardness.clamp(0.0, 1.0)).min(r - aa).max(0.0);
    let outer = r + aa;
    if d <= hard {
        1.0
    } else if d >= outer {
        0.0
    } else {
        1.0 - smooth((d - hard) / (outer - hard).max(1e-4))
    }
}

/// Отпечаток кисти: точки маски и сила для каждой (0..1).
/// `seed` нужен спрею: каждый отпечаток рассыпается по-своему, но повторимо.
pub fn dab(w: usize, h: usize, brush: &Brush, c: [f32; 2], seed: u32) -> Vec<(usize, f32)> {
    let r = brush.radius.max(0.05);
    let res = RES as f32;
    let (i0, i1) = (((c[0] - r) * res - 1.0).floor().max(0.0) as usize, (((c[0] + r) * res + 1.0).ceil().max(0.0) as usize).min(w));
    let (j0, j1) = (((c[1] - r) * res - 1.0).floor().max(0.0) as usize, (((c[1] + r) * res + 1.0).ceil().max(0.0) as usize).min(h));
    let mut out = Vec::new();
    match brush.shape {
        Shape::Cells | Shape::Oval | Shape::Square | Shape::Circle => {
            for j in j0..j1 {
                for i in i0..i1 {
                    let p = sample_pos(i as isize, j as isize);
                    let (dx, dy) = ((p[0] - c[0]).abs(), (p[1] - c[1]).abs());
                    let d = if brush.shape == Shape::Circle { (dx * dx + dy * dy).sqrt() } else { dx.max(dy) };
                    let v = falloff(d, r, brush.hardness) * brush.flow;
                    if v > 0.0 {
                        out.push((j * w + i, v));
                    }
                }
            }
        }
        Shape::Spray => {
            // Пятна радиусом в пятую часть кисти, равномерно по кругу.
            let mut rng = crate::generate::Rng::new(&seed.to_string());
            let dots = ((r * r * 6.0) as usize).clamp(3, 60);
            let spot = Brush { shape: Shape::Circle, radius: (r * 0.2).max(0.2), hardness: brush.hardness * 0.5, flow: brush.flow };
            let mut acc: HashMap<usize, f32> = HashMap::new();
            for _ in 0..dots {
                let a = rng.random() as f32 * std::f32::consts::TAU;
                let d = r * (rng.random() as f32).sqrt();
                for (i, v) in dab(w, h, &spot, [c[0] + a.cos() * d, c[1] + a.sin() * d], 0) {
                    let e = acc.entry(i).or_insert(0.0);
                    *e = 1.0 - (1.0 - *e) * (1.0 - v);
                }
            }
            out.extend(acc);
        }
    }
    out
}

/// Целые клетки прямоугольника между двумя углами (включительно).
pub fn cells(w: usize, h: usize, a: (i32, i32), b: (i32, i32)) -> Vec<(usize, f32)> {
    let cw = (w / RES) as i32;
    let ch = (h / RES) as i32;
    let (x0, x1) = (a.0.min(b.0).max(0), a.0.max(b.0).min(cw - 1));
    let (y0, y1) = (a.1.min(b.1).max(0), a.1.max(b.1).min(ch - 1));
    let mut out = Vec::new();
    if x0 > x1 || y0 > y1 {
        return out;
    }
    for j in y0 as usize * RES..(y1 as usize + 1) * RES {
        for i in x0 as usize * RES..(x1 as usize + 1) * RES {
            out.push((j * w + i, 1.0));
        }
    }
    out
}

/// Овал, вписанный в прямоугольник клеток между двумя углами (включительно),
/// с краем, размытым на шаг маски, — как у круглой кисти.
pub fn oval(w: usize, h: usize, a: (i32, i32), b: (i32, i32)) -> Vec<(usize, f32)> {
    oval_f(w, h, [a.0.min(b.0) as f32, a.1.min(b.1) as f32, (a.0.max(b.0) + 1) as f32, (a.1.max(b.1) + 1) as f32])
}

/// Точки маски от `lo` до `hi` клеток по одной оси: середины подклеток внутри.
fn span(lo: f32, hi: f32, n: usize) -> std::ops::Range<usize> {
    let at = |v: f32| ((v * RES as f32 - 0.5).ceil().max(0.0) as usize).min(n);
    at(lo)..at(hi)
}

/// Прямоугольник `[x0, y0, x1, y1]` в клетках, края — с точностью шага маски
/// (четверть клетки): пол до 9,5 встаёт ровно под стену на 9,5. На целых — то же,
/// что [`cells`].
pub fn rect_f(w: usize, h: usize, r: [f32; 4]) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    for j in span(r[1].min(r[3]), r[1].max(r[3]), h) {
        for i in span(r[0].min(r[2]), r[0].max(r[2]), w) {
            out.push((j * w + i, 1.0));
        }
    }
    out
}

/// Многоугольник по точкам в клетках; край размыт на шаг маски, как у овала.
pub fn poly(w: usize, h: usize, pts: &[[f32; 2]]) -> Vec<(usize, f32)> {
    if pts.len() < 3 {
        return Vec::new();
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        (x0, y0, x1, y1) = (x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1]));
    }
    let pad = 1.0 / RES as f32;
    let mut out = Vec::new();
    for j in span(y0 - pad, y1 + pad, h) {
        for i in span(x0 - pad, x1 + pad, w) {
            let p = sample_pos(i as isize, j as isize);
            let mut inside = false;
            let mut d = f32::MAX;
            for k in 0..pts.len() {
                let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                    inside = !inside;
                }
                let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
                let t = (((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / (ex * ex + ey * ey).max(1e-9)).clamp(0.0, 1.0);
                d = d.min(((p[0] - a[0] - ex * t).powi(2) + (p[1] - a[1] - ey * t).powi(2)).sqrt());
            }
            let v = (0.5 + if inside { d } else { -d } * RES as f32).clamp(0.0, 1.0);
            if v > 0.0 {
                out.push((j * w + i, v));
            }
        }
    }
    out
}

/// Овал, вписанный в прямоугольник `[x0, y0, x1, y1]` клеток (края — дробные),
/// с краем, размытым на шаг маски.
pub fn oval_f(w: usize, h: usize, r: [f32; 4]) -> Vec<(usize, f32)> {
    let (x0, x1) = (r[0].min(r[2]), r[0].max(r[2]));
    let (y0, y1) = (r[1].min(r[3]), r[1].max(r[3]));
    let c = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    let (rx, ry) = (((x1 - x0) / 2.0).max(1e-3), ((y1 - y0) / 2.0).max(1e-3));
    let aa = 0.5 / RES as f32;
    let at = |v: f32, n: usize| ((v * RES as f32).max(0.0) as usize).min(n);
    let mut out = Vec::new();
    for j in at(y0.floor(), h)..at(y1.ceil(), h) {
        for i in at(x0.floor(), w)..at(x1.ceil(), w) {
            let p = sample_pos(i as isize, j as isize);
            let (dx, dy) = ((p[0] - c[0]) / rx, (p[1] - c[1]) / ry);
            // Расстояние до края по нормали — приближённо, через градиент.
            let f = (dx * dx + dy * dy).sqrt();
            let g = ((dx / rx).powi(2) + (dy / ry).powi(2)).sqrt() / f.max(1e-6);
            let d = (f - 1.0) / g.max(1e-6);
            let v = (0.5 - d / (2.0 * aa)).clamp(0.0, 1.0);
            if v > 0.0 {
                out.push((j * w + i, v));
            }
        }
    }
    out
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    #[test]
    fn fractional_shapes_land_on_quarter_cells() {
        let (w, h) = (20 * RES, 16 * RES);
        let mut a = rect_f(w, h, [4.0, 3.0, 14.0, 11.0]);
        let mut b = cells(w, h, (4, 3), (13, 10));
        a.sort_by_key(|p| p.0);
        b.sort_by_key(|p| p.0);
        assert_eq!(a, b, "на целых — как клетки");
        let r = rect_f(w, h, [9.5, 7.25, 12.0, 8.0]);
        assert_eq!(r.len(), (2.5 * RES as f32) as usize * (0.75 * RES as f32) as usize);
        let xs: Vec<usize> = r.iter().map(|p| p.0 % w).collect();
        assert_eq!(*xs.iter().min().unwrap(), 38, "левый край — ровно 9,5");
        // Квадрат многоугольником — тот же прямоугольник.
        let mut p = poly(w, h, &[[4.0, 3.0], [14.0, 3.0], [14.0, 11.0], [4.0, 11.0]]);
        p.sort_by_key(|q| q.0);
        assert_eq!(p, a);
        // Треугольник: половина квадрата 4×4, край размыт.
        let t: f32 = poly(w, h, &[[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]]).iter().map(|q| q.1).sum::<f32>() / (RES * RES) as f32;
        assert!((t - 8.0).abs() < 0.3, "{t}");
        // Дробный овал — площадь πab.
        let o: f32 = oval_f(w, h, [2.5, 2.5, 8.5, 6.5]).iter().map(|q| q.1).sum::<f32>() / (RES * RES) as f32;
        assert!((o - std::f32::consts::PI * 3.0 * 2.0).abs() < 0.3, "{o}");
    }

    #[test]
    fn feather_fades_the_edge_and_keeps_the_middle() {
        let (w, h) = (20 * RES, 20 * RES);
        let mut o = oval_f(w, h, [2.0, 2.0, 18.0, 18.0]);
        soften(w, h, &mut o, 0.35);
        let at = |x: f32, y: f32| o.iter().find(|q| q.0 == (y * RES as f32) as usize * w + (x * RES as f32) as usize).map_or(0.0, |q| q.1);
        assert!(at(10.0, 10.0) > 0.99, "середина");
        assert!(at(2.2, 10.0) < 0.2, "край мягкий: {}", at(2.2, 10.0));
        assert!(at(4.0, 10.0) > at(2.5, 10.0), "к середине сильнее");
    }

    #[test]
    fn feather_does_not_fade_at_the_map_border() {
        // Берег у левого края карты: у рамки он в полную силу, тает только к воде.
        let (w, h) = (20 * RES, 20 * RES);
        let mut r = rect_f(w, h, [-1.0, -1.0, 6.0, 21.0]);
        soften(w, h, &mut r, 0.3);
        let at = |x: f32, y: f32| r.iter().find(|q| q.0 == (y * RES as f32) as usize * w + (x * RES as f32) as usize).map_or(0.0, |q| q.1);
        assert!(at(0.05, 10.0) > 0.99, "у рамки слева: {}", at(0.05, 10.0));
        assert!(at(3.0, 0.05) > 0.99, "у рамки сверху: {}", at(3.0, 0.05));
        assert!(at(5.9, 10.0) < 0.3, "к воде тает: {}", at(5.9, 10.0));
        // Ширина спада у фигуры, что упирается в рамку, та же, что без рамки.
        let mut free = rect_f(w, h, [7.0, 2.0, 14.0, 18.0]);
        soften(w, h, &mut free, 0.3);
        let at2 = |x: f32, y: f32| free.iter().find(|q| q.0 == (y * RES as f32) as usize * w + (x * RES as f32) as usize).map_or(0.0, |q| q.1);
        assert!((at(5.0, 10.0) - at2(13.0, 10.0)).abs() < 0.08, "{} против {}", at(5.0, 10.0), at2(13.0, 10.0));
    }
}

// ----- контур -----

pub type Segment = ([f32; 2], [f32; 2]);

/// Граница маски на пороге — замкнутые ломаные. Квадраты сетки (marching squares),
/// склейка кусков в ломаные, спрямление: точки на одной прямой убираются, срезанные
/// углы прямоугольников возвращаются острыми, остальные изгибы сглаживаются.
pub fn contour(m: &Mask) -> Vec<Segment> {
    contour_lines(contour_raw(m))
}

/// Куски границы по квадратам сетки, ещё не склеенные.
fn contour_raw(m: &Mask) -> Vec<Segment> {
    let v = |i: isize, j: isize| m.get(i, j) as f32;
    let lerp = |a: [f32; 2], b: [f32; 2], va: f32, vb: f32| {
        let t = if (vb - va).abs() < 1e-6 { 0.5 } else { ((LEVEL - va) / (vb - va)).clamp(0.0, 1.0) };
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    };
    let mut pieces: Vec<Segment> = Vec::new();
    let above = |v: u8| v as f32 > LEVEL;
    for j in -1..m.h as isize {
        let mut next = -1isize;
        while next < m.w as isize {
            let i = next;
            next += 1;
            // Квадраты, у которых все четыре точки в двух кусках одной силы по одну
            // сторону порога, границы не дают: весь ряд таких квадратов в куске — мимо.
            // Порядок остальных прежний, поэтому и стены выходят те же.
            if i >= 0 && j >= 0 && j + 1 < m.h as isize && i as usize % CHUNK == 0 {
                let end = (i as usize + CHUNK - 1).min(m.w - 1);
                let (top, low) = (m.uniform_at(i as usize, j as usize), m.uniform_at(i as usize, j as usize + 1));
                if end > i as usize
                    && let (Some(t), Some(l)) = (top, low)
                    && above(t) == above(l)
                {
                    next = end as isize;
                    continue;
                }
            }
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
            let case = (a > LEVEL) as u8 | ((b > LEVEL) as u8) << 1 | ((c > LEVEL) as u8) << 2 | ((d > LEVEL) as u8) << 3;
            if case == 0 || case == 15 {
                continue;
            }
            let (pa, pb, pc, pd) = (sample_pos(i, j), sample_pos(i + 1, j), sample_pos(i + 1, j + 1), sample_pos(i, j + 1));
            let top = || lerp(pa, pb, a, b);
            let right = || lerp(pb, pc, b, c);
            let bottom = || lerp(pd, pc, d, c);
            let left = || lerp(pa, pd, a, d);
            let mut add = |p: [f32; 2], q: [f32; 2]| pieces.push((p, q));
            match case {
                1 | 14 => add(left(), top()),
                2 | 13 => add(top(), right()),
                3 | 12 => add(left(), right()),
                4 | 11 => add(right(), bottom()),
                6 | 9 => add(top(), bottom()),
                7 | 8 => add(left(), bottom()),
                5 => {
                    add(left(), top());
                    add(right(), bottom());
                }
                10 => {
                    add(top(), right());
                    add(left(), bottom());
                }
                _ => {}
            }
        }
    }
    pieces
}

fn contour_lines(pieces: Vec<Segment>) -> Vec<Segment> {
    let mut out = Vec::new();
    for line in join(pieces) {
        let line = round_off(&sharpen(&straighten(&simplify(&line, 0.02))));
        let n = line.len();
        let closed = n > 2 && near(line[0], line[n - 1]);
        let pts = if closed { &line[..n - 1] } else { &line[..] };
        let count = if closed { pts.len() } else { pts.len().saturating_sub(1) };
        for k in 0..count {
            out.push((pts[k], pts[(k + 1) % pts.len()]));
        }
    }
    out
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
}

fn key(p: [f32; 2]) -> (i64, i64) {
    ((p[0] * 1000.0).round() as i64, (p[1] * 1000.0).round() as i64)
}

/// Куски в ломаные. Направление кусков у квадратов сетки не согласовано, поэтому
/// продолжение ищется по любому концу куска.
pub fn join(pieces: Vec<Segment>) -> Vec<Vec<[f32; 2]>> {
    let mut at: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (n, (a, b)) in pieces.iter().enumerate() {
        at.entry(key(*a)).or_default().push(n);
        at.entry(key(*b)).or_default().push(n);
    }
    let mut used = vec![false; pieces.len()];
    // Неиспользованный кусок с концом в точке `p` и его другой конец.
    let next = |p: [f32; 2], used: &[bool]| -> Option<(usize, [f32; 2])> {
        let n = *at.get(&key(p))?.iter().find(|&&n| !used[n])?;
        let (a, b) = pieces[n];
        Some((n, if key(a) == key(p) { b } else { a }))
    };
    let mut lines = Vec::new();
    for s in 0..pieces.len() {
        if used[s] {
            continue;
        }
        used[s] = true;
        let mut line = vec![pieces[s].0, pieces[s].1];
        while let Some((n, q)) = next(*line.last().unwrap(), &used) {
            used[n] = true;
            line.push(q);
        }
        // Не замкнулась — дотянуть и в другую сторону.
        if !near(line[0], *line.last().unwrap()) {
            while let Some((n, q)) = next(line[0], &used) {
                used[n] = true;
                line.insert(0, q);
            }
        }
        lines.push(line);
    }
    lines
}

/// Дуглас — Пекер: точки, отходящие от прямой меньше чем на `eps`, выбрасываются.
fn simplify(line: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let closed = near(line[0], line[line.len() - 1]);
    if closed {
        // Замкнутую режем в самой дальней от начала точке, чтобы концы не съелись.
        let far = (1..line.len() - 1)
            .max_by(|&x, &y| {
                let d = |k: usize| (line[k][0] - line[0][0]).powi(2) + (line[k][1] - line[0][1]).powi(2);
                d(x).total_cmp(&d(y))
            })
            .unwrap_or(1);
        let mut a = simplify(&line[..=far], eps);
        let b = simplify(&line[far..], eps);
        a.pop();
        a.extend(b);
        return a;
    }
    let (first, last) = (line[0], line[line.len() - 1]);
    let (dx, dy) = (last[0] - first[0], last[1] - first[1]);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (k, dmax) = (1..line.len() - 1).map(|k| (k, ((line[k][0] - first[0]) * dy - (line[k][1] - first[1]) * dx).abs() / len)).fold((0, 0.0), |acc, x| if x.1 > acc.1 { x } else { acc });
    if dmax > eps {
        let mut a = simplify(&line[..=k], eps);
        a.pop();
        a.extend(simplify(&line[k..], eps));
        a
    } else {
        vec![first, last]
    }
}

/// Почти ровный участок вдоль сетки — ровный: у квадратной кисти соседние мазки
/// ложатся мягким краем внахлёст, и край гуляет на сотые доли клетки. Участок не
/// короче клетки, разброс не больше `SPREAD`: у круга радиусом меньше пяти клеток
/// такого плоского места нет.
fn straighten(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    const SPREAD: f32 = 0.03;
    const SPAN: f32 = 1.0;
    let closed = line.len() > 3 && near(line[0], line[line.len() - 1]);
    let mut pts: Vec<[f32; 2]> = if closed { line[..line.len() - 1].to_vec() } else { line.to_vec() };
    let n = pts.len();
    if n < 3 {
        return line.to_vec();
    }
    if closed {
        // Начать после косого отрезка: он не входит ни в один ровный участок,
        // и участок не рвётся на стыке начала и конца.
        let slanted = |k: usize| {
            let (a, b) = (pts[(k + n - 1) % n], pts[k]);
            (a[0] - b[0]).abs() > SPREAD && (a[1] - b[1]).abs() > SPREAD
        };
        let start = (0..n).find(|&k| slanted(k)).unwrap_or(0);
        pts.rotate_left(start);
    }
    for axis in 0..2 {
        let other = 1 - axis;
        let mut i = 0;
        while i + 1 < n {
            let (mut lo, mut hi) = (pts[i][axis], pts[i][axis]);
            let mut j = i;
            while j + 1 < n {
                let v = pts[j + 1][axis];
                if v.max(hi) - v.min(lo) > SPREAD {
                    break;
                }
                lo = lo.min(v);
                hi = hi.max(v);
                j += 1;
            }
            if j > i && (pts[j][other] - pts[i][other]).abs() >= SPAN {
                let mean = (i..=j).map(|k| pts[k][axis]).sum::<f32>() / (j - i + 1) as f32;
                let mean = (mean * 1e4).round() / 1e4;
                for p in &mut pts[i..=j] {
                    p[axis] = mean;
                }
                i = j;
            } else {
                i += 1;
            }
        }
    }
    if closed {
        pts.push(pts[0]);
    }
    pts
}

/// Короткий скос между отрезками вдоль сетки под прямым углом — это срезанный угол
/// прямоугольника: вместо скоса ставим точку пересечения. У клеток скос — один отрезок,
/// у квадратной кисти с мягким краем — дуга из нескольких.
fn sharpen(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let closed = line.len() > 3 && near(line[0], line[line.len() - 1]);
    let mut pts: Vec<[f32; 2]> = if closed { line[..line.len() - 1].to_vec() } else { line.to_vec() };
    // Срез угла у клеток — диагональ в полшага сетки: 0,125·√2 ≈ 0,18 клетки.
    let limit = 0.8 / RES as f32;
    // Скругление угла квадратной кисти — до полуклетки по дуге.
    let arc_limit = 0.6;
    let along = |d: [f32; 2]| d[0].abs() < 1e-5 || d[1].abs() < 1e-5;
    let dist = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let mut k = 0;
    'outer: while pts.len() >= 4 && k < pts.len() {
        let n = pts.len();
        for span in 1..=6usize {
            if span + 2 >= n {
                break;
            }
            if !closed && (k == 0 || k + span + 1 >= n) {
                break;
            }
            let (p0, p1) = (pts[(k + n - 1) % n], pts[k]);
            let (p2, p3) = (pts[(k + span) % n], pts[(k + span + 1) % n]);
            let chain: f32 = (0..span).map(|s| dist(pts[(k + s) % n], pts[(k + s + 1) % n])).sum();
            if chain >= arc_limit {
                break;
            }
            if span == 1 && chain >= limit {
                continue;
            }
            let d1 = [p1[0] - p0[0], p1[1] - p0[1]];
            let d2 = [p3[0] - p2[0], p3[1] - p2[1]];
            let (l1, l2) = ((d1[0] * d1[0] + d1[1] * d1[1]).sqrt(), (d2[0] * d2[0] + d2[1] * d2[1]).sqrt());
            // Только вдоль сетки: у круглой кисти перпендикулярные отрезки — не угол комнаты.
            let square = l1 > 1e-6 && l2 > 1e-6 && along(d1) && along(d2) && ((d1[0] * d2[0] + d1[1] * d2[1]) / (l1 * l2)).abs() < 0.05;
            if !square {
                continue;
            }
            // Пересечение прямых p0-p1 и p2-p3.
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            let t = ((p2[0] - p0[0]) * d2[1] - (p2[1] - p0[1]) * d2[0]) / den;
            let x = [p0[0] + d1[0] * t, p0[1] + d1[1] * t];
            let x = [(x[0] * 1e4).round() / 1e4, (x[1] * 1e4).round() / 1e4];
            pts[k] = x;
            for _ in 0..span {
                let at = (k + 1) % pts.len();
                pts.remove(at);
                if at < k {
                    k -= 1;
                }
            }
            continue 'outer;
        }
        k += 1;
    }
    // После острых углов соседние отрезки могут лечь на одну прямую.
    let mut merged: Vec<[f32; 2]> = Vec::new();
    let n = pts.len();
    for k in 0..n {
        if closed || (k > 0 && k + 1 < n) {
            let (a, b, c) = (pts[(k + n - 1) % n], pts[k], pts[(k + 1) % n]);
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            if cross.abs() < 1e-5 {
                continue;
            }
        }
        merged.push(pts[k]);
    }
    if closed && !merged.is_empty() {
        merged.push(merged[0]);
    }
    merged
}

/// Плавные изгибы. Ломаная дробится на точки через `STEP` и сглаживается по Таубину
/// (шаг к середине соседей, потом чуть назад — так кривая не съёживается). Углы
/// между отрезками вдоль сетки и концы открытой ломаной стоят на месте, точки на прямой
/// остаются на ней — комнаты кистью «Клетки» не меняются.
fn round_off(line: &[[f32; 2]]) -> Vec<[f32; 2]> {
    const STEP: f32 = 0.125;
    const PASSES: usize = 12;
    let n = line.len();
    if n < 3 {
        return line.to_vec();
    }
    let closed = near(line[0], line[n - 1]);
    let pts = if closed { &line[..n - 1] } else { line };
    let m = pts.len();
    let along = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 || (a[1] - b[1]).abs() < 1e-5;
    let hard = |k: usize| {
        if !closed && (k == 0 || k + 1 == m) {
            return true;
        }
        let (a, b, c) = (pts[(k + m - 1) % m], pts[k], pts[(k + 1) % m]);
        along(a, b) && along(b, c)
    };
    let mut dense: Vec<[f32; 2]> = Vec::new();
    let mut fixed: Vec<bool> = Vec::new();
    let edges = if closed { m } else { m - 1 };
    for k in 0..edges {
        let (a, b) = (pts[k], pts[(k + 1) % m]);
        dense.push(a);
        fixed.push(hard(k));
        let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let parts = (len / STEP).ceil().max(1.0) as usize;
        for p in 1..parts {
            let t = p as f32 / parts as f32;
            dense.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            fixed.push(false);
        }
    }
    if !closed {
        dense.push(pts[m - 1]);
        fixed.push(true);
    }
    let len = dense.len();
    for pass in 0..PASSES * 2 {
        let f = if pass % 2 == 0 { 0.5 } else { -0.53 };
        let prev = dense.clone();
        for k in 0..len {
            if fixed[k] {
                continue;
            }
            let (a, c) = (prev[(k + len - 1) % len], prev[(k + 1) % len]);
            dense[k][0] += f * ((a[0] + c[0]) / 2.0 - prev[k][0]);
            dense[k][1] += f * ((a[1] + c[1]) / 2.0 - prev[k][1]);
        }
    }
    if closed {
        dense.push(dense[0]);
    }
    simplify(&dense, 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(m: &mut Mask, weights: &[(usize, f32)]) {
        for &(i, v) in weights {
            let x = m.at(i) as f32;
            m.put(i, (x + v * (255.0 - x)).round() as u8);
        }
    }

    fn fill_cells(m: &mut Mask, a: (i32, i32), b: (i32, i32)) {
        let w = cells(m.w, m.h, a, b);
        fill(m, &w);
    }

    #[test]
    fn oval_fills_inscribed_circle() {
        let mut m = Mask::for_cells(12, 12);
        let w = oval(m.w, m.h, (1, 1), (10, 10));
        fill(&mut m, &w);
        // Круг радиусом 5 клеток: площадь ≈ 25π, середина залита, углы рамки пусты.
        assert!((m.area() - 25.0 * std::f32::consts::PI).abs() < 0.5, "{}", m.area());
        let r = RES as isize;
        assert_eq!(m.get(6 * r, 6 * r), 255);
        assert_eq!(m.get(r + 1, r + 1), 0);
        // Контур — одна замкнутая линия.
        assert_eq!(join(contour(&m)).len(), 1);
    }

    fn fill_dab(m: &mut Mask, brush: &Brush, c: [f32; 2], seed: u32) {
        let w = dab(m.w, m.h, brush, c, seed);
        fill(m, &w);
    }

    fn length(s: &[Segment]) -> f32 {
        s.iter().map(|(a, b)| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()).sum()
    }

    #[test]
    fn cell_rectangle_has_four_exact_walls() {
        let mut m = Mask::for_cells(20, 12);
        fill_cells(&mut m, (2, 2), (7, 6));
        assert_eq!(m.area(), 30.0);
        let c = contour(&m);
        assert_eq!(c.len(), 4, "{c:?}");
        let mut xs: Vec<f32> = c.iter().flat_map(|(a, b)| [a[0], b[0]]).collect();
        xs.sort_by(f32::total_cmp);
        assert_eq!((xs[0], xs[xs.len() - 1]), (2.0, 8.0));
        assert!((length(&c) - 22.0).abs() < 1e-3);
    }

    #[test]
    fn l_shape_and_map_edge() {
        let mut m = Mask::for_cells(20, 12);
        fill_cells(&mut m, (2, 2), (7, 6));
        fill_cells(&mut m, (5, 5), (10, 8));
        let c = contour(&m);
        assert!((length(&c) - 32.0).abs() < 1e-3, "{}", length(&c));
        assert!(c.iter().all(|(a, b)| a[0] == b[0] || a[1] == b[1]), "все стены прямые: {c:?}");
        fill_cells(&mut m, (0, 0), (0, 0));
        assert!(contour(&m).iter().any(|(a, b)| (a[1] == 0.0 && b[1] == 0.0) && (a[0] - b[0]).abs() == 1.0), "у края карты стена по краю");
    }

    #[test]
    fn circle_has_round_wall() {
        let mut m = Mask::for_cells(20, 20);
        fill_dab(&mut m, &Brush { shape: Shape::Circle, radius: 4.0, hardness: 1.0, flow: 1.0 }, [10.0, 10.0], 0);
        let c = contour(&m);
        let l = length(&c);
        assert!((l - std::f32::consts::TAU * 4.0).abs() < 1.0, "длина {l}");
        assert!(c.len() >= 12 && c.len() < 200, "отрезков {}", c.len());
        assert!((m.area() - std::f32::consts::PI * 16.0).abs() < 2.0);
    }

    #[test]
    fn soft_square_stroke_keeps_sharp_corners() {
        // Как в мазке: точка берёт самую большую силу из отпечатков.
        let mut m = Mask::for_cells(30, 20);
        let b = Brush { shape: Shape::Square, radius: 1.5, hardness: 0.8, flow: 1.0 };
        let mut most = vec![0f32; m.len()];
        for x in [22.0, 22.45, 22.9, 23.35, 23.8, 24.0] {
            for (i, v) in dab(m.w, m.h, &b, [x, 14.0], 0) {
                most[i] = most[i].max(v);
            }
        }
        for (k, v) in most.into_iter().enumerate() {
            m.put(k, (v * 255.0).round() as u8);
        }
        let c = contour(&m);
        assert_eq!(c.len(), 4, "{c:?}");
        assert!(c.iter().all(|(a, b)| a[0] == b[0] || a[1] == b[1]), "{c:?}");
    }

    #[test]
    fn freehand_corners_are_rounded() {
        // Два круглых мазка крестом: во внутренних углах раньше торчали шипы.
        let mut m = Mask::for_cells(20, 20);
        let b = Brush { shape: Shape::Circle, radius: 1.5, hardness: 0.8, flow: 1.0 };
        for k in 0..=20 {
            let t = 3.0 + k as f32 * 0.7;
            fill_dab(&mut m, &b, [t, 10.0], 0);
            fill_dab(&mut m, &b, [10.0, t], 0);
        }
        for line in join(contour(&m)) {
            for k in 1..line.len() - 1 {
                let (a, b, c) = (line[k - 1], line[k], line[k + 1]);
                let (d1, d2) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
                let cos = (d1[0] * d2[0] + d1[1] * d2[1]) / (d1[0].hypot(d1[1]) * d2[0].hypot(d2[1]));
                assert!(cos > 0.6, "излом {:.0}° в {b:?}", cos.acos().to_degrees());
            }
        }
    }

    #[test]
    fn soften_fades_to_the_edge_and_keeps_the_middle() {
        let (w, h) = (40, 40);
        let mut a = cells(w, h, (2, 2), (7, 7));
        soften(w, h, &mut a, 1.0);
        let at = |x: usize, y: usize| a.iter().find(|(i, _)| *i == y * w + x).map_or(0.0, |p| p.1);
        assert!(at(19, 19) > 0.95, "середина полная: {}", at(19, 19));
        assert!(at(8, 19) < 0.05, "у края почти ноль: {}", at(8, 19));
        assert!(at(12, 19) > at(9, 19) && at(16, 19) > at(12, 19), "спад ровный");
        let before = cells(w, h, (2, 2), (7, 7));
        let mut same = before.clone();
        soften(w, h, &mut same, 0.0);
        assert_eq!(same, before);
    }

    #[test]
    fn soft_brush_fades_and_spray_is_repeatable() {
        let (w, h) = (80, 80);
        let soft = Brush { shape: Shape::Circle, radius: 3.0, hardness: 0.0, flow: 1.0 };
        let d = dab(w, h, &soft, [10.0, 10.0], 0);
        let at = |x: f32, y: f32| {
            let i = (y * RES as f32 - 0.5).round() as usize * w + (x * RES as f32 - 0.5).round() as usize;
            d.iter().find(|(k, _)| *k == i).map_or(0.0, |p| p.1)
        };
        assert!(at(10.0, 10.0) > 0.95 && at(11.5, 10.0) < 0.7 && at(11.5, 10.0) > 0.2);
        let spray = Brush { shape: Shape::Spray, ..soft };
        let mut a = dab(w, h, &spray, [10.0, 10.0], 7);
        let mut b = dab(w, h, &spray, [10.0, 10.0], 7);
        a.sort_by_key(|p| p.0);
        b.sort_by_key(|p| p.0);
        assert_eq!(a, b);
        assert!(a.len() < d.len(), "спрей рыхлее круга");
    }

    #[test]
    fn mask_round_trip_is_small() {
        let mut m = Mask::for_cells(100, 100);
        fill_cells(&mut m, (10, 10), (40, 30));
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.len() < 5000, "{} байт", json.len());
        assert_eq!(serde_json::from_str::<Mask>(&json).unwrap(), m);
    }

    /// Псевдослучайная маска: пустые, залитые и пёстрые куски, края карты не кратны куску.
    fn speckled(w: usize, h: usize) -> (Mask, Vec<u8>) {
        let mut dense = vec![0u8; w * h];
        let mut s = 11u64;
        for j in 0..h {
            for i in 0..w {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                dense[j * w + i] = if i < CHUNK && j < CHUNK {
                    0
                } else if i >= CHUNK && i < 2 * CHUNK && j < CHUNK {
                    255
                } else if (i / 7 + j / 5) % 3 == 0 {
                    (s >> 56) as u8
                } else {
                    0
                };
            }
        }
        (Mask::from_vec(w, h, &dense), dense)
    }

    #[test]
    fn chunks_hold_the_same_points_as_a_flat_array() {
        let (w, h) = (3 * CHUNK + 37, 2 * CHUNK + 5);
        let (m, dense) = speckled(w, h);
        assert_eq!(m.to_vec(), dense);
        assert_eq!(m.at(w + 3), dense[w + 3]);
        assert_eq!(m.get(-1, 0), 0);
        assert_eq!(m.get(w as isize, 0), 0);
        let n = dense.iter().filter(|&&v| v as f32 > LEVEL).count() as f32 / (RES * RES) as f32;
        assert_eq!(m.area(), n);
        // Пустой и залитый куски памяти под точки не держат.
        assert_eq!(m.uniform_at(0, 0), Some(0));
        assert_eq!(m.uniform_at(CHUNK + 1, 3), Some(255));
        assert!(m.data_chunks() < m.chunks.len());
    }

    #[test]
    fn both_file_formats_read_back_the_same_mask() {
        let (w, h) = (2 * CHUNK + 9, CHUNK + 3);
        let (m, dense) = speckled(w, h);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"t\"") && !json.contains("\"z\""), "пишется новый формат");
        assert_eq!(serde_json::from_str::<Mask>(&json).unwrap(), m);
        // Формат 2: все точки подряд, сжатые.
        use std::io::Write;
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&dense).unwrap();
        let z = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, enc.finish().unwrap());
        let old: Mask = serde_json::from_str(&format!("{{\"w\":{w},\"h\":{h},\"z\":\"{z}\"}}")).unwrap();
        assert_eq!(old, m);
        assert_eq!(old.to_vec(), dense);
        assert!(serde_json::from_str::<Mask>(&format!("{{\"w\":{w},\"h\":{},\"z\":\"{z}\"}}", h + 1)).is_err(), "размер не сходится");
        // Пустая большая маска в файле — пара сотен байт.
        let big = serde_json::to_string(&Mask::for_cells(1000, 1000)).unwrap();
        assert!(big.len() < 400, "{} байт", big.len());
    }

    /// Контур прежним обходом: все квадраты подряд, без пропуска кусков.
    fn contour_raw_full(m: &Mask) -> Vec<Segment> {
        let v = |i: isize, j: isize| m.get(i, j) as f32;
        let lerp = |a: [f32; 2], b: [f32; 2], va: f32, vb: f32| {
            let t = if (vb - va).abs() < 1e-6 { 0.5 } else { ((LEVEL - va) / (vb - va)).clamp(0.0, 1.0) };
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        };
        let mut pieces = Vec::new();
        for j in -1..m.h as isize {
            for i in -1..m.w as isize {
                let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1));
                let case = (a > LEVEL) as u8 | ((b > LEVEL) as u8) << 1 | ((c > LEVEL) as u8) << 2 | ((d > LEVEL) as u8) << 3;
                let (pa, pb, pc, pd) = (sample_pos(i, j), sample_pos(i + 1, j), sample_pos(i + 1, j + 1), sample_pos(i, j + 1));
                let (top, right, bottom, left) = (lerp(pa, pb, a, b), lerp(pb, pc, b, c), lerp(pd, pc, d, c), lerp(pa, pd, a, d));
                match case {
                    1 | 14 => pieces.push((left, top)),
                    2 | 13 => pieces.push((top, right)),
                    3 | 12 => pieces.push((left, right)),
                    4 | 11 => pieces.push((right, bottom)),
                    6 | 9 => pieces.push((top, bottom)),
                    7 | 8 => pieces.push((left, bottom)),
                    5 => pieces.extend([(left, top), (right, bottom)]),
                    10 => pieces.extend([(top, right), (left, bottom)]),
                    _ => {}
                }
            }
        }
        pieces
    }

    #[test]
    fn contour_skips_flat_chunks_but_finds_the_same_walls_in_the_same_order() {
        // Комнаты по всей карте, у краёв кусков и карты, круглые и клетками; залитый кусок.
        let mut m = Mask::for_cells(100, 70);
        for (a, b) in [((0, 0), (3, 2)), ((30, 30), (33, 34)), ((31, 0), (40, 5)), ((60, 40), (99, 69)), ((10, 50), (10, 50))] {
            fill_cells(&mut m, a, b);
        }
        fill_dab(&mut m, &Brush { shape: Shape::Circle, radius: 5.0, hardness: 0.5, flow: 1.0 }, [64.0, 20.0], 0);
        m.compact();
        // Кусок клеток 64–95 × 64–69 (у нижнего края карты) — весь в комнате.
        assert!(m.uniform_at(70 * RES, 66 * RES) == Some(255), "залитый кусок без точек");
        assert_eq!(contour_raw(&m), contour_raw_full(&m));
    }

    #[test]
    fn a_copy_shares_chunks_and_an_edit_copies_only_its_own() {
        let (mut m, _) = speckled(3 * CHUNK, 3 * CHUNK);
        let p = (2 * CHUNK as isize + 1, 2 * CHUNK as isize + 1);
        // Точка в пёстром куске, со своей силой, не 7.
        let orig = m.get(p.0, p.1);
        assert!(m.uniform_at(p.0 as usize, p.1 as usize).is_none() && orig != 7);
        let before = m.clone();
        assert_eq!(before.stamp(), m.stamp(), "копия — тот же отпечаток");
        let shared = |a: &Mask, b: &Mask| (0..a.chunks.len()).filter(|&c| matches!((&a.chunks[c], &b.chunks[c]), (Chunk::Data(x), Chunk::Data(y)) if Arc::ptr_eq(x, y))).count();
        let all = shared(&m, &before);
        m.set(2 * CHUNK + 1, 2 * CHUNK + 1, 7);
        assert_eq!(shared(&m, &before), all - 1, "скопирован только тронутый кусок");
        assert_ne!(before.stamp(), m.stamp(), "после правки — новый отпечаток");
        assert_ne!(m, before);
        assert_eq!((before.get(p.0, p.1), m.get(p.0, p.1)), (orig, 7), "копия не изменилась");
        // Тот же байт ещё раз — не правка.
        let s = m.stamp();
        m.set(2 * CHUNK + 1, 2 * CHUNK + 1, 7);
        assert_eq!(m.stamp(), s);
        // Залитое снова целиком — кусок без памяти.
        let mut f = Mask::new(CHUNK, CHUNK);
        f.set(3, 3, 9);
        assert_eq!(f.data_chunks(), 1);
        f.set(3, 3, 0);
        f.compact();
        assert_eq!(f.data_chunks(), 0);
        assert!(f.is_empty());
    }
}
