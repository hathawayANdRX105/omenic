//! termguard.rs — D15：alternate screen / raw mode 进出封装 + panic hook。
//!
//! 三条底线（route §3 T2）：① 正常退出必 leave alternate screen + disable
//! raw mode + show cursor（[`TermGuard::leave`] 幂等，未进入/重复 leave 不碰
//! 终端）；② panic 先还原终端再打印（ratatui *Setup Panic Hooks* recipe），
//! 并把 message + backtrace 写 `$TMPDIR` 崩溃报告（D15 埋点禁令的正式范围）；
//! ③ 还原两步各试各的，一步失败不吞另一步。
//!
//! 进出走 [`TermOps`] 注入边界——生产用 [`CrosstermOps`]，测试用记录型
//! mock 断言还原调用序列（`tests/keys.rs` 的 quit/panic 两例）。

use std::io::{self, Write};
use std::panic::PanicHookInfo;
use std::path::PathBuf;

/// 终端进出操作的可注入边界（route §3 D15 的测试缝）。
pub trait TermOps {
    /// 进入全屏态：alternate screen + raw mode。
    fn enter(&mut self) -> io::Result<()>;
    /// 离开全屏态：disable raw + leave alternate screen + show cursor。
    fn leave(&mut self) -> io::Result<()>;
}

/// crossterm 真实现。
pub struct CrosstermOps;

impl TermOps for CrosstermOps {
    fn enter(&mut self) -> io::Result<()> {
        crossterm::execute!(io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
        crossterm::terminal::enable_raw_mode()
    }

    fn leave(&mut self) -> io::Result<()> {
        restore_terminal()
    }
}

/// 还原终端三样（raw / alternate screen / cursor）：两步都必须尝试，
/// 一步失败不吞另一步；返回先发生错误。幂等，可被 panic hook 复用。
pub fn restore_terminal() -> io::Result<()> {
    let raw = crossterm::terminal::disable_raw_mode();
    let screen = crossterm::execute!(
        io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    raw.and(screen)
}

/// alt-screen/raw 生命周期闸门：enter 一次、leave 至多一次。
pub struct TermGuard<O: TermOps> {
    ops: O,
    active: bool,
}

impl<O: TermOps> TermGuard<O> {
    /// 拿一个闸门（默认未进入）。
    pub fn new(ops: O) -> Self {
        Self { ops, active: false }
    }

    /// 进入全屏态。进入可能只成功一半（alt screen 进了、raw 没进），
    /// 所以先置 active 再执行——失败后调用方仍应 [`Self::leave`] 收尾。
    pub fn enter(&mut self) -> io::Result<()> {
        self.active = true;
        self.ops.enter()
    }

    /// 离开全屏态；未进入/已离开时是 no-op（幂等，不重复碰终端）。
    pub fn leave(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        self.ops.leave()
    }

    /// 当前是否处于全屏态。
    pub fn active(&self) -> bool {
        self.active
    }
}

/// panic hook 形状（与 `std::panic::set_hook` 一致）。
pub type PanicHook = Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static>;

/// 组装「先还原 → 再落崩溃报告 → 最后交原 hook 打印」的 panic hook。
/// 顺序即 D15：还原永远先于任何输出。`restore` / `write_report` 都可注入，
/// 测试据此断言调用顺序（`tests/keys.rs::panic_during_render_reports_and_restores`）。
pub fn build_hook(
    restore: impl Fn() + Send + Sync + 'static,
    write_report: impl Fn(&PanicHookInfo<'_>) + Send + Sync + 'static,
) -> PanicHook {
    let prev = std::panic::take_hook();
    Box::new(move |info| {
        restore();
        write_report(info);
        prev(info);
    })
}

/// 生产 panic hook：装进当前进程（route §3：进屏前调用，渲染期 panic 也
/// 保证先还原再打印，报告写 `$TMPDIR`）。
pub fn install_panic_hook() {
    let hook = build_hook(
        || {
            let _ = restore_terminal();
        },
        |info| {
            let _ = write_crash_report(info);
        },
    );
    std::panic::set_hook(hook);
}

/// `$TMPDIR` 崩溃报告路径（每进程一个文件，重复崩溃覆盖重写）。
pub fn crash_report_path() -> PathBuf {
    std::env::temp_dir().join(format!("omenic-tui-crash-{}.log", std::process::id()))
}

/// 把 panic message + backtrace 写进崩溃报告，返回落盘路径。
pub fn write_crash_report(info: &PanicHookInfo<'_>) -> io::Result<PathBuf> {
    let path = crash_report_path();
    let mut file = std::fs::File::create(&path)?;
    writeln!(file, "omenic-tui crash report")?;
    writeln!(file, "message: {}", panic_message(info))?;
    match info.location() {
        Some(loc) => writeln!(file, "location: {loc}")?,
        None => writeln!(file, "location: unknown")?,
    }
    writeln!(file, "backtrace:")?;
    writeln!(file, "{}", std::backtrace::Backtrace::force_capture())?;
    Ok(path)
}

/// panic payload 取字符串（`&str` / `String` 两种常见形状，其余兜底）。
fn panic_message(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "non-string panic payload".to_string()
}
