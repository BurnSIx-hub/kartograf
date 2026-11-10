"""Кроны карты против крон снимка: насколько лес стоит там же, где на самом деле.

    python -I canopy.py <папка выгрузки> <участок.cover.png> [out.png]

Кроны карты — тайлы деревьев из «Новая карта.json» (кругом 0,85 ширины тайла),
кроны снимка — класс «кроны» подсказки (cover.py). Печатает долю крон на снимке и
на карте, сколько крон снимка накрыто картой (полнота), сколько крон карты стоит
на кронах снимка (точность) и их пересечение к объединению (IoU).
`out` — картинка: зелёное — совпало, красное — лишнее на карте, синее — пропущено.
"""
import json
import os
import sys

import numpy as np
from PIL import Image, ImageDraw

d, cover = sys.argv[1], sys.argv[2]
out = sys.argv[3] if len(sys.argv) > 3 else None
j = json.load(open(os.path.join(d, "Новая карта.json"), encoding="utf-8"))
cv = np.asarray(Image.open(cover).convert("RGB"), int)
sat = (np.abs(cv - np.array([20, 70, 30])).sum(-1) < 30)
H, W = sat.shape
k = W / j["width"]
mask = Image.new("L", (W, H), 0)
dr = ImageDraw.Draw(mask)
for t in j["tiles"]:
    if "дерев" not in t["texture"]["src"]:
        continue
    r = 0.85 * t["width"] / 2 * k
    x, y = t["x"] * k, t["y"] * k
    dr.ellipse((x - r, y - r, x + r, y + r), fill=255)
mp = np.asarray(mask) > 127
inter, union = (sat & mp).sum(), (sat | mp).sum()
print(f"crowns sat {sat.mean():.0%} map {mp.mean():.0%}  recall {inter / max(sat.sum(), 1):.0%}  "
      f"precision {inter / max(mp.sum(), 1):.0%}  IoU {inter / max(union, 1):.0%}")
if out:
    vis = np.full((H, W, 3), 235, np.uint8)
    vis[sat & mp] = (40, 150, 60)
    vis[mp & ~sat] = (210, 70, 60)
    vis[sat & ~mp] = (60, 90, 210)
    Image.fromarray(vis).resize((W // 2, H // 2)).save(out)
