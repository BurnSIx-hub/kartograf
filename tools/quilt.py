"""Большая бесшовная плитка из маленькой: лоскуты шьются швом по щелям (Efros — Freeman).

Исходник — сырая картинка, не бесшовная: в бесшовной уже есть полупрозрачные наложения.
Шов идёт там, где два лоскута совпадают и где темно, поэтому камни и листья остаются
целыми, а не просвечивают друг сквозь друга, как при смешивании. Плитка замкнута.

python quilt.py сырьё.jpg выход.webp сторона_исходника лоскут лоскутов_в_ряду [flips]
"""
import sys
import numpy as np
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
SIDE, B, N = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
# Вытянутые ряды (брусчатка) поворачивать нельзя.
FLIPS_ONLY = len(sys.argv) > 6 and sys.argv[6] == "flips"
OV, TRIES = B // 4, 400
S = B + OV
rng = np.random.default_rng(7)
img = Image.open(src).convert("RGB")
q = min(img.size)
img = img.crop(((img.width - q) // 2, (img.height - q) // 2, (img.width + q) // 2, (img.height + q) // 2)).resize((SIDE, SIDE), Image.LANCZOS)
a = np.asarray(img, dtype=np.float32)
h, w, _ = a.shape
H = N * B
out = np.zeros((H, H, 3), np.float32)
done = np.zeros((H, H), bool)


def seam(err):
    """Путь сверху вниз с наименьшей суммой: номер столбца на каждой строке."""
    c = err.copy()
    for y in range(1, c.shape[0]):
        p = c[y - 1]
        c[y] += np.minimum(np.minimum(np.r_[np.inf, p[:-1]], p), np.r_[p[1:], np.inf])
    x = int(np.argmin(c[-1]))
    path = [x]
    for y in range(c.shape[0] - 2, -1, -1):
        lo, hi = max(x - 1, 0), min(x + 2, c.shape[1])
        x = lo + int(np.argmin(c[y, lo:hi]))
        path.append(x)
    return path[::-1]


def box(m):
    k = np.ones(3) / 3
    m = np.apply_along_axis(lambda r: np.convolve(np.r_[r[0], r, r[-1]], k, "valid"), 0, m)
    return np.apply_along_axis(lambda r: np.convolve(np.r_[r[0], r, r[-1]], k, "valid"), 1, m)


for r in range(N):
    for c in range(N):
        ys, xs = (r * B + np.arange(S)) % H, (c * B + np.arange(S)) % H
        have = done[np.ix_(ys, xs)]
        cur = out[np.ix_(ys, xs)]
        best, cost = [], []
        for _ in range(TRIES):
            oy, ox = rng.integers(h - S), rng.integers(w - S)
            cand = a[oy:oy + S, ox:ox + S]
            # Повороты и отражения: иначе из маленького исходника лист повторяется рядами.
            if not FLIPS_ONLY:
                cand = np.rot90(cand, rng.integers(4))
            if rng.random() < 0.5:
                cand = cand[:, ::-1]
            best.append(cand)
            cost.append(float((((cand - cur) ** 2).sum(2) * have).sum()))
        pick = np.argsort(cost)[: TRIES // 10 if have.any() else 1]
        cand = best[rng.choice(pick)]
        err = ((cand - cur) ** 2).sum(2) + 0.6 * (cand.mean(2) + cur.mean(2)) ** 2
        m = np.ones((S, S), np.float32)
        if have[S // 2, 0]:
            for y, x in enumerate(seam(err[:, :OV])):
                m[y, :x] = 0
        if have[0, S // 2]:
            for x, y in enumerate(seam(err[:OV, :].T)):
                m[:y, x] = 0
        if have[S // 2, S - 1]:
            for y, x in enumerate(seam(err[:, B:])):
                m[y, B + x:] = 0
        if have[S - 1, S // 2]:
            for x, y in enumerate(seam(err[B:, :].T)):
                m[B + y:, x] = 0
        m = np.where(have, box(m), 1.0)[..., None]
        out[np.ix_(ys, xs)] = cand * m + cur * (1 - m)
        done[np.ix_(ys, xs)] = True

# Крупные пятна выдают повтор плитки: то же выравнивание, что в seamless.py.
fy, fx = np.fft.fftfreq(H)[:, None], np.fft.fftfreq(H)[None, :]
kernel = np.exp(-2 * (np.pi * H / 10) ** 2 * (fx * fx + fy * fy))
for c in range(3):
    low = np.real(np.fft.ifft2(np.fft.fft2(out[..., c]) * kernel))
    out[..., c] -= 0.85 * (low - low.mean())

Image.fromarray(np.clip(out, 0, 255).astype(np.uint8)).save(dst, quality=90, method=6)
print(dst, out.shape)
