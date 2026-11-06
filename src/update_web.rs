//! Обновление внутри Foundry не нужно: модуль обновляет сам Foundry.
//! Здесь тот же вид, что у `update.rs`, но без сети, — чтобы окно программы собиралось как есть.

use eframe::egui;

#[derive(Clone)]
pub struct Found {
    pub version: String,
    pub notes: String,
    pub size: u64,
}

#[derive(Default)]
pub struct Updater {
    pub found: Option<Found>,
    pub message: Option<String>,
    pub progress: Option<(u64, u64)>,
    pub installed: bool,
    pub restart: bool,
    pub shown: bool,
}

impl Updater {
    pub fn start(_ctx: Option<egui::Context>) -> Self {
        Self::default()
    }
    pub fn busy(&self) -> bool {
        false
    }
    pub fn check(&mut self, _ctx: &egui::Context, _quiet: bool) {
        self.message = Some("Модуль обновляется через Foundry.".into());
        self.shown = true;
    }
    pub fn install(&mut self, _ctx: &egui::Context) {}
    pub fn poll(&mut self) {}
}

pub fn relaunch() {}
