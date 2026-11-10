"""Лист сгенерированных предметов на белом фоне → отдельные PNG с прозрачностью.

    python -I cutout.py <лист.png|webp> <папка> <Имя> [--cells N] [--min-share 0.02]
                        [--rows 2] [--names Имя_A1,Имя_A2,…]

Фон — белое, связанное с краем листа (заливка от краёв): светлые блики внутри
кроны остаются. Каждый предмет — связное пятно не мельче `--min-share` листа;
сохраняется как `<Имя>_A<k>.png` с запасом по краям. Белая кайма по контуру
снимается обратным смешением с белым. `--cells N` — сколько клеток карты займёт
самая широкая сторона предмета: картинка ужимается до 256 точек на клетку, как у FA.
"""
import os
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

PX_PER_CELL = 256


def cut(sheet, out_dir, name, cells=None, min_share=0.02, tol=28, rows=2, names=None):
    im = Image.open(sheet).convert("RGB")
    a = np.asarray(im, float)
    # Близкое к белому: малая насыщенность и высокая светлота.
    whiteish = (a.min(-1) > 255 - tol) & ((a.max(-1) - a.min(-1)) < tol)
    lab, _ = ndimage.label(whiteish)
    edge = set(np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))) - {0}
    bg = np.isin(lab, list(edge))
    fg = ~bg
    fg = ndimage.binary_opening(fg, iterations=1)
    parts, n = ndimage.label(fg)
    sizes = ndimage.sum(fg, parts, range(1, n + 1))
    keep = [i + 1 for i, s in enumerate(sizes) if s >= min_share * fg.size]
    # Мягкий край: доля «небелого» у границы.
    dist_white = np.clip((255 - a.min(-1)) / 60, 0, 1)
    os.makedirs(out_dir, exist_ok=True)
    saved = []
    boxes = ndimage.find_objects(parts)
    # Порядок чтения: ряд — по середине пятна (`rows` рядов на листе), в ряду — слева направо.
    def place(i):
        sl = boxes[i - 1]
        cy, cx = (sl[0].start + sl[0].stop) / 2, (sl[1].start + sl[1].stop) / 2
        return int(cy * rows / a.shape[0]), cx
    order = sorted(keep, key=place)
    for k, i in enumerate(order, 1):
        sl = boxes[i - 1]
        pad = 12
        y0, y1 = max(0, sl[0].start - pad), min(a.shape[0], sl[0].stop + pad)
        x0, x1 = max(0, sl[1].start - pad), min(a.shape[1], sl[1].stop + pad)
        mask = ndimage.binary_dilation(parts[y0:y1, x0:x1] == i, iterations=2)
        alpha = np.where(mask, np.maximum(dist_white[y0:y1, x0:x1], ndimage.binary_erosion(mask, iterations=3)), 0.0)
        alpha = ndimage.gaussian_filter(alpha, 0.7) * mask
        rgb = a[y0:y1, x0:x1]
        # Обратное смешение с белым: c = a*x + (1-a)*255 → x.
        al = np.clip(alpha, 1e-3, 1)[..., None]
        rgb = np.clip((rgb - (1 - al) * 255) / al, 0, 255)
        img = Image.fromarray(np.dstack([rgb, alpha * 255]).astype(np.uint8), "RGBA")
        # Квадратный холст с центром предмета: кроны FA стоят в середине холста.
        side = max(img.size)
        canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))
        canvas.paste(img, ((side - img.width) // 2, (side - img.height) // 2))
        if cells:
            target = int(cells * PX_PER_CELL)
            canvas = canvas.resize((target, target), Image.LANCZOS)
        path = os.path.join(out_dir, names[k - 1] + ".png" if names and k <= len(names) else f"{name}_A{k}.png")
        canvas.save(path)
        saved.append((path, canvas.size))
    return saved


if __name__ == "__main__":
    args = sys.argv[1:]
    cells = float(args[args.index("--cells") + 1]) if "--cells" in args else None
    share = float(args[args.index("--min-share") + 1]) if "--min-share" in args else 0.02
    rows = int(args[args.index("--rows") + 1]) if "--rows" in args else 2
    names = args[args.index("--names") + 1].split(",") if "--names" in args else None
    for path, size in cut(args[0], args[1], args[2], cells, share, rows=rows, names=names):
        print(os.path.basename(path), size)
