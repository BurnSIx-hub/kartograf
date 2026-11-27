//! Картограф — редактор карт для настольных игр.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports, unused_variables, unused_mut))]

mod app;
mod blend;
mod boundless;
mod check;
#[cfg(not(target_arch = "wasm32"))]
mod control;
mod doors3d;
mod export;
#[cfg(not(target_arch = "wasm32"))]
mod fa3d;
mod generate;
mod glb;
mod grid;
mod library;
mod mappick;
mod map;
mod mask;
mod model3d;
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
mod ground3d;
mod procgen;
mod recovery;
mod relief;
mod roof3d;
#[cfg(not(target_arch = "wasm32"))]
mod render3d;
mod roofs;
mod scene3d;
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
mod windows3d;
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
  --3d                   выгрузка ещё и с 3D-моделью карты (.glb) для 3D Canvas
  --view3d               сразу открыть тестовую зону 3D (с --shot — снять её)
  --cc0 <папка>          набор 3D-моделей CC0 (.glb, .gltf) — на полку «CC0/…», без окна;
                         --cc0-unit <клеток> — клеток в единице модели (по умолчанию метр, 0.656)
  --cc0-get <метка>      набор с зеркала в выпусках Картографа (models-polyhaven-1) — на полку
  --plan <файл.json>     построить карту по плану
  --osm <файл.osm>       местность из выгрузки OpenStreetMap
  --place <широта,долгота> местность по координатам; --cells ШxВ, --trees 0..1
  --update <адрес>       поставить обновление
  --packinfo <пак…>      что в паке
  --wow-glb <клиент> <здание.wmo|карта.wdt> <файл.glb> [ypc=3] [cut=4] [lit] [notop] [set=…] [area=x0,y0,x1,y1] [ground=256]
                         здание WoW 3D-моделью для 3D-вида Foundry
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
    let mut args = app::Args { file: None, selftest: None, shot: None, size: None, data: None, export: None, px: None, plan: None, osm: None, place: None, cells: None, trees: None, three_d: false, view3d: false, cc0: None, cc0_unit: None, cc0_get: None };
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
            "--3d" => args.three_d = true,
            "--view3d" => args.view3d = true,
            "--cc0" => args.cc0 = it.next().map(PathBuf::from),
            "--cc0-unit" => args.cc0_unit = it.next().and_then(|s| s.replace(',', ".").parse().ok()),
            "--cc0-get" => args.cc0_get = it.next(),
            "--update" => std::process::exit(update::cli(&it.next().unwrap_or_default())),
            "--packinfo" => std::process::exit(packinfo(&it.by_ref().map(PathBuf::from).collect::<Vec<_>>())),
            "--wow-glb" => std::process::exit(wow_glb(&it.by_ref().collect::<Vec<_>>())),
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

/// `--wow-glb клиент путь файл.glb [ypc=3] [cut=4] [lit] [set=…]`: без окна — здание (`.wmo`) или
/// карта инстанса (`.wdt`) WoW 3D-моделью glTF: рамка та же, что у карты из «Предметов из WoW»,
/// ярдов на клетку `ypc` (по умолчанию 5/3), срез над полом `cut` ярдов, `lit` — материалы под
/// свет сцены, а не только запечённый свет игры, `notop` — без отдельной модели срезанного верха,
/// `area` — рамка мира у карты (X — север, Y — запад, ярды; у материка обязательна), `ground` —
/// точек текстуры местности на кусок ADT (33⅓ ярда). Модели — по этажам (`<имя>.glb`, `_L1`…),
/// по верхним ярусам (`_S1`…: мосты над улицей и вторые этажи) и верх (`_top`). Рядом пишутся
/// `<имя>.json` (рамка и модели) и `<имя>_walls.json` (стены и зоны ярусов для 3D-сцены Foundry).
#[cfg(not(target_arch = "wasm32"))]
fn wow_glb(args: &[String]) -> i32 {
    let [client, path, out, rest @ ..] = args else {
        say("--wow-glb <папка клиента> <путь в клиенте> <файл.glb> [ypc=3] [cut=4] [lit] [notop] [set=…] [area=x0,y0,x1,y1] [ground=256]");
        return 2;
    };
    let mut opt = wow::gltf::Options::default();
    for a in rest {
        match a.split_once('=') {
            Some(("ypc", v)) => match v.replace(',', ".").parse::<f32>() {
                Ok(y) if y > 0.0 => opt.cells_per_yard = 1.0 / y,
                _ => {
                    say(&format!("ypc: не число «{v}»"));
                    return 2;
                }
            },
            Some(("cut", v)) => match v.replace(',', ".").parse::<f32>() {
                Ok(c) if c > 0.0 => opt.cut = c,
                _ => {
                    say(&format!("cut: не число «{v}»"));
                    return 2;
                }
            },
            Some(("set", v)) => opt.set = Some(v.to_owned()),
            Some(("area", v)) => match v.split(',').map(|s| s.trim().parse::<f32>()).collect::<Result<Vec<f32>, _>>() {
                Ok(r) if r.len() == 4 => opt.area = Some([r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]),
                _ => {
                    say(&format!("area: нужно четыре числа x0,y0,x1,y1, а не «{v}»"));
                    return 2;
                }
            },
            Some(("ground", v)) => match v.parse::<u32>() {
                Ok(g) if (8..=1024).contains(&g) => opt.ground = g,
                _ => {
                    say(&format!("ground: точек на кусок местности — от 8 до 1024, а не «{v}»"));
                    return 2;
                }
            },
            None if a == "lit" => opt.unlit = false,
            None if a == "notop" => opt.top = false,
            _ => {
                say(&format!("неизвестный ключ {a}"));
                return 2;
            }
        }
    }
    let t = std::time::Instant::now();
    let client = match wow::Client::open(std::path::Path::new(client)) {
        Ok(c) => c,
        Err(e) => {
            say(&format!("клиент: {e}"));
            return 1;
        }
    };
    match wow::gltf::export(&client, path, std::path::Path::new(out), &opt) {
        Ok(r) => {
            for (f, b) in r.files.iter().zip(&r.boxes) {
                say(&format!("{} — X {:.1}…{:.1}, Z {:.1}…{:.1}, высота {:.1}…{:.1} кл.", f.display(), b[0][0], b[1][0], b[0][2], b[1][2], b[0][1], b[1][1]));
            }
            say(&format!("карта {}×{} кл., треугольников {}, предметов {}, текстур {}, {:.1} с", r.cells[0], r.cells[1], r.triangles, r.doodads, r.textures, t.elapsed().as_secs_f64()));
            for n in &r.notes {
                say(&format!("  {n}"));
            }
            0
        }
        Err(e) => {
            say(&format!("ошибка: {e}"));
            1
        }
    }
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
    // `--cc0 папка`: набор 3D-моделей — на полку, без окна.
    if let Some(tag) = &args.cc0_get {
        std::process::exit(app::download_pack_cli(tag, &args.data.clone().unwrap_or_else(app::default_data_dir)));
    }
    if let Some(src) = &args.cc0 {
        std::process::exit(app::import_cc0_cli(src, &args.data.clone().unwrap_or_else(app::default_data_dir), args.cc0_unit));
    }
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
    let args = app::Args { file: None, selftest: None, shot: None, size: None, data: None, export: None, px: None, plan: None, osm: None, place: None, cells: None, trees: None, three_d: false, view3d: false, cc0: None, cc0_unit: None, cc0_get: None };
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
        for flag in ["--data-dir", "--mcp", "--aside", "--selftest", "--shot", "--size", "--export", "--px", "--plan", "--osm", "--place", "--cells", "--trees", "--update", "--packinfo", "--wow-glb", "--analyse", "--version", "--help", "--view3d", "--cc0", "--cc0-unit", "--cc0-get"] {
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
