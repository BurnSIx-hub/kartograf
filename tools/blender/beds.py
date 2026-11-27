"""Кровати для 3D-карт — модели Blender с картами PBR к рисункам кроватей FA.

    blender --background --factory-startup --python tools/blender/beds.py -- <папка вывода> [<папка снимков>] [--only=single] [--tex=1024] [--samples=32]

Пишет `bed_<вид>.glb` в `assets/beds3d`:

- `single`, `double` — кровать: столбики с навершиями, спинки, царги, матрас, одеяло с отворотом,
  подушки;
- `hammock` — гамак на распорках: полотно, деревянные распорки с обмоткой, верёвки к кольцам;
- `gathered` — гамак без распорок: полотно, собранное к узлам, подушка;
- `bedroll` — скатка, расстеленная, с ремнями; `roll` — скатка (спальник) свёрнутая, с ремнями;
- `sleeping_bag` — спальный мешок с капюшоном; `mattress` — матрас; `hay` — подстилка из сена;
- `cot` — детская кроватка с решёткой; `stretcher` — носилки.

Единицы — клетки, z — вверх. Изголовье — к +y (в glTF — к −z, к верху рисунка); гамак и свёрнутая
скатка лежат вдоль x. Модель — один кусок с тремя картами в одной развёртке (`kit.bake_maps`), но
три материала с одними и теми же картами: `cloth` (ткань, сено — Картограф кладёт на неё сам
рисунок FA сверху: цвет одеяла, мех, вещи на постели), `wood` (перекрашивается по породе из имени
рисунка) и `keep` (верёвка, кожа, железо — как есть). Ткань запекается нейтрально-серой: тон
даёт рисунок, модель — складки, плетение и затенение.
"""
import math
import os
import sys

import bpy
import mathutils

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kit  # noqa: E402
from kit import ONLY, SHOTS, Graph, bake_maps, box, cyl, export_glb, iron, join, reset, wood  # noqa: E402

OUT = kit.out_dir("assets/beds3d")
noise = mathutils.noise.noise


# --- Материалы ---------------------------------------------------------------------------------

def cloth(name="cloth", scale=55.0):
    """Ткань: нейтрально-серая (цвет даёт рисунок), полотняное плетение, ворс, пятна, затенение
    складок."""
    g = Graph(name)
    p = g.pos
    weave = g.add(*(g.wave(p, scale, 1.2, 2.0, 1.0, d) for d in ("X", "Y", "Z")))
    weave = g.mul(weave, 1 / 3)
    fuzz = g.noise(p, 240.0, 3.0)
    mott = g.noise(p, 5.0, 5.0)
    occl = g.ao(0.06)
    v = g.add(0.5, g.mul(weave, 0.06), g.mul(mott, 0.1), g.mul(fuzz, 0.05))
    base = g.mixc(g.mul(g.math("SUBTRACT", 1.0, occl), 0.55), (0.5, 0.5, 0.5), (0.2, 0.2, 0.2))
    base = g.scalec(base, g.mul(v, 2.0))
    rough = g.clamp(g.add(0.86, g.mul(fuzz, 0.08)))
    height = g.add(g.mul(weave, 0.6), g.mul(fuzz, 0.3), g.mul(mott, 0.2))
    return g.done(base, rough, 0.0, height, 0.35, 0.0007, g.gnormal)


def straw():
    """Сено: соломины в три стороны, щели между ними темнее (цвет даёт рисунок)."""
    g = Graph("straw")
    p = g.pos
    lines = []
    for a in (0.0, 1.1, 2.2):
        q = g.rotate(p, (0.3 * a, 0.5, a))
        lines.append(g.smooth(g.noise(g.vec(q, (1.0, 0.035, 1.0)), 70.0, 2.0), 0.45, 0.62))
    strands = g.math("MAXIMUM", g.math("MAXIMUM", lines[0], lines[1]), lines[2])
    occl = g.ao(0.05)
    v = g.add(0.36, g.mul(strands, 0.24), g.mul(g.noise(p, 8.0, 4.0), 0.1))
    base = g.mixc(g.mul(g.math("SUBTRACT", 1.0, occl), 0.7), (0.5, 0.5, 0.5), (0.12, 0.12, 0.12))
    base = g.scalec(base, g.mul(v, 2.0))
    rough = g.clamp(g.add(0.7, g.mul(strands, -0.12)))
    return g.done(base, rough, 0.0, strands, 0.8, 0.003, g.gnormal)


def rope():
    """Пеньковая верёвка: витки наискосок, волокна, светлые и тёмные пряди."""
    g = Graph("rope")
    p = g.pos
    twist = g.wave(p, 90.0, 3.0, 2.0, 1.0, "DIAGONAL")
    fibre = g.noise(g.vec(p, (1.0, 1.0, 0.15)), 380.0, 2.0)
    base = g.ramp(g.add(g.mul(twist, 0.7), g.mul(fibre, 0.3)), [(0.25, (0.1, 0.07, 0.035)), (0.75, (0.36, 0.27, 0.15))])
    base = g.mixc(g.mul(g.math("SUBTRACT", 1.0, g.ao(0.02)), 0.6), base, g.scalec(base, 0.4))
    return g.done(base, g.clamp(g.add(0.82, g.mul(fibre, 0.1))), 0.0, g.add(twist, g.mul(fibre, 0.4)), 0.7, 0.0015, g.gnormal)


def leather():
    """Ремни: бурая кожа, зерно, потёртые светлые края."""
    g = Graph("leather")
    p = g.pos
    cells, _ = g.voronoi(p, 260.0)
    mott = g.noise(p, 22.0, 6.0)
    normal, e = g.edge(0.003, 0.006)
    base = g.ramp(mott, [(0.3, (0.05, 0.025, 0.012)), (0.7, (0.13, 0.068, 0.032))])
    base = g.mixc(g.mul(e, 0.6), base, g.scalec(base, 1.9))
    rough = g.clamp(g.add(0.55, g.mul(mott, 0.15), g.mul(e, 0.1)))
    return g.done(base, rough, 0.0, g.add(g.mul(cells, 0.5), g.mul(mott, 0.3)), 0.4, 0.0008, normal)


# --- Детали ------------------------------------------------------------------------------------

def mesh(name, mat, verts, faces):
    me = bpy.data.meshes.new(name)
    me.from_pydata(verts, [], faces)
    me.validate()
    me.update()
    o = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(o)
    o.data.materials.append(mat.m)
    return o


def spacing(n):
    """Точки от −1 до 1, гуще у краёв: скругление края подушки выходит гладким."""
    return [-math.cos(math.pi * i / n) for i in range(n + 1)]


def cushion(name, mat, cx, cy, z0, w, d, top, bottom, p=6.0, nu=20, nv=32, lump=0.0, lump_scale=6.0, dimple=None, seed=0.0):
    """Мягкое: подушка, матрас, тюфяк, куча сена. Верх — `z0 + top·f`, низ — `z0 − bottom·f`,
    где f = √((1 − |s|^p)(1 − |t|^p)): у края верх и низ сходятся скруглением (чем больше `p`, тем
    прямее бока). `lump` — бугры, `dimple` — шаг стёжки матраса."""
    su, sv = spacing(nu), spacing(nv)
    idx = {}
    verts = []

    def f(s, t):
        return math.sqrt(max(0.0, (1 - abs(s) ** p) * (1 - abs(t) ** p)))

    def bump(x, y):
        k = 1.0
        if lump:
            k += lump * noise(mathutils.Vector((x * lump_scale, y * lump_scale, seed)))
        if dimple:
            gx, gy = (x / dimple) % 1.0 - 0.5, (y / dimple) % 1.0 - 0.5
            k -= 0.18 * math.exp(-(gx * gx + gy * gy) / 0.01)
        return k

    for j, t in enumerate(sv):
        for i, s in enumerate(su):
            x, y = cx + s * w / 2, cy + t * d / 2
            idx[("t", i, j)] = len(verts)
            verts.append((x, y, z0 + top * f(s, t) * bump(x, y)))
    for j, t in enumerate(sv):
        for i, s in enumerate(su):
            if 0 < i < nu and 0 < j < nv:
                x, y = cx + s * w / 2, cy + t * d / 2
                idx[("b", i, j)] = len(verts)
                verts.append((x, y, z0 - bottom * f(s, t)))

    def b(i, j):
        return idx.get(("b", i, j), idx[("t", i, j)])

    faces = []
    for j in range(nv):
        for i in range(nu):
            faces.append((idx[("t", i, j)], idx[("t", i + 1, j)], idx[("t", i + 1, j + 1)], idx[("t", i, j + 1)]))
            q = (b(i, j), b(i, j + 1), b(i + 1, j + 1), b(i + 1, j))
            if len(set(q)) >= 3:
                faces.append(tuple(dict.fromkeys(q)))
    return mesh(name, mat, verts, faces)


def sheet(name, mat, fn, nu, nv, thick):
    """Полотно по `fn(s, t) → (x, y, z)` (s, t от 0 до 1) толщиной `thick`."""
    verts = [fn(i / nu, j / nv) for j in range(nv + 1) for i in range(nu + 1)]
    faces = [(j * (nu + 1) + i, j * (nu + 1) + i + 1, (j + 1) * (nu + 1) + i + 1, (j + 1) * (nu + 1) + i) for j in range(nv) for i in range(nu)]
    o = mesh(name, mat, verts, faces)
    m = o.modifiers.new("solid", "SOLIDIFY")
    m.thickness = thick
    m.offset = -1.0
    return o


def rod(name, mat, a, b, r, verts=8):
    """Цилиндр от точки `a` до `b`: верёвка, жердь."""
    a, b = mathutils.Vector(a), mathutils.Vector(b)
    o = cyl(name, mat, (a + b) / 2, r, (b - a).length, axis="Z", verts=verts)
    o.rotation_euler = (b - a).to_track_quat("Z", "Y").to_euler()
    return o


def torus(name, mat, at, major, minor, rot=(0.0, 0.0, 0.0), seg=20):
    bpy.ops.mesh.primitive_torus_add(major_radius=major, minor_radius=minor, major_segments=seg, minor_segments=6, location=at, rotation=rot)
    o = bpy.context.active_object
    o.name = name
    o.data.materials.append(mat.m)
    return o


def knob(name, mat, x, y, z, r):
    """Навершие столбика: точёный шар на шейке."""
    neck = cyl(name + "_neck", mat, (x, y, z + r * 0.35), r * 0.55, r * 0.7, axis="Z", verts=12)
    bpy.ops.mesh.primitive_uv_sphere_add(segments=14, ring_count=8, radius=r, location=(x, y, z + r * 1.4))
    o = bpy.context.active_object
    o.name = name
    o.data.materials.append(mat.m)
    return [neck, o]


def fold(o, rr):
    """Свес ткани за край: `o` дальше края — (насколько вбок, насколько вниз) по скруглению `rr`."""
    if o <= 0:
        return o, 0.0
    arc = math.pi / 2 * rr
    if o < arc:
        th = o / rr
        return rr * math.sin(th), rr * (1 - math.cos(th))
    return rr, rr + (o - arc)


def blanket(name, mat, hx, y0, y1, ztop, rr, hang_side, hang_foot, rr_foot=None, nu=26, nv=30, seed=1.0, thick=0.012):
    """Одеяло на матрасе: верх от `y0` (ноги) до `y1` (отворот) на высоте `ztop`, свисает по
    бокам на `hang_side` (скругление `rr`) и в ногах на `hang_foot` (скругление `rr_foot`: у
    кровати в ногах спинка, одеяло подоткнуто), со складками."""
    rf = rr if rr_foot is None else rr_foot
    ex = math.pi / 2 * rr + hang_side
    ey = math.pi / 2 * rf + hang_foot

    def at(s, t):
        u = -hx - ex + s * 2 * (hx + ex)
        v = y0 - ey + t * (y1 - y0 + ey)
        ox, dzx = fold(abs(u) - hx, rr)
        oy, dzy = fold(y0 - v, rf)
        x = math.copysign(min(abs(u), hx) + max(ox, 0.0), u)
        y = max(v, y0) - max(oy, 0.0)
        z = ztop - dzx - dzy
        # Складки: на верху — мягкие бугры, на свесе — волны вдоль края.
        n = noise(mathutils.Vector((u * 9, v * 9, seed)))
        soft = noise(mathutils.Vector((u * 4.0, v * 3.2, seed + 10.0)))
        crease = (1 - abs(noise(mathutils.Vector((u * 7.0 + v * 3.0, v * 5.0, seed + 20.0))))) ** 4
        on_top = 1.0 if dzx + dzy < rr else 0.35
        z += (0.005 * n + 0.009 * soft + 0.007 * crease) * on_top
        if dzx > rr * 0.5:
            x += math.copysign(0.008 * math.sin(v * 38 + 2 * n) * min(1.0, dzx / (rr + 0.04)), u)
        if dzy > rf * 0.5:
            y -= 0.008 * math.sin(u * 38 + 2 * n) * min(1.0, dzy / (rf + 0.04))
        return (x, y, z)

    return sheet(name, mat, at, nu, nv, thick)


# --- Кровати -----------------------------------------------------------------------------------

def mats():
    return {
        "wx": wood("wood_x", angle=0.0, rings=True),
        "wy": wood("wood_y", rings=True, along_y=True),
        "wz": wood("wood_z", rings=True),
        "cloth": cloth("cloth"),
        "linen": cloth("linen", 80.0),
    }


def bed(width, pillows):
    """Кровать шириной `width`: длина 1,3 клетки, изголовье к +y."""
    m = mats()
    L, W = 1.3, width
    hp, fp = 0.072, 0.064  # столбики изголовья и изножья
    parts = []
    for sx in (-1, 1):
        x = sx * (W / 2 - hp / 2)
        parts.append(box("post_h", m["wz"], x - hp / 2, x + hp / 2, L / 2 - hp, L / 2, 0.0, 0.64, bevel=0.008))
        parts += knob("knob_h", m["wz"], x, L / 2 - hp / 2, 0.64, hp * 0.42)
        x = sx * (W / 2 - fp / 2)
        parts.append(box("post_f", m["wz"], x - fp / 2, x + fp / 2, -L / 2, -L / 2 + fp, 0.0, 0.44, bevel=0.007))
        parts += knob("knob_f", m["wz"], x, -L / 2 + fp / 2, 0.44, fp * 0.42)
        # Царга: доска вдоль кровати между столбиками.
        x = sx * (W / 2 - 0.02)
        parts.append(box("rail", m["wy"], x - 0.016, x + 0.016, -L / 2 + fp, L / 2 - hp, 0.15, 0.27, bevel=0.005))
    # Изголовье: брус сверху, филёнка, брус снизу; изножье — ниже.
    yh = L / 2 - hp / 2
    parts.append(box("head_top", m["wx"], -W / 2 + hp, W / 2 - hp, yh - 0.026, yh + 0.026, 0.55, 0.61, bevel=0.007))
    parts.append(box("head_panel", m["wx"], -W / 2 + hp, W / 2 - hp, yh - 0.012, yh + 0.012, 0.27, 0.55, bevel=0.004))
    parts.append(box("head_low", m["wx"], -W / 2 + hp, W / 2 - hp, yh - 0.022, yh + 0.022, 0.2, 0.27, bevel=0.005))
    yf = -L / 2 + fp / 2
    parts.append(box("foot_top", m["wx"], -W / 2 + fp, W / 2 - fp, yf - 0.022, yf + 0.022, 0.36, 0.41, bevel=0.006))
    parts.append(box("foot_panel", m["wx"], -W / 2 + fp, W / 2 - fp, yf - 0.011, yf + 0.011, 0.2, 0.36, bevel=0.004))
    # Матрас на царгах, одеяло с отворотом, подушки.
    mw, my0, my1 = W - 0.09, -L / 2 + fp + 0.008, L / 2 - hp - 0.008
    mz = 0.32
    parts.append(cushion("mattress", m["linen"], 0.0, (my0 + my1) / 2, mz, mw, my1 - my0, 0.055, 0.05, p=9.0, nu=16, nv=28))
    top = mz + 0.055
    yb = my1 - 0.36
    parts.append(blanket("blanket", m["cloth"], mw / 2 - 0.004, my0 + 0.01, yb, top + 0.006, 0.028, 0.035, 0.0, rr_foot=0.012))
    parts.append(blanket("cuff", m["cloth"], mw / 2 + 0.004, yb - 0.1, yb + 0.004, top + 0.019, 0.03, 0.028, 0.0, nu=22, nv=6, seed=4.0))
    pw = (mw - 0.1) / pillows - 0.04 * (pillows - 1)
    for k in range(pillows):
        cx = -mw / 2 + 0.05 + pw / 2 + k * (pw + 0.08)
        parts.append(cushion("pillow", m["linen"], cx, my1 - 0.14, top + 0.004, pw, 0.22, 0.06, 0.018, p=3.0, nu=14, nv=10, lump=0.12, seed=k + 2.0))
    return parts, m


def hammock():
    """Гамак на распорках вдоль x: полотно между распорками, обмотка, верёвки к кольцам."""
    m = mats()
    m["rope"], m["iron"] = rope(), iron()
    xb, wb, wm, zb, sag, curl = 0.8, 0.3, 0.22, 0.6, 0.2, 0.06
    xa, za = xb + 0.33, zb + 0.32

    def cloth_at(s, t):
        u = -1 + 2 * s
        k = 1 - u * u
        w = wb - (wb - wm) * k
        v = -1 + 2 * t
        x = u * (xb - 0.03)
        y = v * w
        z = zb - sag * k + curl * v * v * (0.4 + 0.6 * k) + 0.004 * noise(mathutils.Vector((x * 8, y * 8, 3.0)))
        return (x, y, z)

    parts = [sheet("cloth", m["cloth"], cloth_at, 40, 14, 0.007)]
    for sx in (-1, 1):
        x = sx * xb
        parts.append(rod("bar", m["wy"], (x, -wb - 0.06, zb + 0.01), (x, wb + 0.06, zb + 0.01), 0.022, verts=12))
        for k in range(5):
            y = -wb + 0.04 + k * (2 * wb - 0.08) / 4
            parts.append(torus("lash", m["rope"], (x, y, zb + 0.01), 0.026, 0.006, rot=(math.pi / 2, 0, 0), seg=12))
        for sy in (-1, 1):
            parts.append(rod("rope", m["rope"], (x, sy * (wb + 0.045), zb + 0.01), (sx * (xa - 0.03), 0.0, za), 0.007, verts=6))
        parts.append(torus("ring", m["iron"], (sx * xa, 0.0, za), 0.03, 0.007, rot=(0, 0, math.pi / 2)))
    return parts, m


def gathered():
    """Гамак без распорок: полотно, собранное к концам в узлы, верёвки к кольцам, подушка."""
    m = mats()
    m["rope"], m["iron"] = rope(), iron()
    xg, wm, zg, sag = 0.86, 0.24, 0.62, 0.26
    xa, za = xg + 0.26, zg + 0.22

    def cloth_at(s, t):
        u = -1 + 2 * s
        k = 1 - u * u
        w = wm * max(k, 0.0) ** 0.6 + 0.012
        v = -1 + 2 * t
        z = zg - sag * k + 0.045 * v * v * k + 0.003 * noise(mathutils.Vector((u * 7, v * 5, 5.0)))
        return (u * xg, v * w, z)

    parts = [sheet("cloth", m["cloth"], cloth_at, 44, 14, 0.006)]
    for sx in (-1, 1):
        bpy.ops.mesh.primitive_uv_sphere_add(segments=12, ring_count=7, radius=0.03, location=(sx * (xg + 0.01), 0.0, zg))
        o = bpy.context.active_object
        o.name = "knot"
        o.data.materials.append(m["rope"].m)
        parts.append(o)
        parts.append(rod("rope", m["rope"], (sx * (xg + 0.03), 0.0, zg + 0.01), (sx * (xa - 0.03), 0.0, za), 0.008, verts=6))
        parts.append(torus("ring", m["iron"], (sx * xa, 0.0, za), 0.03, 0.007, rot=(0, 0, math.pi / 2)))
    zc = zg - sag + 0.006
    parts.append(cushion("pillow", m["linen"], 0.0, 0.0, zc + 0.02, 0.24, 0.2, 0.045, 0.02, p=3.0, nu=12, nv=10, lump=0.15, seed=7.0))
    return parts, m


def bedroll():
    """Расстеленная скатка: тонкий стёганый тюфяк, ремни в ногах лежат на полу."""
    m = mats()
    m["leather"] = leather()
    parts = [cushion("pad", m["cloth"], 0.0, 0.0, 0.014, 0.5, 1.3, 0.026, 0.012, p=5.0, nu=14, nv=30, lump=0.08, dimple=0.16)]
    for sx in (-1, 1):
        x = sx * 0.13
        parts.append(box("strap", m["leather"], x - 0.02, x + 0.02, -0.76, -0.6, 0.0, 0.005, bevel=0.002))
    return parts, m


def roll():
    """Свёрнутая скатка вдоль x: валик, два ремня с пряжками."""
    m = mats()
    m["leather"], m["iron"] = leather(), iron()
    r, half = 0.09, 0.28
    o = cyl("roll", m["cloth"], (0.0, 0.0, r), r, 2 * half, axis="X", verts=28)
    bev = o.modifiers.new("bevel", "BEVEL")
    bev.width, bev.segments, bev.limit_method = 0.03, 3, "ANGLE"
    parts = [o]
    for sx in (-1, 1):
        x = sx * 0.15
        parts.append(cyl("strap", m["leather"], (x, 0.0, r), r + 0.006, 0.034, axis="X", verts=28))
        parts.append(box("buckle", m["iron"], x - 0.022, x + 0.022, -r - 0.012, -r + 0.004, r - 0.02, r + 0.02, bevel=0.003))
    return parts, m


def sleeping_bag():
    """Спальный мешок: толстое тело, отворот и капюшон в изголовье."""
    m = mats()
    parts = [cushion("bag", m["cloth"], 0.0, -0.04, 0.012, 0.5, 1.22, 0.06, 0.012, p=4.0, nu=16, nv=30, lump=0.1, seed=3.0)]
    parts.append(cushion("flap", m["cloth"], 0.0, 0.36, 0.06, 0.46, 0.16, 0.025, 0.012, p=3.0, nu=14, nv=8, lump=0.15, seed=5.0))
    parts.append(cushion("hood", m["linen"], 0.0, 0.6, 0.02, 0.42, 0.22, 0.08, 0.02, p=2.5, nu=14, nv=10, lump=0.12, seed=6.0))
    return parts, m


def mattress():
    m = mats()
    return [cushion("mattress", m["linen"], 0.0, 0.0, 0.05, 0.62, 1.3, 0.05, 0.05, p=10.0, nu=18, nv=32, dimple=0.18)], m


def hay():
    """Подстилка из сена: неровная куча."""
    g = straw()
    return [cushion("hay", g, 0.0, 0.0, 0.0, 1.2, 1.7, 0.16, 0.0, p=2.4, nu=26, nv=34, lump=0.45, lump_scale=3.5, seed=2.0)], {"straw": g}


def cot():
    """Детская кроватка: столбики, перекладины, решётка из балясин, матрасик, подушка."""
    m = mats()
    W, L, H = 0.46, 0.86, 0.56
    p = 0.042
    parts = []
    for sx in (-1, 1):
        for sy in (-1, 1):
            x, y = sx * (W / 2 - p / 2), sy * (L / 2 - p / 2)
            parts.append(box("post", m["wz"], x - p / 2, x + p / 2, y - p / 2, y + p / 2, 0.0, H, bevel=0.006))
            parts += knob("knob", m["wz"], x, y, H, p * 0.4)
    for z0, z1 in ((0.13, 0.17), (H - 0.045, H - 0.005)):
        for sx in (-1, 1):
            x = sx * (W / 2 - p / 2)
            parts.append(box("rail", m["wy"], x - 0.014, x + 0.014, -L / 2 + p, L / 2 - p, z0, z1, bevel=0.004))
        for sy in (-1, 1):
            y = sy * (L / 2 - p / 2)
            parts.append(box("rail", m["wx"], -W / 2 + p, W / 2 - p, y - 0.014, y + 0.014, z0, z1, bevel=0.004))
    n = 9
    for k in range(n):
        y = -L / 2 + p + (k + 0.5) * (L - 2 * p) / n
        for sx in (-1, 1):
            parts.append(cyl("spindle", m["wz"], (sx * (W / 2 - p / 2), y, (0.17 + H - 0.045) / 2), 0.008, H - 0.215, axis="Z", verts=8))
    for k in range(4):
        x = -W / 2 + p + (k + 0.5) * (W - 2 * p) / 4
        for sy in (-1, 1):
            parts.append(cyl("spindle", m["wz"], (x, sy * (L / 2 - p / 2), (0.17 + H - 0.045) / 2), 0.008, H - 0.215, axis="Z", verts=8))
    parts.append(cushion("mattress", m["linen"], 0.0, 0.0, 0.21, W - 0.1, L - 0.1, 0.04, 0.035, p=8.0, nu=12, nv=18))
    parts.append(blanket("blanket", m["cloth"], (W - 0.1) / 2 - 0.004, -L / 2 + 0.06, L / 2 - 0.3, 0.256, 0.02, 0.02, 0.0, rr_foot=0.01, nu=18, nv=18))
    parts.append(cushion("pillow", m["linen"], 0.0, L / 2 - 0.14, 0.254, 0.24, 0.13, 0.04, 0.012, p=3.0, nu=10, nv=8, lump=0.12, seed=9.0))
    return parts, m


def stretcher():
    """Носилки: две жерди, полотно между ними, поперечины и ножки."""
    m = mats()
    xp, half, z = 0.25, 0.78, 0.11

    def canvas(s, t):
        u, v = -1 + 2 * s, -1 + 2 * t
        return (u * (xp - 0.012), v * 0.6, z + 0.008 - 0.03 * (1 - u * u) + 0.003 * noise(mathutils.Vector((u * 5, v * 9, 8.0))))

    parts = [sheet("canvas", m["cloth"], canvas, 14, 30, 0.006)]
    for sx in (-1, 1):
        parts.append(rod("pole", m["wy"], (sx * xp, -half, z), (sx * xp, half, z), 0.022, verts=12))
        for sy in (-1, 1):
            parts.append(box("foot", m["wz"], sx * xp - 0.018, sx * xp + 0.018, sy * 0.5 - 0.018, sy * 0.5 + 0.018, 0.0, z - 0.012, bevel=0.004))
    for sy in (-1, 1):
        parts.append(rod("cross", m["wx"], (-xp, sy * 0.63, z - 0.022), (xp, sy * 0.63, z - 0.022), 0.015, verts=10))
    return parts, m


MODELS = {
    "single": lambda: bed(0.66, 1),
    "double": lambda: bed(1.06, 2),
    "hammock": hammock,
    "gathered": gathered,
    "bedroll": bedroll,
    "roll": roll,
    "sleeping_bag": sleeping_bag,
    "mattress": mattress,
    "hay": hay,
    "cot": cot,
    "stretcher": stretcher,
}


# Карты по стольку точек: кровати и гамак на распорках — крупные и частые на картах, прочее — мелкое
# (ткань всё равно красит рисунок FA, у него 256 точек на клетку).
SIZES = {"single": 1024, "double": 1024, "hammock": 1024, "hay": 768}


def classes(o):
    """Класс каждой грани по материалу детали: ткань (красит рисунок), дерево (порода), прочее."""
    out = []
    for p in o.data.polygons:
        name = o.data.materials[p.material_index].name
        out.append("cloth" if name.startswith(("cloth", "linen", "straw")) else "wood" if name.startswith("wood") else "keep")
    return out


def classify(o, fm, cls):
    """Три материала с одними картами — по классам граней."""
    o.data.materials.clear()
    for c in ("cloth", "wood", "keep"):
        m = fm.copy()
        m.name = c
        o.data.materials.append(m)
    order = {"cloth": 0, "wood": 1, "keep": 2}
    for p, c in zip(o.data.polygons, cls):
        p.material_index = order[c]


def shoot(s, kind, obj):
    """Снимки: сверху наискосок и сбоку, на сером полу."""
    floor = bpy.data.materials.new("floor")
    floor.use_nodes = True
    floor.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (0.3, 0.29, 0.27, 1)
    lo = mathutils.Vector([min(v.co[i] for v in obj.data.vertices) for i in range(3)])
    hi = mathutils.Vector([max(v.co[i] for v in obj.data.vertices) for i in range(3)])
    c = (lo + hi) / 2
    size = max(hi.x - lo.x, hi.y - lo.y)
    o = box("floor", type("M", (), {"m": floor}), c.x - 3, c.x + 3, c.y - 3, c.y + 3, -0.02, 0.0)
    o.data.materials.clear()
    o.data.materials.append(floor)
    kit.light_rig(s)
    co = kit.camera(s, (720, 540))
    for side, at in (("top", (c.x + 0.9 * size, c.y - 1.1 * size, 1.4 * size + 0.4)), ("side", (c.x + 1.5 * size, c.y + 0.3 * size, 0.5 * size + 0.2))):
        kit.snap(s, co, os.path.join(SHOTS, f"bed_{kind}_{side}.png"), at, (c.x, c.y, c.z * 0.7))


for kind, make in MODELS.items():
    if ONLY and kind not in ONLY.split(","):
        continue
    s = reset()
    parts, ms = make()
    obj = join(parts, f"bed_{kind}")
    cls = classes(obj)
    fm = bake_maps(kind, obj, [g for g in ms.values()], "bed", min(kit.TEX, SIZES.get(kind, 512)))
    # Материалы деталей — прочь с дороги: классы должны называться ровно cloth, wood, keep.
    for g in ms.values():
        g.m.name = "part_" + g.m.name
    classify(obj, fm, cls)
    path = os.path.join(OUT, f"bed_{kind}.glb")
    tris = export_glb(obj, path)
    print(f"BED {kind}: {path} {os.path.getsize(path)} bytes, ~{tris} triangles", flush=True)
    if SHOTS:
        shoot(s, kind, obj)
