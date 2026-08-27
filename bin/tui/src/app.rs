//! Application state: messages, input, LLM streaming.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;

use adaptor::{Context, Message, Model, StreamEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// One chat message displayed in the UI.
#[derive(Clone)]
pub struct ChatMsg {
    pub role: Role,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

pub struct App {
    pub messages: Vec<ChatMsg>,
    pub input: String,
    pub model: Model,
    pub context: Context,
    pub streaming: bool,
    pub scroll: u16,
    /// Channel receiver for streaming text deltas.
    stream_rx: Option<mpsc::Receiver<StreamEvent>>,
    /// Abort signal shared with the streaming thread.
    abort: Arc<AtomicBool>,
    /// Cursor blink counter (for a simple cursor indicator)
    pub cursor_tick: u16,
}

impl App {
    pub fn new(model: Model) -> Self {
        let context = Context {
            system_prompt: Some("你是对话助手。简短中文回复，代码用 markdown 代码块。".into()),
            messages: vec![],
        };
        App {
            messages: vec![],
            input: String::new(),
            model,
            context,
            streaming: false,
            scroll: 0,
            stream_rx: None,
            abort: Arc::new(AtomicBool::new(false)),
            cursor_tick: 0,
        }
    }

    /// Send the current input and start streaming.
    pub fn send(&mut self) {
        if self.streaming || self.input.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.input);
        self.messages.push(ChatMsg {
            role: Role::User,
            text: text.clone(),
        });
        self.context.messages.push(Message::user_text(text));

        // Add empty assistant message for streaming
        self.messages.push(ChatMsg {
            role: Role::Assistant,
            text: String::new(),
        });

        self.streaming = true;
        self.abort.store(false, Ordering::Relaxed);

        let (tx, rx) = mpsc::channel();
        self.stream_rx = Some(rx);

        let model = self.model.clone();
        let context = self.context.clone();
        let abort = self.abort.clone();

        thread::spawn(move || {
            adaptor::openai::stream_cb(&model, &context, &[], &abort, &mut |ev| {
                let _ = tx.send(ev.clone());
            });
        });
    }

    /// Drain streaming events from the background thread.
    pub fn drain_stream(&mut self) {
        let Some(rx) = &self.stream_rx else {
            self.cursor_tick = self.cursor_tick.wrapping_add(1);
            return;
        };
        let mut accumulated = String::new();
        let mut done = false;
        let mut was_error = false;

        loop {
            match rx.try_recv() {
                Ok(StreamEvent::TextDelta(delta)) => accumulated.push_str(&delta),
                Ok(StreamEvent::Error(e)) => {
                    // Append error to partial text, don't overwrite
                    if let Some(last) = self.messages.last_mut() {
                        last.text.push_str(&format!("\n[Error: {e}]"));
                    }
                    was_error = true;
                    done = true;
                }
                Ok(StreamEvent::Done { .. }) => {
                    done = true;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Thread died without terminal event — recover
                    if !done {
                        if let Some(last) = self.messages.last_mut() {
                            if last.text.is_empty() {
                                last.text = "[stream disconnected]".into();
                            }
                        }
                        done = true;
                    }
                    break;
                }
            }
        }

        if !accumulated.is_empty() {
            if let Some(last) = self.messages.last_mut() {
                last.text.push_str(&accumulated);
            }
        }

        if done {
            // Sync assistant text into context, but not on error
            if !was_error {
                if let Some(last) = self.messages.last() {
                    if last.role == Role::Assistant && !last.text.is_empty() {
                        self.context
                            .messages
                            .push(Message::assistant_text(last.text.clone()));
                    }
                }
            }
            self.streaming = false;
            self.stream_rx = None;
        }

        self.cursor_tick = self.cursor_tick.wrapping_add(1);
    }

    /// Abort current stream
    pub fn abort(&mut self) {
        if self.streaming {
            self.abort.store(true, Ordering::Relaxed);
        }
    }

    /// Handle keyboard input. Returns true to quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.streaming {
                    self.abort();
                } else {
                    return true; // quit
                }
            }
            KeyCode::Esc => {
                if self.streaming {
                    self.abort();
                } else {
                    return true;
                }
            }
            KeyCode::Enter => {
                if !self.streaming {
                    self.send();
                }
            }
            KeyCode::Char(c) => {
                if !self.streaming {
                    self.input.push(c);
                }
            }
            KeyCode::Backspace => {
                if !self.streaming {
                    self.input.pop();
                }
            }
            KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
            }
            _ => {}
        }
        false
    }
}
