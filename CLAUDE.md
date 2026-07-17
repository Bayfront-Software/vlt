# vlt — 開発ガイド

Rust製のローカルシークレットマネージャー。AES-256-GCM + SQLite + macOS Keychain。

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
```

E2E は一時 DB で: `VLT_DB=/tmp/t.db target/release/vlt ...`
（Keychain は実物を使うので、`init` 済みマシンでは既存キーが使われる）

## 実運用メモ

- 署名鍵などのバックアップ手順: `vlt set --file` → `vlt export` →
  .vltx をオフサイト（iCloud Drive / private repo）へ。復元は
  `VLT_DB=/tmp/r.db vlt import backup.vltx` → `vlt get --out` → ハッシュ比較で検証。
