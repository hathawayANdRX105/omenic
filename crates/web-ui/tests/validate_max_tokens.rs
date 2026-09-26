//! Max Tokens input validation extracted from the SettingsModal form.

use web_ui::views::config::validate_max_tokens;

#[test]
fn valid_values_inside_bounds_pass() {
    assert_eq!(validate_max_tokens("4096"), None);
    assert_eq!(validate_max_tokens("  4096  "), None); // trimmed before parse
    assert_eq!(validate_max_tokens("16"), None); // inclusive lower bound
    assert_eq!(validate_max_tokens("200000"), None); // inclusive upper bound
}

#[test]
fn out_of_range_or_non_numeric_values_reject() {
    assert_eq!(validate_max_tokens("15"), Some("Max Tokens 至少 16"));
    assert_eq!(
        validate_max_tokens("200001"),
        Some("Max Tokens 超过上限 200,000")
    );
    assert_eq!(validate_max_tokens("abc"), Some("必须为有效正整数"));
    assert_eq!(validate_max_tokens(""), Some("必须为有效正整数"));
}
