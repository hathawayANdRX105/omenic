use std::path::PathBuf;
use std::process::Command;

/// Locate the dioxus_components crate source so Tailwind can scan its utility
/// classes. The crate lives under CARGO_HOME/registry/src/<hash>/...; we walk the
/// registry cache and return the first matching `src` directory.
fn find_dioxus_components_src() -> Option<PathBuf> {
    let cargo_home = std::env::var("CARGO_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cargo")))
        .unwrap_or_default();
    let registry = PathBuf::from(cargo_home).join("registry/src");
    if let Ok(entries) = std::fs::read_dir(&registry) {
        for outer in entries.flatten() {
            let candidate = outer.path().join("dioxus_components-0.1.2/src");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Compile the Tailwind v4 input (`assets/tailwind-input.css`) into a static
/// stylesheet that is embedded into the binary via `include_str!` in `lib.rs`.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("assets/tailwind-input.css");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let output = out_dir.join("tailwind.gen.css");

    // Build the effective input: clone the project CSS, point its `@source` at an
    // absolute path, and add dioxus_components' `src` so its component utility
    // classes are scanned and emitted. Tailwind v4 resolves `@import "tailwindcss"`
    // from the INPUT FILE's directory hierarchy, so this generated input must live
    // inside the crate (under node_modules) — we drop it next to the real input
    // and it is gitignored.
    let mut css = std::fs::read_to_string(&input).unwrap_or_default();
    let my_src = manifest.join("src").to_string_lossy().replace('\\', "/");
    let my_src_directive = format!("@source \"{my_src}/**/*.rs\";");
    if let Some(idx) = css.find("@source") {
        // Replace the existing relative source directive with an absolute one.
        let line_end = css[idx..].find('\n').map(|e| idx + e).unwrap_or(css.len());
        css.replace_range(idx..line_end, &my_src_directive);
    } else {
        css.push_str(&format!("\n{my_src_directive}\n"));
    }
    if let Some(dc_src) = find_dioxus_components_src() {
        let dc = dc_src.to_string_lossy().replace('\\', "/");
        css.push_str(&format!("@source \"{dc}/**/*.rs\";\n"));
    }
    let gen_input = manifest.join(".tailwind.gen-input.css");
    let _ = std::fs::write(&gen_input, &css);

    let local_bin = manifest.join("node_modules/.bin/tailwindcss");

    // Run Tailwind from the crate root so `@import "tailwindcss"` resolves via the
    // crate's node_modules (the generated input lives in OUT_DIR, whose ancestors
    // do not reach node_modules).
    let ran = if local_bin.exists() {
        Command::new(&local_bin)
            .current_dir(&manifest)
            .args([
                "-i",
                gen_input.to_str().unwrap(),
                "-o",
                output.to_str().unwrap(),
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        false
    };

    if !ran {
        let _ = Command::new("npx")
            .current_dir(&manifest)
            .args([
                "--yes",
                "@tailwindcss/cli",
                "-i",
                gen_input.to_str().unwrap(),
                "-o",
                output.to_str().unwrap(),
            ])
            .status();
    }

    // Guarantee the include target exists so compilation never breaks even if
    // the Tailwind toolchain is unavailable (UI would simply be unstyled).
    if !output.exists() {
        let _ = std::fs::write(&output, "");
    }

    println!("cargo:rerun-if-changed=assets/tailwind-input.css");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");
}
