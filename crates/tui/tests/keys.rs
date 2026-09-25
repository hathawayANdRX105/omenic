//! keys — route §3 T2 按键契约：Enter 非空提交、Esc/Ctrl+C 运行中中止 /
//! 空闲确认退出、Ctrl+D 空 composer 退出、↑/↓ 本地历史；退出与 panic 的
//! 终端还原走 [`TermGuard`] 注入缝（记录型 mock，不碰真终端）。
//!
//! [`App`] 是无 IO 纯状态机：按键进去、[`KeyAction`] / 出站队列出来，
//! 所以这些断言在 CI 无 TTY 环境下照样成立。

use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use omenic_tui::app::{App, KeyAction};
use omenic_tui::termguard::{
    TermGuard, TermOps, build_hook, crash_report_path, write_crash_report,
};

/// 一次按键（kind 默认 Press，路由只认 Press/Repeat）。
fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn enter_key() -> KeyEvent {
    key(KeyCode::Enter, KeyModifiers::NONE)
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

/// 记录型 [`TermOps`]：进出各记一笔。日志外置（Arc），因为 `TermGuard`
/// 持有 ops，测试要在 leave 之后读调用序列。
struct RecordingOps {
    log: Arc<Mutex<Vec<&'static str>>>,
    fail_enter: bool,
}

fn recording(log: Arc<Mutex<Vec<&'static str>>>) -> RecordingOps {
    RecordingOps {
        log,
        fail_enter: false,
    }
}

impl TermOps for RecordingOps {
    fn enter(&mut self) -> std::io::Result<()> {
        self.log.lock().unwrap().push("enter");
        if self.fail_enter {
            return Err(std::io::Error::other("enter failed halfway"));
        }
        Ok(())
    }

    fn leave(&mut self) -> std::io::Result<()> {
        self.log.lock().unwrap().push("leave");
        Ok(())
    }
}

/// Enter 只提交非空：空串/纯空白吞掉不发，非空进队列、清 composer，
/// 出站时才置 running 并把 user 消息折进 transcript（route §3）。
#[test]
fn enter_submits_only_non_empty() {
    let mut app = App::new();
    // 空 Enter：不提交。
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert!(app.queued().is_none(), "空串 Enter 不许提交");
    // 纯空白：同样不提交，但行被清掉（误敲 Enter 不留残行）。
    assert_eq!(app.type_char(' '), KeyAction::None);
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert!(app.input().is_empty(), "空白行 Enter 要清行");
    assert!(app.queued().is_none(), "纯空白不许提交");
    // 非空：进历史/出站队列、composer 清空。
    type_str(&mut app, "hello");
    assert_eq!(app.input(), "hello");
    assert_eq!(app.handle_key(enter_key()), KeyAction::None);
    assert!(app.input().is_empty(), "提交后 composer 清空");
    assert_eq!(app.queued(), Some("hello"));
    // 出站：取走队首、置本轮 running、user 消息进 transcript 投影。
    assert_eq!(app.next_to_send().as_deref(), Some("hello"));
    assert!(app.is_running());
    assert!(app.queued().is_none(), "出队后不留排队行");
    assert_eq!(app.messages().len(), 1);
    assert_eq!(app.messages()[0].content, "hello");
}

/// 运行中 Esc/Ctrl+C = 中止本轮（Abort，不进退出确认）；空闲第一下只要
/// 确认、第二下才 Quit；退出路径经 [`TermGuard`] 还原终端（幂等）。
#[test]
fn esc_aborts_running_turn_and_quit_restores_terminal() {
    let mut app = App::new();
    type_str(&mut app, "hello");
    app.handle_key(enter_key());
    assert_eq!(app.next_to_send().as_deref(), Some("hello"));
    assert!(app.is_running());
    // 运行中 Esc = Abort：中止当前轮，不进入退出确认、不退。
    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        KeyAction::Abort
    );
    assert!(!app.confirm_quit(), "Abort 不许置退出确认");
    assert!(app.is_running(), "Abort 由事件循环发 abort RPC，状态不自退");
    // Ctrl+C 与 Esc 同路由。
    assert_eq!(
        app.handle_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        KeyAction::Abort
    );
    // 本轮收尾回空闲。
    app.note_turn_end();
    assert!(!app.is_running());
    // 空闲：第一下 Esc 只要确认（None），第二下才 Quit。
    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        KeyAction::None
    );
    assert!(app.confirm_quit(), "第一下 Esc 置确认态");
    assert_eq!(
        app.handle_key(key(KeyCode::Esc, KeyModifiers::NONE)),
        KeyAction::Quit
    );
    // 退出收尾：leave alt screen + disable raw + show cursor，且只执行一次。
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut guard = TermGuard::new(recording(log.clone()));
    guard.enter().unwrap();
    guard.leave().unwrap();
    guard.leave().unwrap();
    assert_eq!(log.lock().unwrap().join(","), "enter,leave");
    assert!(!guard.active(), "退出后不再处于全屏态");
}

/// 退出路径的终端状态契约：Ctrl+D 只在空 composer 时退出；alt-screen /
/// raw 的进出严格 enter≤1、leave≤1，未进入就 leave 不碰终端，半进入
/// （enter 中途失败）也能收尾。
#[test]
fn quit_path_leaves_alt_screen_and_raw_mode() {
    // Ctrl+D：composer 空 = 退出；非空 = 忽略（不吞输入）。
    let mut app = App::new();
    assert_eq!(
        app.handle_key(key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        KeyAction::Quit
    );
    app.type_char('x');
    assert_eq!(
        app.handle_key(key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        KeyAction::None
    );
    assert_eq!(app.input(), "x", "非空时 Ctrl+D 不许吞输入");

    // 未进入就 leave：no-op，一次都不碰终端。
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut guard = TermGuard::new(recording(log.clone()));
    guard.leave().unwrap();
    assert!(!guard.active());
    assert!(log.lock().unwrap().is_empty(), "未进入不许碰终端");

    // 进入一次、离开一次：enter = alt screen + raw，leave = raw off +
    // leave alt screen + show cursor；重复 leave 幂等，不再记账。
    guard.enter().unwrap();
    assert!(guard.active());
    guard.leave().unwrap();
    assert!(!guard.active());
    guard.leave().unwrap();
    assert_eq!(log.lock().unwrap().join(","), "enter,leave");

    // 半进入：ops.enter 失败但 active 已置位 → 调用方 leave 仍能收尾。
    let log2 = Arc::new(Mutex::new(Vec::new()));
    let mut ops = recording(log2.clone());
    ops.fail_enter = true;
    let mut half = TermGuard::new(ops);
    assert!(half.enter().is_err());
    assert!(half.active(), "半进入仍标记 active 以便收尾");
    half.leave().unwrap();
    assert!(!half.active());
    assert_eq!(log2.lock().unwrap().join(","), "enter,leave");
}

/// 渲染期 panic：先还原终端、再落 `$TMPDIR` 崩溃报告（顺序即 D15：
/// 还原永远先于任何输出），panic 本身照常向上抛。
#[test]
fn panic_during_render_reports_and_restores() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let restore_log = order.clone();
    let report_log = order.clone();
    let hook = build_hook(
        move || restore_log.lock().unwrap().push("restore"),
        move |info| {
            report_log.lock().unwrap().push("report");
            write_crash_report(info).expect("crash report must be written");
        },
    );
    std::panic::set_hook(hook);

    // 模拟渲染期 panic：hook 内先 restore 后 report，随后继续上抛被接住。
    let result = std::panic::catch_unwind(|| {
        panic!("render exploded");
    });
    assert!(result.is_err(), "panic 必须继续上抛（不是被吞）");
    assert_eq!(
        order.lock().unwrap().join(","),
        "restore,report",
        "还原必须先于崩溃报告"
    );

    // 报告内容：message + location + backtrace，落在 $TMPDIR。
    let path = crash_report_path();
    let report = std::fs::read_to_string(&path).expect("crash report file must exist");
    assert!(report.contains("render exploded"), "report:\n{report}");
    assert!(report.contains("location:"), "report:\n{report}");
    assert!(report.contains("backtrace:"), "report:\n{report}");
    let _ = std::fs::remove_file(&path);
}

/// ↑/↓ 只在本地历史里循环：草稿往返不丢、到边界停住不越界 panic。
#[test]
fn history_up_down_cycles_local_edits() {
    let mut app = App::new();
    for text in ["one", "two"] {
        type_str(&mut app, text);
        app.handle_key(enter_key()); // 提交即入本地历史
    }
    type_str(&mut app, "draft"); // 未提交草稿
    assert_eq!(app.input(), "draft");

    // ↑：草稿 → 最新 → 最老 → 停在最老（不越界）。
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input(), "two");
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input(), "one");
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input(), "one", "到最老一条要停住");

    // ↓：原路返回，越过最新一条恢复草稿，再 ↓ 不越界。
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.input(), "two");
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.input(), "draft", "越过最新一条恢复未提交草稿");
    app.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(app.input(), "draft");

    // 浏览中打字脱离游标；再 ↑ 从最新重入（编辑内容先入草稿位）。
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input(), "two");
    app.type_char('!');
    assert_eq!(app.input(), "two!");
    app.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.input(), "two");

    // 空历史：↑ / ↓ 都是 no-op。
    let mut fresh = App::new();
    fresh.handle_key(key(KeyCode::Up, KeyModifiers::NONE));
    assert!(fresh.input().is_empty());
    fresh.handle_key(key(KeyCode::Down, KeyModifiers::NONE));
    assert!(fresh.input().is_empty());
}
