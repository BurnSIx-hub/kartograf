//! Чтение паков Dungeondraft (`.dungeondraft_pack`). Это архив Godot 3 (PCK, магия `GDPC`):
//! заголовок, оглавление «путь — смещение — размер» и сами файлы подряд, без сжатия.
//! Читается только оглавление; картинка достаётся по смещению, когда её попросят, — пак
//! в несколько гигабайт не грузится в память целиком.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Путь внутри пака без `res://packs/<id>/`, например `textures/objects/Trees/Oak_2x2.webp`.
    pub path: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub file: PathBuf,
    pub name: String,
    pub entries: Vec<Entry>,
}

fn u32le(r: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn u64le(r: &mut impl Read) -> std::io::Result<u64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}

/// Оглавление пака: только картинки из `textures/`.
pub fn read_index(file: &Path) -> Result<Pack, String> {
    let err = |e: std::io::Error| format!("{}: {e}", file.display());
    let mut r = BufReader::with_capacity(1 << 20, File::open(file).map_err(err)?);
    let mut magic = [0; 4];
    r.read_exact(&mut magic).map_err(err)?;
    if &magic != b"GDPC" {
        return Err(format!("{}: это не пак Dungeondraft", file.display()));
    }
    let version = u32le(&mut r).map_err(err)?;
    if version != 1 {
        return Err(format!("{}: неизвестная версия пака {version}", file.display()));
    }
    // Версия движка (3 числа) и 16 запасных слов.
    let mut skip = [0; 12 + 64];
    r.read_exact(&mut skip).map_err(err)?;
    let count = u32le(&mut r).map_err(err)?;
    let mut entries = Vec::new();
    let mut name = String::new();
    for _ in 0..count {
        let len = u32le(&mut r).map_err(err)? as usize;
        if len > 4096 {
            return Err(format!("{}: оглавление повреждено", file.display()));
        }
        let mut raw = vec![0; len];
        r.read_exact(&mut raw).map_err(err)?;
        let offset = u64le(&mut r).map_err(err)?;
        let size = u64le(&mut r).map_err(err)?;
        let mut md5 = [0; 16];
        r.read_exact(&mut md5).map_err(err)?;
        let full = String::from_utf8_lossy(&raw).trim_end_matches('\0').to_owned();
        // res://packs/<id>/textures/...
        let Some(rest) = full.strip_prefix("res://packs/") else { continue };
        let Some((id, inner)) = rest.split_once('/') else { continue };
        if name.is_empty() {
            name = id.to_owned();
        }
        let lower = inner.to_ascii_lowercase();
        if inner.starts_with("textures/") && [".png", ".webp", ".jpg", ".jpeg"].iter().any(|e| lower.ends_with(e)) {
            entries.push(Entry { path: inner.to_owned(), offset, size });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(stem) = file.file_stem() {
        name = stem.to_string_lossy().into_owned();
    }
    Ok(Pack { file: file.to_owned(), name, entries })
}

pub fn read_bytes(file: &Path, offset: u64, size: u64) -> Result<Vec<u8>, String> {
    let mut f = File::open(file).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut buf = vec![0; size as usize];
    f.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Пишет маленький пак того же устройства — для тестов и самопроверки.
pub fn write_test_pack(file: &Path, id: &str, files: &[(&str, Vec<u8>)]) {
    // Тот же порядок полей, что у Godot: заголовок, оглавление, данные.
    let mut head = Vec::new();
    head.extend(b"GDPC");
    for v in [1u32, 3, 4, 2] {
        head.extend(v.to_le_bytes());
    }
    head.extend([0; 64]);
    head.extend((files.len() as u32).to_le_bytes());
    let index_len: usize = files.iter().map(|(p, _)| 4 + format!("res://packs/{id}/{p}").len() + 8 + 8 + 16).sum();
    let mut offset = (head.len() + index_len) as u64;
    let mut data: Vec<u8> = Vec::new();
    for (p, bytes) in files {
        let path = format!("res://packs/{id}/{p}");
        head.extend((path.len() as u32).to_le_bytes());
        head.extend(path.as_bytes());
        head.extend(offset.to_le_bytes());
        head.extend((bytes.len() as u64).to_le_bytes());
        head.extend([0; 16]);
        offset += bytes.len() as u64;
        data.extend(bytes);
    }
    head.extend(data);
    std::fs::write(file, head).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_and_bytes() {
        let dir = std::env::temp_dir().join(format!("kartograf-pck-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("Test.dungeondraft_pack");
        write_test_pack(&file, "AB12", &[("pack.json", b"{}".to_vec()), ("textures/objects/Tree_2x2.png", vec![1, 2, 3]), ("textures/walls/Brick.webp", vec![9; 5])]);
        let pack = read_index(&file).unwrap();
        assert_eq!(pack.name, "Test");
        assert_eq!(pack.entries.len(), 2, "pack.json — не картинка");
        let tree = pack.entries.iter().find(|e| e.path.ends_with("Tree_2x2.png")).unwrap();
        assert_eq!(read_bytes(&file, tree.offset, tree.size).unwrap(), vec![1, 2, 3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn not_a_pack() {
        let file = std::env::temp_dir().join(format!("kartograf-notpack-{}.bin", std::process::id()));
        std::fs::write(&file, b"PK\x03\x04 zip").unwrap();
        assert!(read_index(&file).is_err());
        let _ = std::fs::remove_file(&file);
    }
}
