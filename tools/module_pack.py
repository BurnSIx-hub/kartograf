"""Модуль Foundry для выпуска.

    python tools/module_pack.py stamp <module.json>   вписать версию из Cargo.toml
    python tools/module_pack.py zip <папка>           собрать wasm и положить в папку
                                                      module.zip и module.json

Версия модуля = версия программы: модуль выходит в том же выпуске
kartograf-releases, что и kartograf.exe. Foundry сверяет установленную версию с
`manifest` (module.json последнего выпуска) и берёт архив по `download`.
wasm-bindgen той же версии, что в Cargo.toml, ищется в WASM_BINDGEN или в PATH.
"""
import json, os, re, subprocess, sys, zipfile

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


def build():
    subprocess.run(['cargo', 'build', '--release', '--target', 'wasm32-unknown-unknown'], cwd=ROOT, check=True)
    wbg = os.environ.get('WASM_BINDGEN', 'wasm-bindgen')
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
    cmd, arg = sys.argv[1], sys.argv[2]
    if cmd == 'stamp':
        text = stamped(arg, version())
        open(arg, 'w', encoding='utf-8', newline='\n').write(text)
    elif cmd == 'zip':
        if '--no-build' not in sys.argv:
            build()
        print(pack(arg))
    else:
        sys.exit(__doc__)
