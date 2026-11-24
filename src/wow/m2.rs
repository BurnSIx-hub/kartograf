//! Модели M2 версий 256–264 (WoW 1.x–3.x): вершины, текстуры, материалы, сетка вида 0.
//! У 3.x (264) сетка — во внешнем `<модель>00.skin`, у старших — внутри файла.

pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Texture {
    /// 0 — файл по имени; прочие — подменяемые (кожа существа, вариант предмета).
    pub kind: u32,
    /// 1 — повтор по U, 2 — по V.
    pub flags: u32,
    pub file: String,
}

pub struct Material {
    pub flags: u16,
    /// 0 — сплошной, 1 — с вырезом по альфе, 2 — полупрозрачный, 3+ — свечение и наложения.
    pub blend: u16,
}

/// Кусок сетки с одной текстурой: треугольники — индексы вершин модели.
pub struct Batch {
    pub triangles: Vec<[u32; 3]>,
    pub texture: Option<usize>,
    pub material: usize,
}

pub struct Model {
    pub vertices: Vec<Vertex>,
    pub textures: Vec<Texture>,
    pub materials: Vec<Material>,
    pub batches: Vec<Batch>,
}

struct R<'a> {
    b: &'a [u8],
    at: usize,
}

impl R<'_> {
    fn u32(&mut self) -> Result<u32, String> {
        let v = self.b.get(self.at..self.at + 4).ok_or("M2: короткий файл")?;
        self.at += 4;
        Ok(u32::from_le_bytes(v.try_into().unwrap()))
    }
    fn arr(&mut self) -> Result<(usize, usize), String> {
        Ok((self.u32()? as usize, self.u32()? as usize))
    }
    fn skip(&mut self, n: usize) {
        self.at += n;
    }
}

fn slice(b: &[u8], (count, ofs): (usize, usize), size: usize) -> Result<&[u8], String> {
    b.get(ofs..ofs + count.checked_mul(size).ok_or("M2: переполнение")?).ok_or_else(|| "M2: массив за краем файла".to_owned())
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn string(b: &[u8], (count, ofs): (usize, usize)) -> String {
    b.get(ofs..ofs + count).map(|s| String::from_utf8_lossy(s).trim_end_matches('\0').to_owned()).unwrap_or_default()
}

/// Разобрать модель; `skin` — содержимое `<модель>00.skin` для версии 264 (у старых не нужно).
pub fn parse(b: &[u8], skin: Option<&[u8]>) -> Result<Model, String> {
    if b.get(..4) != Some(b"MD20") {
        return Err("не M2 (нет MD20)".into());
    }
    let mut r = R { b, at: 4 };
    let version = r.u32()?;
    if !(256..=264).contains(&version) {
        return Err(format!("M2 версии {version} не поддержана (нужны 256–264)"));
    }
    r.skip(8); // имя
    r.skip(4); // флаги
    r.skip(8 * 3); // петли, анимации, поиск анимаций
    if version <= 257 {
        r.skip(8); // проигрываемые анимации
    }
    r.skip(8 * 2); // кости, поиск ключевых костей
    let verts = r.arr()?;
    let views = if version < 264 { Some(r.arr()?) } else {
        r.skip(4);
        None
    };
    r.skip(8); // цвета
    let textures = r.arr()?;
    r.skip(8); // веса текстур
    if version <= 257 {
        r.skip(8);
    }
    r.skip(8); // сдвиги текстур
    r.skip(8); // поиск подменяемых
    let materials = r.arr()?;
    r.skip(8); // поиск костей
    let tex_lookup = r.arr()?;

    let vb = slice(b, verts, 48)?;
    let vertices = vb
        .chunks_exact(48)
        .map(|v| Vertex { pos: [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)], normal: [f32_at(v, 20), f32_at(v, 24), f32_at(v, 28)], uv: [f32_at(v, 32), f32_at(v, 36)] })
        .collect();
    let textures = slice(b, textures, 16)?
        .chunks_exact(16)
        .map(|t| Texture { kind: u32_at(t, 0), flags: u32_at(t, 4), file: string(b, (u32_at(t, 8) as usize, u32_at(t, 12) as usize)) })
        .collect();
    let materials = slice(b, materials, 4)?.chunks_exact(4).map(|m| Material { flags: u16_at(m, 0), blend: u16_at(m, 2) }).collect();
    let tex_lookup: Vec<u16> = slice(b, tex_lookup, 2)?.chunks_exact(2).map(|c| u16_at(c, 0)).collect();

    // Вид 0: у старых — в файле, у 264 — в .skin (смещения от начала .skin, после «SKIN»).
    let (sb, view_at) = match views {
        Some(v) => {
            if v.0 == 0 {
                return Err("M2: нет видов сетки".into());
            }
            (b, v.1)
        }
        None => {
            let s = skin.ok_or("нужен файл .skin")?;
            if s.get(..4) != Some(b"SKIN") {
                return Err("не .skin (нет SKIN)".into());
            }
            (s, 4)
        }
    };
    let mut vr = R { b: sb, at: view_at };
    let (index_arr, tri_arr) = (vr.arr()?, vr.arr()?);
    vr.skip(8); // кости вершин
    let (sub_arr, batch_arr) = (vr.arr()?, vr.arr()?);
    let index: Vec<u32> = slice(sb, index_arr, 2)?.chunks_exact(2).map(|c| u16_at(c, 0) as u32).collect();
    let tris: Vec<u32> = slice(sb, tri_arr, 2)?.chunks_exact(2).map(|c| u16_at(c, 0) as u32).collect();
    let sub_size = if version >= 260 { 48 } else { 32 };
    // Начало треугольников подсетки: младшие 16 бит и «уровень» — старшие (сетки больше 65 тыс.).
    let subs: Vec<(usize, usize)> = slice(sb, sub_arr, sub_size)?
        .chunks_exact(sub_size)
        .map(|s| (u16_at(s, 8) as usize + ((u16_at(s, 2) as usize) << 16), u16_at(s, 10) as usize))
        .collect();
    let mut batches = Vec::new();
    for bt in slice(sb, batch_arr, 24)?.chunks_exact(24) {
        let sub = u16_at(bt, 4) as usize;
        let (material, tex_combo) = (u16_at(bt, 10) as usize, u16_at(bt, 16) as usize);
        let Some(&(start, count)) = subs.get(sub) else { continue };
        let tri_idx = tris.get(start..start + count).ok_or("M2: треугольники подсетки за краем")?;
        let triangles = tri_idx
            .chunks_exact(3)
            .filter_map(|t| Some([*index.get(t[0] as usize)?, *index.get(t[1] as usize)?, *index.get(t[2] as usize)?]))
            .collect();
        let texture = tex_lookup.get(tex_combo).map(|&t| t as usize);
        batches.push(Batch { triangles, texture, material });
    }
    Ok(Model { vertices, textures, materials, batches })
}

/// Путь `.skin` вида 0 для модели 3.x: `Path\Name.m2` → `Path\Name00.skin`.
pub fn skin_path(model: &str) -> String {
    let stem = model.rsplit_once('.').map_or(model, |(s, _)| s);
    format!("{stem}00.skin")
}

/// Имя файла модели внутри клиента: в WMO и ADT предметы записаны как `.mdx`/`.mdl`.
pub fn m2_path(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.ends_with(".mdx") || lower.ends_with(".mdl") {
        format!("{}.m2", &name[..name.len() - 4])
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn skin_and_m2_paths() {
        assert_eq!(skin_path("World\\A\\Statue.m2"), "World\\A\\Statue00.skin");
        assert_eq!(m2_path("World\\A\\Statue.MDX"), "World\\A\\Statue.m2");
        assert_eq!(m2_path("World\\A\\Statue.m2"), "World\\A\\Statue.m2");
    }

    /// Самая маленькая модель 3.x: один треугольник, одна текстура, свой .skin.
    pub fn tiny_model() -> (Vec<u8>, Vec<u8>) {
        let mut m = vec![0u8; 0x130];
        m[..4].copy_from_slice(b"MD20");
        m[4..8].copy_from_slice(&264u32.to_le_bytes());
        let put = |m: &mut Vec<u8>, at: usize, v: u32| m[at..at + 4].copy_from_slice(&v.to_le_bytes());
        // Вершины: три штуки по 48 байт.
        let verts_at = m.len();
        for (x, y) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0)] {
            let mut v = vec![0u8; 48];
            v[0..4].copy_from_slice(&x.to_le_bytes());
            v[4..8].copy_from_slice(&y.to_le_bytes());
            v[28..32].copy_from_slice(&1.0f32.to_le_bytes());
            m.extend(v);
        }
        put(&mut m, 0x3C, 3);
        put(&mut m, 0x40, verts_at as u32);
        put(&mut m, 0x44, 1);
        let name_at = m.len();
        m.extend(b"Tex.blp\0");
        let tex_at = m.len();
        m.extend(0u32.to_le_bytes());
        m.extend(3u32.to_le_bytes());
        m.extend(8u32.to_le_bytes());
        m.extend((name_at as u32).to_le_bytes());
        put(&mut m, 0x50, 1);
        put(&mut m, 0x54, tex_at as u32);
        let mat_at = m.len();
        m.extend([0u8, 0, 1, 0]);
        put(&mut m, 0x70, 1);
        put(&mut m, 0x74, mat_at as u32);
        let look_at = m.len();
        m.extend([0u8, 0]);
        put(&mut m, 0x80, 1);
        put(&mut m, 0x84, look_at as u32);

        let mut s = vec![0u8; 4 + 44];
        s[..4].copy_from_slice(b"SKIN");
        let sput = |s: &mut Vec<u8>, at: usize, v: u32| s[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let idx_at = s.len();
        s.extend([0u8, 0, 1, 0, 2, 0]);
        sput(&mut s, 4, 3);
        sput(&mut s, 8, idx_at as u32);
        let tri_at = s.len();
        s.extend([0u8, 0, 1, 0, 2, 0]);
        sput(&mut s, 12, 3);
        sput(&mut s, 16, tri_at as u32);
        let sub_at = s.len();
        let mut sub = vec![0u8; 48];
        sub[10..12].copy_from_slice(&3u16.to_le_bytes());
        s.extend(sub);
        sput(&mut s, 28, 1);
        sput(&mut s, 32, sub_at as u32);
        let batch_at = s.len();
        s.extend(vec![0u8; 24]);
        sput(&mut s, 36, 1);
        sput(&mut s, 40, batch_at as u32);
        (m, s)
    }

    #[test]
    fn parses_a_tiny_wotlk_model_with_its_skin() {
        let (m, s) = tiny_model();
        let model = parse(&m, Some(&s)).unwrap();
        assert_eq!(model.vertices.len(), 3);
        assert_eq!(model.vertices[1].pos, [1.0, 0.0, 0.0]);
        assert_eq!(model.textures[0].file, "Tex.blp");
        assert_eq!(model.textures[0].flags, 3);
        assert_eq!(model.materials[0].blend, 1);
        assert_eq!(model.batches.len(), 1);
        assert_eq!(model.batches[0].triangles, vec![[0, 1, 2]]);
        assert_eq!(model.batches[0].texture, Some(0));
        assert!(parse(&m, None).is_err());
    }
}
