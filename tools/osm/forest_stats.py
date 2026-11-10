"""Лес карты теми же мерами, что лес на снимках (урок кругов сверки OSM).

    python -I forest_stats.py <папка выгрузки> [...]

Кроны — тайлы деревьев из «Новая карта.json» (кругом 0,85 ширины тайла), вода —
синее и бирюзовое на фоне «Новая карта.webp». Печатает долю крон, кучность (во
сколько раз крон гуще в 15 м вокруг кроны, чем в среднем) и долю крон у воды:
в 10 м от неё, в 10–30 м и дальше. На снимках Англии: крон 10–13 %, кучность
3,6–6, у воды 27–55 % против 6–17 % в 10–30 м.
"""
import json
import os
import sys

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

CELL_M = 1.524
for d in sys.argv[1:]:
    j = json.load(open(os.path.join(d, "Новая карта.json"), encoding="utf-8"))
    bg = Image.open(os.path.join(d, "Новая карта.webp")).convert("RGB")
    k = 0.25  # уменьшение для скорости
    W, H = int(j["width"] * k), int(j["height"] * k)
    m_per_px = CELL_M / (j["grid"]["size"] * k)
    mask = Image.new("L", (W, H), 0)
    dr = ImageDraw.Draw(mask)
    for t in j["tiles"]:
        if "дерев" not in t["texture"]["src"]:
            continue
        r = 0.85 * t["width"] / 2 * k
        x, y = t["x"] * k, t["y"] * k
        dr.ellipse((x - r, y - r, x + r, y + r), fill=255)
    trees = np.asarray(mask) > 127
    hsv = np.asarray(bg.resize((W, H)).convert("HSV"), float)
    h, s = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255
    water = (h > 165) & (h < 250) & (s > 0.25)
    win = max(3, int(15 / m_per_px))
    dens = ndimage.uniform_filter(trees.astype(float), size=win)
    clump = dens[trees].mean() / max(trees.mean(), 1e-6) if trees.any() else 0
    bands = ""
    if water.mean() > 0.005:
        dist = ndimage.distance_transform_edt(~water) * m_per_px
        land = ~water
        parts = []
        for lo, hi in [(0, 10), (10, 30), (30, 1e9)]:
            sel = land & (dist >= lo) & (dist < hi)
            parts.append(f"{trees[sel].mean():.0%}" if sel.sum() > 200 else "-")
        bands = " у воды " + " / ".join(parts)
    print(f"{os.path.basename(d):20} крон {trees.mean():.0%}  кучность x{clump:.1f}{bands}")
