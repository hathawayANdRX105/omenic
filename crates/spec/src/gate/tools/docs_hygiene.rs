//! CL-03: documentation hygiene (fullwidth brackets / broken links /
//! stale markers / empty files / CRLF / trailing whitespace).
//!
//! Config-driven port of `.githooks/cleanup/docs_hygiene.py`.
//! Config: `spec/cleanup_docs_hygiene.yaml`.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_yaml::Value as YamlValue;

use crate::gate::shared::{Finding, Severity, load_yaml};

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

fn bool_or(cfg: &YamlValue, key: &str, default: bool) -> bool {
    cfg.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn usize_or(cfg: &YamlValue, key: &str, default: usize) -> usize {
    cfg.get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(default)
}

fn rel_string(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn ignored(rel: &str, ignore: &[String]) -> bool {
    ignore.iter().any(|ig| rel.contains(ig))
}

fn visit_files(
    root: &Path,
    base: &Path,
    extensions: &[String],
    ignore: &[String],
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = rel_string(&path, root);
        if ignored(&rel, ignore) {
            continue;
        }
        if path.is_dir() {
            // skip hidden directories
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            visit_files(root, &path, extensions, ignore, out);
        } else if path.is_file()
            && let Some(ext) = path.extension().and_then(|e| e.to_str())
            && extensions.iter().any(|e| e == &format!(".{}", ext))
        {
            out.push(path);
        }
    }
}

fn check_file(path: &Path, base: &Path, cfg: &YamlValue, findings: &mut Vec<Finding>) {
    let rel = rel_string(path, base);
    // Single read: bytes once, lossy UTF-8 view (Python: read_text errors="replace").
    let raw = fs::read(path).unwrap_or_default();
    let content = String::from_utf8_lossy(&raw);
    let lines: Vec<&str> = content.lines().collect();

    // fullwidth brackets
    if bool_or(cfg, "check_fullwidth", true) {
        let forbidden = strings(
            cfg,
            "forbidden_brackets",
            &[
                "（", "）", "「", "」", "【", "】", "『", "』", "《", "》", "〈", "〉", "﹁", "﹂",
            ],
        );
        let forbidden_chars: Vec<char> = forbidden.iter().flat_map(|s| s.chars()).collect();
        for (i, line) in lines.iter().enumerate() {
            let fb: Vec<char> = line
                .chars()
                .filter(|c| forbidden_chars.contains(c))
                .collect();
            if !fb.is_empty() {
                let unique: std::collections::HashSet<_> = fb.into_iter().collect();
                findings.push(Finding::new(
                    "CL-03",
                    Severity::Warn,
                    format!("{}:{}: fullwidth brackets: {:?}", rel, i + 1, unique).as_str(),
                ));
                break; // only first violation line
            }
        }
    }

    // broken relative links: ](./xxx.md|txt|rst)
    if bool_or(cfg, "broken_link_check", true) {
        let re = Regex::new(r#"\]\((\./[^)]+\.(?:md|txt|rst))\)"#).unwrap();
        for caps in re.captures_iter(&content) {
            if let Some(link) = caps.get(1) {
                let link_str = link.as_str();
                let target = path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(link_str)
                    .clean();
                if !target.exists() {
                    findings.push(Finding::new(
                        "CL-03",
                        Severity::Warn,
                        format!("{}: broken link → {}", rel, link_str).as_str(),
                    ));
                }
            }
        }
    }

    // stale markers
    let stale_keywords = strings(cfg, "stale_marker_keywords", &["TODO", "FIXME", "XXX"]);
    for kw in stale_keywords {
        if content.contains(&kw) {
            findings.push(Finding::new(
                "CL-03",
                Severity::Warn,
                format!("{}: contains stale marker '{}'", rel, kw).as_str(),
            ));
        }
    }

    // min content lines
    let min_lines = usize_or(cfg, "min_content_lines", 3);
    let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();
    if non_empty < min_lines {
        findings.push(Finding::new(
            "CL-03",
            Severity::Warn,
            format!(
                "{}: only {} non-empty line(s), min {}",
                rel, non_empty, min_lines
            )
            .as_str(),
        ));
    }

    // trailing whitespace (only first)
    for (i, line) in lines.iter().enumerate() {
        if line.ends_with(' ') || line.ends_with('\t') {
            findings.push(Finding::new(
                "CL-03",
                Severity::Info,
                format!("{}:{}: trailing whitespace", rel, i + 1).as_str(),
            ));
            break;
        }
    }

    // CRLF line endings
    if raw.windows(2).any(|w| w == b"\r\n") || raw.contains(&b'\r') {
        findings.push(Finding::new(
            "CL-03",
            Severity::Warn,
            format!("{}: contains CRLF line endings", rel).as_str(),
        ));
    }

    // missing trailing newline
    if !content.is_empty() && !content.ends_with('\n') {
        findings.push(Finding::new(
            "CL-03",
            Severity::Info,
            format!("{}: missing trailing newline", rel).as_str(),
        ));
    }

    // clean file
    if findings
        .iter()
        .filter(|f| f.rule_id == "CL-03" && f.msg.contains(&rel))
        .count()
        == 0
    {
        findings.push(Finding::new(
            "docs",
            Severity::Info,
            format!("{}: clean", rel).as_str(),
        ));
    }
}

/// Run CL-03 checks from the repo root.
pub fn run() -> Vec<Finding> {
    run_in(&repo_root())
}

/// Testable core: run CL-03 against `base` (config in `base/.githooks/spec/`).
pub fn run_in(base: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    let config_path = base.join(".githooks/spec/cleanup_docs_hygiene.yaml");
    let cfg = match load_yaml(config_path.to_str().unwrap_or("")) {
        Ok(v) => v,
        Err(_) => {
            findings.push(Finding::new(
                "CL-03",
                Severity::Warn,
                "config not found: cleanup_docs_hygiene.yaml",
            ));
            return findings;
        }
    };

    let extensions = strings(&cfg, "file_extensions", &[".md", ".txt", ".rst"]);
    let ignore = strings(&cfg, "ignore_paths", &[".wt/", "node_modules/", "target/"]);

    let mut files = Vec::new();
    visit_files(base, base, &extensions, &ignore, &mut files);

    if files.is_empty() {
        findings.push(Finding::new(
            "docs",
            Severity::Info,
            "no documentation files found",
        ));
        return findings;
    }

    for file in files {
        check_file(&file, base, &cfg, &mut findings);
    }

    findings
}

fn repo_root() -> PathBuf {
    crate::gate::tools::git::git_root()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

trait PathClean {
    fn clean(&self) -> PathBuf;
}

impl PathClean for PathBuf {
    fn clean(&self) -> PathBuf {
        let mut components = Vec::new();
        for comp in self.components() {
            match comp {
                std::path::Component::ParentDir => {
                    if !components.is_empty() {
                        components.pop();
                    }
                }
                std::path::Component::CurDir => {}
                _ => components.push(comp),
            }
        }
        components.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn make_config(dir: &Path) {
        let spec_dir = dir.join(".githooks/spec");
        fs::create_dir_all(&spec_dir).unwrap();
        let yaml = r#"
file_extensions: [".md", ".txt", ".rst"]
ignore_paths: [".wt/", "node_modules/", "target/"]
forbidden_brackets: ["（", "）", "「", "」", "【", "】", "『", "』", "《", "》", "〈", "〉", "﹁", "﹂"]
check_fullwidth: true
broken_link_check: true
stale_marker_keywords: ["TODO", "FIXME", "XXX"]
min_content_lines: 3
"#;
        fs::write(spec_dir.join("cleanup_docs_hygiene.yaml"), yaml).unwrap();
    }

    #[test]
    fn test_clean_md() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(dir.path(), "README.md", "Line 1\nLine 2\nLine 3\n");
        let findings = run_in(dir.path());
        let clean = findings
            .iter()
            .find(|f| f.rule_id == "docs" && f.msg.contains("clean"));
        assert!(
            clean.is_some(),
            "expected clean finding, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_fullwidth_brackets() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(
            dir.path(),
            "README.md",
            "Line 1\nLine （bad）\nLine 3\nLine 4\n",
        );
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("fullwidth"));
        assert!(
            warn.is_some(),
            "expected fullwidth warning, got: {:?}",
            findings
        );
        // only first violation line reported
        let count = findings
            .iter()
            .filter(|f| f.rule_id == "CL-03" && f.msg.contains("fullwidth"))
            .count();
        assert_eq!(count, 1, "should only report first fullwidth line");
    }

    #[test]
    fn test_broken_relative_link() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(
            dir.path(),
            "README.md",
            "Line 1\n[link](./missing.md)\nLine 3\nLine 4\n",
        );
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("broken link"));
        assert!(
            warn.is_some(),
            "expected broken link warning, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_existing_relative_link_ok() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(
            dir.path(),
            "README.md",
            "Line 1\n[link](./exists.md)\nLine 3\nLine 4\n",
        );
        write_file(dir.path(), "exists.md", "exists\ncontent\nmore\n");
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("broken link"));
        assert!(
            warn.is_none(),
            "existing link should not warn, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_stale_marker_todo() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(
            dir.path(),
            "README.md",
            "Line 1\nTODO: fix this\nLine 3\nLine 4\n",
        );
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("TODO"));
        assert!(warn.is_some(), "expected TODO warning, got: {:?}", findings);
    }

    #[test]
    fn test_min_content_lines() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(dir.path(), "README.md", "Line 1\n\n\n");
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("non-empty line"));
        assert!(
            warn.is_some(),
            "expected min lines warning, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_crlf_line_endings() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        let path = dir.path().join("README.md");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"Line 1\r\nLine 2\r\nLine 3\r\n").unwrap();
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("CRLF"));
        assert!(warn.is_some(), "expected CRLF warning, got: {:?}", findings);
    }

    #[test]
    fn test_trailing_whitespace() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        write_file(dir.path(), "README.md", "Line 1 \nLine 2\nLine 3\nLine 4\n");
        let findings = run_in(dir.path());
        let info = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("trailing whitespace"));
        assert!(
            info.is_some(),
            "expected trailing whitespace info, got: {:?}",
            findings
        );
        let count = findings
            .iter()
            .filter(|f| f.rule_id == "CL-03" && f.msg.contains("trailing whitespace"))
            .count();
        assert_eq!(count, 1, "should only report first trailing whitespace");
    }

    #[test]
    fn test_missing_trailing_newline() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        let path = dir.path().join("README.md");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"Line 1\nLine 2\nLine 3").unwrap(); // no trailing \n
        let findings = run_in(dir.path());
        let info = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("missing trailing newline"));
        assert!(
            info.is_some(),
            "expected missing newline info, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_ignore_substring_match() {
        let dir = tempdir().unwrap();
        make_config(dir.path());
        // file inside .wt/ should be ignored
        write_file(dir.path(), ".wt/ignored.md", "Line 1\nLine 2\nLine 3\n");
        // file inside node_modules/ should be ignored
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        write_file(
            dir.path(),
            "node_modules/pkg/file.md",
            "Line 1\nLine 2\nLine 3\n",
        );
        // file inside target/ should be ignored
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        write_file(
            dir.path(),
            "target/debug/file.md",
            "Line 1\nLine 2\nLine 3\n",
        );
        // normal file should be checked
        write_file(dir.path(), "README.md", "Line 1\nLine 2\nLine 3\n");
        let findings = run_in(dir.path());
        // should only find the clean README.md
        let clean_count = findings
            .iter()
            .filter(|f| f.rule_id == "docs" && f.msg.contains("clean"))
            .count();
        assert_eq!(
            clean_count, 1,
            "expected exactly one clean finding, got: {:?}",
            findings
        );
    }

    #[test]
    fn test_config_not_found_warn() {
        let dir = tempdir().unwrap();
        // no config file
        write_file(dir.path(), "README.md", "Line 1\nLine 2\nLine 3\n");
        let findings = run_in(dir.path());
        let warn = findings
            .iter()
            .find(|f| f.rule_id == "CL-03" && f.msg.contains("config not found"));
        assert!(
            warn.is_some(),
            "expected config not found warning, got: {:?}",
            findings
        );
    }
}
