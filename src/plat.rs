//! То, чем программа и модуль Foundry отличаются: окна выбора файлов, потоки, часы.
//! Программа (exe) берёт настоящее; сборка в WebAssembly — заглушки и то, что умеет браузер.

#[cfg(not(target_arch = "wasm32"))]
pub use rfd::FileDialog;
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Номер процесса — метка автосейва этого запуска. В браузере процессов нет: время в мкс.
pub fn pid() -> u32 {
    #[cfg(not(target_arch = "wasm32"))]
    return std::process::id();
    #[cfg(target_arch = "wasm32")]
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_micros())
}

/// Имя сцены, которую модуль создал или обновил последней, если она ещё есть.
#[cfg(target_arch = "wasm32")]
pub fn last_scene() -> Option<String> {
    web::call("kartografLastScene", &[]).ok()?.as_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn last_scene() -> Option<String> {
    None
}

/// Отдать выгрузку модулю Foundry: файлы — в его данные, `.json` — новой сценой
/// или поверх последней (`update`). Делает это `window.kartografToScene`
/// из `foundry/scripts/main.js`.
#[cfg(target_arch = "wasm32")]
pub fn to_scene(files: &[(std::path::PathBuf, Vec<u8>)], update: bool) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("нет окна")?;
    let f = js_sys::Reflect::get(&window, &"kartografToScene".into()).map_err(|_| "нет kartografToScene")?;
    let f: js_sys::Function = f.dyn_into().map_err(|_| "kartografToScene — не функция")?;
    let list = js_sys::Array::new();
    for (path, bytes) in files {
        let item = js_sys::Object::new();
        let path = path.to_string_lossy().replace('\\', "/");
        let _ = js_sys::Reflect::set(&item, &"path".into(), &path.into());
        let _ = js_sys::Reflect::set(&item, &"bytes".into(), &js_sys::Uint8Array::from(bytes.as_slice()));
        list.push(&item);
    }
    f.call2(&wasm_bindgen::JsValue::NULL, &list, &update.into()).map(|_| ()).map_err(|e| format!("{e:?}"))
}

/// Работа в фоне: в программе — свой поток, в браузере потоков нет, и она идёт сразу.
pub fn spawn<F: FnOnce() + Send + 'static>(f: F) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(f);
    #[cfg(target_arch = "wasm32")]
    f();
}

/// Открыть файл на чтение. В браузере — файл, выбранный в окне браузера
/// (`kartografPick` в `foundry/scripts/main.js`); читается кусками, целиком в память
/// не попадает: паки FA весят по 4–5 ГБ.
#[cfg(not(target_arch = "wasm32"))]
pub fn open(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

#[cfg(target_arch = "wasm32")]
pub fn open(path: &std::path::Path) -> std::io::Result<web::RangeFile> {
    web::RangeFile::open(path)
}

/// Что выбрали в окне браузера с прошлого кадра.
#[cfg(target_arch = "wasm32")]
pub fn take_picked() -> Vec<std::path::PathBuf> {
    let Ok(v) = web::call("kartografTakePicked", &[]) else { return Vec::new() };
    js_sys::Array::from(&v).iter().filter_map(|x| x.as_string()).map(std::path::PathBuf::from).collect()
}

#[cfg(target_arch = "wasm32")]
pub mod web {
    use std::io::{Error, Read, Result, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use wasm_bindgen::{JsCast, JsValue};

    fn io(e: impl std::fmt::Display) -> Error {
        Error::other(e.to_string())
    }

    pub fn call(name: &str, args: &[JsValue]) -> Result<JsValue> {
        let w = web_sys::window().ok_or_else(|| io("нет окна"))?;
        let f: js_sys::Function = js_sys::Reflect::get(&w, &name.into()).ok().and_then(|f| f.dyn_into().ok()).ok_or_else(|| io(format!("нет {name}")))?;
        f.apply(&JsValue::NULL, &args.iter().collect::<js_sys::Array>()).map_err(|e| io(format!("{e:?}")))
    }

    /// Файл из окна браузера: читается кусками по запросу.
    pub struct RangeFile {
        name: String,
        pos: u64,
        len: u64,
    }

    impl RangeFile {
        pub fn open(path: &Path) -> Result<Self> {
            let name = path.to_string_lossy().into_owned();
            let len = call("kartografFileSize", &[JsValue::from_str(&name)])?.as_f64().ok_or_else(|| io(format!("{name}: файл не выбран в этом окне")))? as u64;
            Ok(Self { name, pos: 0, len })
        }
    }

    impl Read for RangeFile {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            if self.pos >= self.len || buf.is_empty() {
                return Ok(0);
            }
            let n = (buf.len() as u64).min(self.len - self.pos).min(16 << 20);
            let v = call("kartografReadSync", &[JsValue::from_str(&self.name), JsValue::from_f64(self.pos as f64), JsValue::from_f64(n as f64)])?;
            let bytes = js_sys::Uint8Array::new(&v);
            let got = bytes.length() as usize;
            bytes.copy_to(&mut buf[..got]);
            self.pos += got as u64;
            Ok(got)
        }
    }

    impl Seek for RangeFile {
        fn seek(&mut self, to: SeekFrom) -> Result<u64> {
            let p = match to {
                SeekFrom::Start(p) => p as i128,
                SeekFrom::End(d) => self.len as i128 + d as i128,
                SeekFrom::Current(d) => self.pos as i128 + d as i128,
            };
            if p < 0 {
                return Err(io("перед началом файла"));
            }
            self.pos = p as u64;
            Ok(self.pos)
        }
    }

    #[allow(dead_code)]
    pub fn name_of(p: &PathBuf) -> String {
        p.to_string_lossy().into_owned()
    }
}

/// В браузере системных окон нет. Окно выбора пака — браузерное: выбранное приходит
/// через `take_picked` кадром позже, а здесь всегда «ничего не выбрали».
/// Папки, сохранение и прочие файлы — пока нет.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct FileDialog {
    exts: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_filter(mut self, _name: impl Into<String>, ext: &[impl ToString]) -> Self {
        self.exts.extend(ext.iter().map(|e| e.to_string()));
        self
    }
    fn browse(&self, multiple: bool) {
        // Пока браузер умеет только паки.
        if !self.exts.iter().any(|e| e == "dungeondraft_pack") {
            return;
        }
        let accept = self.exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(",");
        let _ = web::call("kartografPick", &[wasm_bindgen::JsValue::from_str(&accept), wasm_bindgen::JsValue::from_bool(multiple)]);
    }
    pub fn set_file_name(self, _name: impl Into<String>) -> Self {
        self
    }
    pub fn set_directory<P: AsRef<std::path::Path>>(self, _dir: P) -> Self {
        self
    }
    pub fn pick_file(self) -> Option<std::path::PathBuf> {
        self.browse(false);
        None
    }
    pub fn pick_files(self) -> Option<Vec<std::path::PathBuf>> {
        self.browse(true);
        None
    }
    pub fn pick_folder(self) -> Option<std::path::PathBuf> {
        None
    }
    pub fn save_file(self) -> Option<std::path::PathBuf> {
        None
    }
}
