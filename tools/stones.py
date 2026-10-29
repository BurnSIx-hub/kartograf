"""Бесшовная плитка для кладки и плит: сдвиг на половину и вклейка по швам.

Проход 1: картинка сдвигается на половину по ширине — края по бокам сходятся, зато
посередине встаёт вертикальный разрыв. Его закрывает полоса исходной картинки,
вырезанная по двум путям через тёмные швы раствора (самый дешёвый путь, цена —
яркость обеих картинок). Проход 2 — то же по высоте, путь замкнут по кругу, чтобы
стык слева и справа совпал. Камни режутся по швам, а не пополам.

python stones.py вход.jpg выход.png
"""
import sys
import numpy as np
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
img = np.asarray(Image.open(src).convert("RGB").resize((1024, 1024), Image.LANCZOS), dtype=np.float32)
N = img.shape[0]
Q = 4  # путь ищется на сетке вчетверо мельче
K = 3  # на сколько клеток сетки путь может уйти вбок за шаг


def lum(a):
    return a.mean(2)


def small(c):
    n = c.shape[0] // Q
    return c.reshape(n, Q, n, Q).mean((1, 3))


def path(cost, lo, hi, cyclic=False):
    """Путь сверху вниз по `cost` (строки), столбец в [lo, hi), шаг вбок до K."""
    rows = cost.shape[0]
    band = cost[:, lo:hi]
    width = hi - lo

    def run(start=None):
        acc = band[0].copy()
        if start is not None:
            acc[:] = np.inf
            acc[start] = band[0, start]
        back = np.zeros((rows, width), dtype=np.int64)
        for y in range(1, rows):
            best = np.full(width, np.inf)
            arg = np.zeros(width, dtype=np.int64)
            for d in range(-K, K + 1):
                shifted = np.full(width, np.inf)
                if d >= 0:
                    shifted[d:] = acc[:width - d] if d else acc
                else:
                    shifted[:d] = acc[-d:]
                better = shifted < best
                best[better] = shifted[better]
                arg[better] = (np.arange(width) - d)[better]
            acc = best + band[y] + 0.15 * np.abs(np.arange(width) - arg)
            back[y] = arg
        end = start if start is not None else int(np.argmin(acc))
        total = acc[end]
        xs = [end]
        for y in range(rows - 1, 0, -1):
            xs.append(back[y][xs[-1]])
        return total, np.array(xs[::-1]) + lo

    if not cyclic:
        return run()[1]
    # Замкнутый путь: начало и конец в одном столбце — перебор начала.
    return min((run(s) for s in range(0, width, 2)), key=lambda r: r[0])[1]


def full_path(p):
    """Путь с мелкой сетки на полную: столбец для каждой строки."""
    ys = (np.arange(N) + 0.5) / Q - 0.5
    return np.interp(ys, np.arange(len(p)), p * Q + Q / 2)


def band_mask(left, right):
    xs = np.arange(N)[None, :]
    a = np.clip((xs - left[:, None]) / 2.0 + 0.5, 0, 1)
    b = np.clip((right[:, None] - xs) / 2.0 + 0.5, 0, 1)
    return (a * b)[..., None]


def fix_vertical(base, cyclic):
    """Сдвиг по ширине и вклейка исходника по швам. Возвращает бесшовное по ширине."""
    shifted = np.roll(base, N // 2, axis=1)
    cost = small(lum(base) + lum(shifted))
    cost = (cost - cost.min()) / (np.ptp(cost) + 1e-6)
    n = N // Q
    left = path(cost, n // 4, n // 2 - 4, cyclic)
    right = path(cost, n // 2 + 4, 3 * n // 4, cyclic)
    m = band_mask(full_path(left), full_path(right))
    return m * base + (1 - m) * shifted


step1 = fix_vertical(img, cyclic=False)
# Проход 2 — по высоте: поворот на четверть, тот же разрыв, путь замкнут.
step2 = np.rot90(fix_vertical(np.rot90(step1, 1), cyclic=True), -1)
out = np.clip(step2, 0, 255).astype(np.uint8)
Image.fromarray(out).save(dst)
g = out.astype(np.float32).mean(2)
ix, iy = np.abs(np.diff(g, axis=1)).mean(), np.abs(np.diff(g, axis=0)).mean()
print(f"шов л-п {np.abs(g[:, 0] - g[:, -1]).mean() / ix:.2f}, в-н {np.abs(g[0] - g[-1]).mean() / iy:.2f}")
