//! `subagent run` handler, split out of `cli.rs`.

use tools::Tool;

pub fn subagent_run_cmd(prompts: &[String], max_turns: usize) -> Result<u8, String> {
    use std::sync::atomic::AtomicBool;
    if prompts.is_empty() {
        return Err("at least one --prompt is required".into());
    }
    let args = serde_json::json!({
        "prompts": prompts,
        "max_turns": max_turns,
    });
    let tool = subagent::TaskTool;
    let out = tool
        .execute(&args, &AtomicBool::new(false))
        .map_err(|e| format!("subagent: {e}"))?;
    print!("{out}");
    Ok(0)
}
