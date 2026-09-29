//! 環境変数まわりの純粋な部品（変数名の判定・参照の中の `${VAR}` 展開）。

/// シェルの環境変数名として使えるか（英字か _ で始まり、英数字と _ だけ）。
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `${NAME}` と `${NAME:-既定値}` を `lookup` の値で置き換える（1Password の参照と同じ書き方）。
/// 未定義で既定値も無ければエラー（黙って空にすると別の項目を指してしまうため）。
pub fn expand_vars(input: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| format!("閉じ括弧 }} がありません: {input}"))?;
        let inner = &after[..end];
        let (name, default) = match inner.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (inner, None),
        };
        if !is_env_name(name) {
            return Err(format!("変数名が不正です: ${{{inner}}}"));
        }
        let value = lookup(name)
            .filter(|v| !v.is_empty())
            .or_else(|| default.map(str::to_string))
            .ok_or_else(|| format!("環境変数 {name} が未定義です（既定値は ${{{name}:-dev}} のように書けます）"))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_names() {
        for ok in ["A", "_X", "DATABASE_URL", "api_key2"] {
            assert!(is_env_name(ok), "{ok}");
        }
        for bad in ["", "1A", "A-B", "A B", "ドメイン", "A.B"] {
            assert!(!is_env_name(bad), "{bad}");
        }
    }

    #[test]
    fn expands_defined_vars_and_defaults() {
        let lookup = |n: &str| (n == "APP_ENV").then(|| "prod".to_string());
        assert_eq!(expand_vars("vlt://db/${APP_ENV}/password", lookup).unwrap(), "vlt://db/prod/password");
        assert_eq!(expand_vars("vlt://db/${STAGE:-dev}/pw", lookup).unwrap(), "vlt://db/dev/pw");
        assert_eq!(expand_vars("no vars", lookup).unwrap(), "no vars");
    }

    #[test]
    fn empty_value_falls_back_to_default() {
        let lookup = |_: &str| Some(String::new());
        assert_eq!(expand_vars("${X:-d}", lookup).unwrap(), "d");
    }

    #[test]
    fn undefined_or_malformed_is_an_error() {
        let lookup = |_: &str| None;
        assert!(expand_vars("vlt://${MISSING}/x", lookup).unwrap_err().contains("MISSING"));
        assert!(expand_vars("vlt://${UNCLOSED", lookup).is_err());
        assert!(expand_vars("vlt://${bad-name}", lookup).is_err());
    }
}
