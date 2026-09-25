//! questions — route §4 T3：数字键回答必须带原问题 id 与正确的选项下标
//! （丢 `id` 或发错 index → daemon `user.answer` 打回）；面板渲染编号选项 +
//! 描述 + 批次进度，答完/清空即消失，越界按键忽略，同 id 幂等覆盖。

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use omenic_tui::app::App;
use omenic_tui::ui;
use web_client::QuestionItem;

/// TestBackend 缓冲 → 逐行文本（同 dsh `tests/chat_flow.rs` 的取样法）。
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

#[test]
fn question_answer_sends_id_and_choice() {
    let mut app = App::new();
    let question = QuestionItem::plan_review("Approve this plan?", Some("step one".to_string()));
    let question_id = question.id.clone();
    app.set_pending_questions(vec![question]);

    // pending 有题 → 面板出题干 + 编号选项 + 描述 + 批次进度（route §3）。
    let shown = screen(&app);
    assert!(shown.contains("Approve this plan?"), "面板出题干:\n{shown}");
    assert!(shown.contains("1) Approve"), "编号选项:\n{shown}");
    assert!(
        shown.contains("Accept the plan and proceed"),
        "选项描述:\n{shown}"
    );
    assert!(shown.contains("question 1/1"), "批次进度:\n{shown}");

    // 按数字键 2 → 发送 {id, choice}（choice = 0-based 选项下标）。
    app.type_char('2');
    let request = app.take_answer().expect("数字键要产出回答请求");
    assert_eq!(request.question_id, question_id, "回答必须带原问题 id");
    assert_eq!(request.choice, 1, "choice 必须是数字键对应的选项下标");
    // 答完 → 面板消失（take_answer 摘题；pending 清空就不占行）。
    assert!(app.take_answer().is_none(), "没有第二条待答");
    let answered = screen(&app);
    assert!(
        !answered.contains("Approve this plan?"),
        "答完面板消失:\n{answered}"
    );

    // 越界按键忽略：不产出回答，也不落进 composer（route §3 边界）。
    app.set_pending_questions(vec![QuestionItem::plan_review("Second question?", None)]);
    app.type_char('9');
    assert!(app.take_answer().is_none(), "越界按键不产出回答");
    assert_eq!(app.input(), "", "越界按键要被吞掉，不进 composer");

    // 同 id 重复到达 → 幂等覆盖（以最新 pending 为准，route §3 边界）。
    let first = QuestionItem::plan_review("first summary", None);
    let mut duplicate = QuestionItem::plan_review("second summary", None);
    duplicate.id = first.id.clone();
    app.set_pending_questions(vec![first, duplicate]);
    let overwrite = screen(&app);
    assert!(
        !overwrite.contains("first summary"),
        "同 id 旧值被覆盖:\n{overwrite}"
    );
    assert!(
        overwrite.contains("second summary"),
        "同 id 取最新 pending:\n{overwrite}"
    );
}
