//! The embedder options on `ZegaBuilder` (`sync_history`, `vector_index`, `read_ahead`) and the whole-table node API
//! (`zega::embed`). With no option set, Zega behaves as before; each option changes only what it says.

use std::collections::HashMap;
use std::path::Path;

use zega::vector::{Metric, Vector, VectorIndexing};
use zega::{Value, Zega, ZegaBuilder};

const SCHEMA: &str = "schema { type Word { text: String vector: Vector<2, dot> } }";

fn word(text: &str, x: f32, y: f32) -> (Vec<String>, HashMap<String, Value>) {
    let vector = Vector::new(&[x, y], Metric::Dot).unwrap();
    (
        vec!["Word".to_string()],
        HashMap::from([("text".to_string(), Value::from(text)), ("vector".to_string(), Value::Vector(Box::new(vector)))]),
    )
}

fn nearest(zega: &Zega, x: f32, y: f32, k: usize) -> String {
    zega.run_lang(SCHEMA, &format!("query {{ Word @near(vector, @vector[{x}, {y}], {k}) {{ text @score }} }}")).unwrap().to_string()
}

/// Writes, a query, more writes and a query again; every answer, in order.
fn session(zega: &Zega) -> Vec<String> {
    let ids = zega.transaction(|tx| tx.create_nodes(vec![word("east", 1.0, 0.0), word("north", 0.0, 1.0), word("west", -1.0, 0.0)])).unwrap();
    let mut answers = vec![nearest(zega, 0.9, 0.1, 2), nearest(zega, 0.1, 0.9, 1)];
    let flipped = Vector::new(&[2.0, 0.0], Metric::Dot).unwrap();
    zega.transaction(|tx| tx.update_nodes(vec![(ids[2], HashMap::from([("vector".to_string(), Value::Vector(Box::new(flipped)))]))])).unwrap();
    zega.transaction(|tx| tx.create_nodes(vec![word("south", 0.0, -1.0)])).unwrap();
    answers.push(nearest(zega, 1.0, 0.0, 3));
    answers.push(nearest(zega, 0.0, -1.0, 1));
    answers
}

#[test]
fn vector_index_lazy_answers_every_query_as_the_default_does() {
    let default = session(&Zega::in_memory().build().unwrap());
    let lazy = session(&Zega::in_memory().vector_index(VectorIndexing::Lazy).build().unwrap());
    assert!(default[0].contains("east") && default[2].contains("west"), "{default:?}");
    assert_eq!(lazy, default, "a lazily built index finds the same vectors with the same scores");
}

fn commits(builder: ZegaBuilder) -> u64 {
    let zega = builder.build().unwrap();
    for i in 0..3 {
        zega.transaction(|tx| tx.create_nodes(vec![word(&format!("w{i}"), 1.0, 0.0)])).unwrap();
    }
    assert_eq!(zega.counts().unwrap().nodes, 3, "the writes themselves are the same either way");
    zega.graph_version().unwrap()
}

#[test]
fn sync_history_off_records_no_history_for_an_unlinked_graph_and_the_default_does() {
    assert_eq!(commits(Zega::in_memory()), 3, "default: every commit is a sync history entry");
    assert_eq!(commits(Zega::in_memory().sync_history(true)), 3);
    assert_eq!(commits(Zega::in_memory().sync_history(false)), 0, "an unlinked graph keeps none");
}

/// A .graph file of a graph with lists, vectors and strings, as an import or a checkpoint writes one.
fn graph_file() -> Vec<u8> {
    let zega = Zega::in_memory().build().unwrap();
    let rows: Vec<_> = (0..3000)
        .map(|i| {
            let (labels, mut props) = word(&format!("word {i}"), i as f32, 1.0);
            props.insert("ids".to_string(), Value::List((0..50).map(Value::Int).collect()));
            (labels, props)
        })
        .collect();
    zega.transaction(|tx| tx.create_nodes(rows)).unwrap();
    let mut bytes = Vec::new();
    zega.export(&mut bytes).unwrap();
    bytes
}

fn import(bytes: &[u8], read_ahead: bool) -> Result<Vec<u8>, String> {
    let zega = Zega::in_memory().read_ahead(read_ahead).build().unwrap();
    zega.import(bytes).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    zega.export(&mut out).unwrap();
    Ok(out)
}

#[test]
fn read_ahead_imports_the_same_graph_and_refuses_the_same_broken_files() {
    let bytes = graph_file();
    assert!(bytes.len() > 3 * 64 * 1024, "the nodes section spans several read-ahead blocks: {}", bytes.len());
    assert_eq!(import(&bytes, false).unwrap(), bytes, "the default import exports exactly the file it read");
    assert_eq!(import(&bytes, true).unwrap(), bytes, "so does an import with read-ahead");
    for cut in [bytes.len() / 3, bytes.len() - 40, bytes.len() - 1] {
        assert!(import(&bytes[..cut], false).is_err() && import(&bytes[..cut], true).is_err(), "truncated at {cut}");
    }
    let mut flipped = bytes.clone();
    flipped[bytes.len() / 2] ^= 0x40;
    let (a, b) = (import(&flipped, false).unwrap_err(), import(&flipped, true).unwrap_err());
    assert!(a.contains("checksum") && b.contains("checksum"), "a damaged byte is a checksum error both ways: {a} / {b}");
}

/// Reopening a disk database applies the options to the graph read back.
#[test]
fn options_hold_for_a_graph_read_back_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("g");
    let open = |p: &Path| {
        Zega::open(p.to_str().unwrap()).sync_history(false).vector_index(VectorIndexing::Lazy).read_ahead(true).build().unwrap()
    };
    let first = session(&open(&path));
    let zega = open(&path);
    zega.snapshot().unwrap();
    drop(zega);
    let zega = open(&path);
    assert!(nearest(&zega, 1.0, 0.0, 1).contains("west"), "the graph read back answers as before: {first:?}");
    assert_eq!(zega.graph_version().unwrap(), 0);
}

#[test]
fn embed_writes_are_one_transaction_and_reads_see_them() {
    let zega = Zega::in_memory().build().unwrap();
    let ids = zega.transaction(|tx| tx.create_nodes(vec![word("a", 1.0, 0.0), word("b", 0.0, 1.0)])).unwrap();
    let texts = zega
        .nodes(|n| {
            let mut labelled = n.labelled("Word");
            labelled.sort();
            labelled.iter().map(|id| n.prop(*id, "text").and_then(Value::as_string).unwrap().to_string()).collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(texts, ["a", "b"]);

    // A transaction that fails after a bulk write leaves the graph as it was.
    let failed: zega::Result<()> = zega.transaction(|tx| {
        tx.update_nodes(vec![(ids[0], HashMap::from([("text".to_string(), Value::from("changed"))]))])?;
        tx.create_nodes(vec![word("c", 1.0, 1.0)])?;
        Err(zega::ZegaError::Execution("stop".to_string()))
    });
    assert!(failed.is_err());
    let after = zega.nodes(|n| (n.labelled("Word").len(), n.prop(ids[0], "text").and_then(Value::as_string).map(str::to_string))).unwrap();
    assert_eq!(after, (2, Some("a".to_string())), "the failed transaction's writes were taken back");
    assert!(zega.nodes(|n| n.exists(ids[1]) && !n.exists(u64::MAX)).unwrap());
}
