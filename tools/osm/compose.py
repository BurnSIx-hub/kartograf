"""Сцена Foundry «как в игре»: фон выгрузки и поверх него тайлы (кроны, крыши).

    python -I compose.py <папка выгрузки> <out.jpg> <масштаб>

Папка — то, что пишет `kartograf.exe --export`: «Новая карта.json» и «.webp».
"""
import json
import math
import os
import sys

from PIL import Image

d, out, scale = sys.argv[1], sys.argv[2], float(sys.argv[3])
j = json.load(open(os.path.join(d, "Новая карта.json"), encoding="utf-8"))
bg = Image.open(os.path.join(d, "Новая карта.webp")).convert("RGBA")
size = j["grid"]["size"]
pad = j.get("padding", 0)
ox = math.ceil(pad * j["width"] / size) * size
oy = math.ceil(pad * j["height"] / size) * size
print("padding", pad, ox, oy, "tiles", len(j["tiles"]))
cache = {}
for t in sorted(j["tiles"], key=lambda t: (t.get("elevation", 0), t.get("sort", 0))):
    src = t["texture"]["src"].split("/")[-1]
    p = os.path.join(d, src)
    if not os.path.exists(p):
        continue
    if src not in cache:
        cache[src] = Image.open(p).convert("RGBA")
    im = cache[src].resize((max(1, int(t["width"])), max(1, int(t["height"]))))
    im = im.rotate(-t.get("rotation", 0), expand=True, resample=Image.BICUBIC)
    # Foundry 14: x, y тайла — середина (texture.anchorX/Y = 0.5).
    cx, cy = t["x"] - ox, t["y"] - oy
    bg.alpha_composite(im, (int(cx - im.width / 2), int(cy - im.height / 2)))
bg = bg.convert("RGB")
bg.resize((int(bg.width * scale), int(bg.height * scale)), Image.LANCZOS).save(out, quality=88)
