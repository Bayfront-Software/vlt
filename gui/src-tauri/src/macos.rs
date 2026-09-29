//! macOS 固有の連携（Touch ID・クリップボード・画面ロック検知）。

use std::ptr::NonNull;
use std::sync::mpsc;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSWorkspace, NSWorkspaceWillSleepNotification};
use objc2_foundation::{NSArray, NSDistributedNotificationCenter, NSError, NSNotification, NSString};
use objc2_local_authentication::{LAContext, LAPolicy};

/// LAError.userCancel / systemCancel / appCancel
const LA_ERROR_CANCEL_CODES: [isize; 3] = [-2, -4, -9];

/// Touch ID（使えない・何度も失敗したときは Mac のログインパスワード）で本人確認する。
/// 呼び出し元のスレッドをブロックするので、メインスレッドからは呼ばないこと。
pub fn authenticate(reason: &str) -> Result<(), String> {
    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthentication) }
        .map_err(|e| format!("この Mac では本人確認を使えません: {}", e.localizedDescription()))?;
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    let reply = RcBlock::new(move |ok: Bool, error: *mut NSError| {
        let result = if ok.as_bool() {
            Ok(())
        } else if let Some(error) = unsafe { error.as_ref() } {
            if LA_ERROR_CANCEL_CODES.contains(&error.code()) {
                Err("解錠をキャンセルしました".to_string())
            } else {
                Err(error.localizedDescription().to_string())
            }
        } else {
            Err("本人確認に失敗しました".to_string())
        };
        let _ = tx.send(result);
    });
    let reason = NSString::from_str(reason);
    unsafe {
        context.evaluatePolicy_localizedReason_reply(LAPolicy::DeviceOwnerAuthentication, &reason, &reply)
    };
    rx.recv().map_err(|_| "本人確認の応答がありません".to_string())?
}

/// Touch ID が登録済みで使えるか（画面の文言を「Touch ID で解錠」にするか決める）。
pub fn biometrics_available() -> bool {
    let context = unsafe { LAContext::new() };
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }.is_ok()
}

/// 値をクリップボードへ書く。`org.nspasteboard.ConcealedType` を付けるので、
/// 対応するクリップボード履歴アプリ（Raycast・Alfred・Paste など）は記録しない。
/// 戻り値は書いた直後の changeCount（後で「まだ同じ内容か」を判定する）。
pub fn copy_concealed(text: &str) -> isize {
    let pasteboard = NSPasteboard::generalPasteboard();
    let concealed = NSString::from_str("org.nspasteboard.ConcealedType");
    let string_type = unsafe { NSPasteboardTypeString };
    let types = NSArray::from_slice(&[string_type, &*concealed]);
    pasteboard.clearContents();
    unsafe { pasteboard.declareTypes_owner(&types, None) };
    pasteboard.setString_forType(&NSString::from_str(text), string_type);
    pasteboard.setString_forType(&NSString::from_str(""), &concealed);
    pasteboard.changeCount()
}

/// 秘密ではない文字列（秘密参照など）を普通にコピーする。
pub fn copy_plain(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}

/// コピー後に誰も書き換えていなければ消す。ユーザーが別の物をコピーした後なら消さない。
pub fn clear_if_unchanged(change_count: isize) -> bool {
    let pasteboard = NSPasteboard::generalPasteboard();
    if pasteboard.changeCount() != change_count {
        return false;
    }
    pasteboard.clearContents();
    true
}

/// 画面ロックとスリープで `on_event` を呼ぶ。メインスレッド（Tauri の setup）から呼ぶこと。
pub fn observe_lock_events(on_event: impl Fn(&'static str) + Clone + 'static) {
    let screen = on_event.clone();
    let screen_block = RcBlock::new(move |_: NonNull<NSNotification>| screen("screen-locked"));
    let sleep_block = RcBlock::new(move |_: NonNull<NSNotification>| on_event("sleep"));
    unsafe {
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        let observer = distributed.addObserverForName_object_queue_usingBlock(
            Some(&NSString::from_str("com.apple.screenIsLocked")),
            None,
            None,
            &screen_block,
        );
        // アプリの寿命と同じだけ購読し続けるので、解除用のトークンは手放す。
        std::mem::forget(observer);
        let workspace_center = NSWorkspace::sharedWorkspace().notificationCenter();
        let observer = workspace_center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceWillSleepNotification),
            None,
            None,
            &sleep_block,
        );
        std::mem::forget(observer);
    }
}
