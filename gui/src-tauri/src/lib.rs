//! vlt のデスクトップ GUI（Tauri 2）。
//!
//! 方針:
//! - 値は一覧・詳細に載せない。画面に出すのは「表示」「編集」を明示したときだけ（ops.rs）。
//! - 解錠は Touch ID（または Mac のログインパスワード）で本人確認してから Keychain の
//!   マスターキーを読む。ロックは store を捨てるだけ（マスターキーは Drop で消去）。
//! - 操作が無い時間・画面ロック・スリープで自動的にロックする。
//! - コピーは「履歴アプリに残さない」印を付け、設定秒数後にまだ同じ内容なら消す。
//! - ファイルダイアログやキーチェーン待ちで止まる処理は async コマンドにする。
//!   同期コマンドはメインスレッドで動くため、そこで待つと画面ごと固まる。

#[cfg(target_os = "macos")]
mod macos;
pub mod ops;
pub mod settings;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use vlt::generator::{self, GeneratorOptions, Strength};
use vlt::item::ItemType;
use vlt::store::{self, SecretStore};
use vlt::totp::TotpCode;
use vlt::{crypto, keychain};

use ops::{EditableItem, ItemView, PendingFiles, Summary, TrashView, TypeInfo};
use settings::Settings;

const QUICK_OPEN_SHORTCUT: &str = "CmdOrCtrl+Shift+Space";
/// 取り込めるファイルの上限。項目は JSON に base64 で入るので、巨大なファイルは避ける。
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;

struct AppState {
    vault: Mutex<Option<SecretStore>>,
    last_activity: Mutex<Instant>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    pending_files: Mutex<PendingFiles>,
}

type CmdResult<T> = Result<T, String>;

fn poisoned<T>(_: T) -> String {
    "内部状態が壊れました。アプリを再起動してください".to_string()
}

impl AppState {
    fn touch(&self) {
        if let Ok(mut t) = self.last_activity.lock() {
            *t = Instant::now();
        }
    }

    fn with_store<T>(&self, f: impl FnOnce(&SecretStore) -> CmdResult<T>) -> CmdResult<T> {
        let result = self.with_store_quiet(f);
        self.touch();
        result
    }

    /// 自動ロックのタイマーを延ばさない読み出し（画面の定期更新用）。
    /// これを使わないと、ワンタイムパスワードを表示したままではロックされなくなる。
    fn with_store_quiet<T>(&self, f: impl FnOnce(&SecretStore) -> CmdResult<T>) -> CmdResult<T> {
        let guard = self.vault.lock().map_err(poisoned)?;
        let store = guard.as_ref().ok_or("vault はロックされています")?;
        f(store)
    }

    fn settings(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// ロックして画面へ知らせる。既にロック済みなら何もしない。
    fn lock(&self, app: &AppHandle, reason: &str) {
        let was_unlocked = match self.vault.lock() {
            Ok(mut guard) => guard.take().is_some(),
            Err(_) => false,
        };
        if let Ok(mut pending) = self.pending_files.lock() {
            pending.clear();
        }
        if was_unlocked {
            let _ = app.emit("vault-locked", reason);
        }
    }
}

fn open_store_with_keychain() -> CmdResult<SecretStore> {
    let key_bytes = keychain::load_master_key()?;
    let master_key: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| "Keychain のマスターキー長が不正です。`vlt init` をやり直してください。".to_string())?;
    SecretStore::open(master_key)
}

fn random_token() -> String {
    crypto::generate_salt().iter().map(|b| format!("{b:02x}")).collect()
}

// ---------- 状態・解錠 ----------

#[derive(Serialize)]
struct AppInfo {
    initialized: bool,
    unlocked: bool,
    db_path: String,
    biometrics: bool,
    settings: Settings,
    version: &'static str,
}

#[tauri::command]
fn app_info(state: State<AppState>) -> AppInfo {
    AppInfo {
        initialized: keychain::has_master_key(),
        unlocked: state.vault.lock().map(|g| g.is_some()).unwrap_or(false),
        db_path: store::db_path().display().to_string(),
        #[cfg(target_os = "macos")]
        biometrics: macos::biometrics_available(),
        #[cfg(not(target_os = "macos"))]
        biometrics: false,
        settings: state.settings(),
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
async fn unlock(state: State<'_, AppState>) -> CmdResult<()> {
    let require_auth = state.settings().unlock_with_touch_id;
    let store = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        if require_auth {
            macos::authenticate("vault を解錠します")?;
        }
        #[cfg(not(target_os = "macos"))]
        let _ = require_auth;
        open_store_with_keychain()
    })
    .await
    .map_err(|e| e.to_string())??;
    *state.vault.lock().map_err(poisoned)? = Some(store);
    state.touch();
    Ok(())
}

#[tauri::command]
fn lock(app: AppHandle, state: State<AppState>) {
    state.lock(&app, "manual");
}

/// CLI の `vlt init` と同じガード: 既にマスターキーがあれば絶対に作り直さない。
#[tauri::command]
fn init_vault(state: State<AppState>) -> CmdResult<()> {
    if keychain::has_master_key() {
        return Err("マスターキーは既に Keychain にあります。作り直すと今の vault が二度と開けなくなるため、GUI からは行いません。".into());
    }
    let master_key = crypto::generate_master_key();
    keychain::store_master_key(&master_key)?;
    let store = SecretStore::open(master_key)?;
    *state.vault.lock().map_err(poisoned)? = Some(store);
    state.touch();
    Ok(())
}

/// 画面での操作を自動ロックのタイマーに伝える。
#[tauri::command]
fn touch(state: State<AppState>) {
    state.touch();
}

// ---------- 閲覧 ----------

#[tauri::command]
fn item_types() -> Vec<TypeInfo> {
    ops::item_types()
}

#[tauri::command]
fn list_items(state: State<AppState>) -> CmdResult<Vec<Summary>> {
    state.with_store(ops::list)
}

#[tauri::command]
fn view_item(state: State<AppState>, key: String) -> CmdResult<ItemView> {
    state.with_store(|s| ops::view(s, &key))
}

#[tauri::command]
fn reveal_field(state: State<AppState>, key: String, field: String) -> CmdResult<String> {
    state.with_store(|s| ops::reveal_field(s, &key, &field))
}

#[tauri::command]
fn totp_code(state: State<AppState>, key: String, field: String) -> CmdResult<TotpCode> {
    state.with_store_quiet(|s| ops::totp_code(s, &key, &field))
}

/// 秘密をコピーし、設定秒数後にまだ同じ内容なら消す。消去までの秒数を返す（0 は消さない）。
fn copy_concealed_with_timer(state: &AppState, text: &str) -> u32 {
    let clear_after = state.settings().clipboard_clear_secs;
    #[cfg(target_os = "macos")]
    {
        let change_count = macos::copy_concealed(text);
        if clear_after > 0 {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(u64::from(clear_after)));
                macos::clear_if_unchanged(change_count);
            });
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = text;
    clear_after
}

#[tauri::command]
fn copy_field(state: State<AppState>, key: String, field: String) -> CmdResult<u32> {
    let text = state.with_store(|s| ops::copy_text(s, &key, &field))?;
    Ok(copy_concealed_with_timer(&state, &text))
}

/// 生成したパスワードなど、画面にある秘密をコピーする。
#[tauri::command]
fn copy_secret(state: State<AppState>, text: String) -> u32 {
    state.touch();
    copy_concealed_with_timer(&state, &text)
}

/// 秘密参照のような、秘密ではない文字列をコピーする。
#[tauri::command]
fn copy_plain(state: State<AppState>, text: String) {
    #[cfg(target_os = "macos")]
    macos::copy_plain(&text);
    #[cfg(not(target_os = "macos"))]
    let _ = text;
    state.touch();
}

#[tauri::command]
fn open_url(state: State<AppState>, url: String) -> CmdResult<()> {
    if !ops::is_openable_url(&url) {
        return Err("http(s) の URL だけ開けます".into());
    }
    state.touch();
    std::process::Command::new("open")
        .arg(url.trim())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("ブラウザを開けません: {e}"))
}

// ---------- 編集 ----------

#[tauri::command]
fn template_item(item_type: ItemType) -> EditableItem {
    ops::template(item_type)
}

#[tauri::command]
fn editable_item(state: State<AppState>, key: String) -> CmdResult<EditableItem> {
    state.with_store(|s| ops::editable(s, &key))
}

#[derive(Serialize)]
struct PickedFile {
    token: String,
    filename: String,
    size: usize,
}

/// ファイルを選ばせ、中身は Rust 側に預けて受け取り番号だけを返す。キャンセルなら None。
#[tauri::command]
async fn pick_file(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<PickedFile>> {
    let Some(picked) = app.dialog().file().blocking_pick_file() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{} MB を超えるファイルは保存できません", MAX_FILE_BYTES / 1024 / 1024));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let token = random_token();
    let size = bytes.len();
    state
        .pending_files
        .lock()
        .map_err(poisoned)?
        .insert(token.clone(), (filename.clone(), bytes));
    state.touch();
    Ok(Some(PickedFile { token, filename, size }))
}

#[tauri::command]
fn save_item(
    state: State<AppState>,
    original_key: Option<String>,
    key: String,
    item: EditableItem,
) -> CmdResult<()> {
    let mut pending = state.pending_files.lock().map_err(poisoned)?;
    state.with_store(|s| ops::save(s, original_key.as_deref(), &key, &item, &mut pending))
}

#[tauri::command]
fn set_favorite(state: State<AppState>, key: String, favorite: bool) -> CmdResult<()> {
    state.with_store(|s| ops::set_favorite(s, &key, favorite))
}

#[tauri::command]
fn duplicate_item(state: State<AppState>, key: String) -> CmdResult<String> {
    state.with_store(|s| ops::duplicate(s, &key))
}

#[tauri::command]
fn delete_item(state: State<AppState>, key: String) -> CmdResult<()> {
    state.with_store(|s| ops::delete(s, &key))
}

#[derive(Serialize)]
struct Generated {
    password: String,
    strength: Strength,
    strength_label: &'static str,
}

#[tauri::command]
fn generate_password(state: State<AppState>, options: GeneratorOptions) -> CmdResult<Generated> {
    state.touch();
    let password = generator::generate(&options)?;
    let strength = generator::strength(&password);
    Ok(Generated { password, strength, strength_label: strength.label() })
}

#[derive(Serialize)]
struct StrengthInfo {
    strength: Strength,
    label: &'static str,
}

#[tauri::command]
fn password_strength(password: String) -> StrengthInfo {
    let strength = generator::strength(&password);
    StrengthInfo { strength, label: strength.label() }
}

// ---------- ゴミ箱 ----------

#[tauri::command]
fn list_trash(state: State<AppState>) -> CmdResult<Vec<TrashView>> {
    state.with_store(ops::trash)
}

#[tauri::command]
fn restore_item(state: State<AppState>, id: i64) -> CmdResult<String> {
    state.with_store(|s| s.restore(id))
}

#[tauri::command]
fn purge_trash_item(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.with_store(|s| s.purge_trash_item(id))
}

#[tauri::command]
fn empty_trash(state: State<AppState>) -> CmdResult<()> {
    state.with_store(|s| s.empty_trash())
}

// ---------- ファイル・バックアップ ----------

fn write_private(path: &std::path::Path, bytes: &[u8]) -> CmdResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("{} に書けません: {e}", path.display()))?;
    file.write_all(bytes).map_err(|e| format!("{} に書けません: {e}", path.display()))
}

/// ファイル欄の中身を保存先を選ばせて書き出す（権限 600）。キャンセルなら None。
#[tauri::command]
async fn save_field_file(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
    field: String,
) -> CmdResult<Option<String>> {
    let (filename, bytes) = state.with_store(|s| ops::file_bytes(s, &key, &field))?;
    let Some(target) = app.dialog().file().set_file_name(&filename).blocking_save_file() else {
        return Ok(None);
    };
    let path = target.into_path().map_err(|e| e.to_string())?;
    write_private(&path, &bytes)?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
async fn export_vault(app: AppHandle, state: State<'_, AppState>, passphrase: String) -> CmdResult<Option<String>> {
    let sealed = state.with_store(|s| ops::export_sealed(s, &passphrase))?;
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
    write_private(&path, &sealed)?;
    Ok(Some(path.display().to_string()))
}

#[derive(Serialize)]
struct ImportResult {
    imported: usize,
    skipped: usize,
}

#[tauri::command]
async fn import_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
    overwrite: bool,
) -> CmdResult<Option<ImportResult>> {
    let Some(picked) = app.dialog().file().add_filter("vlt backup", &["vltx"]).blocking_pick_file() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let data = std::fs::read(&path).map_err(|e| format!("{} を読めません: {e}", path.display()))?;
    let (imported, skipped) = state.with_store(|s| ops::import_sealed(s, &data, &passphrase, overwrite))?;
    Ok(Some(ImportResult { imported, skipped }))
}

// ---------- 設定 ----------

fn apply_shortcut(app: &AppHandle, enabled: bool) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister(QUICK_OPEN_SHORTCUT);
    if enabled {
        if let Err(e) = shortcuts.register(QUICK_OPEN_SHORTCUT) {
            eprintln!("vlt: global shortcut {QUICK_OPEN_SHORTCUT} を登録できません: {e}");
        }
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<Settings> {
    let settings = settings.sanitized();
    settings.save(&state.settings_path)?;
    apply_shortcut(&app, settings.global_shortcut);
    *state.settings.lock().map_err(poisoned)? = settings.clone();
    state.touch();
    Ok(settings)
}

// ---------- 起動 ----------

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 自動ロックの見張り。操作が無いまま設定時間が経ったらロックする。
fn spawn_idle_watcher(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(5));
        let state = app.state::<AppState>();
        let idle = state.last_activity.lock().map(|t| t.elapsed().as_secs()).unwrap_or(0);
        if settings::should_auto_lock(idle, state.settings().auto_lock_minutes) {
            state.lock(&app, "idle");
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        show_main_window(app);
                        let _ = app.emit("quick-open", ());
                    }
                })
                .build(),
        )
        .setup(|app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = Settings::load(&settings_path);
            apply_shortcut(app.handle(), settings.global_shortcut);
            app.manage(AppState {
                vault: Mutex::new(None),
                last_activity: Mutex::new(Instant::now()),
                settings: Mutex::new(settings),
                settings_path,
                pending_files: Mutex::new(PendingFiles::new()),
            });
            spawn_idle_watcher(app.handle().clone());
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                macos::observe_lock_events(move |reason| {
                    let state = handle.state::<AppState>();
                    if state.settings().lock_on_screen_lock {
                        state.lock(&handle, reason);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 赤いボタンはウインドウを隠すだけにして常駐する（⌘⇧Space で呼び戻せるように）。
            // 終了は ⌘Q。
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            unlock,
            lock,
            init_vault,
            touch,
            item_types,
            list_items,
            view_item,
            reveal_field,
            totp_code,
            copy_field,
            copy_secret,
            copy_plain,
            open_url,
            template_item,
            editable_item,
            pick_file,
            save_item,
            set_favorite,
            duplicate_item,
            delete_item,
            generate_password,
            password_strength,
            list_trash,
            restore_item,
            purge_trash_item,
            empty_trash,
            save_field_file,
            export_vault,
            import_vault,
            save_settings,
        ])
        .build(tauri::generate_context!())
        .expect("error while building vlt");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
