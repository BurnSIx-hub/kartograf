//! Картограф — редактор карт для настольных игр.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod generate;
mod library;
mod map;
mod mask;
mod pck;
mod recovery;
mod selftest;
mod view;

use eframe::egui;
use std::path::PathBuf;

fn parse_args() -> app::Args {
    let mut args = app::Args { file: None, selftest: None, shot: None, size: None, data: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--selftest" => args.selftest = it.next().map(PathBuf::from),
            "--shot" => args.shot = it.next().map(PathBuf::from),
            "--data-dir" => args.data = it.next().map(PathBuf::from),
            "--packinfo" => std::process::exit(packinfo(&it.by_ref().map(PathBuf::from).collect::<Vec<_>>())),
            "--size" => args.size = it.next().and_then(|s| s.split_once(['x', 'х']).and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))),
            _ => args.file = Some(PathBuf::from(a)),
        }
    }
    args
}

/// `--packinfo пак…`: без окна — сколько в паке ассетов каждого вида, за сколько читается
/// оглавление и раскрывается ли первая картинка каждого вида.
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

fn main() -> eframe::Result {
    let args = parse_args();
    // Самопроверке нужен один и тот же размер окна, иначе точки щелчков поплывут.
    let size = args.size.or(args.selftest.as_ref().map(|_| [1280.0, 800.0])).unwrap_or([1400.0, 900.0]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Картограф").with_inner_size(size).with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("kartograf", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, args)))))
}
