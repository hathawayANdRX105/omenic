//! 首消息标题持久化的错误分类与重入资格回归测试。

use std::cell::Cell;

use omenic_web_client::ClientError;
use omenic_web_page_workspace::{retry_update, title_to_persist};

fn database_missing() -> ClientError {
    ClientError::Server {
        code: "database_missing".into(),
        message: "session not found".into(),
    }
}

#[test]
fn database_missing_once_then_success_retries_once() {
    let calls = Cell::new(0usize);
    let result = retry_update(|| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            Err(database_missing())
        } else {
            Ok(())
        }
    });

    assert!(result.is_ok());
    assert_eq!(calls.get(), 2);
}

#[test]
fn non_database_error_returns_without_retry() {
    let calls = Cell::new(0usize);
    let result = retry_update(|| {
        calls.set(calls.get() + 1);
        Err(ClientError::Protocol("bad response".into()))
    });

    assert!(result.is_err());
    assert_eq!(calls.get(), 1);
}

#[test]
fn repeated_database_missing_is_bounded() {
    let calls = Cell::new(0usize);
    let result = retry_update(|| {
        calls.set(calls.get() + 1);
        Err(database_missing())
    });

    assert!(result.is_err());
    assert_eq!(calls.get(), 2);
}

#[test]
fn later_send_retries_only_the_same_pending_title() {
    assert_eq!(
        title_to_persist(None, "会话 123", "派生标题", true).as_deref(),
        Some("派生标题")
    );
    assert_eq!(
        title_to_persist(Some("派生标题"), "派生标题", "第二条消息", false).as_deref(),
        Some("派生标题")
    );
    assert_eq!(
        title_to_persist(Some("派生标题"), "用户自定义标题", "第二条消息", false),
        None
    );
}
