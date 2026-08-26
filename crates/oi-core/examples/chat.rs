//! Minimal terminal chat over the oi harness — try Phase 1 end to end:
//!
//! ```text
//! AGNES_API_KEY=... cargo run -p oi-core --example chat -- \
//!     --base-url https://apihub.agnes-ai.com/v1 --model agnes-2.5-flash
//! ```
//!
//! Type a prompt; the agent streams its answer and can call the four
//! builtin tools (read_file / write_file / edit / run_bash) in cwd.
//! Ctrl-C aborts the current turn; empty line quits.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use oi_core::runtime::agent::{AgentEvent, ContextLog, HttpLlm, TurnStop, run_agent_streaming};
use oi_core::runtime::llm::Model;
use oi_core::runtime::tools::builtin_tools;

fn main() -> anyhow::Result<()> {
    let mut base_url = String::new();
    let mut model_name = String::new();
    let mut api_key = std::env::var("AGNES_API_KEY").unwrap_or_default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--base-url" => base_url = args.next().unwrap_or_default(),
            "--model" => model_name = args.next().unwrap_or_default(),
            "--api-key" => api_key = args.next().unwrap_or_default(),
            other => anyhow::bail!("unknown arg: {other} (use --base-url --model --api-key)"),
        }
    }
    if base_url.is_empty() || model_name.is_empty() || api_key.is_empty() {
        anyhow::bail!("required: --base-url <url> --model <id> and AGNES_API_KEY (or --api-key)");
    }

    let backend = HttpLlm;
    let tools = builtin_tools();
    let mut context = oi_core::runtime::llm::Context {
        system_prompt: Some(
            "你是运行在用户机器上的编码助手。可以用 read_file/write_file/edit/run_bash 工具读写文件、执行命令。回答用中文，简洁。".to_string(),
        ),
        messages: vec![],
    };
    // Session persistence: replayable JSONL next to where you run it.
    let log = ContextLog::new("oi-chat-context.jsonl");
    let model = Model {
        api_key,
        model: model_name,
        base_url: Some(base_url),
        max_tokens: Some(4096),
    };

    eprintln!("── oi chat │ 空行退出，Ctrl-C 中断当前轮 ──");
    let stdin = std::io::stdin();

    loop {
        print!("\x1b[36myou>\x1b[0m ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        let prompt = line.trim();
        if prompt.is_empty() {
            break;
        }
        let msg = oi_core::runtime::llm::Message::user_text(prompt);
        log.append_message(&msg).ok(); // best-effort evidence
        context.messages.push(msg);

        let signal = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&signal);
        ctrlc::set_handler(move || stop_flag.store(true, std::sync::atomic::Ordering::Relaxed))?;

        print!("\x1b[33mai>\x1b[0m ");
        std::io::stdout().flush()?;
        run_agent_streaming(
            &backend,
            &model,
            &mut context,
            &tools,
            &signal,
            Some(&log),
            &mut |ev| match ev {
                AgentEvent::AssistantText { delta } => {
                    print!("{delta}");
                    std::io::stdout().flush().ok();
                }
                AgentEvent::ToolCall(tc) => {
                    println!(
                        "\n\x1b[35m  [tool] {} {}\x1b[0m",
                        tc.name,
                        serde_json::to_string(&tc.args).unwrap_or_default()
                    );
                }
                AgentEvent::ToolResult { name, result, .. } => {
                    let preview: String = result.chars().take(300).collect();
                    println!(
                        "\x1b[90m  [{name}] {preview}{}\x1b[0m",
                        if result.chars().count() > 300 {
                            "…"
                        } else {
                            ""
                        }
                    );
                }
                AgentEvent::TurnEnd { stop_reason } => {
                    if stop_reason == TurnStop::Aborted {
                        println!("\n\x1b[31m  [aborted]\x1b[0m");
                    }
                }
            },
        );
        println!();
    }
    eprintln!("context saved → oi-chat-context.jsonl");
    Ok(())
}
