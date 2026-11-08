"""Встроенные наборы ассетов: assets/sets/<набор>/<floors|walls|objects|roofs>/*.webp
-> src/sets.rs (таблица с include_bytes!). Запускать после пополнения набора:
    python -I tools/gen_sets.py
"""
import os
root = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..')
base = os.path.join(root, 'assets', 'sets')
rows = []
for set_name in sorted(os.listdir(base)):
    for kind in ('floors', 'walls', 'objects', 'roofs'):
        d = os.path.join(base, set_name, kind)
        if not os.path.isdir(d):
            continue
        for f in sorted(os.listdir(d)):
            if f.lower().endswith(('.webp', '.png')):
                rel = f'{set_name}/{kind}/{f}'
                rows.append(f'    ("{rel}", include_bytes!("../assets/sets/{rel}")),')
out = ['//! Встроенные наборы ассетов (киберпанк и др.). Файл собирает `tools/gen_sets.py`', '//! из `assets/sets/` — руками не править.', '', f'pub const SET_FILES: [(&str, &[u8]); {len(rows)}] = [', *rows, '];', '']
open(os.path.join(root, 'src', 'sets.rs'), 'w', encoding='utf-8', newline='\n').write('\n'.join(out))
print(len(rows), 'файлов')
