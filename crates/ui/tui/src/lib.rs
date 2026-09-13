//! tui — terminal chat with streaming LLM responses.
//!
//! Library entry points consumed by the `cli` binary (`cli` with no
//! subcommand launches the interactive TUI; `cli --test` runs the
//! non-interactive smoke test).

pub mod app;
mod markdown;
pub mod session;
mod ui;

use std::io;

use adaptor::Model;
use ratatui::Terminal;

/// Build an `adaptor::Model` from `Config`, with legacy env fallbacks so
/// existing `AGNES_API_KEY` / `CHAT_MODEL` / `AGNES_BASE_URL` users keep working.
pub fn model_from_config(config: &config::Config) -> Model {
    let api_key = config
        .llm_api_key
        .clone()
        .or_else(|| std::env::var("AGNES_API_KEY").ok())
        .unwrap_or_else(|| {
            eprintln!("no LLM API key: set [llm] api_key or AGNES_API_KEY");
            std::process::exit(1);
        });

    let model = config
        .llm_model
        .clone()
        .or_else(|| std::env::var("CHAT_MODEL").ok())
        .unwrap_or_else(|| "agnes-2.5-flash".into());

    let base_url = config
        .llm_base_url
        .as_deref()
        .map(|u| format!("{}/v1", u.trim_end_matches('/')))
        .or_else(|| {
            std::env::var("AGNES_BASE_URL")
                .ok()
                .map(|u| format!("{}/v1", u.trim_end_matches('/')))
        })
        .or(Some("https://apihub.agnes-ai.com/v1".into()));

    Model {
        api_key,
        model,
        base_url,
        max_tokens: config.llm_max_tokens.or(Some(4096)),
    }
}

/// Interactive terminal chat. Owns the alternate-screen lifecycle.
pub fn run(config: &config::Config) -> io::Result<()> {
    use crossterm::execute;
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use ratatui::backend::CrosstermBackend;

    let model = model_from_config(config);
    let mut app = app::App::init(&model, &config.data_dir);

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Restore terminal on panic before delegating to default hook
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn run_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> io::Result<()> {
    use crossterm::event::{self, Event};

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Poll for crossterm events with a short timeout so we can also
        // drain LLM streaming deltas from the background thread.
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if app.handle_key(key) {
                    break;
                }
            }
        }

        // Drain any streaming deltas that arrived
        app.drain_stream();
    }
    Ok(())
}

/// Non-interactive smoke test: send one message, print streaming response, exit.
pub fn test_mode(config: &config::Config) -> io::Result<()> {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let model = model_from_config(config);

    let ctx = adaptor::Context {
        system_prompt: Some("你是对话助手。简短中文回复，代码用 markdown 代码块。".into()),
        messages: vec![adaptor::Message::user_text("写一行 Rust hello world")],
    };

    let abort = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<adaptor::StreamEvent>();
    let model_clone = model.clone();
    let ctx_clone = ctx.clone();

    thread::spawn(move || {
        adaptor::openai::stream_cb(&model_clone, &ctx_clone, &[], &abort, &mut |ev| {
            let _ = tx.send(ev.clone());
        });
    });

    println!("=== test_mode: sending message ===");
    let mut full = String::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(adaptor::StreamEvent::TextDelta(delta)) => {
                print!("{}", delta);
                use std::io::Write;
                std::io::stdout().flush().ok();
                full.push_str(&delta);
            }
            Ok(adaptor::StreamEvent::Done { stop_reason }) => {
                println!("\n=== DONE: {:?} ===", stop_reason);
                println!("=== Markdown render test ===");
                let lines = markdown::render(&full);
                for line in &lines {
                    println!("{:?}", line);
                }
                return Ok(());
            }
            Ok(adaptor::StreamEvent::Error(e)) => {
                eprintln!("\n=== ERROR: {} ===", e);
                std::process::exit(1);
            }
            _ => {}
        }
    }
}
