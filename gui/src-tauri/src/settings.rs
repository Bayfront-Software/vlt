//! 利用者の設定（アプリの設定ディレクトリの settings.json）。

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const AUTO_LOCK_CHOICES: [u32; 6] = [0, 1, 5, 10, 30, 60];
pub const CLIPBOARD_CHOICES: [u32; 5] = [0, 30, 60, 90, 180];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// 操作が無いまま何分でロックするか。0 はしない。
    pub auto_lock_minutes: u32,
    /// コピーした値を何秒後にクリップボードから消すか。0 は消さない。
    pub clipboard_clear_secs: u32,
    /// 画面ロック・スリープでロックする。
    pub lock_on_screen_lock: bool,
    /// 解錠のたびに Touch ID（またはログインパスワード）を求める。
    pub unlock_with_touch_id: bool,
    /// ⌘⇧Space でどこからでも vlt を呼び出す。
    pub global_shortcut: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_lock_minutes: 10,
            clipboard_clear_secs: 90,
            lock_on_screen_lock: true,
            unlock_with_touch_id: true,
            global_shortcut: true,
        }
    }
}

impl Settings {
    /// 読めない・壊れている・一部だけ、のいずれでも起動を止めず既定値で補う。
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Settings>(&raw).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, raw).map_err(|e| e.to_string())
    }

    /// 選択肢に無い値（手で書き換えた JSON など）は既定値に戻す。
    pub fn sanitized(mut self) -> Self {
        let defaults = Settings::default();
        if !AUTO_LOCK_CHOICES.contains(&self.auto_lock_minutes) {
            self.auto_lock_minutes = defaults.auto_lock_minutes;
        }
        if !CLIPBOARD_CHOICES.contains(&self.clipboard_clear_secs) {
            self.clipboard_clear_secs = defaults.clipboard_clear_secs;
        }
        self
    }
}

/// 最後の操作から `idle_secs` 経ったとき、自動ロックすべきか。
pub fn should_auto_lock(idle_secs: u64, auto_lock_minutes: u32) -> bool {
    auto_lock_minutes > 0 && idle_secs >= u64::from(auto_lock_minutes) * 60
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vlt-settings-{name}-{}.json", std::process::id()))
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        assert_eq!(Settings::load(&temp("missing")), Settings::default());
        let p = temp("corrupt");
        std::fs::write(&p, b"{not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn partial_file_keeps_given_values_and_fills_the_rest() {
        let p = temp("partial");
        std::fs::write(&p, br#"{"auto_lock_minutes": 30}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.auto_lock_minutes, 30);
        assert_eq!(s.clipboard_clear_secs, Settings::default().clipboard_clear_secs);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn out_of_range_values_fall_back_to_defaults() {
        let s = Settings { auto_lock_minutes: 7, clipboard_clear_secs: 5, ..Settings::default() }.sanitized();
        assert_eq!(s.auto_lock_minutes, 10);
        assert_eq!(s.clipboard_clear_secs, 90);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let p = temp("roundtrip");
        let s = Settings { auto_lock_minutes: 1, global_shortcut: false, ..Settings::default() };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn auto_lock_threshold() {
        assert!(!should_auto_lock(599, 10));
        assert!(should_auto_lock(600, 10));
        assert!(!should_auto_lock(100_000, 0), "0 は自動ロックしない");
    }
}
