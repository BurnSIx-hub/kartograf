//! Архивы MPQ клиента WoW 0.x–3.x: чтение файлов по имени и списки `(listfile)`.
//!
//! В клиенте 3.3.5 всё нужное сжато zlib (замер по 33 архивам: 293 тыс. файлов — zlib,
//! 139 — bzip2, и те в служебных `base-`/`backup-` загрузчика), шифрованных нет.
//! Поэтому только zlib и несжатое; шифрование поддержано на всякий случай.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const FLAG_IMPLODE: u32 = 0x0000_0100;
const FLAG_COMPRESS: u32 = 0x0000_0200;
const FLAG_ENCRYPTED: u32 = 0x0001_0000;
const FLAG_FIX_KEY: u32 = 0x0002_0000;
const FLAG_SINGLE_UNIT: u32 = 0x0100_0000;
const FLAG_DELETE_MARKER: u32 = 0x0200_0000;
const FLAG_SECTOR_CRC: u32 = 0x0400_0000;
const FLAG_EXISTS: u32 = 0x8000_0000;

fn crypt_table() -> &'static [u32; 0x500] {
    static TABLE: OnceLock<[u32; 0x500]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 0x500];
        let mut seed: u32 = 0x0010_0001;
        for i in 0..0x100 {
            let mut idx = i;
            for _ in 0..5 {
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                let hi = (seed & 0xFFFF) << 16;
                seed = (seed * 125 + 3) % 0x2A_AAAB;
                t[idx] = hi | (seed & 0xFFFF);
                idx += 0x100;
            }
        }
        t
    })
}

/// Хеш имени: `kind` 0 — место в таблице, 1 и 2 — проверка, 3 — ключ шифра.
pub fn hash(name: &str, kind: u32) -> u32 {
    let t = crypt_table();
    let (mut s1, mut s2) = (0x7FED_7FEDu32, 0xEEEE_EEEEu32);
    for b in name.bytes() {
        let c = match b {
            b'/' => b'\\',
            b'a'..=b'z' => b - 32,
            _ => b,
        } as u32;
        s1 = t[(kind * 0x100 + c) as usize] ^ s1.wrapping_add(s2);
        s2 = c.wrapping_add(s1).wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
    }
    s1
}

fn decrypt(words: &mut [u32], mut key: u32) {
    let t = crypt_table();
    let mut s2 = 0xEEEE_EEEEu32;
    for w in words.iter_mut() {
        s2 = s2.wrapping_add(t[0x400 + (key & 0xFF) as usize]);
        let v = *w ^ key.wrapping_add(s2);
        *w = v;
        key = ((!key << 0x15).wrapping_add(0x1111_1111)) | (key >> 0x0B);
        s2 = v.wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
    }
}

fn decrypt_bytes(data: &mut [u8], key: u32) {
    let n = data.len() / 4;
    let mut words: Vec<u32> = (0..n).map(|i| u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap())).collect();
    decrypt(&mut words, key);
    for (i, w) in words.iter().enumerate() {
        data[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
}

#[derive(Clone, Copy)]
struct HashEntry {
    name1: u32,
    name2: u32,
    block: u32,
}

#[derive(Clone, Copy)]
struct Block {
    offset: u64,
    csize: u32,
    fsize: u32,
    flags: u32,
}

/// Один архив: таблицы прочитаны, файл открыт.
pub struct Archive {
    pub path: PathBuf,
    file: Mutex<File>,
    base: u64,
    sector: u32,
    hashes: Vec<HashEntry>,
    blocks: Vec<Block>,
}

fn read_u32s(f: &mut File, pos: u64, count: usize, len: u64) -> std::io::Result<Vec<u32>> {
    // Защищённые архивы пишут в заголовок размеры больше файла: читаем, сколько есть.
    let avail = len.saturating_sub(pos) / 4;
    let count = count.min(avail as usize);
    let mut buf = vec![0u8; count * 4];
    f.seek(SeekFrom::Start(pos))?;
    f.read_exact(&mut buf)?;
    Ok(buf.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self, String> {
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut f = File::open(path).map_err(err)?;
        let len = f.metadata().map_err(err)?.len();
        // Заголовок — с начала или на границе 512 байт; `MPQ\x1B` — блок пользователя со ссылкой.
        let mut base = 0u64;
        let mut hdr = [0u8; 32];
        loop {
            if base + 32 > len {
                return Err(format!("{}: не архив MPQ", path.display()));
            }
            f.seek(SeekFrom::Start(base)).map_err(err)?;
            f.read_exact(&mut hdr).map_err(err)?;
            match &hdr[..4] {
                b"MPQ\x1A" => break,
                b"MPQ\x1B" => base += u32::from_le_bytes(hdr[8..12].try_into().unwrap()) as u64,
                _ => base += 512,
            }
        }
        let u32_at = |o: usize| u32::from_le_bytes(hdr[o..o + 4].try_into().unwrap());
        let sector_shift = u16::from_le_bytes(hdr[14..16].try_into().unwrap()) as u32;
        let (hash_pos, block_pos, hash_n, block_n) = (u32_at(16) as u64, u32_at(20) as u64, u32_at(24) as usize, u32_at(28) as usize);
        let mut h = read_u32s(&mut f, base + hash_pos, hash_n * 4, len).map_err(err)?;
        decrypt(&mut h, hash("(hash table)", 3));
        let mut b = read_u32s(&mut f, base + block_pos, block_n * 4, len).map_err(err)?;
        decrypt(&mut b, hash("(block table)", 3));
        let hashes = h.chunks_exact(4).map(|c| HashEntry { name1: c[0], name2: c[1], block: c[3] }).collect();
        let blocks = b.chunks_exact(4).map(|c| Block { offset: c[0] as u64, csize: c[1], fsize: c[2], flags: c[3] }).collect();
        Ok(Self { path: path.to_owned(), file: Mutex::new(f), base, sector: 512u32 << sector_shift.min(16), hashes, blocks })
    }

    fn find(&self, name: &str) -> Option<Block> {
        let n = self.hashes.len();
        if n == 0 {
            return None;
        }
        let (start, a, b) = (hash(name, 0) as usize, hash(name, 1), hash(name, 2));
        // Размер таблицы — степень двойки; у защищённых может не быть: тогда остаток.
        let mut i = if n.is_power_of_two() { start & (n - 1) } else { start % n };
        for _ in 0..n {
            let e = self.hashes[i];
            if e.block == 0xFFFF_FFFF {
                return None;
            }
            if e.name1 == a && e.name2 == b && e.block != 0xFFFF_FFFE {
                let blk = *self.blocks.get(e.block as usize)?;
                return (blk.flags & FLAG_EXISTS != 0 && blk.flags & FLAG_DELETE_MARKER == 0).then_some(blk);
            }
            i = (i + 1) % n;
        }
        None
    }

    /// Файл целиком; `None` — нет в архиве, `Err` — есть, но не читается.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        let blk = self.find(name)?;
        Some(self.read_block(name, blk))
    }

    fn read_block(&self, name: &str, blk: Block) -> Result<Vec<u8>, String> {
        let what = |e: String| format!("{name} в {}: {e}", self.path.file_name().unwrap_or_default().to_string_lossy());
        if blk.flags & FLAG_IMPLODE != 0 {
            return Err(what("сжатие PKWARE не поддержано".into()));
        }
        let mut raw = vec![0u8; blk.csize as usize];
        {
            let mut f = self.file.lock().map_err(|_| what("файл занят".into()))?;
            f.seek(SeekFrom::Start(self.base + blk.offset)).map_err(|e| what(e.to_string()))?;
            f.read_exact(&mut raw).map_err(|e| what(e.to_string()))?;
        }
        let fsize = blk.fsize as usize;
        let key = (blk.flags & FLAG_ENCRYPTED != 0).then(|| {
            let base_name = name.rsplit(['\\', '/']).next().unwrap_or(name);
            let k = hash(base_name, 3);
            if blk.flags & FLAG_FIX_KEY != 0 { k.wrapping_add(blk.offset as u32) ^ blk.fsize } else { k }
        });
        if blk.flags & FLAG_SINGLE_UNIT != 0 {
            if let Some(k) = key {
                decrypt_bytes(&mut raw, k);
            }
            return if blk.flags & FLAG_COMPRESS != 0 && raw.len() < fsize { decompress(&raw, fsize).map_err(what) } else { Ok(raw) };
        }
        let sectors = fsize.div_ceil(self.sector as usize);
        if blk.flags & FLAG_COMPRESS == 0 {
            let mut out = raw;
            if let Some(k) = key {
                for (i, chunk) in out.chunks_mut(self.sector as usize).enumerate() {
                    decrypt_bytes(chunk, k.wrapping_add(i as u32));
                }
            }
            out.truncate(fsize);
            return Ok(out);
        }
        let entries = sectors + 1 + usize::from(blk.flags & FLAG_SECTOR_CRC != 0);
        if raw.len() < entries * 4 {
            return Err(what("короткая таблица секторов".into()));
        }
        let mut table: Vec<u32> = raw[..entries * 4].chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        if let Some(k) = key {
            decrypt(&mut table, k.wrapping_sub(1));
        }
        let mut out = Vec::with_capacity(fsize);
        for i in 0..sectors {
            let (a, b) = (table[i] as usize, table[i + 1] as usize);
            if a > b || b > raw.len() {
                return Err(what(format!("сектор {i} за краем")));
            }
            let mut s = raw[a..b].to_vec();
            if let Some(k) = key {
                decrypt_bytes(&mut s, k.wrapping_add(i as u32));
            }
            let want = (fsize - i * self.sector as usize).min(self.sector as usize);
            if s.len() < want {
                out.extend(decompress(&s, want).map_err(what)?);
            } else {
                out.extend_from_slice(&s[..want]);
            }
        }
        Ok(out)
    }

    /// Имена из `(listfile)` архива — как записаны (с регистром).
    pub fn listfile(&self) -> Vec<String> {
        let Some(Ok(bytes)) = self.read("(listfile)") else { return Vec::new() };
        String::from_utf8_lossy(&bytes).split(['\r', '\n', ';']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
    }
}

fn decompress(data: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let (&mask, rest) = data.split_first().ok_or("пустой сектор")?;
    match mask {
        0x02 => {
            let mut out = Vec::with_capacity(size);
            flate2::read::ZlibDecoder::new(rest).read_to_end(&mut out).map_err(|e| format!("zlib: {e}"))?;
            Ok(out)
        }
        m => Err(format!("сжатие 0x{m:02x} не поддержано")),
    }
}

/// Сколько переходов «строчная → заглавная»: чем больше, тем лучше видны слова.
fn humps(s: &str) -> usize {
    s.as_bytes().windows(2).filter(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase()).count()
}

/// Клиент: архивы в порядке игры — первым тот, что главнее.
pub struct Client {
    pub dir: PathBuf,
    pub locale: String,
    pub archives: Vec<Archive>,
    names: OnceLock<Vec<String>>,
    spelling: OnceLock<std::collections::HashMap<String, String>>,
}

/// Место архива в порядке загрузки игры (больше — главнее) или `None`, если архив не грузится.
/// common → common-2 → expansion → lichking → языковые основы → patch, patch-2…9, patch-a…z →
/// patch-<язык>, patch-<язык>-2…9, -a…z. Отключённые (`.mpq.disabled`) сюда не попадают.
pub fn rank(file: &str, locale: &str) -> Option<(u8, u32)> {
    let n = file.to_lowercase();
    let n = n.strip_suffix(".mpq")?;
    let suffix = |s: &str| -> Option<u32> {
        match s {
            "" => Some(0),
            s if s.chars().all(|c| c.is_ascii_digit()) => s.parse().ok(),
            s if s.len() == 1 => Some(100 + s.as_bytes()[0] as u32),
            _ => None,
        }
    };
    const BASE: [&str; 4] = ["common", "common-2", "expansion", "lichking"];
    if let Some(i) = BASE.iter().position(|b| *b == n) {
        return Some((0, i as u32));
    }
    let loc = locale.to_lowercase();
    let local = n.replace(&loc, "L");
    const LOCAL: [&str; 8] = ["backup-L", "base-L", "locale-L", "speech-L", "expansion-locale-L", "expansion-speech-L", "lichking-locale-L", "lichking-speech-L"];
    if let Some(i) = LOCAL.iter().position(|b| *b == local) {
        return Some((1, i as u32));
    }
    if let Some(rest) = local.strip_prefix("patch-L") {
        return Some((3, suffix(rest.strip_prefix('-').unwrap_or(rest))?));
    }
    if let Some(rest) = n.strip_prefix("patch") {
        return Some((2, suffix(rest.strip_prefix('-').unwrap_or(rest))?));
    }
    None
}

impl Client {
    /// Папка клиента (где `Data` или `data`) или сама папка `Data`.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let child = |d: &Path, name: &str| -> Option<PathBuf> {
            std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(name)))
        };
        let data = child(dir, "data").unwrap_or_else(|| dir.to_owned());
        let root = if data == dir { dir.parent().map(Path::to_owned).unwrap_or_default() } else { dir.to_owned() };
        // Язык клиента — из WTF/Config.wtf; иначе первая папка языка в Data.
        let config = child(&root, "wtf").and_then(|w| std::fs::read_to_string(w.join("Config.wtf")).ok()).unwrap_or_default();
        let from_config = config.lines().find_map(|l| l.trim().strip_prefix("SET locale \"").map(|r| r.trim_end_matches('"').to_owned()));
        let locale = from_config
            .or_else(|| {
                std::fs::read_dir(&data).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.len() == 4 && n.chars().all(|c| c.is_ascii_alphabetic()) && data.join(n).is_dir())
            })
            .unwrap_or_else(|| "enUS".into());
        let mut ranked: Vec<((u8, u32), PathBuf)> = Vec::new();
        let mut scan = |d: &Path| {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if let Some(r) = p.file_name().and_then(|f| rank(&f.to_string_lossy(), &locale)) {
                    ranked.push((r, p));
                }
            }
        };
        scan(&data);
        if let Some(loc) = child(&data, &locale) {
            scan(&loc);
        }
        if ranked.is_empty() {
            return Err(format!("{}: нет архивов MPQ", data.display()));
        }
        ranked.sort();
        ranked.reverse();
        let archives = ranked.into_iter().map(|(_, p)| Archive::open(&p)).collect::<Result<Vec<_>, _>>()?;
        Ok(Self { dir: root, locale, archives, names: OnceLock::new(), spelling: OnceLock::new() })
    }

    /// Файл из самого главного архива, где он есть.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, String> {
        for a in &self.archives {
            if let Some(r) = a.read(name) {
                return r;
            }
        }
        Err(format!("нет в архивах: {name}"))
    }

    /// Все имена из списков архивов, без повторов, по алфавиту. Написание — то, где у имени
    /// файла больше «горбов» (`StatueFountain`, а не `statuefountain` из HD-патча): по ним
    /// имя делится на слова.
    pub fn names(&self) -> &[String] {
        self.names.get_or_init(|| {
            use std::collections::HashMap;
            // Папка и имя файла выбираются порознь: в одном архиве папки капсом, в другом файл строчными.
            fn keep(map: &mut HashMap<String, String>, s: &str, score: fn(&str) -> (usize, bool)) {
                let e = map.entry(s.to_lowercase()).or_insert_with(|| s.to_owned());
                if score(s) > score(e) {
                    *e = s.to_owned();
                }
            }
            let (mut dirs, mut files) = (HashMap::new(), HashMap::new());
            for a in &self.archives {
                for n in a.listfile() {
                    let d = n.rsplit_once('\\').map_or("", |(d, _)| d);
                    keep(&mut dirs, d, |s| (humps(s), s.bytes().any(|b| b.is_ascii_lowercase())));
                    keep(&mut files, &n, |s| (humps(s.rsplit('\\').next().unwrap_or(s)), false));
                }
            }
            let mut out: Vec<String> = files
                .into_values()
                .map(|n| match n.rsplit_once('\\') {
                    Some((d, f)) => format!("{}\\{f}", dirs.get(&d.to_lowercase()).map_or(d, String::as_str)),
                    None => n,
                })
                .collect();
            out.sort_by_key(|s| s.to_lowercase());
            out
        })
    }

    /// Имя, как оно лучше всего записано в списках архивов (`GENERALCANDELABRA01.M2` из
    /// расстановки WMO → `…\GeneralCandelabra01.m2`); нет в списках — как есть.
    pub fn spelled(&self, name: &str) -> String {
        let map = self.spelling.get_or_init(|| self.names().iter().map(|n| (n.to_lowercase().replace('/', "\\"), n.clone())).collect());
        map.get(&name.to_lowercase().replace('/', "\\")).cloned().unwrap_or_else(|| name.to_owned())
    }

    /// Имена с окончанием `ext` (`.m2`).
    pub fn list(&self, ext: &str) -> Vec<&str> {
        let ext = ext.to_lowercase();
        self.names().iter().filter(|n| n.to_lowercase().ends_with(&ext)).map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_the_known_table_keys() {
        // Ключи таблиц из описания формата (StormLib): по ним шифруются таблицы.
        assert_eq!(hash("(hash table)", 3), 0xC3AF_3770);
        assert_eq!(hash("(block table)", 3), 0xEC83_B3A3);
        // Регистр и косая не важны.
        assert_eq!(hash("world/a.m2", 0), hash("WORLD\\A.M2", 0));
    }

    #[test]
    fn archives_load_in_the_games_order() {
        let mut names = vec!["patch.mpq", "patch-ruRU-3.MPQ", "common.MPQ", "patch-a.mpq", "patch-3.MPQ", "patch-ruRU.MPQ", "locale-ruRU.MPQ", "lichking.MPQ", "patch-2.MPQ", "patch-ruRU-a.mpq"];
        names.sort_by_key(|n| rank(n, "ruRU").unwrap());
        assert_eq!(names, ["common.MPQ", "lichking.MPQ", "locale-ruRU.MPQ", "patch.mpq", "patch-2.MPQ", "patch-3.MPQ", "patch-a.mpq", "patch-ruRU.MPQ", "patch-ruRU-3.MPQ", "patch-ruRU-a.mpq"]);
        assert_eq!(rank("patch-5.mpq.disabled", "ruRU"), None);
        assert_eq!(rank("wow-update-12340.MPQ", "ruRU"), None);
    }

    #[test]
    fn encryption_round_trips() {
        let t = crypt_table();
        let key = 0x1234_5678u32;
        let plain = [1u32, 2, 3, 0xDEAD_BEEF];
        // Зашифровать по описанию формата, расшифровать нашей функцией.
        let mut enc = plain;
        let (mut k, mut s2) = (key, 0xEEEE_EEEEu32);
        for w in enc.iter_mut() {
            s2 = s2.wrapping_add(t[0x400 + (k & 0xFF) as usize]);
            let v = *w;
            *w = v ^ k.wrapping_add(s2);
            k = ((!k << 0x15).wrapping_add(0x1111_1111)) | (k >> 0x0B);
            s2 = v.wrapping_add(s2).wrapping_add(s2 << 5).wrapping_add(3);
        }
        decrypt(&mut enc, key);
        assert_eq!(enc, plain);
    }
}
