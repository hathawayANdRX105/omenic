//! CL-02: test code checks (naming / assertions / required helpers).
//!
//! Config-driven port of `.githooks/cleanup/tests_check.py`.
//! One config per language: `spec/cleanup_tests_<lang>.yaml`.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::gate::shared::{Finding, Severity, load_yaml};

fn repo_root() -> PathBuf {
    crate::gate::tools::git::git_root()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Run CL-02 checks for all languages from the repo root.
pub fn run() -> Vec<Finding> {
    run_in(&repo_root())
}

/// Testable core: run CL-02 against `base` (configs in `base/.githooks/spec/`).
fn run_in(base: &Path) -> Vec<Finding> {
    let mut all_findings = Vec::new();

    let languages = ["rust", "go", "javascript", "bash"];

    // Single repo walk shared by all languages (was one walk per language).
    let mut repo_files = Vec::new();
    collect_all_files(base, &mut repo_files);

    for lang in languages {
        let config_path = base
            .join(".githooks/spec")
            .join(format!("cleanup_tests_{}.yaml", lang));
        let cfg = match load_yaml(config_path.to_str().unwrap_or("")) {
            Ok(v) => v,
            Err(_) => {
                all_findings.push(Finding::new(
                    "CL-02",
                    Severity::Warn,
                    &format!(
                        "config not found: {}",
                        config_path.file_name().unwrap().to_string_lossy()
                    ),
                ));
                continue;
            }
        };

        if !cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true) {
            all_findings.push(Finding::new(
                "CL-02",
                Severity::Info,
                &format!("{}: disabled in config", lang),
            ));
            continue;
        }

        let includes: Vec<String> = cfg
            .get("paths_include")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let excludes: Vec<String> = cfg
            .get("paths_exclude")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        if includes.is_empty() {
            all_findings.push(Finding::new(
                "CL-02",
                Severity::Info,
                &format!("{}: no test files found matching {:?}", lang, includes),
            ));
            continue;
        }

        let include_regexes: Vec<Regex> = includes
            .iter()
            .filter_map(|pat| glob_to_regex(pat).ok())
            .collect();
        let files: Vec<&PathBuf> = repo_files
            .iter()
            .filter(|file| {
                let rel = file
                    .strip_prefix(base)
                    .unwrap_or(file)
                    .to_string_lossy()
                    .replace('\\', "/");
                !is_ignored(&rel, &excludes) && include_regexes.iter().any(|re| re.is_match(&rel))
            })
            .collect();

        if files.is_empty() {
            all_findings.push(Finding::new(
                "CL-02",
                Severity::Info,
                &format!("{}: no test files found matching {:?}", lang, includes),
            ));
            continue;
        }

        let naming_pattern = cfg
            .get("naming_pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let min_asserts = cfg
            .get("min_assertions_per_test")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as usize;
        let assert_patterns: Vec<String> = cfg
            .get("assert_patterns")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let required_helpers: Vec<String> = cfg
            .get("required_helpers")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Compile config regexes once per language (was per file).
        let naming_re = if naming_pattern.is_empty() {
            None
        } else {
            match Regex::new(&naming_pattern) {
                Ok(re) => Some(re),
                Err(_) => {
                    all_findings.push(Finding::new(
                        "CL-02",
                        Severity::Warn,
                        &format!("{}: invalid regex in config", lang),
                    ));
                    None
                }
            }
        };
        let mut assert_res: Vec<Regex> = Vec::new();
        let mut bad_assert = false;
        for pat in &assert_patterns {
            match Regex::new(pat) {
                Ok(re) => assert_res.push(re),
                Err(_) => bad_assert = true,
            }
        }
        if bad_assert {
            all_findings.push(Finding::new(
                "CL-02",
                Severity::Warn,
                &format!("{}: invalid regex in config", lang),
            ));
        }

        for file in files {
            all_findings.extend(check_file(
                file,
                lang,
                naming_re.as_ref(),
                min_asserts,
                &assert_res,
                &required_helpers,
            ));
        }
    }

    all_findings
}

fn collect_all_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            collect_all_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn is_ignored(rel: &str, excludes: &[String]) -> bool {
    rel.split('/').any(|part| {
        excludes.iter().any(|ex| {
            let ex_clean = ex.trim_matches('/');
            !ex_clean.is_empty() && part == ex_clean
        })
    })
}

fn glob_to_regex(pat: &str) -> Result<Regex, regex::Error> {
    let mut regex_str = String::with_capacity(pat.len() * 2);
    regex_str.push('^');

    let parts: Vec<&str> = pat.split('/').collect();
    let mut prev_was_double_star = false;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 && !prev_was_double_star {
            regex_str.push('/');
        }
        if *part == "**" {
            regex_str.push_str("(?:[^/]+/)*");
            prev_was_double_star = true;
        } else {
            prev_was_double_star = false;
            let mut escaped = String::new();
            for ch in part.chars() {
                match ch {
                    '*' => escaped.push_str("[^/]*"),
                    '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
                    | '\\' => {
                        escaped.push('\\');
                        escaped.push(ch);
                    }
                    _ => escaped.push(ch),
                }
            }
            regex_str.push_str(&escaped);
        }
    }
    regex_str.push('$');
    Regex::new(&regex_str)
}

fn check_file(
    path: &Path,
    _lang: &str,
    naming_re: Option<&Regex>,
    min_asserts: usize,
    assert_res: &[Regex],
    required_helpers: &[String],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let file_name = path.file_name().unwrap().to_string_lossy().to_string();

    let content = std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
        .unwrap_or_default();

    // Naming pattern check
    if let Some(re) = naming_re
        && !re.is_match(&content)
    {
        findings.push(Finding::new(
            "CL-02",
            Severity::Warn,
            &format!(
                "{}: no functions matching naming pattern '{}'",
                file_name,
                re.as_str()
            ),
        ));
    }

    // Assertion count check
    if !assert_res.is_empty() {
        let assert_count: usize = assert_res
            .iter()
            .map(|re| re.find_iter(&content).count())
            .sum();
        if assert_count < min_asserts {
            findings.push(Finding::new(
                "CL-02",
                Severity::Warn,
                &format!(
                    "{}: only {} assertion(s), minimum {}",
                    file_name, assert_count, min_asserts
                ),
            ));
        }
    }

    // Required helpers check
    for helper in required_helpers {
        if !helper.is_empty() && !content.contains(helper) {
            findings.push(Finding::new(
                "CL-02",
                Severity::Warn,
                &format!("{}: missing required helper '{}'", file_name, helper),
            ));
        }
    }

    if findings.is_empty() {
        findings.push(Finding::new(
            "CL-02",
            Severity::Info,
            &format!("{}: checks passed", file_name),
        ));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_test_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn make_all_configs(
        base: &Path,
        rust_enabled: bool,
        go_enabled: bool,
        js_enabled: bool,
        bash_enabled: bool,
    ) {
        let config_dir = base.join(".githooks/spec");
        fs::create_dir_all(&config_dir).unwrap();
        if rust_enabled {
            let config = make_config_yaml(
                "rust",
                true,
                "fn (test_|it_)",
                1,
                &["assert!", "assert_eq!", "assert_ne!"],
                &[],
                &["**/tests/*.rs", "**/*_test.rs", "**/test_*.rs"],
                &["target/", ".wt/"],
            );
            fs::write(config_dir.join("cleanup_tests_rust.yaml"), config).unwrap();
        } else {
            fs::write(
                config_dir.join("cleanup_tests_rust.yaml"),
                "enabled: false\n",
            )
            .unwrap();
        }
        if go_enabled {
            let config = make_config_yaml(
                "go",
                true,
                "func (Test[A-Z])",
                1,
                &["if .+ != .+", "require\\.", "assert\\."],
                &[],
                &["**/*_test.go"],
                &["vendor/", ".wt/"],
            );
            fs::write(config_dir.join("cleanup_tests_go.yaml"), config).unwrap();
        } else {
            fs::write(config_dir.join("cleanup_tests_go.yaml"), "enabled: false\n").unwrap();
        }
        if js_enabled {
            let config = make_config_yaml(
                "javascript",
                true,
                "(it|test)\\(",
                1,
                &["expect\\(", "assert\\."],
                &["afterEach", "describe"],
                &[
                    "**/*.test.js",
                    "**/*.spec.js",
                    "**/*.test.ts",
                    "**/*.spec.ts",
                ],
                &["node_modules/", "dist/", ".wt/"],
            );
            fs::write(config_dir.join("cleanup_tests_javascript.yaml"), config).unwrap();
        } else {
            fs::write(
                config_dir.join("cleanup_tests_javascript.yaml"),
                "enabled: false\n",
            )
            .unwrap();
        }
        if bash_enabled {
            let config = make_config_yaml(
                "bash",
                true,
                "@test",
                1,
                &["\\[ ", "assert "],
                &[],
                &["**/*.bats", "**/test_*.sh"],
                &[".wt/"],
            );
            fs::write(config_dir.join("cleanup_tests_bash.yaml"), config).unwrap();
        } else {
            fs::write(
                config_dir.join("cleanup_tests_bash.yaml"),
                "enabled: false\n",
            )
            .unwrap();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn make_config_yaml(
        _lang: &str,
        enabled: bool,
        naming: &str,
        min_asserts: u64,
        asserts: &[&str],
        helpers: &[&str],
        includes: &[&str],
        excludes: &[&str],
    ) -> String {
        let mut yaml = format!("enabled: {}\n", if enabled { "true" } else { "false" });
        if !naming.is_empty() {
            yaml.push_str(&format!(
                "naming_pattern: '{}'\n",
                naming.replace('\'', "\\'")
            ));
        }
        yaml.push_str(&format!("min_assertions_per_test: {}\n", min_asserts));
        if !asserts.is_empty() {
            yaml.push_str("assert_patterns:\n");
            for a in asserts {
                yaml.push_str(&format!("  - '{}'\n", a.replace('\'', "\\'")));
            }
        }
        if !helpers.is_empty() {
            yaml.push_str("required_helpers:\n");
            for h in helpers {
                yaml.push_str(&format!("  - '{}'\n", h.replace('\'', "\\'")));
            }
        }
        yaml.push_str("paths_include:\n");
        for inc in includes {
            yaml.push_str(&format!("  - \"{}\"\n", inc));
        }
        yaml.push_str("paths_exclude:\n");
        for exc in excludes {
            yaml.push_str(&format!("  - \"{}\"\n", exc));
        }
        yaml
    }

    #[test]
    fn test_rust_checks_passed() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        make_all_configs(base, true, false, false, false);

        write_test_file(
            base,
            "tests/my_test.rs",
            r#"
fn test_something() {
    assert!(true);
}
"#,
        );

        let findings = run_in(base);
        let warn_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .collect();
        assert_eq!(
            warn_findings.len(),
            0,
            "expected no warnings, got: {:?}",
            warn_findings
        );
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Info && f.msg.contains("checks passed")),
            "expected checks passed, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_naming_warn() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        make_all_configs(base, true, false, false, false);

        write_test_file(
            base,
            "tests/my_test.rs",
            r#"
fn something_else() {
    assert!(true);
}
"#,
        );

        let findings = run_in(base);
        let warn_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .collect();
        assert!(
            warn_findings
                .iter()
                .any(|f| f.msg.contains("no functions matching naming pattern")),
            "expected naming warning, got: {:?}",
            warn_findings
        );
    }

    #[test]
    fn test_assertion_count_warn() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        // All disabled, then override rust with min_asserts=2
        make_all_configs(base, false, false, false, false);
        let config_dir = base.join(".githooks/spec");
        let config = make_config_yaml(
            "rust",
            true,
            "fn (test_|it_)",
            2,
            &["assert!", "assert_eq!", "assert_ne!"],
            &[],
            &["**/tests/*.rs", "**/*_test.rs", "**/test_*.rs"],
            &["target/", ".wt/"],
        );
        fs::write(config_dir.join("cleanup_tests_rust.yaml"), config).unwrap();

        write_test_file(
            base,
            "tests/my_test.rs",
            r#"
fn test_something() {
    assert!(true);
}
"#,
        );

        let findings = run_in(base);
        let warn_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .collect();
        assert!(
            warn_findings
                .iter()
                .any(|f| f.msg.contains("only 1 assertion")),
            "expected assertion count warning, got: {:?}",
            warn_findings
        );
    }

    #[test]
    fn test_required_helper_warn() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        make_all_configs(base, false, false, true, false);

        write_test_file(
            base,
            "test/my.test.js",
            r#"
describe('suite', () => {
    it('test', () => {
        expect(true).toBe(true);
    });
});
"#,
        );

        let findings = run_in(base);
        let warn_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .collect();
        assert!(
            warn_findings
                .iter()
                .any(|f| f.msg.contains("missing required helper 'afterEach'")),
            "expected required helper warning, got: {:?}",
            warn_findings
        );
    }

    #[test]
    fn test_enabled_false() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        make_all_configs(base, false, false, false, false);

        let findings = run_in(base);
        let info_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .collect();
        assert!(
            info_findings
                .iter()
                .any(|f| f.msg.contains("rust: disabled in config")),
            "expected disabled info, got: {:?}",
            info_findings
        );
    }

    #[test]
    fn test_config_not_found() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // No config files written at all
        let findings = run_in(base);
        let warn_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .collect();
        assert_eq!(
            warn_findings.len(),
            4,
            "expected 4 config not found warnings, got: {:?}",
            warn_findings
        );
        for f in warn_findings {
            assert!(f.msg.contains("config not found"));
        }
    }

    #[test]
    fn test_no_matching_files() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        make_all_configs(base, true, false, false, false);

        // No test files created
        let findings = run_in(base);
        let info_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .collect();
        assert!(
            info_findings
                .iter()
                .any(|f| f.msg.contains("no test files found matching")),
            "expected no test files info, got: {:?}",
            info_findings
        );
    }

    #[test]
    fn test_glob_to_regex() {
        let re = glob_to_regex("**/tests/*.rs").unwrap();
        assert!(re.is_match("tests/foo.rs"));
        assert!(re.is_match("src/tests/foo.rs"));
        assert!(re.is_match("a/b/c/tests/foo.rs"));
        assert!(!re.is_match("tests/foo.txt"));

        let re = glob_to_regex("**/*_test.rs").unwrap();
        assert!(re.is_match("foo_test.rs"));
        assert!(re.is_match("a/b/foo_test.rs"));
        assert!(!re.is_match("foo.rs"));

        let re = glob_to_regex("**/test_*.rs").unwrap();
        assert!(re.is_match("test_foo.rs"));
        assert!(re.is_match("a/b/test_foo.rs"));
        assert!(!re.is_match("foo_test.rs"));

        let re = glob_to_regex("**/*.bats").unwrap();
        assert!(re.is_match("test.bats"));
        assert!(re.is_match("a/b/test.bats"));

        let re = glob_to_regex("**/test_*.sh").unwrap();
        assert!(re.is_match("test_foo.sh"));
        assert!(re.is_match("a/b/test_foo.sh"));
    }

    #[test]
    fn test_is_ignored() {
        let excludes = vec![
            "target/".to_string(),
            ".wt/".to_string(),
            "vendor/".to_string(),
        ];
        assert!(is_ignored("target/foo.rs", &excludes));
        assert!(is_ignored("src/target/foo.rs", &excludes));
        assert!(is_ignored(".wt/foo.rs", &excludes));
        assert!(is_ignored("vendor/foo.rs", &excludes));
        assert!(!is_ignored("src/foo.rs", &excludes));
        assert!(!is_ignored("tests/foo.rs", &excludes));
    }
}
