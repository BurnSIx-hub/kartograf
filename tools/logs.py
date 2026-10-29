"""Торцы брёвен для частокола: вырезать целые круги из картинки Gemini.

    logs.py src.jpg dst.png [--count 8] [--size 128]

Светлое дерево на тёмной коре: порог, связные области, отбор самых круглых,
не задетых краем кадра. Каждый круг обрезается по своей коре мягкой маской
и получает тёмную обводку. Результат — ряд кругов на прозрачном фоне.
"""
import argparse
import numpy as np
from PIL import Image
from scipy import ndimage

ap = argparse.ArgumentParser()
ap.add_argument("src")
ap.add_argument("dst")
ap.add_argument("--count", type=int, default=8)
ap.add_argument("--size", type=int, default=128)
a = ap.parse_args()

img = Image.open(a.src).convert("RGB")
rgb = np.asarray(img).astype(np.float32)
lum = rgb @ np.array([0.299, 0.587, 0.114], np.float32)
wood = ndimage.binary_opening(lum > np.percentile(lum, 45), iterations=2)
wood = ndimage.binary_fill_holes(wood)
labels, n = ndimage.label(wood)
h, w = lum.shape
found = []
for k, sl in enumerate(ndimage.find_objects(labels), 1):
    ys, xs = sl
    bh, bw = ys.stop - ys.start, xs.stop - xs.start
    if ys.start < 3 or xs.start < 3 or ys.stop > h - 3 or xs.stop > w - 3 or min(bh, bw) < 40:
        continue
    m = labels[sl] == k
    area = m.sum()
    cy, cx = ndimage.center_of_mass(m)
    r = np.sqrt(area / np.pi)
    yy, xx = np.mgrid[0:bh, 0:bw]
    disc = np.hypot(yy - cy, xx - cx) < r
    iou = (m & disc).sum() / (m | disc).sum()
    found.append((iou, ys.start + cy, xs.start + cx, r))
found.sort(reverse=True)
rs = np.median([f[3] for f in found[:40]])
good = [f for f in found if abs(f[3] - rs) < 0.08 * rs][: a.count]
print(len(found), "кругов, взято", len(good), "радиус", round(float(rs), 1), "iou", [round(float(f[0]), 3) for f in good])

S = a.size
out = Image.new("RGBA", (S * len(good), S))
for i, (_, cy, cx, r) in enumerate(good):
    # Кора — тёмное кольцо снаружи светлого дерева: берём его целиком.
    R = r * 1.13
    box = (cx - R, cy - R, cx + R, cy + R)
    tile = img.crop(tuple(int(round(v)) for v in box)).resize((S, S), Image.LANCZOS)
    t = np.asarray(tile).astype(np.float32)
    yy, xx = np.mgrid[0:S, 0:S] + 0.5
    d = np.hypot(yy - S / 2, xx - S / 2) / (S / 2)  # 1 — край круга
    px = 2.0 / S
    alpha = np.clip((0.97 - d) / px + 0.5, 0, 1)
    # Обводка: последние 4% радиуса темнеют до почти чёрного.
    ring = np.clip((d - 0.90) / 0.05, 0, 1)[..., None]
    t = t * (1 - 0.8 * ring) + np.array([22, 15, 10], np.float32) * 0.8 * ring
    rgba = np.dstack([np.clip(t, 0, 255), alpha * 255]).astype(np.uint8)
    out.paste(Image.fromarray(rgba, "RGBA"), (i * S, 0))
out.save(a.dst)
