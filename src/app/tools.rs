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
    Picture { reply: Sender<Result<Value, String>>, area: [f32; 4], max_px: u32, path: Option<PathBuf>, image: bool },
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
                    Waiting::Picture { area, max_px, path, image, .. } => Waiting::Picture { reply: req.reply, area, max_px, path, image },
                    Waiting::Place { .. } => Waiting::Place { reply: req.reply },
                    Waiting::Export { .. } => Waiting::Export { reply: req.reply },
                });
            }
        }
        ctx.request_repaint();
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
            Some(Waiting::Picture { reply, area, max_px, path, image }) => {
                if self.export.is_some() {
                    self.control_wait = Some(Waiting::Picture { reply, area, max_px, path, image });
                } else {
                    let r = match self.picture.take() {
                        Some(img) => picture_reply(img, area, max_px, path.as_deref(), image),
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

    /// Ключ ассета: точный ключ, а нет — первый по имени.
    fn asset_key(&self, name: &str) -> Result<String, String> {
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
                self.start_picture(area, px, roofs);
                return Out::Wait(Waiting::Picture { reply: dummy(), area, max_px, path, image });
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
            "materials": m.materials.iter().enumerate().map(|(i, x)| json!({"index": i, "name": x.name, "texture": match &x.paint { Paint::Texture(t) => Some(t.clone()), Paint::Color(_) => None }})).collect::<Vec<_>>(),
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
                .map(|(i, p)| json!({"index": i, "asset": p.asset, "name": library::label(&p.asset), "x": r2(p.pos[0]), "y": r2(p.pos[1]), "w": r2(p.size[0] * p.scale), "h": r2(p.size[1] * p.scale), "rotation": p.rot, "layer": p.layer(), "light": glow_name(p.light())}))
                .collect();
            out.insert("objects".into(), json!(list));
        }
        if want("walls") {
            let list: Vec<Value> = m.walls.iter().enumerate().filter(|(_, w)| w.points.iter().any(|&p| inside(area, p))).take(limit).map(|(i, w)| json!({"index": i, "points": w.points, "closed": w.closed})).collect();
            out.insert("walls".into(), json!(list));
            out.insert("room_walls".into(), json!("walls along painted rooms are automatic and not listed; change them by painting/erasing rooms"));
        }
        if want("doors") {
            let list: Vec<Value> = m.portals.iter().enumerate().filter(|(_, d)| inside(area, d.center())).take(limit).map(|(i, d)| json!({"index": i, "a": d.a, "b": d.b, "kind": d.kind.label()})).collect();
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
            out.insert("roofs".into(), json!(list));
            if !m.outline_roofs.is_empty() {
                out.insert("outline_roofs".into(), json!(format!("{} roofs over OpenStreetMap buildings are not listed", m.outline_roofs.len())));
            }
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

    fn tool_find(&mut self, a: &Value) -> Res {
        let words: Vec<String> = a["query"].as_str().unwrap_or_default().to_lowercase().replace('_', " ").split_whitespace().map(str::to_owned).collect();
        let kind = a["kind"].as_str();
        let limit = a["limit"].as_u64().unwrap_or(30).clamp(1, 200) as usize;
        let hits: Vec<(String, String, String, Kind)> = self
            .library
            .assets
            .iter()
            // Встроенные названы по-русски («Плиты»), а ключ — по-английски
            // (`builtin:flagstones`): ищется и по нему.
            .filter(|x| kind.is_none_or(|k| kind_name(x.kind) == k) && (x.matches(&words) || words.iter().all(|w| x.key.to_lowercase().replace('_', " ").contains(w.as_str()))))
            .take(limit)
            .map(|x| (x.key.clone(), x.name.clone(), x.folder.clone(), x.kind))
            .collect();
        let list: Vec<Value> = hits
            .into_iter()
            .map(|(key, name, folder, kind)| {
                let size = (kind == Kind::Object).then(|| self.asset_size(&key));
                json!({"key": key, "name": name, "folder": folder, "kind": kind_name(kind), "size_cells": size})
            })
            .collect();
        Ok(json!({"assets": list, "library_loading": self.library.loading()}))
    }

    fn tool_add_object(&mut self, a: &Value) -> Res {
        let asset = self.asset_key(a["asset"].as_str().unwrap_or_default())?;
        let (x, y) = (need(a, "x")?, need(a, "y")?);
        let size = self.asset_size(&asset);
        let light = glow_of(a)?.flatten();
        let layer = layer_of(a)?.flatten();
        self.checkpoint();
        self.map.props.push(Prop { asset, pos: [x, y], size, rot: f(a, "rotation").unwrap_or(0.0), scale: f(a, "scale").unwrap_or(1.0).clamp(0.05, 20.0), shadow: None, light, canopy: None, layer, tint: None, open: None });
        let i = self.map.props.len() - 1;
        self.sel = Some(Sel::Prop(i));
        Ok(json!({"index": i, "size_cells": size, "light": glow_name(self.map.props[i].light())}))
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
        self.checkpoint();
        let p = &mut self.map.props[i];
        if let Some(g) = light {
            p.light = g;
        }
        if let Some(l) = layer {
            p.layer = l;
        }
        if let Some(x) = f(a, "x") {
            p.pos[0] = x;
        }
        if let Some(y) = f(a, "y") {
            p.pos[1] = y;
        }
        if let Some(r) = f(a, "rotation") {
            p.rot = r;
        }
        if let Some(s) = f(a, "scale") {
            p.scale = s.clamp(0.05, 20.0);
        }
        if let (Some(k), Some(sz)) = (asset, size) {
            p.asset = k;
            p.size = sz;
        }
        self.sel = Some(Sel::Prop(i));
        Ok(json!({"index": i, "x": p.pos[0], "y": p.pos[1], "rotation": p.rot, "scale": p.scale, "asset": p.asset, "light": glow_name(p.light())}))
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
            "roofs" => self.map.roofs.len(),
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
                "roofs" => drop(self.map.roofs.remove(i)),
                _ => drop(self.map.paths.remove(i)),
            }
        }
        self.sel = None;
        self.edges_dirty = true;
        Ok(json!({"removed": idx.len()}))
    }

    fn tool_paint(&mut self, a: &Value, erase: bool) -> Res {
        let r = area_of(a).ok_or("нужна area: [x0, y0, x1, y1]")?;
        let (mw, mh) = (self.map.rooms.w, self.map.rooms.h);
        // Клетки области [x0, x1) × [y0, y1): правый и нижний край не входят, и
        // комната [4, 3, 14, 11] стенами встаёт ровно на 4, 14, 3 и 11.
        let c0 = (r[0].floor() as i32, r[1].floor() as i32);
        let c1 = ((r[2].ceil() as i32 - 1).max(c0.0), (r[3].ceil() as i32 - 1).max(c0.1));
        let weights = if a["shape"].as_str() == Some("oval") { mask::oval(mw, mh, c0, c1) } else { mask::cells(mw, mh, c0, c1) };
        if weights.is_empty() {
            return Err("область вне карты".into());
        }
        if erase {
            self.checkpoint();
            self.map.erase(&weights);
            self.edges_dirty = true;
            return Ok(json!({"erased_cells": weights.len() / (mask::RES * mask::RES)}));
        }
        let m = self.material_of(&a["material"])?;
        self.checkpoint();
        let room = a["target"].as_str() == Some("room");
        self.map.paint(if room { Layer::Rooms } else { Layer::Floor }, m, &weights);
        self.edges_dirty = true;
        Ok(json!({"material": m, "cells": weights.len() / (mask::RES * mask::RES), "target": if room { "room" } else { "terrain" }}))
    }

    fn tool_add_wall(&mut self, a: &Value) -> Res {
        let points: Vec<[f32; 2]> = a["points"].as_array().map(|l| l.iter().filter_map(|p| Some([p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])).collect()).unwrap_or_default();
        if points.len() < 2 {
            return Err("нужно хотя бы две точки".into());
        }
        self.checkpoint();
        let low = a["low"].as_bool().unwrap_or(false);
        self.map.walls.push(Wall { points, closed: a["closed"].as_bool().unwrap_or(false), bends: Vec::new(), low, style: None });
        self.edges_dirty = true;
        Ok(json!({"index": self.map.walls.len() - 1}))
    }

    fn tool_add_door(&mut self, a: &Value) -> Res {
        let p = [need(a, "x")?, need(a, "y")?];
        let kind = match a["kind"].as_str().unwrap_or("door") {
            "double" => PortalKind::DoubleDoor,
            "window" => PortalKind::Window,
            "opening" => PortalKind::Opening,
            "secret" => PortalKind::Secret,
            _ => PortalKind::Door,
        };
        let was = self.door_kind;
        self.door_kind = kind;
        let i = self.place_portal(p);
        self.door_kind = was;
        let i = i.ok_or("рядом с точкой нет стены")?;
        let d = &self.map.portals[i];
        Ok(json!({"index": i, "a": d.a, "b": d.b, "kind": d.kind.label()}))
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
        let r = area_of(a).ok_or("нужна area: [x0, y0, x1, y1] — по стенам дома")?;
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

    fn tool_edit_roof(&mut self, a: &Value) -> Res {
        let i = a["index"].as_u64().ok_or("нужен index")? as usize;
        if i >= self.map.roofs.len() {
            return Err(format!("крыши {i} нет: их {}", self.map.roofs.len()));
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
        let points: Vec<[f32; 2]> = a["points"].as_array().map(|l| l.iter().filter_map(|p| Some([p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])).collect()).unwrap_or_default();
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

    /// Материал из ответа ИИ: номер в палитре карты или ключ пола из библиотеки.
    fn material_of(&mut self, v: &Value) -> Result<usize, String> {
        match v {
            Value::Number(n) => {
                let i = n.as_u64().unwrap_or(u64::MAX) as usize;
                if i >= self.map.materials.len() {
                    return Err(format!("материала {i} нет"));
                }
                Ok(i)
            }
            v => {
                let key = self.asset_key(v.as_str().unwrap_or_default())?;
                let mat = Material { name: library::label(&key), paint: Paint::Texture(key), vary: false, live: map::Live::Auto };
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
fn picture_reply(img: image::RgbImage, area: [f32; 4], max_px: u32, path: Option<&std::path::Path>, with_image: bool) -> Res {
    let mut out = json!({"area": area, "pixels": [img.width(), img.height()], "px_per_cell": img.width() as f32 / (area[2] - area[0]).max(1e-3)});
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
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        use base64::Engine;
        out["image_png_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(&png));
        out["pixels"] = json!([img.width(), img.height()]);
        out["px_per_cell"] = json!(img.width() as f32 / (area[2] - area[0]).max(1e-3));
    }
    Ok(out)
}

/// Заглушка: настоящий адресат ответа подставляется в `control_step`.
fn dummy() -> Sender<Result<Value, String>> {
    std::sync::mpsc::channel().0
}
