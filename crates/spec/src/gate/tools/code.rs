//! Code lint dispatcher: CD-01..CD-06.

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value as YamlValue;

use crate::gate::shared::{Finding, Severity, load_yaml, run_external};
use crate::gate::tools::git;

const LANGUAGES: &[&str] = &["rust", "go", "javascript", "typescript", "python", "bash"];

fn repo_root() -> PathBuf {
    git::git_root().unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from("."))
    })
}

fn strings(cfg: &YamlValue, key: &str) -> Vec<String> {
    cfg.get(key)
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn rel_string(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_files(base: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn matches_include(rel: &str, include: &str) -> bool {
    let pat = include.strip_prefix("**/").unwrap_or(include);
    if let Some(prefix) = pat.strip_suffix("/*") {
        return rel
            .strip_prefix(&format!("{prefix}/"))
            .is_some_and(|rest| !rest.contains('/'))
            || rel.contains(&format!("/{prefix}/"));
    }
    if let Some(ext) = pat.strip_prefix("*.") {
        return rel.ends_with(&format!(".{ext}"));
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return rel.ends_with(suffix);
    }
    rel.ends_with(pat) || rel.contains(&format!("/{pat}"))
}

pub fn run_lang(lang: &str, target: &str) -> Vec<Finding> {
    let root = repo_root();
    let cfg_path = root
        .join(".githooks/spec")
        .join(format!("code_{lang}.yaml"));
    if !cfg_path.exists() {
        return vec![Finding::new(
            &format!("code-{lang}"),
            Severity::Warn,
            &format!("config not found: code_{lang}.yaml"),
        )];
    }
    let cfg = load_yaml(cfg_path.to_str().unwrap_or("")).unwrap_or(YamlValue::Null);
    if !cfg.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true) {
        return vec![Finding::new(
            &format!("code-{lang}"),
            Severity::Info,
            &format!("{lang}: disabled in config"),
        )];
    }

    let command = cfg.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if command.is_empty() {
        return vec![Finding::new(
            &format!("code-{lang}"),
            Severity::Warn,
            &format!("{lang}: no command configured"),
        )];
    }
    let args = strings(&cfg, "args");
    let includes = strings(&cfg, "paths_include");
    let excludes = strings(&cfg, "paths_exclude");
    let fail_severity = if cfg.get("fail_severity").and_then(|v| v.as_str()) == Some("FAIL") {
        Severity::Fail
    } else {
        Severity::Warn
    };

    if !includes.is_empty() {
        let base = root.join(target);
        let mut files = Vec::new();
        collect_files(&base, &mut files);
        let found = files.iter().any(|file| {
            let rel = rel_string(file, &base);
            !excludes.iter().any(|x| rel.contains(x))
                && includes.iter().any(|pat| matches_include(&rel, pat))
        });
        if !found {
            return vec![Finding::new(
                &format!("code-{lang}"),
                Severity::Info,
                &format!("{lang}: no matching files (paths_include: {includes:?})"),
            )];
        }
    }

    let mut cmd: Vec<&str> = Vec::with_capacity(args.len() + 2);
    cmd.push(command);
    cmd.extend(args.iter().map(|s| s.as_str()));
    if command != "cargo" {
        cmd.push(target);
    }

    let (mut rc, mut output) = match run_external(&cmd, Some(root.to_string_lossy().as_ref())) {
        Ok(result) => result,
        Err(_) => {
            return vec![Finding::new(
                &format!("code-{lang}"),
                Severity::Warn,
                &format!("{lang}: {command} not installed, skipped"),
            )];
        }
    };

    if rc != 0 && !excludes.is_empty() && !output.is_empty() {
        let kept: Vec<&str> = output
            .lines()
            .filter(|line| !excludes.iter().any(|x| line.contains(x)))
            .collect();
        if kept.is_empty() {
            rc = 0;
            output.clear();
        } else {
            output = kept.join("\n");
        }
    }

    if rc == 0 {
        return vec![Finding::new(
            &format!("code-{lang}"),
            Severity::Info,
            &format!("{lang}: {command} passed"),
        )];
    }
    if rc == 127
        || output.to_lowercase().starts_with("command not found")
        || output.to_lowercase().contains("no such file or directory")
    {
        return vec![Finding::new(
            &format!("code-{lang}"),
            Severity::Warn,
            &format!("{lang}: {command} not installed, skipped"),
        )];
    }
    let msg = if output.is_empty() {
        format!("{command} exited {rc}")
    } else if output.len() > 500 {
        output.chars().take(500).collect()
    } else {
        output
    };
    vec![Finding::new(
        &format!("code-{lang}"),
        fail_severity,
        &format!("{lang}: {command} reported issues:\n{msg}"),
    )]
}

pub fn run_code(langs: Option<&[String]>, target: &str) -> Vec<Finding> {
    let languages: Vec<&str> = match langs {
        Some(langs) => langs.iter().map(|s| s.as_str()).collect(),
        None => LANGUAGES.to_vec(),
    };
    let mut findings = Vec::new();
    for lang in languages {
        findings.extend(run_lang(lang, target));
    }
    findings
}

pub fn run_code_all(target: &str) -> Vec<Finding> {
    run_code(None, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_match_is_close_to_python_path_match() {
        assert!(matches_include("src/main.rs", "**/*.rs"));
        assert!(matches_include("bin/tool", "**/bin/*"));
        assert!(!matches_include("src/main.rs", "**/*.py"));
    }
}
