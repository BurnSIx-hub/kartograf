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
    // Настройки и автосейв редактор читает при запуске — сначала список данных.
    started = dataPreload()
      .catch((err) => console.warn(`${MODULE_ID} | данные редактора не подгрузились`, err))
      .then(() => import(`../pkg/kartograf.js`))
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
    const body = type === "image/webp" ? await lossy(f.bytes) : f.bytes;
    await FP.upload("data", dir, new File([body], name, { type }), {}, { notify: false });
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

// WebP из wasm — без потерь: libwebp в браузерную сборку не входит. Сжимает с
// потерями сам браузер, с тем же качеством, что программа (88): фон 4200×2800
// весил 11,7 МБ. Прозрачность кодировщик браузера держит без потерь. Не вышло
// (браузер не умеет WebP, картинка больше холста) или вышло не легче — как было.
const WEBP_QUALITY = 0.88;

async function lossy(bytes) {
  try {
    const bmp = await createImageBitmap(new Blob([bytes], { type: "image/webp" }), { premultiplyAlpha: "none", colorSpaceConversion: "none" });
    const c = new OffscreenCanvas(bmp.width, bmp.height);
    c.getContext("2d").drawImage(bmp, 0, 0);
    bmp.close();
    const blob = await c.convertToBlob({ type: "image/webp", quality: WEBP_QUALITY });
    return blob.type === "image/webp" && blob.size < bytes.length ? blob : bytes;
  } catch (err) {
    console.warn(`${MODULE_ID} | WebP не сжался, кладу без потерь`, err);
    return bytes;
  }
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

// Паутина — тайл с пометкой `web`: нить между двумя стенами угла. Как бельё
// у «Wild Canvas»: концы (по длинной стороне тайла) стоят — они прицеплены к
// стенам, середина едва колышется, у каждой своя фаза.
const WEB_FRAG = `
varying vec2 vTextureCoord;
uniform sampler2D uSampler;
uniform vec4 inputSize;
uniform vec4 outputFrame;
uniform vec4 inputClamp;
uniform vec2 uA, uB;
uniform float uAmp, uTime, uPhase;
void main() {
  vec2 pos = vTextureCoord * inputSize.xy + outputFrame.xy;
  vec2 ab = uB - uA;
  float len2 = max(dot(ab, ab), 1.0);
  float u = clamp(dot(pos - uA, ab) / len2, 0.0, 1.0);
  vec2 n = vec2(-ab.y, ab.x) / sqrt(len2);
  float bow = sin(3.14159265 * u);
  float w = 0.75 * sin(uTime * 0.9 + uPhase - u * 1.6) + 0.25 * sin(uTime * 2.1 + u * 5.0 + uPhase * 2.0);
  vec2 src = pos - n * uAmp * bow * w - normalize(ab) * uAmp * 0.3 * bow * sin(uTime * 0.7 + uPhase);
  vec2 tc = (src - outputFrame.xy) * inputSize.zw;
  float inside = step(inputClamp.x, tc.x) * step(tc.x, inputClamp.z) * step(inputClamp.y, tc.y) * step(tc.y, inputClamp.w);
  gl_FragColor = texture2D(uSampler, tc) * inside;
}`;
// Размах — доля клетки на всю длину нити: на 10 % сильнее первой версии.
const WEB_AMP = 0.022;
const webs = new Map();
let webTicking = false;

function phaseOf(id) {
  return [...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 1000, 7) / 1000 * Math.PI * 2;
}

function rebuildWebs() {
  for (const w of webs.values()) unhook(w);
  webs.clear();
  for (const tile of canvas.tiles?.placeables ?? []) {
    if (!tile.document.flags?.[MODULE_ID]?.web) continue;
    webs.set(tile.id, { tile, phase: phaseOf(tile.id), filter: null });
  }
  if (webs.size && !webTicking) {
    canvas.app.ticker.add(breathe);
    webTicking = true;
  }
}

function unhook(w) {
  const mesh = w.tile.mesh;
  if (w.filter && mesh && !mesh.destroyed) mesh.filters = (mesh.filters ?? []).filter((f) => f !== w.filter);
  w.filter = null;
}

/** Концы нити на экране: из пометки (доли холста от середины, их нашёл
 *  Картограф по рисунку), иначе — вдоль длинной стороны тайла. */
function webEnds(tile) {
  const d = tile.document;
  const s = d.shape ?? d;
  const rad = ((s.rotation ?? d.rotation) * Math.PI) / 180;
  const [c, sn] = [Math.cos(rad), Math.sin(rad)];
  const flag = d.flags[MODULE_ID].web;
  let e = Array.isArray(flag) ? flag : (d.width >= d.height ? [-0.5, 0, 0.5, 0] : [0, -0.5, 0, 0.5]);
  const at = (fx, fy) => {
    const [lx, ly] = [fx * d.width, fy * d.height];
    return { x: s.x + lx * c - ly * sn, y: s.y + lx * sn + ly * c };
  };
  const [p, q] = [at(e[0], e[1]), at(e[2], e[3])];
  const m = tile.mesh.parent.worldTransform;
  return { a: m.apply(p), b: m.apply(q), len: Math.hypot(q.x - p.x, q.y - p.y) };
}

function breathe() {
  if (!webs.size || !canvas.ready) {
    canvas.app?.ticker.remove(breathe);
    webTicking = false;
    return;
  }
  const t = performance.now() / 1000;
  const grid = canvas.dimensions.size;
  for (const w of webs.values()) {
    const mesh = w.tile.mesh;
    if (!mesh || mesh.destroyed) continue;
    if (!w.filter) {
      w.filter = new PIXI.Filter(undefined, WEB_FRAG, { uA: new Float32Array(2), uB: new Float32Array(2), uAmp: 0, uTime: 0, uPhase: w.phase });
      w.filter.resolution = canvas.app.renderer.resolution;
      mesh.filters = [...(mesh.filters ?? []), w.filter];
    }
    const { a, b, len } = webEnds(w.tile);
    const zoom = mesh.parent.worldTransform.a;
    const amp = WEB_AMP * Math.max(len, grid) * zoom;
    const u = w.filter.uniforms;
    u.uA[0] = a.x; u.uA[1] = a.y; u.uB[0] = b.x; u.uB[1] = b.y;
    u.uAmp = amp;
    u.uTime = t;
    w.filter.padding = Math.ceil(amp * 1.6) + 2;
  }
}

// Огонь — источник света с пометкой `toggle`: у него кнопка, как у двери.
// Мастер гасит и зажигает сам, игрок — через Мастера (сокет): обновлять свет
// игрокам Foundry не даёт. Кнопку игрок видит, только если видит само место.
const SOCKET = `module.${MODULE_ID}`;
let switches = null;

function lightIcon(doc) {
  return doc.hidden ? "icons/svg/light-off.svg" : "icons/svg/light.svg";
}

async function toggleLight(sceneId, lightId) {
  const doc = game.scenes.get(sceneId)?.lights.get(lightId);
  if (!doc?.flags?.[MODULE_ID]?.toggle) return;
  await doc.update({ hidden: !doc.hidden });
}

function requestToggle(doc) {
  if (game.user.isGM) return toggleLight(doc.parent.id, doc.id);
  if (!game.users.activeGM) return ui.notifications.warn("Картограф: огонь переключит Мастер — его сейчас нет в игре");
  game.socket.emit(SOCKET, { action: "light", scene: doc.parent.id, light: doc.id });
}

function rebuildSwitches() {
  if (switches && !switches.destroyed) switches.destroy({ children: true });
  switches = null;
  if (!canvas.ready || !canvas.scene) return;
  const lights = canvas.scene.lights.filter((l) => l.flags?.[MODULE_ID]?.toggle);
  if (!lights.length) return;
  switches = new PIXI.Container();
  switches.sortableChildren = false;
  const size = Math.round(canvas.dimensions.size * 0.4);
  for (const doc of lights) {
    const icon = PIXI.Sprite.from(lightIcon(doc));
    icon.anchor.set(0.5);
    icon.width = icon.height = size;
    icon.position.set(doc.x, doc.y);
    icon.alpha = doc.hidden ? 0.55 : 0.9;
    icon.eventMode = "static";
    icon.cursor = "pointer";
    icon.lightDoc = doc;
    icon.on("pointerdown", (e) => { e.stopPropagation(); requestToggle(doc); });
    switches.addChild(icon);
  }
  canvas.controls.addChild(switches);
  refreshSwitches();
}

/** Кнопки видны Мастеру всегда, игроку — там, где он видит. */
function refreshSwitches() {
  if (!switches || switches.destroyed) return;
  for (const icon of switches.children) {
    const d = icon.lightDoc;
    icon.visible = game.user.isGM || canvas.visibility?.testVisibility({ x: d.x, y: d.y }, { tolerance: 2 }) === true;
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

// Бывает, что окно `showOpenFilePicker` есть, а читать выбранный файл браузер не даёт
// (`getFile` → NotAllowedError) — так во встроенном браузере приложения Claude.
// Тогда раз и навсегда для этого браузера переходим на обычное окно выбора файла.
const NO_FSA_KEY = `${MODULE_ID}.noFileSystemAccess`;
function fsaUsable() {
  if (!window.showOpenFilePicker) return false;
  try { return localStorage.getItem(NO_FSA_KEY) !== "1"; } catch { return true; }
}
function fsaBlocked() {
  try { localStorage.setItem(NO_FSA_KEY, "1"); } catch { /* не запомнится — спросим снова */ }
}

async function unkeep(name) {
  const d = await db();
  await new Promise((ok, no) => {
    const t = d.transaction("packs", "readwrite");
    t.objectStore("packs").delete(name);
    t.oncomplete = ok;
    t.onerror = () => no(t.error);
  });
}

// Паки и своя папка, выбранные раньше: зовётся из нажатия кнопки, чтобы браузер мог
// спросить разрешение.
async function restore() {
  if (!fsaUsable()) return;
  for (const [name, h] of await kept()) {
    if (files.has(name)) continue;
    try {
      let p = await h.queryPermission({ mode: "read" });
      if (p === "prompt") p = await h.requestPermission({ mode: "read" });
      if (p !== "granted") continue;
      if (h.kind === "directory") {
        if (![...files.keys()].some((k) => k.startsWith(`/${h.name}/`))) await openFolderHandle(h);
        continue;
      }
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
  if (fsaUsable()) {
    pickHandles(accept, multiple)
      .then(async (handles) => {
        for (const h of handles) {
          const f = await h.getFile();
          files.set(f.name, f);
          picked.push(f.name);
          keep(f.name, h).catch((err) => console.warn(`${MODULE_ID} | пак не запомнился`, err));
        }
      })
      .catch((err) => {
        if (err.name === "AbortError") return;
        if (err.name !== "NotAllowedError" && err.name !== "SecurityError") {
          console.error(`${MODULE_ID} | выбор пака`, err);
          return;
        }
        console.warn(`${MODULE_ID} | браузер не даёт читать файлы из нового окна выбора — дальше обычное окно`, err);
        fsaBlocked();
        // Пока жест пользователя ещё действует, сразу открываем обычное окно; иначе просим нажать ещё раз.
        if (navigator.userActivation?.isActive) pickInput(accept, multiple);
        else ui.notifications?.warn("Картограф: этот браузер не даёт читать паки через новое окно выбора. Нажмите «добавить пак» ещё раз — откроется обычное окно.");
      });
    return;
  }
  pickInput(accept, multiple);
}

function pickInput(accept, multiple) {
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

// Своя папка картинок — выбранная в окне браузера папка с диска Мастера. Файлы
// ложатся в тот же список, что паки, под путём `/<папка>/<путь внутри>`: Rust
// читает их так же кусками, браузер так же раскрывает. Ручка папки, как у паков,
// помнится в IndexedDB (ключ `dir:<имя>`); где нового окна нет — обычное окно
// выбора папки, и её выбирают заново.
const IMAGE_EXTS = ["png", "jpg", "jpeg", "webp"];
const isImage = (name) => IMAGE_EXTS.includes(name.split(".").pop().toLowerCase());

async function walkDir(handle, prefix, out) {
  for await (const [name, h] of handle.entries()) {
    if (h.kind === "directory") await walkDir(h, `${prefix}/${name}`, out);
    else if (isImage(name)) out.push([`${prefix}/${name}`, await h.getFile()]);
  }
}

/** Папка выбрана: прежнюю своя папку убрать, файлы новой — в список. */
function takeFolder(root, entries) {
  for (const k of [...files.keys()]) if (k.startsWith("/")) files.delete(k);
  for (const [path, f] of entries) files.set(path, f);
  picked.push(`dir:/${root}`);
}

async function forgetFolders() {
  for (const [name] of await kept()) if (name.startsWith("dir:")) await unkeep(name);
}

async function openFolderHandle(h) {
  const entries = [];
  await walkDir(h, `/${h.name}`, entries);
  takeFolder(h.name, entries);
}

function pickFolder() {
  if (fsaUsable() && window.showDirectoryPicker) {
    window.showDirectoryPicker({ mode: "read" })
      .then(async (h) => {
        await openFolderHandle(h);
        await forgetFolders();
        keep(`dir:${h.name}`, h).catch((err) => console.warn(`${MODULE_ID} | папка не запомнилась`, err));
      })
      .catch((err) => {
        if (err.name === "AbortError") return;
        console.warn(`${MODULE_ID} | новое окно выбора папки не сработало — обычное`, err);
        if (err.name === "NotAllowedError" || err.name === "SecurityError") fsaBlocked();
        if (navigator.userActivation?.isActive) pickFolderInput();
        else ui.notifications?.warn("Картограф: выберите папку ещё раз — откроется обычное окно.");
      });
    return;
  }
  pickFolderInput();
}

function pickFolderInput() {
  const input = document.createElement("input");
  input.type = "file";
  input.webkitdirectory = true;
  input.addEventListener("change", () => {
    const all = [...input.files];
    if (!all.length) return;
    const root = all[0].webkitRelativePath.split("/")[0];
    takeFolder(root, all.filter((f) => isImage(f.name)).map((f) => [`/${f.webkitRelativePath}`, f]));
  });
  input.click();
}

// Файлы редактора — настройки, автосейв, карты, объединённые объекты — лежат в
// данных Foundry, в папке `kartograf-editor`: переживают и закрытую вкладку, и
// другой браузер. Rust видит их как `/kartograf-editor/…` и работает синхронно
// (`plat::fs`): список и мелкие `.json` подгружаются до запуска редактора, крупное
// читается по запросу синхронным запросом, запись уходит на сервер в фоне — по
// одной за раз на файл, последняя версия побеждает. Удалить файл Foundry не даёт:
// удалённый остаётся пустым, а пустой считается несуществующим.
const DATA = "kartograf-editor";
const data = new Map(); // путь без «/» → байты; null — есть на сервере, ещё не прочитан
const uploads = new Map();
const madeDirs = new Set();

const dataPath = (p) => (p.startsWith(`/${DATA}/`) ? p.slice(1) : null);

// Foundry принимает файлы только знакомых видов (`CONST.UPLOADABLE_FILE_EXTENSIONS`),
// `.karta` среди них нет. Карта — JSON, на сервере она `<имя>.karta.json`; Rust
// видит прежнее имя.
const uploadable = (p) => p.includes(".") && p.split(".").pop().toLowerCase() in CONST.UPLOADABLE_FILE_EXTENSIONS;
const onServer = (p) => (uploadable(p) ? p : `${p}.json`);
function fromServer(p) {
  const inner = p.endsWith(".json") ? p.slice(0, -5) : null;
  return inner && inner.split("/").pop().includes(".") && !uploadable(inner) ? inner : p;
}
const urlOf = (p) => `${foundry.utils.getRoute(onServer(p).split("/").map(encodeURIComponent).join("/"))}?t=${Date.now()}`;

function fetchSync(path) {
  const x = new XMLHttpRequest();
  x.open("GET", urlOf(path), false);
  x.overrideMimeType("text/plain; charset=x-user-defined");
  x.send();
  if (x.status !== 200) return new Uint8Array(0);
  const s = x.responseText;
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

// ----- сеть для «Местности по координатам» -----
// Сборка места в wasm идёт одним куском на главном потоке, а сеть там — только
// синхронным запросом, который вешает окно. Поэтому всё, что ей заведомо нужно
// (выгрузка OSM, плитки снимка, слои), сперва качается здесь фоном, по шесть
// запросов разом, а сборка берёт готовое из `netCache`. Чего заранее не узнать
// (докачка неполных контуров), она тянет синхронно.
const NET_PARALLEL = 6;
// Скачанное держится в памяти вкладки: повторная сборка того же места не ходит
// в сеть. Старое вытесняется — плитка ≈20 КБ, 600 штук ≈12 МБ.
const NET_KEEP = 600;
const netCache = new Map();
let netJob = { done: 0, total: 0 };

function netKeep(url, bytes) {
  netCache.delete(url);
  netCache.set(url, bytes);
  while (netCache.size > NET_KEEP) netCache.delete(netCache.keys().next().value);
}

function netPrefetch(urls) {
  const all = [...new Set(urls)];
  const todo = all.filter((u) => !netCache.has(u));
  const job = { done: all.length - todo.length, total: all.length };
  netJob = job;
  let next = 0;
  const worker = async () => {
    while (next < todo.length) {
      const url = todo[next++];
      try {
        const r = await fetch(url);
        // Отказ не держим: сборка спросит ещё раз и скажет, что не так.
        if (r.ok) netKeep(url, new Uint8Array(await r.arrayBuffer()));
      } catch (err) {
        console.warn(`${MODULE_ID} | не скачалось ${url}`, err);
      }
      job.done++;
    }
  };
  for (let k = 0; k < NET_PARALLEL; k++) worker();
}

// Плитки карты выбора места и поиск: по одному адресу, фоном, по восемь разом.
// Последнее спрошенное — первым: карту тащат, и нужны те плитки, что видны сейчас.
const GET_PARALLEL = 8;
const getQueue = [];
const gotList = [];
let getActive = 0;

function netGet(url) {
  getQueue.push(url);
  getPump();
}

function getPump() {
  while (getActive < GET_PARALLEL && getQueue.length) {
    const url = getQueue.pop();
    getActive++;
    fetch(url)
      .then(async (r) => {
        if (!r.ok) throw new Error(`${r.status}`);
        const bytes = new Uint8Array(await r.arrayBuffer());
        // Плитки снимка пригодятся и сборке места.
        if (url.includes("/World_Imagery/")) netKeep(url, bytes);
        gotList.push([url, bytes]);
      })
      .catch((err) => gotList.push([url, String(err?.message ?? err)]))
      .finally(() => {
        getActive--;
        getPump();
      });
  }
}

function netFetch(url) {
  const hit = netCache.get(url);
  if (hit) return hit;
  const x = new XMLHttpRequest();
  x.open("GET", url, false);
  x.overrideMimeType("text/plain; charset=x-user-defined");
  x.send();
  const s = x.responseText;
  if (x.status !== 200) throw new Error(`${x.status} ${s.slice(0, 200)}`.trim());
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  netKeep(url, out);
  return out;
}

async function dataPreload() {
  const FP = foundry.applications.apps.FilePicker.implementation;
  const walk = async (dir) => {
    let r;
    try { r = await FP.browse("data", dir); } catch { return; }
    madeDirs.add(dir);
    for (const f of r.files) {
      const p = fromServer(decodeURIComponent(f));
      if (!data.has(p)) data.set(p, null);
    }
    for (const d of r.dirs) await walk(decodeURIComponent(d));
  };
  await walk(DATA);
  // Настройки и описания копий автосейва нужны редактору сразу при запуске.
  await Promise.all([...data.entries()].filter(([p, b]) => b === null && p.endsWith(".json")).map(async ([p]) => {
    try {
      const r = await fetch(urlOf(p));
      data.set(p, r.ok ? new Uint8Array(await r.arrayBuffer()) : new Uint8Array(0));
    } catch {
      data.set(p, new Uint8Array(0));
    }
  }));
}

function dataGet(p) {
  if (!data.has(p)) return null;
  let b = data.get(p);
  if (b === null) { b = fetchSync(p); data.set(p, b); }
  return b.length ? b : null;
}

async function upload(p, bytes) {
  const FP = foundry.applications.apps.FilePicker.implementation;
  const parts = onServer(p).split("/");
  const name = parts.pop();
  for (let i = 1; i <= parts.length; i++) {
    const d = parts.slice(0, i).join("/");
    if (madeDirs.has(d)) continue;
    try { await FP.createDirectory("data", d); } catch { /* уже есть */ }
    madeDirs.add(d);
  }
  const type = name.endsWith(".json") ? "application/json" : name.endsWith(".png") ? "image/png" : "application/octet-stream";
  // Отказ Foundry не бросает, а возвращает false.
  const r = await FP.upload("data", parts.join("/"), new File([bytes], name, { type }), {}, { notify: false });
  if (!r || r.status === "error") throw new Error(r?.message ?? "Foundry не принял файл");
}

function dataPut(p, bytes) {
  data.set(p, bytes);
  const u = uploads.get(p) ?? { busy: false, again: false };
  uploads.set(p, u);
  u.again = true;
  if (u.busy) return;
  u.busy = true;
  (async () => {
    while (u.again) {
      u.again = false;
      try {
        await upload(p, data.get(p) ?? new Uint8Array(0));
      } catch (err) {
        console.error(`${MODULE_ID} | ${p} не записался`, err);
        ui.notifications?.error(`Картограф: файл ${p} не записался в данные Foundry, подробности в консоли`);
      }
    }
    u.busy = false;
  })();
}

/** Файл по пути Rust: из данных Foundry или из выбранной папки. */
function readAny(path) {
  const p = dataPath(path);
  if (p) return dataGet(p);
  const f = files.get(path);
  return f ? readSync(path, 0, f.size) : null;
}

function treeOf(dir) {
  const prefix = dir.endsWith("/") ? dir : `${dir}/`;
  const out = [];
  const p = dataPath(prefix);
  if (p !== null) {
    for (const [k, b] of data) if (k.startsWith(p) && (b === null || b.length)) out.push(`/${k}`);
  } else {
    for (const k of files.keys()) if (k.startsWith(prefix)) out.push(k);
  }
  return out;
}

// Замок окна для автосейва: живая вкладка раз в несколько секунд отмечается в
// localStorage. Копия, чья отметка давно не обновлялась, — от закрытой вкладки.
const ALIVE = `${MODULE_ID}.alive.`;
const ALIVE_EVERY = 5000;
const ALIVE_STALE = 20000;
const holding = new Map();

function hold(id) {
  const beat = () => { try { localStorage.setItem(ALIVE + id, String(Date.now())); } catch { /* без отметки — копию предложат вернуть */ } };
  beat();
  holding.set(id, setInterval(beat, ALIVE_EVERY));
  // Отметки давно закрытых вкладок не копятся.
  try {
    for (let i = localStorage.length - 1; i >= 0; i--) {
      const k = localStorage.key(i);
      if (k?.startsWith(ALIVE) && Date.now() - Number(localStorage.getItem(k)) > 86400000) localStorage.removeItem(k);
    }
  } catch { /* нет хранилища */ }
}

function alive(id) {
  try { return Date.now() - Number(localStorage.getItem(ALIVE + id) ?? 0) < ALIVE_STALE; } catch { return false; }
}

function release(id) {
  clearInterval(holding.get(id));
  holding.delete(id);
  try { localStorage.removeItem(ALIVE + id); } catch { /* нет хранилища */ }
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
  window.kartografPickFolder = pickFolder;
  window.kartografRead = readAny;
  window.kartografWrite = (path, bytes) => {
    const p = dataPath(path);
    if (!p) throw new Error(`${path}: писать можно только в данные редактора`);
    dataPut(p, bytes.slice());
  };
  window.kartografRemove = (path) => {
    const p = dataPath(path);
    if (p && data.has(p)) dataPut(p, new Uint8Array(0));
  };
  window.kartografExists = (path) => {
    const p = dataPath(path);
    return p ? data.has(p) && (data.get(p) === null || data.get(p).length > 0) : files.has(path);
  };
  window.kartografTree = treeOf;
  window.kartografFetch = netFetch;
  window.kartografNetPrefetch = netPrefetch;
  window.kartografNetState = () => [netJob.done, netJob.total];
  window.kartografGet = netGet;
  window.kartografGot = () => gotList.splice(0);
  window.kartografHold = hold;
  window.kartografAlive = alive;
  window.kartografRelease = release;
  game.settings.register(MODULE_ID, "lastScene", { scope: "world", config: false, type: String, default: "" });
  window.kartografLastScene = () => lastScene()?.name ?? null;
  window.kartografToScene = (files, update) => {
    toScene(files, update).catch((err) => { console.error(`${MODULE_ID} | на сцену`, err); ui.notifications.error("Картограф: сцена не создалась, подробности в консоли"); });
  };
  // `takeFolder(имя, [[путь, File]…])` — своя папка без окна выбора (для проверок и макросов).
  game.modules.get(MODULE_ID).api = { open, close, chests, rebuildWebs, rebuildSwitches, toggleLight, takeFolder };
});

Hooks.on("canvasReady", rebuildWebs);
for (const h of ["createTile", "updateTile", "deleteTile"]) Hooks.on(h, () => setTimeout(rebuildWebs, 0));
Hooks.on("canvasReady", rebuildSwitches);
for (const h of ["createAmbientLight", "updateAmbientLight", "deleteAmbientLight"]) Hooks.on(h, () => setTimeout(rebuildSwitches, 0));
Hooks.on("sightRefresh", refreshSwitches);
Hooks.once("ready", () => {
  game.socket.on(SOCKET, (msg) => {
    // Переключает один Мастер — активный, чтобы не дважды.
    if (msg?.action === "light" && game.user === game.users.activeGM) toggleLight(msg.scene, msg.light);
  });
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
