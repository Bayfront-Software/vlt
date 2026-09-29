//! パスワード生成と強度の目安。乱数は OS の CSPRNG（OsRng）を使う。

use rand::rngs::OsRng;
use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

const LOWER: &str = "abcdefghijkmnopqrstuvwxyz"; // l を除く
const UPPER: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ"; // I・O を除く
const DIGITS: &str = "23456789"; // 0・1 を除く（読み間違い防止）
const SYMBOLS: &str = "!@#$%^&*-_=+?";

pub const MIN_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorOptions {
    pub length: usize,
    pub digits: bool,
    pub symbols: bool,
    /// true なら数字だけ（PIN）。
    #[serde(default)]
    pub pin: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 20,
            digits: true,
            symbols: true,
            pin: false,
        }
    }
}

/// 有効な文字種をそれぞれ最低1文字は含むパスワードを作る。
pub fn generate(options: &GeneratorOptions) -> Result<String, String> {
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&options.length) {
        return Err(format!("長さは {MIN_LENGTH}〜{MAX_LENGTH} にしてください"));
    }
    let mut rng = OsRng;
    if options.pin {
        return Ok((0..options.length)
            .map(|_| char::from(b'0' + rng.gen_range(0..10)))
            .collect());
    }
    let mut classes: Vec<&str> = vec![LOWER, UPPER];
    if options.digits {
        classes.push(DIGITS);
    }
    if options.symbols {
        classes.push(SYMBOLS);
    }
    let all: Vec<char> = classes.concat().chars().collect();
    let mut chars: Vec<char> = classes
        .iter()
        .map(|class| {
            let pool: Vec<char> = class.chars().collect();
            pool[rng.gen_range(0..pool.len())]
        })
        .collect();
    while chars.len() < options.length {
        chars.push(all[rng.gen_range(0..all.len())]);
    }
    chars.shuffle(&mut rng);
    Ok(chars.into_iter().collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    Weak,
    Fair,
    Strong,
    VeryStrong,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Self::Weak => "弱い",
            Self::Fair => "普通",
            Self::Strong => "強い",
            Self::VeryStrong => "とても強い",
        }
    }
}

/// 使われている文字種から見積もったエントロピー（ビット）。
/// 辞書語や繰り返しは考慮しない粗い目安で、「短すぎる」「文字種が少ない」を捕まえる用途。
pub fn entropy_bits(password: &str) -> f64 {
    let mut pool = 0u32;
    if password.chars().any(|c| c.is_ascii_lowercase()) {
        pool += 26;
    }
    if password.chars().any(|c| c.is_ascii_uppercase()) {
        pool += 26;
    }
    if password.chars().any(|c| c.is_ascii_digit()) {
        pool += 10;
    }
    if password.chars().any(|c| c.is_ascii_punctuation() || c == ' ') {
        pool += 33;
    }
    if password.chars().any(|c| !c.is_ascii()) {
        pool += 100;
    }
    let distinct = {
        let mut cs: Vec<char> = password.chars().collect();
        cs.sort_unstable();
        cs.dedup();
        cs.len()
    };
    if pool == 0 || distinct <= 1 {
        return 0.0;
    }
    password.chars().count() as f64 * f64::from(pool).log2()
}

pub fn strength(password: &str) -> Strength {
    match entropy_bits(password) {
        b if b < 40.0 => Strength::Weak,
        b if b < 64.0 => Strength::Fair,
        b if b < 100.0 => Strength::Strong,
        _ => Strength::VeryStrong,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_password_has_requested_length_and_every_class() {
        for _ in 0..200 {
            let pw = generate(&GeneratorOptions { length: 8, ..Default::default() }).unwrap();
            assert_eq!(pw.chars().count(), 8);
            assert!(pw.chars().any(|c| LOWER.contains(c)));
            assert!(pw.chars().any(|c| UPPER.contains(c)));
            assert!(pw.chars().any(|c| DIGITS.contains(c)));
            assert!(pw.chars().any(|c| SYMBOLS.contains(c)));
        }
    }

    #[test]
    fn disabled_classes_never_appear() {
        for _ in 0..200 {
            let pw = generate(&GeneratorOptions { length: 32, digits: false, symbols: false, pin: false }).unwrap();
            assert!(pw.chars().all(|c| c.is_ascii_alphabetic()), "{pw}");
        }
    }

    #[test]
    fn pin_is_digits_only() {
        let pin = generate(&GeneratorOptions { length: 6, pin: true, ..Default::default() }).unwrap();
        assert_eq!(pin.len(), 6);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn rejects_out_of_range_length() {
        assert!(generate(&GeneratorOptions { length: 3, ..Default::default() }).is_err());
        assert!(generate(&GeneratorOptions { length: 129, ..Default::default() }).is_err());
    }

    #[test]
    fn two_generations_differ() {
        let opts = GeneratorOptions::default();
        assert_ne!(generate(&opts).unwrap(), generate(&opts).unwrap());
    }

    #[test]
    fn strength_grades_short_simple_and_long_random() {
        assert_eq!(strength(""), Strength::Weak);
        assert_eq!(strength("aaaaaaaaaaaaaaaa"), Strength::Weak);
        assert_eq!(strength("password"), Strength::Weak);
        assert_eq!(strength("Tr0ub4dor&3x"), Strength::Strong);
        let generated = generate(&GeneratorOptions::default()).unwrap();
        assert!(matches!(strength(&generated), Strength::Strong | Strength::VeryStrong));
    }
}
