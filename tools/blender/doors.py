"""Створки дверей для 3D-карт — модели Blender с картами PBR.

    blender --background --factory-startup --python tools/blender/doors.py -- <папка вывода> [<папка снимков>] [--only=wood] [--tex=1024] [--samples=32]

Пишет `door_wood.glb`, `door_metal.glb`, `door_stone.glb`: створка в клетках (Blender: 1 = клетка,
5 футов) — петли на x = 0, другой край на x = 1, низ на z = 0, верх на z = 1,4 (дверь 7 футов),
толщина 0,08 по y вокруг нуля. Картограф растягивает створку по проёму (`src/doors3d.rs`).

Створка — один кусок с одним материалом (3D Canvas читает метки только у такого узла), у него три
карты в одной развёртке: цвет, нормали и ORM (затенение, шероховатость, металл в R, G, B, как в
glTF). Их запекает Cycles из процедурных материалов деталей: дерево с волокнами, сучками и
износом, кованое железо с вмятинами и ржавчиной, листовая сталь, тёсаный камень со сколами;
грязь в углах — по затенению, стёртые рёбра — по скруглению (узел Bevel). Картинки в `.glb` —
WebP без потерь: в сжатии с потерями каналы карт перемешиваются.

Картограф красит тело (у дерева и камня — всё, что не металл; у металлической — всю створку) в
цвет двери с карты, сохраняя разброс тонов; железо — как есть. С папкой снимков — ещё снимки
каждой створки и сами карты для проверки глазами.
"""
import math
import os
import random
import sys

import bpy

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402
from kit import ONLY, SHOTS, bake_maps, bar, box, cyl, dome, export_glb, hole, iron, join, plate, reset, ring, steel, stone, wood  # noqa: E402

W, H, T = 1.0, 1.4, 0.08
OUT = kit.out_dir("assets/doors3d")


# --- Створки -----------------------------------------------------------------------------------

def ring_handle(m, x, z, side=-1):
    """Кольцо на розетке и накладка с замочной скважиной ниже; `side` −1 — спереди (−y), 1 — сзади."""
    y = side * T / 2
    out = lambda d: y + side * d
    lo, hi = sorted((y, out(0.006)))
    parts = [
        cyl("rosette", m["iron"], (x, out(0.004), z), 0.05, 0.008, verts=24, bevel=0.003),
        cyl("pin", m["iron"], (x, out(0.014), z), 0.012, 0.02, verts=12),
        ring("ring", m["iron"], (x, out(0.024), z - 0.045), 0.045, 0.0075),
        plate("keyplate", m["iron"], [(x - 0.02, z - 0.215), (x + 0.02, z - 0.215), (x + 0.022, z - 0.17), (x + 0.02, z - 0.115), (x - 0.02, z - 0.115), (x - 0.022, z - 0.17)], lo, hi, 0.0025),
    ]
    k0, k1 = sorted((out(0.006), out(0.0065)))
    parts.append(cyl("keyhole", m["hole"], (x, (k0 + k1) / 2, z - 0.15), 0.0075, k1 - k0, verts=12))
    parts.append(box("keyslot", m["hole"], x - 0.004, x + 0.004, k0, k1, z - 0.185, z - 0.15))
    return parts


def wood_door():
    rnd = random.Random(7)
    m = {"plank": wood("wood"), "batten": wood("wood_h", 0.0), "iron": iron(), "hole": hole()}
    parts = []
    # Пять досок разной ширины с зазорами, концы чуть неровные.
    widths = [0.205, 0.19, 0.212, 0.196, 0.197]
    k = W / sum(widths)
    x = 0.0
    for i, w in enumerate(widths):
        w *= k
        parts.append(box(f"plank{i}", m["plank"], x + 0.003, x + w - 0.003, -T / 2, T / 2 - 0.022, rnd.uniform(0.0, 0.006), H - rnd.uniform(0.0, 0.008), bevel=0.006))
        x += w
    # Сзади — две поперечины и раскос (дверь держится на них).
    yb0, yb1 = T / 2 - 0.022, T / 2
    parts.append(box("batten_low", m["batten"], 0.05, 0.95, yb0, yb1, 0.18, 0.31, bevel=0.006))
    parts.append(box("batten_high", m["batten"], 0.05, 0.95, yb0, yb1, 1.09, 1.22, bevel=0.006))
    a, b = (0.12, 0.30), (0.88, 1.10)
    m["brace"] = wood("wood_b", math.atan2(b[1] - a[1], b[0] - a[0]))
    parts.append(bar("brace", m["brace"], a, b, 0.1, yb0, yb1 - 0.003))
    # Спереди — железные полосы петель с наконечником-пикой, гвозди; петли у края.
    y = -T / 2
    for zc in (0.245, 1.155):
        strap = [(0.0, -0.026), (0.54, -0.026), (0.59, -0.044), (0.672, 0.0), (0.59, 0.044), (0.54, 0.026), (0.0, 0.026)]
        parts.append(plate("strap", m["iron"], [(sx, zc + sz) for sx, sz in strap], y - 0.007, y, 0.0025))
        for xn in (0.07, 0.2, 0.33, 0.46, 0.6):
            parts.append(dome("nail", m["iron"], (xn, y - 0.007, zc), 0.011))
        parts.append(cyl("knuckle", m["iron"], (0.0, 0.0, zc), 0.022, 0.11, axis="Z", verts=14, bevel=0.003))
    # Гвозди раскоса — по доске.
    x = 0.0
    for w in widths:
        w *= k
        cx = x + w / 2
        t = (cx - a[0]) / (b[0] - a[0])
        if 0.05 < t < 0.95:
            parts.append(dome("nail", m["iron"], (cx, y, a[1] + t * (b[1] - a[1])), 0.01))
        x += w
    parts += ring_handle(m, 0.84, 0.74)
    # Сзади — скоба.
    parts.append(box("grip", m["iron"], 0.82, 0.86, T / 2 + 0.008, T / 2 + 0.022, 0.62, 0.86, bevel=0.004))
    parts.append(box("grip_a", m["iron"], 0.82, 0.86, T / 2, T / 2 + 0.022, 0.62, 0.65, bevel=0.002))
    parts.append(box("grip_b", m["iron"], 0.82, 0.86, T / 2, T / 2 + 0.022, 0.83, 0.86, bevel=0.002))
    return parts, m


def metal_door():
    m = {"steel": steel(), "iron": iron(), "hole": hole()}
    parts = [box("sheet", m["steel"], 0.004, W - 0.004, -T / 2 + 0.008, T / 2 - 0.008, 0.0, H, bevel=0.01)]
    for side in (-1, 1):
        y_out = side * (T / 2)
        y_in = side * (T / 2 - 0.008)
        ys = (min(y_in, y_out), max(y_in, y_out))
        # Рама по краю и две филёнки; бруски рамы не заходят друг в друга (иначе чёрные стыки).
        parts.append(box("rim_l", m["steel"], 0.004, 0.08, *ys, 0.0, H, bevel=0.006))
        parts.append(box("rim_r", m["steel"], W - 0.08, W - 0.004, *ys, 0.0, H, bevel=0.006))
        parts.append(box("rim_t", m["steel"], 0.082, W - 0.082, *ys, H - 0.08, H, bevel=0.006))
        parts.append(box("rim_b", m["steel"], 0.082, W - 0.082, *ys, 0.0, 0.08, bevel=0.006))
        parts.append(box("rim_m", m["steel"], 0.082, W - 0.082, *ys, 0.66, 0.74, bevel=0.006))
        for zc in (0.04, 0.7, H - 0.04):
            for xr in [0.04 + k * 0.0915 for k in range(11)]:
                parts.append(dome("rivet", m["steel"], (xr, y_out + side * 0.002, zc), 0.012, verts=8))
        for zr in [0.16 + k * 0.1 for k in range(5)] + [0.82 + k * 0.1 for k in range(5)]:
            for xr in (0.04, W - 0.04):
                parts.append(dome("rivet", m["steel"], (xr, y_out + side * 0.002, zr), 0.012, verts=8))
    # Ручка-рычаг с накладкой с обеих сторон, спереди — скважина.
    for side in (-1, 1):
        y = side * T / 2
        lo, hi = sorted((y, y + side * 0.008))
        parts.append(box("lockplate", m["iron"], 0.80, 0.90, lo, hi, 0.6, 0.84, bevel=0.004))
        parts.append(cyl("hub", m["iron"], (0.85, y + side * 0.018, 0.76), 0.016, 0.022, verts=14, bevel=0.002))
        lo, hi = sorted((y + side * 0.024, y + side * 0.036))
        parts.append(box("lever", m["iron"], 0.71, 0.86, lo, hi, 0.748, 0.772, bevel=0.004))
        k0, k1 = sorted((y + side * 0.008, y + side * 0.0085))
        parts.append(cyl("keyhole", m["hole"], (0.85, (k0 + k1) / 2, 0.665), 0.0075, k1 - k0, verts=12))
        parts.append(box("keyslot", m["hole"], 0.846, 0.854, k0, k1, 0.63, 0.665))
    for zc in (0.2, 1.2):
        parts.append(cyl("knuckle", m["iron"], (0.0, 0.0, zc), 0.024, 0.14, axis="Z", verts=14, bevel=0.003))
    return parts, m


def stone_door():
    m = {"stone": stone(), "iron": iron(), "hole": hole()}
    parts = [box("slab", m["stone"], 0.004, W - 0.004, -T / 2 + 0.01, T / 2 - 0.01, 0.0, H, bevel=0.012, seg=3)]
    for side in (-1, 1):
        ys = sorted((side * (T / 2 - 0.012), side * (T / 2 + 0.006)))
        # Выпуклая рама и ромб посередине — резьба (скос меньше толщины, иначе она схлопнется).
        parts.append(box("frame_l", m["stone"], 0.06, 0.14, *ys, 0.06, H - 0.06, bevel=0.004))
        parts.append(box("frame_r", m["stone"], W - 0.14, W - 0.06, *ys, 0.06, H - 0.06, bevel=0.004))
        parts.append(box("frame_t", m["stone"], 0.142, W - 0.142, *ys, H - 0.14, H - 0.06, bevel=0.004))
        parts.append(box("frame_b", m["stone"], 0.142, W - 0.142, *ys, 0.06, 0.14, bevel=0.004))
        d = box("diamond", m["stone"], -0.16, 0.16, *ys, -0.16, 0.16, bevel=0.004)
        d.rotation_euler = (0, math.pi / 4, 0)
        d.location = (W / 2, d.location.y, H / 2 + 0.05)
        bpy.ops.object.select_all(action="DESELECT")
        d.select_set(True)
        bpy.context.view_layer.objects.active = d
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=False)
        parts.append(d)
    parts += ring_handle(m, 0.84, 0.74, -1)
    parts += ring_handle(m, 0.84, 0.74, 1)
    for zc in (0.25, 1.15):
        parts.append(cyl("pivot", m["iron"], (0.0, 0.0, zc), 0.026, 0.12, axis="Z", verts=14, bevel=0.003))
    return parts, m


def shoot(s, kind):
    """Снимки готовой створки: спереди, сзади и вблизи."""
    kit.light_rig(s)
    co = kit.camera(s, (520, 640))
    for side, at, aim in (("front", (W / 2 - 0.45, -3.4, H / 2 + 0.35), (W / 2, 0.0, H / 2)), ("back", (W / 2 + 0.45, 3.4, H / 2 + 0.35), (W / 2, 0.0, H / 2)), ("close", (0.95, -0.75, 0.95), (0.62, 0.0, 0.7))):
        kit.snap(s, co, os.path.join(SHOTS, f"door_{kind}_{side}.png"), at, aim)


for kind, make in (("wood", wood_door), ("metal", metal_door), ("stone", stone_door)):
    if ONLY and kind != ONLY:
        continue
    s = reset()
    parts, mats = make()
    door = join(parts, f"door_{kind}")
    bake_maps(kind, door, list(mats.values()), "door")
    path = os.path.join(OUT, f"door_{kind}.glb")
    tris = export_glb(door, path)
    print(f"DOOR {kind}: {path} {os.path.getsize(path)} bytes, ~{tris} triangles", flush=True)
    if SHOTS:
        shoot(s, kind)
