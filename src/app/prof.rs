//! Сколько длится кадр и его части: замер для MCP (`map_info` → `frame`). Сто двадцать
//! последних кадров; время — работа программы за кадр, без отрисовки видеокартой.

use serde_json::{Value, json};
use std::collections::VecDeque;

const KEEP: usize = 120;

#[derive(Default)]
pub struct Prof {
    frames: VecDeque<Vec<(&'static str, f32)>>,
    now: Vec<(&'static str, f32)>,
}

impl Prof {
    /// Прибавить к части кадра `part` время с `since`.
    pub fn add(&mut self, part: &'static str, since: crate::plat::Instant) {
        self.add_ms(part, since.elapsed().as_secs_f32() * 1000.0);
    }

    pub fn add_ms(&mut self, part: &'static str, ms: f32) {
        match self.now.iter_mut().find(|p| p.0 == part) {
            Some(p) => p.1 += ms,
            None => self.now.push((part, ms)),
        }
    }

    pub fn end_frame(&mut self) {
        let f = std::mem::take(&mut self.now);
        if !f.is_empty() {
            self.frames.push_back(f);
            if self.frames.len() > KEEP {
                self.frames.pop_front();
            }
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// По частям: среднее и наибольшее, мс; часть, которой в кадре не было, — ноль.
    pub fn report(&self) -> Value {
        let mut parts: Vec<&'static str> = Vec::new();
        for f in &self.frames {
            for (p, _) in f {
                if !parts.contains(p) {
                    parts.push(p);
                }
            }
        }
        let n = self.frames.len().max(1) as f32;
        // В f64: десятые из f32 в ответе вышли бы хвостом «5.199999809».
        let r1 = |v: f32| (v as f64 * 10.0).round() / 10.0;
        let mut out = serde_json::Map::new();
        out.insert("frames".into(), json!(self.frames.len()));
        for p in parts {
            let all: Vec<f32> = self.frames.iter().map(|f| f.iter().find(|x| x.0 == p).map_or(0.0, |x| x.1)).collect();
            let max = all.iter().copied().fold(0.0, f32::max);
            out.insert(p.into(), json!({"avg_ms": r1(all.iter().sum::<f32>() / n), "max_ms": r1(max)}));
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_averages_over_frames_and_counts_missing_parts_as_zero() {
        let mut p = Prof::default();
        p.now = vec![("frame", 10.0), ("props", 4.0)];
        p.end_frame();
        p.now = vec![("frame", 20.0)];
        p.end_frame();
        let r = p.report();
        assert_eq!(r["frames"], 2);
        assert_eq!(r["frame"]["avg_ms"], 15.0);
        assert_eq!(r["frame"]["max_ms"], 20.0);
        assert_eq!(r["props"]["avg_ms"], 2.0);
        for _ in 0..200 {
            p.now = vec![("frame", 1.0)];
            p.end_frame();
        }
        assert_eq!(p.report()["frames"], KEEP);
    }
}
