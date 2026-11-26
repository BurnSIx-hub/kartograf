//! Предметы из клиента World of Warcraft 1.x–3.x: модель M2 строго сверху — в картинку-ассет.
//! Клиент читается сам (архивы MPQ в порядке игры), без wow.export и выгрузок.

pub mod adt;
pub mod blp;
pub mod gltf;
pub mod m2;
pub mod mpq;
pub mod location;
pub mod render;
pub mod wmo;

use std::path::{Path, PathBuf};

pub use mpq::Client;

/// Что получилось при переносе модели.
pub struct Imported {
    pub file: PathBuf,
    pub cells: [f32; 2],
    pub drawn: usize,
    pub skipped: usize,
    /// Текстуры, которых нет или которые подменяются игрой (кожа существа) — нарисованы серым.
    pub missing: Vec<String>,
    /// Начало координат модели в клетках от середины картинки (`render::Sprite::origin`).
    pub origin: [f32; 2],
}

/// Текстура подменяемого вида: игра берёт её из таблиц. Рядом с моделью обычно лежит
/// одноимённая — `Chest01.m2` → `Chest01*.blp` в той же папке.
fn guess_texture(client: &Client, model: &str) -> Option<String> {
    let stem = model.rsplit_once('.').map_or(model, |(s, _)| s).to_lowercase();
    let dir = stem.rsplit_once('\\').map_or("", |(d, _)| d).to_owned();
    let exact = format!("{stem}.blp");
    let names = client.names();
    if names.iter().any(|n| n.to_lowercase() == exact) {
        return Some(exact);
    }
    names
        .iter()
        .filter(|n| {
            let l = n.to_lowercase();
            l.ends_with(".blp") && l.starts_with(&stem) && l.rsplit_once('\\').map_or("", |(d, _)| d) == dir
        })
        .min_by_key(|n| n.len())
        .cloned()
}

/// Модель, её текстуры (нет — `None`) и что не нашлось.
pub type Loaded = (m2::Model, Vec<Option<blp::Rgba>>, Vec<String>);

/// Модель и её текстуры из клиента — для отрисовки и проб.
pub fn load(client: &Client, path: &str) -> Result<Loaded, String> {
    let path = m2::m2_path(path);
    let bytes = client.read(&path)?;
    let version = bytes.get(4..8).map_or(0, |v| u32::from_le_bytes(v.try_into().unwrap()));
    let skin = if version >= 264 { Some(client.read(&m2::skin_path(&path))?) } else { None };
    let model = m2::parse(&bytes, skin.as_deref())?;
    let mut missing = Vec::new();
    let textures = model
        .textures
        .iter()
        .map(|t| {
            let name = if t.file.is_empty() { guess_texture(client, &path) } else { Some(t.file.clone()) };
            let Some(name) = name else {
                missing.push(format!("подменяемая текстура вида {}", t.kind));
                return None;
            };
            match client.read(&name).and_then(|b| blp::decode(&b)) {
                Ok(img) => Some(img),
                Err(e) => {
                    missing.push(format!("{name}: {e}"));
                    None
                }
            }
        })
        .collect();
    Ok((model, textures, missing))
}

/// Число для имени файла: `1.50` → `1.5`, `2.00` → `2`.
fn num(v: f32) -> String {
    format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Папка внутри `out`: путь модели без `World\` и без имени файла.
pub fn asset_dir(out: &Path, model: &str) -> PathBuf {
    let parts: Vec<&str> = model.split(['\\', '/']).collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    let dirs = if dirs.first().is_some_and(|d| d.eq_ignore_ascii_case("world")) { &dirs[1..] } else { dirs };
    dirs.iter().fold(out.to_owned(), |p, d| p.join(d))
}

/// Слитное имя WoW — словами через `_`: `StormwindBrazier01` → `Stormwind_Brazier_01`.
/// По словам имени Картограф решает, что светит (жаровня, канделябр), и собирает варианты
/// (`_01`, `_02`) в одну семью на полке.
pub fn words_name(stem: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in stem.chars() {
        if !c.is_ascii_alphanumeric() {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let split = (p.is_ascii_lowercase() && c.is_ascii_uppercase()) || (p.is_ascii_alphabetic() && c.is_ascii_digit()) || (p.is_ascii_digit() && c.is_ascii_alphabetic());
            if split && !out.ends_with('_') {
                out.push('_');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out.trim_end_matches('_').to_owned()
}

/// Папка капсом (`PASSIVE DOODADS`) — обычным видом (`Passive Doodads`); прочие как есть.
fn calm_case(part: &str) -> String {
    if !part.chars().any(|c| c.is_ascii_uppercase()) || part.chars().any(|c| c.is_ascii_lowercase()) {
        return part.to_owned();
    }
    let mut out = String::new();
    let mut start = true;
    for c in part.chars() {
        out.push(if start { c } else { c.to_ascii_lowercase() });
        start = !c.is_ascii_alphanumeric();
    }
    out
}

/// Имя в папке без учёта регистра: архивы пишут одну папку по-разному (`DUNGEON`, `Dungeon`),
/// Windows считает их одной, а ключи полки — нет. Берём написание с диска.
fn on_disk(dir: &Path, name: &str) -> Option<std::ffi::OsString> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name()).find(|f| f.to_string_lossy().eq_ignore_ascii_case(name))
}

/// `asset_dir`, но части, что уже есть на диске под другим регистром, — как на диске.
fn resolve_dir(out: &Path, model: &str) -> PathBuf {
    let rel = asset_dir(Path::new(""), model);
    let mut p = out.to_owned();
    for part in rel.components() {
        let part = part.as_os_str().to_string_lossy();
        p = match on_disk(&p, &part) {
            Some(existing) => p.join(existing),
            None => p.join(calm_case(&part)),
        };
    }
    p
}

/// Нарисовать модель сверху и положить картинку в `out` (папку WoW в данных программы):
/// `out/<путь модели>/<Имя>_<ш>x<в>.png`, размер в клетках — в имени, как у ассетов FA.
/// Прежняя картинка той же модели (другого размера) заменяется.
pub fn import(client: &Client, model: &str, out: &Path, opt: &render::Options) -> Result<Imported, String> {
    // Имя — как в списках архивов: из расстановки WMO оно приходит капсом, и слова не видны.
    let model = &client.spelled(&m2::m2_path(model));
    let (m, textures, missing) = load(client, model)?;
    let sprite = render::render(&m, &textures, opt)?;
    let dir = resolve_dir(out, model);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file_name = model.rsplit(['\\', '/']).next().unwrap_or(model);
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    // Имя латиницей, словами через `_`: картинка уходит в Foundry под своим именем.
    let stem = words_name(stem);
    // Прежняя картинка той же модели — с размером в хвосте имени и в любом регистре.
    let prefix = format!("{}_", stem.to_lowercase());
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_lowercase();
        if let Some(rest) = n.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".png"))
            && !rest.contains('_')
            && crate::library::size_from_name(&format!("x_{rest}")).is_some()
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let name = format!("{stem}_{}x{}.png", num(sprite.cells[0]), num(sprite.cells[1]));
    let file = dir.join(&name);
    sprite.img.save(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    // Ключ — как имя легло на диск (полка читает папку и берёт написание оттуда).
    let file = on_disk(&dir, &name).map_or(file, |n| dir.join(n));
    Ok(Imported { file, cells: sprite.cells, drawn: sprite.drawn, skipped: sprite.skipped, missing, origin: sprite.origin })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_folder_follows_the_model_path_without_world() {
        let out = Path::new("O");
        assert_eq!(asset_dir(out, "World\\Dungeon\\SM\\Statue.m2"), Path::new("O").join("Dungeon").join("SM"));
        assert_eq!(asset_dir(out, "Creature\\Bear\\Bear.m2"), Path::new("O").join("Creature").join("Bear"));
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(2.0), "2");
        assert_eq!(num(0.954), "0.95");
    }

    #[test]
    fn names_become_words_so_lights_and_families_work() {
        assert_eq!(words_name("StormwindBrazier01"), "Stormwind_Brazier_01");
        assert_eq!(words_name("CandelabraTall01"), "Candelabra_Tall_01");
        assert_eq!(words_name("statueHMpaladin"), "statue_HMpaladin");
        assert_eq!(words_name("AbbeyShelf01_Unselectable"), "Abbey_Shelf_01_Unselectable");
        assert_eq!(words_name("Bush 02 (old)"), "Bush_02_old");
        assert_eq!(crate::map::glow_by_name(&format!("x/{}_1.41x1.35.png", words_name("StormwindBrazier01"))), Some(crate::map::Glow::Brazier));
        assert_eq!(crate::map::glow_by_name(&format!("x/{}_1x0.4.png", words_name("GeneralCandelabra01"))), Some(crate::map::Glow::Candle));
        assert_eq!(calm_case("PASSIVE DOODADS"), "Passive Doodads");
        assert_eq!(calm_case("ScarletMonastery"), "ScarletMonastery");
    }

    #[test]
    fn folders_keep_the_spelling_already_on_disk() {
        let out = std::env::temp_dir().join(format!("kartograf_wow_case_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(out.join("Dungeon").join("SM")).unwrap();
        assert_eq!(resolve_dir(&out, "World\\DUNGEON\\sm\\Statue.m2"), out.join("Dungeon").join("SM"));
        assert_eq!(resolve_dir(&out, "World\\Dungeon\\Other\\A.m2"), out.join("Dungeon").join("Other"));
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Настоящий клиент: `WOW_CLIENT=папка WOW_OUT=папка cargo test wow_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn wow_real_client_models_to_pictures() {
        let (Ok(dir), Ok(out)) = (std::env::var("WOW_CLIENT"), std::env::var("WOW_OUT")) else { return };
        let t = std::time::Instant::now();
        let client = Client::open(Path::new(&dir)).unwrap();
        println!("архивов {} ({}), язык {}, {:?}", client.archives.len(), client.archives[0].path.display(), client.locale, t.elapsed());
        let models = std::env::var("MODELS").unwrap_or_else(|_| "World\\Dungeon\\ScarletMonastery\\PassiveDoodads\\Statues\\StatueHMPaladin.m2".into());
        for m in models.split(';') {
            let t = std::time::Instant::now();
            match import(&client, m, Path::new(&out), &render::Options::default()) {
                Ok(r) => println!("{m}: {} — {:.2}×{:.2} кл., треуг. {}, пропущено {}, нет текстур {:?}, {:?}", r.file.display(), r.cells[0], r.cells[1], r.drawn, r.skipped, r.missing, t.elapsed()),
                Err(e) => println!("{m}: ОШИБКА {e}"),
            }
        }
    }
}
