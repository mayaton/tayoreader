//! SQLite への読み書き。DB アクセスはすべてこのモジュールを通す。

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use rusqlite_migration::{M, Migrations};

use crate::model::{Entry, EntryFilter, Feed, FeedMeta, NewEntry};
use crate::{Error, Result};

const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("../migrations/001_init.sql"))];

const FEED_COLUMNS: &str =
    "id, url, title, site_url, etag, last_modified, last_fetched_at, last_error, created_at";

const ENTRY_COLUMNS: &str = "e.id, e.feed_id, e.guid, e.url, e.title, e.author, e.summary, e.content, e.published_at, e.fetched_at, e.is_read, e.is_starred";

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
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("store mutex poisoned")
    }

    /// フィードを登録して id を返す。URL が登録済みなら `AlreadySubscribed`。
    pub fn insert_feed(&self, url: &str, title: Option<&str>, now: i64) -> Result<i64> {
        let conn = self.conn();
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

    pub fn find_feed_id_by_url(&self, url: &str) -> Result<Option<i64>> {
        let id = self
            .conn()
            .query_row("SELECT id FROM feeds WHERE url = ?1", [url], |r| r.get(0))
            .optional()?;
        Ok(id)
    }

    /// フィードをフォルダに入れる。フォルダが無ければ作り、既に入っていれば何もしない。
    pub fn add_feed_to_folder(&self, feed_id: i64, folder: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO folders (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [folder],
        )?;
        let folder_id: i64 =
            conn.query_row("SELECT id FROM folders WHERE name = ?1", [folder], |r| {
                r.get(0)
            })?;
        conn.execute(
            "INSERT INTO feed_folders (feed_id, folder_id) VALUES (?1, ?2) ON CONFLICT DO NOTHING",
            params![feed_id, folder_id],
        )?;
        Ok(())
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
        let updated = tx.execute(
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
            let mut insert = tx.prepare(
                "INSERT INTO entries (feed_id, guid, url, title, author, summary, content, published_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(feed_id, guid) DO NOTHING",
            )?;
            let mut update = tx.prepare(
                "UPDATE entries SET url = ?3, title = ?4, author = ?5, summary = ?6, content = ?7, published_at = ?8
                 WHERE feed_id = ?1 AND guid = ?2",
            )?;
            for e in entries {
                let added = insert.execute(params![
                    feed_id,
                    e.guid,
                    e.url,
                    e.title,
                    e.author,
                    e.summary,
                    e.content,
                    e.published_at,
                    now
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
                        e.published_at
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(inserted)
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
        let mut sql = format!("SELECT {ENTRY_COLUMNS} FROM entries e WHERE 1 = 1");
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
                                     JOIN folders f ON f.id = ff.folder_id WHERE f.name = ?)",
            );
            args.push(folder.clone().into());
        }
        sql.push_str(" ORDER BY COALESCE(e.published_at, e.fetched_at) DESC, e.id DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(" LIMIT ?");
            args.push(i64::from(limit).into());
        }
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let entries = stmt
            .query_map(params_from_iter(args), entry_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
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
    })
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
}
