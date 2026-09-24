//! Shared minimal glob matching (`*` wildcard) for guard tool-name patterns.
//!
//! ponytail: one small matcher instead of pulling a glob crate — the specs
//! only need `*`-anywhere patterns (`mcp_*`, `web_*`).

/// Match `pattern` against `text`; `*` matches any (possibly empty) run.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut idx = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text.starts_with(part) {
                return false;
            }
            idx = part.len();
        } else if i == parts.len() - 1 {
            if !text[idx..].ends_with(part) {
                return false;
            }
        } else if let Some(pos) = text[idx..].find(part) {
            idx += pos + part.len();
        } else {
            return false;
        }
    }
    true
}
