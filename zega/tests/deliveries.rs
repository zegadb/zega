//! APS 52: `Zega::deliver`. A document applied once per (source, txid),
//! its receipt durable with its writes, on both stores.

#![cfg(feature = "sqlite")]

use serde_json::Value as Json;
use zega::{Storage, Zega, ZegaError};

const P: &str = "schema { type P { name: String } } unique { P { name } }";

fn doc(names: &[&str]) -> String {
    let mut doc = P.to_string();
    for name in names {
        doc.push_str(&format!("\nmutation {{ P(name: \"{name}\") {{ name }} }}"));
    }
    doc
}

fn open(path: &str, storage: Storage) -> Zega {
    Zega::open(path).storage(storage).wal_flush_every_write().build().unwrap()
}

fn names(zega: &Zega) -> Vec<String> {
    names_in(zega, P)
}

fn names_in(zega: &Zega, schema: &str) -> Vec<String> {
    let rows = match zega.run_lang(schema, "query { P { name } }").unwrap() {
        Json::Array(rows) => rows,
        Json::Null => Vec::new(),
        row => vec![row],
    };
    let mut names: Vec<String> = rows.iter().map(|row| row["name"].as_str().unwrap().to_string()).collect();
    names.sort();
    names
}

fn code(error: ZegaError) -> &'static str {
    match error {
        ZegaError::Delivery { code, .. } => code,
        other => panic!("expected a delivery refusal, got {other:?}"),
    }
}

fn delivered_once_and_kept(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let first = {
        let zega = open(path, storage);
        let first = zega.deliver("pg:1", "100", None, &doc(&["a", "b"])).unwrap();
        assert_eq!(names(&zega), ["a", "b"]);
        // The same delivery again: the original receipt, nothing written.
        assert_eq!(zega.deliver("pg:1", "100", None, &doc(&["a", "b"])).unwrap(), first);
        assert_eq!(names(&zega), ["a", "b"]);
        // The same identity, another document: refused, nothing written.
        assert_eq!(code(zega.deliver("pg:1", "100", None, &doc(&["c"])).unwrap_err()), "delivery_conflict");
        // A document that fails (a duplicate) leaves no receipt and no write,
        // so the identity can be delivered again.
        assert!(zega.deliver("pg:1", "101", None, &doc(&["d", "a"])).is_err());
        assert_eq!(zega.receipt("pg:1", "101").unwrap(), None);
        zega.deliver("pg:1", "101", None, &doc(&["d"])).unwrap();
        // Another source may reuse a txid.
        zega.deliver("pg:2", "100", None, &doc(&["e"])).unwrap();
        assert_eq!(names(&zega), ["a", "b", "d", "e"]);
        first
    };
    // Replayed from the WAL.
    {
        let zega = open(path, storage);
        assert_eq!(zega.receipt("pg:1", "100").unwrap(), Some(first.clone()));
        zega.checkpoint().unwrap();
    }
    // Carried by the checkpoint's .graph file.
    let zega = open(path, storage);
    assert_eq!(zega.receipt("pg:1", "100").unwrap(), Some(first.clone()));
    assert_eq!(zega.deliver("pg:1", "100", None, &doc(&["a", "b"])).unwrap(), first);
    assert_eq!(code(zega.deliver("pg:1", "100", None, &doc(&["z"])).unwrap_err()), "delivery_conflict");
    assert_eq!(names(&zega), ["a", "b", "d", "e"]);
}

#[test]
fn file_delivered_once_and_kept() {
    delivered_once_and_kept(Storage::File);
}

#[test]
fn sqlite_delivered_once_and_kept() {
    delivered_once_and_kept(Storage::Sqlite);
}

#[test]
fn identities_are_short_plain_text() {
    let zega = Zega::in_memory().build().unwrap();
    for (source, txid) in [("", "1"), ("pg", ""), ("pg one", "1"), ("pg", &"9".repeat(129)), ("pg/1", "1")] {
        assert_eq!(code(zega.deliver(source, txid, None, &doc(&["a"])).unwrap_err()), "invalid_identity", "{source:?} {txid:?}");
    }
    assert_eq!(names(&zega), Vec::<String>::new());
}

#[test]
fn receipts_travel_in_an_exported_graph() {
    let zega = Zega::in_memory().build().unwrap();
    let receipt = zega.deliver("pg:1", "7", None, &doc(&["a"])).unwrap();
    let mut bytes = Vec::new();
    zega.export(&mut bytes).unwrap();
    let copy = Zega::in_memory().build().unwrap();
    copy.import(&bytes[..]).unwrap();
    assert_eq!(copy.receipt("pg:1", "7").unwrap(), Some(receipt));
    // A graph that never had a delivery exports no RCPT section at all.
    let mut plain = Vec::new();
    Zega::in_memory().build().unwrap().export(&mut plain).unwrap();
    assert!(!plain.windows(4).any(|tag| tag == b"RCPT"));
}

const P2: &str = "schema { type P { name: String nick?: String } } unique { P { name } }";

/// A delivery that carries a schema stores it with the document's writes, as
/// pg_zega's journal does after an `ALTER TABLE`.
fn schema_delivered_with_the_writes(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let nick = format!("{P2}\nmutation {{ P(name: \"b\" && nick: \"bee\") {{ name }} }}");
    {
        let zega = open(path, storage);
        zega.set_schema(P).unwrap();
        zega.deliver("pg:1", "1", None, &doc(&["a"])).unwrap();
        let receipt = zega.deliver("pg:1", "2", Some(P2), &nick).unwrap();
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P2));
        assert_eq!(zega.deliver("pg:1", "2", Some(P2), &nick).unwrap(), receipt);
        // The same document under another schema is another payload.
        assert_eq!(code(zega.deliver("pg:1", "2", None, &nick).unwrap_err()), "delivery_conflict");
    }
    let zega = open(path, storage);
    assert_eq!(zega.stored_schema().unwrap(), zega_schema(P2));
    assert_eq!(names_in(&zega, P2), ["a", "b"]);
}

/// The stored schema as `stored_schema` reports it.
fn zega_schema(source: &str) -> String {
    let zega = Zega::in_memory().build().unwrap();
    zega.set_schema(source).unwrap();
    zega.stored_schema().unwrap()
}

#[test]
fn file_schema_delivered_with_the_writes() {
    schema_delivered_with_the_writes(Storage::File);
}

#[test]
fn sqlite_schema_delivered_with_the_writes() {
    schema_delivered_with_the_writes(Storage::Sqlite);
}

/// A document that fails takes its delivery's schema back with it; a schema
/// the data cannot satisfy is refused before the document runs.
#[test]
fn a_failed_delivery_keeps_the_old_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = open(path, Storage::Sqlite);
        zega.set_schema(P).unwrap();
        zega.deliver("pg:1", "1", None, &doc(&["a"])).unwrap();
        let duplicate = format!("{P2}\nmutation {{ P(name: \"a\") {{ name }} }}");
        assert!(zega.deliver("pg:1", "2", Some(P2), &duplicate).is_err());
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
        assert_eq!(zega.receipt("pg:1", "2").unwrap(), None);
        let required = "schema { type P { name: String age: Int } } unique { P { name } }";
        let refused = zega.deliver("pg:1", "3", Some(required), required).unwrap_err().to_string();
        assert!(refused.contains("schema change refused"), "{refused}");
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
    }
    let zega = open(path, Storage::Sqlite);
    assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
    assert_eq!(names(&zega), ["a"]);
}

const CHILD_DIR: &str = "ZEGA_DELIVERY_CRASH_CHILD_DIR";
const CHILD_STORAGE: &str = "ZEGA_DELIVERY_CRASH_CHILD_STORAGE";

/// The child: delivers txid i (the node `n<i>`) for i = 1, 2, … and prints
/// `i` once each delivery returns, until it is killed.
#[test]
#[ignore = "run only as the child of a_killed_delivery_is_retried_without_duplicates"]
fn delivery_crash_child() {
    let (Ok(dir), Ok(storage)) = (std::env::var(CHILD_DIR), std::env::var(CHILD_STORAGE)) else { return };
    let storage = if storage == "sqlite" { Storage::Sqlite } else { Storage::File };
    let zega = Zega::open(&dir).storage(storage).build().unwrap();
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let mut i = 0u64;
    loop {
        i += 1;
        zega.deliver("pg:1", &i.to_string(), None, &doc(&[&format!("n{i}")])).unwrap();
        writeln!(out, "ack {i}").unwrap();
        out.flush().unwrap();
    }
}

/// SIGKILL a process mid-stream of deliveries, then redeliver every txid the
/// source might have sent: each acknowledged delivery kept its receipt, and
/// redelivery after the crash writes every node exactly once.
#[test]
fn a_killed_delivery_is_retried_without_duplicates() {
    use std::io::BufRead;
    for storage in [Storage::File, Storage::Sqlite] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "delivery_crash_child", "--ignored", "--nocapture", "--test-threads=1"])
            .env(CHILD_DIR, path)
            .env(CHILD_STORAGE, if storage == Storage::Sqlite { "sqlite" } else { "file" })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut acked = 0u64;
        for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines() {
            if let Some(i) = line.unwrap().strip_prefix("ack ") {
                acked = i.parse().unwrap();
                if acked >= 25 {
                    break;
                }
            }
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let zega = open(path, storage);
        for i in 1..=acked {
            assert!(zega.receipt("pg:1", &i.to_string()).unwrap().is_some(), "{storage:?}: acknowledged {i} has no receipt");
        }
        // The source retries everything it may have sent, and a little more.
        for i in 1..=acked + 5 {
            zega.deliver("pg:1", &i.to_string(), None, &doc(&[&format!("n{i}")])).unwrap();
        }
        assert_eq!(names(&zega).len() as u64, acked + 5, "{storage:?}: a retry wrote twice or not at all");
    }
}
