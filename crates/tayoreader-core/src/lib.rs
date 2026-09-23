//! TayoReader のコアライブラリ。購読・取得・保存を扱う。

pub mod error;
pub mod fetch;
pub mod model;
pub mod opml;
pub mod sanitize;
pub mod service;
pub mod store;

pub use error::{Error, Result};

use std::path::PathBuf;

/// 既定の DB パス（macOS では `~/Library/Application Support/tayoreader/tayoreader.db`）。
/// ホームディレクトリが分からない環境では None。
pub fn default_db_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "tayoreader")
        .map(|dirs| dirs.data_dir().join("tayoreader.db"))
}

/// 現在時刻を UTC の Unix 秒で返す。
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn now_is_after_2020() {
        assert!(super::now() > 1_577_836_800);
    }

    #[test]
    fn default_db_path_ends_with_db_file() {
        let path = super::default_db_path().unwrap();
        assert!(path.ends_with("tayoreader/tayoreader.db"), "{path:?}");
    }
}
