//! 保存と取得を組み合わせたユースケース。CLI と UI はここを入口にする。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::stream::{self, StreamExt};

pub use crate::model::ImportReport;

use crate::fetch::{FetchOutcome, Fetcher};
use crate::model::Feed;
use crate::opml::parse_opml;
use crate::store::{self, Store};
use crate::{Error, Result, now};

const MAX_CONCURRENT_FETCHES: usize = 8;

pub struct Service {
    store: Arc<Store>,
    fetcher: Fetcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedFeed {
    pub feed_id: i64,
    pub title: Option<String>,
    pub new_entries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Updated { new_entries: usize },
    NotModified,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRefresh {
    pub feed_id: i64,
    pub url: String,
    pub outcome: RefreshOutcome,
}

/// refresh の結果。`feeds` は feed_id 順。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub feeds: Vec<FeedRefresh>,
}

impl RefreshReport {
    pub fn new_entries(&self) -> usize {
        self.feeds
            .iter()
            .map(|f| match f.outcome {
                RefreshOutcome::Updated { new_entries } => new_entries,
                _ => 0,
            })
            .sum()
    }

    pub fn failures(&self) -> impl Iterator<Item = &FeedRefresh> {
        self.feeds
            .iter()
            .filter(|f| matches!(f.outcome, RefreshOutcome::Failed(_)))
    }
}

impl Service {
    pub fn new(store: Store, fetcher: Fetcher) -> Self {
        Self {
            store: Arc::new(store),
            fetcher,
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// フィードを取得し、パースできたら購読として保存する（初回の記事も保存）。
    pub async fn add_feed(&self, url: &str, folders: &[String]) -> Result<AddedFeed> {
        for folder in folders {
            store::normalize_folder_name(folder)?;
        }
        let lookup = url.to_owned();
        if self
            .blocking(move |s| s.find_feed_id_by_url(&lookup))
            .await?
            .is_some()
        {
            return Err(Error::AlreadySubscribed(url.to_owned()));
        }
        let FetchOutcome::Fetched { meta, entries } = self.fetcher.fetch(url, None, None).await?
        else {
            return Err(Error::Http("unexpected 304 Not Modified".into()));
        };
        let url = url.to_owned();
        let folders = folders.to_vec();
        let title = meta.title.clone();
        let (feed_id, new_entries) = self
            .blocking(move |s| s.add_subscription(&url, &meta, &folders, &entries, now()))
            .await?;
        Ok(AddedFeed {
            feed_id,
            title,
            new_entries,
        })
    }

    /// 全フィード（`feed_id` 指定時はそのフィードのみ）を取得する。
    ///
    /// フィード単位の取得失敗は `RefreshOutcome::Failed` に積んで続行する。
    /// `Err` になるのは DB の失敗と、指定した `feed_id` が無い場合だけ。
    pub async fn refresh(&self, feed_id: Option<i64>) -> Result<RefreshReport> {
        self.refresh_with_progress(feed_id, |_, _| {}).await
    }

    /// `refresh` と同じ。1 フィード終わるごとに `on_progress(完了数, 総数)` を呼ぶ。
    pub async fn refresh_with_progress(
        &self,
        feed_id: Option<i64>,
        on_progress: impl Fn(usize, usize) + Send + Sync,
    ) -> Result<RefreshReport> {
        let feeds = self
            .blocking(move |s| match feed_id {
                Some(id) => Ok(vec![s.get_feed(id)?]),
                None => s.list_feeds(),
            })
            .await?;
        let total = feeds.len();
        let done = AtomicUsize::new(0);
        let (done, on_progress) = (&done, &on_progress);
        let results: Vec<Result<FeedRefresh>> = stream::iter(feeds)
            .map(|feed| async move {
                let result = self.refresh_one(feed).await;
                on_progress(done.fetch_add(1, Ordering::SeqCst) + 1, total);
                result
            })
            .buffer_unordered(MAX_CONCURRENT_FETCHES)
            .collect()
            .await;
        let mut feeds = results.into_iter().collect::<Result<Vec<_>>>()?;
        feeds.sort_by_key(|f| f.feed_id);
        Ok(RefreshReport { feeds })
    }

    async fn refresh_one(&self, feed: Feed) -> Result<FeedRefresh> {
        let feed_id = feed.id;
        let fetched = self
            .fetcher
            .fetch(
                &feed.url,
                feed.etag.as_deref(),
                feed.last_modified.as_deref(),
            )
            .await;
        let outcome = match fetched {
            Ok(FetchOutcome::NotModified) => {
                self.blocking(move |s| s.mark_not_modified(feed_id, now()))
                    .await?;
                RefreshOutcome::NotModified
            }
            Ok(FetchOutcome::Fetched { meta, entries }) => {
                let new_entries = self
                    .blocking(move |s| s.save_fetched(feed_id, &meta, &entries, now()))
                    .await?;
                RefreshOutcome::Updated { new_entries }
            }
            Err(e) => {
                let message = e.to_string();
                let recorded = message.clone();
                self.blocking(move |s| s.mark_fetch_error(feed_id, &recorded))
                    .await?;
                RefreshOutcome::Failed(message)
            }
        };
        Ok(FeedRefresh {
            feed_id,
            url: feed.url,
            outcome,
        })
    }

    /// OPML から購読を取り込む。記事の取得はしない。
    pub async fn import_opml(&self, xml: &str) -> Result<ImportReport> {
        let feeds = parse_opml(xml)?;
        self.blocking(move |s| s.import_feeds(&feeds, now())).await
    }

    /// DB 処理を blocking スレッドで実行する。
    async fn blocking<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Store) -> Result<T> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        match tokio::task::spawn_blocking(move || f(&store)).await {
            Ok(result) => result,
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntryFilter;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RSS: &str = include_str!("../fixtures/rss.xml");
    const INOREADER: &str = include_str!("../fixtures/inoreader.opml");

    fn service() -> Service {
        Service::new(Store::open_in_memory().unwrap(), Fetcher::new().unwrap())
    }

    async fn serve(server: &MockServer, route: &str, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn add_feed_saves_feed_folders_and_entries() {
        let server = MockServer::start().await;
        serve(
            &server,
            "/feed.xml",
            ResponseTemplate::new(200).set_body_string(RSS),
        )
        .await;
        let svc = service();
        let url = format!("{}/feed.xml", server.uri());

        let added = svc.add_feed(&url, &["Tech".into()]).await.unwrap();
        assert_eq!(added.title.as_deref(), Some("Example Blog"));
        assert_eq!(added.new_entries, 2);

        let feed = svc.store().get_feed(added.feed_id).unwrap();
        assert_eq!(feed.url, url);
        assert_eq!(feed.folders, vec!["Tech"]);
        assert_eq!(feed.site_url.as_deref(), Some("https://example.com/"));
        assert!(feed.last_fetched_at.is_some());
        assert_eq!(
            svc.store()
                .list_entries(&EntryFilter::default())
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn add_feed_rejects_blank_folder_without_fetching() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .expect(0)
            .mount(&server)
            .await;
        let svc = service();
        let err = svc
            .add_feed(&format!("{}/feed.xml", server.uri()), &["  ".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err:?}");
    }

    #[tokio::test]
    async fn add_feed_rejects_duplicate_without_fetching() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .expect(1)
            .mount(&server)
            .await;
        let svc = service();
        svc.add_feed(&server.uri(), &[]).await.unwrap();
        let err = svc.add_feed(&server.uri(), &[]).await.unwrap_err();
        assert!(matches!(err, Error::AlreadySubscribed(_)), "{err:?}");
    }

    #[tokio::test]
    async fn add_feed_with_unparsable_body_saves_nothing() {
        let server = MockServer::start().await;
        serve(
            &server,
            "/",
            ResponseTemplate::new(200).set_body_string("<html></html>"),
        )
        .await;
        let svc = service();
        let err = svc
            .add_feed(&server.uri(), &["Tech".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err:?}");
        assert!(svc.store().list_feeds().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refresh_continues_after_failure() {
        let server = MockServer::start().await;
        serve(
            &server,
            "/ok.xml",
            ResponseTemplate::new(200).set_body_string(RSS),
        )
        .await;
        serve(&server, "/broken.xml", ResponseTemplate::new(500)).await;
        let svc = service();
        let ok = svc
            .store()
            .insert_feed(&format!("{}/ok.xml", server.uri()), None, 1)
            .unwrap();
        let broken = svc
            .store()
            .insert_feed(&format!("{}/broken.xml", server.uri()), None, 1)
            .unwrap();

        let report = svc.refresh(None).await.unwrap();
        assert_eq!(report.feeds.len(), 2);
        assert_eq!(report.feeds[0].feed_id, ok);
        assert_eq!(
            report.feeds[0].outcome,
            RefreshOutcome::Updated { new_entries: 2 }
        );
        assert_eq!(report.feeds[1].feed_id, broken);
        assert!(matches!(&report.feeds[1].outcome, RefreshOutcome::Failed(m) if m.contains("500")));
        assert_eq!(report.new_entries(), 2);
        assert_eq!(report.failures().count(), 1);

        let broken_feed = svc.store().get_feed(broken).unwrap();
        assert!(broken_feed.last_error.unwrap().contains("500"));
        assert_eq!(
            svc.store().get_feed(ok).unwrap().title.as_deref(),
            Some("Example Blog")
        );
    }

    #[tokio::test]
    async fn refresh_uses_etag_and_handles_304() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_string(RSS),
            )
            .mount(&server)
            .await;
        let svc = service();
        let added = svc.add_feed(&server.uri(), &[]).await.unwrap();

        let report = svc.refresh(Some(added.feed_id)).await.unwrap();
        assert_eq!(report.feeds[0].outcome, RefreshOutcome::NotModified);
        assert_eq!(report.new_entries(), 0);
    }

    #[tokio::test]
    async fn refresh_unknown_feed_is_not_found() {
        let err = service().refresh(Some(42)).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    }

    #[tokio::test]
    async fn import_opml_adds_and_merges() {
        let svc = service();
        let existing = svc
            .store()
            .insert_feed("https://example.com/feed.xml", Some("Mine"), 1)
            .unwrap();

        let report = svc.import_opml(INOREADER).await.unwrap();
        assert_eq!(
            report,
            ImportReport {
                added: 3,
                merged: 1
            }
        );

        let feed = svc.store().get_feed(existing).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Mine"));
        assert_eq!(feed.folders, vec!["News", "Tech"]);
        assert_eq!(svc.store().list_feeds().unwrap().len(), 4);

        let again = svc.import_opml(INOREADER).await.unwrap();
        assert_eq!(
            again,
            ImportReport {
                added: 0,
                merged: 4
            }
        );
    }

    #[tokio::test]
    async fn refresh_with_progress_reports_each_feed() {
        let server = MockServer::start().await;
        serve(
            &server,
            "/ok.xml",
            ResponseTemplate::new(200).set_body_string(RSS),
        )
        .await;
        serve(&server, "/broken.xml", ResponseTemplate::new(500)).await;
        let svc = service();
        for route in ["/ok.xml", "/broken.xml", "/missing.xml"] {
            svc.store()
                .insert_feed(&format!("{}{route}", server.uri()), None, 1)
                .unwrap();
        }

        let calls = std::sync::Mutex::new(Vec::new());
        let report = svc
            .refresh_with_progress(None, |done, total| {
                calls.lock().unwrap().push((done, total));
            })
            .await
            .unwrap();
        assert_eq!(report.feeds.len(), 3);
        let mut calls = calls.into_inner().unwrap();
        calls.sort();
        assert_eq!(calls, vec![(1, 3), (2, 3), (3, 3)]);
    }

    #[tokio::test]
    async fn refresh_with_progress_without_feeds_never_calls_back() {
        let called = AtomicUsize::new(0);
        let report = service()
            .refresh_with_progress(None, |_, _| {
                called.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .unwrap();
        assert!(report.feeds.is_empty());
        assert_eq!(called.load(Ordering::SeqCst), 0);
    }
}
