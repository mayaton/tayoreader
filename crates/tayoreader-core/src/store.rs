//! SQLite への読み書き。DB アクセスはすべてこのモジュールを通す。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use rusqlite_migration::{M, Migrations};

use crate::model::{Entry, EntryFilter, Feed, FeedMeta, ImportReport, NewEntry, OpmlFeed};
use crate::sanitize::search_text;
use crate::{Error, Result};

const MIGRATIONS: &[M<'static>] = &[
    M::up(include_str!("../migrations/001_init.sql")),
    M::up(include_str!("../migrations/002_search.sql")),
];

/// 検索用テキストの埋め戻しで 1 トランザクションに入れる件数。
const BACKFILL_BATCH: i64 = 500;

const FEED_COLUMNS: &str =
    "id, url, title, site_url, etag, last_modified, last_fetched_at, last_error, created_at";

const ENTRY_COLUMNS: &str = "e.id, e.feed_id, e.guid, e.url, e.title, e.author, e.summary, e.content, e.published_at, e.fetched_at, e.is_read, e.is_starred, COALESCE(NULLIF(TRIM(f.title), ''), f.url)";

const ENTRY_FROM: &str = "entries e JOIN feeds f ON f.id = e.feed_id";

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// ファイル DB を開く。親ディレクトリが無ければ作る。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        Self::init(conn)
    }

    /// テスト用のインメモリ DB を開く。
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Migrations::from_slice(MIGRATIONS).to_latest(&mut conn)?;
        backfill_plain_text(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("store mutex poisoned")
    }

    /// フィードを登録して id を返す。URL が登録済みなら `AlreadySubscribed`。
    pub fn insert_feed(&self, url: &str, title: Option<&str>, now: i64) -> Result<i64> {
        insert_feed_conn(&self.conn(), url, title, now)
    }

    pub fn find_feed_id_by_url(&self, url: &str) -> Result<Option<i64>> {
        find_feed_id_by_url_conn(&self.conn(), url)
    }

    /// フィードをフォルダに入れる。フォルダが無ければ作り、既に入っていれば何もしない。
    /// 名前の前後の空白は取り除く。空になった場合は `Error::InvalidInput`。
    pub fn add_feed_to_folder(&self, feed_id: i64, folder: &str) -> Result<()> {
        add_feed_to_folder_conn(&self.conn(), feed_id, folder)
    }

    pub fn get_feed(&self, id: i64) -> Result<Feed> {
        let conn = self.conn();
        let mut feed = conn
            .query_row(
                &format!("SELECT {FEED_COLUMNS} FROM feeds WHERE id = ?1"),
                [id],
                feed_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("feed {id}")))?;
        feed.folders = folders_of(&conn, id)?;
        Ok(feed)
    }

    /// 購読一覧をタイトル順（タイトルが無ければ URL）で返す。
    pub fn list_feeds(&self) -> Result<Vec<Feed>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {FEED_COLUMNS} FROM feeds ORDER BY COALESCE(title, url) COLLATE NOCASE, id"
        ))?;
        let mut feeds = stmt
            .query_map([], feed_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for feed in &mut feeds {
            feed.folders = folders_of(&conn, feed.id)?;
        }
        Ok(feeds)
    }

    /// フィードを削除する。記事とフォルダへの所属も CASCADE で消える。
    pub fn remove_feed(&self, id: i64) -> Result<()> {
        let removed = self
            .conn()
            .execute("DELETE FROM feeds WHERE id = ?1", [id])?;
        if removed == 0 {
            return Err(Error::NotFound(format!("feed {id}")));
        }
        Ok(())
    }

    /// 取得成功を記録する。メタ情報を更新し、記事を upsert して新規件数を返す。
    ///
    /// 既存記事は本文とタイトルなどだけを更新し、既読・スター・fetched_at は保持する。
    pub fn save_fetched(
        &self,
        feed_id: i64,
        meta: &FeedMeta,
        entries: &[NewEntry],
        now: i64,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = save_fetched_conn(&tx, feed_id, meta, entries, now)?;
        tx.commit()?;
        Ok(inserted)
    }

    /// フィード登録・フォルダ付与・初回記事の保存を 1 トランザクションで行う。
    /// 戻り値は `(feed_id, 新規記事数)`。
    pub fn add_subscription(
        &self,
        url: &str,
        meta: &FeedMeta,
        folders: &[String],
        entries: &[NewEntry],
        now: i64,
    ) -> Result<(i64, usize)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let feed_id = insert_feed_conn(&tx, url, meta.title.as_deref(), now)?;
        for folder in folders {
            add_feed_to_folder_conn(&tx, feed_id, folder)?;
        }
        let new_entries = save_fetched_conn(&tx, feed_id, meta, entries, now)?;
        tx.commit()?;
        Ok((feed_id, new_entries))
    }

    /// OPML から購読を取り込む。既存 URL はフォルダだけマージする。1 トランザクションで行う。
    pub fn import_feeds(&self, feeds: &[OpmlFeed], now: i64) -> Result<ImportReport> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut report = ImportReport::default();
        for feed in feeds {
            let feed_id = match find_feed_id_by_url_conn(&tx, &feed.url)? {
                Some(id) => {
                    report.merged += 1;
                    id
                }
                None => {
                    report.added += 1;
                    insert_feed_conn(&tx, &feed.url, feed.title.as_deref(), now)?
                }
            };
            for folder in &feed.folders {
                add_feed_to_folder_conn(&tx, feed_id, folder)?;
            }
        }
        tx.commit()?;
        Ok(report)
    }

    /// 304 Not Modified を記録する。
    pub fn mark_not_modified(&self, feed_id: i64, now: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE feeds SET last_fetched_at = ?2, last_error = NULL WHERE id = ?1",
            params![feed_id, now],
        )?;
        Ok(())
    }

    /// 取得失敗を記録する。
    pub fn mark_fetch_error(&self, feed_id: i64, message: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE feeds SET last_error = ?2 WHERE id = ?1",
            params![feed_id, message],
        )?;
        Ok(())
    }

    /// 記事を新しい順（公開日時が無ければ取得日時）で返す。
    pub fn list_entries(&self, filter: &EntryFilter) -> Result<Vec<Entry>> {
        let (condition, mut args) = filter_condition(filter);
        let mut sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM {ENTRY_FROM} WHERE {condition}
             ORDER BY COALESCE(e.published_at, e.fetched_at) DESC, e.id DESC"
        );
        match (filter.limit, filter.offset) {
            (None, None) => {}
            (limit, offset) => {
                // SQLite は OFFSET だけを書けないため、limit が無ければ -1（無制限）にする
                sql.push_str(" LIMIT ? OFFSET ?");
                args.push(limit.map_or(-1, i64::from).into());
                args.push(i64::from(offset.unwrap_or(0)).into());
            }
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let entries = stmt
            .query_map(params_from_iter(args), entry_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    /// 記事を 1 件返す。無ければ `NotFound("entry {id}")`。
    pub fn get_entry(&self, id: i64) -> Result<Entry> {
        self.conn()
            .query_row(
                &format!("SELECT {ENTRY_COLUMNS} FROM {ENTRY_FROM} WHERE e.id = ?1"),
                [id],
                entry_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::NotFound(format!("entry {id}")))
    }

    /// filter に合う記事の件数を返す。`limit` / `offset` は無視する。
    pub fn count_entries(&self, filter: &EntryFilter) -> Result<u32> {
        let (condition, args) = filter_condition(filter);
        let count = self.conn().query_row(
            &format!("SELECT COUNT(*) FROM {ENTRY_FROM} WHERE {condition}"),
            params_from_iter(args),
            |r| r.get(0),
        )?;
        Ok(count)
    }

    /// feed_id ごとの未読数を返す。未読が 0 のフィードは含まない。
    pub fn unread_counts(&self) -> Result<HashMap<i64, u32>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT feed_id, COUNT(*) FROM entries WHERE is_read = 0 GROUP BY feed_id")?;
        let counts = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        Ok(counts)
    }

    /// filter に合う未読をすべて既読にし、更新件数を返す。`limit` / `offset` は無視する。
    pub fn mark_all_read(&self, filter: &EntryFilter) -> Result<usize> {
        let (condition, args) = filter_condition(filter);
        let updated = self.conn().execute(
            &format!(
                "UPDATE entries SET is_read = 1
                 WHERE is_read = 0 AND id IN (SELECT e.id FROM {ENTRY_FROM} WHERE {condition})"
            ),
            params_from_iter(args),
        )?;
        Ok(updated)
    }

    /// 既読・未読を付ける。更新できた件数を返す（存在しない id は無視）。
    pub fn set_read(&self, ids: &[i64], read: bool) -> Result<usize> {
        self.set_flag("is_read", ids, read)
    }

    /// スターを付ける・外す。更新できた件数を返す（存在しない id は無視）。
    pub fn set_starred(&self, ids: &[i64], starred: bool) -> Result<usize> {
        self.set_flag("is_starred", ids, starred)
    }

    fn set_flag(&self, column: &'static str, ids: &[i64], value: bool) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut updated = 0;
        {
            let mut stmt =
                tx.prepare(&format!("UPDATE entries SET {column} = ?1 WHERE id = ?2"))?;
            for id in ids {
                updated += stmt.execute(params![value, id])?;
            }
        }
        tx.commit()?;
        Ok(updated)
    }
}

fn feed_from_row(r: &Row<'_>) -> rusqlite::Result<Feed> {
    Ok(Feed {
        id: r.get(0)?,
        url: r.get(1)?,
        title: r.get(2)?,
        site_url: r.get(3)?,
        etag: r.get(4)?,
        last_modified: r.get(5)?,
        last_fetched_at: r.get(6)?,
        last_error: r.get(7)?,
        created_at: r.get(8)?,
        folders: Vec::new(),
    })
}

fn entry_from_row(r: &Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        feed_id: r.get(1)?,
        guid: r.get(2)?,
        url: r.get(3)?,
        title: r.get(4)?,
        author: r.get(5)?,
        summary: r.get(6)?,
        content: r.get(7)?,
        published_at: r.get(8)?,
        fetched_at: r.get(9)?,
        is_read: r.get(10)?,
        is_starred: r.get(11)?,
        feed_title: r.get(12)?,
    })
}

/// filter の絞り込み条件（`limit` / `offset` 以外）を WHERE 句とパラメータにする。
fn filter_condition(filter: &EntryFilter) -> (String, Vec<Value>) {
    let mut sql = String::from("1 = 1");
    let mut args: Vec<Value> = Vec::new();
    if filter.unread_only {
        sql.push_str(" AND e.is_read = 0");
    }
    if filter.starred_only {
        sql.push_str(" AND e.is_starred = 1");
    }
    if let Some(feed_id) = filter.feed_id {
        sql.push_str(" AND e.feed_id = ?");
        args.push(feed_id.into());
    }
    if let Some(folder) = &filter.folder {
        sql.push_str(
            " AND e.feed_id IN (SELECT ff.feed_id FROM feed_folders ff
                                 JOIN folders fo ON fo.id = ff.folder_id WHERE fo.name = ?)",
        );
        args.push(folder.clone().into());
    }
    (sql, args)
}

/// v2 より前に保存した記事（`plain_text` が NULL）の検索用テキストを埋め、件数を返す。
/// 埋めた記事があれば索引を最適化する。
fn backfill_plain_text(conn: &mut Connection) -> Result<usize> {
    let mut total = 0;
    loop {
        let tx = conn.transaction()?;
        let rows: Vec<(i64, Option<String>, Option<String>)> = {
            let mut stmt = tx.prepare(
                "SELECT id, summary, content FROM entries WHERE plain_text IS NULL LIMIT ?1",
            )?;
            stmt.query_map([BACKFILL_BATCH], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        if rows.is_empty() {
            break;
        }
        {
            // テキストが空の記事も空文字列にして、次に開いたときの対象から外す
            let mut update = tx.prepare("UPDATE entries SET plain_text = ?2 WHERE id = ?1")?;
            for (id, summary, content) in &rows {
                update.execute(params![
                    id,
                    search_text(summary.as_deref(), content.as_deref())
                ])?;
            }
        }
        tx.commit()?;
        total += rows.len();
    }
    if total > 0 {
        conn.execute(
            "INSERT INTO entries_fts(entries_fts) VALUES ('optimize')",
            [],
        )?;
    }
    Ok(total)
}

fn insert_feed_conn(conn: &Connection, url: &str, title: Option<&str>, now: i64) -> Result<i64> {
    let inserted = conn.execute(
        "INSERT INTO feeds (url, title, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(url) DO NOTHING",
        params![url, title, now],
    )?;
    if inserted == 0 {
        return Err(Error::AlreadySubscribed(url.to_owned()));
    }
    Ok(conn.last_insert_rowid())
}

fn find_feed_id_by_url_conn(conn: &Connection, url: &str) -> Result<Option<i64>> {
    let id = conn
        .query_row("SELECT id FROM feeds WHERE url = ?1", [url], |r| r.get(0))
        .optional()?;
    Ok(id)
}

fn save_fetched_conn(
    conn: &Connection,
    feed_id: i64,
    meta: &FeedMeta,
    entries: &[NewEntry],
    now: i64,
) -> Result<usize> {
    let updated = conn.execute(
        "UPDATE feeds SET title = COALESCE(?2, title), site_url = COALESCE(?3, site_url),
                          etag = ?4, last_modified = ?5, last_fetched_at = ?6, last_error = NULL
         WHERE id = ?1",
        params![
            feed_id,
            meta.title,
            meta.site_url,
            meta.etag,
            meta.last_modified,
            now
        ],
    )?;
    if updated == 0 {
        return Err(Error::NotFound(format!("feed {feed_id}")));
    }
    let mut inserted = 0;
    {
        let mut insert = conn.prepare(
            "INSERT INTO entries (feed_id, guid, url, title, author, summary, content, published_at, fetched_at, plain_text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(feed_id, guid) DO NOTHING",
        )?;
        let mut update = conn.prepare(
            "UPDATE entries SET url = ?3, title = ?4, author = ?5, summary = ?6, content = ?7, published_at = ?8,
                                plain_text = ?9
             WHERE feed_id = ?1 AND guid = ?2",
        )?;
        for e in entries {
            let text = search_text(e.summary.as_deref(), e.content.as_deref());
            let added = insert.execute(params![
                feed_id,
                e.guid,
                e.url,
                e.title,
                e.author,
                e.summary,
                e.content,
                e.published_at,
                now,
                text
            ])?;
            if added == 1 {
                inserted += 1;
            } else {
                update.execute(params![
                    feed_id,
                    e.guid,
                    e.url,
                    e.title,
                    e.author,
                    e.summary,
                    e.content,
                    e.published_at,
                    text
                ])?;
            }
        }
    }
    Ok(inserted)
}

/// フォルダ名の前後の空白を取り除く。空になった場合は `Error::InvalidInput`。
pub(crate) fn normalize_folder_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidInput("folder name must not be empty".into()));
    }
    Ok(trimmed.to_owned())
}

fn add_feed_to_folder_conn(conn: &Connection, feed_id: i64, folder: &str) -> Result<()> {
    let folder = normalize_folder_name(folder)?;
    conn.execute(
        "INSERT INTO folders (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
        [&folder],
    )?;
    let folder_id: i64 =
        conn.query_row("SELECT id FROM folders WHERE name = ?1", [&folder], |r| {
            r.get(0)
        })?;
    conn.execute(
        "INSERT INTO feed_folders (feed_id, folder_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![feed_id, folder_id],
    )?;
    Ok(())
}

fn folders_of(conn: &Connection, feed_id: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT f.name FROM folders f JOIN feed_folders ff ON ff.folder_id = f.id
         WHERE ff.feed_id = ?1 ORDER BY f.name",
    )?;
    stmt.query_map([feed_id], |r| r.get(0))?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn entry(guid: &str, published_at: i64) -> NewEntry {
        NewEntry {
            guid: guid.to_owned(),
            title: Some(format!("title {guid}")),
            published_at: Some(published_at),
            ..Default::default()
        }
    }

    fn titles(entries: &[Entry]) -> Vec<&str> {
        entries
            .iter()
            .map(|e| e.title.as_deref().unwrap())
            .collect()
    }

    // ---- feeds / folders ----

    #[test]
    fn busy_timeout_is_five_seconds() {
        let s = store();
        let ms: i64 = s
            .conn()
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(ms, 5000);
    }

    #[test]
    fn insert_and_get_feed() {
        let s = store();
        let id = s
            .insert_feed("https://a.example/feed", Some("A"), 100)
            .unwrap();
        let feed = s.get_feed(id).unwrap();
        assert_eq!(feed.url, "https://a.example/feed");
        assert_eq!(feed.title.as_deref(), Some("A"));
        assert_eq!(feed.created_at, 100);
        assert_eq!(feed.last_fetched_at, None);
        assert!(feed.folders.is_empty());
        assert_eq!(
            s.find_feed_id_by_url("https://a.example/feed").unwrap(),
            Some(id)
        );
        assert_eq!(
            s.find_feed_id_by_url("https://missing.example/").unwrap(),
            None
        );
    }

    #[test]
    fn insert_duplicate_url_is_already_subscribed() {
        let s = store();
        s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let err = s
            .insert_feed("https://a.example/feed", None, 2)
            .unwrap_err();
        assert!(matches!(err, Error::AlreadySubscribed(url) if url == "https://a.example/feed"));
    }

    #[test]
    fn feeds_and_folders_are_many_to_many() {
        let s = store();
        let a = s
            .insert_feed("https://a.example/feed", Some("A"), 1)
            .unwrap();
        let b = s
            .insert_feed("https://b.example/feed", Some("B"), 1)
            .unwrap();
        s.add_feed_to_folder(a, "Tech").unwrap();
        s.add_feed_to_folder(a, "News").unwrap();
        s.add_feed_to_folder(a, "Tech").unwrap();
        s.add_feed_to_folder(b, "Tech").unwrap();
        assert_eq!(s.get_feed(a).unwrap().folders, vec!["News", "Tech"]);
        assert_eq!(s.get_feed(b).unwrap().folders, vec!["Tech"]);
    }

    #[test]
    fn add_feed_to_folder_rejects_blank_name() {
        let s = store();
        let a = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let err = s.add_feed_to_folder(a, "   ").unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
        assert!(s.get_feed(a).unwrap().folders.is_empty());
    }

    #[test]
    fn add_feed_to_folder_trims_whitespace() {
        let s = store();
        let a = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.add_feed_to_folder(a, "  Tech  ").unwrap();
        assert_eq!(s.get_feed(a).unwrap().folders, vec!["Tech"]);
    }

    #[test]
    fn list_feeds_orders_by_title_then_url() {
        let s = store();
        s.insert_feed("https://z.example/feed", Some("beta"), 1)
            .unwrap();
        s.insert_feed("https://y.example/feed", Some("Alpha"), 1)
            .unwrap();
        s.insert_feed("https://c.example/feed", None, 1).unwrap();
        let urls: Vec<String> = s.list_feeds().unwrap().into_iter().map(|f| f.url).collect();
        assert_eq!(
            urls,
            vec![
                "https://y.example/feed",
                "https://z.example/feed",
                "https://c.example/feed"
            ]
        );
    }

    #[test]
    fn get_or_remove_missing_feed_is_not_found() {
        let s = store();
        assert!(matches!(s.get_feed(42), Err(Error::NotFound(_))));
        assert!(matches!(s.remove_feed(42), Err(Error::NotFound(_))));
    }

    #[test]
    fn open_file_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("tayoreader.db");
        let s = Store::open(&path).unwrap();
        s.insert_feed("https://a.example/feed", None, 1).unwrap();
        drop(s);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.list_feeds().unwrap().len(), 1);
    }

    // ---- add_subscription / import_feeds ----

    #[test]
    fn add_subscription_saves_feed_folder_and_entries_in_one_call() {
        let s = store();
        let meta = FeedMeta {
            title: Some("A".into()),
            ..Default::default()
        };
        let (feed_id, new_entries) = s
            .add_subscription(
                "https://a.example/feed",
                &meta,
                &["Tech".into()],
                &[entry("g1", 10)],
                1,
            )
            .unwrap();
        assert_eq!(new_entries, 1);
        let feed = s.get_feed(feed_id).unwrap();
        assert_eq!(feed.title.as_deref(), Some("A"));
        assert_eq!(feed.folders, vec!["Tech"]);
        assert_eq!(s.list_entries(&EntryFilter::default()).unwrap().len(), 1);
    }

    #[test]
    fn add_subscription_is_atomic_on_failure() {
        let s = store();
        let meta = FeedMeta {
            title: Some("A".into()),
            ..Default::default()
        };
        let err = s
            .add_subscription(
                "https://a.example/feed",
                &meta,
                &["Tech".into(), "  ".into()],
                &[entry("g1", 10)],
                1,
            )
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
        assert!(s.list_feeds().unwrap().is_empty());
        assert!(s.list_entries(&EntryFilter::default()).unwrap().is_empty());
    }

    #[test]
    fn import_feeds_adds_and_merges_folders() {
        let s = store();
        let existing = s
            .insert_feed("https://a.example/feed", Some("Mine"), 1)
            .unwrap();
        s.add_feed_to_folder(existing, "News").unwrap();

        let feeds = vec![
            OpmlFeed {
                url: "https://a.example/feed".into(),
                title: Some("From OPML".into()),
                folders: vec!["Tech".into()],
            },
            OpmlFeed {
                url: "https://b.example/feed".into(),
                title: Some("B".into()),
                folders: vec!["Tech".into()],
            },
        ];
        let report = s.import_feeds(&feeds, 2).unwrap();
        assert_eq!(
            report,
            ImportReport {
                added: 1,
                merged: 1
            }
        );

        let existing_feed = s.get_feed(existing).unwrap();
        assert_eq!(existing_feed.title.as_deref(), Some("Mine"));
        assert_eq!(existing_feed.folders, vec!["News", "Tech"]);
        assert_eq!(s.list_feeds().unwrap().len(), 2);
    }

    #[test]
    fn import_feeds_is_atomic_on_failure() {
        let s = store();
        let feeds = vec![
            OpmlFeed {
                url: "https://a.example/feed".into(),
                title: None,
                folders: vec!["Tech".into()],
            },
            OpmlFeed {
                url: "https://b.example/feed".into(),
                title: None,
                folders: vec!["  ".into()],
            },
        ];
        let err = s.import_feeds(&feeds, 1).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
        assert!(s.list_feeds().unwrap().is_empty());
    }

    // ---- entries ----

    #[test]
    fn save_fetched_counts_new_entries_and_keeps_flags() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let meta = FeedMeta::default();
        assert_eq!(
            s.save_fetched(feed, &meta, &[entry("g1", 10), entry("g2", 20)], 100)
                .unwrap(),
            2
        );

        let g1 = s.list_entries(&EntryFilter::default()).unwrap()[1].clone();
        assert_eq!(g1.guid, "g1");
        s.set_read(&[g1.id], true).unwrap();
        s.set_starred(&[g1.id], true).unwrap();

        let mut changed = entry("g1", 10);
        changed.title = Some("title g1 (edited)".into());
        assert_eq!(
            s.save_fetched(feed, &meta, &[changed, entry("g3", 30)], 200)
                .unwrap(),
            1
        );

        let entries = s.list_entries(&EntryFilter::default()).unwrap();
        assert_eq!(
            titles(&entries),
            vec!["title g3", "title g2", "title g1 (edited)"]
        );
        let g1 = &entries[2];
        assert!(g1.is_read);
        assert!(g1.is_starred);
        assert_eq!(g1.fetched_at, 100);
    }

    #[test]
    fn save_fetched_updates_meta_and_keeps_title_when_missing() {
        let s = store();
        let feed = s
            .insert_feed("https://a.example/feed", Some("From OPML"), 1)
            .unwrap();
        s.mark_fetch_error(feed, "boom").unwrap();
        let meta = FeedMeta {
            title: None,
            site_url: Some("https://a.example/".into()),
            etag: Some("\"v1\"".into()),
            last_modified: Some("Wed, 02 Sep 2026 09:00:00 GMT".into()),
        };
        s.save_fetched(feed, &meta, &[], 500).unwrap();
        let f = s.get_feed(feed).unwrap();
        assert_eq!(f.title.as_deref(), Some("From OPML"));
        assert_eq!(f.site_url.as_deref(), Some("https://a.example/"));
        assert_eq!(f.etag.as_deref(), Some("\"v1\""));
        assert_eq!(
            f.last_modified.as_deref(),
            Some("Wed, 02 Sep 2026 09:00:00 GMT")
        );
        assert_eq!(f.last_fetched_at, Some(500));
        assert_eq!(f.last_error, None);
    }

    #[test]
    fn fetch_error_and_not_modified_are_recorded() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.mark_fetch_error(feed, "HTTP 500").unwrap();
        let f = s.get_feed(feed).unwrap();
        assert_eq!(f.last_error.as_deref(), Some("HTTP 500"));
        assert_eq!(f.last_fetched_at, None);

        s.mark_not_modified(feed, 300).unwrap();
        let f = s.get_feed(feed).unwrap();
        assert_eq!(f.last_error, None);
        assert_eq!(f.last_fetched_at, Some(300));
    }

    #[test]
    fn remove_feed_cascades_entries_and_folder_links() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.add_feed_to_folder(feed, "Tech").unwrap();
        s.save_fetched(feed, &FeedMeta::default(), &[entry("g1", 10)], 100)
            .unwrap();
        s.remove_feed(feed).unwrap();
        assert!(s.list_entries(&EntryFilter::default()).unwrap().is_empty());

        let again = s.insert_feed("https://a.example/feed", None, 2).unwrap();
        assert!(s.get_feed(again).unwrap().folders.is_empty());
    }

    #[test]
    fn list_entries_filters() {
        let s = store();
        let a = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let b = s.insert_feed("https://b.example/feed", None, 1).unwrap();
        s.add_feed_to_folder(a, "Tech").unwrap();
        s.save_fetched(
            a,
            &FeedMeta::default(),
            &[entry("a1", 10), entry("a2", 30)],
            100,
        )
        .unwrap();
        s.save_fetched(b, &FeedMeta::default(), &[entry("b1", 20)], 100)
            .unwrap();
        let all = s.list_entries(&EntryFilter::default()).unwrap();
        assert_eq!(titles(&all), vec!["title a2", "title b1", "title a1"]);

        s.set_read(&[all[0].id], true).unwrap();
        s.set_starred(&[all[1].id], true).unwrap();

        let unread = s
            .list_entries(&EntryFilter {
                unread_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(titles(&unread), vec!["title b1", "title a1"]);

        let starred = s
            .list_entries(&EntryFilter {
                starred_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(titles(&starred), vec!["title b1"]);

        let by_feed = s
            .list_entries(&EntryFilter {
                feed_id: Some(b),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(titles(&by_feed), vec!["title b1"]);

        let by_folder = s
            .list_entries(&EntryFilter {
                folder: Some("Tech".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(titles(&by_folder), vec!["title a2", "title a1"]);

        let limited = s
            .list_entries(&EntryFilter {
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(titles(&limited), vec!["title a2"]);
    }

    #[test]
    fn entries_without_published_at_sort_by_fetched_at() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let undated = NewEntry {
            guid: "undated".into(),
            title: Some("undated".into()),
            ..Default::default()
        };
        s.save_fetched(feed, &FeedMeta::default(), &[entry("old", 10), undated], 50)
            .unwrap();
        let entries = s.list_entries(&EntryFilter::default()).unwrap();
        assert_eq!(titles(&entries), vec!["undated", "title old"]);
    }

    #[test]
    fn set_flags_returns_updated_count() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.save_fetched(feed, &FeedMeta::default(), &[entry("g1", 10)], 100)
            .unwrap();
        let id = s.list_entries(&EntryFilter::default()).unwrap()[0].id;
        assert_eq!(s.set_read(&[id, 9999], true).unwrap(), 1);
        assert_eq!(s.set_read(&[id], false).unwrap(), 1);
        assert!(!s.list_entries(&EntryFilter::default()).unwrap()[0].is_read);
        assert_eq!(s.set_starred(&[9999], true).unwrap(), 0);
    }

    // ---- GUI 向けの参照・一括操作 ----

    /// フィード a（フォルダ Tech、記事 a1=10, a2=30）と b（フォルダなし、b1=20）を作る。
    fn two_feeds() -> (Store, i64, i64) {
        let s = store();
        let a = s
            .insert_feed("https://a.example/feed", Some("A"), 1)
            .unwrap();
        // タイトルが空白だけのフィードは URL で表示する
        let b = s
            .insert_feed("https://b.example/feed", Some("  "), 1)
            .unwrap();
        s.add_feed_to_folder(a, "Tech").unwrap();
        s.save_fetched(
            a,
            &FeedMeta::default(),
            &[entry("a1", 10), entry("a2", 30)],
            100,
        )
        .unwrap();
        s.save_fetched(b, &FeedMeta::default(), &[entry("b1", 20)], 100)
            .unwrap();
        (s, a, b)
    }

    fn filter_by_feed(feed_id: i64) -> EntryFilter {
        EntryFilter {
            feed_id: Some(feed_id),
            ..Default::default()
        }
    }

    #[test]
    fn list_entries_includes_feed_title_or_url() {
        let (s, _, _) = two_feeds();
        let feed_titles: Vec<Option<String>> = s
            .list_entries(&EntryFilter::default())
            .unwrap()
            .into_iter()
            .map(|e| e.feed_title)
            .collect();
        assert_eq!(
            feed_titles,
            vec![
                Some("A".into()),
                Some("https://b.example/feed".into()),
                Some("A".into())
            ]
        );
    }

    #[test]
    fn list_entries_pages_with_offset() {
        let (s, _, _) = two_feeds();
        let page = |limit, offset| {
            let entries = s
                .list_entries(&EntryFilter {
                    limit,
                    offset,
                    ..Default::default()
                })
                .unwrap();
            entries.iter().map(|e| e.guid.clone()).collect::<Vec<_>>()
        };
        assert_eq!(page(Some(2), Some(0)), vec!["a2", "b1"]);
        assert_eq!(page(Some(2), Some(2)), vec!["a1"]);
        assert_eq!(page(None, Some(1)), vec!["b1", "a1"]);
        assert!(page(Some(2), Some(5)).is_empty());
    }

    #[test]
    fn get_entry_returns_entry_or_not_found() {
        let (s, a, _) = two_feeds();
        let id = s.list_entries(&filter_by_feed(a)).unwrap()[0].id;
        let e = s.get_entry(id).unwrap();
        assert_eq!(e.guid, "a2");
        assert_eq!(e.feed_title.as_deref(), Some("A"));
        let err = s.get_entry(9999).unwrap_err();
        assert!(
            matches!(&err, Error::NotFound(m) if m == "entry 9999"),
            "{err:?}"
        );
    }

    #[test]
    fn unread_counts_by_feed() {
        let (s, a, b) = two_feeds();
        let a2 = s.list_entries(&filter_by_feed(a)).unwrap()[0].id;
        s.set_read(&[a2], true).unwrap();
        assert_eq!(s.unread_counts().unwrap(), HashMap::from([(a, 1), (b, 1)]));
        s.mark_all_read(&filter_by_feed(b)).unwrap();
        assert_eq!(s.unread_counts().unwrap(), HashMap::from([(a, 1)]));
    }

    #[test]
    fn count_entries_ignores_limit() {
        let (s, a, _) = two_feeds();
        let a2 = s.list_entries(&filter_by_feed(a)).unwrap()[0].id;
        s.set_starred(&[a2], true).unwrap();
        let starred = EntryFilter {
            starred_only: true,
            limit: Some(0),
            ..Default::default()
        };
        assert_eq!(s.count_entries(&starred).unwrap(), 1);
        assert_eq!(s.count_entries(&EntryFilter::default()).unwrap(), 3);
    }

    #[test]
    fn mark_all_read_by_feed_folder_and_starred() {
        let (s, a, b) = two_feeds();
        let unread = |filter: EntryFilter| {
            s.count_entries(&EntryFilter {
                unread_only: true,
                ..filter
            })
            .unwrap()
        };

        let b1 = s.list_entries(&filter_by_feed(b)).unwrap()[0].id;
        s.set_starred(&[b1], true).unwrap();
        let starred = EntryFilter {
            starred_only: true,
            ..Default::default()
        };
        assert_eq!(s.mark_all_read(&starred).unwrap(), 1);
        assert_eq!(unread(filter_by_feed(b)), 0);
        assert_eq!(unread(filter_by_feed(a)), 2);

        let tech = EntryFilter {
            folder: Some("Tech".into()),
            ..Default::default()
        };
        assert_eq!(s.mark_all_read(&tech).unwrap(), 2);
        assert_eq!(s.mark_all_read(&tech).unwrap(), 0);
        assert_eq!(unread(EntryFilter::default()), 0);
    }

    #[test]
    fn mark_all_read_ignores_limit_and_offset() {
        let (s, _, _) = two_feeds();
        let filter = EntryFilter {
            limit: Some(1),
            offset: Some(1),
            ..Default::default()
        };
        assert_eq!(s.mark_all_read(&filter).unwrap(), 3);
    }

    // ---- 検索の索引 ----

    /// 索引から phrase（二重引用符で囲んだ語）に当たる記事 id を返す。
    fn fts_ids(s: &Store, phrase: &str) -> Vec<i64> {
        let conn = s.conn();
        let mut stmt = conn
            .prepare("SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?1 ORDER BY rowid")
            .unwrap();
        stmt.query_map([format!("\"{phrase}\"")], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// 索引の中身（shadow table）。書き換えが起きたかを比べるのに使う。
    fn fts_data(s: &Store) -> Vec<(i64, Vec<u8>)> {
        let conn = s.conn();
        let mut stmt = conn
            .prepare("SELECT id, block FROM entries_fts_data ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    /// 索引と entries の内容が一致していることを確かめる。
    fn assert_fts_intact(s: &Store) {
        s.conn()
            .execute(
                "INSERT INTO entries_fts(entries_fts, rank) VALUES ('integrity-check', 1)",
                [],
            )
            .unwrap();
    }

    fn article(guid: &str, title: &str, summary: Option<&str>, content: Option<&str>) -> NewEntry {
        NewEntry {
            guid: guid.into(),
            title: Some(title.into()),
            summary: summary.map(Into::into),
            content: content.map(Into::into),
            published_at: Some(10),
            ..Default::default()
        }
    }

    #[test]
    fn saving_entries_indexes_title_and_plain_text() {
        let s = store();
        let (feed, _) = s
            .add_subscription(
                "https://a.example/feed",
                &FeedMeta::default(),
                &[],
                &[article(
                    "g1",
                    "台風情報まとめ",
                    Some("要約"),
                    Some(r#"<p>本日の<b>天気予報</b>です。<a href="https://secret.example/">詳細</a></p>"#),
                )],
                1,
            )
            .unwrap();
        let id = s.list_entries(&filter_by_feed(feed)).unwrap()[0].id;
        let plain: String = s
            .conn()
            .query_row("SELECT plain_text FROM entries WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(plain, "要約\n本日の天気予報です。詳細");
        assert_eq!(fts_ids(&s, "台風情報"), vec![id]);
        assert_eq!(fts_ids(&s, "の天気予報で"), vec![id]);
        // タグや属性の中の文字列は索引に入らない
        assert!(fts_ids(&s, "secret").is_empty());
        assert_fts_intact(&s);
    }

    #[test]
    fn updating_entry_replaces_index() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.save_fetched(
            feed,
            &FeedMeta::default(),
            &[article("g1", "古いタイトル", Some("古い本文"), None)],
            100,
        )
        .unwrap();
        s.save_fetched(
            feed,
            &FeedMeta::default(),
            &[article("g1", "新しいタイトル", None, Some("新しい本文"))],
            200,
        )
        .unwrap();
        let id = s.list_entries(&EntryFilter::default()).unwrap()[0].id;
        assert!(fts_ids(&s, "古いタイ").is_empty());
        assert!(fts_ids(&s, "古い本文").is_empty());
        assert_eq!(fts_ids(&s, "新しいタイ"), vec![id]);
        assert_eq!(fts_ids(&s, "新しい本文"), vec![id]);
        assert_fts_intact(&s);
    }

    #[test]
    fn flags_and_unchanged_resave_do_not_rewrite_index() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        let entries = [article("g1", "Rust の記事", Some("本文"), None)];
        s.save_fetched(feed, &FeedMeta::default(), &entries, 100)
            .unwrap();
        let id = s.list_entries(&EntryFilter::default()).unwrap()[0].id;
        let before = fts_data(&s);

        s.set_read(&[id], true).unwrap();
        s.set_starred(&[id], true).unwrap();
        s.set_read(&[id], false).unwrap();
        s.mark_all_read(&EntryFilter::default()).unwrap();
        // 更新で同じ内容が届いても索引は書き換えない
        s.save_fetched(feed, &FeedMeta::default(), &entries, 200)
            .unwrap();

        assert_eq!(fts_data(&s), before);
        assert_fts_intact(&s);
    }

    #[test]
    fn removing_feed_removes_entries_from_index() {
        let s = store();
        let feed = s.insert_feed("https://a.example/feed", None, 1).unwrap();
        s.save_fetched(
            feed,
            &FeedMeta::default(),
            &[article("g1", "消える記事", Some("本文です"), None)],
            100,
        )
        .unwrap();
        assert_eq!(fts_ids(&s, "消える記事").len(), 1);
        s.remove_feed(feed).unwrap();
        assert!(fts_ids(&s, "消える記事").is_empty());
        assert!(fts_ids(&s, "本文です").is_empty());
        assert_fts_intact(&s);
    }

    #[test]
    fn opening_v1_database_backfills_search_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.db");
        {
            // v1 のスキーマのまま記事を入れる（BACKFILL_BATCH をまたぐ件数）
            let mut conn = Connection::open(&path).unwrap();
            Migrations::from_slice(&MIGRATIONS[..1])
                .to_latest(&mut conn)
                .unwrap();
            conn.execute(
                "INSERT INTO feeds (id, url, created_at) VALUES (1, 'https://a.example/feed', 1)",
                [],
            )
            .unwrap();
            let tx = conn.transaction().unwrap();
            for i in 1..=1201 {
                let (title, summary) = match i {
                    1 => (
                        Some("古い記事".to_owned()),
                        Some("<p>既存の台風情報</p>".to_owned()),
                    ),
                    2 => (None, None),
                    _ => (Some(format!("記事 {i}")), Some(format!("<p>本文 {i}</p>"))),
                };
                tx.execute(
                    "INSERT INTO entries (feed_id, guid, title, summary, fetched_at) VALUES (1, ?1, ?2, ?3, 1)",
                    params![format!("g{i}"), title, summary],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }

        let s = Store::open(&path).unwrap();
        let missing: i64 = s
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE plain_text IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(missing, 0);
        assert_eq!(fts_ids(&s, "台風情報"), vec![1]);
        assert_eq!(fts_ids(&s, "古い記事"), vec![1]);
        assert_eq!(fts_ids(&s, "本文 1201"), vec![1201]);
        assert_fts_intact(&s);
        // 埋め戻しは 1 回だけ
        assert_eq!(backfill_plain_text(&mut s.conn()).unwrap(), 0);
        drop(s);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(fts_ids(&reopened, "台風情報"), vec![1]);
    }
}
