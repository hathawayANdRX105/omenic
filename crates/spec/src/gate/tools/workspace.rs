//! Workspace validators: WS-01 tree hygiene and WS-02 file placement.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_yaml::Value as YamlValue;

use crate::gate::shared::{Finding, Severity, load_yaml};
use crate::gate::tools::git;

fn repo_root() -> PathBuf {
    git::git_root().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("."))
    })
}

fn spec(name: &str) -> YamlValue {
    let path = repo_root().join(".githooks/spec").join(name);
    load_yaml(path.to_str().unwrap_or("")).unwrap_or(YamlValue::Null)
}

fn strings(cfg: &YamlValue, key: &str, default: &[&str]) -> Vec<String> {
    cfg.get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| default.iter().map(|s| (*s).to_string()).collect())
}

fn rel_string(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn ignored(rel: &str, ignore: &[String]) -> bool {
    rel.split('/').any(|part| {
        ignore
            .iter()
            .map(|ig| ig.trim_matches('/'))
            .any(|ig| !ig.is_empty() && part == ig)
    })
}

fn visit_dirs(base: &Path, ignore: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    out.push(base.to_path_buf());
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let rel = rel_string(&path, base);
        if ignored(&rel, ignore) {
            continue;
        }
        visit_dirs(&path, ignore, out);
    }
}

fn visit_files(base: &Path, ignore: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = rel_string(&path, base);
        if ignored(&rel, ignore) {
            continue;
        }
        if path.is_dir() {
            visit_files(&path, ignore, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

pub fn run_tree_hygiene(target: &str) -> Vec<Finding> {
    let cfg = spec("workspace_tree_hygiene.yaml");
    let max_depth = cfg.get("max_depth").and_then(|v| v.as_i64()).unwrap_or(5) as usize;
    let ignore = strings(
        &cfg,
        "ignore_paths",
        &[".wt/", "node_modules/", "target/", "__pycache__/"],
    );
    let base = repo_root().join(target);
    let mut dirs = Vec::new();
    visit_dirs(&base, &ignore, &mut dirs);

    let mut findings = Vec::new();
    for dir in dirs {
        let rel = rel_string(&dir, &base);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut dir_count = 0usize;
        let mut files = Vec::new();
        for child in entries.flatten() {
            let Ok(file_type) = child.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                let name = child.file_name().to_string_lossy().to_string();
                if !ignored(&format!("{}/{}", rel, name), &ignore) {
                    dir_count += 1;
                }
            } else if file_type.is_file() {
                files.push(child.file_name().to_string_lossy().to_string());
            }
        }

        if dir_count == 0 && files.is_empty() {
            findings.push(Finding::new(
                "WS-01",
                Severity::Warn,
                &format!("empty directory: {}", rel),
            ));
            continue;
        }
        if dir_count == 0 && files.len() == 1 && files[0] != ".gitkeep" {
            findings.push(Finding::new(
                "WS-01",
                Severity::Warn,
                &format!("single-file dir (consider merging): {}/{}", rel, files[0]),
            ));
        }
        let depth = Path::new(&rel).components().count();
        if !rel.is_empty() && depth > max_depth {
            findings.push(Finding::new(
                "WS-01",
                Severity::Warn,
                &format!("deep nesting ({} > {}): {}", depth, max_depth, rel),
            ));
        }
    }

    for pattern in strings(&cfg, "orphan_patterns", &["tmp/", "__pycache__/"]) {
        let needle = pattern.trim_end_matches('/');
        for dir in find_dirs_named(&base, needle, &ignore) {
            let rel = rel_string(&dir, &base);
            findings.push(Finding::new(
                "WS-01",
                Severity::Warn,
                &format!("potential orphan/residue: {}", rel),
            ));
        }
    }

    if findings.is_empty() {
        findings.push(Finding::new("tree", Severity::Info, "tree hygiene OK"));
    }
    findings
}

fn find_dirs_named(base: &Path, name: &str, ignore: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let rel = rel_string(&path, base);
            if ignored(&rel, ignore) {
                continue;
            }
            if path.file_name().is_some_and(|n| n == name) {
                dirs.push(path.clone());
            }
            stack.push(path);
        }
    }
    dirs
}

pub fn run_file_placement(target: &str) -> Vec<Finding> {
    let cfg = spec("workspace_file_placement.yaml");
    let ignore = strings(
        &cfg,
        "ignore_paths",
        &[".wt/", ".githooks/", "node_modules/", "target/"],
    );
    let base = repo_root().join(target);
    let mut files = Vec::new();
    visit_files(&base, &ignore, &mut files);
    let mut findings = Vec::new();

    if let Some(rules) = cfg.get("forbidden_patterns").and_then(|v| v.as_sequence()) {
        for rule in rules {
            let pattern = rule
                .get("path_regex")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Ok(regex) = Regex::new(pattern) else {
                continue;
            };
            let reason = rule
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("file placement violation");
            let suggestion = rule
                .get("suggestion")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let severity = if rule.get("severity").and_then(|v| v.as_str()) == Some("FAIL") {
                Severity::Fail
            } else {
                Severity::Warn
            };
            for file in &files {
                let rel = rel_string(file, &base);
                if regex.is_match(&rel) {
                    let msg = if suggestion.is_empty() {
                        format!("{}: {}", reason, rel)
                    } else {
                        format!("{}: {} → {}", reason, rel, suggestion)
                    };
                    findings.push(Finding::new("WS-02", severity, &msg));
                }
            }
        }
    }

    if let Some(rules) = cfg.get("expected_locations").and_then(|v| v.as_sequence()) {
        for rule in rules {
            let pattern = rule
                .get("file_pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Ok(regex) = Regex::new(pattern) else {
                continue;
            };
            let expected = rule
                .get("expected_dir")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            for file in &files {
                let rel = rel_string(file, &base);
                if !regex.is_match(&rel) {
                    continue;
                }
                let parent = file.parent().unwrap_or(Path::new(""));
                let parent_rel = rel_string(parent, &base);
                if expected != "." && !parent_rel.contains(expected.trim_end_matches('/')) {
                    findings.push(Finding::new(
                        "WS-02",
                        Severity::Warn,
                        &format!("{} should be in {}, found in {}", rel, expected, parent_rel),
                    ));
                }
            }
        }
    }

    if findings.is_empty() {
        findings.push(Finding::new(
            "placement",
            Severity::Info,
            "file placement OK",
        ));
    }
    findings
}

pub fn run_workspace(target: &str) -> Vec<Finding> {
    let mut findings = run_tree_hygiene(target);
    findings.extend(run_file_placement(target));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_substrings_like_python() {
        let ignore = vec!["target/".to_string(), "node_modules/".to_string()];
        assert!(ignored("target/debug/foo", &ignore));
        assert!(ignored("web/node_modules/pkg", &ignore));
        assert!(!ignored("src/main.rs", &ignore));
    }

    #[test]
    fn ignore_matches_path_components_not_prefixes() {
        let ignore = vec![".git/".to_string(), "target/".to_string()];
        assert!(ignored(".git/config", &ignore));
        assert!(ignored("target/debug/foo", &ignore));
        assert!(ignored("crates/app/target/tmp", &ignore));
        assert!(!ignored(".github/workflows/daily_audit.yml", &ignore));
        assert!(!ignored("src/retargeted.rs", &ignore));
    }
}
