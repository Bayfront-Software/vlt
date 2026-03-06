use security_framework::passwords::{delete_generic_password, get_generic_password, set_generic_password};

const SERVICE: &str = "dev.bayfront.vlt";
const ACCOUNT: &str = "master-key";

pub fn store_master_key(key: &[u8]) -> Result<(), String> {
    // Try to delete existing key first (ignore errors)
    let _ = delete_generic_password(SERVICE, ACCOUNT);
    set_generic_password(SERVICE, ACCOUNT, key).map_err(|e| format!("Failed to store master key in Keychain: {e}"))
}

pub fn load_master_key() -> Result<Vec<u8>, String> {
    get_generic_password(SERVICE, ACCOUNT).map_err(|e| format!("Failed to load master key from Keychain: {e}. Run `vlt init` first."))
}

pub fn delete_master_key() -> Result<(), String> {
    delete_generic_password(SERVICE, ACCOUNT).map_err(|e| format!("Failed to delete master key from Keychain: {e}"))
}
