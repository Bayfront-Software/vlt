#!/usr/bin/env bash
# CLI を実物の Keychain で端から端まで通す。本物のマスターキー（dev.bayfront.vlt）には触れず、
# 専用のサービス名と一時 DB を使い、終わったら両方消す。
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -q
VLT="$PWD/target/release/vlt"
WORK="$(mktemp -d)"
export VLT_DB="$WORK/vault.db"
export VLT_KEYCHAIN_SERVICE="dev.bayfront.vlt.e2e.$$"
cleanup() {
  security delete-generic-password -s "$VLT_KEYCHAIN_SERVICE" -a master-key >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
expect_eq() { [[ "$1" == "$2" ]] || fail "$3: expected [$2] got [$1]"; }

"$VLT" init >/dev/null
"$VLT" set openai/api-key sk-test >/dev/null
expect_eq "$("$VLT" get openai/api-key)" "sk-test" "get primary"
"$VLT" set github/login s3cret --type login >/dev/null
"$VLT" set github/login alice --field username >/dev/null
"$VLT" set github/login GEZDGNBVGY3TQOJQ --field one_time_password >/dev/null
"$VLT" set github/login "backup codes" --field notes >/dev/null
expect_eq "$("$VLT" get github/login)" "s3cret" "set --field keeps primary"
expect_eq "$("$VLT" read vlt://github/login/username)" "alice" "read field"
expect_eq "$("$VLT" read -n vlt://github/login/notes)" "backup codes" "read notes"
[[ "$("$VLT" totp github/login)" =~ ^[0-9]{6}$ ]] || fail "totp code"
[[ "$("$VLT" read 'vlt://github/login?attribute=otp')" =~ ^[0-9]{6}$ ]] || fail "otp attribute"

printf 'user={{ vlt://github/login/username }}\nkey={{ vlt://openai/api-key }}\n' > "$WORK/tpl"
"$VLT" inject -i "$WORK/tpl" -o "$WORK/out" 2>/dev/null
expect_eq "$(cat "$WORK/out")" $'user=alice\nkey=sk-test' "inject"
expect_eq "$(stat -f %Lp "$WORK/out")" "600" "inject output is private"

printf 'API_KEY=vlt://openai/api-key\nPLAIN=hello\n' > "$WORK/.env"
expect_eq "$(GH_USER=vlt://github/login/username "$VLT" run --env-file "$WORK/.env" -- sh -c 'echo "$API_KEY/$PLAIN/$GH_USER"')" "sk-test/hello/alice" "run --env-file"

head -c 64 /dev/urandom > "$WORK/bin"
"$VLT" set android/keystore --file "$WORK/bin" >/dev/null
"$VLT" get android/keystore --out "$WORK/bin.out" >/dev/null
cmp -s "$WORK/bin" "$WORK/bin.out" || fail "binary roundtrip"
"$VLT" get android/keystore >/dev/null 2>&1 && fail "document get without --out must fail"

"$VLT" show github/login | grep -q 'vlt://github/login/username' || fail "show prints references"
"$VLT" show github/login | grep -q 's3cret' && fail "show must mask secrets"
[[ "$("$VLT" generate --length 24)" =~ ^.{24}$ ]] || fail "generate"

"$VLT" delete openai/api-key >/dev/null
"$VLT" get openai/api-key >/dev/null 2>&1 && fail "deleted item still readable"
"$VLT" trash | grep -q openai/api-key || fail "trash lists deleted"
"$VLT" restore openai/api-key >/dev/null
expect_eq "$("$VLT" get openai/api-key)" "sk-test" "restore"

VLT_PASSPHRASE=pw "$VLT" export "$WORK/b.vltx" >/dev/null
export VLT_DB="$WORK/restore.db"
VLT_PASSPHRASE=pw "$VLT" import "$WORK/b.vltx" >/dev/null
expect_eq "$("$VLT" read vlt://github/login/username)" "alice" "export/import keeps structured items"

echo "CLI E2E OK"
