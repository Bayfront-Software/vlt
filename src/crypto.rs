use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

const NONCE_SIZE: usize = 12;

/// Argon2id パラメータ（export ファイルの KDF）。
/// この値を変えると既存 .vltx が開けなくなるため、変更時は必ず
/// portable::FORMAT_VERSION を上げて旧パラメータでの読み込みを残すこと。
pub const KDF_M_COST_KIB: u32 = 65536; // 64 MiB
pub const KDF_T_COST: u32 = 3;
pub const KDF_P_COST: u32 = 4;
pub const SALT_SIZE: usize = 16;

/// パスフレーズと salt から Argon2id で 32 byte 鍵を導出する。
pub fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(KDF_M_COST_KIB, KDF_T_COST, KDF_P_COST, Some(32))
        .map_err(|e| format!("KDF params error: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| format!("Key derivation error: {e}"))?;
    Ok(key)
}

pub fn generate_salt() -> [u8; SALT_SIZE] {
    let mut salt = [0u8; SALT_SIZE];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

pub fn generate_master_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Cipher init error: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption error: {e}"))?;

    // prepend nonce to ciphertext
    let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < NONCE_SIZE {
        return Err("Data too short to contain nonce".to_string());
    }

    let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Cipher init error: {e}"))?;
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = generate_master_key();
        let plain = b"secret value \xf0\x9f\x94\x91";
        let sealed = encrypt(&key, plain).unwrap();
        assert_ne!(&sealed[NONCE_SIZE..], plain.as_slice());
        let opened = decrypt(&key, &sealed).unwrap();
        assert_eq!(opened, plain);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let sealed = encrypt(&generate_master_key(), b"data").unwrap();
        assert!(decrypt(&generate_master_key(), &sealed).is_err());
    }

    #[test]
    fn decrypt_tampered_ciphertext_fails() {
        let key = generate_master_key();
        let mut sealed = encrypt(&key, b"data").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(decrypt(&key, &sealed).is_err());
    }

    #[test]
    fn derive_key_is_deterministic_per_salt() {
        let salt = [7u8; SALT_SIZE];
        let a = derive_key("correct horse battery staple", &salt).unwrap();
        let b = derive_key("correct horse battery staple", &salt).unwrap();
        assert_eq!(a, b);

        let other_salt = [8u8; SALT_SIZE];
        let c = derive_key("correct horse battery staple", &other_salt).unwrap();
        assert_ne!(a, c);

        let d = derive_key("different passphrase", &salt).unwrap();
        assert_ne!(a, d);
    }
}
