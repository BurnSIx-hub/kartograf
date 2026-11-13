//! Холст: ввод мышью — инструменты, изгиб стен, перетаскивание.

use super::*;

impl App {
    pub(super) fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas = rect;
        self.canvas_ppp = ui.ctx().pixels_per_point();
        if !self.fitted && rect.width() > 50.0 {
            self.cam.fit(rect, self.map.width, self.map.height);
            self.fitted = true;
        }
        self.hover = resp.hover_pos().or(self.hover.filter(|_| resp.dragged()));

        if self.export.is_some() {
            // Выгрузка двигает камеру сама; курсор и инструменты молчат.
            self.hover = None;
            self.draw(&painter, rect);
            return;
        }

        // Колесо — масштаб вокруг курсора.
        if let Some(p) = resp.hover_pos() {
            let notches = ui.input(|i| {
                i.events
                    .iter()
                    .map(|e| match e {
                        egui::Event::MouseWheel { unit, delta, .. } => match unit {
                            egui::MouseWheelUnit::Point => delta.y / 50.0,
                            _ => delta.y,
                        },
                        egui::Event::Zoom(f) if *f > 0.0 => f.ln() / WHEEL_STEP.ln(),
                        _ => 0.0,
                    })
                    .sum::<f32>()
            });
            // Ctrl с колесом у кисти — её размер, а не масштаб карты.
            let brush = matches!(self.tool, Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade | Tool::Relief) && !self.brush.shape.drags();
            let (ctrl, alt) = ui.input(|i| (i.modifiers.command, i.modifiers.alt));
            let prop = matches!(self.tool, Tool::Select | Tool::Object);
            if !alt {
                self.alt_wheel = false;
            }
            if notches != 0.0 && self.tool == Tool::Scatter && ctrl {
                self.scatter.radius = (self.scatter.radius * 1.25f32.powf(notches)).clamp(0.25, 10.0);
            } else if notches != 0.0 && brush && ctrl {
                self.brush.radius = (self.brush.radius * 1.25f32.powf(notches)).clamp(0.25, 10.0);
            } else if notches != 0.0 && prop && alt {
                // Alt с колесом — размер выбранного объекта, а без выбора — того, что ставится.
                let k = 1.1f32.powf(notches);
                let range = |v: f32| v.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
                if let Some(Sel::Prop(i)) = self.sel {
                    // Одна правка в истории на всю прокрутку, пока держат Alt.
                    if !self.alt_wheel {
                        self.checkpoint();
                        self.alt_wheel = true;
                    }
                    self.map.props[i].scale = range(self.map.props[i].scale * k);
                } else {
                    self.object_scale = range(self.object_scale * k);
                }
            } else if notches != 0.0 {
                self.cam.zoom_at(rect, p, WHEEL_STEP.powf(notches));
            }
        }

        // Сдвиг: средняя кнопка или пробел с левой.
        let space = ui.input(|i| i.key_down(Key::Space));
        let panning = resp.dragged_by(PointerButton::Middle) || (space && resp.dragged_by(PointerButton::Primary));
        if panning {
            self.tap_from = None;
            self.cam.pan += resp.drag_delta();
        } else {
            self.handle_tool(ui, &resp);
        }

        self.draw(&painter, rect);
    }

    /// Какую стену выгнет протяжка из точки: нарисованные — по отрезку между
    /// вершинами, стены комнат — по прямому участку целиком.
    pub fn bend_target(&mut self, w: [f32; 2]) -> Option<Bend> {
        let mut best: Option<(f32, Bend)> = None;
        for (i, wall) in self.map.walls.iter().enumerate() {
            for (k, (a, b)) in wall.chords().enumerate() {
                let pts = map::arc_points(a, b, wall.bend(k));
                let d = pts.windows(2).map(|p| map::segment_distance(w, p[0], p[1])).fold(f32::INFINITY, f32::min);
                if d <= PICK && best.is_none_or(|(x, _)| d < x) {
                    best = Some((d, Bend::Wall { wall: i, seg: k }));
                }
            }
        }
        if best.is_some() {
            return best.map(|(_, b)| b);
        }
        // Прямой участок вокруг ближайшего отрезка: соседи в том же направлении.
        for line in mask::join(self.room_edges().to_vec()) {
            let Some((k, d)) = line.windows(2).enumerate().map(|(k, p)| (k, map::segment_distance(w, p[0], p[1]))).min_by(|x, y| x.1.total_cmp(&y.1)) else { continue };
            if d > PICK || best.is_some_and(|(x, _)| d >= x) {
                continue;
            }
            let dir = |p: [f32; 2], q: [f32; 2]| {
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                [dx / l, dy / l]
            };
            let d0 = dir(line[k], line[k + 1]);
            let same = |p: [f32; 2], q: [f32; 2]| {
                let e = dir(p, q);
                d0[0] * e[0] + d0[1] * e[1] > 0.9995
            };
            let (mut s, mut e) = (k, k + 1);
            while s > 0 && same(line[s - 1], line[s]) {
                s -= 1;
            }
            while e + 1 < line.len() && same(line[e], line[e + 1]) {
                e += 1;
            }
            best = Some((d, Bend::Room { a: line[s], b: line[e], h: 0.0 }));
        }
        best.map(|(_, b)| b)
    }

    /// Концы выгибаемого участка.
    pub(super) fn bend_chord(&self, bend: Bend) -> ([f32; 2], [f32; 2]) {
        match bend {
            Bend::Room { a, b, .. } => (a, b),
            Bend::Wall { wall, seg } => self.map.walls[wall].chords().nth(seg).unwrap_or(([0.0; 2], [0.0; 2])),
        }
    }

    /// Прогиб под курсором: расстояние от хорды, плюс — влево по ходу.
    /// С привязкой — шагом в четверть клетки.
    pub(super) fn bend_at(&self, bend: Bend, w: [f32; 2], snap: bool) -> f32 {
        let (a, b) = self.bend_chord(bend);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let h = ((w[0] - mid[0]) * dy - (w[1] - mid[1]) * dx) / len;
        let h = if snap { (h * 4.0).round() / 4.0 } else { h };
        let m = map::max_bend(a, b);
        h.clamp(-m, m)
    }

    /// Отпустили: дуга стены комнаты ложится в маску тем полом, что у стены внутри.
    pub fn finish_bend(&mut self) {
        let Some(Bend::Room { a, b, h }) = self.bending.take() else { return };
        if h.abs() < 0.05 {
            return;
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let material = [0.3f32, -0.3]
            .iter()
            .map(|k| [mid[0] + dy / len * k, mid[1] - dx / len * k])
            .map(|p| self.map.room_at(p[0].floor() as i32, p[1].floor() as i32))
            .find(|&m| m > 0)
            .map_or(self.material, |m| m as usize - 1);
        self.checkpoint();
        // Выгнутая стена остаётся какой была, а не берёт вид новых стен.
        let run = self.edges.iter().min_by(|x, y| map::segment_distance(mid, x.0, x.1).total_cmp(&map::segment_distance(mid, y.0, y.1))).copied();
        let keep = run.map(|s| self.map.edge_look(map::edge_key(&s)));
        let fresh = keep.map(|k| std::mem::replace(&mut self.map.wall_style, k));
        self.map.bend_room(a, b, h, material);
        self.edges_dirty = true;
        self.room_edges();
        if let Some(st) = fresh {
            self.map.wall_style = st;
        }
    }

    /// Ctrl+Shift с протяжкой гнёт стену дугой. `true` — ввод занят изгибом.
    pub(super) fn handle_bend(&mut self, ui: &egui::Ui, resp: &egui::Response, pointer: Option<Pos2>) -> bool {
        let rect = self.canvas;
        let (bend_keys, alt) = ui.input(|i| (i.modifiers.command && i.modifiers.shift, i.modifiers.alt));
        if self.bending.is_none() {
            if !bend_keys {
                return false;
            }
            if resp.drag_started_by(PointerButton::Primary)
                && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
            {
                self.bending = self.bend_target(self.cam.to_world(rect, o));
                if matches!(self.bending, Some(Bend::Wall { .. })) {
                    self.checkpoint();
                }
            }
            return true;
        }
        let Some(bend) = self.bending else { return true };
        if let Some(p) = pointer
            && resp.dragged_by(PointerButton::Primary)
        {
            let h = self.bend_at(bend, self.cam.to_world(rect, p), self.settings.snap != alt);
            match bend {
                Bend::Room { a, b, .. } => self.bending = Some(Bend::Room { a, b, h }),
                Bend::Wall { wall, seg } => self.map.walls[wall].set_bend(seg, h),
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.finish_bend();
            self.bending = None;
        }
        true
    }

    /// Нажатие для инструментов, ставящих по точке: отпустили — вот где нажимали.
    /// Быстрый щелчок на ходу тянет мышь на десятки точек, для них это не протяжка.
    pub(super) fn tap(&mut self, ui: &egui::Ui, resp: &egui::Response) -> Option<Pos2> {
        let (pressed, released, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.press_origin().or(i.pointer.interact_pos())));
        if pressed && resp.contains_pointer() {
            self.tap_from = at;
        }
        if released { self.tap_from.take() } else { None }
    }

    pub(super) fn handle_tool(&mut self, ui: &egui::Ui, resp: &egui::Response) {
        let rect = self.canvas;
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        let alt = ui.input(|i| i.modifiers.alt);
        if self.handle_bend(ui, resp, pointer) {
            return;
        }
        let tap = self.tap(ui, resp);
        match self.tool {
            Tool::Floor | Tool::Erase | Tool::Dungeon | Tool::Shade | Tool::Relief if !self.brush.shape.drags() => {
                // Мазок начинается сразу по нажатию, а не после порога протяжки.
                let (pressed, down, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
                if pressed
                    && resp.contains_pointer()
                    && let Some(p) = at
                {
                    self.shade_erase = alt;
                    self.relief_alt = alt;
                    self.begin_stroke(self.cam.to_world(rect, p));
                } else if self.stroke_last.is_some() {
                    match (down, at) {
                        (true, Some(p)) => self.stroke_to(self.cam.to_world(rect, p)),
                        _ => self.end_stroke(),
                    }
                }
            }
            Tool::Floor | Tool::Erase | Tool::Dungeon | Tool::Shade | Tool::Relief => {
                self.shade_erase = alt;
                self.relief_alt = alt;
                if resp.drag_started_by(PointerButton::Primary) {
                    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
                    self.drag_from = origin.map(|p| self.cam.cell(rect, p));
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    if let (Some(a), Some(p)) = (self.drag_from.take(), pointer) {
                        let b = self.drag_corner(a, self.cam.cell(rect, p), ui.input(|i| i.modifiers.shift));
                        self.paint(a, b);
                    }
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let c = self.cam.cell(rect, p);
                    self.paint(c, c);
                }
            }
            Tool::Wall => {
                if let Some(p) = tap {
                    let node = self.cam.node(rect, p);
                    if self.draft.len() >= 3 && self.draft[0] == node {
                        self.finish_wall(true);
                    } else if self.draft.last() != Some(&node) {
                        self.draft.push(node);
                    }
                }
                if resp.double_clicked() || resp.secondary_clicked() {
                    self.finish_wall(false);
                }
            }
            Tool::Door => {
                if let Some(p) = tap {
                    self.place_portal(self.cam.to_world(rect, p));
                }
            }
            Tool::Roof => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    match self.roof_at(w) {
                        Some(i) => {
                            self.checkpoint();
                            let r = &self.map.roofs[i];
                            self.roof_grab = Some((i, w, r.a, r.b));
                            self.sel = Some(Sel::Roof(i));
                        }
                        None => self.roof_from = Some(self.cam.cell(rect, o)),
                    }
                }
                if let (Some((i, w0, a, b)), Some(p)) = (self.roof_grab, pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    // Крыша двигается целыми клетками — остаётся на стенах.
                    let w = self.cam.to_world(rect, p);
                    let d = [(w[0] - w0[0]).round(), (w[1] - w0[1]).round()];
                    let r = &mut self.map.roofs[i];
                    r.a = [a[0] + d[0], a[1] + d[1]];
                    r.b = [b[0] + d[0], b[1] + d[1]];
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.roof_grab = None;
                    if let (Some(a), Some(p)) = (self.roof_from.take(), pointer) {
                        let b = self.cam.cell(rect, p);
                        self.place_roof(a, b);
                    }
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let w = self.cam.to_world(rect, p);
                    self.sel = self.roof_at(w).map(Sel::Roof).or_else(|| self.outline_at(w));
                }
            }
            Tool::Text => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    if let Some(i) = self.map.labels.iter().rposition(|l| l.contains(w)) {
                        self.checkpoint();
                        let p = self.map.labels[i].pos;
                        self.label_grab = Some((i, [w[0] - p[0], w[1] - p[1]]));
                        self.sel = Some(Sel::Label(i));
                    }
                }
                if let (Some((i, g)), Some(p)) = (self.label_grab, pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    let w = self.cam.to_world(rect, p);
                    self.map.labels[i].pos = self.snap_pos([w[0] - g[0], w[1] - g[1]], alt);
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.label_grab = None;
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let w = self.cam.to_world(rect, p);
                    match self.map.labels.iter().rposition(|l| l.contains(w)) {
                        Some(i) => self.sel = Some(Sel::Label(i)),
                        None if !self.label_style.text.trim().is_empty() => {
                            self.checkpoint();
                            let pos = self.snap_pos(w, alt);
                            self.map.labels.push(map::Label { pos, ..self.label_style.clone() });
                            self.sel = Some(Sel::Label(self.map.labels.len() - 1));
                        }
                        None => self.sel = None,
                    }
                }
            }
            Tool::Path => {
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    // Протяжка — от руки: точки через полклетки, на отпускании дорога готова.
                    self.path_draft.push(self.cam.to_world(rect, o));
                    self.path_free = true;
                }
                if self.path_free
                    && resp.dragged_by(PointerButton::Primary)
                    && let (Some(p), Some(last)) = (pointer, self.path_draft.last().copied())
                {
                    let w = self.cam.to_world(rect, p);
                    if ((w[0] - last[0]).powi(2) + (w[1] - last[1]).powi(2)).sqrt() >= 0.5 {
                        self.path_draft.push(w);
                    }
                }
                if self.path_free && resp.drag_stopped_by(PointerButton::Primary) {
                    if let Some(p) = pointer {
                        self.path_draft.push(self.cam.to_world(rect, p));
                    }
                    self.path_free = false;
                    self.finish_path();
                } else if resp.double_clicked() || resp.secondary_clicked() {
                    self.finish_path();
                } else if resp.clicked()
                    && let Some(p) = pointer
                {
                    let w = self.snap_pos(self.cam.to_world(rect, p), alt);
                    if self.path_draft.last() != Some(&w) {
                        self.path_draft.push(w);
                    }
                }
            }
            Tool::Scatter => {
                let (pressed, down, at) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos()));
                if pressed
                    && resp.contains_pointer()
                    && let Some(p) = at
                {
                    self.checkpoint();
                    let w = self.cam.to_world(rect, p);
                    self.scatter_dab(w, alt);
                    self.scatter.last = Some(w);
                } else if let (true, Some(last), Some(p)) = (down, self.scatter.last, at) {
                    let w = self.cam.to_world(rect, p);
                    // Новый отпечаток — когда кисть ушла на половину радиуса.
                    if ((w[0] - last[0]).powi(2) + (w[1] - last[1]).powi(2)).sqrt() >= self.scatter.radius * 0.5 {
                        self.scatter_dab(w, alt);
                        self.scatter.last = Some(w);
                    }
                } else if !down {
                    self.scatter.last = None;
                }
            }
            Tool::Object => {
                let moved = self.moving.is_some();
                self.drag_prop(ui, resp, pointer, alt);
                if let (Some(p), false) = (tap, moved) {
                    if self.object_asset.is_none() {
                        self.browse = Kind::Object;
                    }
                    self.place_prop(self.cam.to_world(rect, p), alt);
                }
            }
            Tool::Select => {
                let shift = ui.input(|i| i.modifiers.shift);
                if resp.drag_started_by(PointerButton::Primary)
                    && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
                {
                    let w = self.cam.to_world(rect, o);
                    match self.pick(w) {
                        // Взяли объект из группы — едет вся группа.
                        Some(Sel::Prop(i)) if self.picked.contains(&i) && self.grip_at(o).is_none() => {
                            self.checkpoint();
                            self.moving_group = Some((w, self.picked.iter().map(|&k| (k, self.map.props[k].pos)).collect()));
                        }
                        Some(_) => {}
                        // Мимо всего — рамка выбора.
                        None if self.grip_at(o).is_none() => self.band_from = Some(w),
                        None => {}
                    }
                }
                if let (Some((from, start)), Some(p)) = (self.moving_group.clone(), pointer)
                    && resp.dragged_by(PointerButton::Primary)
                {
                    let w = self.cam.to_world(rect, p);
                    let mut d = [w[0] - from[0], w[1] - from[1]];
                    if self.settings.snap != alt {
                        d = [snap_half(d[0]), snap_half(d[1])];
                    }
                    for (k, pos) in start {
                        self.map.props[k].pos = [pos[0] + d[0], pos[1] + d[1]];
                    }
                } else if self.band_from.is_none() {
                    self.drag_prop(ui, resp, pointer, alt);
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    self.moving_group = None;
                    if let (Some(a), Some(p)) = (self.band_from.take(), pointer) {
                        let b = self.cam.to_world(rect, p);
                        let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
                        let inside: Vec<usize> = (0..self.map.props.len())
                            .filter(|&k| {
                                let c = self.map.props[k].pos;
                                c[0] >= lo[0] && c[0] <= hi[0] && c[1] >= lo[1] && c[1] <= hi[1]
                            })
                            .collect();
                        if !shift {
                            self.picked.clear();
                        }
                        for k in inside {
                            if !self.picked.contains(&k) {
                                self.picked.push(k);
                            }
                        }
                        self.sel = None;
                    }
                }
                if resp.clicked()
                    && let Some(p) = pointer
                {
                    let hit = self.pick(self.cam.to_world(rect, p));
                    if shift {
                        // Shift — добавить к группе или убрать из неё.
                        if let Some(Sel::Prop(k)) = self.sel.take() {
                            if !self.picked.contains(&k) {
                                self.picked.push(k);
                            }
                        }
                        if let Some(Sel::Prop(k)) = hit {
                            match self.picked.iter().position(|&x| x == k) {
                                Some(at) => {
                                    self.picked.remove(at);
                                }
                                None => self.picked.push(k),
                            }
                        }
                    } else {
                        self.picked.clear();
                        self.sel = hit;
                    }
                }
            }
        }
    }

    /// Ручка выбранного объекта под точкой экрана.
    pub(super) fn grip_at(&self, screen: Pos2) -> Option<(usize, Grip)> {
        let Some(Sel::Prop(i)) = self.sel else { return None };
        let prop = &self.map.props[i];
        let (corners, turn) = grips(prop, TURN_LIFT_PX / self.cam.scale);
        let near = |w: [f32; 2]| self.cam.to_screen(self.canvas, w).distance(screen) <= GRIP_PX + 3.0;
        if near(turn) {
            return Some((i, Grip::Turn));
        }
        if corners.iter().any(|&c| near(c)) {
            let w = self.cam.to_world(self.canvas, screen);
            let d = ((w[0] - prop.pos[0]).powi(2) + (w[1] - prop.pos[1]).powi(2)).sqrt().max(1e-3);
            return Some((i, Grip::Scale(prop.scale, d)));
        }
        None
    }

    /// Протяжка объекта: за ручку — размер или поворот, за тело — сдвиг.
    pub(super) fn drag_prop(&mut self, ui: &egui::Ui, resp: &egui::Response, pointer: Option<Pos2>, alt: bool) {
        let rect = self.canvas;
        if resp.drag_started_by(PointerButton::Primary)
            && let Some(o) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        {
            let w = self.cam.to_world(rect, o);
            let grip = self.grip_at(o).or_else(|| match self.pick(w) {
                Some(Sel::Prop(i)) => {
                    let pos = self.map.props[i].pos;
                    Some((i, Grip::Move([w[0] - pos[0], w[1] - pos[1]])))
                }
                _ => None,
            });
            if let Some((i, g)) = grip {
                self.checkpoint();
                self.moving = Some((i, g));
                self.sel = Some(Sel::Prop(i));
            }
        }
        if let (Some((i, grip)), Some(p)) = (self.moving, pointer)
            && resp.dragged_by(PointerButton::Primary)
        {
            let w = self.cam.to_world(rect, p);
            let snap = self.settings.snap != alt;
            let prop = &mut self.map.props[i];
            let (dx, dy) = (w[0] - prop.pos[0], w[1] - prop.pos[1]);
            match grip {
                Grip::Move(grab) => {
                    let pos = [w[0] - grab[0], w[1] - grab[1]];
                    prop.pos = if snap { [snap_half(pos[0]), snap_half(pos[1])] } else { pos };
                }
                Grip::Scale(s0, d0) => {
                    let s = s0 * (dx * dx + dy * dy).sqrt() / d0;
                    // С привязкой — шагом в четверть.
                    let s = if snap { (s * 4.0).round() / 4.0 } else { s };
                    prop.scale = s.clamp(*SCALE_RANGE.start(), *SCALE_RANGE.end());
                }
                Grip::Turn => {
                    // При нулевом повороте ручка смотрит вверх.
                    let a = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
                    prop.rot = if snap { ((a / 15.0).round() * 15.0) % 360.0 } else { a };
                }
            }
        }
        if resp.drag_stopped_by(PointerButton::Primary) {
            self.moving = None;
        }
    }
}
