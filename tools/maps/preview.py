import base64, json, os, sys
sys.stdout.reconfigure(encoding="utf-8")
exec(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "mcp.py"), encoding="utf-8").read())
out, kind = sys.argv[1], sys.argv[2]
ks = [k if k.startswith("builtin:") else key(k, kind) for k in sys.argv[3:]]
r, t = call("preview_assets", assets=[k for k in ks if k], tile_px=256)
for c in r.get("content", []):
    if c["type"] == "image":
        open(out, "wb").write(base64.b64decode(c["data"]))
print(t[:300], errors)
p.stdin.close(); p.wait()
