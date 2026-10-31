"""Видимые границы встроенных объектов для генераторов: src/prop_bounds.rs.

Холст объекта больше самого предмета (запас под тень), а к стене прижимается предмет.
Запускать из корня проекта после смены картинок в assets/builtin/objects.
"""
import re
import numpy as np
from PIL import Image

text = open("src/library.rs", encoding="utf-8").read()
body = text[text.index("pub const BUILTIN_OBJECTS"):]
body = body[: body.index("];")]
rows = []
for name, w, h in re.findall(r'\("(\w+)", "[^"]+", \[([\d.]+), ([\d.]+)\], include_bytes', body):
    a = np.asarray(Image.open(f"assets/builtin/objects/{name}.webp").convert("RGBA"))[..., 3] > 128
    ys, xs = np.nonzero(a)
    cell = a.shape[1] / float(w)
    cx, cy = a.shape[1] / 2, a.shape[0] / 2
    box = [(xs.min() - cx) / cell, (ys.min() - cy) / cell, (xs.max() + 1 - cx) / cell, (ys.max() + 1 - cy) / cell]
    rows.append(f'    ("{name}", [{box[0]:.3f}, {box[1]:.3f}, {box[2]:.3f}, {box[3]:.3f}]),\n')
with open("src/prop_bounds.rs", "w", encoding="utf-8", newline="\n") as f:
    f.write("// Собрано tools/bounds.py: границы предмета от центра холста, в клетках: лево, верх, право, низ.\n")
    f.write(f"const BOUNDS: [(&str, [f32; 4]); {len(rows)}] = [\n{''.join(rows)}];\n")
print(len(rows))
