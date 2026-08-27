//! tui — terminal chat with streaming LLM responses.

fn main() {
    if let Err(e) = tui::main() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
