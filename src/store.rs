use rusqlite::{params, Connection};
use std::path::PathBuf;

use crate::crypto;

pub struct SecretStore {
    conn: Connection,
    master_key: [u8; 32],
}

fn db_path() -> PathBuf {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
        .join("vlt");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");
    data_dir.join("vault.db")
}

impl SecretStore {
    pub fn open(master_key: [u8; 32]) -> Result<Self, String> {
        let path = db_path();
        let conn =
            Connection::open(&path).map_err(|e| format!("Failed to open database: {e}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS secrets (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL,
                created_at TEXT DEFAULT (datetime('now')),
                updated_at TEXT DEFAULT (datetime('now'))
            );",
        )
        .map_err(|e| format!("Failed to create table: {e}"))?;

        Ok(Self { conn, master_key })
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        let encrypted = crypto::encrypt(&self.master_key, value.as_bytes())?;

        self.conn
            .execute(
                "INSERT INTO secrets (key, value, updated_at) VALUES (?1, ?2, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = datetime('now')",
                params![key, encrypted],
            )
            .map_err(|e| format!("Failed to store secret: {e}"))?;

        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<String, String> {
        let encrypted: Vec<u8> = self
            .conn
            .query_row("SELECT value FROM secrets WHERE key = ?1", params![key], |row| {
                row.get(0)
            })
            .map_err(|_| format!("Secret not found: {key}"))?;

        let decrypted = crypto::decrypt(&self.master_key, &encrypted)?;
        String::from_utf8(decrypted).map_err(|e| format!("UTF-8 decode error: {e}"))
    }

    pub fn delete(&self, key: &str) -> Result<bool, String> {
        let affected = self
            .conn
            .execute("DELETE FROM secrets WHERE key = ?1", params![key])
            .map_err(|e| format!("Failed to delete secret: {e}"))?;
        Ok(affected > 0)
    }

    pub fn list(&self) -> Result<Vec<(String, String, String)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, created_at, updated_at FROM secrets ORDER BY key")
            .map_err(|e| format!("Failed to prepare query: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("Query error: {e}"))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| format!("Row error: {e}"))?);
        }
        Ok(result)
    }
}
