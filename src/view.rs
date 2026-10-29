//! Камера холста: где на экране начало карты и сколько пикселей в клетке.

use eframe::egui::{Pos2, Rect, Vec2, vec2};

pub const MIN_SCALE: f32 = 4.0;
pub const MAX_SCALE: f32 = 400.0;
/// Во сколько раз меняет масштаб один щелчок колеса.
pub const WHEEL_STEP: f32 = 1.15;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Смещение начала карты от левого верхнего угла холста.
    pub pan: Vec2,
    /// Пикселей в одной клетке.
    pub scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { pan: vec2(20.0, 20.0), scale: 48.0 }
    }
}

impl Camera {
    pub fn to_screen(&self, canvas: Rect, w: [f32; 2]) -> Pos2 {
        canvas.min + self.pan + vec2(w[0], w[1]) * self.scale
    }

    pub fn to_world(&self, canvas: Rect, p: Pos2) -> [f32; 2] {
        let v = (p - canvas.min - self.pan) / self.scale;
        [v.x, v.y]
    }

    /// Клетка под точкой экрана.
    pub fn cell(&self, canvas: Rect, p: Pos2) -> (i32, i32) {
        let w = self.to_world(canvas, p);
        (w[0].floor() as i32, w[1].floor() as i32)
    }

    /// Ближайший узел сетки: вершины стен садятся на него.
    pub fn node(&self, canvas: Rect, p: Pos2) -> [f32; 2] {
        let w = self.to_world(canvas, p);
        [w[0].round(), w[1].round()]
    }

    /// Масштаб вокруг точки экрана: то, что было под ней, под ней и остаётся.
    pub fn zoom_at(&mut self, canvas: Rect, p: Pos2, factor: f32) {
        let w = self.to_world(canvas, p);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        self.pan = p - canvas.min - vec2(w[0], w[1]) * self.scale;
    }

    /// Вся карта целиком посередине холста, с полем в клетку.
    pub fn fit(&mut self, canvas: Rect, width: u32, height: u32) {
        let (w, h) = (width as f32 + 2.0, height as f32 + 2.0);
        self.scale = (canvas.width() / w).min(canvas.height() / h).clamp(MIN_SCALE, MAX_SCALE);
        let size = vec2(width as f32, height as f32) * self.scale;
        self.pan = (canvas.size() - size) / 2.0;
    }

    pub fn map_rect(&self, canvas: Rect, width: u32, height: u32) -> Rect {
        Rect::from_min_max(self.to_screen(canvas, [0.0, 0.0]), self.to_screen(canvas, [width as f32, height as f32]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let canvas = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        let mut cam = Camera::default();
        let p = pos2(437.0, 291.0);
        let before = cam.to_world(canvas, p);
        cam.zoom_at(canvas, p, 1.7);
        let after = cam.to_world(canvas, p);
        assert!((before[0] - after[0]).abs() < 1e-3 && (before[1] - after[1]).abs() < 1e-3);
    }

    #[test]
    fn fit_centers_the_map() {
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0));
        let mut cam = Camera::default();
        cam.fit(canvas, 30, 20);
        let r = cam.map_rect(canvas, 30, 20);
        assert!((r.center() - canvas.center()).length() < 0.5);
        assert!(canvas.contains_rect(r));
    }
}
