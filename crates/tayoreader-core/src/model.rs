//! core が扱うデータ型。

/// 購読中のフィード。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    pub id: i64,
    pub url: String,
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub last_fetched_at: Option<i64>,
    pub last_error: Option<String>,
    pub created_at: i64,
    /// 所属フォルダ名（名前順）。
    pub folders: Vec<String>,
}

/// 取得結果から得たフィードのメタ情報。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeedMeta {
    pub title: Option<String>,
    pub site_url: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// 保存前の記事。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewEntry {
    pub guid: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub published_at: Option<i64>,
}

/// 保存済みの記事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub feed_id: i64,
    pub guid: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub published_at: Option<i64>,
    pub fetched_at: i64,
    pub is_read: bool,
    pub is_starred: bool,
    /// フィードのタイトル（無ければフィード URL）。
    pub feed_title: Option<String>,
}

/// 記事一覧の絞り込み条件。`limit` / `offset` は `Store::list_entries` だけが使う。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryFilter {
    pub unread_only: bool,
    pub starred_only: bool,
    pub feed_id: Option<i64>,
    pub folder: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// 検索語。空白（全角空白を含む）で区切った語をすべて含む記事に絞る。空白だけなら絞らない。
    pub query: Option<String>,
}

/// OPML から読み取ったフィード。`folders` は入れ子の内側のフォルダ名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpmlFeed {
    pub url: String,
    pub title: Option<String>,
    pub folders: Vec<String>,
}

/// `Store::import_feeds` の結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 新規に購読したフィード数。
    pub added: usize,
    /// 購読済みでフォルダだけマージしたフィード数。
    pub merged: usize,
}
