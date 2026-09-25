//! init / profile / template subcommand handlers, split out of `work.rs`.

use super::*;

use config::Config;
use store::store::Store;

pub fn init_cmd(json: bool) -> Result<u8, String> {
    let dir = std::env::current_dir().map_err(|e| format!("cwd error: {e}"))?;
    init_cmd_at(&dir, json)
}

/// init internals, testable with an explicit directory.
pub fn init_cmd_at(dir: &std::path::Path, json: bool) -> Result<u8, String> {
    let oi_dir = dir.join(".oi");
    let config_path = oi_dir.join("config.toml");
    let dir_existed = oi_dir.exists();
    let config_existed = config_path.exists();

    if !dir_existed {
        std::fs::create_dir_all(&oi_dir).map_err(|e| format!("could not create .oi/: {e}"))?;
    }
    if !config_existed {
        let default = "omp_path = \"omp\"\n\
                       data_dir = \"./.oi\"\n\
                       model = \"default\"\n";
        std::fs::write(&config_path, default)
            .map_err(|e| format!("could not create .oi/config.toml: {e}"))?;
    }
    // Spec templates (never overwrite user edits).
    store::specs::init::write_default_specs(&oi_dir).map_err(|e| format!("spec templates: {e}"))?;
    // Task templates (never overwrite user edits).
    store::template::write_default_templates(&oi_dir)
        .map_err(|e| format!("task templates: {e}"))?;
    let msg = match (dir_existed, config_existed) {
        (true, true) => "workspace already initialized",
        (false, false) => "initialized: .oi/, .oi/config.toml, .oi/specs/",
        (false, true) => "created: .oi/, .oi/specs/",
        (true, false) => "created: .oi/config.toml, .oi/specs/",
    };
    if json {
        json_ok(msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// Embedded boot/bundle profiles. Files live at the repo root `profiles/`;
/// embedded (not read from disk) so the binary works from any cwd.
const PROFILES: &[(&str, &str)] = &[
    ("boot", include_str!("../../../../../profiles/boot.toml")),
    (
        "bundle",
        include_str!("../../../../../profiles/bundle.toml"),
    ),
];

/// `profile list` -- 名字 + 文件首行描述。
pub fn profile_list_cmd(json: bool) -> Result<u8, String> {
    let rows: Vec<(&str, &str)> = PROFILES
        .iter()
        .map(|(name, body)| (*name, profile_blurb(body)))
        .collect();
    if json {
        let value: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, blurb)| serde_json::json!({ "name": name, "description": blurb }))
            .collect();
        print_json(&value);
    } else {
        for (name, blurb) in &rows {
            println!("{name}\t{blurb}");
        }
    }
    Ok(0)
}

/// 文件首条 `#` 注释即描述（ profiles 的约定：第一行写用途）。
pub fn profile_blurb(body: &str) -> &str {
    body.lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("")
        .trim()
}

/// `profile apply <name>` -- 把 profile 写进 `<dir>/.oi/config.toml`。
/// 与 `init` 同一语义：已存在则拒绝覆盖（返回非零），不动用户配置。
pub fn profile_apply_cmd_at(dir: &std::path::Path, name: &str, json: bool) -> Result<u8, String> {
    let Some((_, body)) = PROFILES.iter().find(|(n, _)| *n == name) else {
        return Err(format!(
            "unknown profile '{name}' (available: {})",
            PROFILES
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    let oi_dir = dir.join(".oi");
    let config_path = oi_dir.join("config.toml");
    if config_path.exists() {
        return Err(format!(
            "{} already exists; refusing to overwrite",
            config_path.display()
        ));
    }
    std::fs::create_dir_all(&oi_dir).map_err(|e| format!("could not create .oi/: {e}"))?;
    std::fs::write(&config_path, body)
        .map_err(|e| format!("could not write .oi/config.toml: {e}"))?;
    let msg = format!("applied profile '{name}': {}", config_path.display());
    if json {
        json_ok(&msg);
    } else {
        println!("{msg}");
    }
    Ok(0)
}

/// `ready` -- list open tasks whose deps are all done, sorted by priority then id.
pub fn template_list_cmd(json: bool) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let templates = store::template::load_all_templates(&config.data_dir)
        .map_err(|e| format!("template error: {e}"))?;
    if templates.is_empty() {
        eprintln!(
            "no templates in {} (run `cli init` first)",
            config.data_dir.join("templates").display()
        );
        return Ok(1);
    }
    if json {
        let list: Vec<serde_json::Value> = templates
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "kind": match t.kind {
                        store::template::TemplateKind::Phase => "phase",
                        store::template::TemplateKind::Step => "step",
                    },
                    "tasks": t.tasks.iter().map(|x| x.key.clone()).collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&list);
    } else {
        for t in &templates {
            let kind = match t.kind {
                store::template::TemplateKind::Phase => "phase",
                store::template::TemplateKind::Step => "step",
            };
            println!(
                "{kind}: {} — {}",
                t.name,
                t.tasks.first().map(|x| x.title.as_str()).unwrap_or("")
            );
            for x in &t.tasks {
                println!("  - {}", x.key);
            }
        }
    }
    Ok(0)
}

/// `template apply` subcommand: create topic + phase + steps from a template.
pub fn template_apply_cmd(
    store: &Store,
    name: &str,
    topic: &str,
    parent: Option<String>,
    json: bool,
) -> Result<u8, String> {
    let config = Config::load().map_err(|e| format!("config error: {e}"))?;
    let ids = store::template::apply(store, &config.data_dir, name, topic, parent)
        .map_err(|e| format!("template error: {e} — try `cli template list` or `cli init`"))?;
    if json {
        let obj = serde_json::json!({ "created": ids });
        print_json(&obj);
    } else {
        for id in &ids {
            println!("created {id}");
        }
    }
    Ok(0)
}
