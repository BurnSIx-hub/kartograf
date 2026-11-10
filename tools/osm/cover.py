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
  сухое   (210, 190, 110) — скошенная, выгоревшая трава (светлое тёплое).

Правила — по оттенку и светлоте относительно самого участка (у снимков разный
свет и баланс): кроны — заметно темнее середины, бурое — тёплый оттенок,
камень — светлое и серое, вода — тёмное и гладкое на большой площади или синее.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageFilter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import satlib  # noqa: E402

PALETTE = np.array([[120, 190, 90], [20, 70, 30], [170, 90, 50], [220, 220, 210], [30, 60, 160], [210, 190, 110]], np.uint8)
GRASS, TREES, BROWN, ROCK, WATER, DRY = range(6)


def blur(x, r):
    return np.asarray(Image.fromarray(np.clip(x * 255, 0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(r)), float) / 255


def classify(im):
    hsv = np.asarray(im.convert("HSV"), float)
    h, s, v = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255, hsv[..., 2] / 255
    vb, vbig = blur(v, 1.5), blur(v, 8)
    texture = blur(np.clip(np.abs(v - blur(v, 3)) * 4, 0, 1), 6)
    water = ((vbig < 0.13) & (texture < 0.12)) | ((h > 165) & (h < 250) & (s > 0.3))
    # Середина — по суше: море в полкадра (Лулворт) тянуло её вниз, и лес не
    # выходил «темнее середины».
    land = v[~water] if (~water).any() else v
    med = np.median(land)
    # Кроны — тёмные пятна шире 3 м: открытие кругом 9 px (~3 м) снимает тонкие
    # тёмные линии — каменные ограды с тенью, — они на Малхеме вставали рядами деревьев.
    dark = Image.fromarray(((vb < med * 0.72) & ~water).astype(np.uint8) * 255)
    trees = np.asarray(dark.filter(ImageFilter.MinFilter(9)).filter(ImageFilter.MaxFilter(9))) > 127
    warm = (h < 62) & (s > 0.28) & (vb > 0.12) & ~trees & ~water
    # Тёплое светлое — сухая и скошенная трава, тёмное и рыжее — орляк и вереск.
    dry = warm & (vb > med * 1.05)
    brown = warm & ~dry
    # Камень — серое: светлое (мел, галька) или пёстрое (известняковая мостовая —
    # серые плиты с травой в трещинах, по светлоте не выше луга).
    # Мерено на Малхеме: мостовая — насыщенность 0,18–0,24 и пестрота 0,15–0,24,
    # луг — 0,34–0,39 и 0,10.
    sb = blur(s, 3)
    rock = (((s < 0.2) & (vb > np.percentile(land, 75))) | ((sb < 0.27) & (texture > 0.13) & (vb > med * 0.85))) & ~water
    cls = np.full(v.shape, GRASS, np.uint8)
    cls[brown] = BROWN
    cls[dry] = DRY
    cls[rock] = ROCK
    cls[trees] = TREES
    cls[water] = WATER
    return cls


for osm in sys.argv[1:]:
    im, fetched = satlib.image(osm)
    cls = classify(im)
    stem = os.path.splitext(osm)[0]
    Image.fromarray(PALETTE[cls]).save(stem + ".cover.png")
    both = Image.new("RGB", (im.width * 2 + 10, im.height), "white")
    both.paste(im, (0, 0))
    both.paste(Image.fromarray(PALETTE[cls]), (im.width + 10, 0))
    both.resize((both.width // 2, both.height // 2)).save(stem + ".cover.jpg", quality=85)
    shares = " ".join(f"{n} {(cls == k).mean():.0%}" for k, n in enumerate(["grass", "trees", "brown", "rock", "water", "dry"]))
    print(os.path.basename(stem), im.size, shares, "fetched", fetched)
