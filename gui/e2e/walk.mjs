// GUI の画面遷移を Playwright で通す。Tauri の invoke はメモリ上のモックに差し替える
// （キーチェーンを触らずに本体画面・ダイアログまで到達し、入力→遷移の配線を確かめる）。
// 使い方: npm run e2e  → e2e/shots/*.png に各画面が残る
import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const uiDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui");
const shotDir = path.join(path.dirname(fileURLToPath(import.meta.url)), "shots");
mkdirSync(shotDir, { recursive: true });

const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".mjs": "text/javascript" };
const server = createServer(async (req, res) => {
  const file = path.join(uiDir, req.url === "/" ? "index.html" : req.url);
  try {
    const body = await readFile(file);
    res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    res.end(body);
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, r));
const url = `http://127.0.0.1:${server.address().port}/`;

const mockTauri = () => {
  const items = new Map([
    ["openai/api-key", { value: "sk-live-1234567890abcdef", binary: false, created_at: "2026-07-18 01:10:02", updated_at: "2026-09-03 04:14:12" }],
    ["notion/integration-token", { value: "ntn_abcdefghijklmnop", binary: false, created_at: "2026-09-03 04:14:12", updated_at: "2026-09-03 04:14:12" }],
    ["android/hushcam/upload-keystore", { value: null, binary: true, created_at: "2026-07-18 01:10:02", updated_at: "2026-07-18 01:10:02" }],
    ["android/hushcam/key-properties", { value: null, binary: true, created_at: "2026-07-18 01:10:02", updated_at: "2026-07-18 01:10:02" }],
    ["cloudflare/pages-token", { value: "cf-token-xyz", binary: false, created_at: "2026-08-05 10:00:00", updated_at: "2026-08-05 10:00:00" }],
  ]);
  let unlocked = false;
  const now = () => new Date().toISOString().slice(0, 19).replace("T", " ");
  const commands = {
    vault_status: () => ({ initialized: true, unlocked, db_path: "/mock/vault.db" }),
    unlock: () => { unlocked = true; },
    lock: () => { unlocked = false; },
    list_secrets: () => [...items].map(([key, v]) => ({ key, binary: v.binary, created_at: v.created_at, updated_at: v.updated_at })),
    reveal_secret: ({ key }) => { const v = items.get(key); if (!v) throw `Secret not found: ${key}`; if (v.binary) throw `${key} はバイナリです`; return v.value; },
    set_secret: ({ key, value }) => { const prev = items.get(key); items.set(key, { value, binary: false, created_at: prev?.created_at ?? now(), updated_at: now() }); },
    rename_secret: ({ from, to }) => { if (items.has(to)) throw `${to} は既に存在します`; items.set(to, items.get(from)); items.delete(from); },
    delete_secret: ({ key }) => { if (!items.delete(key)) throw `Secret not found: ${key}`; },
    copy_secret: () => 30,
    export_vault: () => "/mock/vlt-backup.vltx",
    import_vault: () => ({ imported: 2, skipped: 1 }),
  };
  window.__TAURI__ = { core: { invoke: async (cmd, args = {}) => { const fn = commands[cmd]; if (!fn) throw `unknown command ${cmd}`; window.__calls.push(cmd); return fn(args); } } };
  window.__calls = [];
};

const failures = [];
const check = (cond, label) => { if (!cond) failures.push(label); };

for (const colorScheme of ["light", "dark"]) {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1040, height: 680 }, colorScheme, deviceScaleFactor: 2 });
  await page.addInitScript(mockTauri);
  page.on("pageerror", (e) => failures.push(`pageerror(${colorScheme}): ${e.message}`));
  await page.goto(url);
  const shot = (name) => page.screenshot({ path: path.join(shotDir, `${name}-${colorScheme}.png`) });

  // locked → main
  await page.waitForSelector("#screen-locked:not([hidden])");
  await shot("1-locked");
  await page.click('[data-action="unlock"]');
  await page.waitForSelector("#screen-main:not([hidden])");
  check(await page.locator(".list-item").count() === 5, "一覧に5件");
  await shot("2-main-empty");

  // select → reveal → copy
  await page.click('.list-item[data-key="openai/api-key"]');
  check(await page.textContent("#item-name") === "api-key", "詳細のタイトル");
  check(await page.textContent("#value-text") !== "sk-live-1234567890abcdef", "選択直後は伏せ字");
  await page.click('[data-action="reveal"]');
  await page.waitForFunction(() => document.getElementById("value-text").textContent.startsWith("sk-live"));
  await shot("3-detail-revealed");
  await page.click('[data-action="copy"]');
  await page.waitForSelector("#toast:not([hidden])");
  check((await page.textContent("#toast")).includes("コピー"), "コピーのトースト");

  // binary item
  await page.click('.list-item[data-key="android/hushcam/upload-keystore"]');
  check(!(await page.isHidden("#field-binary")), "バイナリ欄が出る");
  check(await page.isHidden("#field-text"), "テキスト欄は隠れる");
  await shot("4-detail-binary");

  // search
  await page.fill("#search", "notion");
  check(await page.locator(".list-item").count() === 1, "検索で1件に絞る");
  await page.fill("#search", "");

  // ⌘N → new item → save → selected
  await page.keyboard.press("Meta+n");
  await page.waitForSelector("#modal:not([hidden])");
  await shot("5-new-item");
  await page.fill("#modal-body input", "stripe/secret-key");
  await page.fill("#modal-body textarea", "sk_test_xxx");
  await page.click("#modal-ok");
  await page.waitForSelector("#modal", { state: "hidden" });
  check(await page.textContent("#item-name") === "secret-key", "新規作成後に選択される");
  check(await page.locator(".list-item").count() === 6, "一覧が6件");

  // rename
  await page.click('[data-action="rename"]');
  await page.fill("#modal-body input", "stripe/live-key");
  await page.click("#modal-ok");
  await page.waitForSelector("#modal", { state: "hidden" });
  check(await page.textContent("#item-name") === "live-key", "改名後に新キーが選択される");

  // delete → next selected
  await page.click('[data-action="delete"]');
  await page.waitForSelector("#modal:not([hidden])");
  await shot("6-delete-confirm");
  await page.click("#modal-ok");
  await page.waitForSelector("#modal", { state: "hidden" });
  check(await page.locator(".list-item").count() === 5, "削除後は5件");
  check(!(await page.isHidden("#detail-item")), "削除後も別項目が選ばれている");

  // export dialog (menu)
  await page.click('[data-action="menu"]');
  await page.click('[data-action="export"]');
  await page.waitForSelector("#modal:not([hidden])");
  await page.fill('#modal-body input[type="password"] >> nth=0', "pass");
  await page.fill('#modal-body input[type="password"] >> nth=1', "wrong");
  await page.click("#modal-ok");
  check((await page.textContent("#modal-error")).includes("一致"), "不一致でエラー");
  await shot("7-export");
  await page.fill('#modal-body input[type="password"] >> nth=1', "pass");
  await page.click("#modal-ok");
  await page.waitForSelector("#modal", { state: "hidden" });

  // arrow keys + lock
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Meta+l");
  await page.waitForSelector("#screen-locked:not([hidden])");
  check((await page.evaluate(() => window.__calls)).includes("lock"), "⌘L で lock が呼ばれる");

  await browser.close();
}
server.close();

if (failures.length) {
  console.error("E2E FAILED:\n - " + failures.join("\n - "));
  process.exit(1);
}
console.log(`E2E OK: screenshots in ${shotDir}`);
