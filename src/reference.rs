//! 秘密参照（1Password の `op://vault/item/field` に相当）。
//!
//! 形式: `vlt://<項目のキー>[/<フィールド>][?attribute=otp]`
//!   - `vlt://openai/api-key`             … 項目の主役フィールド（パスワード・認証情報など）
//!   - `vlt://github/login/username`      … フィールドを id かラベルで指定
//!   - `vlt://github/login/notes`         … メモ欄
//!   - `vlt://github/login?attribute=otp` … ワンタイムパスワードの現在のコード
//!
//! キー自体が `/` を含むので、「パス全体が項目のキー」をまず探し、無ければ最後の
//! `/` で分けて「キー + フィールド」と読む。完全一致が常に優先なので、v0.2 までの
//! `vlt://<キー>` はそのまま動く。

use std::collections::HashMap;

use crate::env::{expand_vars, is_env_name};
use crate::item::{FieldKind, Item, ItemType, PrimaryValue, NOTES_FIELD};
use crate::store::SecretStore;
use crate::totp;

pub const PREFIX: &str = "vlt://";

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub path: String,
    pub attribute: Option<String>,
}

pub fn is_reference(value: &str) -> bool {
    value.starts_with(PREFIX)
}

pub fn parse(value: &str) -> Result<Reference, String> {
    let body = value
        .strip_prefix(PREFIX)
        .ok_or_else(|| format!("秘密参照は {PREFIX} で始めてください: {value}"))?;
    let (path, query) = match body.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (body, None),
    };
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Err(format!("秘密参照に項目がありません: {value}"));
    }
    let mut attribute = None;
    if let Some(query) = query {
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            match pair.split_once('=') {
                Some(("attribute", v)) => attribute = Some(v.to_string()),
                _ => return Err(format!("秘密参照のクエリが不正です: {pair}")),
            }
        }
    }
    Ok(Reference {
        path: path.to_string(),
        attribute,
    })
}

/// 参照の組み立て（GUI の「秘密参照をコピー」）。主役フィールドなら省く。
pub fn build(key: &str, field: Option<&str>) -> String {
    match field {
        Some(f) => format!("{PREFIX}{key}/{f}"),
        None => format!("{PREFIX}{key}"),
    }
}

fn wants_otp(reference: &Reference) -> Result<bool, String> {
    match reference.attribute.as_deref() {
        None => Ok(false),
        Some("otp") | Some("totp") => Ok(true),
        Some(other) => Err(format!("未対応の attribute です: {other}（otp のみ）")),
    }
}

fn locate<'a>(store: &SecretStore, path: &'a str) -> Result<(&'a str, Option<&'a str>), String> {
    if store.exists(path)? {
        return Ok((path, None));
    }
    if let Some((key, field)) = path.rsplit_once('/') {
        if store.exists(key)? {
            return Ok((key, Some(field)));
        }
    }
    Err(format!("項目が見つかりません: {PREFIX}{path}"))
}

fn otp_code(item: &Item, field_name: Option<&str>) -> Result<String, String> {
    let field = match field_name {
        Some(name) => item.field(name).ok_or_else(|| format!("フィールドがありません: {name}"))?,
        None => item
            .fields
            .iter()
            .find(|f| f.kind == FieldKind::Totp && !f.value.is_empty())
            .ok_or("この項目にワンタイムパスワードがありません")?,
    };
    if field.kind != FieldKind::Totp {
        return Err(format!("{} はワンタイムパスワードではありません", field.label));
    }
    Ok(totp::current_code(&field.value)?.code)
}

/// 解決した値と、それが秘密か（`vlt run` の出力で伏せ字にするか）。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub value: String,
    pub secret: bool,
}

pub fn resolve(store: &SecretStore, reference: &Reference) -> Result<String, String> {
    Ok(resolve_detailed(store, reference)?.value)
}

pub fn resolve_detailed(store: &SecretStore, reference: &Reference) -> Result<Resolved, String> {
    let otp = wants_otp(reference)?;
    let (key, field_name) = locate(store, &reference.path)?;
    let item = store.get_item(key)?;
    if otp {
        return Ok(Resolved { value: otp_code(&item, field_name)?, secret: true });
    }
    match field_name {
        None => match item.primary() {
            Some(PrimaryValue::Text(text)) => {
                // 主役がメモ欄（セキュアノート）なら秘密として扱う
                let secret = item
                    .item_type
                    .primary_field()
                    .and_then(|id| item.field(id))
                    .map_or(true, |f| f.kind.is_secret());
                Ok(Resolved { value: text.to_string(), secret })
            }
            Some(PrimaryValue::File(_)) => Err(format!(
                "{key} はファイルなので参照では渡せません。`vlt get {key} --out <path>` を使ってください"
            )),
            None => Err(format!(
                "{key} は{}で主役のフィールドがありません。{PREFIX}{key}/<フィールド> で指定してください（{}）",
                item.item_type.label(),
                available_fields(&item)
            )),
        },
        Some(name) => {
            if let Some(field) = item.field(name) {
                if field.kind == FieldKind::File {
                    return Err(format!("{key}/{name} はファイルなので参照では渡せません"));
                }
                return Ok(Resolved { value: field.value.clone(), secret: field.kind.is_secret() });
            }
            if name == NOTES_FIELD {
                return Ok(Resolved { value: item.notes.clone(), secret: false });
            }
            Err(format!(
                "{key} にフィールド {name} がありません（{}）",
                available_fields(&item)
            ))
        }
    }
}

fn available_fields(item: &Item) -> String {
    let mut ids: Vec<&str> = item.fields.iter().map(|f| f.id.as_str()).collect();
    ids.push(NOTES_FIELD);
    format!("使えるフィールド: {}", ids.join(", "))
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// 参照の中の `${VAR}` を展開してから読む。参照でない文字列には触れない。
fn parse_expanded(value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Reference, String> {
    parse(&expand_vars(value, lookup)?)
}

/// 値が参照なら解決し、そうでなければそのまま返す（`${VAR}` は `lookup` で展開）。
pub fn resolve_value_with(
    store: &SecretStore,
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    if is_reference(value) {
        resolve(store, &parse_expanded(value, lookup)?)
    } else {
        Ok(value.to_string())
    }
}

/// 値が参照なら解決する（`${VAR}` はこのプロセスの環境変数で展開）。
pub fn resolve_value(store: &SecretStore, value: &str) -> Result<String, String> {
    resolve_value_with(store, value, &process_env)
}

/// `vlt run` / `vlt env` に渡す1つの環境変数。
#[derive(Debug, Clone, PartialEq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    /// 出力で伏せ字にする値か。
    pub secret: bool,
    /// vlt が値を与えた（参照を解決した・環境変数の項目から来た）か。`vlt env` はこれだけを出す。
    pub from_vlt: bool,
}

fn upsert(vars: &mut Vec<EnvVar>, var: EnvVar) {
    vars.retain(|v| v.name != var.name);
    vars.push(var);
}

/// 注入する環境を組み立てる。
/// 1. `base`（このプロセスの環境変数と .env ファイル。後ろが勝つ）
/// 2. `env_items` の環境変数の項目（後ろが勝つ）
/// 3. 値が `vlt://` の参照なら解決する。参照の中の `${VAR}` は 1〜2 の値（解決前）で展開する。
pub fn build_env(
    store: &SecretStore,
    base: Vec<(String, String)>,
    env_items: &[String],
) -> Result<Vec<EnvVar>, String> {
    let mut vars: Vec<EnvVar> = Vec::new();
    for (name, value) in base {
        upsert(&mut vars, EnvVar { name, value, secret: false, from_vlt: false });
    }
    for key in env_items {
        let item = store.get_item(key)?;
        if item.item_type != ItemType::Environment {
            return Err(format!(
                "{key} は環境変数の項目ではありません（種類: {}）",
                item.item_type.label()
            ));
        }
        for field in &item.fields {
            if !is_env_name(&field.label) {
                return Err(format!("{key} の「{}」は環境変数名として使えません（英字か _ で始まり、英数字と _ だけ）", field.label));
            }
            if field.kind == FieldKind::File {
                return Err(format!("{key} の {} はファイルなので環境変数にできません", field.label));
            }
            upsert(
                &mut vars,
                EnvVar { name: field.label.clone(), value: field.value.clone(), secret: field.kind.is_secret(), from_vlt: true },
            );
        }
    }
    let snapshot: HashMap<String, String> = vars.iter().map(|v| (v.name.clone(), v.value.clone())).collect();
    let lookup = |name: &str| snapshot.get(name).cloned();
    for var in vars.iter_mut().filter(|v| is_reference(&v.value)) {
        let reference = parse_expanded(&var.value, &lookup).map_err(|e| format!("{}: {e}", var.name))?;
        let resolved = resolve_detailed(store, &reference).map_err(|e| format!("{}: {e}", var.name))?;
        var.value = resolved.value;
        var.secret |= resolved.secret;
        var.from_vlt = true;
    }
    Ok(vars)
}

/// 環境変数のうち値が参照のものだけを解決した対応表を返す。
pub fn resolve_env(
    store: &SecretStore,
    vars: impl IntoIterator<Item = (String, String)>,
) -> Result<HashMap<String, String>, String> {
    Ok(build_env(store, vars.into_iter().collect(), &[])?
        .into_iter()
        .filter(|v| v.from_vlt)
        .map(|v| (v.name, v.value))
        .collect())
}

/// テンプレート中の `{{ vlt://... }}` を値に置き換える（1Password の `op inject`）。
/// `vlt://` で始まらない `{{ ... }}` は他のテンプレートエンジン用とみなして触らない。
pub fn inject(store: &SecretStore, template: &str) -> Result<String, String> {
    inject_with(store, template, &process_env)
}

pub fn inject_with(
    store: &SecretStore,
    template: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start + 2..].find("}}") else { break };
        let inner = rest[start + 2..start + 2 + len].trim();
        out.push_str(&rest[..start]);
        if is_reference(inner) {
            out.push_str(&resolve(store, &parse_expanded(inner, lookup)?)?);
        } else {
            out.push_str(&rest[start..start + 2 + len + 2]);
        }
        rest = &rest[start + 2 + len + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `.env` 形式を読む（`KEY=VALUE`、`export ` 前置き、`#` コメント、囲み引用符）。
pub fn parse_env_file(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut vars = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("{} 行目に = がありません: {raw}", lineno + 1))?;
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("{} 行目の変数名が不正です: {name}", lineno + 1));
        }
        let value = value.trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        vars.push((name.to_string(), value.to_string()));
    }
    Ok(vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_master_key;
    use crate::item::Field;
    use std::path::PathBuf;

    struct TempDb(PathBuf);
    impl TempDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("vlt-ref-{name}-{}.db", std::process::id()));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn fixture(name: &str) -> (TempDb, SecretStore) {
        let db = TempDb::new(name);
        let store = SecretStore::open_at(&db.0, generate_master_key()).unwrap();
        let mut login = Item::new(ItemType::Login);
        login.field_mut("username").unwrap().value = "alice".into();
        login.field_mut("password").unwrap().value = "pw".into();
        login.field_mut("one_time_password").unwrap().value = "GEZDGNBVGY3TQOJQ".into();
        login.fields.push(Field { value: "R-123".into(), ..Field::new("recovery", "Recovery Code", FieldKind::Concealed) });
        login.notes = "memo".into();
        store.put_item("dev/github", &login).unwrap();
        store.set("dev/legacy-token", "tok").unwrap();
        store.set_bytes("dev/keystore", &[1, 2, 3], true).unwrap();
        store.put_item("dev/me", &Item::new(ItemType::Identity)).unwrap();
        (db, store)
    }

    fn r(store: &SecretStore, s: &str) -> Result<String, String> {
        resolve_value(store, s)
    }

    #[test]
    fn parse_splits_path_and_attribute() {
        assert_eq!(parse("vlt://a/b").unwrap(), Reference { path: "a/b".into(), attribute: None });
        assert_eq!(parse("vlt://a/b/?attribute=otp").unwrap().attribute.as_deref(), Some("otp"));
        assert!(parse("vlt://").is_err());
        assert!(parse("op://a/b").is_err());
        assert!(parse("vlt://a?foo=bar").is_err());
    }

    #[test]
    fn whole_key_resolves_to_primary_field() {
        let (_db, store) = fixture("primary");
        assert_eq!(r(&store, "vlt://dev/github").unwrap(), "pw");
        assert_eq!(r(&store, "vlt://dev/legacy-token").unwrap(), "tok");
    }

    #[test]
    fn trailing_segment_is_field_by_id_or_label() {
        let (_db, store) = fixture("field");
        assert_eq!(r(&store, "vlt://dev/github/username").unwrap(), "alice");
        assert_eq!(r(&store, "vlt://dev/github/recovery code").unwrap(), "R-123");
        assert_eq!(r(&store, "vlt://dev/github/notes").unwrap(), "memo");
        let err = r(&store, "vlt://dev/github/nope").unwrap_err();
        assert!(err.contains("username"), "候補を示す: {err}");
    }

    #[test]
    fn exact_key_wins_over_key_plus_field() {
        let (_db, store) = fixture("exact");
        store.set("dev/github/username", "separate-item").unwrap();
        assert_eq!(r(&store, "vlt://dev/github/username").unwrap(), "separate-item");
    }

    #[test]
    fn otp_attribute_returns_current_code() {
        let (_db, store) = fixture("otp");
        let code = r(&store, "vlt://dev/github?attribute=otp").unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(r(&store, "vlt://dev/github/one_time_password?attribute=otp").unwrap().len(), 6);
        assert_eq!(r(&store, "vlt://dev/github/one_time_password").unwrap(), "GEZDGNBVGY3TQOJQ");
        assert!(r(&store, "vlt://dev/github/username?attribute=otp").is_err());
        assert!(r(&store, "vlt://dev/legacy-token?attribute=otp").is_err());
        assert!(r(&store, "vlt://dev/github?attribute=nope").is_err());
    }

    #[test]
    fn files_and_primary_less_items_are_refused_with_guidance() {
        let (_db, store) = fixture("refuse");
        assert!(r(&store, "vlt://dev/keystore").unwrap_err().contains("--out"));
        assert!(r(&store, "vlt://dev/me").unwrap_err().contains("last_name"));
        assert!(r(&store, "vlt://nothing/here").unwrap_err().contains("見つかりません"));
    }

    #[test]
    fn non_references_pass_through() {
        let (_db, store) = fixture("plain");
        assert_eq!(r(&store, "plain value").unwrap(), "plain value");
    }

    #[test]
    fn resolve_env_only_touches_references_and_names_the_failing_var() {
        let (_db, store) = fixture("env");
        let vars = vec![
            ("TOKEN".to_string(), "vlt://dev/legacy-token".to_string()),
            ("USER".to_string(), "vlt://dev/github/username".to_string()),
            ("HOME".to_string(), "/Users/x".to_string()),
        ];
        let resolved = resolve_env(&store, vars).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved["USER"], "alice");
        let err = resolve_env(&store, vec![("BAD".into(), "vlt://nope".into())]).unwrap_err();
        assert!(err.starts_with("BAD:"), "{err}");
    }

    #[test]
    fn inject_replaces_only_vlt_placeholders() {
        let (_db, store) = fixture("inject");
        let out = inject(&store, "user: {{ vlt://dev/github/username }}\npass: {{vlt://dev/github}}\nkeep: {{ other }}\n").unwrap();
        assert_eq!(out, "user: alice\npass: pw\nkeep: {{ other }}\n");
        assert!(inject(&store, "x={{ vlt://missing }}").is_err());
        assert_eq!(inject(&store, "no refs {{ unclosed").unwrap(), "no refs {{ unclosed");
    }

    #[test]
    fn env_file_parsing() {
        let vars = parse_env_file("# comment\n\nexport A=1\nB = \"vlt://x/y\"\nC='q r'\nD=\n").unwrap();
        assert_eq!(
            vars,
            vec![
                ("A".into(), "1".into()),
                ("B".into(), "vlt://x/y".into()),
                ("C".into(), "q r".into()),
                ("D".into(), "".into()),
            ]
        );
        assert!(parse_env_file("NOEQUALS").is_err());
        assert!(parse_env_file("BAD NAME=1").is_err());
    }

    #[test]
    fn resolve_detailed_reports_whether_the_value_is_secret() {
        let (_db, store) = fixture("detailed");
        let d = |s: &str| resolve_detailed(&store, &parse(s).unwrap()).unwrap();
        assert!(d("vlt://dev/github").secret);
        assert!(!d("vlt://dev/github/username").secret);
        assert!(d("vlt://dev/github/recovery code").secret);
        assert!(d("vlt://dev/github?attribute=otp").secret);
        assert!(!d("vlt://dev/github/notes").secret);
    }

    #[test]
    fn references_expand_env_vars_before_resolving() {
        let (_db, store) = fixture("expand");
        store.set("db/prod/password", "prod-pw").unwrap();
        store.set("db/dev/password", "dev-pw").unwrap();
        let lookup = |n: &str| (n == "APP_ENV").then(|| "prod".to_string());
        assert_eq!(resolve_value_with(&store, "vlt://db/${APP_ENV}/password", &lookup).unwrap(), "prod-pw");
        assert_eq!(resolve_value_with(&store, "vlt://db/${STAGE:-dev}/password", &lookup).unwrap(), "dev-pw");
        let out = inject_with(&store, "pw={{ vlt://db/${APP_ENV}/password }}", &lookup).unwrap();
        assert_eq!(out, "pw=prod-pw");
        assert!(resolve_value_with(&store, "vlt://db/${NOPE}/password", &lookup).is_err());
        // 参照でない文字列の ${} には触れない
        assert_eq!(resolve_value_with(&store, "${HOME}/x", &lookup).unwrap(), "${HOME}/x");
    }

    fn env_item(store: &SecretStore, key: &str, vars: &[(&str, &str, FieldKind)]) {
        let mut item = Item::new(ItemType::Environment);
        for (name, value, kind) in vars {
            let id = item.unique_field_id(name);
            item.fields.push(Field { value: value.to_string(), ..Field::new(&id, name, *kind) });
        }
        store.put_item(key, &item).unwrap();
    }

    fn find<'a>(vars: &'a [EnvVar], name: &str) -> &'a EnvVar {
        vars.iter().find(|v| v.name == name).unwrap_or_else(|| panic!("{name} が無い"))
    }

    #[test]
    fn build_env_merges_base_env_items_and_resolves_references() {
        let (_db, store) = fixture("build-env");
        env_item(&store, "envs/app", &[
            ("DATABASE_URL", "postgres://u:p@db/app", FieldKind::Concealed),
            ("PORT", "5432", FieldKind::Text),
            ("GH_USER", "vlt://dev/github/username", FieldKind::Text),
            ("GH_TOKEN", "vlt://dev/github", FieldKind::Text),
        ]);
        let base = vec![
            ("HOME".to_string(), "/Users/x".to_string()),
            ("PORT".to_string(), "80".to_string()),
            ("LEGACY".to_string(), "vlt://dev/legacy-token".to_string()),
        ];
        let vars = build_env(&store, base, &["envs/app".to_string()]).unwrap();
        assert_eq!(find(&vars, "HOME").value, "/Users/x");
        assert!(!find(&vars, "HOME").from_vlt);
        assert_eq!(find(&vars, "PORT").value, "5432", "項目の値が後勝ち");
        assert!(!find(&vars, "PORT").secret);
        assert!(find(&vars, "DATABASE_URL").secret, "伏せる種類の欄は秘密");
        assert_eq!(find(&vars, "GH_USER").value, "alice");
        assert!(!find(&vars, "GH_USER").secret, "参照先がユーザー名なら秘密ではない");
        assert_eq!(find(&vars, "GH_TOKEN").value, "pw");
        assert!(find(&vars, "GH_TOKEN").secret, "参照先がパスワードなら秘密");
        assert_eq!(find(&vars, "LEGACY").value, "tok");
        assert!(find(&vars, "LEGACY").from_vlt);
        assert_eq!(vars.iter().filter(|v| v.name == "PORT").count(), 1, "同名は1つにまとめる");
    }

    #[test]
    fn build_env_expands_vars_using_earlier_definitions() {
        let (_db, store) = fixture("build-env-expand");
        store.set("db/staging/password", "stg").unwrap();
        env_item(&store, "envs/db", &[("DB_PASSWORD", "vlt://db/${APP_ENV}/password", FieldKind::Text)]);
        let base = vec![("APP_ENV".to_string(), "staging".to_string())];
        let vars = build_env(&store, base, &["envs/db".to_string()]).unwrap();
        assert_eq!(find(&vars, "DB_PASSWORD").value, "stg");
    }

    #[test]
    fn build_env_rejects_non_environment_items_and_bad_names() {
        let (_db, store) = fixture("build-env-bad");
        assert!(build_env(&store, vec![], &["dev/github".to_string()]).unwrap_err().contains("環境変数"));
        env_item(&store, "envs/bad", &[("NOT-VALID", "x", FieldKind::Text)]);
        assert!(build_env(&store, vec![], &["envs/bad".to_string()]).unwrap_err().contains("NOT-VALID"));
        assert!(build_env(&store, vec![], &["envs/missing".to_string()]).is_err());
    }

    #[test]
    fn build_omits_field_for_primary() {
        assert_eq!(build("a/b", None), "vlt://a/b");
        assert_eq!(build("a/b", Some("username")), "vlt://a/b/username");
    }
}
