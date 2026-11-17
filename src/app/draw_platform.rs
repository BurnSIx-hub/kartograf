//! Отрисовка помостов: тень у подножия, боковая стенка, площадка, ребро, лестницы.
//! Геометрия — [`crate::platform`]. Всё ложится на пол, под предметы и стены, и в
//! выгрузке остаётся в картинке карты.

use super::*;
use crate::platform::{self, Platform, TREAD};

fn tone(c: [u8; 3], k: f32, alpha: f32) -> Color32 {
    let v = |x: u8| (x as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(v(c[0]), v(c[1]), v(c[2]), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

const INK: [u8; 3] = [24, 28, 34];
const WHITE: [u8; 3] = [240, 244, 248];

/// Цветные треугольники в клетках карты.
struct Sheet {
    mesh: egui::Mesh,
    cam: crate::view::Camera,
    canvas: Rect,
}

impl Sheet {
    fn point(&mut self, q: [f32; 2], color: Color32) -> u32 {
        self.mesh.vertices.push(egui::epaint::Vertex { pos: self.cam.to_screen(self.canvas, q), uv: egui::epaint::WHITE_UV, color });
        self.mesh.vertices.len() as u32 - 1
    }

    /// Четырёхугольник a–b–c–d, у каждой вершины свой цвет.
    fn quad(&mut self, p: [[f32; 2]; 4], c: [Color32; 4]) {
        let i = [self.point(p[0], c[0]), self.point(p[1], c[1]), self.point(p[2], c[2]), self.point(p[3], c[3])];
        self.mesh.add_triangle(i[0], i[1], i[2]);
        self.mesh.add_triangle(i[0], i[2], i[3]);
    }

    /// Полоса между двумя обходами контура (от `inner` к `outer`) вдоль свободных рёбер.
    fn band(&mut self, inner: &[[f32; 2]], outer: &[[f32; 2]], free: &[bool], c_in: Color32, c_out: Color32) {
        let n = inner.len();
        for i in (0..n).filter(|&i| free[i]) {
            let j = (i + 1) % n;
            self.quad([inner[i], inner[j], outer[j], outer[i]], [c_in, c_in, c_out, c_out]);
        }
    }

    /// Тонкая линия по обходу контура вдоль свободных рёбер.
    fn line(&mut self, ring: &[[f32; 2]], next: &[[f32; 2]], free: &[bool], color: Color32) {
        self.band(ring, next, free, color, color);
    }
}

impl App {
    /// Помосты этажа — после пола, дорог и теней на полу, перед предметами и стенами.
    pub(super) fn draw_platforms(&mut self, painter: &egui::Painter, canvas: Rect) {
        if self.map.platforms.is_empty() {
            return;
        }
        let res = mask::RES as f32;
        let rooms = &self.map.rooms;
        let room = |q: [f32; 2]| rooms.get((q[0] * res).floor() as isize, (q[1] * res).floor() as isize) > 127;
        let mut sheet = Sheet { mesh: egui::Mesh::default(), cam: self.cam, canvas };
        for pl in &self.map.platforms {
            platform_mesh(&mut sheet, pl, &room);
        }
        if !sheet.mesh.is_empty() {
            painter.add(Shape::mesh(sheet.mesh));
        }
    }
}

/// Свободные рёбра контура: за ребром тот же пол, что перед ним, — не стена комнаты.
/// У ребра вдоль стены нет ни боковой стенки, ни тени: помост упирается в стену.
pub fn free_edges(p: &[[f32; 2]], room: &dyn Fn([f32; 2]) -> bool) -> Vec<bool> {
    let nrm = platform::normals(p);
    let n = p.len();
    (0..n)
        .map(|i| {
            let m = [(p[i][0] + p[(i + 1) % n][0]) / 2.0, (p[i][1] + p[(i + 1) % n][1]) / 2.0];
            room([m[0] + nrm[i][0] * 0.3, m[1] + nrm[i][1] * 0.3]) == room([m[0] - nrm[i][0] * 0.15, m[1] - nrm[i][1] * 0.15])
        })
        .collect()
}

fn platform_mesh(sheet: &mut Sheet, pl: &Platform, room: &dyn Fn([f32; 2]) -> bool) {
    let p = platform::tidy(&pl.points);
    if p.len() < 3 {
        return;
    }
    let h = pl.height.clamp(0.02, 1.5);
    let free = free_edges(&p, room);
    let inside = room(p[0]) || room(p[p.len() / 2]);
    // Тень не выходит за стену: вершина тени по ту сторону стены гаснет.
    let lit = |q: [f32; 2]| room(q) == inside;
    let at = |d: f32| platform::offset(&p, &free, d);

    // 1. Тень у подножия: густая у стенки, тает за три высоты (не дальше клетки).
    let reach = (3.0 * h).min(1.0);
    let (foot, mid, far) = (at(h), at(h + reach * 0.3), at(h + reach));
    let n = p.len();
    for i in (0..n).filter(|&i| free[i]) {
        let j = (i + 1) % n;
        let a = |q: [f32; 2], v: f32| if lit(q) { tone([0, 0, 0], 1.0, v) } else { Color32::TRANSPARENT };
        sheet.quad([foot[i], foot[j], mid[j], mid[i]], [a(foot[i], 0.5), a(foot[j], 0.5), a(mid[j], 0.24), a(mid[i], 0.24)]);
        sheet.quad([mid[i], mid[j], far[j], far[i]], [a(mid[i], 0.24), a(mid[j], 0.24), Color32::TRANSPARENT, Color32::TRANSPARENT]);
    }

    // 2. Боковая стенка: от ребра площадки наружу, к полу темнее; по подножию — тёмная линия.
    sheet.band(&p, &foot, &free, tone(pl.side, 1.0, 1.0), tone(pl.side, 0.45, 1.0));
    let hair = (0.016f32).min(h * 0.25);
    sheet.line(&at(h - hair), &foot, &free, tone(INK, 1.0, 0.9));
    // Швы панелей поперёк стенки — через каждые 0,62 клетки пути вдоль края.
    if h >= 0.15 {
        let nrm = platform::normals(&p);
        let seam = tone(pl.side, 0.42, 0.8);
        let (mut walked, mut next) = (0.0f32, 0.31f32);
        for i in 0..n {
            let j = (i + 1) % n;
            let (dx, dy) = (p[j][0] - p[i][0], p[j][1] - p[i][1]);
            let len = dx.hypot(dy).max(1e-6);
            if free[i] {
                while next <= walked + len {
                    let k = (next - walked) / len;
                    let q = [p[i][0] + dx * k, p[i][1] + dy * k];
                    let (ux, uy) = (dx / len * 0.009, dy / len * 0.009);
                    let low = [q[0] + nrm[i][0] * h, q[1] + nrm[i][1] * h];
                    sheet.quad([[q[0] - ux, q[1] - uy], [q[0] + ux, q[1] + uy], [low[0] + ux, low[1] + uy], [low[0] - ux, low[1] - uy]], [seam; 4]);
                    next += 0.62;
                }
            } else {
                next = next.max(walked + len + 0.31);
            }
            walked += len;
        }
    }

    // 3. Площадка: тот же пол, подкрашенный цветом помоста.
    if pl.strength > 0.0 {
        let fill = tone(pl.color, 1.0, pl.strength);
        let base = sheet.mesh.vertices.len() as u32;
        for &q in &p {
            sheet.point(q, fill);
        }
        for t in platform::triangulate(&p) {
            sheet.mesh.add_triangle(base + t[0] as u32, base + t[1] as u32, base + t[2] as u32);
        }
    }

    // 4. Ребро площадки: светлая кромка и тёмная линия по самому краю.
    let lip = (0.04f32).min(h * 0.6);
    sheet.band(&p, &at(-lip), &free, tone(WHITE, 1.0, 0.85), tone(WHITE, 1.0, 0.7));
    sheet.line(&p, &at(hair * 0.75), &free, tone(INK, 1.0, 0.75));

    // 5. Лестницы: ступени от края наружу, кверху шире; верхняя — на ступень ниже площадки.
    for st in &pl.stairs {
        let Some((o, nrm, _)) = platform::stair_frame(&p, st.at) else { continue };
        let steps = st.steps.clamp(1, 16) as f32;
        let run = steps * TREAD;
        let (w1, w0) = (st.width.clamp(0.4, 12.0) / 2.0, st.width.clamp(0.4, 12.0) / 2.0 * (1.0 - 0.06 * steps).max(0.6));
        let tan = [-nrm[1], nrm[0]];
        // Точка лестницы: `s` — от края помоста наружу, `u` — поперёк, в долях полуширины.
        let half = |s: f32| w1 + (w0 - w1) * (s / run);
        let pt = |s: f32, u: f32| [o[0] + nrm[0] * s + tan[0] * u * half(s), o[1] + nrm[1] * s + tan[1] * u * half(s)];
        let across = |s0: f32, s1: f32| [pt(s0, -1.0), pt(s0, 1.0), pt(s1, 1.0), pt(s1, -1.0)];
        // Тень на полу перед нижней ступенью.
        let (dusk, clear) = (tone([0, 0, 0], 1.0, 0.4), Color32::TRANSPARENT);
        sheet.quad(across(run, run + 0.3), [dusk, dusk, clear, clear]);
        // Боковые стенки лестницы: у помоста — во всю его высоту, к нижней ступени сходят на нет.
        for side in [-1.0f32, 1.0] {
            let top_out = [pt(0.0, side)[0] + tan[0] * side * h, pt(0.0, side)[1] + tan[1] * side * h];
            let (light, dark) = (tone(pl.side, 1.0, 1.0), tone(pl.side, 0.45, 1.0));
            sheet.quad([pt(0.0, side), top_out, pt(run, side), pt(run, side)], [light, dark, dark, light]);
        }
        for k in 0..steps as usize {
            let (s0, s1) = (k as f32 * TREAD, (k + 1) as f32 * TREAD);
            // Чем ниже ступень, тем темнее: верхняя почти как площадка.
            let f = 0.62 + 0.34 * (steps - k as f32) / (steps + 1.0);
            let tread = tone(pl.color, f, 1.0);
            sheet.quad(across(s0, s1), [tread; 4]);
            // Тень от ступени выше и её подступёнок.
            sheet.quad(across(s0, s0 + 0.2), [tone([0, 0, 0], 1.0, 0.45), tone([0, 0, 0], 1.0, 0.45), clear, clear]);
            sheet.quad(across(s0, s0 + 0.05), [tone(INK, 1.4, 0.95); 4]);
            // Светлая кромка у внешнего края ступени.
            sheet.quad(across(s1 - 0.05, s1), [tone(WHITE, 1.0, 0.75); 4]);
        }
        // Контур лестницы.
        for side in [-1.0f32, 1.0] {
            let w = 0.014 / half(0.0);
            sheet.quad([pt(0.0, side), pt(0.0, side * (1.0 - w)), pt(run, side * (1.0 - w)), pt(run, side)], [tone(INK, 1.0, 0.9); 4]);
        }
        sheet.quad(across(run - 0.014, run), [tone(INK, 1.0, 0.9); 4]);
    }
}
