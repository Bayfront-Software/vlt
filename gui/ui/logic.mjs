// 画面の判断ロジック（DOM も Tauri も触らない純関数）。
// app.js はここで決めた結果を描画するだけにして、遷移の抜けを node --test で検出する。

/** 起動時の状態から表示する画面を決める。 */
export function resolveScreen(status) {
  if (!status.initialized) return "setup";
  if (!status.unlocked) return "locked";
  return "main";
}

/**
 * 入力待ち画面で「次へ進む操作」を返す。main 以外で null が返ったらソフトロック。
 * ボタンの data-action と一致させ、配線抜けをテストで捕まえる。
 */
export function primaryAction(screen) {
  switch (screen) {
    case "setup":
      return "init";
    case "locked":
      return "unlock";
    case "main":
      return null;
    default:
      throw new Error(`unknown screen: ${screen}`);
  }
}

/** ロックの理由を画面の文にする。 */
export function lockReasonMessage(reason, autoLockMinutes) {
  switch (reason) {
    case "idle":
      return `${autoLockMinutes} 分間操作がなかったのでロックしました`;
    case "screen-locked":
      return "画面がロックされたのでロックしました";
    case "sleep":
      return "スリープしたのでロックしました";
    default:
      return "";
  }
}

/** "openai/api-key" → { namespace: "openai", name: "api-key" }。区切りが無ければ namespace は ""。 */
export function splitKey(key) {
  const idx = key.lastIndexOf("/");
  if (idx < 0) return { namespace: "", name: key };
  return { namespace: key.slice(0, idx), name: key.slice(idx + 1) };
}

// ---------- サイドバーの分類 ----------

/** 分類は "all" | "favorites" | "watchtower" | "trash" | "type:<id>" | "tag:<name>"。 */
export function inCategory(item, category) {
  if (category === "all") return true;
  if (category === "favorites") return item.favorite;
  if (category === "watchtower") return item.weak || item.reused;
  if (category.startsWith("type:")) return item.item_type === category.slice(5);
  if (category.startsWith("tag:")) return item.tags.includes(category.slice(4));
  return false;
}

/** サイドバーに出す件数。種類とタグは 1 件以上あるものだけ。 */
export function sidebarCounts(items, types) {
  const typeCounts = types
    .map((t) => ({ ...t, count: items.filter((i) => i.item_type === t.id).length }))
    .filter((t) => t.count > 0);
  const tagMap = new Map();
  for (const item of items) {
    for (const tag of item.tags) tagMap.set(tag, (tagMap.get(tag) ?? 0) + 1);
  }
  const tags = [...tagMap.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, count]) => ({ name, count }));
  return {
    all: items.length,
    favorites: items.filter((i) => i.favorite).length,
    watchtower: items.filter((i) => i.weak || i.reused).length,
    types: typeCounts,
    tags,
  };
}

/** 空白区切りの語を全て含む項目だけ残す（キー・副題・タグ・種類名を対象、大文字小文字は無視）。 */
export function filterItems(items, query) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return items;
  return items.filter((item) => {
    const haystack = [item.key, item.subtitle ?? "", item.type_label ?? "", ...(item.tags ?? [])]
      .join(" ")
      .toLowerCase();
    return words.every((w) => haystack.includes(w));
  });
}

/** 分類と検索を両方かけた、画面に出す順の項目。 */
export function visibleItems(items, category, query) {
  return filterItems(items.filter((i) => inCategory(i, category)), query);
}

/** namespace ごとにまとめ、namespace 名でソートする。ルート直下は先頭。 */
export function groupItems(items) {
  const groups = new Map();
  for (const item of items) {
    const { namespace } = splitKey(item.key);
    if (!groups.has(namespace)) groups.set(namespace, []);
    groups.get(namespace).push(item);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([namespace, list]) => ({
      namespace,
      items: [...list].sort((a, b) => a.key.localeCompare(b.key)),
    }));
}

/** 一覧の表示順（グループ化した順）でのキー列。↑↓ 移動に使う。 */
export function orderedKeys(items) {
  return groupItems(items).flatMap((g) => g.items.map((i) => i.key));
}

/** 削除後に選ぶ項目。同じ位置→前→なし。 */
export function pickAfterDelete(keys, deletedKey) {
  const idx = keys.indexOf(deletedKey);
  const remaining = keys.filter((k) => k !== deletedKey);
  if (remaining.length === 0) return null;
  return remaining[Math.min(idx < 0 ? 0 : idx, remaining.length - 1)];
}

/** ↑↓ キーでの移動先。端では止まる。未選択なら先頭。 */
export function stepSelection(keys, currentKey, delta) {
  if (keys.length === 0) return null;
  const idx = keys.indexOf(currentKey);
  if (idx < 0) return keys[0];
  return keys[Math.max(0, Math.min(keys.length - 1, idx + delta))];
}

// ---------- 表示の整形 ----------

/** SQLite の datetime('now')（UTC, "YYYY-MM-DD HH:MM:SS"）をローカル時刻の表示文字列にする。 */
export function formatStamp(sqliteUtc, now = new Date()) {
  const date = new Date(sqliteUtc.replace(" ", "T") + "Z");
  if (Number.isNaN(date.getTime())) return sqliteUtc;
  const pad = (n) => String(n).padStart(2, "0");
  const ymd = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  const hm = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  return sameDay ? `今日 ${hm}` : `${ymd} ${hm}`;
}

export function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** ワンタイムパスワードを 3 桁ずつ区切る（"123456" → "123 456"）。 */
export function formatOtp(code) {
  if (code.length <= 4) return code;
  const half = Math.ceil(code.length / 2);
  return `${code.slice(0, half)} ${code.slice(half)}`;
}

/** 「大きく表示」の文字ごとの種別（色分け用）。 */
export function charClasses(value) {
  return [...value].map((ch) => ({
    ch,
    kind: /[0-9]/.test(ch) ? "digit" : /[A-Za-z]/.test(ch) ? "letter" : "symbol",
  }));
}

// ---------- 入力の検証 ----------

/** Rust 側 ops::validate_key と同じ規則（先に画面で弾いて往復を減らす）。 */
export function validateKey(key) {
  if (!key.trim()) return "名前（キー）を入力してください";
  if (key !== key.trim()) return "名前の前後に空白は使えません";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(key)) return "名前に制御文字は使えません";
  if (key.startsWith("/") || key.endsWith("/") || key.includes("//")) {
    return "名前の / は区切りにだけ使えます（先頭・末尾・連続は不可）";
  }
  if (key.includes("?")) return "名前に ? は使えません";
  return null;
}

/** "work, dev  ,Work" → ["work", "dev"]（空と大文字小文字違いの重複を除く）。 */
export function parseTags(text) {
  const seen = new Set();
  const out = [];
  for (const raw of text.split(/[,、]/)) {
    const tag = raw.trim();
    if (!tag || seen.has(tag.toLowerCase())) continue;
    seen.add(tag.toLowerCase());
    out.push(tag);
  }
  return out;
}

/** パスフレーズ入力の検証（書き出し時は確認入力と一致が必要）。 */
export function validatePassphrase({ passphrase, confirm, requireConfirm }) {
  if (!passphrase) return "パスフレーズを入力してください";
  if (requireConfirm && passphrase !== confirm) return "パスフレーズが一致しません";
  return null;
}

/** 編集中の内容が保存済みと違うか（キャンセル時の確認用）。 */
export function isDirty(original, current) {
  return JSON.stringify(original) !== JSON.stringify(current);
}

/** ⌘C / ⌘⇧C でコピーするフィールド。⌘C は主役、⌘⇧C はユーザー名など伏せない代表値。 */
export function shortcutCopyField(view, shift) {
  if (!view) return null;
  if (shift) {
    const candidates = ["username", "email", "ssid", "cardholder"];
    const hit = view.fields.find((f) => candidates.includes(f.id) && f.has_value);
    return hit ? hit.id : null;
  }
  if (view.primary_field && view.fields.some((f) => f.id === view.primary_field && f.has_value && f.kind !== "file")) {
    return view.primary_field;
  }
  return null;
}
