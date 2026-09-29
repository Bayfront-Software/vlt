// GUI の画面遷移を Playwright で通す。Tauri の invoke はメモリ上のモック（e2e/mock.js）に差し替え、
// キーチェーンや Touch ID を使わずに本体画面・ダイアログまで到達して「入力 → 状態遷移」を確かめる。
// 使い方: npm run e2e  → e2e/shots/*.png に各画面が残る
import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const uiDir = path.join(here, "..", "ui");
const shotDir = path.join(here, "shots");
rmSync(shotDir, { recursive: true, force: true });
mkdirSync(shotDir, { recursive: true });
const mockScript = await readFile(path.join(here, "mock.js"), "utf8");

const types = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".mjs": "text/javascript" };
const server = createServer(async (req, res) => {
  const file = path.join(uiDir, req.url === "/" ? "index.html" : req.url);
  try {
    res.writeHead(200, { "content-type": types[path.extname(file)] ?? "application/octet-stream" });
    res.end(await readFile(file));
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((r) => server.listen(0, r));
const url = `http://127.0.0.1:${server.address().port}/`;

const failures = [];
const check = (cond, label) => { if (!cond) failures.push(label); };

for (const colorScheme of ["light", "dark"]) {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1180, height: 760 }, colorScheme, deviceScaleFactor: 2 });
  await page.addInitScript(mockScript);
  page.on("pageerror", (e) => failures.push(`pageerror(${colorScheme}): ${e.message}`));
  page.on("console", (m) => { if (m.type() === "error") failures.push(`console(${colorScheme}): ${m.text()}`); });
  const shot = (name) => page.screenshot({ path: path.join(shotDir, `${name}-${colorScheme}.png`) });
  const hidden = (sel) => page.waitForSelector(sel, { state: "hidden" });
  const clipboard = () => page.evaluate(() => window.__clipboard);

  // 起動: ロック画面が出て、自動で一度だけ解錠を試みる（モックでは即成功）
  await page.goto(url);
  await page.waitForSelector("#screen-main:not([hidden])");
  check((await page.evaluate(() => window.__calls)).filter((c) => c === "unlock").length === 1, "起動時の自動解錠は一度だけ");
  check(await page.locator("#list .row").count() === 7, "一覧に7件");
  check(await page.locator('.cat[data-category="watchtower"] .count.alert').textContent() === "2", "要確認が2件（弱い＋使い回し）");
  await shot("1-main");

  // 項目を選ぶ → 値は伏せられている → 表示 → コピー → 秘密参照をコピー
  await page.click('.row[data-key="dev/github"]');
  await page.waitForSelector("#detail-item:not([hidden])");
  const detailText = await page.textContent("#detail-item");
  check(!detailText.includes("Xk9#mP2"), "詳細の初期表示で値を伏せる");
  check(detailText.includes("alice") && detailText.includes("492 039"), "ユーザー名とワンタイムパスワードが見える");
  await page.hover('#detail-item .field:has([data-field="password"])');
  await page.click('[data-action="reveal-field"][data-field="password"]');
  await page.waitForFunction(() => document.getElementById("detail-item").textContent.includes("Xk9#mP2"));
  await shot("2-detail-login");
  await page.click('[data-action="copy-field"][data-field="password"]');
  check(await clipboard() === "Xk9#mP2$vL8@qR4!", "パスワードをコピー");
  check((await page.textContent("#toast")).includes("90 秒後に消去"), "消去までの秒数を知らせる");
  await page.hover('#detail-item .field:has([data-field="username"])');
  await page.click('[data-action="field-menu"][data-field="username"]');
  await page.click('#popover button:has-text("秘密参照をコピー")');
  check(await clipboard() === "vlt://dev/github/username", "フィールドの秘密参照をコピー");

  // 大きく表示
  await page.hover('#detail-item .field:has([data-field="password"])');
  await page.click('[data-action="field-menu"][data-field="password"]');
  await page.click('#popover button:has-text("大きく表示")');
  await page.waitForSelector("#modal:not([hidden]) .large-type");
  await shot("3-large-type");
  await page.keyboard.press("Escape");
  await hidden("#modal");

  // ⌘C は主役、⌘⇧C はユーザー名
  await page.click("#detail-item h1");
  await page.evaluate(() => window.getSelection().removeAllRanges());
  await page.keyboard.press("Meta+Shift+c");
  check(await clipboard() === "alice", "⌘⇧C でユーザー名");

  // 要確認の分類 → 警告が出る
  await page.click('.cat[data-category="watchtower"]');
  check(await page.locator("#list .row").count() === 2, "要確認で2件");
  await page.click('.row[data-key="home/wifi"]');
  await page.waitForSelector("#detail-item .notice");
  await shot("4-watchtower");

  // SSH 鍵（複数行の秘密）と書類
  await page.click('.cat[data-category="all"]');
  await page.click('.row[data-key="servers/prod-ssh"]');
  await page.waitForSelector('#detail-item [data-field="private_key"]');
  await page.click('.row[data-key="android/hushcam/upload-keystore"]');
  await page.waitForSelector('#detail-item [data-action="save-file"]');
  check((await page.textContent("#detail-item")).includes("upload-keystore.jks"), "書類のファイル名");

  // 検索 → Enter で先頭を選ぶ
  await page.keyboard.press("Meta+f");
  await page.keyboard.type("openai");
  check(await page.locator("#list .row").count() === 1, "検索で1件");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.querySelector("#detail-item h1")?.textContent === "api-key");
  await page.fill("#search", "");
  await page.dispatchEvent("#search", "input");

  // ⌘N → 種類を選ぶ → ログインを作る（パスワード生成・カスタムフィールド・タグ）
  await page.keyboard.press("Meta+n");
  await page.waitForSelector("#modal:not([hidden]) .type-grid");
  await shot("5-type-picker");
  await page.click('.type-choice:has-text("ログイン")');
  await page.waitForSelector("#editor:not([hidden])");
  await page.fill("#edit-key", "dev/stripe");
  check((await page.textContent("#edit-ref-preview")) === "vlt://dev/stripe/<フィールド>", "秘密参照のプレビュー");
  await page.fill("#edit-field-0", "ops@example.com");
  await page.click('[data-action="generate-into"][data-index="1"]');
  await page.waitForSelector("#modal:not([hidden]) .generator-out");
  await page.waitForFunction(() => document.querySelector(".generator-out").textContent.length === 20);
  await shot("6-generator");
  await page.click("#modal-ok");
  await hidden("#modal");
  check((await page.inputValue("#edit-field-1")).length === 20, "生成したパスワードが入る");
  await page.click('[data-action="add-field"]');
  await page.click('#popover button:has-text("パスワード")');
  await page.waitForFunction(() => document.querySelectorAll("#edit-fields .strength .bar").length >= 1);
  check(await page.locator("#edit-fields .edit-field:nth-child(2) + div, #edit-fields .strength .bar").count() >= 1, "他の欄を足してもパスワードの強度表示が消えない");
  await page.click('#edit-fields .edit-field:last-child [data-action="remove-field"]');
  await page.click('[data-action="add-field"]');
  await page.click('#popover button:has-text("パスワード")');
  await page.fill("#edit-fields .edit-field:last-child .label-input", "Webhook Secret");
  await page.fill("#edit-fields .edit-field:last-child input:not(.label-input)", "whsec_123");
  await page.fill("#edit-tags", "work, payments");
  await shot("7-editor");
  await page.keyboard.press("Meta+s");
  await page.waitForSelector("#editor", { state: "hidden" });
  check(await page.textContent("#detail-item h1") === "stripe", "保存後に新しい項目を表示");
  check((await page.textContent("#detail-item")).includes("Webhook Secret"), "カスタムフィールドが保存される");
  check(await page.locator(".cat[data-category='tag:payments']").count() === 1, "新しいタグが分類に出る");

  // 編集 → 名前を変えて保存（改名）→ キャンセルは破棄確認
  await page.keyboard.press("Meta+e");
  await page.waitForSelector("#editor:not([hidden])");
  await page.fill("#edit-key", "dev/stripe-live");
  await page.keyboard.press("Escape");
  await page.waitForSelector("#modal:not([hidden])");
  check((await page.textContent("#modal-title")).includes("破棄"), "変更があれば破棄を確認");
  await page.click('[data-action="modal-cancel"]');
  await hidden("#modal");
  check(!(await page.isHidden("#editor")), "破棄をやめたら編集に戻る");
  await page.click('#editor button[type="submit"]');
  await page.waitForSelector("#editor", { state: "hidden" });
  check(await page.textContent("#detail-item h1") === "stripe-live", "改名できる");

  // お気に入り
  await page.click('[data-action="toggle-favorite"]');
  await page.click('.cat[data-category="favorites"]');
  check(await page.locator('#list .row[data-key="dev/stripe-live"]').count() === 1, "お気に入りに出る");

  // ゴミ箱へ → ゴミ箱で元に戻す
  await page.click('.row[data-key="dev/stripe-live"]');
  await page.keyboard.press("Meta+Backspace");
  await page.waitForSelector("#modal:not([hidden])");
  await page.click("#modal-ok");
  await hidden("#modal");
  await page.click('.cat[data-category="trash"]');
  await page.click('.row[data-action="select-trash"]');
  await page.waitForSelector("#detail-trash:not([hidden])");
  await shot("8-trash");
  await page.click('[data-action="restore"]');
  await page.waitForFunction(() => document.querySelector("#detail-item h1")?.textContent === "stripe-live");
  check(!(await page.isHidden("#detail-item")), "元に戻した項目を表示");

  // 設定 → 保存
  await page.click('[data-action="open-settings"]');
  await page.waitForSelector("#modal:not([hidden]) .setting-row");
  await shot("9-settings");
  await page.selectOption("#modal-body select >> nth=0", "5");
  await page.click("#modal-ok");
  await hidden("#modal");
  check((await page.evaluate(() => window.__calls)).includes("save_settings"), "設定を保存");

  // 自動ロック（Rust 側から通知）→ 理由つきのロック画面 → 解錠で戻る
  await page.evaluate(() => window.__autoLock("idle"));
  await page.waitForSelector("#screen-locked:not([hidden])");
  check((await page.textContent("#lock-reason")).includes("5 分"), "自動ロックの理由を出す");
  check((await page.textContent("#btn-unlock")).includes("Touch ID"), "Touch ID で解錠のボタン");
  await shot("10-locked");
  await page.click('[data-action="unlock"]');
  await page.waitForSelector("#screen-main:not([hidden])");

  // ⌘L でロック
  await page.keyboard.press("Meta+l");
  await page.waitForSelector("#screen-locked:not([hidden])");

  await browser.close();
}
server.close();

if (failures.length) {
  console.error("E2E FAILED:\n - " + failures.join("\n - "));
  process.exit(1);
}
console.log(`E2E OK: screenshots in ${shotDir}`);
