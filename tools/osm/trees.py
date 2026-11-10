"""Анализ крон со снимка: каждое дерево участка — где стоит, какого размера, какой породы.

    python -I trees.py <участок.osm> [<участок.osm> ...]

Берёт снимок (satlib) и подсказку `<имя>.cover.png` (cover.py — сначала её).
Пишет рядом с .osm:
  `<имя>.crowns.json` — кроны: центр в долях рамки (u, v), радиус в метрах,
                        порода и признаки; импорт берёт деревья из него;
  `<имя>.crowns.jpg`  — для глаз: снимок и круги крон цветами пород.

Кроны ищутся по одной, как `Hint::crowns` в импорте (он — запасной путь без
этого файла): центр — самая глубокая точка пятна крон, радиус — глубина ×1,2,
в сплошном пологе — не шире 12–18 м; новая крона выходит из найденных хотя бы
на полрадиуса. Здесь — в родном разрешении снимка и с его цветом.

Породы: `broad` — лиственное, `conifer` — хвойное, `bush` — куст (крона уже 4 м).
Откуда порода (`src`): `nfi:<тип>` — доля хвойных по лесу NFI (nfi.py), `none` —
источника нет, импорт решает по тегу леса OSM и очереди `queue` (0 — первая на
хвойную). Как выбирается — в `species`.
"""
import json
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from scipy import ndimage

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import eu  # noqa: E402
import satlib  # noqa: E402

CELL_M = 1.524
TREES = (20, 70, 30)
CANOPY = (4.0 * CELL_M, 6.0 * CELL_M)  # кроны сплошного полога, радиус
LONE = 7.0 * CELL_M  # самая широкая одиночная крона, радиус
GROW = 1.2  # крона шире круга, вписанного в пятно
OVERLAP = 0.75  # соседи не ближе этой доли суммы радиусов
MIN = 0.9 * CELL_M  # самая узкая крона, радиус
BUSH = 1.3 * CELL_M  # уже — куст
# Ель и пихта уже: сплошной ельник Рустайна — кроны в 4–6 м, а не 12–18.
CONIFER_CANOPY = (1.4 * CELL_M, 2.2 * CELL_M)
CONIFER_LONE = 2.8 * CELL_M


def metres_per_px(osm, w):
    minlat, minlon, maxlat, maxlon = satlib.bounds(osm)
    lat = math.radians((minlat + maxlat) / 2)
    return (maxlon - minlon) * 111320 * math.cos(lat) / w


def blur(x, r):
    return ndimage.gaussian_filter(x, r)


def find(trees, mpp, needles=None):
    """Кроны по одной: [(x, y, r)] в пикселях снимка, r — в метрах. `needles` —
    доля хвойных в каждом пикселе (Copernicus): там кроны уже."""
    depth = ndimage.distance_transform_edt(trees) * mpp
    h, w = trees.shape
    ys, xs = np.nonzero(depth >= MIN)
    d = depth[ys, xs]
    order = np.lexsort((ys * w + xs, -d))
    rng = np.random.default_rng(31)
    out, bucket = [], {}
    size = 8 * CELL_M / mpp
    reach = int(math.ceil(2 * LONE / mpp / size))
    for k in order:
        x, y, inner = xs[k], ys[k], d[k]
        bx, by = int(x // size), int(y // size)
        near = [j for dx in range(-reach, reach + 1) for dy in range(-reach, reach + 1) for j in bucket.get((bx + dx, by + dy), ())]
        dist = [math.hypot(out[j][0] - x, out[j][1] - y) * mpp for j in near]
        if any(dd < out[j][2] for j, dd in zip(near, dist)):
            continue
        ring = (1.6 * inner + 0.5 * CELL_M) / mpp
        around = 0
        for i in range(16):
            px, py = int(x + ring * math.cos(i * math.pi / 8)), int(y + ring * math.sin(i * math.pi / 8))
            around += 0 <= px < w and 0 <= py < h and trees[py, px]
        k = float(needles[y, x]) if needles is not None else 0.0
        lone = LONE + (CONIFER_LONE - LONE) * k
        lo, hi = CANOPY[0] + (CONIFER_CANOPY[0] - CANOPY[0]) * k, CANOPY[1] + (CONIFER_CANOPY[1] - CANOPY[1]) * k
        cap = lone if inner * GROW <= lone and around < 12 else lo + (hi - lo) * rng.random()
        r = min(inner * GROW, cap)
        if any(dd < max(OVERLAP * (r + out[j][2]), out[j][2] + 0.5 * r) for j, dd in zip(near, dist)):
            continue
        bucket.setdefault((bx, by), []).append(len(out))
        out.append((int(x), int(y), float(r)))
    return out


def features(im, trees, crowns, mpp):
    """Признаки каждой кроны по снимку внутри круга в 0,8 её радиуса."""
    rgb = np.asarray(im, float) / 255
    hsv = np.asarray(im.convert("HSV"), float)
    h, s, v = hsv[..., 0] * 360 / 255, hsv[..., 1] / 255, hsv[..., 2] / 255
    exg = 2 * rgb[..., 1] - rgb[..., 0] - rgb[..., 2]
    fine = blur(np.abs(v - blur(v, 1.5)), 1.0)  # мелкая пестрота: иглы, листва
    coarse = blur(np.abs(blur(v, 1.5) - blur(v, 5)), 1.5)  # крупные комья света и тени
    land = ~((v < 0.13) & (blur(np.abs(v - blur(v, 3)), 6) < 0.03))
    med = float(np.median(v[land & ~trees])) if (land & ~trees).any() else float(np.median(v))
    H, W = v.shape
    yy, xx = np.mgrid[0:H, 0:W]
    out = []
    for x, y, r in crowns:
        rp = max(1.5, 0.8 * r / mpp)
        x0, x1, y0, y1 = int(max(0, x - rp)), int(min(W, x + rp + 1)), int(max(0, y - rp)), int(min(H, y + rp + 1))
        m = (xx[y0:y1, x0:x1] - x) ** 2 + (yy[y0:y1, x0:x1] - y) ** 2 <= rp * rp
        sl = (slice(y0, y1), slice(x0, x1))
        vv = v[sl][m]
        bright = vv > np.percentile(vv, 75)
        out.append({
            "light": float(vv.mean() / med),  # светлота к середине суши
            "light_top": float(np.percentile(vv, 90) / med),  # светлая сторона кроны
            "h": float(h[sl][m].mean()),
            "s": float(s[sl][m].mean()),
            "exg": float(exg[sl][m].mean()),
            "exg_top": float(exg[sl][m][bright].mean()),
            "fine": float(fine[sl][m].mean()),
            "coarse": float(coarse[sl][m].mean()),
            "fill": float(trees[sl][m].mean()),
        })
    return out, med


# Доля хвойных в лесу NFI по его типу (определения NFI: Broadleaved — хвойных
# меньше 20 %, Mixed mainly broadleaved — 20–50 %, Mixed mainly conifer — 50–80 %,
# Conifer — больше 80 %). Вырубка и молодняк в Озёрном крае — чаще хвойная
# подсадка, но не наверняка. В лиственном лесу хвойных до 20 %, но какие из крон —
# снимок не скажет, а наугад — те же «ёлки не к месту»: там их нет.
NFI_CONIFER = {"Conifer": 0.9, "Mixed mainly conifer": 0.65, "Mixed mainly broadleaved": 0.35, "Broadleaved": 0.0,
               "Young trees": 0.4, "Felled": 0.4, "Ground prep": 0.4, "Coppice": 0.0, "Coppice with standards": 0.05,
               "Shrub": 0.1, "Low density": 0.2, "Assumed woodland": 0.2}


def merc(lat):
    s = math.sin(math.radians(lat))
    return math.log((1 + s) / (1 - s)) / 2


def nfi(osm, w, h):
    """Леса NFI (nfi.py) в пикселях снимка: (номер леса в каждом пикселе, тип каждого;
    0 — вне лесов). Нет файла — (None, [])."""
    path = os.path.join(os.path.dirname(os.path.abspath(osm)), "nfi", os.path.splitext(os.path.basename(osm))[0] + ".geojson")
    if not os.path.exists(path):
        return None, []
    minlat, minlon, maxlat, maxlon = satlib.bounds(osm)

    def px(lon, lat):
        return ((lon - minlon) / (maxlon - minlon) * w, (merc(maxlat) - merc(lat)) / (merc(maxlat) - merc(minlat)) * h)

    im = Image.new("I", (w, h), 0)
    d = ImageDraw.Draw(im)
    types = ["вне"]
    for f in json.load(open(path, encoding="utf-8")).get("features", []):
        types.append(f["properties"]["IFT_IOA"])
        g = f["geometry"]
        for poly in [g["coordinates"]] if g["type"] == "Polygon" else g["coordinates"]:
            d.polygon([px(*c[:2]) for c in poly[0]], fill=len(types) - 1)
            for hole in poly[1:]:
                d.polygon([px(*c[:2]) for c in hole], fill=0)
    return np.asarray(im), types


def zscore(a):
    a = np.asarray(a, float)
    spread = np.percentile(a, 75) - np.percentile(a, 25)
    return (a - np.median(a)) / (spread / 1.35 + 1e-9)


def clumps(x, y, mpp, seed=5):
    """Плавный шум места, ячейка ~25 м: хвойные в смешанном лесу — куртинами и
    посадками, а не вперемешку по одной."""
    cell = 25 / mpp
    gx, gy = x / cell, y / cell
    x0, y0 = math.floor(gx), math.floor(gy)
    fx, fy = gx - x0, gy - y0
    sx, sy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)

    def hsh(i, j):
        v = (i * 374761393 + j * 668265263 + seed * 2246822519) & 0xFFFFFFFF
        v = ((v ^ (v >> 13)) * 1274126177) & 0xFFFFFFFF
        return ((v ^ (v >> 16)) & 0xFFFF) / 65535 * 2 - 1

    top = hsh(x0, y0) * (1 - sx) + hsh(x0 + 1, y0) * sx
    bottom = hsh(x0, y0 + 1) * (1 - sx) + hsh(x0 + 1, y0 + 1) * sx
    return top * (1 - sy) + bottom * sy


def species(crowns, feats, im, cover, osm, mpp):
    """Порода каждой кроны: [(kind, откуда, место в очереди на хвойную 0..1)] и
    сведения о снимке.

    Цвет снимка в 0,35 м хвою от листвы надёжно не отличает: в лиственных лесах
    NFI Болтона и Лулворта «самые тёмные и холодные» кроны — теневые стороны
    лиственных, вода, асфальт и тени домов. Поэтому долю хвойных задаёт источник —
    лес NFI под кроной (здесь) или тег `leaf_type` леса OSM (в импорте), — а снимок
    только решает, какие кроны встают в очередь первыми: тёмные и холодные среди
    явной зелени, и то лишь когда снимку можно верить (кроны зеленее травы — летний
    снимок с освещёнными кронами; иначе в «кроны» попали тени или голые ветви).
    Остальное в очереди — куртины ~25 м. Вне лесов хвойных нет: одинокие деревья и
    живые изгороди Англии — лиственные."""
    rgb = np.asarray(im, float) / 255
    exg = 2 * rgb[..., 1] - rgb[..., 0] - rgb[..., 2]
    grass = np.abs(cover - np.array((120, 190, 90))).sum(-1) < 30
    grass_exg = float(np.median(exg[grass])) if grass.any() else 0.1
    big = [f for (_, _, r), f in zip(crowns, feats) if r >= BUSH]
    green = float(np.median([f["exg_top"] for f in big])) if big else 0.0
    trusted = green >= 1.1 * grass_exg
    valid = [r >= BUSH and 70 <= f["h"] <= 160 and f["exg_top"] >= 0.8 * grass_exg for (_, _, r), f in zip(crowns, feats)]
    score = np.zeros(len(crowns))
    if trusted and sum(valid) >= 10:
        idx = [i for i, ok in enumerate(valid) if ok]
        z = -zscore([feats[i]["light_top"] for i in idx]) + 0.5 * zscore([feats[i]["h"] for i in idx])
        score[idx] = np.clip(z, -3, 3) / 3
    queue = np.array([clumps(x, y, mpp) for x, y, _ in crowns]) + 0.5 * score
    rank = np.argsort(np.argsort(-queue)) / max(len(crowns) - 1, 1)  # 0 — первая в очереди
    ids, types = nfi(osm, im.width, im.height)
    out = [("bush" if r < BUSH else "broad", "none", float(rank[i])) for i, (_, _, r) in enumerate(crowns)]
    dlt = eu.load(osm, "dlt", im.width, im.height) if ids is None else None
    if dlt is not None:
        # Вне Англии — Copernicus (eu.py): доля хвойных среди лесных пикселей в 10 м
        # вокруг кроны; какие именно кроны — по той же очереди. Вне леса по
        # Copernicus — как без источника.
        reach = max(1, round(10 / mpp))
        for i, (x, y, r) in enumerate(crowns):
            win = dlt[max(0, y - reach):y + reach + 1, max(0, x - reach):x + reach + 1]
            forest = (win == 1) | (win == 2)
            if forest.sum() < 0.2 * win.size:
                continue
            share = float((win == 2).sum() / forest.sum())
            # Малая крона в ельнике — молодая ель, а не куст.
            if r < BUSH and share < 0.6:
                continue
            out[i] = ("conifer" if out[i][2] < share else ("bush" if r < BUSH else "broad"), f"dlt:{share:.2f}", out[i][2])
    if ids is not None:
        groups = {}
        for i, (x, y, r) in enumerate(crowns):
            if r >= BUSH and ids[y, x]:
                groups.setdefault(int(ids[y, x]), []).append(i)
        for g, members in groups.items():
            share = NFI_CONIFER.get(types[g], 0.2)
            members.sort(key=lambda i: -queue[i])
            n = round(share * len(members))
            for k, i in enumerate(members):
                out[i] = ("conifer" if k < n else "broad", "nfi:" + types[g], out[i][2])
    return out, {"trusted": bool(trusted), "crown_green": round(green, 3), "grass_green": round(grass_exg, 3)}


COLORS = {"broad": (90, 230, 60), "conifer": (40, 160, 255), "bush": (255, 240, 60)}


def run(osm):
    stem = os.path.splitext(osm)[0]
    im, _, _ = satlib.work(osm)
    cv = np.asarray(Image.open(stem + ".cover.png").convert("RGB"), int)
    if cv.shape[:2] != (im.height, im.width):
        cv = np.asarray(Image.fromarray(cv.astype(np.uint8)).resize(im.size, Image.NEAREST), int)
    trees = np.abs(cv - np.array(TREES)).sum(-1) < 30
    mpp = metres_per_px(osm, im.width)
    dlt = eu.load(osm, "dlt", im.width, im.height)
    needles = None
    if dlt is not None and nfi(osm, 1, 1)[0] is None:
        win = max(3, round(20 / mpp))
        forest = ndimage.uniform_filter(((dlt == 1) | (dlt == 2)).astype(float), win)
        needles = np.where(forest > 0.2, ndimage.uniform_filter((dlt == 2).astype(float), win) / np.maximum(forest, 1e-6), 0.0)
    crowns = find(trees, mpp, needles)
    feats, med = features(im, trees, crowns, mpp)
    kinds, photo = species(crowns, feats, im, cv, osm, mpp)
    rows = []
    for (x, y, r), f, (k, src, q) in zip(crowns, feats, kinds):
        rows.append({"u": (x + 0.5) / im.width, "v": (y + 0.5) / im.height, "r": round(r, 2), "kind": k, "src": src, "queue": round(q, 4),
                     **{key: round(val, 4) for key, val in f.items()}})
    json.dump({"metres_per_px": mpp, "median_v": med, "photo": photo, "crowns": rows}, open(stem + ".crowns.json", "w", encoding="utf-8"), ensure_ascii=False, indent=0)
    vis = im.copy()
    d = ImageDraw.Draw(vis)
    for (x, y, r), row in zip(crowns, rows):
        rp = r / mpp
        d.ellipse((x - rp, y - rp, x + rp, y + rp), outline=COLORS[row["kind"]], width=2)
    vis.save(stem + ".crowns.jpg", quality=88)
    counts = " ".join(f"{k} {sum(1 for row in rows if row['kind'] == k)}" for k in COLORS)
    trust = "снимку верим" if photo["trusted"] else "снимку не верим"
    print(os.path.basename(stem), f"крон {len(rows)}", counts, trust, f"(зелень крон {photo['crown_green']}, травы {photo['grass_green']})")


for osm in sys.argv[1:] if __name__ == "__main__" else []:
    run(osm)
