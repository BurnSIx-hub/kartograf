# Картограф

Редактор боевых карт для настольных ролевых игр (D&D, Foundry VTT): пол, стены,
двери, предметы, крыши, рельеф, свет. Пишется на Rust (egui); тот же код
собирается модулем Foundry (WASM).

Что умеет:

- **Рисовать руками**: пол кистью, комнаты со стенами по краю, двери и окна,
  предметы, дороги и реки, надписи, крыши, рельеф ярусами, этажи.
- **Строить сам**: подземелье, пещера, лес, городской квартал, городская стена,
  дом, таверна, покои ярла, церковь с погостом, колизей, киберпанк.
- **Строить по плану**: JSON-план комнат и связей (см. [plans/README.md](plans/README.md)) —
  его может написать человек или нейросеть.
- **Местность по координатам**: дома с крышами, дороги, вода и деревья по
  OpenStreetMap и спутниковому снимку; место выбирается рамкой на карте.
- **Выгрузка**: картинка, сцена Foundry VTT со стенами, дверями и светом, файл
  Universal VTT (`.dd2vtt`).
- **Ассеты**: встроенные, паки Dungeondraft (Forgotten Adventures и другие), своя папка картинок.

## Скачать

Готовая программа для Windows:
https://github.com/BurnSIx-hub/kartograf-releases/releases/latest/download/kartograf.exe

Подписи у программы нет: SmartScreen может предупредить — «Подробнее» →
«Выполнить в любом случае». Новые версии программа находит и ставит сама.

## ИИ правит карту (MCP)

`kartograf.exe --mcp` — сервер [MCP](https://modelcontextprotocol.io): ИИ-клиент
(Claude Desktop, Claude Code, Cursor и другие) получает инструменты Картографа и
правит карту **в открытом окне редактора** — каждая правка видна сразу и
отменяется Ctrl+Z. Окна нет — сервер его откроет сам.

Подключить в Claude Desktop (`%APPDATA%\Claude\claude_desktop_config.json`) или
Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "kartograf": {
      "command": "C:\\путь\\к\\kartograf.exe",
      "args": ["--mcp"]
    }
  }
}
```

В Claude Code:

```bash
claude mcp add kartograf -- "C:\путь\к\kartograf.exe" --mcp
```

Инструменты:

| Инструмент | Что делает |
|---|---|
| `map_info`, `describe` | размер, материалы; что на карте — предметы, стены, двери, надписи, дороги с номерами |
| `find_assets` | поиск картинок в библиотеке по словам (`barrel`, `stone floor`) — ключи для остального |
| `add_object`, `edit_object`, `remove` | поставить, сдвинуть, повернуть, заменить, убрать |
| `paint`, `erase` | пол прямоугольником или овалом; `target: "room"` — комната, стены по краю встают сами |
| `add_wall`, `add_door`, `add_label` | стена по точкам, дверь или окно в ближайшую стену, надпись |
| `undo` | отменить |
| `view` | картинка всей карты или области — ИИ видит, что вышло |
| `generate`, `build_plan`, `terrain_from_place` | построить: генератором, по плану, по координатам |
| `new_map`, `open_map`, `save_map`, `export_foundry` | файлы и выгрузка |

Координаты — в клетках (клетка — 5 футов), x вправо, y вниз, (0, 0) — левый
верхний угол. Окно и сервер говорят через `control.json` в папке данных
(`%APPDATA%\Kartograf`): порт на 127.0.0.1 и пароль, без пароля команды не
принимаются.

## Собрать из исходников

Нужен Rust с MSVC (rustup + Visual Studio Build Tools с C++).

```bash
cargo build --release
```

Программа — `target/release/kartograf.exe`. Проверки: `cargo test --release`;
самопроверка окна — `kartograf.exe --selftest <папка>`.

Модуль Foundry: `bash tools/build-module.sh` (нужен `wasm-bindgen` 0.2.129),
результат — в `foundry/`.

## Устройство

| Файл | Что |
|---|---|
| `src/app.rs` | окно редактора: меню, инструменты, библиотека, холст |
| `src/map.rs` | карта: материалы, маски пола, стены, двери, предметы, крыши |
| `src/procgen.rs`, `src/plan.rs`, `src/generate.rs` | план карты и строитель |
| `src/dungeon.rs`, `src/quarter.rs`, `src/walls.rs` | генераторы |
| `src/osm.rs`, `src/place.rs`, `src/mappick.rs` | местность из OpenStreetMap и снимка, выбор места |
| `src/roofs.rs`, `src/relief.rs` | крыши и рельеф |
| `src/export.rs` | выгрузка в Foundry и Universal VTT |
| `src/mcp.rs`, `src/control.rs`, `src/app_tools.rs` | сервер MCP и команды окну |
| `src/library.rs`, `src/pck.rs` | библиотека ассетов, паки Dungeondraft |
| `foundry/` | обёртка модуля Foundry |
