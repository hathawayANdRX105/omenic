use std::path::PathBuf;
use std::process::Command;

/// Compile the Tailwind v4 input (`assets/tailwind-input.css`) into a static
/// stylesheet that is embedded into the binary via `include_str!` in `lib.rs`.
fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest.join("assets/tailwind-input.css");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let output = out_dir.join("tailwind.gen.css");

    let local_bin = manifest.join("node_modules/.bin/tailwindcss");

    let ran = if local_bin.exists() {
        Command::new(&local_bin)
            .args([
                "-i",
                input.to_str().unwrap(),
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
            .args([
                "--yes",
                "@tailwindcss/cli",
                "-i",
                input.to_str().unwrap(),
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
}
