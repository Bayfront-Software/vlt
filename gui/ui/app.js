// 画面の配線。判断は logic.mjs、データは Rust（invoke）に任せ、ここは DOM の更新だけ。
import {
  resolveScreen,
  splitKey,
  filterItems,
  groupItems,
  pickAfterDelete,
  stepSelection,
  formatStamp,
  validateNewItem,
  validatePassphrase,
} from "./logic.mjs";

const invoke = window.__TAURI__?.core?.invoke ?? (async () => { throw new Error("Tauri が無い環境です"); });
const $ = (id) => document.getElementById(id);

/** 表示した値を自動で隠すまでの秒数（1Password と同じ感覚で短めに）。 */
const REVEAL_SECS = 30;

const state = {
  screen: null,
  items: [],
  query: "",
  selectedKey: null,
  revealTimer: null,
  toastTimer: null,
};

// ---------- 画面切替 ----------

async function refreshStatus() {
  const status = await invoke("vault_status");
  showScreen(resolveScreen(status));
}

function showScreen(screen) {
  state.screen = screen;
  for (const name of ["setup", "locked", "main"]) {
    $(`screen-${name}`).hidden = name !== screen;
  }
  if (screen === "main") {
    reloadList().then(() => $("search").focus());
  }
}

// ---------- 一覧 ----------

async function reloadList() {
  state.items = await invoke("list_secrets");
  renderList();
  if (state.selectedKey && !state.items.some((i) => i.key === state.selectedKey)) {
    selectItem(null);
  } else {
    renderDetail();
  }
}

function visibleKeys() {
  return groupItems(filterItems(state.items, state.query)).flatMap((g) => g.items.map((i) => i.key));
}

function renderList() {
  const list = $("list");
  list.replaceChildren();
  const groups = groupItems(filterItems(state.items, state.query));
  if (groups.length === 0) {
    const empty = document.createElement("div");
    empty.className = "list-empty";
    empty.textContent = state.items.length === 0 ? "まだ何も保存されていません" : "該当なし";
    list.append(empty);
    return;
  }
  for (const group of groups) {
    const section = document.createElement("div");
    section.className = "list-group";
    if (group.namespace) {
      const h = document.createElement("h4");
      h.textContent = group.namespace;
      section.append(h);
    }
    for (const item of group.items) {
      const { name } = splitKey(item.key);
      const button = document.createElement("button");
      button.className = "list-item" + (item.key === state.selectedKey ? " selected" : "");
      button.dataset.key = item.key;
      const badge = document.createElement("span");
      badge.className = "badge" + (item.binary ? " bin" : "");
      badge.textContent = item.binary ? "BIN" : name.slice(0, 2).toUpperCase();
      const label = document.createElement("span");
      label.className = "name";
      label.textContent = name;
      button.append(badge, label);
      button.addEventListener("click", () => selectItem(item.key));
      section.append(button);
    }
    list.append(section);
  }
}

function selectItem(key) {
  concealValue();
  state.selectedKey = key;
  renderList();
  renderDetail();
  const selected = $("list").querySelector(".list-item.selected");
  selected?.scrollIntoView({ block: "nearest" });
}

// ---------- 詳細 ----------

function currentItem() {
  return state.items.find((i) => i.key === state.selectedKey) ?? null;
}

function renderDetail() {
  const item = currentItem();
  $("detail-empty").hidden = item !== null;
  $("detail-item").hidden = item === null;
  if (!item) return;
  const { namespace, name } = splitKey(item.key);
  $("item-name").textContent = name;
  $("item-namespace").textContent = namespace || "（ルート）";
  const badge = $("item-badge");
  badge.className = "item-badge" + (item.binary ? " bin" : "");
  badge.textContent = item.binary ? "BIN" : name.slice(0, 2).toUpperCase();
  $("field-text").hidden = item.binary;
  $("field-binary").hidden = !item.binary;
  $("btn-edit").hidden = item.binary;
  $("item-created").textContent = formatStamp(item.created_at);
  $("item-updated").textContent = formatStamp(item.updated_at);
}

async function revealValue() {
  const item = currentItem();
  if (!item || item.binary) return;
  const box = $("value-box");
  if (!box.classList.contains("masked")) {
    concealValue();
    return;
  }
  try {
    const value = await invoke("reveal_secret", { key: item.key });
    $("value-text").textContent = value;
    box.classList.remove("masked");
    $("btn-reveal").textContent = "隠す";
    clearTimeout(state.revealTimer);
    state.revealTimer = setTimeout(concealValue, REVEAL_SECS * 1000);
  } catch (e) {
    toast(String(e), true);
  }
}

function concealValue() {
  clearTimeout(state.revealTimer);
  state.revealTimer = null;
  $("value-text").textContent = "••••••••••••••••";
  $("value-box").classList.add("masked");
  $("btn-reveal").textContent = "表示";
}

async function copyValue() {
  const item = currentItem();
  if (!item) return;
  try {
    const secs = await invoke("copy_secret", { key: item.key });
    toast(`コピーしました（${secs} 秒後に消去）`);
  } catch (e) {
    toast(String(e), true);
  }
}

// ---------- ダイアログ ----------

/**
 * 汎用ダイアログ。build(body) で中身を作り、onSubmit が文字列を返せばエラー表示、
 * null/undefined なら閉じる。
 */
function openModal({ title, okLabel = "OK", danger = false, build, onSubmit }) {
  const modal = $("modal");
  const body = $("modal-body");
  body.replaceChildren();
  $("modal-title").textContent = title;
  $("modal-error").textContent = "";
  const ok = $("modal-ok");
  ok.textContent = okLabel;
  ok.className = "primary" + (danger ? " danger" : "");
  const refs = build(body);
  modal.hidden = false;
  const first = body.querySelector("input, textarea");
  first?.focus();

  const form = $("modal-form");
  form.onsubmit = async (ev) => {
    ev.preventDefault();
    ok.disabled = true;
    try {
      const error = await onSubmit(refs);
      if (error) {
        $("modal-error").textContent = error;
      } else {
        closeModal();
      }
    } catch (e) {
      $("modal-error").textContent = String(e);
    } finally {
      ok.disabled = false;
    }
  };
}

function closeModal() {
  $("modal").hidden = true;
  $("modal-form").onsubmit = null;
}

function field(body, { label, type = "text", value = "", placeholder = "", mono = false }) {
  const l = document.createElement("label");
  l.textContent = label;
  const input = document.createElement(type === "textarea" ? "textarea" : "input");
  if (type !== "textarea") input.type = type;
  input.value = value;
  input.placeholder = placeholder;
  input.spellcheck = false;
  if (mono) input.style.fontFamily = "ui-monospace, Menlo, monospace";
  body.append(l, input);
  return input;
}

function note(body, text, warn = false) {
  const p = document.createElement("p");
  p.className = "note" + (warn ? " warn" : "");
  p.textContent = text;
  body.append(p);
  return p;
}

function newItemDialog() {
  openModal({
    title: "新しい項目",
    okLabel: "保存",
    build: (body) => {
      const key = field(body, { label: "キー", placeholder: "例: openai/api-key", mono: true });
      const modeLabel = document.createElement("label");
      modeLabel.textContent = "種類";
      const seg = document.createElement("div");
      seg.className = "segmented";
      const textBtn = document.createElement("button");
      textBtn.type = "button"; textBtn.textContent = "テキスト"; textBtn.className = "on";
      const fileBtn = document.createElement("button");
      fileBtn.type = "button"; fileBtn.textContent = "ファイル";
      seg.append(textBtn, fileBtn);
      body.append(modeLabel, seg);
      const value = field(body, { label: "値", type: "textarea", mono: true });
      const hint = note(body, "");
      const refs = { key, value, mode: "text" };
      const setMode = (mode) => {
        refs.mode = mode;
        textBtn.classList.toggle("on", mode === "text");
        fileBtn.classList.toggle("on", mode === "file");
        value.hidden = mode === "file";
        value.previousElementSibling.hidden = mode === "file";
        hint.textContent = mode === "file" ? "保存を押すとファイル選択が開きます。中身はバイナリとして暗号化されます。" : "";
      };
      textBtn.addEventListener("click", () => setMode("text"));
      fileBtn.addEventListener("click", () => setMode("file"));
      return refs;
    },
    onSubmit: async ({ key, value, mode }) => {
      const error = validateNewItem({ key: key.value, mode, value: value.value });
      if (error) return error;
      if (state.items.some((i) => i.key === key.value)) return `${key.value} は既にあります`;
      if (mode === "text") {
        await invoke("set_secret", { key: key.value, value: value.value });
      } else {
        const stored = await invoke("import_file_as_secret", { key: key.value });
        if (!stored) return "ファイルが選ばれませんでした";
      }
      await reloadList();
      selectItem(key.value);
      toast("保存しました");
      return null;
    },
  });
}

function editValueDialog() {
  const item = currentItem();
  if (!item || item.binary) return;
  openModal({
    title: `${item.key} の値を書き換える`,
    okLabel: "保存",
    build: (body) => ({ value: field(body, { label: "新しい値", type: "textarea", mono: true }) }),
    onSubmit: async ({ value }) => {
      if (value.value.length === 0) return "値を入力してください";
      await invoke("set_secret", { key: item.key, value: value.value });
      concealValue();
      await reloadList();
      toast("更新しました");
      return null;
    },
  });
}

async function replaceFromFile() {
  const item = currentItem();
  if (!item) return;
  try {
    const stored = await invoke("import_file_as_secret", { key: item.key });
    if (!stored) return;
    concealValue();
    await reloadList();
    toast("ファイルの内容で置き換えました");
  } catch (e) {
    toast(String(e), true);
  }
}

async function saveToFile() {
  const item = currentItem();
  if (!item) return;
  try {
    const path = await invoke("save_secret_to_file", { key: item.key });
    if (path) toast(`保存しました: ${path}`);
  } catch (e) {
    toast(String(e), true);
  }
}

function renameDialog() {
  const item = currentItem();
  if (!item) return;
  openModal({
    title: "名前を変更",
    okLabel: "変更",
    build: (body) => ({ to: field(body, { label: "新しいキー", value: item.key, mono: true }) }),
    onSubmit: async ({ to }) => {
      if (!to.value.trim()) return "キーを入力してください";
      await invoke("rename_secret", { from: item.key, to: to.value });
      await reloadList();
      selectItem(to.value);
      toast("名前を変更しました");
      return null;
    },
  });
}

function deleteDialog() {
  const item = currentItem();
  if (!item) return;
  openModal({
    title: "削除しますか？",
    okLabel: "削除",
    danger: true,
    build: (body) => {
      note(body, `${item.key} を vault から削除します。元に戻せません。`, true);
      return {};
    },
    onSubmit: async () => {
      const next = pickAfterDelete(visibleKeys(), item.key);
      await invoke("delete_secret", { key: item.key });
      await reloadList();
      selectItem(next);
      toast("削除しました");
      return null;
    },
  });
}

function exportDialog() {
  openModal({
    title: "バックアップを書き出す",
    okLabel: "書き出す",
    build: (body) => {
      note(body, "パスフレーズだけで復元できる .vltx を作ります（キーチェーン不要）。パスフレーズを失うと復元できません。");
      const passphrase = field(body, { label: "パスフレーズ", type: "password" });
      const confirm = field(body, { label: "パスフレーズ（確認）", type: "password" });
      return { passphrase, confirm };
    },
    onSubmit: async ({ passphrase, confirm }) => {
      const error = validatePassphrase({ passphrase: passphrase.value, confirm: confirm.value, requireConfirm: true });
      if (error) return error;
      const path = await invoke("export_vault", { passphrase: passphrase.value });
      if (path) toast(`書き出しました: ${path}`);
      return null;
    },
  });
}

function importDialog() {
  openModal({
    title: "バックアップを読み込む",
    okLabel: "ファイルを選んで読み込む",
    build: (body) => {
      const passphrase = field(body, { label: "パスフレーズ", type: "password" });
      const check = document.createElement("label");
      check.className = "check";
      const overwrite = document.createElement("input");
      overwrite.type = "checkbox";
      check.append(overwrite, document.createTextNode("同じキーがあれば上書きする"));
      body.append(check);
      return { passphrase, overwrite };
    },
    onSubmit: async ({ passphrase, overwrite }) => {
      const error = validatePassphrase({ passphrase: passphrase.value, confirm: "", requireConfirm: false });
      if (error) return error;
      const result = await invoke("import_vault", { passphrase: passphrase.value, overwrite: overwrite.checked });
      if (!result) return "ファイルが選ばれませんでした";
      await reloadList();
      toast(`${result.imported} 件を取り込みました（${result.skipped} 件はスキップ）`);
      return null;
    },
  });
}

// ---------- ロック / 解錠 ----------

async function unlock() {
  $("locked-error").textContent = "";
  try {
    await invoke("unlock");
    await refreshStatus();
  } catch (e) {
    $("locked-error").textContent = String(e);
  }
}

async function initVault() {
  $("setup-error").textContent = "";
  try {
    await invoke("init_vault");
    await refreshStatus();
  } catch (e) {
    $("setup-error").textContent = String(e);
  }
}

async function lock() {
  concealValue();
  closeMenu();
  state.selectedKey = null;
  state.query = "";
  $("search").value = "";
  await invoke("lock");
  await refreshStatus();
}

// ---------- 小物 ----------

function toast(message, isError = false) {
  const el = $("toast");
  el.textContent = message;
  el.className = "toast" + (isError ? " error" : "");
  el.hidden = false;
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => { el.hidden = true; }, isError ? 5000 : 2500);
}

function toggleMenu() {
  $("menu-popup").hidden = !$("menu-popup").hidden;
}
function closeMenu() {
  $("menu-popup").hidden = true;
}

// ---------- 配線 ----------

document.addEventListener("click", (ev) => {
  const target = ev.target.closest("[data-action]");
  if (!target) {
    if (!ev.target.closest(".menu")) closeMenu();
    return;
  }
  const action = target.dataset.action;
  if (action !== "menu") closeMenu();
  switch (action) {
    case "init": initVault(); break;
    case "unlock": unlock(); break;
    case "lock": lock(); break;
    case "new": newItemDialog(); break;
    case "menu": toggleMenu(); break;
    case "export": exportDialog(); break;
    case "import": importDialog(); break;
    case "reveal": revealValue(); break;
    case "copy": copyValue(); break;
    case "save-file": saveToFile(); break;
    case "edit": editValueDialog(); break;
    case "replace-file": replaceFromFile(); break;
    case "rename": renameDialog(); break;
    case "delete": deleteDialog(); break;
    case "modal-cancel": closeModal(); break;
    default: break;
  }
});

$("search").addEventListener("input", (ev) => {
  state.query = ev.target.value;
  renderList();
});

document.addEventListener("keydown", (ev) => {
  const modalOpen = !$("modal").hidden;
  if (ev.key === "Escape") {
    if (modalOpen) closeModal();
    else closeMenu();
    return;
  }
  if (modalOpen) return;
  const meta = ev.metaKey || ev.ctrlKey;
  if (meta && ev.key === "f") { ev.preventDefault(); $("search").focus(); $("search").select(); return; }
  if (state.screen !== "main") return;
  if (meta && ev.key === "n") { ev.preventDefault(); newItemDialog(); return; }
  if (meta && ev.key === "l") { ev.preventDefault(); lock(); return; }
  if (meta && ev.key === "c" && currentItem() && !window.getSelection()?.toString()) {
    ev.preventDefault(); copyValue(); return;
  }
  if (ev.key === "ArrowDown" || ev.key === "ArrowUp") {
    ev.preventDefault();
    selectItem(stepSelection(visibleKeys(), state.selectedKey, ev.key === "ArrowDown" ? 1 : -1));
  }
});

window.addEventListener("blur", concealValue);

refreshStatus().catch((e) => {
  showScreen("locked");
  $("locked-error").textContent = String(e);
});
