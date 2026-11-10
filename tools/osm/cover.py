"""Подсказка со снимка: что растёт и лежит на участке, классами по пикселю.

    python -I cover.py <участок.osm> [<участок.osm> ...]

Пишет рядом с .osm `<имя>.cover.png` (импорт подхватывает его сам) и для глаз
`<имя>.cover.jpg` (снимок и классы рядом). Снимок — плитки Esri World Imagery
по рамке участка (satlib), в родном разрешении: zoom 18 — около 0,35 м на пиксель.

Классы — цветами (импорт берёт ближайший):
  трава   (120, 190,  90) — открытая трава, луг, газон;
  кроны   ( 20,  70,  30) — деревья и кусты с их тенью;
  бурое   (170,  90,  50) — орляк, вереск (тёмное тёплое);
  камень  (220, 220, 210) — голый камень, галька, песок, светлые дорожки;
  вода    ( 30,  60, 160);
  сухое   (210, 190, 110) — скошенная, выгоревшая трава (светлое тёплое);
  земля   (150, 120,  85) — голая земля: мергель, пашня, утоптанное (только вне
                            зелёного биома);
  снег    (240, 245, 250) — снег и лёд (только в биоме льда).

Правила — по оттенку и светлоте относительно самого участка (у снимков разный
свет и баланс): кроны — заметно темнее середины, бурое — тёплый оттенок,
камень — светлое и серое, вода — тёмное и гладкое на большой площади или синее.
Биом снимка (photo.py) меняет правила: в Англии всё неопознанное — трава, а в
пустыне (Табернас), на сухом пастбище (Трухильо) и на леднике (Мер-де-Глас)
трава — только зелёное, кроны — только зеленоватое тёмное, камень — только серое
без тёплого оттенка, неопознанное — земля.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageFilter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import photo  # noqa: E402
import satlib  # noqa: E402

PALETTE = np.array([[120, 190, 90], [20, 70, 30], [170, 90, 50], [220, 220, 210], [30, 60, 160], [210, 190, 110], [150, 120, 85], [240, 245, 250]], np.uint8)
GRASS, TREES, BROWN, ROCK, WATER, DRY, EARTH, SNOW = range(8)
NAMES = ["grass", "trees", "brown", "rock", "water", "dry", "earth", "snow"]


def blur(x, r):
    return np.asarray(Image.fromarray(np.clip(x * 255, 0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(r)), float) / 255


def classify(im, biome="green"):
    green_biome = biome == "green"
    hsv = np.asarray(im.convert("HSV"), float)
    h, s, v = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255, hsv[..., 2] / 255
    vb, vbig = blur(v, 1.5), blur(v, 8)
    texture = blur(np.clip(np.abs(v - blur(v, 3)) * 4, 0, 1), 6)
    # Бирюзовое море Лулворта — оттенок 161–163 при насыщенности 0,9: тёмные
    # кроны не бирюзовее 143 и не насыщеннее 0,7.
    water = ((vbig < 0.13) & (texture < 0.12)) | ((h > 165) & (h < 250) & (s > 0.3)) | ((h > 150) & (h < 250) & (s > 0.75))
    # Середина — по суше: море в полкадра (Лулворт) тянуло её вниз, и лес не
    # выходил «темнее середины».
    land = v[~water] if (~water).any() else v
    med = np.median(land)
    # Кроны — тёмные пятна шире 3 м: открытие кругом 9 px (~3 м) снимает тонкие
    # тёмные линии — каменные ограды с тенью, — они на Малхеме вставали рядами деревьев.
    # Освещённая сторона кроны не темнее луга, а маска по одной тени брала
    # полкроны — и на карту вставали две-три мелких вместо одной большой.
    # Мерено на Болтоне: освещённая крона — оттенок 113–123, насыщенность
    # 0,37–0,63, пестрота 0,15–0,25; стриженый газон — 70–86, 0,35, 0,04.
    fine = blur(np.clip(np.abs(v - blur(v, 3)) * 4, 0, 1), 3)
    lit = (h > 105) & (h < 135) & (s > 0.35) & (fine > 0.11) & (vb > med * 0.6)
    shade = vb < med * 0.72
    sb = blur(s, 3)
    if not green_biome:
        # Тёмное без зелени — не кроны: в Трухильо тёмная бурая земля, на леднике
        # тени трещин вставали деревьями. Мерено: кусты Табернаса и дубы — оттенок
        # от 62, бурая земля — 37–45.
        shade &= (h >= 62) & (h <= 170) & (sb >= 0.15)
        # Дерево в сухом краю — тёмное пятно среди светлого; ровная тёмная земля
        # (низ Трухильо) темнее только середины снимка, а не своей округи.
        shade &= vb < blur(vb, 10) * 0.85
        # На леднике тень трещины голубая и тёмная — деревья там только явно зелёные.
        if biome == "ice":
            shade &= sb >= 0.25
    dark = Image.fromarray(((shade | lit) & ~water).astype(np.uint8) * 255)
    # Закрытие сливает тень и свет одной кроны, открытие снимает тонкое.
    trees = dark.filter(ImageFilter.MaxFilter(5)).filter(ImageFilter.MinFilter(5))
    trees = np.asarray(trees.filter(ImageFilter.MinFilter(9)).filter(ImageFilter.MaxFilter(9))) > 127
    trees &= ~water
    warm = (h < 62) & (s > 0.28) & (vb > 0.12) & ~trees & ~water
    # Тёплое светлое — сухая и скошенная трава, тёмное и рыжее — орляк и вереск.
    dry = warm & (vb > med * 1.05)
    brown = warm & ~dry
    # Камень — серое: светлое (мел, галька) или пёстрое (известняковая мостовая —
    # серые плиты с травой в трещинах, по светлоте не выше луга).
    # Мерено на Малхеме: мостовая — насыщенность 0,18–0,24 и пестрота 0,15–0,24,
    # луг — 0,34–0,39 и 0,10.
    rock = (((s < 0.2) & (vb > np.percentile(land, 75))) | ((sb < 0.27) & (texture > 0.13) & (vb > med * 0.85))) & ~water
    if green_biome:
        cls = np.full(v.shape, GRASS, np.uint8)
    else:
        # Вне зелёного биома камень — серое без тёплого оттенка: мергель Табернаса
        # (оттенок 55–58, насыщенность 0,18–0,21) — земля, гранит Трухильо и морена
        # ледника (оттенок 116–118, насыщенность 0,10–0,16, пестрота от 0,23) — камень.
        rock = (sb < 0.2) & ((h >= 62) | (sb < 0.1)) & (texture > 0.13) & ~water
        veg = (sb >= 0.25) & (h >= 60) & (h <= 150)
        cls = np.full(v.shape, EARTH, np.uint8)
        cls[veg] = GRASS
    cls[brown] = BROWN
    cls[dry] = DRY
    cls[rock] = ROCK
    if biome == "ice":
        # Лёд и снег — светлое почти бесцветное и гладкое; пёстрое на леднике — морена.
        snow = (vb > 0.55) & (sb < 0.16) & (h >= 80) & (h <= 230) & (texture < 0.2) & ~water
        cls[snow] = SNOW
    cls[trees] = TREES
    cls[water] = WATER
    return cls


for osm in sys.argv[1:] if __name__ == "__main__" else []:
    im, fetched = satlib.image(osm)
    biome = photo.passport(osm, im)["biome"]
    cls = classify(im, biome)
    stem = os.path.splitext(osm)[0]
    Image.fromarray(PALETTE[cls]).save(stem + ".cover.png")
    both = Image.new("RGB", (im.width * 2 + 10, im.height), "white")
    both.paste(im, (0, 0))
    both.paste(Image.fromarray(PALETTE[cls]), (im.width + 10, 0))
    both.resize((both.width // 2, both.height // 2)).save(stem + ".cover.jpg", quality=85)
    shares = " ".join(f"{n} {(cls == k).mean():.0%}" for k, n in enumerate(NAMES) if (cls == k).any())
    print(os.path.basename(stem), biome, im.size, shares, "fetched", fetched)
