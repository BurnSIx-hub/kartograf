"""Выложить выпуск: python tools/release.py "что нового".

Версия берётся из Cargo.toml — поднять её до запуска. Собирает release и кладёт
kartograf.exe и latest.json в публичный репозиторий выпусков.
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
subprocess.run(['gh', 'release', 'create', tag, exe, path, '--repo', REPO, '--title', version, '--notes', notes or version], check=True)
