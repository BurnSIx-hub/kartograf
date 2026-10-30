"""Мостовая из неправильных плит: каждая плита остаётся целой.

Лоскуты режут крупные камни, а камни неправильной формы не встают в ряды, как в masonry.py.
Поэтому на замкнутой плитке строится мозаика Вороного, а камни исходника вписываются в её
ячейки радиальной деформацией: кромка камня ложится на границу ячейки вместе с мазками.

python paving.py исходник.png выход.webp сторона ячеек шов сид
"""
import sys
import numpy as np
from PIL import Image
from scipy import ndimage as ni
from scipy.spatial import cKDTree

src, dst = sys.argv[1], sys.argv[2]
side, cells, gap, seed = (int(v) for v in sys.argv[3:7])
rng = np.random.default_rng(seed)
BINS = 180
ANGLES = np.linspace(-np.pi, np.pi, BINS, endpoint=False)


def profile(mask):
    """Центр и радиус фигуры по углам."""
    ys, xs = np.nonzero(mask)
    cy, cx = ys.mean(), xs.mean()
    dy, dx = ys - cy, xs - cx
    k = ((np.arctan2(dy, dx) + np.pi) / (2 * np.pi) * BINS).astype(int) % BINS
    r = np.zeros(BINS)
    np.maximum.at(r, k, np.hypot(dy, dx))
    # Пустые углы и зазубрины пикселей.
    r = np.where(r > 0, r, np.nan)
    good = ~np.isnan(r)
    r = np.interp(np.arange(BINS), np.arange(BINS)[good], r[good], period=BINS)
    return cy, cx, ni.gaussian_filter1d(r, 1.5, mode="wrap")


a = np.asarray(Image.open(src).convert("RGB"), dtype=np.float32)
warm = ni.gaussian_filter(a[..., 0] - a[..., 2], 1.0)
body = ni.binary_opening(warm < 24, iterations=4)
labels, n = ni.label(body)
stones = []
for k, box in enumerate(ni.find_objects(labels), 1):
    y, x = box
    if y.start < 3 or x.start < 3 or y.stop > a.shape[0] - 3 or x.stop > a.shape[1] - 3:
        continue
    m = ni.binary_fill_holes(labels[box] == k)
    if m.sum() < 5000 or m.mean() < 0.55:
        continue
    m = ni.binary_dilation(m, iterations=2)
    cy, cx, r = profile(m)
    stones.append((cy + y.start, cx + x.start, r))
print(len(stones), "камней")
joint = np.median(a[ni.binary_erosion(warm > 30, iterations=2)], 0)

# Замкнутая мозаика: несколько шагов Ллойда убирают щепки, разброс размеров остаётся.
sites = rng.random((cells, 2)) * side
grid = np.indices((side, side)).reshape(2, -1).T.astype(np.float32)
shift = np.array([[y, x] for y in (-side, 0, side) for x in (-side, 0, side)])
for step in range(4):
    tree = cKDTree((sites[None] + shift[:, None]).reshape(-1, 2))
    near = tree.query(grid, workers=-1)[1] % cells
    if step == 3:
        break
    ang = grid * 2 * np.pi / side
    for c in range(2):
        s = np.bincount(near, np.sin(ang[:, c]), cells)
        q = np.bincount(near, np.cos(ang[:, c]), cells)
        mean = (np.arctan2(s, q) / (2 * np.pi) * side) % side
        sites[:, c] = sites[:, c] + 0.7 * (((mean - sites[:, c] + side / 2) % side) - side / 2)
near = near.reshape(side, side)

noise = ni.gaussian_filter(rng.normal(0, 1, (side, side)), 5, mode="wrap")
out = joint[None, None] * (1 + (noise / noise.std() * 0.025)[..., None])
recent = []
for c in range(cells):
    # Окно вокруг ячейки: плитка замкнута, ячейка может пересекать край.
    R = 260
    oy, ox = int(sites[c, 0]) - R, int(sites[c, 1]) - R
    ys, xs = (oy + np.arange(2 * R)) % side, (ox + np.arange(2 * R)) % side
    g = np.ix_(ys, xs)
    m = near[g] == c
    # Скругление и зазор: щель между плитами остаётся песчаной.
    m = ni.gaussian_filter(m.astype(np.float32), gap * 1.3) > 0.62
    if m.sum() < 400:
        continue
    cy, cx, rc = profile(m)
    # Камень с похожим контуром мнётся меньше; поворот мал, чтобы свет на кромке не прыгал.
    best, score = None, 1e9
    for s in rng.permutation(len(stones)):
        if s in recent:
            continue
        for turn in rng.integers(-16, 17, 6):
            q = np.roll(stones[s][2], turn)
            v = float(np.std(np.log(rc / q)))
            scale = float(np.exp(np.mean(np.log(q / rc))))
            v += 0.25 * abs(np.log(scale))
            if v < score:
                best, score = (s, turn, q), v
    s, turn, q = best
    recent = (recent + [s])[-14:]
    sy, sx = stones[s][:2]
    yy, xx = np.nonzero(m)
    dy, dx = yy - cy, xx - cx
    t = np.arctan2(dy, dx)
    k = (t + np.pi) / (2 * np.pi) * BINS
    rho = np.hypot(dy, dx) / np.maximum(np.interp(k, np.arange(BINS), rc, period=BINS), 1)
    t2 = t - turn * 2 * np.pi / BINS
    r2 = rho * np.interp(k, np.arange(BINS), q, period=BINS)
    py, px = sy + r2 * np.sin(t2), sx + r2 * np.cos(t2)
    rgb = np.stack([ni.map_coordinates(a[..., i], [py, px], order=1, mode="nearest") for i in range(3)], 1)
    soft = np.ones_like(m, dtype=np.float32)
    soft[~m] = 0
    alpha = ni.gaussian_filter(soft, 0.8)
    lit = 1 + rng.normal(0, 0.025)
    layer = out[g]
    patch = layer.copy()
    patch[yy, xx] = rgb * lit
    # Края окна пустые, поэтому смешиваем только внутри ячейки.
    out[g] = layer * (1 - alpha[..., None]) + patch * alpha[..., None]

Image.fromarray(out.clip(0, 255).astype(np.uint8)).save(dst, quality=90, method=6)
print(dst, cells, "плит")
