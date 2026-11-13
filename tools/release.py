"""Выложить выпуск: python tools/release.py "что нового".

Версия берётся из Cargo.toml — поднять её до запуска. Собирает release и кладёт
в публичный репозиторий выпусков kartograf.exe и latest.json (обновление
программы), а рядом module.zip и module.json (обновление модуля Foundry, его
собирает tools/module_pack.py; нужен WASM_BINDGEN или wasm-bindgen в PATH).
"""
import hashlib, json, os, re, subprocess, sys, tempfile

REPO = 'BurnSIx-hub/kartograf-releases'
notes = sys.argv[1] if len(sys.argv) > 1 else ''
version = re.search(r'^version = "(.+)"', open('Cargo.toml', encoding='utf-8').read(), re.M)[1]
subprocess.run(['cargo', 'build', '--release'], check=True)
exe = 'target/release/kartograf.exe'
data = open(exe, 'rb').read()
tag = f'v{version}'
latest = {
    'version': version, 'notes': notes,
    'url': f'https://github.com/{REPO}/releases/download/{tag}/kartograf.exe',
    'sha256': hashlib.sha256(data).hexdigest(), 'size': len(data),
}
path = os.path.join(tempfile.mkdtemp(), 'latest.json')
json.dump(latest, open(path, 'w', encoding='utf-8'), ensure_ascii=False, indent=1)
here = os.path.dirname(os.path.abspath(__file__))
out = os.path.dirname(path)
subprocess.run([sys.executable, os.path.join(here, 'module_pack.py'), 'zip', out], check=True)
assets = [exe, path, os.path.join(out, 'module.zip'), os.path.join(out, 'module.json')]
subprocess.run(['gh', 'release', 'create', tag, *assets, '--repo', REPO, '--title', version, '--notes', notes or version], check=True)
