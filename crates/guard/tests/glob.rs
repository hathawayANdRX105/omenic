//! Glob matcher behavior for guard tool-name patterns.

use guard::glob_match;

#[test]
fn star_matches_everything() {
    assert!(glob_match("*", "anything"));
}

#[test]
fn prefix_and_suffix_patterns() {
    assert!(glob_match("mcp_*", "mcp_fetch"));
    assert!(!glob_match("mcp_*", "web_fetch"));
    assert!(glob_match("*_fetch", "web_fetch"));
    assert!(!glob_match("*_fetch", "web_search"));
}

#[test]
fn middle_star_and_exact() {
    assert!(glob_match("web_*_tool", "web_fetch_tool"));
    // `*` spans any run (only `/` is special in the reference dialect, and
    // tool names never contain `/`), so a two-segment middle also matches.
    assert!(glob_match("web_*_tool", "web_fetch_search_tool"));
    assert!(glob_match("todo_write", "todo_write"));
    assert!(!glob_match("todo_write", "todo_wrote"));
}
