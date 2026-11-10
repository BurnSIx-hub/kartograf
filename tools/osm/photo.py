"""Паспорт снимка: какой это биом и можно ли верить правилам cover.py и trees.py.

    python -I photo.py <участок.osm> [...]

Пишет рядом с .osm `<имя>.photo.json` и печатает строку. cover.py берёт паспорт
сам (считает, если файла нет). Доли — от суши (без воды):
  green  — растительность: оттенок 60–150 при насыщенности от 0,25;
  straw  — сухое тёплое: оттенок до 62 при насыщенности от 0,25 (солома, выгоревшая
           трава, рыжая земля);
  grey   — голое: насыщенность меньше 0,2 (камень, мергель, лёд, бетон);
  ice    — лёд и снег: светлое и почти бесцветное, не тёплое (оттенок 80–230);
  shadow — глубокая тень: низкое солнце или горы.
Биом — по долям (пороги мерены на восьми участках, см. заметку «уроки местности»):
  ice     — льда и снега больше 20 % (Мер-де-Глас 2026: летний лёд ледника);
  barren  — голого больше половины при растительности меньше 20 % (Табернас);
  dry     — сухого больше 30 % при растительности меньше 45 % (Трухильо);
  bog     — мокрой земли по Copernicus (eu.py, слой waw) больше четверти (Виру
            раба: мох оливково-жёлтый, как выгоревшая трава, — по снимку это «сухое»);
  green   — остальное (Англия).
"""
import json
import os
import sys

import numpy as np
from scipy import ndimage

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import eu  # noqa: E402
import satlib  # noqa: E402


def measure(im):
    hsv = np.asarray(im.convert("HSV"), float)
    h, s, v = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255, hsv[..., 2] / 255
    g = ndimage.gaussian_filter
    vb, sb = g(v, 1.5), g(s, 1.5)
    texture = g(np.clip(np.abs(v - g(v, 3)) * 4, 0, 1), 6)
    water = ((g(v, 8) < 0.13) & (texture < 0.12)) | ((h > 165) & (h < 250) & (s > 0.3)) | ((h > 150) & (h < 250) & (s > 0.75))
    land = ~water
    n = max(land.sum(), 1)
    share = lambda m: float((m & land).sum() / n)
    green = (sb >= 0.25) & (h >= 60) & (h <= 150)
    straw = (sb >= 0.25) & (h < 62)
    grey = sb < 0.2
    # Снег — светлое и бесцветное при любом оттенке: у чисто белого оттенка нет
    # (0), и правило «оттенок 80–230» снег Мёнхсйоха не видело совсем. Лёд —
    # серо-голубое темнее снега (летний лёд Мер-де-Гласа).
    snow = (vb > 0.78) & (sb < 0.12)
    ice = (vb > 0.55) & (sb < 0.16) & (h >= 80) & (h <= 230) & ~snow
    shadow = vb < 0.15
    out = {"green": share(green), "straw": share(straw), "grey": share(grey), "snow": share(snow), "ice": share(ice), "shadow": share(shadow),
           "water": float(water.mean()), "median_v": float(np.median(v[land])) if land.any() else float(np.median(v))}
    if out["ice"] + out["snow"] > 0.2:
        biome = "ice"
    elif out["grey"] > 0.5 and out["green"] < 0.2:
        biome = "barren"
    elif out["straw"] > 0.3 and out["green"] < 0.45:
        biome = "dry"
    else:
        biome = "green"
    out["biome"] = biome
    return {k: (round(x, 3) if isinstance(x, float) else x) for k, x in out.items()}


def passport(osm, im=None):
    """Паспорт участка: из `<имя>.photo.json`, а нет — посчитать и записать."""
    path = os.path.splitext(osm)[0] + ".photo.json"
    if os.path.exists(path) and im is None:
        return json.load(open(path, encoding="utf-8"))
    if im is None:
        im, _, _ = satlib.work(osm)
    p = measure(im)
    waw = eu.load(osm, "waw")
    if waw is not None:
        known = waw < 254
        if known.mean() > 0.5:
            p["wet"] = round(float(np.isin(waw[known], (3, 4)).mean()), 3)
            if p["biome"] != "ice" and p["wet"] > 0.25:
                p["biome"] = "bog"
    json.dump(p, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    return p


if __name__ == "__main__":
    for osm in sys.argv[1:]:
        im, _, _ = satlib.work(osm)
        p = passport(osm, im)
        print(f"{os.path.basename(osm):20} {p['biome']:7} " + " ".join(f"{k} {p[k]:.0%}" for k in ("green", "straw", "grey", "snow", "ice", "shadow", "water")))
