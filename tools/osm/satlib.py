"""Снимок участка .osm: плитки Esri World Imagery, обрезанные ровно по <bounds>.

Общая часть sat.py и cover.py. Плитки кешируются в `sat_tiles` рядом с .osm.
"""
import math
import os
import re
import urllib.request

from PIL import Image

URL = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"


def bounds(osm):
    head = open(osm, encoding="utf-8").read(2000)
    b = dict(re.findall(r'(minlat|minlon|maxlat|maxlon)="([-\d.]+)"', head))
    return tuple(float(b[k]) for k in ("minlat", "minlon", "maxlat", "maxlon"))


def image(osm, z=18):
    """Снимок по рамке участка в родном разрешении плиток; (картинка, скачано плиток)."""
    minlat, minlon, maxlat, maxlon = bounds(osm)

    def px(lat, lon):
        n = 2 ** z * 256
        s = math.sin(math.radians(lat))
        return (lon + 180) / 360 * n, (0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi)) * n

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
                req = urllib.request.Request(URL.format(z=z, x=tx, y=ty), headers={"User-Agent": "kartograf-osm-import-test/0.1"})
                open(f, "wb").write(urllib.request.urlopen(req, timeout=30).read())
                fetched += 1
            canvas.paste(Image.open(f).convert("RGB"), ((tx - tx0) * 256, (ty - ty0) * 256))
    box = (int(x0 - tx0 * 256), int(y0 - ty0 * 256), int(x1 - tx0 * 256), int(y1 - ty0 * 256))
    return canvas.crop(box), fetched
