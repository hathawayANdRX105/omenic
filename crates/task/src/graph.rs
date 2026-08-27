//! Task graph: dependency anti-cycle guard + ready/blocked judgment.
//!
//! Port of compass-ws/dev/bin/cx/store.py graph engine.
//! Implemented in M1.5 (anti-cycle) / M1.6 (ready judgment).
//!
//! M1.5/M1.6: would_dep_cycle, ready.

use crate::model::{Task, TaskStatus};
use std::collections::{HashMap, HashSet};

/// Returns true if adding a dependency edge `task` → `depends_on` would create a cycle.
///
/// # Arguments
/// * `tasks` - Map of task_id → its dependencies (outgoing edges: task depends on these)
/// * `task` - The source task (where the new edge originates)
/// * `depends_on` - The target task (where the new edge points to)
///
/// # Algorithm
/// A cycle forms iff `depends_on` can already reach `task` in the current graph.
/// We run DFS from `depends_on` following dependency edges (i.e., "what does this task depend on?").
/// If we reach `task`, adding `task` → `depends_on` would close the loop.
#[allow(dead_code)] // consumed by store/CLI in M1.8
/// Self-dependency (`task == depends_on`) is also a cycle.
pub fn would_dep_cycle(tasks: &HashMap<String, Vec<String>>, task: &str, depends_on: &str) -> bool {
    // Self-cycle check
    if task == depends_on {
        return true;
    }

    // DFS from depends_on to see if we can reach task
    let mut visited = HashSet::new();
    let mut stack = vec![depends_on.to_string()];

    while let Some(current) = stack.pop() {
        if current == task {
            return true;
        }
        if visited.insert(current.clone()) {
            // Follow dependencies: current depends on these, so we can traverse to them
            if let Some(deps) = tasks.get(&current) {
                for dep in deps {
                    if !visited.contains(dep) {
                        stack.push(dep.clone());
                    }
                }
            }
        }
    }

    false
}

/// A task is ready if:
/// - it exists in the task map
/// - its status is Done (already completed, trivially ready)
/// - it has no dependencies (deps empty)
/// - all its dependencies exist and have status Done
#[allow(dead_code)] // consumed by store/CLI in M1.8
pub fn is_ready(tasks: &HashMap<String, Task>, task_id: &str) -> bool {
    let Some(task) = tasks.get(task_id) else {
        return false;
    };

    // Already done -> ready (trivially)
    if task.status == TaskStatus::Done {
        return true;
    }

    // No dependencies -> ready
    if task.deps.is_empty() {
        return true;
    }

    // All dependencies must exist and be Done
    for dep_id in &task.deps {
        let Some(dep) = tasks.get(dep_id) else {
            return false; // dependency not found
        };
        if dep.status != TaskStatus::Done {
            return false; // dependency not done
        }
    }

    true
}

/// Returns ids of tasks that have `parent_id` as their parent.
pub fn children_of(tasks: &[Task], parent_id: &str) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| t.parent.as_deref() == Some(parent_id))
        .map(|t| t.id.clone())
        .collect()
}

/// Returns ids of tasks that list `dep_id` in their deps.
pub fn dependents(tasks: &[Task], dep_id: &str) -> Vec<String> {
    tasks
        .iter()
        .filter(|t| t.deps.iter().any(|d| d == dep_id))
        .map(|t| t.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Task, TaskKind, TaskStatus};
    use std::collections::HashMap;

    fn mk_tasks(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn mk_task(id: &str, deps: &[&str], status: TaskStatus) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            kind: TaskKind::Task,
            status,
            attempts: 0,
            priority: 2,
            parent: None,
            deps: deps.iter().map(|s| s.to_string()).collect(),
            description: String::new(),
            acceptance: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn mk_tasks_full(pairs: &[(&str, &[&str], TaskStatus)]) -> HashMap<String, Task> {
        pairs
            .iter()
            .map(|(k, v, s)| (k.to_string(), mk_task(k, v, s.clone())))
            .collect()
    }

    #[test]
    fn no_deps_ready() {
        let tasks = mk_tasks_full(&[("a", &[], TaskStatus::Open)]);
        assert!(is_ready(&tasks, "a"));
    }

    #[test]
    fn all_deps_done() {
        let tasks = mk_tasks_full(&[
            ("a", &["b"], TaskStatus::Open),
            ("b", &[], TaskStatus::Done),
        ]);
        assert!(is_ready(&tasks, "a"));
    }

    #[test]
    fn dep_not_done() {
        let tasks = mk_tasks_full(&[
            ("a", &["b"], TaskStatus::Open),
            ("b", &[], TaskStatus::Open),
        ]);
        assert!(!is_ready(&tasks, "a"));
    }

    #[test]
    fn multiple_deps_partial() {
        let tasks = mk_tasks_full(&[
            ("a", &["b", "c"], TaskStatus::Open),
            ("b", &[], TaskStatus::Done),
            ("c", &[], TaskStatus::Open),
        ]);
        assert!(!is_ready(&tasks, "a"));
    }

    #[test]
    fn multiple_deps_all_done() {
        let tasks = mk_tasks_full(&[
            ("a", &["b", "c"], TaskStatus::Open),
            ("b", &[], TaskStatus::Done),
            ("c", &[], TaskStatus::Done),
        ]);
        assert!(is_ready(&tasks, "a"));
    }

    #[test]
    fn task_not_found() {
        let tasks = mk_tasks_full(&[("a", &[], TaskStatus::Open)]);
        assert!(!is_ready(&tasks, "nonexistent"));
    }

    #[test]
    fn self_done() {
        let tasks = mk_tasks_full(&[("a", &[], TaskStatus::Done)]);
        assert!(is_ready(&tasks, "a"));
    }

    #[test]
    fn self_cycle() {
        let tasks = HashMap::new();
        assert!(would_dep_cycle(&tasks, "a", "a"));
    }

    #[test]
    fn direct_cycle() {
        // a -> b, check if adding b -> a creates cycle
        let tasks = mk_tasks(&[("a", &["b"]), ("b", &[])]);
        assert!(would_dep_cycle(&tasks, "b", "a"));
    }

    #[test]
    fn transitive_cycle() {
        // a -> b -> c, check if adding c -> a creates cycle
        let tasks = mk_tasks(&[("a", &["b"]), ("b", &["c"]), ("c", &[])]);
        assert!(would_dep_cycle(&tasks, "c", "a"));
    }

    #[test]
    fn no_cycle() {
        // a -> b -> c, adding d -> a should not create cycle
        let tasks = mk_tasks(&[("a", &["b"]), ("b", &["c"])]);
        assert!(!would_dep_cycle(&tasks, "d", "a"));
    }

    #[test]
    fn disconnected() {
        // a -> b, x and y don't exist in graph
        let tasks = mk_tasks(&[("a", &["b"])]);
        assert!(!would_dep_cycle(&tasks, "x", "y"));
    }

    fn mk_task_parent(id: &str, parent: &str, status: TaskStatus) -> Task {
        let mut t = mk_task(id, &[], status);
        t.parent = Some(parent.to_string());
        t
    }

    #[test]
    fn children_of_finds_children() {
        let tasks = vec![
            mk_task_parent("a", "p1", TaskStatus::Open),
            mk_task("b", &[], TaskStatus::Open),
            mk_task_parent("c", "p1", TaskStatus::Open),
            mk_task_parent("d", "p2", TaskStatus::Open),
        ];
        let children = children_of(&tasks, "p1");
        assert_eq!(children.len(), 2);
        assert!(children.contains(&"a".to_string()));
        assert!(children.contains(&"c".to_string()));
    }

    #[test]
    fn dependents_finds_dependents() {
        let tasks = vec![
            mk_task("a", &[], TaskStatus::Open),
            mk_task("b", &["a"], TaskStatus::Open),
            mk_task("c", &["a"], TaskStatus::Open),
            mk_task("d", &["x"], TaskStatus::Open),
        ];
        let deps = dependents(&tasks, "a");
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"b".to_string()));
        assert!(deps.contains(&"c".to_string()));
    }

    #[test]
    fn deep_transitive_cycle_five_levels() {
        // #43: chain a→b→c→d→e; closing e→a must be detected across 4 hops.
        let tasks = mk_tasks(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &["d"]),
            ("d", &["e"]),
            ("e", &[]),
        ]);
        assert!(would_dep_cycle(&tasks, "e", "a"));
        // The reverse edge a→e is legal: e cannot reach a.
        assert!(!would_dep_cycle(&tasks, "a", "e"));
    }

    #[test]
    fn deep_diamond_closes_cycle() {
        // #43: diamond a→{b,c}, b,c→e. e already reaches a (via both arms),
        // so adding e→a closes the cycle — and b→a / c→a would too.
        let tasks = mk_tasks(&[("a", &["b", "c"]), ("b", &["e"]), ("c", &["e"]), ("e", &[])]);
        assert!(would_dep_cycle(&tasks, "e", "a"));
        assert!(would_dep_cycle(&tasks, "b", "a"));
        // But an unrelated new root stays legal.
        assert!(!would_dep_cycle(&tasks, "x", "a"));
    }

    #[test]
    fn is_ready_missing_dependency() {
        // #43: dep id absent from the map → never ready.
        let tasks = mk_tasks_full(&[("a", &["ghost"], TaskStatus::Open)]);
        assert!(!is_ready(&tasks, "a"));
    }

    #[test]
    fn is_ready_failed_dep_not_ready() {
        // #47 interplay: a Failed dep is not Done → dependents stay gated.
        let tasks = mk_tasks_full(&[
            ("a", &[], TaskStatus::Failed),
            ("b", &["a"], TaskStatus::Open),
        ]);
        assert!(!is_ready(&tasks, "b"));
    }
}
