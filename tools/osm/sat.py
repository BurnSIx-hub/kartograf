"""Спутниковый снимок (Esri World Imagery) ровно по рамке <bounds> из .osm, рядом с картой.

    python -I sat.py <участок.osm> <карта.jpg | -> <out.jpg> [zoom=18]

Плитки снимка кешируются в `sat_tiles` рядом с .osm: повторный круг сеть не трогает.
Слева снимок, справа карта, обе высотой 900, с подписями; `-` вместо карты — один снимок.
"""
import math
import os
import re
import sys
import urllib.request

from PIL import Image, ImageDraw, ImageFont

osm, mappng, out = sys.argv[1], sys.argv[2], sys.argv[3]
z = int(sys.argv[4]) if len(sys.argv) > 4 else 18
head = open(osm, encoding="utf-8").read(2000)
b = dict(re.findall(r'(minlat|minlon|maxlat|maxlon)="([-\d.]+)"', head))
minlat, minlon, maxlat, maxlon = (float(b[k]) for k in ("minlat", "minlon", "maxlat", "maxlon"))


def px(lat, lon):
    n = 2 ** z * 256
    x = (lon + 180) / 360 * n
    s = math.sin(math.radians(lat))
    y = (0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi)) * n
    return x, y


x0, y0 = px(maxlat, minlon)
x1, y1 = px(minlat, maxlon)
tx0, ty0, tx1, ty1 = int(x0 // 256), int(y0 // 256), int(x1 // 256), int(y1 // 256)
cache = os.path.join(os.path.dirname(os.path.abspath(osm)), "sat_tiles")
os.makedirs(cache, exist_ok=True)
canvas = Image.new("RGB", ((tx1 - tx0 + 1) * 256, (ty1 - ty0 + 1) * 256))
fetched = 0
for ty in range(ty0, ty1 + 1):
    for tx in range(tx0, tx1 + 1):
        f = os.path.join(cache, f"{z}_{tx}_{ty}.jpg")
        if not os.path.exists(f):
            url = f"https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{ty}/{tx}"
            req = urllib.request.Request(url, headers={"User-Agent": "kartograf-osm-import-test/0.1"})
            open(f, "wb").write(urllib.request.urlopen(req, timeout=30).read())
            fetched += 1
        canvas.paste(Image.open(f).convert("RGB"), ((tx - tx0) * 256, (ty - ty0) * 256))
sat = canvas.crop((int(x0 - tx0 * 256), int(y0 - ty0 * 256), int(x1 - tx0 * 256), int(y1 - ty0 * 256)))
h = 900
sat = sat.resize((round(sat.width * h / sat.height), h), Image.LANCZOS)
if mappng == "-":
    both = sat
else:
    m = Image.open(mappng).convert("RGB")
    m = m.resize((round(m.width * h / m.height), h), Image.LANCZOS)
    both = Image.new("RGB", (sat.width + m.width + 12, h), (255, 255, 255))
    both.paste(sat, (0, 0))
    both.paste(m, (sat.width + 12, 0))
    d = ImageDraw.Draw(both)
    font = ImageFont.truetype("arial.ttf", 28)
    for x, t in [(0, "снимок"), (sat.width + 12, "карта")]:
        d.rectangle((x, 0, x + 130, 42), fill=(0, 0, 0))
        d.text((x + 8, 6), t, fill=(255, 255, 255), font=font)
both.save(out, quality=88)
print(out, both.size, "плиток", (tx1 - tx0 + 1) * (ty1 - ty0 + 1), "скачано", fetched)
