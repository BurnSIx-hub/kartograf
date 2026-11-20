//! Отрисовка: пол, маски, крыши и их тени.

use super::*;

/// Кусок земли (слоя на кусок маски), собранный в координатах карты: вершины в клетках.
/// Каждый кадр он только переводится в точки экрана — сила вершин, кривые на краю и узор
/// считаются заново, лишь когда сменился сам кусок маски, его соседи или слои над ним.
struct GroundMesh {
    key: u64,
    verts: Vec<egui::epaint::Vertex>,
    idx: Vec<u32>,
    used: u64,
}

/// Куски земли всех слоёв, что видны. Ключ — слой (`slot`) и номер куска маски.
#[derive(Default)]
pub(super) struct GroundCache {
    pieces: std::collections::HashMap<(u64, u32), GroundMesh>,
    bytes: usize,
}

/// Больше памяти под куски земли не держать: давно не видные выбрасываются.
const GROUND_BUDGET: usize = 256 << 20;

impl GroundCache {
    /// Выбросить давно не видные куски, пока память сверх предела.
    pub(super) fn trim(&mut self, frame: u64) {
        if self.bytes <= GROUND_BUDGET {
            return;
        }
        let mut old: Vec<((u64, u32), u64, usize)> = self.pieces.iter().filter(|(_, p)| p.used + 2 < frame).map(|(k, p)| (*k, p.used, piece_bytes(p))).collect();
        old.sort_by_key(|o| o.1);
        for (k, _, b) in old {
            if self.bytes <= GROUND_BUDGET * 3 / 4 {
                break;
            }
            self.pieces.remove(&k);
            self.bytes -= b;
        }
    }

    pub(super) fn stats(&self) -> serde_json::Value {
        serde_json::json!({"pieces": self.pieces.len(), "mb": self.bytes / (1 << 20)})
    }
}

fn piece_bytes(p: &GroundMesh) -> usize {
    p.verts.len() * std::mem::size_of::<egui::epaint::Vertex>() + p.idx.len() * 4
}

impl App {
    /// Слой материала: сетка четырёхугольников по точкам маски, прозрачность вершины —
    /// сила слоя в ней. Края мягкой кисти так и получаются мягкими. `clip` — маска,
    /// за которую слой не выходит (пол комнат — за саму комнату). `above` — слои,
    /// которые лягут поверх: сила в маске — доля слоя в точке, а рисуются слои
    /// друг на друга, поэтому прозрачность пересчитывается (см. [`map::coat_alpha`]).
    /// `slot` — какой это слой (вид и номер): под ним кэшируются его куски.
    pub(super) fn draw_coat(&mut self, painter: &egui::Painter, canvas: Rect, coat: &Coat, above: &[Coat], clip: Option<&Mask>, view: Rect, slot: u64) {
        let Some(material) = self.map.materials.get(coat.material as usize).cloned() else { return };
        self.draw_mask(painter, canvas, &material, &coat.mask, above, clip, view, slot);
    }

    /// Маска, залитая материалом: сила точки — прозрачность. Кусками маски: видимые куски
    /// берутся из кэша и перестраиваются, только если в них (или у соседей, или в слоях
    /// над ними) что-то поменялось.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_mask(&mut self, painter: &egui::Painter, canvas: Rect, material: &Material, m: &Mask, above: &[Coat], clip: Option<&Mask>, view: Rect, slot: u64) {
        let res = RES as f32;
        // Мельче четырёх точек экрана четырёхугольник не дробится: дальше глаз не различит,
        // а на большой карте издалека это сотни тысяч лишних вершин.
        let px = self.cam.scale / res;
        let step = [1usize, 2, 4, 8].into_iter().find(|&s| px * s as f32 >= 4.0).unwrap_or(8);
        let clampi = |v: f32, hi: usize| (v.floor() as isize).clamp(-1, hi as isize);
        let (i0, i1) = (clampi(view.min.x * res - 1.0, m.w), clampi(view.max.x * res + 1.0, m.w));
        let (j0, j1) = (clampi(view.min.y * res - 1.0, m.h), clampi(view.max.y * res + 1.0, m.h));
        if i0 >= i1 || j0 >= j1 {
            return;
        }
        let (tex, tile, tint): (Option<egui::TextureId>, [f32; 2], [u8; 3]) = match &material.paint {
            Paint::Color(c) => (None, [1.0, 1.0], *c),
            Paint::Texture(key) => {
                let key = if material.vary { format!("{}{key}", library::VARY) } else { key.clone() };
                let failed = self.textures.failed(&key, false);
                // Плитка — того размера, что не мельче её на экране: точка картинки — столько точек экрана.
                let k = self.cam.scale * self.canvas_ppp.max(1.0) / PX_PER_CELL;
                match self.textures.get_scaled(&key, k) {
                    // Плитка — по размеру картинки, 256 точек на клетку, как в Dungeondraft.
                    Some((t, px)) => (Some(t.id()), [(px[0] as f32 / PX_PER_CELL).max(MIN_TILE), (px[1] as f32 / PX_PER_CELL).max(MIN_TILE)], [255, 255, 255]),
                    None if failed => (None, [1.0, 1.0], [160, 40, 40]),
                    None => (None, [1.0, 1.0], [60, 60, 60]),
                }
            }
        };
        // Вблизи четырёхугольник у края дробится мельче, сила между точками маски —
        // по кривой Катмулла — Рома через соседние точки. По прямой между четырьмя
        // углами край ложится ступеньками в шаг маски, по кривой — плавной линией.
        let detail = if step == 1 { ((px / 6.0).ceil() as usize).clamp(1, 8) } else { 1 };
        // Общая часть ключа: вид слоя, шаг, дробление, размер маски.
        let base = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (tex, tile[0].to_bits(), tile[1].to_bits(), tint, material.rot.to_bits(), step, detail, m.w, m.h, above.len(), clip.is_some()).hash(&mut h);
            h.finish()
        };
        // Кэшируется кусками: чем мельче дробление вблизи, тем мельче кусок — вершин в куске
        // не больше нескольких десятков тысяч, а за окном лишнего немного.
        let tshift = match detail {
            1 => 0,
            2 => 1,
            3 | 4 => 2,
            _ => 3,
        };
        let tsz = mask::CHUNK >> tshift;
        let (npw, nph) = (m.w.div_ceil(tsz).max(1), m.h.div_ceil(tsz).max(1));
        let piece_of = |v: isize, n: usize| ((v.max(0) as usize) / tsz).min(n - 1);
        let (px0, px1) = (piece_of(i0, npw), piece_of(i1 - 1, npw));
        let (py0, py1) = (piece_of(j0, nph), piece_of(j1 - 1, nph));
        // Отпечаток куска маски — по самому слою, слоям над ним и маске-ограде; ключ куска
        // кэша — по отпечаткам своего куска маски и восьми соседей: квадраты у края берут
        // точки соседа.
        let mix = |h: u64, v: u64| (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
        let (nx, ny) = m.chunks_wh();
        let (cx0, cx1) = ((px0 * tsz / mask::CHUNK).saturating_sub(1), ((px1 * tsz / mask::CHUNK) + 1).min(nx - 1));
        let (cy0, cy1) = ((py0 * tsz / mask::CHUNK).saturating_sub(1), ((py1 * tsz / mask::CHUNK) + 1).min(ny - 1));
        let cw = cx1 - cx0 + 1;
        let mut own = vec![0u64; cw * (cy1 - cy0 + 1)];
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let c = cy * nx + cx;
                let mut h = mix(0x9e37_79b9_7f4a_7c15, m.chunk_gen(c));
                for a in above {
                    h = mix(h, a.mask.chunk_gen(c));
                }
                if let Some(cl) = clip {
                    h = mix(h, cl.chunk_gen(c));
                }
                own[(cy - cy0) * cw + cx - cx0] = h;
            }
        }
        let frame = self.frame;
        let under = self.under_level as u64;
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        let cam = self.cam;
        let started = crate::plat::Instant::now();
        for pj in py0..=py1 {
            for pi in px0..=px1 {
                let (cx, cy) = (pi * tsz / mask::CHUNK, pj * tsz / mask::CHUNK);
                let mut key = mix(base, (pi as u64) << 32 | pj as u64);
                for y in cy.saturating_sub(1)..=(cy + 1).min(ny - 1) {
                    for x in cx.saturating_sub(1)..=(cx + 1).min(nx - 1) {
                        key = mix(key, own[(y - cy0) * cw + x - cx0]);
                    }
                }
                let id = (slot ^ under.rotate_left(56), (tshift as u32) << 28 | (pj * npw + pi) as u32);
                let fresh = self.ground_cache.pieces.get(&id).is_some_and(|p| p.key == key);
                if !fresh {
                    let (verts, idx) = ground_piece(m, above, clip, step, detail, tex.is_some(), tile, tint, material.rot, pi, pj, tsz);
                    let piece = GroundMesh { key, verts, idx, used: frame };
                    self.ground_cache.bytes += piece_bytes(&piece);
                    if let Some(old) = self.ground_cache.pieces.insert(id, piece) {
                        self.ground_cache.bytes -= piece_bytes(&old);
                    }
                }
                let Some(piece) = self.ground_cache.pieces.get_mut(&id) else { continue };
                piece.used = frame;
                // В точки экрана — только сдвиг и масштаб.
                let base_v = mesh.vertices.len() as u32;
                mesh.vertices.extend(piece.verts.iter().map(|v| egui::epaint::Vertex { pos: cam.to_screen(canvas, [v.pos.x, v.pos.y]), ..*v }));
                mesh.indices.extend(piece.idx.iter().map(|&i| i + base_v));
            }
        }
        self.prof.add("ground_pieces", started);
        if mesh.is_empty() {
            return;
        }
        if material.vary && tex.is_some() {
            self.blend.paint(painter, mesh);
        } else {
            painter.add(Shape::mesh(mesh));
        }
    }

    /// Крыша: скаты своей картинкой, каждый со своей освещённостью; гребни на
    /// коньке и рёбрах — ряд того же покрытия вдоль линии; тёмная кромка свеса.
    /// Покрытие крыши точками, в цвете ключа; пак ещё не подключён — `None`.
    pub(super) fn cover_rgba(&mut self, key: &str) -> Option<std::sync::Arc<image::RgbaImage>> {
        if let Some(img) = self.cover_cache.get(key) {
            return Some(img.clone());
        }
        let bytes = self.textures.bytes(key).ok()?;
        let img = std::sync::Arc::new(roofs::solid_cover(library::decode_tinted(key, &bytes).ok()?));
        self.cover_cache.insert(key.to_owned(), img.clone());
        Some(img)
    }

    /// Крыши по контуру: картинкой, нарисованной точками по
    /// [`roofs::render_outline`], — их скаты сходят к любой стороне, мешем egui
    /// так не нарисовать. Разрешение картинки — под масштаб окна ([`outline_ppc`]):
    /// раньше оно было не больше 24 точек на клетку, и рядом с прямоугольной
    /// крышей (её рисует видеокарта в полную резкость) такая выходила мылом.
    /// Рисуют их фоновые потоки ([`RoofJob`]), кадр только забирает готовые: прежде
    /// крыши рисовались в кадре, и смена масштаба над городом давала кадры по
    /// 150–300 мс. Пока новая не готова, видна прежняя.
    pub(super) fn draw_outline_roofs(&mut self, painter: &egui::Painter, canvas: Rect, view: Rect, alpha: f32, light: [f32; 2]) {
        use std::hash::{Hash, Hasher};
        // Готовые картинки — в кэш.
        let mut arrived = Vec::new();
        self.roof_jobs.retain(|_, job| match job.rx.try_recv() {
            Ok(out) => {
                arrived.push((std::mem::take(&mut job.targets), out));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => false,
        });
        for (targets, out) in arrived {
            for (t, (ci, origin, size)) in targets.into_iter().zip(out) {
                let handle = painter.ctx().load_texture(format!("roof-{t}"), ci, egui::TextureOptions::LINEAR);
                self.outline_cache.insert(t, (handle, origin, size));
            }
        }
        let max_jobs = export::threads();
        let roofs = std::mem::take(&mut self.map.outline_roofs);
        let mut pending = false;
        let exact = self.export.is_some();
        let want = (self.cam.scale * self.canvas_ppp).max(1.0);
        let casters = roofs::Casters::new(&roofs, &self.map.props);
        // Скатные прямоугольные крыши и группы врезки рисуются картинкой по граням —
        // после остальных; сеткой остаются плоская и односкатная.
        let rects = self.map.roofs.clone();
        let groups = roofs::raster_groups(&rects, &roofs);
        let snow = self.map.weather == "snow";
        // Что нарисовано в этом кадре: при тесноте в памяти остальное выбрасывается.
        let mut drawn: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let grouped: std::collections::HashSet<usize> = groups.iter().flatten().copied().collect();
        self.joined_rects = grouped.iter().copied().filter(|id| *id < rects.len()).collect();
        for (idx, r) in roofs.iter().enumerate() {
            if grouped.contains(&(rects.len() + idx)) {
                continue;
            }
            let (lo, hi) = r.bounds();
            if hi[0] < view.min.x || hi[1] < view.min.y || lo[0] > view.max.x || lo[1] > view.max.y {
                continue;
            }
            // Тень на крышу от высших соседей и крон: сдвинули дерево — крыша новая.
            let near = casters.near(idx, (lo, hi));
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            for (rings, z) in &near.houses {
                z.to_bits().hash(&mut hs);
                for p in rings.iter().flatten() {
                    (p[0].to_bits(), p[1].to_bits()).hash(&mut hs);
                }
            }
            for c in &near.crowns {
                (c.pos[0].to_bits(), c.pos[1].to_bits(), c.d.to_bits(), c.bare).hash(&mut hs);
            }
            for piece in &r.pieces {
                for p in piece.rings.iter().flatten() {
                    p[0].to_bits().hash(&mut hs);
                    p[1].to_bits().hash(&mut hs);
                }
                (piece.form as u8, &piece.paint, piece.height.to_bits(), piece.eaves.to_bits(), piece.down.map(|d| (d[0].to_bits(), d[1].to_bits())), piece.across, piece.overhang.map(f32::to_bits)).hash(&mut hs);
            }
            (light[0].to_bits(), light[1].to_bits()).hash(&mut hs);
            let (w, h) = ((hi[0].min(self.map.width as f32) - lo[0].max(0.0)).max(1.0), (hi[1].min(self.map.height as f32) - lo[1].max(0.0)).max(1.0));
            let ppc = outline_ppc(want, w.max(h), exact);
            ppc.to_bits().hash(&mut hs);
            let key = hs.finish();
            let ready = self.outline_cache.contains_key(&key);
            if !ready && !self.roof_jobs.contains_key(&key) && self.roof_jobs.len() < max_jobs {
                let mut covers: std::collections::HashMap<String, std::sync::Arc<image::RgbaImage>> = std::collections::HashMap::new();
                for piece in &r.pieces {
                    if let Some(img) = self.cover_rgba(&piece.paint) {
                        covers.insert(piece.paint.clone(), img);
                    }
                }
                if covers.len() < r.pieces.iter().map(|k| &k.paint).collect::<std::collections::BTreeSet<_>>().len() {
                    continue;
                }
                let (roof, clip) = (r.clone(), ([0.0, 0.0], [self.map.width as f32, self.map.height as f32]));
                let rx = spawn_roof(move || {
                    let (img, origin) = roofs::render_outline(&roof, &|k: &str| covers.get(k).map(|a| a.as_ref()), ppc, light, clip, &near);
                    vec![roof_picture(img, origin, ppc)]
                });
                self.roof_jobs.insert(key, RoofJob { targets: vec![key], rx });
            }
            pending |= !ready;
            let shown = if self.outline_cache.contains_key(&key) {
                self.outline_last.insert(idx, key);
                Some(key)
            } else {
                self.outline_last.get(&idx).copied()
            };
            drawn.extend(shown);
            if let Some((handle, origin, size)) = shown.and_then(|k| self.outline_cache.get(&k)) {
                let a = self.cam.to_screen(canvas, *origin);
                let b = self.cam.to_screen(canvas, [origin[0] + size[0], origin[1] + size[1]]);
                painter.image(handle.id(), Rect::from_min_max(a, b), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha((alpha * 255.0).round() as u8));
            }
        }
        // Группы врезки: одна отрисовка на группу, картинка — у каждой крыши своя.
        const GROUP: usize = 1 << 24;
        let map_size = [self.map.width as f32, self.map.height as f32];
        for group in &groups {
            let frame = |id: usize| if id < rects.len() { (rects[id].outline()[0], rects[id].outline()[2]) } else { roofs[id - rects.len()].bounds() };
            let (lo, hi) = group.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(l, h), &id| {
                let (a, b) = frame(id);
                ([l[0].min(a[0]), l[1].min(a[1])], [h[0].max(b[0]), h[1].max(b[1])])
            });
            if hi[0] < view.min.x || hi[1] < view.min.y || lo[0] > view.max.x || lo[1] > view.max.y {
                continue;
            }
            let (w, h) = ((hi[0].min(map_size[0]) - lo[0].max(0.0)).max(1.0), (hi[1].min(map_size[1]) - lo[1].max(0.0)).max(1.0));
            let ppc = outline_ppc(want, w.max(h), exact);
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            let mut paints = std::collections::BTreeSet::new();
            for &id in group {
                id.hash(&mut hs);
                if id < rects.len() {
                    let r = &rects[id];
                    (r.a.map(f32::to_bits), r.b.map(f32::to_bits), r.shape as u8, &r.paint, r.turn, r.overhang.to_bits()).hash(&mut hs);
                    paints.insert(r.paint.clone());
                } else {
                    let piece = &roofs[id - rects.len()].pieces[0];
                    for p in piece.rings.iter().flatten() {
                        (p[0].to_bits(), p[1].to_bits()).hash(&mut hs);
                    }
                    (piece.form as u8, &piece.paint, piece.overhang.map(f32::to_bits)).hash(&mut hs);
                    paints.insert(piece.paint.clone());
                }
            }
            (light[0].to_bits(), light[1].to_bits(), ppc.to_bits(), snow).hash(&mut hs);
            let key = hs.finish();
            let slot = |k: usize| key.wrapping_add(k as u64 + 1);
            let ready = self.outline_cache.contains_key(&slot(0));
            if !ready && !self.roof_jobs.contains_key(&key) && self.roof_jobs.len() < max_jobs {
                let covers: std::collections::HashMap<String, std::sync::Arc<image::RgbaImage>> = paints.iter().filter_map(|k| Some((k.clone(), self.cover_rgba(k)?))).collect();
                if covers.len() == paints.len() {
                    // Потоку — только крыши группы, с номерами по своим спискам.
                    let n = rects.len();
                    let sub_rects: Vec<map::Roof> = group.iter().filter(|&&id| id < n).map(|&id| rects[id].clone()).collect();
                    let sub_outline: Vec<map::OutlineRoof> = group.iter().filter(|&&id| id >= n).map(|&id| roofs[id - n].clone()).collect();
                    let (mut ri, mut oi) = (0, sub_rects.len());
                    let sub_group: Vec<usize> = group
                        .iter()
                        .map(|&id| {
                            let at = if id < n { &mut ri } else { &mut oi };
                            *at += 1;
                            *at - 1
                        })
                        .collect();
                    let rx = spawn_roof(move || {
                        roofs::render_joined(&sub_rects, &sub_outline, &sub_group, &|k: &str| covers.get(k).map(|a| a.as_ref()), ppc, light, ([0.0, 0.0], map_size), snow)
                            .into_iter()
                            .map(|(img, origin)| roof_picture(img, origin, ppc))
                            .collect()
                    });
                    self.roof_jobs.insert(key, RoofJob { targets: (0..group.len()).map(slot).collect(), rx });
                }
            }
            pending |= !ready;
            for (k, &id) in group.iter().enumerate() {
                let shown = if self.outline_cache.contains_key(&slot(k)) {
                    self.outline_last.insert(GROUP + id, slot(k));
                    Some(slot(k))
                } else {
                    self.outline_last.get(&(GROUP + id)).copied()
                };
                drawn.extend(shown);
                if let Some((handle, origin, size)) = shown.and_then(|k| self.outline_cache.get(&k)) {
                    let a = self.cam.to_screen(canvas, *origin);
                    let b = self.cam.to_screen(canvas, [origin[0] + size[0], origin[1] + size[1]]);
                    painter.image(handle.id(), Rect::from_min_max(a, b), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::from_white_alpha((alpha * 255.0).round() as u8));
                }
            }
        }
        // Старые картинки (крыша правилась, свет повернулся, масштаб сменился) —
        // выбросить, кроме показанных.
        self.outline_last.retain(|i, _| if *i >= GROUP { grouped.contains(&(*i - GROUP)) } else { *i < roofs.len() && !grouped.contains(&(rects.len() + *i)) });
        if self.outline_cache.len() > (roofs.len() + rects.len()) * 3 + 16 {
            let keep: std::collections::HashSet<u64> = self.outline_last.values().copied().collect();
            self.outline_cache.retain(|k, _| keep.contains(k));
        }
        // Картинки крыш вблизи — по 256 точек на клетку: обойдя так весь город, можно
        // набрать гигабайт. Больше 80 млн точек (320 МБ) — остаётся только видимое.
        let total: usize = self.outline_cache.values().map(|(h, ..)| h.size()[0] * h.size()[1]).sum();
        if total > 80_000_000 {
            self.outline_cache.retain(|k, _| drawn.contains(k));
            self.outline_last.retain(|_, k| drawn.contains(k));
        }
        self.map.outline_roofs = roofs;
        self.outline_pending |= pending;
        if pending {
            painter.ctx().request_repaint();
        }
    }

    pub(super) fn draw_roof(&mut self, painter: &egui::Painter, canvas: Rect, roof: &map::Roof, alpha: f32, light: [f32; 2]) {
        let cam = self.cam;
        let (tex, tile) = match self.textures.get(&roof.paint, false) {
            Some((t, px)) => (Some(t.id()), roofs::tile_cells(px)),
            None => (None, [1.0, 1.0]),
        };
        let (planes, lines) = roof.geometry();
        let flat = roof.shape == map::RoofShape::Flat;
        let tint = |s: f32| {
            let v = (s * alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            // Скат темнит и то, что видно сквозь него: у полупрозрачной крыши тень та же, что у целой.
            let a = ((1.0 - s * (1.0 - alpha)) * 255.0).round().clamp(0.0, 255.0) as u8;
            if tex.is_some() { Color32::from_rgba_premultiplied(v, v, v, a) } else { Color32::from_rgba_premultiplied((v as f32 * 0.6) as u8, (v as f32 * 0.35) as u8, (v as f32 * 0.3) as u8, a) }
        };
        let mut mesh = match tex {
            Some(id) => egui::Mesh::with_texture(id),
            None => egui::Mesh::default(),
        };
        let uv = |p: [f32; 2], o: [f32; 2], along: [f32; 2], down: [f32; 2]| {
            if tex.is_none() {
                return egui::epaint::WHITE_UV;
            }
            let d = [p[0] - o[0], p[1] - o[1]];
            pos2((d[0] * along[0] + d[1] * along[1]) / tile[0], (d[0] * down[0] + d[1] * down[1]) / tile[1])
        };
        for pl in &planes {
            let color = tint(map::roof_shade(pl.down, light, flat));
            let base = mesh.vertices.len() as u32;
            for &p in &pl.poly {
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: uv(p, pl.origin, pl.along, pl.down), color });
            }
            for k in 1..pl.poly.len() as u32 - 1 {
                mesh.add_triangle(base, base + k, base + k + 1);
            }
        }
        // Снег в редакторе — сеткой в четверть клетки поверх скатов, той же
        // россыпью, что в выгрузке; клетки сетки у края — только внутри ската.
        let mut snow_layer = None;
        if self.map.weather == "snow" && !flat {
            let mut snow = egui::Mesh::default();
            let outline = roof.outline();
            let step = 0.25;
            for pl in &planes {
                let s = map::roof_shade(pl.down, light, flat);
                let (lo, hi) = pl.poly.iter().fold(([f32::MAX; 2], [f32::MIN; 2]), |(l, h), p| ([l[0].min(p[0]), l[1].min(p[1])], [h[0].max(p[0]), h[1].max(p[1])]));
                let color = |p: [f32; 2]| {
                    let k = roofs::snow_cover(p, map::convex_depth(&outline, p).max(0.0)) * alpha;
                    let v = (s * 0.45 + 0.55) * 244.0 * k;
                    Color32::from_rgba_premultiplied(v as u8, (v * 1.01).min(255.0) as u8, (v * 1.02).min(255.0) as u8, (k * 255.0) as u8)
                };
                let mut y = (lo[1] / step).floor() * step;
                while y < hi[1] {
                    let mut x = (lo[0] / step).floor() * step;
                    while x < hi[0] {
                        if pl.inside([x + step / 2.0, y + step / 2.0]) > 0.0 {
                            let base = snow.vertices.len() as u32;
                            for p in [[x, y], [x + step, y], [x + step, y + step], [x, y + step]] {
                                snow.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: egui::epaint::WHITE_UV, color: color(p) });
                            }
                            snow.add_triangle(base, base + 1, base + 2);
                            snow.add_triangle(base, base + 2, base + 3);
                        }
                        x += step;
                    }
                    y += step;
                }
            }
            snow_layer = Some(snow);
        }
        // Гребни поверх скатов.
        let dark = Color32::from_rgba_premultiplied(0, 0, 0, (alpha * 150.0) as u8);
        let mut edges = Vec::new();
        for &(a, b) in &lines {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let along = [dx / len, dy / len];
            let n = [-along[1] * roofs::CAP / 2.0, along[0] * roofs::CAP / 2.0];
            let quad = [[a[0] + n[0], a[1] + n[1]], [b[0] + n[0], b[1] + n[1]], [b[0] - n[0], b[1] - n[1]], [a[0] - n[0], a[1] - n[1]]];
            let down = [-along[1], along[0]];
            let color = tint(roofs::CAP_SHADE);
            let base = mesh.vertices.len() as u32;
            for p in quad {
                let mut t = uv(p, a, along, down);
                t.x *= 2.0;
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: t, color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
            edges.push([quad[0], quad[1]]);
            edges.push([quad[3], quad[2]]);
        }
        painter.add(Shape::mesh(mesh));
        if let Some(s) = snow_layer {
            painter.add(Shape::mesh(s));
        }
        // Парапет плоской крыши: тень на кровлю, потом сам отлив.
        if let Some(pk) = roof.parapet() {
            let (lo, hi) = (roof.outline()[0], roof.outline()[2]);
            let gaps = roofs::rail_gaps(lo, hi, &climb_rects(&self.map.props, &self.textures.sources, &mut self.visible_boxes));
            let w = roofs::PARAPET;
            let mut shade = egui::Mesh::default();
            let dim = Color32::from_rgba_premultiplied(0, 0, 0, (alpha * (1.0 - roofs::PARAPET_SHADE) * 255.0) as u8);
            for (poly, ..) in roofs::rail_pieces([lo[0] + w, lo[1] + w], [hi[0] - w, hi[1] - w], roofs::PARAPET_SHADOW, &gaps) {
                let base = shade.vertices.len() as u32;
                for (k, p) in poly.iter().enumerate() {
                    shade.colored_vertex(cam.to_screen(canvas, *p), if k < 2 { dim } else { Color32::TRANSPARENT });
                }
                shade.add_triangle(base, base + 1, base + 2);
                shade.add_triangle(base, base + 2, base + 3);
            }
            painter.add(Shape::mesh(shade));
            if let Some((t, _)) = self.textures.get(pk, false) {
                let mut band = egui::Mesh::with_texture(t.id());
                let color = tint(1.0);
                for (poly, o, along, down) in roofs::rail_pieces(lo, hi, w, &gaps) {
                    let base = band.vertices.len() as u32;
                    for p in poly {
                        let d = [p[0] - o[0], p[1] - o[1]];
                        let uv = pos2((d[0] * along[0] + d[1] * along[1]) / roofs::PARAPET_TILE, (d[0] * down[0] + d[1] * down[1]) / w);
                        band.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
                    }
                    band.add_triangle(base, base + 1, base + 2);
                    band.add_triangle(base, base + 2, base + 3);
                }
                painter.add(Shape::mesh(band));
            }
            // Сливы и воронки — поверх, клеткой с поворотом.
            for (key, at, rot) in roof.fixtures() {
                let Some((t, _)) = self.textures.get(key, false) else { continue };
                let mut item = egui::Mesh::with_texture(t.id());
                let (s, c) = rot.to_radians().sin_cos();
                let color = Color32::from_white_alpha((alpha * 255.0) as u8);
                for (l, uv) in [([-0.5f32, -0.5f32], pos2(0.0, 0.0)), ([0.5, -0.5], pos2(1.0, 0.0)), ([0.5, 0.5], pos2(1.0, 1.0)), ([-0.5, 0.5], pos2(0.0, 1.0))] {
                    let p = [at[0] + c * l[0] - s * l[1], at[1] + s * l[0] + c * l[1]];
                    item.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv, color });
                }
                item.add_triangle(0, 1, 2);
                item.add_triangle(0, 2, 3);
                painter.add(Shape::mesh(item));
            }
        }
        for [a, b] in edges {
            painter.line_segment([cam.to_screen(canvas, a), cam.to_screen(canvas, b)], Stroke::new(1.0, dark));
        }
        let out: Vec<Pos2> = roof.outline().iter().map(|&p| cam.to_screen(canvas, p)).collect();
        painter.add(Shape::closed_line(out, Stroke::new(roofs::RIM_PX, Color32::from_rgba_premultiplied(14, 10, 8, (alpha * 230.0) as u8))));
    }

    /// Тень крыши на земле: сдвинутый контур, залитый, с мягким краем.
    /// Тень крыши — только на землю: куда накрывает соседняя крыша (неф и
    /// боковые нефы, башня над средокрестием), тени нет, иначе она запекается
    /// в пол под крышей и читается как ступень.
    pub(super) fn draw_roof_shadow(&self, painter: &egui::Painter, canvas: Rect, roof: &map::Roof, sh: &map::Shadows, all: &[map::Roof], me: usize) {
        let cam = self.cam;
        // От стен, а не от края свеса: без крыши на земле не остаётся резкой полосы.
        let (lo, hi) = ([roof.a[0].min(roof.b[0]), roof.a[1].min(roof.b[1])], [roof.a[0].max(roof.b[0]), roof.a[1].max(roof.b[1])]);
        let pts: Vec<[f32; 2]> = roof.outline().into_iter().map(|p| [p[0].clamp(lo[0], hi[0]), p[1].clamp(lo[1], hi[1])]).collect();
        let n = pts.len();
        let c = [pts.iter().map(|p| p[0]).sum::<f32>() / n as f32, pts.iter().map(|p| p[1]).sum::<f32>() / n as f32];
        let o = roofs::shadow_offset(sh);
        let len = (o[0] * o[0] + o[1] * o[1]).sqrt();
        if len < 1e-4 {
            return;
        }
        let along = [o[0] / len * sh.blur, o[1] / len * sh.blur];
        let unit = [o[0] / len, o[1] / len];
        let under = |p: [f32; 2]| {
            let q = [p[0] + unit[0] * 0.15, p[1] + unit[1] * 0.15];
            all.iter().enumerate().any(|(j, r)| {
                j != me && q[0] > r.a[0].min(r.b[0]) && q[0] < r.a[0].max(r.b[0]) && q[1] > r.a[1].min(r.b[1]) && q[1] < r.a[1].max(r.b[1])
            })
        };
        let shift = |p: [f32; 2], d: [f32; 2]| [p[0] + d[0], p[1] + d[1]];
        let dark = Color32::from_black_alpha((sh.opacity * 255.0).round() as u8);
        let far = Color32::from_black_alpha((sh.opacity * (1.0 - sh.fade) * 255.0).round() as u8);
        let mut mesh = egui::Mesh::default();
        let mut quad = |a: [([f32; 2], Color32); 4]| {
            let base = mesh.vertices.len() as u32;
            for (p, color) in a {
                mesh.vertices.push(egui::epaint::Vertex { pos: cam.to_screen(canvas, p), uv: egui::epaint::WHITE_UV, color });
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        };
        // Сторона в тени: тень идёт полосой от края крыши; на границе с освещённой
        // стороной тает вбок.
        let shaded = |k: usize| {
            let (p, q) = (pts[k], pts[(k + 1) % n]);
            let m = [(p[0] + q[0]) / 2.0 - c[0], (p[1] + q[1]) / 2.0 - c[1]];
            let (nx, ny) = (q[1] - p[1], p[0] - q[0]);
            let side = if nx * m[0] + ny * m[1] >= 0.0 { 1.0 } else { -1.0 };
            (nx * o[0] + ny * o[1]) * side > 0.0
        };
        for k in 0..n {
            let (p0, q0) = (pts[k], pts[(k + 1) % n]);
            if shaded(k) {
                // Кусками по четверти клетки: под соседней крышей кусок пропускается.
                let l = ((q0[0] - p0[0]).powi(2) + (q0[1] - p0[1]).powi(2)).sqrt();
                let parts = ((l / 0.25).ceil() as usize).max(1);
                for s in 0..parts {
                    let lerp = |t: f32| [p0[0] + (q0[0] - p0[0]) * t, p0[1] + (q0[1] - p0[1]) * t];
                    let (p, q) = (lerp(s as f32 / parts as f32), lerp((s + 1) as f32 / parts as f32));
                    // Ни у края, ни у дальнего конца тени — не под соседней крышей.
                    let mid = lerp((s as f32 + 0.5) / parts as f32);
                    if under(mid) || under(shift(mid, o)) {
                        continue;
                    }
                    quad([(p, dark), (q, dark), (shift(q, o), far), (shift(p, o), far)]);
                    quad([(shift(p, o), far), (shift(q, o), far), (shift(shift(q, o), along), Color32::TRANSPARENT), (shift(shift(p, o), along), Color32::TRANSPARENT)]);
                }
            }
            let q = q0;
            if shaded(k) != shaded((k + 1) % n) && !under(q) {
                let mut s = [-along[1], along[0]];
                if s[0] * (q[0] - c[0]) + s[1] * (q[1] - c[1]) < 0.0 {
                    s = [-s[0], -s[1]];
                }
                if under(shift(shift(q, s), [o[0] * 0.5, o[1] * 0.5])) || under(shift(q, o)) {
                    continue;
                }
                quad([(q, dark), (shift(q, o), far), (shift(shift(q, o), s), Color32::TRANSPARENT), (shift(q, s), Color32::TRANSPARENT)]);
            }
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// Рамки лестниц и трапов этажа по видимой части картинки (кэш `seen` — по
/// ассету): у края плоской крыши они размыкают перила.
pub(super) fn climb_rects(props: &[map::Prop], sources: &library::Sources, seen: &mut std::collections::HashMap<String, Option<[f32; 4]>>) -> Vec<[f32; 4]> {
    props
        .iter()
        .filter(|p| map::climb_by_name(&p.asset))
        .map(|p| {
            let v = *seen.entry(p.asset.clone()).or_insert_with(|| library::visible_box(sources, &p.asset));
            map::prop_rect(p, v)
        })
        .collect()
}

/// Картинка крыши, которую рисует фоновый поток: куда в кэш крыш её положить (у группы
/// врезки — по месту на крышу) и откуда её взять.
pub(super) struct RoofJob {
    targets: Vec<u64>,
    rx: std::sync::mpsc::Receiver<Vec<RoofPicture>>,
}

/// Готовая для видеокарты картинка крыши, левый верхний угол и размер в клетках.
type RoofPicture = (egui::ColorImage, [f32; 2], [f32; 2]);

fn roof_picture(img: image::RgbaImage, origin: [f32; 2], ppc: f32) -> RoofPicture {
    let size = [img.width() as f32 / ppc, img.height() as f32 / ppc];
    (egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()), origin, size)
}

/// Отрисовка крыши в своём потоке; в браузере потоков нет — сразу.
fn spawn_roof(f: impl FnOnce() -> Vec<RoofPicture> + Send + 'static) -> std::sync::mpsc::Receiver<Vec<RoofPicture>> {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    #[cfg(target_arch = "wasm32")]
    let _ = tx.send(f());
    rx
}

/// Точек на клетку у картинки крыши по контуру: `want` — точек экрана на клетку,
/// `span` — длинная сторона крыши в клетках. В окне — ближайшая ступень не мельче
/// экрана (ступенями, чтобы колесо мыши не перерисовывало крышу на каждый щелчок),
/// не больше родных 256 и картинки в 4096 точек. Для снимка (`exact`) — точка в
/// точку с картинкой снимка.
pub(super) fn outline_ppc(want: f32, span: f32, exact: bool) -> f32 {
    let span = span.max(1.0);
    if exact {
        return want.min(8192.0 / span).max(4.0);
    }
    const STEPS: [f32; 10] = [8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0, 96.0, 128.0, 192.0];
    STEPS.iter().copied().find(|s| *s >= want * 0.85).unwrap_or(library::PX_PER_CELL).min(4096.0 / span).max(4.0)
}

/// Сетка куска `(pi, pj)` со стороной `tsz` точек маски в координатах карты: то же, что рисовалось прежде по
/// всему окну, — те же четырёхугольники, сила, кривые и узор, — но только для квадратов,
/// что начинаются в этом куске (у первого куска ряда — и с края карты, с шага −`step`).
#[allow(clippy::too_many_arguments)]
fn ground_piece(m: &Mask, above: &[Coat], clip: Option<&Mask>, step: usize, detail: usize, textured: bool, tile: [f32; 2], tint: [u8; 3], rot: f32, pi: usize, pj: usize, tsz: usize) -> (Vec<egui::epaint::Vertex>, Vec<u32>) {
    let res = RES as f32;
    let s = step as isize;
    let first = |p: usize| if p == 0 { -s } else { (p * tsz) as isize };
    let (i0, i1) = (first(pi), ((pi + 1) * tsz).min(m.w) as isize);
    let (j0, j1) = (first(pj), ((pj + 1) * tsz).min(m.h) as isize);
    let share = |mask: &Mask, i: isize, j: isize| {
        let v = mask.get(i, j);
        clip.map_or(v, |c| v.min(c.get(i, j)))
    };
    // За краем карты берётся крайняя точка: иначе слой тает к границе и у неё виден фон.
    let (last_i, last_j) = (m.w as isize - 1, m.h as isize - 1);
    let (map_w, map_h) = (m.w as f32 / res, m.h as f32 / res);
    let value = |i: isize, j: isize| {
        let (i, j) = (i.clamp(0, last_i), j.clamp(0, last_j));
        let v = share(m, i, j);
        if v == 0 {
            return 0;
        }
        map::coat_alpha(v, above.iter().map(|c| share(&c.mask, i, j) as u32).sum())
    };
    // Узор повёрнут на `rot` по часовой: точка карты берёт плитку повёрнутой назад.
    let (rs, rc) = rot.to_radians().sin_cos();
    // Вершина в точке маски с дробными номерами (fx, fy) и силой `alpha`; место — в клетках.
    let vertex = |fx: f32, fy: f32, alpha: u8| {
        let w = [((fx + 0.5) / res).clamp(0.0, map_w), ((fy + 0.5) / res).clamp(0.0, map_h)];
        let color = if textured { Color32::from_white_alpha(alpha) } else { Color32::from_rgba_unmultiplied(tint[0], tint[1], tint[2], alpha) };
        // Цветной слой берёт белую точку атласа egui, текстурный — плитку.
        let uv = if textured { pos2((w[0] * rc + w[1] * rs) / tile[0], (w[1] * rc - w[0] * rs) / tile[1]) } else { egui::epaint::WHITE_UV };
        egui::epaint::Vertex { pos: pos2(w[0], w[1]), uv, color }
    };
    let weights: Vec<[f32; 4]> = (0..=detail)
        .map(|k| {
            let t = k as f32 / detail as f32;
            let (t2, t3) = (t * t, t * t * t);
            [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
        })
        .collect();
    let mut mesh = egui::Mesh::default();
    let mut j = j0;
    while j < j1 {
        let mut i = i0;
        while i < i1 {
            let corners = [(i, j), (i + s, j), (i + s, j + s), (i, j + s)];
            let a = corners.map(|(x, y)| value(x, y));
            if a != [0; 4] {
                let mut near = [[0f32; 4]; 4];
                if detail > 1 {
                    for (dy, row) in near.iter_mut().enumerate() {
                        for (dx, v) in row.iter_mut().enumerate() {
                            *v = value(i + dx as isize - 1, j + dy as isize - 1) as f32;
                        }
                    }
                }
                // Дробится не только сам край, но и соседи: кривая берёт точки на шаг
                // дальше, и стык мелкого с крупным иначе дал бы щель.
                let flat = near.iter().flatten().all(|&v| v == near[0][0]);
                let base = mesh.vertices.len() as u32;
                if detail > 1 && !flat {
                    let n = detail;
                    for (ky, wy) in weights.iter().enumerate() {
                        for (kx, wx) in weights.iter().enumerate() {
                            let sum: f32 = (0..4).map(|r| wy[r] * (0..4).map(|c| wx[c] * near[r][c]).sum::<f32>()).sum();
                            let alpha = sum.round().clamp(0.0, 255.0) as u8;
                            mesh.vertices.push(vertex(i as f32 + kx as f32 / n as f32, j as f32 + ky as f32 / n as f32, alpha));
                        }
                    }
                    let row = (n + 1) as u32;
                    for ky in 0..n as u32 {
                        for kx in 0..n as u32 {
                            let q = base + ky * row + kx;
                            mesh.add_triangle(q, q + 1, q + row + 1);
                            mesh.add_triangle(q, q + row + 1, q + row);
                        }
                    }
                } else if detail > 1 {
                    // Ровный кусок вблизи — веер с теми же точками по краям, что у
                    // дроблёного соседа: стык без Т-образных узлов и щелей. Раньше
                    // сплошной кусок раздувался на полпикселя, и полоса раздува
                    // ложилась на соседей светлой сеткой по шагу маски (зимний лес
                    // в Foundry, 2026-11-04).
                    let n = detail as f32;
                    let (x0, y0, d) = (i as f32, j as f32, s as f32);
                    mesh.vertices.push(vertex(x0 + d / 2.0, y0 + d / 2.0, a[0]));
                    let ring: Vec<(f32, f32)> = (0..detail)
                        .map(|k| (k as f32 / n, 0.0))
                        .chain((0..detail).map(|k| (1.0, k as f32 / n)))
                        .chain((0..detail).map(|k| (1.0 - k as f32 / n, 1.0)))
                        .chain((0..detail).map(|k| (0.0, 1.0 - k as f32 / n)))
                        .collect();
                    for &(u, v) in &ring {
                        mesh.vertices.push(vertex(x0 + u * d, y0 + v * d, a[0]));
                    }
                    let count = ring.len() as u32;
                    for k in 0..count {
                        mesh.add_triangle(base, base + 1 + k, base + 1 + (k + 1) % count);
                    }
                } else {
                    for ((x, y), alpha) in corners.into_iter().zip(a) {
                        mesh.vertices.push(vertex(x as f32, y as f32, alpha));
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                    mesh.add_triangle(base, base + 2, base + 3);
                }
            }
            i += s;
        }
        j += s;
    }
    (mesh.vertices, mesh.indices)
}
