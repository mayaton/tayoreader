//! HTTP でフィードを取得してパースする。

use std::time::Duration;

use feed_rs::model::Link;
use reqwest::{StatusCode, header};
use sha2::{Digest, Sha256};

use crate::model::{FeedMeta, NewEntry};
use crate::{Error, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("TayoReader/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// 304 Not Modified。
    NotModified,
    /// 取得してパースできた。`meta` には応答ヘッダの ETag / Last-Modified も入る。
    Fetched {
        meta: FeedMeta,
        entries: Vec<NewEntry>,
    },
}

#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self { client })
    }

    /// フィードを取得する。`etag` / `last_modified` があれば条件付き GET にする。
    pub async fn fetch(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<FetchOutcome> {
        let mut request = self.client.get(url);
        if let Some(etag) = etag {
            request = request.header(header::IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = last_modified {
            request = request.header(header::IF_MODIFIED_SINCE, last_modified);
        }
        let response = request.send().await?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified);
        }
        if !status.is_success() {
            return Err(Error::Http(status.to_string()));
        }
        let etag = header_value(&response, header::ETAG);
        let last_modified = header_value(&response, header::LAST_MODIFIED);
        let body = response.bytes().await?;
        let (mut meta, entries) = parse_feed(&body)?;
        meta.etag = etag;
        meta.last_modified = last_modified;
        Ok(FetchOutcome::Fetched { meta, entries })
    }
}

fn header_value(response: &reqwest::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// フィード本文（RSS / Atom / JSON Feed）をパースする。`meta` の etag / last_modified は空。
pub fn parse_feed(body: &[u8]) -> Result<(FeedMeta, Vec<NewEntry>)> {
    let feed = feed_rs::parser::Builder::new()
        .id_generator(|_, _, _| String::new())
        .build()
        .parse(body)
        .map_err(|e| Error::Parse(e.to_string()))?;
    let meta = FeedMeta {
        title: feed.title.map(|t| t.content),
        site_url: alternate_link(&feed.links),
        ..Default::default()
    };
    let entries = feed
        .entries
        .into_iter()
        .map(|e| {
            let url = alternate_link(&e.links);
            let title = e.title.map(|t| t.content);
            let published_at = e.published.or(e.updated).map(|d| d.timestamp());
            NewEntry {
                guid: entry_guid(&e.id, url.as_deref(), title.as_deref(), published_at),
                author: e.authors.first().map(|p| p.name.clone()),
                summary: e.summary.map(|t| t.content),
                content: e.content.and_then(|c| c.body),
                url,
                title,
                published_at,
            }
        })
        .collect();
    Ok((meta, entries))
}

/// rel が無いか alternate のリンクを優先し、無ければ先頭のリンクを返す。
fn alternate_link(links: &[Link]) -> Option<String> {
    links
        .iter()
        .find(|l| matches!(l.rel.as_deref(), None | Some("alternate")))
        .or_else(|| links.first())
        .map(|l| l.href.clone())
}

/// 記事の guid を決める。id → URL → タイトルと公開日時のハッシュの順に使う。
fn entry_guid(
    id: &str,
    url: Option<&str>,
    title: Option<&str>,
    published_at: Option<i64>,
) -> String {
    if !id.trim().is_empty() {
        return id.to_owned();
    }
    if let Some(url) = url.filter(|u| !u.is_empty()) {
        return url.to_owned();
    }
    let key = format!(
        "{}\n{}",
        title.unwrap_or_default(),
        published_at.map(|p| p.to_string()).unwrap_or_default()
    );
    hex::encode(Sha256::digest(key.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const RSS: &str = include_str!("../fixtures/rss.xml");
    const ATOM: &str = include_str!("../fixtures/atom.xml");

    #[test]
    fn parses_rss() {
        let (meta, entries) = parse_feed(RSS.as_bytes()).unwrap();
        assert_eq!(meta.title.as_deref(), Some("Example Blog"));
        assert_eq!(meta.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(entries.len(), 2);
        let first = &entries[0];
        assert_eq!(first.guid, "https://example.com/posts/2");
        assert_eq!(first.url.as_deref(), Some("https://example.com/posts/2"));
        assert_eq!(first.title.as_deref(), Some("Second post"));
        assert_eq!(first.summary.as_deref(), Some("Summary of the second post"));
        assert_eq!(first.published_at, Some(1_788_339_600));
    }

    #[test]
    fn parses_atom_and_prefers_alternate_link() {
        let (meta, entries) = parse_feed(ATOM.as_bytes()).unwrap();
        assert_eq!(meta.title.as_deref(), Some("Example Atom"));
        assert_eq!(meta.site_url.as_deref(), Some("https://example.org/"));
        let e = &entries[0];
        assert_eq!(e.guid, "urn:uuid:entry-1");
        assert_eq!(e.url.as_deref(), Some("https://example.org/entries/1"));
        assert_eq!(e.author.as_deref(), Some("Bob"));
        assert_eq!(e.content.as_deref(), Some("<p>Hello</p>"));
        assert_eq!(e.published_at, Some(1_788_429_600));
    }

    #[test]
    fn invalid_body_is_parse_error() {
        assert!(matches!(
            parse_feed(b"<html>not a feed</html>"),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn guid_falls_back_to_url_then_hash() {
        assert_eq!(entry_guid("id-1", Some("https://u"), None, None), "id-1");
        assert_eq!(entry_guid(" ", Some("https://u"), None, None), "https://u");
        let hashed = entry_guid("", None, Some("t"), Some(1));
        assert_eq!(hashed.len(), 64);
        assert_eq!(hashed, entry_guid("", Some(""), Some("t"), Some(1)));
        assert_ne!(hashed, entry_guid("", None, Some("t"), Some(2)));
    }

    const RSS_NO_GUID_NO_LINK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example Blog</title>
    <link>https://example.com/</link>
    <description>An example feed</description>
    <item>
      <title>Guidless post</title>
      <pubDate>Wed, 02 Sep 2026 09:00:00 +0000</pubDate>
      <description>Summary of the guidless post</description>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn guid_is_stable_hash_when_id_and_link_are_absent() {
        let (_, entries1) = parse_feed(RSS_NO_GUID_NO_LINK.as_bytes()).unwrap();
        let (_, entries2) = parse_feed(RSS_NO_GUID_NO_LINK.as_bytes()).unwrap();
        assert_eq!(entries1.len(), 1);
        assert_eq!(entries2.len(), 1);
        let guid1 = &entries1[0].guid;
        let guid2 = &entries2[0].guid;
        assert_eq!(guid1, guid2);
        assert_eq!(guid1.len(), 64);
        assert!(guid1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn fetch_returns_entries_and_cache_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/feed.xml"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .insert_header("Last-Modified", "Wed, 02 Sep 2026 09:00:00 GMT")
                    .set_body_string(RSS),
            )
            .mount(&server)
            .await;
        let outcome = Fetcher::new()
            .unwrap()
            .fetch(&format!("{}/feed.xml", server.uri()), None, None)
            .await
            .unwrap();
        let FetchOutcome::Fetched { meta, entries } = outcome else {
            panic!("expected Fetched, got {outcome:?}");
        };
        assert_eq!(meta.etag.as_deref(), Some("\"v1\""));
        assert_eq!(
            meta.last_modified.as_deref(),
            Some("Wed, 02 Sep 2026 09:00:00 GMT")
        );
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn fetch_sends_conditional_headers_and_handles_304() {
        let server = MockServer::start().await;
        // header マッチャは値をカンマで分割するため、日付を含むヘッダは受信リクエストで直接確認する
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"v1\""))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let outcome = Fetcher::new()
            .unwrap()
            .fetch(
                &server.uri(),
                Some("\"v1\""),
                Some("Wed, 02 Sep 2026 09:00:00 GMT"),
            )
            .await
            .unwrap();
        assert_eq!(outcome, FetchOutcome::NotModified);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests[0].headers.get("if-modified-since").unwrap(),
            "Wed, 02 Sep 2026 09:00:00 GMT"
        );
    }

    #[tokio::test]
    async fn fetch_non_success_status_is_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let err = Fetcher::new()
            .unwrap()
            .fetch(&server.uri(), None, None)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "HTTP error: 500 Internal Server Error");
    }

    #[tokio::test]
    async fn fetch_slow_response_is_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;
        let err = Fetcher::with_timeout(Duration::from_millis(100))
            .unwrap()
            .fetch(&server.uri(), None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Timeout), "{err:?}");
    }
}
