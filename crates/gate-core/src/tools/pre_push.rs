//! pre-push hook — workspace + code checks.
//!
//! Port of `.githooks/hooks/pre-push`. Runs the topics listed in
//! `dispatch.yaml` under `pre-push`. Delegates to the Python scripts.

use crate::shared::{exit_code, print_findings};
use crate::tools::git;

// Re-use the Python delegation from pre_commit
use super::pre_commit::run_python_topic;

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
        eprintln!("--- {} ---", topic);
        let topic_findings = match topic.as_str() {
            "workspace" => {
                let mut f = run_python_topic("workspace/tree_hygiene.py");
                f.extend(run_python_topic("workspace/file_placement.py"));
                f
            }
            "code" => run_python_topic("code/lint.py"),
            other => {
                eprintln!("unknown pre-push topic: {}", other);
                vec![]
            }
        };
        findings.extend(topic_findings);
    }

    print_findings(&findings);
    exit_code(&findings)
}
