"""Покрытия крыш, нарисованные программой: бесшовные во обе стороны, 512 px —
две клетки при 256 px на клетку. Как у паков FA, ряды идут поперёк картинки,
низ картинки — к свесу: край каждого ряда лежит на следующем, под ним тень.

python roofs.py папка
"""
import sys
import numpy as np
from PIL import Image

N = 512
OUT = sys.argv[1]
rng = np.random.default_rng(7)
Y, X = np.mgrid[0:N, 0:N].astype(np.float32)


def noise(cells_x, cells_y=None, seed=0):
    """Плавный шум 0..1, бесшовный: сетка случайных значений по кругу."""
    cells_y = cells_y or cells_x
    g = np.random.default_rng(seed).random((cells_y, cells_x)).astype(np.float32)
    fx, fy = X / N * cells_x, Y / N * cells_y
    ix, iy = np.floor(fx).astype(int), np.floor(fy).astype(int)
    tx, ty = fx - ix, fy - iy
    tx, ty = tx * tx * (3 - 2 * tx), ty * ty * (3 - 2 * ty)
    a = g[iy % cells_y, ix % cells_x]
    b = g[iy % cells_y, (ix + 1) % cells_x]
    c = g[(iy + 1) % cells_y, ix % cells_x]
    d = g[(iy + 1) % cells_y, (ix + 1) % cells_x]
    return (a * (1 - tx) + b * tx) * (1 - ty) + (c * (1 - tx) + d * tx) * ty


def fbm(c, seed=0, oct=4):
    v, a, s = 0, 0.5, 0
    for k in range(oct):
        v = v + a * noise(c * 2 ** k, seed=seed + k)
        s += a
        a *= 0.5
    return v / s


def save(rgb, name):
    img = np.clip(rgb, 0, 255).astype(np.uint8)
    Image.fromarray(img, "RGB").save(f"{OUT}/{name}.webp", quality=92)


def rows(n):
    """Номер ряда и доля внутри ряда (0 — верх, 1 — нижний край)."""
    h = N / n
    r = np.floor(Y / h).astype(int)
    return r, (Y - r * h) / h


def per_tile(r, c, seed):
    """Случайное число на плитку (ряд, столбец)."""
    return ((np.sin(r * 12.9898 + c * 78.233 + seed) * 43758.5453) % 1.0).astype(np.float32)


def under_lip(t, width=0.18):
    """Тень у верха ряда — под краем ряда выше."""
    return 1 - 0.55 * np.clip(1 - t / width, 0, 1) ** 1.5


def clay(base, name, glaze=0.0):
    n_rows, n_cols = 8, 10
    r, t = rows(n_rows)
    w = N / n_cols
    c = np.floor(X / w).astype(int)
    u = (X - c * w) / w
    # Желобчатая черепица: поперёк — полукруг, светлее у гребня.
    round_ = np.sqrt(np.clip(1 - (2 * u - 1) ** 2, 0, 1))
    shade = 0.45 + 0.6 * round_
    shade *= under_lip(t) * (0.92 + 0.16 * t)
    var = per_tile(r, c, 1.0)
    col = np.array(base, np.float32) * (0.82 + 0.3 * var)[..., None]
    col *= (0.85 + 0.3 * fbm(16, seed=3))[..., None]
    out = col * shade[..., None]
    # Блик глазури по гребню.
    out += glaze * 255 * (np.clip(round_ - 0.85, 0, 1) / 0.15)[..., None] * (t > 0.2)[..., None] * 0.35
    # Шов между желобками.
    out *= (0.55 + 0.45 * np.clip(np.minimum(u, 1 - u) * w / 2.0, 0, 1))[..., None]
    save(out, name)


def slate(base, name):
    n_rows, n_cols = 9, 6
    r, t = rows(n_rows)
    w = N / n_cols
    xs = (X + (r % 2) * w / 2) % N
    c = np.floor(xs / w).astype(int)
    u = (xs - c * w) / w
    var = per_tile(r, c, 2.0)
    col = np.array(base, np.float32) * (0.8 + 0.35 * var)[..., None]
    col *= (0.8 + 0.4 * fbm(24, seed=5))[..., None]
    shade = under_lip(t, 0.22) * (0.9 + 0.15 * (1 - t))
    # Зазор между плитками и выщербленный нижний край.
    edge = np.minimum(u, 1 - u) * w
    gap = np.clip((edge - 1.0) / 2.0, 0, 1)
    chip = noise(64, 8, seed=9) * 0.12
    bottom = np.clip((1 - t - chip) * (N / n_rows) / 2.0, 0, 1)
    out = col * (shade * (0.35 + 0.65 * gap * bottom))[..., None]
    save(out, name)


def shingle(base, name):
    n_rows = 10
    r, t = rows(n_rows)
    out = np.zeros((N, N, 3), np.float32)
    g = np.random.default_rng(11)
    for row in range(n_rows):
        m = r == row
        # Дощечки разной ширины, сумма — ровно ширина картинки.
        cuts = np.sort(g.choice(np.arange(8, N, 4), size=13, replace=False))
        start = g.integers(0, N)
        edges = (np.concatenate([[0], cuts]) + start) % N
        idx = np.searchsorted(np.sort(edges), (X[m] % N), side="right")
        tile = (idx + row * 31).astype(np.float32)
        var = ((np.sin(tile * 7.31) * 9137.1) % 1.0)
        out[m] = np.array(base, np.float32) * (0.75 + 0.4 * var)[..., None]
        # Шов слева от дощечки.
        se = np.sort(edges)
        d = np.min(np.abs(((X[m])[:, None] - se[None, :] + N / 2) % N - N / 2), axis=1)
        out[m] *= (0.45 + 0.55 * np.clip(d / 2.5, 0, 1))[..., None]
    grain = 0.75 + 0.45 * noise(160, 6, seed=21)
    out *= grain[..., None]
    out *= (under_lip(t, 0.25) * (0.88 + 0.2 * t))[..., None]
    save(out, name)


def thatch(base, name):
    n_rows = 6
    r, t = rows(n_rows)
    straw = 0.55 + 0.6 * noise(220, 5, seed=31) * (0.6 + 0.4 * noise(60, 3, seed=33))
    col = np.array(base, np.float32) * straw[..., None]
    col *= (0.85 + 0.25 * fbm(8, seed=35))[..., None]
    # Нижний край пучков рваный: часть соломинок длиннее.
    ragged = noise(180, 4, seed=37) * 0.25
    tt = (t + ragged) % 1.0
    shade = 1 - 0.6 * np.clip(1 - tt / 0.3, 0, 1) ** 2
    save(col * shade[..., None], name)


def corrugated(base, name, rust=0.0):
    waves = 24
    prof = 0.65 + 0.35 * np.sin(X / N * waves * 2 * np.pi)
    col = np.array(base, np.float32) * (0.85 + 0.25 * fbm(10, seed=41))[..., None]
    if rust:
        rr = np.clip((fbm(12, seed=43) - 0.45) * 3, 0, 1) * rust
        col = col * (1 - rr[..., None]) + np.array([140, 70, 35], np.float32) * rr[..., None]
    # Листы внахлёст: шов раз в четверть высоты.
    _, t = rows(2)
    save(col * (prof * under_lip(t, 0.06))[..., None], name)


def copper(name):
    panels = 6
    w = N / panels
    u = (X % w) / w
    seam = np.exp(-((u - 0.5) * w / 2.5) ** 2)
    base = np.array([96, 150, 128], np.float32)
    pat = fbm(14, seed=51)
    col = base * (0.75 + 0.45 * pat)[..., None]
    brown = np.clip((0.35 - pat) * 4, 0, 1)
    col = col * (1 - brown[..., None]) + np.array([120, 85, 55], np.float32) * brown[..., None] * (0.8 + 0.3 * pat)[..., None]
    out = col * (0.85 + 0.25 * (u > 0.5))[..., None] + 70 * seam[..., None]
    save(out, name)


clay([178, 72, 48], "roof_clay_red")
clay([140, 92, 62], "roof_clay_brown")
clay([70, 120, 82], "roof_clay_green", glaze=1.0)
slate([82, 100, 128], "roof_slate_blue")
slate([78, 78, 84], "roof_slate_gray")
shingle([150, 108, 70], "roof_shingle")
thatch([196, 160, 96], "roof_thatch")
corrugated([138, 140, 142], "roof_metal", rust=0.6)
copper("roof_copper")
print("ok")
