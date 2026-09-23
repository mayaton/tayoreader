//! OPML から購読リストを読み込む。

pub use crate::model::OpmlFeed;
use crate::{Error, Result};

/// OPML を読み、フィードを出現順に返す。同じ URL が複数フォルダにあれば 1 件にまとめる。
pub fn parse_opml(xml: &str) -> Result<Vec<OpmlFeed>> {
    let doc = ::opml::OPML::from_str(xml).map_err(|e| Error::Opml(e.to_string()))?;
    let mut feeds = Vec::new();
    collect(&doc.body.outlines, None, &mut feeds);
    Ok(feeds)
}

fn collect(outlines: &[::opml::Outline], folder: Option<&str>, feeds: &mut Vec<OpmlFeed>) {
    for outline in outlines {
        match outline.xml_url.as_deref().filter(|u| !u.is_empty()) {
            Some(url) => add_feed(feeds, url, outline_title(outline), folder),
            None => {
                let name = outline_title(outline);
                collect(&outline.outlines, name.as_deref().or(folder), feeds);
            }
        }
    }
}

fn add_feed(feeds: &mut Vec<OpmlFeed>, url: &str, title: Option<String>, folder: Option<&str>) {
    let feed = match feeds.iter_mut().position(|f| f.url == url) {
        Some(i) => &mut feeds[i],
        None => {
            feeds.push(OpmlFeed {
                url: url.to_owned(),
                title,
                folders: Vec::new(),
            });
            feeds.last_mut().expect("just pushed")
        }
    };
    if let Some(folder) = folder {
        if !feed.folders.iter().any(|f| f == folder) {
            feed.folders.push(folder.to_owned());
        }
    }
}

/// title 属性を優先し、無ければ text 属性を使う。どちらも空なら None。
fn outline_title(outline: &::opml::Outline) -> Option<String> {
    [outline.title.as_deref(), Some(outline.text.as_str())]
        .into_iter()
        .flatten()
        .find(|t| !t.trim().is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INOREADER: &str = include_str!("../fixtures/inoreader.opml");

    #[test]
    fn parses_inoreader_export() {
        let feeds = parse_opml(INOREADER).unwrap();
        assert_eq!(
            feeds,
            vec![
                OpmlFeed {
                    url: "https://example.com/feed.xml".into(),
                    title: Some("Example Blog".into()),
                    folders: vec!["Tech".into(), "News".into()],
                },
                OpmlFeed {
                    url: "https://example.org/feed.atom".into(),
                    title: Some("Atom Site".into()),
                    folders: vec!["Tech".into()],
                },
                OpmlFeed {
                    url: "https://old.example.net/rss".into(),
                    title: Some("Old Site".into()),
                    folders: vec!["Archive".into()],
                },
                OpmlFeed {
                    url: "https://loose.example.net/feed".into(),
                    title: Some("Loose".into()),
                    folders: vec![],
                },
            ]
        );
    }

    #[test]
    fn unnamed_folder_inherits_parent() {
        let xml = r#"<opml version="2.0"><head/><body>
            <outline text="Tech"><outline text="">
              <outline text="A" xmlUrl="https://a.example/feed"/>
            </outline></outline>
        </body></opml>"#;
        let feeds = parse_opml(xml).unwrap();
        assert_eq!(feeds[0].folders, vec!["Tech"]);
    }

    #[test]
    fn invalid_xml_is_opml_error() {
        assert!(matches!(parse_opml("<nope"), Err(Error::Opml(_))));
    }
}
