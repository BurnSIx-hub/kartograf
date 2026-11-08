"""Зубцы стены: полоса каменных блоков, нарезанная на мерлоны и бойницы.

Мерлон — блок кладки своей высоты с обводкой; между мерлонами бойница — тот же
камень ниже и темнее (подоконник амбразуры), так что полоса сплошная и стена в ней
не рвётся. На длину полосы — ровно три пары, повтор без шва.

    python tools/crenels.py assets/builtin/walls/stone_blocks.webp assets/builtin/walls/crenels.webp
"""
import sys

from PIL import Image, ImageDraw, ImageFilter

src, out = sys.argv[1], sys.argv[2]
im = Image.open(src).convert("RGBA")
# Высота полосы — толщина зубцов (0,5 клетки) × 256 точек.
H = 128
im = im.resize((round(im.width * H / im.height), H), Image.LANCZOS)
w, h = im.size
PAIRS = 3
period = w / PAIRS
merlon = period * 0.68

px = im.load()
res = Image.new("RGBA", (w, h))
rp = res.load()
for x in range(w):
    t = x % period
    gap = t >= merlon
    for y in range(h):
        r, g, b, a = px[x, y]
        if gap:
            # Подоконник бойницы ниже — темнее; к мерлонам тень гуще.
            d = min(t - merlon, period - t)
            k = 0.5 - 0.12 * max(0.0, 1.0 - d / 10.0)
            r, g, b = int(r * k), int(g * k), int(b * k)
        rp[x, y] = (r, g, b, a)

# Торцы мерлонов — тёмной обводкой, как кромки полосы.
draw = ImageDraw.Draw(res)
edge = max(3, h // 28)
for i in range(PAIRS):
    x0 = i * period
    for xe in (x0, x0 + merlon):
        xi = int(round(xe))
        draw.rectangle([xi - edge // 2, 0, xi + edge // 2, h - 1], fill=(28, 24, 22, 255))

# Мерлон чуть светлее к верху холста — свет сверху слева, как у теней карты.
hi = Image.new("RGBA", (w, h), (0, 0, 0, 0))
hp = hi.load()
for x in range(w):
    if x % period < merlon:
        for y in range(h // 6, h // 2):
            hp[x, y] = (255, 255, 240, int(26 * (1 - (y - h // 6) / (h / 3))))
res = Image.alpha_composite(res, hi.filter(ImageFilter.GaussianBlur(2)))
res.save(out, "WEBP", quality=90, method=6)
print(out, res.size)
