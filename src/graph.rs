//! Task graph: dependency anti-cycle guard + ready/blocked judgment.
//!
//! Port of compass-ws/dev/bin/cx/store.py graph engine.
//! Implemented in M1.5 (anti-cycle) / M1.6 (ready judgment).
//!
//! M1.5/M1.6: would_dep_cycle, ready.

use crate::task::{Task, TaskStatus};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Task, TaskKind, TaskStatus};
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
}
