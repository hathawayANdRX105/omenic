//! Persistent-terminal tools (`terminal_*`), moved out of `jobs_terminal.rs`
//! into their own module. Shared helpers stay in the parent, reached via `super::*`.

use super::*;
use serde_json::Value;

use crate::{Tool, ToolError};
#[cfg(debug_assertions)]
use std::path::Path;
#[cfg(debug_assertions)]
use terminal::ScreenSnapshot;
#[cfg(debug_assertions)]
use terminal::screen::{render_ascii, render_json, render_svg};
use terminal::{TerminalId, TerminalRegistry};

// -----------------------------------------------------------------------------
// terminal_create
// -----------------------------------------------------------------------------

pub struct TerminalCreate {
    reg: Arc<TerminalRegistry>,
}

impl TerminalCreate {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalCreate {
    fn name(&self) -> &str {
        "terminal_create"
    }

    fn description(&self) -> String {
        "Open a persistent shell session and return its id. Unlike run_bash, state survives \
         between calls: cd, exported variables and running programs persist until the session is \
         closed. Use terminal_write to send commands and terminal_read to see output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "shell": {
                    "type": "string",
                    "description": "Shell to launch, with any flags. Defaults to bash."
                },
                "cwd": {
                    "type": "string",
                    "description": "Initial working directory. Defaults to the daemon's cwd."
                },
                "cols": {"type": "integer", "description": "Terminal width. Default 80."},
                "rows": {"type": "integer", "description": "Terminal height. Default 24."}
            }
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let shell = opt_str(args, "shell").unwrap_or_else(|| "bash".to_string());
        let cwd = opt_str(args, "cwd").unwrap_or_else(current_dir);
        let cols = opt_dim(args, "cols", 80)?;
        let rows = opt_dim(args, "rows", 24)?;

        let id = self
            .reg
            .create(&shell, &cwd, cols, rows)
            .map_err(terminal_err)?;
        Ok(format!(
            "terminal {} opened ({shell} in {cwd}, {cols}x{rows})\n\
             Send commands with terminal_write, then read output with terminal_read. \
             The shell echoes input, so output includes the command line itself.",
            id.as_str()
        ))
    }
}

// -----------------------------------------------------------------------------
// terminal_write / terminal_read
// -----------------------------------------------------------------------------

pub struct TerminalWrite {
    reg: Arc<TerminalRegistry>,
}

impl TerminalWrite {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalWrite {
    fn name(&self) -> &str {
        "terminal_write"
    }

    fn description(&self) -> String {
        "Send input to a terminal session. Include a trailing newline to run a command. The shell \
         echoes what you send, so terminal_read will show your input as well as its output."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "data": {"type": "string", "description": "Text to send. End with a newline to execute."}
            },
            "required": ["id", "data"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let data = need_str(args, "data")?;
        let id = TerminalId::new(&raw);
        self.reg.write(&id, &data).map_err(terminal_err)?;
        Ok(format!("wrote {} bytes to terminal {raw}", data.len()))
    }
}

pub struct TerminalRead {
    reg: Arc<TerminalRegistry>,
}

impl TerminalRead {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalRead {
    fn name(&self) -> &str {
        "terminal_read"
    }

    fn description(&self) -> String {
        "Read output produced since the last read and clear it. Returns promptly even when the \
         shell is idle — it does not wait for output unless you pass timeout_ms. Call it in a loop \
         after terminal_write to collect a command's result."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "timeout_ms": {
                    "type": "integer",
                    "description": "How long to wait for new output before returning, in milliseconds. Default 0 (return immediately)."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let timeout = opt_u64(args, "timeout_ms").unwrap_or(0);
        let id = TerminalId::new(&raw);
        let out = self.reg.read(&id, Some(timeout)).map_err(terminal_err)?;

        let text = out.text();
        let (text, complete) = truncate_bytes(&text, READ_CHUNK_BYTES);
        let mut body = if text.is_empty() {
            "(no new output)".to_string()
        } else {
            text
        };
        if !complete {
            body.push_str("\n… (output truncated; call terminal_read again for more)");
        }
        if out.exited {
            body.push_str("\n(terminal exited)");
        }
        Ok(body)
    }
}

// -----------------------------------------------------------------------------
// terminal_resize / terminal_kill / terminal_list
// -----------------------------------------------------------------------------

pub struct TerminalResize {
    reg: Arc<TerminalRegistry>,
}

impl TerminalResize {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalResize {
    fn name(&self) -> &str {
        "terminal_resize"
    }

    fn description(&self) -> String {
        "Change a terminal's reported width and height. Full-screen programs (editors, pagers) \
         lay themselves out to these dimensions."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "cols": {"type": "integer"},
                "rows": {"type": "integer"}
            },
            "required": ["id", "cols", "rows"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let cols = need_u16(args, "cols")?;
        let rows = need_u16(args, "rows")?;
        let id = TerminalId::new(&raw);
        self.reg.resize(&id, cols, rows).map_err(terminal_err)?;
        Ok(format!("terminal {raw} resized to {cols}x{rows}"))
    }
}

pub struct TerminalKill {
    reg: Arc<TerminalRegistry>,
}

impl TerminalKill {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalKill {
    fn name(&self) -> &str {
        "terminal_kill"
    }

    fn description(&self) -> String {
        "Close a terminal session and kill its shell. Any process still running in it is \
         terminated."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"id": {"type": "string"}},
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let id = TerminalId::new(&raw);
        self.reg.close(&id).map_err(terminal_err)?;
        Ok(format!("closed terminal {raw}"))
    }
}

pub struct TerminalList {
    reg: Arc<TerminalRegistry>,
}

impl TerminalList {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

impl Tool for TerminalList {
    fn name(&self) -> &str {
        "terminal_list"
    }

    fn description(&self) -> String {
        "List open terminal sessions with their shell, working directory and size.".into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let rows = self.reg.list();
        if rows.is_empty() {
            return Ok("no terminals".into());
        }
        let mut out = String::new();
        for row in rows {
            let ended = if row.exited { " (exited)" } else { "" };
            out.push_str(&format!(
                "{:<12} {:<24} {:<28} {}x{}{}\n",
                row.id.as_str(),
                row.shell,
                row.cwd,
                row.cols,
                row.rows,
                ended
            ));
        }
        Ok(out.trim_end().to_string())
    }
}

// -----------------------------------------------------------------------------
// terminal_screen / terminal_screenshot
// -----------------------------------------------------------------------------

// Both tools ignore the abort signal, like every other tool in this file, but
// for the *other* reason than `terminal_create` / `terminal_write` do. Those
// ignore it because their work outlives the call and `terminal_kill` is how
// you stop it. These two take a picture: there is nothing to cancel, and
// aborting between the snapshot and the return would only throw away the
// frame the model asked for.

#[cfg(debug_assertions)]
pub struct TerminalScreen {
    reg: Arc<TerminalRegistry>,
}

#[cfg(debug_assertions)]
impl TerminalScreen {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

#[cfg(debug_assertions)]
impl Tool for TerminalScreen {
    fn name(&self) -> &str {
        "terminal_screen"
    }

    fn description(&self) -> String {
        "See what a terminal session currently shows, as a full frame rather than a byte \
         stream. This is how you look at a TUI: terminal_read returns the raw bytes produced \
         since the last read, and a full-screen program's output is mostly cursor moves and \
         colour codes that no amount of reading those bytes will resolve into a picture. Call \
         it after terminal_write or terminal_resize to see what the screen now looks like, and \
         terminal_screenshot to keep a frame as files. format text (the default) is an ASCII \
         frame; format json is the same frame as styled lines with per-run colours and the \
         cursor position. The frame is capped at 16 KiB so one oversized terminal cannot \
         flood the context in a single call."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "format": {
                    "type": "string",
                    "enum": ["text", "json"],
                    "description": "Frame shape: an ASCII box, or styled lines as json. Default text."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let shape = opt_str(args, "format").unwrap_or_else(|| "text".to_string());
        let id = TerminalId::new(&raw);
        let frame = self.reg.screen(&id).map_err(terminal_err)?;

        let rendered = match shape.as_str() {
            "text" => render_ascii(&frame.runs, frame.cols, frame.cursor, &frame_note(&frame)),
            "json" => render_json(
                &frame.runs,
                frame.cols,
                frame.rows,
                frame.cursor,
                &frame_note(&frame),
            ),
            other => {
                return Err(ToolError::Message(format!(
                    "unknown format: {other} (expected \"text\" or \"json\")"
                )));
            }
        };
        let (rendered, complete) = truncate_bytes(&rendered, SCREEN_CHUNK_BYTES);
        if complete {
            Ok(rendered)
        } else {
            Ok(format!(
                "{rendered}\n… (frame cut at {SCREEN_CHUNK_BYTES} bytes; \
                 terminal_screenshot writes the whole frame to files)"
            ))
        }
    }
}

#[cfg(debug_assertions)]
pub struct TerminalScreenshot {
    reg: Arc<TerminalRegistry>,
}

#[cfg(debug_assertions)]
impl TerminalScreenshot {
    pub fn new(reg: Arc<TerminalRegistry>) -> Self {
        Self { reg }
    }
}

#[cfg(debug_assertions)]
impl Tool for TerminalScreenshot {
    fn name(&self) -> &str {
        "terminal_screenshot"
    }

    fn description(&self) -> String {
        "Save a terminal session's current frame to three files and return the paths: a .txt \
         ASCII frame, an .svg of the same frame, and a .json of its styled runs. Reach for \
         terminal_screen to glance at a frame; use this one when the frame should be kept, \
         diffed against a later one, or opened as an image — the inline frame is capped at \
         16 KiB, these files are not. name labels the files; without it they are numbered in \
         call order. Nothing is overwritten: each call writes a new set under \
         /tmp/kymido-screenshots/<id>-<pid>/."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Terminal id from terminal_create."},
                "name": {
                    "type": "string",
                    "description": "Label for this set of files. Defaults to a sequence number."
                }
            },
            "required": ["id"]
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let raw = need_str(args, "id")?;
        let name = opt_str(args, "name");
        let id = TerminalId::new(&raw);
        let frame = self.reg.screen(&id).map_err(terminal_err)?;
        let note = frame_note(&frame);

        let dir = Path::new(SCREENSHOT_DIR).join(format!("{raw}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .map_err(|e| ToolError::Message(format!("cannot create {}: {e}", dir.display())))?;
        let seq = SCREENSHOT_SEQ.fetch_add(1, Ordering::Relaxed);
        let label = screenshot_label(name.as_deref());
        let stem = if label.is_empty() {
            format!("{seq:03}")
        } else {
            format!("{seq:03}-{label}")
        };
        let txt = dir.join(format!("{stem}.txt"));
        let svg = dir.join(format!("{stem}.svg"));
        let json_path = dir.join(format!("{stem}.json"));
        write_frame(
            &txt,
            &render_ascii(&frame.runs, frame.cols, frame.cursor, &note),
        )?;
        write_frame(&svg, &render_svg(&frame.runs, frame.cursor, &note))?;
        write_frame(
            &json_path,
            &render_json(&frame.runs, frame.cols, frame.rows, frame.cursor, &note),
        )?;

        Ok(format!(
            "wrote frame of terminal {raw} ({}x{}):\n{}\n{}\n{}",
            frame.cols,
            frame.rows,
            txt.display(),
            svg.display(),
            json_path.display()
        ))
    }
}

/// Metadata line carried by every artifact of one frame.
///
/// A session that has been created but has produced nothing yet is a normal
/// state, not a failure, and an empty box with no explanation reads like a
/// broken screen rather than an empty one.
#[cfg(debug_assertions)]
fn frame_note(frame: &ScreenSnapshot) -> String {
    let geometry = format!("terminal {}x{}", frame.cols, frame.rows);
    if let Some(error) = &frame.reply_error {
        // A program blocked on a cursor-position report never draws, and a
        // blank frame with no reason reads as "still starting up". Say what
        // actually happened.
        format!("{geometry} — emulator reply failed: {error}")
    } else if frame.fed_bytes == 0 {
        format!("{geometry} — no output received yet")
    } else {
        geometry
    }
}

/// Write one frame artifact, reporting the path that failed.
#[cfg(debug_assertions)]
fn write_frame(path: &Path, body: &str) -> Result<(), ToolError> {
    std::fs::write(path, body)
        .map_err(|e| ToolError::Message(format!("cannot write {}: {e}", path.display())))
}

/// A screenshot's file label: the caller's `name` reduced to a path-safe,
/// length-capped fragment, or empty when it supplied none.
///
/// The name reaches this from a model's tool call, and a file name is not a
/// place to trust one: `/` and `..` would let a label climb out of the
/// session's directory. Everything outside `[A-Za-z0-9_-]` becomes `_`, and
/// the label is capped so a long name cannot push the file name past what a
/// filesystem accepts.
#[cfg(debug_assertions)]
fn screenshot_label(name: Option<&str>) -> String {
    name.map(|raw| {
        raw.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(40)
            .collect()
    })
    .unwrap_or_default()
}
