//! Кэш снимков выгрузки. Картинка этажа снимается окном плитками, и снимки — самая долгая
//! часть выгрузки. Здесь картинка делится на куски [`CELL`]×[`CELL`] точек; у каждого куска
//! отпечаток всего, что может в нём нарисоваться: куски масок под ним и вокруг (земля
//! сглаживается по соседям), предметы, стены, проёмы, дороги, крыши и помосты рядом (с
//! полями на тени), всё, что у этажа одно на всю карту, версия программы и картинок.
//! Снятый кусок ложится на диск под своим отпечатком. Повторная выгрузка берёт с диска
//! куски с тем же отпечатком и снимает окном только те, где что-то поменялось.
//!
//! Ошибиться отпечаток может только в одну сторону — снять лишнее: всё, у чего нет места
//! на карте, входит в общую часть, и правка в нём меняет все куски.

use crate::grid::{self, Grid};
use crate::map::Map;
use crate::mask::{self, Mask};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Сторона куска в точках картинки: при 100 точках на клетку — 2,56 клетки.
pub const CELL: u32 = 256;

/// Сколько держит кэш на диске: больше — выбрасываются давно не нужные куски.
pub const DISK_LIMIT: u64 = 3 << 30;

/// Два независимых хэша одних данных — отпечаток в 128 бит. SipHash с нулевыми ключами
/// одинаков от запуска к запуску; смена его в новом Rust меняет и версию программы.
pub struct Twin(std::collections::hash_map::DefaultHasher, std::collections::hash_map::DefaultHasher);

impl Default for Twin {
    fn default() -> Self {
        Self::new()
    }
}

impl Twin {
    pub fn new() -> Self {
        let (mut a, mut b) = (std::collections::hash_map::DefaultHasher::new(), std::collections::hash_map::DefaultHasher::new());
        0x5a17u16.hash(&mut a);
        0xc0deu16.hash(&mut b);
        Self(a, b)
    }

    pub fn finish128(&self) -> u128 {
        (self.0.finish() as u128) << 64 | self.1.finish() as u128
    }
}

impl Hasher for Twin {
    fn finish(&self) -> u64 {
        self.0.finish()
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
        self.1.write(bytes);
    }
}

fn h64<T: Hash + ?Sized>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

fn json_hash<T: serde::Serialize>(v: &T) -> u64 {
    h64(&serde_json::to_vec(v).unwrap_or_default())
}

/// Версия программы для отпечатков: размер и время сборки exe. Новая сборка может рисовать
/// иначе, и прежние куски ей не годятся.
pub fn program_stamp() -> u64 {
    static STAMP: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *STAMP.get_or_init(|| {
        let meta = std::env::current_exe().ok().and_then(|p| std::fs::metadata(p).ok());
        let built = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos());
        h64(&(env!("CARGO_PKG_VERSION"), meta.map(|m| m.len()), built))
    })
}

/// Версия картинок `keys`: пак, место и размер в нём или размер и время файла. Встроенные
/// картинки меняются только с программой.
pub fn assets_stamp<'a>(keys: impl Iterator<Item = &'a str>, sources: &crate::library::Sources) -> u64 {
    use crate::library;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut files: HashMap<PathBuf, Option<(u64, u128)>> = HashMap::new();
    let mut meta = |p: &Path| -> Option<(u64, u128)> {
        *files.entry(p.to_path_buf()).or_insert_with(|| {
            let m = std::fs::metadata(p).ok()?;
            Some((m.len(), m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos()))
        })
    };
    let src = sources.read().ok();
    for key in keys {
        key.hash(&mut h);
        let inner = library::untint(key.strip_prefix(library::VARY).unwrap_or(key)).1;
        if inner.starts_with(library::BUILTIN) || inner.starts_with(library::SET) {
            continue;
        }
        if inner.starts_with("pack:") {
            if let Some((file, offset, size)) = src.as_ref().and_then(|s| s.get(inner).cloned()) {
                (offset, size, meta(&file)).hash(&mut h);
            }
        } else {
            meta(Path::new(inner)).hash(&mut h);
        }
    }
    h.finish()
}

/// Отпечатки плиток маски ([`Mask::chunk_tiles`]) — сеткой по всей карте.
struct MaskPrint {
    tag: u64,
    nx: usize,
    ny: usize,
    tiles: Vec<u64>,
    /// На сколько клеток вокруг куска картинки маска сказывается в нём: земля — на шаг
    /// кривой, комнаты — на тень стен по их краю.
    reach: f32,
    /// Слой земли или пола: где он пуст, его как бы и нет — новый слой в другом конце
    /// карты не трогает куски здесь.
    coat: bool,
}

/// Отпечатки этажа: общее для всех кусков картинки и разложенное по месту.
pub struct Prints {
    global: u64,
    /// Поля вокруг куска, клетки: тень, размытие, ширина стены.
    margin: f32,
    masks: Vec<MaskPrint>,
    items: Vec<(grid::Rect, u64)>,
    grid: Grid,
    /// Отпечаток пустого куска маски.
    zero: u64,
}

impl Prints {
    /// `memo` — отпечатки плиток кусков масок по номеру куска в памяти
    /// ([`Mask::chunk_gen`]): повторной выгрузке не нужно перечитывать нетронутые куски.
    pub fn new(map: &Map, memo: &mut HashMap<u64, std::sync::Arc<Vec<u64>>>) -> Self {
        if memo.len() > 1 << 16 {
            memo.clear();
        }
        // Общее: карта без всего, что лежит по месту. Слои земли и пола — по месту, с
        // материалом: новый слой красит только там, где лёг.
        let mut bare = map.clone();
        let empty = Mask::for_cells(1, 1);
        bare.terrain.clear();
        bare.room_floor.clear();
        bare.rooms = empty.clone();
        if let Some(s) = &mut bare.shade {
            *s = empty.clone();
        }
        let relief = bare.relief.take();
        bare.walls.clear();
        bare.portals.clear();
        bare.props.clear();
        bare.paths.clear();
        bare.roofs.clear();
        bare.outline_roofs.clear();
        bare.platforms.clear();
        bare.edge_looks.clear();
        bare.levels.clear();
        let mut g = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_vec(&bare).unwrap_or_default().hash(&mut g);
        if let Some(r) = &relief {
            (r.w, r.h, &r.face, &r.ridge, &r.under, r.natural).hash(&mut g);
            for v in &r.data {
                v.to_bits().hash(&mut g);
            }
        }
        let global = g.finish();

        let sh = &map.shadows;
        let margin = (4.0 + 3.0 * sh.dist.abs() + 3.0 * sh.blur.abs()).min(32.0);
        let mut masks = Vec::new();
        let mut add = |tag: u64, m: &Mask, coat: bool, reach: f32| {
            let (cx, cy) = m.chunks_wh();
            const N: usize = mask::CHUNK / mask::PRINT_TILE;
            let (nx, ny) = (cx * N, cy * N);
            let mut tiles = vec![0u64; nx * ny];
            for c in 0..cx * cy {
                let part = memo.entry(m.chunk_gen(c)).or_insert_with(|| std::sync::Arc::new(m.chunk_tiles(c))).clone();
                let (ox, oy) = ((c % cx) * N, (c / cx) * N);
                for (t, v) in part.iter().enumerate() {
                    tiles[(oy + t / N) * nx + ox + t % N] = *v;
                }
            }
            masks.push(MaskPrint { tag, nx, ny, tiles, reach, coat });
        };
        // Слой — по материалу, а не по номеру: удалённый слой снизу не сдвигает верхние.
        for c in &map.terrain {
            add(1 << 32 | c.material as u64, &c.mask, true, 2.0);
        }
        for c in &map.room_floor {
            add(2 << 32 | c.material as u64, &c.mask, true, 2.0);
        }
        add(3 << 32, &map.rooms, false, margin + 2.0);
        if let Some(s) = &map.shade {
            add(4 << 32, s, false, 2.0);
        }
        let zero = Mask::empty_tile();

        let mut items: Vec<(grid::Rect, u64)> = Vec::new();
        for p in &map.props {
            let mut b = grid::bounds(&p.corners());
            // Тень кроны отходит от дерева на его поперечник ([`crate::roofs::tree_shade`]).
            if p.is_canopy() && p.casts_shadow() {
                let d = p.size[0].max(p.size[1]) * p.scale;
                b = grid::grow(b, d * crate::roofs::TREE_HEIGHT / crate::roofs::SUN_RISE * 1.1 + 1.0);
            }
            items.push((b, json_hash(p)));
        }
        for w in &map.walls {
            let bend = w.bends.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            items.push((grid::grow(grid::bounds(&w.points), 1.0 + bend), json_hash(w)));
        }
        for p in &map.portals {
            items.push((grid::grow(grid::bounds(&[p.a, p.b]), 1.0), json_hash(p)));
        }
        for (&(x, y), &v) in &map.edge_looks {
            let c = [x as f32 / 64.0, y as f32 / 64.0];
            items.push((grid::grow([c[0], c[1], c[0], c[1]], 1.0), h64(&(0xed9e_u32, x, y, v))));
        }
        for p in &map.paths {
            items.push((grid::grow(grid::bounds(&p.points), 1.0 + p.width.abs()), json_hash(p)));
        }
        for r in &map.roofs {
            items.push((grid::grow(grid::bounds(&r.outline()), 1.0 + r.overhang.abs()), json_hash(r)));
        }
        for r in &map.outline_roofs {
            let (lo, hi) = r.bounds();
            items.push((grid::grow([lo[0], lo[1], hi[0], hi[1]], 1.0), json_hash(r)));
        }
        for p in &map.platforms {
            let (lo, hi) = p.bounds();
            items.push((grid::grow([lo[0], lo[1], hi[0], hi[1]], 3.0), json_hash(p)));
        }
        let grid = Grid::new(&items.iter().map(|i| i.0).collect::<Vec<_>>(), 8.0);
        Self { global, margin, masks, items, grid, zero }
    }

    /// Отпечаток куска картинки, что показывает прямоугольник карты `r` (клетки). `salt` —
    /// всё, что нарисовано не из карты: проход снимка, точки на клетку, версия программы
    /// и картинок.
    pub fn cell(&self, r: grid::Rect, salt: u64) -> u128 {
        let mut h = Twin::new();
        (self.global, salt).hash(&mut h);
        r.map(f32::to_bits).hash(&mut h);
        // Плитки масок под прямоугольником и вокруг, насколько маска сказывается.
        let at = |v: f32| ((v * mask::RES as f32 / mask::PRINT_TILE as f32).floor().max(0.0)) as usize;
        for m in &self.masks {
            if m.nx == 0 || m.ny == 0 {
                continue;
            }
            let (x0, x1) = (at(r[0] - m.reach).min(m.nx - 1), at(r[2] + m.reach).min(m.nx - 1));
            let (y0, y1) = (at(r[1] - m.reach).min(m.ny - 1), at(r[3] + m.reach).min(m.ny - 1));
            let near = (y0..=y1).flat_map(|cy| (x0..=x1).map(move |cx| m.tiles[cy * m.nx + cx]));
            if m.coat && near.clone().all(|c| c == self.zero) {
                continue;
            }
            m.tag.hash(&mut h);
            for c in near {
                c.hash(&mut h);
            }
        }
        // Что лежит рядом — по порядку на карте: порядок решает, кто кого накрывает.
        let q = grid::grow(r, self.margin);
        for i in self.grid.near(q) {
            let (b, v) = &self.items[i as usize];
            if grid::touch(b, &q) || b.iter().any(|x| !x.is_finite()) {
                (i, *v).hash(&mut h);
            }
        }
        h.finish128()
    }
}

/// Левые верхние точки плиток `tile`, которыми снять недостающие куски (`missing` по
/// строкам, `n` — кусков по сторонам, `size` — размер картинки): плитка встаёт у первого
/// ещё не накрытого куска, у края картинки — отступив внутрь, чтобы накрыть больше.
pub fn cover(missing: &[bool], n: [u32; 2], size: [u32; 2], tile: [u32; 2]) -> Vec<[u32; 2]> {
    let mut todo = missing.to_vec();
    let mut out = Vec::new();
    for c in 0..todo.len() {
        if !todo[c] {
            continue;
        }
        let (i, j) = (c as u32 % n[0], c as u32 / n[0]);
        let x = (i * CELL).min(size[0].saturating_sub(tile[0]));
        let y = (j * CELL).min(size[1].saturating_sub(tile[1]));
        out.push([x, y]);
        let (x1, y1) = ((x + tile[0]).min(size[0]), (y + tile[1]).min(size[1]));
        for jj in y.div_ceil(CELL)..n[1] {
            for ii in x.div_ceil(CELL)..n[0] {
                let (cx1, cy1) = (((ii + 1) * CELL).min(size[0]), ((jj + 1) * CELL).min(size[1]));
                if cx1 <= x1 && cy1 <= y1 {
                    todo[(jj * n[0] + ii) as usize] = false;
                }
            }
        }
    }
    out
}

/// Задевает ли плитка с началом `o` хоть один недостающий кусок.
pub fn touches_missing(missing: &[bool], n: [u32; 2], size: [u32; 2], o: [u32; 2], tile: [u32; 2]) -> bool {
    let (x1, y1) = ((o[0] + tile[0]).min(size[0]), (o[1] + tile[1]).min(size[1]));
    (o[1] / CELL..y1.div_ceil(CELL).min(n[1])).any(|j| (o[0] / CELL..x1.div_ceil(CELL).min(n[0])).any(|i| missing[(j * n[0] + i) as usize]))
}

/// Отпечаток картинки по её точкам (`kind` — что это и как сжимается): та же картинка — та
/// же сжатая, без нового сжатия.
pub fn content_key(kind: &str, size: [u32; 2], raw: &[u8]) -> u128 {
    let mut h = Twin::new();
    (program_stamp(), kind, size).hash(&mut h);
    h.write(raw);
    h.finish128()
}

/// Куски на диске: `<папка>/<отпечаток>.ksc` — размер и точки RGB, сжатые deflate.
/// Пишет их свой поток: выгрузка не ждёт сжатия.
#[derive(Clone)]
pub struct Store {
    dir: PathBuf,
    tx: std::sync::mpsc::SyncSender<(u128, u32, u32, Vec<u8>)>,
    /// Сколько кусков ещё ждут записи.
    pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

const MAGIC: &[u8; 4] = b"KSC1";

impl Store {
    pub fn open(dir: PathBuf) -> Self {
        // Очередь короткая: первая выгрузка большой карты — тысячи кусков, и без предела
        // они копились бы в памяти быстрее, чем сжимаются.
        let (tx, rx) = std::sync::mpsc::sync_channel::<(u128, u32, u32, Vec<u8>)>(64);
        let pending = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let writers = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(1, 6);
        for _ in 0..writers {
            let (at, rx, left) = (dir.clone(), rx.clone(), pending.clone());
            std::thread::spawn(move || {
                let _ = std::fs::create_dir_all(&at);
                loop {
                    let job = rx.lock().ok().and_then(|r| r.recv().ok());
                    let Some((fp, w, h, rgb)) = job else { break };
                    let _ = write_cell(&at, fp, w, h, &rgb);
                    left.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
        Self { dir, tx, pending }
    }

    fn path(dir: &Path, fp: u128) -> PathBuf {
        dir.join(format!("{fp:032x}.ksc"))
    }

    /// Кусок с этим отпечатком, если он есть и того же размера. Взятый — свежеет: его
    /// не выбросит [`Store::prune`].
    pub fn load(&self, fp: u128, w: u32, h: u32) -> Option<Vec<u8>> {
        use std::io::Read;
        let path = Self::path(&self.dir, fp);
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() < 12 || &bytes[0..4] != MAGIC {
            return None;
        }
        let (fw, fh) = (u32::from_le_bytes(bytes[4..8].try_into().ok()?), u32::from_le_bytes(bytes[8..12].try_into().ok()?));
        if (fw, fh) != (w, h) {
            return None;
        }
        let mut out = Vec::with_capacity((w * h * 3) as usize);
        flate2::read::DeflateDecoder::new(&bytes[12..]).read_to_end(&mut out).ok()?;
        if out.len() != (w * h * 3) as usize {
            return None;
        }
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Some(out)
    }

    /// Готовая картинка (крыша) с этим отпечатком: `<отпечаток>.krf`.
    pub fn load_blob(&self, fp: u128) -> Option<Vec<u8>> {
        let path = self.dir.join(format!("{fp:032x}.krf"));
        let bytes = std::fs::read(&path).ok()?;
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Some(bytes)
    }

    pub fn save_blob(&self, fp: u128, bytes: &[u8]) {
        let _ = std::fs::create_dir_all(&self.dir);
        let path = self.dir.join(format!("{fp:032x}.krf"));
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    pub fn save(&self, fp: u128, w: u32, h: u32, rgb: Vec<u8>) {
        self.pending.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.tx.send((fp, w, h, rgb)).is_err() {
            self.pending.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Дождаться записи отданных кусков, но не дольше `limit` (выгрузка из командной
    /// строки выходит сразу после записи файлов).
    pub fn flush(&self, limit: std::time::Duration) {
        let until = std::time::Instant::now() + limit;
        while self.pending.load(std::sync::atomic::Ordering::Relaxed) > 0 && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Держать кэш не больше `limit` байт: выбросить самые давние куски.
    pub fn prune(&self, limit: u64) {
        let dir = self.dir.clone();
        std::thread::spawn(move || prune_dir(&dir, limit));
    }
}

fn write_cell(dir: &Path, fp: u128, w: u32, h: u32, rgb: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut z = flate2::write::DeflateEncoder::new(Vec::with_capacity(rgb.len() / 3), flate2::Compression::fast());
    z.write_all(rgb)?;
    let body = z.finish()?;
    let mut bytes = Vec::with_capacity(body.len() + 12);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&w.to_le_bytes());
    bytes.extend_from_slice(&h.to_le_bytes());
    bytes.extend_from_slice(&body);
    // Сначала во временный файл: оборванная запись не оставит битого куска.
    let path = Store::path(dir, fp);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)
}

fn prune_dir(dir: &Path, limit: u64) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ksc" || x == "krf"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= limit / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn town() -> Map {
        let mut m = Map::new(200, 120);
        m.shadows.on = true;
        for i in 0..40 {
            let pos = [5.0 + (i % 10) as f32 * 19.0, 5.0 + (i / 10) as f32 * 28.0];
            m.props.push(crate::map::Prop { asset: format!("Crate_{i}"), pos, size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false });
        }
        m
    }

    fn cells(p: &Prints, w: u32, h: u32, px: f32, salt: u64) -> Vec<u128> {
        let s = CELL as f32 / px;
        (0..(h as f32 / s).ceil() as u32).flat_map(|j| (0..(w as f32 / s).ceil() as u32).map(move |i| [i as f32 * s, j as f32 * s, (i + 1) as f32 * s, (j + 1) as f32 * s])).map(|r| p.cell(r, salt)).collect()
    }

    #[test]
    fn an_edit_changes_only_the_cells_near_it() {
        let mut memo = HashMap::new();
        let m = town();
        let before = cells(&Prints::new(&m, &mut memo), m.width, m.height, 64.0, 1);
        assert_eq!(before, cells(&Prints::new(&m.clone(), &mut memo), m.width, m.height, 64.0, 1), "та же карта — те же отпечатки");
        // Сдвинутый ящик — только куски возле старого и нового места.
        let mut moved = m.clone();
        moved.props[13].pos[0] += 1.0;
        let after = cells(&Prints::new(&moved, &mut memo), m.width, m.height, 64.0, 1);
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        // Кусок 4×4 клетки, поля на тень 5,5 клетки: ящик задевает до 5×5 кусков.
        assert!(changed > 0 && changed <= 25, "сдвиг ящика тронул {changed} из {}", before.len());
        // Мазок маски — куски под ним и вокруг, но не вся карта.
        let mut painted = m.clone();
        painted.rooms.set(40 * mask::RES, 40 * mask::RES, 7);
        let after = cells(&Prints::new(&painted, &mut memo), m.width, m.height, 64.0, 1);
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        assert!(changed > 0 && changed < before.len() / 2, "мазок тронул {changed} из {}", before.len());
        // Новый слой земли в углу — только куски у угла.
        let mut coat = m.clone();
        let mut mask = Mask::for_cells(m.width, m.height);
        mask.set(2, 2, 255);
        coat.terrain.push(crate::map::Coat { material: 0, mask });
        let after = cells(&Prints::new(&coat, &mut memo), m.width, m.height, 64.0, 1);
        let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        // Точка в клетке (0,5; 0,5), плитка 2 клетки и поле 2 клетки: куски до 4 клеток от угла.
        assert!(changed > 0 && changed <= 4, "новый слой тронул {changed} из {}", before.len());
        // Общее (тени, проход снимка) — все куски.
        let mut lit = m.clone();
        lit.shadows.angle += 10.0;
        let after = cells(&Prints::new(&lit, &mut memo), m.width, m.height, 64.0, 1);
        assert!(before.iter().zip(&after).all(|(a, b)| a != b), "тени — вся карта");
        let other = cells(&Prints::new(&m, &mut memo), m.width, m.height, 64.0, 2);
        assert!(before.iter().zip(&other).all(|(a, b)| a != b), "другой проход — вся карта");
    }

    #[test]
    fn missing_cells_are_covered_by_few_tiles() {
        // Картинка 10×8 кусков, плитка окна — 3×2 куска с хвостиком.
        let n = [10, 8];
        let size = [10 * CELL, 8 * CELL - 100];
        let tile = [3 * CELL + 70, 2 * CELL + 30];
        let mut missing = vec![false; 80];
        for (i, j) in [(4, 3), (5, 3), (4, 4), (5, 4), (9, 7)] {
            missing[j * 10 + i] = true;
        }
        let spots = cover(&missing, n, size, tile);
        assert_eq!(spots.len(), 2, "два места правки — две плитки: {spots:?}");
        // Каждый недостающий кусок целиком внутри какой-нибудь плитки.
        for (c, _) in missing.iter().enumerate().filter(|m| *m.1) {
            let (x0, y0) = ((c as u32 % 10) * CELL, (c as u32 / 10) * CELL);
            let (x1, y1) = ((x0 + CELL).min(size[0]), (y0 + CELL).min(size[1]));
            assert!(spots.iter().any(|o| o[0] <= x0 && o[1] <= y0 && x1 <= o[0] + tile[0] && y1 <= o[1] + tile[1]), "кусок {c} не накрыт");
        }
        // Плитка у края отступает внутрь и не выходит за картинку.
        assert!(spots.iter().all(|o| o[0] + tile[0] <= size[0] && o[1] + tile[1] <= size[1]));
        assert!(touches_missing(&missing, n, size, spots[0], tile));
        assert!(!touches_missing(&missing, n, size, [0, 0], tile));
    }

    #[test]
    fn a_cell_comes_back_from_disk_as_it_was_put() {
        let dir = std::env::temp_dir().join(format!("kartograf-shotcache-{}", std::process::id()));
        let store = Store::open(dir.clone());
        let rgb: Vec<u8> = (0..20 * 10 * 3).map(|i| (i * 7 % 251) as u8).collect();
        store.save(42, 20, 10, rgb.clone());
        let mut got = None;
        for _ in 0..200 {
            got = store.load(42, 20, 10);
            if got.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(got.as_deref(), Some(rgb.as_slice()));
        assert!(store.load(42, 10, 20).is_none(), "другой размер — не он");
        assert!(store.load(43, 20, 10).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
