// Tauri の invoke / event をメモリ上で差し替えるモック（ブラウザに注入する）。
// 実物の Rust 側（ops.rs）と同じ「一覧・詳細に伏せる値を載せない」形で返す。
window.__installVltMock = () => {
  const now = () => new Date().toISOString().slice(0, 19).replace("T", " ");
  const TYPES = [
    ["login", "ログイン"], ["password", "パスワード"], ["api_credential", "API 認証情報"], ["secure_note", "セキュアノート"],
    ["credit_card", "クレジットカード"], ["identity", "個人情報"], ["ssh_key", "SSH 鍵"], ["database", "データベース"],
    ["server", "サーバー"], ["software_license", "ソフトウェアライセンス"], ["wifi", "Wi-Fi"], ["bank_account", "銀行口座"], ["document", "書類"],
  ];
  const label = (t) => TYPES.find(([id]) => id === t)[1];
  const PRIMARY = { login: "password", password: "password", database: "password", server: "password", wifi: "password", api_credential: "credential", credit_card: "number", ssh_key: "private_key", software_license: "license_key", bank_account: "account_number", document: "file" };
  const TEMPLATES = {
    login: [["username", "ユーザー名", "text"], ["password", "パスワード", "concealed"], ["website", "ウェブサイト", "url"], ["one_time_password", "ワンタイムパスワード", "totp"]],
    api_credential: [["username", "ユーザー名", "text"], ["credential", "認証情報", "concealed"], ["type", "種類", "text"], ["hostname", "ホスト名", "text"], ["expires", "有効期限", "date"]],
    password: [["password", "パスワード", "concealed"]],
    document: [["file", "ファイル", "file"]],
    ssh_key: [["private_key", "秘密鍵", "secret_block"], ["public_key", "公開鍵", "multiline"], ["fingerprint", "フィンガープリント", "text"], ["key_type", "鍵の種類", "text"]],
  };
  const secretKinds = ["concealed", "secret_block", "totp", "file"];
  const f = (id, lbl, kind, value = "", extra = {}) => ({ id, label: lbl, kind, value, filename: null, ...extra });
  const items = new Map([
    ["dev/github", { type: "login", favorite: true, tags: ["work"], notes: "リカバリーコードは金庫", fields: [f("username", "ユーザー名", "text", "alice"), f("password", "パスワード", "concealed", "Xk9#mP2$vL8@qR4!"), f("website", "ウェブサイト", "url", "https://github.com"), f("one_time_password", "ワンタイムパスワード", "totp", "GEZDGNBVGY3TQOJQ")], created_at: "2026-07-18 01:10:02", updated_at: "2026-09-03 04:14:12" }],
    ["openai/api-key", { type: "api_credential", favorite: false, tags: ["work", "ai"], notes: "", fields: [f("username", "ユーザー名", "text", ""), f("credential", "認証情報", "concealed", "sk-live-1234567890abcdef"), f("type", "種類", "text", "bearer"), f("hostname", "ホスト名", "text", "api.openai.com"), f("expires", "有効期限", "date", "")], created_at: "2026-08-01 10:00:00", updated_at: "2026-09-01 10:00:00" }],
    ["notion/integration-token", { type: "password", favorite: false, tags: [], notes: "", fields: [f("password", "パスワード", "concealed", "ntn_abcdefghijklmnop")], created_at: "2026-09-03 04:14:12", updated_at: "2026-09-03 04:14:12" }],
    ["android/hushcam/upload-keystore", { type: "document", favorite: false, tags: ["android"], notes: "", fields: [f("file", "ファイル", "file", "AAAA", { filename: "upload-keystore.jks", size: 2718 })], created_at: "2026-07-18 01:10:02", updated_at: "2026-07-18 01:10:02" }],
    ["home/wifi", { type: "login", favorite: false, tags: [], notes: "", fields: [f("username", "ユーザー名", "text", "router-admin"), f("password", "パスワード", "concealed", "password")], created_at: "2026-06-01 00:00:00", updated_at: "2026-06-01 00:00:00" }],
    ["home/nas", { type: "login", favorite: false, tags: [], notes: "", fields: [f("username", "ユーザー名", "text", "admin"), f("password", "パスワード", "concealed", "password")], created_at: "2026-06-01 00:00:00", updated_at: "2026-06-01 00:00:00" }],
    ["servers/prod-ssh", { type: "ssh_key", favorite: true, tags: ["infra"], notes: "", fields: [f("private_key", "秘密鍵", "secret_block", "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----"), f("public_key", "公開鍵", "multiline", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI deploy@prod"), f("fingerprint", "フィンガープリント", "text", "SHA256:4f9x…Qk"), f("key_type", "鍵の種類", "text", "ed25519")], created_at: "2026-05-01 00:00:00", updated_at: "2026-05-01 00:00:00" }],
  ]);
  const trash = [];
  let trashId = 1;
  let unlocked = false;
  let settings = { auto_lock_minutes: 10, clipboard_clear_secs: 90, lock_on_screen_lock: true, unlock_with_touch_id: true, global_shortcut: true };
  const listeners = {};
  const strength = (pw) => (pw.length < 9 ? "weak" : pw.length < 12 ? "fair" : pw.length < 17 ? "strong" : "very_strong");
  const strengthLabel = { weak: "弱い", fair: "普通", strong: "強い", very_strong: "とても強い" };
  const need = (key) => { const it = items.get(key); if (!it) throw `Secret not found: ${key}`; return it; };
  const ref = (key, it, fid) => (PRIMARY[it.type] === fid ? `vlt://${key}` : `vlt://${key}/${fid}`);
  const pwOf = (it) => it.fields.find((x) => x.id === "password" && x.kind === "concealed" && x.value)?.value;
  const subtitle = (it) => it.fields.find((x) => ["username", "ssid", "cardholder", "email"].includes(x.id) && x.value && !secretKinds.includes(x.kind))?.value
    ?? it.fields.find((x) => x.kind === "file")?.filename ?? label(it.type);

  const commands = {
    app_info: () => ({ initialized: true, unlocked, db_path: "/Users/you/Library/Application Support/vlt/vault.db", biometrics: true, settings, version: "0.3.0" }),
    unlock: () => { unlocked = true; },
    lock: () => { unlocked = false; (listeners["vault-locked"] ?? []).forEach((cb) => cb({ payload: "manual" })); },
    touch: () => {},
    item_types: () => TYPES.map(([id, l]) => ({ id, label: l })),
    list_items: () => {
      const counts = {};
      for (const it of items.values()) { const p = pwOf(it); if (p) counts[p] = (counts[p] ?? 0) + 1; }
      return [...items].sort(([a], [b]) => a.localeCompare(b)).map(([key, it]) => {
        const p = pwOf(it);
        return { key, item_type: it.type, type_label: label(it.type), subtitle: subtitle(it), favorite: it.favorite, tags: it.tags, created_at: it.created_at, updated_at: it.updated_at, weak: !!p && strength(p) === "weak", reused: !!p && counts[p] > 1 };
      });
    },
    view_item: ({ key }) => {
      const it = need(key);
      return {
        key, item_type: it.type, type_label: label(it.type), notes: it.notes, tags: it.tags, favorite: it.favorite, created_at: it.created_at, updated_at: it.updated_at,
        notes_reference: `vlt://${key}/notes`, primary_field: PRIMARY[it.type] ?? null,
        fields: it.fields.map((x) => ({ id: x.id, label: x.label, kind: x.kind, secret: secretKinds.includes(x.kind), has_value: !!x.value, value: secretKinds.includes(x.kind) ? null : x.value, filename: x.filename, size: x.size ?? null, reference: ref(key, it, x.id), strength: x.kind === "concealed" && x.value ? strength(x.value) : null })),
      };
    },
    reveal_field: ({ key, field }) => { const x = need(key).fields.find((y) => y.id === field); if (x.kind === "file") throw "ファイルは画面に表示できません"; return x.value; },
    totp_code: () => ({ code: "492039", remaining: 17, period: 30 }),
    copy_field: ({ key, field }) => { window.__clipboard = field === "notes" ? need(key).notes : need(key).fields.find((y) => y.id === field).value; return settings.clipboard_clear_secs; },
    copy_secret: ({ text }) => { window.__clipboard = text; return settings.clipboard_clear_secs; },
    copy_plain: ({ text }) => { window.__clipboard = text; },
    open_url: () => {},
    template_item: ({ itemType, options }) => {
      // ops::new_item と同じく、自分で決めるパスワードの種類だけ生成して入れる
      const generates = ["login", "password", "server", "database"].includes(itemType);
      const generated = generates ? commands.generate_password({ options }).password : "";
      return { item_type: itemType, notes: "", tags: [], favorite: false, fields: (TEMPLATES[itemType] ?? []).map(([id, l, kind]) => ({ id, label: l, kind, value: id === "password" ? generated : "", filename: null, pending_file: null })) };
    },
    editable_item: ({ key }) => { const it = need(key); return { item_type: it.type, notes: it.notes, tags: [...it.tags], favorite: it.favorite, fields: it.fields.map((x) => ({ id: x.id, label: x.label, kind: x.kind, value: x.kind === "file" ? "" : x.value, filename: x.filename, pending_file: null })) }; },
    pick_file: () => ({ token: "t1", filename: "service-account.json", size: 1234 }),
    save_item: ({ originalKey, key, item }) => {
      if (items.has(key) && originalKey !== key) throw `${key} は既にあります`;
      const prev = originalKey ? items.get(originalKey) : null;
      if (originalKey && originalKey !== key) items.delete(originalKey);
      let n = 0;
      const fields = item.fields.map((x) => ({ id: x.id || `custom_${++n}`, label: x.label, kind: x.kind, value: x.kind === "file" ? (x.pending_file ? "BBBB" : prev?.fields.find((p) => p.id === x.id)?.value ?? "") : x.value, filename: x.filename, size: x.kind === "file" ? 1234 : undefined }));
      items.set(key, { type: item.item_type, favorite: item.favorite, tags: item.tags, notes: item.notes, fields, created_at: prev?.created_at ?? now(), updated_at: now() });
    },
    set_favorite: ({ key, favorite }) => { need(key).favorite = favorite; },
    duplicate_item: ({ key }) => { const copy = structuredClone(need(key)); copy.favorite = false; items.set(`${key} copy`, copy); return `${key} copy`; },
    delete_item: ({ key }) => { const it = need(key); items.delete(key); trash.unshift({ id: trashId++, key, it, deleted_at: now() }); },
    list_trash: () => trash.map((t) => ({ id: t.id, key: t.key, item_type: t.it.type, type_label: label(t.it.type), deleted_at: t.deleted_at })),
    restore_item: ({ id }) => { const i = trash.findIndex((t) => t.id === id); const t = trash[i]; if (items.has(t.key)) throw `${t.key} は既にあるので戻せません`; items.set(t.key, t.it); trash.splice(i, 1); return t.key; },
    purge_trash_item: ({ id }) => { const i = trash.findIndex((t) => t.id === id); trash.splice(i, 1); },
    empty_trash: () => { trash.length = 0; },
    generate_password: ({ options }) => { const pool = options.pin ? "0123456789" : "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ" + (options.digits ? "23456789" : "") + (options.symbols ? "!@#$%^&*" : ""); let p = ""; for (let i = 0; i < options.length; i++) p += pool[(i * 7 + 3) % pool.length]; const s = strength(p); return { password: p, strength: s, strength_label: strengthLabel[s] }; },
    password_strength: ({ password }) => ({ strength: strength(password), label: strengthLabel[strength(password)] }),
    save_settings: ({ settings: next }) => { settings = next; return settings; },
    export_vault: () => "/Users/you/vlt-backup.vltx",
    import_vault: () => ({ imported: 2, skipped: 1 }),
  };
  window.__calls = [];
  window.__emit = (name, payload) => (listeners[name] ?? []).forEach((cb) => cb({ payload }));
  // Rust 側の自動ロックと同じ順序: 先に store を捨て、それから画面へ通知する。
  window.__autoLock = (reason) => { unlocked = false; window.__emit("vault-locked", reason); };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args = {}) => { window.__calls.push(cmd); const fn = commands[cmd]; if (!fn) throw `unknown command ${cmd}`; return structuredClone(fn(args) ?? null); } },
    event: { listen: async (name, cb) => { (listeners[name] ??= []).push(cb); return () => {}; } },
  };
};
window.__installVltMock();
