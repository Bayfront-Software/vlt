# vlt — 開発ガイド

Rust製のローカルシークレットマネージャー。AES-256-GCM + SQLite + macOS Keychain。
1Password を手本にした種類つき項目・秘密参照・Touch ID 解錠の GUI を持つ。

構成（依存は上から下へ一方向）:
- `src/`（crate `vlt`）= lib + CLI (`src/main.rs`)
  - `crypto` 暗号 / `keychain` マスターキー / `store` SQLite / `portable` .vltx
  - `item` 項目モデル / `reference` 秘密参照と注入する環境の組み立て / `totp` / `generator`
  - `env` 変数名・`${VAR}` 展開 / `mask` 伏せ字 / `runner` `vlt run` の子プロセス中継
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
- **`vlt run` の伏せ字は「出力が端末でないとき」だけ**（`runner::mask_streams`）: 漏れて困るのは
  ログ・CI・AI エージェントの記録で、どれも端末ではない。端末まで伏せるには出力をパイプにする必要があり、
  `vlt run -- claude` のような対話型が壊れる。`op run`（常に伏せる）と既定が違うのは意図的。
  伏せる物が無い・どちらの出力も伏せないときは従来通り exec する。4 文字未満の値は伏せない（出力が潰れるため）。
- **秘密かどうかは参照先のフィールドの種類で決める**（`reference::resolve_detailed`）。ユーザー名は伏せない。
- **環境変数の項目（type environment）**: ラベル＝変数名（`env::is_env_name`）。値に `vlt://` 参照を書け、
  参照の中の `${VAR}` は注入する環境（解決前の値）で展開する。環境変数の項目以外を `--env` に渡すとエラー。
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
- **新規作成時のパスワード自動入力は ログイン・パスワード・サーバー・データベース だけ**（`ops::new_item`）。
  Wi-Fi・カード・銀行・API の値は相手から渡されるもので、勝手に作ると誤って保存される。PIN モードは使わない。
- **3列の幅は仕切りのドラッグで可変**（`logic.clampPaneWidths`）。詳細欄 360px を残すため、狭い窓では
  一覧 → サイドバーの順に詰める。利用者の選んだ幅は localStorage に残し、窓を広げたら戻す。
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
- **伏せ字の文字列 `<concealed by vlt>`**: 利用者のログ検索や CI の判定が依存しうる。変えない。
- **`format` の値（0=RAW, 1=ITEM）と `ItemType::as_str` の文字列**: DB と .vltx に保存される。変えない。
- **参照の解決順（完全一致 → 最後の `/` で分割）**: 変えると既存の `.env` や設定ファイルの参照が別の値を指す。
- **主役フィールド（`ItemType::primary_field`）**: 変えると `vlt get` と `vlt://<key>` の結果が変わる。

触ってよい層: `gui/ui`（見た目・文言）、種類の雛形へのフィールド追加（id は既存と重ねない）、
CLI の表示形式、設定の選択肢。

## 検証

```bash
cargo test                    # コア 83 件（crypto/portable/store/item/reference/totp/generator）
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

## 配布（CLI のみ）

- **CLI は Homebrew の tap（`Bayfront-Software/homebrew-tap`、ローカル `~/Projects/homebrew-tap`）で配る。**
  ソースからビルドする Formula なので、Apple の署名・公証が要らない。利用者は `brew install bayfront-software/tap/vlt`。
- **GUI は配らない**（2026-09-29 判断）。Developer ID の配布自体は住所を公開しないが、利用者の事業の準備
  （バーチャルオフィスでの開業届）が整う来年以降に回す。README にも「ソースからビルドのみ」と書いてある。
- **リリース手順:**
  1. `Cargo.toml` と `gui/src-tauri/tauri.conf.json` の version を上げ、`cargo test` と CI を緑にする
  2. `git tag -a vX.Y.Z -m "vlt X.Y.Z" && git push origin vX.Y.Z`、`gh release create vX.Y.Z --notes-file ...`（英語）
  3. `curl -sL https://github.com/Bayfront-Software/vlt/archive/refs/tags/vX.Y.Z.tar.gz | shasum -a 256`
  4. tap の `Formula/vlt.rb` の `url` と `sha256` を更新して push
  5. `brew install bayfront-software/tap/vlt && brew test ... && brew audit --strict ...` で確かめ、
     開発機では `brew uninstall` と `brew untap` で片付ける（手元の署名済み `~/.cargo/bin/vlt` と二重にしない）
- brew 版は ad-hoc 署名なので、更新のたびにキーチェーンの「常に許可」を聞かれる（Formula の caveats と README に記載）。
- CI（`.github/workflows/ci.yml`）: macOS でコアのテストと警告ゼロのビルド、Ubuntu で画面ロジック。

## 寄付

- `.github/FUNDING.yml`・README・GUI の設定画面から https://github.com/sponsors/gzer0-dev へ誘導する。
  有料化はしない方針（2026-09-29 に判断。監査なしの秘密管理を売る責任と、同期が中心設計と衝突するため）。

## 実運用メモ

- 署名鍵などのバックアップ: `vlt set <key> --file` → `vlt export` → .vltx をオフサイトへ。
  復元確認は `VLT_DB=/tmp/r.db vlt import backup.vltx` → `vlt get --out` → ハッシュ比較。
- スキーマ v2 へ上がる前の vault.db の控えは `~/Library/Application Support/vlt/vault.db.v1-backup-*`。
