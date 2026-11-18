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

Lights in Foundry come from objects: lamps, fires and braziers glow by their name; give any object `light` (e.g. `arcane` for a magic crystal) in `add_object` / `edit_object`. Chimneys, vents and hatches go on a roof with layer `on_roof`: they are part of that roof and vanish with it when players enter the house; `above_roofs` is only for things flying over the town. Call `set_wall_style` before painting rooms to choose their walls. New picture files put into the user's asset folder appear after `reload_assets`. `render_picture` writes the whole map with objects to an image file for a preview.

Rooms and open floors: `paint` with `target: room` puts walls around the painted floor by itself (room_edges) — use it for enclosed rooms only. Open terraces, balconies, porches, piers and courtyards are `target: terrain` with `feather: 0`, edged with `add_wall` `low: true` where a railing belongs.

Facing: most FA furniture is drawn with its back at the TOP of the picture and faces BOTTOM; fireplaces open to the RIGHT. Give `front` (the facing side as `preview_assets` labels it) and `look_at` [x, y] instead of computing `rotation`; `against_wall: true` turns the back to the nearest wall (straight or diagonal) and moves the object so its visible back touches the wall face; `inset` sinks it that many cells into the wall (wall lamps, panels).

Finding assets: `find_assets` ranks results — the exact file name (with or without the `_1x1` size tail) comes first, then whole words of the name in order; broken, damaged, fallen, desert, arctic, cyberpunk and similar side branches sink unless you name them; `exclude` drops words entirely. Before placing furniture whose facing matters (beds, chairs, wall lamps, fireplaces), call `preview_assets`: it shows each picture at rotation 0 with its sides labelled TOP, RIGHT, BOTTOM, LEFT and returns `visible_cells`, the opaque part of the sprite in cells from its centre.

Precision: `paint` and `erase` take fractional areas in quarter cells (a floor up to 9.5 meets a wall added at 9.5), `shape: poly` with `points`, and `feather` for soft ground edges (lawns, mud, shores). `view` with `grid: true` draws cell lines and coordinates along the top and left edges. Doors: `add_door` takes the door leaf / window `asset` and the jamb `frame` from `find_assets` (kind door), a `width`, or an exact segment `a`–`b` on a wall; `edit_door` changes them later. Objects take `shadow` (false for blankets, rugs, flowers on the ground), `recolor` [r, g, b] to paint Colorable rugs, banners and cloth in a colour, and `tint` [r, g, b] to shade a picture by multiplying; a tree picture of your own becomes a crown above tokens in Foundry with `canopy: true` (Forgotten Adventures tree tops are crowns by themselves). `batch` runs many edits in one call and undoes them as one step; if one fails, nothing changes. Roofs: `add_roof` takes `points` for towers, rotundas and slanted wings (`shape: tent` gives a faceted spire). A polygon roof is drawn as sharp as a rectangular one, with flat slopes to every wall, ridge caps and straight valleys from inner corners; `overhang` is kept apart, `points` stay the walls. Gable and hip roofs (and polygon hip / tent roofs) that touch on one floor join by height: put the annex roof against the main house and it cuts into the main slope along diagonal valleys, its ridge ending where it goes under; a shed, flat or tower tent roof, or a roof lying wholly on another, stays on top in order. Pictures: `view` and `render_picture` take `lighting: dusk` or `night` to bake the objects' lights with wall shadows and light through windows into the picture; bright light is daylight and stacked lights do not burn the floor white. The Foundry export is unaffected (Foundry lights the scene itself). With the editor window minimized, edits go on in the background; a picture shows the window for a few seconds without taking focus and minimizes it again.";

fn tools() -> Value {
    let num = |d: &str| json!({"type": "number", "description": d});
    let int = |d: &str| json!({"type": "integer", "description": d});
    let text = |d: &str| json!({"type": "string", "description": d});
    let open_edges = json!({"type": "array", "items": {"type": "integer"}, "description": "Open edges: no side wall, lip, shadow or rail there, the top runs on into ground of the same height. Edge k runs from corner k to corner k + 1 of the outline (repeated points dropped); [] gives every edge its wall back"});
    let point = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2});
    let area = json!({"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4, "description": "[x0, y0, x1, y1] in cells; x1 and y1 are the far edges (a room [4, 3, 14, 11] gets walls on x=4, x=14, y=3, y=11)"});
    let tool = |name: &str, description: &str, properties: Value, required: &[&str]| json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": properties, "required": required}});
    let light = json!({"type": "string", "enum": ["auto", "off", "campfire", "fireplace", "brazier", "candle", "lamp", "arcane", "alarm", "neon"], "description": "How it lights the scene in Foundry; auto — by its name (lamps, fires, braziers glow); alarm — a red emergency light by the wall, its beam slowly sweeping around; neon — a steady cold white electric light (station, lab, future city): never by name, only when chosen"});
    // Свой свет предмета поверх вида: одна лампа на одной карте, вид света у остальных прежний.
    let light_radius = json!({"type": ["number", "null"], "description": "This object's own light radius in cells (the dim edge; the bright part scales with it) instead of what its light kind gives; null — back to the kind's. Tune one lamp on one map this way instead of changing a light kind for every map"});
    let light_color = json!({"type": ["array", "null"], "items": {"type": "integer"}, "description": "Own light colour [r, g, b]; null — the kind's"});
    let light_animation = json!({"type": ["string", "null"], "description": "Own Foundry light animation: none (steady), auto or null (the kind's), or flame, torch, revolving, siren, pulse, reactivepulse, chroma, wave, fog, sunburst, dome, emanation, hexa, ghost, energy, vortex, witchwave, rainbowswirl, radialrainbow, fairy, grid, starlight, smokepatch"});
    let light_speed = json!({"type": ["integer", "null"], "description": "Own animation speed 0..10 (revolving: speed/5 rad per second, 7 — a turn in 4.5 s; siren is 50 times faster; pulse: a period of 12.6/speed s); null — the kind's"});
    let light_intensity = json!({"type": ["integer", "null"], "description": "Own animation intensity 1..10 (revolving: the beam width); null — the kind's"});
    let sunk = json!({"type": ["number", "null"], "description": "How much of the object is under water, 0..1 counted from the bottom of its picture (the front): 0.6 — a seat in water up to its backrest, 1 — it lies on the bottom. The part under the waterline is drawn in the colour of the water below it and stays in the map picture, so Wild Canvas runs its water over it; the dry part above goes to Foundry as the object's own tile. One object, no separate sunken picture needed. No shadow. null — all above water"});
    let layer = json!({"type": "string", "enum": ["auto", "floor", "low", "normal", "high", "above_walls", "on_roof", "above_roofs"], "description": "What covers what: floor (rugs), low, normal, high (trees, wardrobes), above_walls, on_roof (chimneys, vents, hatches: part of the roof, gone with it when players enter), above_roofs (airships, birds flying over everything); auto — by its name"});
    let roofs = json!({"type": "string", "enum": ["solid", "ghost", "hidden"], "description": "Roofs: solid as players see them (default), ghost — see-through to check inside, hidden"});
    let canopy = json!({"type": ["boolean", "string"], "description": "A tree crown in Foundry: its own tile above tokens that fades when a token walks under it and sways in the wind (Wild Canvas). Auto (default) — Forgotten Adventures tree tops and the built-in tree; give true for your own tree pictures, false to keep a crown on the ground"});
    let shadow = json!({"type": ["boolean", "string"], "description": "Cast a shadow: true, false (blankets, rugs, flowers on the ground) or \"auto\" — by its name (flat things have none). Default auto"});
    let tint = json!({"type": ["array", "null"], "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 0, "maxItems": 3, "description": "Multiply the picture by [r, g, b]: shade or darken it (autumn or frosty trees, a dim corner); null or [] removes it"});
    let recolor = json!({"type": ["array", "null"], "items": {"type": "integer", "minimum": 0, "maximum": 255}, "minItems": 0, "maxItems": 3, "description": "Paint the picture in [r, g, b], keeping its pattern and shading: Colorable rugs, banners, cloth, bricks; null or [] restores the original colours"});
    let points = json!({"type": "array", "items": point, "minItems": 3, "description": "Polygon corners in cells, for shape poly"});
    let front = json!({"type": "string", "enum": ["bottom", "top", "left", "right"], "description": "Which side of the picture is the object's face, as labelled in preview_assets; default bottom (most FA furniture: back drawn at the top). Used by look_at and against_wall"});
    let look_at = json!({"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "Point [x, y] in cells the front should face (a chair to its table, a telescope to the lake); sets rotation unless rotation is given"});
    let against_wall = json!({"type": "boolean", "description": "Put the object's back against the nearest wall within 2 cells (room or free-standing, straight or diagonal): it turns to face away from the wall along its normal and moves so its visible back touches the wall face. The reply has the final x, y, rotation and the wall"});
    let inset = json!({"type": "number", "minimum": 0, "maximum": 1, "description": "With against_wall: how deep, in cells, the back sinks into the wall — a wall lamp, a wall panel sit in the wall instead of standing by it; default 0"});
    let lighting = json!({"type": "string", "enum": ["day", "dusk", "night"], "description": "day (default): the picture as is. dusk / night: the objects' lights glow with soft wall shadows, windows let light out onto the ground, roofs hide light under them; cool ambient outside, dim indoors. Bright light shows the floor as by day and never brighter, however many lights overlap (as in Foundry)"});
    let door_kind = json!({"type": "string", "enum": ["door", "double", "window", "opening", "secret"], "description": "Default door"});
    json!([
        tool("list_prefabs", "Prefabs saved in the data folder: name, number of objects, size in cells.", json!({}), &[]),
        tool("save_prefab", "Save objects of the map (indices from describe) as a prefab under a name: a set that is placed again in one go — a table with chairs, a kitchen corner, a console with its chair. The person sees it on the shelf, tab «Префабы».", json!({
            "name": text("Prefab name"), "indices": {"type": "array", "items": {"type": "integer"}, "description": "Object indices"}
        }), &["name", "indices"]),
        tool("place_prefab", "Place a prefab: its centre at x, y, the whole set turned by rotation (degrees clockwise). Returns the indices of the new objects.", json!({
            "name": text("Prefab name"), "x": num("Centre x"), "y": num("Centre y"), "rotation": num("Degrees clockwise, default 0")
        }), &["name", "x", "y"]),
        tool("map_info", "Size, name, level, counts of objects/walls/doors/labels/roads and the floor materials of the current map.", json!({}), &[]),
        tool("describe", "List what is on the map, with indices for editing: objects, walls, doors, labels, paths (roads, rivers), roofs. Optionally only inside an area.", json!({
            "include": {"type": "array", "items": {"type": "string", "enum": ["objects", "walls", "doors", "labels", "paths", "roofs", "platforms", "room_walls", "visible"]}, "description": "Sections to list; default all except room_walls (the automatic walls along painted rooms as segments [[x, y], [x, y]], asked for by name) and visible (adds vis to each object: the opaque part of its picture in cells from its centre before rotation, [left, top, right, bottom]; slower, the pictures are decoded)"},
            "area": area, "limit": int("Max items per section, default 200")
        }), &[]),
        tool("check_map", "Check the current level for what will look or play wrong in Foundry (the same list as the «Проверка» window): roof-only things off roofs and non-roof things on them, anything above roofs over a house (hides the room), vertical things lying flat, pipe ends to nowhere, objects inside each other, blocked doors, objects across house walls, street things indoors and indoor things outside, objects past the map edge, labels on objects, wall lamps off walls, lamp posts not against a wall or in the middle of a passage, objects that block a passage for a one-cell token. Returns count and issues: kind, text, line ('[kind] text'), at [x, y] — where to look, objects — their indices.", json!({}), &[]),
        tool("find_assets", "Search the asset library (built-in pictures plus the user's Dungeondraft packs and folder) by words, best matches first: the exact file name, then whole words of the name in order; 'table' does not rank 'tablet'; broken, damaged, fallen, desert, arctic, cyberpunk and similar branches sink unless named. Height, normal and roughness maps are not in the library. Returns keys to use in add_object / paint / add_door.", json!({
            "query": text("Words to match in the name or folder, e.g. 'barrel', 'oak tree', 'stone floor', or an exact file name 'Chair_Wood_Ashen_A_1x1'"),
            "kind": {"type": "string", "enum": ["floor", "wall", "door", "object", "roof"], "description": "door: door leaves, jambs (Door_Frame), windows and window sills"},
            "exclude": text("Words to drop, space-separated, e.g. 'broken fallen bloody'"),
            "limit": int("Default 30")
        }), &["query"]),
        tool("preview_assets", "Look at assets before placing them: a contact sheet on a dark background, one tile per asset at rotation 0 with its number, the cells it covers and its sides labelled TOP, RIGHT, BOTTOM, LEFT — the values of `front` in add_object / edit_object. JSON gives size in cells, alpha_bbox (opaque part, fractions of the picture: u right, v down) and visible_cells (the same in cells from the object's centre).", json!({
            "assets": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 16, "description": "Asset keys from find_assets"},
            "tile_px": int("Tile size, 64..512, default 192")
        }), &["assets"]),
        tool("add_object", "Place an object (furniture, tree, prop) by asset key. Returns its index.", json!({
            "asset": text("Asset key from find_assets"), "x": num("Centre x, cells"), "y": num("Centre y, cells"),
            "rotation": num("Degrees clockwise, default 0"), "scale": num("Default 1"), "light": light, "layer": layer, "shadow": shadow, "canopy": canopy.clone(), "flip": {"type": "boolean", "description": "Mirror the picture left to right (Dungeondraft: hold A while placing); rotation applies after"},  "tint": tint, "recolor": recolor,
            "light_radius": light_radius.clone(), "light_color": light_color.clone(), "light_animation": light_animation.clone(), "light_speed": light_speed.clone(), "light_intensity": light_intensity.clone(), "sunk": sunk.clone(),
            "front": front.clone(), "look_at": look_at.clone(), "against_wall": against_wall.clone(), "inset": inset.clone()
        }), &["asset", "x", "y"]),
        tool("edit_object", "Move, rotate, scale, light, shade, tint or swap the picture of an object by index (see describe).", json!({
            "index": int("Object index"), "x": num("New centre x"), "y": num("New centre y"),
            "rotation": num("Degrees clockwise"), "scale": num("Scale"), "asset": text("New asset key"), "light": light, "layer": layer, "shadow": shadow, "canopy": canopy, "flip": {"type": "boolean", "description": "Mirror the picture left to right (Dungeondraft: hold A while placing); rotation applies after"},  "tint": tint, "recolor": recolor,
            "light_radius": light_radius, "light_color": light_color, "light_animation": light_animation, "light_speed": light_speed, "light_intensity": light_intensity, "sunk": sunk,
            "front": front, "look_at": look_at, "against_wall": against_wall, "inset": inset
        }), &["index"]),
        tool("set_scene", "What lives in the Foundry scene (Wild Canvas): place sound, scene light, what flies from tree crowns, weather, and the water current. A ship underway stands still while the water runs past it: set current and drift, paint the sea as water and the deck as anything else.", json!({
            "ambience": text("Place sound key, '' for none: forest-summer, forest-spring, forest-autumn, forest-winter, forest-magic, cave-1..3, dungeon-1..3, town-1, town-2, station-1 (ventilation hum), station-2 (reactor hum), tavern, house"),
            "light": {"type": "string", "enum": ["", "day", "dim", "dark"], "description": "Scene light in Foundry, apart from the place sound: day — global light, everything in sight is lit; dim — global light with the scene darkened (tokens see everything in their line of sight, lamps stand out: a station on emergency power, a dusk street); dark — no global light, tokens see only near light sources (place lamps so there are no black gaps between them, or use dim); '' — by the place sound: forest and town are day, everything else dark"},
            "particles": {"type": "string", "enum": ["", "leaves", "fireflies"]},
            "weather": {"type": "string", "enum": ["", "snow"]},
            "current": num("Water current, cells per second, 0..3; 0 — still water. About 1 for a ship under sail"),
            "drift": num("Where the water runs, degrees clockwise from right: 90 — down the map, so the ship sails up"),
            "flows": {"type": "array", "description": "Water that runs differently from the rest (a canal runs, the harbour beside it stands still): each {area: [x0, y0, x1, y1] or points: [[x, y]…], current, drift}. Becomes its own Foundry region; put the seam under a bridge or a lock gate — foam shows at a region edge. Replaces the list; [] clears it.", "items": {"type": "object"}},
            "shadows": {"type": "object", "description": "The sun of the map: {on, angle — where shadows fall, degrees clockwise from right (45 — down-right, 225 — up-left), dist and blur in cells, opacity 0..1, fade 0..1}. Objects, walls and relief tiers cast their shadows this way, in the picture and in Foundry (Wild Canvas object shadows). Only the keys given change"},
            "rails": {"type": "array", "description": "Mine-cart routes, each a list of points [[x, y]…] in cells along the track; the first point is the depot (inside a mine or tunnel), where carts fade into the dark. Carts, wagons and their loads standing on a route ride it back and forth in Foundry as one train, with a rumble heard nearby; '_Fill' cargo is unloaded at the far end. Replaces the list; [] clears it.", "items": {"type": "array"}}
        }), &[]),
        tool("add_level", "Add a floor above or below the current one and switch to it: a ship's lower deck, a cellar, an upper storey. Same size and palette; draw it like any map. In Foundry every floor is a level; a stairs, ladder or hatch object standing in the same place on two neighbouring floors becomes a passage between them. The scene opens on the floor that is current at export. On an upper floor draw only what belongs to it — its rooms, balconies, decks: whatever is left unpainted there (erase clears it) is see-through in Foundry, and the floor below shows through alive, with its ground, furniture, roofs, tree crowns and tokens. Do not copy the ground, roofs or trees of the lower floor up. A roof on the lower floor that lies mostly under the upper floor is shown only from below.", json!({
            "below": {"type": "boolean", "description": "Put the new floor under the current one (hold, cellar); default above"},
            "name": text("Floor name, shown in Foundry")
        }), &[]),
        tool("switch_level", "Switch to another floor (0 — the lowest, see map_info level_names) and/or rename the current one.", json!({
            "level": int("Floor number"), "name": text("New name for the floor")
        }), &[]),
        tool("grow_map", "Add margins to the map, in cells, on every floor: everything drawn moves by left/top. Build the surroundings around a finished map — a harbour around a ship, a street around a house. Undo history is cleared. The map remembers how far it grew left and up (`origin`): when a Foundry scene is updated from a new export, the module moves the tokens already placed there by the same amount.", json!({
            "left": int("Cells to add on the left"), "top": int("Cells to add on top"), "right": int("Cells to add on the right"), "bottom": int("Cells to add at the bottom")
        }), &[]),
        tool("add_roof", "Put a roof over a house: the rectangle of its walls (area) or, for towers, rotundas and slanted wings, the polygon of its walls (points). Players see the roof from outside; it fades when they walk in. Polygon roofs get roof index after the rectangular ones in describe.", json!({
            "area": area.clone(), "points": {"type": "array", "items": point.clone(), "minItems": 3, "description": "Polygon of the walls in cells, instead of area"},
            "paint": text("Roof covering asset key from find_assets (kind roof), default builtin:roof_metal"),
            "shape": {"type": "string", "enum": ["gable", "hip", "shed", "flat", "tent", "dome", "vault"], "description": "Default gable for area, tent for points (a faceted spire with a ridge from every corner); dome and vault only with points"},
            "turn": int("Quarter turns of the ridge, 0..3"), "overhang": num("Cells the roof reaches over the walls, default 0.25, for area and points alike. It is kept apart from the outline: area and points stay the walls, describe shows them with overhang"),
            "rail": text("Railing along the edge of a flat rectangular roof (area only; a polygon roof refuses it): a ship's quarterdeck, a balcony. A wall asset key from find_assets (kind wall). It opens where a ladder or stairs object reaches the edge")
        }), &[]),
        tool("edit_roof", "Change a roof by index (see describe with roofs): covering picture, shape, ridge direction; a polygon roof also takes new points.", json!({
            "index": int("Roof index"), "paint": text("Roof covering asset key from find_assets (kind roof), e.g. builtin:roof_copper"),
            "shape": {"type": "string", "enum": ["gable", "hip", "shed", "flat", "tent", "dome", "vault"]}, "turn": int("Quarter turns of the ridge, 0..3"),
            "points": {"type": "array", "items": point.clone(), "minItems": 3, "description": "Polygon roofs: new outline of the walls"}, "area": area.clone(), "overhang": num("Cells over the walls; changes the eaves only, area and points stay the walls"),
            "rail": text("Railing along the edge of a flat roof: a wall asset key (kind wall); empty string removes it. It opens where a ladder or stairs object reaches the edge")
        }), &["index"]),
        tool("add_platform", "A raised platform on the floor: a stage, a dais, a reactor podium, the tiers of an auditorium. Height reads the way hand-drawn battle maps show it: the top is the same floor tinted lighter, a bright lip runs along its edge, a side wall flares outward from the edge (the higher the platform, the wider) and a shadow lies at its foot; stairs are steps running outward from the edge, wider at the top. Edges along a room wall get no side wall or shadow. It is part of the floor: drawn under objects and walls, baked into the map picture for Foundry, moved and resized by edit_platform — no per-map picture needed. Put it down after the floor, before the furniture on it. Several platforms stack in the order added: tiers of a hall are platforms one inside another, each a step (height 0.08) higher.", json!({
            "points": {"type": "array", "items": point.clone(), "minItems": 3, "description": "Outline of the top in cells, any polygon"},
            "area": area.clone(),
            "circle": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "[cx, cy, r]: a round platform; with area — the part of the circle inside that rectangle (a dais by the wall of a hall: area is the hall)"},
            "outside_circle": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "[cx, cy, r] with area: the rectangle without the circle that bites into it from one side or corner — a tier of an amphitheatre behind the arc of its step"},
            "height": num("Width of the visible side wall in cells, the platform's height to the eye: about 0.08 a step, 0.4 a dais five steps high; default 0.3"),
            "color": {"type": ["array", "null"], "items": {"type": "integer"}, "description": "Tint of the top [r, g, b], default white; steps take this colour too"},
            "strength": num("How strongly the top is tinted, 0..1, default 0.1 (a little lighter than the floor); 0.4–0.6 turns a dark floor into light plating"),
            "side": {"type": ["array", "null"], "items": {"type": "integer"}, "description": "Colour of the side wall [r, g, b], default dark steel [98, 106, 116]; stone, wood — to match the room"},
            "stairs": {"type": "array", "items": {"type": "object"}, "description": "Stairs up the platform: each {at: [x, y] — a point near the edge where they join (the nearest point of the outline is taken, the steps run outward from that edge), width — cells at the top, default 2, steps — how many, default 3; a step is 0.4 cells deep}"},
            "rail": {"type": "boolean", "description": "Railing along the free edges (not along room walls), open at every stair and running down both sides of it; posts every 1.6 cells at most. Tokens cannot cross a railed edge: put stairs where people go up"},
            "lights": {"type": ["boolean", "array", "null"], "items": {"type": "integer"}, "description": "Little lights: short glowing strips along the lip every 2.4 cells, two on every stair tread, a glowing cap on every railing post. true — cold cyan, [r, g, b] — that colour, false or null — none"},
            "grates": {"type": "array", "items": {"type": "object"}, "description": "Floor grates along the inside of the edge: each {at: [x, y] — a point near the edge (the nearest point of the outline is the middle of the grate), length — cells along the edge, default 3}; a dark slotted strip 0.3–0.68 cells in from the edge, following its curve. The list replaces the old one, [] removes them"},
            "open": open_edges.clone()
        }), &[]),
        tool("edit_platform", "Change a platform by index (see describe with platforms): outline (points, area, circle, outside_circle as in add_platform), height, colours, stairs (the list replaces the old one, [] removes them), rail, lights, grates. To move a platform give it the new outline.", json!({
            "index": int("Platform index"),
            "points": {"type": "array", "items": point.clone(), "minItems": 3}, "area": area.clone(),
            "circle": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3}, "outside_circle": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
            "height": num("Width of the visible side wall, cells"), "color": {"type": ["array", "null"], "items": {"type": "integer"}}, "strength": num("0..1"),
            "side": {"type": ["array", "null"], "items": {"type": "integer"}}, "stairs": {"type": "array", "items": {"type": "object"}},
            "rail": {"type": "boolean"}, "lights": {"type": ["boolean", "array", "null"], "items": {"type": "integer"}}, "grates": {"type": "array", "items": {"type": "object"}},
            "open": open_edges
        }), &["index"]),
        tool("remove", "Remove items by indices: objects, walls, doors, labels, paths, roofs or platforms. Indices of later items shift down.", json!({
            "what": {"type": "string", "enum": ["objects", "walls", "doors", "labels", "paths", "roofs", "platforms"]},
            "indices": {"type": "array", "items": {"type": "integer"}}
        }), &["what", "indices"]),
        tool("paint", "Paint floor over a rectangle, oval or polygon. Edges are exact to a quarter cell, so fractional areas line up with add_wall (a room [4, 3, 9.5, 7.5] gets walls on x=9.5 and y=7.5). target 'terrain' paints ground; 'room' paints a room floor and puts walls all around it by itself (room_edges) — so open terraces, balconies, porches and wooden piers are terrain with feather 0, edged with add_wall low: true where needed.", json!({
            "material": text("Floor asset key from find_assets (kind floor), or a material index from map_info"),
            "area": area, "shape": {"type": "string", "enum": ["rect", "oval", "poly"], "description": "Default rect; poly takes points instead of area"},
            "points": points,
            "target": {"type": "string", "enum": ["terrain", "room"], "description": "Default terrain"},
            "feather": num("Terrain only, 0..1, default 0: soft edge — the share of the way from the edge to the middle over which the paint fades. About 0.35 for lawns, mud and shores"),
            "angle": num("Turn the floor pattern, degrees clockwise (45 for parquet along a diagonal wall). Without it a library key lies straight and a material index keeps its own angle (map_info shows it)"),
            "live": {"type": "string", "enum": ["auto", "off", "water", "lava", "swamp", "neon"], "description": "What this paint comes alive as in Foundry (Wild Canvas); default auto — by the picture's name (water, lava, swamp, cyberpunk grid floor → neon). off keeps a floor still: a sci-fi grid floor without running neon, a decorative pool"}
        }), &["material"]),
        tool("erase", "Erase floor and rooms over a rectangle, oval or polygon, exact to a quarter cell.", json!({
            "area": area, "shape": {"type": "string", "enum": ["rect", "oval", "poly"]}, "points": points,
            "feather": num("0..1, default 0: erase with a soft edge")
        }), &[]),
        tool("relief", "Shape the ground: raise a ridge, sink a hollow, cut a cliff. Heights are in tiers of 5 ft (one cell). Where the ground rises steeper than 1.5 tiers per cell it is a cliff: a band of rock face runs down from the edge (wider the higher), scree at its foot, and the higher tier casts a shadow; a gentler step is a narrow ridge. In Foundry a cliff edge is a wall: no walking across, sight passes over it. Over a rectangle, oval or polygon, exact to a quarter cell; feather softens the edge into a slope.", json!({
            "area": area.clone(), "shape": {"type": "string", "enum": ["rect", "oval", "poly"], "description": "Default rect; poly takes points instead of area"},
            "points": points.clone(),
            "mode": {"type": "string", "enum": ["set", "raise", "lower", "smooth"], "description": "set (default) — ground goes to height; raise / lower — by height; smooth — rounds the steps off"},
            "height": num("Tiers (5 ft each): the height for set, the step for raise and lower. Default 1"),
            "feather": num("0..1, default 0: share of the way from the edge to the middle over which the change fades — 0 a sheer cliff all around, 0.5 a cliff melting into a slope"),
            "face": text("Picture of the cliff face, a wall asset key from find_assets (kind wall, e.g. a Cliff_… path texture); '' — the built-in rock. Applies to the whole map"),
            "ridge": text("Picture of a ridge step (kind wall); '' — built-in. Whole map"),
            "under": text("Picture of the scree under a cliff (kind wall); '' — built-in. Whole map"),
            "natural": {"type": "boolean", "description": "Real terrain look: lower tiers are not darkened by depth and gentle slopes get no ridge lines; only the cliffs and their shadows show. Whole map"}
        }), &[]),
        tool("add_wall", "Add a free-standing wall through points (cells). Room walls come from painting with target 'room' instead.", json!({
            "points": {"type": "array", "items": point, "minItems": 2}, "closed": {"type": "boolean", "description": "Connect the last point to the first"},
            "low": {"type": "boolean", "description": "Low wall — railing, ship's bulwark, parapet: in Foundry it stops movement, but sight and light pass over it"}
        }), &["points"]),
        tool("add_door", "Put a door, window or opening into a wall: near a point (snapped to the grid along the wall) or exactly on a segment a–b (both ends are projected onto the nearest wall). A door in the same place replaces the old one.", json!({
            "x": num("Point near the wall, cells"), "y": num("Point near the wall, cells"),
            "a": point.clone(), "b": point.clone(),
            "kind": door_kind.clone(),
            "width": num("Cells along the wall, with x/y; default 1, double 2"),
            "asset": text("Door leaf, window or window sill picture from find_assets (kind door); '' for none. Default: a wooden door, or a stone sill for a window"),
            "frame": text("Jamb picture from find_assets (kind door, e.g. Door_Frame_… or builtin:frame_stone_1); '' for none. Built-in jambs fit the width by themselves")
        }), &[]),
        tool("edit_door", "Change a door, window or opening by index (see describe with doors): kind, pictures, or move it to a segment a–b on a wall.", json!({
            "index": int("Door index"), "kind": door_kind, "a": point.clone(), "b": point.clone(),
            "asset": text("Door leaf / window / sill picture key; '' removes it"), "frame": text("Jamb picture key; '' removes it")
        }), &["index"]),
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
        tool("reload_assets", "Re-read the user's asset folder: pictures added there while the editor was open become available to find_assets. Also re-reads live.json in the folder's root — the user's own living machines for Foundry (Wild Canvas 0.30+): {\"File_Name_Start\": {\"fans\": [[cx, cy, r, speed]], \"lamps\": [[x, y, r, \"#rrggbb\"]], \"leds\": [x, y, colour, …], \"ledSize\": 0.006}}, fractions of the picture (leds: x and y in thousandths, colour 0 cyan, 1 magenta or a number 0xRRGGBB). An object whose file name starts with such a key gets its own tile and the layout in the tile flag: fans spin, lamps smoulder and flare, leds blink. Returns the keys read as live.", json!({}), &[]),
        tool("undo", "Undo the last changes. A batch is one step (one per floor it touched).", json!({"steps": int("Default 1")}), &[]),
        tool("batch", "Run many edits in one call; they are undone as one step (one per floor touched). If any edit fails, the whole batch is rolled back and the map stays as it was. Not allowed inside: view, render_picture, preview_assets, export_foundry, save_map, open_map, new_map, generate, build_plan, terrain_from_place, grow_map, add_level, undo, reload_assets, batch.", json!({
            "ops": {"type": "array", "minItems": 1, "maxItems": 1000, "items": {"type": "object", "properties": {"tool": text("Tool name, e.g. add_object"), "arguments": {"type": "object", "description": "Its arguments"}}, "required": ["tool"]}}
        }), &["ops"]),
        tool("view", "Look at the map: returns a PNG picture of the whole map or of an area, as the editor draws it (objects, doors, roofs; no editor windows).", json!({
            "area": area, "max_px": int("Longest side of the picture, 128..2048, default 1024"), "roofs": roofs, "lighting": lighting.clone(),
            "grid": {"type": "boolean", "description": "Draw cell lines (every fifth darker) and coordinates along the top and left margins; grid_margin_px in the reply is [left, top]: picture point (px, py) is cell (area.x0 + (px - left) / px_per_cell, area.y0 + (py - top) / px_per_cell)"}
        }), &[]),
        tool("render_picture", "Write a picture of the map with all objects to a file (PNG, JPEG or WebP by the extension) — for a preview or to share. Nothing is returned but the file.", json!({
            "path": text("File path"), "px": int("Pixels per cell, 10..300, default 70"), "area": area, "roofs": roofs, "lighting": lighting,
            "grid": {"type": "boolean", "description": "Cell lines and coordinates on the margins, as in view"}
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
        tool("export_foundry", "Export the map for Foundry VTT and Universal VTT into a folder: picture, scene JSON with walls, doors and lights, .dd2vtt. In Foundry the walls of every floor (with their shadow, windows, door frames and the above_walls layer) go into the level's own foreground image, above furniture and tokens and below roofs; objects are tiles on the floor that carry their own shadows, so a moved tile takes its shadow along; bushes lie on the ground under tokens; roofs and tree crowns sit at the top of their own floor. The picture of an upper floor is transparent where nothing is painted on it, and the floors below show through (level visibility); their tiles belong to the upper level too. Every roof gets a surface region (core behaviour defineSurface) along its eaves at the roof's elevation, and the roof tile uses SURFACE occlusion: the roof opens over a token inside, under the mouse and while Alt is held, and nothing flies, shines or looks through it. The floor of every upper storey is a surface too: a ceiling for the storey below and a floor for its own; stairs pass through.", json!({"dir": text("Folder"), "px": int("Pixels per cell, default 140")}), &["dir"]),
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
        for n in ["reload_assets", "render_picture", "set_wall_style", "add_path", "edit_label", "edit_roof", "add_roof", "set_scene", "add_level", "switch_level", "grow_map", "edit_door", "preview_assets", "batch", "check_map"] {
            assert!(names.contains(&n), "{n}");
        }
        let props = |tool: &str| list.as_array().unwrap().iter().find(|t| t["name"] == tool).unwrap()["inputSchema"]["properties"].clone();
        for k in ["shadow", "canopy", "tint", "recolor", "flip"] {
            assert!(props("add_object").get(k).is_some() && props("edit_object").get(k).is_some(), "{k}");
        }
        for k in ["asset", "frame", "a", "b", "width"] {
            assert!(props("add_door").get(k).is_some(), "add_door {k}");
        }
        assert!(props("paint").get("feather").is_some() && props("paint")["shape"]["enum"].as_array().unwrap().iter().any(|v| v == "poly"));
        assert!(props("set_scene").get("shadows").is_some());
        for k in ["mode", "height", "feather", "face", "points", "natural"] {
            assert!(props("relief").get(k).is_some(), "relief {k}");
        }
        assert!(props("find_assets").get("exclude").is_some() && props("view").get("grid").is_some());
        for k in ["front", "look_at", "against_wall", "inset"] {
            assert!(props("add_object").get(k).is_some() && props("edit_object").get(k).is_some(), "{k}");
        }
        assert!(props("view").get("lighting").is_some() && props("render_picture").get("lighting").is_some());
        assert!(props("add_roof").get("points").is_some() && props("edit_roof").get("points").is_some() && props("paint").get("angle").is_some());
        // Крыша по контуру приходит без area — строгий клиент не должен её отклонять.
        let roof = list.as_array().unwrap().iter().find(|t| t["name"] == "add_roof").unwrap();
        assert!(roof["inputSchema"]["required"].as_array().is_none_or(|r| r.is_empty()), "add_roof: area не обязательна");
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
