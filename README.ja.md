# vlt

AI開発者のための軽量シークレットマネージャー。

APIキーを `.env` ファイルにばらまくのはやめましょう。`vlt` はシークレットを暗号化されたローカルvaultに保存し、`vlt://` 参照を実行時に解決します。機密情報がディスクやgit履歴に残ることはありません。

## 特徴

- **AES-256-GCM** 暗号化ローカルvault（SQLite）
- **OS Keychain** 連携によるマスターキー管理（macOS Keychain）
- **`vlt://` 参照スキーム** — 環境変数に参照を設定し、実行時に解決
- **ゼロコンフィグ** — 単一バイナリ、デーモン不要、クラウドアカウント不要
- **シェル統合** — `eval "$(vlt env)"` でシームレスなワークフロー

## インストール

### ソースからビルド（Rust 1.70+ 必要）

```bash
cargo install --path .
```

### GitHubからビルド

```bash
git clone https://github.com/Bayfront-Software/vlt.git
cd vlt
cargo build --release
cp target/release/vlt ~/.cargo/bin/
```

## クイックスタート

```bash
# vault を初期化（マスターキーをOS Keychainに保存）
vlt init

# シークレットを保存
vlt set openai/api-key "sk-..."
vlt set anthropic/api-key "sk-ant-..."

# パイプで入力
cat key.txt | vlt set github/token

# シークレットを取得
vlt get openai/api-key

# 全キーを一覧表示
vlt list

# シークレットを削除
vlt delete openai/api-key
```

## 使い方

### シークレットを解決してコマンド実行

環境変数に `vlt://` 参照を設定し、`vlt run` でインメモリ解決してからコマンドを実行します：

```bash
OPENAI_API_KEY="vlt://openai/api-key" vlt run -- python app.py
```

子プロセスには実際のシークレット値が渡されます。参照文字列がシェル設定の外に出ることはありません。

### シェル統合

`~/.zshrc` や `~/.bashrc` に追加：

```bash
export OPENAI_API_KEY="vlt://openai/api-key"
export ANTHROPIC_API_KEY="vlt://anthropic/api-key"
```

`vlt run` 経由で任意のツールを起動：

```bash
vlt run -- claude
vlt run -- python train.py
```

または現在のシェルに解決済みシークレットをエクスポート：

```bash
eval "$(vlt env)"
```

## 仕組み

```
┌──────────────────────────────────────┐
│  vlt CLI                             │
│  init / set / get / run / env        │
├──────────────────────────────────────┤
│  解決エンジン                          │
│  環境変数の vlt:// 参照をスキャン        │
│  インメモリでのみ置換                    │
├──────────────┬───────────────────────┤
│  マスターキー  │  暗号化Vault           │
│  OS Keychain │  SQLite + AES-256-GCM │
└──────────────┴───────────────────────┘
```

1. `vlt init` が256ビットのマスターキーを生成し、OS Keychainに保存
2. `vlt set` が各シークレット値をAES-256-GCM（値ごとに固有のnonce）で暗号化し、ローカルのSQLiteデータベースに保存
3. `vlt run` が環境変数から `vlt://` プレフィックスをスキャンし、参照されたシークレットを復号して `exec` で子プロセスに渡す
4. シークレットが平文で存在するのはプロセスメモリ上のみ — ディスクにもgitにも残らない

## セキュリティモデル

| レイヤー | 実装 |
|---|---|
| 暗号化 | AES-256-GCM（値ごとにランダム12バイトnonce） |
| 鍵管理 | macOS Keychain（システム認証で保護） |
| Vault保管先 | `~/Library/Application Support/vlt/vault.db` |
| 実行時 | シークレットはプロセスメモリ上のみ、envで子プロセスに渡す |

## ライセンス

MIT

## コントリビュート

コントリビュート歓迎です。変更を加える前に、まずissueを開いて議論してください。
