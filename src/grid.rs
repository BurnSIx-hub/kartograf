//! Пространственная сетка: какие прямоугольники лежат возле места. Карта делится на
//! корзины со стороной `cell` клеток; прямоугольник записан в каждую корзину, которую
//! задевает. Запрос перебирает только корзины своего окна, а не все предметы карты:
//! кадр, выбор мышью и проверка не зависят от того, сколько всего стоит на карте.
//!
//! Ответ — с запасом (всё, что в задетых корзинах): точную проверку пересечения
//! делает тот, кто спрашивает, тем же условием, что и без сетки.

/// Прямоугольник `[x0, y0, x1, y1]` в клетках.
pub type Rect = [f32; 4];

/// Больше корзин сетка не заводит: предметы далеко за краем карты не раздувают её —
/// корзины тогда крупнее.
const MAX_BUCKETS: usize = 1 << 20;

#[derive(Clone, Debug, Default)]
pub struct Grid {
    cell: f32,
    x0: f32,
    y0: f32,
    w: usize,
    h: usize,
    /// Корзина `k` — номера `items[starts[k]..starts[k + 1]]`, по возрастанию.
    starts: Vec<u32>,
    items: Vec<u32>,
    /// Прямоугольники с нечислами (NaN, бесконечность): отдаются любому запросу.
    odd: Vec<u32>,
}

impl Grid {
    /// Сетка над прямоугольниками `boxes` (номер прямоугольника — его место в срезе).
    pub fn new(boxes: &[Rect], cell: f32) -> Self {
        let finite = |b: &Rect| b.iter().all(|v| v.is_finite());
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for b in boxes.iter().filter(|b| finite(b)) {
            lo = [lo[0].min(b[0].min(b[2])), lo[1].min(b[1].min(b[3]))];
            hi = [hi[0].max(b[0].max(b[2])), hi[1].max(b[1].max(b[3]))];
        }
        let odd: Vec<u32> = (0..boxes.len()).filter(|&i| !finite(&boxes[i])).map(|i| i as u32).collect();
        if lo[0] > hi[0] {
            return Self { cell: cell.max(1e-3), w: 0, h: 0, starts: vec![0], odd, ..Self::default() };
        }
        let mut cell = cell.max(1e-3);
        let span = |c: f32| (((hi[0] - lo[0]) / c).floor() as usize + 1, ((hi[1] - lo[1]) / c).floor() as usize + 1);
        let (mut w, mut h) = span(cell);
        while w * h > MAX_BUCKETS {
            cell *= 2.0;
            (w, h) = span(cell);
        }
        let mut grid = Self { cell, x0: lo[0], y0: lo[1], w, h, starts: vec![0; w * h + 1], items: Vec::new(), odd };
        // Два прохода: сколько в каждой корзине, затем раскладка подряд.
        for b in boxes.iter().filter(|b| finite(b)) {
            let (i0, j0, i1, j1) = grid.span_of(b);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    grid.starts[j * w + i + 1] += 1;
                }
            }
        }
        for k in 0..w * h {
            grid.starts[k + 1] += grid.starts[k];
        }
        let mut fill = grid.starts.clone();
        grid.items = vec![0; grid.starts[w * h] as usize];
        for (n, b) in boxes.iter().enumerate().filter(|(_, b)| finite(b)) {
            let (i0, j0, i1, j1) = grid.span_of(b);
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let k = j * w + i;
                    grid.items[fill[k] as usize] = n as u32;
                    fill[k] += 1;
                }
            }
        }
        grid
    }

    /// Корзины, которые задевает прямоугольник, — в пределах сетки.
    fn span_of(&self, b: &Rect) -> (usize, usize, usize, usize) {
        let at = |v: f32, o: f32, n: usize| (((v - o) / self.cell).floor().max(0.0) as usize).min(n.saturating_sub(1));
        (at(b[0].min(b[2]), self.x0, self.w), at(b[1].min(b[3]), self.y0, self.h), at(b[0].max(b[2]), self.x0, self.w), at(b[1].max(b[3]), self.y0, self.h))
    }

    /// Номера прямоугольников возле `r` — по возрастанию, без повторов, с запасом.
    pub fn near(&self, r: Rect) -> Vec<u32> {
        let mut out = self.odd.clone();
        if self.w > 0 && r.iter().all(|v| !v.is_nan()) {
            let (x0, y0, x1, y1) = (r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3]));
            // Окно целиком мимо сетки — пусто (с краями: корзины считаются замкнутыми).
            let right = self.x0 + self.cell * self.w as f32;
            let bottom = self.y0 + self.cell * self.h as f32;
            if x1 >= self.x0 && y1 >= self.y0 && x0 <= right && y0 <= bottom {
                let (i0, j0, i1, j1) = self.span_of(&[x0, y0, x1, y1]);
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let k = j * self.w + i;
                        out.extend_from_slice(&self.items[self.starts[k] as usize..self.starts[k + 1] as usize]);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Прямоугольник, охватывающий точки.
pub fn bounds(points: &[[f32; 2]]) -> Rect {
    points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// Пересекаются ли прямоугольники (касание — тоже), как `egui::Rect::intersects`.
pub fn touch(a: &Rect, b: &Rect) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// Прямоугольник, раздвинутый на `d` во все стороны.
pub fn grow(r: Rect, d: f32) -> Rect {
    [r[0] - d, r[1] - d, r[2] + d, r[3] + d]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Перебором: что задевает окно.
    fn brute(boxes: &[Rect], r: Rect) -> Vec<u32> {
        (0..boxes.len() as u32).filter(|&i| touch(&boxes[i as usize], &r)).collect()
    }

    #[test]
    fn finds_the_same_as_brute_force_and_only_nearby() {
        // Псевдослучайные предметы разных размеров, часть за краем карты.
        let mut s = 7u64;
        let mut rnd = || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (s >> 33) as f32 / (1u64 << 31) as f32
        };
        let boxes: Vec<Rect> = (0..3000)
            .map(|_| {
                let (x, y) = (rnd() * 500.0 - 50.0, rnd() * 500.0 - 50.0);
                let (w, h) = (0.2 + rnd() * rnd() * 30.0, 0.2 + rnd() * 3.0);
                [x, y, x + w, y + h]
            })
            .collect();
        let grid = Grid::new(&boxes, 8.0);
        for k in 0..200 {
            let (x, y) = (rnd() * 600.0 - 100.0, rnd() * 600.0 - 100.0);
            let s = if k % 10 == 0 { 300.0 } else { rnd() * 20.0 };
            let r = [x, y, x + s, y + s];
            let near = grid.near(r);
            let hit: Vec<u32> = near.iter().copied().filter(|&i| touch(&boxes[i as usize], &r)).collect();
            assert_eq!(hit, brute(&boxes, r), "окно {r:?}");
            assert!(near.windows(2).all(|w| w[0] < w[1]), "по возрастанию и без повторов");
            if s < 5.0 {
                assert!(near.len() < 300, "небольшое окно — только соседи, а не вся карта: {}", near.len());
            }
        }
    }

    #[test]
    fn odd_numbers_and_far_away_boxes_do_not_break_it() {
        // Предмет за миллиард клеток делает корзины крупными: ответ с запасом, но верный.
        let boxes = vec![[0.0, 0.0, 1.0, 1.0], [f32::NAN, 0.0, 1.0, 1.0], [1e9, 1e9, 1e9 + 1.0, 1e9 + 1.0], [5.0, 5.0, 6.0, 6.0]];
        let grid = Grid::new(&boxes, 4.0);
        let exact = |r: Rect| -> Vec<u32> { grid.near(r).into_iter().filter(|&i| i == 1 || touch(&boxes[i as usize], &r)).collect() };
        assert_eq!(exact([0.5, 0.5, 0.6, 0.6]), vec![0, 1], "нечисло — всякому запросу");
        assert_eq!(exact([1e9, 1e9, 1e9 + 0.5, 1e9 + 0.5]), vec![1, 2]);
        assert_eq!(grid.near([-100.0, -100.0, -90.0, -90.0]), vec![1], "мимо сетки — только нечисло");
        assert!(Grid::new(&[], 4.0).near([0.0, 0.0, 10.0, 10.0]).is_empty());
    }
}
