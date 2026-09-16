//! Logic tests for `tools::truncate_output` spill-file naming.
//!
//! These write into the real spill dir (`tools::SPILL_DIR`, i.e. /tmp) because
//! that is where the function under test writes; every file a test creates is
//! removed by the test that created it, so nothing is left behind.

use std::fs;
use std::process::id;

use tools::truncate_output;

/// Build `n` lines whose every line carries `mark`, so content is uniquely
/// identifiable after a round trip through a spill file.
fn marked_lines(n: usize, mark: &str) -> String {
    (0..n)
        .map(|i| format!("{mark}-{i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pull the spill path out of a truncated-output message.
///
/// Message shape: `[output truncated: ... full output: /tmp/oi-output-...]\n...`
fn spill_path_from(msg: &str) -> std::path::PathBuf {
    let start = msg
        .find("full output: ")
        .expect("truncated message must carry a spill path")
        + "full output: ".len();
    let rest = &msg[start..];
    let end = rest.find(']').expect("spill path must be terminated by ]");
    std::path::PathBuf::from(&rest[..end])
}

#[test]
fn short_output_is_returned_untouched() {
    // 200 lines sits exactly at the cap: it must come back verbatim and spill
    // nothing.
    let input = marked_lines(200, "SHORT");
    let out = truncate_output(&input).expect("truncate succeeds");
    assert_eq!(out, input, "output at the line cap must not be truncated");
    assert!(
        !out.contains("full output:"),
        "no spill path should appear in an untruncated result"
    );

    // No spill happened: no file in the spill dir holds our unique marker.
    let mut spilled = false;
    if let Ok(entries) = fs::read_dir("/tmp") {
        for entry in entries.flatten() {
            if fs::read_to_string(entry.path()).is_ok_and(|contents| contents.contains("SHORT-199"))
            {
                spilled = true;
            }
        }
    }
    assert!(
        !spilled,
        "short output must not have been written to a spill file"
    );
}

#[test]
fn two_overflows_do_not_collide() {
    let msg_a = truncate_output(&marked_lines(300, "MARK-A")).expect("truncate A succeeds");
    let msg_b = truncate_output(&marked_lines(300, "MARK-B")).expect("truncate B succeeds");

    // The tail that is kept back is the last 200 of 300 lines: marks 100..=299
    // survive, mark 99 is dropped.
    assert!(msg_a.contains("MARK-A-299"), "result keeps the last lines");
    assert!(
        !msg_a.contains("MARK-A-99"),
        "result must drop lines before the last {}",
        tools::MAX_OUTPUT_LINES
    );

    let path_a = spill_path_from(&msg_a);
    let path_b = spill_path_from(&msg_b);
    assert_ne!(
        path_a, path_b,
        "two overflows must not share one spill filename"
    );

    let file_a = fs::read_to_string(&path_a).expect("spill file A must exist");
    let file_b = fs::read_to_string(&path_b).expect("spill file B must exist");
    assert!(file_a.contains("MARK-A-299"), "spill A keeps A's full text");
    assert!(
        !file_a.contains("MARK-B-299"),
        "spill A must not have been overwritten by B"
    );
    assert!(file_b.contains("MARK-B-299"), "spill B keeps B's full text");
    assert!(
        !file_b.contains("MARK-A-299"),
        "spill B must not have been overwritten by A"
    );

    // Don't leave test artifacts behind in the spill dir.
    let _ = fs::remove_file(path_a);
    let _ = fs::remove_file(path_b);
}

#[test]
fn spill_file_name_encodes_pid_and_rising_seq() {
    let prefix = format!("oi-output-{}-", id());
    let msg_a = truncate_output(&marked_lines(300, "NAME-A")).expect("truncate succeeds");
    let msg_b = truncate_output(&marked_lines(300, "NAME-B")).expect("truncate succeeds");

    let name_a = spill_path_from(&msg_a)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let name_b = spill_path_from(&msg_b)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    assert!(
        name_a.starts_with(&prefix),
        "{name_a} should start with {prefix}"
    );
    assert!(
        name_b.starts_with(&prefix),
        "{name_b} should start with {prefix}"
    );

    // The segment after the pid is the sequence number, and it rises.
    let seq_a: u64 = name_a[prefix.len()..]
        .trim_end_matches(".txt")
        .parse()
        .expect("segment after pid must be numeric");
    let seq_b: u64 = name_b[prefix.len()..]
        .trim_end_matches(".txt")
        .parse()
        .expect("segment after pid must be numeric");
    assert!(
        seq_b > seq_a,
        "sequence must rise between calls: {seq_a} -> {seq_b}"
    );

    let _ = fs::remove_file(spill_path_from(&msg_a));
    let _ = fs::remove_file(spill_path_from(&msg_b));
}
