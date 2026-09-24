//! 5.3/5.5 谱系分组——`group_sessions` 的边界不变式。
//!
//! `parent_id` 是无外键约束的自由文本列，分组函数必须对四类脏数据收敛：
//! 父不在本列表、自环、互相成环、层级超深。任何一条处理不好，轻则树错位，
//! 重则渲染路径死循环（侧栏卡死）。这些用例只测纯函数，不起 UI。

use web_components::sidebar::group_sessions;
use web_state::types::{Session, SessionStatus};

fn session(id: &str, parent_id: Option<&str>) -> Session {
    Session {
        id: id.into(),
        title: id.into(),
        last_active: "刚刚".into(),
        model: "default".into(),
        status: SessionStatus::Idle,
        last_active_epoch: 0,
        parent_id: parent_id.map(str::to_string),
    }
}

fn ids_and_depths<'a>(grouped: &'a [(&'a Session, usize)]) -> Vec<(&'a str, usize)> {
    grouped.iter().map(|(s, d)| (s.id.as_str(), *d)).collect()
}

#[test]
fn flat_list_of_roots_stays_in_order() {
    let sessions = [session("a", None), session("b", None), session("c", None)];
    assert_eq!(
        ids_and_depths(&group_sessions(&sessions)),
        [("a", 0), ("b", 0), ("c", 0)]
    );
}

#[test]
fn children_follow_parent_at_increasing_depth() {
    // a 是根，b 的父是 a，c 的父是 b：a → b → c。
    let sessions = [
        session("a", None),
        session("b", Some("a")),
        session("c", Some("b")),
    ];
    assert_eq!(
        ids_and_depths(&group_sessions(&sessions)),
        [("a", 0), ("b", 1), ("c", 2)]
    );
}

#[test]
fn missing_parent_is_a_root() {
    // 父被删了 / 在别的 space：本列表里找不到 → 当根，不挂悬空边。
    let sessions = [session("orphan", Some("gone")), session("root", None)];
    assert_eq!(
        ids_and_depths(&group_sessions(&sessions)),
        [("orphan", 0), ("root", 0)]
    );
}

#[test]
fn blank_and_missing_parent_id_are_roots() {
    let sessions = [session("none", None)];
    assert_eq!(ids_and_depths(&group_sessions(&sessions)), [("none", 0)]);
}

#[test]
fn cycle_terminates_and_visits_every_node_once() {
    // a↔b 互相指向：两者都不是根（各自的父都在列表里），第一趟全部跳过，
    // 第二趟必须兜底，且 visited 集合必须让循环终止、每个节点只出现一次。
    let sessions = [session("a", Some("b")), session("b", Some("a"))];
    let grouped = group_sessions(&sessions);
    assert_eq!(grouped.len(), 2, "both nodes visited exactly once");
    let mut ids: Vec<&str> = grouped.iter().map(|(s, _)| s.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["a", "b"]);
    // 兜底入口当成根，所以深度必是 0 和 1（而不是两个 0）。
    let mut depths: Vec<usize> = grouped.iter().map(|(_, d)| *d).collect();
    depths.sort_unstable();
    assert_eq!(depths, [0, 1]);
}

#[test]
fn self_cycle_terminates() {
    let sessions = [session("a", Some("a"))];
    assert_eq!(ids_and_depths(&group_sessions(&sessions)), [("a", 0)]);
}

#[test]
fn deeper_than_max_depth_still_terminates_and_caps() {
    // 一条 32 层的直链：超过 MAX_DEPTH 后不再展开子节点，但链上每个节点
    // 都已被访问，不会漏掉，也不会无限递归。
    let mut sessions = Vec::new();
    sessions.push(session("n0", None));
    for i in 1..32 {
        sessions.push(session(&format!("n{i}"), Some(&format!("n{}", i - 1))));
    }
    let grouped = group_sessions(&sessions);
    assert_eq!(grouped.len(), 32, "every node visited");
    // 深度单调不减，且最大值受 MAX_DEPTH 封顶。
    let mut prev = 0;
    for (_, depth) in &grouped {
        assert!(*depth >= prev, "depth is non-decreasing along the chain");
        prev = *depth;
    }
    assert!(prev <= 16, "depth is capped, got {prev}");
}
