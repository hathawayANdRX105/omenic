//! Pure task-graph renderers (no store I/O), split out of `work.rs`.

use store::{Task, TaskStatus};

pub fn status_glyph(t: &Task, map: &std::collections::HashMap<String, Task>) -> &'static str {
    match t.status {
        TaskStatus::Done => "✓",
        TaskStatus::InProgress => "◐",
        TaskStatus::Failed => "✗",
        TaskStatus::Open => {
            if store::graph::is_ready(map, &t.id) {
                "○"
            } else {
                "●"
            }
        }
    }
}

/// cx task_line format: `<icon> <id> ● P<priority> <title>`.
pub fn task_line(t: &Task, map: &std::collections::HashMap<String, Task>) -> String {
    format!(
        "{} {} ● P{} {}",
        status_glyph(t, map),
        t.id,
        t.priority,
        t.title
    )
}

/// Status legend footer, matching bd/cx output conventions.
pub fn status_legend() -> &'static str {
    "Status: ○ open  ◐ in_progress  ✗ failed  ● blocked  ✓ done\n"
}

/// Render the task tree as an indented plan view (roots first, children
/// nested under their parent with box-drawing prefixes), cx/bd style.
///
/// Tasks whose `parent` is missing (dangling) or `None` are treated as roots.
/// A visited set guards against parent cycles in malformed stores.
pub fn render_plan(tasks: &[Task]) -> String {
    use std::collections::{HashMap, HashSet};

    if tasks.is_empty() {
        return "(no tasks)\n".to_string();
    }

    let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();

    let mut children: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut roots: Vec<&Task> = Vec::new();
    for t in tasks {
        match t.parent.as_deref() {
            Some(p) if ids.contains(p) => children.entry(p).or_default().push(t),
            _ => roots.push(t),
        }
    }

    /// Stable topological order of a sibling list: if `a` depends on `b`
    /// (both in the list), `b` renders before `a`. Kahn's algorithm with
    /// declaration order as the tie-breaker (sibling chain from #216).
    fn topo_sort<'a>(kids: &[&'a Task], map: &HashMap<String, Task>) -> Vec<&'a Task> {
        use std::collections::HashMap as H;
        let ids: std::collections::HashSet<&str> = kids.iter().map(|k| k.id.as_str()).collect();
        let mut indeg: H<&str, usize> = H::new();
        let mut deps_map: H<&str, Vec<&str>> = H::new();
        let mut order: H<&str, usize> = H::new();
        for (i, k) in kids.iter().enumerate() {
            indeg.insert(k.id.as_str(), 0);
            order.insert(k.id.as_str(), i);
        }
        for k in kids {
            for d in &k.deps {
                if ids.contains(d.as_str()) && d != &k.id {
                    deps_map.entry(d.as_str()).or_default().push(k.id.as_str());
                    *indeg.get_mut(k.id.as_str()).unwrap() += 1;
                }
            }
        }
        let mut queue: Vec<&Task> = kids
            .iter()
            .filter(|k| indeg[k.id.as_str()] == 0)
            .copied()
            .collect();
        let mut out = Vec::new();
        while !queue.is_empty() {
            queue.sort_by_key(|k| order[k.id.as_str()]);
            let n = queue.remove(0);
            out.push(n);
            if let Some(ms) = deps_map.get(n.id.as_str()).cloned() {
                for m in ms {
                    let e = indeg.get_mut(m).unwrap();
                    *e -= 1;
                    if *e == 0
                        && let Some(t) = kids.iter().find(|k| k.id == m)
                    {
                        queue.push(t);
                    }
                }
            }
        }
        let _ = map; // reserved: ready/blocked glyph already computed by task_line
        out
    }

    fn print_children(
        parent: &Task,
        children: &HashMap<&str, Vec<&Task>>,
        map: &HashMap<String, Task>,
        prefix: &str,
        visited: &mut HashSet<String>,
        out: &mut String,
    ) {
        let Some(kids) = children.get(parent.id.as_str()) else {
            return;
        };
        for (i, kid) in topo_sort(kids, map).iter().enumerate() {
            let is_last = i == kids.len() - 1;
            let branch = if is_last { "└─ " } else { "├─ " };
            // Mark before printing so a cycle back-edge is skipped, not re-printed.
            if !visited.insert(kid.id.clone()) {
                continue;
            }
            out.push_str(&format!("{prefix}{branch}{}\n", task_line(kid, map)));
            let next_prefix = format!("{prefix}{}", if is_last { "   " } else { "│  " });
            print_children(kid, children, map, &next_prefix, visited, out);
        }
    }

    let mut visited: HashSet<String> = HashSet::new();
    let mut out = String::new();
    // Root tasks first; a visited guard prevents cycles from re-printing.
    for root in &roots {
        if !visited.insert(root.id.clone()) {
            continue;
        }
        out.push_str(&format!("{}\n", task_line(root, &map)));
        print_children(root, &children, &map, "", &mut visited, &mut out);
    }
    // Fallback: tasks in a pure parent-cycle (no root exists) still show once.
    for t in tasks {
        if !visited.insert(t.id.clone()) {
            continue;
        }
        out.push_str(&format!("{}\n", task_line(t, &map)));
        print_children(t, &children, &map, "", &mut visited, &mut out);
    }
    out.push_str(status_legend());
    out
}

/// Return ids of tasks newly unblocked by completing `done_id`: status Open,
/// all deps Done (via `is_ready`), and `done_id` listed among their deps.
pub fn suggest_next(tasks: &[Task], done_id: &str) -> Vec<String> {
    use std::collections::HashMap;
    let map: HashMap<String, Task> = tasks.iter().map(|t| (t.id.clone(), t.clone())).collect();
    tasks
        .iter()
        .filter(|t| {
            t.status == TaskStatus::Open
                && t.deps.iter().any(|d| d == done_id)
                && store::graph::is_ready(&map, &t.id)
        })
        .map(|t| t.id.clone())
        .collect()
}

/// Render the task graph as Graphviz DOT.
///
/// Dependency edges are solid (`dep -> task`); parent→child edges are dotted
/// with `arrowhead=none`. Nodes are colored by status.
pub fn render_dot(tasks: &[Task]) -> String {
    if tasks.is_empty() {
        return "digraph omenic {\n}\n".to_string();
    }

    fn status_color(s: &TaskStatus) -> &'static str {
        match s {
            TaskStatus::Open => "#e8f4fd",
            TaskStatus::InProgress => "#fff3cd",
            TaskStatus::Failed => "#f8d7da",
            TaskStatus::Done => "#d4edda",
        }
    }

    let mut out = String::new();
    out.push_str("digraph omenic {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=box, style=\"rounded,filled\"];\n");

    // Nodes
    for t in tasks {
        let color = status_color(&t.status);
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        let esc_title = t.title.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "  \"{esc_id}\" [label=\"{esc_id}\\nP{} | {esc_title}\", fillcolor=\"{color}\"];\n",
            t.priority
        ));
    }

    // Dependency edges (solid): dep -> task
    for t in tasks {
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        for dep in &t.deps {
            let esc_dep = dep.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("  \"{esc_dep}\" -> \"{esc_id}\";\n"));
        }
    }

    // Parent → child edges (dotted, no arrowhead)
    for t in tasks {
        let esc_id = t.id.replace('\\', "\\\\").replace('"', "\\\"");
        if let Some(parent) = &t.parent {
            let esc_parent = parent.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!(
                "  \"{esc_parent}\" -> \"{esc_id}\" [style=dotted, arrowhead=none];\n"
            ));
        }
    }

    out.push_str("}\n");
    out
}
