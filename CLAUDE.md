# vlt — 開発ガイド

Rust製のローカルシークレットマネージャー。AES-256-GCM + SQLite + macOS Keychain。

構成: ルート crate `vlt` = lib（crypto/store/keychain/portable/resolve）+ CLI (`src/main.rs`)。
`gui/` = Tauri 2 のデスクトップ GUI（`gui/src-tauri` が lib を path 依存で使う。
フロントは `gui/ui` の静的 HTML/CSS/JS、バンドラ無し）。

## 最重要ルール

- **タスク完了前に `cargo test` と `cargo build --release`（警告ゼロ）を通す。**
- テストなしで crypto / portable / store を変更しない（TDD）。

## 設計判断の「なぜ」

- **マスターキーは OS Keychain のみ、vault.db は持ち出し不可**:
  vault.db を盗まれても Keychain がなければ復号できない設計。裏返しとして
  **vault.db のコピーはバックアップにならない**。マシンをまたぐ持ち出しは
  必ず `vlt export`（.vltx）を使う。これがこのツールの中心的な安全性トレードオフ。
- **.vltx はパスフレーズ（Argon2id）でのみ守られる**: マシン非依存の復元が
  目的なので Keychain に依存してはいけない。KDF は argon2id
  (m=64MiB, t=3, p=4)。GPU総当たり耐性のためメモリハードにしている。
- **`init` は既存マスターキーがあると失敗する**: 旧実装は黙って Keychain を
  上書きし、既存 vault が静かに全損する事故経路だった。`--force` で明示破壊のみ許可。
- **バイナリ値は `binary` フラグで区別**: `get`（テキスト出力）がバイナリを
  端末に吐いて壊れるのを防ぎ、`--out` を強制するため。DB スキーマは
  PRAGMA user_version で移行管理（v0→v1 で binary カラム追加）。
- **`VLT_DB` 環境変数**: テストと復元検証（別vaultへのimport確認）のための
  保存先差し替え。プロダクション利用では未設定が前提。
- **GUI は値を一覧に載せず、表示は明示的な reveal のみ（30 秒で自動的に隠す）**:
  1Password と同じ「見せない既定」。コピーした値も 30 秒後にクリップボードから消す
  （その間に別の物がコピーされていれば消さない）。
- **GUI の起動時の状態確認は Keychain の値を読まない**（`keychain::has_master_key`）:
  値を読むと macOS の許可ダイアログが出るため、ダイアログは「解錠」ボタンまで出さない。
  初回の解錠で「常に許可」を押せば以後は出ない。バイナリを差し替える（再ビルド）と
  また聞かれることがある（Keychain の ACL はアプリ単位のため）。
- **ファイル選択・保存・クリップボードは Rust 側**（tauri-plugin-dialog / clipboard-manager）:
  値を WebView に長く置かないためと、JS 側の権限を `core:default` だけに絞るため。
- **GUI に `init --force` 相当は無い**: vault 全損経路は CLI の明示操作にだけ残す。
- **`VLT_PASSPHRASE` 環境変数**: スクリプトからの export/import 用。
  対話時は rpassword で非表示入力（確認2回）。

## 触ってはいけない層（保護境界）

- **`src/crypto.rs` の暗号形式**（nonce 12B || ciphertext、AES-256-GCM）と
  **KDF パラメータ定数**: 変えると既存 vault / .vltx が開けなくなる。
  変更が必要なら portable::FORMAT_VERSION を上げ、旧形式の読み込みを残すこと。
- **`src/portable.rs` の .vltx envelope 構造**: 同上。version フィールドが
  互換性の生命線。テスト `unseal_rejects_unknown_version` が門番。
- **store のスキーマ移行**: user_version の分岐を消さない。旧DBが読めなくなる。

## 検証

```bash
cargo test                    # 13+ tests（crypto/portable/store）
cargo build --release         # 警告ゼロを維持
cd gui && npm run verify      # node --test(画面ロジック) + Playwright E2E + cargo test/build
cd gui && npm run build       # .app / .dmg を gui/src-tauri/target/release/bundle/ に作る
```

GUI の画面遷移は `gui/ui/logic.mjs` の純関数に集約し、`logic.test.mjs` が
「main 以外の全画面に次へ進む操作がある」ことと「index.html に同名の data-action ボタンがある」
ことを検査する（ソフトロック禁止ルール）。`gui/e2e/walk.mjs` は Tauri の invoke を
メモリ上のモックに差し替えて、解錠→選択→表示→新規→改名→削除→書き出し→ロックを実際に
クリックで通し、ライト/ダークのスクショを `gui/e2e/shots/` に残す。
実 Keychain を使う本物のアプリは人が一度「常に許可」を押す必要があるため、
機械検証はモック経由で行い、実機は `~/Applications/vlt.app` を開いて確かめる。

E2E は一時 DB で: `VLT_DB=/tmp/t.db target/release/vlt ...`
（Keychain は実物を使うので、`init` 済みマシンでは既存キーが使われる）

## 実運用メモ

- 署名鍵などのバックアップ手順: `vlt set --file` → `vlt export` →
  .vltx をオフサイト（iCloud Drive / private repo）へ。復元は
  `VLT_DB=/tmp/r.db vlt import backup.vltx` → `vlt get --out` → ハッシュ比較で検証。
