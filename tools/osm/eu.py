"""Европейские слои Copernicus (HRL 2018, сетка 10 м) по рамке участка — то, чего нет
в снимке и OSM вне Англии:

    python -I eu.py <участок.osm> [...]

  `<имя>.dlt.tif` — Dominant Leaf Type: 0 — не лес, 1 — лиственное, 2 — хвойное
                    (порода крон в trees.py, как NFI в Англии);
  `<имя>.waw.tif` — Water & Wetness: 0 — сухо, 1 — вода постоянно, 2 — вода
                    временами, 3 — мокро постоянно, 4 — мокро временами
                    (болото в паспорте снимка, photo.py).

Рамка — в градусах, растр — в долготе и широте, пиксель ~2 м (ближайший сосед:
значения — классы). 255 — вне покрытия. Источник — Европейское агентство по
окружающей среде (EEA), image.discomap.eea.europa.eu; данные Copernicus Land
Monitoring Service, свободные с указанием источника.
"""
import math
import os
import sys
import urllib.parse
import urllib.request

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import satlib  # noqa: E402

BASE = "https://image.discomap.eea.europa.eu/arcgis/rest/services/GioLandPublic/{}/ImageServer/exportImage"
LAYERS = {"dlt": "HRL_DominantLeafType2018", "waw": "HRL_WaterWetness_2018"}
STEP_M = 2.0


def path(osm, layer):
    return os.path.splitext(osm)[0] + f".{layer}.tif"


def fetch(osm, layer):
    minlat, minlon, maxlat, maxlon = satlib.bounds(osm)
    w_m = (maxlon - minlon) * 111320 * math.cos(math.radians((minlat + maxlat) / 2))
    h_m = (maxlat - minlat) * 110574
    size = (max(4, round(w_m / STEP_M)), max(4, round(h_m / STEP_M)))
    q = {"bbox": f"{minlon},{minlat},{maxlon},{maxlat}", "bboxSR": "4326", "imageSR": "4326", "size": f"{size[0]},{size[1]}",
         "format": "tiff", "pixelType": "U8", "compression": "None", "interpolation": "RSP_NearestNeighbor", "f": "image"}
    req = urllib.request.Request(BASE.format(LAYERS[layer]) + "?" + urllib.parse.urlencode(q), headers={"User-Agent": "kartograf-osm-import-test/0.1"})
    data = urllib.request.urlopen(req, timeout=60).read()
    open(path(osm, layer), "wb").write(data)
    return data


def load(osm, layer, w=None, h=None):
    """Слой как массив классов, растянутый до (w, h) ближайшим соседом; нет файла — None."""
    p = path(osm, layer)
    if not os.path.exists(p):
        return None
    im = Image.open(p)
    try:
        im.load()
    except OSError:
        # Плитки TIFF пустые (смещение 0): вся рамка вне данных — ледник в Альпах.
        im = Image.new("L", im.size, 255)
    if w and h:
        im = im.resize((w, h), Image.NEAREST)
    return np.asarray(im).astype(np.uint8)


if __name__ == "__main__":
    for osm in sys.argv[1:]:
        for layer in LAYERS:
            data = fetch(osm, layer)
            a = load(osm, layer)
            vals, counts = np.unique(a, return_counts=True)
            print(os.path.basename(path(osm, layer)), len(data), "байт", a.shape, dict(zip(vals.tolist(), (counts / a.size).round(2).tolist())))
