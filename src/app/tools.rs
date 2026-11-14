//! Команды ИИ в окне редактора ([`crate::control`], [`crate::mcp`]): что делает
//! каждый инструмент MCP. Правка — как рукой: точка отмены, видна сразу.

use super::*;
use crate::map::{Glow, Label, Path as Road};
use serde_json::{Value, json};
use std::sync::mpsc::Sender;

/// Ответ, которого ждут несколько кадров: картинка, место по координатам, выгрузка.
pub enum Waiting {
    /// Картинка области снимается плитками ([`App::start_picture`]): вернуть её ИИ
    /// (`max_px` — сторона) и/или записать в файл.
    Picture { reply: Sender<Result<Value, String>>, area: [f32; 4], max_px: u32, path: Option<PathBuf>, image: bool, grid: bool, light: Option<Box<(crate::lighting::Scene, crate::lighting::Time)>> },
    Place { reply: Sender<Result<Value, String>> },
    Export { reply: Sender<Result<Value, String>> },
}

/// Готовый ответ или ожидание.
enum Out {
    Done(Result<Value, String>),
    Wait(Waiting),
}

type Res = Result<Value, String>;

fn f(a: &Value, k: &str) -> Option<f32> {
    a.get(k).and_then(|v| v.as_f64()).map(|v| v as f32)
}

fn need(a: &Value, k: &str) -> Result<f32, String> {
    f(a, k).ok_or(format!("нужно число «{k}»"))
}

fn area_of(a: &Value) -> Option<[f32; 4]> {
    let v = a.get("area")?.as_array()?;
    let n: Vec<f32> = v.iter().filter_map(|x| x.as_f64().map(|x| x as f32)).collect();
    (n.len() == 4).then(|| [n[0].min(n[2]), n[1].min(n[3]), n[0].max(n[2]), n[1].max(n[3])])
}

fn inside(area: Option<[f32; 4]>, p: [f32; 2]) -> bool {
    area.is_none_or(|r| p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3])
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Floor => "floor",
        Kind::Wall => "wall",
        Kind::Door => "door",
        Kind::Object => "object",
        Kind::Roof => "roof",
    }
}

impl App {
    /// Кадр управления: досмотреть ожидание, иначе взять следующую команду.
    pub(super) fn control_step(&mut self, ctx: &egui::Context) {
        if self.control.is_none() {
            return;
        }
        if self.control_wait.is_some() {
            self.control_wait_step(ctx);
            return;
        }
        let Some(req) = self.control.as_ref().and_then(|c| c.rx.try_recv().ok()) else { return };
        // Окно «Новая карта» при запуске из MCP ждёт человека — ИИ его снимает.
        self.new_map = None;
        match self.run_tool(&req.tool, &req.args) {
            Out::Done(r) => {
                let _ = req.reply.send(r);
            }
            Out::Wait(w) => {
                self.control_wait = Some(match w {
                    Waiting::Picture { area, max_px, path, image, grid, light, .. } => Waiting::Picture { reply: req.reply, area, max_px, path, image, grid, light },
                    Waiting::Place { .. } => Waiting::Place { reply: req.reply },
                    Waiting::Export { .. } => Waiting::Export { reply: req.reply },
                });
            }
        }
        ctx.request_repaint();
    }

    /// Окно свёрнуто: `ui` не идёт, команды ИИ — отсюда. Правки карты делаются и так;
    /// снимку, выгрузке и поиску места нужны кадры — окно разворачивается, не отнимая
    /// фокус, и потом сворачивается обратно ([`Self::hide_after_ai`]). Раньше вызов
    /// при свёрнутом окне ждал, пока окно не развернут руками.
    pub(super) fn control_while_minimized(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().minimized) != Some(true) {
            return;
        }
        self.control_step(ctx);
        if (self.control_wait.is_some() || self.export.is_some()) && self.shown_for_ai.is_none() {
            self.shown_for_ai = Some(ctx.input(|i| i.time));
            self.show_window(ctx, false);
        }
    }

    /// Окно развернули для снимка — свернуть, когда полторы секунды нечего ждать:
    /// снимки подряд не мигают окном. Человек сам открыл окно — не трогать.
    pub(super) fn hide_after_ai(&mut self, ctx: &egui::Context) {
        let Some(since) = self.shown_for_ai else { return };
        let now = ctx.input(|i| i.time);
        if self.control_wait.is_some() || self.export.is_some() {
            self.shown_for_ai = Some(now);
        } else if ctx.input(|i| i.viewport().focused) == Some(true) {
            self.shown_for_ai = None;
        } else if now - since >= 1.5 {
            self.shown_for_ai = None;
            self.show_window(ctx, true);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }
    }

    /// Ошибка, что правка положила в окно ошибки, — в ответ ИИ, а не на экран.
    fn take_error(&mut self) -> Result<(), String> {
        match self.error.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn control_wait_step(&mut self, ctx: &egui::Context) {
        ctx.request_repaint();
        match self.control_wait.take() {
            Some(Waiting::Picture { reply, area, max_px, path, image, grid, light }) => {
                if self.export.is_some() {
                    self.control_wait = Some(Waiting::Picture { reply, area, max_px, path, image, grid, light });
                } else {
                    let r = match self.picture.take() {
                        Some(mut img) => {
                            if let Some(l) = &light {
                                crate::lighting::apply(&mut img, area, &l.0, l.1);
                            }
                            picture_reply(img, area, max_px, path.as_deref(), image, grid)
                        }
                        None => Err("картинка не снялась".into()),
                    };
                    let _ = reply.send(r);
                }
            }
            Some(Waiting::Place { reply }) => {
                if self.place.busy() {
                    self.control_wait = Some(Waiting::Place { reply });
                } else {
                    let r = self.take_error().map(|_| json!({"done": self.notice.clone().unwrap_or_default(), "width": self.map.width, "height": self.map.height}));
                    let _ = reply.send(r);
                }
            }
            Some(Waiting::Export { reply }) => {
                if self.export.is_some() {
                    self.control_wait = Some(Waiting::Export { reply });
                } else {
                    let r = self.take_error().map(|_| json!({"done": self.notice.clone().unwrap_or_default()}));
                    let _ = reply.send(r);
                }
            }
            None => {}
        }
    }

    /// Ключ ассета: точный ключ, а нет — первый по имени. Ключ в цвете (`tint:…:ключ`)
    /// годится, если годится сама картинка.
    fn asset_key(&self, name: &str) -> Result<String, String> {
        if let (Some(_), inner) = library::untint(name) {
            self.asset_key(inner)?;
            return Ok(name.to_owned());
        }
        if self.library.assets.iter().any(|a| a.key == name) || name.starts_with(library::BUILTIN) || name.starts_with("set:") {
            return Ok(name.to_owned());
        }
        let low = name.to_lowercase();
        self.library
            .assets
            .iter()
            .find(|a| a.name.to_lowercase() == low)
            .or_else(|| self.library.assets.iter().find(|a| a.name.to_lowercase().contains(&low)))
            .map(|a| a.key.clone())
            .ok_or(format!("нет ассета «{name}» — найдите ключ через find_assets"))
    }

    fn run_tool(&mut self, tool: &str, a: &Value) -> Out {
        let r = match tool {
            "map_info" => Ok(self.tool_info()),
            "describe" => Ok(self.tool_describe(a)),
            "find_assets" => self.tool_find(a),
            "add_object" => self.tool_add_object(a),
            "edit_object" => self.tool_edit_object(a),
            "remove" => self.tool_remove(a),
            "paint" => self.tool_paint(a, false),
            "erase" => self.tool_paint(a, true),
            "add_wall" => self.tool_add_wall(a),
            "add_door" => self.tool_add_door(a),
            "edit_door" => self.tool_edit_door(a),
            "preview_assets" => self.tool_preview_assets(a),
            "batch" => self.tool_batch(a),
            "add_label" => self.tool_add_label(a),
            "edit_label" => self.tool_edit_label(a),
            "add_path" => self.tool_add_path(a),
            "set_wall_style" => self.tool_wall_style(a),
            "edit_roof" => self.tool_edit_roof(a),
            "add_roof" => self.tool_add_roof(a),
            "set_scene" => self.tool_set_scene(a),
            "add_level" => self.tool_add_level(a),
            "switch_level" => self.tool_switch_level(a),
            "grow_map" => self.tool_grow_map(a),
            "reload_assets" => {
                if let Some(dir) = self.settings.textures_dir.clone() {
                    self.library.set_folder(&dir);
                }
                let count = |k: Kind| self.library.assets.iter().filter(|x| x.kind == k).count();
                Ok(json!({"folder": self.settings.textures_dir, "objects": count(Kind::Object), "floors": count(Kind::Floor), "walls": count(Kind::Wall), "doors": count(Kind::Door), "library_loading": self.library.loading()}))
            }
            "undo" => {
                let n = a["steps"].as_u64().unwrap_or(1).clamp(1, 50);
                for _ in 0..n {
                    self.undo();
                }
                Ok(json!({"undone": n}))
            }
            "view" | "render_picture" => {
                let area = area_of(a).unwrap_or([0.0, 0.0, self.map.width as f32, self.map.height as f32]);
                let longest = (area[2] - area[0]).max(area[3] - area[1]).max(1.0);
                let (max_px, px, path, image) = if tool == "view" {
                    let max_px = a["max_px"].as_u64().unwrap_or(1024).clamp(128, 2048) as u32;
                    (max_px, (max_px as f32 / longest).floor().max(2.0) as u32, None, true)
                } else {
                    let Some(path) = a["path"].as_str().filter(|p| !p.is_empty()) else { return Out::Done(Err("нужен path — куда записать картинку".into())) };
                    let px = a["px"].as_u64().unwrap_or(70).clamp(10, 300) as u32;
                    (u32::MAX, px, Some(PathBuf::from(path)), false)
                };
                if self.export.is_some() {
                    return Out::Done(Err("идёт выгрузка — подождите".into()));
                }
                let roofs = match a["roofs"].as_str().unwrap_or("solid") {
                    "ghost" => 0.35,
                    "hidden" => 0.0,
                    _ => 1.0,
                };
                let time = match crate::lighting::Time::parse(a["lighting"].as_str().unwrap_or("day")) {
                    Ok(t) => t,
                    Err(e) => return Out::Done(Err(e)),
                };
                // Вечер и ночь: огни, стены и окна — те же, что уйдут в Foundry.
                let light = (time != crate::lighting::Time::Day).then(|| {
                    let lay = crate::export::layout(&self.map, &self.map.room_edges());
                    let covers = if roofs >= 1.0 { self.roof_outlines() } else { Vec::new() };
                    Box::new((crate::lighting::Scene::new(&lay, self.map.rooms.clone(), covers), time))
                });
                self.start_picture(area, px, roofs);
                let grid = a["grid"].as_bool().unwrap_or(false);
                return Out::Wait(Waiting::Picture { reply: dummy(), area, max_px, path, image, grid, light });
            }
            "generate" => self.tool_generate(a),
            "build_plan" => self.tool_plan(a),
            "terrain_from_place" => return self.tool_place(a),
            "new_map" => {
                let (w, h) = (a["width"].as_u64().unwrap_or(30) as u32, a["height"].as_u64().unwrap_or(20) as u32);
                self.new_map(w, h);
                Ok(json!({"width": self.map.width, "height": self.map.height}))
            }
            "open_map" => {
                let path = PathBuf::from(a["path"].as_str().unwrap_or_default());
                if self.open_path(&path) { Ok(self.tool_info()) } else { Err(String::new()) }
            }
            "save_map" => {
                let path = a["path"].as_str().map(PathBuf::from).or_else(|| self.path.clone());
                match path {
                    Some(p) if self.save_to(&p) => Ok(json!({"saved": p})),
                    Some(_) => Err(String::new()),
                    None => Err("у карты ещё нет файла — укажите path".into()),
                }
            }
            "export_foundry" => {
                let dir = PathBuf::from(a["dir"].as_str().unwrap_or_default());
                if dir.as_os_str().is_empty() {
                    Err("нужна папка dir".into())
                } else {
                    let px = a["px"].as_u64().unwrap_or(EXPORT_PX as u64).clamp(50, 400) as u32;
                    let name = self.name();
                    self.start_export(dir, name, px, false);
                    return Out::Wait(Waiting::Export { reply: dummy() });
                }
            }
            other => Err(format!("нет инструмента {other}")),
        };
        // Ошибка, что правка положила в окно, — в ответ; пустая — та же ошибка из окна.
        let r = match (r, self.error.take()) {
            (Err(e), Some(w)) if e.is_empty() => Err(w),
            (Err(e), _) => Err(e),
            (Ok(_), Some(w)) => Err(w),
            (Ok(v), None) => Ok(v),
        };
        Out::Done(r)
    }

    fn tool_info(&self) -> Value {
        let m = &self.map;
        json!({
            "width": m.width, "height": m.height,
            "name": self.name(),
            "file": self.path,
            "unsaved": self.dirty,
            "level": self.level, "levels": self.levels.len(),
            "level_names": (0..self.levels.len()).map(|k| self.level_name(k)).collect::<Vec<_>>(),
            "seed": m.seed,
            "scene": {"ambience": m.ambience, "particles": m.particles, "weather": m.weather, "current": m.current, "drift": m.drift},
            "objects": m.props.len(), "walls": m.walls.len(), "doors": m.portals.len(),
            "labels": m.labels.len(), "paths": m.paths.len(), "roofs": m.roofs.len() + m.outline_roofs.len(),
            "materials": m.materials.iter().enumerate().map(|(i, x)| {
                let mut j = json!({"index": i, "name": x.name, "texture": match &x.paint { Paint::Texture(t) => Some(t.clone()), Paint::Color(_) => None }});
                if x.rot != 0.0 {
                    j["angle"] = json!(x.rot);
                }
                j
            }).collect::<Vec<_>>(),
            "cell": "5 ft (1.524 m); x right, y down, origin top-left",
        })
    }

    fn tool_describe(&self, a: &Value) -> Value {
        let want = |s: &str| a["include"].as_array().is_none_or(|l| l.iter().any(|x| x.as_str() == Some(s)));
        let area = area_of(a);
        let limit = a["limit"].as_u64().unwrap_or(200) as usize;
        let m = &self.map;
        let mut out = serde_json::Map::new();
        let r2 = |v: f32| (v * 100.0).round() / 100.0;
        if want("objects") {
            let list: Vec<Value> = m
                .props
                .iter()
                .enumerate()
                .filter(|(_, p)| inside(area, p.pos))
                .take(limit)
                .map(|(i, p)| json!({"index": i, "asset": p.asset, "name": library::label(&p.asset), "x": r2(p.pos[0]), "y": r2(p.pos[1]), "w": r2(p.size[0] * p.scale), "h": r2(p.size[1] * p.scale), "rotation": p.rot, "layer": p.layer(), "light": glow_name(p.light()), "shadow": p.casts_shadow(), "tint": p.tint, "recolor": library::untint(&p.asset).0}))
                .collect();
            out.insert("objects".into(), json!(list));
        }
        if want("walls") {
            let list: Vec<Value> = m.walls.iter().enumerate().filter(|(_, w)| w.points.iter().any(|&p| inside(area, p))).take(limit).map(|(i, w)| json!({"index": i, "points": w.points, "closed": w.closed})).collect();
            out.insert("walls".into(), json!(list));
            out.insert("room_walls".into(), json!("walls along painted rooms are automatic and not listed; change them by painting/erasing rooms"));
        }
        if want("doors") {
            let list: Vec<Value> = m.portals.iter().enumerate().filter(|(_, d)| inside(area, d.center())).take(limit).map(|(i, d)| door_json(i, d)).collect();
            out.insert("doors".into(), json!(list));
        }
        if want("labels") {
            let list: Vec<Value> = m.labels.iter().enumerate().filter(|(_, l)| inside(area, l.pos)).take(limit).map(|(i, l)| json!({"index": i, "text": l.text, "x": l.pos[0], "y": l.pos[1], "size": l.size})).collect();
            out.insert("labels".into(), json!(list));
        }
        if want("roofs") {
            let list: Vec<Value> = m
                .roofs
                .iter()
                .enumerate()
                .filter(|(_, r)| inside(area, [(r.a[0] + r.b[0]) / 2.0, (r.a[1] + r.b[1]) / 2.0]))
                .take(limit)
                .map(|(i, r)| json!({"index": i, "area": [r.a[0], r.a[1], r.b[0], r.b[1]], "shape": format!("{:?}", r.shape).to_lowercase(), "paint": r.paint, "name": library::label(&r.paint), "rail": r.rail}))
                .collect();
            let n = m.roofs.len();
            let mut list = list;
            list.extend(
                m.outline_roofs
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.pieces.first().and_then(|p| p.rings.first()).is_some_and(|ring| ring.iter().any(|&q| inside(area, q))))
                    .take(limit.saturating_sub(list.len()))
                    .map(|(i, r)| {
                        let p = &r.pieces[0];
                        json!({"index": n + i, "points": p.rings[0].iter().map(|q| [r2(q[0]), r2(q[1])]).collect::<Vec<_>>(), "shape": form_name(p.form), "paint": p.paint, "name": library::label(&p.paint), "pieces": r.pieces.len()})
                    }),
            );
            out.insert("roofs".into(), json!(list));
        }
        if want("paths") {
            let list: Vec<Value> = m
                .paths
                .iter()
                .enumerate()
                .filter(|(_, p)| p.points.iter().any(|&q| inside(area, q)))
                .take(limit)
                .map(|(i, p)| json!({"index": i, "material": m.materials.get(p.material).map(|x| x.name.clone()), "width": p.width, "points": p.points}))
                .collect();
            out.insert("paths".into(), json!(list));
        }
        Value::Object(out)
    }

    /// Поиск по словам с ранжированием ([`library::Asset::score`]): то же имя — первым,
    /// слово целиком выше куска чужого слова (`table` ≠ `tablet`), ломаное и особые
    /// ветки паков — ниже, если их не назвали; `exclude` — убрать совсем.
    fn tool_find(&mut self, a: &Value) -> Res {
        let words = library::name_tokens(a["query"].as_str().unwrap_or_default());
        let exclude = library::name_tokens(a["exclude"].as_str().unwrap_or_default());
        let kind = a["kind"].as_str();
        let limit = a["limit"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
        let mut ranked: Vec<(f32, usize)> = self
            .library
            .assets
            .iter()
            .enumerate()
            .filter(|(_, x)| kind.is_none_or(|k| kind_name(x.kind) == k) && (exclude.is_empty() || !x.mentions(&exclude)))
            .filter_map(|(i, x)| Some((x.score(&words)?, i)))
            .collect();
        ranked.sort_by(|p, q| q.0.total_cmp(&p.0).then(p.1.cmp(&q.1)));
        let total = ranked.len();
        let hits: Vec<(String, String, String, Kind)> = ranked
            .into_iter()
            .take(limit)
            .map(|(_, i)| {
                let x = &self.library.assets[i];
                (x.key.clone(), x.name.clone(), x.folder.clone(), x.kind)
            })
            .collect();
        let list: Vec<Value> = hits
            .into_iter()
            .map(|(key, name, folder, kind)| {
                let size = (kind == Kind::Object).then(|| self.asset_size(&key));
                json!({"key": key, "name": name, "folder": folder, "kind": kind_name(kind), "size_cells": size})
            })
            .collect();
        Ok(json!({"assets": list, "found": total, "library_loading": self.library.loading()}))
    }

    fn tool_add_object(&mut self, a: &Value) -> Res {
        let mut asset = self.asset_key(a["asset"].as_str().unwrap_or_default())?;
        if let Some(Some(c)) = color_of(a, "recolor")? {
            asset = library::tint_key(c, &asset);
        }
        let (x, y) = (need(a, "x")?, need(a, "y")?);
        let size = self.asset_size(&asset);
        let light = glow_of(a)?.flatten();
        let layer = layer_of(a)?.flatten();
        let shadow = shadow_of(a)?.flatten();
        let tint = color_of(a, "tint")?.flatten();
        let scale = f(a, "scale").unwrap_or(1.0).clamp(0.05, 20.0);
        let (pos, rot, wall) = self.facing(a, &asset, size, scale, [x, y], 0.0)?;
        self.checkpoint();
        self.map.props.push(Prop { asset, pos, size, rot, scale, shadow, light, canopy: None, layer, tint, open: None });
        let i = self.map.props.len() - 1;
        self.sel = Some(Sel::Prop(i));
        let p = &self.map.props[i];
        Ok(json!({"index": i, "x": r3(p.pos[0]), "y": r3(p.pos[1]), "rotation": r3(p.rot), "size_cells": size, "light": glow_name(p.light()), "shadow": p.casts_shadow(), "tint": p.tint, "recolor": library::untint(&p.asset).0, "wall": wall}))
    }

    fn tool_edit_object(&mut self, a: &Value) -> Res {
        let i = a["index"].as_u64().ok_or("нужен index")? as usize;
        if i >= self.map.props.len() {
            return Err(format!("объекта {i} нет: их {}", self.map.props.len()));
        }
        let asset = match a["asset"].as_str() {
            Some(n) => Some(self.asset_key(n)?),
            None => None,
        };
        let size = asset.as_deref().map(|k| self.asset_size(k));
        let light = glow_of(a)?;
        let layer = layer_of(a)?;
        let shadow = shadow_of(a)?;
        let tint = color_of(a, "tint")?;
        let recolor = color_of(a, "recolor")?;
        let now = &self.map.props[i];
        let pos = [f(a, "x").unwrap_or(now.pos[0]), f(a, "y").unwrap_or(now.pos[1])];
        let scale = f(a, "scale").map_or(now.scale, |s| s.clamp(0.05, 20.0));
        let (key, sz, rot0) = (asset.clone().unwrap_or_else(|| now.asset.clone()), size.unwrap_or(now.size), now.rot);
        let (pos, rot, wall) = self.facing(a, &key, sz, scale, pos, rot0)?;
        self.checkpoint();
        let p = &mut self.map.props[i];
        p.pos = pos;
        p.rot = rot;
        if let Some(g) = light {
            p.light = g;
        }
        if let Some(s) = shadow {
            p.shadow = s;
        }
        if let Some(t) = tint {
            p.tint = t;
        }
        if let Some(l) = layer {
            p.layer = l;
        }
        p.scale = scale;
        if let (Some(k), Some(sz)) = (asset, size) {
            p.asset = k;
            p.size = sz;
        }
        match recolor {
            Some(Some(c)) => p.asset = library::tint_key(c, &p.asset),
            Some(None) => p.asset = library::untint(&p.asset).1.to_owned(),
            None => {}
        }
        self.sel = Some(Sel::Prop(i));
        Ok(json!({"index": i, "x": r3(p.pos[0]), "y": r3(p.pos[1]), "rotation": r3(p.rot), "scale": p.scale, "asset": p.asset, "light": glow_name(p.light()), "shadow": p.casts_shadow(), "tint": p.tint, "recolor": library::untint(&p.asset).0, "wall": wall}))
    }

    /// Где встанет предмет и как повернётся: `look_at` — лицом (`front`) к точке,
    /// `against_wall` — спиной к ближайшей стене, вплотную по видимой части
    /// картинки. Явный `rotation` главнее обоих. Третье — стена, к которой прижат.
    fn facing(&mut self, a: &Value, key: &str, size: [f32; 2], scale: f32, pos: [f32; 2], rot_now: f32) -> Result<([f32; 2], f32, Option<Value>), String> {
        let front = a["front"].as_str().unwrap_or("bottom");
        let base = front_base(front)?;
        let look = point_at(a, "look_at");
        let explicit = f(a, "rotation");
        if a["against_wall"].as_bool() != Some(true) {
            let rot = match (explicit, look) {
                (Some(r), _) => r,
                (None, Some(t)) => look_rotation(pos, t, base).unwrap_or(rot_now),
                (None, None) => rot_now,
            };
            return Ok((pos, rot, None));
        }
        let (seg, half) = self.nearest_wall_face(pos, WALL_REACH).ok_or(format!("в {WALL_REACH} клетках от точки нет стены"))?;
        let rooms = &self.map.rooms;
        let inside = |q: [f32; 2]| rooms.get((q[0] * mask::RES as f32).floor() as isize, (q[1] * mask::RES as f32).floor() as isize) > 127;
        let (p, n) = wall_side(seg, pos, half, look, inside);
        let rot = explicit.unwrap_or_else(|| look_rotation(p, [p[0] + n[0], p[1] + n[1]], base).unwrap_or(rot_now));
        let vb = library::visible_box(&self.textures.sources, key).unwrap_or([-0.5, -0.5, 0.5, 0.5]);
        let back = back_extent(front, vb, size) * scale;
        let d = half + back + 0.01;
        Ok(([p[0] + n[0] * d, p[1] + n[1] * d], rot, Some(json!({"a": seg.0, "b": seg.1, "normal": [r3(n[0]), r3(n[1])]}))))
    }

    /// Контуры крыш этажа: прямоугольные — с выносом, по контуру — внешние кольца.
    fn roof_outlines(&self) -> Vec<Vec<[f32; 2]>> {
        let mut out: Vec<Vec<[f32; 2]>> = self
            .map
            .roofs
            .iter()
            .map(|r| {
                let o = r.overhang;
                let (x0, y0, x1, y1) = (r.a[0].min(r.b[0]) - o, r.a[1].min(r.b[1]) - o, r.a[0].max(r.b[0]) + o, r.a[1].max(r.b[1]) + o);
                vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
            })
            .collect();
        for r in &self.map.outline_roofs {
            out.extend(r.pieces.iter().filter_map(|p| p.rings.first().cloned()));
        }
        out
    }

    /// Ближайшая к точке стена — свободная или комнаты — и половина её толщины.
    fn nearest_wall_face(&mut self, p: [f32; 2], reach: f32) -> Option<(Segment, f32)> {
        let edges = self.room_edges().to_vec();
        let mut faces: Vec<(Segment, f32)> = edges.iter().map(|s| (*s, self.map.edge_look(map::edge_key(s)).width / 2.0)).collect();
        for w in &self.map.walls {
            let half = w.style.as_ref().unwrap_or(&self.map.wall_style).width / 2.0;
            faces.extend(w.segments().map(|s| (s, half)));
        }
        faces.into_iter().map(|f| (map::segment_distance(p, f.0.0, f.0.1), f)).filter(|(d, _)| *d <= reach).min_by(|x, y| x.0.total_cmp(&y.0)).map(|(_, f)| f)
    }

    fn tool_remove(&mut self, a: &Value) -> Res {
        let mut idx: Vec<usize> = a["indices"].as_array().map(|l| l.iter().filter_map(|x| x.as_u64().map(|v| v as usize)).collect()).unwrap_or_default();
        idx.sort_unstable();
        idx.dedup();
        let what = a["what"].as_str().unwrap_or_default();
        let len = match what {
            "objects" => self.map.props.len(),
            "walls" => self.map.walls.len(),
            "doors" => self.map.portals.len(),
            "labels" => self.map.labels.len(),
            "paths" => self.map.paths.len(),
            "roofs" => self.map.roofs.len() + self.map.outline_roofs.len(),
            _ => return Err("what: objects, walls, doors, labels, paths или roofs".into()),
        };
        if let Some(&bad) = idx.iter().find(|&&i| i >= len) {
            return Err(format!("{what}: номера {bad} нет, их {len}"));
        }
        self.checkpoint();
        for &i in idx.iter().rev() {
            match what {
                "objects" => drop(self.map.props.remove(i)),
                "walls" => drop(self.map.walls.remove(i)),
                "doors" => drop(self.map.portals.remove(i)),
                "labels" => drop(self.map.labels.remove(i)),
                "roofs" if i >= self.map.roofs.len() => drop(self.map.outline_roofs.remove(i - self.map.roofs.len())),
                "roofs" => drop(self.map.roofs.remove(i)),
                _ => drop(self.map.paths.remove(i)),
            }
        }
        self.sel = None;
        self.edges_dirty = true;
        Ok(json!({"removed": idx.len()}))
    }

    /// Пол по прямоугольнику, овалу или многоугольнику. Края — с шагом маски, четверть
    /// клетки: комната [4, 3, 14, 11] стенами встаёт ровно на 4, 14, 3 и 11, а пол
    /// до 9,5 — под стену `add_wall` на 9,5. `feather` (земля и стирание) — мягкий
    /// край: доля пути от края к середине, на которой сила спадает.
    fn tool_paint(&mut self, a: &Value, erase: bool) -> Res {
        let (mw, mh) = (self.map.rooms.w, self.map.rooms.h);
        let shape = a["shape"].as_str().unwrap_or("rect");
        let mut weights = match shape {
            "poly" => {
                let pts = points_of(a);
                if pts.len() < 3 {
                    return Err("shape poly: нужны points — хотя бы три точки [x, y]".into());
                }
                mask::poly(mw, mh, &pts)
            }
            "rect" | "oval" => {
                let r = area_of(a).ok_or("нужна area: [x0, y0, x1, y1]")?;
                if shape == "oval" { mask::oval_f(mw, mh, r) } else { mask::rect_f(mw, mh, r) }
            }
            other => return Err(format!("shape «{other}»: rect, oval или poly")),
        };
        if weights.is_empty() {
            return Err("область пустая или вне карты".into());
        }
        let room = !erase && a["target"].as_str() == Some("room");
        let feather = f(a, "feather").unwrap_or(0.0).clamp(0.0, 1.0);
        if feather > 0.0 && room {
            return Err("feather — для земли (target terrain) и стирания: у комнаты край — её стены".into());
        }
        mask::soften(mw, mh, &mut weights, feather);
        let cells = (weights.iter().map(|w| w.1).sum::<f32>() / (mask::RES * mask::RES) as f32 * 100.0).round() / 100.0;
        if erase {
            self.checkpoint();
            self.map.erase(&weights);
            self.edges_dirty = true;
            return Ok(json!({"erased_cells": cells, "feather": feather}));
        }
        let m = self.material_turned(&a["material"], f(a, "angle"))?;
        self.checkpoint();
        self.map.paint(if room { Layer::Rooms } else { Layer::Floor }, m, &weights);
        self.edges_dirty = true;
        Ok(json!({"material": m, "cells": cells, "target": if room { "room" } else { "terrain" }, "feather": feather, "angle": self.map.materials[m].rot}))
    }

    fn tool_add_wall(&mut self, a: &Value) -> Res {
        let points = points_of(a);
        if points.len() < 2 {
            return Err("нужно хотя бы две точки".into());
        }
        self.checkpoint();
        let low = a["low"].as_bool().unwrap_or(false);
        self.map.walls.push(Wall { points, closed: a["closed"].as_bool().unwrap_or(false), bends: Vec::new(), low, style: None });
        self.edges_dirty = true;
        Ok(json!({"index": self.map.walls.len() - 1}))
    }

    /// Картинки проёма по ответу ИИ: `asset` — створка, окно или подоконник, `frame` —
    /// косяк; `""` — без картинки. Не названо — как в окне редактора, но по виду
    /// проёма: окну — подоконник, двери — створка.
    fn door_pictures(&self, a: &Value, kind: PortalKind) -> Result<(Option<String>, Option<String>), String> {
        let pick = |k: &str| -> Result<Option<String>, String> {
            match a[k].as_str() {
                Some("") => Ok(None),
                Some(n) => Ok(Some(self.asset_key(n)?)),
                None => Err(String::new()),
            }
        };
        let window_like = |k: &str| k.contains("window") || k.to_lowercase().contains("sill");
        let asset = match pick("asset") {
            Ok(v) => v,
            Err(e) if !e.is_empty() => return Err(e),
            Err(_) => match kind {
                PortalKind::Opening | PortalKind::Secret => None,
                PortalKind::Window => Some(self.door_asset.clone().filter(|k| window_like(k)).unwrap_or_else(|| format!("{}window_stone", library::BUILTIN))),
                _ => Some(self.door_asset.clone().filter(|k| !window_like(k)).unwrap_or_else(|| library::DEFAULT_DOOR.into())),
            },
        };
        let frame = match pick("frame") {
            Ok(v) => v,
            Err(e) if !e.is_empty() => return Err(e),
            Err(_) => if kind.hidden() { None } else { self.door_frame.clone() },
        };
        Ok((asset, frame))
    }

    /// Проём по отрезку `a`–`b`: оба конца ложатся на ближайшую стену (её линию в
    /// пределах отрезка стены), без привязки к сетке.
    fn door_on_wall(&mut self, a: [f32; 2], b: [f32; 2]) -> Result<([f32; 2], [f32; 2]), String> {
        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let (s0, s1) = self
            .all_wall_segments()
            .into_iter()
            .map(|s| (s, map::segment_distance(mid, s.0, s.1)))
            .filter(|(_, d)| *d <= DOOR_REACH)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .ok_or("у середины отрезка нет стены")?
            .0;
        let (dx, dy) = (s1[0] - s0[0], s1[1] - s0[1]);
        let len2 = (dx * dx + dy * dy).max(1e-9);
        let on = |p: [f32; 2]| {
            let t = (((p[0] - s0[0]) * dx + (p[1] - s0[1]) * dy) / len2).clamp(0.0, 1.0);
            [s0[0] + dx * t, s0[1] + dy * t]
        };
        let (mut p, mut q) = (on(a), on(b));
        if ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() < 0.2 {
            return Err("проём короче 0,2 клетки — концы сошлись на стене".into());
        }
        if (q[0], q[1]) < (p[0], p[1]) {
            std::mem::swap(&mut p, &mut q);
        }
        Ok((p, q))
    }

    fn tool_add_door(&mut self, a: &Value) -> Res {
        let kind = portal_kind(a["kind"].as_str().unwrap_or("door"))?;
        let (asset, frame) = self.door_pictures(a, kind)?;
        let i = if let (Some(pa), Some(pb)) = (point_at(a, "a"), point_at(a, "b")) {
            let (pa, pb) = self.door_on_wall(pa, pb)?;
            let w = ((pb[0] - pa[0]).powi(2) + (pb[1] - pa[1]).powi(2)).sqrt();
            let frame = frame.map(|k| library::frame_for_width(&k, w));
            let portal = Portal { a: pa, b: pb, kind, asset, frame };
            let clash = self.map.portals.iter().position(|q| map::segment_distance(q.center(), portal.a, portal.b) < 0.01);
            self.checkpoint();
            let i = match clash {
                Some(i) => {
                    self.map.portals[i] = portal;
                    i
                }
                None => {
                    self.map.portals.push(portal);
                    self.map.portals.len() - 1
                }
            };
            self.sel = Some(Sel::Portal(i));
            i
        } else {
            let p = [need(a, "x").map_err(|_| "нужны x, y — точка у стены — или отрезок a, b".to_owned())?, need(a, "y")?];
            let width = f(a, "width").unwrap_or(if kind == PortalKind::DoubleDoor { 2.0 } else { self.door_width }).clamp(0.5, 8.0);
            // Окно редактора помнит свой выбор: подменить на время и вернуть.
            let was = (self.door_kind, self.door_width, self.door_asset.clone(), self.door_frame.clone());
            (self.door_kind, self.door_width, self.door_asset, self.door_frame) = (kind, width, asset, frame);
            let i = self.place_portal(p);
            (self.door_kind, self.door_width, self.door_asset, self.door_frame) = was;
            i.ok_or("рядом с точкой нет стены")?
        };
        Ok(door_json(i, &self.map.portals[i]))
    }

    /// Проём по номеру: вид, картинки створки и косяка, место по отрезку.
    fn tool_edit_door(&mut self, a: &Value) -> Res {
        let i = a["index"].as_u64().ok_or("нужен index")? as usize;
        if i >= self.map.portals.len() {
            return Err(format!("проёма {i} нет: их {}", self.map.portals.len()));
        }
        let kind = match a["kind"].as_str() {
            Some(k) => portal_kind(k)?,
            None => self.map.portals[i].kind,
        };
        let pick = |k: &str| -> Result<Option<Option<String>>, String> {
            match a[k].as_str() {
                None => Ok(None),
                Some("") => Ok(Some(None)),
                Some(n) => Ok(Some(Some(self.asset_key(n)?))),
            }
        };
        let (asset, frame) = (pick("asset")?, pick("frame")?);
        let ends = match (point_at(a, "a"), point_at(a, "b")) {
            (Some(pa), Some(pb)) => Some(self.door_on_wall(pa, pb)?),
            (None, None) => None,
            _ => return Err("отрезок — оба конца: a и b".into()),
        };
        self.checkpoint();
        let d = &mut self.map.portals[i];
        d.kind = kind;
        if let Some((pa, pb)) = ends {
            (d.a, d.b) = (pa, pb);
        }
        if let Some(v) = asset {
            d.asset = v;
        }
        if let Some(v) = frame {
            d.frame = v;
        }
        if matches!(kind, PortalKind::Opening) {
            d.asset = None;
        }
        let w = ((d.b[0] - d.a[0]).powi(2) + (d.b[1] - d.a[1]).powi(2)).sqrt();
        d.frame = d.frame.take().map(|k| library::frame_for_width(&k, w));
        self.sel = Some(Sel::Portal(i));
        self.edges_dirty = true;
        Ok(door_json(i, &self.map.portals[i]))
    }

    /// Лист ассетов: куда смотрит картинка при повороте 0, размер в клетках и где
    /// в ней непрозрачное. Картинка ИИ — в ответ, подробности — JSON.
    fn tool_preview_assets(&mut self, a: &Value) -> Res {
        let keys: Vec<String> = a["assets"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        if keys.is_empty() || keys.len() > 16 {
            return Err("assets: от 1 до 16 ключей из find_assets".into());
        }
        let tile = a["tile_px"].as_u64().unwrap_or(192).clamp(64, 512) as u32;
        let mut items = Vec::new();
        let mut info = Vec::new();
        for (n, name) in keys.iter().enumerate() {
            let key = self.asset_key(name)?;
            let size = self.asset_size(&key);
            let img = library::load_rgba(&self.textures.sources, &key).ok_or(format!("{name}: картинка не читается (пак не подключён?)"))?;
            let bbox = crate::sheet::alpha_bbox(&img);
            let r2 = |v: f32| (v * 1000.0).round() / 1000.0;
            info.push(json!({
                "tile": n + 1, "key": key, "name": library::label(&key), "size_cells": size, "pixels": [img.width(), img.height()],
                "alpha_bbox": bbox.map(|b| b.map(r2)),
                // То же в клетках от середины объекта при повороте 0 и масштабе 1.
                "visible_cells": bbox.map(|b| [r2((b[0] - 0.5) * size[0]), r2((b[1] - 0.5) * size[1]), r2((b[2] - 0.5) * size[0]), r2((b[3] - 0.5) * size[1])]),
            }));
            items.push((img, size));
        }
        let sheet = crate::sheet::contact_sheet(&items, tile);
        let mut png = Vec::new();
        sheet.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        use base64::Engine;
        Ok(json!({
            "assets": info,
            "legend": "each tile shows the asset at rotation 0, number top-left; TOP, RIGHT, BOTTOM, LEFT label the sides of the picture and are the values of `front` in add_object / edit_object (most FA furniture faces BOTTOM: its back is drawn at the top); faint lines are the cells it covers",
            "image_png_base64": base64::engine::general_purpose::STANDARD.encode(&png),
        }))
    }

    /// Пачка правок за один вызов: одна точка отмены на каждый тронутый этаж. Ошибка —
    /// вся пачка откатывается и ничего не меняется.
    fn tool_batch(&mut self, a: &Value) -> Res {
        const NOT_IN_BATCH: [&str; 15] = ["batch", "view", "render_picture", "export_foundry", "terrain_from_place", "generate", "build_plan", "new_map", "open_map", "save_map", "grow_map", "add_level", "undo", "reload_assets", "preview_assets"];
        let ops = a["ops"].as_array().ok_or("нужен ops: [{\"tool\": …, \"arguments\": {…}}]")?;
        if ops.is_empty() || ops.len() > 1000 {
            return Err("ops: от 1 до 1000 правок".into());
        }
        let mut list = Vec::new();
        for (k, op) in ops.iter().enumerate() {
            let tool = op["tool"].as_str().ok_or(format!("ops[{k}]: нет tool"))?;
            if NOT_IN_BATCH.contains(&tool) {
                return Err(format!("ops[{k}]: {tool} в batch нельзя — вызовите отдельно"));
            }
            list.push((tool.to_owned(), op.get("arguments").or_else(|| op.get("args")).cloned().unwrap_or(json!({}))));
        }
        let start = self.level;
        self.batch = Some(Vec::new());
        let mut results = Vec::new();
        let mut failed = None;
        for (k, (tool, args)) in list.iter().enumerate() {
            match self.run_tool(tool, args) {
                Out::Done(Ok(v)) => results.push(v),
                Out::Done(Err(e)) => {
                    failed = Some(format!("ops[{k}] {tool}: {e}"));
                    break;
                }
                Out::Wait(_) => {
                    failed = Some(format!("ops[{k}] {tool}: ждёт кадров — в batch нельзя"));
                    break;
                }
            }
        }
        let touched = self.batch.take().unwrap_or_default();
        if let Some(e) = failed {
            for _ in 0..touched.len() {
                self.undo();
            }
            self.redo.clear();
            self.switch_level(start);
            return Err(format!("{e} — пачка откатана, карта как была"));
        }
        Ok(json!({"done": results.len(), "results": results, "undo_steps": touched.len()}))
    }

    fn tool_add_label(&mut self, a: &Value) -> Res {
        let text = a["text"].as_str().unwrap_or_default().trim().to_owned();
        if text.is_empty() {
            return Err("пустая надпись".into());
        }
        let mut l = Label::new(text, [need(a, "x")?, need(a, "y")?]);
        if let Some(s) = f(a, "size") {
            l.size = s.clamp(0.2, 20.0);
        }
        self.checkpoint();
        self.map.labels.push(l);
        Ok(json!({"index": self.map.labels.len() - 1}))
    }

    fn tool_edit_label(&mut self, a: &Value) -> Res {
        let i = a["index"].as_u64().ok_or("нужен index")? as usize;
        if i >= self.map.labels.len() {
            return Err(format!("подписи {i} нет: их {}", self.map.labels.len()));
        }
        let text = a["text"].as_str().map(|t| t.trim().to_owned());
        if text.as_deref() == Some("") {
            return Err("пустая надпись — удалите её через remove".into());
        }
        self.checkpoint();
        let l = &mut self.map.labels[i];
        if let Some(t) = text {
            l.text = t;
        }
        if let Some(x) = f(a, "x") {
            l.pos[0] = x;
        }
        if let Some(y) = f(a, "y") {
            l.pos[1] = y;
        }
        if let Some(s) = f(a, "size") {
            l.size = s.clamp(0.2, 20.0);
        }
        if let Some(r) = f(a, "rotation") {
            l.rot = r;
        }
        if let Some(g) = a["gm_only"].as_bool() {
            l.gm = g;
        }
        Ok(json!({"index": i, "text": l.text, "x": l.pos[0], "y": l.pos[1], "size": l.size, "rotation": l.rot, "gm_only": l.gm}))
    }

    fn levels_reply(&self) -> Value {
        json!({"level": self.level, "levels": (0..self.levels.len()).map(|k| self.level_name(k)).collect::<Vec<_>>()})
    }

    /// Новый этаж выше или ниже текущего; редактор переходит на него.
    fn tool_add_level(&mut self, a: &Value) -> Res {
        self.add_level(!a["below"].as_bool().unwrap_or(false));
        if let Some(n) = a["name"].as_str() {
            self.map.name = n.to_owned();
        }
        Ok(self.levels_reply())
    }

    /// Поля к карте по краям, на всех этажах; нарисованное сдвигается. Отменить
    /// нельзя: история хранит этаж, а размер у этажей общий.
    fn tool_grow_map(&mut self, a: &Value) -> Res {
        let side = |k: &str| a[k].as_u64().unwrap_or(0).min(400) as u32;
        let (l, t, r, b) = (side("left"), side("top"), side("right"), side("bottom"));
        if l + t + r + b == 0 {
            return Err("нужно хоть одно поле: left, top, right или bottom (клетки)".into());
        }
        let (w, h) = (self.map.width + l + r, self.map.height + t + b);
        if w > 600 || h > 600 {
            return Err(format!("карта {w}×{h} — больше 600 клеток по стороне"));
        }
        self.map.grow(l, t, r, b);
        for (k, m) in self.levels.iter_mut().enumerate() {
            if k != self.level || m.width + l + r == w {
                m.grow(l, t, r, b);
            }
        }
        self.undo.clear();
        self.redo.clear();
        self.touch();
        self.refresh_below();
        Ok(json!({"width": w, "height": h, "levels": self.levels.len().max(1)}))
    }

    fn tool_switch_level(&mut self, a: &Value) -> Res {
        if let Some(k) = a["level"].as_u64() {
            let k = k as usize;
            if k >= self.levels.len() {
                return Err(format!("этажей {}: номер от 0 до {}", self.levels.len(), self.levels.len() - 1));
            }
            self.switch_level(k);
        }
        if let Some(n) = a["name"].as_str() {
            self.checkpoint();
            self.map.name = n.to_owned();
        }
        Ok(self.levels_reply())
    }

    /// Живое сцены для «Wild Canvas»: звук места, что летит от крон, погода, течение.
    fn tool_set_scene(&mut self, a: &Value) -> Res {
        use crate::plan::{PARTICLES, PLACES, WEATHERS};
        let pick = |key: &str, list: &[(&str, &str)]| -> Result<Option<String>, String> {
            match a[key].as_str() {
                None => Ok(None),
                Some(v) if list.iter().any(|(k, _)| *k == v) => Ok(Some(v.to_owned())),
                Some(v) => Err(format!("{key} «{v}»: одно из {}", list.iter().map(|(k, _)| format!("«{k}»")).collect::<Vec<_>>().join(", "))),
            }
        };
        let (ambience, particles, weather) = (pick("ambience", &PLACES)?, pick("particles", &PARTICLES)?, pick("weather", &WEATHERS)?);
        let current = f(a, "current").map(|v| v.clamp(0.0, 3.0));
        let drift = f(a, "drift").map(|v| v.rem_euclid(360.0));
        self.checkpoint();
        let m = &mut self.map;
        if let Some(v) = ambience {
            m.ambience = v;
        }
        if let Some(v) = particles {
            m.particles = v;
        }
        if let Some(v) = weather {
            m.weather = v;
        }
        if let Some(v) = current {
            m.current = v;
        }
        if let Some(v) = drift {
            m.drift = v;
        }
        Ok(json!({"ambience": m.ambience, "particles": m.particles, "weather": m.weather, "current": m.current, "drift": m.drift}))
    }

    fn tool_add_roof(&mut self, a: &Value) -> Res {
        if a.get("points").is_some_and(|v| !v.is_null()) {
            return self.tool_add_outline_roof(a);
        }
        let r = area_of(a).ok_or("нужна area: [x0, y0, x1, y1] по стенам дома — или points: контур многоугольником")?;
        if r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0 {
            return Err("крыша меньше клетки".into());
        }
        let paint = self.asset_key(a["paint"].as_str().unwrap_or("builtin:roof_metal"))?;
        let shape = roof_shape(a["shape"].as_str().unwrap_or("gable"))?;
        let rail = match a["rail"].as_str() {
            None | Some("") => None,
            Some(k) => Some(self.asset_key(k)?),
        };
        self.checkpoint();
        let mut roof = map::Roof::new([r[0], r[1]], [r[2], r[3]], shape, paint);
        roof.rail = rail;
        if let Some(t) = a["turn"].as_u64() {
            roof.turn = (t % 4) as u8;
        }
        if let Some(o) = f(a, "overhang") {
            roof.overhang = o.clamp(0.0, 2.0);
        }
        self.map.roofs.push(roof);
        Ok(json!({"index": self.map.roofs.len() - 1}))
    }

    /// Крыша по контуру: башня, ротонда, эркер. Контур — по стенам, вынос — наружу
    /// по нормалям рёбер. Шатёр — грани к вершине, вальма — скаты ко всем сторонам.
    fn tool_add_outline_roof(&mut self, a: &Value) -> Res {
        let pts = points_of(a);
        if pts.len() < 3 {
            return Err("points: хотя бы три точки [x, y] по стенам".into());
        }
        let paint = self.asset_key(a["paint"].as_str().unwrap_or("builtin:roof_metal"))?;
        let form = roof_form(a["shape"].as_str().unwrap_or("tent"))?;
        let ring = grow_ring(&pts, f(a, "overhang").unwrap_or(0.25).clamp(0.0, 2.0));
        let mut piece = map::RoofPiece { rings: vec![ring], form, paint, height: 0.0, eaves: 0.0, down: None, across: false };
        piece.fit_form();
        for _ in 0..a["turn"].as_u64().unwrap_or(0) % 4 {
            piece.turn();
        }
        self.checkpoint();
        self.map.outline_roofs.push(map::OutlineRoof { pieces: vec![piece] });
        let k = self.map.outline_roofs.len() - 1;
        self.sel = Some(Sel::Outline(k, 0));
        Ok(json!({"index": self.map.roofs.len() + k, "shape": form_name(form), "points": self.map.outline_roofs[k].pieces[0].rings[0]}))
    }

    /// Крыша по контуру по номеру из describe (после прямоугольных).
    fn tool_edit_outline_roof(&mut self, a: &Value, k: usize) -> Res {
        let paint = match a["paint"].as_str() {
            Some(p) => Some(self.asset_key(p)?),
            None => None,
        };
        let form = match a["shape"].as_str() {
            Some(s) => Some(roof_form(s)?),
            None => None,
        };
        let pts = points_of(a);
        if !pts.is_empty() && pts.len() < 3 {
            return Err("points: хотя бы три точки".into());
        }
        self.checkpoint();
        let p = &mut self.map.outline_roofs[k].pieces[0];
        if let Some(v) = paint {
            p.paint = v;
        }
        if let Some(v) = form {
            p.form = v;
            p.fit_form();
        }
        if pts.len() >= 3 {
            p.rings = vec![grow_ring(&pts, f(a, "overhang").unwrap_or(0.25).clamp(0.0, 2.0))];
            p.down = None;
            p.fit_form();
        }
        for _ in 0..a["turn"].as_u64().unwrap_or(0) % 4 {
            p.turn();
        }
        self.sel = Some(Sel::Outline(k, 0));
        Ok(json!({"index": self.map.roofs.len() + k, "shape": form_name(p.form), "paint": p.paint, "points": p.rings[0]}))
    }

    fn tool_edit_roof(&mut self, a: &Value) -> Res {
        let i = a["index"].as_u64().ok_or("нужен index")? as usize;
        let n = self.map.roofs.len();
        if i >= n && i < n + self.map.outline_roofs.len() {
            return self.tool_edit_outline_roof(a, i - n);
        }
        if i >= n {
            return Err(format!("крыши {i} нет: их {}", n + self.map.outline_roofs.len()));
        }
        let paint = match a["paint"].as_str() {
            Some(k) => Some(self.asset_key(k)?),
            None => None,
        };
        let shape = match a["shape"].as_str() {
            None => None,
            Some(s) => Some(roof_shape(s)?),
        };
        let rail = match a["rail"].as_str() {
            None => None,
            Some("") => Some(None),
            Some(k) => Some(Some(self.asset_key(k)?)),
        };
        self.checkpoint();
        let r = &mut self.map.roofs[i];
        if let Some(v) = rail {
            r.rail = v;
        }
        if let Some(p) = paint {
            r.paint = p;
        }
        if let Some(s) = shape {
            r.shape = s;
        }
        if let Some(t) = a["turn"].as_u64() {
            r.turn = (t % 4) as u8;
        }
        Ok(json!({"index": i, "shape": format!("{:?}", r.shape).to_lowercase(), "paint": r.paint, "turn": r.turn, "rail": r.rail}))
    }

    fn tool_add_path(&mut self, a: &Value) -> Res {
        let points = points_of(a);
        if points.len() < 2 {
            return Err("нужно хотя бы две точки".into());
        }
        let m = self.material_of(&a["material"])?;
        let mut road = Road::template(m);
        road.points = points;
        if let Some(w) = f(a, "width") {
            road.width = w.clamp(0.2, 40.0);
        }
        if let Some(s) = f(a, "soft") {
            road.soft = s.clamp(0.0, 1.0);
        }
        if let Some(r) = f(a, "rough") {
            road.rough = r.clamp(0.0, 1.0);
        }
        if let Some(b) = a["along"].as_bool() {
            road.along = b;
        }
        self.checkpoint();
        self.map.paths.push(road);
        Ok(json!({"index": self.map.paths.len() - 1, "material": m}))
    }

    /// Вид стен: новых (комнаты, которые закрасят дальше, и свободные стены) или,
    /// с `area`, уже стоящих в области. Стоящие при смене вида новых не меняются.
    fn tool_wall_style(&mut self, a: &Value) -> Res {
        let mut st = self.map.wall_style.clone();
        if let Some(k) = a["asset"].as_str() {
            st.paint = Paint::Texture(self.asset_key(k)?);
            if f(a, "width").is_none() && st.width < 0.3 {
                st.width = 0.35;
            }
        }
        if let Some(c) = a["color"].as_array() {
            let c: Vec<u8> = c.iter().filter_map(|v| v.as_u64().map(|v| v.min(255) as u8)).collect();
            if c.len() != 3 {
                return Err("color: [r, g, b]".into());
            }
            st.paint = Paint::Color([c[0], c[1], c[2]]);
        }
        if let Some(w) = f(a, "width") {
            st.width = w.clamp(0.05, 2.0);
        }
        self.checkpoint();
        self.sel = None;
        let edges = self.room_edges().to_vec();
        let mid = |s: &Segment| [(s.0[0] + s.1[0]) / 2.0, (s.0[1] + s.1[1]) / 2.0];
        let r = if let Some(area) = area_of(a) {
            let pick: Vec<Segment> = edges.iter().filter(|s| inside(Some(area), mid(s))).copied().collect();
            self.map.set_edge_look(&pick, &st);
            let mut n = 0;
            for w in self.map.walls.iter_mut().filter(|w| w.points.iter().all(|&p| inside(Some(area), p))) {
                w.style = Some(st.clone());
                n += 1;
            }
            json!({"room_edges": pick.len(), "walls": n})
        } else {
            // Стоящие остаются какими были: стенам комнат без своего вида он
            // записывается, свободным — тоже.
            let old = self.map.wall_style.clone();
            let bare: Vec<Segment> = edges.iter().filter(|s| !self.map.edge_looks.contains_key(&map::edge_key(s))).copied().collect();
            self.map.set_edge_look(&bare, &old);
            for w in self.map.walls.iter_mut().filter(|w| w.style.is_none()) {
                w.style = Some(old.clone());
            }
            self.map.wall_style = st.clone();
            json!({"for": "new walls"})
        };
        self.edges_dirty = true;
        let paint = match &st.paint {
            Paint::Texture(k) => json!(k),
            Paint::Color(c) => json!(c),
        };
        Ok(json!({"style": {"paint": paint, "width": st.width}, "changed": r}))
    }

    /// Материал с узором, повёрнутым на `angle` градусов по часовой: свой номер в
    /// палитре на каждый угол. Без `angle` номер из палитры остаётся со своим
    /// поворотом, а новый ключ ложится ровно.
    fn material_turned(&mut self, v: &Value, angle: Option<f32>) -> Result<usize, String> {
        let m = self.material_of(v)?;
        let Some(angle) = angle else { return Ok(m) };
        let angle = angle.rem_euclid(360.0);
        if (self.map.materials[m].rot - angle).abs() < 1e-3 {
            return Ok(m);
        }
        let mut mat = self.map.materials[m].clone();
        mat.rot = angle;
        self.map.material_index(mat).ok_or("в карте уже 255 материалов".into())
    }

    /// Материал из ответа ИИ: номер в палитре карты (числом или строкой «40») или
    /// ключ пола из библиотеки.
    fn material_of(&mut self, v: &Value) -> Result<usize, String> {
        // Строка из одних цифр — номер, а не имя: иначе «40» искалось бы в именах
        // ассетов и брало первый попавшийся с «40» в имени.
        let index = match v {
            Value::Number(n) => Some(n.as_u64().unwrap_or(u64::MAX)),
            Value::String(s) if !s.trim().is_empty() && s.trim().bytes().all(|b| b.is_ascii_digit()) => Some(s.trim().parse().unwrap_or(u64::MAX)),
            _ => None,
        };
        match index {
            Some(i) => {
                let i = i as usize;
                if i >= self.map.materials.len() {
                    return Err(format!("материала {i} нет"));
                }
                Ok(i)
            }
            None => {
                let key = self.asset_key(v.as_str().unwrap_or_default())?;
                let mat = Material { name: library::label(&key), paint: Paint::Texture(key), vary: false, live: map::Live::Auto, rot: 0.0 };
                self.map.material_index(mat).ok_or("в карте уже 255 материалов".into())
            }
        }
    }

    fn tool_generate(&mut self, a: &Value) -> Res {
        let kind: generate::Kind = serde_json::from_value(a["kind"].clone()).map_err(|_| format!("kind: {}", generate::Kind::ALL.iter().map(|k| serde_json::to_string(k).unwrap_or_default()).collect::<Vec<_>>().join(", ")))?;
        if let (Some(w), Some(h)) = (a["width"].as_u64(), a["height"].as_u64()) {
            self.new_map(w as u32, h as u32);
        }
        let mut o = generate::Options { kind, seed: a["seed"].as_str().map_or_else(generate::new_seed, str::to_owned), library: self.settings.gen_library, ..Default::default() };
        if let Some(d) = f(a, "density") {
            o.density = d.clamp(0.0, 1.0);
        }
        if let Some(r) = a["rooms"].as_u64() {
            o.rooms = r.clamp(1, 60) as u32;
        }
        let n = self.generate(&o);
        Ok(json!({"built": n, "seed": o.seed, "width": self.map.width, "height": self.map.height, "note": self.notice.clone()}))
    }

    fn tool_plan(&mut self, a: &Value) -> Res {
        let plan = Plan::from_json(&a["plan"].to_string())?;
        let r = self.build_plan(&plan, true).ok_or(String::new())?;
        Ok(json!({"built": r.count, "warnings": r.warnings, "width": self.map.width, "height": self.map.height}))
    }

    fn tool_place(&mut self, a: &Value) -> Out {
        let (Some(lat), Some(lon)) = (a["lat"].as_f64(), a["lon"].as_f64()) else { return Out::Done(Err("нужны lat и lon".into())) };
        if self.place.busy() {
            return Out::Done(Err("место уже собирается".into()));
        }
        let cells = [a["width"].as_u64().unwrap_or(40).clamp(8, 256) as u32, a["height"].as_u64().unwrap_or(30).clamp(8, 256) as u32];
        self.place.trees = f(a, "trees").unwrap_or(0.0).clamp(0.0, 1.0);
        self.place.error = None;
        self.place.start = Some((lat, lon, cells));
        Out::Wait(Waiting::Place { reply: dummy() })
    }
}

/// Как далеко от точки ищется стена для `against_wall`, в клетках.
const WALL_REACH: f32 = 2.0;

fn r3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

/// Угол стороны `front` картинки при повороте 0, по часовой от «вправо»: экран, y вниз.
fn front_base(front: &str) -> Result<f32, String> {
    Ok(match front {
        "right" => 0.0,
        "bottom" => 90.0,
        "left" => 180.0,
        "top" => 270.0,
        _ => return Err("front: top, right, bottom или left — сторона картинки в preview_assets".into()),
    })
}

/// Поворот, при котором сторона с углом `base` смотрит из `from` на `to`; точки
/// совпали — `None`.
fn look_rotation(from: [f32; 2], to: [f32; 2], base: f32) -> Option<f32> {
    let (dx, dy) = ((to[0] - from[0]) as f64, (to[1] - from[1]) as f64);
    if dx.hypot(dy) < 1e-6 {
        return None;
    }
    let target = dy.atan2(dx).to_degrees();
    Some((((target - base as f64) % 360.0 + 360.0) % 360.0) as f32)
}

/// Проекция точки на стену и нормаль к ней — в сторону точки. Точка в толще стены
/// (`half` — половина толщины): к `look`, а нет его — внутрь комнаты (`inside`),
/// иначе туда, куда точка чуть сдвинута.
fn wall_side(seg: Segment, p: [f32; 2], half: f32, look: Option<[f32; 2]>, inside: impl Fn([f32; 2]) -> bool) -> ([f32; 2], [f32; 2]) {
    let (a, b) = seg;
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let len = (ex * ex + ey * ey).sqrt().max(1e-6);
    let t = (((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / (len * len)).clamp(0.0, 1.0);
    let q = [a[0] + ex * t, a[1] + ey * t];
    let mut n = [-ey / len, ex / len];
    let side = |v: [f32; 2]| (v[0] - q[0]) * n[0] + (v[1] - q[1]) * n[1];
    let s = side(p);
    let (ahead, behind) = (inside([q[0] + n[0] * (half + 0.3), q[1] + n[1] * (half + 0.3)]), inside([q[0] - n[0] * (half + 0.3), q[1] - n[1] * (half + 0.3)]));
    let flip = if s.abs() > half + 0.05 {
        s < 0.0
    } else if let Some(l) = look.filter(|l| side(*l).abs() > 1e-3) {
        side(l) < 0.0
    } else if ahead != behind {
        behind
    } else {
        s < 0.0
    };
    if flip {
        n = [-n[0], -n[1]];
    }
    (q, n)
}

/// Сколько видимой картинки за её серединой со стороны спины (против `front`), в
/// клетках при масштабе 1; `vb` — видимая рамка в долях от середины (−0,5…0,5).
fn back_extent(front: &str, vb: [f32; 4], size: [f32; 2]) -> f32 {
    match front {
        "top" => vb[3] * size[1],
        "right" => -vb[0] * size[0],
        "left" => vb[2] * size[0],
        _ => -vb[1] * size[1],
    }
    .max(0.0)
}

/// Точки `[[x, y], …]` из поля `points`.
fn points_of(a: &Value) -> Vec<[f32; 2]> {
    a["points"].as_array().map(|l| l.iter().filter_map(|p| Some([p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])).collect()).unwrap_or_default()
}

/// Точка `[x, y]` из поля.
fn point_at(a: &Value, k: &str) -> Option<[f32; 2]> {
    let v = a.get(k)?.as_array()?;
    Some([v.first()?.as_f64()? as f32, v.get(1)?.as_f64()? as f32])
}

fn portal_kind(s: &str) -> Result<PortalKind, String> {
    Ok(match s {
        "door" => PortalKind::Door,
        "double" => PortalKind::DoubleDoor,
        "window" => PortalKind::Window,
        "opening" => PortalKind::Opening,
        "secret" => PortalKind::Secret,
        _ => return Err("kind: door, double, window, opening или secret".into()),
    })
}

fn portal_kind_name(k: PortalKind) -> &'static str {
    match k {
        PortalKind::Door => "door",
        PortalKind::DoubleDoor => "double",
        PortalKind::Window => "window",
        PortalKind::Opening => "opening",
        PortalKind::Secret => "secret",
    }
}

fn door_json(i: usize, d: &Portal) -> Value {
    let w = ((d.b[0] - d.a[0]).powi(2) + (d.b[1] - d.a[1]).powi(2)).sqrt();
    json!({"index": i, "a": d.a, "b": d.b, "width": (w * 100.0).round() / 100.0, "kind": portal_kind_name(d.kind), "asset": d.asset, "frame": d.frame})
}

/// Тень из ответа ИИ: нет поля — не трогать; `"auto"` — по имени (плоское без тени).
fn shadow_of(a: &Value) -> Result<Option<Option<bool>>, String> {
    match a.get("shadow") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(Some(*b))),
        Some(v) if v.as_str() == Some("auto") => Ok(Some(None)),
        _ => Err("shadow: true, false или \"auto\"".into()),
    }
}

/// Цвет из ответа ИИ (`tint` — умножением, `recolor` — перекраска с узором): нет
/// поля — не трогать; `[r, g, b]` — этот; `null` или `[]` — убрать.
fn color_of(a: &Value, k: &str) -> Result<Option<Option<[u8; 3]>>, String> {
    match a.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::Array(l)) if l.is_empty() => Ok(Some(None)),
        Some(Value::Array(l)) if l.len() == 3 => {
            let c: Vec<u8> = l.iter().filter_map(|v| v.as_u64().map(|v| v.min(255) as u8)).collect();
            if c.len() == 3 { Ok(Some(Some([c[0], c[1], c[2]]))) } else { Err(format!("{k}: [r, g, b], числа 0..255")) }
        }
        _ => Err(format!("{k}: [r, g, b], числа 0..255; null — убрать")),
    }
}

/// Свет из ответа ИИ: нет поля — не трогать; `auto` — по имени; иначе свой.
fn glow_of(a: &Value) -> Result<Option<Option<Glow>>, String> {
    let Some(s) = a["light"].as_str() else { return Ok(None) };
    let g = match s {
        "auto" => None,
        "off" => Some(Glow::Off),
        "campfire" => Some(Glow::Campfire),
        "fireplace" => Some(Glow::Fireplace),
        "brazier" => Some(Glow::Brazier),
        "candle" => Some(Glow::Candle),
        "lamp" => Some(Glow::Lamp),
        "arcane" => Some(Glow::Arcane),
        _ => return Err("light: auto, off, campfire, fireplace, brazier, candle, lamp или arcane".into()),
    };
    Ok(Some(g))
}

/// Вид крыши по контуру из ответа ИИ.
fn roof_form(s: &str) -> Result<map::RoofForm, String> {
    Ok(match s {
        "gable" => map::RoofForm::Gabled,
        "hip" => map::RoofForm::Pitched,
        "shed" => map::RoofForm::Shed,
        "flat" => map::RoofForm::Flat,
        "tent" => map::RoofForm::Spire,
        "dome" => map::RoofForm::Dome,
        "vault" => map::RoofForm::Round,
        _ => return Err("shape: tent, hip, gable, shed, flat, dome или vault".into()),
    })
}

fn form_name(f: map::RoofForm) -> &'static str {
    match f {
        map::RoofForm::Gabled => "gable",
        map::RoofForm::Pitched => "hip",
        map::RoofForm::Shed => "shed",
        map::RoofForm::Flat => "flat",
        map::RoofForm::Spire => "tent",
        map::RoofForm::Dome => "dome",
        map::RoofForm::Round => "vault",
        map::RoofForm::Rows => "rows",
    }
}

/// Контур, раздвинутый наружу на `d` клеток по нормалям рёбер (вершины — по
/// биссектрисе, у острых углов вынос не больше тройного).
fn grow_ring(pts: &[[f32; 2]], d: f32) -> Vec<[f32; 2]> {
    let n = pts.len();
    let area: f32 = (0..n).map(|k| pts[k][0] * pts[(k + 1) % n][1] - pts[(k + 1) % n][0] * pts[k][1]).sum();
    // При площади больше нуля (по часовой на экране, y вниз) наружу — правая нормаль.
    let out = |a: [f32; 2], b: [f32; 2]| {
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let l = (ex * ex + ey * ey).sqrt().max(1e-6);
        if area > 0.0 { [ey / l, -ex / l] } else { [-ey / l, ex / l] }
    };
    (0..n)
        .map(|k| {
            let (p, prev, next) = (pts[k], pts[(k + n - 1) % n], pts[(k + 1) % n]);
            let (n1, n2) = (out(prev, p), out(p, next));
            let dot = n1[0] * n2[0] + n1[1] * n2[1];
            let mut m = [(n1[0] + n2[0]) / (1.0 + dot).max(1e-3), (n1[1] + n2[1]) / (1.0 + dot).max(1e-3)];
            let ml = (m[0] * m[0] + m[1] * m[1]).sqrt();
            if ml > 3.0 {
                m = [m[0] * 3.0 / ml, m[1] * 3.0 / ml];
            }
            [p[0] + m[0] * d, p[1] + m[1] * d]
        })
        .collect()
}

fn roof_shape(s: &str) -> Result<map::RoofShape, String> {
    Ok(match s {
        "gable" => map::RoofShape::Gable,
        "hip" => map::RoofShape::Hip,
        "shed" => map::RoofShape::Shed,
        "flat" => map::RoofShape::Flat,
        "tent" => map::RoofShape::Tent,
        _ => return Err("shape: gable, hip, shed, flat или tent".into()),
    })
}

/// Слой из ответа ИИ: нет поля — не трогать; `auto` — по имени; иначе свой.
fn layer_of(a: &Value) -> Result<Option<Option<i16>>, String> {
    let v = &a["layer"];
    if v.is_null() {
        return Ok(None);
    }
    if let Some(n) = v.as_i64() {
        return Ok(Some(Some(n.clamp(0, 1000) as i16)));
    }
    let l = match v.as_str().unwrap_or_default() {
        "auto" => None,
        "floor" => Some(100),
        "low" => Some(200),
        "normal" => Some(map::LAYER_DEFAULT),
        "high" => Some(400),
        "above_walls" => Some(map::ABOVE_WALLS),
        "on_roof" => Some(map::ON_ROOF),
        "above_roofs" => Some(map::ABOVE_ROOFS),
        _ => return Err("layer: auto, floor, low, normal, high, above_walls, on_roof или above_roofs".into()),
    };
    Ok(Some(l))
}

/// Как светит объект — словом для ИИ; не светит — `null`.
fn glow_name(g: Option<Glow>) -> Option<String> {
    g.filter(|g| *g != Glow::Off).map(|g| format!("{g:?}").to_lowercase())
}

/// Готовая картинка: в файл (по расширению — PNG, JPEG или WebP) и/или ИИ в ответ.
fn picture_reply(img: image::RgbImage, area: [f32; 4], max_px: u32, path: Option<&std::path::Path>, with_image: bool, grid: bool) -> Res {
    let mut out = json!({"area": area, "pixels": [img.width(), img.height()], "px_per_cell": img.width() as f32 / (area[2] - area[0]).max(1e-3)});
    // В файл сетка идёт сразу; ИИ — после уменьшения, чтобы цифры читались.
    let img = if grid && !with_image {
        let (g, m) = crate::sheet::ruler(&img, area);
        out["grid_margin_px"] = json!(m);
        g
    } else {
        img
    };
    if let Some(p) = path {
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(dir) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let bytes = match ext.as_str() {
            "webp" => crate::export::webp_bytes(&image::DynamicImage::ImageRgb8(img.clone()).to_rgba8(), 90.0),
            "jpg" | "jpeg" => {
                let mut v = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, 90).encode_image(&img).map_err(|e| e.to_string())?;
                v
            }
            _ => {
                let mut v = Vec::new();
                img.write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).map_err(|e| e.to_string())?;
                v
            }
        };
        std::fs::write(p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        out["saved"] = json!(p);
    }
    if with_image {
        let k = (max_px as f32 / img.width().max(img.height()) as f32).min(1.0);
        let img = if k < 1.0 { image::imageops::resize(&img, ((img.width() as f32 * k) as u32).max(1), ((img.height() as f32 * k) as u32).max(1), image::imageops::FilterType::Triangle) } else { img };
        let ppc = img.width() as f32 / (area[2] - area[0]).max(1e-3);
        let img = if grid {
            let (g, m) = crate::sheet::ruler(&img, area);
            // Точка (px, py) картинки — клетка (area.x0 + (px - left) / ppc, area.y0 + (py - top) / ppc).
            out["grid_margin_px"] = json!(m);
            g
        } else {
            img
        };
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        use base64::Engine;
        out["image_png_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&png));
        out["pixels"] = json!([img.width(), img.height()]);
        out["px_per_cell"] = json!(ppc);
    }
    Ok(out)
}

/// Заглушка: настоящий адресат ответа подставляется в `control_step`.
fn dummy() -> Sender<Result<Value, String>> {
    std::sync::mpsc::channel().0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_at_turns_the_front_side_to_the_point() {
        let rot = |p: [f32; 2], t: [f32; 2], front: &str| look_rotation(p, t, front_base(front).unwrap()).unwrap();
        assert!((rot([5.28, 14.85], [6.05, 15.45], "bottom") - 307.9).abs() < 0.1, "кресло слева от стола");
        assert!((rot([6.82, 14.85], [6.05, 15.45], "bottom") - 52.1).abs() < 0.1, "кресло справа");
        assert!((rot([17.28, 8.5], [16.55, 8.5], "bottom") - 90.0).abs() < 1e-3, "стул");
        assert!(rot([3.02, 8.75], [5.35, 8.75], "right").abs() < 1e-3, "камин");
        assert!((rot([19.25, 3.55], [22.0, 0.8], "top") - 45.0).abs() < 1e-3, "телескоп");
        assert_eq!(look_rotation([1.0, 1.0], [1.0, 1.0], 90.0), None);
        assert!(front_base("front").is_err());
    }

    #[test]
    fn against_a_diagonal_wall_the_back_touches_its_face() {
        // Стена под 45°, шкаф 2×1 с видимой глубиной 0,5 клетки (спинка — верх картинки).
        let seg: Segment = ([0.0, 4.0], [4.0, 0.0]);
        let (q, n) = wall_side(seg, [2.6, 2.6], 0.2, None, |_| false);
        assert!((q[0] - 2.0).abs() < 1e-4 && (q[1] - 2.0).abs() < 1e-4, "{q:?}");
        assert!(n[0] > 0.7 && n[1] > 0.7, "нормаль к точке: {n:?}");
        let back = back_extent("bottom", [-0.5, -0.25, 0.5, 0.25], [2.0, 1.0]);
        assert!((back - 0.25).abs() < 1e-6);
        let rot = look_rotation(q, [q[0] + n[0], q[1] + n[1]], front_base("bottom").unwrap()).unwrap();
        assert!((rot - 315.0).abs() < 1e-3, "фасад от стены: {rot}");
        // Точка прямо на стене: сторона — к look_at, а без него — внутрь комнаты.
        let (_, n2) = wall_side(seg, [2.0, 2.0], 0.2, Some([0.0, 0.0]), |_| false);
        assert!(n2[0] < 0.0 && n2[1] < 0.0);
        let (_, n3) = wall_side(seg, [2.0, 2.0], 0.2, None, |p| p[0] + p[1] < 4.0);
        assert!(n3[0] < 0.0 && n3[1] < 0.0, "внутрь комнаты: {n3:?}");
        // Точка в толще стены, чуть снаружи, — всё равно внутрь комнаты.
        let (_, n4) = wall_side(seg, [2.05, 2.05], 0.2, None, |p| p[0] + p[1] < 4.0);
        assert!(n4[0] < 0.0 && n4[1] < 0.0, "из толщи стены — в комнату: {n4:?}");
        assert_eq!(back_extent("right", [-0.3, -0.5, 0.5, 0.5], [1.0, 1.0]), 0.3);
    }

    #[test]
    fn roof_outline_grows_outwards_either_way_round() {
        let sq = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0]];
        let g = grow_ring(&sq, 0.25);
        assert!((g[0][0] + 0.25).abs() < 1e-5 && (g[0][1] + 0.25).abs() < 1e-5, "{g:?}");
        assert!((g[2][0] - 2.25).abs() < 1e-5 && (g[2][1] - 2.25).abs() < 1e-5);
        let back: Vec<[f32; 2]> = sq.iter().rev().copied().collect();
        let h = grow_ring(&back, 0.25);
        assert!((h[3][0] + 0.25).abs() < 1e-5 && (h[3][1] + 0.25).abs() < 1e-5, "в обратном обходе — тоже наружу: {h:?}");
        // Восьмиугольник: вершины уходят от середины на d / cos(22,5°).
        let oct: Vec<[f32; 2]> = (0..8).map(|k| { let t = std::f32::consts::FRAC_PI_4 * k as f32 + std::f32::consts::FRAC_PI_8; [5.0 + 3.0 * t.cos(), 5.0 + 3.0 * t.sin()] }).collect();
        let o = grow_ring(&oct, 0.3);
        let r = ((o[0][0] - 5.0).powi(2) + (o[0][1] - 5.0).powi(2)).sqrt();
        assert!((r - (3.0 + 0.3 / (std::f32::consts::FRAC_PI_8).cos())).abs() < 1e-3, "{r}");
        assert!(roof_form("tent").is_ok() && roof_form("pyramid").is_err());
    }
}
