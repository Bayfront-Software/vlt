//! vlt のコアライブラリ。CLI (`src/main.rs`) と GUI (`gui/src-tauri`) の両方がここを使う。
//! 暗号形式・.vltx 構造・スキーマ移行は CLAUDE.md の保護境界に従うこと。

pub mod crypto;
pub mod keychain;
pub mod portable;
pub mod resolve;
pub mod store;
