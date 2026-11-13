//! Отрисовка карты: общий порядок, предметы, двери и окна.

use super::*;

impl App {
    pub(super) fn draw(&mut self, painter: &egui::Painter, canvas: Rect) {
        let cam = self.cam;
        let s = cam.scale;
        let (mw, mh) = (self.map.width as i32, self.map.height as i32);
        let map_rect = cam.map_rect(canvas, self.map.width, self.map.height);
        painter.rect_filled(map_rect, 0.0, Color32::from_gray(38));

        // Рисуется только видимая часть: на большой карте при сильном приближении
        // остальное всё равно за краем окна.
        let (tl, br) = (cam.to_world(canvas, canvas.min), cam.to_world(canvas, canvas.max));
        let view = Rect::from_min_max(pos2(tl[0], tl[1]), pos2(br[0], br[1]));
        let (x0, x1) = ((tl[0].floor() as i32).max(0), (br[0].ceil() as i32).min(mw));
        let (y0, y1) = ((tl[1].floor() as i32).max(0), (br[1].ceil() as i32).min(mh));

        // Пол: местность, затем пол комнат, обрезанный по самим комнатам.
        let terrain = std::mem::take(&mut self.map.terrain);
        let room_floor = std::mem::take(&mut self.map.room_floor);
        for (k, coat) in terrain.iter().enumerate() {
            self.draw_coat(painter, canvas, coat, &terrain[k + 1..], None, view);
        }
        let rooms = std::mem::replace(&mut self.map.rooms, Mask { w: 0, h: 0, data: Vec::new() });
        for (k, coat) in room_floor.iter().enumerate() {
            self.draw_coat(painter, canvas, coat, &room_floor[k + 1..], Some(&rooms), view);
        }
        self.map.terrain = terrain;
        self.map.room_floor = room_floor;
        self.map.rooms = rooms;

        let relief = self.relief_look();

        // Дороги и реки — поверх пола.
        let paths = std::mem::take(&mut self.map.paths);
        for (i, path) in paths.iter().enumerate() {
            self.draw_path(painter, canvas, path, i as u32);
        }
        self.map.paths = paths;

        // Затемнение ярусов и их тени — поверх пола и дорог.
        if let Some(look) = &relief {
            let black = Material { name: String::new(), paint: Paint::Color([10, 14, 26]), vary: false, live: map::Live::Auto };
            self.draw_mask(painter, canvas, &black, &look.dark, &[], None, view);
        }

        // По кромкам ярусов полосы — осыпь под обрывом, гребни уступов, стенки
        // обрывов — поверх затемнения: стенка не тонет в тени нижнего яруса.
        // Гребень под дорогой гаснет: тропа идёт через уступ.
        if let Some(look) = &relief {
            let (face, ridge, under) = self.map.relief.as_ref().map(|r| (r.face_key(), r.ridge_key(), r.under_key())).unwrap_or_default();
            for (e, fade) in look.edges.iter().zip(&look.fade).filter(|(e, _)| e.kind == crate::relief::EdgeKind::Cliff) {
                let inner = e.width.iter().map(|w| w * 0.85).collect::<Vec<_>>();
                let outer = e.width.iter().map(|w| w * 0.85 + crate::relief::UNDER_WIDTH * (0.5 + 0.5 * (w / 1.2).min(1.0))).collect::<Vec<_>>();
                self.draw_relief_strip(painter, canvas, &under, e, &inner, &outer, fade, view);
            }
            for (e, fade) in look.edges.iter().zip(&look.fade).filter(|(e, _)| e.kind == crate::relief::EdgeKind::Ridge) {
                // Трава кромки — на верхнем ярусе, у полос гребня она в верхней пятой части.
                let inner = vec![-0.16; e.points.len()];
                self.draw_relief_strip(painter, canvas, &ridge, e, &inner, &e.width, fade, view);
            }
            for (e, fade) in look.edges.iter().zip(&look.fade).filter(|(e, _)| e.kind == crate::relief::EdgeKind::Cliff) {
                // Кромка чуть заходит на высокий ярус: рваный верх полосы ложится на край.
                let inner = vec![-0.12; e.points.len()];
                self.draw_relief_strip(painter, canvas, &face, e, &inner, &e.width, fade, view);
            }
        }

        // Нарисованная тень — поверх пола и дорог.
        if let Some(shade) = self.map.shade.take() {
            let black = Material { name: String::new(), paint: Paint::Color([0, 0, 0]), vary: false, live: map::Live::Auto };
            self.draw_mask(painter, canvas, &black, &shade, &[], None, view);
            self.map.shade = Some(shade);
        }
        // Тени крон — там же: на полу, под предметами и стенами.
        if let Some(trees) = self.tree_shade() {
            let black = Material { name: String::new(), paint: Paint::Color([0, 0, 0]), vary: false, live: map::Live::Auto };
            self.draw_mask(painter, canvas, &black, &trees, &[], None, view);
        }

        let exporting = self.export.is_some();
        // Протянутая заливка — рамкой, пока кнопку не отпустили.
        if let (Some(a), Some(p)) = (self.drag_from, self.hover.filter(|_| !exporting)) {
            let b = self.drag_corner(a, cam.cell(canvas, p), painter.ctx().input(|i| i.modifiers.shift));
            let r = Rect::from_min_max(
                cam.to_screen(canvas, [a.0.min(b.0) as f32, a.1.min(b.1) as f32]),
                cam.to_screen(canvas, [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0]),
            );
            let blue = Color32::from_rgb(120, 180, 255);
            if self.brush.shape == Tip::Oval {
                painter.rect_stroke(r, 0.0, Stroke::new(1.0, blue.gamma_multiply(0.4)), egui::StrokeKind::Inside);
                let n = 72;
                let pts: Vec<Pos2> = (0..n)
                    .map(|k| {
                        let t = k as f32 / n as f32 * std::f32::consts::TAU;
                        r.center() + vec2(t.cos() * r.width() / 2.0, t.sin() * r.height() / 2.0)
                    })
                    .collect();
                painter.add(Shape::closed_line(pts, Stroke::new(2.0, blue)));
            } else {
                painter.rect_stroke(r.intersect(map_rect), 0.0, Stroke::new(2.0, blue), egui::StrokeKind::Inside);
            }
            let (w, h) = ((a.0 - b.0).abs() + 1, (a.1 - b.1).abs() + 1);
            painter.text(r.right_bottom() + vec2(6.0, 4.0), egui::Align2::LEFT_TOP, format!("{w}×{h}"), egui::FontId::proportional(13.0), blue);
        }

        // Сетка. На мелком масштабе она сливается в сплошной фон — тогда её нет.
        // В выгрузке сетки нет: её рисует сам VTT.
        if s >= 10.0 && !exporting {
            let grid = Stroke::new(1.0, Color32::from_black_alpha(60));
            for x in x0..=x1 {
                painter.line_segment([cam.to_screen(canvas, [x as f32, y0 as f32]), cam.to_screen(canvas, [x as f32, y1 as f32])], grid);
            }
            for y in y0..=y1 {
                painter.line_segment([cam.to_screen(canvas, [x0 as f32, y as f32]), cam.to_screen(canvas, [x1 as f32, y as f32])], grid);
            }
        }
        if !exporting {
            painter.rect_stroke(map_rect, 0.0, Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Outside);
        }

        // Стены этажа ниже — пунктиром, чтобы верхний лёг на них.
        if !self.below.is_empty() && !exporting {
            let ghost = Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 220, 140, 120));
            for (a, b) in &self.below {
                painter.add(Shape::dashed_line(&[cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], ghost, 6.0, 4.0));
            }
        }

        // Объекты — под стенами, как мебель у стены; что выше слоем — позже.
        let shadows = self.map.shadows.clone();
        self.draw_props(painter, canvas, view, exporting, i16::MIN..map::ABOVE_WALLS, 1.0);

        // Стены комнат и нарисованные вручную — за вычетом проёмов. Тени стен
        // ложатся и на объекты у стены.
        let style = self.map.wall_style.clone();
        let mut looks: Vec<(map::WallStyle, Vec<Segment>)> = Vec::new();
        for s in self.room_edges().to_vec() {
            let st = self.map.edge_look(map::edge_key(&s));
            match looks.iter_mut().find(|l| l.0 == st) {
                Some(l) => l.1.push(s),
                None => looks.push((st, vec![s])),
            }
        }
        for l in &mut looks {
            l.1 = map::cut_portals_wide(&l.1, &self.map.portals, l.0.width / 2.0);
        }
        if shadows.on {
            // Стена под крышей своей тени не даёт: её накрывает тень крыши, а вдвоём они
            // чернили двери и всё у стены на теневой стороне дома.
            let roofs = if self.roof_view == RoofView::Hidden && self.export.is_none() { Vec::new() } else { self.map.roofs.clone() };
            let open = |segs: &[Segment]| -> Vec<Segment> {
                // Середина — чуть в обе стороны от стены: стена на стыке двух крыш
                // (перегородка) тоже под крышей, хоть ни одна её не накрывает целиком.
                segs.iter()
                    .copied()
                    .filter(|s| {
                        let m = [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
                        let (dx, dy) = (s.1[0] - s.0[0], s.1[1] - s.0[1]);
                        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                        let n = [-dy / l * 0.1, dx / l * 0.1];
                        !roofs.iter().any(|r| r.contains(m) || r.contains([m[0] + n[0], m[1] + n[1]]) || r.contains([m[0] - n[0], m[1] - n[1]]))
                    })
                    .collect()
            };
            for sh in shadows.layers() {
                for (st, edges) in &looks {
                    self.draw_wall_shadow(painter, canvas, &open(edges), st.width, &sh);
                }
                for w in self.map.walls.clone() {
                    let width = w.style.as_ref().map_or(style.width, |s| s.width);
                    self.draw_wall_shadow(painter, canvas, &open(&map::cut_portals_wide(&w.segments().collect::<Vec<_>>(), &self.map.portals, width / 2.0)), width, &sh);
                }
            }
            // У подножия — с обеих сторон и под крышей тоже: два слоя, узкий
            // потемнее и широкий посветлее, — край тает мягко.
            if shadows.ao > 0.0 {
                for (blur, part) in [(0.45, 0.55), (1.1, 0.45)] {
                    let sh = map::Shadows { dist: 0.0, blur, opacity: shadows.ao * part, fade: 0.0, ..shadows.clone() };
                    for (st, edges) in &looks {
                        self.draw_wall_shadow(painter, canvas, edges, st.width, &sh);
                    }
                    for w in self.map.walls.clone() {
                        let width = w.style.as_ref().map_or(style.width, |s| s.width);
                        self.draw_wall_shadow(painter, canvas, &map::cut_portals_wide(&w.segments().collect::<Vec<_>>(), &self.map.portals, width / 2.0), width, &sh);
                    }
                }
            }
        }
        if let Some(Sel::Edge(key, whole)) = self.sel {
            let width = self.map.edge_look(key).width;
            for (a, b) in self.edge_run(key, whole) {
                painter.line_segment([cam.to_screen(canvas, a), cam.to_screen(canvas, b)], Stroke::new(width * s + 6.0, Color32::from_rgb(120, 180, 255)));
            }
        }
        for (st, edges) in &looks {
            self.draw_walls(painter, canvas, st, edges);
        }
        let walls = self.map.walls.clone();
        for (i, wall) in walls.iter().enumerate() {
            let segs: Vec<Segment> = wall.segments().collect();
            let own = wall.style.as_ref().unwrap_or(&style);
            if self.sel == Some(Sel::Wall(i)) {
                for (a, b) in &segs {
                    painter.line_segment([cam.to_screen(canvas, *a), cam.to_screen(canvas, *b)], Stroke::new(own.width * s + 6.0, Color32::from_rgb(120, 180, 255)));
                }
            }
            self.draw_walls(painter, canvas, own, &map::cut_portals_wide(&segs, &self.map.portals, own.width / 2.0));
        }

        let portals = self.map.portals.clone();
        // Проём меряется по своей стене: у городской стены карты камень, а у домов
        // за ней фахверк — по общему виду косяки и подоконники домов раздувало.
        let edges = self.room_edges().to_vec();
        for (i, portal) in portals.iter().enumerate() {
            let c = portal.center();
            let own = walls
                .iter()
                .find(|w| !w.low && w.distance(c) < 0.06)
                .map(|w| w.style.clone().unwrap_or_else(|| style.clone()))
                .or_else(|| edges.iter().find(|s| map::segment_distance(c, s.0, s.1) < 0.06).map(|s| self.map.edge_look(map::edge_key(s))))
                .unwrap_or_else(|| style.clone());
            self.draw_portal(painter, canvas, portal, &own);
            if self.sel == Some(Sel::Portal(i)) {
                painter.line_segment([cam.to_screen(canvas, portal.a), cam.to_screen(canvas, portal.b)], Stroke::new(3.0, Color32::from_rgb(120, 180, 255)));
            }
        }

        // Изгиб: какую стену возьмёт протяжка и куда ляжет дуга стены комнаты.
        let blue = Color32::from_rgb(120, 180, 255);
        match self.bending {
            Some(Bend::Room { a, b, h }) => {
                let q: Vec<Pos2> = map::arc_points(a, b, h).iter().map(|&p| cam.to_screen(canvas, p)).collect();
                painter.add(Shape::line(q, Stroke::new(3.0, blue)));
            }
            Some(_) => {}
            None => {
                let keys = painter.ctx().input(|i| i.modifiers.command && i.modifiers.shift);
                if keys && let Some(h) = self.hover.filter(|p| canvas.contains(*p)) {
                    let target = self.bend_target(cam.to_world(canvas, h));
                    if let Some(t) = target {
                        let (a, b) = self.bend_chord(t);
                        let k = match t {
                            Bend::Wall { wall, seg } => self.map.walls[wall].bend(seg),
                            Bend::Room { .. } => 0.0,
                        };
                        let q: Vec<Pos2> = map::arc_points(a, b, k).iter().map(|&p| cam.to_screen(canvas, p)).collect();
                        painter.add(Shape::line(q, Stroke::new(3.0, blue)));
                    }
                }
            }
        }

        // Группа выбранных и рамка выбора.
        if !exporting {
            let green = Color32::from_rgb(140, 220, 160);
            for &k in &self.picked {
                if let Some(p) = self.map.props.get(k) {
                    let q: Vec<Pos2> = p.corners().iter().map(|&c| cam.to_screen(canvas, c)).collect();
                    painter.add(Shape::closed_line(q, Stroke::new(1.5, green)));
                }
            }
            if let (Some(a), Some(h)) = (self.band_from, self.hover) {
                let r = Rect::from_two_pos(cam.to_screen(canvas, a), h);
                painter.rect(r, 0.0, Color32::from_rgba_unmultiplied(140, 220, 160, 25), Stroke::new(1.0, green), egui::StrokeKind::Middle);
            }
        }

        // Ручки выбранного объекта — поверх стен и всего остального.
        if let Some(Sel::Prop(i)) = self.sel
            && matches!(self.tool, Tool::Select | Tool::Object)
        {
            let (corners, turn) = grips(&self.map.props[i], TURN_LIFT_PX / s);
            let q: Vec<Pos2> = corners.iter().map(|&p| cam.to_screen(canvas, p)).collect();
            painter.add(Shape::closed_line(q.clone(), Stroke::new(1.5, blue)));
            let t = cam.to_screen(canvas, turn);
            painter.line_segment([q[0].lerp(q[1], 0.5), t], Stroke::new(1.5, blue));
            painter.circle(t, GRIP_PX * 0.8, Color32::from_gray(30), Stroke::new(2.0, blue));
            for c in q {
                painter.rect(Rect::from_center_size(c, vec2(GRIP_PX * 1.6, GRIP_PX * 1.6)), 2.0, Color32::from_gray(30), Stroke::new(2.0, blue), egui::StrokeKind::Middle);
            }
        }

        // Призрак того, что поставит щелчок.
        let hover = self.hover.filter(|p| canvas.contains(*p));
        match (self.tool, hover) {
            (Tool::Object, Some(h)) => {
                if let Some(asset) = self.object_asset.clone() {
                    let w = cam.to_world(canvas, h);
                    let alt = painter.ctx().input(|i| i.modifiers.alt);
                    let pos = self.snap_pos(w, alt);
                    let size = self.asset_size(&asset);
                    let ghost = Prop { asset, pos, size, rot: self.object_rot, scale: self.object_scale, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None };
                    self.draw_prop(painter, canvas, &ghost, Color32::from_white_alpha(150));
                }
            }
            (Tool::Scatter, Some(h)) => {
                let alt = painter.ctx().input(|i| i.modifiers.alt);
                let color = if alt { Color32::from_rgb(255, 130, 110) } else { Color32::from_rgb(140, 220, 160) };
                painter.circle_stroke(h, self.scatter.radius * s, Stroke::new(1.5, color));
            }
            (Tool::Dungeon | Tool::Floor | Tool::Erase | Tool::Shade | Tool::Relief, Some(h)) if !self.brush.shape.drags() => {
                // Контур кисти: куда ляжет мазок.
                let r = self.brush.radius * s;
                let alt = painter.ctx().input(|i| i.modifiers.alt);
                let erase = self.tool == Tool::Erase || (self.tool == Tool::Shade && alt) || (self.tool == Tool::Relief && ((self.relief_mode == ReliefMode::Lower) != alt));
                let color = if erase { Color32::from_rgb(255, 130, 110) } else { Color32::from_rgb(120, 180, 255) };
                let stroke = Stroke::new(1.5, color);
                match self.brush.shape {
                    Tip::Square => {
                        painter.rect_stroke(Rect::from_center_size(h, vec2(2.0 * r, 2.0 * r)), 0.0, stroke, egui::StrokeKind::Middle);
                    }
                    _ => {
                        painter.circle_stroke(h, r, stroke);
                    }
                }
                if self.brush.hardness < 0.95 {
                    let inner = (r * self.brush.hardness).max(2.0);
                    painter.circle_stroke(h, inner, Stroke::new(1.0, color.gamma_multiply(0.5)));
                }
            }
            (Tool::Wall, Some(h)) => {
                painter.circle_stroke(cam.to_screen(canvas, cam.node(canvas, h)), 6.0, Stroke::new(1.5, Color32::from_rgb(255, 200, 90)));
            }
            _ => {}
        }

        // Слой «над стенами» — между стенами и крышами.
        self.draw_props(painter, canvas, view, exporting, map::ABOVE_WALLS..map::ON_ROOF, 1.0);

        // Крыши — над всем, что в доме; тень их ложится и в выгрузку, а сами
        // они уходят в Foundry отдельными тайлами.
        // Картинка для ИИ и превью — с крышами, какие ей заказали; выгрузка — без них
        // (крыши уходят в Foundry тайлами).
        let roof_alpha = match &self.export {
            Some(e) if e.picture => e.roof_alpha,
            Some(_) => 0.0,
            None => self.roof_alpha(),
        };
        // Скрытые крыши — только в редакторе: в выгрузку они идут всегда, с тенями.
        if self.roof_view != RoofView::Hidden || exporting {
            let roofs = self.map.roofs.clone();
            if shadows.on {
                for (i, r) in roofs.iter().enumerate() {
                    self.draw_roof_shadow(painter, canvas, r, &shadows, &roofs, i);
                }
            }
            if roof_alpha > 0.0 {
                self.draw_outline_roofs(painter, canvas, view, roof_alpha, shadows.light());
                if let Some(Sel::Outline(i, k)) = self.sel
                    && let Some(piece) = self.map.outline_roofs.get(i).and_then(|r| r.pieces.get(k))
                {
                    for ring in &piece.rings {
                        let q: Vec<Pos2> = ring.iter().map(|&p| cam.to_screen(canvas, p)).collect();
                        painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
                    }
                }
                for (i, r) in roofs.iter().enumerate() {
                    self.draw_roof(painter, canvas, r, roof_alpha, shadows.light());
                    if self.sel == Some(Sel::Roof(i)) {
                        let q: Vec<Pos2> = r.outline().iter().map(|&p| cam.to_screen(canvas, p)).collect();
                        painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
                    }
                }
            }
        }
        if let (Some(a), Some(p)) = (self.roof_from, self.hover.filter(|_| !exporting)) {
            let b = cam.cell(canvas, p);
            let r = Rect::from_min_max(
                cam.to_screen(canvas, [a.0.min(b.0) as f32, a.1.min(b.1) as f32]),
                cam.to_screen(canvas, [a.0.max(b.0) as f32 + 1.0, a.1.max(b.1) as f32 + 1.0]),
            );
            painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 200, 90)), egui::StrokeKind::Inside);
        }

        // Крышное — поверх крыш и тает вместе с ними: скрытые крыши — нет и его;
        // в выгрузке его нет в картинке, оно запечено в тайлы крыш.
        let on_roof = if self.roof_view == RoofView::Hidden && !exporting { 0.0 } else { roof_alpha };
        self.draw_props(painter, canvas, view, exporting, map::ON_ROOF..map::ABOVE_ROOFS, on_roof);
        self.draw_props(painter, canvas, view, exporting, map::ABOVE_ROOFS..i16::MAX, 1.0);

        // Подписи — поверх всего, с тёмной каймой, чтобы читались на любом полу.
        for (i, label) in self.map.labels.clone().iter().enumerate() {
            if exporting && label.gm {
                continue;
            }
            self.draw_label(painter, canvas, label);
            if self.sel == Some(Sel::Label(i)) && !exporting {
                let [hw, hh] = label.half_extent();
                let (sn, cs) = label.rot.to_radians().sin_cos();
                let q: Vec<Pos2> = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]]
                    .iter()
                    .map(|[x, y]| cam.to_screen(canvas, [label.pos[0] + x * cs - y * sn, label.pos[1] + x * sn + y * cs]))
                    .collect();
                painter.add(Shape::closed_line(q, Stroke::new(1.5, Color32::from_rgb(120, 180, 255))));
            }
        }

        // Выбранная дорога — осевой линией; начатая — линией до курсора.
        if let Some(Sel::Path(i)) = self.sel
            && !exporting
            && let Some(p) = self.map.paths.get(i)
        {
            let line: Vec<Pos2> = p.curve().iter().map(|&q| cam.to_screen(canvas, q)).collect();
            painter.add(Shape::line(line, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
            for q in &p.points {
                painter.circle_stroke(cam.to_screen(canvas, *q), 4.0, Stroke::new(1.5, Color32::from_rgb(120, 180, 255)));
            }
        }
        if !self.path_draft.is_empty() {
            let mut pts = self.path_draft.clone();
            if let (Some(h), false) = (hover, self.path_free) {
                pts.push(self.snap_pos(cam.to_world(canvas, h), painter.ctx().input(|i| i.modifiers.alt)));
            }
            let line: Vec<Pos2> = map::smooth_line(&pts).iter().map(|&q| cam.to_screen(canvas, q)).collect();
            let gold = Color32::from_rgb(255, 200, 90);
            painter.add(Shape::line(line, Stroke::new((self.path_style.width * s).max(3.0), gold.gamma_multiply(0.35))));
            if !self.path_free {
                for q in &self.path_draft {
                    painter.circle_filled(cam.to_screen(canvas, *q), 4.0, gold);
                }
            }
        }

        // Начатая стена и отрезок до курсора.
        let outer = (style.width * s).max(3.0);
        if !self.draft.is_empty() {
            let mut points: Vec<Pos2> = self.draft.iter().map(|&p| cam.to_screen(canvas, p)).collect();
            if let Some(h) = hover {
                points.push(cam.to_screen(canvas, cam.node(canvas, h)));
            }
            painter.add(Shape::line(points.clone(), Stroke::new(outer * 0.6, Color32::from_rgb(255, 200, 90))));
            for p in &points[..self.draft.len()] {
                painter.circle_filled(*p, 4.0, Color32::from_rgb(255, 200, 90));
            }
        }
    }

    /// Картинка, натянутая на четыре угла (в клетках).
    pub(super) fn quad(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, corners: [[f32; 2]; 4], uv: [Pos2; 4], tint: Color32) -> bool {
        let Some((tex, _)) = self.textures.get(key, false) else { return false };
        let mut mesh = egui::Mesh::with_texture(tex.id());
        for (p, uv) in corners.iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex { pos: self.cam.to_screen(canvas, *p), uv, color: tint });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
        true
    }

    pub(super) fn draw_prop(&mut self, painter: &egui::Painter, canvas: Rect, prop: &Prop, tint: Color32) {
        let c = prop.corners();
        let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        if !self.quad(painter, canvas, &prop.asset, c, full, tint) {
            // Картинка ещё грузится или не нашлась.
            let q: Vec<Pos2> = c.iter().map(|&p| self.cam.to_screen(canvas, p)).collect();
            let color = if self.textures.failed(&prop.asset, false) { Color32::from_rgb(200, 60, 60) } else { Color32::from_gray(140) };
            painter.add(Shape::convex_polygon(q, Color32::from_black_alpha(60), Stroke::new(1.0, color)));
        }
    }

    /// Дверь — картинкой вдоль проёма или простой плашкой; окно — светлой полосой.
    /// Картинка вдоль отрезка `s`→`e`: длина — отрезок, толщина — по пропорциям
    /// картинки (у FA холст створки 1×1 с доской посередине). Правый край картинки
    /// (петля створки) — у `e`. `false` — картинка ещё не готова.
    pub(super) fn draw_along(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2]) -> bool {
        self.draw_across(painter, canvas, key, s, e, None)
    }

    /// Как `draw_along`, но картинка отражена поперёк: та же сторона смотрит туда же.
    pub(super) fn draw_mirrored(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2]) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let Some((_, px)) = self.textures.get(key, false) else { return false };
        let half = len * px[1] as f32 / px[0].max(1) as f32 / 2.0;
        let (nx, ny) = (-dy / len * half, dx / len * half);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let flipped = [pos2(0.0, 1.0), pos2(1.0, 1.0), pos2(1.0, 0.0), pos2(0.0, 0.0)];
        self.quad(painter, canvas, key, c, flipped, Color32::WHITE)
    }

    /// Картинка, нарисованная стоя, вдоль отрезка: её высота идёт по отрезку.
    pub(super) fn draw_turned(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2], thick: f32) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return false;
        }
        let (nx, ny) = (-dy / len * thick / 2.0, dx / len * thick / 2.0);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let turned = [pos2(0.0, 0.0), pos2(0.0, 1.0), pos2(1.0, 1.0), pos2(1.0, 0.0)];
        self.quad(painter, canvas, key, c, turned, Color32::WHITE)
    }

    /// То же, но толщина холста поперёк отрезка задана (`thick`, клетки).
    pub(super) fn draw_across(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, s: [f32; 2], e: [f32; 2], thick: Option<f32>) -> bool {
        let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let Some((_, px)) = self.textures.get(key, false) else { return false };
        let half = thick.unwrap_or(len * px[1] as f32 / px[0].max(1) as f32) / 2.0;
        let (nx, ny) = (-dy / len * half, dx / len * half);
        let c = [[s[0] + nx, s[1] + ny], [e[0] + nx, e[1] + ny], [e[0] - nx, e[1] - ny], [s[0] - nx, s[1] - ny]];
        let full = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        self.quad(painter, canvas, key, c, full, Color32::WHITE)
    }

    /// Толщина холста подоконника поперёк стены: видимая доска — чуть толще стены,
    /// как в Dungeondraft. Нарисована FA под свои стены, у нас они тоньше.
    pub(super) fn sill_thick(&mut self, key: &str, wall_width: f32) -> f32 {
        wall_width * 1.1 / self.band(key)
    }

    /// Доля холста поперёк, занятая рисунком (подоконник, косяк, полоса стены).
    pub(super) fn band(&mut self, key: &str) -> f32 {
        let sources = &self.textures.sources;
        *self.window_bands.entry(key.to_owned()).or_insert_with(|| library::visible_box(sources, key).map_or(1.0, |[_, t, _, b]| (b - t).max(0.05)))
    }

    /// Проём: косяк в свой размер посередине, затем створка, две створки или окно.
    /// В выгрузке створок нет — они уходят в Foundry отдельной картинкой.
    pub(super) fn draw_portal(&mut self, painter: &egui::Painter, canvas: Rect, p: &Portal, wall: &map::WallStyle) {
        let wall_width = wall.width;
        let (dx, dy) = (p.b[0] - p.a[0], p.b[1] - p.a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len == 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mid = p.center();
        if p.own_window() {
            // В разрезе стены подоконник FA и рама поверх, по всей длине проёма.
            let key = p.asset.clone().unwrap_or_default();
            let thick = self.sill_thick(&key, wall_width);
            if !self.draw_across(painter, canvas, &key, p.a, p.b, Some(thick)) {
                let (nx, ny) = (-uy * wall_width * 0.3, ux * wall_width * 0.3);
                let q = [[p.a[0] + nx, p.a[1] + ny], [p.b[0] + nx, p.b[1] + ny], [p.b[0] - nx, p.b[1] - ny], [p.a[0] - nx, p.a[1] - ny]];
                let q: Vec<Pos2> = q.iter().map(|&w| self.cam.to_screen(canvas, w)).collect();
                painter.add(Shape::convex_polygon(q, Color32::from_rgb(150, 196, 220), Stroke::new(1.5, Color32::from_rgb(20, 16, 13))));
            } else {
                if let Some(f) = p.frame.clone() {
                    self.draw_across(painter, canvas, &f, p.a, p.b, Some(thick));
                }
                // Само окно — стекло с переплётом поверх подоконника.
                let assets = &self.library.assets;
                let glass = self.window_glass.entry((key.clone(), len >= 1.5)).or_insert_with(|| library::window_glass(assets, &key, len)).clone();
                if let Some(g) = glass {
                    // Часть стёкол FA нарисована стоя: такое кладётся вдоль проёма
                    // поворотом, иначе ложилось поперёк стены, ребром.
                    let standing = self.textures.get(&g, false).is_some_and(|(_, px)| px[1] > px[0]);
                    if standing {
                        self.draw_turned(painter, canvas, &g, p.a, p.b, thick);
                    } else {
                        self.draw_across(painter, canvas, &g, p.a, p.b, Some(thick));
                    }
                }
            }
            return;
        }
        if let Some(f) = p.frame.clone() {
            let size = self.asset_size(&f);
            // Косяк подобран под целую ширину; проём уже или шире — стойки те же,
            // меняется просвет между ними.
            let along = (size[0] + len - len.round()).max(len);
            let (hx, hy) = (ux * along / 2.0, uy * along / 2.0);
            // Стойки косяка ровно в толщину стены. Рисунок занимает не весь холст
            // поперёк (у FA — две пятых), и холст растягивается так, чтобы в стену
            // попала видимая часть: иначе стойки выходили тоньше стены и она торчала.
            // У стен FA камень тоже не во всю полосу — стойка по видимой стене.
            let band = self.band(&f);
            let seen = match wall.paint.clone() {
                Paint::Texture(w) => self.band(&w),
                Paint::Color(_) => 1.0,
            };
            let thick = Some(wall_width * seen / band);
            self.draw_across(painter, canvas, &f, [mid[0] - hx, mid[1] - hy], [mid[0] + hx, mid[1] + hy], thick);
        }
        let rect_around = |half: f32| {
            let (nx, ny) = (-uy * half, ux * half);
            [[p.a[0] + nx, p.a[1] + ny], [p.b[0] + nx, p.b[1] + ny], [p.b[0] - nx, p.b[1] - ny], [p.a[0] - nx, p.a[1] - ny]]
        };
        let edge = Stroke::new(1.5, Color32::from_rgb(20, 16, 13));
        let to_screen = |c: [[f32; 2]; 4], cam: &Camera| c.iter().map(|&w| cam.to_screen(canvas, w)).collect::<Vec<_>>();
        let plain = |painter: &egui::Painter, cam: &Camera, half: f32, color: Color32| {
            painter.add(Shape::convex_polygon(to_screen(rect_around(half), cam), color, edge));
        };
        let exporting = self.export.is_some();
        // В выгрузке двери — тайлы Foundry, в картинке их нет; в картинке для ИИ — есть.
        let leaves_out = exporting && !self.export.as_ref().is_some_and(|e| e.picture);
        match p.kind {
            PortalKind::Opening => {}
            PortalKind::Door | PortalKind::DoubleDoor if leaves_out => {}
            PortalKind::Door => {
                // Петля (правый край картинки) — у начала проёма: там её повесит и Foundry.
                let key = p.asset.clone();
                if !key.is_some_and(|k| self.draw_along(painter, canvas, &k, p.b, p.a)) {
                    plain(painter, &self.cam, wall_width * 0.35, Color32::from_rgb(132, 92, 54));
                }
            }
            PortalKind::DoubleDoor => {
                // Створки на петлях у краёв проёма, сходятся посередине.
                // Вторая — отражение первой: поворот на 180° показывал её другой стороной.
                let key = p.asset.clone().unwrap_or_else(|| library::DEFAULT_DOOR.into());
                self.draw_along(painter, canvas, &key, mid, p.a);
                self.draw_mirrored(painter, canvas, &key, mid, p.b);
            }
            PortalKind::Window => {
                let key = p.asset.clone();
                if !key.is_some_and(|k| self.draw_along(painter, canvas, &k, p.a, p.b)) {
                    plain(painter, &self.cam, wall_width * 0.3, Color32::from_rgb(150, 196, 220));
                }
            }
            // Тайный ход виден только ведущему: пунктир по стене, в выгрузку не попадает.
            PortalKind::Secret if exporting => {}
            PortalKind::Secret => {
                let pts: Vec<Pos2> = to_screen(rect_around(wall_width * 0.5 + 0.08), &self.cam);
                let mut ring = pts.clone();
                ring.push(pts[0]);
                painter.extend(Shape::dashed_line(&ring, Stroke::new(1.5, ACCENT), 5.0, 4.0));
            }
        }
    }
}
