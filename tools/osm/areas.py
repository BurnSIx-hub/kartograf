"""Контуры природы в участке: замкнут ли, сколько точек, где лежит (доли рамки 0..1), теги;
мультиполигоны — сколько их линий есть в выгрузке.

    python -I areas.py <участок.osm>
"""
import sys, math, xml.etree.ElementTree as ET
f = sys.argv[1]
root = ET.parse(f).getroot()
b = root.find("bounds").attrib
minlat, minlon, maxlat, maxlon = (float(b[k]) for k in ("minlat", "minlon", "maxlat", "maxlon"))
nodes = {n.get("id"): (float(n.get("lat")), float(n.get("lon"))) for n in root.findall("node")}
ways = {}
KEYS = ("natural", "landuse", "leisure", "man_made", "place", "waterway", "surface", "rock", "geological", "leaf_type", "name", "tidal", "barrier", "highway", "building")
def frac(lat, lon):
    return ((lon - minlon) / (maxlon - minlon), (maxlat - lat) / (maxlat - minlat))
for w in root.findall("way"):
    tags = {t.get("k"): t.get("v") for t in w.findall("tag")}
    nds = [nd.get("ref") for nd in w.findall("nd")]
    ways[w.get("id")] = (nds, tags)
    if any(k in tags for k in ("natural", "landuse", "leisure", "man_made", "place")):
        pts = [frac(*nodes[i]) for i in nds if i in nodes]
        xs, ys = [p[0] for p in pts], [p[1] for p in pts]
        closed = nds[0] == nds[-1]
        t = {k: tags[k] for k in KEYS if k in tags}
        print(f"way {w.get('id')} {'closed' if closed else 'open  '} n={len(nds):4} x {min(xs):.2f}..{max(xs):.2f} y {min(ys):.2f}..{max(ys):.2f} {t}")
for r in root.findall("relation"):
    tags = {t.get("k"): t.get("v") for t in r.findall("tag")}
    if any(k in tags for k in ("natural", "landuse", "leisure", "place")):
        mem = [(m.get("type"), m.get("ref"), m.get("role")) for m in r.findall("member")]
        have = sum(1 for m in mem if m[0] == "way" and m[1] in ways)
        print(f"rel {r.get('id')} members {len(mem)} (ways here {have}) {{{', '.join(f'{k}={tags[k]}' for k in KEYS if k in tags)}}} type={tags.get('type')}")
