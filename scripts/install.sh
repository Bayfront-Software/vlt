#!/usr/bin/env bash
# CLI と GUI をビルドし、同じ署名 ID で署名して入れる。
#   CLI → ~/.cargo/bin/vlt、GUI → /Applications/vlt.app
#
# なぜ署名するか: キーチェーンの「常に許可」はアプリの署名に紐づく。未署名（ad-hoc）だと
# ビルドのたびに別アプリ扱いになり、毎回キーチェーンのパスワードを聞かれる。
# Apple Development / Developer ID の証明書で署名すると再ビルド後も許可が続く。
#
# 署名 ID は VLT_SIGN_IDENTITY で指定できる（無ければキーチェーンから自動で選ぶ）。
set -euo pipefail
cd "$(dirname "$0")/.."

# 失効した証明書で署名すると、macOS は実行時に「マルウェア」と判定してゴミ箱へ移す
# （find-identity の "valid" はローカルの判定だけで、失効は見ていない）。
# 同名の証明書が複数あることもあるので、候補を OCSP で1つずつ確かめてから使う。
is_not_revoked() {
  local hash="$1" pem
  pem="$(mktemp)"
  security find-certificate -a -Z -p "$HOME/Library/Keychains/login.keychain-db" \
    | awk -v h="$hash" '/SHA-1 hash/{on=($3==h)} on&&/BEGIN CERT/{p=1} p{print} /END CERT/{p=0}' > "$pem"
  local ok=1
  [[ -s "$pem" ]] && security verify-cert -c "$pem" -p codeSign -R ocsp -R require >/dev/null 2>&1 && ok=0
  rm -f "$pem"
  return $ok
}

identity="${VLT_SIGN_IDENTITY:-}"
if [[ -z "$identity" ]]; then
  while read -r hash; do
    if is_not_revoked "$hash"; then
      identity="$hash"
      break
    fi
    echo "skip: 証明書 $hash は失効しています" >&2
  done < <(security find-identity -v -p codesigning | awk '/"(Developer ID Application|Apple Development):/{print $2}')
fi
if [[ -z "$identity" ]]; then
  echo "warning: 署名用の証明書が見つからないので ad-hoc 署名にします（再ビルドのたびにキーチェーンの許可を聞かれます）" >&2
  identity="-"
fi

# DB スキーマを v2 に上げる前（初回だけ）に vault.db の控えを取る。
db="$HOME/Library/Application Support/vlt/vault.db"
if [[ -f "$db" ]] && [[ "$(sqlite3 "$db" 'PRAGMA user_version;')" -lt 2 ]]; then
  backup="$db.v1-backup-$(date +%Y%m%d%H%M%S)"
  cp "$db" "$backup"
  chmod 600 "$backup"
  echo "vault.db の控え: $backup"
fi

echo "==> CLI"
cargo build --release -q
codesign --force --options runtime --identifier dev.bayfront.vlt.cli --sign "$identity" target/release/vlt
install -m 755 target/release/vlt "$HOME/.cargo/bin/vlt"

echo "==> GUI"
(cd gui && APPLE_SIGNING_IDENTITY="$identity" npx tauri build --bundles app)
rm -rf /Applications/vlt.app
cp -R gui/src-tauri/target/release/bundle/macos/vlt.app /Applications/vlt.app

codesign --verify --strict /Applications/vlt.app
codesign --verify --strict "$HOME/.cargo/bin/vlt"
echo "installed: $HOME/.cargo/bin/vlt, /Applications/vlt.app"
echo "初回だけ、キーチェーンの許可ダイアログで「常に許可」を選んでください（CLI と GUI で各1回）。"
