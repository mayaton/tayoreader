//! フロントエンドへ返す型と、core のデータからそれを組み立てる関数。

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use tayoreader_core::model::{Entry, EntryFilter, Feed};
use tayoreader_core::sanitize::{article_html, plain_snippet};
use tayoreader_core::service::{AddedFeed, ImportReport, RefreshOutcome, RefreshReport};
use tayoreader_core::store::Store;
use tayoreader_core::{Error, Result};

/// 記事一覧の snippet の最大文字数。
const SNIPPET_CHARS: usize = 200;
const NO_TITLE: &str = "(no title)";

/// サイドバーで選んでいる項目。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Selection {
    All,
    Starred,
    Folder { name: String },
    Feed { id: i64 },
}

impl Selection {
    /// 一覧の絞り込み条件にする。スターは既読の記事も含めて出すため `unread_only` を無視する。
    pub fn filter(&self, unread_only: bool) -> EntryFilter {
        match self {
            Selection::All => EntryFilter {
                unread_only,
                ..Default::default()
            },
            Selection::Starred => EntryFilter {
                starred_only: true,
                ..Default::default()
            },
            Selection::Folder { name } => EntryFilter {
                unread_only,
                folder: Some(name.clone()),
                ..Default::default()
            },
            Selection::Feed { id } => EntryFilter {
                unread_only,
                feed_id: Some(*id),
                ..Default::default()
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: i64,
    pub title: String,
    pub unread: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderItem {
    pub name: String,
    pub unread: u32,
    pub feeds: Vec<FeedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SidebarView {
    pub total_unread: u32,
    pub starred: u32,
    pub folders: Vec<FolderItem>,
    pub unfiled: Vec<FeedItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: i64,
    pub feed_id: i64,
    pub feed_title: String,
    pub title: String,
    pub snippet: String,
    pub published_at: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub feed_title: String,
    pub published_at: i64,
    pub html: String,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedFeed {
    pub url: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSummary {
    pub new_entries: usize,
    pub failed: Vec<FailedFeed>,
}

impl From<&RefreshReport> for RefreshSummary {
    fn from(report: &RefreshReport) -> Self {
        let failed = report
            .failures()
            .filter_map(|f| match &f.outcome {
                RefreshOutcome::Failed(message) => Some(FailedFeed {
                    url: f.url.clone(),
                    message: message.clone(),
                }),
                _ => None,
            })
            .collect();
        Self {
            new_entries: report.new_entries(),
            failed,
        }
    }
}

/// `refresh-progress` イベントの中身。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RefreshProgress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedFeedView {
    pub feed_id: i64,
    pub title: String,
    pub new_entries: usize,
}

impl AddedFeedView {
    pub fn new(added: AddedFeed, url: &str) -> Self {
        Self {
            feed_id: added.feed_id,
            title: non_blank(added.title.as_deref()).unwrap_or(url).to_owned(),
            new_entries: added.new_entries,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImportSummary {
    pub added: usize,
    pub merged: usize,
}

impl From<ImportReport> for ImportSummary {
    fn from(r: ImportReport) -> Self {
        Self {
            added: r.added,
            merged: r.merged,
        }
    }
}

/// コマンドのエラー。`kind` は core の `Error` の variant 名か `Busy`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandError {
    pub kind: String,
    pub message: String,
}

impl CommandError {
    /// 更新中に更新を求められたとき。
    pub fn busy() -> Self {
        Self {
            kind: "Busy".into(),
            message: "refresh is already running".into(),
        }
    }
}

impl From<Error> for CommandError {
    fn from(e: Error) -> Self {
        let kind = match &e {
            Error::Http(_) => "Http",
            Error::Timeout => "Timeout",
            Error::Parse(_) => "Parse",
            Error::Db(_) => "Db",
            Error::Migration(_) => "Migration",
            Error::Opml(_) => "Opml",
            Error::NotFound(_) => "NotFound",
            Error::AlreadySubscribed(_) => "AlreadySubscribed",
            Error::InvalidInput(_) => "InvalidInput",
            Error::Io(_) => "Io",
        };
        Self {
            kind: kind.into(),
            message: e.to_string(),
        }
    }
}

/// サイドバーを組み立てる。
pub fn sidebar(store: &Store) -> Result<SidebarView> {
    let starred = store.count_entries(&Selection::Starred.filter(false))?;
    Ok(build_sidebar(
        &store.list_feeds()?,
        &store.unread_counts()?,
        starred,
    ))
}

/// フォルダとフィードを名前順（大文字小文字を無視）に並べる。
/// 複数フォルダに属するフィードは各フォルダに出し、`total_unread` では 1 回だけ数える。
pub fn build_sidebar(feeds: &[Feed], unread: &HashMap<i64, u32>, starred: u32) -> SidebarView {
    let mut feeds: Vec<(&Feed, FeedItem)> = feeds
        .iter()
        .map(|feed| {
            let item = FeedItem {
                id: feed.id,
                title: feed_title(feed).to_owned(),
                unread: unread.get(&feed.id).copied().unwrap_or(0),
                error: feed.last_error.clone(),
            };
            (feed, item)
        })
        .collect();
    feeds.sort_by_cached_key(|(feed, item)| (item.title.to_lowercase(), feed.id));

    let mut folders: Vec<FolderItem> = folder_names(feeds.iter().map(|(feed, _)| *feed))
        .into_iter()
        .map(|name| {
            let members: Vec<FeedItem> = feeds
                .iter()
                .filter(|(feed, _)| feed.folders.contains(&name))
                .map(|(_, item)| item.clone())
                .collect();
            FolderItem {
                unread: members.iter().map(|f| f.unread).sum(),
                name,
                feeds: members,
            }
        })
        .collect();
    folders.sort_by_cached_key(|f| (f.name.to_lowercase(), f.name.clone()));

    SidebarView {
        total_unread: feeds.iter().map(|(_, item)| item.unread).sum(),
        starred,
        folders,
        unfiled: feeds
            .iter()
            .filter(|(feed, _)| feed.folders.is_empty())
            .map(|(_, item)| item.clone())
            .collect(),
    }
}

/// 購読で使われているフォルダ名を名前順（大文字小文字を無視）で返す。
pub fn list_folders(store: &Store) -> Result<Vec<String>> {
    let mut names: Vec<String> = folder_names(store.list_feeds()?.iter())
        .into_iter()
        .collect();
    names.sort_by_cached_key(|n| (n.to_lowercase(), n.clone()));
    Ok(names)
}

fn folder_names<'a>(feeds: impl Iterator<Item = &'a Feed>) -> BTreeSet<String> {
    feeds.flat_map(|f| f.folders.iter().cloned()).collect()
}

/// 記事一覧を返す。
pub fn list_entries(
    store: &Store,
    selection: &Selection,
    unread_only: bool,
    offset: u32,
    limit: u32,
) -> Result<Vec<EntrySummary>> {
    let filter = EntryFilter {
        offset: Some(offset),
        limit: Some(limit),
        ..selection.filter(unread_only)
    };
    Ok(store.list_entries(&filter)?.iter().map(summarize).collect())
}

pub fn summarize(entry: &Entry) -> EntrySummary {
    let body = entry.summary.as_deref().or(entry.content.as_deref());
    EntrySummary {
        id: entry.id,
        feed_id: entry.feed_id,
        feed_title: entry.feed_title.clone().unwrap_or_default(),
        title: entry_title(entry).to_owned(),
        snippet: body
            .map(|html| plain_snippet(html, SNIPPET_CHARS))
            .unwrap_or_default(),
        published_at: entry.published_at.unwrap_or(entry.fetched_at),
        is_read: entry.is_read,
        is_starred: entry.is_starred,
    }
}

/// 本文を返す。HTML は必ずサニタイズする。
pub fn get_article(store: &Store, id: i64) -> Result<Article> {
    let entry = store.get_entry(id)?;
    let feed = store.get_feed(entry.feed_id)?;
    Ok(article(&entry, &feed))
}

pub fn article(entry: &Entry, feed: &Feed) -> Article {
    let body = entry
        .content
        .as_deref()
        .or(entry.summary.as_deref())
        .unwrap_or_default();
    let base = entry.url.as_deref().or(feed.site_url.as_deref());
    Article {
        id: entry.id,
        title: entry_title(entry).to_owned(),
        url: entry.url.clone(),
        author: entry.author.clone(),
        feed_title: feed_title(feed).to_owned(),
        published_at: entry.published_at.unwrap_or(entry.fetched_at),
        html: article_html(body, base),
        is_read: entry.is_read,
        is_starred: entry.is_starred,
    }
}

fn feed_title(feed: &Feed) -> &str {
    non_blank(feed.title.as_deref()).unwrap_or(&feed.url)
}

fn entry_title(entry: &Entry) -> &str {
    non_blank(entry.title.as_deref()).unwrap_or(NO_TITLE)
}

fn non_blank(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tayoreader_core::model::{FeedMeta, NewEntry};

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn feed(s: &Store, url: &str, title: Option<&str>, folders: &[&str]) -> i64 {
        let id = s.insert_feed(url, title, 1).unwrap();
        for folder in folders {
            s.add_feed_to_folder(id, folder).unwrap();
        }
        id
    }

    fn save(s: &Store, feed_id: i64, entries: &[NewEntry]) {
        s.save_fetched(feed_id, &FeedMeta::default(), entries, 100)
            .unwrap();
    }

    fn entry(guid: &str, published_at: Option<i64>) -> NewEntry {
        NewEntry {
            guid: guid.into(),
            title: Some(format!("title {guid}")),
            published_at,
            ..Default::default()
        }
    }

    // ---- Selection ----

    #[test]
    fn selection_deserializes_from_tagged_json() {
        let parse = |json: &str| serde_json::from_str::<Selection>(json).unwrap();
        assert_eq!(parse(r#"{"kind":"all"}"#), Selection::All);
        assert_eq!(parse(r#"{"kind":"starred"}"#), Selection::Starred);
        assert_eq!(
            parse(r#"{"kind":"folder","name":"Tech"}"#),
            Selection::Folder {
                name: "Tech".into()
            }
        );
        assert_eq!(
            parse(r#"{"kind":"feed","id":3}"#),
            Selection::Feed { id: 3 }
        );
    }

    #[test]
    fn selection_to_filter() {
        assert_eq!(
            Selection::All.filter(true),
            EntryFilter {
                unread_only: true,
                ..Default::default()
            }
        );
        assert_eq!(
            Selection::Starred.filter(true),
            EntryFilter {
                starred_only: true,
                ..Default::default()
            }
        );
        assert_eq!(
            Selection::Folder {
                name: "Tech".into()
            }
            .filter(false),
            EntryFilter {
                folder: Some("Tech".into()),
                ..Default::default()
            }
        );
        assert_eq!(
            Selection::Feed { id: 7 }.filter(true),
            EntryFilter {
                unread_only: true,
                feed_id: Some(7),
                ..Default::default()
            }
        );
    }

    // ---- sidebar ----

    #[test]
    fn sidebar_sorts_folders_and_feeds_ignoring_case() {
        let s = store();
        let b = feed(&s, "https://b.example/", Some("beta"), &["tech"]);
        let a = feed(&s, "https://a.example/", Some("Alpha"), &["tech", "News"]);
        let loose = feed(&s, "https://z.example/", None, &[]);
        save(&s, a, &[entry("a1", Some(1)), entry("a2", Some(2))]);
        save(&s, b, &[entry("b1", Some(3))]);
        save(&s, loose, &[entry("z1", Some(4))]);

        let view = sidebar(&s).unwrap();
        let folders: Vec<(&str, u32, Vec<&str>)> = view
            .folders
            .iter()
            .map(|f| {
                (
                    f.name.as_str(),
                    f.unread,
                    f.feeds.iter().map(|x| x.title.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            folders,
            vec![
                ("News", 2, vec!["Alpha"]),
                ("tech", 3, vec!["Alpha", "beta"]),
            ]
        );
        assert_eq!(
            view.unfiled,
            vec![FeedItem {
                id: loose,
                title: "https://z.example/".into(),
                unread: 1,
                error: None,
            }]
        );
        // Alpha は 2 フォルダに出るが合計では 1 回だけ数える
        assert_eq!(view.total_unread, 4);
        assert_eq!(view.starred, 0);
    }

    #[test]
    fn sidebar_counts_starred_and_shows_errors_and_blank_titles() {
        let s = store();
        let a = feed(&s, "https://a.example/", Some("  "), &[]);
        save(&s, a, &[entry("a1", Some(1)), entry("a2", Some(2))]);
        let ids: Vec<i64> = s
            .list_entries(&EntryFilter::default())
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();
        s.set_starred(&ids, true).unwrap();
        s.set_read(&ids, true).unwrap();
        s.mark_fetch_error(a, "HTTP error: 500").unwrap();

        let view = sidebar(&s).unwrap();
        assert_eq!(view.starred, 2);
        assert_eq!(view.total_unread, 0);
        assert_eq!(view.unfiled[0].title, "https://a.example/");
        assert_eq!(view.unfiled[0].error.as_deref(), Some("HTTP error: 500"));
    }

    #[test]
    fn sidebar_of_empty_store_has_only_fixed_items() {
        let view = sidebar(&store()).unwrap();
        assert_eq!(
            view,
            SidebarView {
                total_unread: 0,
                starred: 0,
                folders: vec![],
                unfiled: vec![],
            }
        );
    }

    #[test]
    fn folders_differing_only_in_case_are_separate() {
        let s = store();
        feed(&s, "https://a.example/", Some("A"), &["tech"]);
        feed(&s, "https://b.example/", Some("B"), &["Tech"]);
        let names: Vec<String> = sidebar(&s)
            .unwrap()
            .folders
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["Tech", "tech"]);
    }

    #[test]
    fn list_folders_returns_used_folder_names() {
        let s = store();
        feed(&s, "https://a.example/", None, &["tech", "News"]);
        feed(&s, "https://b.example/", None, &["tech"]);
        assert_eq!(list_folders(&s).unwrap(), vec!["News", "tech"]);
    }

    // ---- entries / article ----

    #[test]
    fn list_entries_pages_and_summarizes() {
        let s = store();
        let a = feed(&s, "https://a.example/", Some("A"), &[]);
        let mut first = entry("a1", Some(50));
        first.summary = Some("<p>Hello &amp; <b>world</b></p>".into());
        let untitled = NewEntry {
            guid: "a2".into(),
            ..Default::default()
        };
        save(&s, a, &[first, untitled]);

        let page = list_entries(&s, &Selection::All, true, 0, 1).unwrap();
        assert_eq!(page.len(), 1);
        // 公開日時の無い記事は取得日時（100）で並ぶので先頭に来る
        let e = &page[0];
        assert_eq!(e.title, "(no title)");
        assert_eq!(e.published_at, 100);
        assert_eq!(e.feed_title, "A");
        assert_eq!(e.snippet, "");

        let next = list_entries(&s, &Selection::All, true, 1, 1).unwrap();
        assert_eq!(next[0].title, "title a1");
        assert_eq!(next[0].snippet, "Hello & world");
        assert_eq!(next[0].published_at, 50);
    }

    #[test]
    fn get_article_sanitizes_and_resolves_against_entry_or_site_url() {
        let s = store();
        let a = feed(&s, "https://a.example/feed", Some("A"), &[]);
        s.save_fetched(
            a,
            &FeedMeta {
                site_url: Some("https://a.example/".into()),
                ..Default::default()
            },
            &[
                NewEntry {
                    guid: "with-url".into(),
                    url: Some("https://a.example/posts/1".into()),
                    author: Some("Alice".into()),
                    content: Some(r#"<p onclick="x()">Hi</p><img src="img.png">"#.into()),
                    summary: Some("summary".into()),
                    published_at: Some(2),
                    ..Default::default()
                },
                NewEntry {
                    guid: "without-url".into(),
                    summary: Some(r#"<a href="/about">about</a>"#.into()),
                    published_at: Some(1),
                    ..Default::default()
                },
            ],
            100,
        )
        .unwrap();
        let ids: Vec<i64> = s
            .list_entries(&EntryFilter::default())
            .unwrap()
            .iter()
            .map(|e| e.id)
            .collect();

        let first = get_article(&s, ids[0]).unwrap();
        assert_eq!(first.title, "(no title)");
        assert_eq!(first.url.as_deref(), Some("https://a.example/posts/1"));
        assert_eq!(first.author.as_deref(), Some("Alice"));
        assert_eq!(first.feed_title, "A");
        assert_eq!(first.published_at, 2);
        assert_eq!(
            first.html,
            r#"<p>Hi</p><img src="https://a.example/posts/img.png">"#
        );

        let second = get_article(&s, ids[1]).unwrap();
        assert!(
            second.html.contains(r#"href="https://a.example/about""#),
            "{}",
            second.html
        );

        let err = get_article(&s, 9999).unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    }

    #[test]
    fn article_without_body_is_empty() {
        let s = store();
        let a = feed(&s, "https://a.example/", Some("A"), &[]);
        save(&s, a, &[entry("empty", Some(1))]);
        let id = s.list_entries(&EntryFilter::default()).unwrap()[0].id;
        assert_eq!(get_article(&s, id).unwrap().html, "");
        let summary = &list_entries(&s, &Selection::All, false, 0, 10).unwrap()[0];
        assert_eq!(summary.snippet, "");
    }

    // ---- 結果とエラーの変換 ----

    #[test]
    fn refresh_summary_lists_failures() {
        use tayoreader_core::service::FeedRefresh;
        let report = RefreshReport {
            feeds: vec![
                FeedRefresh {
                    feed_id: 1,
                    url: "https://ok.example/".into(),
                    outcome: RefreshOutcome::Updated { new_entries: 3 },
                },
                FeedRefresh {
                    feed_id: 2,
                    url: "https://ng.example/".into(),
                    outcome: RefreshOutcome::Failed("request timed out".into()),
                },
            ],
        };
        assert_eq!(
            RefreshSummary::from(&report),
            RefreshSummary {
                new_entries: 3,
                failed: vec![FailedFeed {
                    url: "https://ng.example/".into(),
                    message: "request timed out".into(),
                }],
            }
        );
    }

    #[test]
    fn command_error_uses_variant_name_as_kind() {
        let e = CommandError::from(Error::NotFound("entry 1".into()));
        assert_eq!(e.kind, "NotFound");
        assert_eq!(e.message, "entry 1 not found");
        assert_eq!(CommandError::from(Error::Timeout).kind, "Timeout");
        assert_eq!(
            CommandError::from(Error::InvalidInput("x".into())).kind,
            "InvalidInput"
        );
        assert_eq!(CommandError::busy().kind, "Busy");
        let json = serde_json::to_string(&CommandError::busy()).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"Busy","message":"refresh is already running"}"#
        );
    }

    #[test]
    fn views_serialize_in_camel_case() {
        let summary = EntrySummary {
            id: 1,
            feed_id: 2,
            feed_title: "A".into(),
            title: "t".into(),
            snippet: "s".into(),
            published_at: 3,
            is_read: false,
            is_starred: true,
        };
        assert_eq!(
            serde_json::to_string(&summary).unwrap(),
            r#"{"id":1,"feedId":2,"feedTitle":"A","title":"t","snippet":"s","publishedAt":3,"isRead":false,"isStarred":true}"#
        );
    }

    #[test]
    fn added_feed_falls_back_to_url() {
        let view = AddedFeedView::new(
            AddedFeed {
                feed_id: 1,
                title: None,
                new_entries: 2,
            },
            "https://a.example/feed",
        );
        assert_eq!(view.title, "https://a.example/feed");
    }
}
