use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

const DEFAULT_SERVICE: &str = "dev.bayfront.vlt";
const ACCOUNT: &str = "master-key";

/// Keychain のサービス名。VLT_KEYCHAIN_SERVICE で差し替えられる
/// （E2E で本物のマスターキーに触れずに init から通すため。VLT_DB と対で使う）。
fn service() -> String {
    std::env::var("VLT_KEYCHAIN_SERVICE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SERVICE.to_string())
}

pub fn store_master_key(key: &[u8]) -> Result<(), String> {
    // Try to delete existing key first (ignore errors)
    let _ = delete_generic_password(&service(), ACCOUNT);
    set_generic_password(&service(), ACCOUNT, key).map_err(|e| format!("Failed to store master key in Keychain: {e}"))
}

pub fn load_master_key() -> Result<Vec<u8>, String> {
    get_generic_password(&service(), ACCOUNT).map_err(|e| format!("Failed to load master key from Keychain: {e}. Run `vlt init` first."))
}

/// マスターキーが存在するかを、値を読まずに調べる。
/// 値を読む（get_generic_password）と macOS の許可ダイアログが出るため、
/// GUI の起動時の状態表示はこちらを使い、ダイアログは明示的な「解錠」まで出さない。
pub fn has_master_key() -> bool {
    ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(&service())
        .account(ACCOUNT)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search()
        .map(|items| !items.is_empty())
        .unwrap_or(false)
}

/// マスターキーの削除。CLI では未使用（GUI の将来機能・テスト向け）。

pub fn delete_master_key() -> Result<(), String> {
    delete_generic_password(&service(), ACCOUNT).map_err(|e| format!("Failed to delete master key from Keychain: {e}"))
}
