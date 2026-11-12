//! APS 52: `Zega::deliver`. A document applied once per (source, txid),
//! its receipt durable with its writes, on both stores.

#![cfg(feature = "sqlite")]

use serde_json::Value as Json;
use zega::receipts::{payload_sha256, Flush, Payload};
use zega::receipts::BatchError;
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
        let first = zega.deliver("pg:1", "100", &Payload::document(&doc(&["a", "b"]))).unwrap();
        assert_eq!(names(&zega), ["a", "b"]);
        // The same delivery again: the original receipt, nothing written.
        assert_eq!(zega.deliver("pg:1", "100", &Payload::document(&doc(&["a", "b"]))).unwrap(), first);
        assert_eq!(names(&zega), ["a", "b"]);
        // The same identity, another document: refused, nothing written.
        assert_eq!(code(zega.deliver("pg:1", "100", &Payload::document(&doc(&["c"]))).unwrap_err()), "delivery_conflict");
        // A document that fails (a duplicate) leaves no receipt and no write,
        // so the identity can be delivered again.
        assert!(zega.deliver("pg:1", "101", &Payload::document(&doc(&["d", "a"]))).is_err());
        assert_eq!(zega.receipt("pg:1", "101").unwrap(), None);
        zega.deliver("pg:1", "101", &Payload::document(&doc(&["d"]))).unwrap();
        // Another source may reuse a txid.
        zega.deliver("pg:2", "100", &Payload::document(&doc(&["e"]))).unwrap();
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
    assert_eq!(zega.deliver("pg:1", "100", &Payload::document(&doc(&["a", "b"]))).unwrap(), first);
    assert_eq!(code(zega.deliver("pg:1", "100", &Payload::document(&doc(&["z"]))).unwrap_err()), "delivery_conflict");
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
        assert_eq!(code(zega.deliver(source, txid, &Payload::document(&doc(&["a"]))).unwrap_err()), "invalid_identity", "{source:?} {txid:?}");
    }
    assert_eq!(names(&zega), Vec::<String>::new());
}

#[test]
fn receipts_travel_in_an_exported_graph() {
    let zega = Zega::in_memory().build().unwrap();
    let receipt = zega.deliver("pg:1", "7", &Payload::document(&doc(&["a"]))).unwrap();
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
        zega.deliver("pg:1", "1", &Payload::document(&doc(&["a"]))).unwrap();
        let receipt = zega.deliver("pg:1", "2", &Payload { schema: Some(P2), ..Payload::document(&nick) }).unwrap();
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P2));
        assert_eq!(zega.deliver("pg:1", "2", &Payload { schema: Some(P2), ..Payload::document(&nick) }).unwrap(), receipt);
        // The same document under another schema is another payload.
        assert_eq!(code(zega.deliver("pg:1", "2", &Payload::document(&nick)).unwrap_err()), "delivery_conflict");
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
        zega.deliver("pg:1", "1", &Payload::document(&doc(&["a"]))).unwrap();
        let duplicate = format!("{P2}\nmutation {{ P(name: \"a\") {{ name }} }}");
        assert!(zega.deliver("pg:1", "2", &Payload { schema: Some(P2), ..Payload::document(&duplicate) }).is_err());
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
        assert_eq!(zega.receipt("pg:1", "2").unwrap(), None);
        let required = "schema { type P { name: String age: Int } } unique { P { name } }";
        let refused = zega.deliver("pg:1", "3", &Payload { schema: Some(required), ..Payload::document(required) }).unwrap_err().to_string();
        assert!(refused.contains("schema change refused"), "{refused}");
        assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
    }
    let zega = open(path, Storage::Sqlite);
    assert_eq!(zega.stored_schema().unwrap(), zega_schema(P));
    assert_eq!(names(&zega), ["a"]);
}

const LIKES: &str = "schema { type P { name: String likes -> Q[] } type Q { title: String } } unique { P { name } Q { title } }";
/// LIKES with a field P's stored nodes do not have: refused while they exist.
const AGED: &str = "schema { type P { name: String age: Int likes -> Q[] } type Q { title: String } } unique { P { name } Q { title } }";

/// As pg_zega writes it: the rows first, then the edge between them.
const LINK: &str = "mutation { P(name: \"NAME\") { likes -> link Q(title: \"x\") { title } } }";

fn node_and_link(name: &str) -> String {
    format!("mutation {{ Q(title: \"x\") {{ title }} }}\nmutation {{ P(name: \"{name}\") {{ name }} }}\n{}", LINK.replace("NAME", name))
}

fn liked(zega: &Zega, schema: &str, name: &str) -> Json {
    zega.run_lang(schema, &format!("query {{ P(name = \"{name}\") {{ likes -> Q {{ title }} }} }}")).unwrap()
}

fn titles(zega: &Zega, schema: &str) -> Json {
    zega.run_lang(schema, "query { Q { title } }").unwrap()
}

/// pg_zega's `zega.import(p)` in journal mode: flush P and its relationship,
/// store a schema the old P nodes could not satisfy, and load P again, as one
/// delivery. Q is not named, so it is kept.
fn flush_then_reload(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let load = format!("{LIKES}\n{}\nmutation {{ P(name: \"b\") {{ name }} }}", node_and_link("a"));
    let reload = format!("{AGED}\nmutation {{ P(name: \"c\" && age: 1) {{ name }} }}\n{}", LINK.replace("NAME", "c"));
    let flush = Flush { labels: vec!["P".into()], relationships: vec!["likes".into()], ..Flush::default() };
    let import = Payload { flush: flush.clone(), schema: Some(AGED), ..Payload::document(&reload) };
    {
        let zega = open(path, storage);
        zega.set_schema(LIKES).unwrap();
        zega.deliver("pg:1", "1", &Payload::document(&load)).unwrap();
        // Without the flush the schema change is refused: a and b have no age.
        let unflushed = Payload { flush: Flush::default(), ..import.clone() };
        assert!(zega.deliver("pg:1", "2", &unflushed).unwrap_err().to_string().contains("schema change refused"));
        let receipt = zega.deliver("pg:1", "2", &import).unwrap();
        assert_eq!(names_in(&zega, AGED), ["c"]);
        assert_eq!(liked(&zega, AGED, "c"), serde_json::json!({"likes": [{"title": "x"}]}));
        assert_eq!(titles(&zega, AGED), serde_json::json!([{"title": "x"}]));
        // Again: the same receipt, and nothing flushed twice.
        assert_eq!(zega.deliver("pg:1", "2", &import).unwrap(), receipt);
        assert_eq!(names_in(&zega, AGED), ["c"]);
        // The flush is part of the payload.
        let other = Payload { flush: Flush { all: true, ..Flush::default() }, ..import.clone() };
        assert_eq!(code(zega.deliver("pg:1", "2", &other).unwrap_err()), "delivery_conflict");
    }
    // The deletes are replayed from the WAL with the rest of the delivery.
    let zega = open(path, storage);
    assert_eq!(names_in(&zega, AGED), ["c"]);
    assert_eq!(liked(&zega, AGED, "c"), serde_json::json!({"likes": [{"title": "x"}]}));
}

#[test]
fn file_flush_then_reload() {
    flush_then_reload(Storage::File);
}

#[test]
fn sqlite_flush_then_reload() {
    flush_then_reload(Storage::Sqlite);
}

/// A relationship flush keeps both ends; a flush of everything keeps the
/// receipts, and a delivery that fails after its flush deletes nothing.
#[test]
fn flush_relationships_or_everything() {
    let zega = Zega::in_memory().build().unwrap();
    zega.set_schema(LIKES).unwrap();
    let load = format!("{LIKES}\n{}", node_and_link("a"));
    zega.deliver("pg:1", "1", &Payload::document(&load)).unwrap();
    let unlink = Payload { flush: Flush { relationships: vec!["likes".into()], ..Flush::default() }, ..Payload::document(LIKES) };
    zega.deliver("pg:1", "2", &unlink).unwrap();
    assert_eq!(names_in(&zega, LIKES), ["a"]);
    assert_eq!(liked(&zega, LIKES, "a"), serde_json::json!({"likes": []}));
    assert_eq!(titles(&zega, LIKES), serde_json::json!([{"title": "x"}]));

    let everything = Flush { all: true, ..Flush::default() };
    let failing = format!("{LIKES}\nmutation {{ P(name: \"d\") {{ name }} }}\nmutation {{ P(name: \"d\") {{ name }} }}");
    assert!(zega.deliver("pg:1", "3", &Payload { flush: everything.clone(), ..Payload::document(&failing) }).is_err());
    assert_eq!(names_in(&zega, LIKES), ["a"]);
    assert_eq!(titles(&zega, LIKES), serde_json::json!([{"title": "x"}]));

    zega.deliver("pg:1", "3", &Payload { flush: everything, ..Payload::document(LIKES) }).unwrap();
    assert_eq!(names_in(&zega, LIKES), Vec::<String>::new());
    assert_eq!(titles(&zega, LIKES), serde_json::json!([]));
    assert!(zega.receipt("pg:1", "1").unwrap().is_some());
}

fn receipts_of(zega: &Zega, source: &str, txids: &[&str]) -> Vec<bool> {
    txids.iter().map(|txid| zega.receipt(source, txid).unwrap().is_some()).collect()
}

/// pg_zega's worker sends `forget_earlier` on every delivery: a graph keeps
/// one receipt per source, in the WAL and the checkpoint alike, and a
/// delivery that fails forgets nothing.
fn forget_earlier_keeps_the_latest(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    {
        let zega = open(path, storage);
        zega.deliver("pg:1", "1", &Payload::document(&doc(&["a"]))).unwrap();
        zega.deliver("pg:1", "2", &Payload::document(&doc(&["b"]))).unwrap();
        zega.deliver("pg:2", "1", &Payload::document(&doc(&["c"]))).unwrap();
        // A delivery that fails (a duplicate) forgets nothing.
        let duplicate = doc(&["a"]);
        assert!(zega.deliver("pg:1", "3", &Payload { forget_earlier: true, ..Payload::document(&duplicate) }).is_err());
        assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3"]), [true, true, false]);
        let three = doc(&["d"]);
        let receipt = zega.deliver("pg:1", "3", &Payload { forget_earlier: true, ..Payload::document(&three) }).unwrap();
        assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3"]), [false, false, true]);
        // Another source's receipts stay.
        assert_eq!(receipts_of(&zega, "pg:2", &["1"]), [true]);
        // The flag is not part of the payload: a retry without it is the same delivery.
        assert_eq!(zega.deliver("pg:1", "3", &Payload::document(&three)).unwrap(), receipt);
        assert_eq!(names(&zega), ["a", "b", "c", "d"]);
    }
    // Replayed from the WAL.
    {
        let zega = open(path, storage);
        assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3"]), [false, false, true]);
        zega.checkpoint().unwrap();
    }
    // Carried by the checkpoint.
    let zega = open(path, storage);
    assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3"]), [false, false, true]);
    assert_eq!(receipts_of(&zega, "pg:2", &["1"]), [true]);
    assert_eq!(names(&zega), ["a", "b", "c", "d"]);
}

#[test]
fn file_forget_earlier_keeps_the_latest() {
    forget_earlier_keeps_the_latest(Storage::File);
}

#[test]
fn sqlite_forget_earlier_keeps_the_latest() {
    forget_earlier_keeps_the_latest(Storage::Sqlite);
}

/// pg_zega's worker sends a round of journal rows as one batch: one
/// transaction, one commit, a receipt each, or nothing when any fails.
fn a_batch_is_one_transaction(storage: Storage) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let docs: Vec<String> = ["a", "b", "c", "d", "e", "f"].iter().map(|n| doc(&[n])).collect();
    let duplicate = doc(&["a"]);
    {
        let zega = open(path, storage);
        let first = [("1", Payload::document(&docs[0])), ("2", Payload::document(&docs[1])), ("3", Payload::document(&docs[2]))];
        let receipts = zega.deliver_all("pg:1", &first).unwrap();
        assert_eq!(receipts.iter().map(|r| r.txid.as_str()).collect::<Vec<_>>(), ["1", "2", "3"]);
        assert_eq!(names(&zega), ["a", "b", "c"]);
        // The same batch again: the same receipts, nothing written twice.
        assert_eq!(zega.deliver_all("pg:1", &first).unwrap(), receipts);
        assert_eq!(names(&zega), ["a", "b", "c"]);
        // The second delivery fails: the first is not kept either.
        let failing = [("4", Payload::document(&docs[3])), ("5", Payload::document(&duplicate))];
        let BatchError { index, .. } = zega.deliver_all("pg:1", &failing).unwrap_err();
        assert_eq!(index, Some(1));
        assert_eq!(names(&zega), ["a", "b", "c"]);
        assert_eq!(receipts_of(&zega, "pg:1", &["4", "5"]), [false, false]);
        // forget_earlier on the last delivery keeps the whole batch's receipts.
        let forget = [("6", Payload::document(&docs[4])), ("7", Payload { forget_earlier: true, ..Payload::document(&docs[5]) })];
        zega.deliver_all("pg:1", &forget).unwrap();
        assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3", "6", "7"]), [false, false, false, true, true]);
    }
    // Replayed from the WAL.
    let zega = open(path, storage);
    assert_eq!(names(&zega), ["a", "b", "c", "e", "f"]);
    assert_eq!(receipts_of(&zega, "pg:1", &["1", "2", "3", "6", "7"]), [false, false, false, true, true]);
}

#[test]
fn file_a_batch_is_one_transaction() {
    a_batch_is_one_transaction(Storage::File);
}

#[test]
fn sqlite_a_batch_is_one_transaction() {
    a_batch_is_one_transaction(Storage::Sqlite);
}

/// Receipts written before flushes existed keep their digest, so their
/// deliveries are still recognised when the relay sends them again.
#[test]
fn a_payload_without_a_flush_hashes_as_before() {
    let hex = |digest: [u8; 32]| digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(payload_sha256(&Payload::document("d"))), V1_DOCUMENT);
    assert_eq!(hex(payload_sha256(&Payload { schema: Some("s"), ..Payload::document("d") })), V1_SCHEMA);
    // A flush of nothing is no flush.
    assert_eq!(hex(payload_sha256(&Payload { flush: Flush { labels: vec![], ..Flush::default() }, ..Payload::document("d") })), V1_DOCUMENT);
}

const V1_DOCUMENT: &str = "149d046ea8e4855ef7d230aab86ac7a766d0e2a86ae1cfbe1dc308a18fd324d6";
const V1_SCHEMA: &str = "08ab367b718fcc78c433695d2da2d4a3bf8d0d0386b67ff97386c591af7e1b69";

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
        zega.deliver("pg:1", &i.to_string(), &Payload::document(&doc(&[&format!("n{i}")]))).unwrap();
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
            zega.deliver("pg:1", &i.to_string(), &Payload::document(&doc(&[&format!("n{i}")]))).unwrap();
        }
        assert_eq!(names(&zega).len() as u64, acked + 5, "{storage:?}: a retry wrote twice or not at all");
    }
}
