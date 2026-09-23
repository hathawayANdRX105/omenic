//! pump.rs — worker run 订阅泵：一条订阅连接 → `mpsc<AgentEvent>`。
//!
//! route §2 架构铁律的前半段：`WebDaemon::subscribe_worker_run` 起独立
//! `std::thread` 读帧，帧过 [`WireTranslator`]（worker wire 不是直接的
//! AgentEvent）翻译后送 mpsc；prompt 走另一条连接、阻塞到 turn 结束，
//! 两边互不抢线程、没有死锁面。订阅断线 → 泵线程退出 → channel 关闭，
//! `run()` 侧下次 [`Receiver::recv_timeout`] 得到 `Disconnected`，映射成
//! 退出码 3 的单行断线错误（route §3 边界、§6 断线 smoke）。
//!
//! keepalive：[`RunFilteredSubscription::next_event`] 到点返回 `Ok(None)`
//! （tick 语义）而不是永久挂读——tick 只维持泵活着不退出（route §3）。
//! 接收端已死没有主动探测手段（std mpsc `Sender` 无 `is_closed`），由
//! 下一次 `send` 的 `Err`、订阅断线（`Err(_) => break`）或进程退出收线；
//! `run()` 的接收等待复用同一个 [`KEEPALIVE`]，超时只 continue。

use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use omenic_web_client::daemon::RunFilteredSubscription;
use omenic_web_state::convert::WireTranslator;
use omenic_web_state::ui_state::AgentEvent;

/// 订阅读帧的 keepalive 间隔；`run()` 的事件接收等待用同一个值。
pub(crate) const KEEPALIVE: Duration = Duration::from_secs(5);

/// 起泵线程，返回事件接收端。线程起不来 → `io::Error`（调用方经
/// `TuiError::Io` 变单行错误，不 panic）。
pub(crate) fn spawn(sub: RunFilteredSubscription) -> std::io::Result<Receiver<AgentEvent>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("oi-tui-pump".into())
        .spawn(move || pump_loop(sub, tx))?;
    Ok(rx)
}

/// 泵循环：读一帧 → 翻译 → 送 mpsc；断线或接收端消失即收线。
fn pump_loop(mut sub: RunFilteredSubscription, tx: mpsc::Sender<AgentEvent>) {
    // translator 状态归本订阅：断线即随线程丢弃，重连是新订阅新实例
    // （convert.rs 的配对语义要求按连接持有）。
    let mut translator = WireTranslator::new();
    loop {
        match sub.next_event(KEEPALIVE) {
            Ok(Some(frame)) => {
                if let Some(ev) = translator.translate(&frame.event)
                    && tx.send(ev).is_err()
                {
                    // 接收端已释放（run() 提前返回）：收线，本订阅随 Drop 断开。
                    break;
                }
            }
            // keepalive tick：不退出（route §3）。接收端消失无主动探测
            // （std mpsc Sender 没有 is_closed），下一次 send 的 Err 或
            // 订阅断线即收线；进程退出时随进程收线。
            Ok(None) => {}
            // 读错误 = 断线：关闭 channel 就是给 run() 的断线信号。
            Err(_) => break,
        }
    }
}
