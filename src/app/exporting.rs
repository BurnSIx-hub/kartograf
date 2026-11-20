//! Выгрузка: снимки холста плитками, запись файлов, снимки окна.

use super::*;
use crate::shotcache;

impl App {
    /// Начать выгрузку: плитки по размеру холста, камера и выбор запоминаются.
    pub fn start_export(&mut self, dir: PathBuf, name: String, px: u32, quit: bool) {
        let began = crate::plat::Instant::now();
        // Большая карта — мельче: иначе Foundry картинку не покажет.
        let px = export::fit_px(self.map.width, self.map.height, px);
        let (w, h) = (self.map.width * px, self.map.height * px);
        self.notice = None;
        let start_level = self.level;
        let sel = self.sel.take();
        self.switch_level(0);
        // Пни под кронами из библиотеки — только если крон в выгрузке есть.
        let crowns = self.settings.export_canopies && (0..self.levels.len()).any(|k| if k == self.level { &self.map } else { &self.levels[k] }.props.iter().any(|p| p.is_canopy()));
        let stumps = if self.settings.gen_library && crowns { Palette::library_keys(&self.library.assets, "stump") } else { Vec::new() };
        let prepare = began.elapsed().as_secs_f32();
        self.export = Some(Export {
            dir,
            stem: export::ascii_name(&name),
            name,
            px,
            tile: [0, 0],
            tiles: Vec::new(),
            next: 0,
            image: image::RgbImage::new(w, h),
            done: Vec::new(),
            doing: 0,
            start_level,
            cam: self.cam,
            sel,
            settle: 0,
            asked: false,
            quit,
            canopies: self.settings.export_canopies,
            update: false,
            stumps,
            picture: false,
            origin: [0.0, 0.0],
            roof_alpha: 0.0,
            wait: 0,
            wall_pass: 0,
            floor: None,
            floor_white: None,
            wall_shot: None,
            whites: Vec::new(),
            walls: Vec::new(),
            began,
            shots_taken: 0,
            cached: false,
            planned: false,
            missing: Vec::new(),
            prints: None,
            prints_now: Vec::new(),
            cells_total: 0,
            cells_cached: 0,
            planning: 0.0,
            prepare,
            fresh: false,
            district: None,
        });
    }

    /// Выгрузить только район `area` = `[x0, y0, x1, y1]` клеток — отдельной сценой, в полном
    /// разрешении: у огромной карты целиком Foundry покажет лишь 16 точек на клетку.
    /// Снимается район с самой карты; стены, крыши и предметы, что задевают край, идут в
    /// сцену целиком. Сцена и файлы — с клетками района в имени.
    pub fn start_district(&mut self, dir: PathBuf, name: String, px: u32, area: [u32; 4], quit: bool) -> Result<(), String> {
        let (x0, y0) = (area[0].min(self.map.width), area[1].min(self.map.height));
        let (x1, y1) = (area[2].min(self.map.width), area[3].min(self.map.height));
        if x1 <= x0 || y1 <= y0 {
            return Err(format!("район {x0},{y0}–{x1},{y1} пустой: карта {}×{}", self.map.width, self.map.height));
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut maps = Vec::new();
        let mut edges = Vec::new();
        for k in 0..self.levels.len() {
            let src = if k == self.level { &self.map } else { &self.levels[k] };
            maps.push(src.crop(x0, y0, w, h));
            edges.push(export::clip_segments(&src.room_edges(), [x0 as f32, y0 as f32, x1 as f32, y1 as f32], [-(x0 as f32), -(y0 as f32)]));
        }
        self.start_export(dir, format!("{name} {x0},{y0}–{x1},{y1}"), px, quit);
        let px = export::fit_px(w, h, px);
        if let Some(e) = &mut self.export {
            e.px = px;
            e.origin = [x0 as f32, y0 as f32];
            e.image = image::RgbImage::new(w * px, h * px);
            e.district = Some(District { area: [x0, y0, x1, y1], maps, edges });
        }
        Ok(())
    }

    /// Видимая в окне часть карты, целыми клетками: `[x0, y0, x1, y1]`.
    pub(super) fn view_cells(&self) -> [u32; 4] {
        let (a, b) = (self.cam.to_world(self.canvas, self.canvas.min), self.cam.to_world(self.canvas, self.canvas.max));
        let (mw, mh) = (self.map.width as f32, self.map.height as f32);
        let x0 = a[0].floor().clamp(0.0, mw - 1.0) as u32;
        let y0 = a[1].floor().clamp(0.0, mh - 1.0) as u32;
        let x1 = (b[0].ceil().clamp(1.0, mw) as u32).max(x0 + 1);
        let y1 = (b[1].ceil().clamp(1.0, mh) as u32).max(y0 + 1);
        [x0, y0, x1, y1]
    }

    /// Этаж `k` для файлов выгрузки: вся карта или вырезанный район.
    fn export_level<'a>(&'a self, e: &'a Export, k: usize) -> &'a Map {
        match &e.district {
            Some(d) => &d.maps[k],
            None if k == self.level => &self.map,
            None => &self.levels[k],
        }
    }

    /// Снять область карты одной картинкой, как её рисует редактор, но без окон,
    /// сетки и выделения: предметы и двери в картинке, этаж текущий. Плитками, как
    /// выгрузка, — размер не упирается в окно. Готовое — в `self.picture`.
    pub fn start_picture(&mut self, area: [f32; 4], px: u32, roof_alpha: f32) {
        let px = px.clamp(2, 400);
        let (w, h) = ((((area[2] - area[0]) * px as f32).ceil() as u32).max(1), (((area[3] - area[1]) * px as f32).ceil() as u32).max(1));
        self.picture = None;
        let sel = self.sel.take();
        self.export = Some(Export {
            dir: PathBuf::new(),
            name: String::new(),
            stem: String::new(),
            px,
            tile: [0, 0],
            tiles: Vec::new(),
            next: 0,
            image: image::RgbImage::new(w, h),
            done: Vec::new(),
            doing: self.level,
            start_level: self.level,
            cam: self.cam,
            sel,
            settle: 0,
            asked: false,
            quit: false,
            canopies: false,
            update: false,
            stumps: Vec::new(),
            picture: true,
            origin: [area[0], area[1]],
            roof_alpha,
            wait: 0,
            wall_pass: 0,
            floor: None,
            floor_white: None,
            wall_shot: None,
            whites: Vec::new(),
            walls: Vec::new(),
            began: crate::plat::Instant::now(),
            shots_taken: 0,
            cached: false,
            planned: false,
            missing: Vec::new(),
            prints: None,
            prints_now: Vec::new(),
            cells_total: 0,
            cells_cached: 0,
            planning: 0.0,
            prepare: 0.0,
            fresh: false,
            district: None,
        });
    }

    /// Развернуть свёрнутое окно (или свернуть обратно), не отнимая фокус.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn show_window(&self, ctx: &egui::Context, minimized: bool) {
        if !crate::plat::show_quietly(self.window_code, minimized) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(minimized));
        }
    }

    /// Шаг выгрузки до отрисовки кадра: поставить камеру на плитку и, когда
    /// картинки догрузились, попросить снимок.
    pub(super) fn export_step(&mut self, ctx: &egui::Context) {
        if self.export.is_none()
            && let Some((dir, px)) = self.export_args.clone()
            && self.frame >= 10
            && self.pending_plan.is_none()
            && self.pending_osm.is_none()
            && !self.place.busy()
            && !self.library.loading()
            && self.textures.idle()
            && self.canvas.width() > 100.0
        {
            self.export_args = None;
            let name = self.name();
            self.start_export(dir, name, px, true);
        }
        let ppp = ctx.pixels_per_point();
        let canvas = self.canvas;
        let idle = self.textures.idle() && !self.library.loading() && !self.outline_pending;
        let Some(e) = &mut self.export else { return };
        ctx.request_repaint();
        if e.asked {
            return;
        }
        if e.tile == [0, 0] {
            // Окно только что развернули для снимка: дать ему принять размер.
            e.wait += 1;
            if e.wait < 3 {
                return;
            }
            // Плитки — по холсту развёрнутого окна. Кэш — если в окно влезает кусок.
            e.tile = [((canvas.width() * ppp).floor() as u32).saturating_sub(2).max(64), ((canvas.height() * ppp).floor() as u32).saturating_sub(2).max(64)];
            e.cached = !e.picture && cfg!(not(target_arch = "wasm32")) && e.tile[0] >= shotcache::CELL && e.tile[1] >= shotcache::CELL;
        }
        if !e.planned {
            self.plan_pass(ppp);
            if self.export.as_ref().is_some_and(|e| e.tiles.is_empty()) {
                // Весь проход нашёлся в кэше: снимать нечего.
                self.pass_done();
                return;
            }
        }
        let Some(e) = &mut self.export else { return };
        let [tx, ty] = e.tiles[e.next];
        // Начало плитки ложится на целый пиксель экрана: плитки стыкуются без шва.
        let origin = [(canvas.min.x * ppp).ceil(), (canvas.min.y * ppp).ceil()];
        let scale = e.px as f32 / ppp;
        let world = [e.origin[0] + tx as f32 / e.px as f32, e.origin[1] + ty as f32 / e.px as f32];
        self.cam.scale = scale;
        self.cam.pan = vec2(origin[0] / ppp - canvas.min.x - world[0] * scale, origin[1] / ppp - canvas.min.y - world[1] * scale);
        e.settle += 1;
        if e.settle >= 3 && idle {
            e.asked = true;
            e.shots_taken += 1;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Кэш снимков на диске (у программы, не в браузере): открывается при первой выгрузке.
    #[cfg(not(target_arch = "wasm32"))]
    fn shot_store(&mut self) -> &shotcache::Store {
        let dir = self.data_dir.join("cache").join("shots");
        self.shot_store.get_or_insert_with(|| shotcache::Store::open(dir))
    }

    /// Что общего у всех кусков прохода, кроме карты: программа, картинки, проход снимка,
    /// точки на клетку, масштаб окна.
    fn shot_salt(&self, e: &Export, ppp: f32) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let used = self.map.used_assets();
        let keys = used.iter().map(String::as_str).chain(e.stumps.iter().map(String::as_str));
        (shotcache::program_stamp(), shotcache::assets_stamp(keys, &self.textures.sources)).hash(&mut h);
        (e.shot() as u8, e.pass(), e.void(), e.px, ppp.to_bits(), shotcache::CELL, e.canopies, &e.stumps).hash(&mut h);
        // Стёкла подоконников и пни берутся из библиотеки: другая библиотека — другие.
        self.library.assets.len().hash(&mut h);
        for a in &self.library.assets {
            a.key.hash(&mut h);
        }
        h.finish()
    }

    /// Разложить проход на плитки. С кэшем: отпечаток каждого куска картинки, найденные на
    /// диске куски — сразу в картинку, плитки — только там, где куска нет.
    fn plan_pass(&mut self, ppp: f32) {
        let Some(mut e) = self.export.take() else { return };
        e.planned = true;
        let (w, h) = (e.image.width(), e.image.height());
        let tile = e.tile;
        // Плитки окна — их левые верхние точки в картинке.
        let grid: Vec<[u32; 2]> = (0..h.div_ceil(tile[1])).flat_map(|ty| (0..w.div_ceil(tile[0])).map(move |tx| [tx * tile[0], ty * tile[1]])).collect();
        #[cfg(not(target_arch = "wasm32"))]
        if e.cached {
            use shotcache::CELL;
            let started = crate::plat::Instant::now();
            let (nx, ny) = (w.div_ceil(CELL), h.div_ceil(CELL));
            if e.prints.as_ref().map(|p| p.0) != Some(e.doing) {
                e.prints = Some((e.doing, shotcache::Prints::new(&self.map, &mut self.chunk_memo)));
            }
            let salt = self.shot_salt(&e, ppp);
            let px = e.px as f32;
            let o = e.origin;
            let prints = &e.prints.as_ref().expect("отпечатки этажа").1;
            e.prints_now = (0..ny * nx)
                .map(|c| {
                    let (i, j) = (c % nx, c / nx);
                    let (x0, y0, x1, y1) = (i * CELL, j * CELL, ((i + 1) * CELL).min(w), ((j + 1) * CELL).min(h));
                    prints.cell([o[0] + x0 as f32 / px, o[1] + y0 as f32 / px, o[0] + x1 as f32 / px, o[1] + y1 as f32 / px], salt)
                })
                .collect();
            // Куски с диска — сразу в картинку, полосами по потокам: полоса кусков —
            // свои строки картинки, потоки друг другу не мешают.
            let fps = std::mem::take(&mut e.prints_now);
            let store = self.shot_store();
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
            let row = (w * 3) as usize;
            let raw: &mut [u8] = &mut e.image;
            let mut per: Vec<Vec<(u32, &mut [u8])>> = (0..threads).map(|_| Vec::new()).collect();
            for (j, band) in raw.chunks_mut(row * CELL as usize).enumerate() {
                per[j % threads].push((j as u32, band));
            }
            let fps_ref = &fps;
            let fresh = e.fresh;
            let missing: Vec<u32> = std::thread::scope(|s| {
                let jobs: Vec<_> = per
                    .into_iter()
                    .map(|list| {
                        s.spawn(move || {
                            let mut miss = Vec::new();
                            for (j, band) in list {
                                let ch = (CELL).min(h - j * CELL);
                                for i in 0..nx {
                                    let c = j * nx + i;
                                    let cw = CELL.min(w - i * CELL);
                                    match (!fresh).then(|| store.load(fps_ref[c as usize], cw, ch)).flatten() {
                                        Some(rgb) => {
                                            let line = (cw * 3) as usize;
                                            for y in 0..ch as usize {
                                                let at = y * row + (i * CELL * 3) as usize;
                                                band[at..at + line].copy_from_slice(&rgb[y * line..(y + 1) * line]);
                                            }
                                        }
                                        None => miss.push(c),
                                    }
                                }
                            }
                            miss
                        })
                    })
                    .collect();
                jobs.into_iter().flat_map(|j| j.join().unwrap_or_default()).collect()
            });
            e.prints_now = fps;
            e.missing = vec![false; (nx * ny) as usize];
            for &c in &missing {
                e.missing[c as usize] = true;
            }
            // Недостающее — плитками от недостающих кусков; если так выходит не меньше
            // плиток, чем сеткой окна (первая выгрузка), — сеткой.
            let spots = shotcache::cover(&e.missing, [nx, ny], [w, h], tile);
            let miss = &e.missing;
            e.tiles = if spots.len() < grid.len() { spots } else { grid.into_iter().filter(|o| shotcache::touches_missing(miss, [nx, ny], [w, h], *o, tile)).collect() };
            e.cells_total += nx * ny;
            e.cells_cached += nx * ny - missing.len() as u32;
            e.planning += started.elapsed().as_secs_f32();
            self.export = Some(e);
            return;
        }
        e.tiles = grid;
        e.missing.clear();
        self.export = Some(e);
    }

    /// Снимок пришёл: вырезать плитку в большую картинку; последняя — записать файлы.
    pub(super) fn export_tile(&mut self, ctx: &egui::Context, shot: &egui::ColorImage) {
        let ppp = ctx.pixels_per_point();
        let canvas = self.canvas;
        let Some(e) = &mut self.export else { return };
        let [x0, y0] = e.tiles[e.next];
        let (ox, oy) = ((canvas.min.x * ppp).ceil() as usize, (canvas.min.y * ppp).ceil() as usize);
        let w = e.tile[0].min(e.image.width() - x0);
        let h = e.tile[1].min(e.image.height() - y0);
        let [sw, sh] = shot.size;
        let iw = e.image.width() as usize;
        let raw: &mut [u8] = &mut e.image;
        for y in 0..h as usize {
            let sy = oy + y;
            if sy >= sh {
                break;
            }
            let at = ((y0 as usize + y) * iw + x0 as usize) * 3;
            for x in 0..(w as usize).min(sw.saturating_sub(ox)) {
                let c = shot.pixels[sy * sw + ox + x];
                raw[at + x * 3..at + x * 3 + 3].copy_from_slice(&[c.r(), c.g(), c.b()]);
            }
        }
        let Some(e) = &mut self.export else { return };
        e.next += 1;
        e.asked = false;
        e.settle = 0;
        if e.next < e.tiles.len() {
            return;
        }
        self.pass_done();
    }

    /// Снятые куски, которых не было в кэше, — на диск. Кусок мог лечь на две плитки,
    /// поэтому — когда снят весь проход.
    #[cfg(not(target_arch = "wasm32"))]
    fn save_missing(&mut self) {
        use shotcache::CELL;
        if !self.export.as_ref().is_some_and(|e| e.cached && e.missing.iter().any(|&m| m)) {
            return;
        }
        // Выгрузка — на время в сторону: куски уходят в очередь записи по одному, а не
        // копятся все разом (у большой карты — тысячи кусков, сотни мегабайт).
        let Some(mut e) = self.export.take() else { return };
        let missing = std::mem::take(&mut e.missing);
        let prints = std::mem::take(&mut e.prints_now);
        let (iw, ih) = (e.image.width(), e.image.height());
        let nx = iw.div_ceil(CELL);
        let store = self.shot_store();
        for (c, _) in missing.iter().enumerate().filter(|m| *m.1) {
            let (cx, cy) = ((c as u32 % nx) * CELL, (c as u32 / nx) * CELL);
            let (cw, ch) = (CELL.min(iw - cx), CELL.min(ih - cy));
            let mut rgb = Vec::with_capacity((cw * ch * 3) as usize);
            for y in cy..cy + ch {
                let at = ((y * iw + cx) * 3) as usize;
                rgb.extend_from_slice(&e.image.as_raw()[at..at + (cw * 3) as usize]);
            }
            store.save(prints[c], cw, ch, rgb);
        }
        self.export = Some(e);
    }

    /// Проход снят: следующий проход, следующий этаж или запись файлов.
    fn pass_done(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        self.save_missing();
        let Some(e) = &mut self.export else { return };
        e.next = 0;
        e.planned = false;
        e.asked = false;
        e.settle = 0;
        // Только картинка: вернуть камеру и выбор, файлов не писать.
        if e.picture {
            let Some(e) = self.export.take() else { return };
            self.cam = e.cam;
            self.sel = e.sel;
            self.picture = Some(e.image);
            return;
        }
        // Этаж снимается в несколько проходов: с тайлами — пол без стен, стены на
        // чёрном и на белом; верхний этаж — ещё раз на белой пустоте.
        let (shots, at) = (e.shots(), e.wall_pass as usize);
        if at + 1 < shots.len() {
            let fresh = image::RgbImage::new(e.image.width(), e.image.height());
            let shot = std::mem::replace(&mut e.image, fresh);
            match shots[at] {
                Shot::Whole | Shot::Floor => e.floor = Some(shot),
                Shot::White => e.floor_white = Some(shot),
                Shot::WallsBlack | Shot::WallsWhite => e.wall_shot = Some(shot),
            }
            e.wall_pass += 1;
            return;
        }
        // Последний проход: в `image` возвращается первый снимок этажа.
        if let Some(floor) = e.floor.take() {
            let last = std::mem::replace(&mut e.image, floor);
            match (shots[at], e.wall_shot.take()) {
                (Shot::WallsWhite, Some(black)) => e.walls.push([black, last]),
                (Shot::White, _) => e.floor_white = Some(last),
                _ => {}
            }
        }
        let white = e.floor_white.take();
        e.whites.push(white);
        e.wall_pass = 0;
        // Этаж снят — следующий, пока не кончатся.
        if e.doing + 1 < self.levels.len() {
            let fresh = image::RgbImage::new(e.image.width(), e.image.height());
            e.done.push(std::mem::replace(&mut e.image, fresh));
            e.doing += 1;
            let k = e.doing;
            self.switch_level(k);
            return;
        }
        let Some(mut e) = self.export.take() else { return };
        let last = std::mem::replace(&mut e.image, image::RgbImage::new(1, 1));
        e.done.push(last);
        self.switch_level(e.start_level);
        self.cam = e.cam;
        self.sel = e.sel.take();
        let shot_s = e.began.elapsed().as_secs_f32();
        let result = self.write_export(&e);
        if let Some(t) = &mut self.export_timing {
            let r = |s: f32| (s * 100.0).round() / 100.0;
            t["shots"] = serde_json::json!(e.shots_taken);
            t["shooting"] = serde_json::json!(r(shot_s - e.planning));
            t["planning"] = serde_json::json!(r(e.planning));
            t["prepare"] = serde_json::json!(r(e.prepare));
            t["cells"] = serde_json::json!(e.cells_total);
            t["cells_cached"] = serde_json::json!(e.cells_cached);
            t["total"] = serde_json::json!(r(e.began.elapsed().as_secs_f32()));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(store) = &self.shot_store {
            store.prune(shotcache::DISK_LIMIT);
        }
        match &result {
            Ok(msg) => self.notice = Some(msg.clone()),
            Err(err) => self.error = Some(format!("Выгрузка не удалась: {err}")),
        }
        if e.quit {
            match result {
                Ok(msg) => println!("{msg}"),
                Err(err) => eprintln!("{err}"),
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(store) = &self.shot_store {
                store.flush(std::time::Duration::from_secs(20));
            }
            self.recovery.close();
            std::process::exit(0);
        }
    }

    /// Картинка, сцена Foundry и Universal VTT рядом друг с другом; у нескольких
    /// этажей — картинка и `.dd2vtt` на каждый, сцена одна, этажи в ней — уровни.
    pub(super) fn write_export(&mut self, e: &Export) -> Result<String, String> {
        // Файлы копятся здесь: программа пишет их на диск, модуль отдаёт Foundry.
        let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let mut laps = Laps::new();
        // Кэш — и крышам, и сжатию фона и стен в потоках (им — своя копия ручки).
        #[cfg(not(target_arch = "wasm32"))]
        let cache: Option<shotcache::Store> = Some(self.shot_store().clone());
        #[cfg(target_arch = "wasm32")]
        let cache: Option<shotcache::Store> = None;
        let n = e.done.len();
        // Имена файлов — латиницей: кириллицу Foundry превращает в длинные `%D0%9A…`.
        let base = if e.stem.is_empty() { export::ascii_name(&e.name) } else { e.stem.clone() };
        let stem = |k: usize| if n == 1 { base.clone() } else { format!("{}_{}", base, k + 1) };
        // Картинки ассетов — по одной на ассет, по его имени, в общей папке, как библиотека FA.
        let mut shelf = Shelf::new(export::shared_dir(&e.dir));
        let mut srcs = Vec::new();
        let mut layouts = Vec::new();
        // Контуры палаток по картинке — один раз на картинку.
        let mut hulls: std::collections::HashMap<String, Vec<[f32; 2]>> = std::collections::HashMap::new();
        // Фон верхнего этажа, где на нём есть пустота, — с прозрачностью: в Foundry
        // сквозь неё виден этаж ниже. Закрашенный целиком — сплошной, как у первого.
        let mut open: Vec<Option<image::RgbaImage>> = Vec::new();
        let mut bg_paths: Vec<PathBuf> = Vec::new();
        // Фон этажа сжимается в своём потоке, пока рисуются крыши и тайлы: у большой карты
        // это десятки секунд. WebP вышел или JPEG — видно, когда поток закончил.
        let mut bg_jobs: Vec<Pending<Result<(Vec<u8>, &'static str), String>>> = Vec::new();
        for (k, img) in e.done.iter().enumerate() {
            let holes = e.whites.get(k).and_then(|w| w.as_ref()).and_then(|white| export::floor_from_pair(img, white));
            let job = match &holes {
                Some(rgba) => {
                    let (rgba, cache) = (rgba.clone(), cache.clone());
                    Pending::spawn(move || encode_cached(cache, "floor-rgba-88", rgba.dimensions().into(), rgba.as_raw(), || Ok(export::layer_bytes(&rgba, 88.0))))
                }
                None => {
                    let (img, cache) = (img.clone(), cache.clone());
                    Pending::spawn(move || encode_cached(cache, "floor-rgb-88", img.dimensions().into(), img.as_raw(), || background_bytes(&img)))
                }
            };
            bg_jobs.push(job);
            open.push(holes);
            let map = self.export_level(e, k);
            // Вырез воды под мостом — по нарисованному: рамка картинки, по разу на картинку.
            let sources = &self.textures.sources;
            let boxes = std::cell::RefCell::new(std::collections::HashMap::new());
            let seen = |key: &str| *boxes.borrow_mut().entry(key.to_owned()).or_insert_with(|| library::visible_box(sources, key));
            let edges = match &e.district {
                Some(d) => d.edges[k].clone(),
                None => map.room_edges(),
            };
            let mut lay = export::layout_seen(map, &edges, &seen);
            // Стены палаток — по нарисованному пологу, а не по полям картинки.
            let mut tents = Vec::new();
            for p in map.props.iter().filter(|p| map::awning_by_name(&p.asset)) {
                if !hulls.contains_key(&p.asset) {
                    let hull = self.textures.bytes(&p.asset).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| export::alpha_hull(&i.to_rgba8())).unwrap_or_default();
                    hulls.insert(p.asset.clone(), hull);
                }
                tents.extend(export::tent_walls(p, Some(&hulls[&p.asset])));
            }
            lay.tents = tents;
            if e.district.is_some() {
                lay.clip_to(map.width as f32, map.height as f32);
            }
            layouts.push(lay);
            laps.lap("layout");
        }
        // Створки — отдельными картинками рядом со сценой: Foundry их двигает.
        let mut leaves = std::collections::HashMap::new();
        for key in layouts.iter().flat_map(|l| l.portals.iter()).filter_map(|p| p.leaf.clone()) {
            if leaves.contains_key(&key) {
                continue;
            }
            // Foundry вращает створку вокруг левого конца картинки, а петля у FA
            // и у встроенных нарисована справа: створка уходит отражённой.
            let img = image::DynamicImage::ImageRgba8(self.textures.rgba(&key)?).fliph();
            let width = img.width();
            let src = shelf.put(&format!("{}_leaf", export::asset_stem(&key)), export::webp_bytes(&img.to_rgba8(), 90.0), &mut files);
            leaves.insert(key, export::LeafFile { src, width });
        }
        laps.lap("leaves");
        // Крыши — картинками по одной, рядом со сценой.
        let mut roof_tiles: Vec<Vec<export::RoofTile>> = Vec::new();
        let mut roof_count = 0;
        let mut canopies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        // Слой стен каждого этажа (он же для Universal VTT) и его файл.
        let mut wall_layers: std::collections::HashMap<usize, image::RgbaImage> = std::collections::HashMap::new();
        let mut foregrounds: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        let mut wall_paths: std::collections::HashMap<usize, PathBuf> = std::collections::HashMap::new();
        let mut wall_jobs: Vec<(usize, Pending<Result<(image::RgbaImage, Vec<u8>, &'static str), String>>)> = Vec::new();
        let store = cache.as_ref();
        for k in 0..n {
            let map = self.export_level(e, k);
            let mut tiles = Vec::new();
            let mut pics = self.roof_pictures(map, e.px, store)?;
            let rects = map.roofs.len();
            let p = e.px as f32;
            for (ri, r) in map.roofs.iter().enumerate() {
                let Some(pic) = pics.remove(&ri) else { continue };
                let fixtures = r.fixtures();
                let smoke = smoke_spots(&on_roof(map, |q| r.contains(q)), p);
                roof_count += 1;
                let file = e.dir.join(format!("{}_roof_{roof_count}.webp", stem(k)));
                files.push((file.clone(), pic.bytes));
                tiles.push(export::RoofTile { src: export::foundry_src(&file), rect: [(pic.origin[0] * p).round() as i64, (pic.origin[1] * p).round() as i64, (pic.size[0] as f32 * p / pic.ppc).round() as i64, (pic.size[1] as f32 * p / pic.ppc).round() as i64], rotation: 0.0, shelter: true, fade: true, leaves: false, low: false, wind: 1.0, rope: false, web: None, chest: None, living: None, scuppers: fixtures.iter().filter(|f| f.0.contains("Scupper")).map(|f| [f.1[0] * p, f.1[1] * p, f.2]).collect(), chimneys: smoke, surface: vec![(r.outline().iter().map(|v| [v[0] * p, v[1] * p]).collect(), false)], ..Default::default() });
            }
            // Крыши по контуру (из OSM) — тоже по одной. Крышное над ними (свод ангара,
            // купол, шатёр) — в их картинке: отдельным тайлом оно висело бы над комнатой.
            for (idx, r) in map.outline_roofs.iter().enumerate() {
                let Some(pic) = pics.remove(&(rects + idx)) else { continue };
                let smoke = smoke_spots(&on_roof(map, |q| r.contains(q)), p);
                roof_count += 1;
                let file = e.dir.join(format!("{}_roof_{roof_count}.webp", stem(k)));
                files.push((file.clone(), pic.bytes));
                tiles.push(export::RoofTile {
                    src: export::foundry_src(&file),
                    rect: [(pic.origin[0] * p).round() as i64, (pic.origin[1] * p).round() as i64, (pic.size[0] as f32 * p / pic.ppc).round() as i64, (pic.size[1] as f32 * p / pic.ppc).round() as i64],
                    rotation: 0.0,
                    shelter: true,
                    fade: true,
                    leaves: false,
                    low: false,
                    wind: 1.0,
                    rope: false,
                    web: None,
                    chest: None,
                    living: None,
                    scuppers: Vec::new(),
                    chimneys: smoke,
                    outline: roofs::outline(r).into_iter().map(|(ring, hole)| (ring.iter().map(|v| [v[0] * p, v[1] * p]).collect(), hole)).collect(),
                    surface: roofs::eaves(r).into_iter().map(|(ring, hole)| (ring.iter().map(|v| [v[0] * p, v[1] * p]).collect(), hole)).collect(),
                    ..Default::default()
                });
            }
            laps.lap("roofs");
            // Кроны — один файл на вид дерева, выше токенов, как крыши.
            for prop in map.props.iter().filter(|p| e.canopies && (p.is_canopy() || map::bush_by_name(&p.asset) || map::awning_by_name(&p.asset) || map::line_by_name(&p.asset))) {
                // Подкрашенная крона — свой файл: та же картинка в другом цвете.
                let look = prop.tint.map_or_else(|| prop.asset.clone(), |t| format!("{}#{t:?}", prop.asset));
                if !canopies.contains_key(&look) {
                    let mut img = image::DynamicImage::ImageRgba8(self.textures.rgba(&prop.asset)?);
                    if let Some(t) = prop.tint {
                        let mut rgba = img.to_rgba8();
                        for p in rgba.pixels_mut() {
                            for c in 0..3 {
                                p[c] = (p[c] as u16 * t[c] as u16 / 255) as u8;
                            }
                        }
                        img = image::DynamicImage::ImageRgba8(rgba);
                    }
                    // В родном разрешении, как в паке; огромное — не больше 4096 точек.
                    let img = if img.width().max(img.height()) > 4096 { img.resize(4096, 4096, image::imageops::FilterType::Lanczos3) } else { img };
                    // Имя — по ассету; подкрашенная крона — с тоном.
                    let name = match prop.tint {
                        Some([r, g, b]) => format!("{}_t{r:02x}{g:02x}{b:02x}", export::asset_stem(&prop.asset)),
                        None => export::asset_stem(&prop.asset),
                    };
                    let src = shelf.put(&name, export::webp_bytes(&img.to_rgba8(), 88.0), &mut files);
                    canopies.insert(look.clone(), src);
                }
                let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                let mut tile = export::canopy_tile(canopies[&look].clone(), prop.pos, size, prop.rot, e.px);
                tile.flip = prop.flip;
                tile.fade = !map::is_bare_tree(&prop.asset);
                // Навес не качается, но держит дождь; куст и крона — под ветром.
                tile.shelter = !prop.is_canopy() && map::awning_by_name(&prop.asset);
                tile.low = tile.shelter;
                // Бельё колышется, но не гаснет: оно выше голов, и листья с него не летят.
                if map::line_by_name(&prop.asset) {
                    tile.fade = false;
                    tile.leaves = false;
                    tile.rope = true;
                }
                // Куст ниже кроны и жёстче: лишь колышется. Он по колено: лежит на
                // земле под токенами и не гаснет, как крона над головой.
                if map::bush_by_name(&prop.asset) {
                    tile.wind = 0.35;
                    tile.ground = true;
                    tile.fade = false;
                }
                tiles.push(tile);
            }
            // Паутина и сундуки — тайлами на полу: паутину покачивает, сундук
            // открывается. Картинка — одна на ассет.
            // Остальные предметы — тоже тайлами, в родном разрешении картинки,
            // по слою: кто выше, тот позже.
            // Крышное над крышей уже в её картинке — и над прямоугольной, и по контуру.
            let baked = |p: &Prop| (map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer()) && (map.roofs.iter().any(|r| r.contains(p.pos)) || map.outline_roofs.iter().any(|r| r.contains(p.pos)));
            let mut items: Vec<&Prop> = map.props.iter().filter(|p| e.canopies && p.own_tile() && !baked(p) && !p.with_walls()).collect();
            items.sort_by_key(|p| p.layer());
            for prop in items {
                let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                // Картинка — одна на ассет и без тени (тень рисует «Wild Canvas» по пометке
                // плитки); предмет в воде — только сухой верх, своя картинка на ватерлинию.
                let sunk = prop.sunk();
                let mut save = |key: &str, files: &mut Vec<(PathBuf, Vec<u8>)>| -> Result<String, String> {
                    let slot = format!("{key}{}", if sunk > 0.0 { format!("#dry{sunk:.3}") } else { String::new() });
                    if let Some(src) = canopies.get(&slot) {
                        return Ok(src.clone());
                    }
                    let name = if sunk > 0.0 { format!("{}_dry{:02}", export::asset_stem(key), (sunk * 100.0).round() as u32) } else { export::asset_stem(key) };
                    let src = shelf.put(&name, self.asset_webp(key, sunk)?, files);
                    canopies.insert(slot, src.clone());
                    Ok(src)
                };
                let src = save(&prop.asset, &mut files)?;
                let mut tile = export::canopy_tile(src.clone(), prop.pos, size, prop.rot, e.px);
                tile.flip = prop.flip;
                tile.fade = false;
                tile.leaves = false;
                tile.item = true;
                tile.shadow = map.shadows.on && prop.carries_shadow();
                tile.above = prop.layer() >= map::ON_ROOF;
                tile.hover = map::hover_by_name(&prop.asset);
                tile.afloat = map::afloat_by_name(&prop.asset) && map.on_water(prop.pos);
                tile.swing = if map::swing_by_name(&prop.asset) { library::strand_ends(&self.textures.sources, &prop.asset) } else { None };
                tile.slew = map::slew_by_name(&prop.asset).and_then(|o| export::slew_arc(map, prop, o).map(|arc| [o[0], o[1], arc]));
                tile.rail = export::rail_ride(map, prop, e.px);
                tile.tint = prop.tint;
                if !(map::web_by_name(&prop.asset) || map::chest_variants(&prop.asset).is_some() || map::speaker_by_name(&prop.asset)) {
                    tiles.push(tile);
                    continue;
                }
                if let Some(living) = map::living_by_name(&prop.asset) {
                    tile.living = Some(living);
                    tile.layout = map::live_layout(&prop.asset).map(|(_, layout)| layout);
                    tiles.push(tile);
                    continue;
                }
                match map::chest_variants(&prop.asset) {
                    // Нет открытого или пустого в библиотеке — тот же закрытый.
                    Some((open, empty)) => {
                        let open = save(&open, &mut files).unwrap_or_else(|_| src.clone());
                        let empty = save(&empty, &mut files).unwrap_or_else(|_| open.clone());
                        tile.chest = Some((open, empty));
                    }
                    None => tile.web = Some(library::strand_ends(&self.textures.sources, &prop.asset)),
                }
                tiles.push(tile);
            }
            laps.lap("tiles");
            // Стены этажа — слоем во всю сцену, в `foreground` уровня: ядро держит его на
            // верхе этажа, над мебелью и токенами и под крышами. В нём же тень стен,
            // окна, косяки и слой «над стенами».
            if let Some([black, white]) = e.walls.get(k) {
                let layer = export::layer_from_pair(black, white);
                if export::opaque_box(&layer, 8).is_some() {
                    let cache = cache.clone();
                    wall_jobs.push((k, Pending::spawn(move || {
                        let (bytes, ext) = encode_cached(cache, "walls-rgba-90", layer.dimensions().into(), layer.as_raw(), || Ok(export::layer_bytes(&layer, 90.0)))?;
                        Ok((layer, bytes, ext))
                    })));
                }
            }
            roof_tiles.push(tiles);
            laps.lap("walls");
        }
        // Сжатое в потоках — в файлы.
        for (k, job) in wall_jobs {
            let (layer, bytes, ext) = job.join()??;
            let file = e.dir.join(format!("{}_walls.{ext}", stem(k)));
            files.push((file.clone(), bytes));
            foregrounds.insert(k, export::foundry_src(&file));
            wall_paths.insert(k, file);
            wall_layers.insert(k, layer);
        }
        laps.lap("walls_wait");
        for (k, job) in bg_jobs.into_iter().enumerate() {
            let (bytes, ext) = job.join()??;
            let img_path = e.dir.join(format!("{}.{ext}", stem(k)));
            files.push((img_path.clone(), bytes));
            bg_paths.push(img_path.clone());
            srcs.push(export::foundry_src(&img_path));
        }
        laps.lap("background_wait");
        // Верхний этаж с пустотой — куском, где на нём нарисовано, а не во всю сцену: стена по
        // гребню над замком в 140×110 клеток занимает малую долю сцены, а слой во всю сцену —
        // сотни мегабайт видеопамяти у каждого игрока. Фон и стены режутся по одной рамке
        // (у уровня они кладутся по общим `textures`); кусок почти во всю сцену не режется.
        let mut pieces: std::collections::HashMap<usize, ([u32; 2], [u32; 4])> = std::collections::HashMap::new();
        for k in 1..n {
            let Some(bg) = open[k].as_ref() else { continue };
            let size = [bg.width(), bg.height()];
            let walls = wall_layers.get(&k);
            let Some(piece) = export::level_piece_box(&[export::opaque_box(bg, 8), walls.and_then(|l| export::opaque_box(l, 8))], size, 4) else { continue };
            if piece[2] as u64 * piece[3] as u64 * 4 > size[0] as u64 * size[1] as u64 * 3 {
                continue;
            }
            let cut = |img: &image::RgbaImage| image::imageops::crop_imm(img, piece[0], piece[1], piece[2], piece[3]).to_image();
            let (bytes, ext) = export::layer_bytes(&cut(bg), 88.0);
            let path = e.dir.join(format!("{}.{ext}", stem(k)));
            files.retain(|(p, _)| p != &bg_paths[k]);
            files.push((path.clone(), bytes));
            srcs[k] = export::foundry_src(&path);
            if let (Some(layer), Some(old)) = (walls, wall_paths.get(&k)) {
                let (bytes, ext) = export::layer_bytes(&cut(layer), 90.0);
                let path = e.dir.join(format!("{}_walls.{ext}", stem(k)));
                files.retain(|(p, _)| p != old);
                files.push((path.clone(), bytes));
                foregrounds.insert(k, export::foundry_src(&path));
            }
            pieces.insert(k, (size, piece));
        }
        let levels: Vec<export::SceneLevel> = (0..n)
            .map(|k| export::SceneLevel { name: if n == 1 { e.name.clone() } else { self.level_name(k) }, image_src: srcs[k].clone(), layout: &layouts[k], roofs: std::mem::take(&mut roof_tiles[k]) })
            .collect();
        let layers: Vec<export::LevelLayers> = (0..n).map(|k| export::LevelLayers { foreground: foregrounds.remove(&k), solid: open[k].as_ref().map(export::Solid::of) }).collect();
        let mut scene = export::foundry_layered(self.export_level(e, self.level), &e.name, &levels, &layers, e.px, &leaves);
        let scene_px = [scene["width"].as_u64().unwrap_or(0) as u32, scene["height"].as_u64().unwrap_or(0) as u32];
        for (k, (size, piece)) in &pieces {
            scene["levels"][*k]["textures"] = export::level_piece_textures(scene_px, *size, *piece);
        }
        // Тени предметов рисует «Wild Canvas»: солнце — у сцены.
        let shadows = &self.export_level(e, self.level).shadows;
        if shadows.on {
            scene["flags"][export::LIVE_MODULE]["shadows"] = export::shadow_flags(shadows);
        }
        // Пути картинок — с меткой содержимого: Foundry не покажет прежнюю под тем же именем.
        let mut tags: std::collections::HashMap<String, String> = files.iter().map(|(p, b)| (export::foundry_src(p), export::content_tag(b))).collect();
        tags.extend(shelf.tags.clone());
        export::tag_sources(&mut scene, &tags);
        // Район: модуль Foundry видит, какой кусок карты в сцене.
        if let Some(d) = &e.district {
            scene["flags"][export::OWN_MODULE]["district"] = serde_json::json!(d.area);
        }
        // Сцена открывается на том этаже, что открыт в редакторе: у корабля — палуба, а не трюм.
        if n > 1 {
            scene["initialLevel"] = serde_json::json!(export::level_id(self.level));
        }
        let scene_path = e.dir.join(format!("{base}.json"));
        files.push((scene_path, serde_json::to_string_pretty(&scene).unwrap_or_default().into_bytes()));
        laps.lap("scene");
        // Universal VTT — для других VTT; модулю он не нужен, сцена создаётся сразу.
        #[cfg(not(target_arch = "wasm32"))]
        {
            use base64::Engine;
            // В Universal VTT тайлов нет: предметы, ушедшие в тайлы, впечатываются
            // в его картинку.
            let mut flats: Vec<image::RgbImage> = Vec::new();
            let sources = &self.textures.sources;
            for k in 0..n {
                let map = self.export_level(e, k);
                // Прозрачности в нём нет: в пустоте и дырах верхнего этажа — картинка этажа
                // ниже, как его видно в Foundry (у самого нижнего — серая пустота).
                let mut flat = match (&open[k], k.checked_sub(1).and_then(|b| flats.get(b))) {
                    (Some(rgba), Some(under)) => export::over_image(rgba, under),
                    (Some(rgba), None) => export::over_color(rgba, [38, 38, 38]),
                    (None, _) => e.done[k].clone(),
                };
                // Крыш в Universal VTT нет — нет и крышного.
                let mut items: Vec<&Prop> = map.props.iter().filter(|p| e.canopies && p.own_tile() && !p.with_walls() && !(map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer())).collect();
                items.sort_by_key(|p| p.layer());
                let p = e.px as f32;
                // Тень предмета в фоне больше не лежит — впечатывается вместе с ним.
                let shade = |prop: &Prop| map.shadows.on && prop.carries_shadow();
                let size_of = |prop: &Prop| [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                let slot_of = |prop: &Prop| if shade(prop) { format!("{}#тень{k}|{:.0}|{:.3}", prop.asset, prop.rot.rem_euclid(360.0), prop.scale) } else { prop.asset.clone() };
                // Партиями: картинки партии (тень — размытием) считают потоки, потом потоки же
                // впечатывают их полосами картинки, по порядку слоёв. У города тысячи предметов
                // со своим поворотом: помнить картинку каждого — гигабайты.
                let mut i = 0;
                while i < items.len() {
                    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                    let mut reps: Vec<&Prop> = Vec::new();
                    let mut ids = Vec::new();
                    let mut j = i;
                    while j < items.len() {
                        let s = slot_of(items[j]);
                        let id = match index.get(&s) {
                            Some(&id) => id,
                            None if reps.len() >= 96 => break,
                            None => {
                                index.insert(s, reps.len());
                                reps.push(items[j]);
                                reps.len() - 1
                            }
                        };
                        ids.push(id);
                        j += 1;
                    }
                    let pics = export::par_map(&reps, |prop: &&Prop| {
                        let raw = library::load_rgba(sources, &prop.asset)?;
                        Some(if shade(prop) { export::with_shadow(&raw, size_of(prop), prop.rot, &map.shadows) } else { (raw, [1.0, 1.0]) })
                    });
                    let batch = &items[i..j];
                    let w = flat.width();
                    let raw: &mut [u8] = &mut flat;
                    export::par_bands(raw, w as usize * 3, |y0, band| {
                        let rows = y0 as u32..y0 as u32 + (band.len() / (w as usize * 3)) as u32;
                        for (prop, &id) in batch.iter().zip(&ids) {
                            if let Some((pic, grow)) = &pics[id] {
                                let size = size_of(prop);
                                export::stamp_rows(band, w, rows.clone(), pic, [prop.pos[0] * p, prop.pos[1] * p], [size[0] * grow[0] * p, size[1] * grow[1] * p], prop.rot, prop.tint);
                            }
                        }
                    });
                    i = j;
                }
                // Стены — поверх предметов, как в редакторе.
                if let Some(layer) = wall_layers.get(&k) {
                    export::stamp_layer(&mut flat, layer, [0, 0]);
                }
                let mut jpg = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(&flat).map_err(|x| x.to_string())?;
                flats.push(flat);
                let uv = export::dd2vtt(map, e.px, &base64::engine::general_purpose::STANDARD.encode(&jpg), &layouts[k]);
                files.push((e.dir.join(format!("{}.dd2vtt", stem(k))), uv.to_string().into_bytes()));
            }
            laps.lap("uvtt");
            // Для сверки кэша снимков: снятое без сжатия (WebP с потерями шумит по всей
            // картинке от малейшей разницы во входе). Только по переменной окружения.
            if std::env::var_os("KARTOGRAF_EXPORT_RAW").is_some() {
                for (k, img) in e.done.iter().enumerate() {
                    files.push((e.dir.join(format!("raw_{k}.png")), png_bytes(img)));
                }
                for (k, [black, white]) in e.walls.iter().enumerate() {
                    files.push((e.dir.join(format!("raw_{k}_walls_black.png")), png_bytes(black)));
                    files.push((e.dir.join(format!("raw_{k}_walls_white.png")), png_bytes(white)));
                }
            }
            std::fs::create_dir_all(&e.dir).map_err(|x| x.to_string())?;
            for (path, bytes) in &files {
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).map_err(|x| format!("{}: {x}", d.display()))?;
                }
                std::fs::write(path, bytes).map_err(|x| format!("{}: {x}", path.display()))?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::plat::to_scene(&files, e.update)?;
        }
        laps.lap("write");
        self.export_timing = Some(laps.json());
        let walls: usize = layouts.iter().flat_map(|l| l.walls.iter()).map(|l| l.len().saturating_sub(1)).sum();
        let (w, h) = (e.done[0].width(), e.done[0].height());
        Ok(format!(
            "{} {}: {}{}картинка {w}×{h}, стен {walls}, крыш {roof_count}, проёмов {}, огней {}",
            if cfg!(target_arch = "wasm32") { "На сцену —" } else { "Выгружено в" },
            e.dir.display(),
            e.district.as_ref().map_or_else(String::new, |d| format!("район {},{}–{},{}, ", d.area[0], d.area[1], d.area[2], d.area[3])),
            if n > 1 { format!("этажей {n}, ") } else { String::new() },
            layouts.iter().map(|l| l.portals.len()).sum::<usize>(),
            layouts.iter().map(|l| l.lights.len()).sum::<usize>()
        ))
    }

    /// Картинка ассета для общей папки: WebP пака как есть, если его не перекрашивали и
    /// он не в воде; иначе — заново (перекраска, сухой верх), не больше 4096 точек.
    fn asset_webp(&self, key: &str, sunk: f32) -> Result<Vec<u8>, String> {
        if sunk <= 0.0 && !key.starts_with(library::TINT) {
            let raw = self.textures.bytes(key)?;
            if raw.len() > 12 && &raw[0..4] == b"RIFF" && &raw[8..12] == b"WEBP" {
                return Ok(raw);
            }
        }
        let mut img = self.textures.rgba(key)?;
        if img.width().max(img.height()) > 4096 {
            img = image::DynamicImage::ImageRgba8(img).resize(4096, 4096, image::imageops::FilterType::Lanczos3).to_rgba8();
        }
        if sunk > 0.0 {
            img = export::dry_part(&img, sunk);
        }
        Ok(export::webp_bytes(&img, 90.0))
    }

    /// `--shot файл.png`: снять окно, когда оно устоялось, и выйти.
    pub(super) fn screenshots(&mut self, ctx: &egui::Context) {
        let images: Vec<_> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
                .collect()
        });
        if self.export.as_ref().is_some_and(|e| e.asked) {
            if let Some(image) = images.first() {
                self.export_tile(ctx, image);
            }
            ctx.request_repaint();
            return;
        }
        if let Some(test) = &mut self.test {
            for image in &images {
                test.screenshot(image);
            }
            return;
        }
        let Some(path) = &self.shot else { return };
        if let Some(image) = images.first() {
            if let Err(e) = save_png(path, image) {
                eprintln!("снимок не сохранён: {e}");
            }
            self.recovery.close();
            std::process::exit(0);
        } else if !self.shot_asked && self.frame >= 10 && !self.library.loading() && self.textures.idle() {
            self.shot_asked = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }
}

/// Работа в своём потоке (у программы) или сразу (в браузере потоков нет).
enum Pending<T> {
    Ready(T),
    Thread(std::thread::JoinHandle<T>),
}

impl<T: Send + 'static> Pending<T> {
    fn spawn(f: impl FnOnce() -> T + Send + 'static) -> Self {
        if cfg!(target_arch = "wasm32") { Self::Ready(f()) } else { Self::Thread(std::thread::spawn(f)) }
    }

    fn join(self) -> Result<T, String> {
        match self {
            Self::Ready(v) => Ok(v),
            Self::Thread(h) => h.join().map_err(|_| "поток выгрузки упал".to_owned()),
        }
    }
}

/// PNG без потерь (сверка кэша снимков).
#[cfg(not(target_arch = "wasm32"))]
fn png_bytes(img: &image::RgbImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

/// Сжатая картинка по отпечатку её точек: та же картинка (правка не задела этот слой —
/// мебель и деревья в Foundry тайлы, фон и стены от них не меняются) — те же байты с
/// диска. Сжатие слоя стен большой карты — десятки секунд.
fn encode_cached(cache: Option<shotcache::Store>, kind: &str, size: [u32; 2], raw: &[u8], encode: impl FnOnce() -> Result<(Vec<u8>, &'static str), String>) -> Result<(Vec<u8>, &'static str), String> {
    let key = cache.as_ref().map(|_| shotcache::content_key(kind, size, raw));
    if let (Some(c), Some(k)) = (&cache, key)
        && let Some(blob) = c.load_blob(k)
        && let Some((&code, bytes)) = blob.split_first()
    {
        return Ok((bytes.to_vec(), match code {
            0 => "webp",
            1 => "jpg",
            _ => "png",
        }));
    }
    let (bytes, ext) = encode()?;
    if let (Some(c), Some(k)) = (&cache, key) {
        let mut blob = Vec::with_capacity(bytes.len() + 1);
        blob.push(match ext {
            "webp" => 0,
            "jpg" => 1,
            _ => 2,
        });
        blob.extend_from_slice(&bytes);
        c.save_blob(k, &blob);
    }
    Ok((bytes, ext))
}

/// Фон этажа без пустоты: WebP, а не вышел (велик или libwebp не справился) — JPEG.
fn background_bytes(img: &image::RgbImage) -> Result<(Vec<u8>, &'static str), String> {
    use image::buffer::ConvertBuffer;
    if img.width().max(img.height()) <= export::WEBP_MAX_SIDE
        && let Some(webp) = export::try_webp(&img.convert(), 88.0)
    {
        return Ok((webp, "webp"));
    }
    let mut jpg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(img).map_err(|x| x.to_string())?;
    Ok((jpg, "jpg"))
}

/// Время частей выгрузки: каждая отметка прибавляет к своей части время с прошлой.
struct Laps {
    at: crate::plat::Instant,
    parts: Vec<(&'static str, f32)>,
}

impl Laps {
    fn new() -> Self {
        Self { at: crate::plat::Instant::now(), parts: Vec::new() }
    }

    fn lap(&mut self, part: &'static str) {
        let s = self.at.elapsed().as_secs_f32();
        self.at = crate::plat::Instant::now();
        match self.parts.iter_mut().find(|p| p.0 == part) {
            Some(p) => p.1 += s,
            None => self.parts.push((part, s)),
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::Value::Object(self.parts.iter().map(|(k, s)| (k.to_string(), serde_json::json!((s * 100.0).round() / 100.0))).collect())
    }
}

/// Дымоходы среди крышного (ключ, середина в клетках, поворот, размер): середина
/// и большая сторона в точках сцены — «Wild Canvas» пускает над ними дым.
fn smoke_spots(mine: &[(String, [f32; 2], f32, [f32; 2])], px: f32) -> Vec<[f32; 3]> {
    mine.iter().filter(|m| map::smokes_by_name(&m.0)).map(|m| [m.1[0] * px, m.1[1] * px, m.3[0].max(m.3[1]) * px]).collect()
}

/// Общая папка ассетов выгрузки: картинка кладётся по имени ассета; имя уже занято другой
/// картинкой (другой пак, обновлённый ассет) — к нему метка содержимого. Та же картинка,
/// что уже лежит на диске, не пишется заново.
struct Shelf {
    dir: PathBuf,
    /// Путь в сцене → метка содержимого (для `?v=`).
    tags: std::collections::HashMap<String, String>,
    /// Имя#метка → путь в сцене: в одной выгрузке картинка кладётся раз.
    placed: std::collections::HashMap<String, String>,
    /// Имя → метка картинки, что легла под ним в этой выгрузке.
    used: std::collections::HashMap<String, String>,
}

impl Shelf {
    fn new(dir: PathBuf) -> Self {
        Self { dir, tags: Default::default(), placed: Default::default(), used: Default::default() }
    }

    fn put(&mut self, name: &str, bytes: Vec<u8>, files: &mut Vec<(PathBuf, Vec<u8>)>) -> String {
        let tag = export::content_tag(&bytes);
        let key = format!("{name}#{tag}");
        if let Some(src) = self.placed.get(&key) {
            return src.clone();
        }
        // Что лежит на диске под этим путём: та же картинка, другая или ничего.
        let disk = |p: &std::path::Path| -> Option<bool> {
            if cfg!(target_arch = "wasm32") {
                return None;
            }
            std::fs::read(p).ok().map(|old| old == bytes)
        };
        let plain = self.dir.join(format!("{name}.webp"));
        let clash = self.used.get(name).is_some_and(|t| *t != tag) || disk(&plain) == Some(false);
        let path = if clash { self.dir.join(format!("{name}_{tag}.webp")) } else { plain };
        if !clash {
            self.used.insert(name.to_owned(), tag.clone());
        }
        let src = export::asset_src(&path);
        if disk(&path) != Some(true) {
            files.push((path, bytes));
        }
        self.tags.insert(src.clone(), tag);
        self.placed.insert(key, src.clone());
        src
    }
}

/// Картинка крыши для Foundry: WebP, левый верхний угол в клетках, точек на клетку, размер.
/// Размер 0 — у крыши картинки нет (контур целиком за краем карты).
struct RoofPic {
    bytes: Vec<u8>,
    origin: [f32; 2],
    ppc: f32,
    size: [u32; 2],
}

impl RoofPic {
    fn to_blob(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes.len() + 20);
        for v in [self.origin[0], self.origin[1], self.ppc] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.size[0].to_le_bytes());
        out.extend_from_slice(&self.size[1].to_le_bytes());
        out.extend_from_slice(&self.bytes);
        out
    }

    fn from_blob(b: &[u8]) -> Option<Self> {
        let f = |i: usize| b.get(i..i + 4).and_then(|s| s.try_into().ok());
        Some(Self {
            origin: [f32::from_le_bytes(f(0)?), f32::from_le_bytes(f(4)?)],
            ppc: f32::from_le_bytes(f(8)?),
            size: [u32::from_le_bytes(f(12)?), u32::from_le_bytes(f(16)?)],
            bytes: b.get(20..)?.to_vec(),
        })
    }
}

/// Что рисует работа над крышами: группу врезанных друг в друга, одну прямоугольную или
/// одну по контуру.
enum RoofJob {
    Group(usize),
    Rect(usize),
    Outline(usize),
}

/// Что крыша несёт на себе (слой «На крыше»): ключ, середина, поворот, размер.
type OnRoof = Vec<(String, [f32; 2], f32, [f32; 2])>;

fn on_roof(map: &Map, contains: impl Fn([f32; 2]) -> bool) -> OnRoof {
    map.props.iter().filter(|p| (map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer()) && contains(p.pos)).map(|p| (p.asset.clone(), p.pos, p.rot, [p.size[0] * p.scale, p.size[1] * p.scale])).collect()
}

impl App {
    /// Картинки крыш этажа по номеру крыши (прямоугольные, затем по контуру со сдвигом на
    /// число прямоугольных). Крыши, что врезаются друг в друга, рисуются одной группой;
    /// картинка у каждой своя — только то, что у неё видно поверх соседей. Рисуют и
    /// сжимают потоки; работа с тем же отпечатком (крыша, её соседи по тени, вклеенное,
    /// покрытия, свет) берётся из кэша на диске.
    fn roof_pictures(&self, map: &Map, px: u32, store: Option<&shotcache::Store>) -> Result<std::collections::HashMap<usize, RoofPic>, String> {
        use std::collections::{HashMap, HashSet};
        use std::hash::Hash;
        let light = map.shadows.light();
        let snow = map.weather == "snow";
        let clip = ([0.0, 0.0], [map.width as f32, map.height as f32]);
        let p = px as f32;
        let mut seen = HashMap::new();
        let climbs = super::draw_ground::climb_rects(&map.props, &self.textures.sources, &mut seen);
        let groups = roofs::raster_groups(&map.roofs, &map.outline_roofs);
        let grouped: HashSet<usize> = groups.iter().flatten().copied().collect();
        let rects = map.roofs.len();
        let mine_rect: Vec<OnRoof> = map.roofs.iter().map(|r| on_roof(map, |q| r.contains(q))).collect();
        let mine_outline: Vec<OnRoof> = map.outline_roofs.iter().map(|r| on_roof(map, |q| r.contains(q))).collect();

        // Покрытия и всё, что вклеивается, — заранее, по разу: дальше крыши рисуют потоки.
        let mut covers: HashMap<String, (image::RgbaImage, [f32; 2])> = HashMap::new();
        let mut solids: HashMap<String, image::RgbaImage> = HashMap::new();
        let solid = |key: &str, solids: &mut HashMap<String, image::RgbaImage>| -> Result<(), String> {
            if !solids.contains_key(key) {
                let bytes = self.textures.bytes(key)?;
                solids.insert(key.to_owned(), roofs::solid_cover(library::decode_tinted(key, &bytes)?));
            }
            Ok(())
        };
        let item = |key: &str, covers: &mut HashMap<String, (image::RgbaImage, [f32; 2])>| -> Result<(), String> {
            if !covers.contains_key(key) {
                let bytes = self.textures.bytes(key)?;
                covers.insert(key.to_owned(), (library::decode_tinted(key, &bytes)?, [roofs::PARAPET_TILE, roofs::PARAPET]));
            }
            Ok(())
        };
        for &id in groups.iter().flatten() {
            solid(if id < rects { &map.roofs[id].paint } else { &map.outline_roofs[id - rects].pieces[0].paint }, &mut solids)?;
        }
        for (ri, r) in map.roofs.iter().enumerate() {
            if !covers.contains_key(&r.paint) {
                let bytes = self.textures.bytes(&r.paint)?;
                let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(&r.paint)))?.to_rgba8();
                let tile = roofs::tile_cells([img.width(), img.height()]);
                covers.insert(r.paint.clone(), (img, tile));
            }
            for pk in r.parapet().into_iter().map(str::to_owned).chain(r.fixtures().iter().map(|f| f.0.to_owned())).chain(mine_rect[ri].iter().map(|m| m.0.clone())) {
                item(&pk, &mut covers)?;
            }
        }
        for (idx, r) in map.outline_roofs.iter().enumerate() {
            for piece in &r.pieces {
                solid(&piece.paint, &mut solids)?;
            }
            for m in &mine_outline[idx] {
                item(&m.0, &mut covers)?;
            }
        }
        let casters = roofs::Casters::new(&map.outline_roofs, &map.props);

        // Крыша — в родном разрешении покрытия (256 на клетку), не в мелком масштабе
        // сцены: Foundry растянет тайл сам. Огромная — не больше 8192 точек. Дома из OSM —
        // в масштабе сцены (их сотни); заданная руками крыша по контуру — как прямоугольная.
        let fine = |span: f32| p.max(library::PX_PER_CELL.min(8192.0 / span.max(1.0)));
        let rect_ppc = |r: &map::Roof| fine((r.b[0] - r.a[0]).max(r.b[1] - r.a[1]) + 2.0 * r.overhang + 1.5);
        let outline_ppc = |r: &map::OutlineRoof| {
            let (lo, hi) = r.bounds();
            if r.pieces.iter().any(|k| k.overhang.is_some()) { fine((hi[0] - lo[0]).max(hi[1] - lo[1]) + 1.5) } else { p }
        };
        let group_ppc: Vec<f32> = groups
            .iter()
            .map(|group| {
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for &id in group {
                    let (a, b) = if id < rects { (map.roofs[id].outline()[0], map.roofs[id].outline()[2]) } else { map.outline_roofs[id - rects].bounds() };
                    (lo, hi) = ([lo[0].min(a[0]), lo[1].min(a[1])], [hi[0].max(b[0]), hi[1].max(b[1])]);
                }
                fine((hi[0] - lo[0]).max(hi[1] - lo[1]) + 1.5)
            })
            .collect();
        let items_of = |id: usize| -> Vec<roofs::Fixture> {
            if id < rects {
                let mut items: Vec<roofs::Fixture> = map.roofs[id].fixtures().iter().map(|f| (&covers[f.0].0, f.1, f.2, [1.0, 1.0])).collect();
                items.extend(mine_rect[id].iter().map(|m| (&covers[&m.0].0, m.1, m.2, m.3)));
                items
            } else {
                mine_outline[id - rects].iter().map(|m| (&covers[&m.0].0, m.1, m.2, m.3)).collect()
            }
        };

        // Отпечатки работ: всё, от чего зависит картинка.
        let sources = &self.textures.sources;
        let fp = |job: &RoofJob| -> u128 {
            let mut h = shotcache::Twin::new();
            (shotcache::program_stamp(), px, light.map(f32::to_bits), snow, map.width, map.height).hash(&mut h);
            let mut keys: Vec<String> = Vec::new();
            let mut member = |id: usize, h: &mut shotcache::Twin| {
                if id < rects {
                    let r = &map.roofs[id];
                    serde_json::to_vec(r).unwrap_or_default().hash(h);
                    keys.push(r.paint.clone());
                    keys.extend(r.parapet().map(str::to_owned));
                    keys.extend(r.fixtures().iter().map(|f| f.0.to_owned()));
                    for m in &mine_rect[id] {
                        (&m.0, m.1.map(f32::to_bits), m.2.to_bits(), m.3.map(f32::to_bits)).hash(h);
                        keys.push(m.0.clone());
                    }
                } else {
                    let r = &map.outline_roofs[id - rects];
                    serde_json::to_vec(r).unwrap_or_default().hash(h);
                    keys.extend(r.pieces.iter().map(|k| k.paint.clone()));
                    for m in &mine_outline[id - rects] {
                        (&m.0, m.1.map(f32::to_bits), m.2.to_bits(), m.3.map(f32::to_bits)).hash(h);
                        keys.push(m.0.clone());
                    }
                }
            };
            match *job {
                RoofJob::Group(g) => {
                    (1u8, group_ppc[g].to_bits()).hash(&mut h);
                    for &id in &groups[g] {
                        member(id, &mut h);
                    }
                }
                RoofJob::Rect(ri) => {
                    (2u8, rect_ppc(&map.roofs[ri]).to_bits()).hash(&mut h);
                    member(ri, &mut h);
                    for c in &climbs {
                        c.map(f32::to_bits).hash(&mut h);
                    }
                }
                RoofJob::Outline(idx) => {
                    let r = &map.outline_roofs[idx];
                    (3u8, outline_ppc(r).to_bits()).hash(&mut h);
                    member(rects + idx, &mut h);
                    let near = casters.near(idx, r.bounds());
                    for (rings, z) in &near.houses {
                        z.to_bits().hash(&mut h);
                        for q in rings.iter().flatten() {
                            q.map(f32::to_bits).hash(&mut h);
                        }
                    }
                    for c in &near.crowns {
                        (c.pos.map(f32::to_bits), c.d.to_bits(), c.bare).hash(&mut h);
                    }
                }
            }
            shotcache::assets_stamp(keys.iter().map(String::as_str), sources).hash(&mut h);
            h.finish128()
        };
        let mut jobs: Vec<(RoofJob, Vec<usize>)> = (0..groups.len()).map(|g| (RoofJob::Group(g), groups[g].clone())).collect();
        jobs.extend((0..rects).filter(|i| !grouped.contains(i)).map(|i| (RoofJob::Rect(i), vec![i])));
        jobs.extend((0..map.outline_roofs.len()).filter(|i| !grouped.contains(&(rects + i))).map(|i| (RoofJob::Outline(i), vec![rects + i])));

        let mut out: HashMap<usize, RoofPic> = HashMap::new();
        let mut todo: Vec<(RoofJob, Vec<usize>, u128)> = Vec::new();
        let slot = |f: u128, j: usize| -> u128 { f ^ (j as u128 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835) };
        for (job, ids) in jobs {
            let f = fp(&job);
            let cached: Option<Vec<RoofPic>> = store.and_then(|s| ids.iter().enumerate().map(|(j, _)| s.load_blob(slot(f, j)).and_then(|b| RoofPic::from_blob(&b))).collect());
            match cached {
                Some(pics) => out.extend(ids.into_iter().zip(pics)),
                None => todo.push((job, ids, f)),
            }
        }

        let encode = |img: &image::RgbaImage, origin: [f32; 2], ppc: f32| RoofPic { bytes: export::webp_bytes(img, 88.0), origin, ppc, size: [img.width(), img.height()] };
        let none = || RoofPic { bytes: Vec::new(), origin: [0.0, 0.0], ppc: 1.0, size: [0, 0] };
        let run = |job: &RoofJob| -> Vec<RoofPic> {
            match *job {
                RoofJob::Group(g) => {
                    let ppc = group_ppc[g];
                    let imgs = roofs::render_joined(&map.roofs, &map.outline_roofs, &groups[g], &|k: &str| solids.get(k), ppc, light, clip, snow);
                    groups[g]
                        .iter()
                        .zip(imgs)
                        .map(|(&id, (mut img, origin))| {
                            if id >= rects && img.width() <= 1 {
                                return none();
                            }
                            roofs::paste_fixtures(&mut img, origin, ppc, &items_of(id));
                            encode(&img, origin, ppc)
                        })
                        .collect()
                }
                RoofJob::Rect(ri) => {
                    let r = &map.roofs[ri];
                    let (tex, tile) = &covers[&r.paint];
                    let parapet = r.parapet().map(|k| &covers[k].0);
                    let ppc = rect_ppc(r);
                    let (img, origin) = roofs::render(r, tex, *tile, ppc, light, parapet, &climbs, &items_of(ri), snow);
                    vec![encode(&img, origin, ppc)]
                }
                RoofJob::Outline(idx) => {
                    let r = &map.outline_roofs[idx];
                    let ppc = outline_ppc(r);
                    let near = casters.near(idx, r.bounds());
                    let (mut img, origin) = roofs::render_outline(r, &|k: &str| solids.get(k), ppc, light, clip, &near);
                    if img.width() <= 1 {
                        return vec![none()];
                    }
                    roofs::paste_fixtures(&mut img, origin, ppc, &items_of(rects + idx));
                    vec![encode(&img, origin, ppc)]
                }
            }
        };
        // Потоки берут работы по одной: группа врезки бывает в разы дольше одиночной крыши.
        let done = std::sync::Mutex::new(Vec::new());
        let next = std::sync::atomic::AtomicUsize::new(0);
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 6).min(todo.len().max(1));
        if cfg!(target_arch = "wasm32") || threads <= 1 {
            for (job, ids, f) in &todo {
                done.lock().map_err(|_| "поток крыш упал".to_owned())?.push((ids.clone(), *f, run(job)));
            }
        } else {
            std::thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| {
                        loop {
                            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            let Some((job, ids, f)) = todo.get(i) else { break };
                            let pics = run(job);
                            if let Ok(mut d) = done.lock() {
                                d.push((ids.clone(), *f, pics));
                            }
                        }
                    });
                }
            });
        }
        for (ids, f, pics) in done.into_inner().map_err(|_| "поток крыш упал".to_owned())? {
            for (j, (id, pic)) in ids.into_iter().zip(pics).enumerate() {
                if let Some(s) = store {
                    s.save_blob(slot(f, j), &pic.to_blob());
                }
                out.insert(id, pic);
            }
        }
        out.retain(|_, pic| pic.size[0] > 0);
        Ok(out)
    }
}
