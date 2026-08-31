//! Application state: sessions, input, LLM streaming.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use adaptor::{Context, Message, Model, StreamEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::session::{SessionStore, list_sessions};

/// One chat message displayed in the UI.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ChatMsg {
    pub role: Role,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// One chat session: messages + context + persistent store handle.
pub struct Session {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMsg>,
    pub context: Context,
    store: SessionStore,
}

/// UI focus: session list sidebar or message input.
#[derive(Clone, Copy, PartialEq)]
pub enum Focus {
    Sessions,
    Input,
}

pub struct App {
    pub sessions: Vec<Session>,
    pub active: usize,
    pub focus: Focus,
    pub input: String,
    pub model: Model,
    pub streaming: bool,
    pub scroll: u16,
    /// Sidebar scroll offset.
    pub session_scroll: u16,
    /// Channel receiver for streaming text deltas.
    stream_rx: Option<mpsc::Receiver<StreamEvent>>,
    /// Abort signal shared with the streaming thread.
    abort: Arc<AtomicBool>,
    /// Cursor blink counter.
    pub cursor_tick: u16,
    /// Directory for session JSONL files.
    data_dir: std::path::PathBuf,
}

/// System prompt shared across sessions.
const SYSTEM_PROMPT: &str = "你是对话助手。简短中文回复，代码用 markdown 代码块。";

impl App {
    /// Initialize: load existing sessions from disk, or create a fresh one.
    pub fn init(model: &Model, data_dir: &Path) -> Self {
        let entries = list_sessions(data_dir);
        let mut sessions = Vec::new();

        for (id, _mtime) in &entries {
            let store = SessionStore::open(data_dir, id);
            let messages = store.load().unwrap_or_default();

            // Rebuild context from loaded messages.
            let mut context = Context {
                system_prompt: Some(SYSTEM_PROMPT.into()),
                messages: Vec::with_capacity(messages.len()),
            };
            for msg in &messages {
                match msg.role {
                    Role::User => context.messages.push(Message::user_text(msg.text.clone())),
                    Role::Assistant => context
                        .messages
                        .push(Message::assistant_text(msg.text.clone())),
                }
            }

            let title = title_from_messages(&messages);
            sessions.push(Session {
                id: id.clone(),
                title,
                messages,
                context,
                store,
            });
        }

        // If no sessions exist, create one.
        if sessions.is_empty() {
            sessions.push(new_session(data_dir));
        }

        let active = sessions.len() - 1;

        App {
            sessions,
            active,
            focus: Focus::Input,
            input: String::new(),
            model: model.clone(),
            streaming: false,
            scroll: 0,
            session_scroll: 0,
            stream_rx: None,
            abort: Arc::new(AtomicBool::new(false)),
            cursor_tick: 0,
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Borrow the active session.
    pub fn active_session(&self) -> &Session {
        &self.sessions[self.active]
    }

    /// Mutably borrow the active session.
    pub fn active_session_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active]
    }

    /// Send the current input and start streaming.
    pub fn send(&mut self) {
        if self.streaming || self.input.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.input);
        let user_msg = ChatMsg {
            role: Role::User,
            text: text.clone(),
        };

        let session = &mut self.sessions[self.active];
        session.messages.push(user_msg.clone());
        session.context.messages.push(Message::user_text(text));

        // Update title if this is the first message.
        if session.title == "新会话" {
            session.title = title_from_text(&user_msg.text);
        }

        // Persist user message.
        let _ = session.store.append_msg(&user_msg);

        // Add empty assistant message for streaming.
        session.messages.push(ChatMsg {
            role: Role::Assistant,
            text: String::new(),
        });

        self.streaming = true;
        self.abort.store(false, Ordering::Relaxed);

        let (tx, rx) = mpsc::channel();
        self.stream_rx = Some(rx);

        let model = self.model.clone();
        let context = session.context.clone();
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
                    if let Some(last) = self.sessions[self.active].messages.last_mut() {
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
                    if !done {
                        if let Some(last) = self.sessions[self.active].messages.last_mut() {
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
            if let Some(last) = self.sessions[self.active].messages.last_mut() {
                last.text.push_str(&accumulated);
            }
        }

        if done {
            let session = &mut self.sessions[self.active];
            if !was_error {
                if let Some(last) = session.messages.last() {
                    if last.role == Role::Assistant && !last.text.is_empty() {
                        session
                            .context
                            .messages
                            .push(Message::assistant_text(last.text.clone()));
                        // Persist assistant message.
                        let _ = session.store.append_msg(last);
                    }
                }
            } else {
                // Error case: still persist whatever partial text we got.
                if let Some(last) = session.messages.last() {
                    if last.role == Role::Assistant && !last.text.is_empty() {
                        let _ = session.store.append_msg(last);
                    }
                }
            }
            self.streaming = false;
            self.stream_rx = None;
        }

        self.cursor_tick = self.cursor_tick.wrapping_add(1);
    }

    /// Abort current stream.
    pub fn abort(&mut self) {
        if self.streaming {
            self.abort.store(true, Ordering::Relaxed);
        }
    }

    /// Create a new session and switch to it.
    pub fn new_session(&mut self) {
        let session = new_session(&self.data_dir);
        self.sessions.push(session);
        self.active = self.sessions.len() - 1;
        self.scroll = 0;
        self.focus = Focus::Input;
    }

    /// Delete a session by index. Switches to the nearest remaining one.
    pub fn delete_session(&mut self, idx: usize) -> bool {
        if self.sessions.len() <= 1 {
            return false; // keep at least one
        }
        let id = self.sessions[idx].id.clone();
        let _ = SessionStore::delete(&self.data_dir, &id);
        self.sessions.remove(idx);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        }
        if idx <= self.active && self.active > 0 {
            // adjustment after removal below
        }
        self.scroll = 0;
        true
    }

    /// Switch active session by index.
    pub fn switch_session(&mut self, idx: usize) {
        if idx < self.sessions.len() {
            self.active = idx;
            self.scroll = 0;
            self.focus = Focus::Input;
        }
    }

    /// Handle keyboard input. Returns true to quit.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Tab: toggle focus
        if key.code == KeyCode::Tab {
            self.focus = match self.focus {
                Focus::Sessions => Focus::Input,
                Focus::Input => Focus::Sessions,
            };
            return false;
        }

        match self.focus {
            Focus::Sessions => self.handle_session_key(key),
            Focus::Input => self.handle_input_key(key),
        }
    }

    fn handle_session_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                if self.session_scroll > 0 {
                    self.session_scroll -= 1;
                }
            }
            KeyCode::Down => {
                if (self.session_scroll as usize) + 1 < self.sessions.len() {
                    self.session_scroll += 1;
                }
            }
            KeyCode::Enter => {
                self.switch_session(self.session_scroll as usize);
            }
            KeyCode::Char('n') => {
                self.new_session();
            }
            KeyCode::Char('d') => {
                self.delete_session(self.session_scroll as usize);
            }
            KeyCode::Esc => {
                self.focus = Focus::Input;
            }
            _ => {}
        }
        false
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> bool {
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
                // In input mode, Up/Down switch to session list focus
                self.focus = Focus::Sessions;
            }
            KeyCode::Down => {
                self.focus = Focus::Sessions;
            }
            _ => {}
        }
        false
    }
}

// --- helpers ---

/// Create a fresh session with a timestamp-based id.
fn new_session(data_dir: &Path) -> Session {
    let id = session_id_now();
    let store = SessionStore::open(data_dir, &id);
    Session {
        id,
        title: "新会话".into(),
        messages: vec![],
        context: Context {
            system_prompt: Some(SYSTEM_PROMPT.into()),
            messages: vec![],
        },
        store,
    }
}

/// Generate a session id from current UTC time: `s-YYYYMMDD-HHMMSS`.
fn session_id_now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs = now;
    // Convert epoch seconds to a human-readable UTC timestamp.
    let days = secs / 86400;
    let rem = secs % 86400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("s-{y:04}{m:02}{d:02}-{hour:02}{min:02}{sec:02}")
}

/// Derive a title from the first user message, truncated to 30 chars.
fn title_from_text(text: &str) -> String {
    let trimmed = text.trim();
    let result: String = trimmed.chars().take(30).collect();
    if trimmed.chars().count() > 30 {
        format!("{result}…")
    } else {
        result
    }
}

/// Derive a title from messages (first user message, or fallback).
fn title_from_messages(messages: &[ChatMsg]) -> String {
    for msg in messages {
        if msg.role == Role::User {
            return title_from_text(&msg.text);
        }
    }
    "新会话".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_short_text_unchanged() {
        assert_eq!(title_from_text("hello"), "hello");
    }

    #[test]
    fn title_trims_whitespace() {
        assert_eq!(title_from_text("  hi  "), "hi");
    }

    #[test]
    fn title_truncates_at_30_chars() {
        let s = "a".repeat(40);
        let title = title_from_text(&s);
        assert_eq!(title.chars().count(), 31, "30 chars + ellipsis");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn title_exactly_30_no_ellipsis() {
        let s = "a".repeat(30);
        let title = title_from_text(&s);
        assert_eq!(title.chars().count(), 30);
        assert!(!title.ends_with('…'));
    }

    #[test]
    fn title_unicode_counts_chars_not_bytes() {
        // 3 Chinese chars = 3 char, well under 30
        assert_eq!(title_from_text("你好世"), "你好世");
    }

    #[test]
    fn title_unicode_truncates_correctly() {
        // 35 Chinese chars → 30 + ellipsis
        let s = "中".repeat(35);
        let title = title_from_text(&s);
        assert_eq!(title.chars().count(), 31);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn title_from_messages_uses_first_user() {
        let msgs = vec![
            ChatMsg {
                role: Role::Assistant,
                text: "hi".into(),
            },
            ChatMsg {
                role: Role::User,
                text: "first user msg".into(),
            },
            ChatMsg {
                role: Role::User,
                text: "second user msg".into(),
            },
        ];
        assert_eq!(title_from_messages(&msgs), "first user msg");
    }

    #[test]
    fn title_from_messages_no_user_fallback() {
        let msgs = vec![ChatMsg {
            role: Role::Assistant,
            text: "hi".into(),
        }];
        assert_eq!(title_from_messages(&msgs), "新会话");
    }

    #[test]
    fn title_from_messages_empty() {
        assert_eq!(title_from_messages(&[]), "新会话");
    }

    #[test]
    fn session_id_format() {
        let id = session_id_now();
        assert!(id.starts_with("s-"), "should start with s-");
        assert_eq!(id.len(), "s-YYYYMMDD-HHMMSS".len(), "s-8+1+6 = 16");
    }

    #[test]
    fn session_id_components_valid() {
        let id = session_id_now();
        // s-YYYYMMDD-HHMMSS
        let parts: Vec<&str> = id.splitn(3, '-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "s");
        let date = parts[1];
        let time = parts[2];
        assert_eq!(date.len(), 8, "YYYYMMDD = 8 digits");
        assert_eq!(time.len(), 6, "HHMMSS = 6 digits");

        // Month and day in valid ranges
        let month: u32 = date[4..6].parse().unwrap();
        let day: u32 = date[6..8].parse().unwrap();
        assert!((1..=12).contains(&month), "month {month} out of range");
        assert!((1..=31).contains(&day), "day {day} out of range");
    }
}
