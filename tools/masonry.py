"""Кладка без повтора: камни из картинок Gemini раскладываются заново, случайно.

Сшивать лоскутами плитку и кирпич нельзя: ряды разъедутся. Поэтому из исходников
вырезается каждый камень с его сколами и трещинами, а на замкнутой плитке строится
новая раскладка, и в неё камни ложатся случайно, с поворотом и отражением. Трещина
оказывается на случайном камне, и рисунок не складывается в заметный узор.

python masonry.py выход.webp сторона шов сид исходник... -- раскладка [moss=картинка]
вид: rows Н1,Н2 Д1,Д2,..  ряды высотой Н, камни длиной Д
     grid Ш                сетка клеток Ш
     pack Ш                прямоугольники 1-2 клетки Ш внахлёст
Сторона кратна размеру исходников: 256 px на клетку. Масштаб исходника — по SCALE.
"""
import sys
import numpy as np
from PIL import Image
from scipy import ndimage as ni

args = sys.argv[1:]
cut = args.index("--")
dst, side, gap, seed = args[0], int(args[1]), int(args[2]), int(args[3])
sources, layout = args[4:cut], args[cut + 1:]
moss = next((a[5:] for a in layout if a.startswith("moss=")), None)
layout = [a for a in layout if not a.startswith("moss=")]
rng = np.random.default_rng(seed)
SCALE = 1.0  # исходники уже 256 px на клетку


def otsu(v):
    h, e = np.histogram(v, 64)
    c = (e[:-1] + e[1:]) / 2
    w = h.cumsum().astype(float)
    m = (h * c).cumsum()
    t = w[-1]
    between = (m[-1] * w - m * t) ** 2 / np.maximum(w * (t - w), 1)
    return c[int(np.argmax(between))]


def stones(path):
    """Камни картинки: RGBA по рамке, и цвет швов."""
    img = Image.open(path).convert("RGB")
    img = img.resize((round(img.width * SCALE), round(img.height * SCALE)), Image.LANCZOS)
    a = np.asarray(img, dtype=np.float32)
    lum = ni.gaussian_filter(a.mean(2), 1.2)
    joint = lum < otsu(lum)
    body = ni.binary_opening(~joint, iterations=3)
    labels, n = ni.label(body)
    out = []
    for k, box in enumerate(ni.find_objects(labels), 1):
        y, x = box
        h, w = y.stop - y.start, x.stop - x.start
        if y.start < 3 or x.start < 3 or y.stop > a.shape[0] - 3 or x.stop > a.shape[1] - 3 or h < 24 or w < 24:
            continue
        pad = 5
        ys, xs = slice(max(y.start - pad, 0), y.stop + pad), slice(max(x.start - pad, 0), x.stop + pad)
        m = labels[ys, xs] == k
        # Весь камень с тёмной кромкой, дырки в нём (трещины) — тоже камень.
        m = ni.binary_fill_holes(ni.binary_closing(m, iterations=6))
        m = ni.binary_dilation(m, iterations=2)
        # Камень занимает свою рамку: Г-образные оставили бы дыры.
        yy, xx = np.nonzero(m)
        tight = slice(yy.min(), yy.max() + 1), slice(xx.min(), xx.max() + 1)
        m = m[tight]
        if m.mean() < 0.86:
            continue
        ys, xs = slice(ys.start + tight[0].start, ys.start + tight[0].stop), slice(xs.start + tight[1].start, xs.start + tight[1].stop)
        alpha = ni.gaussian_filter(m.astype(np.float32), 0.9) * 255
        rgb = a[ys, xs]
        # Трещины и выбоины — тёмное на камне вдали от края. Убрав их, получаем чистый
        # камень того же мазка: иначе почти каждый камень в раскладке выходит битым.
        inner = ni.binary_erosion(m, iterations=9)
        g = rgb.mean(2)
        flaw = ni.binary_dilation((g < ni.gaussian_filter(g, 14) - 16) & inner, iterations=2)
        damage = flaw.sum() / max(inner.sum(), 1)
        out.append((np.dstack([rgb, alpha]), damage))
        if damage > 0.004:
            keep = (~flaw).astype(np.float32)
            fill = np.dstack([ni.gaussian_filter(rgb[..., c] * keep, 7) / np.maximum(ni.gaussian_filter(keep, 7), 1e-3) for c in range(3)])
            soft = ni.gaussian_filter(flaw.astype(np.float32), 1.2)[..., None].clip(0, 1)
            out.append((np.dstack([rgb * (1 - soft) + fill * soft, alpha]), 0.0))
    return out, np.median(a[joint], 0)


def resized(p, w, h):
    return np.asarray(Image.fromarray(p.clip(0, 255).astype(np.uint8), "RGBA").resize((w, h), Image.BICUBIC), dtype=np.float32)


def rows(heights, lengths):
    out, y = [], 0
    while y < side:
        h = int(rng.choice(heights))
        if side - y - h < min(heights):
            h = side - y
        x, start = 0, int(rng.integers(side))
        while x < side:
            w = int(rng.choice(lengths))
            if side - x - w < min(lengths):
                w = side - x
            out.append(((x + start) % side, y, w, h))
            x += w
        y += h
    return out


def grid(u):
    n = round(side / u)
    return [(round(i * side / n), round(j * side / n), round((i + 1) * side / n) - round(i * side / n), round((j + 1) * side / n) - round(j * side / n)) for j in range(n) for i in range(n)]


def pack(u):
    n = side // u
    free = np.ones((n, n), bool)
    out = []
    for j in range(n):
        for i in range(n):
            if not free[j, i]:
                continue
            sizes = [(2, 2), (2, 1), (1, 2), (3, 2), (1, 1), (2, 3)]
            for s in rng.permutation(len(sizes)):
                w, h = sizes[s]
                cells = [((j + b) % n, (i + a) % n) for a in range(w) for b in range(h)]
                if all(free[c] for c in cells):
                    for c in cells:
                        free[c] = False
                    out.append((i * u, j * u, w * u, h * u))
                    break
    return out


pool, joints = [], []
for s in sources:
    p, j = stones(s)
    print(s.rsplit("/", 1)[-1], len(p))
    pool += p
    joints.append(j)
kind = layout[0]
nums = [[int(v) for v in x.split(",")] for x in layout[1:]]
rects = rows(*nums) if kind == "rows" else grid(*nums[0]) if kind == "grid" else pack(*nums[0])

mean = np.median(joints, 0)
noise = ni.gaussian_filter(rng.normal(0, 1, (side, side)), 6, mode="wrap")
out = mean[None, None] + (noise / noise.std() * 2)[..., None]
recent = []
cover = np.zeros((side, side), np.float32)
for x, y, w, h in rects:
    w2, h2 = w - gap, h - gap
    want = w2 / h2
    roll = rng.random()
    lo, hi = (0, 0.004) if roll < 0.8 else (0.004, 0.06) if roll < 0.985 else (0.06, 9)
    fits = [k for k in range(len(pool)) if lo <= pool[k][1] < hi] or list(range(len(pool)))
    k = min(fits, key=lambda k: abs(np.log(pool[k][0].shape[1] / pool[k][0].shape[0]) - np.log(want)) + rng.random() * 0.7 + (2 if k in recent else 0))
    recent = (recent + [k])[-12:]
    p = pool[k][0]
    if rng.random() < 0.5:
        p = p[::-1, ::-1]
    if rng.random() < 0.5:
        p = p[:, ::-1]
    r = resized(p, w2, h2)
    a = r[..., 3:] / 255
    lit = 1 + rng.normal(0, 0.03)
    ys, xs = (y + gap // 2 + np.arange(h2)) % side, (x + gap // 2 + np.arange(w2)) % side
    g = np.ix_(ys, xs)
    out[g] = out[g] * (1 - a) + r[..., :3] * lit * a
    cover[g] = np.maximum(cover[g], a[..., 0])

if moss:
    # Мох растёт из швов: куски зелени исходника, только у швов и пятнами.
    src = np.asarray(Image.open(moss).convert("RGB").resize((1024, 1024)), dtype=np.float32)
    green = np.clip((src[..., 1] - np.maximum(src[..., 0], src[..., 2]) - 4) / 18, 0, 1)
    near = ni.gaussian_filter(1 - cover, 8, mode="wrap") > 0.12
    field = ni.gaussian_filter(rng.normal(0, 1, (side, side)), 48, mode="wrap")
    patches = (ni.gaussian_filter((near & (field > field.std() * 0.5)).astype(np.float32), 3, mode="wrap"))
    ys, xs = np.nonzero(green > 0.6)
    size = 110
    for _ in range(900):
        i = rng.integers(len(ys))
        sy, sx = np.clip(ys[i] - size // 2, 0, 1024 - size), np.clip(xs[i] - size // 2, 0, 1024 - size)
        ty, tx = (int(rng.integers(side)) + np.arange(size)) % side, (int(rng.integers(side)) + np.arange(size)) % side
        g = np.ix_(ty, tx)
        a = (green[sy:sy + size, sx:sx + size] * patches[g])[..., None]
        out[g] = out[g] * (1 - a) + src[sy:sy + size, sx:sx + size] * a

Image.fromarray(out.clip(0, 255).astype(np.uint8)).save(dst, quality=88, method=6)
print(dst, len(pool), "камней,", len(rects), "мест")
