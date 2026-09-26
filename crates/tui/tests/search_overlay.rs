//! search_overlay — route §3 T11 / issue #472 Done-when：转录搜索 overlay 的
//! 双入口（Ctrl+R + `/search`）、`session.search` 取数与状态真值、Enter /
//! Shift+Enter 循环导航、命中跳转定位 + 高亮、**ESC 还原打开前的滚动位置**
//!（对照 bug：搜索后滚动位置丢失）、overlay 期间按键不漏进 composer。
//!
//! [`App`] 是无 IO 纯状态机（按键进去、状态/视口快照/`KeyAction` 出来），
//! RPC 归 `app.rs` 事件循环——测试直接喂回执（[`App::set_search_hits`]），
//! 断的全是状态机与几何，CI 无 TTY 环境照样成立；渲染断言走 ratatui
//! `TestBackend`（同 `tests/slash_palette.rs` 取样法）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use omenic_tui::app::{App, KeyAction};
use omenic_tui::search;
use omenic_tui::slash::{self, Action};
use omenic_tui::theme;
use omenic_tui::ui;
use omenic_tui::ui::search_overlay;
use web_state::types::ChatMessage;

/// 80×24 下的 transcript 几何（`layout::split` 只切高度：transcript 恒满宽
/// 80、高 24 − dock 3 = 21，再由 footer 让 1 行 → 视口 20 行；断言里的
/// 精确数字都来自 `sync` 喂进去的这份几何）。
const SCREEN_W: u16 = 80;
const SCREEN_H: u16 = 24;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn shift_enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 每帧几何同步（事件循环 `draw` 前的那一步，测试走同一入口；顺带喂
/// `view_width`——命中跳转没有它就 no-op）。
fn sync(app: &mut App, width: u16, height: u16) {
    ui::sync_viewport(app, Rect::new(0, 0, width, height));
}

/// TestBackend 缓冲 → 逐行文本（同 `tests/slash_palette.rs` 的取样法）。
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

/// 画一帧取整屏文本。
fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(SCREEN_W, SCREEN_H)).expect("terminal");
    terminal
        .draw(|frame| ui::draw(frame, app))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

fn view_top(app: &App) -> usize {
    let view = app.viewport();
    view.view_top(view.total(), view.height())
}

/// 单行历史消息的通用构造（三处构造点共用一份字段序列：无推理、无工具、
/// 无附件、零时间戳——变的只有 id / 角色 / 正文）。
fn mk_msg(id: String, role: &str, content: String) -> ChatMessage {
    ChatMessage {
        id,
        role: role.to_string(),
        content,
        reasoning: String::new(),
        tool_calls: vec![],
        parts: vec![],
        attachments: vec![],
        timestamp: String::new(),
        ts_epoch_ms: 0,
    }
}

/// 一条单行历史消息（id 与 `session.search` 回执同口径 `{sid}-{seq}`）。
fn line_msg(i: usize) -> ChatMessage {
    mk_msg(format!("s-1-{i:03}"), "user", format!("mid-{i:03}"))
}

/// 60 条单行历史（每条 1 行 → total 60；80×24 视口 20 行、max 40）。
fn scrollable_app(lines: usize) -> App {
    let mut app = App::new();
    app.start_session("s-1", (0..lines).map(line_msg).collect());
    sync(&mut app, SCREEN_W, SCREEN_H);
    app
}

/// 注入几条命中的回执（id 与 transcript 同源 → 两遍映射第一遍就中）。
fn hits(indices: &[usize]) -> Vec<ChatMessage> {
    indices.iter().map(|&i| line_msg(i)).collect()
}

// ── ① ESC 还原滚动位置（对照 bug：搜索后滚动位置丢失） ──────────────────────

/// 打开 overlay 时快照视口、跳转把它挪走、ESC **原样交还**——offset /
/// follow / composer 草稿三点都回到开 overlay 之前的状态。
#[test]
fn esc_restores_scroll_position_after_navigation() {
    let mut app = scrollable_app(60);
    // 钉底 offset=40（follow）→ PgUp 一页脱钩到 21（page_rows = 20 − 1）。
    app.handle_key(key(KeyCode::PageUp));
    let saved = *app.viewport();
    let top_before = view_top(&app);
    assert!(top_before > 0 && top_before < app.viewport().max_offset());
    assert!(
        !app.viewport().is_following(),
        "前提：已脱钩（还原才看得出）"
    );

    // composer 草稿先放着（overlay 不许碰它）。
    type_str(&mut app, "draft");
    app.handle_key(ctrl('r'));
    assert!(app.search_open(), "Ctrl+R 打开 overlay");
    assert_eq!(app.input(), "draft", "打开时 composer 原文保留");
    assert_eq!(
        app.search_state().saved_viewport(),
        Some(saved),
        "打开瞬间必须快照视口（ESC 还原的正本）"
    );

    // 命中跳转把视口挪走。
    app.set_search_hits(hits(&[50]));
    app.handle_key(key(KeyCode::Enter));
    let top_after_jump = view_top(&app);
    assert_ne!(top_after_jump, top_before, "跳转必须真的移动了视口");

    // ESC：关 overlay + 视口逐字节还原。
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search_open(), "ESC 关闭 overlay");
    assert_eq!(*app.viewport(), saved, "ESC 必须原样交还打开前的视口");
    assert_eq!(view_top(&app), top_before, "首行位置回来");
    assert_eq!(app.input(), "draft", "ESC 不许动 composer 草稿");
    assert_eq!(app.search_state().saved_viewport(), None, "快照随关闭交还");
}

// ── ② Enter / Shift+Enter 循环导航 ─────────────────────────────────────────

/// 预跳转口径（vim `n`/`N`）+ 循环边界：Enter 落首条、Shift+Enter 落末条，
/// 之后 Enter 逐条向前到尾回头、Shift+Enter 反向到头回尾。
#[test]
fn enter_and_shift_enter_navigate_hits_with_wrap() {
    let mut app = scrollable_app(60);
    app.handle_key(ctrl('r'));
    type_str(&mut app, "mid");
    assert!(
        app.search_needs_fetch(),
        "查询变更即待取数（事件循环的取数判据）"
    );
    app.set_search_hits(hits(&[1, 2, 3]));
    assert!(!app.search_needs_fetch(), "回执落账即撤 dirty");
    assert_eq!(app.search_state().hits().len(), 3);
    assert!(!app.search_state().jumped(), "取数不自动跳转");

    // 预跳转：Enter → 首条。
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search_state().current(), 0);
    assert!(app.search_state().jumped());
    // 循环向前：1 → 2 → 0（到尾回头）。
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search_state().current(), 1);
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search_state().current(), 2);
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search_state().current(), 0, "到尾回头");
    // 循环向后：2 → 1（到头回尾）。
    app.handle_key(shift_enter());
    assert_eq!(app.search_state().current(), 2, "到头回尾");
    app.handle_key(shift_enter());
    assert_eq!(app.search_state().current(), 1);

    // 当前命中下标映射到 transcript 的消息下标（跳转读的就是它）。
    assert_eq!(
        app.search_state().current_hit().and_then(|hit| hit.msg),
        Some(2),
        "第 2 条命中映射到消息下标 2"
    );

    // 预跳转的反向口径：新起一台，Shift+Enter 直落末条。
    let mut fresh = scrollable_app(60);
    fresh.handle_key(ctrl('r'));
    fresh.set_search_hits(hits(&[1, 2, 3]));
    fresh.handle_key(shift_enter());
    assert_eq!(
        fresh.search_state().current(),
        2,
        "未跳过时 Shift+Enter 落末条"
    );

    // 无命中：导航是 no-op，状态行的 no matches 就是全部真相。
    app.set_search_hits(vec![]);
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.search_state().current(), 0, "无命中不动游标");
    assert_eq!(app.search_state().status(), "no matches for \"mid\"");
}

// ── ③ 无命中 / 报错 / 触顶状态显式 ────────────────────────────────────────

/// 状态行按真值阶梯逐级显式：空查询（type to search）→ 待取数 → 无命中 →
/// 命中计数（触顶 `N+`）→ RPC 失败（search error），谁也不冒充谁。
#[test]
fn no_match_state_is_explicit() {
    let mut app = scrollable_app(60);
    app.handle_key(ctrl('r'));

    // 空查询：不发 RPC、也不冒充无命中。
    assert_eq!(app.search_state().status(), "type to search");
    assert!(!app.search_needs_fetch(), "空查询不取数（存储侧是错误）");

    // 敲词 = 待取数。
    type_str(&mut app, "zzz");
    assert_eq!(app.search_state().status(), "searching...");

    // 回执为空 = 显式无命中（带查询词）——文案要真的上屏（overlay 是状态的
    // 唯一出口）。
    app.set_search_hits(vec![]);
    assert_eq!(app.search_state().status(), "no matches for \"zzz\"");
    assert!(!app.search_needs_fetch());
    let shown = screen(&app);
    assert!(
        shown.contains("no matches for \"zzz\""),
        "无命中状态必须可见:\n{shown}"
    );

    // 再编辑 = 清掉旧结果重新取数（绝不拿旧结果冒充新查询的结果）。
    type_str(&mut app, "!");
    assert_eq!(app.search_state().status(), "searching...");
    assert!(app.search_state().hits().is_empty(), "查询变更即清命中");

    // 取数失败 = 报错，不静默、不冒充无命中。
    app.set_search_error("daemon down");
    assert_eq!(app.search_state().status(), "search error: daemon down");
    assert!(app.search_state().error().is_some());

    // 触顶：回执打满 FETCH_LIMIT → `N+ matches`（不假装是全量）。
    let mut capped = scrollable_app(60);
    capped.handle_key(ctrl('r'));
    type_str(&mut capped, "mid");
    let flood: Vec<ChatMessage> = (0..search::FETCH_LIMIT as usize)
        .map(|i| mk_msg(format!("remote-{i}"), "assistant", format!("mid-{i}")))
        .collect();
    capped.set_search_hits(flood);
    assert_eq!(
        capped.search_state().status(),
        format!("{}+ matches", search::FETCH_LIMIT),
        "打满上限要标 `+`，不装全量"
    );
}

// ── ④ 双入口等效 ──────────────────────────────────────────────────────────

/// Ctrl+R 与 `/search` 是**同一落点**（[`App::open_search`]）：注册表登记、
/// 两段 Enter 执行、开合与占行一致；inline 档关闸后两个入口都不产生 overlay。
#[test]
fn ctrl_r_and_slash_search_open_the_same_overlay() {
    // 注册表入口就位（面板、`/help`、模糊过滤同源）。
    let cmd = slash::find("/search").expect("/search 必须注册");
    assert_eq!(cmd.action, Action::Search);
    assert!(!cmd.description.is_empty());
    assert!(slash::help_text().contains("/search"), "/help 必含 /search");
    assert!(
        slash::filter("/se")
            .iter()
            .any(|candidate| candidate.name == "/search"),
        "面板模糊过滤命中 /search"
    );

    // 入口一：Ctrl+R（composer 无感、视口被快照）。
    let mut via_ctrl = App::new();
    via_ctrl.handle_key(ctrl('r'));
    assert!(via_ctrl.search_open(), "Ctrl+R 打开 overlay");
    assert_eq!(via_ctrl.search_rows(), search::OVERLAY_ROWS, "占 3 行");
    assert!(via_ctrl.search_state().saved_viewport().is_some());
    assert!(!via_ctrl.confirm_quit(), "开 overlay 顺带撤掉未决退出确认");

    // 入口二：/search 两段 Enter（第一段补全、第二段执行）。
    let mut via_command = App::new();
    type_str(&mut via_command, "/search");
    assert!(via_command.slash_visible(), "行首 /search 触发面板");
    assert_eq!(via_command.handle_key(key(KeyCode::Enter)), KeyAction::None);
    assert_eq!(via_command.input(), "/search", "第一段补全");
    assert!(!via_command.search_open(), "第一段只补全不开 overlay");
    assert_eq!(via_command.handle_key(key(KeyCode::Enter)), KeyAction::None);
    assert!(via_command.search_open(), "第二段执行 = 开 overlay");
    assert!(via_command.input().is_empty(), "命令执行清空 composer");
    assert!(
        via_command.take_prompt().is_none(),
        "开 overlay 不产生模型回合"
    );
    assert!(
        via_command.take_intent().is_none(),
        "开 overlay 是本地动作，不入意图队列"
    );
    assert_eq!(via_command.search_rows(), search::OVERLAY_ROWS);

    // 两入口落到同一状态（同源：open_search）。
    assert_eq!(
        via_ctrl.search_state().status(),
        via_command.search_state().status()
    );

    // 总闸关（inline 档）：两个入口都不产生 overlay，键位与合入前一致。
    let mut inline = App::new();
    inline.disable_search();
    inline.handle_key(ctrl('r'));
    assert!(!inline.search_open(), "关闸后 Ctrl+R 不开 overlay");
    type_str(&mut inline, "/search");
    inline.handle_key(key(KeyCode::Enter));
    inline.handle_key(key(KeyCode::Enter));
    assert!(!inline.search_open(), "关闸后 /search 不开 overlay");
    assert_eq!(inline.search_rows(), 0);
}

// ── ⑤ overlay 期间按键不漏进 composer / 既有路由 ──────────────────────────

/// overlay 打开 = 整段短路：查询编辑独占、翻页与退出键被吞、行首 `/` 不
/// 武装斜杠面板、Enter 不提交 prompt；composer 草稿与视口都不被扰动。
#[test]
fn overlay_keys_never_leak_to_composer() {
    let mut app = scrollable_app(60);
    app.handle_key(key(KeyCode::PageUp));
    let top_before = view_top(&app);
    app.handle_key(ctrl('r'));
    assert!(app.search_open());

    // 字符进查询词，不进 composer。
    type_str(&mut app, "mid");
    assert_eq!(app.search_state().query(), "mid");
    assert_eq!(app.input(), "", "overlay 期间字符不许进 composer");

    // 行首 `/` 不武装斜杠面板（开合两态互斥）。
    type_str(&mut app, "/");
    assert!(!app.slash_visible(), "overlay 期间斜杠面板让位");
    assert_eq!(app.slash_rows(), 0);
    app.handle_key(key(KeyCode::Backspace)); // 查询词退回 "mid"

    // 退格只动查询词。
    app.handle_key(key(KeyCode::Backspace));
    assert_eq!(app.search_state().query(), "mi");
    assert_eq!(app.input(), "");

    // 翻页 / End / 工具卡不许动视口与工具卡状态。
    let tools_before = app.tools_expanded();
    app.handle_key(key(KeyCode::PageUp));
    app.handle_key(key(KeyCode::PageDown));
    app.handle_key(key(KeyCode::End));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(
        view_top(&app),
        top_before,
        "翻页键不许在 overlay 期间动视口"
    );
    assert_eq!(app.tools_expanded(), tools_before, "Tab 不翻工具卡");

    // 退出 / 中断 / 切换键被吞：一律 None（不产生退出/中断/入队副作用）。
    assert_eq!(
        app.handle_key(ctrl('k')),
        KeyAction::None,
        "Ctrl+K 不开 picker"
    );
    assert_eq!(
        app.handle_key(ctrl('c')),
        KeyAction::None,
        "Ctrl+C 不进中断路由"
    );
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        KeyAction::None,
        "Enter 不提交 prompt"
    );
    assert!(app.queued().is_none(), "overlay 期间不入出站队列");
    assert!(app.take_prompt().is_none(), "overlay 期间不产生模型回合");
    assert!(!app.is_running());

    // 对照组：overlay 关着时同一个 Ctrl+D 就是退出——短路是真短路。
    let mut control = App::new();
    assert_eq!(
        control.handle_key(ctrl('d')),
        KeyAction::Quit,
        "对照：关着时 Ctrl+D 退出"
    );
    assert_eq!(
        app.handle_key(ctrl('d')),
        KeyAction::None,
        "开着时 Ctrl+D 被吞"
    );

    // ESC 收口：视口回开 overlay 前的值，composer 仍是空的。
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.search_open());
    assert_eq!(view_top(&app), top_before, "ESC 还原视口");
    assert_eq!(app.input(), "", "ESC 后 composer 仍空");
}

// ── ⑥ 跳转定位 + 高亮 ─────────────────────────────────────────────────────

/// 行坐标与 transcript 断行同和（漂移即红）、跳转把视口顶对准命中行、
/// 高亮段是唯一的 brand 加粗来源（D11 语义 token）。
#[test]
fn jump_positions_and_highlights_hit_line() {
    let mut app = scrollable_app(60);

    // ① 断行同源：`line_offset` 数到末尾 == `viewport().total()`（同一份
    //    `count_wrapped` / `tool_card::rows` 核心，分叉即测试红）。
    let total = search_overlay::line_offset(
        app.messages(),
        app.tools_expanded(),
        app.messages().len(),
        SCREEN_W,
    );
    assert_eq!(total, app.viewport().total(), "命中行坐标与总行数同和");

    // ② 跳转落位：目标行 == 视口首行（超出 max 夹到 max）。
    app.handle_key(ctrl('r'));
    app.set_search_hits(hits(&[30]));
    app.handle_key(key(KeyCode::Enter));
    let line = search_overlay::line_offset(app.messages(), app.tools_expanded(), 30, SCREEN_W);
    let top = view_top(&app);
    assert_eq!(
        top,
        line.min(app.viewport().max_offset()),
        "跳转把视口顶对准命中行"
    );
    assert!(
        top <= line && line < top + app.viewport().height(),
        "命中行必须落在可见窗口内（line={line}, top={top}）"
    );
    assert_eq!(
        app.search_state().current_hit().and_then(|hit| hit.msg),
        Some(30),
        "回执 id 与 transcript 同源 → 第一遍映射即中"
    );

    // ③ 高亮：命中段 = brand 加粗，其余段 = 正文（唯一高亮来源）。
    let spans = search_overlay::excerpt_spans("alpha needle beta", "needle", SCREEN_W as usize);
    let lit = spans
        .iter()
        .find(|span| span.content.contains("needle"))
        .expect("命中段必须在 excerpt 里");
    assert_eq!(lit.style, theme::brand_bold(), "命中段走 brand 加粗 token");
    assert!(
        spans
            .iter()
            .filter(|span| span.content.contains("alpha") || span.content.contains("beta"))
            .all(|span| span.style == theme::base()),
        "非命中段走正文 token"
    );

    // ④ 渲染：查询行与状态行上屏，composer 行不长出查询词。
    type_str(&mut app, "needle");
    app.set_search_hits(vec![mk_msg(
        "s-1-000".to_string(),
        "user",
        "a needle in the haystack".to_string(),
    )]);
    let shown = screen(&app);
    assert!(shown.contains("> needle"), "查询行上屏:\n{shown}");
    assert!(shown.contains("1 matches"), "状态行计数上屏:\n{shown}");
    assert!(
        shown.contains("enter next · shift+enter prev · esc restore"),
        "按键提示上屏:\n{shown}"
    );
}
