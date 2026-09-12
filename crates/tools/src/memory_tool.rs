//! Memory tools: `memory_append` / `memory_search` / `memory_list`.
//!
//! Thin wrappers over the `memory` crate's JSONL store, exposed to the
//! agent as tools. Default-off: [`memory_store`] is the crate's single
//! resolution point and only hands out a live [`memory::Memory`] when
//! `OMENIC_MEMORY=1` *and* `OMENIC_MEMORY_DIR` names an existing
//! directory; every other combination yields a disabled handle whose
//! operations are no-ops (the directory is never created). Because the
//! switch is read on every `execute`, callers never branch on a flag and
//! tests can flip env vars around a single tool instance.
//!
//! These tools skip the [`crate::Policy`] layer: they run no commands,
//! and `memory_append` writes only the local memory JSONL.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::{Tool, ToolError, arg_str};

/// Live error → tool error (errors are values in this codebase).
impl From<memory::MemoryError> for ToolError {
    fn from(e: memory::MemoryError) -> Self {
        ToolError::Message(e.to_string())
    }
}

/// Resolve the memory store from the environment. Default-off: anything
/// short of `OMENIC_MEMORY=1` plus an *existing* `OMENIC_MEMORY_DIR`
/// yields a disabled handle, so no tool call can ever create the
/// directory implicitly.
pub(crate) fn memory_store() -> memory::Memory {
    if std::env::var("OMENIC_MEMORY").as_deref() != Ok("1") {
        return memory::Memory::disabled();
    }
    let Ok(dir) = std::env::var("OMENIC_MEMORY_DIR") else {
        return memory::Memory::disabled();
    };
    let path = Path::new(&dir);
    if !path.is_dir() {
        return memory::Memory::disabled();
    }
    memory::Memory::open(path).unwrap_or_else(|_| memory::Memory::disabled())
}

/// Emitted by all three tools when the store is disabled, before any I/O.
const DISABLED: &str = "memory disabled (set OMENIC_MEMORY=1 and \
    OMENIC_MEMORY_DIR to an existing directory to enable)";

fn parse_category(s: &str) -> Result<memory::Category, ToolError> {
    use memory::Category;
    match s {
        "fact" => Ok(Category::Fact),
        "preference" => Ok(Category::Preference),
        "entity" => Ok(Category::Entity),
        "correction" => Ok(Category::Correction),
        "custom" => Ok(Category::Custom),
        other => Err(ToolError::Message(format!(
            "invalid category: {other} (expected fact|preference|entity|correction|custom)"
        ))),
    }
}

fn category_name(c: memory::Category) -> &'static str {
    use memory::Category;
    match c {
        Category::Fact => "fact",
        Category::Preference => "preference",
        Category::Entity => "entity",
        Category::Correction => "correction",
        Category::Custom => "custom",
    }
}

/// Store one memory. `text` is required; `category` and `tags` optional.
pub struct MemoryAppendTool;

impl Tool for MemoryAppendTool {
    fn name(&self) -> &str {
        "memory_append"
    }

    fn description(&self) -> String {
        "写入一条本地记忆。参数：text（内容，必填）、category（fact|preference|entity|correction|custom，可选，默认 custom）、tags（字符串数组，可选）。默认关闭，需 OMENIC_MEMORY=1 与 OMENIC_MEMORY_DIR。".into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string"},
                "category": {
                    "type": "string",
                    "enum": ["fact", "preference", "entity", "correction", "custom"],
                },
                "tags": {"type": "array", "items": {"type": "string"}},
            },
            "required": ["text"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let text = arg_str(args, "text")?;
        let category = match args.get("category") {
            Some(v) => parse_category(
                v.as_str()
                    .ok_or_else(|| ToolError::Message("category must be a string".into()))?,
            )?,
            None => memory::Category::Custom,
        };
        let tags = match args.get("tags") {
            Some(Value::Null) | None => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .map(|t| {
                    t.as_str().map(str::to_string).ok_or_else(|| {
                        ToolError::Message("tags must be an array of strings".into())
                    })
                })
                .collect::<Result<_, _>>()?,
            Some(_) => {
                return Err(ToolError::Message(
                    "tags must be an array of strings".into(),
                ));
            }
        };

        let mut mem = memory_store();
        if !mem.enabled() {
            return Ok(DISABLED.into());
        }

        let mut entry = memory::MemoryEntry::new(text);
        entry.category = category;
        entry.tags = tags;
        let id = mem.append(entry)?;
        Ok(format!("remembered #{id}"))
    }
}

/// Recall the top hits for a query (direct match + graph cascade).
pub struct MemorySearchTool;

impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> String {
        "召回与查询最相关的记忆，最多 5 条，一行一条（含 id 与 score）。参数：query（查询文本）。"
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
        })
    }

    fn execute(&self, args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let query = arg_str(args, "query")?;

        let mem = memory_store();
        if !mem.enabled() {
            return Ok(DISABLED.into());
        }

        let hits = mem.recall(query, 5)?;
        if hits.is_empty() {
            return Ok(format!("no memory hits for {query:?}"));
        }

        let lines: Vec<String> = hits
            .iter()
            .map(|h| format!("#{} score={:.2} {}", h.id, h.score, h.text))
            .collect();
        Ok(format!("{} hits:\n{}", lines.len(), lines.join("\n")))
    }
}

/// List the most recent active memories.
pub struct MemoryListTool;

impl Tool for MemoryListTool {
    fn name(&self) -> &str {
        "memory_list"
    }

    fn description(&self) -> String {
        "列出最近 50 条有效（active）记忆，格式 `#<id> [<category>] <text>`，按 id 升序。无参数。"
            .into()
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(&self, _args: &Value, _signal: &AtomicBool) -> Result<String, ToolError> {
        let mem = memory_store();
        if !mem.enabled() {
            return Ok(DISABLED.into());
        }

        let mut entries: Vec<_> = mem.list()?.into_iter().filter(|e| e.active).collect();
        if entries.len() > 50 {
            entries = entries.split_off(entries.len() - 50);
        }
        if entries.is_empty() {
            return Ok("no active memories".into());
        }
        let lines: Vec<String> = entries
            .iter()
            .map(|e| format!("#{} [{}] {}", e.id, category_name(e.category), e.text))
            .collect();
        Ok(lines.join("\n"))
    }
}
