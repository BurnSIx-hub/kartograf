"""Окна для 3D-карт — модели Blender с картами PBR.

    blender --background --factory-startup --python tools/blender/windows.py -- <папка вывода> [<папка снимков>] [--only=wood] [--tex=1024] [--samples=32]

Пишет `window_wood.glb`, `window_stone.glb` и `glass.glb`. Окно — проём в клетках (Blender: 1 =
клетка): x от 0 до 1 вдоль стены, z от 0 (подоконник, 3 фута над полом) до 0,8 (верх проёма,
7 футов), по y — стена толщиной 2·R вокруг нуля; стекло — плоскость y = 0. Картограф растягивает
окно по проёму и по толщине стены (`src/windows3d.rs`): у краёв (откосы, рама) — как есть, середина
тянется; откосы идут до лица стены, подоконник выступает за него, как в модели.

- деревянное: откосы и подоконник из досок, рама из двух створок с фрамугой, переплёты, железные
  петли и вертушка;
- каменное: подоконник, откосы из тёсаных блоков, перемычка, средник и поперечина (крестовое
  окно), свинцовые переплёты ромбом, железные прутья.

Рама — один кусок с одним материалом и тремя картами (как створки дверей, `doors.py`). Стекло
(`glass.glb`) — плоскость с бесшовной картинкой на клетку: цвет с прозрачностью, нормали (рябь
старого стекла, пузырьки), ORM (гладкое, мутнее пятнами); Картограф кладёт его по проёму.
"""
import math
import os
import sys

import bpy
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402
from kit import ONLY, SHOTS, Graph, bake_maps, bar, box, cyl, export_glb, iron, join, reset, stone, wood  # noqa: E402

W, H = 1.0, 0.8
# Полутолщина стены в модели: откосы — от −R до R.
R = 0.125

OUT = kit.out_dir("assets/windows3d")


def lead():
    """Свинцовые жилки переплёта: тёмный тусклый металл с белёсым налётом пятнами."""
    g = Graph("lead")
    p = g.pos
    ox = g.noise(p, 22.0, 6.0)
    fine = g.noise(p, 220.0, 3.0)
    white = g.smooth(ox, 0.55, 0.75)
    normal, e = g.edge(0.002, 0.004)
    base = g.ramp(fine, [(0.3, (0.045, 0.047, 0.05)), (0.7, (0.085, 0.087, 0.09))])
    base = g.mixc(g.mul(white, 0.55), base, (0.3, 0.3, 0.28))
    base = g.mixc(g.mul(e, 0.4), base, (0.16, 0.16, 0.17))
    metal = g.math("SUBTRACT", 0.85, g.mul(white, 0.6))
    rough = g.clamp(g.add(0.55, g.mul(white, 0.3), g.mul(fine, 0.1), g.mul(e, -0.15)))
    height = g.add(g.mul(fine, 0.4), g.mul(white, 0.3))
    return g.done(base, rough, metal, height, 0.3, 0.0005, normal)


# --- Окна ------------------------------------------------------------------------------------

def wood_window():
    m = {"v": wood("wood_v", rings=True), "h": wood("wood_h", 0.0, rings=True), "iron": iron()}
    parts = []
    lin, sill = 0.03, 0.035
    # Подоконник — доска с выступом за лицо стены; откосы и верх — доски до лица стены.
    parts.append(box("sill", m["h"], 0.0, W, -R - 0.05, R + 0.05, 0.0, sill, bevel=0.008))
    parts.append(box("jamb_l", m["v"], 0.0, lin, -R, R, sill, H - lin, bevel=0.004, seg=1))
    parts.append(box("jamb_r", m["v"], W - lin, W, -R, R, sill, H - lin, bevel=0.004, seg=1))
    parts.append(box("head", m["h"], 0.0, W, -R, R, H - lin, H, bevel=0.004, seg=1))
    # Рама посередине толщины: две створки встык по средней стойке, над ними фрамуга.
    f, st = 0.028, 0.045
    x0, x1, z0, z1 = lin, W - lin, sill, H - lin
    mid, mw = W / 2, 0.03
    rb, rt = z0 + 0.06, z1 - 0.045
    t0, t1 = 0.5, 0.545
    parts.append(box("stile_l", m["v"], x0, x0 + st, -f, f, z0, z1, bevel=0.004, seg=1))
    parts.append(box("stile_r", m["v"], x1 - st, x1, -f, f, z0, z1, bevel=0.004, seg=1))
    parts.append(box("rail_b", m["h"], x0 + st, x1 - st, -f, f, z0, rb, bevel=0.004, seg=1))
    parts.append(box("rail_t", m["h"], x0 + st, x1 - st, -f, f, rt, z1, bevel=0.004, seg=1))
    parts.append(box("mullion", m["v"], mid - mw, mid + mw, -f, f, rb, rt, bevel=0.004, seg=1))
    for a, b in ((x0 + st, mid - mw), (mid + mw, x1 - st)):
        parts.append(box("transom", m["h"], a, b, -f, f, t0, t1, bevel=0.004, seg=1))
        # Переплёты: внизу крестом (четыре стекла), во фрамуге — пополам.
        gb, gd = 0.007, 0.012
        cx, cz = (a + b) / 2, (rb + t0) / 2
        parts.append(box("bar_v", m["v"], cx - gb, cx + gb, -gd, gd, rb, t0, bevel=0.003, seg=1))
        parts.append(box("bar_h", m["h"], a, cx - gb, -gd, gd, cz - gb, cz + gb, bevel=0.003, seg=1))
        parts.append(box("bar_h", m["h"], cx + gb, b, -gd, gd, cz - gb, cz + gb, bevel=0.003, seg=1))
        parts.append(box("bar_u", m["v"], cx - gb, cx + gb, -gd, gd, t1, rt, bevel=0.003, seg=1))
    # Железо: петли снаружи (−y) на крайних стойках, вертушка изнутри на средней.
    for x, sgn in ((x0, 1), (x1, -1)):
        for zc in (z0 + 0.1, t0 - 0.07):
            xa, xb = sorted((x + sgn * 0.004, x + sgn * 0.075))
            parts.append(box("hinge", m["iron"], xa, xb, -f - 0.004, -f, zc - 0.014, zc + 0.014, bevel=0.0015, seg=1))
            parts.append(cyl("knuckle", m["iron"], (x, -f - 0.004, zc), 0.007, 0.032, axis="Z", verts=10))
    parts.append(box("latch_plate", m["iron"], mid - 0.012, mid + 0.012, f, f + 0.004, 0.27, 0.33, bevel=0.0015, seg=1))
    parts.append(cyl("latch_pin", m["iron"], (mid, f + 0.008, 0.3), 0.005, 0.01, verts=10))
    parts.append(box("latch", m["iron"], mid - 0.045, mid + 0.045, f + 0.012, f + 0.017, 0.293, 0.307, bevel=0.002, seg=1))
    return parts, m


def clip(p, d, rect):
    """Отрезок прямой `p + t·d` внутри прямоугольника `rect` (x0, z0, x1, z1) или `None`."""
    lo, hi = -1e9, 1e9
    for k, (a, b) in enumerate(((rect[0], rect[2]), (rect[1], rect[3]))):
        if abs(d[k]) < 1e-9:
            if not a <= p[k] <= b:
                return None
            continue
        t0, t1 = sorted(((a - p[k]) / d[k], (b - p[k]) / d[k]))
        lo, hi = max(lo, t0), min(hi, t1)
    if hi - lo < 1e-4:
        return None
    return (p[0] + lo * d[0], p[1] + lo * d[1]), (p[0] + hi * d[0], p[1] + hi * d[1])


def lattice(m, rect, dw=0.095, dh=0.14):
    """Свинцовые жилки ромбом по свету `rect` (x0, z0, x1, z1): прямые обоих наклонов через
    середину света с шагом в ромб."""
    cx, cz = (rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2
    span = (rect[2] - rect[0]) / dw + (rect[3] - rect[1]) / dh
    parts = []
    for sgn in (1, -1):
        d = (dw, sgn * dh)
        n = int(span) // 2 + 1
        for k in range(-n, n + 1):
            # Сдвиг вдоль x на ромб — соседняя прямая того же наклона.
            seg = clip((cx + k * dw, cz), d, rect)
            if seg and math.dist(*seg) > 0.01:
                parts.append(bar("came", m["lead"], seg[0], seg[1], 0.008, -0.005, 0.005, bevel=0.0))
    # По краю света — жилка рамкой (держит стекло в пазу камня).
    x0, z0, x1, z1 = rect
    for a, b in (((x0, z0 + 0.004), (x1, z0 + 0.004)), ((x0, z1 - 0.004), (x1, z1 - 0.004)), ((x0 + 0.004, z0), (x0 + 0.004, z1)), ((x1 - 0.004, z0), (x1 - 0.004, z1))):
        parts.append(bar("came_edge", m["lead"], a, b, 0.008, -0.006, 0.006, bevel=0.0))
    return parts


def stone_window():
    m = {"stone": stone("stone", veins=False), "lead": lead(), "iron": iron()}
    parts = []
    jw, sill, top = 0.06, 0.05, 0.07
    parts.append(box("sill", m["stone"], 0.0, W, -R - 0.06, R + 0.06, 0.0, sill, bevel=0.01))
    # Откосы — по три тёсаных блока со швами, сверху — перемычка во всю толщину.
    cuts = [sill, 0.27, 0.5, H - top]
    for xa, xb in ((0.0, jw), (W - jw, W)):
        for i in range(3):
            z0 = cuts[i] + (0.0015 if i else 0.0)
            parts.append(box("jamb", m["stone"], xa, xb, -R, R, z0, cuts[i + 1] - 0.0015, bevel=0.008, seg=1))
    parts.append(box("lintel", m["stone"], 0.0, W, -R, R, H - top + 0.0015, H, bevel=0.008, seg=1))
    # Средник и поперечина со скошенными рёбрами — крест посередине толщины.
    mid, mw = W / 2, 0.04
    t0, t1 = 0.49, 0.55
    parts.append(box("mullion", m["stone"], mid - mw, mid + mw, -0.07, 0.07, sill, H - top, bevel=0.014, seg=1))
    for a, b in ((jw, mid - mw), (mid + mw, W - jw)):
        parts.append(box("transom", m["stone"], a, b, -0.06, 0.06, t0, t1, bevel=0.012, seg=1))
        parts += lattice(m, (a, sill, b, t0))
        parts += lattice(m, (a, t1, b, H - top))
        # Железный прут поперёк нижнего света изнутри.
        parts.append(cyl("saddle", m["iron"], ((a + b) / 2, 0.012, (sill + t0) / 2), 0.0045, b - a, axis="X", verts=8))
    return parts, m


# --- Стекло ----------------------------------------------------------------------------------

def periodic(rnd, n, f0, f1):
    """Бесшовный шум n×n: белый шум, пропущенный по частотам от `f0` до `f1` раз на картинку;
    среднее 0, разброс 1."""
    w = np.fft.fft2(rnd.standard_normal((n, n)))
    f = np.fft.fftfreq(n) * n
    r = np.hypot(f[None, :], f[:, None])
    mid, width = math.log(math.sqrt(f0 * f1)), max(math.log(f1 / f0) / 2, 0.2)
    band = np.exp(-((np.log(np.maximum(r, 1e-6)) - mid) ** 2) / (2 * width * width))
    band[0, 0] = 0.0
    out = np.real(np.fft.ifft2(w * band))
    return (out - out.mean()) / (out.std() + 1e-9)


def glass_maps(n=512, seed=11):
    """Картинки стекла на клетку (строки снизу вверх, как `pixels` Blender): цвет с прозрачностью
    (sRGB), нормали (OpenGL), ORM."""
    rnd = np.random.default_rng(seed)
    ripple = periodic(rnd, n, 2.0, 5.0)
    swirl = periodic(rnd, n, 7.0, 15.0)
    fine = periodic(rnd, n, 40.0, 90.0)
    smudge = periodic(rnd, n, 1.0, 3.0)
    tint = periodic(rnd, n, 1.0, 4.0)
    # Пузырьки — мелкие купола, бесшовно (по модулю картинки).
    yy, xx = np.mgrid[0:n, 0:n]
    bub = np.zeros((n, n))
    for _ in range(14):
        cx, cy, rad = rnd.uniform(0, n), rnd.uniform(0, n), rnd.uniform(1.0, 2.6)
        dx = (xx - cx + n / 2) % n - n / 2
        dy = (yy - cy + n / 2) % n - n / 2
        bub = np.maximum(bub, np.clip(1.0 - (dx * dx + dy * dy) / (rad * rad), 0.0, 1.0))
    height = ripple * 1.0 + swirl * 0.3 + fine * 0.04 + bub * 0.6
    gx = (np.roll(height, -1, 1) - np.roll(height, 1, 1)) / 2
    gy = (np.roll(height, -1, 0) - np.roll(height, 1, 0)) / 2
    k = 1.6
    nrm = np.stack([-k * gx, -k * gy, np.ones_like(height)], -1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)
    normal = np.concatenate([nrm * 0.5 + 0.5, np.ones((n, n, 1))], -1)
    dirt = np.clip(smudge * 0.5 + 0.3, 0.0, 1.0)
    shade = 1.0 + 0.05 * tint
    rgb = np.stack([0.70 * shade, 0.79 * shade, 0.75 * shade], -1) * (1.0 - 0.12 * dirt[..., None]) + bub[..., None] * 0.03
    alpha = np.clip(0.24 + 0.14 * dirt + 0.05 * bub, 0.0, 1.0)
    base = np.concatenate([np.clip(rgb, 0, 1), alpha[..., None]], -1)
    rough = np.clip(0.05 + 0.3 * dirt ** 1.5 + 0.1 * bub, 0.0, 1.0)
    orm = np.stack([np.ones((n, n)), rough, np.zeros((n, n)), np.ones((n, n))], -1)
    return base, normal, orm


def glass_material(n=512):
    base, normal, orm = glass_maps(n)
    imgs = {}
    for name, arr, color in (("glass_base", base, True), ("glass_normal", normal, False), ("glass_orm", orm, False)):
        img = bpy.data.images.new(name, n, n, alpha=True, float_buffer=not color)
        img.colorspace_settings.name = "sRGB" if color else "Non-Color"
        img.pixels.foreach_set(arr.astype(np.float32).ravel())
        img.update()
        imgs[name] = img
        if SHOTS:
            img.filepath_raw = os.path.join(SHOTS, f"window_{name}.png")
            img.file_format = "PNG"
            img.save()
    fm = bpy.data.materials.new("glass")
    fm.use_nodes = True
    fm.surface_render_method = "BLENDED"
    nt = fm.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    tb = nt.nodes.new("ShaderNodeTexImage")
    tb.image = imgs["glass_base"]
    nt.links.new(tb.outputs["Color"], bsdf.inputs["Base Color"])
    nt.links.new(tb.outputs["Alpha"], bsdf.inputs["Alpha"])
    tn = nt.nodes.new("ShaderNodeTexImage")
    tn.image = imgs["glass_normal"]
    nm = nt.nodes.new("ShaderNodeNormalMap")
    nt.links.new(tn.outputs["Color"], nm.inputs["Color"])
    nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
    to = nt.nodes.new("ShaderNodeTexImage")
    to.image = imgs["glass_orm"]
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    nt.links.new(to.outputs["Color"], sep.inputs["Color"])
    nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
    nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
    nt.links.new(sep.outputs["Red"], kit.gltf_output(nt).inputs["Occlusion"])
    return fm


def plane(name, mat, x0, x1, z0, z1, uv_scale=1.0):
    """Плоскость y = 0 лицом к −y; развёртка — по месту, клетка на картинку."""
    me = bpy.data.meshes.new(name)
    me.from_pydata([(x0, 0, z0), (x1, 0, z0), (x1, 0, z1), (x0, 0, z1)], [], [(0, 1, 2, 3)])
    uv = me.uv_layers.new(name="UVMap")
    for li, (x, z) in enumerate(((x0, z0), (x1, z0), (x1, z1), (x0, z1))):
        uv.data[li].uv = (x * uv_scale, z * uv_scale)
    me.materials.append(mat)
    o = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(o)
    return o


# --- Снимки ----------------------------------------------------------------------------------

def shoot(s, kind, glass):
    """Окно в куске стены со стеклом: снаружи, изнутри и вблизи."""
    wall = bpy.data.materials.new("plaster")
    wall.use_nodes = True
    wall.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.42, 0.38, 0.33, 1)
    wall.node_tree.nodes["Principled BSDF"].inputs["Roughness"].default_value = 0.9
    for a in ((-0.7, 0.0, -0.6, H + 0.6), (W, W + 0.7, -0.6, H + 0.6), (0.0, W, -0.6, 0.0), (0.0, W, H, H + 0.6)):
        o = box("wall", type("M", (), {"m": wall}), a[0], a[1], -R, R, a[2], a[3])
        o.data.materials.clear()
        o.data.materials.append(wall)
    plane("glass", glass, 0.0, W, 0.0, H)
    # Тёмная комната за окном: видно, как стекло отражает и пропускает.
    room = bpy.data.materials.new("room")
    room.use_nodes = True
    room.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.05, 0.045, 0.04, 1)
    o = box("room", type("M", (), {"m": room}), -1.5, 2.5, 1.2, 1.3, -0.6, H + 1.2)
    o.data.materials.clear()
    o.data.materials.append(room)
    kit.light_rig(s)
    co = kit.camera(s, (640, 560))
    for side, at, aim in (("front", (W / 2 - 0.5, -2.6, H / 2 + 0.3), (W / 2, 0.0, H / 2)), ("back", (W / 2 + 0.4, 1.1, H / 2 + 0.15), (W / 2, 0.0, H / 2)), ("close", (0.85, -0.55, 0.7), (0.4, 0.0, 0.35))):
        kit.snap(s, co, os.path.join(SHOTS, f"window_{kind}_{side}.png"), at, aim)


for kind, make in (("wood", wood_window), ("stone", stone_window)):
    if ONLY and kind != ONLY:
        continue
    s = reset()
    parts, mats = make()
    win = join(parts, f"window_{kind}")
    bake_maps(kind, win, list(mats.values()), "window")
    path = os.path.join(OUT, f"window_{kind}.glb")
    tris = export_glb(win, path)
    print(f"WINDOW {kind}: {path} {os.path.getsize(path)} bytes, ~{tris} triangles", flush=True)
    if SHOTS:
        shoot(s, kind, glass_material())

if not ONLY or ONLY == "glass":
    reset()
    g = plane("glass", glass_material(), 0.0, 1.0, 0.0, 1.0)
    path = os.path.join(OUT, "glass.glb")
    export_glb(g, path)
    print(f"WINDOW glass: {path} {os.path.getsize(path)} bytes", flush=True)
