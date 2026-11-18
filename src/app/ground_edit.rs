//! Кусок местности в «Выборе»: щелчок берёт связный кусок слоя (вода, трава, плиты)
//! под курсором, протяжка двигает его, ручки рамки растягивают и сжимают.
//!
//! Местность — маски долей (`Map::terrain`), а не предметы: кусок — точки маски одного
//! слоя, связные с той, где щёлкнули, вместе с мягким краем. Где кусок был, его доля
//! отходит слою, что лежит вокруг (озеро в траве оставляет траву); на новом месте он
//! кладётся как мазок кисти — растянутым по новой рамке.

use super::*;
use std::collections::VecDeque;

/// Ядро куска — точки, где слоя не меньше этого (из 255).
const CORE: u8 = 32;
/// Мягкий край слабее ядра берётся не дальше стольких точек маски от него.
const FRINGE: usize = 8;

/// Выбранный кусок местности.
#[derive(Clone, Debug, PartialEq)]
pub struct GroundPiece {
    /// Номер материала слоя (`Coat::material`).
    pub material: u8,
    /// Точки маски куска и доля слоя в них.
    pub points: Vec<(usize, u8)>,
    /// Рамка в точках маски: `[i0, j0, i1, j1)`.
    pub rect: [usize; 4],
    /// Слой, которому отходит место, откуда кусок ушёл; нет — там останется пусто.
    pub fill: Option<u8>,
    /// Контур для показа, клетки.
    pub outline: Vec<Segment>,
}

impl GroundPiece {
    /// Рамка в клетках: `[x0, y0, x1, y1]`.
    pub fn cells(&self) -> [f32; 4] {
        let r = RES as f32;
        [self.rect[0] as f32 / r, self.rect[1] as f32 / r, self.rect[2] as f32 / r, self.rect[3] as f32 / r]
    }
}

/// Что тянут: ручка рамки (`[-1..1, -1..1]`, `[0, 0]` — весь кусок).
#[derive(Clone, Debug)]
pub struct GroundGrab {
    handle: [i8; 2],
    from: [f32; 2],
    piece: GroundPiece,
    before: Vec<Coat>,
    touched: Option<[usize; 4]>,
    /// Последняя рамка, куда положен кусок.
    to: [f32; 4],
}

fn neighbours(p: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (i, j) = (p % w, p / w);
    [(i > 0).then(|| p - 1), (i + 1 < w).then(|| p + 1), (j > 0).then(|| p - w), (j + 1 < h).then(|| p + w)].into_iter().flatten()
}

fn union(a: [usize; 4], b: [usize; 4]) -> [usize; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Рамка точек и контур куска.
fn shape(points: &[(usize, u8)], w: usize) -> ([usize; 4], Vec<Segment>) {
    let mut r = [usize::MAX, usize::MAX, 0, 0];
    for &(p, _) in points {
        let (i, j) = (p % w, p / w);
        r = [r[0].min(i), r[1].min(j), r[2].max(i + 1), r[3].max(j + 1)];
    }
    if points.is_empty() {
        return ([0; 4], Vec::new());
    }
    let (lw, lh) = (r[2] - r[0], r[3] - r[1]);
    let mut local = Mask { w: lw, h: lh, data: vec![0; lw * lh] };
    for &(p, v) in points {
        local.data[(p / w - r[1]) * lw + (p % w - r[0])] = v;
    }
    let (ox, oy) = (r[0] as f32 / RES as f32, r[1] as f32 / RES as f32);
    let outline = mask::contour(&local).into_iter().map(|(a, b)| ([a[0] + ox, a[1] + oy], [b[0] + ox, b[1] + oy])).collect();
    (r, outline)
}

/// Кусок местности под точкой `w` (клетки): слой, которого там больше всех, и всё,
/// что с этой точкой связно. Нет местности — `None`.
pub fn piece_at(map: &Map, w: [f32; 2]) -> Option<GroundPiece> {
    let (mw, mh) = (map.rooms.w, map.rooms.h);
    let (i, j) = ((w[0] * RES as f32).floor(), (w[1] * RES as f32).floor());
    if i < 0.0 || j < 0.0 || i as usize >= mw || j as usize >= mh {
        return None;
    }
    let at = j as usize * mw + i as usize;
    // Слоя больше всех; при равенстве — верхний.
    let mut best: Option<(usize, u8)> = None;
    for (k, c) in map.terrain.iter().enumerate() {
        let v = c.mask.data.get(at).copied().unwrap_or(0);
        if best.is_none_or(|(_, b)| v >= b) {
            best = Some((k, v));
        }
    }
    let (k, top) = best?;
    if top < CORE {
        return None;
    }
    let data = &map.terrain[k].mask.data;
    let mut seen = vec![false; mw * mh];
    let mut points = Vec::new();
    let mut queue = VecDeque::from([at]);
    seen[at] = true;
    while let Some(p) = queue.pop_front() {
        points.push(p);
        for n in neighbours(p, mw, mh) {
            if !seen[n] && data[n] >= CORE {
                seen[n] = true;
                queue.push_back(n);
            }
        }
    }
    let mut ring = points.clone();
    for _ in 0..FRINGE {
        let mut next = Vec::new();
        for &p in &ring {
            for n in neighbours(p, mw, mh) {
                if !seen[n] && data[n] > 0 {
                    seen[n] = true;
                    next.push(n);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        points.extend(&next);
        ring = next;
    }
    // Что вокруг: слой, который чаще всех главный в точках у края куска.
    let mut count = vec![0usize; map.terrain.len()];
    let mut looked = vec![false; mw * mh];
    for &p in &points {
        for n in neighbours(p, mw, mh) {
            if seen[n] || looked[n] {
                continue;
            }
            looked[n] = true;
            if let Some((c, v)) = map.terrain.iter().enumerate().filter(|(c, _)| *c != k).map(|(c, coat)| (c, coat.mask.data[n])).max_by_key(|x| x.1)
                && v >= 64
            {
                count[c] += 1;
            }
        }
    }
    let fill = count.iter().enumerate().filter(|x| *x.1 > 0).max_by_key(|x| *x.1).map(|(c, _)| map.terrain[c].material);
    let points: Vec<(usize, u8)> = points.into_iter().map(|p| (p, data[p])).collect();
    let (rect, outline) = shape(&points, mw);
    Some(GroundPiece { material: map.terrain[k].material, points, rect, fill, outline })
}

/// Снять кусок: его доля отходит слою вокруг. Для удаления.
pub fn lift(map: &mut Map, piece: &GroundPiece) {
    let Some(k) = map.terrain.iter().position(|c| c.material == piece.material) else { return };
    let f = piece.fill.and_then(|m| map.terrain.iter().position(|c| c.material == m));
    for &(p, _) in &piece.points {
        let cur = map.terrain[k].mask.data[p];
        map.terrain[k].mask.data[p] = 0;
        if let Some(f) = f {
            let x = &mut map.terrain[f].mask.data[p];
            *x = (*x as u16 + cur as u16).min(255) as u8;
        }
    }
}

/// Кусок с места `piece.rect` — в рамку `to` (клетки), растянутым по ней. Местность
/// до протяжки — `before`; задетое прошлым шагом (`touched`) сперва возвращается.
/// Возвращает точки куска на новом месте.
pub fn put(map: &mut Map, before: &[Coat], piece: &GroundPiece, to: [f32; 4], touched: &mut Option<[usize; 4]>) -> Vec<(usize, u8)> {
    let (mw, mh) = (map.rooms.w, map.rooms.h);
    let res = RES as f32;
    let d = [to[0] * res, to[1] * res, to[2] * res, to[3] * res];
    let target = [(d[0].floor().max(0.0) as usize).min(mw), (d[1].floor().max(0.0) as usize).min(mh), (d[2].ceil().max(0.0) as usize).min(mw), (d[3].ceil().max(0.0) as usize).min(mh)];
    let src = piece.rect;
    if map.terrain.len() != before.len() {
        map.terrain = before.to_vec();
    } else {
        let r = touched.map_or(union(src, target), |t| union(union(src, target), t));
        for (c, b) in map.terrain.iter_mut().zip(before) {
            for j in r[1]..r[3] {
                let (a, e) = (j * mw + r[0], j * mw + r[2]);
                c.mask.data[a..e].copy_from_slice(&b.mask.data[a..e]);
            }
        }
    }
    *touched = Some(union(src, target));
    let Some(k) = map.terrain.iter().position(|c| c.material == piece.material) else { return Vec::new() };
    // Взять кусок с прежнего места (там его доля — у слоя вокруг).
    let lifted = GroundPiece { points: piece.points.iter().map(|&(p, _)| (p, map.terrain[k].mask.data[p])).collect(), ..piece.clone() };
    lift(map, &lifted);
    let (sw, sh) = (src[2] - src[0], src[3] - src[1]);
    if sw == 0 || sh == 0 || d[2] - d[0] < 1e-3 || d[3] - d[1] < 1e-3 {
        return Vec::new();
    }
    let mut raster = vec![0u8; sw * sh];
    for &(p, v) in &lifted.points {
        raster[(p / mw - src[1]) * sw + (p % mw - src[0])] = v;
    }
    let sample = |u: f32, v: f32| -> f32 {
        let (u, v) = (u.clamp(0.0, (sw - 1) as f32), v.clamp(0.0, (sh - 1) as f32));
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let at = |x: usize, y: usize| raster[y * sw + x] as f32;
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        top + (bottom - top) * fy
    };
    let mut weights = Vec::new();
    for tj in target[1]..target[3] {
        let cy = tj as f32 + 0.5;
        if cy < d[1] || cy > d[3] {
            continue;
        }
        for ti in target[0]..target[2] {
            let cx = ti as f32 + 0.5;
            if cx < d[0] || cx > d[2] {
                continue;
            }
            let u = (cx - d[0]) / (d[2] - d[0]) * sw as f32 - 0.5;
            let v = (cy - d[1]) / (d[3] - d[1]) * sh as f32 - 0.5;
            let s = sample(u, v);
            if s >= 0.5 {
                weights.push((tj * mw + ti, (s / 255.0).min(1.0)));
            }
        }
    }
    map.paint(Layer::Floor, piece.material as usize, &weights);
    let k = map.terrain.iter().position(|c| c.material == piece.material).unwrap_or(k);
    weights.iter().map(|&(p, _)| (p, map.terrain[k].mask.data[p])).collect()
}

/// Новая рамка: ручка `handle` сдвинута на `d` клеток. Края — на шаг `step`;
/// `keep` — у угловой ручки без искажения.
pub fn drag_rect(r: [f32; 4], handle: [i8; 2], d: [f32; 2], keep: bool, step: f32) -> [f32; 4] {
    let snap = |v: f32| (v / step).round() * step;
    if handle == [0, 0] {
        let (dx, dy) = (snap(d[0]), snap(d[1]));
        return [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy];
    }
    let min = step.max(0.25);
    let [mut x0, mut y0, mut x1, mut y1] = r;
    match handle[0] {
        -1 => x0 = snap(x0 + d[0]).min(x1 - min),
        1 => x1 = snap(x1 + d[0]).max(x0 + min),
        _ => {}
    }
    match handle[1] {
        -1 => y0 = snap(y0 + d[1]).min(y1 - min),
        1 => y1 = snap(y1 + d[1]).max(y0 + min),
        _ => {}
    }
    if keep && handle[0] != 0 && handle[1] != 0 {
        let (w0, h0) = (r[2] - r[0], r[3] - r[1]);
        let (sx, sy) = ((x1 - x0) / w0, (y1 - y0) / h0);
        let s = if (sx - 1.0).abs() >= (sy - 1.0).abs() { sx } else { sy };
        let (w, h) = (w0 * s, h0 * s);
        if handle[0] < 0 {
            x0 = x1 - w;
        } else {
            x1 = x0 + w;
        }
        if handle[1] < 0 {
            y0 = y1 - h;
        } else {
            y1 = y0 + h;
        }
    }
    [x0, y0, x1, y1]
}

const HANDLES: [[i8; 2]; 8] = [[-1, -1], [0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0]];

fn handle_pos(r: [f32; 4], h: [i8; 2]) -> [f32; 2] {
    let pick = |a: f32, b: f32, k: i8| match k {
        -1 => a,
        1 => b,
        _ => (a + b) / 2.0,
    };
    [pick(r[0], r[2], h[0]), pick(r[1], r[3], h[1])]
}

impl App {
    /// Выбранный кусок местности — только пока выбран он, а не что-то другое.
    pub fn ground_sel(&self) -> Option<&GroundPiece> {
        self.ground.as_ref().filter(|_| self.sel == Some(Sel::Ground))
    }

    /// Кусок местности под точкой: запоминает его и отдаёт выбор. В комнате — нет:
    /// там виден пол комнаты, а не местность.
    pub fn ground_pick(&mut self, w: [f32; 2]) -> Option<Sel> {
        let (i, j) = ((w[0] * RES as f32).floor() as isize, (w[1] * RES as f32).floor() as isize);
        if self.map.rooms.get(i, j) >= 128 {
            return None;
        }
        self.ground = Some(piece_at(&self.map, w)?);
        Some(Sel::Ground)
    }

    /// Рамка выбранного куска: во время протяжки — та, куда его кладут.
    fn ground_frame(&self) -> Option<[f32; 4]> {
        match &self.ground_grab {
            Some(g) => Some(g.to),
            None => self.ground_sel().map(GroundPiece::cells),
        }
    }

    /// Ручка рамки выбранного куска под точкой экрана.
    pub(super) fn ground_handle_at(&self, screen: Pos2) -> Option<[i8; 2]> {
        let r = self.ground_frame()?;
        HANDLES.into_iter().find(|&h| self.cam.to_screen(self.canvas, handle_pos(r, h)).distance(screen) <= GRIP_PX + 3.0)
    }

    /// Начать протяжку выбранного: ручка рамки или тело куска местности, помост, его
    /// лестница или решётка. `true` — взяли.
    pub(super) fn select_grab(&mut self, w: [f32; 2], screen: Pos2) -> bool {
        if let Some(piece) = self.ground_sel().cloned() {
            let r = piece.cells();
            let handle = self.ground_handle_at(screen).or_else(|| (self.pick(w).is_none() && w[0] >= r[0] && w[0] <= r[2] && w[1] >= r[1] && w[1] <= r[3]).then_some([0, 0]));
            if let Some(handle) = handle {
                self.checkpoint();
                // Доли — как сейчас: после выбора могли рисовать.
                let k = self.map.terrain.iter().position(|c| c.material == piece.material);
                let points = match k {
                    Some(k) => piece.points.iter().map(|&(p, _)| (p, self.map.terrain[k].mask.data[p])).collect(),
                    None => Vec::new(),
                };
                let piece = GroundPiece { points, ..piece };
                self.ground_grab = Some(GroundGrab { handle, from: w, to: piece.cells(), before: self.map.terrain.clone(), piece, touched: None });
                return true;
            }
        }
        if let Some(i) = self.selected_platform()
            && self.pick(w).is_none()
        {
            let grab = self.platform_part_at(i, w).or_else(|| (self.platform_at(w) == Some(i)).then(|| platform_edit::PlatformGrab::Whole(i, w, self.map.platforms[i].clone())));
            if grab.is_some() {
                self.checkpoint();
                self.platform_grab = grab;
                return true;
            }
        }
        false
    }

    /// Протяжка куска местности: рамка за ручкой, кусок — в неё.
    pub(super) fn ground_drag(&mut self, w: [f32; 2], keep: bool, snap: bool) {
        let Some(mut g) = self.ground_grab.take() else { return };
        let step = if snap { 0.5 } else { 1.0 / RES as f32 };
        let to = drag_rect(g.piece.cells(), g.handle, [w[0] - g.from[0], w[1] - g.from[1]], keep, step);
        if to != g.to || g.touched.is_none() {
            put(&mut self.map, &g.before, &g.piece, to, &mut g.touched);
            g.to = to;
        }
        self.ground_grab = Some(g);
    }

    /// Конец протяжки: выбранным становится кусок на новом месте.
    pub(super) fn ground_drag_end(&mut self) {
        let Some(mut g) = self.ground_grab.take() else { return };
        let points = put(&mut self.map, &g.before, &g.piece, g.to, &mut g.touched);
        let (rect, outline) = shape(&points, self.map.rooms.w);
        self.ground = (!points.is_empty()).then_some(GroundPiece { points, rect, outline, ..g.piece });
        if self.ground.is_none() {
            self.sel = None;
        }
        self.touch();
    }

    /// Убрать выбранный кусок: место отходит слою вокруг.
    pub(super) fn delete_ground(&mut self) {
        if let Some(piece) = self.ground.take() {
            lift(&mut self.map, &piece);
            self.map.prune();
        }
    }

    /// Контур выбранного куска, его рамка и ручки.
    pub(super) fn draw_ground_marks(&self, painter: &egui::Painter, canvas: Rect) {
        let Some(r) = self.ground_frame() else { return };
        let cam = self.cam;
        let blue = Color32::from_rgb(120, 180, 255);
        if self.ground_grab.is_none()
            && let Some(piece) = self.ground_sel()
        {
            for (a, b) in &piece.outline {
                painter.line_segment([cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], Stroke::new(2.0, blue));
            }
        }
        let frame = Rect::from_two_pos(cam.to_screen(canvas, [r[0], r[1]]), cam.to_screen(canvas, [r[2], r[3]]));
        painter.rect_stroke(frame, 0.0, Stroke::new(1.0, blue.gamma_multiply(0.8)), egui::StrokeKind::Middle);
        for h in HANDLES {
            let c = cam.to_screen(canvas, handle_pos(r, h));
            painter.rect(Rect::from_center_size(c, vec2(GRIP_PX * 1.6, GRIP_PX * 1.6)), 2.0, Color32::from_gray(30), Stroke::new(2.0, blue), egui::StrokeKind::Middle);
        }
        // Курсор над ручкой — куда она тянет.
        let hover = self.ground_grab.as_ref().map(|g| g.handle).or_else(|| self.hover.and_then(|h| self.ground_handle_at(h)));
        if let Some(h) = hover {
            painter.ctx().set_cursor_icon(match h {
                [0, 0] => egui::CursorIcon::Grabbing,
                [-1, -1] | [1, 1] => egui::CursorIcon::ResizeNwSe,
                [1, -1] | [-1, 1] => egui::CursorIcon::ResizeNeSw,
                [_, 0] => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::ResizeVertical,
            });
        }
    }

    /// Панель выбранного куска: что это и как с ним быть.
    pub(super) fn ground_panel(&mut self, ui: &mut egui::Ui) {
        let Some(piece) = self.ground_sel() else { return };
        let name = self.map.materials.get(piece.material as usize).map_or_else(|| "?".to_owned(), |m| m.name.clone());
        let r = self.ground_frame().unwrap_or_else(|| piece.cells());
        ui.strong(format!("Местность: {name}"));
        ui.label(format!("{:.2} × {:.2} кл.", r[2] - r[0], r[3] - r[1]));
        ui.weak("Протяжка — двигать, ручки рамки — растягивать и сжимать, Shift — без искажения. Delete — убрать.");
        if ui.small_button("убрать").clicked() {
            self.checkpoint();
            self.delete_ground();
            self.sel = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Карта 10×10: трава всюду, посередине пруд 2×2 клетки.
    fn pond() -> Map {
        let mut m = Map::new(10, 10);
        let all = mask::cells(m.rooms.w, m.rooms.h, (0, 0), (9, 9));
        m.paint(Layer::Floor, 3, &all);
        let water = mask::cells(m.rooms.w, m.rooms.h, (4, 4), (5, 5));
        m.paint(Layer::Floor, 5, &water);
        m
    }

    fn share(m: &Map, material: u8, x: f32, y: f32) -> u8 {
        let p = (y * RES as f32) as usize * m.rooms.w + (x * RES as f32) as usize;
        m.terrain.iter().find(|c| c.material == material).map_or(0, |c| c.mask.data[p])
    }

    #[test]
    fn picks_the_pond_with_grass_around() {
        let m = pond();
        let piece = piece_at(&m, [4.5, 4.5]).expect("пруд");
        assert_eq!(piece.material, 5);
        assert_eq!(piece.cells(), [4.0, 4.0, 6.0, 6.0]);
        assert_eq!(piece.fill, Some(3), "вокруг трава");
        assert_eq!(piece.points.len(), 2 * 2 * RES * RES);
        assert!(!piece.outline.is_empty());
        let grass = piece_at(&m, [1.0, 1.0]).expect("трава");
        assert_eq!(grass.material, 3);
        assert_eq!(grass.cells(), [0.0, 0.0, 10.0, 10.0]);
    }

    #[test]
    fn moving_leaves_grass_behind() {
        let mut m = pond();
        let piece = piece_at(&m, [4.5, 4.5]).unwrap();
        let before = m.terrain.clone();
        let mut touched = None;
        let pts = put(&mut m, &before, &piece, [1.0, 1.0, 3.0, 3.0], &mut touched);
        assert_eq!(pts.len(), 2 * 2 * RES * RES);
        assert_eq!(share(&m, 5, 1.5, 1.5), 255, "пруд на новом месте");
        assert_eq!(share(&m, 3, 1.5, 1.5), 0);
        assert_eq!(share(&m, 5, 4.5, 4.5), 0, "на старом месте пруда нет");
        assert_eq!(share(&m, 3, 4.5, 4.5), 255, "там трава");
        // Следующий шаг протяжки начинается с исходной местности, а не копит следы.
        put(&mut m, &before, &piece, [6.0, 6.0, 8.0, 8.0], &mut touched);
        assert_eq!(share(&m, 5, 1.5, 1.5), 0, "прошлое место вернулось");
        assert_eq!(share(&m, 3, 1.5, 1.5), 255);
        assert_eq!(share(&m, 5, 7.5, 7.5), 255);
    }

    #[test]
    fn stretching_and_shrinking() {
        let mut m = pond();
        let piece = piece_at(&m, [4.5, 4.5]).unwrap();
        let before = m.terrain.clone();
        let mut touched = None;
        put(&mut m, &before, &piece, [2.0, 4.0, 8.0, 6.0], &mut touched);
        assert_eq!(share(&m, 5, 2.2, 5.0), 255, "растянут влево");
        assert_eq!(share(&m, 5, 7.8, 5.0), 255, "и вправо");
        assert_eq!(share(&m, 5, 5.0, 3.5), 0, "выше рамки не вылез");
        put(&mut m, &before, &piece, [4.0, 4.0, 5.0, 5.0], &mut touched);
        assert_eq!(share(&m, 5, 4.5, 4.5), 255, "сжат в клетку");
        assert_eq!(share(&m, 5, 5.5, 5.5), 0, "остальное — трава");
        assert_eq!(share(&m, 3, 5.5, 5.5), 255);
    }

    #[test]
    fn rect_follows_the_handle() {
        let r = [4.0, 4.0, 6.0, 6.0];
        assert_eq!(drag_rect(r, [0, 0], [1.1, -0.9], false, 0.5), [5.0, 3.0, 7.0, 5.0], "сдвиг на полклетки");
        assert_eq!(drag_rect(r, [1, 0], [2.0, 5.0], false, 0.25), [4.0, 4.0, 8.0, 6.0], "правый край — только вширь");
        assert_eq!(drag_rect(r, [-1, -1], [5.0, 5.0], false, 0.25), [5.75, 5.75, 6.0, 6.0], "не выворачивается");
        assert_eq!(drag_rect(r, [1, 1], [2.0, 0.0], true, 0.25), [4.0, 4.0, 8.0, 8.0], "Shift — без искажения");
    }

    #[test]
    fn delete_gives_place_to_grass() {
        let mut m = pond();
        let piece = piece_at(&m, [4.5, 4.5]).unwrap();
        lift(&mut m, &piece);
        assert_eq!(share(&m, 5, 4.5, 4.5), 0);
        assert_eq!(share(&m, 3, 4.5, 4.5), 255);
    }
}
