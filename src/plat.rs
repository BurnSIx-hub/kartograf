//! То, чем программа и модуль Foundry отличаются: окна выбора файлов, потоки, часы.
//! Программа (exe) берёт настоящее; сборка в WebAssembly — заглушки и то, что умеет браузер.

#[cfg(not(target_arch = "wasm32"))]
pub use rfd::FileDialog;
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
pub use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_arch = "wasm32")]
pub use web_time::{Instant, SystemTime, UNIX_EPOCH};

/// Окно программы по его коду в Windows (`HWND`), 0 — неизвестно.
#[cfg(not(target_arch = "wasm32"))]
pub fn window_code(frame: &eframe::Frame) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

/// Развернуть свёрнутое окно или свернуть обратно, не отнимая фокус у того, что
/// открыто поверх (игра, другая программа). Не Windows — `false`: пусть окно
/// разворачивает eframe.
pub fn show_quietly(code: isize, minimized: bool) -> bool {
    #[cfg(windows)]
    if code != 0 {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        }
        // SW_SHOWMINNOACTIVE и SW_SHOWNOACTIVATE.
        unsafe { ShowWindow(code, if minimized { 7 } else { 4 }) };
        return true;
    }
    let _ = (code, minimized);
    false
}

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

/// Файлы редактора: настройки, автосейв, карты, запечённые объекты, своя папка
/// картинок. В программе — диск. В модуле — данные Foundry (папка [`DATA_ROOT`])
/// и файлы, выбранные в окне браузера (`foundry/scripts/main.js`): список и мелкие
/// файлы подгружены до запуска, крупные читаются по запросу, запись уходит на
/// сервер в фоне. Удалять файлы Foundry не даёт — удалённый остаётся пустым.
pub mod fs {
    use std::io::Result;
    use std::path::{Path, PathBuf};

    #[cfg(not(target_arch = "wasm32"))]
    pub fn read(path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, bytes)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    /// Файлы в папке и её подпапках, без порядка.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn tree(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p) } else { out.push(p) }
            }
        }
        out
    }

    #[cfg(target_arch = "wasm32")]
    fn js(path: &Path) -> wasm_bindgen::JsValue {
        wasm_bindgen::JsValue::from_str(&path.to_string_lossy().replace('\\', "/"))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn read(path: &Path) -> Result<Vec<u8>> {
        let v = super::web::call("kartografRead", &[js(path)])?;
        if v.is_null() || v.is_undefined() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}: нет файла", path.display())));
        }
        Ok(js_sys::Uint8Array::new(&v).to_vec())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
        super::web::call("kartografWrite", &[js(path), js_sys::Uint8Array::from(bytes).into()]).map(|_| ())
    }

    #[cfg(target_arch = "wasm32")]
    pub fn remove(path: &Path) {
        let _ = super::web::call("kartografRemove", &[js(path)]);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn exists(path: &Path) -> bool {
        super::web::call("kartografExists", &[js(path)]).is_ok_and(|v| v.as_bool() == Some(true))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn tree(dir: &Path) -> Vec<PathBuf> {
        let Ok(v) = super::web::call("kartografTree", &[js(dir)]) else { return Vec::new() };
        js_sys::Array::from(&v).iter().filter_map(|x| x.as_string()).map(PathBuf::from).collect()
    }

    /// Прямые файлы папки.
    pub fn list(dir: &Path) -> Vec<PathBuf> {
        tree(dir).into_iter().filter(|p| p.parent() == Some(dir)).collect()
    }
}

/// Папка данных редактора в модуле — в данных Foundry.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const DATA_ROOT: &str = "/kartograf-editor";

/// Замок окна для автосейва (`recovery`): чужую копию предлагаем вернуть, только
/// если её окно уже не работает. В программе это файл, открытый без права делить
/// его с другими; в модуле — отметка вкладки в браузере, которую она обновляет,
/// пока жива.
#[cfg(target_arch = "wasm32")]
pub fn hold(id: &str) -> bool {
    web::call("kartografHold", &[wasm_bindgen::JsValue::from_str(id)]).is_ok()
}

#[cfg(target_arch = "wasm32")]
pub fn alive(id: &str) -> bool {
    web::call("kartografAlive", &[wasm_bindgen::JsValue::from_str(id)]).is_ok_and(|v| v.as_bool() == Some(true))
}

#[cfg(target_arch = "wasm32")]
pub fn release(id: &str) {
    let _ = web::call("kartografRelease", &[wasm_bindgen::JsValue::from_str(id)]);
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

/// Сеть в браузере (`kartografFetch` в `foundry/scripts/main.js`): заранее
/// скачанное ([`net_prefetch`]) или синхронный запрос.
#[cfg(target_arch = "wasm32")]
pub fn net_fetch(url: &str) -> Result<Vec<u8>, String> {
    let v = web::call("kartografFetch", &[wasm_bindgen::JsValue::from_str(url)]).map_err(|e| e.to_string())?;
    Ok(js_sys::Uint8Array::new(&v).to_vec())
}

/// Скачать фоном, по нескольку разом; ход — [`net_state`].
#[cfg(target_arch = "wasm32")]
pub fn net_prefetch(urls: &[String]) {
    let list: js_sys::Array = urls.iter().map(|u| wasm_bindgen::JsValue::from_str(u)).collect();
    let _ = web::call("kartografNetPrefetch", &[list.into()]);
}

/// Скачать фоном одно (`kartografGet`); что пришло — [`net_got`].
#[cfg(target_arch = "wasm32")]
pub fn net_get_async(url: &str) {
    let _ = web::call("kartografGet", &[wasm_bindgen::JsValue::from_str(url)]);
}

/// Что пришло с прошлого раза по [`net_get_async`]: адрес и байты или ошибка.
#[cfg(target_arch = "wasm32")]
pub fn net_got() -> Vec<(String, Result<Vec<u8>, String>)> {
    use wasm_bindgen::JsCast;
    let Ok(v) = web::call("kartografGot", &[]) else { return Vec::new() };
    js_sys::Array::from(&v)
        .iter()
        .filter_map(|pair| {
            let pair = js_sys::Array::from(&pair);
            let url = pair.get(0).as_string()?;
            let body = pair.get(1);
            Some((url, if body.is_instance_of::<js_sys::Uint8Array>() { Ok(js_sys::Uint8Array::new(&body).to_vec()) } else { Err(body.as_string().unwrap_or_default()) }))
        })
        .collect()
}

/// Сколько из начатого [`net_prefetch`] уже пришло (или не пришло) — и всего.
#[cfg(target_arch = "wasm32")]
pub fn net_state() -> (u32, u32) {
    let Ok(v) = web::call("kartografNetState", &[]) else { return (0, 0) };
    let a = js_sys::Array::from(&v);
    let n = |i: u32| a.get(i).as_f64().unwrap_or(0.0) as u32;
    (n(0), n(1))
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

/// В браузере системных окон нет. Окна выбора пака и папки — браузерные: выбранное
/// приходит через `take_picked` кадром позже, а здесь всегда «ничего не выбрали».
/// Карты сохраняются и открываются в данных Foundry без окна (`App::save_as`,
/// `App::open`); прочие файлы — пока нет.
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
    /// Папка выбирается окном браузера; что выбрали, придёт через `take_picked`
    /// строкой `dir:/<имя папки>`.
    pub fn pick_folder(self) -> Option<std::path::PathBuf> {
        let _ = web::call("kartografPickFolder", &[]);
        None
    }
    pub fn save_file(self) -> Option<std::path::PathBuf> {
        None
    }
}
