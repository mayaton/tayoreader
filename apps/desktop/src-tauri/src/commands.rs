//! フロントエンドから呼ぶ Tauri コマンド。view と Service を呼ぶだけにする。

use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;
use crate::view::{
    self, AddedFeedView, Article, CommandError, EntrySummary, ImportSummary, RefreshProgress,
    RefreshSummary, Selection, SidebarView,
};

type CmdResult<T> = Result<T, CommandError>;

/// 更新の進捗を送るイベント名。
pub const REFRESH_PROGRESS: &str = "refresh-progress";

#[tauri::command]
pub async fn sidebar(state: State<'_, AppState>) -> CmdResult<SidebarView> {
    Ok(view::sidebar(state.service.store())?)
}

#[tauri::command]
pub async fn list_entries(
    state: State<'_, AppState>,
    selection: Selection,
    unread_only: bool,
    offset: u32,
    limit: u32,
) -> CmdResult<Vec<EntrySummary>> {
    Ok(view::list_entries(
        state.service.store(),
        &selection,
        unread_only,
        offset,
        limit,
    )?)
}

#[tauri::command]
pub async fn get_article(state: State<'_, AppState>, id: i64) -> CmdResult<Article> {
    Ok(view::get_article(state.service.store(), id)?)
}

#[tauri::command]
pub async fn set_read(state: State<'_, AppState>, ids: Vec<i64>, read: bool) -> CmdResult<usize> {
    Ok(state.service.store().set_read(&ids, read)?)
}

#[tauri::command]
pub async fn set_starred(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    starred: bool,
) -> CmdResult<usize> {
    Ok(state.service.store().set_starred(&ids, starred)?)
}

#[tauri::command]
pub async fn mark_all_read(state: State<'_, AppState>, selection: Selection) -> CmdResult<usize> {
    Ok(state
        .service
        .store()
        .mark_all_read(&selection.filter(false))?)
}

#[tauri::command]
pub async fn refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    feed_id: Option<i64>,
) -> CmdResult<RefreshSummary> {
    let Some(_guard) = state.try_begin_refresh() else {
        return Err(CommandError::busy());
    };
    let report = state
        .service
        .refresh_with_progress(feed_id, |done, total| {
            // 進捗表示が欠けるだけなので送信失敗は無視する
            let _ = app.emit(REFRESH_PROGRESS, RefreshProgress { done, total });
        })
        .await?;
    Ok(RefreshSummary::from(&report))
}

#[tauri::command]
pub async fn add_feed(
    state: State<'_, AppState>,
    url: String,
    folders: Vec<String>,
) -> CmdResult<AddedFeedView> {
    let url = url.trim();
    let added = state.service.add_feed(url, &folders).await?;
    Ok(AddedFeedView::new(added, url))
}

#[tauri::command]
pub async fn remove_feed(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    Ok(state.service.store().remove_feed(id)?)
}

#[tauri::command]
pub async fn import_opml(state: State<'_, AppState>, path: String) -> CmdResult<ImportSummary> {
    let xml = std::fs::read_to_string(&path).map_err(tayoreader_core::Error::from)?;
    Ok(state.service.import_opml(&xml).await?.into())
}

#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    Ok(view::list_folders(state.service.store())?)
}
