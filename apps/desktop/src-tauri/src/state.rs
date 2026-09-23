//! アプリ全体で共有する状態。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tayoreader_core::fetch::Fetcher;
use tayoreader_core::service::Service;
use tayoreader_core::store::Store;

/// DB パスを上書きする環境変数。手動確認で一時 DB を使うためのもの。
pub const DB_ENV: &str = "TAYOREADER_DB";

pub struct AppState {
    pub service: Service,
    refreshing: AtomicBool,
}

impl AppState {
    pub fn open(path: &Path) -> tayoreader_core::Result<Self> {
        Ok(Self::new(Service::new(Store::open(path)?, Fetcher::new()?)))
    }

    pub fn new(service: Service) -> Self {
        Self {
            service,
            refreshing: AtomicBool::new(false),
        }
    }

    /// 更新を始める。既に更新中なら None。戻り値を drop すると更新中を解除する。
    pub fn try_begin_refresh(&self) -> Option<RefreshGuard<'_>> {
        self.refreshing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| RefreshGuard(&self.refreshing))
    }
}

pub struct RefreshGuard<'a>(&'a AtomicBool);

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// DB パスを決める。`TAYOREADER_DB` が空でなければそれを、無ければ CLI と同じ既定パスを使う。
pub fn db_path() -> Option<PathBuf> {
    resolve_db_path(std::env::var_os(DB_ENV))
}

fn resolve_db_path(env: Option<OsString>) -> Option<PathBuf> {
    env.filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(tayoreader_core::default_db_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new(Service::new(
            Store::open_in_memory().unwrap(),
            Fetcher::new().unwrap(),
        ))
    }

    #[test]
    fn refresh_guard_rejects_overlap_until_dropped() {
        let s = state();
        let guard = s.try_begin_refresh();
        assert!(guard.is_some());
        assert!(s.try_begin_refresh().is_none());
        drop(guard);
        assert!(s.try_begin_refresh().is_some());
    }

    #[test]
    fn db_path_prefers_env_and_ignores_empty() {
        assert_eq!(
            resolve_db_path(Some("/tmp/t.db".into())),
            Some(PathBuf::from("/tmp/t.db"))
        );
        assert_eq!(
            resolve_db_path(Some("".into())),
            tayoreader_core::default_db_path()
        );
        assert_eq!(resolve_db_path(None), tayoreader_core::default_db_path());
    }
}
