use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

use crate::crypto;
use crate::portable::Entry;

/// スキーマ版数。PRAGMA user_version で管理する。
/// v0: 初期スキーマ（binary カラムなし、v0.1.0 時代）
/// v1: binary カラム追加（バイナリ値対応）
const SCHEMA_VERSION: i64 = 1;

pub struct SecretStore {
    conn: Connection,
    master_key: [u8; 32],
}

fn db_path() -> PathBuf {
    // テスト・移行作業用に VLT_DB で保存先を差し替えられる。
    if let Ok(p) = std::env::var("VLT_DB") {
        let path = PathBuf::from(p);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create data directory");
        }
        return path;
    }
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
        .join("vlt");
    std::fs::create_dir_all(&data_dir).expect("Failed to create data directory");
    data_dir.join("vault.db")
}

impl SecretStore {
    pub fn open(master_key: [u8; 32]) -> Result<Self, String> {
        Self::open_at(&db_path(), master_key)
    }

    pub fn open_at(path: &Path, master_key: [u8; 32]) -> Result<Self, String> {
        let conn =
            Connection::open(path).map_err(|e| format!("Failed to open database: {e}"))?;

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|e| format!("Failed to read schema version: {e}"))?;

        if version < 1 {
            // v0 からの移行: テーブルが無ければ新規作成、あれば binary カラムを追加。
            // 旧データは全てテキスト値なので binary=0 のままで正しい。
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS secrets (
                    key TEXT PRIMARY KEY,
                    value BLOB NOT NULL,
                    created_at TEXT DEFAULT (datetime('now')),
                    updated_at TEXT DEFAULT (datetime('now'))
                );",
            )
            .map_err(|e| format!("Failed to create table: {e}"))?;

            let has_binary: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('secrets') WHERE name='binary'",
                    [],
                    |row| row.get::<_, i64>(0).map(|n| n > 0),
                )
                .map_err(|e| format!("Failed to inspect schema: {e}"))?;
            if !has_binary {
                conn.execute_batch(
                    "ALTER TABLE secrets ADD COLUMN binary INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(|e| format!("Failed to migrate schema: {e}"))?;
            }
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(|e| format!("Failed to set schema version: {e}"))?;
        }

        Ok(Self { conn, master_key })
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.set_bytes(key, value.as_bytes(), false)
    }

    pub fn set_bytes(&self, key: &str, value: &[u8], binary: bool) -> Result<(), String> {
        let encrypted = crypto::encrypt(&self.master_key, value)?;
        self.conn
            .execute(
                "INSERT INTO secrets (key, value, binary, updated_at)
                 VALUES (?1, ?2, ?3, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET
                   value = ?2, binary = ?3, updated_at = datetime('now')",
                params![key, encrypted, binary as i64],
            )
            .map_err(|e| format!("Failed to store secret: {e}"))?;
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<String, String> {
        let (bytes, _) = self.get_bytes(key)?;
        String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode error: {e}"))
    }

    /// 復号済みバイト列と binary フラグを返す。
    pub fn get_bytes(&self, key: &str) -> Result<(Vec<u8>, bool), String> {
        let row: Option<(Vec<u8>, i64)> = self
            .conn
            .query_row(
                "SELECT value, binary FROM secrets WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("Query error: {e}"))?;
        let (encrypted, binary) = row.ok_or_else(|| format!("Secret not found: {key}"))?;
        let decrypted = crypto::decrypt(&self.master_key, &encrypted)?;
        Ok((decrypted, binary != 0))
    }

    pub fn delete(&self, key: &str) -> Result<bool, String> {
        let affected = self
            .conn
            .execute("DELETE FROM secrets WHERE key = ?1", params![key])
            .map_err(|e| format!("Failed to delete secret: {e}"))?;
        Ok(affected > 0)
    }

    /// (key, created_at, updated_at, binary)
    pub fn list(&self) -> Result<Vec<(String, String, String, bool)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, created_at, updated_at, binary FROM secrets ORDER BY key")
            .map_err(|e| format!("Failed to prepare query: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })
            .map_err(|e| format!("Query error: {e}"))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| format!("Row error: {e}"))?);
        }
        Ok(result)
    }

    /// 全 secret を復号して可搬 Entry 列にする（export 用）。
    pub fn export_entries(&self) -> Result<Vec<Entry>, String> {
        let mut entries = Vec::new();
        for (key, created_at, updated_at, _) in self.list()? {
            let (value, binary) = self.get_bytes(&key)?;
            entries.push(Entry {
                key,
                value,
                binary,
                created_at,
                updated_at,
            });
        }
        Ok(entries)
    }

    /// Entry 列を取り込む。overwrite=false では既存キーをスキップする。
    /// 戻り値: (imported, skipped)
    pub fn import_entries(
        &self,
        entries: &[Entry],
        overwrite: bool,
    ) -> Result<(usize, usize), String> {
        let mut imported = 0;
        let mut skipped = 0;
        for entry in entries {
            let exists = self.get_bytes(&entry.key).is_ok();
            if exists && !overwrite {
                skipped += 1;
                continue;
            }
            let encrypted = crypto::encrypt(&self.master_key, &entry.value)?;
            self.conn
                .execute(
                    "INSERT INTO secrets (key, value, binary, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(key) DO UPDATE SET
                       value = ?2, binary = ?3, created_at = ?4, updated_at = ?5",
                    params![
                        entry.key,
                        encrypted,
                        entry.binary as i64,
                        entry.created_at,
                        entry.updated_at
                    ],
                )
                .map_err(|e| format!("Failed to import {}: {e}", entry.key))?;
            imported += 1;
        }
        Ok((imported, skipped))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDb(PathBuf);
    impl TempDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vlt-test-{}-{}.db",
                name,
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn text_and_binary_roundtrip() {
        let db = TempDb::new("roundtrip");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();

        store.set("plain/text", "hello").unwrap();
        assert_eq!(store.get("plain/text").unwrap(), "hello");
        let (_, binary) = store.get_bytes("plain/text").unwrap();
        assert!(!binary);

        let blob = vec![0xfe, 0xed, 0x00, 0xff, 0x10];
        store.set_bytes("bin/keystore", &blob, true).unwrap();
        let (got, binary) = store.get_bytes("bin/keystore").unwrap();
        assert_eq!(got, blob);
        assert!(binary);
        // バイナリ値は UTF-8 とは限らないため get(String) はエラーになり得る
        assert!(store.get("bin/keystore").is_err());
    }

    #[test]
    fn export_import_roundtrip_into_fresh_store() {
        let db1 = TempDb::new("export-src");
        let src = SecretStore::open_at(&db1.0, crypto::generate_master_key()).unwrap();
        src.set("a/text", "value-a").unwrap();
        src.set_bytes("b/bin", &[1, 2, 3, 254], true).unwrap();

        let entries = src.export_entries().unwrap();
        assert_eq!(entries.len(), 2);

        // 別マスターキーの新しい vault に取り込めること（= 可搬性の本質）
        let db2 = TempDb::new("export-dst");
        let dst = SecretStore::open_at(&db2.0, crypto::generate_master_key()).unwrap();
        let (imported, skipped) = dst.import_entries(&entries, false).unwrap();
        assert_eq!((imported, skipped), (2, 0));
        assert_eq!(dst.get("a/text").unwrap(), "value-a");
        assert_eq!(dst.get_bytes("b/bin").unwrap(), (vec![1, 2, 3, 254], true));
    }

    #[test]
    fn import_skips_existing_unless_overwrite() {
        let db = TempDb::new("import-skip");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.set("k", "local-value").unwrap();

        let incoming = vec![Entry {
            key: "k".into(),
            value: b"backup-value".to_vec(),
            binary: false,
            created_at: "2026-01-01 00:00:00".into(),
            updated_at: "2026-01-01 00:00:00".into(),
        }];

        let (imported, skipped) = store.import_entries(&incoming, false).unwrap();
        assert_eq!((imported, skipped), (0, 1));
        assert_eq!(store.get("k").unwrap(), "local-value");

        let (imported, skipped) = store.import_entries(&incoming, true).unwrap();
        assert_eq!((imported, skipped), (1, 0));
        assert_eq!(store.get("k").unwrap(), "backup-value");
    }

    #[test]
    fn migrates_v0_schema_preserving_rows() {
        let db = TempDb::new("migrate");
        let master = crypto::generate_master_key();

        // v0.1.0 時代のスキーマと行を手作りする
        {
            let conn = Connection::open(&db.0).unwrap();
            conn.execute_batch(
                "CREATE TABLE secrets (
                    key TEXT PRIMARY KEY,
                    value BLOB NOT NULL,
                    created_at TEXT DEFAULT (datetime('now')),
                    updated_at TEXT DEFAULT (datetime('now'))
                );",
            )
            .unwrap();
            let encrypted = crypto::encrypt(&master, b"legacy-value").unwrap();
            conn.execute(
                "INSERT INTO secrets (key, value) VALUES ('legacy/key', ?1)",
                params![encrypted],
            )
            .unwrap();
        }

        let store = SecretStore::open_at(&db.0, master).unwrap();
        assert_eq!(store.get("legacy/key").unwrap(), "legacy-value");
        let (_, binary) = store.get_bytes("legacy/key").unwrap();
        assert!(!binary, "既存行はテキスト扱いのまま");
    }
}
