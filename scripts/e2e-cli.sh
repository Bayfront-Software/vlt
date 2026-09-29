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
# 出力が端末でないので、秘密（API_KEY）は伏せ字、ユーザー名（秘密でない）はそのまま
expect_eq "$(GH_USER=vlt://github/login/username "$VLT" run --env-file "$WORK/.env" -- sh -c 'echo "$API_KEY/$PLAIN/$GH_USER"')" "<concealed by vlt>/hello/alice" "run masks secrets in non-tty output"
expect_eq "$("$VLT" run --no-masking --env-file "$WORK/.env" -- sh -c 'echo "$API_KEY"')" "sk-test" "run --no-masking"
expect_eq "$("$VLT" run --env-file "$WORK/.env" -- sh -c 'test "$API_KEY" = sk-test && echo injected')" "injected" "masking does not change the injected value"
"$VLT" run --env-file "$WORK/.env" -- sh -c 'exit 7' && fail "exit code must propagate"; [[ $? -eq 7 ]] || fail "exit code 7"

# 環境変数の項目（.env をディスクに置かずに注入）＋ ${APP_ENV} の展開
"$VLT" set db/prod/password prod-pw >/dev/null
"$VLT" set db/dev/password dev-pw >/dev/null
"$VLT" set envs/myapp 'postgres://u:p@db/app' --type environment --field DATABASE_URL >/dev/null
"$VLT" set envs/myapp 'vlt://db/${APP_ENV:-dev}/password' --field DB_PASSWORD >/dev/null
"$VLT" set envs/myapp bad --field 'NOT-VALID' >/dev/null 2>&1 && fail "invalid env var name must be rejected"
"$VLT" set envs/myapp 3000 --field PORT --plain >/dev/null
expect_eq "$("$VLT" run --env envs/myapp -- sh -c 'echo "port=$PORT"')" "port=3000" "--plain values are not masked"
grep -q 'vlt://envs/myapp/DATABASE_URL' <<<"$("$VLT" show envs/myapp)" || fail "show uses variable names for environment items"
expect_eq "$("$VLT" read vlt://envs/myapp/DATABASE_URL)" "postgres://u:p@db/app" "reference by variable name"
expect_eq "$("$VLT" run --no-masking --env envs/myapp -- sh -c 'echo "$DATABASE_URL $DB_PASSWORD"')" "postgres://u:p@db/app dev-pw" "run --env"
expect_eq "$(APP_ENV=prod "$VLT" run --no-masking --env envs/myapp -- sh -c 'echo "$DB_PASSWORD"')" "prod-pw" "\${APP_ENV} in references"
expect_eq "$("$VLT" run --env envs/myapp -- sh -c 'echo "$DB_PASSWORD"')" "<concealed by vlt>" "env item secrets are masked"
grep -q "^export DATABASE_URL='postgres://u:p@db/app'$" <<<"$("$VLT" env --env envs/myapp)" || fail "env --env prints exports"
"$VLT" run --env github/login -- true 2>/dev/null && fail "non-environment item must be rejected"

"$VLT" read vlt://openai/api-key --out "$WORK/key.txt" 2>/dev/null
expect_eq "$(cat "$WORK/key.txt")" "sk-test" "read --out"
expect_eq "$(stat -f %Lp "$WORK/key.txt")" "600" "read --out is private"
"$VLT" list --json | python3 -c 'import json,sys; d=json.load(sys.stdin); assert any(r["key"]=="envs/myapp" and r["type"]=="environment" for r in d)' || fail "list --json"
"$VLT" show github/login --json | python3 -c 'import json,sys; d=json.load(sys.stdin); f={x["id"]:x for x in d["fields"]}; assert f["password"]["value"] is None and f["username"]["value"]=="alice" and f["username"]["reference"]=="vlt://github/login/username"' || fail "show --json masks secrets"
grep -q '#compdef vlt' <<<"$("$VLT" completions zsh)" || fail "completions"

head -c 64 /dev/urandom > "$WORK/bin"
"$VLT" set android/keystore --file "$WORK/bin" >/dev/null
"$VLT" get android/keystore --out "$WORK/bin.out" >/dev/null
cmp -s "$WORK/bin" "$WORK/bin.out" || fail "binary roundtrip"
"$VLT" get android/keystore >/dev/null 2>&1 && fail "document get without --out must fail"

grep -q 'vlt://github/login/username' <<<"$("$VLT" show github/login)" || fail "show prints references"
grep -q 's3cret' <<<"$("$VLT" show github/login)" && fail "show must mask secrets"
[[ "$("$VLT" generate --length 24)" =~ ^.{24}$ ]] || fail "generate"

"$VLT" delete openai/api-key >/dev/null
"$VLT" get openai/api-key >/dev/null 2>&1 && fail "deleted item still readable"
grep -q openai/api-key <<<"$("$VLT" trash)" || fail "trash lists deleted"
"$VLT" restore openai/api-key >/dev/null
expect_eq "$("$VLT" get openai/api-key)" "sk-test" "restore"

VLT_PASSPHRASE=pw "$VLT" export "$WORK/b.vltx" >/dev/null
export VLT_DB="$WORK/restore.db"
VLT_PASSPHRASE=pw "$VLT" import "$WORK/b.vltx" >/dev/null
expect_eq "$("$VLT" read vlt://github/login/username)" "alice" "export/import keeps structured items"

echo "CLI E2E OK"
