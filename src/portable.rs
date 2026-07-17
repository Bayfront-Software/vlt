//! 可搬バックアップ形式 (.vltx) の封緘・開封。
//!
//! vault.db のマスターキーは OS Keychain にしか存在しないため、
//! マシンが失われると vault.db 単体では復元できない。.vltx は
//! パスフレーズだけで別マシンでも復元できる持ち出し用フォーマット。
//!
//! 形式: JSON envelope
//!   { format: "vltx", version: 1, kdf: {algo, m_cost, t_cost, p_cost},
//!     salt: base64, payload: base64(nonce || AES-256-GCM ciphertext) }
//! payload の平文は Entry の JSON 配列。
//! version を上げる変更をしたら、旧 version の読み込みを必ず残すこと。

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

use crate::crypto;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    /// 値本体。テキストの場合も UTF-8 バイト列として持つ。
    #[serde(with = "b64_bytes")]
    pub value: Vec<u8>,
    pub binary: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize)]
struct KdfParams {
    algo: String,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    kdf: KdfParams,
    salt: String,
    payload: String,
}

/// entries をパスフレーズで封緘して .vltx バイト列にする。
pub fn seal(entries: &[Entry], passphrase: &str) -> Result<Vec<u8>, String> {
    let salt = crypto::generate_salt();
    let key = crypto::derive_key(passphrase, &salt)?;
    let plaintext =
        serde_json::to_vec(entries).map_err(|e| format!("Serialize error: {e}"))?;
    let sealed = crypto::encrypt(&key, &plaintext)?;

    let envelope = Envelope {
        format: "vltx".to_string(),
        version: FORMAT_VERSION,
        kdf: KdfParams {
            algo: "argon2id".to_string(),
            m_cost: crypto::KDF_M_COST_KIB,
            t_cost: crypto::KDF_T_COST,
            p_cost: crypto::KDF_P_COST,
        },
        salt: B64.encode(salt),
        payload: B64.encode(sealed),
    };
    serde_json::to_vec_pretty(&envelope).map_err(|e| format!("Serialize error: {e}"))
}

/// .vltx バイト列をパスフレーズで開封して entries に戻す。
pub fn unseal(data: &[u8], passphrase: &str) -> Result<Vec<Entry>, String> {
    let envelope: Envelope =
        serde_json::from_slice(data).map_err(|e| format!("Not a vltx file: {e}"))?;
    if envelope.format != "vltx" {
        return Err(format!("Not a vltx file (format: {})", envelope.format));
    }
    if envelope.version != FORMAT_VERSION {
        return Err(format!(
            "Unsupported vltx version {} (this vlt supports {})",
            envelope.version, FORMAT_VERSION
        ));
    }
    if envelope.kdf.algo != "argon2id" {
        return Err(format!("Unsupported KDF: {}", envelope.kdf.algo));
    }

    let salt = B64
        .decode(&envelope.salt)
        .map_err(|e| format!("Corrupt salt: {e}"))?;
    let payload = B64
        .decode(&envelope.payload)
        .map_err(|e| format!("Corrupt payload: {e}"))?;

    let key = crypto::derive_key(passphrase, &salt)?;
    let plaintext = crypto::decrypt(&key, &payload)
        .map_err(|_| "Wrong passphrase or corrupted file".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|e| format!("Corrupt entries: {e}"))
}

/// serde 用: Vec<u8> <-> base64 文字列
mod b64_bytes {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&B64.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        B64.decode(s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<Entry> {
        vec![
            Entry {
                key: "android/hushcam/store-password".into(),
                value: b"p@ssw0rd".to_vec(),
                binary: false,
                created_at: "2026-07-18 00:00:00".into(),
                updated_at: "2026-07-18 00:00:00".into(),
            },
            Entry {
                key: "android/hushcam/upload-keystore".into(),
                // バイナリ (非UTF-8) を含む
                value: vec![0xfe, 0xed, 0xfe, 0xed, 0x00, 0x01, 0xff],
                binary: true,
                created_at: "2026-07-18 00:00:00".into(),
                updated_at: "2026-07-18 00:00:01".into(),
            },
        ]
    }

    #[test]
    fn seal_unseal_roundtrip_preserves_entries() {
        let entries = sample_entries();
        let sealed = seal(&entries, "test passphrase").unwrap();
        let opened = unseal(&sealed, "test passphrase").unwrap();
        assert_eq!(opened, entries);
    }

    #[test]
    fn unseal_with_wrong_passphrase_fails() {
        let sealed = seal(&sample_entries(), "right").unwrap();
        let err = unseal(&sealed, "wrong").unwrap_err();
        assert!(err.contains("Wrong passphrase"), "unexpected error: {err}");
    }

    #[test]
    fn sealed_bytes_do_not_leak_plaintext() {
        let sealed = seal(&sample_entries(), "test passphrase").unwrap();
        let text = String::from_utf8_lossy(&sealed);
        assert!(!text.contains("p@ssw0rd"));
        assert!(!text.contains("store-password"));
    }

    #[test]
    fn unseal_rejects_unknown_version() {
        let sealed = seal(&sample_entries(), "pw").unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&sealed).unwrap();
        v["version"] = serde_json::json!(999);
        let err = unseal(&serde_json::to_vec(&v).unwrap(), "pw").unwrap_err();
        assert!(err.contains("version"), "unexpected error: {err}");
    }

    #[test]
    fn unseal_rejects_non_vltx_json() {
        let err = unseal(br#"{"hello":"world"}"#, "pw").unwrap_err();
        assert!(err.contains("vltx"), "unexpected error: {err}");
    }
}
