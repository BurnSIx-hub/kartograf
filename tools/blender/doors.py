"""Створки дверей для 3D-карт — модели Blender.

    blender --background --factory-startup --python tools/blender/doors.py -- <папка вывода> [<папка снимков>]

Пишет `door_wood.glb`, `door_metal.glb`, `door_stone.glb`: створка в клетках (Blender: 1 = клетка,
5 футов) — петли на x = 0, другой край на x = 1, низ на z = 0, верх на z = 1,4 (дверь 7 футов),
толщина 0,08 по y вокруг нуля. Картограф растягивает створку по проёму (`src/doors3d.rs`).

Две части: тело с серой запечённой фактурой (дерево, металл, камень — Картограф красит её в цвет
породы из имени рисунка двери) и железо одним цветом (петли, кольцо, заклёпки). С папкой снимков —
ещё по снимку каждой створки спереди и сзади для проверки глазами.
"""
import math
import os
import sys
import tempfile

import bpy
import mathutils

W, H, T = 1.0, 1.4, 0.08
TEX = 512

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = os.path.abspath(argv[0] if argv else "assets/doors3d")
SHOTS = os.path.abspath(argv[1]) if len(argv) > 1 else None
os.makedirs(OUT, exist_ok=True)


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    s = bpy.context.scene
    s.unit_settings.system = "NONE"
    return s


def box(name, x0, x1, y0, y1, z0, z1, bevel=0.0, seg=2):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2))
    o = bpy.context.active_object
    o.name = name
    o.scale = (x1 - x0, y1 - y0, z1 - z0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if bevel > 0:
        m = o.modifiers.new("bevel", "BEVEL")
        m.width = bevel
        m.segments = seg
        m.limit_method = "ANGLE"
    return o


def cyl(name, at, r, depth, axis="Y", verts=16):
    rot = {"X": (0, math.pi / 2, 0), "Y": (math.pi / 2, 0, 0), "Z": (0, 0, 0)}[axis]
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=r, depth=depth, location=at, rotation=rot)
    o = bpy.context.active_object
    o.name = name
    return o


def dome(name, at, r, face=-1, verts=8):
    """Шляпка гвоздя, заклёпка: полусфера лицом по ±y."""
    bpy.ops.mesh.primitive_uv_sphere_add(segments=verts, ring_count=4, radius=r, location=at)
    o = bpy.context.active_object
    o.name = name
    o.scale = (1, 0.5, 1)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return o


def ring(name, at, major, minor):
    """Кольцо ручки: тор в плоскости xz (висит на створке)."""
    bpy.ops.mesh.primitive_torus_add(major_radius=major, minor_radius=minor, major_segments=20, minor_segments=6, location=at, rotation=(math.pi / 2, 0, 0))
    o = bpy.context.active_object
    o.name = name
    return o


def bar(name, a, b, width, y0, y1):
    """Брусок от точки `a` до `b` в плоскости xz (раскос двери)."""
    (ax, az), (bx, bz) = a, b
    length = math.hypot(bx - ax, bz - az)
    o = box(name, -length / 2, length / 2, y0, y1, -width / 2, width / 2, bevel=0.004)
    o.rotation_euler = (0, -math.atan2(bz - az, bx - ax), 0)
    o.location = ((ax + bx) / 2, 0, (az + bz) / 2)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=False)
    return o


def join(objs, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
        bpy.context.view_layer.objects.active = o
        for m in list(o.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    o = bpy.context.active_object
    o.name = name
    return o


def node(nodes, kind, x, y):
    n = nodes.new(kind)
    n.location = (x, y)
    return n


def grain(kind):
    """Процедурная фактура тела (серая, 0,55–1): выход «цвет» узла смешения."""
    m = bpy.data.materials.new(f"{kind}_grain")
    m.use_nodes = True
    nt = m.node_tree
    nodes, links = nt.nodes, nt.links
    for n in list(nodes):
        nodes.remove(n)
    coord = node(nodes, "ShaderNodeTexCoord", -900, 0)
    out = node(nodes, "ShaderNodeOutputMaterial", 600, 0)
    emit = node(nodes, "ShaderNodeEmission", 400, 0)
    ramp = node(nodes, "ShaderNodeValToRGB", 150, 0)
    if kind == "wood":
        # Волокна вдоль досок (по z): полосы по x, искривлённые шумом, и мелкая рябь.
        mapping = node(nodes, "ShaderNodeMapping", -700, 0)
        mapping.inputs["Scale"].default_value = (1.0, 1.0, 0.08)
        wave = node(nodes, "ShaderNodeTexWave", -450, 100)
        wave.wave_type = "BANDS"
        wave.bands_direction = "X"
        wave.inputs["Scale"].default_value = 9.0
        wave.inputs["Distortion"].default_value = 6.0
        wave.inputs["Detail"].default_value = 4.0
        wave.inputs["Detail Scale"].default_value = 1.5
        fine = node(nodes, "ShaderNodeTexNoise", -450, -150)
        fine.inputs["Scale"].default_value = 140.0
        fine.inputs["Detail"].default_value = 6.0
        mix = node(nodes, "ShaderNodeMix", -150, 0)
        mix.data_type = "FLOAT"
        mix.inputs["Factor"].default_value = 0.3
        links.new(coord.outputs["Object"], mapping.inputs["Vector"])
        links.new(mapping.outputs["Vector"], wave.inputs["Vector"])
        links.new(coord.outputs["Object"], fine.inputs["Vector"])
        links.new(wave.outputs["Fac"], mix.inputs["A"])
        links.new(fine.outputs["Fac"], mix.inputs["B"])
        src = mix.outputs["Result"]
        stops = [(0.0, 0.3), (0.55, 0.62), (1.0, 1.0)]
    elif kind == "metal":
        # Кованый лист: крупные пятна и мелкая шероховатость.
        big = node(nodes, "ShaderNodeTexNoise", -450, 100)
        big.inputs["Scale"].default_value = 7.0
        big.inputs["Detail"].default_value = 8.0
        big.inputs["Roughness"].default_value = 0.65
        fine = node(nodes, "ShaderNodeTexNoise", -450, -150)
        fine.inputs["Scale"].default_value = 220.0
        mix = node(nodes, "ShaderNodeMix", -150, 0)
        mix.data_type = "FLOAT"
        mix.inputs["Factor"].default_value = 0.35
        links.new(coord.outputs["Object"], big.inputs["Vector"])
        links.new(coord.outputs["Object"], fine.inputs["Vector"])
        links.new(big.outputs["Fac"], mix.inputs["A"])
        links.new(fine.outputs["Fac"], mix.inputs["B"])
        src = mix.outputs["Result"]
        stops = [(0.25, 0.42), (0.55, 0.7), (0.8, 1.0)]
    else:
        # Тёсаный камень: ячейки и зернистость.
        vor = node(nodes, "ShaderNodeTexVoronoi", -450, 100)
        vor.inputs["Scale"].default_value = 14.0
        fine = node(nodes, "ShaderNodeTexNoise", -450, -150)
        fine.inputs["Scale"].default_value = 90.0
        fine.inputs["Detail"].default_value = 8.0
        mix = node(nodes, "ShaderNodeMix", -150, 0)
        mix.data_type = "FLOAT"
        mix.inputs["Factor"].default_value = 0.55
        links.new(coord.outputs["Object"], vor.inputs["Vector"])
        links.new(coord.outputs["Object"], fine.inputs["Vector"])
        links.new(vor.outputs["Distance"], mix.inputs["A"])
        links.new(fine.outputs["Fac"], mix.inputs["B"])
        src = mix.outputs["Result"]
        stops = [(0.2, 0.42), (0.5, 0.7), (0.85, 1.0)]
    els = ramp.color_ramp.elements
    while len(els) > 1:
        els.remove(els[-1])
    els[0].position = stops[0][0]
    els[0].color = (stops[0][1],) * 3 + (1,)
    for pos, v in stops[1:]:
        e = els.new(pos)
        e.color = (v, v, v, 1)
    links.new(src, ramp.inputs["Fac"])
    links.new(ramp.outputs["Color"], emit.inputs["Color"])
    links.new(emit.outputs["Emission"], out.inputs["Surface"])
    return m


def bake_body(body, kind):
    """Развёртка тела и запекание фактуры в картинку; материал — картинка в основном цвете."""
    bpy.ops.object.select_all(action="DESELECT")
    body.select_set(True)
    bpy.context.view_layer.objects.active = body
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.01)
    bpy.ops.object.mode_set(mode="OBJECT")
    img = bpy.data.images.new(f"door_{kind}", TEX, TEX, alpha=False)
    img.colorspace_settings.name = "sRGB"
    m = grain(kind)
    target = m.node_tree.nodes.new("ShaderNodeTexImage")
    target.image = img
    m.node_tree.nodes.active = target
    body.data.materials.clear()
    body.data.materials.append(m)
    s = bpy.context.scene
    s.render.engine = "CYCLES"
    s.cycles.device = "CPU"
    s.cycles.samples = 1
    s.render.bake.margin = 8
    bpy.ops.object.bake(type="EMIT")
    # Картинка — внутрь .glb; файлом — только рядом со снимками (для проверки глазами).
    img.filepath_raw = os.path.join(SHOTS or tempfile.gettempdir(), f"door_{kind}_grain.png")
    img.file_format = "PNG"
    img.save()
    img.pack()
    # Готовый материал: картинка — основной цвет.
    final = bpy.data.materials.new(f"{kind}")
    final.use_nodes = True
    nt = final.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = img
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = {"wood": 0.8, "metal": 0.45, "stone": 0.9}[kind]
    bsdf.inputs["Metallic"].default_value = 0.6 if kind == "metal" else 0.0
    body.data.materials.clear()
    body.data.materials.append(final)
    bpy.data.materials.remove(m)


def iron_material():
    m = bpy.data.materials.new("iron")
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = (0.045, 0.042, 0.04, 1)
    b.inputs["Metallic"].default_value = 0.85
    b.inputs["Roughness"].default_value = 0.5
    return m


def ring_handle(x, z, side=-1):
    """Кольцо ручки с накладкой и замочная накладка ниже; `side` −1 — спереди (−y), 1 — сзади."""
    y = side * T / 2
    out = lambda d: y + side * d
    return [
        cyl("plate", (x, out(0.004), z), 0.05, 0.008, verts=20),
        cyl("pin", (x, out(0.014), z), 0.012, 0.02, verts=10),
        ring("ring", (x, out(0.024), z - 0.045), 0.045, 0.0075),
        box("keyplate", x - 0.018, x + 0.018, min(y, out(0.006)), max(y, out(0.006)), z - 0.2, z - 0.12, bevel=0.003),
    ]


def wood_door():
    planks, iron = [], []
    n = 5
    pw = W / n
    for i in range(n):
        planks.append(box(f"plank{i}", i * pw + 0.003, (i + 1) * pw - 0.003, -T / 2, T / 2 - 0.022, 0.0, H, bevel=0.007))
    # Сзади — две поперечины и раскос (дверь держится на них).
    yb0, yb1 = T / 2 - 0.022, T / 2
    planks.append(box("batten_low", 0.05, 0.95, yb0, yb1, 0.18, 0.31, bevel=0.006))
    planks.append(box("batten_high", 0.05, 0.95, yb0, yb1, 1.09, 1.22, bevel=0.006))
    planks.append(bar("brace", (0.12, 0.30), (0.88, 1.10), 0.1, yb0, yb1 - 0.003))
    # Спереди — железные полосы петель с гвоздями и петли у края.
    y = -T / 2
    for zc in (0.245, 1.155):
        iron.append(box("strap", 0.0, 0.64, y - 0.007, y, zc - 0.026, zc + 0.026, bevel=0.003))
        iron.append(cyl("strap_end", (0.64, y - 0.0035, zc), 0.036, 0.007, verts=16))
        for xn in (0.08, 0.22, 0.36, 0.5, 0.64):
            iron.append(dome("nail", (xn, y - 0.007, zc), 0.011))
        iron.append(cyl("knuckle", (0.0, 0.0, zc), 0.022, 0.11, axis="Z", verts=12))
    iron += ring_handle(0.84, 0.74)
    # Сзади — скоба.
    iron.append(box("grip", 0.82, 0.86, T / 2 + 0.008, T / 2 + 0.022, 0.62, 0.86, bevel=0.004))
    iron.append(box("grip_a", 0.82, 0.86, T / 2, T / 2 + 0.022, 0.62, 0.65))
    iron.append(box("grip_b", 0.82, 0.86, T / 2, T / 2 + 0.022, 0.83, 0.86))
    return planks, iron


def metal_door():
    body, iron = [], []
    body.append(box("sheet", 0.004, W - 0.004, -T / 2 + 0.008, T / 2 - 0.008, 0.0, H, bevel=0.01))
    for side in (-1, 1):
        y_out = side * (T / 2)
        y_in = side * (T / 2 - 0.008)
        ys = (min(y_in, y_out), max(y_in, y_out))
        # Рама по краю и две филёнки.
        # Бруски рамы не заходят друг в друга: в местах наложения грани спорили бы (чёрные углы).
        body.append(box("rim_l", 0.004, 0.08, *ys, 0.0, H, bevel=0.006))
        body.append(box("rim_r", W - 0.08, W - 0.004, *ys, 0.0, H, bevel=0.006))
        body.append(box("rim_t", 0.082, W - 0.082, *ys, H - 0.08, H, bevel=0.006))
        body.append(box("rim_b", 0.082, W - 0.082, *ys, 0.0, 0.08, bevel=0.006))
        body.append(box("rim_m", 0.082, W - 0.082, *ys, 0.66, 0.74, bevel=0.006))
        for zc in (0.04, 0.7, H - 0.04):
            for xr in [0.04 + k * 0.0915 for k in range(11)]:
                iron.append(dome("rivet", (xr, y_out + side * 0.002, zc), 0.012))
        for zr in [0.16 + k * 0.1 for k in range(5)] + [0.82 + k * 0.1 for k in range(5)]:
            for xr in (0.04, W - 0.04):
                iron.append(dome("rivet", (xr, y_out + side * 0.002, zr), 0.012))
    # Ручка-рычаг с накладкой с обеих сторон.
    for side in (-1, 1):
        y = side * T / 2
        iron.append(box("lockplate", 0.80, 0.90, min(y, y + side * 0.008), max(y, y + side * 0.008), 0.6, 0.84, bevel=0.004))
        iron.append(cyl("hub", (0.85, y + side * 0.018, 0.76), 0.016, 0.022, verts=12))
        iron.append(box("lever", 0.71, 0.86, min(y + side * 0.024, y + side * 0.036), max(y + side * 0.024, y + side * 0.036), 0.748, 0.772, bevel=0.004))
    for zc in (0.2, 1.2):
        iron.append(cyl("knuckle", (0.0, 0.0, zc), 0.024, 0.14, axis="Z", verts=12))
    return body, iron


def stone_door():
    body, iron = [], []
    body.append(box("slab", 0.004, W - 0.004, -T / 2 + 0.01, T / 2 - 0.01, 0.0, H, bevel=0.012, seg=3))
    for side in (-1, 1):
        ys = sorted((side * (T / 2 - 0.012), side * (T / 2 + 0.006)))
        # Выпуклая рама и ромб посередине — резьба (скос меньше толщины, иначе она схлопнется).
        body.append(box("frame_l", 0.06, 0.14, *ys, 0.06, H - 0.06, bevel=0.004))
        body.append(box("frame_r", W - 0.14, W - 0.06, *ys, 0.06, H - 0.06, bevel=0.004))
        body.append(box("frame_t", 0.142, W - 0.142, *ys, H - 0.14, H - 0.06, bevel=0.004))
        body.append(box("frame_b", 0.142, W - 0.142, *ys, 0.06, 0.14, bevel=0.004))
        d = box("diamond", -0.16, 0.16, *ys, -0.16, 0.16, bevel=0.004)
        d.rotation_euler = (0, math.pi / 4, 0)
        d.location = (W / 2, d.location.y, H / 2 + 0.05)
        bpy.ops.object.select_all(action="DESELECT")
        d.select_set(True)
        bpy.context.view_layer.objects.active = d
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=False)
        body.append(d)
    iron += ring_handle(0.84, 0.74, -1)
    iron += ring_handle(0.84, 0.74, 1)
    for zc in (0.25, 1.15):
        iron.append(cyl("pivot", (0.0, 0.0, zc), 0.026, 0.12, axis="Z", verts=12))
    return body, iron


def light_rig(s):
    w = bpy.data.worlds.new("w")
    w.use_nodes = True
    w.node_tree.nodes["Background"].inputs["Color"].default_value = (0.35, 0.38, 0.42, 1)
    w.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
    s.world = w
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 3.5
    o = bpy.data.objects.new("sun", sun)
    o.rotation_euler = (math.radians(50), math.radians(-25), math.radians(-30))
    s.collection.objects.link(o)


def shoot(s, kind, objs):
    """Снимки створки спереди и сзади (Cycles, мало проходов)."""
    light_rig(s)
    cam = bpy.data.cameras.new("cam")
    cam.lens = 50
    co = bpy.data.objects.new("cam", cam)
    s.collection.objects.link(co)
    s.camera = co
    s.render.engine = "CYCLES"
    s.cycles.samples = 24
    s.render.resolution_x, s.render.resolution_y = 520, 640
    s.render.film_transparent = False
    for side, y in (("front", -1), ("back", 1)):
        co.location = (W / 2 + 0.9 * y * 0.5, y * 3.4, H / 2 + 0.35)
        aim = mathutils.Vector((W / 2, 0.0, H / 2)) - co.location
        co.rotation_euler = aim.to_track_quat("-Z", "Y").to_euler()
        s.render.filepath = os.path.join(SHOTS, f"door_{kind}_{side}.png")
        bpy.ops.render.render(write_still=True)


for kind, make in (("wood", wood_door), ("metal", metal_door), ("stone", stone_door)):
    s = reset()
    body_parts, iron_parts = make()
    body = join(body_parts, kind)
    bake_body(body, kind)
    iron = join(iron_parts, "iron")
    iron.data.materials.clear()
    iron.data.materials.append(iron_material())
    for o in (body, iron):
        bpy.ops.object.select_all(action="DESELECT")
        o.select_set(True)
        bpy.context.view_layer.objects.active = o
        bpy.ops.object.shade_smooth_by_angle(angle=math.radians(35))
    bpy.ops.object.select_all(action="DESELECT")
    body.select_set(True)
    iron.select_set(True)
    path = os.path.join(OUT, f"door_{kind}.glb")
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True, export_apply=True, export_yup=True, export_image_format="AUTO", export_animations=False)
    tris = sum(len(p.vertices) - 2 for o in (body, iron) for p in o.data.polygons)
    print(f"DOOR {kind}: {path} {os.path.getsize(path)} bytes, ~{tris} triangles")
    if SHOTS:
        os.makedirs(SHOTS, exist_ok=True)
        shoot(s, kind, (body, iron))
