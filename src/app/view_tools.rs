//! Как в Dungeondraft: ровная линия с Shift, точки стен и дорог с привязкой и без,
//! линейка (R) и предпросмотр света (L).

use super::*;

/// Направления ровной линии: по сетке и под 45°.
const DIRS: [[f32; 2]; 8] = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0], [1.0, -1.0]];

/// Точка `to`, притянутая к ровной линии из `from` — вдоль сетки или под 45°, как
/// Shift в Paint. С привязкой шаг по линии — целая клетка (по диагонали — клетка
/// по каждой оси), так что конец остаётся на узле.
pub fn straight(from: [f32; 2], to: [f32; 2], snap: bool) -> [f32; 2] {
    let d = [to[0] - from[0], to[1] - from[1]];
    if d[0] == 0.0 && d[1] == 0.0 {
        return to;
    }
    let k = ((d[1].atan2(d[0]) / std::f32::consts::FRAC_PI_4).round() as i32).rem_euclid(8) as usize;
    let u = DIRS[k];
    let t = (d[0] * u[0] + d[1] * u[1]) / (u[0] * u[0] + u[1] * u[1]);
    let t = if snap { t.round() } else { t };
    [from[0] + u[0] * t, from[1] + u[1] * t]
}

/// Расстояние на линейке: клетки и футы (клетка — 5 футов).
pub fn ruler_text(a: [f32; 2], b: [f32; 2]) -> String {
    let cells = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    format!("{:.1} кл. · {:.0} фт", cells, cells * 5.0)
}

/// Свет в окне: сцена (по правке карты) и посчитанная сетка (по виду). Только в
/// настольной сборке: `lighting` в сборку для браузера не входит.
#[cfg(not(target_arch = "wasm32"))]
pub struct LightCache {
    rev: u64,
    roofs: bool,
    scene: crate::lighting::Scene,
    grid: Option<([f32; 4], f32, usize, usize, Vec<[f32; 6]>)>,
}

impl App {
    /// Привязка сейчас: общая настройка, Alt — на время наоборот.
    pub(super) fn snap_now(&self, alt: bool) -> bool {
        self.settings.snap != alt
    }

    /// Вершина стены под точкой экрана: с привязкой — узел сетки, без — где щёлкнули;
    /// с Shift — на ровной линии от прошлой вершины.
    pub(super) fn wall_point(&self, screen: Pos2, alt: bool, shift: bool) -> [f32; 2] {
        let w = self.cam.to_world(self.canvas, screen);
        let snap = self.snap_now(alt);
        match self.draft.last() {
            Some(&last) if shift => {
                let q = straight(last, w, snap);
                // От вершины не на узле ровная линия с привязкой ведёт к точке на линии, а не к узлу.
                if snap && last[0].fract() == 0.0 && last[1].fract() == 0.0 { [q[0].round(), q[1].round()] } else { q }
            }
            _ if snap => [w[0].round(), w[1].round()],
            _ => w,
        }
    }

    /// Точка дороги под точкой экрана: как у объекта (полклетки), с Shift — по ровной линии.
    pub(super) fn path_point(&self, screen: Pos2, alt: bool, shift: bool) -> [f32; 2] {
        let w = self.cam.to_world(self.canvas, screen);
        match self.path_draft.last() {
            Some(&last) if shift => straight(last, w, false),
            _ => self.snap_pos(w, alt),
        }
    }

    /// Линейка: протяжка меряет, карта не меняется. С привязкой концы — в серединах клеток.
    pub(super) fn ruler_input(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let alt = ui.input(|i| i.modifiers.alt);
        let snap = self.snap_now(alt);
        let at = |app: &App, p: Pos2| {
            let w = app.cam.to_world(app.canvas, p);
            if snap { [w[0].floor() + 0.5, w[1].floor() + 0.5] } else { w }
        };
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin())
        {
            let a = at(self, o);
            self.ruler_line = Some((a, a));
        }
        if resp.dragged_by(PointerButton::Primary)
            && let (Some((a, _)), Some(p)) = (self.ruler_line, resp.interact_pointer_pos())
        {
            let shift = ui.input(|i| i.modifiers.shift);
            let b = at(self, p);
            self.ruler_line = Some((a, if shift { straight(a, b, false) } else { b }));
        }
        if resp.secondary_clicked() {
            self.ruler_line = None;
        }
    }

    pub(super) fn draw_ruler(&self, painter: &egui::Painter, canvas: Rect) {
        let Some((a, b)) = self.ruler_line else { return };
        let (pa, pb) = (self.cam.to_screen(canvas, a), self.cam.to_screen(canvas, b));
        let gold = Color32::from_rgb(255, 210, 90);
        painter.line_segment([pa, pb], Stroke::new(4.0, Color32::from_black_alpha(140)));
        painter.line_segment([pa, pb], Stroke::new(2.0, gold));
        painter.circle_filled(pa, 4.0, gold);
        painter.circle_filled(pb, 4.0, gold);
        let text = ruler_text(a, b);
        let font = egui::FontId::proportional(15.0);
        let galley = painter.layout_no_wrap(text, font, gold);
        let at = pb + vec2(10.0, -10.0 - galley.size().y);
        painter.rect_filled(Rect::from_min_size(at - vec2(4.0, 2.0), galley.size() + vec2(8.0, 4.0)), 3.0, Color32::from_black_alpha(190));
        painter.galley(at, galley, gold);
    }

    /// Свет поверх вида — то же, что ляжет на ночной снимок: огни предметов, стены и
    /// окна. Множитель и сияние — одним слоем: в смеси egui (альфа уже умножена)
    /// цвет точки = свечение + под ней × (1 − альфа).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn draw_light(&mut self, painter: &egui::Painter, canvas: Rect, view: Rect) {
        let roofs = self.roof_alpha() >= 1.0;
        if self.light.as_ref().is_none_or(|c| c.rev != self.rev || c.roofs != roofs) {
            let lay = crate::export::layout(&self.map, &self.map.room_edges());
            let covers = if roofs { self.roof_outlines() } else { Vec::new() };
            let scene = crate::lighting::Scene::new(&lay, self.map.rooms.clone(), covers);
            self.light = Some(LightCache { rev: self.rev, roofs, scene, grid: None });
        }
        let time = if self.map.scene_light == "dark" || self.map.scene_light.is_empty() { crate::lighting::Time::Night } else { crate::lighting::Time::Dusk };
        // Точка света — около 8 точек экрана, не мельче десятой клетки.
        let cell = (8.0 / self.cam.scale).max(1.0 / crate::lighting::PER_CELL);
        let (mw, mh) = (self.map.width as f32, self.map.height as f32);
        let snap = |v: f32| (v / cell).floor() * cell;
        let area = [snap(view.min.x.max(0.0)) - cell, snap(view.min.y.max(0.0)) - cell, view.max.x.min(mw) + cell, view.max.y.min(mh) + cell];
        let (lw, lh) = (((area[2] - area[0]) / cell).ceil().max(1.0) as usize, ((area[3] - area[1]) / cell).ceil().max(1.0) as usize);
        let cam = self.cam;
        let Some(cache) = self.light.as_mut() else { return };
        if cache.grid.as_ref().is_none_or(|g| g.0 != area || g.1 != cell) {
            let map = crate::lighting::preview(&cache.scene, time, [area[0], area[1]], cell, lw, lh);
            cache.grid = Some((area, cell, lw, lh, map));
        }
        let Some((area, cell, lw, lh, map)) = cache.grid.as_ref() else { return };
        let (cell, lw, lh) = (*cell, *lw, *lh);
        let mut mesh = egui::Mesh::default();
        for (k, t) in map.iter().enumerate() {
            let (i, j) = (k % lw, k / lw);
            let p = cam.to_screen(canvas, [area[0] + (i as f32 + 0.5) * cell, area[1] + (j as f32 + 0.5) * cell]);
            // Под точкой остаётся доля по самому тёмному каналу; цвет огня — разница
            // каналов, прибавленная к средне-серой картинке; светлее дня — сиянием.
            let low = t[0].min(t[1]).min(t[2]);
            let keep = low.min(1.0);
            let glow = |c: usize| ((t[3 + c] + (t[c].min(1.0) - keep) * 0.5 + (t[c] - 1.0).max(0.0) * 0.4) * 255.0).round().clamp(0.0, 255.0) as u8;
            let color = Color32::from_rgba_premultiplied(glow(0), glow(1), glow(2), ((1.0 - keep) * 255.0).round() as u8);
            mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::epaint::WHITE_UV, color });
        }
        for j in 0..lh.saturating_sub(1) {
            for i in 0..lw.saturating_sub(1) {
                let a = (j * lw + i) as u32;
                let (b, c, d) = (a + 1, a + lw as u32 + 1, a + lw as u32);
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(a, c, d);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_line_like_paint() {
        assert_eq!(straight([1.0, 1.0], [5.3, 1.4], false), [5.3, 1.0], "почти вдоль — вдоль");
        assert_eq!(straight([1.0, 1.0], [1.2, 4.0], false), [1.0, 4.0], "почти вниз — вниз");
        let d = straight([0.0, 0.0], [3.0, 2.6], false);
        assert!((d[0] - d[1]).abs() < 1e-5 && (d[0] - 2.8).abs() < 1e-5, "под 45°: {d:?}");
        assert_eq!(straight([2.0, 2.0], [4.6, 0.1], true), [4.0, 0.0], "с привязкой — узел на диагонали");
        assert_eq!(straight([1.0, 1.0], [1.0, 1.0], false), [1.0, 1.0]);
    }

    #[test]
    fn mirrored_prop_swaps_left_and_right() {
        let p = Prop { asset: "x".into(), pos: [3.0, 3.0], size: [2.0, 1.0], rot: 30.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        let q = Prop { flip: true, ..p.clone() };
        let (a, b) = (p.corners(), q.corners());
        assert_eq!(b, [a[1], a[0], a[3], a[2]], "левый край картинки — справа");
        assert!(q.contains([3.9, 3.0]) && !q.contains([4.2, 3.0]), "форма та же");
        assert!(!serde_json::to_string(&p).unwrap().contains("flip"), "неотражённый файл не меняется");
    }

    #[test]
    fn ruler_counts_feet() {
        assert_eq!(ruler_text([0.0, 0.0], [3.0, 4.0]), "5.0 кл. · 25 фт");
    }
}
