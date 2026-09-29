# vlt — 開発ガイド

Rust製のローカルシークレットマネージャー。AES-256-GCM + SQLite + macOS Keychain。
1Password を手本にした種類つき項目・秘密参照・Touch ID 解錠の GUI を持つ。

構成（依存は上から下へ一方向）:
- `src/`（crate `vlt`）= lib + CLI (`src/main.rs`)
  - `crypto` 暗号 / `keychain` マスターキー / `store` SQLite / `portable` .vltx
  - `item` 項目モデル / `reference` 秘密参照 / `totp` / `generator`
- `gui/src-tauri`（crate `vlt-gui`）= lib を path 依存で使う Tauri 2 アプリ
  - `ops.rs` 画面用の操作（純粋・一時 DB でテスト）/ `lib.rs` コマンドと常駐処理 /
    `macos.rs` Touch ID・クリップボード・画面ロック検知 / `settings.rs`
- `gui/ui` = 静的 HTML/CSS/JS（バンドラ無し）。判断は `logic.mjs` の純関数、`app.js` は描画と配線

## 最重要ルール

- **タスク完了前に `cargo test`・`cargo build --release`（警告ゼロ）・`cd gui && npm run verify` を通す。**
- テストなしで crypto / portable / store / item / reference を変更しない（TDD）。
- **インストールは `scripts/install.sh` だけで行う**（署名 ID の選び方に理由がある。下記）。

## 設計判断の「なぜ」

- **マスターキーは OS Keychain のみ、vault.db は持ち出し不可**:
  vault.db を盗まれても Keychain がなければ復号できない。裏返しとして
  **vault.db のコピーはバックアップにならない**。持ち出しは必ず `vlt export`（.vltx）。
- **.vltx はパスフレーズ（Argon2id m=64MiB,t=3,p=4）でのみ守る**: マシン非依存の復元が目的。
- **`init` は既存マスターキーがあると失敗する**（旧実装は黙って上書きし vault を全損させた）。
  GUI には `--force` 相当を置かない。
- **項目 = キー（識別子）+ 種類 + フィールド列 + メモ + タグ + お気に入り**（`item.rs`）。
  JSON にして丸ごと暗号化する。平文で DB に出るのはキー・種類・日時だけ。
  種類ごとに「主役フィールド」があり、`vlt get <key>` と `vlt://<key>` はそれを返す。
- **旧形式（v0.2 の生の値）の行は書き換えない**: `format` 列 0=RAW / 1=ITEM。RAW は読むときに
  `Item::from_legacy`（テキスト→パスワード、バイナリ→書類）で解釈する。全行を書き換える移行は
  途中失敗で vault を壊す経路になるので採らなかった。編集して保存した行から ITEM になる。
- **秘密参照 `vlt://<キー>[/<フィールド>][?attribute=otp]`**: キー自体が `/` を含むので、
  「パス全体が項目のキー」を先に探し、無ければ最後の `/` でキーとフィールドに分ける。
  完全一致が常に勝つので v0.2 までの `vlt://<キー>` は壊れない。フィールドは id → ラベル
  （大文字小文字無視）の順で探す。ファイルは参照で渡さない（環境変数に入らないため）。
  キーに `?` を許さないのはクエリと区別できなくなるから（GUI/ops で弾く）。
- **削除はゴミ箱へ（30 日で自動消去）**: GUI も CLI も。完全削除は `--purge` かゴミ箱から。
- **Touch ID 解錠**: 解錠時に LocalAuthentication（Touch ID、無ければログインパスワード）で本人確認
  してから Keychain を読む。Keychain 項目自体に生体認証の ACL を付ける方式は、データ保護キーチェーン
  と provisioning profile が要り、自作の署名だけでは使えないので採らなかった。
  CLI には本人確認を入れていない（AI エージェントやスクリプトから非対話で使うのが主用途のため）。
- **署名は失効確認つき（`scripts/install.sh`）**: キーチェーンの「常に許可」は署名の designated
  requirement（チーム＋証明書名）に紐づくので、正規の証明書で署名すれば再ビルド後も許可が続く。
  ad-hoc だとビルドのたびに許可を聞かれる。**失効した証明書で署名すると macOS は実行時に
  「マルウェア」と判定してゴミ箱へ移す**（2026-09-29 に実際に CLI が消えた。同名の証明書が2つあり、
  `find-identity -v` の "valid" は失効を見ない）。だから候補を OCSP で確かめてから使う。
- **GUI は値を一覧・詳細に載せない**（`ops::list` / `ops::view` は伏せる種類の値を None にする）。
  WebView に値が渡るのは「表示」「コピー」「編集」を明示したときだけ。ファイルの中身は一度も渡さない
  （取り込みは Rust 側で受け取り番号に預け、保存は Rust 側のダイアログで書く）。
- **コピーは `org.nspasteboard.ConcealedType` 付き**（Raycast などの履歴に残さない）、設定秒数後に
  changeCount が変わっていなければ消す（その間に別の物をコピーしていたら消さない）。
- **自動ロック**: 最後の操作から N 分・画面ロック・スリープでロック。TOTP の定期更新は
  `with_store_quiet` で読み、操作に数えない（数えると表示中はロックされなくなる）。
- **ファイルダイアログ・Touch ID・キーチェーン待ちは async コマンド**: Tauri の同期コマンドは
  メインスレッドで動くので、そこで待つと画面ごと固まる。
- **閉じるボタンは隠すだけ（常駐）**、⌘Q で終了。⌘⇧Space（設定で無効化可）で呼び出す。
- **ウインドウのドラッグは上端の全幅の帯（`.drag-strip`）で行う**: `data-tauri-drag-region` は
  子要素の上では効かないので、帯には子を持たせない。権限 `core:window:allow-start-dragging` が必要。
- **vault.db は 600、既定の保存先は 700**: 中身は暗号化済みでもキー名・件数・日時は平文のため。
- **`VLT_DB` / `VLT_KEYCHAIN_SERVICE`**: 保存先と Keychain のサービス名の差し替え。本物の
  マスターキーに触れずに E2E を回すためのもので、普段は未設定が前提。
- **`VLT_PASSPHRASE`**: スクリプトからの export/import 用。

## 触ってはいけない層（保護境界）

- **`src/crypto.rs` の暗号形式**（nonce 12B || ciphertext、AES-256-GCM）と **KDF パラメータ定数**:
  変えると既存 vault / .vltx が開けなくなる。変えるなら portable::FORMAT_VERSION を上げ旧形式の読み込みを残す。
- **`src/portable.rs` の .vltx envelope**: `MIN_FORMAT_VERSION..=FORMAT_VERSION` を読める。v1 を読む
  テスト `unseal_reads_v1_files_without_format_field` と `unseal_rejects_unknown_version` が門番。
- **store のスキーマ移行**: `user_version` の `< 1` / `< 2` の分岐を消さない。新しい列は追加だけ。
  `SCHEMA_VERSION` より新しい DB は開かない（古い vlt が新しい DB を壊さないため）。
- **`format` の値（0=RAW, 1=ITEM）と `ItemType::as_str` の文字列**: DB と .vltx に保存される。変えない。
- **参照の解決順（完全一致 → 最後の `/` で分割）**: 変えると既存の `.env` や設定ファイルの参照が別の値を指す。
- **主役フィールド（`ItemType::primary_field`）**: 変えると `vlt get` と `vlt://<key>` の結果が変わる。

触ってよい層: `gui/ui`（見た目・文言）、種類の雛形へのフィールド追加（id は既存と重ねない）、
CLI の表示形式、設定の選択肢。

## 検証

```bash
cargo test                    # コア 61 件（crypto/portable/store/item/reference/totp/generator）
cargo build --release         # 警告ゼロ
./scripts/e2e-cli.sh          # CLI を実 Keychain で端から端まで（専用サービス名＋一時 DB、後始末つき）
cd gui && npm run verify      # 画面ロジック(node --test) + Playwright E2E(モック invoke) + cargo test/build
./scripts/install.sh          # 署名して ~/.cargo/bin/vlt と /Applications/vlt.app へ
```

- `gui/ui/logic.test.mjs` は「main 以外の全画面に次へ進む操作がある」「ダイアログと編集に抜ける操作がある」
  「HTML と動的ボタンの data-action がすべて app.js に定義されている」を検査する（ソフトロック禁止）。
- `gui/e2e/walk.mjs` は `e2e/mock.js` で invoke を差し替え、解錠→表示→コピー→秘密参照→新規（生成器・
  カスタムフィールド・タグ）→改名→お気に入り→ゴミ箱→復元→設定→自動ロック→⌘L をクリックで通し、
  ライト/ダークのスクショを `gui/e2e/shots/` に残す。モックは ops.rs と同じ「値を伏せる」形で返すこと。
- 実 Keychain と Touch ID を通る本物のアプリは、人が一度「常に許可」と指紋を当てる必要がある。
  機械検証はモック経由。**利用者がこの Mac を使っている最中に GUI を起動して撮らない**
  （フォーカスを奪って入力を横取りし、画面全体の撮影は私的な画面を写す）。

## 実運用メモ

- 署名鍵などのバックアップ: `vlt set <key> --file` → `vlt export` → .vltx をオフサイトへ。
  復元確認は `VLT_DB=/tmp/r.db vlt import backup.vltx` → `vlt get --out` → ハッシュ比較。
- スキーマ v2 へ上がる前の vault.db の控えは `~/Library/Application Support/vlt/vault.db.v1-backup-*`。
