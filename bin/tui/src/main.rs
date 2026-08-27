//! tui-chat: terminal chat with streaming LLM responses.
//!
//! Uses oi-core's LLM stream for OpenAI-compatible APIs.
//! ratatui for rendering, crossterm for terminal IO.

mod app;
mod markdown;
mod ui;

use std::io;

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

fn main() -> io::Result<()> {
    let api_key = std::env::var("AGNES_API_KEY").unwrap_or_else(|_| {
        eprintln!("AGNES_API_KEY not set");
        std::process::exit(1);
    });

    let model = adaptor::Model {
        api_key,
        model: std::env::var("CHAT_MODEL").unwrap_or_else(|_| "agnes-2.5-flash".into()),
        base_url: std::env::var("AGNES_BASE_URL")
            .ok()
            .map(|u| format!("{}/v1", u.trim_end_matches('/')))
            .or(Some("https://apihub.agnes-ai.com/v1".into())),
        max_tokens: Some(4096),
    };

    // --test: non-interactive smoke test, no TTY needed
    if std::env::args().any(|a| a == "--test") {
        return test_mode(model);
    }

    let mut app = app::App::new(model);

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

    let result = run(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Non-interactive test: send one message, print streaming response, exit.
fn test_mode(model: adaptor::Model) -> io::Result<()> {
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let ctx = adaptor::Context {
        system_prompt: Some("你是对话助手。简短中文回复，代码用 markdown 代码块。".into()),
        messages: vec![adaptor::Message::user_text("写一行 Rust hello world")],
    };

    let abort = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel::<adaptor::StreamEvent>();
    let model_clone = model.clone();
    let ctx_clone = ctx.clone();

    thread::spawn(move || {
        let events = adaptor::openai::stream(&model_clone, &ctx_clone, &[], &abort);
        for ev in events {
            let _ = tx.send(ev);
        }
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
                let lines = crate::markdown::render(&full);
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

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> io::Result<()> {
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
