//! CL-03: documentation hygiene (fullwidth brackets / broken links /
//! stale markers / empty files / CRLF / trailing whitespace).
//!
//! Config-driven port of `.githooks/cleanup/docs_hygiene.py`.
//! Config: `spec/cleanup_docs_hygiene.yaml`.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_yaml::Value as YamlValue;

use crate::shared::{Finding, Severity, load_yaml};

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
    crate::tools::git::git_root()
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
