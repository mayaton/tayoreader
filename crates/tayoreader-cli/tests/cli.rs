use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS: &str = include_str!("../../tayoreader-core/fixtures/rss.xml");

fn tayoreader(db: &Path) -> assert_cmd::Command {
    let mut cmd = cargo_bin_cmd!("tayoreader");
    cmd.arg("--db").arg(db);
    cmd
}

#[test]
fn prints_version() {
    cargo_bin_cmd!("tayoreader")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(concat!(
            "tayoreader ",
            env!("CARGO_PKG_VERSION")
        )));
}

// assert_cmd は同期的にプロセスを待つので、モックサーバーを別ワーカーで動かすため multi_thread にする。
#[tokio::test(flavor = "multi_thread")]
async fn add_refresh_list_and_mark_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tayoreader.db");
    let url = format!("{}/feed.xml", server.uri());

    tayoreader(&db)
        .args(["add", &url, "--folder", "Tech"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "added feed 1: Example Blog (2 entries)",
        ));
    tayoreader(&db)
        .arg("feeds")
        .assert()
        .success()
        .stdout(predicate::str::contains("Example Blog").and(predicate::str::contains("[Tech]")));
    tayoreader(&db)
        .arg("refresh")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "refreshed 1 feeds: 0 new entries, 0 failed",
        ));
    tayoreader(&db)
        .args(["entries", "--unread"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Second post").and(predicate::str::contains("First post")),
        );

    // 1 件目に保存されたのはフィード先頭の "Second post"
    tayoreader(&db)
        .args(["read", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("updated 1 of 1 entries"));
    tayoreader(&db)
        .args(["entries", "--unread", "--folder", "Tech"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("First post")
                .and(predicate::str::contains("Second post").not()),
        );
    tayoreader(&db).args(["star", "2"]).assert().success();
    tayoreader(&db)
        .args(["entries", "--starred"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("First post")
                .and(predicate::str::contains("Second post").not()),
        );

    tayoreader(&db)
        .args(["remove", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("removed feed 1"));
    tayoreader(&db)
        .arg("entries")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn import_opml_then_refresh_reports_failures() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/broken.xml"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tayoreader.db");
    let opml = dir.path().join("subs.opml");
    std::fs::write(
        &opml,
        format!(
            r#"<opml version="1.0"><head/><body>
                 <outline text="Tech">
                   <outline text="OK" type="rss" xmlUrl="{uri}/ok.xml"/>
                   <outline text="Broken" type="rss" xmlUrl="{uri}/broken.xml"/>
                 </outline>
               </body></opml>"#,
            uri = server.uri()
        ),
    )
    .unwrap();

    tayoreader(&db)
        .arg("import-opml")
        .arg(&opml)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "imported 2 feeds (0 already subscribed)",
        ));
    tayoreader(&db)
        .arg("refresh")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "refreshed 2 feeds: 2 new entries, 1 failed",
        ))
        .stderr(predicate::str::contains("broken.xml").and(predicate::str::contains("500")));
    tayoreader(&db)
        .arg("feeds")
        .assert()
        .success()
        .stdout(predicate::str::contains("! HTTP error: HTTP 500"));
}

#[test]
fn unknown_feed_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    tayoreader(&dir.path().join("tayoreader.db"))
        .args(["remove", "42"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("feed 42 not found"));
}
