import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

// data-tauri-drag-region は core:window:allow-start-dragging が無いと黙って効かない
// （core:default には含まれない）。ウインドウが動かせない事故を防ぐ。
test("ドラッグ領域を使うならウインドウ移動の権限がある", async () => {
  const html = await readFile(new URL("./index.html", import.meta.url), "utf8");
  const cap = JSON.parse(await readFile(new URL("../src-tauri/capabilities/default.json", import.meta.url), "utf8"));
  if (html.includes("data-tauri-drag-region")) {
    assert.ok(cap.permissions.includes("core:window:allow-start-dragging"));
  }
});
