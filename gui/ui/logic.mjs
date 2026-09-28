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

/** "openai/api-key" → { namespace: "openai", name: "api-key" }。区切りが無ければ namespace は ""。 */
export function splitKey(key) {
  const idx = key.lastIndexOf("/");
  if (idx < 0) return { namespace: "", name: key };
  return { namespace: key.slice(0, idx), name: key.slice(idx + 1) };
}

/** 空白区切りの語を全て含む項目だけ残す（大文字小文字は無視）。 */
export function filterItems(items, query) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return items;
  return items.filter((item) => {
    const haystack = item.key.toLowerCase();
    return words.every((w) => haystack.includes(w));
  });
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

/** 削除後に選ぶ項目。同じ位置→前→なし。 */
export function pickAfterDelete(orderedKeys, deletedKey) {
  const idx = orderedKeys.indexOf(deletedKey);
  const remaining = orderedKeys.filter((k) => k !== deletedKey);
  if (remaining.length === 0) return null;
  return remaining[Math.min(idx < 0 ? 0 : idx, remaining.length - 1)];
}

/** ↑↓ キーでの移動先。端では止まる。未選択なら先頭。 */
export function stepSelection(orderedKeys, currentKey, delta) {
  if (orderedKeys.length === 0) return null;
  const idx = orderedKeys.indexOf(currentKey);
  if (idx < 0) return orderedKeys[0];
  const next = Math.max(0, Math.min(orderedKeys.length - 1, idx + delta));
  return orderedKeys[next];
}

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

/** 新規作成フォームの検証。エラー文字列 or null。 */
export function validateNewItem({ key, mode, value }) {
  if (!key.trim()) return "キーを入力してください";
  if (key !== key.trim()) return "キーの前後に空白は使えません";
  if (mode === "text" && value.length === 0) return "値を入力してください";
  return null;
}

/** パスフレーズ入力の検証（書き出し時は確認入力と一致が必要）。 */
export function validatePassphrase({ passphrase, confirm, requireConfirm }) {
  if (!passphrase) return "パスフレーズを入力してください";
  if (requireConfirm && passphrase !== confirm) return "パスフレーズが一致しません";
  return null;
}
