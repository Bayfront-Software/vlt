//! vlt のコアライブラリ。CLI (`src/main.rs`) と GUI (`gui/src-tauri`) の両方がここを使う。
//! 暗号形式・.vltx 構造・スキーマ移行は CLAUDE.md の保護境界に従うこと。

pub mod crypto;
pub mod env;
pub mod generator;
pub mod item;
pub mod keychain;
pub mod mask;
pub mod portable;
pub mod reference;
pub mod runner;
pub mod store;
pub mod totp;
