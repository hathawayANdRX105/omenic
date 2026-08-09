//! JSONL append-only task store: fcntl lock + latest-wins + atomic compact.
//!
//! Port of compass-ws/dev/bin/cx/store.py.
//! Implemented in M1.3 (append-only) / M1.4 (compact).

// M1.3/M1.4: create/update/read_all/compact.
