use std::path::PathBuf;
use std::process::Command;

/// Compile the Tailwind v4 input (`assets/tailwind-input.css`) into a static
/// stylesheet that is embedded into the binary via `include_str!` in `app.rs`.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("assets/tailwind-input.css");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let output = out_dir.join("tailwind.gen.css");

    // Build the effective input: clone the project CSS, point its first `@source`
    // (bin/web/src) at an absolute path, and keep the crates/web line so every
    // web crate's rsx classes are scanned. Tailwind v4 resolves
    // `@import "tailwindcss"` from the INPUT FILE's directory hierarchy, so this
    // generated input must live inside the crate — we drop it next to the real
    // input and it is gitignored.
    let mut css = std::fs::read_to_string(&input).unwrap_or_default();
    // Every `@source` glob must be absolute. Tailwind v4.3 resolves a relative
    // glob only when it carries an explicit `./` prefix, and resolves it
    // against the INPUT file's directory — which is the generated input in the
    // crate root, not assets/. A bare `../../..` glob silently matches nothing,
    // which is how the web crates' rsx classes fell out of the stylesheet.
    // Normalizing here makes the globs in assets/tailwind-input.css
    // depth-independent.
    let mut out = String::with_capacity(css.len());
    let mut saw_source = false;
    for line in css.split_inclusive('\n') {
        let Some(rest) = line.trim_start().strip_prefix("@source ") else {
            out.push_str(line);
            continue;
        };
        let Some((glob, tail)) = rest
            .split_once('"')
            .and_then(|(g, t)| Some((g, t.split_once('"')?)))
        else {
            out.push_str(line);
            continue;
        };
        saw_source = true;
        let normalized = glob.trim_start_matches("./");
        let abs = if normalized.starts_with('/') {
            normalized.to_string()
        } else {
            manifest
                .join(normalized)
                .to_string_lossy()
                .replace('\\', "/")
        };
        // tail.0 is the remainder of the glob inside the quotes, tail.1 the
        // closing quote plus the line's own newline.
        out.push_str(&format!("@source \"{abs}{}\"{}", tail.0, tail.1));
    }
    css = out;
    if !saw_source {
        let my_src = manifest.join("src").to_string_lossy().replace('\\', "/");
        css.push_str(&format!("\n@source \"{my_src}/**/*.rs\";\n"));
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
    // rsx classes live in the web crates; their edits must re-run this script.
    println!("cargo:rerun-if-changed=../../crates/web-ui");
}
