//! TayoReader のコアライブラリ。購読・取得・保存を扱う。

pub mod error;
pub mod model;
pub mod store;

pub use error::{Error, Result};

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
}
