//! session_picker — route §4 `picker_filters_and_selects_session`：
//! 过滤失效 / 选中错行 让它红。ratatui `TestBackend` buffer 断言（照
//! dsh `tests/chat_flow.rs:465` 的取样法），无真终端、无 daemon——
//! `PickerState` 是纯状态机，按键进去、`PickerAction` 出来，重查由
//! 这里的 fake search 代替 daemon 侧 `search_sessions`。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omenic_tui::ui::session_picker::{PickerAction, PickerItem, PickerState, RunBadge, render};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::buffer::CellWidth;
use web_state::types::{Session, SessionStatus};

/// TestBackend 缓冲 → 逐行文本（同 `tests/layout.rs` 的取样法）+ 宽字符续格跳过。
///
/// 双宽 grapheme（如「无会话」）占两格：ratatui-core `set_stringn` 把首格写
/// 进 symbol、随后的续格 `reset()` 成默认空格格。逐格 naive 拼接会得到
/// `无 会 话`（2026-09-25 CI 红实录），故按 [`CellWidth`] 的实际显示宽跳过
/// 续格，还原出连续原文再断言。其余四份测试只断言 ASCII，不受此影响。
fn buffer_text(buffer: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                out.push_str(cell.symbol());
                // 宽 ≥2 时连带吃掉 (width-1) 个续格；`.max(1)` 防零宽下溢。
                x += cell.cell_width().max(1) - 1;
            }
            x += 1;
        }
        out.push('\n');
    }
    out
}

/// 把 picker 画进 60×12 的 TestBackend，取整屏文本。
fn draw(state: &PickerState) -> String {
    let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
    terminal
        .draw(|frame| render(frame, state))
        .expect("draw must fit without panicking");
    buffer_text(terminal.backend().buffer())
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn session(id: &str, title: &str) -> Session {
    Session {
        id: id.to_string(),
        title: title.to_string(),
        last_active: String::new(),
        model: String::new(),
        status: SessionStatus::Idle,
        last_active_epoch: 0,
        parent_id: None,
    }
}

fn item(id: &str, title: &str) -> PickerItem {
    PickerItem {
        session: session(id, title),
        badge: None,
    }
}

/// daemon 侧 `search_sessions` 的本地替身：空词 = 全量，非空 = 标题子串
///（真实过滤在存储侧，picker 只负责把词送出去、把回执换进来）。
fn fake_search(all: &[PickerItem], query: &str) -> Vec<PickerItem> {
    if query.trim().is_empty() {
        return all.to_vec();
    }
    all.iter()
        .filter(|it| it.session.title.contains(query))
        .cloned()
        .collect()
}

/// 走一遍「输入即过滤 → Enter 选中」：每个字符都必须要求驱动层按新词
/// 重查（过滤词丢失 = 过滤失效），选中必须携带高亮行的 id（游标没跟着
/// 结果集走 = 选中错行）。
#[test]
fn picker_filters_and_selects_session() {
    let all = vec![
        item("s-alpha", "alpha notes"),
        item("s-beta", "beta notes"),
        item("s-gamma", "gamma logs"),
    ];
    let mut state = PickerState::new("", all.clone());

    // 起步全量：三行俱全、没有「无会话」。
    let text = draw(&state);
    assert!(text.contains("alpha notes"), "起步列表缺首行:\n{text}");
    assert!(text.contains("beta notes"), "起步列表缺次行:\n{text}");
    assert!(text.contains("gamma logs"), "起步列表缺末行:\n{text}");
    assert!(!text.contains("无会话"), "非空列表不许显示无会话:\n{text}");

    // 选中错行的红线：↑↓ 夹紧在列表内，Enter 携带高亮行的 id。
    assert_eq!(state.handle_key(key(KeyCode::Down)), PickerAction::None);
    assert_eq!(state.cursor(), 1);
    match state.handle_key(key(KeyCode::Enter)) {
        PickerAction::Select(id) => assert_eq!(id, "s-beta", "Enter 选错了行"),
        other => panic!("Enter 应选中，拿到 {other:?}"),
    }

    // 输入即过滤：每个按键都要求按累积词重查，回执换进状态。
    for c in "beta".chars() {
        match state.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)) {
            PickerAction::Refetch(query) => {
                let items = fake_search(&all, &query);
                state.set_items(items);
            }
            other => panic!("输入应触发 Refetch，拿到 {other:?}"),
        }
    }
    assert_eq!(state.filter(), "beta", "过滤词必须逐字送达重查");
    assert_eq!(state.items().len(), 1, "过滤失效：只剩命中行");
    assert_eq!(state.items()[0].session.id, "s-beta");
    assert_eq!(state.cursor(), 0, "换入新结果后游标回顶");

    let text = draw(&state);
    assert!(text.contains("beta notes"), "命中行必须渲染:\n{text}");
    assert!(
        !text.contains("alpha notes"),
        "被过滤掉的行不许再渲染:\n{text}"
    );
    match state.handle_key(key(KeyCode::Enter)) {
        PickerAction::Select(id) => assert_eq!(id, "s-beta"),
        other => panic!("过滤后 Enter 应选中命中行，拿到 {other:?}"),
    }

    // ESC = 返回原会话（驱动层映射 Ok(None)）。
    assert_eq!(state.handle_key(key(KeyCode::Esc)), PickerAction::Cancel);

    // 空列表：「无会话」单行不崩；Enter 不选中、↑↓ 越界不动。
    state.set_items(Vec::new());
    let text = draw(&state);
    assert!(text.contains("无会话"), "空列表必须显示无会话单行:\n{text}");
    assert_eq!(state.handle_key(key(KeyCode::Enter)), PickerAction::None);
    assert_eq!(state.handle_key(key(KeyCode::Down)), PickerAction::None);
    assert_eq!(state.handle_key(key(KeyCode::Up)), PickerAction::None);
}

/// route §3 T4：picker 每行渲染 `runs_for_session` 的 Active/Aborted 徽章
///（正常收尾的 run 不挂徽章，行内不出现这两个标签）。
#[test]
fn picker_renders_active_and_aborted_run_badges() {
    let state = PickerState::new(
        "",
        vec![
            PickerItem {
                session: session("s-live", "live session"),
                badge: Some(RunBadge::Active),
            },
            PickerItem {
                session: session("s-dead", "aborted session"),
                badge: Some(RunBadge::Aborted),
            },
            item("s-ok", "finished session"),
        ],
    );
    let text = draw(&state);
    let row_of = |needle: &str| {
        text.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("`{needle}` missing:\n{text}"))
    };
    let live = row_of("live session");
    let dead = row_of("aborted session");
    let done = row_of("finished session");
    assert!(
        text.lines().nth(live).unwrap().contains("[Active]"),
        "在飞 run 缺 Active 徽章:\n{text}"
    );
    assert!(
        text.lines().nth(dead).unwrap().contains("[Aborted]"),
        "中止 run 缺 Aborted 徽章:\n{text}"
    );
    assert!(
        !text.lines().nth(done).unwrap().contains("[Active]")
            && !text.lines().nth(done).unwrap().contains("[Aborted]"),
        "正常收尾的 run 不挂徽章:\n{text}"
    );
}
