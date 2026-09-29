// 画面の配線。判断は logic.mjs、データは Rust（invoke）に任せ、ここは DOM の更新だけ。
// 利用者のデータ（キー・値・メモ）は必ず textContent で入れる。innerHTML には固定のアイコンだけ。
import * as L from "./logic.mjs";
import { icon, typeBadge } from "./icons.mjs";

const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke ?? (async () => { throw new Error("Tauri が無い環境です"); });
const listen = tauri?.event?.listen ?? (async () => () => {});
const $ = (id) => document.getElementById(id);

/** 表示した値を自動で隠すまでの秒数。 */
const REVEAL_SECS = 30;
/** 画面の操作を自動ロックのタイマーへ伝える間隔（毎回送らない）。 */
const TOUCH_THROTTLE_MS = 15000;

const FIELD_KINDS = [
  { kind: "text", label: "テキスト" },
  { kind: "concealed", label: "パスワード" },
  { kind: "multiline", label: "複数行テキスト" },
  { kind: "secret_block", label: "秘密のテキスト（複数行）" },
  { kind: "url", label: "URL" },
  { kind: "email", label: "メール" },
  { kind: "phone", label: "電話番号" },
  { kind: "date", label: "日付" },
  { kind: "totp", label: "ワンタイムパスワード" },
  { kind: "file", label: "ファイル" },
];

const state = {
  info: null,
  types: [],
  items: [],
  trash: [],
  category: "all",
  query: "",
  selectedKey: null,
  selectedTrashId: null,
  view: null,
  revealed: new Map(),
  revealTimer: null,
  otp: new Map(),
  otpTimer: null,
  editing: null,
  lockReason: "",
  autoUnlockTried: false,
  lastTouch: 0,
  toastTimer: null,
};

// ---------- DOM ヘルパ ----------

function h(tag, props = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === "class") el.className = v;
    else if (k === "icon") el.innerHTML = v; // 固定のアイコン文字列だけを渡すこと
    else if (k === "dataset") Object.assign(el.dataset, v);
    else if (k === "style") el.setAttribute("style", v);
    else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
    else if (typeof v === "boolean") el[k] = v;
    else if (k === "value") el.value = v;
    else el.setAttribute(k, v);
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    el.append(child instanceof Node ? child : String(child));
  }
  return el;
}

function fromHtml(trustedHtml) {
  const t = document.createElement("template");
  t.innerHTML = trustedHtml.trim();
  return t.content.firstElementChild;
}

function iconButton(name, title, dataset, extraClass = "") {
  return h("button", { type: "button", class: `icon-btn ${extraClass}`.trim(), title, "aria-label": title, dataset, icon: icon(name, { size: 17 }) });
}

function toast(message, isError = false) {
  const el = $("toast");
  el.textContent = message;
  el.className = "toast" + (isError ? " error" : "");
  el.hidden = false;
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => { el.hidden = true; }, isError ? 5000 : 2400);
}

async function guarded(fn) {
  try {
    return await fn();
  } catch (e) {
    toast(String(e), true);
    return undefined;
  }
}

// ---------- 画面切替 ----------

async function refreshStatus() {
  state.info = await invoke("app_info");
  showScreen(L.resolveScreen(state.info));
}

function showScreen(screen) {
  for (const name of ["setup", "locked", "main"]) $(`screen-${name}`).hidden = name !== screen;
  closeModal();
  closePopover();
  if (screen === "locked") renderLocked();
  if (screen === "main") enterMain();
}

function renderLocked() {
  const bio = state.info?.biometrics && state.info?.settings?.unlock_with_touch_id;
  const btn = $("btn-unlock");
  btn.replaceChildren();
  if (bio) btn.append(fromHtml(icon("fingerprint", { size: 18 })));
  btn.append(bio ? "Touch ID で解錠" : "解錠");
  $("lock-reason").textContent = L.lockReasonMessage(state.lockReason, state.info?.settings?.auto_lock_minutes ?? 10);
  $("unlock-hint").textContent = state.info?.settings?.unlock_with_touch_id
    ? "Touch ID か Mac のログインパスワードで本人確認します"
    : "キーチェーンからマスターキーを読み込みます";
  if (!state.autoUnlockTried) {
    // 起動直後だけ自動で本人確認を出す。手動ロック後にすぐ出すと鬱陶しいので一度きり。
    state.autoUnlockTried = true;
    setTimeout(() => { if (!$("screen-locked").hidden) actions.unlock(); }, 250);
  }
}

async function enterMain() {
  state.types = state.types.length ? state.types : await invoke("item_types");
  await reloadItems();
  $("search").focus();
}

// ---------- 一覧 ----------

async function reloadItems() {
  state.items = await invoke("list_items");
  if (state.category === "trash") state.trash = await invoke("list_trash");
  if (state.selectedKey && !state.items.some((i) => i.key === state.selectedKey)) state.selectedKey = null;
  renderCategories();
  renderList();
  await renderDetail();
}

function categoryTitle(category) {
  if (category === "all") return "すべての項目";
  if (category === "favorites") return "お気に入り";
  if (category === "watchtower") return "要確認のパスワード";
  if (category === "trash") return "ゴミ箱";
  if (category.startsWith("type:")) return state.types.find((t) => t.id === category.slice(5))?.label ?? "";
  if (category.startsWith("tag:")) return `#${category.slice(4)}`;
  return "";
}

function renderCategories() {
  const counts = L.sidebarCounts(state.items, state.types);
  const nav = $("categories");
  nav.replaceChildren();
  const cat = (category, iconHtml, label, count, alert = false) =>
    h("button", { type: "button", class: "cat" + (state.category === category ? " selected" : ""), dataset: { action: "select-category", category } },
      fromHtml(iconHtml), h("span", { class: "label" }, label),
      count == null ? null : h("span", { class: "count" + (alert && count > 0 ? " alert" : "") }, String(count)));
  nav.append(
    cat("all", icon("all"), "すべての項目", counts.all),
    cat("favorites", icon("star"), "お気に入り", counts.favorites),
    cat("watchtower", icon("shield"), "要確認", counts.watchtower, true),
  );
  if (counts.types.length) {
    nav.append(h("div", { class: "cat-heading" }, "種類"));
    for (const t of counts.types) nav.append(cat(`type:${t.id}`, typeBadge(t.id, "sm"), t.label, t.count));
  }
  if (counts.tags.length) {
    nav.append(h("div", { class: "cat-heading" }, "タグ"));
    for (const t of counts.tags) nav.append(cat(`tag:${t.name}`, icon("tag"), t.name, t.count));
  }
  nav.append(h("div", { class: "cat-heading" }, ""), cat("trash", icon("trash"), "ゴミ箱", null));
}

function visible() {
  return L.visibleItems(state.items, state.category, state.query);
}

function renderList() {
  const list = $("list");
  list.replaceChildren();
  const isTrash = state.category === "trash";
  $("list-heading").textContent = categoryTitle(state.category);
  $("btn-empty-trash").hidden = !isTrash || state.trash.length === 0;
  $("btn-new").hidden = isTrash;

  if (isTrash) {
    const rows = L.filterItems(state.trash.map((t) => ({ ...t, subtitle: t.type_label, tags: [] })), state.query);
    $("list-count").textContent = String(rows.length);
    if (!rows.length) {
      list.append(h("div", { class: "list-empty" }, "ゴミ箱は空です。", h("br"), `削除した項目は ${30} 日間ここに残ります。`));
      return;
    }
    for (const t of rows) {
      list.append(h("button", { type: "button", class: "row" + (t.id === state.selectedTrashId ? " selected" : ""), dataset: { action: "select-trash", id: String(t.id) } },
        fromHtml(typeBadge(t.item_type)),
        h("span", { class: "text" }, h("div", { class: "title" }, t.key), h("div", { class: "sub" }, `${L.formatStamp(t.deleted_at)} に削除`))));
    }
    return;
  }

  const items = visible();
  $("list-count").textContent = String(items.length);
  if (!items.length) {
    const msg = state.items.length === 0
      ? ["まだ項目がありません。", h("br"), "⌘N で追加できます。"]
      : state.query ? ["「", state.query, "」に一致する項目はありません。"] : ["この分類の項目はありません。"];
    list.append(h("div", { class: "list-empty" }, ...msg));
    return;
  }
  const grouped = state.category === "all" || state.category.startsWith("type:") || state.category.startsWith("tag:");
  const groups = grouped ? L.groupItems(items) : [{ namespace: "", items: [...items].sort((a, b) => a.key.localeCompare(b.key)) }];
  for (const group of groups) {
    const section = h("div", { class: "list-group" });
    if (group.namespace && grouped) section.append(h("h4", {}, group.namespace));
    for (const item of group.items) {
      const { namespace, name } = L.splitKey(item.key);
      const sub = !grouped && namespace ? `${namespace} · ${item.subtitle}` : item.subtitle;
      section.append(h("button", { type: "button", class: "row" + (item.key === state.selectedKey ? " selected" : ""), dataset: { action: "select-item", key: item.key } },
        fromHtml(typeBadge(item.item_type)),
        h("span", { class: "text" }, h("div", { class: "title" }, name), h("div", { class: "sub" }, sub)),
        h("span", { class: "flags" },
          item.favorite ? h("span", { class: "fav", icon: icon("star", { size: 13 }) }) : null,
          item.weak || item.reused ? fromHtml(icon("shield", { size: 13 })) : null)));
    }
    list.append(section);
  }
  list.querySelector(".row.selected")?.scrollIntoView({ block: "nearest" });
}

async function selectItem(key) {
  if (state.editing && !(await confirmDiscard())) return;
  state.editing = null;
  state.selectedKey = key;
  renderList();
  await renderDetail();
}

// ---------- 詳細 ----------

function concealAll() {
  clearTimeout(state.revealTimer);
  state.revealTimer = null;
  if (state.revealed.size === 0) return;
  state.revealed.clear();
  if (state.view && !state.editing) renderItemView();
}

function stopOtp() {
  clearInterval(state.otpTimer);
  state.otpTimer = null;
  state.otp.clear();
}

async function renderDetail() {
  stopOtp();
  state.revealed.clear();
  const editing = !!state.editing;
  $("editor").hidden = !editing;
  if (editing) {
    $("detail-empty").hidden = $("detail-item").hidden = $("detail-trash").hidden = true;
    return;
  }
  if (state.category === "trash") {
    const t = state.trash.find((x) => x.id === state.selectedTrashId);
    $("detail-item").hidden = true;
    $("detail-trash").hidden = !t;
    $("detail-empty").hidden = !!t;
    if (t) renderTrashView(t);
    else renderEmpty("削除した項目を選ぶと、元に戻すか完全に削除できます。");
    return;
  }
  $("detail-trash").hidden = true;
  if (!state.selectedKey) {
    state.view = null;
    $("detail-item").hidden = true;
    $("detail-empty").hidden = false;
    renderEmpty("項目を選んでください。");
    return;
  }
  const view = await guarded(() => invoke("view_item", { key: state.selectedKey }));
  if (!view) return;
  state.view = view;
  $("detail-empty").hidden = true;
  $("detail-item").hidden = false;
  renderItemView();
  startOtp();
}

function renderEmpty(message) {
  $("detail-empty").replaceChildren(h("div", {},
    h("p", {}, message),
    h("p", { class: "muted" }, "⌘N 新規　⌘F 検索　⌘C コピー　⌘⇧C ユーザー名をコピー　⌘L ロック")));
}

function summaryOf(key) {
  return state.items.find((i) => i.key === key);
}

function fieldRow(view, f) {
  const revealed = state.revealed.get(f.id);
  const main = h("div", { class: "field-main" }, h("span", { class: "field-label" }, f.label));
  const actionsEl = h("div", { class: "field-actions" });

  if (f.kind === "totp") {
    const code = state.otp.get(f.id);
    main.append(h("div", { class: "otp", id: `otp-${f.id}` },
      h("span", { class: "code" }, code ? L.formatOtp(code.code) : "··· ···"),
      otpRing(code)));
    actionsEl.classList.add("pinned");
    actionsEl.append(iconButton("copy", "コードをコピー", { action: "copy-field", field: f.id }));
  } else if (f.kind === "file") {
    main.append(h("div", { class: "field-value" }, f.filename || "（名前なし）", h("span", { class: "muted" }, `　${L.formatSize(f.size ?? 0)}`)));
    actionsEl.classList.add("pinned");
    actionsEl.append(h("button", { type: "button", class: "btn secondary small", dataset: { action: "save-file", field: f.id }, icon: icon("download", { size: 14 }) + "<span>保存</span>" }));
  } else if (f.secret) {
    const shown = revealed != null;
    main.append(h("div", { class: "field-value " + (shown ? "mono" : "masked") }, shown ? revealed : "••••••••••••"));
    if (f.strength) {
      const labels = { weak: "弱い", fair: "普通", strong: "強い", very_strong: "とても強い" };
      main.append(h("div", { class: `strength ${f.strength}` }, h("span", { class: "bar" }, h("i")), labels[f.strength]));
    }
    actionsEl.append(
      iconButton(shown ? "eye-off" : "eye", shown ? "隠す" : "表示", { action: "reveal-field", field: f.id }),
      iconButton("copy", "コピー", { action: "copy-field", field: f.id }),
    );
  } else if (f.kind === "url") {
    main.append(h("div", { class: "field-value link", dataset: { action: "open-url", url: f.value } }, f.value));
    actionsEl.append(iconButton("copy", "コピー", { action: "copy-field", field: f.id }));
  } else {
    main.append(h("div", { class: "field-value" + (f.kind === "multiline" ? " mono" : "") }, f.value));
    actionsEl.append(iconButton("copy", "コピー", { action: "copy-field", field: f.id }));
  }
  if (f.kind !== "file") actionsEl.append(iconButton("more", "その他", { action: "field-menu", field: f.id }));
  return h("div", { class: "field" }, main, actionsEl);
}

function otpRing(code) {
  const r = 7;
  const circumference = 2 * Math.PI * r;
  const left = code ? code.remaining / code.period : 1;
  const ring = fromHtml(`<svg class="otp-ring" viewBox="0 0 20 20"><circle class="track" cx="10" cy="10" r="${r}"/><circle class="left" cx="10" cy="10" r="${r}" stroke-dasharray="${circumference}" stroke-dashoffset="${circumference * (1 - left)}"/></svg>`);
  if (code && code.remaining <= 5) ring.classList.add("ending");
  return ring;
}

function renderItemView() {
  const view = state.view;
  const summary = summaryOf(view.key);
  const { namespace, name } = L.splitKey(view.key);
  const root = $("detail-item");
  root.replaceChildren();

  root.append(h("header", { class: "item-head" },
    fromHtml(typeBadge(view.item_type, "lg")),
    h("div", { class: "item-titles" }, h("h1", {}, name), h("p", {}, [view.type_label, namespace].filter(Boolean).join(" · "))),
    h("div", { class: "item-actions" },
      iconButton("star", view.favorite ? "お気に入りから外す" : "お気に入りに追加", { action: "toggle-favorite" }, view.favorite ? "on" : ""),
      h("button", { type: "button", class: "btn secondary small", dataset: { action: "edit" }, title: "編集 ⌘E", icon: icon("pencil", { size: 14 }) + "<span>編集</span>" }),
      iconButton("more", "その他", { action: "item-menu" }))));

  if (summary?.weak || summary?.reused) {
    const reasons = [];
    if (summary.weak) reasons.push("このパスワードは弱いです。");
    if (summary.reused) reasons.push("同じパスワードを他の項目でも使っています。");
    root.append(h("div", { class: "notice" }, fromHtml(icon("shield", { size: 16 })), h("span", {}, reasons.join(" "), "編集からパスワードを生成し直せます。")));
  }

  const filled = view.fields.filter((f) => f.has_value);
  if (filled.length) root.append(h("section", { class: "card" }, filled.map((f) => fieldRow(view, f))));

  if (view.notes) {
    root.append(h("section", { class: "card" }, h("div", { class: "field" },
      h("div", { class: "field-main" }, h("span", { class: "field-label" }, "メモ"), h("div", { class: "field-value" }, view.notes)),
      h("div", { class: "field-actions" },
        iconButton("copy", "コピー", { action: "copy-field", field: "notes" }),
        iconButton("reference", "秘密参照をコピー", { action: "copy-reference", ref: view.notes_reference })))));
  }

  if (!filled.length && !view.notes) {
    root.append(h("section", { class: "card" }, h("div", { class: "field" }, h("div", { class: "field-main muted" }, "まだ何も入っていません。「編集」から入力できます。"))));
  }

  if (view.tags.length) {
    root.append(h("div", { class: "tags" }, view.tags.map((t) => h("button", { type: "button", class: "chip", dataset: { action: "select-category", category: `tag:${t}` } }, `#${t}`))));
  }

  root.append(h("div", { class: "meta" },
    h("span", {}, `作成 ${L.formatStamp(view.created_at)}`),
    h("span", {}, `更新 ${L.formatStamp(view.updated_at)}`),
    view.primary_field ? h("span", {}, "参照 ", h("code", {}, `vlt://${view.key}`)) : null));
}

function renderTrashView(t) {
  const { namespace, name } = L.splitKey(t.key);
  $("detail-trash").replaceChildren(
    h("header", { class: "item-head" },
      fromHtml(typeBadge(t.item_type, "lg")),
      h("div", { class: "item-titles" }, h("h1", {}, name), h("p", {}, [t.type_label, namespace].filter(Boolean).join(" · ")))),
    h("section", { class: "card" }, h("div", { class: "field" }, h("div", { class: "field-main" },
      h("span", { class: "field-label" }, "削除日時"), h("div", { class: "field-value" }, L.formatStamp(t.deleted_at)),
      h("p", { class: "muted" }, "削除から 30 日経つと自動で完全に削除されます。")))),
    h("div", { class: "item-actions" },
      h("button", { type: "button", class: "btn primary", dataset: { action: "restore", id: String(t.id) }, icon: icon("restore", { size: 15 }) + "<span>元に戻す</span>" }),
      h("button", { type: "button", class: "btn secondary", dataset: { action: "purge", id: String(t.id) } }, "完全に削除")));
}

// ---------- ワンタイムパスワード ----------

async function fetchOtp(fieldId) {
  if (!state.view) return;
  const key = state.view.key;
  const code = await invoke("totp_code", { key, field: fieldId }).catch(() => null);
  if (!code || state.view?.key !== key) return;
  state.otp.set(fieldId, code);
  paintOtp(fieldId);
}

function paintOtp(fieldId) {
  const el = document.getElementById(`otp-${fieldId}`);
  const code = state.otp.get(fieldId);
  if (!el || !code) return;
  el.replaceChildren(h("span", { class: "code" }, L.formatOtp(code.code)), otpRing(code));
}

function startOtp() {
  const fields = state.view?.fields.filter((f) => f.kind === "totp" && f.has_value) ?? [];
  if (!fields.length) return;
  for (const f of fields) fetchOtp(f.id);
  // 残り秒数は手元で数え、切り替わったときだけ問い合わせる（毎秒の問い合わせを避ける）。
  state.otpTimer = setInterval(() => {
    for (const f of fields) {
      const code = state.otp.get(f.id);
      if (!code) continue;
      if (code.remaining <= 1) fetchOtp(f.id);
      else {
        state.otp.set(f.id, { ...code, remaining: code.remaining - 1 });
        paintOtp(f.id);
      }
    }
  }, 1000);
}

// ---------- 編集 ----------

async function startEdit(originalKey, item) {
  const snapshot = structuredClone({ key: originalKey ?? "", item });
  state.editing = { originalKey, key: originalKey ?? "", item: structuredClone(item), snapshot, shown: new Set() };
  stopOtp();
  await renderDetail();
  renderEditor();
  (originalKey ? $("edit-key") : $("edit-key")).focus();
}

function editorPayload() {
  const e = state.editing;
  return { key: e.key, item: e.item };
}

async function confirmDiscard() {
  if (!state.editing) return true;
  const { snapshot } = state.editing;
  if (!L.isDirty(snapshot, editorPayload())) return true;
  return new Promise((resolve) => {
    openModal({
      title: "編集中の内容を破棄しますか？",
      okLabel: "破棄",
      danger: true,
      build: (body) => { body.append(h("p", { class: "note" }, "保存していない変更は失われます。")); return {}; },
      onSubmit: async () => { resolve(true); return null; },
      onCancel: () => resolve(false),
    });
  });
}

function renderEditor() {
  const e = state.editing;
  const typeLabel = state.types.find((t) => t.id === e.item.item_type)?.label ?? "";
  $("edit-badge").replaceChildren(fromHtml(typeBadge(e.item.item_type, "lg")));
  $("edit-type").textContent = e.originalKey ? `${typeLabel}を編集` : `新しい${typeLabel}`;
  $("edit-key").value = e.key;
  updateRefPreview();
  $("edit-notes").value = e.item.notes;
  $("edit-tags").value = e.item.tags.join(", ");
  $("edit-favorite").checked = e.item.favorite;
  $("edit-error").textContent = "";
  renderEditFields();
}

function updateRefPreview() {
  const key = state.editing?.key;
  $("edit-ref-preview").textContent = key ? `vlt://${key}/<フィールド>` : "vlt://…";
}

// 欄ごとにタイマーを持つ（共有すると、後の欄の描画が前の欄の更新を取り消してしまう）。
const strengthTimers = new WeakMap();
function paintStrength(el, value) {
  clearTimeout(strengthTimers.get(el));
  if (!value) { el.replaceChildren(); return; }
  strengthTimers.set(el, setTimeout(async () => {
    const info = await invoke("password_strength", { password: value }).catch(() => null);
    if (!info) return;
    el.className = `strength ${info.strength}`;
    el.replaceChildren(h("span", { class: "bar" }, h("i")), info.label);
  }, 150));
}

function renderEditFields() {
  const e = state.editing;
  const root = $("edit-fields");
  root.replaceChildren();
  if (!e.item.fields.length) {
    root.append(h("p", { class: "muted", style: "margin:12px 0" }, "フィールドはありません。メモ欄か「フィールドを追加」を使ってください。"));
  }
  e.item.fields.forEach((f, index) => {
    const set = (patch) => Object.assign(e.item.fields[index], patch);
    const label = h("input", { class: "label-input", value: f.label, "aria-label": "フィールド名", oninput: (ev) => set({ label: ev.target.value }) });
    const remove = iconButton("x", "このフィールドを削除", { action: "remove-field", index: String(index) }, "danger");
    const controls = h("div", { class: "controls" });
    const idAttr = `edit-field-${index}`;

    if (f.kind === "file") {
      const name = f.filename || (f.pending_file ? "" : "");
      controls.append(h("div", { class: "file-pick" },
        h("button", { type: "button", class: "btn secondary small", dataset: { action: "pick-file", index: String(index) } }, f.filename ? "別のファイルを選ぶ" : "ファイルを選ぶ"),
        h("span", {}, name || "未選択")));
    } else if (f.kind === "multiline" || f.kind === "secret_block") {
      controls.append(h("textarea", { id: idAttr, rows: f.kind === "secret_block" ? "5" : "3", spellcheck: "false", value: f.value, oninput: (ev) => set({ value: ev.target.value }) }));
    } else {
      const concealed = f.kind === "concealed";
      const shown = e.shown.has(index);
      const type = concealed && !shown ? "password" : f.kind === "email" ? "email" : f.kind === "date" ? "date" : "text";
      const placeholder = f.kind === "totp" ? "otpauth://totp/… または base32 の秘密" : f.kind === "url" ? "https://" : "";
      const strength = h("div", { class: "strength" });
      const input = h("input", {
        id: idAttr, type, value: f.value, placeholder, spellcheck: "false",
        class: concealed || f.kind === "totp" ? "mono" : "",
        oninput: (ev) => { set({ value: ev.target.value }); if (concealed) paintStrength(strength, ev.target.value); },
      });
      controls.append(input);
      if (concealed) {
        controls.append(
          iconButton(shown ? "eye-off" : "eye", shown ? "隠す" : "表示", { action: "toggle-edit-visibility", index: String(index) }),
          iconButton("dice", "パスワードを生成", { action: "generate-into", index: String(index) }));
        paintStrength(strength, f.value);
      }
      const wrap = h("div", { class: "edit-field" }, label, remove, controls);
      if (concealed) wrap.append(h("div", { style: "grid-column:1/-1" }, strength));
      root.append(wrap);
      return;
    }
    root.append(h("div", { class: "edit-field" }, label, remove, controls));
  });
}

async function saveEditor() {
  const e = state.editing;
  if (!e) return;
  e.key = $("edit-key").value;
  e.item.notes = $("edit-notes").value;
  e.item.tags = L.parseTags($("edit-tags").value);
  e.item.favorite = $("edit-favorite").checked;
  const keyError = L.validateKey(e.key);
  if (keyError) { $("edit-error").textContent = keyError; $("edit-key").focus(); return; }
  try {
    await invoke("save_item", { originalKey: e.originalKey, key: e.key, item: e.item });
  } catch (err) {
    $("edit-error").textContent = String(err);
    return;
  }
  const savedKey = e.key;
  state.editing = null;
  if (state.category === "trash") state.category = "all";
  state.selectedKey = savedKey;
  await reloadItems();
  if (!visible().some((i) => i.key === savedKey)) {
    state.category = "all";
    state.query = "";
    $("search").value = "";
    renderCategories();
    renderList();
  }
  toast("保存しました");
}

// ---------- ダイアログ ----------

let modalCancel = null;

function openModal({ title, okLabel = "OK", danger = false, wide = false, hideOk = false, cancelLabel = "キャンセル", build, onSubmit, onCancel }) {
  closePopover();
  const body = $("modal-body");
  body.replaceChildren();
  $("modal-title").textContent = title;
  $("modal-error").textContent = "";
  $("modal-form").className = "modal-card" + (wide ? " wide" : "");
  const ok = $("modal-ok");
  ok.textContent = okLabel;
  ok.className = "btn " + (danger ? "danger" : "primary");
  ok.hidden = hideOk;
  $("modal-cancel").textContent = cancelLabel;
  const refs = build(body) ?? {};
  modalCancel = onCancel ?? null;
  $("modal").hidden = false;
  (body.querySelector("input, textarea, select, .type-choice") ?? ok).focus();
  $("modal-form").onsubmit = async (ev) => {
    ev.preventDefault();
    ok.disabled = true;
    try {
      const error = await onSubmit(refs);
      if (error) $("modal-error").textContent = error;
      else { modalCancel = null; closeModal(); }
    } catch (e) {
      $("modal-error").textContent = String(e);
    } finally {
      ok.disabled = false;
    }
  };
}

function closeModal() {
  if ($("modal").hidden) return;
  $("modal").hidden = true;
  $("modal-form").onsubmit = null;
  const cancel = modalCancel;
  modalCancel = null;
  cancel?.();
}

function labeled(body, labelText, control) {
  body.append(h("label", { class: "field-label" }, labelText), control);
  return control;
}

function typePickerDialog() {
  openModal({
    title: "新しい項目の種類",
    wide: true,
    hideOk: true,
    build: (body) => {
      const grid = h("div", { class: "type-grid" });
      for (const t of state.types) {
        grid.append(h("button", { type: "button", class: "type-choice", onclick: async () => {
          modalCancel = null;
          closeModal();
          const template = await invoke("template_item", { itemType: t.id });
          state.selectedKey = null;
          if (state.category === "trash") state.category = "all";
          renderCategories();
          renderList();
          await startEdit(null, template);
        } }, fromHtml(typeBadge(t.id)), h("span", {}, t.label)));
      }
      body.append(grid);
      return {};
    },
    onSubmit: async () => null,
  });
}

function loadGeneratorOptions() {
  try {
    const saved = JSON.parse(localStorage.getItem("vlt.generator") ?? "null");
    if (saved && typeof saved.length === "number") return saved;
  } catch { /* 保存できない環境でも既定値で動く */ }
  return { length: 20, digits: true, symbols: true, pin: false };
}

function saveGeneratorOptions(options) {
  try { localStorage.setItem("vlt.generator", JSON.stringify(options)); } catch { /* 無視 */ }
}

function generatorDialog(onUse) {
  const options = loadGeneratorOptions();
  let current = "";
  openModal({
    title: "パスワードを生成",
    okLabel: onUse ? "この値を使う" : "コピー",
    cancelLabel: "閉じる",
    build: (body) => {
      const out = h("div", { class: "generator-out" });
      const strength = h("div", { class: "strength" });
      const lenLabel = h("span", { class: "len" }, String(options.length));
      const range = h("input", { type: "range", min: "4", max: "64", value: String(options.length) });
      const digits = h("input", { type: "checkbox", checked: options.digits });
      const symbols = h("input", { type: "checkbox", checked: options.symbols });
      const pin = h("input", { type: "checkbox", checked: options.pin });
      const regenerate = async () => {
        Object.assign(options, { length: Number(range.value), digits: digits.checked, symbols: symbols.checked, pin: pin.checked });
        digits.disabled = symbols.disabled = options.pin;
        lenLabel.textContent = String(options.length);
        saveGeneratorOptions(options);
        const g = await guarded(() => invoke("generate_password", { options }));
        if (!g) return;
        current = g.password;
        out.textContent = g.password;
        strength.className = `strength ${g.strength}`;
        strength.replaceChildren(h("span", { class: "bar" }, h("i")), g.strength_label);
      };
      range.addEventListener("input", regenerate);
      for (const c of [digits, symbols, pin]) c.addEventListener("change", regenerate);
      body.append(out,
        h("div", { class: "gen-row" }, strength, h("span", { class: "spacer" }),
          h("button", { type: "button", class: "btn ghost small", onclick: regenerate, icon: icon("refresh", { size: 14 }) + "<span>作り直す</span>" })),
        h("div", { class: "gen-row" }, h("span", {}, "長さ"), range, lenLabel),
        h("div", { class: "gen-options" },
          h("label", {}, digits, "数字"), h("label", {}, symbols, "記号"), h("label", {}, pin, "数字だけ（PIN）")));
      regenerate();
      return {};
    },
    onSubmit: async () => {
      if (!current) return "生成できていません";
      if (onUse) { onUse(current); return null; }
      const secs = await invoke("copy_secret", { text: current });
      toast(copiedMessage(secs));
      return "";
    },
  });
}

function copiedMessage(secs) {
  return secs > 0 ? `コピーしました（${secs} 秒後に消去）` : "コピーしました";
}

function largeTypeDialog(value, label) {
  openModal({
    title: label,
    wide: true,
    hideOk: true,
    cancelLabel: "閉じる",
    build: (body) => {
      body.append(h("div", { class: "large-type" }, L.charClasses(value).map((c, i) => h("span", { class: c.kind }, c.ch, h("small", {}, String(i + 1))))));
      return {};
    },
    onSubmit: async () => null,
  });
}

function settingsDialog() {
  const s = { ...state.info.settings };
  openModal({
    title: "設定",
    okLabel: "保存",
    wide: true,
    build: (body) => {
      const select = (value, choices, fmt) => h("select", {}, choices.map((c) => h("option", { value: String(c), selected: c === value }, fmt(c))));
      const autoLock = select(s.auto_lock_minutes, [1, 5, 10, 30, 60, 0], (m) => (m ? `${m} 分` : "しない"));
      const clip = select(s.clipboard_clear_secs, [30, 60, 90, 180, 0], (v) => (v ? `${v} 秒後` : "消さない"));
      const check = (v) => h("input", { type: "checkbox", checked: v });
      const screenLock = check(s.lock_on_screen_lock);
      const touchId = check(s.unlock_with_touch_id);
      const shortcut = check(s.global_shortcut);
      const row = (title, desc, control) => h("div", { class: "setting-row" }, h("div", {}, h("div", {}, title), desc ? h("div", { class: "desc" }, desc) : null), control);
      body.append(
        row("自動ロック", "操作がないまま経過したらロックします", autoLock),
        row("画面ロック・スリープでロック", null, screenLock),
        row(state.info.biometrics ? "解錠に Touch ID を使う" : "解錠に本人確認を使う", "オフにするとキーチェーンの許可だけで開きます", touchId),
        row("クリップボードを消去", "コピーした値を自動で消します（その間に別の物をコピーしたら消しません）", clip),
        row("⌘⇧Space で呼び出す", "どのアプリからでも vlt を前面に出して検索できます", shortcut),
        row("バックアップ", "パスフレーズで暗号化した .vltx を書き出し／読み込みします",
          h("span", { style: "display:flex;gap:6px" },
            h("button", { type: "button", class: "btn secondary small", dataset: { action: "export" } }, "書き出す"),
            h("button", { type: "button", class: "btn secondary small", dataset: { action: "import" } }, "読み込む"))),
        row("vault の場所", null, h("code", {}, state.info.db_path)),
        row("バージョン", null, h("span", { class: "muted" }, `vlt ${state.info.version}`)));
      return { autoLock, clip, screenLock, touchId, shortcut };
    },
    onSubmit: async (r) => {
      const next = {
        auto_lock_minutes: Number(r.autoLock.value),
        clipboard_clear_secs: Number(r.clip.value),
        lock_on_screen_lock: r.screenLock.checked,
        unlock_with_touch_id: r.touchId.checked,
        global_shortcut: r.shortcut.checked,
      };
      state.info.settings = await invoke("save_settings", { settings: next });
      toast("設定を保存しました");
      return null;
    },
  });
}

function exportDialog() {
  openModal({
    title: "バックアップを書き出す",
    okLabel: "書き出す",
    build: (body) => {
      body.append(h("p", { class: "note" }, "パスフレーズだけで復元できる .vltx を作ります（キーチェーン不要・別の Mac でも開けます）。パスフレーズを失うと復元できません。"));
      const passphrase = labeled(body, "パスフレーズ", h("input", { type: "password" }));
      const confirm = labeled(body, "パスフレーズ（確認）", h("input", { type: "password" }));
      return { passphrase, confirm };
    },
    onSubmit: async ({ passphrase, confirm }) => {
      const error = L.validatePassphrase({ passphrase: passphrase.value, confirm: confirm.value, requireConfirm: true });
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
      const passphrase = labeled(body, "パスフレーズ", h("input", { type: "password" }));
      const overwrite = h("input", { type: "checkbox" });
      body.append(h("label", { class: "check" }, overwrite, "同じ名前の項目があれば上書きする"));
      return { passphrase, overwrite };
    },
    onSubmit: async ({ passphrase, overwrite }) => {
      const error = L.validatePassphrase({ passphrase: passphrase.value, confirm: "", requireConfirm: false });
      if (error) return error;
      const result = await invoke("import_vault", { passphrase: passphrase.value, overwrite: overwrite.checked });
      if (!result) return "ファイルが選ばれませんでした";
      await reloadItems();
      toast(`${result.imported} 件を取り込みました（${result.skipped} 件はスキップ）`);
      return null;
    },
  });
}

// ---------- ポップオーバー ----------

function openPopover(anchor, entries) {
  const pop = $("popover");
  pop.replaceChildren();
  for (const entry of entries) {
    if (entry === "-") { pop.append(h("hr")); continue; }
    pop.append(h("button", { type: "button", class: entry.danger ? "danger" : "", onclick: () => { closePopover(); entry.run(); }, icon: entry.icon ? icon(entry.icon, { size: 15 }) : "" }, entry.label));
  }
  pop.hidden = false;
  const r = anchor.getBoundingClientRect();
  const w = pop.offsetWidth;
  const hgt = pop.offsetHeight;
  pop.style.left = `${Math.max(8, Math.min(window.innerWidth - w - 8, r.right - w))}px`;
  pop.style.top = `${r.bottom + hgt + 8 > window.innerHeight ? r.top - hgt - 4 : r.bottom + 4}px`;
  pop.querySelector("button")?.focus();
}

function closePopover() {
  $("popover").hidden = true;
}

// ---------- 操作 ----------

async function copyField(fieldId) {
  if (!state.view) return;
  const secs = await guarded(() => invoke("copy_field", { key: state.view.key, field: fieldId }));
  if (secs != null) toast(copiedMessage(secs));
}

async function copyReference(ref) {
  await guarded(() => invoke("copy_plain", { text: ref }));
  toast(`秘密参照をコピーしました: ${ref}`);
}

async function revealField(fieldId) {
  if (state.revealed.has(fieldId)) {
    state.revealed.delete(fieldId);
  } else {
    const value = await guarded(() => invoke("reveal_field", { key: state.view.key, field: fieldId }));
    if (value == null) return;
    state.revealed.set(fieldId, value);
    clearTimeout(state.revealTimer);
    state.revealTimer = setTimeout(concealAll, REVEAL_SECS * 1000);
  }
  renderItemView();
  startOtpRepaint();
}

function startOtpRepaint() {
  for (const id of state.otp.keys()) paintOtp(id);
}

async function moveToTrash(key) {
  openModal({
    title: "ゴミ箱に移しますか？",
    okLabel: "ゴミ箱に移す",
    danger: true,
    build: (body) => { body.append(h("p", { class: "note" }, `${key} をゴミ箱に移します。30 日以内ならゴミ箱から元に戻せます。`)); return {}; },
    onSubmit: async () => {
      const next = L.pickAfterDelete(L.orderedKeys(visible()), key);
      await invoke("delete_item", { key });
      state.selectedKey = next;
      await reloadItems();
      toast("ゴミ箱に移しました");
      return null;
    },
  });
}

async function unlock() {
  $("locked-error").textContent = "";
  const btn = $("btn-unlock");
  btn.disabled = true;
  try {
    await invoke("unlock");
    state.lockReason = "";
    await refreshStatus();
  } catch (e) {
    $("locked-error").textContent = String(e);
  } finally {
    btn.disabled = false;
  }
}

function resetSession() {
  stopOtp();
  concealAll();
  state.view = null;
  state.editing = null;
  state.selectedKey = null;
  state.selectedTrashId = null;
  state.items = [];
  state.trash = [];
  state.query = "";
  $("search").value = "";
  $("toast").hidden = true;
}

// data-action の処理。index.html と動的に作るボタンの data-action はすべてここに定義する
// （logic.test.mjs が「押しても何も起きないボタン」を検出する）。
const actions = {
  init: async () => {
    $("setup-error").textContent = "";
    try { await invoke("init_vault"); await refreshStatus(); } catch (e) { $("setup-error").textContent = String(e); }
  },
  unlock: () => unlock(),
  lock: async () => { await invoke("lock"); },
  "select-category": async (el) => {
    if (state.editing && !(await confirmDiscard())) return;
    state.editing = null;
    state.category = el.dataset.category;
    state.selectedTrashId = null;
    if (state.category === "trash") state.trash = await invoke("list_trash");
    const keys = L.orderedKeys(visible());
    if (!keys.includes(state.selectedKey)) state.selectedKey = null;
    renderCategories();
    renderList();
    await renderDetail();
  },
  "select-item": (el) => selectItem(el.dataset.key),
  "select-trash": async (el) => {
    state.selectedTrashId = Number(el.dataset.id);
    renderList();
    await renderDetail();
  },
  new: async () => {
    if (state.editing && !(await confirmDiscard())) return;
    state.editing = null;
    typePickerDialog();
  },
  edit: async () => {
    if (!state.view) return;
    const item = await guarded(() => invoke("editable_item", { key: state.view.key }));
    if (item) await startEdit(state.view.key, item);
  },
  "edit-cancel": async () => {
    if (!(await confirmDiscard())) return;
    state.editing = null;
    await renderDetail();
  },
  "add-field": (el) => {
    openPopover(el, FIELD_KINDS.map((k) => ({
      label: k.label,
      run: () => {
        state.editing.item.fields.push({ id: "", label: k.label, kind: k.kind, value: "", filename: null, pending_file: null });
        renderEditFields();
        const inputs = $("edit-fields").querySelectorAll(".label-input");
        inputs[inputs.length - 1]?.select();
      },
    })));
  },
  "remove-field": (el) => {
    state.editing.item.fields.splice(Number(el.dataset.index), 1);
    state.editing.shown.clear();
    renderEditFields();
  },
  "toggle-edit-visibility": (el) => {
    const index = Number(el.dataset.index);
    const shown = state.editing.shown;
    if (shown.has(index)) shown.delete(index); else shown.add(index);
    renderEditFields();
    $(`edit-field-${index}`)?.focus();
  },
  "generate-into": (el) => {
    const index = Number(el.dataset.index);
    generatorDialog((password) => {
      state.editing.item.fields[index].value = password;
      state.editing.shown.add(index);
      renderEditFields();
    });
  },
  "pick-file": async (el) => {
    const index = Number(el.dataset.index);
    const picked = await guarded(() => invoke("pick_file"));
    if (!picked) return;
    Object.assign(state.editing.item.fields[index], { pending_file: picked.token, filename: picked.filename });
    renderEditFields();
  },
  "reveal-field": (el) => revealField(el.dataset.field),
  "copy-field": (el) => copyField(el.dataset.field),
  "copy-reference": (el) => copyReference(el.dataset.ref),
  "open-url": (el) => guarded(() => invoke("open_url", { url: el.dataset.url })),
  "save-file": async (el) => {
    const path = await guarded(() => invoke("save_field_file", { key: state.view.key, field: el.dataset.field }));
    if (path) toast(`保存しました: ${path}`);
  },
  "field-menu": (el) => {
    const f = state.view.fields.find((x) => x.id === el.dataset.field);
    if (!f) return;
    const entries = [];
    if (f.secret && f.kind !== "totp") {
      entries.push({ label: "大きく表示", icon: "expand", run: async () => {
        const value = state.revealed.get(f.id) ?? await guarded(() => invoke("reveal_field", { key: state.view.key, field: f.id }));
        if (value != null) largeTypeDialog(value, f.label);
      } });
    } else if (f.value) {
      entries.push({ label: "大きく表示", icon: "expand", run: () => largeTypeDialog(f.value, f.label) });
    }
    entries.push({ label: "秘密参照をコピー", icon: "reference", run: () => copyReference(f.reference) });
    if (f.kind === "totp") {
      entries.push({ label: "コード用の秘密参照をコピー", icon: "reference", run: () => copyReference(`${f.reference}?attribute=otp`) });
    }
    openPopover(el, entries);
  },
  "toggle-favorite": async () => {
    const view = state.view;
    await guarded(() => invoke("set_favorite", { key: view.key, favorite: !view.favorite }));
    await reloadItems();
  },
  "item-menu": (el) => {
    const view = state.view;
    const entries = [];
    if (view.primary_field) entries.push({ label: "秘密参照をコピー", icon: "reference", run: () => copyReference(`vlt://${view.key}`) });
    entries.push({ label: "複製", icon: "copy", run: async () => {
      const key = await guarded(() => invoke("duplicate_item", { key: view.key }));
      if (!key) return;
      state.selectedKey = key;
      await reloadItems();
      toast(`${key} を作りました`);
    } });
    entries.push("-", { label: "ゴミ箱に移す", icon: "trash", danger: true, run: () => moveToTrash(view.key) });
    openPopover(el, entries);
  },
  restore: async (el) => {
    const key = await guarded(() => invoke("restore_item", { id: Number(el.dataset.id) }));
    if (!key) return;
    state.category = "all";
    state.selectedTrashId = null;
    state.selectedKey = key;
    await reloadItems();
    toast(`${key} を元に戻しました`);
  },
  purge: (el) => {
    const id = Number(el.dataset.id);
    openModal({
      title: "完全に削除しますか？",
      okLabel: "完全に削除",
      danger: true,
      build: (body) => { body.append(h("p", { class: "note warn" }, "元に戻せません。")); return {}; },
      onSubmit: async () => {
        await invoke("purge_trash_item", { id });
        state.selectedTrashId = null;
        state.trash = await invoke("list_trash");
        renderList();
        await renderDetail();
        return null;
      },
    });
  },
  "empty-trash": () => {
    openModal({
      title: "ゴミ箱を空にしますか？",
      okLabel: "空にする",
      danger: true,
      build: (body) => { body.append(h("p", { class: "note warn" }, `${state.trash.length} 件を完全に削除します。元に戻せません。`)); return {}; },
      onSubmit: async () => {
        await invoke("empty_trash");
        state.selectedTrashId = null;
        state.trash = [];
        renderList();
        await renderDetail();
        return null;
      },
    });
  },
  "open-generator": () => generatorDialog(null),
  "open-settings": () => settingsDialog(),
  export: () => exportDialog(),
  import: () => importDialog(),
  "modal-cancel": () => closeModal(),
};

// ---------- イベント ----------

document.addEventListener("click", (ev) => {
  const target = ev.target.closest("[data-action]");
  if (!ev.target.closest("#popover") && !ev.target.closest('[data-action="add-field"], [data-action="field-menu"], [data-action="item-menu"]')) closePopover();
  if (!target || target.disabled) return;
  const handler = actions[target.dataset.action];
  if (handler) {
    ev.preventDefault();
    Promise.resolve(handler(target, ev)).catch((e) => toast(String(e), true));
  }
});

$("editor").addEventListener("submit", (ev) => { ev.preventDefault(); saveEditor(); });
$("edit-key").addEventListener("input", (ev) => { state.editing.key = ev.target.value; updateRefPreview(); });
$("edit-notes").addEventListener("input", (ev) => { state.editing.item.notes = ev.target.value; });
$("edit-tags").addEventListener("input", (ev) => { state.editing.item.tags = L.parseTags(ev.target.value); });
$("edit-favorite").addEventListener("change", (ev) => { state.editing.item.favorite = ev.target.checked; });

$("search").addEventListener("input", (ev) => {
  state.query = ev.target.value;
  renderList();
});

function inTextInput(el) {
  return el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
}

document.addEventListener("keydown", (ev) => {
  const meta = ev.metaKey || ev.ctrlKey;
  const modalOpen = !$("modal").hidden;
  if (ev.key === "Escape") {
    if (!$("popover").hidden) { closePopover(); return; }
    if (modalOpen) { closeModal(); return; }
    if (state.editing) { actions["edit-cancel"](); return; }
    if (document.activeElement === $("search") && state.query) {
      $("search").value = state.query = "";
      renderList();
    }
    return;
  }
  if (modalOpen || $("screen-main").hidden) return;
  const key = ev.key.toLowerCase();
  if (state.editing) {
    if (meta && key === "s") { ev.preventDefault(); saveEditor(); }
    return;
  }
  if (meta && key === "f") { ev.preventDefault(); $("search").focus(); $("search").select(); return; }
  if (meta && key === "n") { ev.preventDefault(); actions.new(); return; }
  if (meta && key === "l") { ev.preventDefault(); actions.lock(); return; }
  if (meta && key === "e" && state.view) { ev.preventDefault(); actions.edit(); return; }
  if (meta && key === "backspace" && state.view) { ev.preventDefault(); moveToTrash(state.view.key); return; }
  if (meta && key === "c" && state.view && !window.getSelection()?.toString() && !inTextInput(document.activeElement)) {
    const field = L.shortcutCopyField(state.view, ev.shiftKey);
    if (field) { ev.preventDefault(); copyField(field); }
    return;
  }
  const inSearch = document.activeElement === $("search");
  if ((ev.key === "ArrowDown" || ev.key === "ArrowUp") && (inSearch || !inTextInput(document.activeElement))) {
    ev.preventDefault();
    if (state.category === "trash") return;
    selectItem(L.stepSelection(L.orderedKeys(visible()), state.selectedKey, ev.key === "ArrowDown" ? 1 : -1));
    return;
  }
  if (ev.key === "Enter" && inSearch) {
    const first = L.orderedKeys(visible())[0];
    if (first) { ev.preventDefault(); selectItem(first); $("search").blur(); }
  }
});

// 画面での操作を自動ロックのタイマーへ（間引いて）伝える。
for (const type of ["keydown", "mousedown", "wheel"]) {
  document.addEventListener(type, () => {
    if ($("screen-main").hidden) return;
    const now = Date.now();
    if (now - state.lastTouch < TOUCH_THROTTLE_MS) return;
    state.lastTouch = now;
    invoke("touch").catch(() => {});
  }, { passive: true });
}

window.addEventListener("blur", concealAll);

listen("vault-locked", async (event) => {
  state.lockReason = event.payload ?? "";
  resetSession();
  await refreshStatus();
});

listen("quick-open", () => {
  if (!$("screen-locked").hidden) { unlock(); return; }
  if (!$("screen-main").hidden && !state.editing && $("modal").hidden) {
    $("search").focus();
    $("search").select();
  }
});

// 固定アイコンを差し込む
$("setup-mark").innerHTML = icon("lock", { size: 38, strokeWidth: 1.5 });
$("locked-mark").innerHTML = icon("lock", { size: 38, strokeWidth: 1.5 });
$("search-icon").replaceChildren(fromHtml('<svg class="ico" viewBox="0 0 20 20" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="8.5" cy="8.5" r="5.5"/><path d="M13 13l4 4"/></svg>'));
$("btn-generator").innerHTML = icon("dice");
$("btn-settings").innerHTML = icon("gear");
$("btn-lock").innerHTML = icon("lock");
$("btn-new").prepend(fromHtml(icon("plus", { size: 14, strokeWidth: 2 })));
$("btn-add-field").prepend(fromHtml(icon("plus", { size: 14, strokeWidth: 2 })));

refreshStatus().catch((e) => {
  showScreen("locked");
  $("locked-error").textContent = String(e);
});
