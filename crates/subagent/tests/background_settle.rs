//! 后台子代理跑完后，结果留在 runtime 里可取，且完成事件能通过
//! `on_settled` 钩子外发（daemon 把它接到 aside 通道）。
//!
//! Red when: 结果没被 `settle` 存下来（`result` 取不到），或钩子没在 settle
//! 时触发（模型永远收不到完成通知），或 64 条上限把刚存的结果挤掉。

use std::sync::{Arc, Mutex};

use subagent::provider::SubagentResult;
use subagent::runtime::SubagentRuntimeService;

fn settle_and_probe() -> (
    SubagentRuntimeService,
    Arc<Mutex<Vec<(String, SubagentResult)>>>,
) {
    let service = SubagentRuntimeService::default();
    let seen = Arc::new(Mutex::new(Vec::<(String, SubagentResult)>::new()));
    let seen_hook = Arc::clone(&seen);
    service.set_on_settled(Arc::new(move |run_id, result| {
        seen_hook
            .lock()
            .unwrap()
            .push((run_id.to_string(), result.clone()));
    }));
    (service, seen)
}

/// settle 存下结果，`result` 能取回同一个值。
#[test]
fn settle_stores_the_result_for_later_fetch() {
    let (service, _seen) = settle_and_probe();
    let result = SubagentResult::Completed {
        output: "the answer".into(),
    };
    service.settle("sub-1", &result);
    assert_eq!(
        service.result("sub-1"),
        Some(SubagentResult::Completed {
            output: "the answer".into()
        })
    );
}

/// settle 触发 on_settled 钩子，携带正确的 run_id 与结果。
#[test]
fn settle_fires_the_completion_hook() {
    let (service, seen) = settle_and_probe();
    service.settle(
        "sub-7",
        &SubagentResult::Failed {
            error: "boom".into(),
        },
    );
    let events = seen.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, "sub-7");
    assert_eq!(
        events[0].1,
        SubagentResult::Failed {
            error: "boom".into()
        }
    );
}

/// 未 settle 的 run，`result` 取不到（不能把"还没跑完"谎报成"跑完了"）。
#[test]
fn unsettled_run_has_no_result() {
    let (service, _seen) = settle_and_probe();
    assert_eq!(service.result("sub-999"), None);
}

/// 64 条上限：最旧的结果被挤掉，最新的还在。
#[test]
fn settled_results_are_bounded() {
    let service = SubagentRuntimeService::default();
    for i in 0..70 {
        service.settle(&format!("sub-{i}"), &SubagentResult::Aborted);
    }
    // 最早的 sub-0/sub-1 已被挤出，最新的仍在。
    assert_eq!(service.result("sub-0"), None);
    assert_eq!(service.result("sub-1"), None);
    assert_eq!(service.result("sub-69"), Some(SubagentResult::Aborted));
}
