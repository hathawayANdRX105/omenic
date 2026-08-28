//! Render blank skeleton from a loaded spec.

use super::Spec;

pub fn render_skeleton(spec: &Spec, title: &str) -> String {
    let mut out = String::new();
    if spec.name == "review" {
        out.push_str("## Agent 🤖 - CRG Review: <english-title>\n\n");
    } else {
        let t = if title.trim().is_empty() {
            "# <标题>".to_string()
        } else {
            format!("# {title}")
        };
        out.push_str(&format!("{t}\n\n"));
    }
    for f in &spec.fields {
        out.push_str(&format!("## {}\n", f.heading));
        if f.checkbox {
            out.push_str(&format!("<!-- {} -->\n- [ ]\n", f.hint));
        } else {
            out.push_str(&format!("<!-- {} -->\n\n", f.hint));
        }
        out.push('\n');
    }
    out
}
