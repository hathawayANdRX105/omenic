use protocol::{AbortSignal, ToolResult, ToolSpec};
use serde_json::Value;
use std::sync::Arc;
use tools::ToolCatalog;

struct DummyTool;

impl protocol::Tool for DummyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "sample".to_string(),
            description: "test".to_string(),
            params_schema: Value::Object(serde_json::Map::new()),
        }
    }
    fn execute(
        &self,
        _args: &Value,
        _abort: &AbortSignal,
    ) -> Result<ToolResult, protocol::ToolError> {
        Ok(ToolResult {
            output: "ok".to_string(),
            is_error: false,
        })
    }
}

#[test]
fn empty_catalog_has_zero_specs() {
    let cat = ToolCatalog::new();
    assert!(cat.specs().is_empty(), "fresh catalog must have zero specs");
}

#[test]
fn catalog_register_and_find() {
    let cat = ToolCatalog::new();
    cat.register(Arc::new(DummyTool));
    let specs = cat.specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "sample");
    let found = cat.find("sample");
    assert!(found.is_some());
    let not_found = cat.find("nope");
    assert!(not_found.is_none());
}

#[test]
fn default_catalog_has_every_builtin_tool() {
    use tools::default_catalog;
    let names: Vec<String> = default_catalog()
        .specs()
        .into_iter()
        .map(|s| s.name)
        .collect();
    for expected in [
        "read_file",
        "write_file",
        "edit",
        "run_bash",
        "grep",
        "glob",
        "delete_file",
        "memory_append",
        "memory_search",
        "memory_list",
    ] {
        assert!(names.iter().any(|n| n == expected), "missing {expected}");
    }
}

#[test]
fn default_catalog_runs_builtin_tool() {
    use protocol::AbortSignal;
    use tools::{ToolExecutor, default_catalog};
    let catalog = default_catalog();
    let spec = catalog
        .specs()
        .into_iter()
        .find(|s| s.name == "run_bash")
        .unwrap();
    let out = catalog
        .execute(
            &spec,
            &serde_json::json!({"command": "echo hello-harness"}),
            &AbortSignal::new(),
        )
        .expect("run_bash must execute");
    assert!(!out.is_error);
    assert!(out.output.contains("hello-harness"), "got: {}", out.output);
}
