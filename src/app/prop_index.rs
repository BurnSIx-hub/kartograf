//! Указатель предметов этажа: слой и рамка каждого и пространственная сетка над ними.
//! Кадр берёт из сетки только предметы возле окна и сортирует только их; раньше он
//! четыре раза за кадр перебирал и сортировал все предметы карты, а слой каждого
//! считал заново по имени файла — 10 000 ящиков стоили около 100 мс на кадр даже
//! тогда, когда в окне их было двадцать.
//!
//! Предметы двигают мышью и правят в панели, не всегда отмечая правку, поэтому
//! указатель сверяется не с номером правки, а с отпечатком самих предметов: он
//! дешёвый (числа, без строк) и меняется от любого сдвига, поворота или замены.

use super::*;
use crate::grid::{self, Grid};

/// Сторона корзины сетки, клеток: на карте 400×400 — 2500 корзин.
const BUCKET: f32 = 8.0;

#[derive(Default)]
pub(super) struct PropIndex {
    sig: Option<u64>,
    /// Слой каждого предмета, как [`Prop::layer`].
    pub layers: Vec<i16>,
    /// Сетка над рамками картинок: охватами углов ([`Prop::corners`]).
    pub grid: Grid,
}

/// Отпечаток всего, от чего зависят слой и рамка предмета.
fn signature(props: &[Prop]) -> u64 {
    let mut h: u64 = props.len() as u64 ^ 0x9e37_79b9_7f4a_7c15;
    let mut mix = |v: u64| h = (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    mix(props.as_ptr() as u64);
    for p in props {
        mix(p.pos[0].to_bits() as u64 | (p.pos[1].to_bits() as u64) << 32);
        mix(p.size[0].to_bits() as u64 | (p.size[1].to_bits() as u64) << 32);
        mix(p.rot.to_bits() as u64 | (p.scale.to_bits() as u64) << 32);
        // Ключ меняется только заменой строки — у новой строки и новое место в памяти.
        mix(p.asset.as_ptr() as u64 ^ (p.asset.len() as u64) << 48);
        mix(p.layer.map_or(1 << 20, |l| l as u16 as u64) | (p.sunk.map_or(1 << 40, |s| s.to_bits() as u64) << 21) | (p.flip as u64) << 62);
    }
    h
}

impl PropIndex {
    fn build(props: &[Prop], sig: u64) -> Self {
        let layers = props.iter().map(Prop::layer).collect();
        let boxes: Vec<grid::Rect> = props.iter().map(|p| grid::bounds(&p.corners())).collect();
        let grid = Grid::new(&boxes, BUCKET);
        Self { sig: Some(sig), layers, grid }
    }

    /// Предметы слоёв, которые берёт `keep`, чья рамка может задеть окно `r` (с
    /// запасом), — в порядке рисования: по слою, внутри слоя — как поставлены.
    pub fn near_in_order(&self, r: grid::Rect, keep: impl Fn(i16) -> bool) -> Vec<usize> {
        let mut order: Vec<usize> = self.grid.near(r).into_iter().map(|i| i as usize).filter(|&i| keep(self.layers[i])).collect();
        // Номера из сетки — по возрастанию, сортировка устойчивая: как прежняя по всем.
        order.sort_by_key(|&i| self.layers[i]);
        order
    }
}

impl App {
    /// Сверить указатель с предметами текущего этажа и пересобрать, если они поменялись.
    pub(super) fn refresh_props_index(&mut self) {
        let sig = signature(&self.map.props);
        if self.props_index.sig != Some(sig) || self.props_index.layers.len() != self.map.props.len() {
            let started = crate::plat::Instant::now();
            self.props_index = PropIndex::build(&self.map.props, sig);
            self.prof.add("props_index", started);
        }
    }

    /// Указатель этажа ниже: у него своё место, чтобы этажи не вытесняли друг друга.
    pub(super) fn swap_props_index(&mut self, other: &mut PropIndex) {
        std::mem::swap(&mut self.props_index, other);
    }

    /// Верхний предмет под точкой `w` — тот, что нарисован последним. Указатель обычно
    /// свежий (кадр его сверил); если предметы поменялись после кадра — перебор всех.
    pub(super) fn prop_under(&self, w: [f32; 2]) -> Option<usize> {
        let props = &self.map.props;
        let ix = &self.props_index;
        let order = if ix.sig == Some(signature(props)) && ix.layers.len() == props.len() {
            // Окно чуть шире точки: на краю рамки поворот туда и обратно даёт погрешность.
            ix.near_in_order(grid::grow([w[0], w[1], w[0], w[1]], 1e-3), |_| true)
        } else {
            let mut all: Vec<usize> = (0..props.len()).collect();
            all.sort_by_key(|&i| props[i].layer());
            all
        };
        order.into_iter().rev().find(|&i| props[i].contains(w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(asset: &str, pos: [f32; 2], layer: Option<i16>) -> Prop {
        Prop { asset: asset.into(), pos, size: [1.0, 1.0], rot: 15.0, scale: 1.0, shadow: None, light: None, canopy: None, layer, tint: None, open: None, glow: None, sunk: None, flip: false, inspect: None }
    }

    #[test]
    fn order_near_the_window_is_the_old_full_order_cut_to_the_window() {
        let mut props = Vec::new();
        for k in 0..2000 {
            let (x, y) = ((k * 37 % 400) as f32 + 0.3, (k * 91 % 400) as f32 + 0.7);
            let asset = match k % 4 {
                0 => "builtin:tree",
                1 => "pack:fa.dungeondraft_pack|textures/objects/Rug_Round_A.webp",
                _ => "builtin:chair",
            };
            props.push(prop(asset, [x, y], (k % 13 == 0).then_some(map::ABOVE_WALLS)));
        }
        let ix = PropIndex::build(&props, signature(&props));
        let view = [100.0, 100.0, 140.0, 130.0];
        for layers in [i16::MIN..map::ABOVE_WALLS, map::ABOVE_WALLS..map::ON_ROOF, map::ON_ROOF..map::ABOVE_ROOFS] {
            // Как было: все предметы слоёв, по слою, и из них — задевающие окно.
            let mut old: Vec<usize> = (0..props.len()).filter(|&i| layers.contains(&props[i].layer())).collect();
            old.sort_by_key(|&i| props[i].layer());
            old.retain(|&i| grid::touch(&grid::bounds(&props[i].corners()), &view));
            let mut new = ix.near_in_order(view, |l| layers.contains(&l));
            new.retain(|&i| grid::touch(&grid::bounds(&props[i].corners()), &view));
            assert_eq!(new, old, "{layers:?}");
        }
    }

    #[test]
    fn any_move_turn_or_new_asset_changes_the_signature() {
        // Правки на месте, в том же списке: так их делают мышь и панель.
        let mut props = vec![prop("builtin:chair", [1.0, 1.0], None), prop("builtin:tree", [5.0, 5.0], None)];
        let mut last = signature(&props);
        assert_eq!(signature(&props), last, "без правок — тот же");
        let edits: [(&str, fn(&mut Vec<Prop>)); 6] = [
            ("сдвиг", |p| p[1].pos[0] += 0.01),
            ("поворот", |p| p[0].rot += 1.0),
            ("масштаб", |p| p[0].scale *= 1.1),
            ("ассет", |p| p[0].asset = "builtin:barrel".into()),
            ("слой", |p| p[0].layer = Some(map::ABOVE_WALLS)),
            ("в воду", |p| p[1].sunk = Some(1.0)),
        ];
        for (what, edit) in edits {
            edit(&mut props);
            let now = signature(&props);
            assert_ne!(now, last, "{what}");
            last = now;
        }
    }
}
