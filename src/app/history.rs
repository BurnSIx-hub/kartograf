//! История правок и этажи.

use super::*;

impl App {
    /// Запомнить карту перед правкой.
    pub fn checkpoint(&mut self) {
        // В пачке правок (`batch`) этаж запоминается один раз — до первой правки.
        if let Some(seen) = &mut self.batch {
            if seen.contains(&self.level) {
                self.redo.clear();
                self.touch();
                return;
            }
            seen.push(self.level);
        }
        self.undo.push((self.level, self.map.clone()));
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.touch();
    }

    pub(super) fn touch(&mut self) {
        self.dirty = true;
        self.rev += 1;
        self.edges_dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some((level, prev)) = self.undo.pop() {
            // Правка была на другом этаже — сначала туда.
            self.switch_level(level);
            self.redo.push((level, std::mem::replace(&mut self.map, prev)));
            self.after_history();
        }
    }

    pub fn redo(&mut self) {
        if let Some((level, next)) = self.redo.pop() {
            self.switch_level(level);
            self.undo.push((level, std::mem::replace(&mut self.map, next)));
            self.after_history();
        }
    }

    /// Перейти на этаж `k`: живая карта уходит на своё место в списке.
    pub fn switch_level(&mut self, k: usize) {
        if k == self.level || k >= self.levels.len() {
            return;
        }
        std::mem::swap(&mut self.map, &mut self.levels[self.level]);
        std::mem::swap(&mut self.map, &mut self.levels[k]);
        self.level = k;
        self.sel = None;
        self.picked.clear();
        self.draft.clear();
        self.path_draft.clear();
        self.moving = None;
        self.edges_dirty = true;
        self.material = self.material.min(self.map.materials.len().saturating_sub(1));
        self.refresh_below();
    }

    /// Карта для файла: первый этаж, остальные — в его `levels`.
    pub fn project(&self) -> Map {
        let mut all: Vec<Map> = self.levels.iter().enumerate().map(|(k, m)| if k == self.level { self.map.clone() } else { m.clone() }).collect();
        let mut root = all.remove(0);
        root.levels = all;
        root
    }

    /// Новый этаж выше текущего (`above`) или ниже; история правок сбрасывается.
    pub fn add_level(&mut self, above: bool) {
        let fresh = self.map.new_level();
        let at = if above { self.level + 1 } else { self.level };
        self.levels[self.level] = self.map.clone();
        self.levels.insert(at, fresh);
        self.level = if above { self.level } else { self.level + 1 };
        self.undo.clear();
        self.redo.clear();
        self.switch_level(at);
        self.touch();
    }

    pub fn remove_level(&mut self) {
        if self.levels.len() < 2 {
            return;
        }
        self.levels.remove(self.level);
        self.level = self.level.min(self.levels.len() - 1);
        self.map = self.levels[self.level].clone();
        self.undo.clear();
        self.redo.clear();
        self.sel = None;
        self.edges_dirty = true;
        self.refresh_below();
        self.touch();
    }

    pub(super) fn refresh_below(&mut self) {
        let k = self.level;
        self.below = if k > 0 { self.levels[k - 1].room_edges().into_iter().chain(self.levels[k - 1].walls.iter().flat_map(|w| w.segments().collect::<Vec<_>>())).collect() } else { Vec::new() };
    }

    pub fn level_name(&self, k: usize) -> String {
        let m = if k == self.level { &self.map } else { &self.levels[k] };
        if m.name.trim().is_empty() { format!("Этаж {}", k + 1) } else { m.name.clone() }
    }

    pub(super) fn after_history(&mut self) {
        self.sel = self.sel.filter(|s| self.sel_exists(*s));
        self.material = self.material.min(self.map.materials.len().saturating_sub(1));
        self.draft.clear();
        self.path_draft.clear();
        self.moving = None;
        self.touch();
    }

    pub(super) fn sel_exists(&self, s: Sel) -> bool {
        match s {
            Sel::Wall(i) => i < self.map.walls.len(),
            Sel::Edge(..) => true,
            Sel::Portal(i) => i < self.map.portals.len(),
            Sel::Prop(i) => i < self.map.props.len(),
            Sel::Path(i) => i < self.map.paths.len(),
            Sel::Label(i) => i < self.map.labels.len(),
            Sel::Roof(i) => i < self.map.roofs.len(),
            Sel::Outline(i, k) => self.map.outline_roofs.get(i).is_some_and(|r| k < r.pieces.len()),
        }
    }
}
