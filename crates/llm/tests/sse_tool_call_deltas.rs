//! SSE tool_call 续帧累积回归。
//!
//! 覆盖 `sse::SseParser` 对「首帧带 id/name、续帧把 id/name 重复为空串」
//! 这一 newapi 类网关帧形的处理——空串是**存在的字段**，不是缺失字段。

use llm::StopReason;
use llm::sse::SseParser;

/// 回归：newapi 类网关（本地 3100 网关实测帧形）把 tool_call 续帧的
/// `id` / `function.name` 重复为**空串**、只带 `arguments` 分片。解析器若
/// 无条件覆盖，首帧的真实值会被冲成 `""`——模型每次工具调用都以
/// `tool "" not found` 失败、turn 以 `stop_reason: error` 结束，看板/
/// 文件读写等全部工具链静默失效。CI 的 mock 遵循严格 OpenAI 语义
/// （name 只在首帧出现），该形态只有真网关产生，故钉在此处。
#[test]
fn empty_name_id_continuation_deltas_do_not_clobber() {
    let mut p = SseParser::new();
    // 首帧：id + name + 第一个参数字符。
    p.handle_data(r#"{"choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"chatcmpl-tool-8c28","type":"function","function":{"name":"todo_add","arguments":"{"}}]}}]}"#);
    // 续帧（网关原样）：id/type/name 全空串，只推进 arguments。
    p.handle_data(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"","type":"","function":{"name":"","arguments":"\"title\": "}}]}}]}"#);
    p.handle_data(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"","type":"","function":{"name":"","arguments":"\"测试\""}}]}}]}"#);
    let last = p.handle_data(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"","type":"","function":{"name":"","arguments":"}"}}]},"finish_reason":"tool_calls"}]}"#);
    assert_eq!(last.stop_reason, Some(StopReason::ToolUse));

    let calls = p.flush();
    assert_eq!(calls.len(), 1, "同一 index 必须累积成单个调用");
    assert_eq!(calls[0].name, "todo_add", "续帧空 name 不得覆盖首帧");
    assert_eq!(calls[0].id, "chatcmpl-tool-8c28", "续帧空 id 不得覆盖首帧");
    assert_eq!(
        calls[0].args,
        serde_json::json!({"title": "测试"}),
        "跨帧 arguments 分片必须完整拼接"
    );
}

/// `reasoning_content`（deepseek-reasoner 思考链）逐帧透传为
/// `reasoning_delta`。空串是**存在的字段**而不是缺失字段——若解析器不守
/// `!reasoning.is_empty()` 门，普通文本帧会把上一帧的思考增量冲掉，web 端
/// 「思考过程」块出现空跳变。与 tool_call 续帧同一类网关帧形回归，钉在一处。
#[test]
fn reasoning_content_delta_passthrough_ignores_empty() {
    let mut p = SseParser::new();
    // 纯思考帧。
    let out = p.handle_data(
        r#"{"choices":[{"delta":{"role":"assistant","reasoning_content":"让我想想"}}]}"#,
    );
    assert_eq!(out.reasoning_delta.as_deref(), Some("让我想想"));

    // 普通文本帧：reasoning_content 缺失 → None，不得携带上一帧残留。
    let out = p.handle_data(r#"{"choices":[{"delta":{"content":"答案"}}]}"#);
    assert_eq!(out.reasoning_delta, None);

    // 空串 reasoning_content 是存在的字段：必须被忽略而不是变成 Some("")。
    let out = p.handle_data(r#"{"choices":[{"delta":{"reasoning_content":""}}]}"#);
    assert_eq!(out.reasoning_delta, None);

    // 同帧文本 + 思考：两条通道各自独立。
    let out =
        p.handle_data(r#"{"choices":[{"delta":{"content":"好","reasoning_content":"继续想"}}]}"#);
    assert_eq!(out.reasoning_delta.as_deref(), Some("继续想"));
    assert_eq!(out.text_delta.as_deref(), Some("好"));
}
