//! Предметы в 3D: у кого есть модель и как предмет выходит из картинки пола.
//!
//! Модель предмета — `.glb` рядом с его картинкой на полке ([`crate::model3d`], пространство
//! предмета). Её кладут две раздельные библиотеки — чтобы не спутать, что можно отдавать:
//! - **WoW** (`данные/wow`, на полке «WoW/…»): модели клиента игры — только для своего стола.
//!   У картинок, перенесённых до моделей, `.glb` пишется при первой нужде: модель ищется в
//!   клиенте по папке и имени картинки;
//! - **CC0** (`данные/cc0`, на полке «CC0/…»): свободные наборы (`.glb`, `.gltf`) — можно делиться.
//!   Набор переносится целиком: вид сверху для полки и модель в пространстве предмета.
//!
//! Рисункам без своей модели (ассеты FA, свои папки) модель подбирается по имени с полки
//! «CC0/Poly Haven» ([`crate::fa3d`]) и пишется в `данные/cache/fa3d`.

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

pub(super) enum Cc0Note {
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
/// `unit` — клеток в единице файла: у glTF это метр ([`model3d::PER_METER`]), но наборы бывают
/// и в своих единицах (у KayKit стена в 4 единицы — 10 футов, единица — полклетки).
/// `taken` — имена, уже занятые в этом переносе: у двух моделей набора имя на полке может
/// совпасть без учёта регистра (`WoodenTable_02` и `wooden_table_02`), а Windows его не различает —
/// вторая получает хвост `_2`.
pub(super) fn import_cc0_file(src: &Path, root: &Path, out: &Path, unit: f32, taken: &mut std::collections::HashSet<String>) -> Result<PathBuf, String> {
    let bytes = std::fs::read(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut m = model3d::read_gltf(&bytes, src.parent(), unit)?;
    let shift = m.normalize();
    let (img, cells) = model3d::top_view(&m, library::PX_PER_CELL)?;
    let pack = root.file_name().map_or_else(|| "CC0".into(), |n| n.to_string_lossy().into_owned());
    let rel = src.parent().and_then(|p| p.strip_prefix(root).ok()).map(Path::to_owned).unwrap_or_default();
    let dir = out.join(pack).join(rel);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // `barrel.gltf.glb` (так пишет KayKit) — просто `barrel`; `chair_1k` (Poly Haven) — `chair`.
    let raw = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let raw = raw.strip_suffix(".gltf").unwrap_or(&raw);
    let raw = ["_1k", "_2k", "_4k", "_8k"].iter().find_map(|t| raw.strip_suffix(t)).unwrap_or(raw);
    let mut stem = plain_name(raw);
    let dir_key = dir.to_string_lossy().to_lowercase();
    let base = stem.clone();
    let mut k = 2;
    while !taken.insert(format!("{dir_key}|{}", stem.to_lowercase())) {
        stem = format!("{base}_{k}");
        k += 1;
    }
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
    // Модель — исходная, со всеми картами и в своём разрешении: только поставлена в рамку предмета.
    let glb = model3d::wrap_gltf(&bytes, src.parent(), unit, shift)?;
    std::fs::write(file.with_extension("glb"), glb).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(file)
}

/// Описание набора на полке (`данные/cc0/<набор>/pack.json`): чей он и на каких условиях; у набора
/// с зеркала — ещё список файлов для сверки.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PackInfo {
    pub name: String,
    /// Автор или студия.
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub license_url: String,
    /// Как авторы просят их указывать.
    #[serde(default)]
    pub note: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PackFile>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PackFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// Наборы, что лежат зеркалом в выпусках Картографа на GitHub: название и метка выпуска.
pub(super) const MIRRORS: &[(&str, &str)] = &[("Poly Haven", "models-polyhaven-1")];

/// Адрес выпусков с наборами: зеркало на GitHub; для проверки — `KARTOGRAF_MODELS_URL`
/// (папка, где лежат `<метка>/pack.json` и файлы).
fn mirror_url(tag: &str, file: &str) -> String {
    match std::env::var("KARTOGRAF_MODELS_URL") {
        Ok(base) => format!("{}/{tag}/{file}", base.trim_end_matches('/')),
        Err(_) => format!("{}/download/{tag}/{file}", crate::update::RELEASES),
    }
}

fn http() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_connect(Some(std::time::Duration::from_secs(15))).timeout_global(Some(std::time::Duration::from_secs(600))).build().into()
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let mut r = http().get(url).header("User-Agent", concat!("Kartograf/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| format!("{url}: {e}"))?;
    r.body_mut().with_config().limit(1 << 31).read_to_vec().map_err(|e| format!("{url}: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Имя файла набора — только простое имя: без папок и выходов наверх.
fn plain_file(name: &str) -> bool {
    !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != ".."
}

/// Описание набора (`pack.json` в его папке: автор, лицензия, ссылка) — на полку рядом с моделями.
fn copy_pack_info(src: &Path, out: &Path) {
    let Some(name) = src.file_name() else { return };
    if let Ok(bytes) = std::fs::read(src.join("pack.json")) {
        let _ = std::fs::create_dir_all(out.join(name));
        let _ = std::fs::write(out.join(name).join("pack.json"), bytes);
    }
}

/// Скачать набор с зеркала в `данные/cc0/<набор>/`: описание, затем файлы со сверкой SHA-256
/// (уже лежащие и совпавшие — не качать). `note` — сколько файлов всего и каждый готовый.
pub(super) fn download_pack(tag: &str, cc0: &Path, mut note: impl FnMut(Cc0Note)) -> Result<PackInfo, String> {
    let info: PackInfo = serde_json::from_slice(&fetch(&mirror_url(tag, "pack.json"))?).map_err(|e| format!("pack.json: {e}"))?;
    if !plain_file(&info.name) {
        return Err(format!("странное имя набора: {}", info.name));
    }
    let dir = cc0.join(&info.name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    note(Cc0Note::Total(info.files.len()));
    for f in &info.files {
        let r = (|| {
            if !plain_file(&f.name) {
                return Err(format!("странное имя файла: {}", f.name));
            }
            let path = dir.join(&f.name);
            if std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == f.sha256) {
                return Ok(());
            }
            let bytes = fetch(&mirror_url(tag, &f.name))?;
            if sha256_hex(&bytes) != f.sha256 {
                return Err(format!("{}: не сошлась SHA-256", f.name));
            }
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))
        })();
        note(Cc0Note::One(r));
    }
    let mut keep = info.clone();
    keep.files.clear();
    std::fs::write(dir.join("pack.json"), serde_json::to_string_pretty(&keep).unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(info)
}

/// `--cc0-get метка`: набор с зеркала — на полку, без окна.
pub fn download_pack_cli(tag: &str, data: &Path) -> i32 {
    let (mut total, mut done, mut failed) = (0, 0, 0);
    let r = download_pack(tag, &data.join("cc0"), |n| match n {
        Cc0Note::Total(t) => total = t,
        Cc0Note::One(Ok(())) => done += 1,
        Cc0Note::One(Err(e)) => {
            failed += 1;
            eprintln!("{e}");
        }
        Cc0Note::End => {}
    });
    match r {
        Ok(info) => {
            println!("{}: на полке {} файлов из {total}", info.name, done);
            (failed > 0) as i32
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Кто чей в сцене: что использовано, чьё, на каких условиях, ссылка и просьба авторов.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Credit {
    pub what: String,
    pub by: String,
    pub license: String,
    pub url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Описание набора на полке; нет — по имени папки.
pub(super) fn pack_info(cc0: &Path, name: &str) -> PackInfo {
    std::fs::read(cc0.join(name).join("pack.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| PackInfo { name: name.to_owned(), license: "CC0".into(), ..Default::default() })
}

/// Авторы всего, что есть среди ключей картинок `used`: наборы CC0 (по их описанию), WoW
/// (Blizzard, только для своего стола), ассеты Forgotten Adventures (их просьба — ссылка и фраза),
/// прочие паки Dungeondraft.
pub(super) fn credits(used: &std::collections::BTreeSet<String>, cc0: &Path, wow: &[PathBuf]) -> Vec<Credit> {
    let low = |p: &str| p.replace('\\', "/").to_lowercase();
    let cc0_low = low(&cc0.to_string_lossy()) + "/";
    let wow_low: Vec<String> = wow.iter().map(|d| low(&d.to_string_lossy()) + "/").collect();
    let mut packs = std::collections::BTreeSet::new();
    let (mut wow_used, mut fa) = (false, false);
    let mut dd = std::collections::BTreeSet::new();
    for key in used {
        let k = low(key);
        if let Some(rest) = k.strip_prefix(&cc0_low) {
            // Имя набора — как папка лежит на диске.
            let lower = rest.split('/').next().unwrap_or_default();
            let real = std::fs::read_dir(cc0).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.to_lowercase() == lower);
            packs.insert(real.unwrap_or_else(|| lower.to_owned()));
        } else if wow_low.iter().any(|w| k.starts_with(w.as_str())) {
            wow_used = true;
        } else if k.contains("fa_objects") || k.contains("fa_textures") || k.contains("forgotten") {
            fa = true;
        } else if let Some(i) = key.find("pack:") {
            // Ключ пака — `pack:<файл пака>|<путь внутри>`, перед ним бывает подкраска `tint:…:`.
            let pack = key[i + 5..].split('|').next().unwrap_or_default();
            let stem = Path::new(pack).file_stem().map_or_else(|| pack.to_owned(), |s| s.to_string_lossy().into_owned());
            dd.insert(stem);
        }
    }
    let mut out = Vec::new();
    for p in packs {
        let i = pack_info(cc0, &p);
        out.push(Credit { what: format!("3D-модели «{}»", i.name), by: i.by, license: i.license, url: i.url, note: i.note });
    }
    if fa {
        out.push(Credit { what: "Ассеты Forgotten Adventures".into(), by: "Forgotten Adventures".into(), license: "лицензия Forgotten Adventures".into(), url: "https://www.forgotten-adventures.net".into(), note: "Maps were created using assets from Forgotten Adventures.".into() });
    }
    for d in dd {
        out.push(Credit { what: format!("Пак Dungeondraft «{d}»"), by: String::new(), license: String::new(), url: String::new(), note: String::new() });
    }
    if wow_used {
        out.push(Credit { what: "Модели и картинки World of Warcraft".into(), by: "Blizzard Entertainment".into(), license: "только для своего стола — не распространять".into(), url: String::new(), note: String::new() });
    }
    out
}

/// Авторы текстом — файлом рядом со сценой.
pub(super) fn credits_text(scene: &str, credits: &[Credit]) -> String {
    let mut s = format!("Авторы — «{scene}»\n\n");
    for c in credits {
        s.push_str(&format!("- {}", c.what));
        for part in [&c.by, &c.license, &c.url] {
            if !part.is_empty() {
                s.push_str(&format!(" — {part}"));
            }
        }
        if !c.note.is_empty() {
            s.push_str(&format!("\n  {}", c.note));
        }
        s.push('\n');
    }
    s.push_str("\nКарта сделана в Картографе.\n");
    s
}

/// `--cc0 папка [--cc0-unit клеток]`: набор — на полку в папке данных `data` (`cc0/<набор>/…`),
/// итог — в консоль.
pub fn import_cc0_cli(src: &Path, data: &Path, unit: Option<f32>) -> i32 {
    let unit = unit.unwrap_or(model3d::PER_METER);
    let out = data.join("cc0");
    let files = model_files(src);
    let mut failed = 0;
    let mut taken = std::collections::HashSet::new();
    for f in &files {
        match import_cc0_file(f, src, &out, unit, &mut taken) {
            Ok(png) => println!("{}", png.display()),
            Err(e) => {
                failed += 1;
                eprintln!("{}: {e}", f.display());
            }
        }
    }
    copy_pack_info(src, &out);
    println!("CC0: на полке {} моделей из {} ({})", files.len() - failed, files.len(), out.display());
    if files.is_empty() || failed == files.len() { 1 } else { 0 }
}

impl App {
    pub(super) fn cc0_dir(&self) -> PathBuf {
        self.data_dir.join("cc0")
    }

    /// Скачать набор с зеркала на полку — в фоне, ход — как у переноса своего набора.
    pub(super) fn start_mirror(&mut self, tag: &'static str) {
        let (tx, rx) = mpsc::channel();
        let cc0 = self.cc0_dir();
        crate::plat::spawn(move || {
            let tx2 = tx.clone();
            if let Err(e) = download_pack(tag, &cc0, move |n| {
                let _ = tx2.send(n);
            }) {
                let _ = tx.send(Cc0Note::One(Err(e)));
            }
            let _ = tx.send(Cc0Note::End);
        });
        self.cc0 = Some(Cc0Job { rx, done: 0, total: 0, errors: Vec::new() });
    }

    /// Авторы всего, что на этажах выгрузки: ключи картинок всех этажей.
    pub(crate) fn credits_for(&self, used: &std::collections::BTreeSet<String>) -> Vec<Credit> {
        credits(used, &self.cc0_dir(), &[self.wow_dir(), crate::wow::location::tiles_dir(&self.data_dir)])
    }

    /// Открыть окно «3D-модели: наборы и авторы» (меню, самопроверка).
    pub(crate) fn open_models_window(&mut self) {
        self.models_open = true;
    }

    pub(crate) fn close_models_window(&mut self) {
        self.models_open = false;
    }

    /// Окно «3D-модели: наборы и авторы»: что на полке и чьё, наборы с зеркала, свой набор.
    pub(super) fn models_window(&mut self, ctx: &egui::Context) {
        if !self.models_open {
            return;
        }
        let mut open = true;
        let mut get: Option<&'static str> = None;
        let mut own = false;
        let cc0 = self.cc0_dir();
        let busy = self.cc0.as_ref().map(|j| (j.done, j.total));
        let count = |dir: &Path, ext: &str| std::fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))).count();
        let mut packs: Vec<String> = std::fs::read_dir(&cc0).into_iter().flatten().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        packs.sort();
        let wow = library::scan(&self.wow_dir()).len();
        let mut matched = self.settings.fa_models;
        egui::Window::new("3D-модели: наборы и авторы").open(&mut open).default_width(560.0).show(ctx, |ui| {
            ui.heading("На полке");
            if packs.is_empty() && wow == 0 {
                ui.weak("Наборов 3D-моделей пока нет.");
            }
            for p in &packs {
                let info = pack_info(&cc0, p);
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("CC0/{p}"));
                    ui.label(format!("— моделей {}", count(&cc0.join(p), "glb")));
                });
                ui.horizontal_wrapped(|ui| {
                    let who = if info.by.is_empty() { "автор не указан".to_owned() } else { info.by.clone() };
                    ui.label(format!("{who}, {}", if info.license.is_empty() { "лицензия не указана" } else { &info.license }));
                    if !info.url.is_empty() {
                        ui.hyperlink(&info.url);
                    }
                });
                if !info.note.is_empty() {
                    ui.weak(&info.note);
                }
                ui.add_space(4.0);
            }
            if wow > 0 {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("WoW/…");
                    ui.label(format!("— предметов {wow}: Blizzard Entertainment, из клиента игры — только для своего стола, не распространять."));
                });
            }
            ui.separator();
            ui.checkbox(&mut matched, format!("Подбирать модели к рисункам FA по имени (из «CC0/{}»)", crate::fa3d::PACK)).on_hover_text("Бочкам, бочонкам, ящикам, столам, стульям, табуретам, книжным шкафам, вёдрам и фонарям без своей модели — модель с полки по имени рисунка, в размер нарисованного; у полных бочек и ящиков с товаром сверху — сам рисунок. В окне 3D и в выгрузке с 3D-моделью; на карте и в 2D — прежние рисунки");
            ui.separator();
            ui.heading("Скачать");
            ui.weak("Наборы CC0 с зеркала в выпусках Картографа на GitHub: модели, вид сверху и масштаб уже готовы.");
            for (title, tag) in MIRRORS {
                ui.horizontal(|ui| {
                    ui.label(*title);
                    let have = packs.iter().any(|p| p == title);
                    if ui.add_enabled(busy.is_none(), egui::Button::new(if have { "Скачать заново" } else { "Скачать" })).clicked() {
                        get = Some(tag);
                    }
                });
            }
            if let Some((done, total)) = busy {
                ui.add(egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 }).text(format!("{done} из {total}")));
            }
            ui.separator();
            if ui.add_enabled(busy.is_none(), egui::Button::new("Перенести свой набор (.glb, .gltf)…")).on_hover_text("Папка с моделями: вид сверху и модель каждой — на полку «CC0/<папка>». Единица — метр; для наборов в своих единицах — ключ --cc0-unit").clicked() {
                own = true;
            }
            ui.weak("Авторы того, что стоит на карте, уходят и в выгрузку: в сцену и файлом «…_credits.txt» рядом с ней.");
        });
        self.models_open = open;
        if matched != self.settings.fa_models {
            self.settings.fa_models = matched;
            self.models3d.clear();
            self.fa3d.clear();
            self.save_settings();
        }
        if let Some(tag) = get {
            self.start_mirror(tag);
        }
        if own && let Some(dir) = crate::plat::FileDialog::new().pick_folder() {
            self.start_cc0(dir);
        }
    }

    /// Перенести набор CC0 из папки на полку («CC0/<набор>/…») — в фоне.
    pub(super) fn start_cc0(&mut self, src: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let out = self.cc0_dir();
        crate::plat::spawn(move || {
            let files = model_files(&src);
            let _ = tx.send(Cc0Note::Total(files.len()));
            let mut taken = std::collections::HashSet::new();
            for f in files {
                let r = import_cc0_file(&f, &src, &out, model3d::PER_METER, &mut taken).map(|_| ()).map_err(|e| format!("{}: {e}", f.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
                let _ = tx.send(Cc0Note::One(r));
            }
            copy_pack_info(&src, &out);
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
        self.fa3d.clear();
        let ok = job.done - job.errors.len();
        self.notice = Some(if job.total == 0 { "CC0: в папке нет моделей .glb и .gltf".into() } else { format!("CC0: на полке {ok} моделей из {} (папка «CC0/…»)", job.total) });
        if !job.errors.is_empty() {
            self.error = Some(format!("CC0: не перенеслись {} моделей:\n{}", job.errors.len(), job.errors.iter().take(12).cloned().collect::<Vec<_>>().join("\n")));
        }
    }

    /// Файл модели предмета: `.glb` рядом с картинкой. У картинки полки WoW без него — найти
    /// модель в клиенте (если он задан) и записать. У прочих — подобрать по имени рисунка
    /// ([`crate::fa3d`]).
    fn model_file(&mut self, key: &str) -> Option<PathBuf> {
        let png = Path::new(key);
        let glb = png.with_extension("glb");
        if glb.is_file() {
            return Some(glb);
        }
        let low = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
        let wow = self.wow_dir();
        if !low(png).starts_with(&(low(&wow) + "/")) {
            let cc0 = low(&self.cc0_dir()) + "/";
            return if low(png).contains(&cc0) { None } else { self.matched_model(key).map(|(glb, _)| glb) };
        }
        // Картинка полки WoW: модель — из клиента, если он задан.
        self.settings.wow_client.as_ref()?;
        let client = self.wow_client_now(None).ok()?;
        let model = crate::wow::model_for(&wow, png, &self.wow.models)?;
        let (m, textures, _) = crate::wow::load(&client, &model).ok()?;
        let stem = png.file_stem()?.to_string_lossy().into_owned();
        crate::wow::write_model(&m, &textures, &glb, &stem).ok()?;
        Some(glb)
    }

    /// Модель, подобранная к рисунку по имени (с запоминанием): записанная `.glb` и модель с полки,
    /// из которой она. Подбор выключен, правила нет, набора на полке нет, ни одна модель не
    /// подходит по сторонам — `None`.
    pub(super) fn matched_model(&mut self, key: &str) -> Option<(PathBuf, PathBuf)> {
        if !self.settings.fa_models {
            return None;
        }
        if let Some(hit) = self.fa3d.get(key) {
            return hit.clone();
        }
        let hit = self.match_model(key);
        self.fa3d.insert(key.to_owned(), hit.clone());
        hit
    }

    fn match_model(&mut self, key: &str) -> Option<(PathBuf, PathBuf)> {
        use crate::fa3d;
        let rule = fa3d::rule_for(key)?;
        let pack = self.cc0_dir().join(fa3d::PACK);
        let img = self.textures.rgba(key).ok()?;
        let opaque = fa3d::opaque_box(&img)?;
        let frame = library::picture_size(key, [img.width(), img.height()]);
        let mut best: Option<(PathBuf, fa3d::Placement)> = None;
        for name in rule.models {
            let Some(file) = fa3d::shelf_file(&pack, name) else { continue };
            let Some(bounds) = model3d::read_file(&file, 1.0).ok().and_then(|m| m.bounds()) else { continue };
            let p = fa3d::place(bounds, frame, opaque, rule.lying);
            if best.as_ref().is_none_or(|(_, b)| p.skew < b.skew) {
                best = Some((file, p));
            }
        }
        let (src, p) = best.filter(|(_, p)| p.skew <= rule.stretch)?;
        let meta = std::fs::metadata(&src).ok()?;
        let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        let tag = sha256_hex(format!("{key}|{}|{}|{stamp}|{}", src.display(), meta.len(), fa3d::VERSION).as_bytes());
        let dir = self.data_dir.join("cache").join("fa3d");
        let out = dir.join(format!("{}_{}.glb", crate::export::asset_stem(key), &tag[..10]));
        if !out.is_file() {
            // Полная бочка, ящик с товаром: сверху — сам рисунок, чуть выше верха модели.
            use image::ImageEncoder as _;
            let mut png = Vec::new();
            if rule.top {
                image::codecs::png::PngEncoder::new(&mut png).write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8).ok()?;
            }
            let lid = rule.top.then_some((png.as_slice(), p.top + 0.01));
            let glb = model3d::place_gltf(&std::fs::read(&src).ok()?, p.matrix, p.frame, lid).ok()?;
            std::fs::create_dir_all(&dir).ok()?;
            std::fs::write(&out, glb).ok()?;
        }
        Some((out, src))
    }

    /// Ключи картинок `used` и модели с полки, подобранные к ним: авторы моделей — тоже в
    /// авторы сцены.
    pub(super) fn with_matched(&self, used: &mut std::collections::BTreeSet<String>) {
        let extra: Vec<String> = used.iter().filter_map(|k| self.fa3d.get(k).cloned().flatten()).map(|(_, src)| src.to_string_lossy().into_owned()).collect();
        used.extend(extra);
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

/// Предмет для укладки в 3D: середина, рамка (клетки), поворот (градусы) и верх его модели над
/// его же низом (клетки; нет модели — плоский).
pub(super) struct Rest {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub rot: f32,
    pub top: Option<f32>,
}

/// На какой высоте над полом стоит каждый предмет (клетки), по порядку рисования на карте (снизу
/// вверх): на 2D-карте «на столе» — просто слой выше, в 3D подсвечник иначе провалился бы под стол.
/// Предмет встаёт на верх модели, что ниже его по порядку, если его середина внутри её рамки и сам
/// он меньше её; таких несколько — на самую высокую.
pub(super) fn rests(items: &[Rest]) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::with_capacity(items.len());
    // Окно 3D укладывает заново при каждой правке предметов: поворот — один раз на предмет.
    let turns: Vec<(f32, f32)> = items.iter().map(|it| (-it.rot).to_radians().sin_cos()).collect();
    for (i, it) in items.iter().enumerate() {
        let area = it.size[0] * it.size[1];
        let mut h = 0.0f32;
        for (j, under) in items[..i].iter().enumerate() {
            let Some(top) = under.top else { continue };
            if under.size[0] * under.size[1] <= area {
                continue;
            }
            // Середина предмета — в осях нижнего (поворот по часовой на экране, y вниз).
            let (s, c) = turns[j];
            let (dx, dy) = (it.pos[0] - under.pos[0], it.pos[1] - under.pos[1]);
            let (lx, ly) = (dx * c - dy * s, dx * s + dy * c);
            if lx.abs() <= under.size[0] / 2.0 && ly.abs() <= under.size[1] / 2.0 {
                h = h.max(out[j] + top);
            }
        }
        out.push(h);
    }
    out
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
    fn a_candle_stands_on_the_table_under_it_and_a_chair_beside_stays_on_the_floor() {
        let r = |pos: [f32; 2], size: [f32; 2], rot: f32, top: Option<f32>| Rest { pos, size, rot, top };
        let items = [
            r([5.0, 5.0], [2.0, 1.0], 90.0, Some(0.5)), // стол, повёрнут: вдоль y
            r([5.0, 5.8], [0.2, 0.2], 0.0, Some(0.3)),  // подсвечник у края — внутри повёрнутого стола
            r([5.0, 5.8], [0.1, 0.1], 0.0, None),       // огонёк на подсвечнике
            r([6.2, 5.0], [0.5, 0.5], 0.0, Some(0.9)),  // стул рядом — на полу
            r([5.0, 5.0], [3.0, 3.0], 0.0, None),       // ковёр больше стола — на полу
        ];
        let h = rests(&items);
        assert_eq!(h[0], 0.0);
        assert_eq!(h[1], 0.5, "на столе");
        assert!((h[2] - 0.8).abs() < 1e-6, "на подсвечнике на столе");
        assert_eq!(h[3], 0.0, "стул не на столе");
        assert_eq!(h[4], 0.0, "больше стола — на полу");
    }

    #[test]
    fn credits_name_every_source_and_the_wording_authors_ask_for() {
        let dir = std::env::temp_dir().join(format!("kartograf-credits-{}", std::process::id()));
        let cc0 = dir.join("cc0");
        std::fs::create_dir_all(cc0.join("Poly Haven")).unwrap();
        let info = PackInfo { name: "Poly Haven".into(), by: "Poly Haven".into(), url: "https://polyhaven.com".into(), license: "CC0 1.0".into(), ..Default::default() };
        std::fs::write(cc0.join("Poly Haven").join("pack.json"), serde_json::to_string(&info).unwrap()).unwrap();
        let wow = dir.join("wow");
        let used: std::collections::BTreeSet<String> = [
            cc0.join("Poly Haven").join("Barrel_01_0.39x0.39.png").to_string_lossy().into_owned(),
            wow.join("Generic").join("Torch_0.2x0.2.png").to_string_lossy().into_owned(),
            "pack:E:\\D&D\\FA_Objects_A_v3.52.dungeondraft_pack|textures/objects/Barrel_A1_1x1.webp".into(),
            "tint:96322a:pack:E:\\packs\\2MT World Map.dungeondraft_pack|textures/hex.png".into(),
        ]
        .into_iter()
        .collect();
        let c = credits(&used, &cc0, &[wow]);
        let _ = std::fs::remove_dir_all(&dir);
        let whats: Vec<&str> = c.iter().map(|c| c.what.as_str()).collect();
        assert_eq!(whats, ["3D-модели «Poly Haven»", "Ассеты Forgotten Adventures", "Пак Dungeondraft «2MT World Map»", "Модели и картинки World of Warcraft"]);
        assert_eq!(c[0].url, "https://polyhaven.com");
        assert!(c[1].note.contains("created using assets from Forgotten Adventures") && c[1].url.contains("forgotten-adventures.net"));
        assert!(c[3].license.contains("не распространять"));
        let text = credits_text("Таверна", &c);
        assert!(text.contains("Poly Haven — CC0 1.0 — https://polyhaven.com"), "{text}");
    }

    #[test]
    fn a_mirror_file_name_cannot_climb_out_of_its_pack() {
        assert!(plain_file("Barrel_01_0.39x0.39.glb"));
        for bad in ["", "..", "../x.glb", "a/b.glb", "a\\b.glb", "C:x.glb"] {
            assert!(!plain_file(bad), "{bad}");
        }
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
        std::fs::write(pack.join("props").join("crateSmall.gltf.glb"), m.write_glb("crate").unwrap()).unwrap();
        let mut taken = std::collections::HashSet::new();
        let png = import_cc0_file(&pack.join("props").join("crateSmall.gltf.glb"), &pack, &out, model3d::PER_METER, &mut taken).unwrap();
        // Та же модель под другим регистром имени — рядом, а не поверх.
        std::fs::copy(pack.join("props").join("crateSmall.gltf.glb"), pack.join("props").join("CrateSmall.glb")).unwrap();
        let twin = import_cc0_file(&pack.join("props").join("CrateSmall.glb"), &pack, &out, model3d::PER_METER, &mut taken).unwrap();
        assert!(png.is_file() && twin.is_file() && twin != png, "{} {}", png.display(), twin.display());
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
