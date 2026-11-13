//! Отрисовка: надписи, тени, рельеф, дороги, предметы, стены.

use super::*;

impl App {
    /// Подпись: текст по центру своей точки, повёрнутый, с каймой из восьми
    /// тёмных копий вокруг.
    pub(super) fn draw_label(&self, painter: &egui::Painter, canvas: Rect, label: &map::Label) {
        let px = label.size * self.cam.scale;
        if px < 3.0 || label.text.is_empty() {
            return;
        }
        let font = egui::FontId::proportional(px);
        let color = Color32::from_rgb(label.color[0], label.color[1], label.color[2]);
        let galley = painter.layout(label.text.clone(), font.clone(), color, f32::INFINITY);
        let dark = painter.layout(label.text.clone(), font, Color32::from_black_alpha(200), f32::INFINITY);
        let c = self.cam.to_screen(canvas, label.pos);
        let angle = label.rot.to_radians();
        let half = galley.size() / 2.0;
        // Угол текста — левый верхний; поворот — вокруг него, так что середину
        // сдвигаем повёрнутой половиной размера.
        let rot = egui::emath::Rot2::from_angle(angle);
        let origin = c - rot * half;
        let ring = (px * 0.07).max(1.0);
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let o = vec2(a.cos(), a.sin()) * ring;
            painter.add(egui::epaint::TextShape::new(origin + o, dark.clone(), Color32::BLACK).with_angle(angle));
        }
        painter.add(egui::epaint::TextShape::new(origin, galley, color).with_angle(angle));
    }

    /// Тень объекта: размытый силуэт той же картинки, сдвинутый от света.
    pub(super) fn draw_prop_shadow(&mut self, painter: &egui::Painter, canvas: Rect, prop: &Prop, sh: &map::Shadows) {
        let blur = ((sh.blur * 10.0).round() as u32).clamp(1, 10);
        let key = format!("{}{blur}:{}", library::SHADOW, prop.asset);
        let Some((tex, info)) = self.textures.get(&key, false) else { return };
        let (id, [tw, th]) = (tex.id(), tex.size());
        let (pad, sw) = (info[0] as f32, info[1].max(1) as f32);
        let sh_h = (th as f32 - 2.0 * pad).max(1.0);
        let _ = tw;
        let (w, h) = (prop.size[0] * prop.scale, prop.size[1] * prop.scale);
        let (hw, hh) = (w / 2.0 + pad / sw * w, h / 2.0 + pad / sh_h * h);
        let (s, c) = prop.rot.to_radians().sin_cos();
        let o = sh.offset();
        let corners = [[-hw, -hh], [hw, -hh], [hw, hh], [-hw, hh]].map(|[x, y]| [prop.pos[0] + x * c - y * s + o[0], prop.pos[1] + x * s + y * c + o[1]]);
        let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        let tint = Color32::from_white_alpha((sh.opacity * 255.0).round() as u8);
        let mut mesh = egui::Mesh::with_texture(id);
        for (p, uv) in corners.iter().zip(uv) {
            mesh.vertices.push(egui::epaint::Vertex { pos: self.cam.to_screen(canvas, *p), uv, color: tint });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
    }

    /// Тень стен: лента шире стены на размытие, сдвинутая от света; края тают.
    pub(super) fn draw_wall_shadow(&mut self, painter: &egui::Painter, canvas: Rect, segs: &[Segment], width: f32, sh: &map::Shadows) {
        // Стена нулевой толщины — невидимая (перила пролёта для VTT): и тени нет.
        if width <= 0.0 {
            return;
        }
        let cam = self.cam;
        let o = sh.offset();
        let (outer_hw, inner_hw) = (width / 2.0 + sh.blur / 2.0, (width / 2.0 - sh.blur / 2.0).max(0.01));
        let dark = Color32::from_black_alpha((sh.opacity * 255.0).round() as u8);
        let mut mesh = egui::Mesh::default();
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let rows = shadow_rows(&line, closed, outer_hw, inner_hw);
            if rows.len() < 2 {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for row in &rows {
                for (p, solid) in row {
                    let q = [p[0] + o[0], p[1] + o[1]];
                    let color = if *solid { dark } else { Color32::TRANSPARENT };
                    mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, q), uv: egui::epaint::WHITE_UV, color });
                }
            }
            for k in 0..rows.len() as u32 - 1 {
                let (p, q) = (base + k * 4, base + (k + 1) * 4);
                for r in 0..3 {
                    mesh.add_triangle(p + r, p + r + 1, q + r + 1);
                    mesh.add_triangle(p + r, q + r + 1, q + r);
                }
            }
        }
        if !mesh.is_empty() {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Дорога лентой вдоль плавной линии: четыре ряда вершин поперёк — внешний край
    /// прозрачный, внутренние сплошные, между ними край тает. Ширина каждой стороны
    /// гуляет по своему шуму, как берег. Текстура — по сетке, как пол, или вдоль.
    /// Свет, тень и обрывы рельефа; считаются заново, только когда поменялись
    /// высоты или солнце.
    /// Тени крон: считаются заново, только когда сменились деревья, крыши,
    /// солнце или нарисованная тень.
    pub(super) fn tree_shade(&mut self) -> Option<std::rc::Rc<Mask>> {
        use std::hash::{Hash, Hasher};
        let m = &self.map;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (m.rooms.w, m.rooms.h, m.shadows.on, m.shadows.angle.to_bits()).hash(&mut h);
        for p in m.props.iter().filter(|p| p.is_canopy()) {
            p.asset.hash(&mut h);
            (p.shadow, p.canopy).hash(&mut h);
            for v in [p.pos[0], p.pos[1], p.size[0], p.size[1], p.scale] {
                v.to_bits().hash(&mut h);
            }
        }
        for r in &m.roofs {
            for v in [r.a[0], r.a[1], r.b[0], r.b[1]] {
                v.to_bits().hash(&mut h);
            }
        }
        m.outline_roofs.len().hash(&mut h);
        for r in &m.outline_roofs {
            let (lo, hi) = r.bounds();
            for v in [lo[0], lo[1], hi[0], hi[1]] {
                v.to_bits().hash(&mut h);
            }
        }
        m.shade.as_ref().map(|s| &s.data).hash(&mut h);
        let key = h.finish();
        if let Some((k, trees)) = &self.tree_shade
            && *k == key
        {
            return trees.clone();
        }
        let trees = roofs::tree_shade(m).map(std::rc::Rc::new);
        self.tree_shade = Some((key, trees.clone()));
        trees
    }

    pub(super) fn relief_look(&mut self) -> Option<std::rc::Rc<crate::relief::Look>> {
        use std::hash::{Hash, Hasher};
        let r = self.map.relief.as_ref()?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (r.w, r.h).hash(&mut h);
        for v in &r.data {
            v.to_bits().hash(&mut h);
        }
        self.map.shadows.angle.to_bits().hash(&mut h);
        for p in &self.map.paths {
            p.width.to_bits().hash(&mut h);
            for q in &p.points {
                (q[0].to_bits(), q[1].to_bits()).hash(&mut h);
            }
        }
        let key = h.finish();
        if let Some((k, look)) = &self.relief_look
            && *k == key
        {
            return Some(look.clone());
        }
        // Гребень под дорогой и мостом гаснет: тропа идёт через уступ поверх него;
        // тени и затемнение там — от плавного съезда, а не от ступени.
        let roads: Vec<(Vec<[f32; 2]>, f32)> = self.map.paths.iter().map(|p| (p.curve(), p.width / 2.0)).collect();
        let look = crate::relief::look(r, self.map.shadows.angle, &roads);
        let look = std::rc::Rc::new(look);
        self.relief_look = Some((key, look.clone()));
        Some(look)
    }

    /// Полоса рельефа вдоль кромки: от `inner` до `outer` клеток вниз по склону
    /// (вправо по ходу кромки). Верх картинки — к кромке, как у полос FA.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_relief_strip(&mut self, painter: &egui::Painter, canvas: Rect, key: &str, e: &crate::relief::Edge, inner: &[f32], outer: &[f32], fade: &[f32], view: Rect) {
        let n = e.points.len();
        if n < 2 {
            return;
        }
        let reach = outer.iter().copied().fold(0.0f32, f32::max) + 1.0;
        if !e.points.iter().any(|p| view.expand(reach).contains(pos2(p[0], p[1]))) {
            return;
        }
        let Some((tex, px)) = self.textures.get(key, false).map(|(t, px)| (t.id(), px)) else { return };
        let aspect = px[0] as f32 / px[1].max(1) as f32;
        let mean = (0..n).map(|k| outer[k] - inner[k]).sum::<f32>() / n as f32;
        let mut repeat = (aspect * mean.max(0.2)).max(0.5);
        let total: f32 = e.points.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>()
            + if e.closed { (e.points[0][0] - e.points[n - 1][0]).hypot(e.points[0][1] - e.points[n - 1][1]) } else { 0.0 };
        if e.closed {
            repeat = total / (total / repeat).round().max(1.0);
        }
        let cam = self.cam;
        let mut mesh = egui::Mesh::with_texture(tex);
        let mut along = 0.0;
        let count = if e.closed { n + 1 } else { n };
        for k in 0..count {
            let i = k % n;
            if k > 0 {
                let q = e.points[(k - 1) % n];
                along += (e.points[i][0] - q[0]).hypot(e.points[i][1] - q[1]);
            }
            let p = e.points[i];
            let up = e.smooth_up(i, 4);
            let at = |d: f32| cam.to_screen(canvas, [p[0] - up[0] * d, p[1] - up[1] * d]);
            let u = along / repeat;
            // Гаснущая полоса гаснет сперва снизу: у конца не прямой срез поперёк,
            // а косой — стенка ниже и ниже уходит в склон.
            let f = fade[i].clamp(0.0, 1.0);
            let alpha = |v: f32| Color32::from_white_alpha((v * 255.0).round() as u8);
            mesh.vertices.push(egui::epaint::Vertex { pos: at(inner[i]), uv: pos2(u, 0.0), color: alpha(f) });
            mesh.vertices.push(egui::epaint::Vertex { pos: at(outer[i]), uv: pos2(u, 1.0), color: alpha(f * f * f) });
            if k > 0 {
                let b = (k as u32 - 1) * 2;
                mesh.add_triangle(b, b + 1, b + 3);
                mesh.add_triangle(b, b + 3, b + 2);
            }
        }
        painter.add(Shape::mesh(mesh));
    }

    pub(super) fn draw_path(&mut self, painter: &egui::Painter, canvas: Rect, path: &map::Path, seed: u32) {
        let Some(material) = self.map.materials.get(path.material) else { return };
        let (tex, tile, tint) = match &material.paint {
            Paint::Color(c) => (None, [1.0, 1.0], rgb(*c)),
            Paint::Texture(key) => {
                let key = if material.vary && !path.along { format!("{}{key}", library::VARY) } else { key.clone() };
                match self.textures.get(&key, false) {
                    Some((t, px)) => (Some(t.id()), [(px[0] as f32 / PX_PER_CELL).max(MIN_TILE), (px[1] as f32 / PX_PER_CELL).max(MIN_TILE)], Color32::WHITE),
                    None => (None, [1.0, 1.0], Color32::from_gray(60)),
                }
            }
        };
        let organic = tex.is_some() && material.vary && !path.along;
        let line = path.curve();
        if line.len() < 2 {
            return;
        }
        let mut dist = vec![0.0f32; line.len()];
        for k in 1..line.len() {
            dist[k] = dist[k - 1] + ((line[k][0] - line[k - 1][0]).powi(2) + (line[k][1] - line[k - 1][1]).powi(2)).sqrt();
        }
        let total = dist[line.len() - 1].max(1e-4);
        let hw = path.width / 2.0;
        // Край не тоньше полутора точек экрана, иначе он лесенкой.
        let fade = (hw * path.soft).max(1.5 / self.cam.scale).min(hw);
        let cam = self.cam;
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        for k in 0..line.len() {
            let (a, b) = (line[k.saturating_sub(1)], line[(k + 1).min(line.len() - 1)]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let n = [-dy / len, dx / len];
            let d = dist[k];
            let side = |salt: u32| hw * (1.0 + path.rough * 0.8 * (value_noise(d / (0.9 + hw * 0.5), seed.wrapping_mul(2).wrapping_add(salt)) - 0.5));
            let (wl, wr) = (side(0), side(1));
            // На концах дорога тает так же, как по краю.
            let end = ((d.min(total - d)) / fade).clamp(0.0, 1.0);
            let c = line[k];
            let rows = [(wl, 0.0), (wl - fade, 1.0), (-(wr - fade), 1.0), (-wr, 0.0)];
            for (off, a) in rows {
                let p = [c[0] + n[0] * off, c[1] + n[1] * off];
                let uv = match tex {
                    None => egui::epaint::WHITE_UV,
                    Some(_) if path.along => pos2(d / tile[0], (hw - off) / tile[1]),
                    Some(_) => pos2(p[0] / tile[0], p[1] / tile[1]),
                };
                let alpha = (a * end * 255.0).round() as u8;
                let color = if tex.is_some() { Color32::from_white_alpha(alpha) } else { Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), alpha) };
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
            }
            if k > 0 {
                let (p, q) = (((k - 1) * 4) as u32, (k * 4) as u32);
                for r in 0..3 {
                    mesh.add_triangle(p + r, p + r + 1, q + r + 1);
                    mesh.add_triangle(p + r, q + r + 1, q + r);
                }
            }
        }
        if organic {
            self.blend.paint(painter, mesh);
        } else {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Стены лентой: отрезки склеиваются в ломаные, у каждой вершины лента
    /// поворачивает по биссектрисе — без нахлёстов и швов. Обводка — по двум краям
    /// ленты, текстура идёт вдоль без разрывов; на замкнутой стене её шаг подогнан,
    /// чтобы целое число повторов сошлось на стыке.
    /// Порядок рисования объектов: по слою, внутри слоя — как поставлены.
    pub(super) fn prop_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.map.props.len()).collect();
        order.sort_by_key(|&i| self.map.props[i].layer());
        order
    }

    /// Объекты слоёв из `layers`: сначала все их тени, потом сами.
    /// `fade` — общая прозрачность (крышное тает вместе с крышей); с ней и без тени.
    pub(super) fn draw_props(&mut self, painter: &egui::Painter, canvas: Rect, view: Rect, exporting: bool, layers: std::ops::Range<i16>, fade: f32) {
        if fade <= 0.0 {
            return;
        }
        let cam = self.cam;
        let mut order: Vec<usize> = (0..self.map.props.len()).filter(|&i| layers.contains(&self.map.props[i].layer())).collect();
        if order.is_empty() {
            return;
        }
        order.sort_by_key(|&i| self.map.props[i].layer());
        // Объекты на время рисования вынуты из карты, а не скопированы: рисованию
        // нужен `&mut self` (картинки грузятся), а карту оно не читает.
        let props = std::mem::take(&mut self.map.props);
        let shadows = self.map.shadows.clone();
        if shadows.on && fade >= 1.0 {
            // Тень кроны — своя, сдвинутая на высоту дерева ([`roofs::tree_shade`]).
            for prop in order.iter().map(|&i| &props[i]).filter(|p| p.casts_shadow() && !p.is_canopy()) {
                let c = prop.corners();
                let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1]))).expand(shadows.dist + shadows.blur * 2.0);
                if bounds.intersects(view) {
                    for sh in shadows.layers() {
                        self.draw_prop_shadow(painter, canvas, prop, &sh);
                    }
                }
            }
        }
        for i in order {
            let prop = &props[i];
            let c = prop.corners();
            let bounds = Rect::from_points(&c.map(|p| pos2(p[0], p[1])));
            if !bounds.intersects(view) {
                continue;
            }
            let up = exporting && self.export.as_ref().is_some_and(|e| e.canopies);
            if up && !prop.is_canopy() && (map::bush_by_name(&prop.asset) || map::awning_by_name(&prop.asset) || map::line_by_name(&prop.asset) || prop.own_tile()) {
                // Куст, навес, бельё и всякий предмет уйдут в свой тайл целиком; тень — в фоне.
            } else if exporting && self.export.as_ref().is_some_and(|e| e.canopies) && prop.is_canopy() && map::is_bare_tree(&prop.asset) {
                // Голое дерево целиком уйдёт в тайл; пень под ветками смотрится плохо.
            } else if exporting && self.export.as_ref().is_some_and(|e| e.canopies) && prop.is_canopy() {
                // Крона уйдёт в свой тайл: под ней в картинке пень.
                // Пень размером со ствол, а не со всю крону.
                let asset = self.export.as_ref().map(|e| &e.stumps).filter(|s| !s.is_empty()).map_or_else(
                    || library::STUMP.to_owned(),
                    // Вариант пня от места дерева: тот же на каждой плитке выгрузки.
                    |s| s[(prop.pos[0] * 7.0 + prop.pos[1] * 13.0).abs() as usize % s.len()].clone(),
                );
                let stump = Prop { asset, size: [3.0, 3.0], scale: prop.scale * prop.size[0].max(prop.size[1]) * library::STUMP_SHARE / 3.0, ..prop.clone() };
                self.draw_prop(painter, canvas, &stump, Color32::WHITE);
            } else {
                let tint = prop.tint.map_or(Color32::WHITE, |[r, g, b]| Color32::from_rgb(r, g, b));
                self.draw_prop(painter, canvas, prop, if fade < 1.0 { tint.gamma_multiply(fade) } else { tint });
            }
            if self.sel == Some(Sel::Prop(i)) {
                let q: Vec<Pos2> = c.iter().map(|&p| cam.to_screen(canvas, p)).collect();
                painter.add(Shape::closed_line(q, Stroke::new(2.0, Color32::from_rgb(120, 180, 255))));
            }
        }
        self.map.props = props;
    }

    pub(super) fn draw_walls(&mut self, painter: &egui::Painter, canvas: Rect, style: &map::WallStyle, segs: &[Segment]) {
        if style.width <= 0.0 {
            return;
        }
        let cam = self.cam;
        let hw = style.width / 2.0;
        let edge = Stroke::new(1.5, Color32::from_rgb(20, 16, 13));
        let (tex, aspect, fill) = match &style.paint {
            Paint::Color(c) => (None, 4.0, rgb(*c)),
            Paint::Texture(key) => match self.textures.get(key, false) {
                Some((t, px)) => (Some(t.id()), px[0] as f32 / px[1].max(1) as f32, Color32::WHITE),
                None => (None, 4.0, Color32::from_gray(70)),
            },
        };
        if let (Some(id), Paint::Texture(key)) = (tex, &style.paint) {
            if let Some(n) = library::builtin_wall_stamps(key) {
                self.draw_stamped(painter, canvas, style.width, id, n, segs);
                return;
            }
        }
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        // Заливка ляжет под обводку: место под неё занято заранее.
        let slot = painter.add(Shape::Noop);
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let rows = ribbon(&line, closed, hw);
            if rows.len() < 2 {
                continue;
            }
            let total = rows[rows.len() - 1].2;
            let mut repeat = style.width * aspect;
            if closed {
                repeat = total / (total / repeat).round().max(1.0);
            }
            // У каждого участка свои вершины. На скосе угла внешняя и внутренняя точки
            // ряда не напротив друг друга: с общей координатой по длине участок —
            // трапеция из двух треугольников, и текстура в ней ломалась ёлочкой.
            // Координата — проекция точки на направление участка: внутри участка
            // рисунок лежит ровно, а на углу срезается по скосу, как в Dungeondraft.
            for k in 0..rows.len() - 1 {
                let ((l0, r0, d0), (l1, r1, _)) = (rows[k], rows[k + 1]);
                let mid = |l: [f32; 2], r: [f32; 2]| [(l[0] + r[0]) / 2.0, (l[1] + r[1]) / 2.0];
                let (c0, c1) = (mid(l0, r0), mid(l1, r1));
                let (dx, dy) = (c1[0] - c0[0], c1[1] - c0[1]);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let t = [dx / len, dy / len];
                let along = |p: [f32; 2]| d0 + (p[0] - c0[0]) * t[0] + (p[1] - c0[1]) * t[1];
                let base = mesh.vertices.len() as u32;
                for (p, v) in [(l0, 0.0), (r0, 1.0), (l1, 0.0), (r1, 1.0)] {
                    let uv = if tex.is_some() { pos2(along(p) / repeat, v) } else { egui::epaint::WHITE_UV };
                    mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color: fill });
                }
                mesh.add_triangle(base, base + 1, base + 3);
                mesh.add_triangle(base, base + 3, base + 2);
            }
            // У текстурной стены обводка нарисована в самой полосе (как в паках FA),
            // и у рваной кромки прямая линия поверх только мешает.
            if tex.is_some() {
                continue;
            }
            let left: Vec<Pos2> = rows.iter().map(|(l, _, _)| cam.to_screen(canvas, *l)).collect();
            let right: Vec<Pos2> = rows.iter().map(|(_, r, _)| cam.to_screen(canvas, *r)).collect();
            if closed {
                painter.add(Shape::closed_line(left[..left.len() - 1].to_vec(), edge));
                painter.add(Shape::closed_line(right[..right.len() - 1].to_vec(), edge));
            } else {
                // Открытая — одним контуром: левый край, торец, правый край назад.
                let mut outline = left;
                outline.extend(right.into_iter().rev());
                painter.add(Shape::closed_line(outline, edge));
            }
        }
        if !mesh.is_empty() {
            painter.set(slot, Shape::mesh(mesh));
        }
    }

    /// Стена из отдельных штук — брёвен частокола. Полоса из кругов на изгибе
    /// сжимается и режет брёвна, поэтому каждое бревно — свой квадрат: шаг вдоль
    /// стены ровный, в каждом остром углу стоит бревно. Под ними тёмная лента,
    /// чтобы между кругами не просвечивал пол.
    pub(super) fn draw_stamped(&mut self, painter: &egui::Painter, canvas: Rect, width: f32, id: egui::TextureId, n: u32, segs: &[Segment]) {
        let cam = self.cam;
        let mut under = egui::Mesh::default();
        let mut logs = egui::Mesh::with_texture(id);
        let bark = Color32::from_rgb(30, 21, 14);
        for line in mask::join(segs.to_vec()) {
            let closed = line.len() > 3 && (line[0][0] - line[line.len() - 1][0]).abs() < 1e-3 && (line[0][1] - line[line.len() - 1][1]).abs() < 1e-3;
            let rows = ribbon(&line, closed, width * 0.28);
            for k in 0..rows.len().saturating_sub(1) {
                let base = under.vertices.len() as u32;
                for p in [rows[k].0, rows[k].1, rows[k + 1].0, rows[k + 1].1] {
                    under.colored_vertex(cam.to_screen(canvas, p), bark);
                }
                under.add_triangle(base, base + 1, base + 3);
                under.add_triangle(base, base + 3, base + 2);
            }
            let r = width / 2.0;
            for c in stamp_points(&line, closed, width * 0.9) {
                // Поворот и вид бревна — от места: при правке соседних стен не прыгают.
                let h = ((c[0] * 73.13).round() as i64).wrapping_mul(31).wrapping_add((c[1] * 91.7).round() as i64).wrapping_mul(2654435761) as u64;
                let a = (h % 6283) as f32 / 1000.0;
                let v = ((h >> 16) % n as u64) as f32;
                let (ax, ay) = ([a.cos() * r, a.sin() * r], [-a.sin() * r, a.cos() * r]);
                let base = logs.vertices.len() as u32;
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    let p = [c[0] + ax[0] * sx + ay[0] * sy, c[1] + ax[1] * sx + ay[1] * sy];
                    let uv = pos2((v + (sx + 1.0) / 2.0) / n as f32, (sy + 1.0) / 2.0);
                    logs.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color: Color32::WHITE });
                }
                logs.add_triangle(base, base + 1, base + 3);
                logs.add_triangle(base, base + 3, base + 2);
            }
        }
        painter.add(Shape::mesh(under));
        painter.add(Shape::mesh(logs));
    }
}
