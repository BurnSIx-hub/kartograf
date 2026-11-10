"""Лист картинок из папки (DUMP=папка у тестов fa_slots, find_fa): 8 в ряд с подписями.

    python -I sheet.py <папка> <out.jpg> [слово,слово]
"""
import sys, os
from PIL import Image, ImageDraw
d, out = sys.argv[1], sys.argv[2]
pat = sys.argv[3].lower().split(",") if len(sys.argv) > 3 else [""]
files = sorted(f for f in os.listdir(d) if any(p in f.lower() for p in pat))
cols, s = 8, 160
rows = (len(files) + cols - 1) // cols
im = Image.new("RGB", (cols * s, rows * (s + 16)), (255, 255, 255))
dr = ImageDraw.Draw(im)
for k, f in enumerate(files):
    t = Image.open(os.path.join(d, f)).convert("RGB").resize((s, s))
    x, y = (k % cols) * s, (k // cols) * (s + 16)
    im.paste(t, (x, y))
    dr.text((x + 2, y + s + 2), f[:26], fill=(0, 0, 0))
im.save(out, quality=88)
print(len(files))
