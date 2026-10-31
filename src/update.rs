//! Обновление самой программы с GitHub — как у Lazy Music.
//!
//! Код лежит в приватном репозитории, поэтому выпуски — в публичном `RELEASES` без кода:
//! там `kartograf.exe` и рядом `latest.json` с версией, тем, что нового, адресом и SHA-256.
//! Скачанное сверяется по SHA-256. Запущенный exe Windows заменить не даёт, но переименовать
//! разрешает: он уходит в `.old`, новый встаёт на его место, `.old` удаляется при следующем запуске.
//! Карты и настройки лежат отдельно и не трогаются.

use eframe::egui;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

pub const RELEASES: &str = "https://github.com/BurnSIx-hub/kartograf-releases/releases";

#[derive(Clone, Deserialize)]
pub struct Found {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

enum Msg {
    Latest(Option<Found>),
    Progress(u64, u64),
    Installed,
    Failed(String),
}

/// Состояние окна обновления; работа идёт в своём потоке, сюда приходят только сообщения.
#[derive(Default)]
pub struct Updater {
    pub found: Option<Found>,
    pub message: Option<String>,
    pub progress: Option<(u64, u64)>,
    pub installed: bool,
    pub restart: bool,
    pub shown: bool,
    quiet: bool,
    rx: Option<Receiver<Msg>>,
}

impl Updater {
    /// `ctx` пуст у самопроверки и выгрузки: им в сеть ходить незачем.
    pub fn start(ctx: Option<egui::Context>) -> Self {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::fs::remove_file(old_path(&exe));
        }
        let mut this = Self::default();
        if let Some(ctx) = ctx {
            this.check(&ctx, true);
        }
        this
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Спросить GitHub. `quiet` — окно только если есть что поставить.
    pub fn check(&mut self, ctx: &egui::Context, quiet: bool) {
        if self.busy() {
            return;
        }
        self.quiet = quiet;
        self.shown = !quiet;
        self.message = (!quiet).then(|| "Проверяю…".to_owned());
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(match latest(&format!("{RELEASES}/latest/download/latest.json")) {
                Ok(f) => Msg::Latest(f),
                Err(e) => Msg::Failed(e),
            });
            ctx.request_repaint();
        });
    }

    pub fn install(&mut self, ctx: &egui::Context) {
        let Some(found) = self.found.clone() else { return };
        if self.busy() {
            return;
        }
        self.quiet = false;
        self.message = None;
        self.progress = Some((0, found.size));
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let repaint = ctx.clone();
            let sender = tx.clone();
            let done = install(&found, move |got, total| {
                let _ = sender.send(Msg::Progress(got, total));
                repaint.request_repaint();
            });
            let _ = tx.send(done.map_or_else(Msg::Failed, |_| Msg::Installed));
            ctx.request_repaint();
        });
    }

    /// Разобрать пришедшее из потока; вызывается каждый кадр.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut finished = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Progress(got, total) => self.progress = Some((got, total)),
                Msg::Latest(found) => {
                    finished = true;
                    self.message = found.is_none().then(|| format!("Стоит последняя версия: {}.", env!("CARGO_PKG_VERSION")));
                    self.shown |= found.is_some();
                    self.found = found;
                }
                Msg::Installed => {
                    finished = true;
                    self.installed = true;
                    self.progress = None;
                }
                Msg::Failed(e) => {
                    finished = true;
                    self.progress = None;
                    self.message = Some(e);
                    self.shown = !self.quiet;
                }
            }
        }
        if finished {
            self.rx = None;
        }
    }
}

fn agent(timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).timeout_global(timeout).build().into()
}

/// Что выложено, если оно новее этой копии.
fn latest(url: &str) -> Result<Option<Found>, String> {
    let fail = |e: String| format!("Не удалось проверить обновления: {e}");
    let mut response = agent(Some(Duration::from_secs(30))).get(url).call().map_err(|e| fail(e.to_string()))?;
    let text = response.body_mut().read_to_string().map_err(|e| fail(e.to_string()))?;
    let found: Found = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
    Ok(newer(&found.version, env!("CARGO_PKG_VERSION")).then_some(found))
}

fn newer(version: &str, current: &str) -> bool {
    let parts = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect::<Vec<u32>>();
    parts(version) > parts(current)
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("exe.old")
}

/// Скачать, сверить и поставить на место запущенного exe.
fn install(found: &Found, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let next = exe.with_extension("exe.new");
    let fail = |e: String| format!("Обновление не скачалось: {e}");
    let response = agent(None).get(&found.url).call().map_err(|e| fail(e.to_string()))?;
    let mut body = response.into_body().into_reader();
    let mut bytes = Vec::with_capacity(found.size as usize);
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = body.read(&mut chunk).map_err(|e| fail(e.to_string()))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        progress(bytes.len() as u64, found.size);
    }
    check_bytes(&bytes, found)?;
    std::fs::write(&next, &bytes).map_err(|e| format!("{}: {e}", next.display()))?;
    let old = old_path(&exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("{}: {e}", exe.display()))?;
    if let Err(e) = std::fs::rename(&next, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("{}: {e}", exe.display()));
    }
    Ok(())
}

fn check_bytes(bytes: &[u8], found: &Found) -> Result<(), String> {
    let sum: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if bytes.len() as u64 != found.size || !sum.eq_ignore_ascii_case(&found.sha256) || !bytes.starts_with(b"MZ") {
        return Err("Скачанный файл не совпал с выпуском, ничего не заменено.".to_owned());
    }
    Ok(())
}

/// Запустить уже новую копию: вызывается при закрытии окна.
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}

/// `--update <адрес latest.json>`: без окна — та же проверка и замена, для проверки выпуска.
pub fn cli(url: &str) -> i32 {
    let done = latest(url).and_then(|found| match found {
        Some(f) => install(&f, |_, _| {}).map(|_| f.version),
        None => Err("новее нет".to_owned()),
    });
    match done {
        Ok(version) => {
            println!("поставлена {version}");
            0
        }
        Err(e) => {
            println!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(newer("0.1.10", "0.1.9"));
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
    }

    #[test]
    fn foreign_file_is_refused() {
        let good = b"MZ payload";
        let sum: String = Sha256::digest(good).iter().map(|b| format!("{b:02x}")).collect();
        let found = Found { version: "9".into(), notes: String::new(), url: String::new(), sha256: sum, size: good.len() as u64 };
        assert!(check_bytes(good, &found).is_ok());
        assert!(check_bytes(b"MZ payl0ad", &found).is_err());
        assert!(check_bytes(b"MZ", &found).is_err());
    }
}
