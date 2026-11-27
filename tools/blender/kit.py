"""Общее для моделей Blender Картографа (`doors.py`, `windows.py`): детали, процедурные материалы,
развёртка, запекание карт PBR и выгрузка в `.glb`.

Единицы — клетки (1 = 5 футов), z — вверх. Модель — один кусок с одним материалом и тремя картами в
одной развёртке: цвет, нормали и ORM (затенение, шероховатость, металл в R, G, B, как в glTF). Их
запекает Cycles из процедурных материалов деталей; картинки в `.glb` — WebP без потерь (в сжатии с
потерями каналы карт перемешиваются).

Ключи скрипта: `-- <папка вывода> [<папка снимков>] [--only=<вид>] [--tex=1024] [--samples=32]`.
"""
import math
import os
import sys

import bmesh
import bpy
import mathutils
import numpy as np

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
opts = dict(a[2:].split("=", 1) for a in args if a.startswith("--") and "=" in a)
pos = [a for a in args if not a.startswith("--")]
SHOTS = os.path.abspath(pos[1]) if len(pos) > 1 else None
TEX = int(opts.get("tex", 1024))
SAMPLES = int(opts.get("samples", 32))
ONLY = opts.get("only")
if SHOTS:
    os.makedirs(SHOTS, exist_ok=True)


def out_dir(default):
    """Папка вывода: первый ключ или `default`."""
    out = os.path.abspath(pos[0] if pos else default)
    os.makedirs(out, exist_ok=True)
    return out


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    s = bpy.context.scene
    s.unit_settings.system = "NONE"
    s.render.engine = "CYCLES"
    s.cycles.device = "CPU"
    return s


# --- Детали -----------------------------------------------------------------------------------

def finish(o, mat, bevel=0.0, seg=2):
    o.data.materials.append(mat.m)
    if bevel > 0:
        m = o.modifiers.new("bevel", "BEVEL")
        m.width = bevel
        m.segments = seg
        m.limit_method = "ANGLE"
    return o


def box(name, mat, x0, x1, y0, y1, z0, z1, bevel=0.0, seg=2):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2))
    o = bpy.context.active_object
    o.name = name
    o.scale = (x1 - x0, y1 - y0, z1 - z0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(o, mat, bevel, seg)


def cyl(name, mat, at, r, depth, axis="Y", verts=16, bevel=0.0):
    rot = {"X": (0, math.pi / 2, 0), "Y": (math.pi / 2, 0, 0), "Z": (0, 0, 0)}[axis]
    bpy.ops.mesh.primitive_cylinder_add(vertices=verts, radius=r, depth=depth, location=at, rotation=rot)
    o = bpy.context.active_object
    o.name = name
    return finish(o, mat, bevel, 1)


def dome(name, mat, at, r, verts=10):
    """Шляпка гвоздя, заклёпка: приплюснутый шар лицом по ±y."""
    bpy.ops.mesh.primitive_uv_sphere_add(segments=verts, ring_count=4 if verts < 10 else 5, radius=r, location=at)
    o = bpy.context.active_object
    o.name = name
    o.scale = (1, 0.45, 1)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    return finish(o, mat)


def ring(name, mat, at, major, minor):
    """Кольцо ручки: тор в плоскости xz (висит на створке)."""
    bpy.ops.mesh.primitive_torus_add(major_radius=major, minor_radius=minor, major_segments=28, minor_segments=8, location=at, rotation=(math.pi / 2, 0, 0))
    o = bpy.context.active_object
    o.name = name
    return finish(o, mat)


def plate(name, mat, pts, y0, y1, bevel=0.0):
    """Плоская деталь по контуру `pts` (x, z) толщиной от `y0` до `y1`: полоса петли с наконечником."""
    me = bpy.data.meshes.new(name)
    bm = bmesh.new()
    face = bm.faces.new([bm.verts.new((x, y0, z)) for x, z in pts])
    top = bmesh.ops.extrude_face_region(bm, geom=[face])
    bmesh.ops.translate(bm, vec=(0, y1 - y0, 0), verts=[v for v in top["geom"] if isinstance(v, bmesh.types.BMVert)])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(me)
    bm.free()
    o = bpy.data.objects.new(name, me)
    bpy.context.collection.objects.link(o)
    return finish(o, mat, bevel, 1)


def bar(name, mat, a, b, width, y0, y1, bevel=0.005, seg=2):
    """Брусок от точки `a` до `b` в плоскости xz (раскос двери, свинцовая жилка окна)."""
    (ax, az), (bx, bz) = a, b
    length = math.hypot(bx - ax, bz - az)
    o = box(name, mat, -length / 2, length / 2, y0, y1, -width / 2, width / 2, bevel=bevel, seg=seg)
    o.rotation_euler = (0, -math.atan2(bz - az, bx - ax), 0)
    # Только по x и z: по толщине брусок остаётся, где его поставили (иначе уходит внутрь досок).
    o.location = ((ax + bx) / 2, o.location.y, (az + bz) / 2)
    bpy.ops.object.select_all(action="DESELECT")
    o.select_set(True)
    bpy.context.view_layer.objects.active = o
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
    # Начало — в нуле: материалы берут место точки в осях створки.
    bpy.context.scene.cursor.location = (0, 0, 0)
    bpy.ops.object.origin_set(type="ORIGIN_CURSOR")
    bpy.ops.object.shade_smooth_by_angle(angle=math.radians(35))
    return o


# --- Материалы: узлы ---------------------------------------------------------------------------

class Graph:
    """Процедурный материал: каналы `base`, `rough`, `metal` и рельеф (через Bump в нормаль BSDF)."""

    def __init__(self, name):
        self.m = bpy.data.materials.new(name)
        self.m.use_nodes = True
        self.nt = self.m.node_tree
        self.nt.nodes.clear()
        self.out = self.node("ShaderNodeOutputMaterial")
        self.bsdf = self.node("ShaderNodeBsdfPrincipled")
        self.link(self.bsdf.outputs["BSDF"], self.out.inputs["Surface"])
        geo = self.node("ShaderNodeNewGeometry")
        self.pos, self.gnormal, self.island = geo.outputs["Position"], geo.outputs["Normal"], geo.outputs["Random Per Island"]
        self.z = self.node("ShaderNodeSeparateXYZ")
        self.link(self.pos, self.z.inputs[0])
        self.z = self.z.outputs["Z"]
        self.ch = {}

    def node(self, kind, **props):
        n = self.nt.nodes.new(kind)
        for k, v in props.items():
            setattr(n, k, v)
        return n

    def link(self, a, b):
        self.nt.links.new(a, b)

    def put(self, sock, v):
        if isinstance(v, bpy.types.NodeSocket):
            self.link(v, sock)
        elif isinstance(v, (int, float)) and sock.type in ("RGBA", "VECTOR"):
            sock.default_value = (v, v, v, 1.0)[: len(sock.default_value)]
        elif sock.type == "RGBA" and len(v) == 3:
            sock.default_value = (*v, 1.0)
        else:
            sock.default_value = v

    @staticmethod
    def sock(n, ident, out=False):
        socks = n.outputs if out else n.inputs
        for s in socks:
            if s.identifier == ident and s.enabled:
                return s
        return next(s for s in socks if s.name == ident and s.enabled)

    def math(self, op, a, b=0.0, clamp=False):
        n = self.node("ShaderNodeMath", operation=op, use_clamp=clamp)
        self.put(n.inputs[0], a)
        self.put(n.inputs[1], b)
        return n.outputs[0]

    def add(self, *xs):
        acc = xs[0]
        for x in xs[1:]:
            acc = self.math("ADD", acc, x)
        return acc

    def mul(self, a, b):
        return self.math("MULTIPLY", a, b)

    def clamp(self, x):
        return self.math("ADD", x, 0.0, clamp=True)

    def smooth(self, x, lo, hi, to=(0.0, 1.0)):
        n = self.node("ShaderNodeMapRange", interpolation_type="SMOOTHSTEP", clamp=True)
        self.put(n.inputs["Value"], x)
        n.inputs["From Min"].default_value, n.inputs["From Max"].default_value = lo, hi
        n.inputs["To Min"].default_value, n.inputs["To Max"].default_value = to
        return n.outputs["Result"]

    def lerp(self, t, a, b):
        n = self.node("ShaderNodeMix", data_type="FLOAT", clamp_factor=True)
        self.put(self.sock(n, "Factor_Float"), t)
        self.put(self.sock(n, "A_Float"), a)
        self.put(self.sock(n, "B_Float"), b)
        return self.sock(n, "Result_Float", out=True)

    def mixc(self, t, a, b, blend="MIX"):
        n = self.node("ShaderNodeMix", data_type="RGBA", blend_type=blend, clamp_factor=True)
        self.put(self.sock(n, "Factor_Float"), t)
        self.put(self.sock(n, "A_Color"), a)
        self.put(self.sock(n, "B_Color"), b)
        return self.sock(n, "Result_Color", out=True)

    def scalec(self, c, k):
        n = self.node("ShaderNodeVectorMath", operation="SCALE")
        self.put(n.inputs[0], c)
        self.put(self.sock(n, "Scale"), k)
        return n.outputs["Vector"]

    def ramp(self, fac, stops):
        n = self.node("ShaderNodeValToRGB")
        els = n.color_ramp.elements
        while len(els) > 1:
            els.remove(els[-1])
        els[0].position, els[0].color = stops[0][0], (*stops[0][1], 1.0)
        for p, c in stops[1:]:
            e = els.new(p)
            e.color = (*c, 1.0)
        self.put(n.inputs["Fac"], fac)
        return n.outputs["Color"]

    def vec(self, v, scale=(1, 1, 1), add=None):
        n = self.node("ShaderNodeVectorMath", operation="MULTIPLY")
        self.put(n.inputs[0], v)
        n.inputs[1].default_value = scale
        out = n.outputs["Vector"]
        if add is not None:
            a = self.node("ShaderNodeVectorMath", operation="ADD")
            self.put(a.inputs[0], out)
            self.put(a.inputs[1], add)
            out = a.outputs["Vector"]
        return out

    def combine(self, x, y, z):
        n = self.node("ShaderNodeCombineXYZ")
        for i, v in enumerate((x, y, z)):
            self.put(n.inputs[i], v)
        return n.outputs["Vector"]

    def rotate_y(self, v, angle):
        n = self.node("ShaderNodeMapping", vector_type="POINT")
        self.put(n.inputs["Vector"], v)
        n.inputs["Rotation"].default_value = (0.0, angle, 0.0)
        return n.outputs["Vector"]

    def noise(self, v, scale, detail=4.0, rough=0.5, distortion=0.0, color=False):
        n = self.node("ShaderNodeTexNoise", noise_dimensions="3D")
        self.put(n.inputs["Vector"], v)
        n.inputs["Scale"].default_value = scale
        n.inputs["Detail"].default_value = detail
        n.inputs["Roughness"].default_value = rough
        n.inputs["Distortion"].default_value = distortion
        return n.outputs["Color" if color else "Fac"]

    def voronoi(self, v, scale, rand=1.0, feature="F1"):
        n = self.node("ShaderNodeTexVoronoi", voronoi_dimensions="3D", feature=feature)
        self.put(n.inputs["Vector"], v)
        n.inputs["Scale"].default_value = scale
        n.inputs["Randomness"].default_value = rand
        return n.outputs["Distance"], n.outputs["Color"]

    def wave(self, v, scale, distortion, detail=3.0, dscale=1.0, direction="X", rings=False):
        """Полосы поперёк `direction`; `rings` — кольца вокруг оси z (цилиндры)."""
        if rings:
            n = self.node("ShaderNodeTexWave", wave_type="RINGS", rings_direction="Z")
        else:
            n = self.node("ShaderNodeTexWave", wave_type="BANDS", bands_direction=direction)
        self.put(n.inputs["Vector"], v)
        n.inputs["Scale"].default_value = scale
        n.inputs["Distortion"].default_value = distortion
        n.inputs["Detail"].default_value = detail
        n.inputs["Detail Scale"].default_value = dscale
        return n.outputs["Fac"]

    def ao(self, distance):
        n = self.node("ShaderNodeAmbientOcclusion", samples=16, only_local=False)
        n.inputs["Distance"].default_value = distance
        return n.outputs["AO"]

    def edge(self, radius, reach=0.02):
        """Скруглённая нормаль рёбер (радиус `radius`) и маска рёбер (0 — плоскость, 1 — ребро):
        отличие нормали, сглаженной на `reach`, от настоящей — так видны и рёбра, скруглённые
        самой моделью."""
        b = self.node("ShaderNodeBevel", samples=8)
        b.inputs["Radius"].default_value = radius
        wide = self.node("ShaderNodeBevel", samples=8)
        wide.inputs["Radius"].default_value = reach
        d = self.node("ShaderNodeVectorMath", operation="DOT_PRODUCT")
        self.link(wide.outputs["Normal"], d.inputs[0])
        self.link(self.gnormal, d.inputs[1])
        return b.outputs["Normal"], self.smooth(self.math("SUBTRACT", 1.0, d.outputs["Value"]), 0.01, 0.12)

    def done(self, base, rough, metal, height, strength, distance, normal):
        bump = self.node("ShaderNodeBump")
        bump.inputs["Strength"].default_value = strength
        bump.inputs["Distance"].default_value = distance
        self.put(bump.inputs["Height"], height)
        self.put(bump.inputs["Normal"], normal)
        self.link(bump.outputs["Normal"], self.bsdf.inputs["Normal"])
        self.put(self.bsdf.inputs["Base Color"], base)
        self.put(self.bsdf.inputs["Roughness"], rough)
        self.put(self.bsdf.inputs["Metallic"], metal)
        self.ch = {"base": base, "rough": rough, "metal": metal, "ao": self.ao(0.12)}
        return self


def wood(name, angle=math.pi / 2, rings=False):
    """Дерево: волокна вдоль детали (`angle` — направление детали в плоскости xz от оси x; доски
    стоят — π/2). Годичные слои — полосами поперёк x (видны на гранях xz: створки дверей) или
    `rings` — кольцами вокруг оси волокон, как в бревне (видны на всех гранях вдоль волокон:
    откосы и рама окна)."""
    g = Graph(name)
    isl = g.island
    p = g.rotate_y(g.pos, angle - math.pi / 2)
    p = g.vec(p, add=g.combine(g.mul(isl, 17.3), g.mul(isl, 5.1), g.mul(isl, 11.7)))
    warp = g.vec(g.noise(g.vec(p, (3, 3, 0.5)), 1.0, 3.0, color=True), add=(-0.5, -0.5, -0.5))
    q = g.vec(warp, (0.035, 0.035, 0.0), add=p)
    growth = g.wave(g.vec(q, (1, 1, 0.06)), 14.0, 7.0, 4.0, 1.4, rings=rings)
    late = g.smooth(growth, 0.6, 0.97)
    pores = g.noise(g.vec(q, (1, 1, 0.025)), 260.0, 2.0)
    broad = g.noise(g.vec(q, (1, 1, 0.15)), 6.0, 4.0)
    # Сучки: редкие ячейки, вытянутые вдоль волокон.
    kd, kc = g.voronoi(g.vec(q, (1, 1, 0.35)), 4.0)
    knot = g.mul(g.smooth(kd, 0.07, 0.02), g.smooth(kc, 0.8, 0.84))
    grain = g.add(g.mul(g.math("SUBTRACT", 1.0, late), 0.55), g.mul(pores, 0.25), g.mul(broad, 0.3))
    base = g.ramp(grain, [(0.15, (0.1, 0.05, 0.021)), (0.5, (0.2, 0.1, 0.042)), (0.9, (0.31, 0.17, 0.077))])
    # Доска к доске — свой тон.
    base = g.scalec(base, g.add(0.78, g.mul(isl, 0.4)))
    base = g.mixc(g.mul(g.math("FRACT", g.mul(isl, 7.7)), 0.35), base, (0.24, 0.1, 0.05))
    base = g.mixc(g.mul(knot, 0.85), base, (0.05, 0.025, 0.012))
    normal, e = g.edge(0.006, 0.015)
    wear = g.mul(e, g.smooth(g.noise(g.pos, 35.0, 5.0), 0.38, 0.62))
    base = g.mixc(g.mul(wear, 0.55), base, g.scalec(base, 1.7))
    dirt = g.ao(0.05)
    base = g.mixc(g.math("SUBTRACT", 1.0, dirt), base, g.scalec(base, 0.4))
    grime = g.mul(g.smooth(g.z, 0.32, 0.0), g.smooth(g.noise(g.pos, 9.0, 6.0), 0.3, 0.7))
    base = g.mixc(g.mul(grime, 0.6), base, (0.045, 0.04, 0.032))
    rough = g.clamp(g.add(0.66, g.mul(late, 0.14), g.mul(grime, 0.08), g.mul(wear, -0.16), g.mul(knot, 0.06)))
    height = g.add(g.mul(grain, 0.6), g.mul(pores, 0.5), g.mul(knot, -0.4))
    return g.done(base, rough, 0.0, height, 0.6, 0.0015, normal)


def iron():
    """Кованое железо: вмятины от молота, ржавчина в углах и пятнами, стёртые до блеска рёбра."""
    g = Graph("iron")
    p = g.pos
    dent, _ = g.voronoi(p, 70.0)
    fine = g.noise(p, 300.0, 4.0)
    rn = g.noise(p, 14.0, 8.0, 0.6)
    occl = g.ao(0.03)
    rust = g.clamp(g.mul(g.add(g.smooth(rn, 0.56, 0.68), g.mul(g.math("SUBTRACT", 1.0, occl), 0.45)), 0.85))
    normal, e = g.edge(0.004, 0.008)
    wear = g.mul(e, g.smooth(g.noise(p, 40.0, 4.0), 0.35, 0.6))
    base = g.ramp(fine, [(0.3, (0.022, 0.021, 0.02)), (0.7, (0.05, 0.048, 0.045))])
    rustc = g.ramp(g.noise(p, 60.0, 6.0), [(0.3, (0.05, 0.02, 0.007)), (0.7, (0.16, 0.062, 0.022))])
    base = g.mixc(rust, base, rustc)
    base = g.mixc(g.mul(wear, 0.8), base, (0.3, 0.29, 0.28))
    metal = g.math("SUBTRACT", 1.0, g.mul(rust, 0.7))
    rough = g.clamp(g.add(0.48, g.mul(fine, 0.15), g.mul(rust, 0.38), g.mul(wear, -0.22)))
    height = g.add(g.mul(g.smooth(dent, 0.0, 0.5), 0.6), g.mul(rust, g.mul(g.noise(p, 120.0, 3.0), 0.5)), g.mul(fine, 0.2))
    return g.done(base, rough, metal, height, 0.5, 0.0008, normal)


def steel():
    """Листовая сталь: пятна, потёки ржавчины снизу, царапины, светлые рёбра."""
    g = Graph("steel")
    p = g.pos
    smudge = g.noise(p, 3.0, 6.0)
    streak = g.noise(g.vec(p, (1, 1, 0.06)), 40.0, 3.0)
    rn = g.noise(p, 10.0, 8.0, 0.6)
    occl = g.ao(0.04)
    low = g.smooth(g.z, 0.4, 0.0)
    rust = g.clamp(g.add(g.mul(g.smooth(rn, 0.6, 0.7), 0.7), g.mul(low, g.mul(g.smooth(streak, 0.45, 0.7), 0.8)), g.mul(g.math("SUBTRACT", 1.0, occl), 0.6)))
    scratch = g.mul(g.smooth(g.wave(p, 25.0, 15.0, 2.0, 1.0, "DIAGONAL"), 0.96, 1.0), g.smooth(g.noise(p, 5.0, 2.0), 0.5, 0.7))
    normal, e = g.edge(0.005, 0.015)
    wear = g.mul(e, g.smooth(g.noise(p, 35.0, 4.0), 0.35, 0.6))
    base = g.ramp(smudge, [(0.3, (0.2, 0.205, 0.215)), (0.7, (0.34, 0.345, 0.355))])
    base = g.mixc(g.mul(rust, 0.85), base, g.ramp(g.noise(p, 50.0, 5.0), [(0.3, (0.08, 0.03, 0.01)), (0.7, (0.22, 0.09, 0.035))]))
    base = g.mixc(g.clamp(g.add(g.mul(wear, 0.8), g.mul(scratch, 0.5))), base, (0.55, 0.55, 0.56))
    metal = g.math("SUBTRACT", 0.8, g.mul(rust, 0.55))
    rough = g.clamp(g.add(0.56, g.mul(smudge, 0.22), g.mul(rust, 0.3), g.mul(wear, -0.2), g.mul(scratch, -0.15)))
    dents, _ = g.voronoi(p, 18.0)
    height = g.add(g.mul(g.smooth(dents, 0.0, 0.6), 0.3), g.mul(rust, 0.3), g.mul(scratch, -0.4))
    return g.done(base, rough, metal, height, 0.4, 0.001, normal)


def stone(name="stone", veins=True):
    """Тёсаный камень: зерно, крапины, тонкие жилы (`veins`), следы резца, сколы на рёбрах, грязь
    внизу."""
    g = Graph(name)
    p = g.pos
    big = g.noise(p, 2.5, 8.0, 0.55)
    mid = g.noise(p, 12.0, 8.0, 0.65)
    fine = g.noise(p, 90.0, 5.0, 0.6)
    grit = g.noise(p, 420.0, 2.0)
    sd, sc = g.voronoi(p, 160.0)
    speck = g.mul(g.smooth(sd, 0.3, 0.0), g.smooth(sc, 0.62, 0.68))
    vein = g.mul(g.smooth(g.wave(p, 1.2, 18.0, 6.0, 1.0, "DIAGONAL"), 0.975, 1.0), 0.25)
    chisel = g.noise(g.vec(g.rotate_y(p, 0.6), (1.0, 1.0, 0.12)), 70.0, 3.0)
    normal, e = g.edge(0.006, 0.03)
    _, cc = g.voronoi(p, 22.0)
    chip = g.mul(g.smooth(e, 0.15, 0.5), g.mul(g.smooth(cc, 0.45, 0.55), g.smooth(g.noise(p, 30.0, 4.0), 0.5, 0.6)))
    occl = g.ao(0.05)
    tone = g.add(g.mul(mid, 0.5), g.mul(big, 0.3), g.mul(fine, 0.2))
    base = g.ramp(tone, [(0.3, (0.12, 0.115, 0.105)), (0.5, (0.22, 0.21, 0.195)), (0.7, (0.32, 0.31, 0.29))])
    base = g.mixc(g.mul(grit, 0.35), base, g.scalec(base, 0.7))
    base = g.mixc(g.mul(speck, 0.85), base, (0.05, 0.05, 0.048))
    if veins:
        base = g.mixc(vein, base, (0.4, 0.39, 0.37))
    base = g.mixc(g.math("SUBTRACT", 1.0, occl), base, g.scalec(base, 0.4))
    grime = g.mul(g.smooth(g.z, 0.35, 0.0), g.smooth(g.noise(p, 8.0, 6.0), 0.3, 0.7))
    base = g.mixc(g.mul(grime, 0.55), base, (0.06, 0.065, 0.05))
    base = g.mixc(g.mul(chip, 0.8), base, (0.31, 0.3, 0.285))
    rough = g.clamp(g.add(0.82, g.mul(fine, 0.12), g.mul(chip, -0.06)))
    height = g.add(g.mul(mid, 0.45), g.mul(fine, 0.45), g.mul(grit, 0.35), g.mul(chisel, 0.2), g.mul(chip, -1.4), g.mul(speck, -0.15))
    return g.done(base, rough, 0.0, height, 1.0, 0.003, normal)


def hole():
    g = Graph("hole")
    return g.done((0.004, 0.004, 0.004), 0.9, 0.0, 0.0, 0.0, 0.001, g.gnormal)


# --- Развёртка, запекание, выгрузка -------------------------------------------------------------

def unwrap(o):
    bpy.ops.object.select_all(action="DESELECT")
    o.select_set(True)
    bpy.context.view_layer.objects.active = o
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(angle_limit=math.radians(66), island_margin=0.006)
    # Острова — плотно по всей картинке, с поворотом.
    bpy.ops.uv.pack_islands(rotate=True, margin=0.006)
    bpy.ops.object.mode_set(mode="OBJECT")


def image(name, color, size=None):
    size = size or TEX
    img = bpy.data.images.new(name, size, size, alpha=False, float_buffer=not color)
    img.colorspace_settings.name = "sRGB" if color else "Non-Color"
    return img


def bake(graphs, channel, img):
    """Запечь канал всех материалов в `img`: цвет, шероховатость, металл, затенение — свечением,
    нормали — как есть (с рельефом Bump)."""
    for g in graphs:
        tgt = getattr(g, "target", None) or g.node("ShaderNodeTexImage")
        g.target = tgt
        tgt.image = img
        g.nt.nodes.active = tgt
        for l in list(g.out.inputs["Surface"].links):
            g.nt.links.remove(l)
        if channel == "normal":
            g.link(g.bsdf.outputs["BSDF"], g.out.inputs["Surface"])
        else:
            em = getattr(g, "emit", None) or g.node("ShaderNodeEmission")
            g.emit = em
            for l in list(em.inputs["Color"].links):
                g.nt.links.remove(l)
            g.put(em.inputs["Color"], g.ch[channel])
            g.link(em.outputs["Emission"], g.out.inputs["Surface"])
    s = bpy.context.scene
    s.cycles.samples = SAMPLES
    s.render.bake.margin = 6
    s.render.bake.margin_type = "EXTEND"
    if channel == "normal":
        bpy.ops.object.bake(type="NORMAL", normal_space="TANGENT")
    else:
        bpy.ops.object.bake(type="EMIT")


def pixels(img):
    w, h = img.size
    a = np.empty(w * h * 4, np.float32)
    img.pixels.foreach_get(a)
    return a.reshape(h, w, 4)


def gltf_output(nt):
    """Узел «glTF Material Output»: по нему выгрузка glTF берёт карту затенения."""
    g = bpy.data.node_groups.get("glTF Material Output")
    if g is None:
        g = bpy.data.node_groups.new("glTF Material Output", "ShaderNodeTree")
        g.interface.new_socket(name="Occlusion", in_out="INPUT", socket_type="NodeSocketFloat")
    n = nt.nodes.new("ShaderNodeGroup")
    n.node_tree = g
    return n


def bake_maps(kind, obj, graphs, prefix):
    """Развернуть `obj`, запечь каналы материалов `graphs` в цвет, нормали и ORM и поставить ему
    готовый материал из трёх карт. С папкой снимков — карты в `<prefix>_<вид>_*.png`."""
    unwrap(obj)
    imgs = {"base": image(f"{kind}_base", True), "normal": image(f"{kind}_normal", False)}
    for ch in ("rough", "metal", "ao"):
        imgs[ch] = image(f"{kind}_{ch}", False)
    for ch in ("base", "normal", "rough", "metal", "ao"):
        bake(graphs, ch, imgs[ch])
        print(f"{prefix.upper()} {kind}: baked {ch}", flush=True)
    # ORM: затенение, шероховатость, металл — в одну картинку, как в glTF.
    orm = image(f"{kind}_orm", False)
    o = np.stack([pixels(imgs["ao"])[..., 0], pixels(imgs["rough"])[..., 0], pixels(imgs["metal"])[..., 0], np.ones((TEX, TEX), np.float32)], -1)
    orm.pixels.foreach_set(o.ravel())
    orm.update()
    if SHOTS:
        for img in (imgs["base"], imgs["normal"], orm):
            img.filepath_raw = os.path.join(SHOTS, f"{prefix}_{img.name}.png")
            img.file_format = "PNG"
            img.save()
    # Готовый материал: три карты.
    fm = bpy.data.materials.new(kind)
    fm.use_nodes = True
    nt = fm.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    tb = nt.nodes.new("ShaderNodeTexImage")
    tb.image = imgs["base"]
    nt.links.new(tb.outputs["Color"], bsdf.inputs["Base Color"])
    tn = nt.nodes.new("ShaderNodeTexImage")
    tn.image = imgs["normal"]
    nm = nt.nodes.new("ShaderNodeNormalMap")
    nt.links.new(tn.outputs["Color"], nm.inputs["Color"])
    nt.links.new(nm.outputs["Normal"], bsdf.inputs["Normal"])
    to = nt.nodes.new("ShaderNodeTexImage")
    to.image = orm
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    nt.links.new(to.outputs["Color"], sep.inputs["Color"])
    nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
    nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
    nt.links.new(sep.outputs["Red"], gltf_output(nt).inputs["Occlusion"])
    obj.data.materials.clear()
    obj.data.materials.append(fm)
    for p in obj.data.polygons:
        p.material_index = 0
    return fm


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


def export_glb(obj, path):
    """Выгрузить `obj` в `.glb` (y — вверх, картинки — WebP без потерь); число треугольников."""
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True, export_apply=True, export_yup=True, export_image_format="WEBP", export_image_quality=100, export_animations=False)
    return sum(len(p.vertices) - 2 for p in obj.data.polygons)


def camera(s, res):
    """Камера снимков (Cycles с подавлением шума)."""
    cam = bpy.data.cameras.new("cam")
    cam.lens = 50
    co = bpy.data.objects.new("cam", cam)
    s.collection.objects.link(co)
    s.camera = co
    s.cycles.samples = 48
    s.cycles.use_denoising = True
    s.render.resolution_x, s.render.resolution_y = res
    s.render.film_transparent = False
    return co


def snap(s, co, path, at, aim):
    co.location = at
    co.rotation_euler = (mathutils.Vector(aim) - co.location).to_track_quat("-Z", "Y").to_euler()
    s.render.filepath = path
    bpy.ops.render.render(write_still=True)
