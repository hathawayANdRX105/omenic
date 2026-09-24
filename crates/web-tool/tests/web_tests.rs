//! web_fetch / web_search 的边界与 SSRF 策略回归。
//!
//! 不触网：策略测试全部用 IP 字面量与 `.local` 主机名（解析确定失败），
//! fetch/search 的真实请求路径留给 CI 之外的本地冒烟（无证书不跑）。

use serde_json::json;
use web_tool::{
    UrlPolicyError, ensure_public_url, html_to_text, parse_queries, parse_url, render_search,
};

#[test]
fn url_policy_rejects_every_non_public_literal() {
    for url in [
        "http://127.0.0.1/admin",
        "http://10.0.0.1/",
        "http://192.168.1.1/",
        "http://172.16.0.1/",
        "http://169.254.169.254/latest/meta-data",
        "http://0.0.0.0/",
        "http://[::1]/",
        "http://[fc00::1]/",
        "http://[fe80::1]/",
        "http://localhost:8080/",
        "http://printer.local/",
        "ftp://93.184.216.34/",
        "file:///etc/passwd",
        "http:///nohost",
    ] {
        assert!(
            ensure_public_url(url).is_err(),
            "policy accepted non-public URL {url}"
        );
    }
    assert_eq!(
        ensure_public_url("http://127.0.0.1/"),
        Err(UrlPolicyError::PrivateAddress)
    );
    assert_eq!(
        ensure_public_url("ftp://93.184.216.34/"),
        Err(UrlPolicyError::NotHttp)
    );
    assert_eq!(
        ensure_public_url("http://printer.local/"),
        Err(UrlPolicyError::PrivateAddress)
    );
}

#[test]
fn url_policy_accepts_public_literal() {
    assert!(ensure_public_url("https://93.184.216.34/docs").is_ok());
    assert!(ensure_public_url("http://1.1.1.1/").is_ok());
}

#[test]
fn html_conversion_keeps_content_and_drops_active_or_hidden_nodes() {
    let text = html_to_text(
        r#"<h1>Title &amp; more</h1><script>ignore()</script><p>Hello <a href="https://example.test/a">source</a>.</p><div hidden>secret</div><ul><li>one</li><li>two</li></ul>"#,
    )
    .unwrap();
    assert!(text.contains("# Title & more"), "{text:?}");
    assert!(
        text.contains("Hello source (https://example.test/a)."),
        "{text:?}"
    );
    assert!(text.contains("- one"), "{text:?}");
    assert!(!text.contains("ignore"), "script content leaked: {text:?}");
    assert!(!text.contains("secret"), "hidden content leaked: {text:?}");
}

#[test]
fn unsafe_html_errors_instead_of_returning_raw() {
    // 超过 MAX_HTML_DEPTH(512) 的嵌套：宁可 Err 也不把原始 HTML 交给模型。
    let html = "<div>".repeat(513) + "payload" + &"</div>".repeat(513);
    assert!(html_to_text(&html).is_err());
    // 未闭合注释同样不可安全转换。
    assert!(html_to_text("<!-- never closed <p>x</p>").is_err());
}

#[test]
fn fetch_parser_accepts_only_one_bounded_url() {
    assert_eq!(
        parse_url(&json!({"url":"https://example.test/a"})).unwrap(),
        "https://example.test/a"
    );
    for invalid in [
        json!({}),
        json!({"url":" "}),
        json!({"url":1}),
        json!({"url":"https://example.test","extra":true}),
        json!({"url":"https://example.test/\n"}),
    ] {
        assert!(parse_url(&invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn search_parser_bounds_query_list() {
    assert_eq!(
        parse_queries(&json!({"queries":["rust"]})).unwrap(),
        vec!["rust"]
    );
    for invalid in [
        json!({}),
        json!({"queries":[]}),
        json!({"queries":["a","b","c","d","e"]}),
        json!({"queries":[""]}),
        json!({"queries":["ok","  "]}),
        json!({"queries":"not-an-array"}),
        json!({"queries":["ok"],"extra":1}),
    ] {
        assert!(parse_queries(&invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn search_render_caps_sources_and_snippets_and_marks_untrusted() {
    let rows: Vec<serde_json::Value> = (0..10)
        .map(|i| json!({"title": format!("t{i}"), "url": format!("https://example.test/{i}"), "snippet": "x".repeat(900)}))
        .collect();
    let text = render_search(&json!({"results": rows}));
    assert!(text.starts_with("External web content follows"), "{text:?}");
    // 行数封顶 8，剩余显式告知；snippet 截到 400 字符。
    let listed = text.matches("https://example.test/").count();
    assert_eq!(listed, 8, "source cap broken: {text:?}");
    assert!(text.contains("(2 further sources omitted.)"), "{text:?}");
    let snippet_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("xxx")).collect();
    assert!(
        snippet_lines.iter().all(|l| l.chars().count() == 400),
        "snippet cap broken: {:?}",
        snippet_lines.first().map(|l| l.chars().count())
    );
    // 不可解析的响应体：显式告知，不静默空结果。
    let empty = render_search(&json!({"unexpected": true}));
    assert!(empty.contains("No parsable results"), "{empty:?}");
    assert!(empty.contains("untrusted data"), "{empty:?}");
}
