//! vlt のデスクトップ GUI（Tauri 2）。
//!
//! 方針:
//! - 値は一覧に載せない。画面に出すのは `reveal_secret` を明示的に呼んだときだけ。
//! - ファイル選択・保存・クリップボードは Rust 側で行う。WebView に値を
//!   長く滞在させないためと、JS 側の権限を `core:default` だけに絞るため。
//! - コピーした値は 30 秒後に自動で消す（その間に別の物がコピーされていれば消さない）。
//! - 「ロック」はメモリ上の `SecretStore` を捨てるだけ。マスターキーは OS Keychain に
//!   あるので、次の解錠で Keychain へ再び問い合わせる（＝1Password の解錠に相当）。

pub mod ops;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use vlt::store::{self, SecretStore};
use vlt::{crypto, keychain};

use ops::SecretMeta;

/// コピーした値をクリップボードから消すまでの秒数。
const CLIPBOARD_CLEAR_SECS: u64 = 30;

struct Vault(Mutex<Option<SecretStore>>);

#[derive(Serialize)]
struct VaultStatus {
    initialized: bool,
    unlocked: bool,
    db_path: String,
}

fn with_store<T>(
    vault: &State<Vault>,
    f: impl FnOnce(&SecretStore) -> Result<T, String>,
) -> Result<T, String> {
    let guard = vault.0.lock().map_err(|_| "vault lock poisoned".to_string())?;
    let store = guard.as_ref().ok_or("vault はロックされています")?;
    f(store)
}

fn open_store_with_keychain() -> Result<SecretStore, String> {
    let key_bytes = keychain::load_master_key()?;
    let master_key: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "Keychain のマスターキー長が不正です。`vlt init` をやり直してください。".to_string())?;
    SecretStore::open(master_key)
}

#[tauri::command]
fn vault_status(vault: State<Vault>) -> VaultStatus {
    let unlocked = vault.0.lock().map(|g| g.is_some()).unwrap_or(false);
    VaultStatus {
        initialized: keychain::has_master_key(),
        unlocked,
        db_path: store::db_path().display().to_string(),
    }
}

#[tauri::command]
fn unlock(vault: State<Vault>) -> Result<(), String> {
    let store = open_store_with_keychain()?;
    *vault.0.lock().map_err(|_| "vault lock poisoned")? = Some(store);
    Ok(())
}

#[tauri::command]
fn lock(vault: State<Vault>) -> Result<(), String> {
    *vault.0.lock().map_err(|_| "vault lock poisoned")? = None;
    Ok(())
}

/// CLI の `vlt init` と同じガード: 既にマスターキーがあれば絶対に作り直さない。
#[tauri::command]
fn init_vault(vault: State<Vault>) -> Result<(), String> {
    if keychain::has_master_key() {
        return Err("マスターキーは既に Keychain にあります。作り直すと今の vault が二度と開けなくなるため、GUI からは行いません。".into());
    }
    let master_key = crypto::generate_master_key();
    keychain::store_master_key(&master_key)?;
    let store = SecretStore::open(master_key)?;
    *vault.0.lock().map_err(|_| "vault lock poisoned")? = Some(store);
    Ok(())
}

#[tauri::command]
fn list_secrets(vault: State<Vault>) -> Result<Vec<SecretMeta>, String> {
    with_store(&vault, ops::list)
}

#[tauri::command]
fn reveal_secret(vault: State<Vault>, key: String) -> Result<String, String> {
    with_store(&vault, |s| ops::reveal_text(s, &key))
}

#[tauri::command]
fn set_secret(vault: State<Vault>, key: String, value: String) -> Result<(), String> {
    with_store(&vault, |s| ops::set_text(s, &key, &value))
}

#[tauri::command]
fn rename_secret(vault: State<Vault>, from: String, to: String) -> Result<(), String> {
    with_store(&vault, |s| ops::rename(s, &from, &to))
}

#[tauri::command]
fn delete_secret(vault: State<Vault>, key: String) -> Result<(), String> {
    with_store(&vault, |s| ops::delete(s, &key))
}

/// 値をクリップボードへ。30 秒後、まだ同じ値なら消す。
#[tauri::command]
fn copy_secret(app: AppHandle, vault: State<Vault>, key: String) -> Result<u64, String> {
    let value = with_store(&vault, |s| ops::reveal_text(s, &key))?;
    app.clipboard()
        .write_text(value.clone())
        .map_err(|e| format!("clipboard error: {e}"))?;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(CLIPBOARD_CLEAR_SECS));
        let current = app.clipboard().read_text().ok();
        if ops::should_clear_clipboard(current.as_deref(), &value) {
            let _ = app.clipboard().write_text(String::new());
        }
    });
    Ok(CLIPBOARD_CLEAR_SECS)
}

/// ファイルを選ばせてバイナリとして保存する。キャンセルなら Ok(false)。
#[tauri::command]
fn import_file_as_secret(app: AppHandle, vault: State<Vault>, key: String) -> Result<bool, String> {
    let Some(picked) = app.dialog().file().blocking_pick_file() else {
        return Ok(false);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    with_store(&vault, |s| ops::set_binary(s, &key, &bytes))?;
    Ok(true)
}

/// 保存先を選ばせて値（テキストでもバイナリでも）を書き出す。キャンセルなら None。
#[tauri::command]
fn save_secret_to_file(app: AppHandle, vault: State<Vault>, key: String) -> Result<Option<String>, String> {
    let (bytes, _) = with_store(&vault, |s| s.get_bytes(&key))?;
    let suggested = key.rsplit('/').next().unwrap_or("secret").to_string();
    let Some(target) = app.dialog().file().set_file_name(&suggested).blocking_save_file() else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, &bytes).map_err(|e| format!("{} に書けません: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
fn export_vault(app: AppHandle, vault: State<Vault>, passphrase: String) -> Result<Option<String>, String> {
    let sealed = with_store(&vault, |s| ops::export_sealed(s, &passphrase))?;
    let Some(target) = app
        .dialog()
        .file()
        .add_filter("vlt backup", &["vltx"])
        .set_file_name("vlt-backup.vltx")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, &sealed).map_err(|e| format!("{} に書けません: {e}", path.display()))?;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
struct ImportResult {
    imported: usize,
    skipped: usize,
}

#[tauri::command]
fn import_vault(
    app: AppHandle,
    vault: State<Vault>,
    passphrase: String,
    overwrite: bool,
) -> Result<Option<ImportResult>, String> {
    let Some(picked) = app
        .dialog()
        .file()
        .add_filter("vlt backup", &["vltx"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let data = std::fs::read(&path).map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    let (imported, skipped) = with_store(&vault, |s| ops::import_sealed(s, &data, &passphrase, overwrite))?;
    Ok(Some(ImportResult { imported, skipped }))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(Vault(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            vault_status,
            unlock,
            lock,
            init_vault,
            list_secrets,
            reveal_secret,
            set_secret,
            rename_secret,
            delete_secret,
            copy_secret,
            import_file_as_secret,
            save_secret_to_file,
            export_vault,
            import_vault,
        ])
        .run(tauri::generate_context!())
        .expect("error while running vlt");
}
