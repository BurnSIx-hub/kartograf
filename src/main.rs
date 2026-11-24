//! Картограф — редактор карт для настольных игр.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports, unused_variables, unused_mut))]

mod app;
mod blend;
mod boundless;
mod check;
#[cfg(not(target_arch = "wasm32"))]
mod control;
mod export;
mod generate;
mod grid;
mod library;
mod mappick;
mod map;
mod mask;
#[cfg(not(target_arch = "wasm32"))]
mod mcp;
#[cfg(not(target_arch = "wasm32"))]
mod sheet;
#[cfg(not(target_arch = "wasm32"))]
mod lighting;
mod palette;
mod pck;
mod place;
mod plan;
mod plat;
mod platform;
mod prefab;
mod procgen;
mod recovery;
mod relief;
mod roofs;
mod selftest;
mod shotcache;
mod skeleton;
mod sets;
#[cfg(not(target_arch = "wasm32"))]
mod update;
#[cfg(target_arch = "wasm32")]
#[path = "update_web.rs"]
mod update;
mod view;
#[cfg(not(target_arch = "wasm32"))]
mod wow;

use eframe::egui;
use std::path::PathBuf;

/// Ключи запуска: печатаются по `--help` и при неизвестном ключе.
#[cfg(not(target_arch = "wasm32"))]
const USAGE: &str = "\
kartograf [карта.karta] [ключи]
  --data-dir <папка>     папка данных: настройки, паки, свои ассеты
  --mcp                  без окна: сервер MCP, правит карту в открытом окне
  --aside                окно за краем экрана и без фокуса
  --selftest <папка>     самопроверка окна, итог в <папка>/report.txt; данные — только свои, <папка>/data
  --shot <файл.png>      снимок карты и выход; --size ШxВ — размер окна
  --export <папка>       выгрузка для Foundry и выход; --px <точек на клетку>
  --plan <файл.json>     построить карту по плану
  --osm <файл.osm>       местность из выгрузки OpenStreetMap
  --place <широта,долгота> местность по координатам; --cells ШxВ, --trees 0..1
  --update <адрес>       поставить обновление
  --packinfo <пак…>      что в паке
  --analyse <участок.osm…> сверка анализа снимка
  --version, --help";

/// Сказать в консоль. У программы без консоли (выпуск под Windows) сначала подключиться
/// к консоли того, кто запустил: иначе в cmd и PowerShell ответа не видно.
#[cfg(not(target_arch = "wasm32"))]
fn say(text: &str) {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn AttachConsole(process: u32) -> i32;
        }
        // SAFETY: вызов без указателей; `u32::MAX` — консоль родителя. Нет её или вывод уже
        // перенаправлен в канал или файл — ничего не меняется.
        unsafe {
            AttachConsole(u32::MAX);
        }
    }
    println!("{text}");
}

/// Разобрать ключи запуска. `Err` — окно открывать нельзя: ключ не узнан или сочетание
/// ключей испортит чужие данные.
#[cfg(not(target_arch = "wasm32"))]
fn parse_args(list: impl IntoIterator<Item = String>) -> Result<app::Args, String> {
    let mut args = app::Args { file: None, selftest: None, shot: None, size: None, data: None, export: None, px: None, plan: None, osm: None, place: None, cells: None, trees: None };
    let mut it = list.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--selftest" => args.selftest = it.next().map(PathBuf::from),
            "--shot" => args.shot = it.next().map(PathBuf::from),
            "--data-dir" => args.data = it.next().map(PathBuf::from),
            "--export" => args.export = it.next().map(PathBuf::from),
            "--px" => args.px = it.next().and_then(|s| s.parse().ok()),
            "--plan" => args.plan = it.next().map(PathBuf::from),
            "--osm" => args.osm = it.next().map(PathBuf::from),
            "--place" => args.place = it.next(),
            "--trees" => args.trees = it.next().and_then(|s| s.parse().ok()),
            "--cells" => args.cells = it.next().and_then(|s| s.split_once(['x', 'х']).and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))),
            "--aside" => {}
            "--update" => std::process::exit(update::cli(&it.next().unwrap_or_default())),
            "--packinfo" => std::process::exit(packinfo(&it.by_ref().map(PathBuf::from).collect::<Vec<_>>())),
            "--analyse" => std::process::exit(place::compare(&it.by_ref().map(PathBuf::from).collect::<Vec<_>>())),
            "--size" => args.size = it.next().and_then(|s| s.split_once(['x', 'х']).and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))),
            "--version" => {
                say(&format!("kartograf {}", env!("CARGO_PKG_VERSION")));
                std::process::exit(0)
            }
            "--help" | "-h" | "/?" => {
                say(USAGE);
                std::process::exit(0)
            }
            // Ключ, которого нет, — опечатка, а не имя карты: раньше `--version` открывал
            // окно с такой «картой» поверх всего.
            _ if a.starts_with("--") => return Err(format!("неизвестный ключ {a}")),
            _ => args.file = Some(PathBuf::from(a)),
        }
    }
    // Самопроверка ставит свою папку текстур и пробный пак и переписывает settings.json:
    // в рабочей папке данных это ломает настройки пользователя.
    if args.selftest.is_some() && args.data.is_some() {
        return Err("--selftest работает только в своей папке данных (<папка>/data): с --data-dir самопроверка переписала бы настройки этой папки. Уберите --data-dir".to_owned());
    }
    Ok(args)
}

/// `--packinfo пак…`: без окна — сколько в паке ассетов каждого вида, за сколько читается
/// оглавление и раскрывается ли первая картинка каждого вида.
#[cfg(not(target_arch = "wasm32"))]
fn packinfo(files: &[PathBuf]) -> i32 {
    let mut code = 0;
    for f in files {
        let started = std::time::Instant::now();
        match pck::read_index(f) {
            Ok(pack) => {
                println!("{}: {} картинок, оглавление {:.2} с", pack.name, pack.entries.len(), started.elapsed().as_secs_f64());
                let mut seen = std::collections::BTreeMap::new();
                for e in &pack.entries {
                    let section = e.path.split('/').nth(1).unwrap_or("?").to_owned();
                    let first = seen.entry(section).or_insert((0, e.clone()));
                    first.0 += 1;
                }
                for (section, (n, e)) in seen {
                    let t = std::time::Instant::now();
                    let decoded = pck::read_bytes(&pack.file, e.offset, e.size).and_then(|b| image::load_from_memory(&b).map_err(|x| x.to_string()));
                    match decoded {
                        Ok(img) => println!("  {section}: {n}; {} — {}×{}, {:.0} мс", e.path, img.width(), img.height(), t.elapsed().as_secs_f64() * 1000.0),
                        Err(err) => {
                            println!("  {section}: {n}; {} НЕ РАСКРЫЛАСЬ: {err}", e.path);
                            code = 1;
                        }
                    }
                }
            }
            Err(e) => {
                println!("{e}");
                code = 2;
            }
        }
    }
    code
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // `--mcp`: без окна — сервер MCP для ИИ; правит карту в открытом окне редактора.
    let all: Vec<String> = std::env::args().collect();
    if all.iter().any(|a| a == "--mcp") {
        let dir = all.iter().position(|a| a == "--data-dir").and_then(|i| all.get(i + 1)).map(PathBuf::from);
        let explicit = dir.is_some();
        std::process::exit(mcp::serve(dir.unwrap_or_else(app::default_data_dir), explicit));
    }
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            say(&format!("{e}\n\n{USAGE}"));
            std::process::exit(2)
        }
    };
    // Самопроверке нужен один и тот же размер окна, иначе точки щелчков поплывут.
    let size = args.size.or(args.selftest.as_ref().map(|_| [1280.0, 800.0])).unwrap_or([1400.0, 900.0]);
    let mut viewport = egui::ViewportBuilder::default().with_title("Картограф").with_inner_size(size).with_min_inner_size([640.0, 420.0]);
    // `--aside`: окно за краем экрана и без фокуса — снимок не мешает, пока впереди игра.
    if std::env::args().any(|a| a == "--aside") {
        viewport = viewport.with_position([-5000.0, 100.0]).with_active(false);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("kartograf", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
}

/// Модуль Foundry: окно рисуется на холсте `#kartograf-canvas`, который заводит обёртка модуля.
#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    let args = app::Args { file: None, selftest: None, shot: None, size: None, data: None, export: None, px: None, plan: None, osm: None, place: None, cells: None, trees: None };
    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("kartograf-canvas"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("нет холста #kartograf-canvas");
        let started = eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
            .await;
        if let Err(e) = started {
            web_sys::console::error_1(&e);
        }
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn parse(list: &[&str]) -> Result<app::Args, String> {
        parse_args(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn unknown_flag_is_refused_not_opened_as_a_map() {
        assert_eq!(parse(&["--versoin"]).err().as_deref(), Some("неизвестный ключ --versoin"));
        let ok = parse(&["карта.karta", "--data-dir", "d", "--aside", "--px", "140"]).ok().expect("знакомые ключи");
        assert_eq!(ok.file, Some(PathBuf::from("карта.karta")));
        assert_eq!((ok.data, ok.px), (Some(PathBuf::from("d")), Some(140)));
        // Каждый ключ, который разбирается, есть в справке.
        for flag in ["--data-dir", "--mcp", "--aside", "--selftest", "--shot", "--size", "--export", "--px", "--plan", "--osm", "--place", "--cells", "--trees", "--update", "--packinfo", "--analyse", "--version", "--help"] {
            assert!(USAGE.contains(flag), "{flag}");
        }
    }

    #[test]
    fn selftest_keeps_to_its_own_data_dir() {
        assert!(parse(&["--aside", "--selftest", "t"]).is_ok());
        let refused = parse(&["--selftest", "t", "--data-dir", "d"]).err().expect("отказ");
        assert!(refused.contains("--data-dir"), "{refused}");
        assert!(parse(&["--data-dir", "d", "--selftest", "t"]).is_err(), "порядок ключей не важен");
    }
}
