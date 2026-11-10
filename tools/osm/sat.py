"""Спутниковый снимок (Esri World Imagery) ровно по рамке <bounds> из .osm, рядом с картой.

    python -I sat.py <участок.osm> <карта.jpg | -> <out.jpg> [zoom=18]

Плитки снимка кешируются в `sat_tiles` рядом с .osm: повторный круг сеть не трогает.
Слева снимок, справа карта, обе высотой 900, с подписями; `-` вместо карты — один снимок.
"""
import os
import sys

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import satlib  # noqa: E402

osm, mappng, out = sys.argv[1], sys.argv[2], sys.argv[3]
z = int(sys.argv[4]) if len(sys.argv) > 4 else 18
sat, fetched = satlib.image(osm, z)
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
print(os.path.basename(out), both.size, "fetched", fetched)
