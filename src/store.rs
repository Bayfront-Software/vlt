use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

use crate::crypto;
use crate::item::{Item, ItemType, PrimaryValue};
use crate::portable::Entry;

/// スキーマ版数。PRAGMA user_version で管理する。
/// v0: 初期スキーマ（binary カラムなし、v0.1.0 時代）
/// v1: binary カラム追加（バイナリ値対応）
/// v2: format / kind カラムと trash テーブル（種類つき項目・ゴミ箱、v0.3.0）
const SCHEMA_VERSION: i64 = 2;

/// 行の中身の形式。RAW は v0.2 以前の「生の値」、ITEM は Item の JSON。
/// RAW の行は書き換えずに残し、読むときに Item::from_legacy で解釈する
/// （移行で全行を書き換えると、途中失敗で vault を壊す経路が生まれるため）。
const FORMAT_RAW: i64 = 0;
const FORMAT_ITEM: i64 = 1;

/// ゴミ箱に入れた項目を自動で完全削除するまでの日数（1Password と同じ 30 日）。
pub const TRASH_RETENTION_DAYS: i64 = 30;

pub struct SecretStore {
    conn: Connection,
    master_key: [u8; 32],
}

impl Drop for SecretStore {
    // ロック時にメモリ上のマスターキーを消す（GUI のロックは store を捨てることで行う）。
    fn drop(&mut self) {
        self.master_key.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemSummary {
    pub key: String,
    pub item_type: ItemType,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrashEntry {
    pub id: i64,
    pub key: String,
    pub item_type: ItemType,
    pub deleted_at: String,
}

/// 既定の vault.db の場所（VLT_DB があればそれ）。GUI の表示用にも使う。
pub fn db_path() -> PathBuf {
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
    // 既定の保存先だけを本人専用にする（VLT_DB の親は /tmp などの共有フォルダがあり得るので触らない）。
    let _ = restrict_permissions(&data_dir, 0o700);
    data_dir.join("vault.db")
}

fn restrict_permissions(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// kind 列（ITEM 行）か binary フラグ（RAW 行）から種類を決める。
fn type_of(kind: Option<String>, binary: bool) -> ItemType {
    kind.as_deref()
        .and_then(ItemType::parse)
        .unwrap_or(if binary { ItemType::Document } else { ItemType::Password })
}

fn db_err(context: &str) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |e| format!("{context}: {e}")
}

impl SecretStore {
    pub fn open(master_key: [u8; 32]) -> Result<Self, String> {
        Self::open_at(&db_path(), master_key)
    }

    pub fn open_at(path: &Path, master_key: [u8; 32]) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(db_err("Failed to open database"))?;
        // 中身は暗号化済みだが、キー名・件数・日時は平文なので本人以外に読ませない。
        restrict_permissions(path, 0o600).map_err(|e| format!("Failed to restrict permissions: {e}"))?;

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_err("Failed to read schema version"))?;

        // 新しい vlt が作った DB を古い vlt が開いて壊さないように止める。
        if version > SCHEMA_VERSION {
            return Err(format!(
                "この vault はより新しい vlt で作られています（schema v{version}）。vlt を更新してください"
            ));
        }

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
            .map_err(db_err("Failed to create table"))?;

            let has_binary: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('secrets') WHERE name='binary'",
                    [],
                    |row| row.get::<_, i64>(0).map(|n| n > 0),
                )
                .map_err(db_err("Failed to inspect schema"))?;
            if !has_binary {
                conn.execute_batch(
                    "ALTER TABLE secrets ADD COLUMN binary INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(db_err("Failed to migrate schema"))?;
            }
            conn.pragma_update(None, "user_version", 1)
                .map_err(db_err("Failed to set schema version"))?;
        }

        if version < 2 {
            // v1 → v2: 列とテーブルを足すだけで既存行には触れない（既存行は format=0 = RAW）。
            conn.execute_batch(
                "BEGIN;
                 ALTER TABLE secrets ADD COLUMN format INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE secrets ADD COLUMN kind TEXT;
                 CREATE TABLE IF NOT EXISTS trash (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    key TEXT NOT NULL,
                    value BLOB NOT NULL,
                    binary INTEGER NOT NULL DEFAULT 0,
                    format INTEGER NOT NULL DEFAULT 0,
                    kind TEXT,
                    created_at TEXT,
                    updated_at TEXT,
                    deleted_at TEXT NOT NULL DEFAULT (datetime('now'))
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )
            .map_err(db_err("Failed to migrate schema to v2"))?;
        }

        conn.execute(
            "DELETE FROM trash WHERE deleted_at < datetime('now', ?1)",
            params![format!("-{TRASH_RETENTION_DAYS} days")],
        )
        .map_err(db_err("Failed to purge old trash"))?;

        Ok(Self { conn, master_key })
    }

    /// (復号済みの中身, binary, format)
    fn read_row(&self, key: &str) -> Result<Option<(Vec<u8>, bool, i64)>, String> {
        let row: Option<(Vec<u8>, i64, i64)> = self
            .conn
            .query_row(
                "SELECT value, binary, format FROM secrets WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_err("Query error"))?;
        match row {
            None => Ok(None),
            Some((encrypted, binary, format)) => {
                let decrypted = crypto::decrypt(&self.master_key, &encrypted)?;
                Ok(Some((decrypted, binary != 0, format)))
            }
        }
    }

    pub fn exists(&self, key: &str) -> Result<bool, String> {
        self.conn
            .query_row("SELECT 1 FROM secrets WHERE key = ?1", params![key], |_| Ok(()))
            .optional()
            .map(|r| r.is_some())
            .map_err(db_err("Query error"))
    }

    pub fn get_item(&self, key: &str) -> Result<Item, String> {
        let (payload, binary, format) = self
            .read_row(key)?
            .ok_or_else(|| format!("Secret not found: {key}"))?;
        match format {
            FORMAT_ITEM => Item::from_json(&payload),
            FORMAT_RAW => Item::from_legacy(key, &payload, binary),
            other => Err(format!("{key} の保存形式 {other} を読めません。vlt を更新してください")),
        }
    }

    /// 項目を作成または丸ごと置き換える。作成日時は保つ。
    pub fn put_item(&self, key: &str, item: &Item) -> Result<(), String> {
        let encrypted = crypto::encrypt(&self.master_key, &item.to_json())?;
        self.conn
            .execute(
                "INSERT INTO secrets (key, value, binary, format, kind, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(key) DO UPDATE SET
                   value = ?2, binary = ?3, format = ?4, kind = ?5, updated_at = datetime('now')",
                params![key, encrypted, item.is_binary() as i64, FORMAT_ITEM, item.item_type.as_str()],
            )
            .map_err(db_err("Failed to store secret"))?;
        Ok(())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.set_bytes(key, value.as_bytes(), false)
    }

    /// CLI の `vlt set`。既存の項目なら主役フィールドだけを書き換え（ユーザー名などは残す）、
    /// 無ければテキストはパスワード項目、バイナリは書類項目として作る。
    pub fn set_bytes(&self, key: &str, value: &[u8], binary: bool) -> Result<(), String> {
        let item = if self.exists(key)? {
            let mut item = self.get_item(key)?;
            if binary {
                let field = item
                    .fields
                    .iter_mut()
                    .find(|f| f.kind == crate::item::FieldKind::File)
                    .ok_or_else(|| format!("{key} は{}なのでファイルは保存できません", item.item_type.label()))?;
                let filename = field.filename.clone().unwrap_or_default();
                *field = crate::item::Field { filename: Some(filename), ..crate::item::Field::file("", value) };
            } else {
                let text = std::str::from_utf8(value).map_err(|e| format!("UTF-8 decode error: {e}"))?;
                item.set_primary_text(text)?;
            }
            item
        } else {
            Item::from_legacy(key, value, binary)?
        };
        self.put_item(key, &item)
    }

    pub fn get(&self, key: &str) -> Result<String, String> {
        let (bytes, _) = self.get_bytes(key)?;
        String::from_utf8(bytes).map_err(|e| format!("UTF-8 decode error: {e}"))
    }

    /// 主役フィールドの値と「ファイルか」を返す（CLI の `vlt get` と旧来の参照解決）。
    pub fn get_bytes(&self, key: &str) -> Result<(Vec<u8>, bool), String> {
        let (payload, binary, format) = self
            .read_row(key)?
            .ok_or_else(|| format!("Secret not found: {key}"))?;
        match format {
            FORMAT_RAW => return Ok((payload, binary)),
            FORMAT_ITEM => {}
            other => return Err(format!("{key} の保存形式 {other} を読めません。vlt を更新してください")),
        }
        let item = Item::from_json(&payload)?;
        match item.primary() {
            Some(PrimaryValue::Text(text)) => Ok((text.as_bytes().to_vec(), false)),
            Some(PrimaryValue::File(field)) => Ok((field.file_bytes()?, true)),
            None => Err(format!(
                "{key} は{}で主役のフィールドがありません。フィールド名を指定してください",
                item.item_type.label()
            )),
        }
    }

    /// ゴミ箱へ移す。見つからなければ false。
    pub fn delete(&self, key: &str) -> Result<bool, String> {
        let tx = self.conn.unchecked_transaction().map_err(db_err("Failed to begin"))?;
        tx.execute(
            "INSERT INTO trash (key, value, binary, format, kind, created_at, updated_at)
             SELECT key, value, binary, format, kind, created_at, updated_at FROM secrets WHERE key = ?1",
            params![key],
        )
        .map_err(db_err("Failed to move to trash"))?;
        let affected = tx
            .execute("DELETE FROM secrets WHERE key = ?1", params![key])
            .map_err(db_err("Failed to delete secret"))?;
        tx.commit().map_err(db_err("Failed to commit"))?;
        Ok(affected > 0)
    }

    /// ゴミ箱を経由せずに完全に消す。見つからなければ false。
    pub fn purge(&self, key: &str) -> Result<bool, String> {
        let affected = self
            .conn
            .execute("DELETE FROM secrets WHERE key = ?1", params![key])
            .map_err(db_err("Failed to delete secret"))?;
        Ok(affected > 0)
    }

    pub fn list_trash(&self) -> Result<Vec<TrashEntry>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, key, binary, kind, deleted_at FROM trash ORDER BY deleted_at DESC, id DESC")
            .map_err(db_err("Failed to prepare query"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(TrashEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    item_type: type_of(row.get(3)?, row.get::<_, i64>(2)? != 0),
                    deleted_at: row.get(4)?,
                })
            })
            .map_err(db_err("Query error"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_err("Row error"))
    }

    /// ゴミ箱から戻す。同じキーの項目が既にあれば拒否する（上書きで消さないため）。
    pub fn restore(&self, trash_id: i64) -> Result<String, String> {
        let tx = self.conn.unchecked_transaction().map_err(db_err("Failed to begin"))?;
        let key: String = tx
            .query_row("SELECT key FROM trash WHERE id = ?1", params![trash_id], |r| r.get(0))
            .optional()
            .map_err(db_err("Query error"))?
            .ok_or("ゴミ箱に見つかりません")?;
        let taken = tx
            .query_row("SELECT 1 FROM secrets WHERE key = ?1", params![key], |_| Ok(()))
            .optional()
            .map_err(db_err("Query error"))?
            .is_some();
        if taken {
            return Err(format!("{key} は既にあるので戻せません。先に名前を変えてください"));
        }
        tx.execute(
            "INSERT INTO secrets (key, value, binary, format, kind, created_at, updated_at)
             SELECT key, value, binary, format, kind, created_at, updated_at FROM trash WHERE id = ?1",
            params![trash_id],
        )
        .map_err(db_err("Failed to restore"))?;
        tx.execute("DELETE FROM trash WHERE id = ?1", params![trash_id])
            .map_err(db_err("Failed to restore"))?;
        tx.commit().map_err(db_err("Failed to commit"))?;
        Ok(key)
    }

    pub fn purge_trash_item(&self, trash_id: i64) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM trash WHERE id = ?1", params![trash_id])
            .map_err(db_err("Failed to delete"))?;
        Ok(())
    }

    pub fn empty_trash(&self) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM trash", [])
            .map_err(db_err("Failed to empty trash"))?;
        Ok(())
    }

    /// キーを変える。宛先が既にあれば拒否する。
    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        if self.exists(to)? {
            return Err(format!("{to} は既に存在します"));
        }
        let affected = self
            .conn
            .execute(
                "UPDATE secrets SET key = ?2, updated_at = datetime('now') WHERE key = ?1",
                params![from, to],
            )
            .map_err(db_err("Failed to rename"))?;
        if affected == 0 {
            return Err(format!("Secret not found: {from}"));
        }
        Ok(())
    }

    /// (key, created_at, updated_at, binary)
    pub fn list(&self) -> Result<Vec<(String, String, String, bool)>, String> {
        Ok(self
            .list_rows()?
            .into_iter()
            .map(|(summary, binary)| (summary.key, summary.created_at, summary.updated_at, binary))
            .collect())
    }

    /// 復号せずに分かる範囲の一覧（キー・種類・日時）。
    pub fn list_items(&self) -> Result<Vec<ItemSummary>, String> {
        Ok(self.list_rows()?.into_iter().map(|(summary, _)| summary).collect())
    }

    fn list_rows(&self) -> Result<Vec<(ItemSummary, bool)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, created_at, updated_at, binary, kind FROM secrets ORDER BY key")
            .map_err(db_err("Failed to prepare query"))?;
        let rows = stmt
            .query_map([], |row| {
                let binary = row.get::<_, i64>(3)? != 0;
                Ok((
                    ItemSummary {
                        key: row.get(0)?,
                        created_at: row.get(1)?,
                        updated_at: row.get(2)?,
                        item_type: type_of(row.get(4)?, binary),
                    },
                    binary,
                ))
            })
            .map_err(db_err("Query error"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_err("Row error"))
    }

    /// 全項目を復号して可搬 Entry 列にする（export 用）。中身は保存形式のまま運ぶ。
    pub fn export_entries(&self) -> Result<Vec<Entry>, String> {
        let mut entries = Vec::new();
        for (key, created_at, updated_at, _) in self.list()? {
            let (value, binary, format) = self
                .read_row(&key)?
                .ok_or_else(|| format!("Secret not found: {key}"))?;
            entries.push(Entry {
                key,
                value,
                binary,
                created_at,
                updated_at,
                format: format as u8,
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
            if self.exists(&entry.key)? && !overwrite {
                skipped += 1;
                continue;
            }
            let format = i64::from(entry.format);
            let kind = if format == FORMAT_ITEM {
                Some(Item::from_json(&entry.value)?.item_type.as_str())
            } else {
                None
            };
            let encrypted = crypto::encrypt(&self.master_key, &entry.value)?;
            self.conn
                .execute(
                    "INSERT INTO secrets (key, value, binary, format, kind, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(key) DO UPDATE SET
                       value = ?2, binary = ?3, format = ?4, kind = ?5, created_at = ?6, updated_at = ?7",
                    params![
                        entry.key,
                        encrypted,
                        entry.binary as i64,
                        format,
                        kind,
                        entry.created_at,
                        entry.updated_at
                    ],
                )
                .map_err(|e| format!("Failed to import {}: {e}", entry.key))?;
            imported += 1;
        }
        Ok((imported, skipped))
    }

    /// テスト用: v0.2 以前の形式（RAW）の行を直接書く。
    #[cfg(test)]
    pub(crate) fn put_raw_for_test(&self, key: &str, value: &[u8], binary: bool) {
        let encrypted = crypto::encrypt(&self.master_key, value).unwrap();
        self.conn
            .execute(
                "INSERT INTO secrets (key, value, binary, format) VALUES (?1, ?2, ?3, ?4)",
                params![key, encrypted, binary as i64, FORMAT_RAW],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, ItemType};

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
            format: 0,
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

    #[test]
    fn put_and_get_item_roundtrip() {
        let db = TempDb::new("item");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        let mut item = Item::new(ItemType::Login);
        item.field_mut("username").unwrap().value = "alice".into();
        item.field_mut("password").unwrap().value = "pw".into();
        store.put_item("github/login", &item).unwrap();
        assert_eq!(store.get_item("github/login").unwrap(), item);
        // 旧 API からは主役フィールド（パスワード）が見える
        assert_eq!(store.get("github/login").unwrap(), "pw");
        let summaries = store.list_items().unwrap();
        assert_eq!(summaries[0].item_type, ItemType::Login);
    }

    #[test]
    fn legacy_rows_are_read_as_items() {
        let db = TempDb::new("legacy-item");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.put_raw_for_test("old/text", b"v", false);
        store.put_raw_for_test("old/bin", &[1, 2], true);
        assert_eq!(store.get_item("old/text").unwrap().item_type, ItemType::Password);
        assert_eq!(store.get_item("old/bin").unwrap().item_type, ItemType::Document);
        assert_eq!(store.get_bytes("old/bin").unwrap(), (vec![1, 2], true));
        let types: Vec<_> = store.list_items().unwrap().into_iter().map(|s| s.item_type).collect();
        assert_eq!(types, vec![ItemType::Document, ItemType::Password]);
    }

    #[test]
    fn set_on_existing_item_updates_only_primary_field() {
        let db = TempDb::new("set-primary");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        let mut item = Item::new(ItemType::Login);
        item.field_mut("username").unwrap().value = "alice".into();
        store.put_item("site", &item).unwrap();
        store.set("site", "new-pw").unwrap();
        let got = store.get_item("site").unwrap();
        assert_eq!(got.field("username").unwrap().value, "alice");
        assert_eq!(got.field("password").unwrap().value, "new-pw");
    }

    #[test]
    fn delete_moves_to_trash_and_restore_brings_it_back() {
        let db = TempDb::new("trash");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.set("k", "v").unwrap();
        assert!(store.delete("k").unwrap());
        assert!(store.get("k").is_err());
        let trash = store.list_trash().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].key, "k");
        store.restore(trash[0].id).unwrap();
        assert_eq!(store.get("k").unwrap(), "v");
        assert!(store.list_trash().unwrap().is_empty());
    }

    #[test]
    fn restore_refuses_when_key_is_taken() {
        let db = TempDb::new("trash-clash");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.set("k", "old").unwrap();
        store.delete("k").unwrap();
        store.set("k", "new").unwrap();
        let id = store.list_trash().unwrap()[0].id;
        assert!(store.restore(id).is_err());
        assert_eq!(store.get("k").unwrap(), "new");
        assert_eq!(store.list_trash().unwrap().len(), 1, "失敗した復元でゴミ箱から消えない");
    }

    #[test]
    fn purge_removes_permanently() {
        let db = TempDb::new("purge");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.set("a", "1").unwrap();
        store.set("b", "2").unwrap();
        assert!(store.purge("a").unwrap());
        assert!(store.list_trash().unwrap().is_empty(), "purge はゴミ箱を経由しない");
        store.delete("b").unwrap();
        store.empty_trash().unwrap();
        assert!(store.list_trash().unwrap().is_empty());
    }

    #[test]
    fn trash_older_than_retention_is_purged_on_open() {
        let db = TempDb::new("retention");
        let master = crypto::generate_master_key();
        {
            let store = SecretStore::open_at(&db.0, master).unwrap();
            store.set("old", "1").unwrap();
            store.set("recent", "2").unwrap();
            store.delete("old").unwrap();
            store.delete("recent").unwrap();
            store
                .conn
                .execute("UPDATE trash SET deleted_at = datetime('now', '-31 days') WHERE key = 'old'", [])
                .unwrap();
        }
        let store = SecretStore::open_at(&db.0, master).unwrap();
        let keys: Vec<_> = store.list_trash().unwrap().into_iter().map(|t| t.key).collect();
        assert_eq!(keys, vec!["recent".to_string()]);
    }

    #[test]
    fn rename_is_atomic_and_refuses_existing_target() {
        let db = TempDb::new("rename");
        let store = SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        store.set("a", "va").unwrap();
        store.set("b", "vb").unwrap();
        assert!(store.rename("a", "b").is_err());
        store.rename("a", "c").unwrap();
        assert!(store.get("a").is_err());
        assert_eq!(store.get("c").unwrap(), "va");
        assert!(store.rename("missing", "z").is_err());
    }

    #[test]
    fn migrates_v1_schema_to_v2_preserving_rows() {
        let db = TempDb::new("migrate-v1");
        let master = crypto::generate_master_key();
        {
            let conn = Connection::open(&db.0).unwrap();
            conn.execute_batch(
                "CREATE TABLE secrets (
                    key TEXT PRIMARY KEY,
                    value BLOB NOT NULL,
                    created_at TEXT DEFAULT (datetime('now')),
                    updated_at TEXT DEFAULT (datetime('now')),
                    binary INTEGER NOT NULL DEFAULT 0
                );
                PRAGMA user_version = 1;",
            )
            .unwrap();
            let encrypted = crypto::encrypt(&master, &[7, 7]).unwrap();
            conn.execute(
                "INSERT INTO secrets (key, value, binary) VALUES ('ks', ?1, 1)",
                params![encrypted],
            )
            .unwrap();
        }
        let store = SecretStore::open_at(&db.0, master).unwrap();
        assert_eq!(store.get_bytes("ks").unwrap(), (vec![7, 7], true));
        let version: i64 = store.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn database_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let db = TempDb::new("perms");
        std::fs::write(&db.0, b"").unwrap();
        std::fs::set_permissions(&db.0, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&db.0).unwrap();
        SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        let mode = std::fs::metadata(&db.0).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "新規の vault.db は本人だけが読める");
    }

    #[test]
    fn existing_database_permissions_are_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let db = TempDb::new("perms-existing");
        SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        std::fs::set_permissions(&db.0, std::fs::Permissions::from_mode(0o644)).unwrap();
        SecretStore::open_at(&db.0, crypto::generate_master_key()).unwrap();
        let mode = std::fs::metadata(&db.0).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn refuses_database_from_newer_vlt() {
        let db = TempDb::new("future");
        {
            let conn = Connection::open(&db.0).unwrap();
            conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        }
        let err = SecretStore::open_at(&db.0, crypto::generate_master_key()).err().unwrap();
        assert!(err.contains("新しい vlt"), "{err}");
    }

    #[test]
    fn export_import_keeps_structured_items() {
        let db1 = TempDb::new("export-item-src");
        let src = SecretStore::open_at(&db1.0, crypto::generate_master_key()).unwrap();
        let mut item = Item::new(ItemType::Login);
        item.field_mut("username").unwrap().value = "alice".into();
        item.tags = vec!["work".into()];
        src.put_item("site", &item).unwrap();
        let entries = src.export_entries().unwrap();

        let db2 = TempDb::new("export-item-dst");
        let dst = SecretStore::open_at(&db2.0, crypto::generate_master_key()).unwrap();
        dst.import_entries(&entries, false).unwrap();
        assert_eq!(dst.get_item("site").unwrap(), item);
    }
}
