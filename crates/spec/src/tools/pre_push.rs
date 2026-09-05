//! pre-push hook — workspace + code checks.
//!
//! Port of `.githooks/hooks/pre-push`. Runs the topics listed in
//! `dispatch.yaml` under `pre-push`. Uses native Rust validators.

use crate::shared::{apply_global_overrides, exit_code, print_findings};
use crate::tools::{checklist, code, git, workspace};

/// `gate pre-push` — runs dispatched workspace + code topics.
pub fn run() -> i32 {
    let githooks_root =
        git::find_githooks_dir().unwrap_or_else(|| std::path::PathBuf::from(".githooks"));
    let spec_dir = githooks_root.join("spec");
    let dispatch_path = spec_dir.join("dispatch.yaml");
    let cfg = crate::shared::load_yaml(dispatch_path.to_str().unwrap_or("")).ok();

    let topics: Vec<String> = match &cfg {
        Some(c) => c
            .get("pre-push")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|t| t.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        None => vec!["workspace".into(), "code".into()],
    };

    let mut findings = Vec::new();

    for topic in &topics {
        let topic_findings = match topic.as_str() {
            "workspace" => workspace::run_workspace("."),
            "code" => code::run_code_all("."),
            "checklist" => checklist::run_all(checklist::HookScope::PrePush),
            other => {
                eprintln!("unknown pre-push topic: {}", other);
                vec![]
            }
        };
        findings.extend(topic_findings);
    }

    apply_global_overrides(&mut findings);
    print_findings(&findings);
    exit_code(&findings)
}
