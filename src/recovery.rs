//! Автосохранение: копия несохранённой карты на случай, если программа не доживёт
//! до «Сохранить». Устроено как в Borderless Docs.
//!
//! Копия пишется не поверх файла пользователя, а в папку `recovery` рядом с настройками:
//! сам файл карты меняет только явное сохранение. Копия исчезает, когда карта сохранена
//! или закрыта по-хорошему; если программа упала, копия остаётся, и при следующем запуске
//! редактор предлагает её вернуть.
//!
//! У каждого окна свой файл-замок, который окно держит открытым. Замок, который удаётся
//! открыть, остался от упавшей программы, — значит, и копия при нём ничья.

use crate::map::Map;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};

/// Не чаще чем раз в столько секунд.
pub const INTERVAL: f64 = 15.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    /// Имя карты, как оно стояло в заголовке окна.
    pub name: String,
    /// Файл, из которого карту открыли; `None` — её ещё не сохраняли.
    pub path: Option<PathBuf>,
    /// Когда сделана копия: секунды от начала 1970 года.
    pub saved: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Orphan {
    pub id: String,
    pub meta: Meta,
}

pub struct Recovery {
    dir: PathBuf,
    id: String,
    lock: Option<File>,
    saved_rev: u64,
    last: f64,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Открывает файл так, чтобы никто другой не смог открыть его, пока он открыт у нас.
fn open_exclusive(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options.open(path)
}

/// Запись через временный файл: оборванная запись не портит прежнюю копию.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

impl Recovery {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("recovery");
        let _ = std::fs::create_dir_all(&dir);
        // Счётчик — на случай двух копий в одном процессе за одну секунду.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{}-{}-{n}", std::process::id(), now_unix());
        let lock = open_exclusive(&dir.join(format!("{id}.lock"))).ok();
        Self { dir, id, lock, saved_rev: 0, last: f64::NEG_INFINITY }
    }

    fn file(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Пора ли делать копию: карта изменена, с прошлой копии прошло время и правки новые.
    pub fn due(&self, now: f64, dirty: bool, rev: u64) -> bool {
        dirty && rev != self.saved_rev && now - self.last >= INTERVAL && self.lock.is_some()
    }

    pub fn snapshot(&mut self, now: f64, rev: u64, map: &Map, meta: &Meta) {
        self.last = now;
        let meta_json = serde_json::to_vec_pretty(meta).unwrap_or_default();
        if write_whole(&self.file(&self.id, "karta"), map.to_json().as_bytes()).is_ok() && write_whole(&self.file(&self.id, "json"), &meta_json).is_ok() {
            self.saved_rev = rev;
        }
    }

    /// Карта сохранена или от правок отказались: копия больше не нужна.
    pub fn clear(&mut self) {
        let _ = std::fs::remove_file(self.file(&self.id, "karta"));
        let _ = std::fs::remove_file(self.file(&self.id, "json"));
        self.saved_rev = 0;
    }

    pub fn has_copy(&self) -> bool {
        self.file(&self.id, "karta").exists()
    }

    /// Окно закрывается как положено: убрать и копию, и замок.
    pub fn close(&mut self) {
        self.clear();
        self.lock = None;
        let _ = std::fs::remove_file(self.file(&self.id, "lock"));
    }

    /// Копии, хозяева которых уже не работают.
    pub fn orphans(&self) -> Vec<Orphan> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if id == self.id {
                continue;
            }
            // Замок не открывается — хозяин жив. Открылся — сразу отпускаем.
            let lock = self.file(&id, "lock");
            if lock.exists() && open_exclusive(&lock).is_err() {
                continue;
            }
            let meta = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok());
            if let Some(meta) = meta
                && self.file(&id, "karta").exists()
            {
                out.push(Orphan { id, meta });
            }
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.meta.saved));
        out
    }

    pub fn load(&self, orphan: &Orphan) -> Result<Map, String> {
        std::fs::read_to_string(self.file(&orphan.id, "karta")).map_err(|e| e.to_string()).and_then(|t| Map::from_json(&t))
    }

    pub fn discard(&self, orphan: &Orphan) {
        for ext in ["karta", "json", "lock"] {
            let _ = std::fs::remove_file(self.file(&orphan.id, ext));
        }
    }

    /// Для самопроверки: отпустить замок, как будто программа упала.
    pub fn drop_lock(&mut self) -> String {
        self.lock = None;
        self.id.clone()
    }
}
