"""Связь со скриптами кругов: MCP Картографа (`kartograf --mcp`) правит открытое окно.

KEXE — путь к kartograf.exe (по умолчанию target/release в этом репозитории),
KDATA — папка данных с паками (`--data-dir`); без неё — данные по умолчанию.
call(tool, **args) → (ответ, текст); key(name) — ключ ассета (крупнейший из
одноимённых); obj / roof_obj / pipe_run — расстановка.
"""
import base64, json, os as _os, subprocess, sys, time
_root = _os.path.dirname(_os.path.dirname(_os.path.dirname(_os.path.abspath(__file__))))
EXE = _os.environ.get("KEXE") or _os.path.join(_root, "target", "release", "kartograf.exe")
_args = [EXE, "--mcp"] + (["--data-dir", _os.environ["KDATA"]] if _os.environ.get("KDATA") else [])
p = subprocess.Popen(_args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8")
_rid = 0
errors = []


def call(tool, **args):
    global _rid
    _rid += 1
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": _rid, "method": "tools/call", "params": {"name": tool, "arguments": args}}, ensure_ascii=False) + "\n")
    p.stdin.flush()
    r = json.loads(p.stdout.readline()).get("result", {})
    text = " ".join(c.get("text", "") for c in r.get("content", []) if c["type"] == "text")
    if r.get("isError"):
        errors.append(f"{tool} {json.dumps(args, ensure_ascii=False)[:120]}: {text[:200]}")
    return r, text


keys = {}


def key(name, kind="object"):
    if name in keys:
        return keys[name]
    _, t = call("find_assets", query=name, kind=kind, limit=40)
    # Одноимённые ассеты из разных паков бывают разного размера — берётся крупнейший.
    hits = [a for a in json.loads(t)["assets"] if a["name"] == name or a["key"] == name]
    if hits:
        best = max(hits, key=lambda a: (a.get("size_cells") or [0, 0])[0] * (a.get("size_cells") or [0, 0])[1])
        keys[name] = best["key"]
        return best["key"]
    errors.append(f"нет ассета {name}")
    keys[name] = None
    return None


def obj(name, x, y, rot=None, scale=1, **kw):
    """Поворот не передаётся, если его задают look_at / against_wall (иначе они не действуют)."""
    k = key(name)
    if k:
        if rot is None and not ("look_at" in kw or kw.get("against_wall")):
            rot = 0
        if rot is not None:
            kw["rotation"] = rot
        call("add_object", asset=k, x=x, y=y, scale=scale, **kw)


def roof_obj(name, x, y, rot=0, scale=1, **kw):
    obj(name, x, y, rot, scale, layer="on_roof", **kw)


def pipe_run(x0, y0, x1, y1, layer=None):
    """Труба по прямой (горизонталь или вертикаль), куски по 4 и 2 клетки."""
    extra = {"layer": layer} if layer else {}
    if y0 == y1:
        x = min(x0, x1)
        end = max(x0, x1)
        while end - x >= 4:
            obj("Pipe_Brass_Straight_A_1x4", x + 2, y0, 90, **extra); x += 4
        while end - x >= 2:
            obj("Pipe_Brass_Straight_B_1x2", x + 1, y0, 90, **extra); x += 2
    else:
        y = min(y0, y1)
        end = max(y0, y1)
        while end - y >= 4:
            obj("Pipe_Brass_Straight_A_1x4", x0, y + 2, **extra); y += 4
        while end - y >= 2:
            obj("Pipe_Brass_Straight_B_1x2", x0, y + 1, **extra); y += 2


