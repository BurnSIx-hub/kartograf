"""Снимок участка .osm: плитки Esri World Imagery, обрезанные ровно по <bounds>.

Общая часть sat.py и cover.py. Плитки кешируются в `sat_tiles` рядом с .osm.
"""
import math
import os
import re
import urllib.request

import numpy as np
from PIL import Image

URL = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}"


def bounds(osm):
    head = open(osm, encoding="utf-8").read(2000)
    b = dict(re.findall(r'(minlat|minlon|maxlat|maxlon)="([-\d.]+)"', head))
    return tuple(float(b[k]) for k in ("minlat", "minlon", "maxlat", "maxlon"))


# Плитка-заглушка Esri «Map data not yet available»: ровный серый 204.
BLANK = 204
# Сверху вниз: мельче 18 не берём (это уже ~0,5 м и крупнее).
ZOOMS = (20, 19, 18)
# Крупный масштаб — только боевым участкам (сторона до 200 м): участок в 380 м на
# 19-м — сотня плиток, а подсказке хватает 18-го.
BATTLE_M = 200
# Правила cover.py и trees.py мерены на 0,35–0,48 м: снимок мельче ужимается до
# 0,3 м — пестрота и тени в пикселях остаются те же, что при замерах.
REF_MPP = 0.3


def mpp(lat, z):
    """Метров на пиксель плитки масштаба z на широте lat."""
    return 156543.03392 * math.cos(math.radians(lat)) / 2 ** z


def _px(lat, lon, z):
    n = 2 ** z * 256
    s = math.sin(math.radians(lat))
    return (lon + 180) / 360 * n, (0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi)) * n


def _tile(cache, z, tx, ty):
    """Плитка из кеша или сети; (картинка, скачана ли)."""
    f = os.path.join(cache, f"{z}_{tx}_{ty}.jpg")
    got = False
    if not os.path.exists(f):
        req = urllib.request.Request(URL.format(z=z, x=tx, y=ty), headers={"User-Agent": "kartograf-osm-import-test/0.1"})
        open(f, "wb").write(urllib.request.urlopen(req, timeout=30).read())
        got = True
    return Image.open(f).convert("RGB"), got


def blank(im):
    """Заглушка «нет данных»: почти вся плитка ровно серая 204."""
    a = np.asarray(im.resize((64, 64)), int)
    return (np.abs(a - BLANK).max(-1) <= 4).mean() > 0.6


def _span(osm, z):
    minlat, minlon, maxlat, maxlon = bounds(osm)
    x0, y0 = _px(maxlat, minlon, z)
    x1, y1 = _px(minlat, maxlon, z)
    return x0, y0, x1, y1


def zoom(osm):
    """Самый крупный масштаб, где у рамки есть снимок (все плитки — не заглушки).
    Боевой участок в Швейцарии — 20 (0,1 м), в Германии — 19, в Эстонии — 18."""
    minlat, minlon, maxlat, maxlon = bounds(osm)
    side = max((maxlon - minlon) * 111320 * math.cos(math.radians((minlat + maxlat) / 2)), (maxlat - minlat) * 110574)
    if side > BATTLE_M:
        return 18
    cache = os.path.join(os.path.dirname(os.path.abspath(osm)), "sat_tiles")
    os.makedirs(cache, exist_ok=True)
    for z in ZOOMS:
        x0, y0, x1, y1 = _span(osm, z)
        # Сначала середина: заглушку видно по одной плитке.
        mid, _ = _tile(cache, z, int((x0 + x1) / 2 // 256), int((y0 + y1) / 2 // 256))
        if blank(mid):
            continue
        tiles = [(tx, ty) for ty in range(int(y0 // 256), int(y1 // 256) + 1) for tx in range(int(x0 // 256), int(x1 // 256) + 1)]
        # Участок в сотни метров на 20 — сотни плиток: крупный масштаб только для боевых.
        if len(tiles) > 64 and z > 18:
            continue
        if not any(blank(_tile(cache, z, tx, ty)[0]) for tx, ty in tiles):
            return z
    return ZOOMS[-1]


def image(osm, z=18):
    """Снимок по рамке участка в родном разрешении плиток; (картинка, скачано плиток).
    `z=None` — самый крупный масштаб, где есть снимок (`zoom`)."""
    if z is None:
        z = zoom(osm)
    x0, y0, x1, y1 = _span(osm, z)
    tx0, ty0, tx1, ty1 = int(x0 // 256), int(y0 // 256), int(x1 // 256), int(y1 // 256)
    cache = os.path.join(os.path.dirname(os.path.abspath(osm)), "sat_tiles")
    os.makedirs(cache, exist_ok=True)
    canvas = Image.new("RGB", ((tx1 - tx0 + 1) * 256, (ty1 - ty0 + 1) * 256))
    fetched = 0
    for ty in range(ty0, ty1 + 1):
        for tx in range(tx0, tx1 + 1):
            im, got = _tile(cache, z, tx, ty)
            fetched += got
            canvas.paste(im, ((tx - tx0) * 256, (ty - ty0) * 256))
    box = (int(x0 - tx0 * 256), int(y0 - ty0 * 256), int(x1 - tx0 * 256), int(y1 - ty0 * 256))
    return canvas.crop(box), fetched


def work(osm):
    """Снимок для правил подсказки: самый крупный масштаб (`zoom`), ужатый до
    `REF_MPP`, если он мельче; (картинка, скачано плиток, метров на пиксель)."""
    z = zoom(osm)
    im, fetched = image(osm, z)
    minlat, _, maxlat, _ = bounds(osm)
    m = mpp((minlat + maxlat) / 2, z)
    if m < REF_MPP:
        k = m / REF_MPP
        im = im.resize((max(1, round(im.width * k)), max(1, round(im.height * k))), Image.LANCZOS)
        m = REF_MPP
    return im, fetched, m
