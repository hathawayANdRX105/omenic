//! slash_palette — route §3 T9 / handoff §4 ①–④：两段 Enter 状态机（第一段
//! 补全、第二段执行、fencing 挡双触发）、行首 `/` 才触发、模糊过滤命中与
//! 无命中、面板键路由（Up/Down/Tab 导航、Escape 关闭还原 composer）。
//!
//! [`App`] 是无 IO 纯状态机（按键进去、[`KeyAction`] / 出站队列 / 意图队列
//! 出来），所以这些断言在 CI 无 TTY 环境下照样成立；渲染断言走 ratatui
//! `TestBackend`（同 `tests/layout.rs` 取样法），不碰真终端。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use omenic_tui::app::{App, KeyAction};
use omenic_tui::slash;
use omenic_tui::ui;
use web_state::ui_state::AgentEvent;

/// T9 首批五条 + T11 `/search` + T13 `/rewind`（顺序 = 面板默认顺序）。
const FIRST_BATCH: [&str; 7] = [
    "/help",
    "/clear",
    "/model",
    "/sessions",
    "/theme",
    "/search",
    "/rewind",
];

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn enter_key() -> KeyEvent {
    key(KeyCode::Enter)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 两段 Enter：第一段选中补全、第二段执行（`/help` 之外的命令同一节奏）。
fn two_stage_enter(app: &mut App) {
    assert_eq!(app.handle_key(enter_key()), KeyAction::None, "第一段只补全");
    assert_eq!(app.handle_key(enter_key()), KeyAction::None, "第二段执行");
}

/// TestBackend 缓冲 → 逐行文本（同 `tests/layout.rs` 的取样法；本文件断言
/// 全 ASCII，无宽字符续格问题）。
fn buffer_text(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
            }
        }
        out.push('\n');
    }
    out
}

/// 渲染一帧 80×24 并取整屏文本。
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

/// ① 两段 Enter：第一段选中补全（不执行、不入队）、第二段执行——一条
/// Enter 既补全又执行（双触发）被 fencing 挡死，命令全程不产生模型回合。
#[test]
fn two_stage_enter_completes_then_executes() {
    let mut app = App::new();
    type_str(&mut app, "/h");
    assert!(app.slash_visible(), "行首 `/h` 触发面板");
    assert_eq!(
        app.slash_selected().map(|cmd| cmd.name),
        Some("/help"),
        "高亮默认第一项"
    );
    let before = app.messages().len();

    // 第一段：补全，不执行（fencing）。
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert_eq!(app.input(), "/help", "第一段 Enter 补全选中项");
    assert!(!app.slash_visible(), "补全后面板收起（第二段才执行）");
    assert_eq!(app.messages().len(), before, "第一段不许执行命令");
    assert!(app.queued().is_none(), "补全不进 prompt 队列");
    assert!(app.take_prompt().is_none(), "补全不产生模型回合");

    // 第二段：执行（/help 输出进本地 transcript）。
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert!(app.input().is_empty(), "执行后 composer 清空");
    assert_eq!(app.messages().len(), before + 1, "/help 输出一条本地消息");
    let out = app.messages().last().expect("本地输出必须在");
    assert_eq!(out.role, "system", "命令输出不是模型回合（不落库）");
    assert!(out.content.contains("/sessions"), "输出含注册表命令");
    assert!(app.queued().is_none(), "命令执行不进 prompt 队列");
    assert!(app.take_prompt().is_none(), "命令执行不产生模型回合");
    assert!(!app.is_running(), "命令执行不置 running");
}

/// ② 行首 `/` 才触发：非行首照常当文本提交；删掉行首 `/` 即收起；关闸
///（inline 档）后面板恒不可见、行首 `/` 保持 T8 普通文本语义。
#[test]
fn palette_opens_only_at_line_start() {
    // 非行首：不触发、不拦截提交。
    let mut app = App::new();
    type_str(&mut app, "a/b");
    assert!(!app.slash_visible(), "非行首 `/` 不触发面板");
    assert_eq!(app.slash_rows(), 0, "非行首不占渲染行");
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert_eq!(app.queued(), Some("a/b"), "非行首 `/` 照常当文本提交");

    // 行首触发；删掉行首 `/` 即收起。
    let mut app = App::new();
    type_str(&mut app, "/");
    assert!(app.slash_visible(), "行首 `/` 触发面板");
    assert_eq!(app.slash_rows(), FIRST_BATCH.len() as u16, "起步全量七行");
    app.handle_key(key(KeyCode::Backspace));
    assert!(!app.slash_visible(), "删掉行首 `/` 收起面板");

    // 关闸（inline 档）：不触发、不补全、不当命令拦截。
    let mut app = App::new();
    app.disable_slash();
    type_str(&mut app, "/help");
    assert!(!app.slash_visible(), "关闸后面板恒不可见");
    assert_eq!(app.slash_rows(), 0);
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert_eq!(app.queued(), Some("/help"), "关闸时行首 `/` 照常提交");
}

/// ③ 模糊过滤：subsequence 命中、大小写不敏感、注册表顺序不变；无命中
/// 面板仍在（给提示行）但候选为空。
#[test]
fn fuzzy_filter_hits_and_misses() {
    let matches = |query: &str| -> Vec<&'static str> {
        let mut app = App::new();
        type_str(&mut app, query);
        app.slash_matches().iter().map(|cmd| cmd.name).collect()
    };

    assert_eq!(
        matches("/"),
        FIRST_BATCH,
        "空查询 = 注册表全量（表序即面板序）"
    );
    // T11：`/search` 登记后 `/se` 不再唯一——子序列同时命中 `/sessions`
    // 与 `/search`（注册表顺序：sessions 在前、search 追加在表尾）。
    assert_eq!(
        matches("/se"),
        ["/sessions", "/search"],
        "子序列命中按表序返回多条"
    );
    assert_eq!(matches("/sear"), ["/search"], "更长子序列收敛到唯一项");
    assert_eq!(matches("/the"), ["/theme"], "/theme 唯一命中");
    assert_eq!(matches("/cl"), ["/clear"], "/clear 唯一命中");
    assert_eq!(matches("/hp"), ["/help"], "非子串子序列也算命中");
    assert_eq!(matches("/SE"), ["/sessions", "/search"], "大小写不敏感");
    assert!(
        matches("/zzz").is_empty(),
        "无命中 = 空候选（不许悄悄回退到全量）"
    );

    // 无命中时面板仍在（1 行提示），可见性与占行不崩。
    let mut app = App::new();
    type_str(&mut app, "/zzz");
    assert!(app.slash_visible(), "无命中面板不自动关闭");
    assert!(app.slash_matches().is_empty());
    assert_eq!(app.slash_rows(), 1, "无命中留 1 行提示");
    assert_eq!(app.slash_selected(), None, "无命中没有高亮项");
}

/// ④ 面板键路由：Up/BackTab 上、Down/Tab 下、到顶/到底夹紧不环绕；面板
/// 可见时 Tab 归导航（不翻工具卡）；Escape 关面板且 composer 原文保留、
/// 不进退出确认；随后编辑重新武装面板。
#[test]
fn palette_navigation_and_escape_restore_composer() {
    let mut app = App::new();
    type_str(&mut app, "/");
    let highlighted = |app: &App| app.slash_selected().map(|cmd| cmd.name);

    assert_eq!(highlighted(&app), Some("/help"));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/clear"));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(highlighted(&app), Some("/model"));
    assert!(!app.tools_expanded(), "面板可见时 Tab 归导航，不翻工具卡");
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/sessions"));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/theme"));
    // T11：`/search` 是表尾第 6 条；T13：`/rewind` 追加为第 7 条，到底
    // 夹紧点随之再后移一位。
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/search"));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/rewind"));
    app.handle_key(key(KeyCode::Down));
    assert_eq!(highlighted(&app), Some("/rewind"), "到底夹紧不环绕");
    app.handle_key(key(KeyCode::BackTab));
    assert_eq!(highlighted(&app), Some("/search"));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(highlighted(&app), Some("/theme"));
    app.handle_key(key(KeyCode::Up));
    assert_eq!(highlighted(&app), Some("/sessions"));
    for _ in 0..4 {
        app.handle_key(key(KeyCode::Up));
    }
    assert_eq!(highlighted(&app), Some("/help"), "到顶夹紧不环绕");

    // Escape：关面板、composer 原文保留、不进退出确认。
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.slash_visible(), "Escape 关闭面板");
    assert_eq!(app.input(), "/", "Escape 不许清 composer 原文");
    assert_eq!(app.slash_rows(), 0, "关闭后不占渲染行");
    assert!(!app.confirm_quit(), "面板 Esc 是关面板，不进退出确认");

    // 关面板后按键回到既有路由：Tab 翻工具卡。
    app.handle_key(key(KeyCode::Tab));
    assert!(app.tools_expanded(), "关面板后 Tab 恢复工具卡路由");

    // 再编辑 = 解除抑制重新武装（Esc 只关当次）。
    app.type_char('h');
    assert!(app.slash_visible(), "编辑后面板重新出现");
    assert_eq!(highlighted(&app), Some("/help"), "编辑后游标回顶");
}

/// 未知斜杠行（无候选、非注册名）：报状态行、原文留在 composer、既不发
/// 消息也不产生模型回合。
#[test]
fn unknown_slash_line_reports_status_without_model_turn() {
    let mut app = App::new();
    type_str(&mut app, "/zzz");
    assert!(app.slash_visible());
    assert!(app.slash_matches().is_empty(), "无候选");
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert_eq!(app.input(), "/zzz", "未知命令原文留在 composer 供改");
    assert!(
        app.status_text().contains("unknown command"),
        "状态行要可见：{}",
        app.status_text()
    );
    assert!(app.queued().is_none(), "未知命令不许发消息");
    assert!(app.take_prompt().is_none(), "未知命令不产生模型回合");
    assert!(app.messages().is_empty(), "未知命令不产生任何消息");
}

/// 注册命令的既有落点：`/sessions` 产出开 picker 的意图（事件循环消费，
/// App 自己无 IO）、`/model` `/theme` 回显本地输出——三条都不进 prompt 路径。
#[test]
fn commands_emit_intents_or_local_output_never_prompts() {
    let mut app = App::new();
    app.set_model("test-model");

    // /sessions：意图队列出队一次即空，prompt 队列始终空。
    type_str(&mut app, "/sessions");
    two_stage_enter(&mut app);
    assert_eq!(app.take_intent(), Some(slash::Intent::OpenSessions));
    assert_eq!(app.take_intent(), None, "一次执行只产一条意图");
    assert!(app.queued().is_none());
    assert!(app.take_prompt().is_none());
    assert!(
        app.messages().is_empty(),
        "开 picker 不产生 transcript 输出"
    );

    // /model：回显 footer 同源的运行时配置。
    type_str(&mut app, "/model");
    two_stage_enter(&mut app);
    assert_eq!(
        app.messages().last().map(|m| m.content.as_str()),
        Some("model: test-model"),
        "/model 回显注入的 model"
    );
    assert!(app.take_prompt().is_none());

    // /theme：回显主题现状（没有切换能力就不编）。
    type_str(&mut app, "/theme");
    two_stage_enter(&mut app);
    let theme = app
        .messages()
        .last()
        .map(|m| m.content.as_str())
        .expect("/theme 要有本地输出");
    assert!(theme.starts_with("theme:"), "/theme 回显现状：{theme}");
    assert!(app.take_prompt().is_none());
}

/// `/clear` 视图清屏：空闲清本地投影（daemon 数据不动）；运行中禁清
///（流式投影与 TurnEnd 落库源都在这张表里），报状态行。
#[test]
fn clear_wipes_local_view_but_never_while_running() {
    let mut app = App::new();
    type_str(&mut app, "hello");
    app.handle_key(enter_key());
    assert_eq!(app.next_to_send().as_deref(), Some("hello"));
    assert_eq!(app.messages().len(), 1, "出站折叠一条 user 消息");
    app.note_turn_end();

    // 空闲清屏：本地投影清空、滚动回钉底。
    type_str(&mut app, "/clear");
    two_stage_enter(&mut app);
    assert!(app.messages().is_empty(), "空闲 /clear 清本地视图");
    assert_eq!(app.scroll(), 0, "清屏后滚动钉底");
    assert!(app.queued().is_none() && app.take_prompt().is_none());

    // 运行中禁清：报状态行，投影原样保留。
    type_str(&mut app, "hi");
    app.handle_key(enter_key());
    app.take_prompt().expect("出站置 running");
    let running_msgs = app.messages().len();
    type_str(&mut app, "/clear");
    two_stage_enter(&mut app);
    assert_eq!(
        app.messages().len(),
        running_msgs,
        "运行中 /clear 不许清投影"
    );
    assert!(
        app.status_text().contains("clear"),
        "运行中禁清要可见：{}",
        app.status_text()
    );
    // 运行中 /sessions 同样不开 picker（与 Ctrl+K 同一运行中不开口径）。
    type_str(&mut app, "/sessions");
    two_stage_enter(&mut app);
    assert_eq!(app.take_intent(), None, "运行中不产开 picker 意图");
    assert!(
        app.status_text().contains("sessions"),
        "运行中不开要可见：{}",
        app.status_text()
    );
}

/// 流式中执行命令的落位安全：本地输出插到流式 assistant **之前**，
/// `UiState::apply` 的增量仍落在原消息上（追加到它后面会把本轮回复劈成
/// 两条、后半截丢落库）。
#[test]
fn local_output_during_turn_keeps_streaming_assistant_intact() {
    let mut app = App::new();
    type_str(&mut app, "hi");
    app.handle_key(enter_key());
    app.take_prompt().expect("出站置 running");
    app.apply_event(&AgentEvent::AssistantText {
        delta: "partial".into(),
    });
    assert_eq!(app.messages().last().unwrap().role, "assistant");

    type_str(&mut app, "/help");
    two_stage_enter(&mut app);
    assert_eq!(
        app.messages().last().unwrap().role,
        "assistant",
        "本地输出不许追加到流式 assistant 之后"
    );
    assert!(
        app.messages().iter().any(|m| m.role == "system"),
        "命令输出仍在（插在 assistant 之前）"
    );

    app.apply_event(&AgentEvent::AssistantText {
        delta: " more".into(),
    });
    let assistants: Vec<&str> = app
        .messages()
        .iter()
        .filter(|m| m.role == "assistant")
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(assistants, ["partial more"], "增量不劈消息");
}

/// 面板渲染（TestBackend）：候选名 + 描述上屏、无命中出提示行、关闭后
/// 整块消失（不残留占位）。
#[test]
fn palette_renders_candidates_and_no_match_row() {
    let mut app = App::new();
    type_str(&mut app, "/");
    let shown = screen(&app);
    assert!(shown.contains("/sessions"), "候选名上屏:\n{shown}");
    assert!(
        shown.contains("open the session picker"),
        "描述上屏:\n{shown}"
    );
    assert!(shown.contains("/clear"), "全量候选都上屏:\n{shown}");

    let mut miss = App::new();
    type_str(&mut miss, "/zz");
    let text = screen(&miss);
    assert!(
        text.contains("no matching command"),
        "无命中出提示行:\n{text}"
    );

    miss.handle_key(key(KeyCode::Esc));
    let closed = screen(&miss);
    assert!(
        !closed.contains("no matching command"),
        "关面板后整块消失:\n{closed}"
    );
}
