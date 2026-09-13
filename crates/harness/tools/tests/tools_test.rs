use omenic_harness_tools::ToolCatalog;

#[test]
fn empty_catalog_has_zero_specs() {
    let cat = ToolCatalog::new();
    assert!(cat.specs().is_empty(), "fresh catalog must have zero specs");
}

#[test]
fn catalog_register_and_find() {
    use omenic_harness_core::{AbortSignal, ToolResult, ToolSpec};
    use serde_json::Value;
    use std::sync::Arc;

    struct DummyTool;
    impl omenic_harness_tools::Tool for DummyTool {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "dummy".to_string(),
                description: "test".to_string(),
                params_schema: Value::Object(serde_json::Map::new()),
            }
        }
        fn execute(
            &self,
            _args: &Value,
            _abort: &AbortSignal,
        ) -> Result<ToolResult, omenic_harness_core::ToolError> {
            Ok(ToolResult {
                output: "ok".to_string(),
                is_error: false,
            })
        }
    }

    let mut cat = ToolCatalog::new();
    cat.register(Arc::new(DummyTool));
    let specs = cat.specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "dummy");
    let found = cat.find("dummy");
    assert!(found.is_some());
    let not_found = cat.find("nope");
    assert!(not_found.is_none());
}
