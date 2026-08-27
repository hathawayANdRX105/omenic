//! Task graph: dependency anti-cycle guard + ready/blocked judgment.
//!
//! Port of compass-ws/dev/bin/cx/store.py graph engine.
//! Implemented in M1.5 (anti-cycle) / M1.6 (ready judgment).
//!
//! M1.5/M1.6: would_dep_cycle, ready.

use crate::{Task, TaskStatus};
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
