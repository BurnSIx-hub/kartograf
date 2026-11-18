//! Отрисовка помостов: тень у подножия, боковая стенка, площадка, решётки, ребро,
//! лестницы, огоньки и перила. Геометрия — [`crate::platform`]. Всё ложится на пол, под
//! предметы и стены, и в выгрузке остаётся в картинке карты.

use super::*;
use crate::platform::{self, Platform, TREAD};

fn tone(c: [u8; 3], k: f32, alpha: f32) -> Color32 {
    let v = |x: u8| (x as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(v(c[0]), v(c[1]), v(c[2]), (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Цвет, разбавленный белым на долю `k`: сердцевина огонька.
fn whiter(c: [u8; 3], k: f32) -> [u8; 3] {
    c.map(|x| (x as f32 + (255.0 - x as f32) * k).round() as u8)
}

const INK: [u8; 3] = [24, 28, 34];
const WHITE: [u8; 3] = [240, 244, 248];
/// Трубка и столбики перил — светлый металл.
const RAIL: [u8; 3] = [208, 214, 222];

/// Цветные треугольники в клетках карты.
pub(super) struct Sheet {
    pub mesh: egui::Mesh,
    pub cam: crate::view::Camera,
    pub canvas: Rect,
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

    /// Прямая полоса ширины `w` по оси a–b.
    pub(super) fn bar(&mut self, a: [f32; 2], b: [f32; 2], w: f32, color: Color32) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-6);
        let (nx, ny) = (-dy / l * w / 2.0, dx / l * w / 2.0);
        self.quad([[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]], [color; 4]);
    }

    /// Круг: от середины цвета `mid` к краю цвета `rim`.
    pub(super) fn disc(&mut self, c: [f32; 2], r: f32, mid: Color32, rim: Color32) {
        let o = self.point(c, mid);
        let n = 12;
        let first = self.point([c[0] + r, c[1]], rim);
        let mut prev = first;
        for k in 1..=n {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            let q = if k == n { first } else { self.point([c[0] + r * a.cos(), c[1] + r * a.sin()], rim) };
            self.mesh.add_triangle(o, prev, q);
            prev = q;
        }
    }

    /// Огонёк-полоска: светлая сердцевина и ореол, тающий в стороны и с концов.
    fn glow(&mut self, a: [f32; 2], b: [f32; 2], w: f32, c: [u8; 3]) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = dx.hypot(dy).max(1e-6);
        let (ux, uy) = (dx / l, dy / l);
        let r = w * 2.6;
        let (halo, clear) = (tone(c, 1.0, 0.42), Color32::TRANSPARENT);
        for sgn in [-1.0f32, 1.0] {
            let (nx, ny) = (-uy * r * sgn, ux * r * sgn);
            self.quad([a, b, [b[0] + nx, b[1] + ny], [a[0] + nx, a[1] + ny]], [halo, halo, clear, clear]);
            // Концы ореола — клином: тает и вдоль полоски.
            for (o, back) in [(a, -1.0f32), (b, 1.0)] {
                let i = [self.point(o, halo), self.point([o[0] + nx, o[1] + ny], clear), self.point([o[0] + ux * r * back, o[1] + uy * r * back], clear)];
                self.mesh.add_triangle(i[0], i[1], i[2]);
            }
        }
        self.bar([a[0] - ux * w * 0.3, a[1] - uy * w * 0.3], [b[0] + ux * w * 0.3, b[1] + uy * w * 0.3], w, tone(whiter(c, 0.45), 1.0, 1.0));
    }

    /// Толстая ломаная со скруглёнными стыками.
    fn tube(&mut self, line: &[[f32; 2]], w: f32, color: Color32) {
        for s in line.windows(2) {
            self.bar(s[0], s[1], w, color);
        }
        for &q in line {
            self.disc(q, w / 2.0, color, color);
        }
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
        let outlines: Vec<Vec<[f32; 2]>> = self.map.platforms.iter().map(|pl| platform::tidy(&pl.points)).collect();
        for (i, pl) in self.map.platforms.iter().enumerate() {
            platform_mesh(&mut sheet, pl, &room, &outlines[i + 1..]);
        }
        if !sheet.mesh.is_empty() {
            painter.add(Shape::mesh(sheet.mesh));
        }
    }

    /// Свободные рёбра помоста `i` по комнатам этой карты.
    pub fn platform_free(&self, i: usize) -> Vec<bool> {
        let res = mask::RES as f32;
        let rooms = &self.map.rooms;
        let room = |q: [f32; 2]| rooms.get((q[0] * res).floor() as isize, (q[1] * res).floor() as isize) > 127;
        let pl = &self.map.platforms[i];
        pl.sides(free_edges(&platform::tidy(&pl.points), &room))
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

/// `above` — контуры помостов, положенных позже (выше): по общему с ними краю огоньки
/// рисует только верхний — ярусы зала делят внешние края.
fn platform_mesh(sheet: &mut Sheet, pl: &Platform, room: &dyn Fn([f32; 2]) -> bool, above: &[Vec<[f32; 2]>]) {
    let p = platform::tidy(&pl.points);
    if p.len() < 3 {
        return;
    }
    let h = pl.height.clamp(0.02, 1.5);
    let free = pl.sides(free_edges(&p, room));
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

    // 4. Решётки у края: тёмная полоса в рамке, прорези поперёк через 0,12 клетки.
    for g in &pl.grates {
        let strip = platform::grate_strip(&p, g, 0.12);
        if strip.len() < 2 {
            continue;
        }
        let (hole, slat, rim) = (tone([12, 14, 18], 1.0, 0.92), tone([118, 126, 136], 1.0, 1.0), tone(INK, 1.0, 0.95));
        for w in strip.windows(2) {
            let ((a0, b0), (a1, b1)) = (w[0], w[1]);
            sheet.quad([a0, a1, b1, b0], [hole; 4]);
        }
        for &(a, b) in &strip[1..strip.len() - 1] {
            sheet.bar(a, b, 0.045, slat);
        }
        let (outer, inner): (Vec<[f32; 2]>, Vec<[f32; 2]>) = strip.iter().copied().unzip();
        sheet.tube(&outer, 0.03, rim);
        sheet.tube(&inner, 0.03, rim);
        sheet.tube(&[strip[0].0, strip[0].1], 0.05, rim);
        sheet.tube(&[strip[strip.len() - 1].0, strip[strip.len() - 1].1], 0.05, rim);
    }

    // 5. Ребро площадки: светлая кромка и тёмная линия по самому краю.
    let lip = (0.04f32).min(h * 0.6);
    sheet.band(&p, &at(-lip), &free, tone(WHITE, 1.0, 0.85), tone(WHITE, 1.0, 0.7));
    sheet.line(&p, &at(hair * 0.75), &free, tone(INK, 1.0, 0.75));

    // 6. Лестницы: ступени от края наружу, кверху шире; верхняя — на ступень ниже площадки.
    let geos: Vec<platform::StairGeo> = pl.stairs.iter().filter_map(|st| platform::stair_geo(&p, st)).collect();
    for (st, g) in pl.stairs.iter().zip(&geos) {
        let steps = st.steps.clamp(1, 16) as f32;
        let run = g.run;
        let across = |s0: f32, s1: f32| [g.pt(s0, -1.0), g.pt(s0, 1.0), g.pt(s1, 1.0), g.pt(s1, -1.0)];
        // Тень на полу перед нижней ступенью.
        let (dusk, clear) = (tone([0, 0, 0], 1.0, 0.4), Color32::TRANSPARENT);
        sheet.quad(across(run, run + 0.3), [dusk, dusk, clear, clear]);
        // Боковые стенки лестницы: у помоста — во всю его высоту, к нижней ступени сходят на нет.
        for side in [-1.0f32, 1.0] {
            let top_out = [g.pt(0.0, side)[0] + g.tan[0] * side * h, g.pt(0.0, side)[1] + g.tan[1] * side * h];
            let (light, dark) = (tone(pl.side, 1.0, 1.0), tone(pl.side, 0.45, 1.0));
            sheet.quad([g.pt(0.0, side), top_out, g.pt(run, side), g.pt(run, side)], [light, dark, dark, light]);
        }
        for k in 0..steps as usize {
            let (s0, s1) = (k as f32 * TREAD, (k + 1) as f32 * TREAD);
            // Ступень — между цветом стенки и площадки: верхняя ближе к площадке, нижняя к стенке.
            let f = 0.42 + 0.36 * (steps - k as f32) / (steps + 1.0);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
            let tread = tone([mix(pl.side[0], pl.color[0]), mix(pl.side[1], pl.color[1]), mix(pl.side[2], pl.color[2])], 1.0, 1.0);
            sheet.quad(across(s0, s1), [tread; 4]);
            // Тень от ступени выше и её подступёнок.
            sheet.quad(across(s0, s0 + 0.2), [tone([0, 0, 0], 1.0, 0.45), tone([0, 0, 0], 1.0, 0.45), clear, clear]);
            sheet.quad(across(s0, s0 + 0.05), [tone(INK, 1.4, 0.95); 4]);
            // Светлая кромка у внешнего края ступени.
            sheet.quad(across(s1 - 0.05, s1), [tone(WHITE, 1.0, 0.75); 4]);
            // Огоньки на ступени: две полоски поперёк марша, посередине проступи.
            if let Some(c) = pl.lights {
                let sm = (s0 + s1) / 2.0 + 0.02;
                let x = g.half(sm) * 0.52;
                let half = (g.half(sm) * 0.2).clamp(0.08, 0.2);
                for sgn in [-1.0f32, 1.0] {
                    sheet.glow(g.at(sm, sgn * x - half), g.at(sm, sgn * x + half), 0.04, c);
                }
            }
        }
        // Контур лестницы.
        for side in [-1.0f32, 1.0] {
            let w = 0.014 / g.half(0.0);
            sheet.quad([g.pt(0.0, side), g.pt(0.0, side * (1.0 - w)), g.pt(run, side * (1.0 - w)), g.pt(run, side)], [tone(INK, 1.0, 0.9); 4]);
        }
        sheet.quad(across(run - 0.014, run), [tone(INK, 1.0, 0.9); 4]);
    }

    // 7. Огоньки по кромке: короткие полоски через 2,4 клетки пути вдоль свободного края,
    // мимо лестниц.
    if let Some(c) = pl.lights {
        let ring = at(-lip * 0.5);
        let (mut walked, mut next) = (0.0f32, 1.2f32);
        let in_stair = |q: [f32; 2]| {
            geos.iter().any(|g| {
                let d = [q[0] - g.o[0], q[1] - g.o[1]];
                let (s, u) = (d[0] * g.nrm[0] + d[1] * g.nrm[1], d[0] * g.tan[0] + d[1] * g.tan[1]);
                s > -0.5 && s < g.run && u.abs() < g.w1 + 0.3
            })
        };
        for i in 0..n {
            let j = (i + 1) % n;
            let (dx, dy) = (ring[j][0] - ring[i][0], ring[j][1] - ring[i][1]);
            let len = dx.hypot(dy).max(1e-6);
            if free[i] {
                while next <= walked + len {
                    let k = (next - walked) / len;
                    let q = [ring[i][0] + dx * k, ring[i][1] + dy * k];
                    let (ux, uy) = (dx / len * 0.24, dy / len * 0.24);
                    if !in_stair(q) && !above.iter().any(|o| o.len() >= 3 && platform::contains(o, q)) {
                        sheet.glow([q[0] - ux, q[1] - uy], [q[0] + ux, q[1] + uy], 0.035, c);
                    }
                    next += 2.4;
                }
            } else {
                next = next.max(walked + len + 1.2);
            }
            walked += len;
        }
    }

    // 8. Перила: тень на полу, тёмный контур, светлая трубка; столбики — с огоньком наверху.
    if pl.rail {
        let runs = platform::rails(&p, &free, &pl.stairs);
        let drop = [0.035f32, 0.05];
        for (line, _) in &runs {
            let shade: Vec<[f32; 2]> = line.iter().map(|q| [q[0] + drop[0], q[1] + drop[1]]).collect();
            sheet.tube(&shade, 0.07, tone([0, 0, 0], 1.0, 0.3));
        }
        for (line, _) in &runs {
            sheet.tube(line, 0.085, tone(INK, 1.0, 0.95));
        }
        for (line, _) in &runs {
            sheet.tube(line, 0.05, tone(RAIL, 1.0, 1.0));
        }
        for (line, closed) in &runs {
            for q in platform::posts(line, *closed) {
                sheet.disc([q[0] + drop[0], q[1] + drop[1]], 0.11, tone([0, 0, 0], 1.0, 0.35), Color32::TRANSPARENT);
                sheet.disc(q, 0.1, tone(INK, 1.0, 1.0), tone(INK, 1.0, 1.0));
                sheet.disc(q, 0.078, tone(WHITE, 1.0, 1.0), tone(RAIL, 0.86, 1.0));
                match pl.lights {
                    Some(c) => {
                        sheet.disc(q, 0.2, tone(c, 1.0, 0.45), Color32::TRANSPARENT);
                        sheet.disc(q, 0.045, tone(whiter(c, 0.5), 1.0, 1.0), tone(c, 1.0, 1.0));
                    }
                    None => sheet.disc(q, 0.028, tone(INK, 1.0, 0.8), tone(INK, 1.0, 0.8)),
                }
            }
        }
    }
}
