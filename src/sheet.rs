//! Картинки для ИИ ([`crate::mcp`]): лист ассетов (`preview_assets`) — стороны
//! картинки при повороте 0 и где в клетке её непрозрачная часть, — и линейка
//! клеток поверх снимка карты (`view` с `grid`). Буквы и цифры — свой мелкий
//! шрифт 3×5: шрифта для растровых картинок в программе нет.

use image::{Rgb, RgbImage, RgbaImage};

/// Цифры 3×5 точек, построчно, старший бит — левая точка.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Заглавные буквы 3×5 — только те, что нужны подписям сторон.
fn letter(c: char) -> Option<[u8; 5]> {
    Some(match c {
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        d => DIGITS[d.to_digit(10)? as usize],
    })
}

/// Ширина числа в точках при масштабе `k`: цифра 3, промежуток 1.
pub fn number_width(n: u32, k: u32) -> u32 {
    text_width(&n.to_string(), k)
}

/// Ширина строки в точках при масштабе `k`.
pub fn text_width(s: &str, k: u32) -> u32 {
    (s.chars().count() as u32 * 4).saturating_sub(1) * k
}

fn draw_glyph(img: &mut RgbImage, x: i64, y: i64, glyph: [u8; 5], k: u32, color: [u8; 3]) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..3 {
            if bits >> (2 - col) & 1 == 0 {
                continue;
            }
            for dy in 0..k as i64 {
                for dx in 0..k as i64 {
                    let (px, py) = (x + col * k as i64 + dx, y + row as i64 * k as i64 + dy);
                    if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                        img.put_pixel(px as u32, py as u32, Rgb(color));
                    }
                }
            }
        }
    }
}

/// Строка (цифры и буквы подписей) шрифтом 3×5, увеличенным в `k` раз, левым
/// верхним углом в `(x, y)`; `down` — буквы столбиком сверху вниз.
pub fn draw_text(img: &mut RgbImage, x: i64, y: i64, s: &str, k: u32, color: [u8; 3], down: bool) {
    for (pos, ch) in s.chars().enumerate() {
        let Some(glyph) = letter(ch) else { continue };
        let (gx, gy) = if down { (x, y + pos as i64 * 6 * k as i64) } else { (x + pos as i64 * 4 * k as i64, y) };
        draw_glyph(img, gx, gy, glyph, k, color);
    }
}

/// Число цифрами 3×5, увеличенными в `k` раз, левым верхним углом в `(x, y)`.
pub fn draw_number(img: &mut RgbImage, x: i64, y: i64, n: u32, k: u32, color: [u8; 3]) {
    draw_text(img, x, y, &n.to_string(), k, color, false);
}

fn blend(img: &mut RgbImage, x: u32, y: u32, c: [u8; 3], a: f32) {
    if x < img.width() && y < img.height() {
        let p = img.get_pixel_mut(x, y);
        for k in 0..3 {
            p[k] = (p[k] as f32 * (1.0 - a) + c[k] as f32 * a).round() as u8;
        }
    }
}

/// Плотная рамка непрозрачного (альфа > 40) в долях картинки: `[u0, v0, u1, v1]`,
/// u — вправо, v — вниз. Пустая картинка — `None`.
pub fn alpha_bbox(img: &RgbaImage) -> Option<[f32; 4]> {
    let (w, h) = img.dimensions();
    let (mut l, mut t, mut r, mut b) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 40 {
            (l, t, r, b) = (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1));
        }
    }
    (r > l && b > t).then(|| [l as f32 / w as f32, t as f32 / h as f32, r as f32 / w as f32, b as f32 / h as f32])
}

const BACK: [u8; 3] = [38, 40, 46];
const PAPER: [u8; 3] = [56, 60, 68];
const INK: [u8; 3] = [226, 228, 232];
const MUTED: [u8; 3] = [150, 156, 168];

/// Лист картинок: каждая — в своей плитке `tile` точек на тёмном фоне, по своему
/// размеру в клетках (`size`, ширина × высота), при повороте 0, с сеткой клеток
/// под ней, номером плитки слева вверху и подписями сторон картинки — TOP, RIGHT,
/// BOTTOM, LEFT: те же слова, что `front` у предмета. Стрелку сверху ИИ читал как
/// «сюда смотрит фасад», а у мебели FA сверху спинка.
pub fn contact_sheet(items: &[(RgbaImage, [f32; 2])], tile: u32) -> RgbImage {
    let n = items.len().max(1) as u32;
    let cols = (n as f32).sqrt().ceil() as u32;
    let rows = n.div_ceil(cols);
    let gap = 6;
    let mut sheet = RgbImage::from_pixel(cols * (tile + gap) + gap, rows * (tile + gap) + gap, Rgb(BACK));
    for (k, (img, size)) in items.iter().enumerate() {
        let (cx, cy) = (gap + (k as u32 % cols) * (tile + gap), gap + (k as u32 / cols) * (tile + gap));
        for y in cy..cy + tile {
            for x in cx..cx + tile {
                sheet.put_pixel(x, y, Rgb(PAPER));
            }
        }
        // Поле под картинку: отступы под номер и подписи сторон; масштаб — по клеткам.
        let room = tile.saturating_sub(44).max(8) as f32;
        let (sw, sh) = (size[0].max(0.01), size[1].max(0.01));
        let ppc = room / sw.max(sh);
        let (dw, dh) = ((sw * ppc).round().max(1.0) as u32, (sh * ppc).round().max(1.0) as u32);
        let (ox, oy) = (cx + (tile - dw) / 2, cy + (tile - dh) / 2);
        // Клетки, которые занимает ассет.
        for i in 0..=sw.ceil() as u32 {
            let x = ox + ((i as f32 * ppc).round() as u32).min(dw.saturating_sub(1));
            for y in oy..oy + dh {
                blend(&mut sheet, x, y, INK, 0.18);
            }
        }
        for j in 0..=sh.ceil() as u32 {
            let y = oy + ((j as f32 * ppc).round() as u32).min(dh.saturating_sub(1));
            for x in ox..ox + dw {
                blend(&mut sheet, x, y, INK, 0.18);
            }
        }
        let fit = image::imageops::resize(img, dw, dh, image::imageops::FilterType::Triangle);
        for (x, y, p) in fit.enumerate_pixels() {
            blend(&mut sheet, ox + x, oy + y, [p[0], p[1], p[2]], p[3] as f32 / 255.0);
        }
        // Стороны картинки при повороте 0: приглушённо, у середины каждого края.
        let (mx, my) = ((ox + dw / 2) as i64, (oy + dh / 2) as i64);
        let k1 = 1;
        let tw = |s: &str| text_width(s, k1) as i64;
        draw_text(&mut sheet, mx - tw("TOP") / 2, oy as i64 - 8, "TOP", k1, MUTED, false);
        draw_text(&mut sheet, mx - tw("BOTTOM") / 2, (oy + dh) as i64 + 3, "BOTTOM", k1, MUTED, false);
        draw_text(&mut sheet, ox as i64 - 7, my - 6 * 4 / 2, "LEFT", k1, MUTED, true);
        draw_text(&mut sheet, (ox + dw) as i64 + 4, my - 6 * 5 / 2, "RIGHT", k1, MUTED, true);
        draw_number(&mut sheet, cx as i64 + 4, cy as i64 + 4, k as u32 + 1, 2, INK);
    }
    sheet
}

/// Сетка клеток и линейка поверх снимка карты: слева и сверху поле с номерами
/// линий сетки (координаты клеток), по картинке — тонкие линии, каждая пятая
/// заметнее. `area` — что снято, в клетках. Возвращает картинку и поля (слева, сверху).
pub fn ruler(img: &RgbImage, area: [f32; 4]) -> (RgbImage, [u32; 2]) {
    let ppc = img.width() as f32 / (area[2] - area[0]).max(1e-3);
    let k = 2;
    let digits_h = 5 * k;
    let widest = number_width(area[2].max(area[3]).ceil().max(0.0) as u32, k);
    let (left, top) = (widest + 6, digits_h + 6);
    let mut out = RgbImage::from_pixel(img.width() + left, img.height() + top, Rgb(BACK));
    image::imageops::replace(&mut out, img, left as i64, top as i64);
    // Шаг подписей: число не налезает на соседнее.
    let step = [1, 2, 5, 10, 20, 50, 100].into_iter().find(|s| *s as f32 * ppc >= (widest + 6) as f32).unwrap_or(200);
    let lines = |lo: f32, hi: f32| (lo.ceil() as i64..=hi.floor() as i64).collect::<Vec<_>>();
    for x in lines(area[0], area[2]) {
        let px = left as f32 + (x as f32 - area[0]) * ppc;
        let px = (px.round() as u32).min(out.width() - 1);
        let a = if x % 5 == 0 { 0.55 } else { 0.28 };
        for y in top..out.height() {
            blend(&mut out, px, y, [0, 0, 0], a);
        }
        if x % step == 0 && x >= 0 {
            let w = number_width(x as u32, k) as i64;
            let at = (px as i64 - w / 2).clamp(left as i64 - w / 2, out.width() as i64 - w);
            draw_number(&mut out, at, 3, x as u32, k, INK);
        }
    }
    for y in lines(area[1], area[3]) {
        let py = top as f32 + (y as f32 - area[1]) * ppc;
        let py = (py.round() as u32).min(out.height() - 1);
        let a = if y % 5 == 0 { 0.55 } else { 0.28 };
        for x in left..out.width() {
            blend(&mut out, x, py, [0, 0, 0], a);
        }
        if y % step == 0 && y >= 0 {
            let w = number_width(y as u32, k) as i64;
            let at = (py as i64 - digits_h as i64 / 2).clamp(top as i64 - digits_h as i64 / 2, out.height() as i64 - digits_h as i64);
            draw_number(&mut out, left as i64 - 3 - w, at, y as u32, k, INK);
        }
    }
    (out, [left, top])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_finds_the_opaque_part() {
        let img = RgbaImage::from_fn(100, 50, |x, y| if (20..60).contains(&x) && (10..40).contains(&y) { image::Rgba([1, 2, 3, 255]) } else { image::Rgba([0; 4]) });
        assert_eq!(alpha_bbox(&img), Some([0.2, 0.2, 0.6, 0.8]));
        assert_eq!(alpha_bbox(&RgbaImage::new(4, 4)), None);
    }

    #[test]
    fn numbers_are_drawn_where_asked() {
        let mut img = RgbImage::new(40, 20);
        draw_number(&mut img, 2, 2, 17, 2, [255, 255, 255]);
        assert_eq!(number_width(17, 2), 14);
        // «1»: верхняя точка — средняя из трёх; «7»: верхний ряд целиком.
        assert_eq!(img.get_pixel(2 + 2, 2)[0], 255);
        assert_eq!(img.get_pixel(2, 2)[0], 0);
        assert_eq!(img.get_pixel(2 + 8, 2)[0], 255);
        assert_eq!(img.get_pixel(2 + 8 + 4, 2 + 9)[0], 0, "низ «7» — посередине");
    }

    #[test]
    fn ruler_adds_margins_and_lines_on_cell_edges() {
        let img = RgbImage::from_pixel(200, 100, Rgb([255, 255, 255]));
        let (out, [l, t]) = ruler(&img, [0.0, 0.0, 10.0, 5.0]);
        assert_eq!((out.width(), out.height()), (200 + l, 100 + t));
        // Линия клетки 3 — на 60-й точке снимка, между линиями — белое.
        assert!(out.get_pixel(l + 60, t + 50)[0] < 255);
        assert_eq!(out.get_pixel(l + 70, t + 50)[0], 255);
    }

    #[test]
    fn side_labels_are_written() {
        let mut img = RgbImage::new(30, 10);
        draw_text(&mut img, 0, 0, "TOP", 1, [255, 255, 255], false);
        // «T»: верхний ряд целиком, ниже — середина.
        assert_eq!((img.get_pixel(0, 0)[0], img.get_pixel(2, 0)[0], img.get_pixel(0, 2)[0], img.get_pixel(1, 2)[0]), (255, 255, 0, 255));
        assert_eq!(text_width("RIGHT", 1), 19);
        let mut tall = RgbImage::new(4, 40);
        draw_text(&mut tall, 0, 0, "LEFT", 1, [255, 255, 255], true);
        assert_eq!(tall.get_pixel(0, 6 * 3 + 4)[0], 0, "у «T» внизу — только середина");
        assert_eq!(tall.get_pixel(1, 6 * 3 + 4)[0], 255);
    }

    #[test]
    fn sheet_has_a_tile_per_asset() {
        let a = (RgbaImage::from_pixel(64, 32, image::Rgba([200, 0, 0, 255])), [2.0, 1.0]);
        let s = contact_sheet(&[a.clone(), a.clone(), a], 96);
        assert_eq!((s.width(), s.height()), (2 * 102 + 6, 2 * 102 + 6));
    }
}
