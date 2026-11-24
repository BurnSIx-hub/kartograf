//! Текстуры BLP2 (WoW 1.x–3.x): палитра, DXT1/3/5, несжатые BGRA. Только верхний мип.

/// Картинка RGBA построчно.
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<[u8; 4]>,
}

impl Rgba {
    /// Цвет с повтором или прижимом по краям, билинейно; `u`, `v` в долях.
    pub fn sample(&self, u: f32, v: f32, wrap: [bool; 2]) -> [f32; 4] {
        let fx = u * self.w as f32 - 0.5;
        let fy = v * self.h as f32 - 0.5;
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let ix = |x: i64| -> usize {
            let w = self.w as i64;
            (if wrap[0] { x.rem_euclid(w) } else { x.clamp(0, w - 1) }) as usize
        };
        let iy = |y: i64| -> usize {
            let h = self.h as i64;
            (if wrap[1] { y.rem_euclid(h) } else { y.clamp(0, h - 1) }) as usize
        };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let p = |x: i64, y: i64| self.px[iy(y) * self.w as usize + ix(x)];
        let (a, b, c, d) = (p(x0, y0), p(x0 + 1, y0), p(x0, y0 + 1), p(x0 + 1, y0 + 1));
        let mut out = [0f32; 4];
        for k in 0..4 {
            let top = a[k] as f32 * (1.0 - tx) + b[k] as f32 * tx;
            let bot = c[k] as f32 * (1.0 - tx) + d[k] as f32 * tx;
            out[k] = (top * (1.0 - ty) + bot * ty) / 255.0;
        }
        out
    }
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| "BLP: короткий файл".to_owned())
}

pub fn decode(b: &[u8]) -> Result<Rgba, String> {
    if b.get(..4) != Some(b"BLP2") {
        return Err(format!("не BLP2 ({:?})", String::from_utf8_lossy(b.get(..4).unwrap_or_default())));
    }
    let (compression, alpha_depth, alpha_type) = (b[8], b[9], b[10]);
    let (w, h) = (u32_at(b, 12)?, u32_at(b, 16)?);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return Err(format!("BLP: размер {w}×{h}"));
    }
    let (ofs, size) = (u32_at(b, 20)? as usize, u32_at(b, 84)? as usize);
    let data = b.get(ofs..ofs + size).ok_or("BLP: мип за краем файла")?;
    let n = (w * h) as usize;
    let px = match compression {
        1 => {
            let pal: Vec<[u8; 4]> = (0..256).map(|i| b.get(148 + i * 4..152 + i * 4).map_or([0; 4], |c| [c[2], c[1], c[0], 255])).collect();
            let idx = data.get(..n).ok_or("BLP: мало точек")?;
            let alpha = &data[n..];
            (0..n)
                .map(|i| {
                    let mut c = pal[idx[i] as usize];
                    c[3] = match alpha_depth {
                        0 => 255,
                        1 => alpha.get(i / 8).map_or(255, |a| if a >> (i % 8) & 1 == 1 { 255 } else { 0 }),
                        4 => alpha.get(i / 2).map_or(255, |a| ((a >> ((i % 2) * 4)) & 0xF) * 17),
                        _ => alpha.get(i).copied().unwrap_or(255),
                    };
                    c
                })
                .collect()
        }
        2 => {
            let kind = if alpha_depth <= 1 { Dxt::One } else if alpha_type == 7 { Dxt::Five } else { Dxt::Three };
            dxt(data, w, h, kind)?
        }
        3 => data.chunks_exact(4).take(n).map(|c| [c[2], c[1], c[0], c[3]]).collect(),
        c => return Err(format!("BLP: сжатие {c}")),
    };
    if px.len() < n {
        return Err("BLP: мало точек".into());
    }
    Ok(Rgba { w, h, px })
}

#[derive(Clone, Copy, PartialEq)]
enum Dxt {
    One,
    Three,
    Five,
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

fn dxt(data: &[u8], w: u32, h: u32, kind: Dxt) -> Result<Vec<[u8; 4]>, String> {
    let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
    let block = if kind == Dxt::One { 8 } else { 16 };
    if data.len() < bw * bh * block {
        return Err("BLP: мало блоков DXT".into());
    }
    let mut px = vec![[0u8; 4]; (w * h) as usize];
    for by in 0..bh {
        for bx in 0..bw {
            let blk = &data[(by * bw + bx) * block..][..block];
            let (alpha, color) = if kind == Dxt::One { (&[][..], blk) } else { blk.split_at(8) };
            let c0 = u16::from_le_bytes([color[0], color[1]]);
            let c1 = u16::from_le_bytes([color[2], color[3]]);
            let (a, b) = (rgb565(c0), rgb565(c1));
            let mix = |p: u32, q: u32, d: u32| -> [u8; 3] { std::array::from_fn(|k| ((a[k] as u32 * p + b[k] as u32 * q) / d) as u8) };
            let four = kind != Dxt::One || c0 > c1;
            let pal: [[u8; 4]; 4] = if four {
                let (m1, m2) = (mix(2, 1, 3), mix(1, 2, 3));
                [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], [m1[0], m1[1], m1[2], 255], [m2[0], m2[1], m2[2], 255]]
            } else {
                let m = mix(1, 1, 2);
                [[a[0], a[1], a[2], 255], [b[0], b[1], b[2], 255], [m[0], m[1], m[2], 255], [0, 0, 0, 0]]
            };
            let bits = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
            // DXT5: две опорные альфы и 3-битные индексы.
            let a5: [u8; 8] = if kind == Dxt::Five {
                let (a0, a1) = (alpha[0] as u32, alpha[1] as u32);
                std::array::from_fn(|i| match i {
                    0 => a0 as u8,
                    1 => a1 as u8,
                    _ if a0 > a1 => ((a0 * (8 - i as u32) + a1 * (i as u32 - 1)) / 7) as u8,
                    6 => 0,
                    7 => 255,
                    _ => ((a0 * (6 - i as u32) + a1 * (i as u32 - 1)) / 5) as u8,
                })
            } else {
                [0; 8]
            };
            let a5bits = if kind == Dxt::Five { (2..8).rev().fold(0u64, |acc, i| (acc << 8) | alpha[i] as u64) } else { 0 };
            for i in 0..16 {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x >= w as usize || y >= h as usize {
                    continue;
                }
                let mut c = pal[((bits >> (i * 2)) & 3) as usize];
                match kind {
                    Dxt::Three => c[3] = ((alpha[i / 2] >> ((i % 2) * 4)) & 0xF) * 17,
                    Dxt::Five => c[3] = a5[((a5bits >> (i * 3)) & 7) as usize],
                    Dxt::One => {}
                }
                px[y * w as usize + x] = c;
            }
        }
    }
    Ok(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blp2(compression: u8, alpha_depth: u8, alpha_type: u8, w: u32, h: u32, data: &[u8], palette: &[[u8; 4]]) -> Vec<u8> {
        let mut b = vec![0u8; 1172];
        b[..4].copy_from_slice(b"BLP2");
        b[4] = 1;
        b[8] = compression;
        b[9] = alpha_depth;
        b[10] = alpha_type;
        b[12..16].copy_from_slice(&w.to_le_bytes());
        b[16..20].copy_from_slice(&h.to_le_bytes());
        b[20..24].copy_from_slice(&1172u32.to_le_bytes());
        b[84..88].copy_from_slice(&(data.len() as u32).to_le_bytes());
        for (i, c) in palette.iter().enumerate() {
            b[148 + i * 4..152 + i * 4].copy_from_slice(c);
        }
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn dxt1_block_gives_its_two_colours_and_the_blends() {
        // c0 = чистый красный 565, c1 = чистый синий; индексы по строкам 0,1,2,3.
        let red: u16 = 31 << 11;
        let blue: u16 = 31;
        let mut data = Vec::new();
        data.extend_from_slice(&red.to_le_bytes());
        data.extend_from_slice(&blue.to_le_bytes());
        data.extend_from_slice(&[0b0000_0000, 0b0101_0101, 0b1010_1010, 0b1111_1111]);
        let img = decode(&blp2(2, 0, 0, 4, 4, &data, &[])).unwrap();
        assert_eq!(img.px[0], [255, 0, 0, 255]);
        assert_eq!(img.px[4], [0, 0, 255, 255]);
        assert_eq!(img.px[8], [170, 0, 85, 255]);
        assert_eq!(img.px[12], [85, 0, 170, 255]);
    }

    #[test]
    fn palette_with_one_bit_alpha() {
        // Палитра BGRA: 0 — зелёный, 1 — белый; альфа по битам: 1, 0, 1, 1.
        let pal = [[0, 255, 0, 0], [255, 255, 255, 0]];
        let data = [0u8, 1, 1, 0, 0b0000_1101];
        let img = decode(&blp2(1, 1, 0, 2, 2, &data, &pal)).unwrap();
        assert_eq!(img.px, vec![[0, 255, 0, 255], [255, 255, 255, 0], [255, 255, 255, 255], [0, 255, 0, 255]]);
    }

    #[test]
    fn sampling_wraps_or_clamps() {
        let img = Rgba { w: 2, h: 1, px: vec![[0, 0, 0, 255], [255, 255, 255, 255]] };
        // Ровно в середине первой точки — её цвет.
        assert_eq!(img.sample(0.25, 0.5, [false, false])[0], 0.0);
        // Слева за краем: с повтором — смесь с правой точкой, с прижимом — левая.
        assert!(img.sample(0.0, 0.5, [true, false])[0] > 0.4);
        assert_eq!(img.sample(0.0, 0.5, [false, false])[0], 0.0);
    }
}
