//! Кисти и протяжки: пол, рельеф, стены комнат.

use super::*;

impl App {
    /// Кисти «Клетки» и «Овал»: прямоугольник клеток или овал в нём.
    pub fn paint(&mut self, a: (i32, i32), b: (i32, i32)) {
        let mut probe = self.map.clone();
        if self.tool == Tool::Relief {
            let (w, h) = (probe.rooms.w, probe.rooms.h);
            let mut weights = if self.brush.shape == Tip::Oval { mask::oval(w, h, a, b) } else { mask::cells(w, h, a, b) };
            mask::soften(w, h, &mut weights, 1.0 - self.brush.hardness);
            let base = probe.relief.clone().unwrap_or_else(|| crate::relief::Relief::for_cells(probe.width, probe.height));
            let c = [(a.0.min(b.0) + a.0.max(b.0) + 1) as f32 / 2.0, (a.1.min(b.1) + a.1.max(b.1) + 1) as f32 / 2.0];
            self.relief_target = base.at(c);
            probe.relief = Some(self.sculpt(&base, &weights));
            probe.prune();
            if probe != self.map {
                self.checkpoint();
                self.map = probe;
            }
            return;
        }
        if self.tool == Tool::Shade {
            let (w, h) = (probe.rooms.w, probe.rooms.h);
            let mut weights = if self.brush.shape == Tip::Oval { mask::oval(w, h, a, b) } else { mask::cells(w, h, a, b) };
            mask::soften(w, h, &mut weights, self.shade_soft);
            if self.shade_erase {
                probe.shade_fade(&weights);
            } else {
                probe.shade_paint(&weights, self.shade_level);
            }
            probe.prune();
            if probe != self.map {
                self.checkpoint();
                self.map = probe;
            }
            return;
        }
        let (layer, material) = match self.tool {
            Tool::Erase => (Layer::Floor, None),
            Tool::Dungeon => (Layer::Rooms, Some(self.material)),
            _ => (Layer::Floor, Some(self.material)),
        };
        let changed = if self.brush.shape == Tip::Oval {
            probe.paint_oval(layer, a, b, material)
        } else {
            probe.paint_rect(layer, a, b, material)
        };
        if changed {
            self.checkpoint();
            self.map = probe;
        }
    }

    /// Второй угол рамки; с Shift у овала — квадрат, чтобы вышел круг.
    pub(super) fn drag_corner(&self, a: (i32, i32), b: (i32, i32), shift: bool) -> (i32, i32) {
        if !(shift && self.brush.shape == Tip::Oval) {
            return b;
        }
        let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
        (a.0 + n * if b.0 < a.0 { -1 } else { 1 }, a.1 + n * if b.1 < a.1 { -1 } else { 1 })
    }

    /// Один отпечаток кисти в точке карты. В пределах мазка точка получает самую
    /// большую силу из легших на неё отпечатков, а не их сумму: иначе мягкие края
    /// соседних отпечатков складываются, середина края выпирает, а углы квадратной
    /// кисти проседают. Докладывается только прибавка: слои смешиваются через
    /// множитель (1 − сила), и прибавка `(new − old) / (1 − old)` даёт ровно `new`.
    pub(super) fn dab(&mut self, w: [f32; 2]) {
        let mut brush = self.brush;
        if self.tool == Tool::Shade {
            brush.hardness = 1.0 - self.shade_soft;
        }
        let mut weights = mask::dab(self.map.rooms.w, self.map.rooms.h, &brush, w, self.dabs);
        self.dabs = self.dabs.wrapping_add(1);
        weights.retain_mut(|(i, v)| {
            let old = self.stroke_max.get(i).copied().unwrap_or(0.0);
            if *v <= old {
                return false;
            }
            self.stroke_max.insert(*i, *v);
            *v = if old >= 1.0 { 0.0 } else { (*v - old) / (1.0 - old) };
            *v > 0.0
        });
        if self.tool == Tool::Relief {
            let base = match &self.relief_base {
                Some(b) => b.clone(),
                None => return,
            };
            let mut relief = self.map.relief.take().unwrap_or_else(|| base.clone());
            if self.relief_mode == ReliefMode::Smooth {
                relief.smooth(&weights);
            } else {
                // Высота — от того, что было до мазка, по полной силе отпечатка:
                // сколько ни води, плато не выше заданного.
                let raw: Vec<(usize, f32)> = weights.iter().map(|&(i, _)| (i, self.stroke_max.get(&i).copied().unwrap_or(0.0))).collect();
                let done = self.sculpt(&base, &raw);
                for &(i, _) in &raw {
                    relief.data[i] = done.data[i];
                }
            }
            self.map.relief = Some(relief);
            return;
        }
        match self.tool {
            Tool::Shade if self.shade_erase => self.map.shade_fade(&weights),
            Tool::Shade => {
                // Темнеет до уровня по полной силе отпечатка, а не по прибавке.
                let raw: Vec<(usize, f32)> = weights.iter().map(|&(i, _)| (i, self.stroke_max.get(&i).copied().unwrap_or(0.0))).collect();
                self.map.shade_paint(&raw, self.shade_level);
            }
            Tool::Erase => self.map.erase(&weights),
            Tool::Dungeon => self.map.paint(Layer::Rooms, self.material, &weights),
            _ => self.map.paint(Layer::Floor, self.material, &weights),
        }
        self.edges_dirty = true;
    }

    /// Высоты после мазка по весам `weights` поверх `base` — по режиму кисти.
    pub(super) fn sculpt(&self, base: &crate::relief::Relief, weights: &[(usize, f32)]) -> crate::relief::Relief {
        let mut r = base.clone();
        let mode = match (self.relief_mode, self.relief_alt) {
            (ReliefMode::Raise, true) => ReliefMode::Lower,
            (ReliefMode::Lower, true) => ReliefMode::Raise,
            (m, _) => m,
        };
        match mode {
            ReliefMode::Smooth => {
                for _ in 0..3 {
                    r.smooth(weights);
                }
            }
            _ => {
                for &(i, v) in weights {
                    let b = base.data[i];
                    r.data[i] = match mode {
                        ReliefMode::Raise => b + self.relief_height * v,
                        ReliefMode::Lower => b - self.relief_height * v,
                        _ => b + (self.relief_target - b) * v,
                    };
                }
            }
        }
        r
    }

    /// Начало мазка: одна правка в истории на весь мазок.
    pub fn begin_stroke(&mut self, w: [f32; 2]) {
        self.checkpoint();
        if self.tool == Tool::Relief {
            let base = self.map.relief.clone().unwrap_or_else(|| crate::relief::Relief::for_cells(self.map.width, self.map.height));
            self.relief_target = base.at(w);
            self.relief_base = Some(base);
        }
        self.stroke_last = Some(w);
        self.stroke_tip = w;
        self.stroke_max.clear();
        self.dab(w);
    }

    /// Ведение мазка: отпечатки через равные промежутки, чтобы след был сплошным
    /// при любой скорости мыши.
    pub fn stroke_to(&mut self, w: [f32; 2]) {
        let Some(last) = self.stroke_last else { return };
        self.stroke_tip = w;
        let gap = (self.brush.radius * if self.brush.shape == Tip::Spray { 0.8 } else { 0.3 }).max(0.08);
        let (dx, dy) = (w[0] - last[0], w[1] - last[1]);
        let d = (dx * dx + dy * dy).sqrt();
        let n = (d / gap).floor() as usize;
        for k in 1..=n {
            let t = k as f32 * gap / d;
            self.dab([last[0] + dx * t, last[1] + dy * t]);
        }
        if n > 0 {
            let t = n as f32 * gap / d;
            self.stroke_last = Some([last[0] + dx * t, last[1] + dy * t]);
        }
    }

    pub fn end_stroke(&mut self) {
        if let Some(last) = self.stroke_last {
            // Последний отпечаток — под курсором, иначе мазок короче, чем вели.
            // Спрей не добиваем: лишнее пятно в конце заметно.
            let tip = self.stroke_tip;
            if self.brush.shape != Tip::Spray && (tip[0] - last[0]).abs() + (tip[1] - last[1]).abs() > 1e-3 {
                self.dab(tip);
            }
            self.stroke_last = None;
            self.relief_base = None;
            self.map.prune();
            self.edges_dirty = true;
        }
    }

    /// Стены вокруг комнат; пересчитываются, только когда комнаты менялись.
    pub fn room_edges(&mut self) -> &[Segment] {
        if self.edges_dirty {
            let edges = self.map.room_edges();
            self.map.sync_edge_looks(&edges);
            self.edges = edges;
            self.edges_dirty = false;
        }
        &self.edges
    }

    /// Отрезки выбранной стены комнаты: прямой участок или весь её контур.
    pub(super) fn edge_run(&self, key: (i32, i32), whole: bool) -> Vec<Segment> {
        for line in mask::join(self.edges.clone()) {
            let Some(k) = line.windows(2).position(|p| map::edge_key(&(p[0], p[1])) == key) else { continue };
            let seg = |i: usize| (line[i], line[i + 1]);
            if whole {
                return (0..line.len() - 1).map(seg).collect();
            }
            let dir = |p: [f32; 2], q: [f32; 2]| {
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-6);
                [dx / l, dy / l]
            };
            let d0 = dir(line[k], line[k + 1]);
            let same = |i: usize| {
                let e = dir(line[i], line[i + 1]);
                d0[0] * e[0] + d0[1] * e[1] > 0.9995
            };
            let (mut s, mut e) = (k, k);
            while s > 0 && same(s - 1) {
                s -= 1;
            }
            while e + 2 < line.len() && same(e + 1) {
                e += 1;
            }
            return (s..=e).map(seg).collect();
        }
        Vec::new()
    }

    /// Все стоящие стены — в один вид.
    pub(super) fn apply_wall_style_to_all(&mut self) {
        self.checkpoint();
        let st = self.map.wall_style.clone();
        let edges = self.room_edges().to_vec();
        self.map.set_edge_look(&edges, &st);
        for w in &mut self.map.walls {
            w.style = None;
        }
    }

    /// Поставить начатую стену на карту. Из одной точки стены не бывает.
    pub fn finish_wall(&mut self, closed: bool) {
        let points = std::mem::take(&mut self.draft);
        if points.len() >= 2 {
            self.checkpoint();
            self.map.walls.push(Wall { closed: closed && points.len() >= 3, points, bends: Vec::new(), low: false, style: None });
            self.sel = Some(Sel::Wall(self.map.walls.len() - 1));
        }
    }
}
