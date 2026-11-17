"""Модуль Foundry для выпуска.

    python tools/module_pack.py stamp <module.json>   вписать версию из Cargo.toml
    python tools/module_pack.py zip <папка>           собрать wasm и положить в папку
                                                      module.zip и module.json
    python tools/module_pack.py check                 есть ли wasm-bindgen нужной версии

Версия модуля = версия программы: модуль выходит в том же выпуске
kartograf-releases, что и kartograf.exe. Foundry сверяет установленную версию с
`manifest` (module.json последнего выпуска) и берёт архив по `download`.
wasm-bindgen той же версии, что в Cargo.toml, ищется в WASM_BINDGEN или в PATH
(обычное место — ~/.cargo/bin, туда его кладёт `cargo install wasm-bindgen-cli`).
"""
import json, os, re, shutil, subprocess, sys, zipfile

REPO = 'BurnSIx-hub/kartograf-releases'
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FOUNDRY = os.path.join(ROOT, 'foundry')


def version():
    toml = open(os.path.join(ROOT, 'Cargo.toml'), encoding='utf-8').read()
    return re.search(r'^version = "(.+)"', toml, re.M)[1]


def stamped(source, v):
    m = json.load(open(source, encoding='utf-8'))
    m['version'] = v
    m['manifest'] = f'https://github.com/{REPO}/releases/latest/download/module.json'
    m['download'] = f'https://github.com/{REPO}/releases/download/v{v}/module.zip'
    return json.dumps(m, ensure_ascii=False, indent=2) + '\n'


def wasm_bindgen():
    """Путь к wasm-bindgen той же версии, что крейт в Cargo.toml: с другой версией модуль
    соберётся, но в Foundry не запустится. Нет или не тот — выход с тем, как поставить."""
    toml = open(os.path.join(ROOT, 'Cargo.toml'), encoding='utf-8').read()
    need = re.search(r'^wasm-bindgen = "=?([0-9.]+)"', toml, re.M)[1]
    how = f'поставить: cargo install wasm-bindgen-cli --version {need} --locked'
    wbg = shutil.which(os.environ.get('WASM_BINDGEN') or 'wasm-bindgen')
    if not wbg:
        sys.exit(f'нет wasm-bindgen {need}: ни в WASM_BINDGEN, ни в PATH; {how}')
    have = subprocess.run([wbg, '--version'], capture_output=True, text=True).stdout.split()[-1:]
    if have != [need]:
        sys.exit(f'{wbg}: версия {have[0] if have else "?"}, а крейту в Cargo.toml нужна {need}; {how}')
    return wbg


def build():
    wbg = wasm_bindgen()
    subprocess.run(['cargo', 'build', '--release', '--target', 'wasm32-unknown-unknown'], cwd=ROOT, check=True)
    wasm = os.path.join(ROOT, 'target', 'wasm32-unknown-unknown', 'release', 'kartograf.wasm')
    subprocess.run([wbg, '--target', 'web', '--no-typescript', '--out-dir', os.path.join(FOUNDRY, 'pkg'), wasm], check=True)


def pack(out):
    v = version()
    os.makedirs(out, exist_ok=True)
    manifest = stamped(os.path.join(FOUNDRY, 'module.json'), v)
    for need in ('pkg/kartograf.js', 'pkg/kartograf_bg.wasm', 'scripts/main.js'):
        if not os.path.isfile(os.path.join(FOUNDRY, need)):
            sys.exit(f'нет foundry/{need}')
    path = os.path.join(out, 'module.zip')
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        z.writestr('module.json', manifest)
        z.write(os.path.join(ROOT, 'LICENSE'), 'LICENSE')
        for dirpath, _, files in os.walk(FOUNDRY):
            for f in sorted(files):
                full = os.path.join(dirpath, f)
                rel = os.path.relpath(full, FOUNDRY).replace(os.sep, '/')
                if rel != 'module.json':
                    z.write(full, rel)
    open(os.path.join(out, 'module.json'), 'w', encoding='utf-8', newline='\n').write(manifest)
    return path


if __name__ == '__main__':
    cmd, arg = sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else ''
    if cmd == 'check':
        print(wasm_bindgen())
    elif cmd == 'stamp':
        text = stamped(arg, version())
        open(arg, 'w', encoding='utf-8', newline='\n').write(text)
    elif cmd == 'zip':
        if '--no-build' not in sys.argv:
            build()
        print(pack(arg))
    else:
        sys.exit(__doc__)
