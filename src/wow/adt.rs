//! Местность WoW 3.x (ADT): куски рельефа по 33⅓ ярда (высоты, дыры, слои текстур с масками,
//! цвета вершин, тени), вода в куске (`MCLQ`) и на весь файл (`MH2O`), расстановка моделей
//! (`MDDF`) и зданий (`MODF`). Карта инстанса (WDT) — какие файлы из 64×64 есть и не одно ли
//! это здание.

use super::wmo::Liquid;

/// Сторона файла местности, ярды.
pub const TILE: f32 = 1600.0 / 3.0;
/// Сторона куска (16 на файл).
pub const CHUNK: f32 = TILE / 16.0;
/// Шаг сетки высот (8 на кусок) — он же клетка воды.
pub const UNIT: f32 = CHUNK / 8.0;
/// Координаты расстановки отсчитаны от угла мира, мировые — от середины.
pub const ORIGIN: f32 = 32.0 * TILE;

/// Место модели или здания в мире: координаты и углы расстановки (градусы), масштаб, набор
/// обстановки здания.
#[derive(Clone, Debug)]
pub struct Place {
    pub name: usize,
    pub unique: u32,
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub scale: f32,
    pub set: u16,
    /// Рамка здания в мире `[x0, y0, x1, y1]` из `MODF` (у моделей и поставленного по запросу — нет).
    pub bounds: Option<[f32; 4]>,
}

impl Place {
    /// Точка модели — в мир (X — север, Y — запад, Z — вверх). Проверено по рамкам `MODF`
    /// копей: здание повёрнуто на `rot[1] + 180°` вокруг вертикали.
    pub fn to_world(&self, m: [f32; 3]) -> [f32; 3] {
        let (s, c) = self.rot[1].to_radians().sin_cos();
        let (x0, y0) = (ORIGIN - self.pos[2], ORIGIN - self.pos[0]);
        let (mx, my, mz) = (m[0] * self.scale, m[1] * self.scale, m[2] * self.scale);
        [x0 - mx * c + my * s, y0 - mx * s - my * c, self.pos[1] + mz]
    }

    /// Направление модели — в мир (без сдвига и масштаба).
    pub fn turn(&self, v: [f32; 3]) -> [f32; 3] {
        let (s, c) = self.rot[1].to_radians().sin_cos();
        [-v[0] * c + v[1] * s, -v[0] * s - v[1] * c, v[2]]
    }

    /// Точка мира — в модель (по плоскости; обратное к `to_world`).
    pub fn to_model(&self, w: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rot[1].to_radians().sin_cos();
        let (dx, dy) = (w[0] - (ORIGIN - self.pos[2]), w[1] - (ORIGIN - self.pos[0]));
        [-(dx * c + dy * s) / self.scale, (dx * s - dy * c) / self.scale]
    }

    /// Поворот модели вокруг вертикали в мире, радианы.
    pub fn yaw(&self) -> f32 {
        self.rot[1].to_radians() + std::f32::consts::PI
    }
}

/// Как красить кусок местности в сцене: текстуры слоёв (номер в текстурах сцены) с масками
/// 64×64 и запечённая тень.
#[derive(Clone, Debug)]
pub struct Paint {
    pub layers: Vec<(Option<usize>, Option<Vec<u8>>)>,
    pub shadow: Option<Vec<u8>>,
}

/// Слой текстуры куска: номер в `MTEX` и маска 64×64 (у первого слоя маски нет).
#[derive(Clone, Debug)]
pub struct Layer {
    pub tex: usize,
    pub alpha: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct Chunk {
    /// Угол куска: самые северная и западная точки (`X`, `Y`) и основание высот.
    pub pos: [f32; 3],
    /// 145 высот над основанием: 9 узлов, 8 середин, 9 узлов…
    pub heights: Vec<f32>,
    /// Цвета вершин (`MCCV`, 1.0 — как есть), если есть.
    pub colors: Option<Vec<[f32; 3]>>,
    /// Дыры 4×4: бит `(строка / 2) · 4 + столбец / 2`.
    pub holes: u16,
    pub layers: Vec<Layer>,
    /// Запечённая тень 64×64 (1 — в тени).
    pub shadow: Option<Vec<u8>>,
    /// Вода куска сеткой 8×8 и вид: 0 вода, 1 океан, 2 магма, 3 слизь.
    pub liquid: Option<Liquid>,
}

impl Chunk {
    /// Вершина `k` из 145 в мире.
    pub fn vertex(&self, k: usize) -> [f32; 3] {
        let (row, col) = (k / 17, k % 17);
        let (i, j) = if col < 9 { (row as f32, col as f32) } else { (row as f32 + 0.5, (col - 9) as f32 + 0.5) };
        [self.pos[0] - i * UNIT, self.pos[1] - j * UNIT, self.pos[2] + self.heights.get(k).copied().unwrap_or(0.0)]
    }

    /// Доля куска по столбцам и строкам (0–1) у вершины `k`: для масок и текстур.
    pub fn uv(k: usize) -> [f32; 2] {
        let (row, col) = (k / 17, k % 17);
        if col < 9 { [col as f32 / 8.0, row as f32 / 8.0] } else { [((col - 9) as f32 + 0.5) / 8.0, (row as f32 + 0.5) / 8.0] }
    }

    /// Треугольники куска без дыр: в каждом квадрате 4 вокруг середины.
    pub fn triangles(&self) -> Vec<[u32; 3]> {
        let mut out = Vec::with_capacity(256);
        for i in 0..8u32 {
            for j in 0..8u32 {
                if self.holes & (1 << ((i / 2) * 4 + j / 2)) != 0 {
                    continue;
                }
                let c = i * 17 + 9 + j;
                let (a, b, d, e) = (i * 17 + j, i * 17 + j + 1, (i + 1) * 17 + j + 1, (i + 1) * 17 + j);
                // Обход против часовой стрелки сверху: нормаль вверх (мир: X — север, Y — запад).
                out.extend([[c, b, a], [c, d, b], [c, e, d], [c, a, e]]);
            }
        }
        out
    }
}

pub struct Adt {
    pub textures: Vec<String>,
    pub models: Vec<String>,
    pub wmos: Vec<String>,
    pub doodads: Vec<Place>,
    pub objects: Vec<Place>,
    pub chunks: Vec<Chunk>,
}

/// Карта (WDT): флаги (`MPHD`: 0x1 — только здание, 0x4 — маски по 8 бит), файлы местности
/// `(x, y)` — `<карта>_<x>_<y>.adt`, здание у карты-здания.
pub struct Wdt {
    pub flags: u32,
    pub tiles: Vec<(u32, u32)>,
    pub wmo: Option<String>,
}

fn chunks(b: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let mut tag: [u8; 4] = b[i..i + 4].try_into().unwrap();
        tag.reverse();
        let n = u32_at(b, i + 4) as usize;
        out.push((tag, i + 8, n.min(b.len() - i - 8)));
        i += 8 + n;
    }
    out
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}
fn u16_at(b: &[u8], o: usize) -> u16 {
    b.get(o..o + 2).map_or(0, |s| u16::from_le_bytes(s.try_into().unwrap()))
}
fn f32_at(b: &[u8], o: usize) -> f32 {
    b.get(o..o + 4).map_or(0.0, |s| f32::from_le_bytes(s.try_into().unwrap()))
}

fn names(blob: &[u8]) -> Vec<String> {
    blob.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect()
}

/// Имена по смещениям (`MMID`/`MWID` в блоб `MMDX`/`MWMO`).
fn names_at(blob: &[u8], ids: &[u8]) -> Vec<String> {
    ids.chunks_exact(4)
        .map(|c| {
            let o = u32::from_le_bytes(c.try_into().unwrap()) as usize;
            blob.get(o..).map(|s| String::from_utf8_lossy(s.split(|&c| c == 0).next().unwrap_or_default()).into_owned()).unwrap_or_default()
        })
        .collect()
}

fn modf(e: &[u8]) -> Place {
    Place {
        name: u32_at(e, 0) as usize,
        unique: u32_at(e, 4),
        pos: [f32_at(e, 8), f32_at(e, 12), f32_at(e, 16)],
        rot: [f32_at(e, 20), f32_at(e, 24), f32_at(e, 28)],
        scale: 1.0,
        set: u16_at(e, 58),
        // Рамка в координатах расстановки: X мира = ORIGIN − z, Y = ORIGIN − x.
        bounds: Some([ORIGIN - f32_at(e, 52), ORIGIN - f32_at(e, 44), ORIGIN - f32_at(e, 40), ORIGIN - f32_at(e, 32)]),
    }
}

/// Файл местности `(x, y)` задевает рамку мира `[x0, y0, x1, y1]`: его X мира — от
/// `(32 − y − 1)·TILE` до `(32 − y)·TILE`, Y — так же по `x`.
pub fn tile_touches(x: u32, y: u32, r: [f32; 4]) -> bool {
    let (xn, yw) = ((32.0 - y as f32) * TILE, (32.0 - x as f32) * TILE);
    xn >= r[0] && xn - TILE <= r[2] && yw >= r[1] && yw - TILE <= r[3]
}

pub fn parse_wdt(b: &[u8]) -> Result<Wdt, String> {
    let c = chunks(b);
    let get = |t: &[u8; 4]| c.iter().find(|(tag, _, _)| tag == t).map(|&(_, o, n)| &b[o..o + n]);
    let flags = get(b"MPHD").map_or(0, |d| u32_at(d, 0));
    let main = get(b"MAIN").ok_or("WDT: нет MAIN")?;
    let tiles = (0..4096u32).filter(|&k| u32_at(main, k as usize * 8) & 1 != 0).map(|k| (k % 64, k / 64)).collect();
    let wmo = get(b"MWMO").map(names).and_then(|n| n.into_iter().next());
    Ok(Wdt { flags, tiles, wmo })
}

/// Маска слоя: 8 бит (большие маски карты) — сжатая (`flags & 0x200`) или 4096 байт; иначе
/// 4 бита, 2048 байт, младшая тетрада — левая точка; без флага куска 0x8000 последние
/// строка и столбец — копии предпоследних.
fn alpha_map(mcal: &[u8], ofs: usize, flags: u32, big: bool, fix: bool) -> Vec<u8> {
    let mut a = vec![0u8; 4096];
    let src = mcal.get(ofs..).unwrap_or_default();
    if big && flags & 0x200 != 0 {
        let (mut i, mut o) = (0, 0);
        while o < 4096 && i < src.len() {
            let h = src[i];
            i += 1;
            let n = (h & 0x7F) as usize;
            if h & 0x80 != 0 {
                let v = src.get(i).copied().unwrap_or(0);
                i += 1;
                for _ in 0..n.min(4096 - o) {
                    a[o] = v;
                    o += 1;
                }
            } else {
                for _ in 0..n.min(4096 - o) {
                    a[o] = src.get(i).copied().unwrap_or(0);
                    i += 1;
                    o += 1;
                }
            }
        }
    } else if big {
        for (o, v) in a.iter_mut().enumerate() {
            *v = src.get(o).copied().unwrap_or(0);
        }
    } else {
        for o in 0..2048 {
            let v = src.get(o).copied().unwrap_or(0);
            a[o * 2] = (v & 0x0F) * 17;
            a[o * 2 + 1] = (v >> 4) * 17;
        }
        if fix {
            for r in 0..64 {
                a[r * 64 + 63] = a[r * 64 + 62];
            }
            for col in 0..64 {
                a[63 * 64 + col] = a[62 * 64 + col];
            }
        }
    }
    a
}

/// Вода куска старым способом (`MCLQ`): высоты 9×9 и клетки 8×8 (0x0F — пусто). Сетка
/// переворачивается: у `Liquid` угол — самые южная и восточная точки, шаги — на север и запад.
fn mclq(d: &[u8], pos: [f32; 3], kind: u8) -> Option<Liquid> {
    if d.len() < 8 + 81 * 8 + 64 {
        return None;
    }
    let h = |i: usize, j: usize| f32_at(d, 8 + (i * 9 + j) * 8 + 4);
    let tile = |i: usize, j: usize| d[8 + 81 * 8 + i * 8 + j];
    // У `Liquid` номер узла — `iy · 9 + ix`, ix растёт на север (строка куска i = 8 − ix),
    // iy — на запад (столбец j = 8 − iy).
    let heights = (0..81).map(|k| h(8 - k % 9, 8 - k / 9)).collect();
    let shown: Vec<bool> = (0..64).map(|k| tile(7 - k % 8, 7 - k / 8) & 0x0F != 0x0F).collect();
    shown.iter().any(|&s| s).then(|| Liquid { corner: [pos[0] - 8.0 * UNIT, pos[1] - 8.0 * UNIT, f32_at(d, 0)], verts: [9, 9], tiles: [8, 8], heights, shown, tile_kind: kind })
}

/// Вид воды по номеру LiquidType.dbc (как у зданий с флагом 0x4): код клетки.
fn liquid_code(id: u16) -> u8 {
    match id {
        n @ 1..=12 => ((n - 1) & 3) as u8,
        15 | 19 | 121 | 141 => 2,
        20 | 21 | 181 => 3,
        _ => 0,
    }
}

/// Вода на весь файл (`MH2O`, 3.x): для каждого куска — первый слой, клетки по маске.
fn mh2o(d: &[u8], chunks: &mut [Chunk]) {
    for (k, ch) in chunks.iter_mut().enumerate() {
        let (ofs, layers) = (u32_at(d, k * 12) as usize, u32_at(d, k * 12 + 4));
        if ofs == 0 || layers == 0 || ofs + 24 > d.len() {
            continue;
        }
        let inst = &d[ofs..ofs + 24];
        let (id, lvf) = (u16_at(inst, 0), u16_at(inst, 2));
        let (min_h, _max_h) = (f32_at(inst, 4), f32_at(inst, 8));
        let (x0, y0, w, h) = (inst[12] as usize, inst[13] as usize, inst[14] as usize, inst[15] as usize);
        let (ofs_mask, ofs_vert) = (u32_at(inst, 16) as usize, u32_at(inst, 20) as usize);
        if w == 0 || h == 0 || x0 + w > 8 || y0 + h > 8 {
            continue;
        }
        // Высоты узлов (w+1)×(h+1), если формат их хранит (0, 1, 3), иначе — ровно.
        let vh = |r: usize, c: usize| -> f32 {
            if ofs_vert == 0 || lvf == 2 {
                return min_h;
            }
            f32_at(d, ofs_vert + (r * (w + 1) + c) * 4)
        };
        let has = |r: usize, c: usize| -> bool {
            if ofs_mask == 0 {
                return true;
            }
            let bit = r * w + c;
            d.get(ofs_mask + bit / 8).is_some_and(|b| b & (1 << (bit % 8)) != 0)
        };
        // Сетка 9×9 куска: строки (x0 — столбец по Y, y0 — строка по X), вне слоя — ровно.
        let mut grid = [[min_h; 9]; 9];
        let mut tiles = [[false; 8]; 8];
        for r in 0..=h {
            for c in 0..=w {
                grid[y0 + r][x0 + c] = vh(r, c);
            }
        }
        for r in 0..h {
            for c in 0..w {
                tiles[y0 + r][x0 + c] = has(r, c);
            }
        }
        let heights = (0..81).map(|k| grid[8 - k % 9][8 - k / 9]).collect();
        let shown = (0..64).map(|k| tiles[7 - k % 8][7 - k / 8]).collect();
        ch.liquid = Some(Liquid { corner: [ch.pos[0] - 8.0 * UNIT, ch.pos[1] - 8.0 * UNIT, min_h], verts: [9, 9], tiles: [8, 8], heights, shown, tile_kind: liquid_code(id) });
    }
}

/// Файл местности. `big` — у карты маски по 8 бит (`MPHD & 0x4`).
pub fn parse_adt(b: &[u8], big: bool) -> Result<Adt, String> {
    let c = chunks(b);
    let get = |t: &[u8; 4]| c.iter().find(|(tag, _, _)| tag == t).map(|&(_, o, n)| &b[o..o + n]);
    let textures = get(b"MTEX").map(names).unwrap_or_default();
    let models = names_at(get(b"MMDX").unwrap_or_default(), get(b"MMID").unwrap_or_default());
    let wmos = names_at(get(b"MWMO").unwrap_or_default(), get(b"MWID").unwrap_or_default());
    let doodads = get(b"MDDF")
        .unwrap_or_default()
        .chunks_exact(36)
        .map(|e| Place { name: u32_at(e, 0) as usize, unique: u32_at(e, 4), pos: [f32_at(e, 8), f32_at(e, 12), f32_at(e, 16)], rot: [f32_at(e, 20), f32_at(e, 24), f32_at(e, 28)], scale: u16_at(e, 32) as f32 / 1024.0, set: 0, bounds: None })
        .collect();
    let objects = get(b"MODF").unwrap_or_default().chunks_exact(64).map(modf).collect();
    let mut out = Vec::with_capacity(256);
    for &(tag, o, n) in &c {
        if &tag != b"MCNK" || n < 128 {
            continue;
        }
        // Смещения подчастей — от начала куска с его заголовком: `o - 8`.
        let base = o - 8;
        let m = &b[base..o + n];
        let h = &b[o..o + 128];
        let flags = u32_at(h, 0);
        let n_layers = u32_at(h, 0x0C) as usize;
        let sub = |ofs: u32| -> &[u8] {
            let ofs = ofs as usize;
            if ofs == 0 || ofs + 8 > m.len() {
                return &[];
            }
            let size = u32_at(m, ofs + 4) as usize;
            &m[ofs + 8..(ofs + 8 + size).min(m.len())]
        };
        let pos = [f32_at(h, 0x68), f32_at(h, 0x6C), f32_at(h, 0x70)];
        let mcvt = sub(u32_at(h, 0x14));
        let heights: Vec<f32> = (0..145).map(|k| f32_at(mcvt, k * 4)).collect();
        let colors = (u32_at(h, 0x74) != 0).then(|| sub(u32_at(h, 0x74))).filter(|d| d.len() >= 145 * 4).map(|d| (0..145).map(|k| [d[k * 4 + 2] as f32 / 127.0, d[k * 4 + 1] as f32 / 127.0, d[k * 4] as f32 / 127.0]).collect());
        let mcly = sub(u32_at(h, 0x1C));
        let mcal = sub(u32_at(h, 0x24));
        let fix = flags & 0x8000 == 0;
        let layers = (0..n_layers.min(mcly.len() / 16))
            .map(|i| {
                let l = &mcly[i * 16..i * 16 + 16];
                let (tex, lflags, ofs) = (u32_at(l, 0) as usize, u32_at(l, 4), u32_at(l, 8) as usize);
                let alpha = (i > 0 && lflags & 0x100 != 0).then(|| alpha_map(mcal, ofs, lflags, big, fix));
                Layer { tex, alpha }
            })
            .collect();
        let shadow = (flags & 0x1 != 0).then(|| sub(u32_at(h, 0x2C))).filter(|d| d.len() >= 512).map(|d| (0..4096).map(|i| (d[i / 8] >> (i % 8)) & 1).collect());
        // Вид старой воды — по флагам куска: 0x4 река, 0x8 океан, 0x10 магма, 0x20 слизь.
        let kind = if flags & 0x10 != 0 { 2 } else if flags & 0x20 != 0 { 3 } else if flags & 0x8 != 0 { 1 } else { 0 };
        let (ofs_liq, size_liq) = (u32_at(h, 0x60) as usize, u32_at(h, 0x64) as usize);
        let liquid = (flags & 0x3C != 0 && size_liq > 8 && ofs_liq + size_liq <= m.len()).then(|| mclq(&m[ofs_liq + 8..ofs_liq + size_liq], pos, kind)).flatten();
        out.push(Chunk { pos, heights, colors, holes: u16_at(h, 0x3C), layers, shadow, liquid });
    }
    if let Some(d) = get(b"MH2O") {
        mh2o(d, &mut out);
    }
    Ok(Adt { textures, models, wmos, doodads, objects, chunks: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_touches_only_its_own_square_of_the_world() {
        // Торговый квартал Штормграда (X −8808, Y 638) — в файле Azeroth_30_48.
        let r = [-8810.0, 636.0, -8806.0, 640.0];
        assert!(tile_touches(30, 48, r));
        assert!(!tile_touches(31, 48, r), "Y 0…533 — восточнее");
        assert!(!tile_touches(30, 47, r), "X севернее");
        // Рамка на стыке четырёх файлов задевает все четыре.
        let corner = [-TILE - 1.0, -1.0, -TILE + 1.0, 1.0];
        assert!([(31, 32), (32, 32), (31, 33), (32, 33)].iter().all(|&(x, y)| tile_touches(x, y, corner)));
    }

    #[test]
    fn placement_turns_and_moves_like_the_game() {
        // Здание копей: расстановка (17718,3; 29,8; 17223,1), поворот −91°. Начало модели — в
        // середину рамки мира по X и Y (проверено по рамке MODF: X −323…22, Y −896…−337).
        let p = Place { name: 0, unique: 0, pos: [17718.3, 29.8, 17223.1], rot: [0.0, -91.0, 0.0], scale: 1.0, set: 0, bounds: None };
        let o = p.to_world([0.0, 0.0, 0.0]);
        assert!((o[0] - (ORIGIN - 17223.1)).abs() < 0.01 && (o[1] - (ORIGIN - 17718.3)).abs() < 0.01 && (o[2] - 29.8).abs() < 1e-4);
        // Без поворота модель развёрнута на 180°: её +X смотрит на юг.
        let q = Place { rot: [0.0; 3], ..p.clone() };
        let a = q.to_world([1.0, 0.0, 0.0]);
        let b = q.to_world([0.0, 0.0, 0.0]);
        assert!((a[0] - b[0] + 1.0).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3);
        assert!((q.yaw() - std::f32::consts::PI).abs() < 1e-6);
        // Туда и обратно.
        let w = p.to_world([12.0, -7.0, 3.0]);
        let m = p.to_model([w[0], w[1]]);
        assert!((m[0] - 12.0).abs() < 1e-2 && (m[1] + 7.0).abs() < 1e-2, "{m:?}");
    }

    #[test]
    fn modf_bounds_are_world_rectangles() {
        // Здание копей: рамка расстановки (17404, −22, 17045)–(17963, 87, 17390) → мир X −323…22, Y −896…−337.
        let mut e = vec![0u8; 64];
        for (i, v) in [17718.3f32, 29.8, 17223.1, 0.0, -91.0, 0.0, 17404.0, -22.0, 17045.0, 17963.0, 87.0, 17390.0].iter().enumerate() {
            e[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let b = modf(&e).bounds.unwrap();
        assert!((b[0] + 323.3).abs() < 1.0 && (b[2] - 21.7).abs() < 1.0 && (b[1] + 896.3).abs() < 1.0 && (b[3] + 337.3).abs() < 1.0, "{b:?}");
    }

    #[test]
    fn chunk_vertices_triangles_and_holes() {
        let ch = Chunk { pos: [100.0, 200.0, 5.0], heights: vec![1.0; 145], colors: None, holes: 0, layers: Vec::new(), shadow: None, liquid: None };
        assert_eq!(ch.vertex(0), [100.0, 200.0, 6.0]);
        // Последний узел — юго-восточный угол куска, середина первого квадрата — полшага внутрь.
        let last = ch.vertex(144);
        assert!((last[0] - (100.0 - CHUNK)).abs() < 1e-3 && (last[1] - (200.0 - CHUNK)).abs() < 1e-3);
        let mid = ch.vertex(9);
        assert!((mid[0] - (100.0 - UNIT / 2.0)).abs() < 1e-3);
        assert_eq!(ch.triangles().len(), 256);
        // Треугольники смотрят вверх: иначе рельеф не пол.
        for t in ch.triangles() {
            let [p, q, r] = t.map(|i| ch.vertex(i as usize));
            let (u, v) = ([q[0] - p[0], q[1] - p[1]], [r[0] - p[0], r[1] - p[1]]);
            assert!(u[0] * v[1] - u[1] * v[0] > 0.0, "треугольник {t:?} смотрит вниз");
        }
        let holed = Chunk { holes: 1, ..ch };
        assert_eq!(holed.triangles().len(), 256 - 16, "дыра — квадрат 2×2");
    }

    #[test]
    fn old_water_tiles_land_where_the_chunk_has_them() {
        // Вода только в клетке строки 0, столбца 2 куска: x от pos0 − UNIT до pos0, y от pos1 − 3·UNIT
        // до pos1 − 2·UNIT. Высоты узлов — номер строки.
        let mut d = vec![0u8; 8 + 81 * 8 + 64 + 84];
        for i in 0..9 {
            for j in 0..9 {
                d[8 + (i * 9 + j) * 8 + 4..8 + (i * 9 + j) * 8 + 8].copy_from_slice(&(i as f32).to_le_bytes());
            }
        }
        for t in 0..64 {
            d[8 + 81 * 8 + t] = if t == 2 { 0 } else { 0x0F };
        }
        let pos = [100.0, 200.0, 0.0];
        let l = mclq(&d, pos, 0).unwrap();
        let k = l.shown.iter().position(|&s| s).unwrap();
        let (tx, ty) = (k % 8, k / 8);
        let (x, y) = (l.corner[0] + (tx as f32 + 0.5) * UNIT, l.corner[1] + (ty as f32 + 0.5) * UNIT);
        assert!(x > pos[0] - UNIT && x < pos[0], "по X — первая строка: {x}");
        assert!(y > pos[1] - 3.0 * UNIT && y < pos[1] - 2.0 * UNIT, "по Y — третий столбец: {y}");
        // Узел у северного края (ix = 8) — строка 0: высота 0; у южного (ix = 0) — строка 8.
        assert_eq!((l.heights[8], l.heights[0]), (0.0, 8.0));
    }

    #[test]
    fn four_bit_alpha_is_unpacked_and_the_edge_fixed() {
        let mut mcal = vec![0u8; 2048];
        mcal[0] = 0xF0; // точка 0 — 0, точка 1 — 15
        mcal[31] = 0x0F; // точки 62 и 63 первой строки: 15 и 0 → 63 = копия 62
        let a = alpha_map(&mcal, 0, 0x100, false, true);
        assert_eq!((a[0], a[1], a[62], a[63]), (0, 255, 255, 255));
        let big = alpha_map(&[0x82, 7, 0x01, 9], 0, 0x300, true, false);
        assert_eq!(&big[..3], &[7, 7, 9]);
    }

    /// Настоящий файл: `ADT=…\DeadminesInstance_33_32.adt cargo test adt_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn adt_real() {
        let Ok(path) = std::env::var("ADT") else { return };
        let a = parse_adt(&std::fs::read(path).unwrap(), false).unwrap();
        let water = a.chunks.iter().filter(|c| c.liquid.is_some()).count();
        println!("кусков {}, текстур {}, моделей {}/{}, зданий {}/{}, с водой {water}", a.chunks.len(), a.textures.len(), a.models.len(), a.doodads.len(), a.wmos.len(), a.objects.len());
        assert_eq!(a.chunks.len(), 256);
    }
}
