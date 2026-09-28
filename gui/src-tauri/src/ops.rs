//! GUI の操作をまとめた層。Tauri のコマンドはこの関数を呼ぶだけにして、
//! ここは `SecretStore` だけを受け取る純粋な形に保つ（テストが一時 DB で回せる）。

use serde::Serialize;
use vlt::portable::{self, Entry};
use vlt::store::SecretStore;

/// 一覧に出すメタ情報。値そのものは含めない（画面へは明示的な reveal でのみ渡す）。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SecretMeta {
    pub key: String,
    pub binary: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list(store: &SecretStore) -> Result<Vec<SecretMeta>, String> {
    Ok(store
        .list()?
        .into_iter()
        .map(|(key, created_at, updated_at, binary)| SecretMeta {
            key,
            binary,
            created_at,
            updated_at,
        })
        .collect())
}

/// テキスト値を復号して返す。バイナリは画面に出せないので拒否する。
pub fn reveal_text(store: &SecretStore, key: &str) -> Result<String, String> {
    let (bytes, binary) = store.get_bytes(key)?;
    if binary {
        return Err(format!("{key} はバイナリです。「ファイルへ保存」を使ってください。"));
    }
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode error: {e}"))
}

pub fn set_text(store: &SecretStore, key: &str, value: &str) -> Result<(), String> {
    validate_key(key)?;
    store.set(key, value)
}

pub fn set_binary(store: &SecretStore, key: &str, bytes: &[u8]) -> Result<(), String> {
    validate_key(key)?;
    store.set_bytes(key, bytes, true)
}

/// 改名は「新キーへ複製 → 旧キーを削除」。宛先が既にあれば上書きせず拒否する。
pub fn rename(store: &SecretStore, from: &str, to: &str) -> Result<(), String> {
    validate_key(to)?;
    if from == to {
        return Ok(());
    }
    if store.get_bytes(to).is_ok() {
        return Err(format!("{to} は既に存在します"));
    }
    let (bytes, binary) = store.get_bytes(from)?;
    store.set_bytes(to, &bytes, binary)?;
    store.delete(from)?;
    Ok(())
}

pub fn delete(store: &SecretStore, key: &str) -> Result<(), String> {
    if store.delete(key)? {
        Ok(())
    } else {
        Err(format!("Secret not found: {key}"))
    }
}

pub fn export_sealed(store: &SecretStore, passphrase: &str) -> Result<Vec<u8>, String> {
    validate_passphrase(passphrase)?;
    let entries = store.export_entries()?;
    if entries.is_empty() {
        return Err("vault が空なので書き出すものがありません".into());
    }
    portable::seal(&entries, passphrase)
}

/// 戻り値: (imported, skipped)
pub fn import_sealed(
    store: &SecretStore,
    data: &[u8],
    passphrase: &str,
    overwrite: bool,
) -> Result<(usize, usize), String> {
    validate_passphrase(passphrase)?;
    let entries: Vec<Entry> = portable::unseal(data, passphrase)?;
    store.import_entries(&entries, overwrite)
}

fn validate_key(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("キーを入力してください".into());
    }
    if trimmed != key {
        return Err("キーの前後に空白は使えません".into());
    }
    if key.chars().any(char::is_control) {
        return Err("キーに制御文字は使えません".into());
    }
    Ok(())
}

fn validate_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.is_empty() {
        return Err("パスフレーズを入力してください".into());
    }
    Ok(())
}

/// クリップボードの自動消去判定。ユーザーが別の物をコピーした後なら消さない
/// （1Password と同じ振る舞い。他人の作業を壊さないため）。
pub fn should_clear_clipboard(current: Option<&str>, copied: &str) -> bool {
    current == Some(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vlt::crypto::generate_master_key;

    struct TempDb(PathBuf);
    impl TempDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vlt-gui-test-{name}-{}.db",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }
        fn open(&self) -> SecretStore {
            SecretStore::open_at(&self.0, generate_master_key()).unwrap()
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn list_returns_meta_without_values() {
        let db = TempDb::new("list");
        let store = db.open();
        set_text(&store, "openai/api-key", "sk-xxx").unwrap();
        set_binary(&store, "android/keystore", &[1, 2, 3]).unwrap();

        let metas = list(&store).unwrap();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].key, "android/keystore");
        assert!(metas[0].binary);
        assert_eq!(metas[1].key, "openai/api-key");
        assert!(!metas[1].binary);
        let json = serde_json::to_string(&metas).unwrap();
        assert!(!json.contains("sk-xxx"), "一覧に値が漏れてはいけない");
    }

    #[test]
    fn reveal_text_refuses_binary() {
        let db = TempDb::new("reveal");
        let store = db.open();
        set_text(&store, "t", "hello").unwrap();
        set_binary(&store, "b", &[0xff, 0x00]).unwrap();
        assert_eq!(reveal_text(&store, "t").unwrap(), "hello");
        assert!(reveal_text(&store, "b").unwrap_err().contains("バイナリ"));
    }

    #[test]
    fn set_text_rejects_blank_or_padded_key() {
        let db = TempDb::new("validate");
        let store = db.open();
        assert!(set_text(&store, "", "v").is_err());
        assert!(set_text(&store, "  ", "v").is_err());
        assert!(set_text(&store, " padded", "v").is_err());
        assert!(set_text(&store, "a\nb", "v").is_err());
        assert!(set_text(&store, "ok/key", "v").is_ok());
    }

    #[test]
    fn rename_moves_value_and_keeps_binary_flag() {
        let db = TempDb::new("rename");
        let store = db.open();
        set_binary(&store, "old", &[9, 8, 7]).unwrap();
        rename(&store, "old", "new").unwrap();
        assert!(store.get_bytes("old").is_err());
        assert_eq!(store.get_bytes("new").unwrap(), (vec![9, 8, 7], true));
    }

    #[test]
    fn rename_refuses_to_overwrite_existing_target() {
        let db = TempDb::new("rename-clash");
        let store = db.open();
        set_text(&store, "a", "va").unwrap();
        set_text(&store, "b", "vb").unwrap();
        assert!(rename(&store, "a", "b").is_err());
        assert_eq!(store.get("a").unwrap(), "va");
        assert_eq!(store.get("b").unwrap(), "vb");
    }

    #[test]
    fn delete_reports_missing_key() {
        let db = TempDb::new("delete");
        let store = db.open();
        set_text(&store, "k", "v").unwrap();
        delete(&store, "k").unwrap();
        assert!(delete(&store, "k").is_err());
    }

    #[test]
    fn export_import_roundtrip_with_passphrase() {
        let src_db = TempDb::new("export");
        let src = src_db.open();
        set_text(&src, "k", "v").unwrap();
        let sealed = export_sealed(&src, "pass").unwrap();

        let dst_db = TempDb::new("import");
        let dst = dst_db.open();
        assert!(import_sealed(&dst, &sealed, "wrong", false).is_err());
        assert_eq!(import_sealed(&dst, &sealed, "pass", false).unwrap(), (1, 0));
        assert_eq!(dst.get("k").unwrap(), "v");
    }

    #[test]
    fn export_rejects_empty_passphrase_and_empty_vault() {
        let db = TempDb::new("export-empty");
        let store = db.open();
        assert!(export_sealed(&store, "").is_err());
        assert!(export_sealed(&store, "pass").is_err());
    }

    #[test]
    fn clipboard_is_cleared_only_if_untouched() {
        assert!(should_clear_clipboard(Some("secret"), "secret"));
        assert!(!should_clear_clipboard(Some("other"), "secret"));
        assert!(!should_clear_clipboard(None, "secret"));
    }
}
