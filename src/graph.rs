//! Task graph: dependency anti-cycle guard + ready/blocked judgment.
//!
//! Port of compass-ws/dev/bin/cx/store.py graph engine.
//! Implemented in M1.5 (anti-cycle) / M1.6 (ready judgment).
//!
//! M1.5/M1.6: would_dep_cycle, ready.

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn mk_tasks(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect()
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
