//! ワンタイムパスワード（TOTP, RFC 6238）。
//!
//! 値は `otpauth://totp/...?secret=...` の URI か、base32 の秘密そのもの（空白・小文字可）。
//! サービスの QR コードの中身がそのまま URI なので、それを貼れば動く。

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TotpConfig {
    pub secret: Vec<u8>,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TotpCode {
    pub code: String,
    /// 今のコードが切り替わるまでの秒数。
    pub remaining: u64,
    pub period: u64,
}

/// RFC 4648 base32（パディング・空白・小文字を許容）。
pub fn decode_base32(input: &str) -> Result<Vec<u8>, String> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bits: u64 = 0;
    let mut bit_count = 0;
    let mut out = Vec::new();
    for c in input.chars().filter(|c| !c.is_whitespace() && *c != '=' && *c != '-') {
        let upper = c.to_ascii_uppercase() as u8;
        let value = ALPHABET
            .iter()
            .position(|&a| a == upper)
            .ok_or_else(|| format!("base32 として不正な文字です: {c}"))? as u64;
        bits = (bits << 5) | value;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
            bits &= (1 << bit_count) - 1;
        }
    }
    if out.is_empty() {
        return Err("秘密が空です".into());
    }
    Ok(out)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse(value: &str) -> Result<TotpConfig, String> {
    let value = value.trim();
    let mut config = TotpConfig {
        secret: vec![],
        algorithm: Algorithm::Sha1,
        digits: 6,
        period: 30,
    };
    let Some(rest) = value.strip_prefix("otpauth://") else {
        config.secret = decode_base32(value)?;
        return Ok(config);
    };
    if !rest.to_ascii_lowercase().starts_with("totp/") {
        return Err("otpauth URI は totp だけ対応しています（hotp は非対応）".into());
    }
    let query = rest.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut secret = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(decode_base32(&v)?),
            "algorithm" => {
                config.algorithm = match v.to_ascii_uppercase().as_str() {
                    "SHA1" => Algorithm::Sha1,
                    "SHA256" => Algorithm::Sha256,
                    "SHA512" => Algorithm::Sha512,
                    other => return Err(format!("未対応のアルゴリズムです: {other}")),
                }
            }
            "digits" => {
                config.digits = v.parse().map_err(|_| format!("digits が不正です: {v}"))?;
                if !(6..=10).contains(&config.digits) {
                    return Err(format!("digits は 6〜10 にしてください: {v}"));
                }
            }
            "period" => {
                config.period = v.parse().map_err(|_| format!("period が不正です: {v}"))?;
                if config.period == 0 {
                    return Err("period は 1 以上にしてください".into());
                }
            }
            _ => {}
        }
    }
    config.secret = secret.ok_or("otpauth URI に secret がありません")?;
    Ok(config)
}

fn hmac_digest(algorithm: Algorithm, key: &[u8], msg: &[u8]) -> Vec<u8> {
    macro_rules! run {
        ($h:ty) => {{
            let mut mac = <Hmac<$h>>::new_from_slice(key).expect("HMAC は任意長の鍵を受け付ける");
            mac.update(msg);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    match algorithm {
        Algorithm::Sha1 => run!(Sha1),
        Algorithm::Sha256 => run!(Sha256),
        Algorithm::Sha512 => run!(Sha512),
    }
}

/// UNIX 時刻 `unix_secs` 時点のコード。
pub fn code_at(config: &TotpConfig, unix_secs: u64) -> TotpCode {
    let counter = unix_secs / config.period;
    let digest = hmac_digest(config.algorithm, &config.secret, &counter.to_be_bytes());
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    let code = u64::from(binary) % 10u64.pow(config.digits);
    TotpCode {
        code: format!("{:0width$}", code, width = config.digits as usize),
        remaining: config.period - unix_secs % config.period,
        period: config.period,
    }
}

pub fn current_code(value: &str) -> Result<TotpCode, String> {
    let config = parse(value)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("時計が不正です: {e}"))?
        .as_secs();
    Ok(code_at(&config, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        let mut out = String::new();
        let (mut bits, mut n) = (0u64, 0);
        for &b in bytes {
            bits = (bits << 8) | u64::from(b);
            n += 8;
            while n >= 5 {
                n -= 5;
                out.push(ALPHABET[((bits >> n) & 31) as usize] as char);
            }
        }
        if n > 0 {
            out.push(ALPHABET[((bits << (5 - n)) & 31) as usize] as char);
        }
        out
    }

    // RFC 6238 付録 B のテストベクタ（8 桁）
    #[test]
    fn rfc6238_vectors_for_all_algorithms() {
        let cases = [
            (Algorithm::Sha1, b"12345678901234567890".to_vec(), "SHA1"),
            (Algorithm::Sha256, b"12345678901234567890123456789012".to_vec(), "SHA256"),
            (
                Algorithm::Sha512,
                b"1234567890123456789012345678901234567890123456789012345678901234".to_vec(),
                "SHA512",
            ),
        ];
        let expected = [
            (59u64, ["94287082", "46119246", "90693936"]),
            (1111111109, ["07081804", "68084774", "25091201"]),
            (2000000000, ["69279037", "90698825", "38618901"]),
        ];
        for (i, (algorithm, secret, name)) in cases.iter().enumerate() {
            let uri = format!("otpauth://totp/x?secret={}&algorithm={name}&digits=8", b32(secret));
            let config = parse(&uri).unwrap();
            assert_eq!(config.algorithm, *algorithm);
            for (t, codes) in expected {
                assert_eq!(code_at(&config, t).code, codes[i], "{name} at {t}");
            }
        }
    }

    #[test]
    fn bare_base32_secret_with_spaces_and_lowercase() {
        let config = parse("gezd gnbv gy3t qojq").unwrap();
        assert_eq!(config.secret, b"1234567890");
        assert_eq!((config.digits, config.period), (6, 30));
    }

    #[test]
    fn remaining_counts_down_to_next_period() {
        let config = parse("GEZDGNBVGY3TQOJQ").unwrap();
        assert_eq!(code_at(&config, 60).remaining, 30);
        assert_eq!(code_at(&config, 89).remaining, 1);
        assert_eq!(code_at(&config, 60).code.len(), 6);
    }

    #[test]
    fn rejects_hotp_missing_secret_and_garbage() {
        assert!(parse("otpauth://hotp/x?secret=GEZDGNBV").is_err());
        assert!(parse("otpauth://totp/x?issuer=y").is_err());
        assert!(parse("not base32!").is_err());
        assert!(parse("").is_err());
        assert!(parse("otpauth://totp/x?secret=GEZDGNBV&digits=3").is_err());
    }

    #[test]
    fn percent_encoded_label_does_not_break_parsing() {
        let config = parse("otpauth://totp/GitHub%3Aalice?secret=GEZDGNBVGY3TQOJQ&issuer=GitHub&period=60").unwrap();
        assert_eq!(config.period, 60);
    }
}
