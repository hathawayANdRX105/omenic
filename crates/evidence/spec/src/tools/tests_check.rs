//! CL-02: test code checks (naming / assertions / required helpers).
//!
//! Config-driven port of `.githooks/cleanup/tests_check.py`.
//! One config per language: `spec/cleanup_tests_<lang>.yaml`.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::shared::{Finding, Severity, load_yaml};

fn repo_root() -> PathBuf {
    crate::tools::git::git_root()
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
