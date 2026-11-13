//! `kartograf.exe --mcp`: сервер MCP (Model Context Protocol) по stdio. ИИ-клиент
//! (Claude, Cursor и другие) видит инструменты Картографа и правит карту в
//! открытом окне редактора — через [`crate::control`]. Окна нет — оно запускается.

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// Версия протокола, если клиент свою не назвал.
const PROTOCOL: &str = "2025-06-18";

const GUIDE: &str = "Kartograf is a battle-map editor for tabletop RPGs (D&D, Foundry VTT). These tools drive the editor window that is open on the user's screen: every change shows up live and can be undone with the `undo` tool or Ctrl+Z.

Coordinates are grid cells: x to the right, y down, (0, 0) is the top-left corner of the map; one cell is 5 ft (1.524 m). Object positions are centres; sizes are in cells.

Typical loop: `map_info` → `find_assets` to get asset keys → edit (`add_object`, `paint`, `add_wall`, `add_door`, …) → `view` to look at the result → `save_map` / `export_foundry`. To start from something, use `generate` (procedural maps), `build_plan` (a JSON plan, see plans/README.md in the repository) or `terrain_from_place` (real terrain from OpenStreetMap and satellite imagery).

Lights in Foundry come from objects: lamps, fires and braziers glow by their name; give any object `light` (e.g. `arcane` for a magic crystal) in `add_object` / `edit_object`. Chimneys, vents and hatches go on a roof with layer `on_roof`: they are part of that roof and vanish with it when players enter the house; `above_roofs` is only for things flying over the town. Call `set_wall_style` before painting rooms to choose their walls. New picture files put into the user's asset folder appear after `reload_assets`. `render_picture` writes the whole map with objects to an image file for a preview.";

fn tools() -> Value {
    let num = |d: &str| json!({"type": "number", "description": d});
    let int = |d: &str| json!({"type": "integer", "description": d});
    let text = |d: &str| json!({"type": "string", "description": d});
    let point = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2});
    let area = json!({"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4, "description": "[x0, y0, x1, y1] in cells; x1 and y1 are the far edges (a room [4, 3, 14, 11] gets walls on x=4, x=14, y=3, y=11)"});
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": properties, "required": required}});
    let light = json!({"type": "string", "enum": ["auto", "off", "campfire", "fireplace", "brazier", "candle", "lamp", "arcane"], "description": "How it lights the scene in Foundry; auto — by its name (lamps, fires, braziers glow)"});
    let layer = json!({"type": "string", "enum": ["auto", "floor", "low", "normal", "high", "above_walls", "on_roof", "above_roofs"], "description": "What covers what: floor (rugs), low, normal, high (trees, wardrobes), above_walls, on_roof (chimneys, vents, hatches: part of the roof, gone with it when players enter), above_roofs (airships, birds flying over everything); auto — by its name"});
    let roofs = json!({"type": "string", "enum": ["solid", "ghost", "hidden"], "description": "Roofs: solid as players see them (default), ghost — see-through to check inside, hidden"});
    json!([
        tool("map_info", "Size, name, level, counts of objects/walls/doors/labels/roads and the floor materials of the current map.", json!({}), &[]),
        tool("describe", "List what is on the map, with indices for editing: objects, walls, doors, labels, paths (roads, rivers), roofs. Optionally only inside an area.", json!({
            "include": {"type": "array", "items": {"type": "string", "enum": ["objects", "walls", "doors", "labels", "paths", "roofs"]}, "description": "Sections to list; default all"},
            "area": area, "limit": int("Max items per section, default 200")
        }), &[]),
        tool("find_assets", "Search the asset library (built-in pictures plus the user's Dungeondraft packs and folder) by words. Returns keys to use in add_object / paint.", json!({
            "query": text("Words to match in the name or folder, e.g. 'barrel', 'oak tree', 'stone floor'"),
            "kind": {"type": "string", "enum": ["floor", "wall", "door", "object", "roof"]},
            "limit": int("Default 30")
        }), &["query"]),
        tool("add_object", "Place an object (furniture, tree, prop) by asset key. Returns its index.", json!({
            "asset": text("Asset key from find_assets"), "x": num("Centre x, cells"), "y": num("Centre y, cells"),
            "rotation": num("Degrees clockwise, default 0"), "scale": num("Default 1"), "light": light, "layer": layer
        }), &["asset", "x", "y"]),
        tool("edit_object", "Move, rotate, scale, light or swap the picture of an object by index (see describe).", json!({
            "index": int("Object index"), "x": num("New centre x"), "y": num("New centre y"),
            "rotation": num("Degrees clockwise"), "scale": num("Scale"), "asset": text("New asset key"), "light": light, "layer": layer
        }), &["index"]),
        tool("set_scene", "What lives in the Foundry scene (Wild Canvas): place sound, what flies from tree crowns, weather, and the water current. A ship underway stands still while the water runs past it: set current and drift, paint the sea as water and the deck as anything else.", json!({
            "ambience": text("Place sound key, '' for none: forest-summer, forest-spring, forest-autumn, forest-winter, forest-magic, cave-1..3, dungeon-1..3, town-1, town-2, tavern, house"),
            "particles": {"type": "string", "enum": ["", "leaves", "fireflies"]},
            "weather": {"type": "string", "enum": ["", "snow"]},
            "current": num("Water current, cells per second, 0..3; 0 — still water. About 1 for a ship under sail"),
            "drift": num("Where the water runs, degrees clockwise from right: 90 — down the map, so the ship sails up")
        }), &[]),
        tool("add_level", "Add a floor above or below the current one and switch to it: a ship's lower deck, a cellar, an upper storey. Same size and palette; draw it like any map. In Foundry every floor is a level; a stairs, ladder or hatch object standing in the same place on two neighbouring floors becomes a passage between them. The scene opens on the floor that is current at export.", json!({
            "below": {"type": "boolean", "description": "Put the new floor under the current one (hold, cellar); default above"},
            "name": text("Floor name, shown in Foundry")
        }), &[]),
        tool("switch_level", "Switch to another floor (0 — the lowest, see map_info level_names) and/or rename the current one.", json!({
            "level": int("Floor number"), "name": text("New name for the floor")
        }), &[]),
        tool("add_roof", "Put a roof over a house: the rectangle of its walls in cells. Players see the roof from outside; it fades when they walk in.", json!({
            "area": area, "paint": text("Roof covering asset key from find_assets (kind roof), default builtin:roof_metal"),
            "shape": {"type": "string", "enum": ["gable", "hip", "shed", "flat", "tent"], "description": "Default gable"},
            "turn": int("Quarter turns of the ridge, 0..3"), "overhang": num("Cells over the walls, default 0.25")
        }), &["area"]),
        tool("edit_roof", "Change a roof by index (see describe with roofs): covering picture, shape, ridge direction.", json!({
            "index": int("Roof index"), "paint": text("Roof covering asset key from find_assets (kind roof), e.g. builtin:roof_copper"),
            "shape": {"type": "string", "enum": ["gable", "hip", "shed", "flat", "tent"]}, "turn": int("Quarter turns of the ridge, 0..3")
        }), &["index"]),
        tool("remove", "Remove items by indices: objects, walls, doors, labels, paths or roofs. Indices of later items shift down.", json!({
            "what": {"type": "string", "enum": ["objects", "walls", "doors", "labels", "paths", "roofs"]},
            "indices": {"type": "array", "items": {"type": "integer"}}
        }), &["what", "indices"]),
        tool("paint", "Paint floor over a rectangle or oval of cells. target 'terrain' paints ground; 'room' paints a room floor and walls appear along its edge automatically (dungeons, houses).", json!({
            "material": text("Floor asset key from find_assets (kind floor), or a material index from map_info"),
            "area": area, "shape": {"type": "string", "enum": ["rect", "oval"], "description": "Default rect"},
            "target": {"type": "string", "enum": ["terrain", "room"], "description": "Default terrain"}
        }), &["material", "area"]),
        tool("erase", "Erase floor and rooms over a rectangle or oval of cells.", json!({"area": area, "shape": {"type": "string", "enum": ["rect", "oval"]}}), &["area"]),
        tool("add_wall", "Add a free-standing wall through points (cells). Room walls come from painting with target 'room' instead.", json!({
            "points": {"type": "array", "items": point, "minItems": 2}, "closed": {"type": "boolean", "description": "Connect the last point to the first"},
            "low": {"type": "boolean", "description": "Low wall — railing, ship's bulwark, parapet: in Foundry it stops movement, but sight and light pass over it"}
        }), &["points"]),
        tool("add_door", "Put a door, window or opening into the wall nearest to a point.", json!({
            "x": num("Point near the wall, cells"), "y": num("Point near the wall, cells"),
            "kind": {"type": "string", "enum": ["door", "double", "window", "opening", "secret"], "description": "Default door"}
        }), &["x", "y"]),
        tool("add_label", "Write a label on the map (room name, street).", json!({"text": text("Text"), "x": num("Centre x"), "y": num("Centre y"), "size": num("Letter height in cells, default 1")}), &["text", "x", "y"]),
        tool("edit_label", "Change a label by index (see describe): text, place, size, rotation, or hide it from players.", json!({
            "index": int("Label index"), "text": text("New text"), "x": num("Centre x"), "y": num("Centre y"),
            "size": num("Letter height in cells"), "rotation": num("Degrees clockwise"), "gm_only": {"type": "boolean", "description": "Only the GM sees it: not in the exported picture"}
        }), &["index"]),
        tool("add_path", "Lay a road, path, river or stream: a band of floor along a smooth line through the points, over the ground and under objects.", json!({
            "points": {"type": "array", "items": point, "minItems": 2}, "material": text("Floor asset key from find_assets (kind floor), or a material index from map_info"),
            "width": num("Cells, default 2"), "soft": num("0..1, how much the edge fades, default 0.35"), "rough": num("0..1, how ragged the edge is, default 0.25"),
            "along": {"type": "boolean", "description": "Texture follows the line (cobbled road, carpet) instead of lying on the grid"}
        }), &["points", "material"]),
        tool("set_wall_style", "Choose how walls look. Without area: for walls made from now on (paint rooms after this); walls already standing keep their look. With area: restyle the walls standing in it.", json!({
            "asset": text("Wall asset key from find_assets (kind wall)"), "color": {"type": "array", "items": {"type": "integer"}, "minItems": 3, "maxItems": 3, "description": "Plain colour [r, g, b] instead of a picture"},
            "width": num("Thickness in cells, default 0.25 (0.35 for pictures)"), "area": area
        }), &[]),
        tool("reload_assets", "Re-read the user's asset folder: pictures added there while the editor was open become available to find_assets.", json!({}), &[]),
        tool("undo", "Undo the last changes.", json!({"steps": int("Default 1")}), &[]),
        tool("view", "Look at the map: returns a PNG picture of the whole map or of an area, as the editor draws it (objects, doors, roofs; no grid or editor windows).", json!({"area": area, "max_px": int("Longest side of the picture, 128..2048, default 1024"), "roofs": roofs}), &[]),
        tool("render_picture", "Write a picture of the map with all objects to a file (PNG, JPEG or WebP by the extension) — for a preview or to share. Nothing is returned but the file.", json!({
            "path": text("File path"), "px": int("Pixels per cell, 10..300, default 70"), "area": area, "roofs": roofs
        }), &["path"]),
        tool("generate", "Replace the map with a procedural one.", json!({
            "kind": {"type": "string", "enum": ["dungeon", "cave", "forest", "town", "walls", "house", "tavern", "jarl", "church", "arena", "cyberpunk"]},
            "width": int("Map width in cells (keeps the current size if omitted)"), "height": int("Map height in cells"),
            "seed": text("Same seed — same map"), "density": num("0..1, how full"), "rooms": int("Dungeon: number of rooms")
        }), &["kind"]),
        tool("build_plan", "Build the map from a JSON plan: rooms, corridors, doors, furniture, terrain. Format: plans/README.md in the Kartograf repository.", json!({"plan": {"type": "object", "description": "The plan"}}), &["plan"]),
        tool("terrain_from_place", "Build real terrain from OpenStreetMap and satellite imagery around a point: buildings with roofs, roads, water, trees. Takes from seconds to a minute.", json!({
            "lat": num("Latitude"), "lon": num("Longitude"), "width": int("Cells, 8..256, default 40"), "height": int("Cells, 8..256, default 30"),
            "trees": num("0..1, extra groves on top of what the photo shows")
        }), &["lat", "lon"]),
        tool("new_map", "Start an empty map.", json!({"width": int("Cells"), "height": int("Cells")}), &["width", "height"]),
        tool("open_map", "Open a .karta file.", json!({"path": text("File path")}), &["path"]),
        tool("save_map", "Save the map to a .karta file (the current file if no path).", json!({"path": text("File path")}), &[]),
        tool("export_foundry", "Export the map for Foundry VTT and Universal VTT into a folder: picture, scene JSON with walls, doors and lights, .dd2vtt.", json!({"dir": text("Folder"), "px": int("Pixels per cell, default 140")}), &["dir"]),
    ])
}

/// Ответ инструмента в виде MCP: картинка — картинкой, остальное — текстом JSON.
fn content(v: Value) -> Value {
    if let Some(png) = v.get("image_png_base64").and_then(|p| p.as_str()) {
        let mut meta = v.clone();
        if let Some(o) = meta.as_object_mut() {
            o.remove("image_png_base64");
        }
        return json!({"content": [{"type": "image", "data": png, "mimeType": "image/png"}, {"type": "text", "text": meta.to_string()}]});
    }
    json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]})
}

/// Свои stdin/stdout/stderr — не в наследство окну. Иначе окно держит каналы
/// клиента MCP открытыми: сервер вышел, а клиент конца вывода не дождётся, пока
/// окно живо (так зависала проверка).
#[cfg(windows)]
fn keep_std_handles() {
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        fn SetHandleInformation(handle: *mut core::ffi::c_void, mask: u32, flags: u32) -> i32;
    }
    const HANDLE_FLAG_INHERIT: u32 = 1;
    for which in [-10i32, -11, -12] {
        // SAFETY: свои стандартные дескрипторы процесса; пустой или неверный
        // SetHandleInformation просто не тронет.
        unsafe {
            let h = GetStdHandle(which as u32);
            if !h.is_null() && h as isize != -1 {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
fn keep_std_handles() {}

/// Запустить окно редактора с той же папкой данных.
fn launch(data_dir: Option<&Path>) {
    keep_std_handles();
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    if let Some(d) = data_dir {
        cmd.arg("--data-dir").arg(d);
    }
    let _ = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
}

/// Читать запросы JSON-RPC построчно со stdin, отвечать в stdout.
pub fn serve(data_dir: PathBuf, explicit: bool) -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(out, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}}));
            let _ = out.flush();
            continue;
        };
        // Уведомления (без id) ответа не ждут.
        let Some(id) = msg.get("id").cloned() else { continue };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: Result<Value, (i64, String)> = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "kartograf", "version": env!("CARGO_PKG_VERSION")},
                "instructions": GUIDE,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let dir = explicit.then_some(data_dir.as_path());
                Ok(match crate::control::call(&data_dir, &|| launch(dir), name, &args) {
                    Ok(v) => content(v),
                    Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
                })
            }
            other => Err((-32601, format!("unknown method {other}"))),
        };
        let reply = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        let _ = writeln!(out, "{reply}");
        let _ = out.flush();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Каждый инструмент описан как следует: имя, описание, схема-объект, а
    /// обязательные поля есть среди свойств.
    /// Инструменты, которых не хватило на первой живой карте (Иерихон), на месте.
    #[test]
    fn tools_found_missing_on_a_real_map_are_there() {
        let list = tools();
        let names: Vec<&str> = list.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        for n in ["reload_assets", "render_picture", "set_wall_style", "add_path", "edit_label", "edit_roof", "add_roof", "set_scene", "add_level", "switch_level"] {
            assert!(names.contains(&n), "{n}");
        }
        let add = list.as_array().unwrap().iter().find(|t| t["name"] == "add_object").unwrap();
        assert!(add["inputSchema"]["properties"]["light"]["enum"].as_array().unwrap().iter().any(|v| v == "arcane"));
    }

    #[test]
    fn tools_are_well_formed() {
        let list = tools();
        let list = list.as_array().unwrap();
        assert!(list.len() >= 20);
        for t in list {
            let name = t["name"].as_str().unwrap();
            assert!(!t["description"].as_str().unwrap().is_empty(), "{name}");
            assert_eq!(t["inputSchema"]["type"], "object", "{name}");
            for r in t["inputSchema"]["required"].as_array().unwrap() {
                assert!(t["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{name}: {r}");
            }
        }
    }

    #[test]
    fn picture_comes_back_as_image_content() {
        let c = content(json!({"image_png_base64": "AAA", "area": [0, 0, 4, 4]}));
        assert_eq!(c["content"][0]["type"], "image");
        assert_eq!(c["content"][0]["data"], "AAA");
        assert!(c["content"][1]["text"].as_str().unwrap().contains("area"));
    }
}
