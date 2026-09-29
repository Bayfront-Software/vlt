//! GUI の操作をまとめた層。Tauri のコマンドはこの関数を呼ぶだけにして、
//! ここは `SecretStore` だけを受け取る純粋な形に保つ（テストが一時 DB で回せる）。
//!
//! 画面に渡すデータの原則:
//! - 一覧と詳細（`list` / `view`）には伏せる種類の値を入れない（`value: None`）。
//! - 値が WebView に渡るのは「表示」（`reveal_field`）と「編集」（`editable`）だけ。
//! - ファイルの中身は一度も WebView に渡さない（保存・取り込みは Rust 側のダイアログ）。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use vlt::generator::{self, Strength};
use vlt::item::{Field, FieldKind, Item, ItemType, NOTES_FIELD};
use vlt::portable::{self, Entry};
use vlt::reference;
use vlt::store::SecretStore;
use vlt::totp::{self, TotpCode};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TypeInfo {
    pub id: &'static str,
    pub label: &'static str,
}

pub fn item_types() -> Vec<TypeInfo> {
    ItemType::ALL
        .into_iter()
        .map(|t| TypeInfo { id: t.as_str(), label: t.label() })
        .collect()
}

/// 一覧の1行。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Summary {
    pub key: String,
    pub item_type: &'static str,
    pub type_label: &'static str,
    pub subtitle: String,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    /// パスワード欄が弱い。
    pub weak: bool,
    /// 同じパスワードを他の項目でも使っている。
    pub reused: bool,
}

/// 使い回しと弱さの判定対象にするのは「人が決めたパスワード」の欄だけ。
/// API トークンや秘密鍵まで入れると、長い乱数を「使い回し」扱いするなど雑音になる。
fn password_of(item: &Item) -> Option<&str> {
    item.field("password")
        .filter(|f| f.id == "password" && f.kind == FieldKind::Concealed && !f.value.is_empty())
        .map(|f| f.value.as_str())
}

pub fn list(store: &SecretStore) -> Result<Vec<Summary>, String> {
    let mut rows = Vec::new();
    for summary in store.list_items()? {
        let item = store.get_item(&summary.key)?;
        rows.push((summary, item));
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (_, item) in &rows {
        if let Some(pw) = password_of(item) {
            *counts.entry(pw).or_default() += 1;
        }
    }
    Ok(rows
        .iter()
        .map(|(summary, item)| {
            let pw = password_of(item);
            Summary {
                key: summary.key.clone(),
                item_type: item.item_type.as_str(),
                type_label: item.item_type.label(),
                subtitle: item.subtitle(),
                favorite: item.favorite,
                tags: item.tags.clone(),
                created_at: summary.created_at.clone(),
                updated_at: summary.updated_at.clone(),
                weak: pw.is_some_and(|p| generator::strength(p) == Strength::Weak),
                reused: pw.is_some_and(|p| counts[p] > 1),
            }
        })
        .collect())
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FieldView {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    pub secret: bool,
    pub has_value: bool,
    /// 伏せない種類のときだけ入る。
    pub value: Option<String>,
    pub filename: Option<String>,
    pub size: Option<usize>,
    pub reference: String,
    pub strength: Option<Strength>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ItemView {
    pub key: String,
    pub item_type: &'static str,
    pub type_label: &'static str,
    pub fields: Vec<FieldView>,
    pub notes: String,
    pub notes_reference: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub created_at: String,
    pub updated_at: String,
    /// `vlt://<key>` が指すもの。主役が無い種類は None。
    pub primary_field: Option<String>,
}

fn field_reference(key: &str, item: &Item, field: &Field) -> String {
    if item.item_type.primary_field() == Some(field.id.as_str()) {
        reference::build(key, None)
    } else {
        reference::build(key, Some(&field.id))
    }
}

pub fn view(store: &SecretStore, key: &str) -> Result<ItemView, String> {
    let item = store.get_item(key)?;
    let summary = store
        .list_items()?
        .into_iter()
        .find(|s| s.key == key)
        .ok_or_else(|| format!("Secret not found: {key}"))?;
    let fields = item
        .fields
        .iter()
        .map(|f| {
            let secret = f.kind.is_secret();
            FieldView {
                id: f.id.clone(),
                label: f.label.clone(),
                kind: f.kind,
                secret,
                has_value: !f.value.is_empty(),
                value: (!secret).then(|| f.value.clone()),
                filename: f.filename.clone(),
                size: (f.kind == FieldKind::File).then(|| f.file_bytes().map(|b| b.len()).unwrap_or(0)),
                reference: field_reference(key, &item, f),
                strength: (f.kind == FieldKind::Concealed && !f.value.is_empty())
                    .then(|| generator::strength(&f.value)),
            }
        })
        .collect();
    Ok(ItemView {
        key: key.to_string(),
        item_type: item.item_type.as_str(),
        type_label: item.item_type.label(),
        fields,
        notes: item.notes.clone(),
        notes_reference: if item.item_type == ItemType::SecureNote {
            reference::build(key, None)
        } else {
            reference::build(key, Some(NOTES_FIELD))
        },
        tags: item.tags.clone(),
        favorite: item.favorite,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        primary_field: item.item_type.primary_field().map(str::to_string),
    })
}

fn find_field<'a>(item: &'a Item, key: &str, field_id: &str) -> Result<&'a Field, String> {
    item.fields
        .iter()
        .find(|f| f.id == field_id)
        .ok_or_else(|| format!("{key} にフィールド {field_id} がありません"))
}

/// 伏せていた値を返す。ファイルは画面に出さない。
pub fn reveal_field(store: &SecretStore, key: &str, field_id: &str) -> Result<String, String> {
    let item = store.get_item(key)?;
    let field = find_field(&item, key, field_id)?;
    if field.kind == FieldKind::File {
        return Err("ファイルは画面に表示できません。「保存」を使ってください".into());
    }
    Ok(field.value.clone())
}

/// コピーする文字列。ワンタイムパスワードは秘密ではなく現在のコードを渡す。
pub fn copy_text(store: &SecretStore, key: &str, field_id: &str) -> Result<String, String> {
    if field_id == NOTES_FIELD {
        return Ok(store.get_item(key)?.notes);
    }
    let item = store.get_item(key)?;
    let field = find_field(&item, key, field_id)?;
    match field.kind {
        FieldKind::File => Err("ファイルはコピーできません".into()),
        FieldKind::Totp => Ok(totp::current_code(&field.value)?.code),
        _ if field.value.is_empty() => Err(format!("{} は空です", field.label)),
        _ => Ok(field.value.clone()),
    }
}

pub fn totp_code(store: &SecretStore, key: &str, field_id: &str) -> Result<TotpCode, String> {
    let item = store.get_item(key)?;
    let field = find_field(&item, key, field_id)?;
    if field.kind != FieldKind::Totp {
        return Err(format!("{} はワンタイムパスワードではありません", field.label));
    }
    totp::current_code(&field.value)
}

/// 保存済みファイルの中身（Rust 側のダイアログで書き出す用）。
pub fn file_bytes(store: &SecretStore, key: &str, field_id: &str) -> Result<(String, Vec<u8>), String> {
    let item = store.get_item(key)?;
    let field = find_field(&item, key, field_id)?;
    if field.kind != FieldKind::File {
        return Err(format!("{} はファイルではありません", field.label));
    }
    let fallback = key.rsplit('/').next().unwrap_or("file").to_string();
    let name = field.filename.clone().filter(|n| !n.is_empty()).unwrap_or(fallback);
    Ok((name, field.file_bytes()?))
}

/// 編集フォームとやり取りする形。ファイルの中身は含めない。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EditableField {
    /// 新しく足したカスタムフィールドは空（保存時に採番）。
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub filename: Option<String>,
    /// ファイル欄: 新しく選んだファイルの受け取り番号。None なら既存の中身を保つ。
    #[serde(default)]
    pub pending_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EditableItem {
    pub item_type: ItemType,
    pub fields: Vec<EditableField>,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
}

fn to_editable(item: &Item) -> EditableItem {
    EditableItem {
        item_type: item.item_type,
        fields: item
            .fields
            .iter()
            .map(|f| EditableField {
                id: f.id.clone(),
                label: f.label.clone(),
                kind: f.kind,
                value: if f.kind == FieldKind::File { String::new() } else { f.value.clone() },
                filename: f.filename.clone(),
                pending_file: None,
            })
            .collect(),
        notes: item.notes.clone(),
        tags: item.tags.clone(),
        favorite: item.favorite,
    }
}

pub fn editable(store: &SecretStore, key: &str) -> Result<EditableItem, String> {
    Ok(to_editable(&store.get_item(key)?))
}

pub fn template(item_type: ItemType) -> EditableItem {
    to_editable(&Item::new(item_type))
}

pub fn validate_key(key: &str) -> Result<(), String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("名前（キー）を入力してください".into());
    }
    if trimmed != key {
        return Err("名前の前後に空白は使えません".into());
    }
    if key.chars().any(char::is_control) {
        return Err("名前に制御文字は使えません".into());
    }
    if key.starts_with('/') || key.ends_with('/') || key.contains("//") {
        return Err("名前の / は区切りにだけ使えます（先頭・末尾・連続は不可）".into());
    }
    if key.contains('?') {
        return Err("名前に ? は使えません（秘密参照のクエリと区別できなくなるため）".into());
    }
    Ok(())
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.to_lowercase()))
        .collect()
}

/// ファイル取り込みの一時置き場（受け取り番号 → 名前と中身）。
pub type PendingFiles = HashMap<String, (String, Vec<u8>)>;

/// 項目を保存する。`original_key` が Some なら既存項目の編集（キーが変われば改名）。
pub fn save(
    store: &SecretStore,
    original_key: Option<&str>,
    key: &str,
    edited: &EditableItem,
    pending: &mut PendingFiles,
) -> Result<(), String> {
    validate_key(key)?;
    let existing = match original_key {
        Some(orig) => Some(store.get_item(orig)?),
        None => None,
    };
    let target_taken = store.exists(key)?;
    if target_taken && original_key != Some(key) {
        return Err(format!("{key} は既にあります"));
    }

    let mut item = Item {
        item_type: edited.item_type,
        fields: vec![],
        notes: edited.notes.clone(),
        tags: normalize_tags(&edited.tags),
        favorite: edited.favorite,
    };
    for f in &edited.fields {
        let label = f.label.trim();
        if label.is_empty() {
            return Err("フィールド名が空の欄があります".into());
        }
        let id = if f.id.is_empty() { item.unique_field_id(label) } else { f.id.clone() };
        if item.fields.iter().any(|x| x.id == id) {
            return Err(format!("フィールド {id} が重複しています"));
        }
        let mut field = Field {
            id,
            label: label.to_string(),
            kind: f.kind,
            value: f.value.clone(),
            filename: f.filename.clone(),
        };
        if f.kind == FieldKind::File {
            if let Some(token) = &f.pending_file {
                let (name, bytes) = pending
                    .get(token)
                    .ok_or("選んだファイルが見つかりません。もう一度選んでください")?;
                field = Field { id: field.id, label: field.label, ..Field::file(name, bytes) };
            } else {
                let old = existing
                    .as_ref()
                    .and_then(|e| e.fields.iter().find(|x| x.id == field.id && x.kind == FieldKind::File));
                field.value = old.map(|o| o.value.clone()).unwrap_or_default();
            }
        }
        if f.kind == FieldKind::Totp && !field.value.trim().is_empty() {
            totp::parse(&field.value).map_err(|e| format!("{}: {e}", field.label))?;
            field.value = field.value.trim().to_string();
        }
        item.fields.push(field);
    }
    if item.item_type == ItemType::Document
        && !item.fields.iter().any(|f| f.kind == FieldKind::File && !f.value.is_empty())
    {
        return Err("書類にはファイルを選んでください".into());
    }

    if let Some(orig) = original_key {
        if orig != key {
            store.rename(orig, key)?;
        }
    }
    store.put_item(key, &item)?;
    for f in &edited.fields {
        if let Some(token) = &f.pending_file {
            pending.remove(token);
        }
    }
    Ok(())
}

pub fn set_favorite(store: &SecretStore, key: &str, favorite: bool) -> Result<(), String> {
    let mut item = store.get_item(key)?;
    item.favorite = favorite;
    store.put_item(key, &item)
}

/// 「<キー> のコピー」を作って、そのキーを返す。
pub fn duplicate(store: &SecretStore, key: &str) -> Result<String, String> {
    let mut item = store.get_item(key)?;
    item.favorite = false;
    let base = format!("{key} copy");
    let mut candidate = base.clone();
    let mut n = 2;
    while store.exists(&candidate)? {
        candidate = format!("{base} {n}");
        n += 1;
    }
    store.put_item(&candidate, &item)?;
    Ok(candidate)
}

pub fn delete(store: &SecretStore, key: &str) -> Result<(), String> {
    if store.delete(key)? {
        Ok(())
    } else {
        Err(format!("Secret not found: {key}"))
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TrashView {
    pub id: i64,
    pub key: String,
    pub type_label: &'static str,
    pub item_type: &'static str,
    pub deleted_at: String,
}

pub fn trash(store: &SecretStore) -> Result<Vec<TrashView>, String> {
    Ok(store
        .list_trash()?
        .into_iter()
        .map(|t| TrashView {
            id: t.id,
            key: t.key,
            type_label: t.item_type.label(),
            item_type: t.item_type.as_str(),
            deleted_at: t.deleted_at,
        })
        .collect())
}

pub fn export_sealed(store: &SecretStore, passphrase: &str) -> Result<Vec<u8>, String> {
    if passphrase.is_empty() {
        return Err("パスフレーズを入力してください".into());
    }
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
    if passphrase.is_empty() {
        return Err("パスフレーズを入力してください".into());
    }
    let entries: Vec<Entry> = portable::unseal(data, passphrase)?;
    store.import_entries(&entries, overwrite)
}

/// ウェブサイト欄を開くときに許す URL（file:// やアプリ起動スキームを開かせない）。
pub fn is_openable_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && !url.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use vlt::crypto::generate_master_key;

    struct TempDb(PathBuf);
    impl TempDb {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("vlt-gui-test-{name}-{}.db", std::process::id()));
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

    fn login(username: &str, password: &str) -> EditableItem {
        let mut e = template(ItemType::Login);
        e.fields.iter_mut().find(|f| f.id == "username").unwrap().value = username.into();
        e.fields.iter_mut().find(|f| f.id == "password").unwrap().value = password.into();
        e
    }

    fn save_new(store: &SecretStore, key: &str, e: &EditableItem) {
        save(store, None, key, e, &mut PendingFiles::new()).unwrap();
    }

    #[test]
    fn list_and_view_never_carry_concealed_values() {
        let db = TempDb::new("no-leak");
        let store = db.open();
        save_new(&store, "github", &login("alice", "hunter2-secret"));
        let json = serde_json::to_string(&(list(&store).unwrap(), view(&store, "github").unwrap())).unwrap();
        assert!(!json.contains("hunter2-secret"), "一覧・詳細に値が漏れてはいけない");
        assert!(json.contains("alice"), "伏せない値は出る");
    }

    #[test]
    fn view_carries_references_for_every_field() {
        let db = TempDb::new("refs");
        let store = db.open();
        save_new(&store, "dev/github", &login("alice", "pw"));
        let v = view(&store, "dev/github").unwrap();
        let reference_of = |id: &str| v.fields.iter().find(|f| f.id == id).unwrap().reference.clone();
        assert_eq!(reference_of("password"), "vlt://dev/github");
        assert_eq!(reference_of("username"), "vlt://dev/github/username");
        assert_eq!(v.notes_reference, "vlt://dev/github/notes");
        // 表示している参照が実際に解決できること
        for f in v.fields.iter().filter(|f| f.has_value) {
            let parsed = reference::parse(&f.reference).unwrap();
            assert!(reference::resolve(&store, &parsed).is_ok(), "{}", f.reference);
        }
    }

    #[test]
    fn weak_and_reused_passwords_are_flagged_only_on_password_fields() {
        let db = TempDb::new("watch");
        let store = db.open();
        save_new(&store, "a", &login("x", "password"));
        save_new(&store, "b", &login("y", "password"));
        save_new(&store, "c", &login("z", "Xk9#mP2$vL8@qR4!"));
        let mut api = template(ItemType::ApiCredential);
        api.fields.iter_mut().find(|f| f.id == "credential").unwrap().value = "short".into();
        save_new(&store, "api", &api);
        let rows = list(&store).unwrap();
        let get = |k: &str| rows.iter().find(|r| r.key == k).unwrap();
        assert!(get("a").weak && get("a").reused);
        assert!(!get("c").weak && !get("c").reused);
        assert!(!get("api").weak, "API トークンは対象外");
    }

    #[test]
    fn copy_text_gives_totp_code_not_secret() {
        let db = TempDb::new("copy-totp");
        let store = db.open();
        let mut e = login("alice", "pw");
        e.fields.iter_mut().find(|f| f.id == "one_time_password").unwrap().value = "GEZDGNBVGY3TQOJQ".into();
        save_new(&store, "g", &e);
        let code = copy_text(&store, "g", "one_time_password").unwrap();
        assert_eq!(code.len(), 6);
        assert_eq!(copy_text(&store, "g", "password").unwrap(), "pw");
        assert!(copy_text(&store, "g", "website").is_err(), "空欄はコピーしない");
    }

    #[test]
    fn save_rejects_invalid_totp_secret() {
        let db = TempDb::new("bad-totp");
        let store = db.open();
        let mut e = login("alice", "pw");
        e.fields.iter_mut().find(|f| f.id == "one_time_password").unwrap().value = "not-base32!".into();
        let err = save(&store, None, "g", &e, &mut PendingFiles::new()).unwrap_err();
        assert!(err.contains("ワンタイムパスワード"), "{err}");
    }

    #[test]
    fn save_assigns_ids_to_custom_fields_and_normalizes_tags() {
        let db = TempDb::new("custom");
        let store = db.open();
        let mut e = login("alice", "pw");
        e.fields.push(EditableField {
            id: String::new(),
            label: "Recovery Code".into(),
            kind: FieldKind::Concealed,
            value: "R1".into(),
            filename: None,
            pending_file: None,
        });
        e.tags = vec![" work ".into(), "Work".into(), "".into(), "dev".into()];
        save_new(&store, "g", &e);
        let item = store.get_item("g").unwrap();
        assert_eq!(item.field("recovery_code").unwrap().value, "R1");
        assert_eq!(item.tags, vec!["work".to_string(), "dev".to_string()]);
    }

    #[test]
    fn save_edit_with_new_key_renames_and_keeps_file_when_untouched() {
        let db = TempDb::new("edit");
        let store = db.open();
        let mut pending = PendingFiles::new();
        pending.insert("t1".into(), ("key.jks".into(), vec![1, 2, 3]));
        let mut doc = template(ItemType::Document);
        doc.fields[0].pending_file = Some("t1".into());
        save(&store, None, "old", &doc, &mut pending).unwrap();
        assert!(pending.is_empty(), "使った受け取り番号は消える");

        let mut edited = editable(&store, "old").unwrap();
        assert_eq!(edited.fields[0].value, "", "編集フォームにファイルの中身は渡さない");
        edited.notes = "memo".into();
        save(&store, Some("old"), "new", &edited, &mut pending).unwrap();
        assert!(!store.exists("old").unwrap());
        let (name, bytes) = file_bytes(&store, "new", "file").unwrap();
        assert_eq!((name.as_str(), bytes), ("key.jks", vec![1, 2, 3]));
        assert_eq!(store.get_item("new").unwrap().notes, "memo");
    }

    #[test]
    fn save_refuses_taken_key_and_empty_document() {
        let db = TempDb::new("refuse");
        let store = db.open();
        save_new(&store, "a", &login("x", "1"));
        assert!(save(&store, None, "a", &login("y", "2"), &mut PendingFiles::new()).is_err());
        save_new(&store, "b", &login("x", "1"));
        let e = editable(&store, "b").unwrap();
        assert!(save(&store, Some("b"), "a", &e, &mut PendingFiles::new()).is_err(), "改名先が既存");
        assert!(save(&store, None, "doc", &template(ItemType::Document), &mut PendingFiles::new()).is_err());
    }

    #[test]
    fn validate_key_rules() {
        for bad in ["", " a", "a ", "/a", "a/", "a//b", "a?b", "a\nb"] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
        for good in ["a", "openai/api-key", "android/hushcam/keystore", "日本語/キー"] {
            assert!(validate_key(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn reveal_refuses_files() {
        let db = TempDb::new("reveal");
        let store = db.open();
        store.set_bytes("doc", &[9], true).unwrap();
        assert!(reveal_field(&store, "doc", "file").is_err());
    }

    #[test]
    fn duplicate_picks_free_name_and_drops_favorite() {
        let db = TempDb::new("dup");
        let store = db.open();
        let mut e = login("x", "1");
        e.favorite = true;
        save_new(&store, "a", &e);
        assert_eq!(duplicate(&store, "a").unwrap(), "a copy");
        assert_eq!(duplicate(&store, "a").unwrap(), "a copy 2");
        assert!(!store.get_item("a copy").unwrap().favorite);
    }

    #[test]
    fn trash_roundtrip_through_ops() {
        let db = TempDb::new("trash");
        let store = db.open();
        store.set("k", "v").unwrap();
        delete(&store, "k").unwrap();
        let t = trash(&store).unwrap();
        assert_eq!(t[0].key, "k");
        store.restore(t[0].id).unwrap();
        assert_eq!(store.get("k").unwrap(), "v");
    }

    #[test]
    fn export_import_roundtrip_with_passphrase() {
        let src_db = TempDb::new("export");
        let src = src_db.open();
        save_new(&src, "g", &login("alice", "pw"));
        let sealed = export_sealed(&src, "pass").unwrap();
        let dst_db = TempDb::new("import");
        let dst = dst_db.open();
        assert!(import_sealed(&dst, &sealed, "wrong", false).is_err());
        assert_eq!(import_sealed(&dst, &sealed, "pass", false).unwrap(), (1, 0));
        assert_eq!(dst.get_item("g").unwrap().field("username").unwrap().value, "alice");
    }

    #[test]
    fn only_http_urls_are_openable() {
        assert!(is_openable_url("https://github.com"));
        assert!(is_openable_url("HTTP://example.com"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "vscode://x", "ftp://x", "https://a\nb"] {
            assert!(!is_openable_url(bad), "{bad}");
        }
    }
}
