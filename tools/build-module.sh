#!/usr/bin/env bash
# Сборка Картографа модулем Foundry: wasm → foundry/pkg. Первый аргумент — куда ещё
# скопировать модуль (папка modules песочницы или Foundry).
# Нужен wasm-bindgen той же версии, что в Cargo.toml (=0.2.129): WASM_BINDGEN=путь/к/wasm-bindgen.exe
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-unknown-unknown
"${WASM_BINDGEN:-wasm-bindgen}" --target web --no-typescript --out-dir foundry/pkg target/wasm32-unknown-unknown/release/kartograf.wasm
if [ -n "${1:-}" ]; then
  rm -rf "$1/burn-kartograf"
  cp -r foundry "$1/burn-kartograf"
  cp LICENSE "$1/burn-kartograf/"
  # Версия из Cargo.toml: Foundry сверяет её с выпуском и предлагает обновиться.
  python tools/module_pack.py stamp "$1/burn-kartograf/module.json"
fi
ls -la foundry/pkg
