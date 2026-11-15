//! Выгрузка: снимки холста плитками, запись файлов, снимки окна.

use super::*;

impl App {
    /// Начать выгрузку: плитки по размеру холста, камера и выбор запоминаются.
    pub fn start_export(&mut self, dir: PathBuf, name: String, px: u32, quit: bool) {
        // Большая карта — мельче: иначе Foundry картинку не покажет.
        let px = export::fit_px(self.map.width, self.map.height, px);
        let (w, h) = (self.map.width * px, self.map.height * px);
        self.notice = None;
        let start_level = self.level;
        let sel = self.sel.take();
        self.switch_level(0);
        let stumps = if self.settings.gen_library { Palette::from_library(&self.library.assets, Box::new(|_| None)).keys("stump") } else { Vec::new() };
        self.export = Some(Export {
            dir,
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
        });
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
        if e.tiles.is_empty() {
            // Окно только что развернули для снимка: дать ему принять размер.
            e.wait += 1;
            if e.wait < 3 {
                return;
            }
            // Плитки — по холсту развёрнутого окна.
            e.tile = [((canvas.width() * ppp).floor() as u32).saturating_sub(2).max(64), ((canvas.height() * ppp).floor() as u32).saturating_sub(2).max(64)];
            let (w, h) = (e.image.width(), e.image.height());
            e.tiles = (0..h.div_ceil(e.tile[1])).flat_map(|ty| (0..w.div_ceil(e.tile[0])).map(move |tx| [tx, ty])).collect();
        }
        let [tx, ty] = e.tiles[e.next];
        // Начало плитки ложится на целый пиксель экрана: плитки стыкуются без шва.
        let origin = [(canvas.min.x * ppp).ceil(), (canvas.min.y * ppp).ceil()];
        let scale = e.px as f32 / ppp;
        let world = [e.origin[0] + (tx * e.tile[0]) as f32 / e.px as f32, e.origin[1] + (ty * e.tile[1]) as f32 / e.px as f32];
        self.cam.scale = scale;
        self.cam.pan = vec2(origin[0] / ppp - canvas.min.x - world[0] * scale, origin[1] / ppp - canvas.min.y - world[1] * scale);
        e.settle += 1;
        if e.settle >= 3 && idle {
            e.asked = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
    }

    /// Снимок пришёл: вырезать плитку в большую картинку; последняя — записать файлы.
    pub(super) fn export_tile(&mut self, ctx: &egui::Context, shot: &egui::ColorImage) {
        let ppp = ctx.pixels_per_point();
        let canvas = self.canvas;
        let Some(e) = &mut self.export else { return };
        let [tx, ty] = e.tiles[e.next];
        let (ox, oy) = ((canvas.min.x * ppp).ceil() as usize, (canvas.min.y * ppp).ceil() as usize);
        let (x0, y0) = (tx * e.tile[0], ty * e.tile[1]);
        let w = e.tile[0].min(e.image.width() - x0);
        let h = e.tile[1].min(e.image.height() - y0);
        let [sw, sh] = shot.size;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (sx, sy) = (ox + x, oy + y);
                if sx < sw && sy < sh {
                    let c = shot.pixels[sy * sw + sx];
                    e.image.put_pixel(x0 + x as u32, y0 + y as u32, image::Rgb([c.r(), c.g(), c.b()]));
                }
            }
        }
        e.next += 1;
        e.asked = false;
        e.settle = 0;
        if e.next < e.tiles.len() {
            return;
        }
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
            e.next = 0;
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
            e.next = 0;
            let k = e.doing;
            self.switch_level(k);
            return;
        }
        let Some(mut e) = self.export.take() else { return };
        let last = std::mem::replace(&mut e.image, image::RgbImage::new(1, 1));
        e.done.push(last);
        self.switch_level(e.start_level);
        self.cam = e.cam;
        self.sel = e.sel;
        let result = self.write_export(&e);
        match &result {
            Ok(msg) => self.notice = Some(msg.clone()),
            Err(err) => self.error = Some(format!("Выгрузка не удалась: {err}")),
        }
        if e.quit {
            match result {
                Ok(msg) => println!("{msg}"),
                Err(err) => eprintln!("{err}"),
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
        let n = e.done.len();
        let stem = |k: usize| if n == 1 { e.name.clone() } else { format!("{}_{}", e.name, k + 1) };
        let mut jpgs = Vec::new();
        let mut srcs = Vec::new();
        let mut layouts = Vec::new();
        // Контуры палаток по картинке — один раз на картинку.
        let mut hulls: std::collections::HashMap<String, Vec<[f32; 2]>> = std::collections::HashMap::new();
        // Фон верхнего этажа, где на нём есть пустота, — с прозрачностью: в Foundry
        // сквозь неё виден этаж ниже. Закрашенный целиком — сплошной, как у первого.
        let mut open: Vec<Option<image::RgbaImage>> = Vec::new();
        for (k, img) in e.done.iter().enumerate() {
            let holes = e.whites.get(k).and_then(|w| w.as_ref()).and_then(|white| export::floor_from_pair(img, white));
            // JPEG нужен и для Universal VTT: он лежит внутри .dd2vtt.
            let mut jpg = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(img).map_err(|x| x.to_string())?;
            let (bytes, ext) = match &holes {
                Some(rgba) => export::layer_bytes(rgba, 88.0),
                // Не вышел WebP (велика или libwebp не справился) — JPEG.
                None => match (img.width().max(img.height()) <= export::WEBP_MAX_SIDE).then(|| export::try_webp(&image::DynamicImage::ImageRgb8(img.clone()).to_rgba8(), 88.0)).flatten() {
                    Some(webp) => (webp, "webp"),
                    None => (jpg.clone(), "jpg"),
                },
            };
            open.push(holes);
            let img_path = e.dir.join(format!("{}.{ext}", stem(k)));
            files.push((img_path.clone(), bytes));
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            // Вырез воды под мостом — по нарисованному: рамка картинки, по разу на картинку.
            let sources = &self.textures.sources;
            let boxes = std::cell::RefCell::new(std::collections::HashMap::new());
            let seen = |key: &str| *boxes.borrow_mut().entry(key.to_owned()).or_insert_with(|| library::visible_box(sources, key));
            let mut lay = export::layout_seen(map, &map.room_edges(), &seen);
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
            layouts.push(lay);
            srcs.push(export::foundry_src(&img_path));
            jpgs.push(jpg);
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
            let file = e.dir.join(format!("{}_створка_{}.webp", e.name, leaves.len() + 1));
            files.push((file.clone(), export::webp_bytes(&img.to_rgba8(), 90.0)));
            leaves.insert(key, export::LeafFile { src: export::foundry_src(&file), width });
        }
        // Крыши — картинками по одной, рядом со сценой.
        let mut covers: std::collections::HashMap<String, (image::RgbaImage, [f32; 2])> = std::collections::HashMap::new();
        let mut roof_tiles: Vec<Vec<export::RoofTile>> = Vec::new();
        let mut roof_count = 0;
        let mut outline_covers: std::collections::HashMap<String, image::RgbaImage> = std::collections::HashMap::new();
        let mut canopies: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut seen = std::collections::HashMap::new();
        // Слой стен каждого этажа (он же для Universal VTT), его файл и во сколько раз
        // тайл предмета шире картинки без тени.
        let mut wall_layers: std::collections::HashMap<usize, image::RgbaImage> = std::collections::HashMap::new();
        let mut foregrounds: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
        let mut grows: std::collections::HashMap<String, [f32; 2]> = std::collections::HashMap::new();
        for k in 0..n {
            let map = if k == self.level { &self.map } else { &self.levels[k] };
            let light = map.shadows.light();
            let climbs = super::draw_ground::climb_rects(&map.props, &self.textures.sources, &mut seen);
            let mut tiles = Vec::new();
            // Крыши, что врезаются друг в друга, — одной отрисовкой на группу; картинка
            // и тайл у каждой свои: в них только то, что у неё видно поверх соседей.
            // Группа рисуется, когда очередь доходит до первой её крыши, а не все заранее:
            // у города в сотни крыш картинки всех групп разом — гигабайты памяти.
            let mut joined: std::collections::HashMap<usize, (image::RgbaImage, [f32; 2], f32)> = std::collections::HashMap::new();
            let groups = roofs::raster_groups(&map.roofs, &map.outline_roofs);
            let group_of: std::collections::HashMap<usize, usize> = groups.iter().enumerate().flat_map(|(g, ids)| ids.iter().map(move |&id| (id, g))).collect();
            let rects = map.roofs.len();
            let mut spans = Vec::new();
            for group in &groups {
                let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
                for &id in group {
                    let (paint, (a, b)) = if id < rects { (&map.roofs[id].paint, (map.roofs[id].outline()[0], map.roofs[id].outline()[2])) } else { (&map.outline_roofs[id - rects].pieces[0].paint, map.outline_roofs[id - rects].bounds()) };
                    (lo, hi) = ([lo[0].min(a[0]), lo[1].min(a[1])], [hi[0].max(b[0]), hi[1].max(b[1])]);
                    if !outline_covers.contains_key(paint) {
                        let bytes = self.textures.bytes(paint)?;
                        outline_covers.insert(paint.clone(), roofs::solid_cover(library::decode_tinted(paint, &bytes)?));
                    }
                }
                spans.push((hi[0] - lo[0]).max(hi[1] - lo[1]) + 1.5);
            }
            let mut drawn = std::collections::HashSet::new();
            let mut draw_group = |id: usize, joined: &mut std::collections::HashMap<usize, (image::RgbaImage, [f32; 2], f32)>, covers: &std::collections::HashMap<String, image::RgbaImage>| {
                let Some(&g) = group_of.get(&id) else { return };
                if !drawn.insert(g) {
                    return;
                }
                let ppc = (e.px as f32).max(library::PX_PER_CELL.min(8192.0 / spans[g].max(1.0)));
                let out = roofs::render_joined(&map.roofs, &map.outline_roofs, &groups[g], &|k: &str| covers.get(k), ppc, light, ([0.0, 0.0], [map.width as f32, map.height as f32]), map.weather == "snow");
                for (id, (img, origin)) in groups[g].iter().zip(out) {
                    joined.insert(*id, (img, origin, ppc));
                }
            };
            for (ri, r) in map.roofs.iter().enumerate() {
                if !covers.contains_key(&r.paint) {
                    let bytes = self.textures.bytes(&r.paint)?;
                    let img = image::load_from_memory(&bytes).map_err(|x| format!("{}: {x}", library::label(&r.paint)))?.to_rgba8();
                    let tile = roofs::tile_cells([img.width(), img.height()]);
                    covers.insert(r.paint.clone(), (img, tile));
                }
                let fixtures = r.fixtures();
                // Дымоходы, вытяжки, люки этой крыши (слой «На крыше») — в её картинку:
                // тает крыша, когда игрок вошёл в дом, — тают и они.
                let mine: Vec<(String, [f32; 2], f32, [f32; 2])> = map
                    .props
                    .iter()
                    .filter(|p| (map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer()) && r.contains(p.pos))
                    .map(|p| (p.asset.clone(), p.pos, p.rot, [p.size[0] * p.scale, p.size[1] * p.scale]))
                    .collect();
                for pk in r.parapet().into_iter().chain(fixtures.iter().map(|f| f.0)).map(str::to_owned).chain(mine.iter().map(|m| m.0.clone())) {
                    if let std::collections::hash_map::Entry::Vacant(slot) = covers.entry(pk.clone()) {
                        let bytes = self.textures.bytes(&pk)?;
                        slot.insert((library::decode_tinted(&pk, &bytes)?, [roofs::PARAPET_TILE, roofs::PARAPET]));
                    }
                }
                let (tex, tile) = &covers[&r.paint];
                let parapet = r.parapet().map(|pk| &covers[pk].0);
                let mut items: Vec<roofs::Fixture> = fixtures.iter().map(|f| (&covers[f.0].0, f.1, f.2, [1.0, 1.0])).collect();
                items.extend(mine.iter().map(|m| (&covers[&m.0].0, m.1, m.2, m.3)));
                let smoke = smoke_spots(&mine, e.px as f32);
                // Крыша — в родном разрешении покрытия (256 на клетку), не в мелком
                // масштабе сцены: Foundry растянет тайл сам. Огромная — не больше 8192 точек.
                let span = (r.b[0] - r.a[0]).max(r.b[1] - r.a[1]) + 2.0 * r.overhang + 1.5;
                let ppc = (e.px as f32).max(library::PX_PER_CELL.min(8192.0 / span.max(1.0)));
                draw_group(ri, &mut joined, &outline_covers);
                let (img, origin, ppc) = match joined.remove(&ri) {
                    Some((mut img, origin, ppc)) => {
                        roofs::paste_fixtures(&mut img, origin, ppc, &items);
                        (img, origin, ppc)
                    }
                    None => {
                        let (img, origin) = roofs::render(r, tex, *tile, ppc, light, parapet, &climbs, &items, map.weather == "snow");
                        (img, origin, ppc)
                    }
                };
                roof_count += 1;
                let file = e.dir.join(format!("{}_крыша_{roof_count}.webp", stem(k)));
                files.push((file.clone(), export::webp_bytes(&img, 88.0)));
                let p = e.px as f32;
                tiles.push(export::RoofTile { src: export::foundry_src(&file), rect: [(origin[0] * p).round() as i64, (origin[1] * p).round() as i64, (img.width() as f32 * p / ppc).round() as i64, (img.height() as f32 * p / ppc).round() as i64], rotation: 0.0, shelter: true, fade: true, leaves: false, low: false, wind: 1.0, rope: false, web: None, chest: None, living: None, scuppers: fixtures.iter().filter(|f| f.0.contains("Scupper")).map(|f| [f.1[0] * p, f.1[1] * p, f.2]).collect(), chimneys: smoke, surface: vec![(r.outline().iter().map(|v| [v[0] * p, v[1] * p]).collect(), false)], ..Default::default() });
            }
            // Крыши по контуру (из OSM) — тоже по одной, в масштабе сцены: их сотни.
            let casters = roofs::Casters::new(&map.outline_roofs, &map.props);
            for (idx, r) in map.outline_roofs.iter().enumerate() {
                for piece in &r.pieces {
                    if !outline_covers.contains_key(&piece.paint) {
                        let bytes = self.textures.bytes(&piece.paint)?;
                        outline_covers.insert(piece.paint.clone(), roofs::solid_cover(library::decode_tinted(&piece.paint, &bytes)?));
                    }
                }
                // Дома из OSM — в масштабе сцены: их сотни. Крыша, заданная руками, — в
                // родном разрешении покрытия, как прямоугольная: Foundry растянет тайл сам.
                let p = e.px as f32;
                let (lo, hi) = r.bounds();
                let span = (hi[0] - lo[0]).max(hi[1] - lo[1]) + 1.5;
                let ppc = if r.pieces.iter().any(|k| k.overhang.is_some()) { p.max(library::PX_PER_CELL.min(8192.0 / span.max(1.0))) } else { p };
                let near = casters.near(idx, r.bounds());
                draw_group(rects + idx, &mut joined, &outline_covers);
                let (mut img, origin, ppc) = match joined.remove(&(rects + idx)) {
                    Some(j) => j,
                    None => {
                        let (img, origin) = roofs::render_outline(r, &|k: &str| outline_covers.get(k), ppc, light, ([0.0, 0.0], [map.width as f32, map.height as f32]), &near);
                        (img, origin, ppc)
                    }
                };
                if img.width() <= 1 {
                    continue;
                }
                // Крышное над крышей по контуру (свод ангара, купол, шатёр) — тоже в её
                // картинку: отдельным тайлом оно висело бы над комнатой, когда крыша тает.
                let mine: Vec<(String, [f32; 2], f32, [f32; 2])> = map
                    .props
                    .iter()
                    .filter(|p| (map::ON_ROOF..map::ABOVE_ROOFS).contains(&p.layer()) && r.contains(p.pos))
                    .map(|p| (p.asset.clone(), p.pos, p.rot, [p.size[0] * p.scale, p.size[1] * p.scale]))
                    .collect();
                for m in &mine {
                    if let std::collections::hash_map::Entry::Vacant(slot) = covers.entry(m.0.clone()) {
                        let bytes = self.textures.bytes(&m.0)?;
                        slot.insert((library::decode_tinted(&m.0, &bytes)?, [roofs::PARAPET_TILE, roofs::PARAPET]));
                    }
                }
                let items: Vec<roofs::Fixture> = mine.iter().map(|m| (&covers[&m.0].0, m.1, m.2, m.3)).collect();
                roofs::paste_fixtures(&mut img, origin, ppc, &items);
                let smoke = smoke_spots(&mine, p);
                roof_count += 1;
                let file = e.dir.join(format!("{}_крыша_{roof_count}.webp", stem(k)));
                files.push((file.clone(), export::webp_bytes(&img, 88.0)));
                tiles.push(export::RoofTile {
                    src: export::foundry_src(&file),
                    rect: [(origin[0] * p).round() as i64, (origin[1] * p).round() as i64, (img.width() as f32 * p / ppc).round() as i64, (img.height() as f32 * p / ppc).round() as i64],
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
                    let side = (prop.size[0].max(prop.size[1]) * (e.px as f32 * 1.5).max(library::PX_PER_CELL)).round() as u32;
                    let img = if img.width().max(img.height()) > side { img.resize(side, side, image::imageops::FilterType::Lanczos3) } else { img };
                    // Имя файла — по тому, что это: навес лавки, куст, бельё или крона.
                    let what = if map::awning_by_name(&prop.asset) {
                        "навес"
                    } else if map::bush_by_name(&prop.asset) {
                        "куст"
                    } else if map::line_by_name(&prop.asset) {
                        "бельё"
                    } else {
                        "дерево"
                    };
                    let file = e.dir.join(format!("{}_{what}_{}.webp", e.name, canopies.len() + 1));
                    files.push((file.clone(), export::webp_bytes(&img.to_rgba8(), 88.0)));
                    canopies.insert(look.clone(), export::foundry_src(&file));
                }
                let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                let mut tile = export::canopy_tile(canopies[&look].clone(), prop.pos, size, prop.rot, e.px);
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
                // Тень — в картинку тайла: своя на каждый поворот и размер предмета.
                let shade = (map.shadows.on && prop.carries_shadow()).then(|| format!("#тень{k}|{:.0}|{:.3}", prop.rot.rem_euclid(360.0), prop.scale));
                let mut save = |key: &str, files: &mut Vec<(PathBuf, Vec<u8>)>| -> Result<(String, [f32; 2]), String> {
                    let slot = format!("{key}{}", shade.as_deref().unwrap_or(""));
                    if let Some(src) = canopies.get(&slot) {
                        return Ok((src.clone(), grows.get(&slot).copied().unwrap_or([1.0, 1.0])));
                    }
                    let img = image::DynamicImage::ImageRgba8(self.textures.rgba(key)?);
                    let side = (prop.size[0].max(prop.size[1]) * (e.px as f32 * 1.5).max(library::PX_PER_CELL)).round() as u32;
                    let img = if img.width().max(img.height()) > side { img.resize(side, side, image::imageops::FilterType::Lanczos3) } else { img };
                    let (img, grow) = if shade.is_some() { export::with_shadow(&img.to_rgba8(), size, prop.rot, &map.shadows) } else { (img.to_rgba8(), [1.0, 1.0]) };
                    grows.insert(slot.clone(), grow);
                    let file = e.dir.join(format!("{}_предмет_{}.webp", e.name, canopies.len() + 1));
                    files.push((file.clone(), export::webp_bytes(&img, 90.0)));
                    let src = export::foundry_src(&file);
                    canopies.insert(slot, src.clone());
                    Ok((src, grow))
                };
                let (src, grow) = save(&prop.asset, &mut files)?;
                let mut tile = export::canopy_tile(src.clone(), prop.pos, [size[0] * grow[0], size[1] * grow[1]], prop.rot, e.px);
                tile.fade = false;
                tile.leaves = false;
                tile.item = true;
                tile.above = prop.layer() >= map::ON_ROOF;
                tile.hover = map::hover_by_name(&prop.asset);
                tile.swing = if map::swing_by_name(&prop.asset) { library::strand_ends(&self.textures.sources, &prop.asset) } else { None };
                tile.tint = prop.tint;
                if !(map::web_by_name(&prop.asset) || map::chest_variants(&prop.asset).is_some() || map::speaker_by_name(&prop.asset)) {
                    tiles.push(tile);
                    continue;
                }
                if let Some(living) = map::living_by_name(&prop.asset) {
                    tile.living = Some(living);
                    tiles.push(tile);
                    continue;
                }
                match map::chest_variants(&prop.asset) {
                    // Нет открытого или пустого в библиотеке — тот же закрытый.
                    Some((open, empty)) => {
                        let open = save(&open, &mut files).map_or_else(|_| src.clone(), |s| s.0);
                        let empty = save(&empty, &mut files).map_or_else(|_| open.clone(), |s| s.0);
                        tile.chest = Some((open, empty));
                    }
                    None => tile.web = Some(library::strand_ends(&self.textures.sources, &prop.asset)),
                }
                tiles.push(tile);
            }
            // Стены этажа — слоем во всю сцену, в `foreground` уровня: ядро держит его на
            // верхе этажа, над мебелью и токенами и под крышами. В нём же тень стен,
            // окна, косяки и слой «над стенами».
            if let Some([black, white]) = e.walls.get(k) {
                let layer = export::layer_from_pair(black, white);
                if export::opaque_box(&layer, 8).is_some() {
                    let (bytes, ext) = export::layer_bytes(&layer, 90.0);
                    let file = e.dir.join(format!("{}_стены.{ext}", stem(k)));
                    files.push((file.clone(), bytes));
                    foregrounds.insert(k, export::foundry_src(&file));
                    wall_layers.insert(k, layer);
                }
            }
            roof_tiles.push(tiles);
        }
        let levels: Vec<export::SceneLevel> = (0..n)
            .map(|k| export::SceneLevel { name: if n == 1 { e.name.clone() } else { self.level_name(k) }, image_src: srcs[k].clone(), layout: &layouts[k], roofs: std::mem::take(&mut roof_tiles[k]) })
            .collect();
        let layers: Vec<export::LevelLayers> = (0..n).map(|k| export::LevelLayers { foreground: foregrounds.remove(&k), solid: open[k].as_ref().map(export::Solid::of) }).collect();
        let mut scene = export::foundry_layered(&self.map, &e.name, &levels, &layers, e.px, &leaves);
        // Пути картинок — с меткой содержимого: Foundry не покажет прежнюю под тем же именем.
        let tags: std::collections::HashMap<String, String> = files.iter().map(|(p, b)| (export::foundry_src(p), export::content_tag(b))).collect();
        export::tag_sources(&mut scene, &tags);
        // Сцена открывается на том этаже, что открыт в редакторе: у корабля — палуба, а не трюм.
        if n > 1 {
            scene["initialLevel"] = serde_json::json!(export::level_id(self.level));
        }
        let scene_path = e.dir.join(format!("{}.json", e.name));
        files.push((scene_path, serde_json::to_string_pretty(&scene).unwrap_or_default().into_bytes()));
        // Universal VTT — для других VTT; модулю он не нужен, сцена создаётся сразу.
        #[cfg(not(target_arch = "wasm32"))]
        {
            use base64::Engine;
            // В Universal VTT тайлов нет: предметы, ушедшие в тайлы, впечатываются
            // в его картинку.
            let mut pics: std::collections::HashMap<String, Option<(image::RgbaImage, [f32; 2])>> = std::collections::HashMap::new();
            let mut flats: Vec<image::RgbImage> = Vec::new();
            for k in 0..n {
                let map = if k == self.level { &self.map } else { &self.levels[k] };
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
                for prop in items {
                    let size = [prop.size[0] * prop.scale, prop.size[1] * prop.scale];
                    // Тень предмета в фоне больше не лежит — впечатывается вместе с ним.
                    let shade = map.shadows.on && prop.carries_shadow();
                    let slot = if shade { format!("{}#тень{k}|{:.0}|{:.3}", prop.asset, prop.rot.rem_euclid(360.0), prop.scale) } else { prop.asset.clone() };
                    // У города тысячи предметов со своим поворотом: помнить картинку каждого — гигабайты.
                    if pics.len() >= 64 && !pics.contains_key(&slot) {
                        pics.clear();
                    }
                    let pic = pics.entry(slot).or_insert_with(|| {
                        let raw = self.textures.rgba(&prop.asset).ok()?;
                        Some(if shade { export::with_shadow(&raw, size, prop.rot, &map.shadows) } else { (raw, [1.0, 1.0]) })
                    });
                    if let Some((pic, grow)) = pic {
                        export::stamp(&mut flat, pic, [prop.pos[0] * p, prop.pos[1] * p], [size[0] * grow[0] * p, size[1] * grow[1] * p], prop.rot, prop.tint);
                    }
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
            std::fs::create_dir_all(&e.dir).map_err(|x| x.to_string())?;
            for (path, bytes) in &files {
                std::fs::write(path, bytes).map_err(|x| format!("{}: {x}", path.display()))?;
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = &jpgs;
            crate::plat::to_scene(&files, e.update)?;
        }
        let walls: usize = layouts.iter().flat_map(|l| l.walls.iter()).map(|l| l.len().saturating_sub(1)).sum();
        let (w, h) = (e.done[0].width(), e.done[0].height());
        Ok(format!(
            "{} {}: {}картинка {w}×{h}, стен {walls}, крыш {roof_count}, проёмов {}, огней {}",
            if cfg!(target_arch = "wasm32") { "На сцену —" } else { "Выгружено в" },
            e.dir.display(),
            if n > 1 { format!("этажей {n}, ") } else { String::new() },
            layouts.iter().map(|l| l.portals.len()).sum::<usize>(),
            layouts.iter().map(|l| l.lights.len()).sum::<usize>()
        ))
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

/// Дымоходы среди крышного (ключ, середина в клетках, поворот, размер): середина
/// и большая сторона в точках сцены — «Wild Canvas» пускает над ними дым.
fn smoke_spots(mine: &[(String, [f32; 2], f32, [f32; 2])], px: f32) -> Vec<[f32; 3]> {
    mine.iter().filter(|m| map::smokes_by_name(&m.0)).map(|m| [m.1[0] * px, m.1[1] * px, m.3[0].max(m.3[1]) * px]).collect()
}
