"""Сколько каких тегов в участке: что вообще есть на местности.

    python -I tags.py <участок.osm> [...]
"""
import sys, collections, xml.etree.ElementTree as ET
KEYS = ("natural", "landuse", "leisure", "amenity", "highway", "surface", "barrier", "waterway", "water", "historic", "building", "wetland", "leaf_type", "tracktype", "man_made", "ruins")
for f in sys.argv[1:]:
    root = ET.parse(f).getroot()
    c = collections.Counter()
    for el in root:
        if el.tag not in ("node", "way", "relation"):
            continue
        tags = {t.get("k"): t.get("v") for t in el.findall("tag")}
        for k in KEYS:
            if k in tags:
                extra = ""
                if k == "highway":
                    extra = "/" + tags.get("surface", "-")
                c[f"{el.tag[0]} {k}={tags[k]}{extra}"] += 1
    print("==", f)
    for k, v in sorted(c.items()):
        print(f"  {v:4} {k}")
