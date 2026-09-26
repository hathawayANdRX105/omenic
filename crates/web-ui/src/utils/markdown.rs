//! Markdown 渲染：CommonMark → HTML → ammonia 白名单清洗。

use pulldown_cmark::{Options as MarkdownOptions, Parser, html};

/// Markdown → HTML。CommonMark 会透传原始 HTML（经 `dangerous_inner_html`
/// 注入 DOM，是真实 XSS 面），所以生成后必须过 ammonia 白名单清洗：
/// 剥离 script/事件属性/javascript: URL，只留安全标签。
/// `pub` 供 `tests/markdown_sanitize.rs` 集成测试直接断言。
pub fn markdown_to_html(input: &str) -> String {
    let mut opts = MarkdownOptions::empty();
    opts.insert(MarkdownOptions::ENABLE_STRIKETHROUGH);
    opts.insert(MarkdownOptions::ENABLE_TASKLISTS);
    opts.insert(MarkdownOptions::ENABLE_TABLES);
    let parser = Parser::new_ext(input, opts);
    let mut output = String::new();
    html::push_html(&mut output, parser);
    ammonia::clean(&output)
}
