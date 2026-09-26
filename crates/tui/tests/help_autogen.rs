//! help_autogen — handoff §4 ⑤（jcode 口径）：`/help` 输出**遍历注册表**
//! 生成——新注册命令不可能缺席帮助。硬编码命令清单会让这里红。
//!
//! 三层断言：① 首批七条就位（集合/顺序钉死）；② `help_text()` 覆盖注册表
//! 每一项的名称与描述、行数 = 表头 + 条数；③ 真正执行 `/help`（两段 Enter）
//! 后 transcript 的输出同样覆盖全集，且不产生模型回合。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use omenic_tui::app::App;
use omenic_tui::slash::{self, COMMANDS};

/// T9 首批五条 + T11 `/search` + T13 `/rewind`（route §3 T9/T11/T13 定的
/// 命令集；增删注册表必红这一行）。
const FIRST_BATCH: [&str; 7] = [
    "/help",
    "/clear",
    "/model",
    "/sessions",
    "/theme",
    "/search",
    "/rewind",
];

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.type_char(c);
    }
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

/// ① 首批命令集就位：名称 + 一行描述齐全（面板与 /help 的同源数据）。
#[test]
fn first_batch_commands_are_registered() {
    let names: Vec<&str> = COMMANDS.iter().map(|cmd| cmd.name).collect();
    assert_eq!(names, FIRST_BATCH, "首批注册表集合/顺序变了");
    for cmd in COMMANDS {
        assert!(!cmd.description.is_empty(), "{} 缺一行描述", cmd.name);
        assert!(cmd.name.starts_with('/'), "{} 要带前导 `/`", cmd.name);
    }
}

/// ② `help_text()` 遍历生成：每条注册项的名称与描述都出现，行数不多不少
/// （bug：新注册命令未出现在帮助）。
#[test]
fn help_text_lists_every_registered_command() {
    let help = slash::help_text();
    for cmd in COMMANDS {
        assert!(help.contains(cmd.name), "帮助漏命令 {}:\n{help}", cmd.name);
        assert!(
            help.contains(cmd.description),
            "帮助漏 {} 的描述:\n{help}",
            cmd.name
        );
    }
    assert_eq!(
        help.lines().count(),
        COMMANDS.len() + 1,
        "行数 = 表头 1 + 注册表条数（纯遍历生成，无手写清单）"
    );
}

/// ③ 执行 `/help`（两段 Enter）：transcript 输出与 `help_text()` 同源、
/// 覆盖注册表全集；命令执行不产生模型回合。
#[test]
fn executing_help_prints_all_commands_without_model_turn() {
    let mut app = App::new();
    type_str(&mut app, "/help");
    assert_eq!(app.handle_key(enter()), omenic_tui::app::KeyAction::None);
    assert_eq!(app.input(), "/help", "第一段补全");
    assert!(
        app.messages().is_empty(),
        "第一段不执行（fencing：补全与执行不许同拍）"
    );
    assert_eq!(app.handle_key(enter()), omenic_tui::app::KeyAction::None);

    let out = app.messages().last().expect("/help 必须有输出");
    assert_eq!(out.content, slash::help_text(), "/help 输出与注册表同源");
    for cmd in COMMANDS {
        assert!(
            out.content.contains(cmd.name),
            "输出漏命令 {}:\n{}",
            cmd.name,
            out.content
        );
    }
    assert!(
        FIRST_BATCH.iter().all(|name| out.content.contains(name)),
        "首批命令必须全在帮助里:\n{}",
        out.content
    );
    assert!(app.queued().is_none(), "命令执行不进 prompt 队列");
    assert!(app.take_prompt().is_none(), "命令执行不产生模型回合");
    assert!(!app.is_running());
}
