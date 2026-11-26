//! Здания и подземелья WMO (WoW 1.x–3.x, версия 17): корень (материалы, текстуры,
//! расстановка предметов) и группы (`<имя>_NNN.wmo`: вершины, треугольники, цвета вершин).

/// Материал: текстура и как кладётся.
#[derive(Clone)]
pub struct Material {
    pub flags: u32,
    /// 0 — сплошной, 1 — с вырезом по альфе, 2 — полупрозрачный, дальше — свечения.
    pub blend: u32,
    pub texture: String,
}

/// Предмет на своём месте: модель (имя из `MODN`, обычно `.mdx`), место, поворот, масштаб.
pub struct Doodad {
    pub model: String,
    pub pos: [f32; 3],
    /// Кватернион x, y, z, w.
    pub rot: [f32; 4],
    pub scale: f32,
}

/// Набор предметов: первый — всегда стоящий (`Set_$DefaultGlobal`), прочие — варианты
/// обстановки (место в мире выбирает один) и праздники.
pub struct DoodadSet {
    pub name: String,
    pub first: usize,
    pub count: usize,
}

#[derive(Clone)]
pub struct Group {
    /// Флаги группы (`MOGP`): 0x2000 — внутренняя, 0x8 — наружная.
    #[allow(dead_code)]
    pub flags: u32,
    pub verts: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Запечённый свет внутренних групп (BGRA → RGB 0–1), если есть.
    pub colors: Option<Vec<[f32; 3]>>,
    pub tris: Vec<[u32; 3]>,
    /// Флаги треугольника (`MOPY`) и номер материала (255 — только столкновение).
    pub tri_flags: Vec<u8>,
    pub tri_mat: Vec<u8>,
    /// Вода (`MLIQ`): угол, шаг 4,1667 ярда, высоты узлов, видимые клетки.
    pub liquid: Option<Liquid>,
    /// Вид жидкости группы (`MOGP.groupLiquid`): номер из LiquidType.dbc или старый код.
    pub group_liquid: u32,
    /// Номер первого материала группы в сцене: у карты с несколькими зданиями материалы подряд.
    pub mat_base: usize,
    /// Кусок местности: слои текстур с масками вместо материала.
    pub terrain: Option<Box<super::adt::Paint>>,
}

#[derive(Clone, Debug)]
pub struct Liquid {
    pub corner: [f32; 3],
    pub verts: [usize; 2],
    pub tiles: [usize; 2],
    pub heights: Vec<f32>,
    pub shown: Vec<bool>,
    /// Младшие биты флагов первой видимой клетки — старый код вида жидкости.
    pub tile_kind: u8,
}

/// Что за жидкость: вода, лава или слизь — рисуются разным цветом.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LiquidKind {
    Water,
    Magma,
    Slime,
}

/// Вид жидкости группы. Корень с флагом 0x4 (`MOHD`) пишет номер LiquidType.dbc: 1–12 —
/// вода, океан, магма, слизь по кругу (обычные, медленные, быстрые), 15, 19, 121, 141 —
/// магма, 20, 21, 181 — слизь, прочие — вода. Без флага — по флагам клеток: & 3 — 0 вода,
/// 1 океан, 2 магма, 3 слизь (Огненная пропасть — 2, Стенающие пещеры — 4).
pub fn liquid_kind(root_flags: u16, group_liquid: u32, tile_kind: u8) -> LiquidKind {
    let basic = |n: u32| match n & 3 {
        2 => LiquidKind::Magma,
        3 => LiquidKind::Slime,
        _ => LiquidKind::Water,
    };
    if root_flags & 0x4 == 0 {
        return basic(tile_kind as u32);
    }
    match group_liquid {
        n @ 1..=12 => basic(n - 1),
        15 | 19 | 121 | 141 => LiquidKind::Magma,
        20 | 21 | 181 => LiquidKind::Slime,
        _ => LiquidKind::Water,
    }
}

pub const LIQUID_STEP: f32 = 4.166_666_5;

impl Group {
    /// Треугольник держит фишку: столкновение или видимый не «деталь» (`MOPY`).
    pub fn collides(&self, t: usize) -> bool {
        let f = self.tri_flags[t];
        f & 0x08 != 0 || (f & 0x20 != 0 && f & 0x04 == 0)
    }
    /// Треугольник рисуется: есть материал.
    pub fn renders(&self, t: usize) -> bool {
        self.tri_mat[t] != 0xFF
    }
}

pub struct Wmo {
    /// Флаги корня (`MOHD`): 0x4 — вид жидкости групп — номер LiquidType.dbc.
    pub flags: u16,
    pub materials: Vec<Material>,
    pub doodads: Vec<Doodad>,
    pub sets: Vec<DoodadSet>,
    pub group_count: usize,
}

fn chunks(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let mut tag: [u8; 4] = b[i..i + 4].try_into().unwrap();
        tag.reverse();
        let n = u32::from_le_bytes(b[i + 4..i + 8].try_into().unwrap()) as usize;
        let end = (i + 8 + n).min(b.len());
        out.push((tag, &b[i + 8..end]));
        i += 8 + n;
    }
    out
}

fn find<'a>(c: &[([u8; 4], &'a [u8])], tag: &[u8; 4]) -> Option<&'a [u8]> {
    c.iter().find(|(t, _)| t == tag).map(|(_, d)| *d)
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    b.get(o..o + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}
fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Строка из блоба имён по смещению.
fn name_at(blob: &[u8], ofs: usize) -> String {
    blob.get(ofs..).map(|s| String::from_utf8_lossy(s.split(|&c| c == 0).next().unwrap_or_default()).into_owned()).unwrap_or_default()
}

pub fn parse_root(b: &[u8]) -> Result<Wmo, String> {
    let c = chunks(b);
    let mohd = find(&c, b"MOHD").ok_or("WMO: нет MOHD (это файл группы, а не корень?)")?;
    let group_count = u32_at(mohd, 4) as usize;
    let flags = mohd.get(60..62).map_or(0, |f| u16::from_le_bytes([f[0], f[1]]));
    let motx = find(&c, b"MOTX").unwrap_or_default();
    let materials = find(&c, b"MOMT")
        .unwrap_or_default()
        .chunks_exact(64)
        .map(|m| Material { flags: u32_at(m, 0), blend: u32_at(m, 8), texture: name_at(motx, u32_at(m, 12) as usize) })
        .collect();
    let modn = find(&c, b"MODN").unwrap_or_default();
    let doodads = find(&c, b"MODD")
        .unwrap_or_default()
        .chunks_exact(40)
        .map(|d| Doodad {
            model: name_at(modn, (u32_at(d, 0) & 0x00FF_FFFF) as usize),
            pos: [f32_at(d, 4), f32_at(d, 8), f32_at(d, 12)],
            rot: [f32_at(d, 16), f32_at(d, 20), f32_at(d, 24), f32_at(d, 28)],
            scale: f32_at(d, 32),
        })
        .collect();
    let sets = find(&c, b"MODS")
        .unwrap_or_default()
        .chunks_exact(32)
        .map(|s| DoodadSet { name: name_at(&s[..20], 0), first: u32_at(s, 20) as usize, count: u32_at(s, 24) as usize })
        .collect();
    Ok(Wmo { flags, materials, doodads, sets, group_count })
}

pub fn parse_group(b: &[u8]) -> Result<Group, String> {
    let c = chunks(b);
    let mogp = find(&c, b"MOGP").ok_or("WMO: в группе нет MOGP")?;
    if mogp.len() < 0x44 {
        return Err("WMO: короткий MOGP".into());
    }
    let flags = u32_at(mogp, 8);
    let group_liquid = u32_at(mogp, 0x34);
    let s = chunks(&mogp[0x44..]);
    let verts: Vec<[f32; 3]> = find(&s, b"MOVT").unwrap_or_default().chunks_exact(12).map(|v| [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]).collect();
    let normals = find(&s, b"MONR").unwrap_or_default().chunks_exact(12).map(|v| [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)]).collect();
    let uvs = find(&s, b"MOTV").unwrap_or_default().chunks_exact(8).map(|v| [f32_at(v, 0), f32_at(v, 4)]).collect();
    let colors = find(&s, b"MOCV").map(|d| d.chunks_exact(4).map(|c| [c[2] as f32 / 255.0, c[1] as f32 / 255.0, c[0] as f32 / 255.0]).collect());
    let idx: Vec<u32> = find(&s, b"MOVI").unwrap_or_default().chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect();
    let tris: Vec<[u32; 3]> = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).filter(|t| t.iter().all(|&i| (i as usize) < verts.len())).collect();
    let mopy = find(&s, b"MOPY").unwrap_or_default();
    let (mut tri_flags, mut tri_mat): (Vec<u8>, Vec<u8>) = mopy.chunks_exact(2).map(|p| (p[0], p[1])).unzip();
    tri_flags.resize(tris.len(), 0);
    tri_mat.resize(tris.len(), 0xFF);
    let liquid = find(&s, b"MLIQ").and_then(|l| {
        if l.len() < 30 {
            return None;
        }
        let verts = [u32_at(l, 0) as usize, u32_at(l, 4) as usize];
        let tiles = [u32_at(l, 8) as usize, u32_at(l, 12) as usize];
        let corner = [f32_at(l, 16), f32_at(l, 20), f32_at(l, 24)];
        let nv = verts[0] * verts[1];
        let heights: Vec<f32> = (0..nv).map(|i| l.get(30 + i * 8 + 4..30 + i * 8 + 8).map_or(corner[2], |h| f32::from_le_bytes(h.try_into().unwrap()))).collect();
        let tile_at = 30 + nv * 8;
        // Клетка с 0x0F в младших битах (тип 15) — пустая.
        let shown: Vec<bool> = (0..tiles[0] * tiles[1]).map(|i| l.get(tile_at + i).is_some_and(|f| f & 0x0F != 0x0F)).collect();
        let tile_kind = shown.iter().position(|&s| s).and_then(|i| l.get(tile_at + i)).map_or(0, |f| f & 0x0F);
        Some(Liquid { corner, verts, tiles, heights, shown, tile_kind })
    });
    Ok(Group { flags, verts, normals, uvs, colors, tris, tri_flags, tri_mat, liquid, group_liquid, mat_base: 0, terrain: None })
}

/// Поворот предмета вокруг вертикали (рыскание) из кватерниона, в радианах.
pub fn yaw(q: [f32; 4]) -> f32 {
    let [x, y, z, w] = q;
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// Путь файла группы: `Path\Name.wmo` → `Path\Name_007.wmo`.
pub fn group_path(root: &str, i: usize) -> String {
    let stem = root.rsplit_once('.').map_or(root, |(s, _)| s);
    format!("{stem}_{i:03}.wmo")
}

/// Корень WMO, а не файл группы (`_000.wmo`).
pub fn is_root(path: &str) -> bool {
    let lower = path.to_lowercase();
    let Some(stem) = lower.strip_suffix(".wmo") else { return false };
    !stem.rsplit_once('_').is_some_and(|(_, tail)| tail.len() == 3 && tail.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_files_are_not_roots() {
        assert!(is_root("World\\wmo\\Dungeon\\SM\\Monestary_Cathedral.wmo"));
        assert!(!is_root("World\\wmo\\Dungeon\\SM\\Monestary_Cathedral_012.wmo"));
        assert!(is_root("World\\wmo\\A\\Gate_02.wmo"));
        assert_eq!(group_path("A\\B.wmo", 7), "A\\B_007.wmo");
    }

    #[test]
    fn liquid_kinds_from_dbc_ids_and_tile_flags() {
        // Номер LiquidType.dbc (флаг корня 0x4): Чёрная гора — 7 (медленная магма),
        // Подгород — 4 (слизь), Стальгорн — 3 (магма), 13 — вода WMO, 20 — слизь WMO.
        assert_eq!(liquid_kind(0x5, 7, 0), LiquidKind::Magma);
        assert_eq!(liquid_kind(0x5, 4, 0), LiquidKind::Slime);
        assert_eq!(liquid_kind(0x5, 3, 0), LiquidKind::Magma);
        assert_eq!(liquid_kind(0x4, 13, 0), LiquidKind::Water);
        assert_eq!(liquid_kind(0x4, 20, 0), LiquidKind::Slime);
        // Без флага — по клеткам: Огненная пропасть (15, клетка 2) — лава, Стенающие
        // пещеры (15, клетка 4) — вода.
        assert_eq!(liquid_kind(0, 15, 2), LiquidKind::Magma);
        assert_eq!(liquid_kind(0, 15, 4), LiquidKind::Water);
    }

    #[test]
    fn yaw_of_quaternions() {
        // Без поворота — 0; на 90° вокруг Z — π/2.
        assert!(yaw([0.0, 0.0, 0.0, 1.0]).abs() < 1e-6);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((yaw([0.0, 0.0, h, h]) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }
}
