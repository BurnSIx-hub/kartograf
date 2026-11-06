// 🔥 Картограф в Foundry. Сам редактор — тот же Rust, что у программы, собранный
// в WebAssembly (pkg/). Здесь только окно: холст поверх Foundry, кнопка в каталоге сцен.
// Игрокам ничего не грузится: wasm подтягивается при первом открытии у Мастера.

const MODULE_ID = "burn-kartograf";
let root = null;
let started = null;

function build() {
  root = document.createElement("div");
  root.id = "kartograf-root";
  root.style.cssText = "position:fixed;inset:0;z-index:2147483000;background:#1b1b1b;display:flex;flex-direction:column";
  root.innerHTML = `
    <div style="display:flex;align-items:center;gap:8px;padding:4px 8px;background:#111;color:#ddd;font:13px sans-serif">
      <span>🔥 Картограф</span><span id="kartograf-status" style="opacity:.6"></span>
      <button type="button" id="kartograf-close" style="margin-left:auto;width:auto;line-height:20px;padding:0 10px">Закрыть</button>
    </div>
    <canvas id="kartograf-canvas" tabindex="0" style="flex:1;width:100%;height:100%;outline:none"></canvas>`;
  document.body.append(root);
  root.querySelector("#kartograf-close").addEventListener("click", close);
  // Клавиши редактора не должны долетать до Foundry (пробел, Delete, стрелки…).
  for (const type of ["keydown", "keyup", "keypress"]) {
    root.addEventListener(type, (e) => e.stopPropagation());
  }
}

export async function open() {
  if (!game.user.isGM) return;
  if (!root) build();
  root.style.display = "flex";
  if (!started) {
    const status = root.querySelector("#kartograf-status");
    status.textContent = "загрузка…";
    const t = performance.now();
    started = import(`../pkg/kartograf.js`)
      .then((m) => m.default({ module_or_path: `modules/${MODULE_ID}/pkg/kartograf_bg.wasm` }))
      .then(() => { status.textContent = `загружен за ${((performance.now() - t) / 1000).toFixed(1)} с`; })
      .catch((err) => { status.textContent = "не загрузился, подробности в консоли"; console.error(`${MODULE_ID} |`, err); started = null; });
  }
  root.querySelector("#kartograf-canvas").focus();
  return started;
}

export function close() {
  if (root) root.style.display = "none";
}

// Выгрузка из Картографа: картинки — в данные Foundry, `.json` — новой сценой
// или поверх последней. Зовёт её Rust (plat::to_scene) с массивом { path, bytes }.
// Что поставили на сцену в Foundry сами, при обновлении остаётся.
const KEEP = ["tokens", "notes", "sounds", "drawings", "templates"];

function lastScene() {
  return game.scenes.get(game.settings.get(MODULE_ID, "lastScene"));
}

async function toScene(files, update) {
  const FP = foundry.applications.apps.FilePicker.implementation;
  const dirs = new Set();
  for (const f of files) {
    const parts = f.path.split("/").slice(0, -1);
    for (let i = 1; i <= parts.length; i++) dirs.add(parts.slice(0, i).join("/"));
  }
  for (const d of [...dirs].sort((a, b) => a.length - b.length)) {
    try { await FP.createDirectory("data", d); } catch (err) { /* уже есть */ }
  }
  let scene = null;
  for (const f of files) {
    const name = f.path.split("/").pop();
    const dir = f.path.split("/").slice(0, -1).join("/");
    if (name.endsWith(".json")) { scene = JSON.parse(new TextDecoder().decode(f.bytes)); continue; }
    const type = name.endsWith(".webp") ? "image/webp" : name.endsWith(".jpg") ? "image/jpeg" : "application/octet-stream";
    await FP.upload("data", dir, new File([f.bytes], name, { type }), {}, { notify: false });
  }
  if (!scene) return;
  delete scene._id;
  const old = update ? lastScene() : null;
  let doc;
  if (old) {
    // Как «Import Data» у сцены: всё заменяется данными из файла, кроме сохранённого.
    for (const k of KEEP) scene[k] = old.toObject()[k] ?? [];
    doc = await old.importFromJSON(JSON.stringify(scene));
    ui.notifications.info(`Картограф: сцена «${doc.name}» обновлена`);
  } else {
    doc = await Scene.implementation.create(scene);
    ui.notifications.info(`Картограф: сцена «${doc.name}» создана`);
  }
  await game.settings.set(MODULE_ID, "lastScene", doc.id);
  return doc;
}

// Файлы с диска Мастера (паки Dungeondraft) — через окно браузера. Rust читает их
// кусками и синхронно (plat::web::RangeFile): кусок файла отдаётся blob-ссылкой
// синхронному запросу. Иначе пришлось бы держать в памяти пак целиком, а FA — 4–5 ГБ.
const files = new Map();
const picked = [];

function pick(accept, multiple) {
  const input = document.createElement("input");
  input.type = "file";
  input.accept = accept;
  input.multiple = multiple;
  input.addEventListener("change", () => {
    for (const f of input.files) {
      files.set(f.name, f);
      picked.push(f.name);
    }
  });
  input.click();
}

function readSync(name, offset, size) {
  const f = files.get(name);
  if (!f) throw new Error(`${name}: файл не выбран`);
  const url = URL.createObjectURL(f.slice(offset, offset + size));
  try {
    const x = new XMLHttpRequest();
    x.open("GET", url, false);
    x.overrideMimeType("text/plain; charset=x-user-defined");
    x.send();
    const s = x.responseText;
    const out = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
    return out;
  } finally {
    URL.revokeObjectURL(url);
  }
}

Hooks.once("init", () => {
  window.kartografPick = pick;
  window.kartografTakePicked = () => picked.splice(0);
  window.kartografFileSize = (name) => files.get(name)?.size;
  window.kartografReadSync = readSync;
  game.settings.register(MODULE_ID, "lastScene", { scope: "world", config: false, type: String, default: "" });
  window.kartografLastScene = () => lastScene()?.name ?? null;
  window.kartografToScene = (files, update) => {
    toScene(files, update).catch((err) => { console.error(`${MODULE_ID} | на сцену`, err); ui.notifications.error("Картограф: сцена не создалась, подробности в консоли"); });
  };
  game.modules.get(MODULE_ID).api = { open, close };
});

Hooks.on("renderSceneDirectory", (app, html) => {
  if (!game.user.isGM) return;
  const el = html instanceof HTMLElement ? html : html[0];
  if (el.querySelector(".kartograf-open")) return;
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "kartograf-open";
  btn.innerHTML = `<i class="fa-solid fa-map"></i> Картограф`;
  btn.addEventListener("click", () => open());
  const header = el.querySelector(".directory-header .header-actions") ?? el.querySelector(".directory-header") ?? el;
  header.append(btn);
});
