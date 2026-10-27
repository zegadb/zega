//! APS 54 phase 1: the same durability behaviour on both stores. Every
//! scenario is one function taking a [`Storage`]; each runs on the file log
//! and on the SQLite log, and the last tests compare the two.

#![cfg(feature = "sqlite")]

use serde_json::Value as Json;
use zega::{Checkpoint, Storage, Zega};

const PEOPLE: &str = r#"schema { type P { name: String friend: KNOWS -> P[] } } unique { P { name } }"#;

fn rows(result: Json) -> Vec<Json> {
    match result {
        Json::Array(rows) => rows,
        Json::Null => Vec::new(),
        row => vec![row],
    }
}

fn names(zega: &Zega) -> Vec<String> {
    let mut names: Vec<String> = rows(zega.run_lang(PEOPLE, "query { P { name } }").unwrap())
        .into_iter()
        .map(|row| row["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

fn open(path: &str, storage: Storage) -> Zega {
    Zega::open(path).storage(storage).wal_flush_every_write().build().unwrap()
}

fn write(zega: &Zega, mutation: &str) {
    zega.run_lang(PEOPLE, mutation).unwrap();
}

/// Writes, a multi-clause statement, a refused statement, then a reopen:
/// what the graph held before the reopen is exactly what it holds after.
fn writes_survive_reopen(storage: Storage) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let before = {
        let zega = open(path, storage);
        write(&zega, r#"mutation { P(name: "a") }"#);
        write(&zega, r#"mutation { P(name: "b") }"#);
        write(&zega, r#"mutation { P(name: "b") { friend -> link P(name: "a") { name } } }"#);
        // Refused, and nothing of either is written: a duplicate of a
        // unique name, and a link to a P that does not exist.
        assert!(zega.run_lang(PEOPLE, r#"mutation { P(name: "a") }"#).is_err());
        assert!(zega.run_lang(PEOPLE, r#"mutation { P(name: "b") { friend -> link P(name: "zzz") { name } } }"#).is_err());
        zega.snapshot_bytes().unwrap()
    };
    let zega = open(path, storage);
    assert_eq!(names(&zega), ["a", "b"]);
    let after = zega.snapshot_bytes().unwrap();
    assert_eq!(after, before, "{storage:?}: reopen changed the graph");
    after
}

/// A checkpoint, writes after it, and a reopen that replays only those.
fn checkpoint_then_writes(storage: Storage) -> (Checkpoint, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let checkpoint = {
        let zega = open(path, storage);
        write(&zega, r#"mutation { P(name: "before") }"#);
        let checkpoint = zega.checkpoint().unwrap().expect("a disk database checkpoints");
        write(&zega, r#"mutation { P(name: "after") }"#);
        checkpoint
    };
    let zega = open(path, storage);
    assert_eq!(names(&zega), ["after", "before"]);
    // A second checkpoint rotates the log the first one left.
    zega.checkpoint().unwrap().unwrap();
    drop(zega);
    let zega = open(path, storage);
    assert_eq!(names(&zega), ["after", "before"]);
    (checkpoint, zega.snapshot_bytes().unwrap())
}

/// The store a directory was created with is the only one that opens it.
fn other_storage_is_refused(created: Storage, other: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    write(&open(path, created), r#"mutation { P(name: "a") }"#);
    let refused = Zega::open(path).storage(other).build().err().expect("opened with the other storage");
    assert!(refused.to_string().contains("keeps its log in"), "{refused}");
    assert_eq!(names(&open(path, created)), ["a"]);
}

#[test]
fn file_writes_survive_reopen() {
    writes_survive_reopen(Storage::File);
}

#[test]
fn sqlite_writes_survive_reopen() {
    writes_survive_reopen(Storage::Sqlite);
}

#[test]
fn file_checkpoint_then_writes() {
    checkpoint_then_writes(Storage::File);
}

#[test]
fn sqlite_checkpoint_then_writes() {
    checkpoint_then_writes(Storage::Sqlite);
}

#[test]
fn a_file_directory_refuses_sqlite() {
    other_storage_is_refused(Storage::File, Storage::Sqlite);
}

#[test]
fn a_sqlite_directory_refuses_file() {
    other_storage_is_refused(Storage::Sqlite, Storage::File);
}

/// The same writes give the same graph, byte for byte, on both stores.
#[test]
fn both_stores_hold_the_same_graph() {
    assert_eq!(writes_survive_reopen(Storage::Sqlite), writes_survive_reopen(Storage::File));
    assert_eq!(checkpoint_then_writes(Storage::Sqlite).1, checkpoint_then_writes(Storage::File).1);
}

/// Log positions are file-log bytes on both stores, so checkpoint sizing
/// (`snapshot_every`) means the same whichever one a database uses.
#[test]
fn both_stores_report_the_same_log_positions() {
    let (sqlite, _) = checkpoint_then_writes(Storage::Sqlite);
    let (file, _) = checkpoint_then_writes(Storage::File);
    assert_eq!(
        (sqlite.wal_bytes_before, sqlite.wal_bytes_after),
        (file.wal_bytes_before, file.wal_bytes_after)
    );
    assert!(sqlite.wal_bytes_before > sqlite.wal_bytes_after, "the checkpoint shortened the log");
}
