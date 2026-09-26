//! T8 inline 档验收测试（route §3 T8 设计注记 + handoff §4 清单）。
//!
//! 全部断言只依据「inline 模块自己吐出的字节序列」与纯函数返回值：不起
//! daemon、不进事件循环、不碰真 TTY（CI 无 TTY，真人交互另走验收记录）。
//! 白盒强检靠 [`Sim`]——只认本模块序列的极简终端模型：未知控制序列 panic、
//! CUP 越界 panic、**ED2 一律 panic**（清视口只许发生在 poison teardown
//! 字节里），同步更新一帧不闭合也 panic。
//!
//! 覆盖（handoff §4 对照）：
//! - ① 成稿 commit 不重绘已上滚历史行（#455 本体，逐帧前缀比对）
//! - ② 回合落定才 commit（世代只在 TurnEnd / resize 前进，无变化不出帧）
//! - ③ 半截写 → poison teardown（ED2 在 teardown 字节里）
//! - ④ 退出序列（同步收尾 → 还光标 → 关 paste → 滚动区复位，历史零触碰）
//! - ⑤ resize 锚定（账本重算、dock 钉新屏底、历史前缀不动、过小几何拒绝）
//! - ⑥ 流式追加逐行溢出写即定稿（越界即上滚，scrollback 只追加）
//! - ⑦/⑧ 探针门与 CLI 解析的增量在 `mode_probe.rs`

use std::io::{self, Write};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use omenic_tui::app::App;
use omenic_tui::inline as inl;
use serde_json::json;
use web_state::ui_state::{AgentEvent, UiState};

// ─────────────────────────── 终端模拟器（白盒强检） ───────────────────────────

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// 只认 inline 序列的极简终端：主屏 + 原生 scrollback + pending-wrap。
///
/// 行模型是「一字符一格」的简化（东亚宽字符不喂进 Sim，按格裁剪由
/// [`inl::composer_line`] 自己的纯函数测试守）。
#[derive(Debug)]
struct Sim {
    rows: usize,
    cols: usize,
    /// 已上滚出行（原生历史；只能追加，任何回写都会被前缀断言抓住）。
    scrollback: Vec<String>,
    /// 可见屏。
    screen: Vec<Vec<char>>,
    /// 光标（0 基 row, col）。
    cursor: (usize, usize),
    /// autowrap 挂起态（末格已写、尚未寻址）。
    pending_wrap: bool,
    /// DEC 同步更新开合（一帧喂完必须闭合）。
    sync: bool,
    /// dock 两行的 0 基行号。
    dock: (usize, usize),
}

impl Sim {
    fn new(rows: usize, cols: usize) -> Self {
        assert!(rows >= 3, "至少要留得出 transcript 区 + 2 行 dock");
        Self {
            rows,
            cols,
            scrollback: Vec::new(),
            screen: (0..rows).map(|_| vec![' '; cols]).collect(),
            cursor: (0, 0),
            pending_wrap: false,
            sync: false,
            dock: (rows - 2, rows - 1),
        }
    }

    /// 预置几行旧 transcript（模拟 attach 之前的终端历史）。
    fn seed(&mut self, lines: &[&str]) {
        for (i, line) in lines.iter().enumerate() {
            assert!(i < self.dock.0, "seed 行 {i} 必须落在 transcript 区");
            self.screen[i] = Self::pad(line, self.cols);
        }
    }

    fn pad(text: &str, cols: usize) -> Vec<char> {
        let mut row: Vec<char> = text.chars().take(cols).collect();
        while row.len() < cols {
            row.push(' ');
        }
        row
    }

    fn putc(&mut self, ch: char) {
        if self.pending_wrap {
            self.newline(); // 真实 autowrap：挂起态下一笔先落到下一行
        }
        let (r, c) = self.cursor;
        assert!(
            r < self.rows && c < self.cols,
            "写出屏外：row {r} col {c}（{}x{}）",
            self.rows,
            self.cols
        );
        self.screen[r][c] = ch;
        if c + 1 >= self.cols {
            self.pending_wrap = true;
        } else {
            self.cursor.1 = c + 1;
        }
    }

    /// LF：下移一行、列不动（`\\r\\n` 序列里列已被 `\\r` 归零）。
    fn newline(&mut self) {
        if self.cursor.0 + 1 >= self.rows {
            let top = self.screen.remove(0);
            self.scrollback.push(top.into_iter().collect());
            self.screen.push(vec![' '; self.cols]);
        } else {
            self.cursor.0 += 1;
        }
        self.pending_wrap = false;
    }

    /// 喂一帧字节；喂完同步更新必须闭合。
    fn feed(&mut self, bytes: &[u8]) {
        let text = std::str::from_utf8(bytes).expect("inline 输出必须是合法 UTF-8");
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\x1b' => match chars.next() {
                    Some('[') => self.csi(&mut chars),
                    other => panic!("inline 不许发 ESC 序列: {other:?}"),
                },
                '\n' => self.newline(),
                '\r' => {
                    self.cursor.1 = 0;
                    self.pending_wrap = false;
                }
                c if c >= ' ' && c != '\x7f' => self.putc(c),
                c => panic!("意外控制字符 {c:?}"),
            }
        }
        assert!(
            !self.sync,
            "DEC 同步更新未闭合：{}",
            String::from_utf8_lossy(bytes)
        );
    }

    fn csi(&mut self, it: &mut Chars<'_>) {
        let mut params = String::new();
        let mut priv_mode = false;
        let mut intermediates = String::new();
        loop {
            match it.next() {
                Some('?') if !priv_mode && params.is_empty() && intermediates.is_empty() => {
                    priv_mode = true;
                }
                Some(c) if c.is_ascii_digit() || c == ';' || c == ':' => params.push(c),
                Some(c @ (' '..='/')) => intermediates.push(c),
                Some(final_byte) => {
                    assert!(
                        intermediates.is_empty(),
                        "inline 不发中间字节：{intermediates:?}"
                    );
                    self.dispatch(priv_mode, &params, final_byte);
                    return;
                }
                None => panic!("CSI 未闭合"),
            }
        }
    }

    fn dispatch(&mut self, priv_mode: bool, params: &str, final_byte: char) {
        if priv_mode {
            assert!(
                matches!(final_byte, 'h' | 'l'),
                "意外私有序列 ?{params}{final_byte}"
            );
            match params {
                "2026" => self.sync = final_byte == 'h',
                // 光标显隐 / origin / bracketed paste：允许并跟踪同步位。
                "25" | "6" | "2004" => {}
                "1049" | "1047" | "47" => {
                    panic!("inline 禁 alternate screen: ?{params}{final_byte}")
                }
                "1000" | "1002" | "1003" | "1006" | "1015" => {
                    panic!("inline 禁鼠标捕获: ?{params}{final_byte}")
                }
                other => panic!("未知私有模式 ?{other}{final_byte}"),
            }
            return;
        }
        match final_byte {
            'H' | 'f' => {
                let mut parts = params.split(';');
                let r = Self::cup_num(parts.next()).max(1);
                let c = Self::cup_num(parts.next()).max(1);
                assert!(r <= self.rows, "CUP 行 {r} 越界（{} 行）", self.rows);
                assert!(c <= self.cols, "CUP 列 {c} 越界（{} 列）", self.cols);
                self.cursor = (r - 1, c - 1);
                self.pending_wrap = false;
            }
            // ED2 属 poison teardown 专用；正常帧与退出序列连 ED 都不许有。
            'J' => panic!("出现 ED{params}（ED2 只属于 poison teardown）"),
            'K' => match params {
                "" | "0" => {
                    let (r, c) = self.cursor;
                    for i in c..self.cols {
                        self.screen[r][i] = ' ';
                    }
                }
                "2" => {
                    let (r, _) = self.cursor;
                    self.screen[r] = vec![' '; self.cols];
                }
                other => panic!("意外 EL{other}"),
            },
            'm' => {} // SGR：inline 帧不着色，出现即透传
            'r' => assert!(params.is_empty(), "意外 DECSTBM {params}"),
            other => panic!("意外 CSI 终字节 {other:?} params={params:?}"),
        }
    }

    fn cup_num(part: Option<&str>) -> usize {
        match part {
            None | Some("") => 1,
            Some(p) => p.parse().unwrap_or_else(|_| panic!("非法 CUP 参数 {p:?}")),
        }
    }

    /// 已成稿内容 = 原生 scrollback ++ 可见 transcript 区（dock 两行除外）。
    fn content_log(&self) -> Vec<String> {
        self.scrollback
            .iter()
            .cloned()
            .chain(
                self.screen[..self.dock.0]
                    .iter()
                    .map(|row| row.iter().collect::<String>()),
            )
            .map(|row| row.trim_end().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn row_text(&self, row: usize) -> String {
        self.screen[row]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn dock_text(&self) -> (String, String) {
        (self.row_text(self.dock.0), self.row_text(self.dock.1))
    }

    /// 终端 resize 的简化模型：transcript 区顶部对齐保留（截尾/补空行），
    /// dock 两行弃置重建，scrollback 不受影响。
    fn resize(&mut self, rows: usize, cols: usize) {
        assert!(rows >= 3, "resize 后至少留得出 dock");
        let keep: Vec<Vec<char>> = self.screen[..self.dock.0].to_vec();
        self.rows = rows;
        self.cols = cols;
        self.dock = (rows - 2, rows - 1);
        self.screen = (0..rows)
            .map(|i| {
                if i < keep.len() {
                    Self::pad(&keep[i].iter().collect::<String>(), cols)
                } else {
                    vec![' '; cols]
                }
            })
            .collect();
        self.cursor = (0, 0); // 帧内 CUP 会重新寻址
        self.pending_wrap = false;
    }
}

// ─────────────────────────── 帧断言小工具 ───────────────────────────

/// 正常帧的硬约束：同步更新成对包壳、零 ED2。
fn assert_clean_frame(name: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    assert!(
        bytes.starts_with(inl::SYNC_BEGIN),
        "{name} 缺同步 begin：{text:?}"
    );
    assert!(
        bytes.ends_with(inl::SYNC_END),
        "{name} 缺同步 end：{text:?}"
    );
    let begins = text.matches("\x1b[?2026h").count();
    let ends = text.matches("\x1b[?2026l").count();
    assert_eq!(begins, ends, "{name} 同步 begin/end 不配对：{text:?}");
    assert!(!text.contains("\x1b[2J"), "{name} 含 ED2：{text:?}");
}
// ─────────────────────────── ① 进屏 ───────────────────────────

/// attach：旧屏推入原生 scrollback、dock 落末两行、账本开到 generation 1，
/// 全程无 alternate screen / 无鼠标捕获（route §3 T8 设计注记 ②）。
#[test]
fn attach_pushes_old_screen_into_scrollback_and_opens_generation_1() {
    let (cols, rows) = (40u16, 8u16);
    let mut app = App::new();
    app.set_model("m");
    let mut screen = inl::Screen::new(cols, rows);
    assert_eq!(screen.generation(), 0, "attach 之前不开账本");

    let composer = inl::composer_line("hi", cols);
    let status = inl::status_line(&app);
    let frame = screen.attach(&composer, &status);
    assert_clean_frame("attach", &frame);

    // 帧序：同步包裹 + dh-rs 同款进屏预置 + DOCK_ROWS+1 个 CRLF 推旧行。
    let prefix: Vec<u8> = [inl::SYNC_BEGIN, inl::ATTACH_PRELUDE].concat();
    assert!(
        frame.starts_with(&prefix),
        "attach 预置序列次序：{}",
        String::from_utf8_lossy(&frame)
    );
    let text = String::from_utf8_lossy(&frame).into_owned();
    assert_eq!(
        text.matches("\r\n").count(),
        usize::from(inl::DOCK_ROWS) + 1,
        "旧行由 dock_rows+1 个 CRLF 顶进 scrollback：{text:?}"
    );
    assert!(!text.contains("1049"), "attach 不许进 alt-screen：{text:?}");
    assert!(
        !text.contains("?1000") && !text.contains("?1006"),
        "attach 不启鼠标捕获：{text:?}"
    );
    assert_eq!(screen.generation(), 1, "attach 开账本 = 1");
    assert_eq!(
        screen.ledger().row,
        rows - inl::DOCK_ROWS,
        "尾行钉 transcript 底"
    );
    assert_eq!(screen.ledger().col, 1);

    // 白盒回放：旧行进 scrollback、dock 落位、可见内容 = 旧 transcript。
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.seed(&["old-1", "old-2", "old-3", "old-4", "old-5", "old-6"]);
    sim.feed(&frame);
    assert_eq!(
        sim.scrollback.len(),
        3,
        "前 3 行旧屏进原生历史: {:?}",
        sim.scrollback
    );
    assert_eq!(
        sim.content_log(),
        ["old-1", "old-2", "old-3", "old-4", "old-5", "old-6"].map(str::to_string),
        "旧 transcript 完整保留"
    );
    assert_eq!(
        sim.dock_text(),
        ("> hi".to_string(), "m · idle".to_string())
    );
}

// ─────────────────────────── ② 流式写即定稿（#455 本体） ───────────────────────────

/// 流式追加逐行溢出：**每一帧之后**已成稿前缀都不许被改写（追加输出打花
/// scrollback 的 bug 正本），且世代在流式期不推进、scrollback 只追加。
#[test]
fn streaming_writes_forward_only_and_never_rewrites_history() {
    let (cols, rows) = (60u16, 10u16);
    let mut app = App::new();
    app.set_model("m");
    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    let composer = inl::composer_line(app.input(), cols);
    let status = inl::status_line(&app);
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.feed(&screen.attach(&composer, &status));

    let mut log = sim.content_log();
    let mut sb: Vec<String> = Vec::new();
    for i in 0..40 {
        screen.feed(
            &AgentEvent::ToolCall {
                id: format!("t{i}"),
                name: "read_file".into(),
                args: json!({ "path": format!("/tmp/x{i}") }),
            },
            &mut proj,
        );
        assert!(screen.has_pending(), "事件必须先进暂存（{i}）");
        let frame = screen
            .take_frame(&composer, &status)
            .unwrap_or_else(|| panic!("流式第 {i} 帧必须上屏"));
        assert_clean_frame(&format!("stream[{i}]"), &frame);
        sim.feed(&frame);
        assert!(!screen.has_pending(), "take_frame 收走暂存（{i}）");

        let now = sim.content_log();
        assert!(
            now.starts_with(&log),
            "追加输出打花了 scrollback（#455）\nframe={}\nbefore={log:?}\nafter={now:?}",
            String::from_utf8_lossy(&frame)
        );
        assert!(sim.scrollback.starts_with(&sb), "scrollback 只许追加：{i}");
        assert_eq!(screen.generation(), 1, "流式期不推进世代（{i}）");
        log = now;
        sb = sim.scrollback.clone();
    }

    assert!(!sim.scrollback.is_empty(), "40 行应冲出可见区进 scrollback");
    for i in 0..40 {
        let want = format!("[tool] read_file: /tmp/x{i}");
        assert!(
            log.iter().any(|line| *line == want),
            "成稿行 {want:?} 不见了（写即定稿）：{log:?}"
        );
    }
}

/// 流式溢出的另一半：越界即上滚，已上滚行从此零寻址（Sim 对越界 CUP
/// 直接 panic，能寻址到 scrollback 的任何写入都会在这里红）。
#[test]
fn overflow_scrolls_natively_and_redocks_on_every_frame() {
    let (cols, rows) = (40u16, 8u16);
    let mut app = App::new();
    app.set_model("m");
    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    let composer = inl::composer_line(app.input(), cols);
    let status = inl::status_line(&app);
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.feed(&screen.attach(&composer, &status));

    let mut overflowed = false;
    for i in 0..12 {
        screen.feed(
            &AgentEvent::AssistantText {
                delta: format!("line-{i}\n"),
            },
            &mut proj,
        );
        let frame = screen
            .take_frame(&composer, &status)
            .unwrap_or_else(|| panic!("第 {i} 帧必须上屏"));
        sim.feed(&frame);
        // dock 恒钉末两行，重绘只动这两行（行尾空格已 trim）。
        let (c, s) = sim.dock_text();
        assert_eq!(c, ">", "composer 行恒在屏底：{c:?}");
        assert_eq!(s, "m · idle", "状态行恒在屏底：{s:?}");
        if sim.scrollback.iter().any(|line| line.starts_with("line-")) {
            overflowed = true;
        }
    }
    assert!(
        overflowed,
        "8 行屏灌 12 行必须有成稿行滚进原生 scrollback：{:?}",
        sim.scrollback
    );
    assert_eq!(screen.generation(), 1, "纯流式不推进世代");
}

// ─────────────────────────── ③ 回合落定 commit ───────────────────────────

/// commit 语义：无变化不出帧；TurnEnd 才推进世代 + 收尾状态行，且 commit
/// 帧只动 dock 两行（正文一个字节都不重写）。
#[test]
fn commit_frame_only_on_turn_settle_and_rewrites_nothing() {
    let (cols, rows) = (60u16, 10u16);
    let mut app = App::new();
    app.set_model("m");
    // 起跑一轮：状态行进 running（`esc abort` 段在状态行，不在 composer）。
    app.type_char('h');
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.take_prompt().expect("出站");
    assert!(app.is_running());

    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    let composer = inl::composer_line(app.input(), cols);
    let running = inl::status_line(&app);
    assert!(running.contains("esc abort"), "running 状态行: {running:?}");
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.feed(&screen.attach(&composer, &running));
    assert_eq!(screen.generation(), 1, "attach = 1");

    // 流式正文落屏：世代不动。
    screen.feed(
        &AgentEvent::AssistantText {
            delta: "Hello".into(),
        },
        &mut proj,
    );
    let stream = screen
        .take_frame(&composer, &running)
        .expect("流式帧必须上屏");
    sim.feed(&stream);
    assert_eq!(screen.generation(), 1, "流式不推进世代");
    assert!(
        sim.content_log().iter().any(|l| l.starts_with("Hello")),
        "正文已上屏: {:?}",
        sim.content_log()
    );
    // 无增量 + dock 未变 → 一帧都不写（光标与屏幕保持不动）。
    assert!(
        screen.take_frame(&composer, &running).is_none(),
        "无变化不出帧"
    );
    assert!(!screen.has_pending());

    let before = sim.content_log();

    // 回合落定：世代推进 + 状态收尾 → commit 帧只重绘 dock。
    screen.feed(
        &AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
        &mut proj,
    );
    assert_eq!(screen.generation(), 2, "TurnEnd = 回合落定 commit");
    app.note_turn_end();
    let idle = inl::status_line(&app);
    assert_eq!(idle, "m · idle", "落定后状态行收尾");
    let commit = screen
        .take_frame(&composer, &idle)
        .expect("状态行变化 → 必须有 commit 帧");
    assert_clean_frame("commit", &commit);
    let commit_text = String::from_utf8_lossy(&commit);
    assert!(
        !commit_text.contains("Hello"),
        "commit 帧不许重写正文：{commit_text}"
    );
    sim.feed(&commit);
    assert_eq!(
        sim.content_log(),
        before,
        "commit 只动 dock 两行，历史零触碰"
    );
    assert_eq!(
        sim.dock_text(),
        (composer.trim_end().to_string(), "m · idle".to_string())
    );
    assert_eq!(screen.generation(), 2, "没有第二次落定不再推进");
}

/// 流式尾巴在落定前可合并（同一行内续写），落定帧本身不含正文。
#[test]
fn streaming_tail_merges_in_place_until_settle() {
    let (cols, rows) = (40u16, 8u16);
    let mut app = App::new();
    app.set_model("m");
    // 先起跑一轮：落定时状态行 running → idle，dock 才有变化可出 commit 帧。
    app.type_char('h');
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.take_prompt().expect("出站");
    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    let composer = inl::composer_line(app.input(), cols);
    let status = inl::status_line(&app);
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.feed(&screen.attach(&composer, &status));

    screen.feed(
        &AgentEvent::AssistantText {
            delta: "Hel".into(),
        },
        &mut proj,
    );
    let f1 = screen.take_frame(&composer, &status).expect("chunk 1 上屏");
    sim.feed(&f1);
    assert!(
        sim.content_log().iter().any(|l| l.starts_with("Hel")),
        "{:?}",
        sim.content_log()
    );

    screen.feed(&AgentEvent::AssistantText { delta: "lo".into() }, &mut proj);
    let f2 = screen.take_frame(&composer, &status).expect("chunk 2 上屏");
    sim.feed(&f2);
    assert!(
        sim.content_log().iter().any(|l| l == "Hello"),
        "同尾行合并（不重开行、不打花）：{:?}",
        sim.content_log()
    );

    let before = sim.content_log();
    screen.feed(
        &AgentEvent::TurnEnd {
            stop_reason: "end_turn".into(),
        },
        &mut proj,
    );
    assert_eq!(screen.generation(), 2);
    app.note_turn_end();
    let commit = screen
        .take_frame(&composer, &inl::status_line(&app))
        .expect("状态收尾 → commit 帧");
    assert!(
        !String::from_utf8_lossy(&commit).contains("Hello"),
        "commit 帧只收尾 dock：{}",
        String::from_utf8_lossy(&commit)
    );
    sim.feed(&commit);
    assert_eq!(sim.content_log(), before, "正文原地不动，只是没被重写");
}

// ─────────────────────────── ④ 半截写 poison ───────────────────────────

/// 半截写模拟：第 1 次 write 只落一半，第 2 次 write 失败（其后恢复，
/// 让 poison teardown 落得进缓冲区）。
struct TornWriter {
    calls: usize,
    out: Vec<u8>,
}

impl Write for TornWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        match self.calls {
            1 => {
                let half = buf.len() / 2;
                self.out.extend_from_slice(&buf[..half]);
                Ok(half)
            }
            2 => Err(io::Error::other("torn write")),
            _ => {
                self.out.extend_from_slice(buf);
                Ok(buf.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 半截写 → poison：teardown 补写 ED2 清视口（半截草稿不进原生历史）+
/// 错误原样抛回；正常帧永远零 ED2。
#[test]
fn partial_write_poisons_with_ed2_teardown() {
    let (cols, rows) = (40u16, 8u16);
    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    screen.feed(
        &AgentEvent::AssistantText {
            delta: "hello world\n".into(),
        },
        &mut proj,
    );
    let frame = screen.take_frame("> ", "m · idle").expect("有暂存就有帧");
    assert_clean_frame("stream", &frame);

    // 健康路径：整帧落盘、零附加。
    let mut healthy = Vec::new();
    inl::apply_frame(&mut healthy, &frame).expect("健康写不报错");
    assert_eq!(healthy, frame, "健康路径不许多写一个字节");

    // 半截路径。
    let mut torn = TornWriter {
        calls: 0,
        out: Vec::new(),
    };
    let err = inl::apply_frame(&mut torn, &frame).expect_err("半截写必须 poison");
    assert!(!err.to_string().is_empty(), "错误要抛回 CLI（非零退出码）");
    let half = frame.len() / 2;
    assert_eq!(&torn.out[..half], &frame[..half], "前半帧确实已落盘");
    assert!(
        torn.out.ends_with(inl::POISON_TEARDOWN_BYTES),
        "半截帧之后必须补写 poison teardown：{}",
        String::from_utf8_lossy(&torn.out)
    );
    assert!(
        String::from_utf8_lossy(inl::POISON_TEARDOWN_BYTES).contains("\x1b[2J"),
        "teardown 用 ED2 清视口"
    );
    assert!(
        String::from_utf8_lossy(inl::POISON_TEARDOWN_BYTES).contains("\x1b[?2026l"),
        "teardown 先收同步更新"
    );

    // 写失败记 poison：退出侧据此跳过干净 detach（ED2 已经清过屏）。
    screen.mark_poisoned();
    assert!(screen.poisoned(), "写失败必须可观察为 poison");
    let detach = screen.detach();
    assert!(
        !String::from_utf8_lossy(&detach).contains("\x1b[2J"),
        "poison 后即便构造 detach 也不再补 ED2（teardown 只写一次）"
    );
}

// ─────────────────────────── ⑤ 退出序列 ───────────────────────────

/// 退出序列次序：同步收尾 → 还光标 → 关 paste → 滚动区复位；无 alt-screen
/// 可退、无 ED2（干净退出不清视口，transcript 完整留 scrollback）。
#[test]
fn detach_sequence_order_and_no_alt_screen() {
    let bytes = inl::detach_bytes(Some((7, 1, 8)));
    let text = String::from_utf8(bytes).expect("退出序列是合法 UTF-8");
    let i_sync = text.find("\x1b[?2026l").expect("① 同步更新收尾");
    let i_show = text.find("\x1b[?25h").expect("② 恢复光标");
    let i_paste = text.find("\x1b[?2004l").expect("③ 关 bracketed paste");
    let i_region = text.find("\x1b[r").expect("④ 复位滚动区");
    assert!(
        i_sync < i_show && i_show < i_paste && i_paste < i_region,
        "退出序列次序：{text:?}"
    );
    assert!(text.starts_with("\x1b[?2026l"), "先收同步更新：{text:?}");
    assert!(!text.contains("1049"), "无 alt-screen 可退：{text:?}");
    assert!(!text.contains("\x1b[2J"), "干净退出没有 ED2：{text:?}");
    assert!(!text.contains("?1000"), "不启过鼠标捕获就不关它");
}

/// 退出后 transcript 完整留在 scrollback：历史零触碰、dock 两行清空、
/// 光标停到 transcript 尾之后（shell 提示符接在其下）。
#[test]
fn detach_clears_dock_but_keeps_history_intact() {
    let (cols, rows) = (40u16, 8u16);
    let mut app = App::new();
    app.set_model("m");
    let mut screen = inl::Screen::new(cols, rows);
    let mut proj = UiState::default();
    let composer = inl::composer_line("hi", cols);
    let status = inl::status_line(&app);
    let mut sim = Sim::new(rows.into(), cols.into());
    sim.seed(&["seed-line"]);
    sim.feed(&screen.attach(&composer, &status));

    screen.feed(
        &AgentEvent::AssistantText {
            delta: "tail-line\n".into(),
        },
        &mut proj,
    );
    sim.feed(&screen.take_frame(&composer, &status).expect("frame"));
    let before = sim.content_log();
    assert!(
        before.iter().any(|l| l == "tail-line"),
        "内容已在屏上：{before:?}"
    );

    let detach = screen.detach();
    assert!(!String::from_utf8_lossy(&detach).contains("\x1b[2J"));
    sim.feed(&detach);
    assert_eq!(sim.content_log(), before, "退出零触碰已成稿内容");
    let (c, s) = sim.dock_text();
    assert!(c.is_empty() && s.is_empty(), "dock 两行清空：{c:?}/{s:?}");

    // 停靠 = CUP 到 min(尾行, dock 顶) 后再 `\r\n` 落一行、归列 0。
    let park_row = screen.ledger().row.min(rows - inl::DOCK_ROWS + 1);
    assert_eq!(
        sim.cursor,
        (usize::from(park_row), 0),
        "光标停到 transcript 尾之后"
    );
}

// ─────────────────────────── ⑥ resize 锚定 ───────────────────────────

/// resize（D17-3）：先 adopt 新几何再落笔——账本重算、generation++、dock
/// 钉新屏底、**已入 scrollback 行零触碰**（历史前缀稳定），过小几何拒绝
/// 且不改账本。
#[test]
fn resize_adopts_geometry_reanchors_dock_and_keeps_history_prefix() {
    let (cols0, rows0) = (40u16, 12u16);
    let mut app = App::new();
    app.set_model("m");
    let mut screen = inl::Screen::new(cols0, rows0);
    let mut proj = UiState::default();
    let composer = inl::composer_line(app.input(), cols0);
    let status = inl::status_line(&app);
    let mut sim = Sim::new(rows0.into(), cols0.into());
    sim.feed(&screen.attach(&composer, &status));
    for i in 0..8 {
        screen.feed(
            &AgentEvent::AssistantText {
                delta: format!("row{i}\n"),
            },
            &mut proj,
        );
        let frame = screen
            .take_frame(&composer, &status)
            .unwrap_or_else(|| panic!("第 {i} 帧必须上屏"));
        sim.feed(&frame);
    }
    assert_eq!(screen.generation(), 1);
    let before = sim.content_log();
    assert_eq!(before.len(), 8, "8 行已成稿：{before:?}");

    // 终端先变，再喂 resize 帧（D17-3：先 adopt 再落笔）。
    let (cols1, rows1) = (34u16, 14u16);
    sim.resize(rows1.into(), cols1.into());
    let frame = screen
        .resize(cols1, rows1, &composer, &status)
        .expect("合法几何必须接受");
    assert_clean_frame("resize", &frame);
    assert_eq!(screen.generation(), 2, "resize 推进世代");
    assert_eq!(screen.size(), (cols1, rows1), "账本采用新几何");
    assert_eq!(
        screen.ledger().row,
        rows1 - inl::DOCK_ROWS,
        "尾行钉到新屏底之上"
    );
    assert_eq!(screen.ledger().col, 1);
    sim.feed(&frame);
    let after = sim.content_log();
    assert!(
        after.starts_with(&before),
        "resize 打花了历史行\nbefore={before:?}\nafter={after:?}"
    );
    assert_eq!(
        sim.dock,
        (usize::from(rows1 - 2), usize::from(rows1 - 1)),
        "dock 钉新屏底"
    );
    assert_eq!(
        sim.dock_text(),
        (composer.trim_end().to_string(), "m · idle".to_string())
    );

    // 新内容落在新几何 transcript 区，历史前缀依旧不动。
    screen.feed(
        &AgentEvent::AssistantText {
            delta: "after-resize\n".into(),
        },
        &mut proj,
    );
    let frame = screen
        .take_frame(&composer, &status)
        .expect("resize 后首帧");
    sim.feed(&frame);
    let after = sim.content_log();
    assert!(
        after.starts_with(&before),
        "resize 后追加也不许动历史\nbefore={before:?}\nafter={after:?}"
    );
    assert!(
        after.iter().any(|l| l == "after-resize"),
        "新内容落新几何：{after:?}"
    );
    assert_eq!(screen.generation(), 2, "普通帧不推进世代");

    // 过小几何：拒绝 + 不改账本 + 不吐帧。
    let size = screen.size();
    let gen0 = screen.generation();
    assert!(
        screen
            .resize(cols1, inl::DOCK_ROWS, &composer, &status)
            .is_err(),
        "rows ≤ dock 必须拒绝"
    );
    assert!(
        screen.resize(0, rows1, &composer, &status).is_err(),
        "cols = 0 必须拒绝"
    );
    assert_eq!(screen.size(), size, "拒绝不许改账本");
    assert_eq!(screen.generation(), gen0, "拒绝不许推进世代");
    assert!(!screen.has_pending(), "拒绝不许留下半帧");
}

// ─────────────────────────── ⑦ dock 两行构造器 ───────────────────────────

/// composer 行：末尾可视窗口 + 按格裁剪（复刻 ui/composer 的窗口语义，
/// 另加防屏底 autowrap 的 cell clip）。
#[test]
fn composer_line_windows_from_the_tail_and_clips_by_cells() {
    assert_eq!(inl::composer_line("hi", 40), "> hi");
    assert_eq!(inl::composer_line("", 40), "> ");

    let long = "abcdefghij".repeat(10);
    let line = inl::composer_line(&long, 12);
    assert_eq!(line, "> abcdefghij", "只留末尾窗口（cols-2 字符）");
    assert!(line.chars().count() <= 12, "按格裁到列宽：{line:?}");

    // 东亚宽字符按 2 格裁（内置 char_cells 启发，不引 unicode-width）。
    let cjk = "中".repeat(8);
    assert_eq!(inl::composer_line(&cjk, 10), "> 中中中中");

    // 列宽极限：cols=1 只剩提示符首字符。
    assert_eq!(inl::composer_line("hello", 1), ">");
}

/// 状态行 = T5 footer 摊平 + 排队标记；`esc abort` 属状态行、不进 composer。
#[test]
fn status_line_flattens_footer_marks_running_and_queued() {
    let mut app = App::new();
    app.set_model("m");
    assert_eq!(inl::status_line(&app), "m · idle", "空闲 = footer 摊平");

    // 编辑中：composer 只有提示符 + 输入，无排队标记。
    app.type_char('x');
    assert_eq!(inl::composer_line(app.input(), 40), "> x");
    assert!(!inl::composer_line(app.input(), 40).contains("esc abort"));
    assert!(app.queued().is_none());
    assert!(!inl::status_line(&app).contains("queued"));

    // Enter 出站前 = 队首待发 → queued 标记。
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.queued().is_some());
    assert!(
        inl::status_line(&app).ends_with(" · queued"),
        "{}",
        inl::status_line(&app)
    );

    // 出站后 running：esc abort 进状态行，queued 消失。
    app.take_prompt().expect("出站");
    assert!(app.is_running());
    let running = inl::status_line(&app);
    assert!(running.contains("esc abort"), "running 状态行: {running:?}");
    assert!(!running.contains("queued"));
    assert!(
        !inl::composer_line(app.input(), 40).contains("esc abort"),
        "esc abort 不许进 composer"
    );

    // 运行中再提交一条：入队但不出站。
    app.type_char('y');
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.take_prompt().is_none(), "运行中不许出站");
    assert!(app.queued().is_some());
    let queued = inl::status_line(&app);
    assert!(queued.contains(" · queued"), "{queued:?}");
    assert!(queued.contains("esc abort"), "{queued:?}");
}

/// T6 翻页键在 inline 档是 no-op（回看/搜索/复制归终端原生 scrollback）。
#[test]
fn t6_scrollback_keys_are_noop_in_inline() {
    let key = |code, mods| KeyEvent::new(code, mods);
    for k in [
        key(KeyCode::PageUp, KeyModifiers::NONE),
        key(KeyCode::PageDown, KeyModifiers::NONE),
        key(KeyCode::End, KeyModifiers::NONE),
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
    ] {
        assert!(inl::scrollback_key(&k), "{k:?} 归终端原生 scrollback");
    }
    for k in [
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Esc, KeyModifiers::NONE),
        key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        key(KeyCode::Char('x'), KeyModifiers::NONE),
        key(KeyCode::Up, KeyModifiers::NONE),
        key(KeyCode::Tab, KeyModifiers::NONE),
        key(KeyCode::Backspace, KeyModifiers::NONE),
    ] {
        assert!(!inl::scrollback_key(&k), "{k:?} 是编辑/导航键，不许被吞");
    }
    let release = KeyEvent {
        kind: KeyEventKind::Release,
        ..key(KeyCode::PageUp, KeyModifiers::NONE)
    };
    assert!(!inl::scrollback_key(&release), "Release 不触发");
    let repeat = KeyEvent {
        kind: KeyEventKind::Repeat,
        ..key(KeyCode::PageDown, KeyModifiers::NONE)
    };
    assert!(inl::scrollback_key(&repeat), "长按 Repeat 照旧 no-op");
}

/// bracketed paste：只取首行、滤控制字符、不隐式提交（Enter 语义只归
/// 物理回车）。
#[test]
fn paste_takes_first_line_filters_controls_and_never_submits() {
    let mut app = App::new();
    inl::feed_paste(&mut app, "line one\nline two");
    assert_eq!(app.input(), "line one", "多行粘贴只取首行");
    assert!(app.queued().is_none(), "粘贴不隐式提交");
    assert!(app.messages().is_empty(), "不产生消息");
    assert!(!inl::status_line(&app).contains("queued"));

    inl::feed_paste(&mut app, "\nsecond");
    assert_eq!(app.input(), "line one", "首行为空则不动 composer");

    let mut ctrl = App::new();
    inl::feed_paste(&mut ctrl, "a\x07b\tc\rd");
    assert_eq!(ctrl.input(), "abc", "控制字符滤除、\\r 之后整段丢弃");
}
