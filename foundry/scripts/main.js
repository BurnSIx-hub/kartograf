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

// Пока открыт Картограф, сцена Foundry под ним не видна, но рисуется дальше —
// шейдеры, ветер, частицы делят с редактором процессор и видеокарту. Не
// останавливаем её, а реже обновляем: игра, звук и связь идут как шли.
const SCENE_FPS = 8;
let sceneFps = null;

function dimScene() {
  const ticker = canvas?.app?.ticker;
  if (!ticker || sceneFps !== null) return;
  sceneFps = ticker.maxFPS;
  ticker.maxFPS = SCENE_FPS;
}

function wakeScene() {
  const ticker = canvas?.app?.ticker;
  if (ticker && sceneFps !== null) ticker.maxFPS = sceneFps;
  sceneFps = null;
}

export async function open() {
  if (!game.user.isGM) return;
  if (!root) build();
  root.style.display = "flex";
  dimScene();
  restore().catch((err) => console.warn(`${MODULE_ID} | паки не восстановились`, err));
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
  wakeScene();
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
  await chests(doc).catch((err) => console.error(`${MODULE_ID} | сундуки`, err));
  return doc;
}

// Сундук с сокровищем приходит тайлом с пометкой `chest` (картинки открытого и
// пустого). С Item Piles он становится контейнером, как в Map Forge: щелчок
// открывает, внутрь Мастер кладёт добычу. Без Item Piles остаётся картинкой.
// Сундук, уже стоящий на том же месте (сцену обновили), не трогается — с добычей.
const CHEST_SFX_OPEN = `modules/${MODULE_ID}/sounds/chest-open.mp3`;
const CHEST_SFX_CLOSE = `modules/${MODULE_ID}/sounds/chest-close.mp3`;

async function chests(scene) {
  const marked = scene.tiles.filter((t) => t.flags?.[MODULE_ID]?.chest);
  if (!marked.length) return;
  const api = game.modules.get("item-piles")?.active ? game.itempiles?.API : null;
  if (!api?.createItemPile) {
    ui.notifications.info("Картограф: сундуки откроются, если включить модуль Item Piles");
    return;
  }
  const grid = scene.grid.size;
  const done = [];
  for (const t of marked) {
    const { open, empty } = t.flags[MODULE_ID].chest;
    const x = Math.round(t.x - t.width / 2), y = Math.round(t.y - t.height / 2);
    const there = scene.tokens.some((k) => k.flags?.[MODULE_ID]?.chest && Math.abs(k.x - x) < 2 && Math.abs(k.y - y) < 2);
    if (!there) {
      await api.createItemPile({
        sceneId: scene.id,
        position: { x, y },
        createActor: true,
        actorOverrides: { name: "Сундук" },
        tokenOverrides: {
          name: "Сундук",
          displayName: CONST.TOKEN_DISPLAY_MODES.OWNER,
          width: t.width / grid, height: t.height / grid,
          rotation: t.rotation ?? 0,
          texture: { src: t.texture.src },
          flags: { [MODULE_ID]: { chest: true } }
        },
        itemPileFlags: {
          type: "container", closed: true,
          closedImage: t.texture.src, openedImage: open, emptyImage: empty, lockedImage: t.texture.src,
          openSound: CHEST_SFX_OPEN, closeSound: CHEST_SFX_CLOSE
        }
      });
    }
    done.push(t.id);
  }
  await scene.deleteEmbeddedDocuments("Tile", done);
}

// Паутина — тайл с пометкой `web`: едва заметно колышется, у каждой своя фаза.
const webs = new Map();
let webTicking = false;

function rebuildWebs() {
  webs.clear();
  for (const tile of canvas.tiles?.placeables ?? []) {
    if (!tile.document.flags?.[MODULE_ID]?.web) continue;
    const phase = [...tile.id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 1000, 7) / 1000 * Math.PI * 2;
    webs.set(tile.id, { tile, phase });
  }
  if (webs.size && !webTicking) {
    canvas.app.ticker.add(breathe);
    webTicking = true;
  }
}

function breathe() {
  if (!webs.size || !canvas.ready) {
    canvas.app?.ticker.remove(breathe);
    webTicking = false;
    return;
  }
  const t = performance.now() / 1000;
  for (const { tile, phase } of webs.values()) {
    const mesh = tile.mesh;
    if (!mesh || mesh.destroyed) continue;
    mesh.skew.set(Math.sin(t * 0.8 + phase) * 0.018, Math.sin(t * 0.55 + phase * 1.7) * 0.014);
  }
}

// Файлы с диска Мастера (паки Dungeondraft) — через окно браузера. Rust читает их
// кусками и синхронно (plat::web::RangeFile): кусок файла отдаётся blob-ссылкой
// синхронному запросу. Иначе пришлось бы держать в памяти пак целиком, а FA — 4–5 ГБ.
const files = new Map();
const picked = [];

// Выбранные паки помнятся между запусками: окно `showOpenFilePicker` отдаёт ручку
// файла, она хранится в IndexedDB, а при следующем открытии браузер лишь спрашивает
// разрешение на чтение. Где этого окна нет — обычное, и паки выбирают заново.
function db() {
  return new Promise((ok, no) => {
    const r = indexedDB.open(MODULE_ID, 1);
    r.onupgradeneeded = () => r.result.createObjectStore("packs");
    r.onsuccess = () => ok(r.result);
    r.onerror = () => no(r.error);
  });
}

async function keep(name, handle) {
  const d = await db();
  await new Promise((ok, no) => {
    const t = d.transaction("packs", "readwrite");
    t.objectStore("packs").put(handle, name);
    t.oncomplete = ok;
    t.onerror = () => no(t.error);
  });
}

async function kept() {
  const d = await db();
  return new Promise((ok, no) => {
    const out = [];
    const c = d.transaction("packs").objectStore("packs").openCursor();
    c.onsuccess = () => {
      const cur = c.result;
      if (!cur) return ok(out);
      out.push([cur.key, cur.value]);
      cur.continue();
    };
    c.onerror = () => no(c.error);
  });
}

// Паки, выбранные раньше: зовётся из нажатия кнопки, чтобы браузер мог спросить разрешение.
async function restore() {
  if (!window.showOpenFilePicker) return;
  for (const [name, h] of await kept()) {
    if (files.has(name)) continue;
    try {
      let p = await h.queryPermission({ mode: "read" });
      if (p === "prompt") p = await h.requestPermission({ mode: "read" });
      if (p !== "granted") continue;
      files.set(name, await h.getFile());
      picked.push(name);
    } catch (err) {
      console.warn(`${MODULE_ID} | пак ${name} не открылся`, err);
    }
  }
}

async function pickHandles(accept, multiple) {
  const types = [{ description: "Пак Dungeondraft", accept: { "application/octet-stream": accept.split(",") } }];
  try {
    return await window.showOpenFilePicker({ multiple, types });
  } catch (err) {
    // Длинное расширение браузер может не принять в фильтр — тогда без фильтра.
    if (err.name === "TypeError") return window.showOpenFilePicker({ multiple });
    throw err;
  }
}

function pick(accept, multiple) {
  if (window.showOpenFilePicker) {
    pickHandles(accept, multiple)
      .then(async (handles) => {
        for (const h of handles) {
          const f = await h.getFile();
          files.set(f.name, f);
          picked.push(f.name);
          keep(f.name, h).catch((err) => console.warn(`${MODULE_ID} | пак не запомнился`, err));
        }
      })
      .catch((err) => { if (err.name !== "AbortError") console.error(`${MODULE_ID} | выбор пака`, err); });
    return;
  }
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

// Картинки из паков раскрывает браузер: `createImageBitmap` работает вне основного
// потока и быстрее декодера в wasm. Миниатюры библиотеки — вперёд крупных картинок
// карты, иначе на большой карте библиотека ждёт сотни раскрытий. Готовое забирает
// кадр редактора (`kartografTakeDecoded`).
const decodeQueue = { thumbs: [], full: [] };
const decoded = [];
let decoding = 0;
const DECODE_AT_ONCE = 6;

let blurs = null;
/** Умеет ли холст браузера размывать (`ctx.filter`). */
function canBlur() {
  if (blurs === null) {
    try {
      const g = new OffscreenCanvas(1, 1).getContext("2d");
      g.filter = "blur(2px)";
      blurs = g.filter === "blur(2px)";
    } catch {
      blurs = false;
    }
  }
  return blurs;
}

function pumpDecode() {
  while (decoding < DECODE_AT_ONCE) {
    const job = decodeQueue.thumbs.shift() ?? decodeQueue.full.shift();
    if (!job) return;
    decoding++;
    decodeOne(job).finally(() => { decoding--; pumpDecode(); });
  }
}

// Тень (`blur` > 0) — как `shadow_image` в Rust: силуэт картинки не крупнее `side`,
// чёрный, размытый, с полями 3r+1. Три прохода ящика радиуса r — это почти Гаусс
// с σ = √(r(r+1)). Вместо размера картинки отдаём [поле, ширина силуэта].
async function decodeOne({ key, thumb, name, offset, size, side, blur, ppcBase }) {
  try {
    const f = files.get(name);
    if (!f) throw new Error(`${name}: файл не выбран`);
    const bmp = await createImageBitmap(f.slice(offset, offset + size), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
    const ow = bmp.width, oh = bmp.height;
    const k = Math.min(1, side / Math.max(ow, oh, 1));
    const w = Math.max(1, Math.round(ow * k)), h = Math.max(1, Math.round(oh * k));
    if (blur > 0) {
      const ppc = w / Math.max(ow / ppcBase, 1e-3);
      const r = Math.min(64, Math.max(1, Math.round((blur * ppc) / 2)));
      const pad = 3 * r + 1;
      const pw = w + 2 * pad, ph = h + 2 * pad;
      const canvas = new OffscreenCanvas(pw, ph);
      const g = canvas.getContext("2d", { willReadFrequently: true });
      g.filter = `brightness(0) blur(${Math.sqrt(r * (r + 1)).toFixed(2)}px)`;
      g.drawImage(bmp, pad, pad, w, h);
      bmp.close();
      decoded.push({ key, thumb, w: pw, h: ph, ow: pad, oh: w, data: g.getImageData(0, 0, pw, ph).data });
      return;
    }
    const canvas = new OffscreenCanvas(w, h);
    const g = canvas.getContext("2d", { willReadFrequently: true });
    g.imageSmoothingQuality = "high";
    g.drawImage(bmp, 0, 0, w, h);
    bmp.close();
    decoded.push({ key, thumb, w, h, ow, oh, data: g.getImageData(0, 0, w, h).data });
  } catch (err) {
    console.warn(`${MODULE_ID} | картинка не раскрылась: ${key}`, err);
    decoded.push({ key, thumb, data: null });
  }
}

Hooks.once("init", () => {
  window.kartografDecode = (key, thumb, name, offset, size, side, blur = 0, ppcBase = 256) => {
    if (blur > 0 && !canBlur()) return false;
    (thumb ? decodeQueue.thumbs : decodeQueue.full).push({ key, thumb, name, offset, size, side, blur, ppcBase });
    pumpDecode();
    return true;
  };
  window.kartografTakeDecoded = () => decoded.splice(0);
  window.kartografDecoding = () => decoding + decodeQueue.thumbs.length + decodeQueue.full.length;
  window.kartografPick = pick;
  window.kartografTakePicked = () => picked.splice(0);
  window.kartografFileSize = (name) => files.get(name)?.size;
  window.kartografReadSync = readSync;
  game.settings.register(MODULE_ID, "lastScene", { scope: "world", config: false, type: String, default: "" });
  window.kartografLastScene = () => lastScene()?.name ?? null;
  window.kartografToScene = (files, update) => {
    toScene(files, update).catch((err) => { console.error(`${MODULE_ID} | на сцену`, err); ui.notifications.error("Картограф: сцена не создалась, подробности в консоли"); });
  };
  game.modules.get(MODULE_ID).api = { open, close, chests, rebuildWebs };
});

Hooks.on("canvasReady", rebuildWebs);
for (const h of ["createTile", "updateTile", "deleteTile"]) Hooks.on(h, () => setTimeout(rebuildWebs, 0));

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
