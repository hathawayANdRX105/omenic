//! Tests for SessionStore: append → load → delete, corrupt-line trim,
//! and list_sessions ordering.

use std::io::Write;

use tui::app::{ChatMsg, Role};
use tui::session::{SessionStore, list_sessions};

fn temp_data_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    (dir, path)
}

fn user_msg(text: &str) -> ChatMsg {
    ChatMsg {
        role: Role::User,
        text: text.into(),
    }
}

fn asst_msg(text: &str) -> ChatMsg {
    ChatMsg {
        role: Role::Assistant,
        text: text.into(),
    }
}

// --- append / load round-trip ---

#[test]
fn append_and_load_roundtrip() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-test1");

    let msgs = vec![
        user_msg("你好"),
        asst_msg("你好！有什么可以帮你的？"),
        user_msg("写一行 Rust"),
    ];

    for msg in &msgs {
        store.append_msg(msg).unwrap();
    }

    let loaded = store.load().unwrap();
    assert_eq!(loaded.len(), msgs.len());
    for (i, msg) in msgs.iter().enumerate() {
        assert_eq!(loaded[i].role, msg.role);
        assert_eq!(loaded[i].text, msg.text);
    }
}

#[test]
fn load_empty_returns_empty_vec() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-empty");

    let loaded = store.load().unwrap();
    assert!(loaded.is_empty());
}

#[test]
fn load_nonexistent_file_returns_empty() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-nope");

    assert!(!store.test_path().exists());
    let loaded = store.load().unwrap();
    assert!(loaded.is_empty());
}

// --- delete ---

#[test]
fn delete_removes_file() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-del");
    store.append_msg(&user_msg("x")).unwrap();
    assert_eq!(store.load().unwrap().len(), 1);

    SessionStore::delete(&data_dir, "s-del").unwrap();
    let store2 = SessionStore::open(&data_dir, "s-del");
    assert!(store2.load().unwrap().is_empty());
}

#[test]
fn delete_nonexistent_is_noop() {
    let (_guard, data_dir) = temp_data_dir();
    SessionStore::delete(&data_dir, "s-never-existed").unwrap();
}

// --- corrupt line handling ---

#[test]
fn corrupt_trailing_line_is_trimmed() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-corrupt");

    store.append_msg(&user_msg("msg1")).unwrap();
    store.append_msg(&asst_msg("msg2")).unwrap();

    let path = store.test_path().to_path_buf();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"not valid json\n")
        .unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(
        loaded.len(),
        2,
        "should keep 2 valid, trim corrupt trailing"
    );
    assert_eq!(loaded[0].text, "msg1");
    assert_eq!(loaded[1].text, "msg2");

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        !content.contains("not valid json"),
        "corrupt line should be trimmed from disk"
    );
}

#[test]
fn corrupt_middle_line_returns_error() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-corrupt-mid");

    store.append_msg(&user_msg("msg1")).unwrap();

    let path = store.test_path().to_path_buf();
    let content = std::fs::read_to_string(&path).unwrap();
    let corrupt = format!("{content}not valid json\n{{\"role\":\"user\",\"text\":\"msg3\"}}\n");
    std::fs::write(&path, corrupt).unwrap();

    let result = store.load();
    assert!(result.is_err(), "corrupt non-trailing line should error");
}

// --- list_sessions ---

#[test]
fn list_sessions_empty_dir() {
    let (_guard, data_dir) = temp_data_dir();
    let entries = list_sessions(&data_dir);
    assert!(entries.is_empty());
}

#[test]
fn list_sessions_ignores_non_jsonl() {
    let (_guard, data_dir) = temp_data_dir();
    let dir = data_dir.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s-1.jsonl"), b"").unwrap();
    std::fs::write(dir.join("readme.txt"), b"").unwrap();
    std::fs::write(dir.join("s-2.json"), b"").unwrap();

    let entries = list_sessions(&data_dir);
    assert_eq!(entries.len(), 1, "only .jsonl files should be listed");
    assert_eq!(entries[0].0, "s-1");
}

#[test]
fn list_sessions_returns_all_ids() {
    let (_guard, data_dir) = temp_data_dir();
    let dir = data_dir.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s-a.jsonl"), b"").unwrap();
    std::fs::write(dir.join("s-b.jsonl"), b"").unwrap();
    std::fs::write(dir.join("s-c.jsonl"), b"").unwrap();

    let entries = list_sessions(&data_dir);
    let ids: Vec<&str> = entries.iter().map(|(id, _mtime)| id.as_str()).collect();
    assert!(ids.contains(&"s-a"));
    assert!(ids.contains(&"s-b"));
    assert!(ids.contains(&"s-c"));
}

// --- UTF-8 round-trip ---

#[test]
fn unicode_text_roundtrip() {
    let (_guard, data_dir) = temp_data_dir();
    let store = SessionStore::open(&data_dir, "s-unicode");

    let text = "你好世界 🌍 こんにちは мир";
    store.append_msg(&user_msg(text)).unwrap();

    let loaded = store.load().unwrap();
    assert_eq!(loaded[0].text, text);
}
