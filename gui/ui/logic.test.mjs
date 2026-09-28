import { test } from "node:test";
import assert from "node:assert/strict";
import {
  resolveScreen,
  primaryAction,
  splitKey,
  filterItems,
  groupItems,
  pickAfterDelete,
  stepSelection,
  formatStamp,
  validateNewItem,
  validatePassphrase,
} from "./logic.mjs";

test("初期化前は setup、初期化済み未解錠は locked、解錠済みは main", () => {
  assert.equal(resolveScreen({ initialized: false, unlocked: false }), "setup");
  assert.equal(resolveScreen({ initialized: true, unlocked: false }), "locked");
  assert.equal(resolveScreen({ initialized: true, unlocked: true }), "main");
});

test("main 以外の全画面に『次へ進む操作』がある（ソフトロック禁止）", () => {
  for (const initialized of [false, true]) {
    for (const unlocked of [false, true]) {
      const screen = resolveScreen({ initialized, unlocked });
      if (screen === "main") continue;
      assert.ok(primaryAction(screen), `${screen} に primaryAction が無い`);
    }
  }
});

test("index.html の各画面に primaryAction と同名の data-action ボタンがある", async () => {
  const fs = await import("node:fs/promises");
  const html = await fs.readFile(new URL("./index.html", import.meta.url), "utf8");
  for (const screen of ["setup", "locked"]) {
    const action = primaryAction(screen);
    const sectionMatch = html.match(new RegExp(`<section[^>]*id="screen-${screen}"[\\s\\S]*?</section>`));
    assert.ok(sectionMatch, `screen-${screen} セクションが無い`);
    assert.ok(sectionMatch[0].includes(`data-action="${action}"`), `${screen} に data-action="${action}" が無い`);
  }
  // app.js 側でその action を invoke に配線していること
  const js = await fs.readFile(new URL("./app.js", import.meta.url), "utf8");
  for (const action of ["init", "unlock"]) {
    assert.ok(js.includes(`case "${action}"`), `app.js が ${action} を処理していない`);
  }
});

test("splitKey は最後の / で namespace と name に分ける", () => {
  assert.deepEqual(splitKey("openai/api-key"), { namespace: "openai", name: "api-key" });
  assert.deepEqual(splitKey("android/hushcam/keystore"), { namespace: "android/hushcam", name: "keystore" });
  assert.deepEqual(splitKey("token"), { namespace: "", name: "token" });
});

test("filterItems は語をすべて含む項目だけ残し、大文字小文字を無視する", () => {
  const items = [{ key: "openai/api-key" }, { key: "notion/integration-token" }, { key: "android/keystore" }];
  assert.deepEqual(filterItems(items, "").map((i) => i.key), items.map((i) => i.key));
  assert.deepEqual(filterItems(items, "KEY").map((i) => i.key), ["openai/api-key", "android/keystore"]);
  assert.deepEqual(filterItems(items, "and key").map((i) => i.key), ["android/keystore"]);
  assert.deepEqual(filterItems(items, "zzz"), []);
});

test("groupItems は namespace ごとにまとめてソートする", () => {
  const items = [{ key: "b/2" }, { key: "root" }, { key: "a/1" }, { key: "b/1" }];
  const groups = groupItems(items);
  assert.deepEqual(groups.map((g) => g.namespace), ["", "a", "b"]);
  assert.deepEqual(groups[2].items.map((i) => i.key), ["b/1", "b/2"]);
});

test("pickAfterDelete は同じ位置→前→null の順で選ぶ", () => {
  assert.equal(pickAfterDelete(["a", "b", "c"], "b"), "c");
  assert.equal(pickAfterDelete(["a", "b", "c"], "c"), "b");
  assert.equal(pickAfterDelete(["a"], "a"), null);
});

test("stepSelection は端で止まり未選択なら先頭", () => {
  assert.equal(stepSelection(["a", "b"], null, 1), "a");
  assert.equal(stepSelection(["a", "b"], "a", 1), "b");
  assert.equal(stepSelection(["a", "b"], "b", 1), "b");
  assert.equal(stepSelection(["a", "b"], "a", -1), "a");
  assert.equal(stepSelection([], null, 1), null);
});

test("formatStamp は UTC をローカルに直し、今日なら『今日』を付ける", () => {
  const now = new Date(2026, 8, 29, 12, 0, 0);
  const todayUtc = new Date(Date.UTC(2026, 8, 29, 1, 5, 0));
  const pad = (n) => String(n).padStart(2, "0");
  const asSqlite = `${todayUtc.getUTCFullYear()}-${pad(todayUtc.getUTCMonth() + 1)}-${pad(todayUtc.getUTCDate())} 01:05:00`;
  const expectedHm = `${pad(todayUtc.getHours())}:${pad(todayUtc.getMinutes())}`;
  const shown = formatStamp(asSqlite, now);
  if (todayUtc.getDate() === now.getDate()) {
    assert.equal(shown, `今日 ${expectedHm}`);
  } else {
    assert.ok(shown.endsWith(expectedHm));
  }
  assert.equal(formatStamp("2026-07-18 01:10:02", now).startsWith("2026-07-18"), true);
  assert.equal(formatStamp("garbage", now), "garbage");
});

test("validateNewItem はキーと値の欠落を拒む", () => {
  assert.equal(validateNewItem({ key: "", mode: "text", value: "v" }), "キーを入力してください");
  assert.equal(validateNewItem({ key: " k", mode: "text", value: "v" }), "キーの前後に空白は使えません");
  assert.equal(validateNewItem({ key: "k", mode: "text", value: "" }), "値を入力してください");
  assert.equal(validateNewItem({ key: "k", mode: "file", value: "" }), null);
  assert.equal(validateNewItem({ key: "k", mode: "text", value: "v" }), null);
});

test("validatePassphrase は空と不一致を拒む", () => {
  assert.equal(validatePassphrase({ passphrase: "", confirm: "", requireConfirm: true }), "パスフレーズを入力してください");
  assert.equal(validatePassphrase({ passphrase: "a", confirm: "b", requireConfirm: true }), "パスフレーズが一致しません");
  assert.equal(validatePassphrase({ passphrase: "a", confirm: "b", requireConfirm: false }), null);
  assert.equal(validatePassphrase({ passphrase: "a", confirm: "a", requireConfirm: true }), null);
});
