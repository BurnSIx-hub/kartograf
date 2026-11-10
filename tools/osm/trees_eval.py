"""Сверка пород крон (trees.py) с картой лесов NFI (nfi.py).

    python -I trees_eval.py <участок.osm> [...]

Каждой кроне — тип леса NFI под её центром. Печатает по типам: сколько крон, середину
признаков и долю, названную хвойной. Ожидаемая доля хвойных по определению NFI:
Broadleaved — до 20 %, Mixed mainly broadleaved — 20–50 %, Mixed mainly conifer —
50–80 %, Conifer — от 80 %. Вне лесов NFI (мельче 0,5 га) — «вне».
"""
import json
import math
import os
import sys
from collections import defaultdict

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import satlib  # noqa: E402

KEYS = ["light", "light_top", "h", "s", "exg", "exg_top", "fine", "coarse", "r"]
EXPECT = {"Broadleaved": (0, 0.2), "Mixed mainly broadleaved": (0.2, 0.5), "Mixed mainly conifer": (0.5, 0.8), "Conifer": (0.8, 1.0)}


def merc(lat):
    s = math.sin(math.radians(lat))
    return math.log((1 + s) / (1 - s)) / 2


def nfi_raster(osm, w, h):
    """Тип леса NFI в каждом пикселе снимка: (картинка индексов, список типов)."""
    minlat, minlon, maxlat, maxlon = satlib.bounds(osm)
    stem = os.path.splitext(os.path.basename(osm))[0]
    path = os.path.join(os.path.dirname(os.path.abspath(osm)), "nfi", stem + ".geojson")
    names = ["вне"]
    im = Image.new("L", (w, h), 0)
    if not os.path.exists(path):
        return np.asarray(im), names
    d = ImageDraw.Draw(im)
    px = lambda lon, lat: ((lon - minlon) / (maxlon - minlon) * w, (merc(maxlat) - merc(lat)) / (merc(maxlat) - merc(minlat)) * h)
    for f in json.load(open(path, encoding="utf-8"))["features"]:
        t = f["properties"]["IFT_IOA"]
        if t not in names:
            names.append(t)
        g = f["geometry"]
        polys = [g["coordinates"]] if g["type"] == "Polygon" else g["coordinates"]
        for p in polys:
            d.polygon([px(*c[:2]) for c in p[0]], fill=names.index(t))
            for hole in p[1:]:
                d.polygon([px(*c[:2]) for c in hole], fill=0)
    return np.asarray(im), names


def rows(osm):
    stem = os.path.splitext(osm)[0]
    j = json.load(open(stem + ".crowns.json", encoding="utf-8"))
    w, h = Image.open(stem + ".cover.png").size
    lab, names = nfi_raster(osm, w, h)
    out = []
    for c in j["crowns"]:
        x, y = min(w - 1, int(c["u"] * w)), min(h - 1, int(c["v"] * h))
        out.append((names[lab[y, x]], c))
    return out


if __name__ == "__main__":
    total = defaultdict(lambda: [0, 0])
    for osm in sys.argv[1:]:
        print(f"== {os.path.basename(osm)}")
        by = defaultdict(list)
        for t, c in rows(osm):
            if c["kind"] != "bush":
                by[t].append(c)
        for t, cs in sorted(by.items(), key=lambda kv: -len(kv[1])):
            con = sum(c["kind"] == "conifer" for c in cs) / len(cs)
            med = " ".join(f"{k} {np.median([c[k] for c in cs]):.3g}" for k in KEYS)
            lo, hi = EXPECT.get(t, (None, None))
            ok = "" if lo is None else (" ок" if lo <= con <= hi else f" ждали {lo:.0%}–{hi:.0%}")
            print(f"  {t:26} {len(cs):4} хвойных {con:4.0%}{ok} | {med}")
            if lo is not None:
                total[t][0] += sum(c["kind"] == "conifer" for c in cs)
                total[t][1] += len(cs)
    print("== всего")
    for t, (k, n) in total.items():
        lo, hi = EXPECT[t]
        print(f"  {t:26} {n:4} хвойных {k / n:4.0%} (ждали {lo:.0%}–{hi:.0%})")
