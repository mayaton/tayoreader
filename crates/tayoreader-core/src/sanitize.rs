//! 記事 HTML のサニタイズと、一覧用のプレーンテキスト抜粋。

use std::borrow::Cow;
use std::collections::HashSet;

use ammonia::{Builder, Url, UrlRelative};

/// 中身ごと取り除くタグ。
const DROP_WITH_CONTENT: [&str; 5] = ["script", "style", "iframe", "object", "form"];

/// 記事本文を WebView に出してよい HTML にする。
///
/// `base_url`（記事 URL、無ければサイト URL）があれば相対 URL を絶対 URL に解決する。
/// 無い、または解釈できない場合は相対 URL を取り除く。
pub fn article_html(html: &str, base_url: Option<&str>) -> String {
    let base = base_url.and_then(|u| Url::parse(u).ok());
    let srcset_base = base.clone();
    let mut builder = Builder::default();
    builder
        .clean_content_tags(HashSet::from(DROP_WITH_CONTENT))
        .add_tag_attributes("img", &["srcset"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "data"]))
        .set_tag_attribute_value("a", "target", "_blank")
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(move |element, attribute, value| {
            filter_attribute(element, attribute, value, srcset_base.as_ref())
        })
        .url_relative(match base {
            Some(base) => UrlRelative::RewriteWithBase(base),
            None => UrlRelative::Deny,
        });
    builder.clean(html).to_string()
}

/// data: を img 以外で使わせず、srcset の URL を解決する。
fn filter_attribute<'a>(
    element: &str,
    attribute: &str,
    value: &'a str,
    base: Option<&Url>,
) -> Option<Cow<'a, str>> {
    match (element, attribute) {
        ("img", "srcset") => resolve_srcset(value, base).map(Cow::Owned),
        (_, "href") if value.trim_start().to_ascii_lowercase().starts_with("data:") => None,
        _ => Some(Cow::Borrowed(value)),
    }
}

/// srcset の各候補を http / https の絶対 URL にする。残る候補が無ければ None。
fn resolve_srcset(value: &str, base: Option<&Url>) -> Option<String> {
    // data: は URL 中のカンマで候補の区切りと区別できないため、srcset では認めない
    if value.to_ascii_lowercase().contains("data:") {
        return None;
    }
    let candidates: Vec<String> = value
        .split(',')
        .filter_map(|candidate| {
            let mut parts = candidate.split_whitespace();
            let url = parts.next()?;
            let url = match base {
                Some(base) => base.join(url).ok()?,
                None => Url::parse(url).ok()?,
            };
            if !matches!(url.scheme(), "http" | "https") {
                return None;
            }
            let descriptor: Vec<&str> = parts.collect();
            Some(if descriptor.is_empty() {
                url.to_string()
            } else {
                format!("{url} {}", descriptor.join(" "))
            })
        })
        .collect();
    (!candidates.is_empty()).then(|| candidates.join(", "))
}

/// タグを除いたテキストを返す。空白は 1 つにまとめ、`max_chars` 文字を超えたら切って `…` を付ける。
pub fn plain_snippet(html: &str, max_chars: usize) -> String {
    let text = Builder::empty()
        .clean_content_tags(HashSet::from(DROP_WITH_CONTENT))
        .clean(html)
        .to_string();
    // ammonia はテキストを再エスケープして返すので戻す（&amp; は最後）
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut cut: String = normalized.chars().take(max_chars).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_scripts_frames_forms_and_event_handlers() {
        let html = r#"<p onclick="evil()">Hi<script>alert(1)</script></p>
<iframe src="https://x.example/"><p>fallback</p></iframe>
<object data="x.swf">obj</object><embed src="y.swf">
<form action="/s"><input name="q">form text</form><style>p{}</style>"#;
        let out = article_html(html, Some("https://example.com/"));
        for banned in [
            "onclick", "script", "alert", "iframe", "fallback", "object", "obj", "embed", "form",
            "input", "style",
        ] {
            assert!(!out.contains(banned), "{banned} remains in {out}");
        }
        assert!(out.contains("<p>Hi</p>"), "{out}");
    }

    #[test]
    fn links_open_in_new_window_without_opener() {
        let out = article_html(r#"<a href="https://a.example/">a</a>"#, None);
        assert!(out.contains(r#"href="https://a.example/""#), "{out}");
        assert!(out.contains(r#"target="_blank""#), "{out}");
        assert!(out.contains(r#"rel="noopener noreferrer""#), "{out}");
    }

    #[test]
    fn resolves_relative_urls_against_base() {
        let html = r#"<a href="/about">a</a><img src="img/x.png" srcset="img/x2.png 2x, https://cdn.example/x3.png 3x" alt="x">"#;
        let out = article_html(html, Some("https://example.com/posts/1"));
        assert!(out.contains(r#"href="https://example.com/about""#), "{out}");
        assert!(
            out.contains(r#"src="https://example.com/posts/img/x.png""#),
            "{out}"
        );
        assert!(
            out.contains(
                r#"srcset="https://example.com/posts/img/x2.png 2x, https://cdn.example/x3.png 3x""#
            ),
            "{out}"
        );
        assert!(out.contains(r#"alt="x""#), "{out}");
    }

    #[test]
    fn drops_relative_urls_without_base() {
        let out = article_html(r#"<a href="/about">a</a><img src="x.png">"#, None);
        assert!(!out.contains("href"), "{out}");
        assert!(!out.contains("src"), "{out}");
    }

    #[test]
    fn allows_only_safe_schemes_and_data_only_for_img() {
        let html = r#"<a href="javascript:alert(1)">j</a><a href="mailto:a@example.com">m</a>
<a href="data:text/html,x">d</a><img src="data:image/png;base64,AAAA">
<img srcset="data:image/png;base64,AAAA 1x">"#;
        let out = article_html(html, Some("https://example.com/"));
        assert!(!out.contains("javascript"), "{out}");
        assert!(out.contains(r#"href="mailto:a@example.com""#), "{out}");
        assert!(!out.contains("data:text"), "{out}");
        assert!(out.contains(r#"src="data:image/png;base64,AAAA""#), "{out}");
        assert!(!out.contains("srcset"), "{out}");
    }

    #[test]
    fn snippet_strips_tags_and_decodes_entities() {
        let html = "<p>Tom &amp; Jerry &lt;3&gt; &quot;hi&quot;&nbsp;there</p><script>x()</script>";
        assert_eq!(plain_snippet(html, 100), r#"Tom & Jerry <3> "hi" there"#);
    }

    #[test]
    fn snippet_collapses_whitespace() {
        assert_eq!(plain_snippet("  a\n\n  b\t c  ", 100), "a b c");
    }

    #[test]
    fn snippet_truncates_by_chars() {
        assert_eq!(plain_snippet("日本語のテキスト", 3), "日本語…");
        assert_eq!(plain_snippet("abc", 3), "abc");
        assert_eq!(plain_snippet("ab cd", 3), "ab…");
    }
}
