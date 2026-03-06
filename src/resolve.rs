use crate::store::SecretStore;
use std::collections::HashMap;
use std::env;

const PREFIX: &str = "vlt://";

/// Scan environment variables and resolve any vlt:// references.
/// Returns a map of env var name -> resolved value for variables that had references.
pub fn resolve_env(store: &SecretStore) -> Result<HashMap<String, String>, String> {
    let mut resolved = HashMap::new();

    for (name, value) in env::vars() {
        if let Some(secret_key) = value.strip_prefix(PREFIX) {
            let secret_value = store.get(secret_key)?;
            resolved.insert(name, secret_value);
        }
    }

    Ok(resolved)
}

/// Resolve a single value if it's a vlt:// reference, otherwise return as-is.
pub fn resolve_value(store: &SecretStore, value: &str) -> Result<String, String> {
    if let Some(secret_key) = value.strip_prefix(PREFIX) {
        store.get(secret_key)
    } else {
        Ok(value.to_string())
    }
}
