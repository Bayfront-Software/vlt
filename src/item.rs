//! 項目（1Password の「アイテム」に相当）のモデル。
//!
//! vault の1行 = 1項目。キー（例: `openai/api-key`）が識別子で、秘密参照
//! `vlt://<キー>/<フィールド>` の前半になる。中身は種類ごとのテンプレートから
//! 作ったフィールド列＋メモ＋タグ＋お気に入りで、JSON にして暗号化して保存する。
//!
//! v0.2 以前の行は「生の値＋binary フラグ」なので、読むときに
//! [`Item::from_legacy`] でパスワード項目／書類項目として見せる（DB は書き換えない）。

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Login,
    Password,
    ApiCredential,
    SecureNote,
    CreditCard,
    Identity,
    SshKey,
    Database,
    Server,
    SoftwareLicense,
    Wifi,
    BankAccount,
    Document,
    /// 環境変数の組（1Password Environments / envchain / Doppler 相当）。
    /// フィールドのラベルが変数名になり、`vlt run --env <key>` でまとめて注入する。
    Environment,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Multiline,
    /// 伏せて表示する1行の秘密（パスワード・トークン）。
    Concealed,
    /// 伏せて表示する複数行の秘密（秘密鍵・ライセンスキー）。
    SecretBlock,
    Url,
    Email,
    Phone,
    Date,
    /// otpauth:// URI か base32 の秘密。参照や画面では現在のコードを出せる。
    Totp,
    /// ファイル本体（value は base64、filename に元の名前）。
    File,
}

impl FieldKind {
    /// 画面で既定では伏せる種類か。
    pub fn is_secret(self) -> bool {
        matches!(self, Self::Concealed | Self::SecretBlock | Self::Totp | Self::File)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Field {
    pub id: String,
    pub label: String,
    pub kind: FieldKind,
    #[serde(default)]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

impl Field {
    pub fn new(id: &str, label: &str, kind: FieldKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            value: String::new(),
            filename: None,
        }
    }

    pub fn file(filename: &str, bytes: &[u8]) -> Self {
        Self {
            id: "file".into(),
            label: "ファイル".into(),
            kind: FieldKind::File,
            value: B64.encode(bytes),
            filename: Some(filename.into()),
        }
    }

    /// File フィールドの中身を取り出す。
    pub fn file_bytes(&self) -> Result<Vec<u8>, String> {
        B64.decode(&self.value)
            .map_err(|e| format!("ファイルの中身が壊れています: {e}"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Item {
    #[serde(rename = "type")]
    pub item_type: ItemType,
    #[serde(default)]
    pub fields: Vec<Field>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
}

/// 参照・`vlt get` で「フィールド名なし」のときに返す値。
#[derive(Debug, PartialEq)]
pub enum PrimaryValue<'a> {
    Text(&'a str),
    File(&'a Field),
}

/// メモ欄を指すフィールド名（参照 `vlt://key/notes`）。
pub const NOTES_FIELD: &str = "notes";

impl ItemType {
    pub const ALL: [ItemType; 14] = [
        Self::Login,
        Self::Password,
        Self::ApiCredential,
        Self::SecureNote,
        Self::CreditCard,
        Self::Identity,
        Self::SshKey,
        Self::Database,
        Self::Server,
        Self::SoftwareLicense,
        Self::Wifi,
        Self::BankAccount,
        Self::Document,
        Self::Environment,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Password => "password",
            Self::ApiCredential => "api_credential",
            Self::SecureNote => "secure_note",
            Self::CreditCard => "credit_card",
            Self::Identity => "identity",
            Self::SshKey => "ssh_key",
            Self::Database => "database",
            Self::Server => "server",
            Self::SoftwareLicense => "software_license",
            Self::Wifi => "wifi",
            Self::BankAccount => "bank_account",
            Self::Document => "document",
            Self::Environment => "environment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "ログイン",
            Self::Password => "パスワード",
            Self::ApiCredential => "API 認証情報",
            Self::SecureNote => "セキュアノート",
            Self::CreditCard => "クレジットカード",
            Self::Identity => "個人情報",
            Self::SshKey => "SSH 鍵",
            Self::Database => "データベース",
            Self::Server => "サーバー",
            Self::SoftwareLicense => "ソフトウェアライセンス",
            Self::Wifi => "Wi-Fi",
            Self::BankAccount => "銀行口座",
            Self::Document => "書類",
            Self::Environment => "環境変数",
        }
    }

    /// フィールド名を省いた参照が指すフィールド。None はメモ欄（セキュアノート）か、
    /// 主役が決まらない種類（個人情報）。
    pub fn primary_field(self) -> Option<&'static str> {
        match self {
            Self::Login | Self::Password | Self::Database | Self::Server | Self::Wifi => Some("password"),
            Self::ApiCredential => Some("credential"),
            Self::CreditCard => Some("number"),
            Self::SshKey => Some("private_key"),
            Self::SoftwareLicense => Some("license_key"),
            Self::BankAccount => Some("account_number"),
            Self::Document => Some("file"),
            Self::SecureNote | Self::Identity | Self::Environment => None,
        }
    }

    /// 新規作成時のフィールド雛形。
    pub fn template(self) -> Vec<Field> {
        use FieldKind::*;
        let f = Field::new;
        match self {
            Self::Login => vec![
                f("username", "ユーザー名", Text),
                f("password", "パスワード", Concealed),
                f("website", "ウェブサイト", Url),
                f("one_time_password", "ワンタイムパスワード", Totp),
            ],
            Self::Password => vec![f("password", "パスワード", Concealed)],
            Self::ApiCredential => vec![
                f("username", "ユーザー名", Text),
                f("credential", "認証情報", Concealed),
                f("type", "種類", Text),
                f("hostname", "ホスト名", Text),
                f("expires", "有効期限", Date),
            ],
            Self::SecureNote => vec![],
            Self::CreditCard => vec![
                f("cardholder", "カード名義", Text),
                f("number", "カード番号", Concealed),
                f("expiry", "有効期限 (MM/YY)", Text),
                f("cvv", "セキュリティコード", Concealed),
                f("pin", "PIN", Concealed),
                f("brand", "ブランド", Text),
            ],
            Self::Identity => vec![
                f("last_name", "姓", Text),
                f("first_name", "名", Text),
                f("email", "メール", Email),
                f("phone", "電話番号", Phone),
                f("address", "住所", Multiline),
                f("birth_date", "生年月日", Date),
                f("company", "会社", Text),
            ],
            Self::SshKey => vec![
                f("private_key", "秘密鍵", SecretBlock),
                f("public_key", "公開鍵", Multiline),
                f("fingerprint", "フィンガープリント", Text),
                f("key_type", "鍵の種類", Text),
            ],
            Self::Database => vec![
                f("type", "種類", Text),
                f("server", "サーバー", Text),
                f("port", "ポート", Text),
                f("database", "データベース名", Text),
                f("username", "ユーザー名", Text),
                f("password", "パスワード", Concealed),
                f("connection_string", "接続文字列", Concealed),
            ],
            Self::Server => vec![
                f("url", "URL", Url),
                f("username", "ユーザー名", Text),
                f("password", "パスワード", Concealed),
                f("admin_console", "管理画面", Url),
            ],
            Self::SoftwareLicense => vec![
                f("license_key", "ライセンスキー", SecretBlock),
                f("version", "バージョン", Text),
                f("licensed_to", "登録名", Text),
                f("email", "登録メール", Email),
                f("purchase_date", "購入日", Date),
            ],
            Self::Wifi => vec![
                f("ssid", "ネットワーク名", Text),
                f("password", "パスワード", Concealed),
                f("security", "セキュリティ", Text),
            ],
            Self::BankAccount => vec![
                f("bank_name", "銀行名", Text),
                f("branch", "支店", Text),
                f("account_type", "口座種別", Text),
                f("account_number", "口座番号", Concealed),
                f("holder", "名義", Text),
                f("pin", "暗証番号", Concealed),
            ],
            Self::Environment => vec![],
            Self::Document => vec![Field {
                filename: Some(String::new()),
                ..f("file", "ファイル", File)
            }],
        }
    }
}

impl Item {
    pub fn new(item_type: ItemType) -> Self {
        Self {
            item_type,
            fields: item_type.template(),
            notes: String::new(),
            tags: vec![],
            favorite: false,
        }
    }

    /// v0.2 以前の行（生の値＋binary）を項目として解釈する。
    /// テキストはパスワード項目、バイナリは書類項目になる。
    pub fn from_legacy(key: &str, value: &[u8], binary: bool) -> Result<Self, String> {
        if binary {
            let filename = key.rsplit('/').next().unwrap_or(key);
            let mut item = Self::new(ItemType::Document);
            item.fields = vec![Field::file(filename, value)];
            return Ok(item);
        }
        let text = String::from_utf8(value.to_vec()).map_err(|e| format!("UTF-8 decode error: {e}"))?;
        let mut item = Self::new(ItemType::Password);
        item.fields[0].value = text;
        Ok(item)
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Item は常に JSON 化できる")
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("項目の形式が壊れています: {e}"))
    }

    /// id の完全一致 → ラベルの大文字小文字を無視した一致、の順で探す。
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.id == name)
            .or_else(|| self.fields.iter().find(|f| f.label.eq_ignore_ascii_case(name)))
    }

    pub fn field_mut(&mut self, name: &str) -> Option<&mut Field> {
        let idx = self
            .fields
            .iter()
            .position(|f| f.id == name)
            .or_else(|| self.fields.iter().position(|f| f.label.eq_ignore_ascii_case(name)))?;
        self.fields.get_mut(idx)
    }

    pub fn primary(&self) -> Option<PrimaryValue<'_>> {
        match self.item_type.primary_field() {
            None if self.item_type == ItemType::SecureNote => Some(PrimaryValue::Text(&self.notes)),
            None => None,
            Some(id) => {
                let field = self.field(id)?;
                Some(match field.kind {
                    FieldKind::File => PrimaryValue::File(field),
                    _ => PrimaryValue::Text(&field.value),
                })
            }
        }
    }

    /// 主役フィールドがファイルか（CLI の `get` がテキスト出力を拒む判定に使う）。
    pub fn is_binary(&self) -> bool {
        matches!(self.primary(), Some(PrimaryValue::File(_)))
    }

    /// 主役フィールドにテキストを書く（CLI の `vlt set <key> <value>`）。
    pub fn set_primary_text(&mut self, value: &str) -> Result<(), String> {
        match self.item_type.primary_field() {
            None if self.item_type == ItemType::SecureNote => {
                self.notes = value.to_string();
                Ok(())
            }
            None => Err(format!("{} には主役のフィールドが無いので --field で指定してください", self.item_type.label())),
            Some(id) => {
                let field = self.field_mut(id).ok_or_else(|| format!("フィールド {id} がありません"))?;
                if field.kind == FieldKind::File {
                    return Err("書類項目の中身は --file で置き換えてください".into());
                }
                field.value = value.to_string();
                Ok(())
            }
        }
    }

    /// 一覧の2行目に出す、伏せる必要のない代表値（ユーザー名など）。
    pub fn subtitle(&self) -> String {
        if self.item_type == ItemType::Environment {
            return format!("{} 個の変数", self.fields.len());
        }
        const PREFERRED: [&str; 7] = ["username", "ssid", "cardholder", "email", "server", "url", "bank_name"];
        for id in PREFERRED {
            if let Some(field) = self.field(id) {
                if !field.value.is_empty() && !field.kind.is_secret() {
                    return field.value.clone();
                }
            }
        }
        if let Some(field) = self.fields.iter().find(|f| f.kind == FieldKind::File) {
            return field.filename.clone().unwrap_or_default();
        }
        self.item_type.label().to_string()
    }

    /// ラベルから重複しない id を作る（カスタムフィールド追加時）。
    pub fn unique_field_id(&self, label: &str) -> String {
        let slug: String = label
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
            .split('_')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        let base = if slug.is_empty() { "field".to_string() } else { slug };
        if self.fields.iter().all(|f| f.id != base) && base != NOTES_FIELD {
            return base;
        }
        (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|id| self.fields.iter().all(|f| &f.id != id))
            .expect("無限列から必ず見つかる")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_type_has_label_and_roundtrips_through_str() {
        for t in ItemType::ALL {
            assert_eq!(ItemType::parse(t.as_str()), Some(t));
            assert!(!t.label().is_empty());
        }
    }

    #[test]
    fn every_primary_field_exists_in_its_template() {
        for t in ItemType::ALL {
            if let Some(id) = t.primary_field() {
                assert!(t.template().iter().any(|f| f.id == id), "{t:?} の雛形に {id} が無い");
            }
        }
    }

    #[test]
    fn template_field_ids_are_unique_and_never_shadow_notes() {
        for t in ItemType::ALL {
            let ids: Vec<_> = t.template().into_iter().map(|f| f.id).collect();
            let mut sorted = ids.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(ids.len(), sorted.len(), "{t:?} の id が重複");
            assert!(!ids.iter().any(|id| id == NOTES_FIELD));
        }
    }

    #[test]
    fn environment_item_has_no_template_and_no_primary() {
        let mut env = Item::new(ItemType::Environment);
        assert!(env.fields.is_empty());
        assert!(env.primary().is_none());
        assert_eq!(ItemType::parse("environment"), Some(ItemType::Environment));
        env.fields.push(Field::new("a", "A", FieldKind::Concealed));
        env.fields.push(Field::new("b", "B", FieldKind::Text));
        assert_eq!(env.subtitle(), "2 個の変数");
    }

    #[test]
    fn legacy_text_becomes_password_item() {
        let item = Item::from_legacy("notion/token", b"secret", false).unwrap();
        assert_eq!(item.item_type, ItemType::Password);
        assert_eq!(item.primary(), Some(PrimaryValue::Text("secret")));
        assert!(!item.is_binary());
    }

    #[test]
    fn legacy_binary_becomes_document_named_after_key() {
        let item = Item::from_legacy("android/upload-keystore", &[0xfe, 0x00], true).unwrap();
        assert_eq!(item.item_type, ItemType::Document);
        let Some(PrimaryValue::File(field)) = item.primary() else { panic!("file primary expected") };
        assert_eq!(field.filename.as_deref(), Some("upload-keystore"));
        assert_eq!(field.file_bytes().unwrap(), vec![0xfe, 0x00]);
        assert!(item.is_binary());
    }

    #[test]
    fn json_roundtrip_keeps_everything() {
        let mut item = Item::new(ItemType::Login);
        item.field_mut("username").unwrap().value = "me@example.com".into();
        item.field_mut("password").unwrap().value = "pw".into();
        item.notes = "memo".into();
        item.tags = vec!["work".into()];
        item.favorite = true;
        assert_eq!(Item::from_json(&item.to_json()).unwrap(), item);
    }

    #[test]
    fn field_lookup_by_id_then_label_case_insensitive() {
        let mut item = Item::new(ItemType::Login);
        item.fields.push(Field { value: "x".into(), ..Field::new("custom", "Recovery Code", FieldKind::Concealed) });
        assert_eq!(item.field("password").unwrap().id, "password");
        assert_eq!(item.field("recovery code").unwrap().id, "custom");
        assert!(item.field("nope").is_none());
    }

    #[test]
    fn secure_note_primary_is_notes_and_identity_has_none() {
        let mut note = Item::new(ItemType::SecureNote);
        note.set_primary_text("hello").unwrap();
        assert_eq!(note.primary(), Some(PrimaryValue::Text("hello")));
        let mut identity = Item::new(ItemType::Identity);
        assert!(identity.primary().is_none());
        assert!(identity.set_primary_text("x").is_err());
    }

    #[test]
    fn set_primary_text_refuses_document() {
        let mut doc = Item::new(ItemType::Document);
        assert!(doc.set_primary_text("x").is_err());
    }

    #[test]
    fn subtitle_prefers_username_and_never_shows_secrets() {
        let mut item = Item::new(ItemType::Login);
        item.field_mut("password").unwrap().value = "hunter2".into();
        assert_eq!(item.subtitle(), "ログイン");
        item.field_mut("username").unwrap().value = "alice".into();
        assert_eq!(item.subtitle(), "alice");
    }

    #[test]
    fn unique_field_id_slugifies_and_avoids_collisions() {
        let mut item = Item::new(ItemType::Login);
        assert_eq!(item.unique_field_id("Recovery Code"), "recovery_code");
        assert_eq!(item.unique_field_id("パスワード2"), "2");
        assert_eq!(item.unique_field_id("秘密"), "field");
        assert_eq!(item.unique_field_id("password"), "password_2");
        assert_eq!(item.unique_field_id("notes"), "notes_2");
        item.fields.push(Field::new("field", "x", FieldKind::Text));
        assert_eq!(item.unique_field_id("秘密"), "field_2");
    }
}
