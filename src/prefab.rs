//! Префаб: набор предметов, сохранённый целиком — стол со стульями, угол кухни, пульт
//! с креслом, — и поставленный заново одним щелчком, с поворотом.
//!
//! Лежат в папке данных, в `prefabs/`, по файлу на префаб (имя файла — латиницей):
//! ими можно поделиться, положив файл в ту же папку у другого.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::map::Prop;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefab {
    pub name: String,
    /// Предметы; середины — от середины префаба, в клетках, поворот — как на карте.
    pub props: Vec<Prop>,
}

/// Поворот точки на `deg` градусов по часовой (ось y вниз), как поворот предмета.
fn turn(v: [f32; 2], deg: f32) -> [f32; 2] {
    let (s, c) = deg.to_radians().sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
}

impl Prefab {
    /// Из предметов карты. Середина — середина рамки их середин, на полклетки: префаб,
    /// поставленный с привязкой к сетке, ложится по сетке так же, как лежал.
    pub fn from_props(name: &str, props: &[Prop]) -> Self {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in props {
            lo = [lo[0].min(p.pos[0]), lo[1].min(p.pos[1])];
            hi = [hi[0].max(p.pos[0]), hi[1].max(p.pos[1])];
        }
        let half = |v: f32| (v * 2.0).round() / 2.0;
        let c = if props.is_empty() { [0.0, 0.0] } else { [half((lo[0] + hi[0]) / 2.0), half((lo[1] + hi[1]) / 2.0)] };
        let props = props.iter().map(|p| Prop { pos: [p.pos[0] - c[0], p.pos[1] - c[1]], ..p.clone() }).collect();
        Self { name: name.trim().to_owned(), props }
    }

    /// Предметы префаба на карте: середина в `at`, всё повёрнуто на `deg` градусов по часовой.
    pub fn placed(&self, at: [f32; 2], deg: f32) -> Vec<Prop> {
        self.props
            .iter()
            .map(|p| {
                let v = turn(p.pos, deg);
                Prop { pos: [at[0] + v[0], at[1] + v[1]], rot: (p.rot + deg).rem_euclid(360.0), ..p.clone() }
            })
            .collect()
    }

    /// Ширина и высота рамки всех предметов, клетки.
    pub fn span(&self) -> [f32; 2] {
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in &self.props {
            for c in p.corners() {
                lo = [lo[0].min(c[0]), lo[1].min(c[1])];
                hi = [hi[0].max(c[0]), hi[1].max(c[1])];
            }
        }
        if self.props.is_empty() { [0.0, 0.0] } else { [hi[0] - lo[0], hi[1] - lo[1]] }
    }
}

/// Папка префабов в папке данных.
pub fn dir(data: &Path) -> PathBuf {
    data.join("prefabs")
}

/// Все префабы папки, по имени.
pub fn load_all(dir: &Path) -> Vec<(PathBuf, Prefab)> {
    let mut out: Vec<(PathBuf, Prefab)> = crate::plat::fs::tree(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|p| {
            let bytes = crate::plat::fs::read(&p).ok()?;
            let pf: Prefab = serde_json::from_slice(&bytes).ok()?;
            (!pf.props.is_empty()).then_some((p, pf))
        })
        .collect();
    out.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()));
    out
}

/// Сохранить префаб: файл — имя латиницей. Префаб с тем же именем заменяется; другой
/// префаб, чьё имя дало то же латиницей, не затирается — новому номер.
pub fn save(dir: &Path, pf: &Prefab) -> Result<PathBuf, String> {
    let base = crate::export::ascii_name(&pf.name);
    let mut n = 1;
    let path = loop {
        let file = dir.join(if n == 1 { format!("{base}.json") } else { format!("{base}_{n}.json") });
        let other = crate::plat::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Prefab>(&b).ok()).is_some_and(|old| old.name != pf.name);
        if !other {
            break file;
        }
        n += 1;
    };
    let text = serde_json::to_string_pretty(pf).map_err(|e| e.to_string())?;
    crate::plat::fs::write(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(asset: &str, pos: [f32; 2], rot: f32) -> Prop {
        Prop { asset: asset.into(), pos, size: [1.0, 1.0], rot, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false }
    }

    #[test]
    fn a_table_with_chairs_turns_as_one() {
        // Стол 2×1 и два стула лицом к нему — сверху и снизу.
        let set = [prop("table", [5.0, 5.5], 0.0), prop("chair", [5.0, 4.5], 180.0), prop("chair", [5.0, 6.5], 0.0)];
        let pf = Prefab::from_props(" Стол со стульями ", &set);
        assert_eq!(pf.name, "Стол со стульями");
        assert_eq!(pf.props[0].pos, [0.0, 0.0], "середина — на столе");
        // Поставлен там же без поворота — те же места.
        let same = pf.placed([5.0, 5.5], 0.0);
        assert_eq!(same.iter().map(|p| p.pos).collect::<Vec<_>>(), set.iter().map(|p| p.pos).collect::<Vec<_>>());
        // Повёрнут на 90° по часовой: стул сверху уходит вправо и смотрит влево.
        let turned = pf.placed([10.0, 10.0], 90.0);
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
        assert!(near(turned[1].pos, [11.0, 10.0]), "{:?}", turned[1].pos);
        assert_eq!(turned[1].rot, 270.0);
        assert!(near(turned[2].pos, [9.0, 10.0]) && turned[2].rot == 90.0);
        assert!(pf.span()[1] > 2.9);
    }

    #[test]
    fn prefabs_are_files_with_latin_names() {
        let dir = std::env::temp_dir().join(format!("kartograf_prefabs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pf = Prefab::from_props("Угол кухни", &[prop("stove", [1.0, 1.0], 0.0)]);
        let path = save(&dir, &pf).unwrap();
        assert_eq!(path.file_name().unwrap(), "Ugol_kukhni.json");
        // Тот же префаб — на место; другой с тем же именем латиницей — рядом.
        assert_eq!(save(&dir, &pf).unwrap(), path);
        let other = Prefab { name: "Угол  кухни".into(), ..pf.clone() };
        assert_eq!(save(&dir, &other).unwrap().file_name().unwrap(), "Ugol_kukhni_2.json");
        let all = load_all(&dir);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].1.props.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
