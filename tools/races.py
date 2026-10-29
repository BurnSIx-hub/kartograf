"""Гномья и дроу-стены из готовой полосы кладки (`walls.py`), без новой генерации.

Гномья — гранит потеплее, по обоим краям бронзовая окова с заклёпками.
Дроу — тёмный обсидиан с фиолетовым отливом, швы раствора светятся.
Всё, что добавляется, идёт вдоль полосы с шагом, кратным её длине, — полоса
остаётся бесшовной.

python races.py полоса_кладки папка_вывода
"""
import sys
import numpy as np
from PIL import Image, ImageFilter

src, out = sys.argv[1], sys.argv[2]
im = Image.open(src).convert("RGBA")
rgba = np.asarray(im, dtype=np.float32)
H, W = rgba.shape[:2]
rgb, alpha = rgba[..., :3], rgba[..., 3:]
lum = rgb.mean(2, keepdims=True)


def save(arr, name):
    Image.fromarray(arr.clip(0, 255).astype(np.uint8), "RGBA").save(f"{out}/{name}.webp", lossless=True)


# --- гномья ---
granite = lum * np.array([1.08, 0.98, 0.88]) + np.array([6, 2, -4])
d = granite.copy()
band = max(8, round(H * 0.18))
edge = max(3, round(H * 0.045))  # тёмная обводка полосы уже есть — окова внутри неё
y = np.arange(H)[:, None]
for top in (True, False):
    lo, hi = (edge, edge + band) if top else (H - edge - band, H - edge)
    t = (y - lo) / max(1, hi - lo)
    inside = ((y >= lo) & (y < hi))[..., None]
    # Бронза: светлый блик посередине, тёмные кромки.
    shine = 1.0 - np.abs(t - 0.45) * 1.6
    bronze = np.array([112, 78, 42]) + shine[..., None] * np.array([62, 46, 24])
    d = np.where(inside, np.broadcast_to(bronze, d.shape), d)
    for yy in (lo, hi - 1):
        d[yy] = np.array([52, 34, 18])
    # Заклёпки — шаг, кратный длине полосы.
    n = max(1, round(W / (H * 0.9)))
    step = W / n
    r = band * 0.36
    cy = (lo + hi) / 2
    yy, xx = np.mgrid[0:H, 0:W]
    for k in range(n):
        cx = (k + 0.5) * step
        dist = np.sqrt((xx - cx) ** 2 + (yy - cy) ** 2)
        m = dist < r
        g = np.clip(1 - dist / r, 0, 1)
        d[m] = (np.array([90, 60, 30]) + g[..., None] * np.array([150, 125, 80]))[m]
save(np.concatenate([d, alpha], 2), "dwarven")

# --- дроу ---
dark = lum * np.array([0.55, 0.47, 0.70]) + np.array([4, 0, 12])
# Швы раствора — самые тёмные места кладки; они и светятся.
lm = lum[..., 0]
seam = np.clip((70 - lm) / 40, 0, 1)
inner = np.zeros((H, W))
inner[edge + 2:H - edge - 2] = 1
seam *= inner
glow = np.asarray(Image.fromarray((seam * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(max(2, H / 30))), dtype=np.float32) / 255
glow = np.clip(glow * 1.25, 0, 1)[..., None]
violet = np.array([190, 70, 255])
d = dark * (1 - glow * 0.5) + violet * glow * 0.55
core = (seam > 0.75)[..., None]
d = np.where(core, d * 0.55 + np.array([225, 160, 255]) * 0.45, d)
save(np.concatenate([d, alpha], 2), "drow")
print("ok", W, H)
