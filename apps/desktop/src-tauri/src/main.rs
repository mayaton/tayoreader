// release ビルドで Windows のコンソールを出さない（macOS では影響しない）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;
mod view;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::state::AppState;

/// メニュー「OPML を読み込む…」の id。フロントへ同名のイベントを送る。
const MENU_IMPORT_OPML: &str = "menu-import-opml";

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .menu(build_menu)
        .on_menu_event(|app, event| {
            if event.id() == MENU_IMPORT_OPML {
                let _ = app.emit(MENU_IMPORT_OPML, ());
            }
        })
        .setup(|app| {
            match open_state() {
                Ok(state) => {
                    app.manage(state);
                    open_main_window(app.handle())?;
                }
                Err(message) => {
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(message)
                        .title("TayoReader を起動できません")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sidebar,
            commands::list_entries,
            commands::get_article,
            commands::set_read,
            commands::set_starred,
            commands::mark_all_read,
            commands::refresh,
            commands::add_feed,
            commands::remove_feed,
            commands::import_opml,
            commands::list_folders,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TayoReader");
}

fn open_state() -> Result<AppState, String> {
    let path = state::db_path().ok_or("アプリのデータディレクトリを決められませんでした。")?;
    AppState::open(&path)
        .map_err(|e| format!("データベースを開けませんでした。\n{}\n{e}", path.display()))
}

fn open_main_window(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("TayoReader")
        .inner_size(1200.0, 760.0)
        .min_inner_size(900.0, 560.0)
        // 本文のリンクで WebView ごと外部サイトへ遷移させない（リンクは opener で開く）
        .on_navigation(is_app_url)
        .build()?;
    Ok(())
}

/// アプリ自身の画面の URL か（本番は tauri://localhost、開発時は http://localhost:1420）。
/// 開発時の localhost 許可はデバッグビルドに限定し、リリースビルドでは tauri:// のみ許可する。
fn is_app_url(url: &Url) -> bool {
    if url.scheme() == "tauri" {
        return true;
    }
    cfg!(debug_assertions)
        && url.scheme() == "http"
        && url.host_str() == Some("localhost")
        && url.port() == Some(1420)
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let import = MenuItem::with_id(
        app,
        MENU_IMPORT_OPML,
        "OPML を読み込む…",
        true,
        Some("CmdOrCtrl+Shift+O"),
    )?;
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                "TayoReader",
                true,
                &[
                    &PredefinedMenuItem::about(app, None, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::quit(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "ファイル",
                true,
                &[
                    &import,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?,
            // テキスト入力でのコピー・ペーストに必要
            &Submenu::with_items(
                app,
                "編集",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "ウインドウ",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                ],
            )?,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // テストは常にデバッグビルド（cfg(debug_assertions) が真）として実行される。
    #[test]
    fn only_app_urls_are_navigable() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(is_app_url(&url("tauri://localhost/")));
        assert!(is_app_url(&url("http://localhost:1420/")));
        assert!(!is_app_url(&url("https://example.com/")));
        assert!(!is_app_url(&url("http://localhost.example.com/")));
        // devUrl のポート以外は許可しない
        assert!(!is_app_url(&url("http://localhost:8080/")));
        assert!(!is_app_url(&url("http://localhost/")));
        // スキームが http 以外（https）の localhost も許可しない
        assert!(!is_app_url(&url("https://localhost:1420/")));
    }
}
