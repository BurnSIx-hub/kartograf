"""Самопроверка карты в открытом окне: смысл предметов, трубы, наложения, двери, стены, крыши.

python -I check.py — печатает нарушения и итог (0 — чисто). Дома — по крышам:
прямоугольник `area` или контур `points`. Первый раз — «Воздушная гавань» Иерихона.
"""
import json, math, os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())

_, t = call("describe", include=["objects", "roofs", "doors", "labels", "walls"], limit=3000)
d = json.loads(t)
objs, roofs, doors, labels, walls = d["objects"], d.get("roofs", []), d.get("doors", []), d.get("labels", []), d.get("walls", [])
_, t = call("map_info")
W, H = json.loads(t)["width"], json.loads(t)["height"]
issues = []
ROOMS = {}


def in_poly(p, poly):
    x, y = p; c = False
    for i in range(len(poly)):
        x1, y1 = poly[i]; x2, y2 = poly[(i + 1) % len(poly)]
        if (y1 > y) != (y2 > y) and x < x1 + (y - y1) * (x2 - x1) / (y2 - y1):
            c = not c
    return c


def room_of(p):
    for n, poly in ROOMS.items():
        if in_poly(p, poly):
            return n
    return None


def box(o, k=1.0, pad=0.0):
    a = math.radians(o["rotation"] or 0)
    w, h = o["w"] * o.get("scale", 1) * k, o["h"] * o.get("scale", 1) * k
    hw = (abs(w * math.cos(a)) + abs(h * math.sin(a))) / 2 + pad
    hh = (abs(w * math.sin(a)) + abs(h * math.cos(a))) / 2 + pad
    return (o["x"] - hw, o["y"] - hh, o["x"] + hw, o["y"] + hh)


def corners(o, k=0.8):
    """Углы видимой части (холст FA с запасом под тень — берётся 80 %)."""
    a = math.radians(o["rotation"] or 0)
    w, h = o["w"] * o.get("scale", 1) * k / 2, o["h"] * o.get("scale", 1) * k / 2
    out = []
    for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
        dx, dy = sx * w, sy * h
        out.append((o["x"] + dx * math.cos(a) - dy * math.sin(a), o["y"] + dx * math.sin(a) + dy * math.cos(a)))
    return out


def inside_box(p, b, pad=0.0):
    return b[0] - pad <= p[0] <= b[2] + pad and b[1] - pad <= p[1] <= b[3] + pad


def roof_at(p, pad=0.0):
    for r in roofs:
        if "points" in r and r["points"]:
            if in_poly(p, r["points"]):
                return True
        elif inside_box(p, r["area"], pad):
            return True
    return False


def deck_at(p):
    """Плоская крыша — палуба, терраса: по ней ходят, на ней стоит что угодно."""
    for r in roofs:
        if r.get("shape") != "flat":
            continue
        if r.get("points") and in_poly(p, r["points"]) or not r.get("points") and r.get("area") and inside_box(p, r["area"]):
            return True
    return False


for _r in roofs:
    if _r.get("points"):
        ROOMS[f"дом {_r['index']}"] = _r["points"]
    elif _r.get("area"):
        _x0, _y0, _x1, _y1 = _r["area"]
        ROOMS[f"дом {_r['index']}"] = [[_x0, _y0], [_x1, _y0], [_x1, _y1], [_x0, _y1]]


has = lambda o, *ws: any(w.lower() in o["name"].lower() for w in ws)
name = lambda o: o["name"]
# Мост, мостки, помост — пол над водой (как в Картографе, `bridge_by_name`): на нём стоят и ходят.
bridge = lambda o: any(w in ("bridge", "мост") for w in "".join(c if c.isalnum() else " " for c in o["name"].lower()).split())
ROOF_OK = ("Chimney", "Steam", "Vent", "Hatch", "Skylight", "Roof_A1_", "Roof_A2_", "Water_Tank", "Pipe_Tank", "Pipe_Brass", "Signal_Beacon", "Antenna", "Weathervane")
# На фасаде снаружи, у самой стены: настенный фонарь, бра, вывеска, тент над витриной.
FACADE = ("Wall_Lamp", "Sconce", "Hanging_Sign", "Tarp_Awning")
FLYING = ("Airship_Gnome", "Airship_Cargo", "Bird", "Balloon")
VERTICAL = ("Clock_Metal_Brass_Face", "Clock_Metal_Brass_Hand", "Wall_Trophy")
CONNECT = ("Boiler", "Tank", "Reactor", "Chimney", "Valve", "Manhole", "Gauge", "Gas_Holder", "Mooring_Mast", "Gas_Cylinders")
INDOOR = ("Table_Round", "Table_Rectangle", "Bookshelf", "Desk", "Stove", "Oven", "Cupboard", "Chandelier",
          "Wall_Lamp", "Bar_Wood", "Stool", "Keg", "Shelf_", "Fireplace", "Arranged_Bed", "Globe", "Telescope", "Map_", "Hammer_Rack",
          "Forge", "Anvil", "Scaffolding", "Engine_Steam", "Sail_Table", "Envelope_Frame")
INDOOR = tuple(w for w in INDOOR if w != "Telescope")   # подзорная труба и на смотровой площадке
OUTDOOR = ("Lamp_Street", "Mooring_Post", "Windsock", "Gyrocopter", "Landing_Pad", "Signal_Beacon", "Cart_Wood",
           "Лавка", "Ящик с цветами", "Sign_Post", "MineCart", "Track_", "Crane_Steam", "Mooring_Mast", "Gas_Holder", "Winch_Metal",
           "Automaton", "Net_01", "Plant_Pot", "Table_Wrought", "Chair_Wrought", "Sign_Wood")
# Бывают и под крышей: лебёдка подъёмника, автожир в мастерской, комнатный цветок, страж-автоматон в банке.
OUTDOOR = tuple(w for w in OUTDOOR if w not in ("Gyrocopter", "Winch_Metal", "Automaton", "Plant_Pot", "Track_"))  # и рельсы рудника в цех

# 1. Крыши и полёт
for o in objs:
    if 800 <= o["layer"] < 900:
        if not has(o, *ROOF_OK) and not deck_at((o["x"], o["y"])):
            issues.append(f"[крыша] #{o['index']} {name(o)} — такому не место на крыше")
        if not roof_at((o["x"], o["y"]), 0.1):
            issues.append(f"[крыша] #{o['index']} {name(o)} на крыше, но не над крышей ({o['x']:.1f}, {o['y']:.1f})")
    # Подъёмный кран не летает, но стрелой выше всего — Картограф ставит его над крышами сам.
    if o["layer"] >= 900 and not has(o, *FLYING, "Crane_"):
        issues.append(f"[крыша] #{o['index']} {name(o)} над крышами, а это не летающее")
    if has(o, *FLYING) and not has(o, "Bird_House", "Birdcage") and o["layer"] < 900:
        issues.append(f"[крыша] #{o['index']} {name(o)} летающее, а слой {o['layer']}")
    if o["layer"] >= 900:
        for c in corners(o):
            if roof_at(c):
                issues.append(f"[крыша] #{o['index']} {name(o)} висит над домом — закроет комнату в Foundry"); break
# 2. Вертикальное плашмя
for o in objs:
    if has(o, *VERTICAL):
        issues.append(f"[плашмя] #{o['index']} {name(o)}")
# 3. Трубы: концы
pipes = [o for o in objs if o["name"].startswith("Pipe_Brass_Straight")]
ends = []
for o in pipes:
    a = math.radians(o["rotation"] or 0)
    L = o["h"] / 2
    dx, dy = -math.sin(a) * L, math.cos(a) * L
    ends += [(o, (o["x"] + dx, o["y"] + dy)), (o, (o["x"] - dx, o["y"] - dy))]
# Колено: при повороте 0 выходы вниз и вправо, на краю клетки.
for o in objs:
    if o["name"].startswith("Pipe_Brass_Elbow"):
        a = math.radians(o["rotation"] or 0)
        for ux, uy in ((0, 0.5), (0.5, 0)):
            ends.append((o, (o["x"] + ux * math.cos(a) - uy * math.sin(a), o["y"] + ux * math.sin(a) + uy * math.cos(a))))


def on_pipe(e, q):
    """Конец лежит на другом куске той же трубы — внахлёст, а не в пустоте."""
    a = math.radians(q["rotation"] or 0)
    ax, ay = -math.sin(a), math.cos(a)
    u = (e[0] - q["x"]) * ax + (e[1] - q["y"]) * ay
    v = abs((e[0] - q["x"]) * ay - (e[1] - q["y"]) * ax)
    return abs(u) <= q["h"] / 2 + 0.05 and v < 0.2


for o, e in ends:
    if any(q is not o and math.dist(e, f) < 0.35 for q, f in ends):
        continue
    if any(q is not o and on_pipe(e, q) for q in pipes):
        continue
    if any(q is not o and has(q, *CONNECT) and inside_box(e, box(q, 1, 0.3)) for q in objs):
        continue
    issues.append(f"[труба] #{o['index']}: конец ({e[0]:.1f}, {e[1]:.1f}) ни к чему не подключён")
# 4. Наложения одного слоя
SKIP = ("Steam", "Track_", "Shadow_", "Rug", "Pipe_Brass", "Manhole", "Floor_", "Landing_Pad", "Puddle", "Map_", "Lantern", "Toolbox", "Cattail", "Lilypad", "Reeds_", "Skull_Decor",
        "Gauge", "Propeller_Metal", "Box_Luggage_Lid_Leather_Brown_Metal_Brass_A11", "Valve")
# Части одного устройства: лафет и ствол, мачта и кофель-планка вокруг неё.
PARTS = (("Cannon_Base", "Cannon_Metal"), ("Mast_Wood", "Rigging_Mast"), ("Railing_", "Railing_"),  # куски одного ограждения — внахлёст
         ("MineCart", "Ore_Minecart_Fill"), ("Water_Wheel", "Water_Wheel"), ("Bellows", "_Beam_"),  # груз в вагонетке, колесо и ось, вал мехов
         ("Ore_Pile", "Ore_Pile"), ("Slag_", "Slag_"), ("Coal_", "Coal_"), ("Ingot_", "Ingot_"),  # кучи руды, шлака, угля, штабель слитков
         ("_Beam_", "_Beam_"))  # брусья одной крепи: стойки под верхняком
# Кладут поверх другого: сеть на груз, цепь на ящик, подстилку в клетку, саван на доску.
ON_TOP = ("Net_01", "Chain_Metal", "Bedroll", "Shroud", "Bandage", "Candle", "Tang_Metal_Molten")
cand = [o for o in objs if not has(o, *SKIP) and o["w"] * o["h"] >= 1 and o["layer"] < 800 and not o.get("canopy")]


def beam_run(a, b):
    """Куски одного бруса внахлёст (комингс люка, балка): оба — брусья, один поворот, на одной оси."""
    if not (has(a, "_Beam_") and has(b, "_Beam_")):
        return False
    ra, rb = a["rotation"] or 0, b["rotation"] or 0
    if min((ra - rb) % 180, (rb - ra) % 180) > 1:
        return False
    t = math.radians(ra)            # при повороте 0 брус идёт вдоль y
    return abs((b["x"] - a["x"]) * math.cos(t) + (b["y"] - a["y"]) * math.sin(t)) < 0.05


for i, a in enumerate(cand):
    ba = box(a, 0.8)
    for b in cand[i + 1:]:
        if a["layer"] != b["layer"]:
            continue
        # Вещь на помосте — не наложение; два куска моста внахлёст — наложение.
        if bridge(a) != bridge(b):
            continue
        if any(has(a, x) and has(b, y) or has(a, y) and has(b, x) for x, y in PARTS):
            continue
        if has(a, *ON_TOP) or has(b, *ON_TOP):
            continue
        if beam_run(a, b):
            continue
        # Мелочь на столе (книги, свечи, черепа, ступка) — так и задумано.
        if any(has(x, "Table", "Desk", "Counter", "Bar_") for x in (a, b)) and min(a["w"] * a["h"], b["w"] * b["h"]) <= 2:
            continue
        bb = box(b, 0.8)
        ix = max(0, min(ba[2], bb[2]) - max(ba[0], bb[0])) * max(0, min(ba[3], bb[3]) - max(ba[1], bb[1]))
        small = min((ba[2] - ba[0]) * (ba[3] - ba[1]), (bb[2] - bb[0]) * (bb[3] - bb[1]))
        if small > 0 and ix / small > 0.3:
            issues.append(f"[наложение] #{a['index']} {name(a)} и #{b['index']} {name(b)} ({ix / small:.0%})")
# 5. Двери: свободный проход
for dd in doors:
    if dd.get("kind") == "window":
        continue
    c = ((dd["a"][0] + dd["b"][0]) / 2, (dd["a"][1] + dd["b"][1]) / 2)
    (x1, y1), (x2, y2) = dd["a"], dd["b"]
    L2 = (x2 - x1) ** 2 + (y2 - y1) ** 2 or 1
    for o in objs:
        if o["layer"] >= 800 or o.get("canopy") or bridge(o) or has(o, "Rug", "Door_Mat", "Steam", "Track_", "Manhole", "Floor_", "Landing_Pad", "Propeller_Wood", "Wall_Lamp") or has(o, *FACADE):
            continue
        tt = max(0, min(1, ((o["x"] - x1) * (x2 - x1) + (o["y"] - y1) * (y2 - y1)) / L2))
        dseg = math.dist((o["x"], o["y"]), (x1 + tt * (x2 - x1), y1 + tt * (y2 - y1)))
        reach = 0.5 + 0.4 * min(o["w"], o["h"])
        if dseg < reach and o["w"] * o["h"] >= 1:
            issues.append(f"[дверь] у двери #{dd['index']} ({c[0]:.1f}, {c[1]:.1f}) стоит #{o['index']} {name(o)}")
# 6. Предмет через стену дома: углы видимой части по разные стороны стены
for o in objs:
    # Крона над стеной — не предмет поперёк неё: она висит над крышей. Труба входит в дом сквозь стену.
    if o["layer"] >= 800 or o.get("canopy") or bridge(o) or has(o, "Wall_Lamp", "Notice_Board", "Propeller_Wood", "Landing_Pad", "Fireplace", "Oven_Brick", "Pipe_Brass", "Track_", "_Beam_"):
        continue
    if has(o, *FACADE) and roof_at((o["x"], o["y"]), 1.0):
        continue
    rs = {room_of(c) for c in corners(o, 0.7)}
    if len(rs) > 1:
        issues.append(f"[стена] #{o['index']} {name(o)} ({o['x']:.1f}, {o['y']:.1f}) стоит поперёк стены: {rs}")
# 7. Уличное не в домах, домашнее не на улице. Обнесённое стенами со всех сторон — палуба корабля
# под фальшбортом, трюм, двор за оградой — не улица: стены растром, заливка от края карты.
STEP = 0.25
GW, GH = int(W / STEP) + 2, int(H / STEP) + 2
solid = bytearray(GW * GH)
for w in walls:
    pts = w["points"] + ([w["points"][0]] if w.get("closed") else [])
    for (x1, y1), (x2, y2) in zip(pts, pts[1:]):
        n = max(1, int(math.dist((x1, y1), (x2, y2)) / 0.05))
        for k in range(n + 1):
            gx, gy = int((x1 + (x2 - x1) * k / n) / STEP) + 1, int((y1 + (y2 - y1) * k / n) / STEP) + 1
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    if 0 <= gx + dx < GW and 0 <= gy + dy < GH:
                        solid[(gy + dy) * GW + gx + dx] = 1
# Проход в ограде, где стоит трап или лестница, ведёт на другой этаж, а не на улицу.
for o in objs:
    if has(o, "Ladder", "Stairs", "Trapdoor", "Hatch"):
        b = box(o, 0.8)
        for gy in range(int(b[1] / STEP) + 1, int(b[3] / STEP) + 2):
            for gx in range(int(b[0] / STEP) + 1, int(b[2] / STEP) + 2):
                if 0 <= gx < GW and 0 <= gy < GH:
                    solid[gy * GW + gx] = 1
outside = bytearray(GW * GH)
todo = [i for i in range(GW)] + [(GH - 1) * GW + i for i in range(GW)] + [j * GW for j in range(GH)] + [j * GW + GW - 1 for j in range(GH)]
for i in todo:
    outside[i] = 1
while todo:
    i = todo.pop()
    for j in (i - 1, i + 1, i - GW, i + GW):
        if 0 <= j < GW * GH and not outside[j] and not solid[j] and abs(j % GW - i % GW) <= 1:
            outside[j] = 1
            todo.append(j)
walled = lambda x, y: not outside[(int(y / STEP) + 1) * GW + int(x / STEP) + 1] if 0 <= x < W and 0 <= y < H else False
for o in objs:
    if o["layer"] >= 800:
        continue
    r = room_of((o["x"], o["y"]))
    if r and has(o, *OUTDOOR):
        issues.append(f"[улица] #{o['index']} {name(o)} внутри: {r}")
    if not r and has(o, *INDOOR) and not walled(o["x"], o["y"]) and not (has(o, *FACADE) and roof_at((o["x"], o["y"]), 1.0)):
        issues.append(f"[дом] #{o['index']} {name(o)} на улице ({o['x']:.1f}, {o['y']:.1f})")
# 8. За краем карты
for o in objs:
    if has(o, "Tree", "Дерево") or o.get("canopy"):
        continue  # крона за краем карты — нормально
    if has(o, "Track_", "Shadow_"):
        continue  # рельсы и темнота штольни уходят за край — рудник там продолжается
    b = box(o, 0.8)
    if b[0] < -0.2 or b[1] < -0.2 or b[2] > W + 0.2 or b[3] > H + 0.2:
        issues.append(f"[край] #{o['index']} {name(o)} вылезает за карту")
# 9. Подписи не на предметах
for l in labels:
    hw, hh = len(l["text"]) * l["size"] * 0.29, l["size"] * 0.6
    lb = (l["x"] - hw, l["y"] - hh, l["x"] + hw, l["y"] + hh)
    for o in objs:
        if o["layer"] >= 800 or has(o, "Track_", "Steam", "Pipe_Brass"):
            continue
        if any(inside_box(c, lb) for c in corners(o)) and o["w"] * o["h"] >= 1:
            issues.append(f"[подпись] «{l['text']}» лежит на #{o['index']} {name(o)}")
# 10. Настенная лампа — у стены
for o in objs:
    if has(o, "Wall_Lamp"):
        near = False
        segs = [(poly[i], poly[(i + 1) % len(poly)]) for poly in ROOMS.values() for i in range(len(poly))]
        segs += [(w['points'][i], w['points'][i + 1]) for w in walls for i in range(len(w['points']) - 1)]
        for (x1, y1), (x2, y2) in segs:
            if True:
                L2 = (x2 - x1) ** 2 + (y2 - y1) ** 2
                tt = max(0, min(1, ((o["x"] - x1) * (x2 - x1) + (o["y"] - y1) * (y2 - y1)) / L2))
                if math.dist((o["x"], o["y"]), (x1 + tt * (x2 - x1), y1 + tt * (y2 - y1))) < 0.7:
                    near = True
        if not near:
            issues.append(f"[лампа] #{o['index']} настенная лампа не у стены")

for s in issues:
    print(s)
print(f"ИТОГО нарушений: {len(issues)} (объектов {len(objs)}, крыш {len(roofs)}, дверей {len(doors)})")
p.stdin.close(); p.wait()
