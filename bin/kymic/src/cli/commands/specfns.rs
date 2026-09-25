//! Spec subcommand handlers, split out of `work.rs`.

use super::*;

use config::Config;

/// `spec list` subcommand: list spec tables loaded from `<data>/specs/`.
pub fn spec_list_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let specs = spec::template::parse::load_all_specs(&config.data_dir)
        .map_err(|e| format!("spec error: {e}"))?;
    if specs.is_empty() {
        eprintln!(
            "no spec templates in {} (run `cli init` first)",
            config.data_dir.join("specs").display()
        );
        return Ok(1);
    }
    if json {
        let list: Vec<serde_json::Value> = specs
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "description": s.description,
                    "fields": s.fields.iter().map(|f| {
                        serde_json::json!({
                            "heading": f.heading,
                            "required": f.required,
                            "checkbox": f.checkbox,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&list);
    } else {
        for s in &specs {
            println!("{} — {}", s.name, s.description);
            for f in &s.fields {
                let mark = if f.required { "req" } else { "opt" };
                let cb = if f.checkbox { " [checkbox]" } else { "" };
                println!("  {mark}: ## {}{cb}", f.heading);
            }
        }
    }
    Ok(0)
}

/// `spec new` subcommand: generate a blank spec table skeleton.
pub fn spec_new_cmd(
    kind: &str,
    title: Option<String>,
    output: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let spec = spec::template::parse::load_spec(&config.data_dir, kind)
        .map_err(|e| format!("spec error: {e} — try `cli spec list` or `cli init`"))?;
    let doc = spec::template::render::render_skeleton(&spec, title.as_deref().unwrap_or(""));
    match output {
        Some(path) => {
            std::fs::write(&path, &doc).map_err(|e| format!("write {}: {e}", path))?;
            let msg = format!("spec skeleton written to {path}");
            if json {
                json_ok(&msg);
            } else {
                println!("{msg}");
            }
        }
        None => {
            use std::io::Write;
            print!("{doc}");
            std::io::stdout().flush().ok();
        }
    }
    Ok(0)
}

/// `spec check` subcommand: validate a filled document against the kind's rules.
pub fn spec_check_cmd(kind: &str, file: &str, json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let spec = spec::template::parse::load_spec(&config.data_dir, kind)
        .map_err(|e| format!("spec error: {e} — try `cli spec list` or `cli init`"))?;
    let findings = spec::template::check::check_file(&spec, std::path::Path::new(file))?;
    let fails: Vec<_> = findings.iter().filter(|f| f.fail).collect();
    if json {
        let obj = serde_json::json!({
            "spec": spec.name,
            "file": file,
            "ok": fails.is_empty(),
            "findings": findings.iter().map(|f| serde_json::json!({
                "rule": f.rule,
                "fail": f.fail,
                "message": f.message,
            })).collect::<Vec<_>>(),
        });
        print_json(&obj);
    } else {
        for f in &findings {
            let tag = if f.fail { "FAIL" } else { "ok  " };
            println!("{tag} [{}] {}", f.rule, f.message);
        }
        if fails.is_empty() {
            println!("RESULT: ALL PASS");
        } else {
            println!("RESULT: FAIL ({} issue(s))", fails.len());
        }
    }
    Ok(if fails.is_empty() { 0 } else { 1 })
}

/// `spec view` subcommand: print a spec document for the agent to inspect.
pub fn spec_view_cmd(file: &str) -> Result<u8, String> {
    let doc = std::fs::read_to_string(file).map_err(|e| format!("read {}: {e}", file))?;
    use std::io::Write;
    print!("{doc}");
    std::io::stdout().flush().ok();
    Ok(0)
}
