"""Леса участка из National Forest Inventory England 2022 (Forestry Commission): полигоны
с типом леса — сверка породы крон (trees_eval.py).

    python -I nfi.py <участок.osm> [...]

Пишет `nfi/<имя>.geojson` рядом с .osm. Лицензия — Open Government Licence; подпись:
© Forestry Commission copyright. Contains Ordnance Survey data © Crown copyright and
database right 2022.
"""
import os
import sys
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import satlib  # noqa: E402

URL = "https://services2.arcgis.com/mHXjwgl3OARRqqD4/arcgis/rest/services/National_Forest_Inventory_England_2022/FeatureServer/0/query"

for osm in sys.argv[1:]:
    minlat, minlon, maxlat, maxlon = satlib.bounds(osm)
    q = {"where": "1=1", "geometry": f"{minlon},{minlat},{maxlon},{maxlat}", "geometryType": "esriGeometryEnvelope", "inSR": "4326",
         "spatialRel": "esriSpatialRelIntersects", "outFields": "IFT_IOA,CATEGORY", "outSR": "4326", "f": "geojson"}
    out = os.path.join(os.path.dirname(os.path.abspath(osm)), "nfi", os.path.splitext(os.path.basename(osm))[0] + ".geojson")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    req = urllib.request.Request(URL + "?" + urllib.parse.urlencode(q), headers={"User-Agent": "kartograf-osm-import-test/0.1"})
    data = urllib.request.urlopen(req, timeout=60).read()
    open(out, "wb").write(data)
    print(os.path.basename(out), len(data), "байт")
