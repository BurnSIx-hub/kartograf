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
//!
//! В модуле Foundry файлы — в данных Foundry (`plat::fs`), а замок — отметка вкладки
//! браузера (`plat::hold`): закрытая вкладка её не обновляет, и копия становится ничьей.
//! Вкладку модуль не закрывает «по-хорошему», так что копия несохранённой карты ждёт
//! следующего открытия Картографа.

use crate::map::Map;
use crate::plat::fs;
use serde::{Deserialize, Serialize};
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

#[cfg(not(target_arch = "wasm32"))]
type Lock = std::fs::File;
#[cfg(target_arch = "wasm32")]
type Lock = ();

pub struct Recovery {
    dir: PathBuf,
    id: String,
    lock: Option<Lock>,
    saved_rev: u64,
    last: f64,
}

pub fn now_unix() -> u64 {
    crate::plat::SystemTime::now().duration_since(crate::plat::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Открывает файл так, чтобы никто другой не смог открыть его, пока он открыт у нас.
#[cfg(not(target_arch = "wasm32"))]
fn open_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
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
#[cfg(not(target_arch = "wasm32"))]
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// В модуле файл на сервере Foundry заменяется целиком.
#[cfg(target_arch = "wasm32")]
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::write(path, bytes)
}

/// Взять замок окна.
#[cfg(not(target_arch = "wasm32"))]
fn take_lock(path: &Path, _id: &str) -> Option<Lock> {
    open_exclusive(path).ok()
}

#[cfg(target_arch = "wasm32")]
fn take_lock(_path: &Path, id: &str) -> Option<Lock> {
    crate::plat::hold(id).then_some(())
}

/// Хозяин копии ещё работает: его замок не открыть.
#[cfg(not(target_arch = "wasm32"))]
fn owner_alive(path: &Path, _id: &str) -> bool {
    path.exists() && open_exclusive(path).is_err()
}

#[cfg(target_arch = "wasm32")]
fn owner_alive(_path: &Path, id: &str) -> bool {
    crate::plat::alive(id)
}

/// Отпустить свой замок.
#[cfg(not(target_arch = "wasm32"))]
fn release_lock(path: &Path, _id: &str) {
    let _ = std::fs::remove_file(path);
}

#[cfg(target_arch = "wasm32")]
fn release_lock(_path: &Path, id: &str) {
    crate::plat::release(id);
}

impl Recovery {
    pub fn new(data_dir: &Path) -> Self {
        let dir = data_dir.join("recovery");
        #[cfg(not(target_arch = "wasm32"))]
        let _ = std::fs::create_dir_all(&dir);
        // Счётчик — на случай двух копий в одном процессе за одну секунду.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("{}-{}-{n}", crate::plat::pid(), now_unix());
        let lock = take_lock(&dir.join(format!("{id}.lock")), &id);
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
        // Пустые пути — ничего не трогать: копии и не было (в модуле удаление — запись).
        if self.has_copy() {
            fs::remove(&self.file(&self.id, "karta"));
            fs::remove(&self.file(&self.id, "json"));
        }
        self.saved_rev = 0;
    }

    pub fn has_copy(&self) -> bool {
        fs::exists(&self.file(&self.id, "karta"))
    }

    /// Окно закрывается как положено: убрать и копию, и замок.
    pub fn close(&mut self) {
        self.clear();
        self.lock = None;
        release_lock(&self.file(&self.id, "lock"), &self.id);
    }

    /// Копии, хозяева которых уже не работают.
    pub fn orphans(&self) -> Vec<Orphan> {
        let mut out = Vec::new();
        for path in fs::list(&self.dir) {
            if path.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            if id == self.id {
                continue;
            }
            // Замок не открывается — хозяин жив. Открылся — сразу отпускаем.
            if owner_alive(&self.file(&id, "lock"), &id) {
                continue;
            }
            let meta = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok());
            if let Some(meta) = meta
                && fs::exists(&self.file(&id, "karta"))
            {
                out.push(Orphan { id, meta });
            }
        }
        out.sort_by_key(|o| std::cmp::Reverse(o.meta.saved));
        out
    }

    pub fn load(&self, orphan: &Orphan) -> Result<Map, String> {
        fs::read(&self.file(&orphan.id, "karta")).map_err(|e| e.to_string()).and_then(|b| String::from_utf8(b).map_err(|e| e.to_string())).and_then(|t| Map::from_json(&t))
    }

    pub fn discard(&self, orphan: &Orphan) {
        for ext in ["karta", "json"] {
            fs::remove(&self.file(&orphan.id, ext));
        }
        release_lock(&self.file(&orphan.id, "lock"), &orphan.id);
    }

    /// Для самопроверки: отпустить замок, как будто программа упала.
    pub fn drop_lock(&mut self) -> String {
        self.lock = None;
        #[cfg(target_arch = "wasm32")]
        crate::plat::release(&self.id);
        self.id.clone()
    }
}
