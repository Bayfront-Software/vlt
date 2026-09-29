import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import * as L from "./logic.mjs";

const html = await readFile(new URL("./index.html", import.meta.url), "utf8");
const js = await readFile(new URL("./app.js", import.meta.url), "utf8");

test("初期化前は setup、初期化済み未解錠は locked、解錠済みは main", () => {
  assert.equal(L.resolveScreen({ initialized: false, unlocked: false }), "setup");
  assert.equal(L.resolveScreen({ initialized: true, unlocked: false }), "locked");
  assert.equal(L.resolveScreen({ initialized: true, unlocked: true }), "main");
});

test("main 以外の全画面に『次へ進む操作』があり、HTML と app.js に配線されている（ソフトロック禁止）", () => {
  for (const initialized of [false, true]) {
    for (const unlocked of [false, true]) {
      const screen = L.resolveScreen({ initialized, unlocked });
      if (screen === "main") continue;
      const action = L.primaryAction(screen);
      assert.ok(action, `${screen} に primaryAction が無い`);
      const section = html.match(new RegExp(`<section[^>]*id="screen-${screen}"[\\s\\S]*?</section>`));
      assert.ok(section, `screen-${screen} が無い`);
      assert.ok(section[0].includes(`data-action="${action}"`), `${screen} に data-action="${action}" が無い`);
      assert.ok(js.includes(`${action}:`), `app.js の actions に ${action} が無い`);
    }
  }
});

test("入力待ちの重ね画面（ダイアログ・編集）には必ず抜ける操作がある", () => {
  const modal = html.match(/<div id="modal"[\s\S]*?<\/form>/);
  assert.ok(modal && modal[0].includes('data-action="modal-cancel"'), "ダイアログにキャンセルが無い");
  const editor = html.match(/<form id="editor"[\s\S]*?<\/form>/);
  assert.ok(editor, "編集フォームが無い");
  assert.ok(editor[0].includes('data-action="edit-cancel"') && editor[0].includes('type="submit"'), "編集に保存/キャンセルが無い");
  for (const action of ["modal-cancel", "edit-cancel"]) {
    assert.ok(js.includes(`"${action}":`), `app.js が ${action} を処理していない`);
  }
  assert.ok(/key === "Escape"/.test(js), "Esc で閉じられない");
});

test("HTML の data-action はすべて app.js の actions に定義がある（押しても何も起きないボタンを作らない）", () => {
  const actions = [...html.matchAll(/data-action="([^"]+)"/g)].map((m) => m[1]);
  const dynamic = [...js.matchAll(/dataset: \{ action: "([^"]+)"/g)].map((m) => m[1]);
  for (const action of new Set([...actions, ...dynamic])) {
    const defined = js.includes(`"${action}":`) || js.includes(`${action}:`);
    assert.ok(defined, `data-action="${action}" に対応する処理が無い`);
  }
});

test("lockReasonMessage は理由ごとの文を返す", () => {
  assert.match(L.lockReasonMessage("idle", 10), /10 分/);
  assert.match(L.lockReasonMessage("screen-locked", 10), /画面/);
  assert.equal(L.lockReasonMessage("manual", 10), "");
});

const items = [
  { key: "dev/github", item_type: "login", type_label: "ログイン", subtitle: "alice", favorite: true, tags: ["work"], weak: false, reused: true },
  { key: "dev/aws", item_type: "api_credential", type_label: "API 認証情報", subtitle: "ci", favorite: false, tags: ["work", "ci"], weak: false, reused: false },
  { key: "home/wifi", item_type: "wifi", type_label: "Wi-Fi", subtitle: "MyNet", favorite: false, tags: [], weak: true, reused: false },
  { key: "token", item_type: "password", type_label: "パスワード", subtitle: "パスワード", favorite: false, tags: [], weak: false, reused: false },
];

test("inCategory は各分類を正しく判定する", () => {
  const keys = (c) => items.filter((i) => L.inCategory(i, c)).map((i) => i.key);
  assert.equal(keys("all").length, 4);
  assert.deepEqual(keys("favorites"), ["dev/github"]);
  assert.deepEqual(keys("watchtower"), ["dev/github", "home/wifi"]);
  assert.deepEqual(keys("type:wifi"), ["home/wifi"]);
  assert.deepEqual(keys("tag:ci"), ["dev/aws"]);
  assert.deepEqual(keys("trash"), []);
});

test("sidebarCounts は 0 件の種類を出さずタグを名前順に数える", () => {
  const types = [{ id: "login", label: "ログイン" }, { id: "ssh_key", label: "SSH 鍵" }, { id: "wifi", label: "Wi-Fi" }];
  const c = L.sidebarCounts(items, types);
  assert.equal(c.all, 4);
  assert.equal(c.favorites, 1);
  assert.equal(c.watchtower, 2);
  assert.deepEqual(c.types.map((t) => t.id), ["login", "wifi"]);
  assert.deepEqual(c.tags, [{ name: "ci", count: 1 }, { name: "work", count: 2 }]);
});

test("filterItems はキー・副題・タグ・種類名を対象に全語一致で絞る", () => {
  const keys = (q) => L.filterItems(items, q).map((i) => i.key);
  assert.equal(keys("").length, 4);
  assert.deepEqual(keys("ALICE"), ["dev/github"]);
  assert.deepEqual(keys("work ci"), ["dev/aws"]);
  assert.deepEqual(keys("wi-fi"), ["home/wifi"]);
  assert.deepEqual(keys("zzz"), []);
});

test("visibleItems は分類と検索を両方かける", () => {
  assert.deepEqual(L.visibleItems(items, "tag:work", "aws").map((i) => i.key), ["dev/aws"]);
});

test("groupItems と orderedKeys は namespace 順・キー順に並べる", () => {
  const groups = L.groupItems(items);
  assert.deepEqual(groups.map((g) => g.namespace), ["", "dev", "home"]);
  assert.deepEqual(L.orderedKeys(items), ["token", "dev/aws", "dev/github", "home/wifi"]);
});

test("splitKey は最後の / で namespace と name に分ける", () => {
  assert.deepEqual(L.splitKey("android/hushcam/keystore"), { namespace: "android/hushcam", name: "keystore" });
  assert.deepEqual(L.splitKey("token"), { namespace: "", name: "token" });
});

test("pickAfterDelete と stepSelection", () => {
  assert.equal(L.pickAfterDelete(["a", "b", "c"], "b"), "c");
  assert.equal(L.pickAfterDelete(["a", "b", "c"], "c"), "b");
  assert.equal(L.pickAfterDelete(["a"], "a"), null);
  assert.equal(L.stepSelection(["a", "b"], null, 1), "a");
  assert.equal(L.stepSelection(["a", "b"], "b", 1), "b");
  assert.equal(L.stepSelection(["a", "b"], "b", -1), "a");
  assert.equal(L.stepSelection([], null, 1), null);
});

test("formatStamp は UTC をローカルに直し、今日なら『今日』を付ける", () => {
  const now = new Date(2026, 8, 29, 12, 0, 0);
  const noonUtc = new Date(Date.UTC(now.getFullYear(), now.getMonth(), now.getDate(), 3, 0, 0));
  const pad = (n) => String(n).padStart(2, "0");
  const sqlite = `${noonUtc.getUTCFullYear()}-${pad(noonUtc.getUTCMonth() + 1)}-${pad(noonUtc.getUTCDate())} 03:00:00`;
  const shown = L.formatStamp(sqlite, now);
  assert.ok(shown.endsWith(`${pad(noonUtc.getHours())}:00`), shown);
  assert.ok(L.formatStamp("2026-07-18 01:10:02", now).startsWith("2026-07-18"));
  assert.equal(L.formatStamp("garbage", now), "garbage");
});

test("formatSize・formatOtp・charClasses", () => {
  assert.equal(L.formatSize(512), "512 B");
  assert.equal(L.formatSize(2048), "2.0 KB");
  assert.equal(L.formatSize(3 * 1024 * 1024), "3.0 MB");
  assert.equal(L.formatOtp("123456"), "123 456");
  assert.equal(L.formatOtp("12345678"), "1234 5678");
  assert.deepEqual(L.charClasses("a1!").map((c) => c.kind), ["letter", "digit", "symbol"]);
});

test("validateKey は Rust 側と同じ規則で弾く", () => {
  for (const bad of ["", " a", "a ", "/a", "a/", "a//b", "a?b", "a\nb"]) {
    assert.ok(L.validateKey(bad), JSON.stringify(bad));
  }
  for (const good of ["a", "openai/api-key", "日本語/キー"]) {
    assert.equal(L.validateKey(good), null, good);
  }
});

test("parseTags は読点区切りも受け、重複と空を除く", () => {
  assert.deepEqual(L.parseTags("work, dev ,Work、個人,, "), ["work", "dev", "個人"]);
  assert.deepEqual(L.parseTags(""), []);
});

test("validatePassphrase と isDirty", () => {
  assert.ok(L.validatePassphrase({ passphrase: "", confirm: "", requireConfirm: true }));
  assert.ok(L.validatePassphrase({ passphrase: "a", confirm: "b", requireConfirm: true }));
  assert.equal(L.validatePassphrase({ passphrase: "a", confirm: "b", requireConfirm: false }), null);
  assert.equal(L.isDirty({ a: 1 }, { a: 1 }), false);
  assert.equal(L.isDirty({ a: 1 }, { a: 2 }), true);
});

test("shortcutCopyField は ⌘C で主役、⌘⇧C でユーザー名を選ぶ", () => {
  const view = {
    primary_field: "password",
    fields: [
      { id: "username", has_value: true, kind: "text" },
      { id: "password", has_value: true, kind: "concealed" },
    ],
  };
  assert.equal(L.shortcutCopyField(view, false), "password");
  assert.equal(L.shortcutCopyField(view, true), "username");
  assert.equal(L.shortcutCopyField({ primary_field: "file", fields: [{ id: "file", has_value: true, kind: "file" }] }, false), null);
  assert.equal(L.shortcutCopyField(null, false), null);
});

test("clampPaneWidths は各列を上下限に収め、詳細欄の最低幅を残す", () => {
  const { sidebar, list } = L.PANE_LIMITS;
  assert.deepEqual(L.clampPaneWidths({ sidebar: 10, list: 10 }, 1400), { sidebar: sidebar.min, list: list.min });
  assert.deepEqual(L.clampPaneWidths({ sidebar: 9999, list: 9999 }, 3000), { sidebar: sidebar.max, list: list.max });
  // 狭い窓では一覧から詰め、足りなければサイドバーも詰める
  const narrow = L.clampPaneWidths({ sidebar: 300, list: 500 }, 1000);
  assert.equal(narrow.sidebar + narrow.list + L.PANE_LIMITS.detailMin, 1000);
  assert.equal(narrow.sidebar, 300 - Math.max(0, 300 + list.min + L.PANE_LIMITS.detailMin - 1000));
  const tiny = L.clampPaneWidths({ sidebar: 300, list: 500 }, 700);
  assert.deepEqual(tiny, { sidebar: sidebar.min, list: list.min }, "下限より小さくはしない");
  assert.deepEqual(L.clampPaneWidths(L.PANE_DEFAULTS, 1180), L.PANE_DEFAULTS, "既定値はそのまま");
});

test("parsePaneWidths は壊れた保存値を既定値に戻す", () => {
  assert.deepEqual(L.parsePaneWidths(null), L.PANE_DEFAULTS);
  assert.deepEqual(L.parsePaneWidths("{bad"), L.PANE_DEFAULTS);
  assert.deepEqual(L.parsePaneWidths('{"sidebar":"x","list":250}'), { ...L.PANE_DEFAULTS, list: 250 });
  assert.deepEqual(L.parsePaneWidths('{"sidebar":180,"list":260}'), { sidebar: 180, list: 260 });
});
