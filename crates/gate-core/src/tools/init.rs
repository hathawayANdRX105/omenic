//! gate init — install / uninstall the gate binary and configure git hooks.

use std::fs;
use std::path::{Path, PathBuf};

use super::git;

const HOOK_PRE_COMMIT: &str = "\
#!/usr/bin/env bash
# gate-managed hook — delegates to the gate binary
exec gate pre-commit
";

const HOOK_PRE_PUSH: &str = "\
#!/usr/bin/env bash
# gate-managed hook — delegates to the gate binary
exec gate pre-push
";

const HOOK_MERGE: &str = "\
#!/usr/bin/env bash
# gate-managed hook — delegates to the gate binary
exec gate merge
";

/// `gate init` — copy current binary to `~/.local/bin/gate`, configure
/// `core.hooksPath`, and write hook templates to `.githooks/hooks/`.
pub fn install() -> anyhow::Result<()> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let install_dir = PathBuf::from(&home).join(".local").join("bin");
    let target = install_dir.join("gate");

    fs::create_dir_all(&install_dir)?;

    let current_exe = std::env::current_exe()?;
    // current_exe() canonicalizes symlinks; compare against canonical target
    // so a symlinked HOME/.local/bin doesn't mismatch and self-truncate.
    let already_installed = match fs::canonicalize(&target) {
        Ok(t) => t == current_exe,
        Err(_) => false,
    };
    if !already_installed {
        // Broken symlink or non-canonicalizable target: fs::copy would follow
        // the dangling link and leave it behind — remove it first so the real
        // binary replaces the link.
        if target
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            fs::remove_file(&target)?;
        }
        fs::copy(&current_exe, &target)?;
        chmod_755(&target);
    }

    // git config core.hooksPath .githooks/hooks
    let rc = std::process::Command::new("git")
        .args(["config", "core.hooksPath", ".githooks/hooks"])
        .status()?;
    if !rc.success() {
        anyhow::bail!("git config core.hooksPath failed");
    }

    write_hook_templates()?;

    println!("✓ Installed gate → {}", target.display());
    println!("  git hooksPath → .githooks/hooks");

    // Verify install
    let rc = std::process::Command::new(&target)
        .arg("--version")
        .output();
    if let Ok(o) = rc {
        if o.status.success() {
            println!("  ✓ {}", String::from_utf8_lossy(&o.stdout).trim());
        }
    }

    Ok(())
}

/// `gate init --uninstall` — remove `~/.local/bin/gate` and unset hooksPath.
pub fn uninstall() -> anyhow::Result<()> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let target = PathBuf::from(&home).join(".local").join("bin").join("gate");

    if target.exists() {
        fs::remove_file(&target)?;
        println!("✓ Removed {}", target.display());
    } else {
        println!("  Not installed: {}", target.display());
    }

    // Restore default hooksPath (ignore failure if not set).
    let _ = std::process::Command::new("git")
        .args(["config", "--unset", "core.hooksPath"])
        .status();

    println!("  git hooksPath unset");
    Ok(())
}

/// Write hook template shell stubs to `.githooks/hooks/`.
fn write_hook_templates() -> anyhow::Result<()> {
    let githooks = git::find_githooks_dir()
        .ok_or_else(|| anyhow::anyhow!("could not find .githooks directory"))?;
    let hooks_dir = githooks.join("hooks");
    fs::create_dir_all(&hooks_dir)?;

    write_template(&hooks_dir.join("pre-commit"), HOOK_PRE_COMMIT)?;
    write_template(&hooks_dir.join("pre-push"), HOOK_PRE_PUSH)?;
    write_template(&hooks_dir.join("merge"), HOOK_MERGE)?;

    println!("  hook templates: pre-commit, pre-push, merge");
    Ok(())
}

fn write_template(path: &Path, content: &str) -> anyhow::Result<()> {
    fs::write(path, content)?;
    chmod_755(path);
    Ok(())
}
fn chmod_755(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_templates_call_gate() {
        assert!(HOOK_PRE_COMMIT.contains("gate pre-commit"));
        assert!(HOOK_PRE_PUSH.contains("gate pre-push"));
        assert!(HOOK_MERGE.contains("gate merge"));
    }
}
