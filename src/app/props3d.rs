//! Предметы в 3D: у кого есть модель и как предмет выходит из картинки пола.
//!
//! Модель предмета — `.glb` рядом с его картинкой на полке ([`crate::model3d`], пространство
//! предмета). Её кладут две раздельные библиотеки — чтобы не спутать, что можно отдавать:
//! - **WoW** (`данные/wow`, на полке «WoW/…»): модели клиента игры — только для своего стола.
//!   У картинок, перенесённых до моделей, `.glb` пишется при первой нужде: модель ищется в
//!   клиенте по папке и имени картинки;
//! - **CC0** (`данные/cc0`, на полке «CC0/…»): свободные наборы (`.glb`, `.gltf`) — можно делиться.
//!   Набор переносится целиком: вид сверху для полки и модель в пространстве предмета.

use super::*;
use crate::model3d::{self, Model3d};
use std::collections::HashMap;
use std::sync::{Arc, mpsc};

/// Как предмет выходит в 3D. В выгрузке с тайлами (и в снимке для окна 3D) такие предметы не в
/// картинке пола, а своими тайлами Foundry — в окне 3D их надо поставить самому.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Lift {
    /// На полу: мебель, сундук, куст.
    Floor,
    /// Над головой, ниже крыш: навес, бельё на верёвке.
    Mid,
    /// Крона дерева: над головой, выше стен.
    Crown,
}

/// Где предмет в 3D (`None` — в картинке пола). Те же правила, что у снимка выгрузки с тайлами.
pub(super) fn lift(p: &Prop, map: &Map) -> Option<Lift> {
    let on_roof = (map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer()) && (map.roofs.iter().any(|r| r.contains(p.pos)) || map.outline_roofs.iter().any(|r| r.contains(p.pos)));
    if on_roof {
        return None;
    }
    if p.is_canopy() {
        return Some(Lift::Crown);
    }
    if map::awning_by_name(&p.asset) || map::line_by_name(&p.asset) {
        return Some(Lift::Mid);
    }
    (p.with_walls() || map::bush_by_name(&p.asset) || p.own_tile()).then_some(Lift::Floor)
}

/// Перенос набора CC0 на полку в фоне: сколько сделано из скольких, ошибки, конец.
pub(super) struct Cc0Job {
    rx: mpsc::Receiver<Cc0Note>,
    pub done: usize,
    pub total: usize,
    pub errors: Vec<String>,
}

enum Cc0Note {
    Total(usize),
    One(Result<(), String>),
    End,
}

/// Модели в папке и её подпапках: `.glb` и `.gltf`.
fn model_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| matches!(x.to_string_lossy().to_lowercase().as_str(), "glb" | "gltf")) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Имя для полки и Foundry: латиница и цифры словами через `_`.
fn plain_name(stem: &str) -> String {
    let w = crate::wow::words_name(stem);
    if w.is_empty() { "model".into() } else { w }
}

/// Один файл набора — на полку: вид сверху `<Имя>_<ш>x<в>.png` и модель `.glb` рядом.
pub(super) fn import_cc0_file(src: &Path, root: &Path, out: &Path) -> Result<PathBuf, String> {
    let mut m = model3d::read_file(src, model3d::PER_METER)?;
    m.normalize();
    let (img, cells) = model3d::top_view(&m, library::PX_PER_CELL)?;
    let pack = root.file_name().map_or_else(|| "CC0".into(), |n| n.to_string_lossy().into_owned());
    let rel = src.parent().and_then(|p| p.strip_prefix(root).ok()).map(Path::to_owned).unwrap_or_default();
    let dir = out.join(pack).join(rel);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = plain_name(&src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let num = |v: f32| format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_owned();
    // Прежние картинка и модель того же предмета (другого размера) — прочь.
    let prefix = format!("{}_", stem.to_lowercase());
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if let Some(rest) = n.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".png").or_else(|| r.strip_suffix(".glb")))
            && !rest.contains('_')
            && library::size_from_name(&format!("x_{rest}")).is_some()
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let file = dir.join(format!("{stem}_{}x{}.png", num(cells[0]), num(cells[1])));
    img.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let glb = m.write_glb(&stem)?;
    std::fs::write(file.with_extension("glb"), glb).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

/// `--cc0 папка`: набор — на полку в папке данных `data` (`cc0/<набор>/…`), итог — в консоль.
pub fn import_cc0_cli(src: &Path, data: &Path) -> i32 {
    let out = data.join("cc0");
    let files = model_files(src);
    let mut failed = 0;
    for f in &files {
        match import_cc0_file(f, src, &out) {
            Ok(png) => println!("{}", png.display()),
            Err(e) => {
                failed += 1;
                eprintln!("{}: {e}", f.display());
            }
        }
    }
    println!("CC0: на полке {} моделей из {} ({})", files.len() - failed, files.len(), out.display());
    if files.is_empty() || failed == files.len() { 1 } else { 0 }
}

impl App {
    pub(super) fn cc0_dir(&self) -> PathBuf {
        self.data_dir.join("cc0")
    }

    /// Перенести набор CC0 из папки на полку («CC0/<набор>/…») — в фоне.
    pub(super) fn start_cc0(&mut self, src: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let out = self.cc0_dir();
        crate::plat::spawn(move || {
            let files = model_files(&src);
            let _ = tx.send(Cc0Note::Total(files.len()));
            for f in files {
                let r = import_cc0_file(&f, &src, &out).map(|_| ()).map_err(|e| format!("{}: {e}", f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
                let _ = tx.send(Cc0Note::One(r));
            }
            let _ = tx.send(Cc0Note::End);
        });
        self.cc0 = Some(Cc0Job { rx, done: 0, total: 0, errors: Vec::new() });
    }

    /// Ход переноса CC0: по концу — полка и итог в строке состояния.
    pub(super) fn cc0_poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.cc0 else { return };
        let mut end = false;
        while let Ok(note) = job.rx.try_recv() {
            match note {
                Cc0Note::Total(n) => job.total = n,
                Cc0Note::One(r) => {
                    job.done += 1;
                    if let Err(e) = r {
                        job.errors.push(e);
                    }
                }
                Cc0Note::End => end = true,
            }
        }
        if !end {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        let Some(job) = self.cc0.take() else { return };
        self.library.set_cc0(&self.cc0_dir());
        self.models3d.clear();
        let ok = job.done - job.errors.len();
        self.notice = Some(if job.total == 0 { "CC0: в папке нет моделей .glb и .gltf".into() } else { format!("CC0: на полке {ok} моделей из {} (папка «CC0/…»)", job.total) });
        if !job.errors.is_empty() {
            self.error = Some(format!("CC0: не перенеслись {} моделей:\n{}", job.errors.len(), job.errors.iter().take(12).cloned().collect::<Vec<_>>().join("\n")));
        }
    }

    /// Файл модели предмета: `.glb` рядом с картинкой. У картинки полки WoW без него — найти
    /// модель в клиенте (если он задан) и записать.
    fn model_file(&mut self, key: &str) -> Option<PathBuf> {
        let png = Path::new(key);
        let glb = png.with_extension("glb");
        if glb.is_file() {
            return Some(glb);
        }
        let low = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
        let wow = self.wow_dir();
        if !low(png).starts_with(&(low(&wow) + "/")) || self.settings.wow_client.is_none() {
            return None;
        }
        let client = self.wow_client_now(None).ok()?;
        let model = crate::wow::model_for(&wow, png, &self.wow.models)?;
        let (m, textures, _) = crate::wow::load(&client, &model).ok()?;
        let stem = png.file_stem()?.to_string_lossy().into_owned();
        crate::wow::write_model(&m, &textures, &glb, &stem).ok()?;
        Some(glb)
    }

    /// Модель предмета по ключу картинки (с запоминанием, и «нет» тоже).
    pub(super) fn prop_model(&mut self, key: &str) -> Option<Arc<Model3d>> {
        if let Some(m) = self.models3d.get(key) {
            return m.clone();
        }
        let m = self.model_file(key).and_then(|f| model3d::read_file(&f, 1.0).ok()).map(Arc::new);
        self.models3d.insert(key.to_owned(), m.clone());
        m
    }

    /// Модели предметов по ключам картинок: ключ → модель и её `.glb` (для выгрузки).
    pub(super) fn models_for(&mut self, keys: Vec<String>) -> HashMap<String, (Arc<Model3d>, PathBuf)> {
        let keys: std::collections::BTreeSet<String> = keys.into_iter().collect();
        let mut out = HashMap::new();
        for k in keys {
            if let Some(m) = self.prop_model(&k)
                && let Some(f) = self.model_file(&k)
            {
                out.insert(k, (m, f));
            }
        }
        out
    }

    /// Ключ картинки — с полки WoW (модели только для своего стола).
    pub(super) fn on_wow_shelf(&self, key: &str) -> bool {
        let low = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
        low(Path::new(key)).starts_with(&(low(&self.wow_dir()) + "/"))
    }
}

/// Во сколько раз растянуть модель по x, y, z, чтобы она легла в рамку предмета `size` (клетки):
/// по горизонтали — в рамку, по высоте — средним из двух.
pub(super) fn fit(model: &Model3d, size: [f32; 2]) -> [f32; 3] {
    let Some([lo, hi]) = model.bounds() else { return [1.0; 3] };
    let (w, d) = ((hi[0] - lo[0]).max(1e-3), (hi[2] - lo[2]).max(1e-3));
    let (sx, sz) = (size[0] / w, size[1] / d);
    [sx, (sx + sz) / 2.0, sz]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furniture_is_lifted_out_of_the_floor_and_a_rug_stays_in_it() {
        let map = Map::new(10, 10);
        let prop = |asset: &str| Prop { asset: asset.into(), pos: [3.0, 3.0], size: [1.0, 1.0], rot: 0.0, scale: 1.0, shadow: None, light: None, canopy: None, layer: None, tint: None, open: None, glow: None, sunk: None, flip: false };
        assert_eq!(lift(&prop("pack:fa|textures/objects/Barrel_Wood_Dark_A_1x1.webp"), &map), Some(Lift::Floor));
        assert_eq!(lift(&prop("pack:fa|textures/objects/Rug_Rectangle_C1_Colorable_3x2.webp"), &map), None);
        let mut tree = prop("pack:fa|textures/objects/Tree_Oak_A_6x6.webp");
        tree.canopy = Some(true);
        assert_eq!(lift(&tree, &map), Some(Lift::Crown));
    }

    #[test]
    fn a_cc0_model_goes_to_the_shelf_with_its_picture_and_model() {
        let dir = std::env::temp_dir().join(format!("kartograf-cc0-{}", std::process::id()));
        let (pack, out) = (dir.join("Набор"), dir.join("out"));
        std::fs::create_dir_all(pack.join("props")).unwrap();
        // Ящик 1,524 × 0,762 м — клетка на полклетки, начало посередине высоты.
        let mut m = Model3d::default();
        let v = |p: [f32; 3], n: [f32; 3]| crate::scene3d::Vertex { p, n, uv: [0.0, 0.0] };
        let (x, z) = (0.762, 0.381);
        m.parts.push(model3d::Part3 {
            mesh: crate::scene3d::Mesh { v: vec![v([-x, 0.3, -z], [0.0, 1.0, 0.0]), v([x, 0.3, -z], [0.0, 1.0, 0.0]), v([x, 0.3, z], [0.0, 1.0, 0.0]), v([-x, -0.3, z], [0.0, 1.0, 0.0])], idx: vec![0, 2, 1, 0, 3, 2] },
            material: crate::scene3d::Material { look: crate::scene3d::Look::Color([120, 80, 40]), alpha: crate::scene3d::Alpha::Opaque, two_sided: true, repeat: true, opacity: 1.0 },
        });
        std::fs::write(pack.join("props").join("crateSmall.glb"), m.write_glb("crate").unwrap()).unwrap();
        let png = import_cc0_file(&pack.join("props").join("crateSmall.glb"), &pack, &out).unwrap();
        assert!(png.starts_with(out.join("Набор").join("props")), "{}", png.display());
        let name = png.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("crate_Small_1.0") || name.starts_with("crate_Small_1x") || name.starts_with("crate_Small_1."), "{name}");
        let back = model3d::read_file(&png.with_extension("glb"), 1.0).unwrap();
        let [lo, hi] = back.bounds().unwrap();
        assert!((hi[0] - lo[0] - 1.0).abs() < 1e-3 && (hi[2] - lo[2] - 0.5).abs() < 1e-3, "метры — в клетки: {lo:?} {hi:?}");
        assert!(lo[1].abs() < 1e-4, "на полу");
        assert_eq!(fit(&back, [2.0, 1.0]), [2.0, 2.0, 2.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
