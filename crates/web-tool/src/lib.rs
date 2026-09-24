pub mod fetch;
pub mod parse;

pub use fetch::{UrlPolicyError, WebFetchTool, ensure_public_url, parse_url};
pub use parse::{WebSearchTool, html_to_text, parse_queries, render_search};

pub(crate) const MAX_HTML_DEPTH: usize = 512;
pub(crate) const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
