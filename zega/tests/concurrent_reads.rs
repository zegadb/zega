//! zegadb/zega#147: reads share the graph. A long `*path` must not hold up
//! other reads, a write waits for the reads in progress and then lands whole,
//! and `zql_is_read` tells a host which requests may run side by side.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};
use zega::graph_file::json_lines::JsonLinesOptions;
use zega::{zql_is_read, Zega, ZqlEntryPoint};

const SCHEMA: &str = "schema {
  type Junction { n: Int at: Point road -> Junction[] { cost: Float<km> } }
  type Pair { id: Int }
}
unique { Junction { n } }";

/// A `side` x `side` grid of junctions, every neighbour joined both ways at
/// cost 1, loaded through the bulk path so that building it stays cheap.
fn grid(side: u64) -> Zega {
    let mut lines = String::new();
    for n in 0..side * side {
        let (row, column) = (n / side, n % side);
        let at = json!({"lat": 51.0 + row as f64 * 1e-3, "lon": -114.0 + column as f64 * 1e-3});
        lines.push_str(&json!({"type": "node", "id": n + 1, "labels": ["Junction"], "props": {"n": n, "at": at}}).to_string());
        lines.push('\n');
    }
    for n in 0..side * side {
        let (row, column) = (n / side, n % side);
        let mut next = Vec::new();
        if column + 1 < side {
            next.push(n + 1);
        }
        if row + 1 < side {
            next.push(n + side);
        }
        for to in next {
            for (from, to) in [(n, to), (to, n)] {
                lines.push_str(&json!({"type": "rel", "kind": "road", "from": from + 1, "to": to + 1, "props": {"cost": 1.0}}).to_string());
                lines.push('\n');
            }
        }
    }
    let db = Zega::in_memory().traversal_work_budget(usize::MAX).build().unwrap();
    let options = JsonLinesOptions { schema: Some(SCHEMA.to_string()), meta: BTreeMap::new() };
    db.import_json_lines(lines.as_bytes(), &options).unwrap();
    db
}

const SIDE: u64 = 200;

/// Corner to corner by Dijkstra: no guess, so it settles most of the grid.
fn long_path() -> String {
    format!("{{ Junction(n: 0) {{ road *path by &cost -> Junction(n: {}) {{ n }} }} }}", SIDE * SIDE - 1)
}

fn path_cost(db: &Zega) -> Json {
    db.run_lang_read(SCHEMA, &long_path()).unwrap()["road"]["cost"].clone()
}

const SHORT: &str = "{ Junction(n: 7) { n } }";

/// A result as rows: a list, one row, or none.
fn rows(result: Json) -> Vec<Json> {
    match result {
        Json::Array(rows) => rows,
        Json::Null => Vec::new(),
        row => vec![row],
    }
}

#[test]
fn short_reads_answer_while_a_long_path_runs() {
    let db = Arc::new(grid(SIDE));
    let started = Instant::now();
    assert_eq!(path_cost(&db), json!((2 * (SIDE - 1)) as f64));
    let alone = started.elapsed();
    assert!(alone >= Duration::from_millis(100), "the long path took only {alone:?}: too short to prove anything");

    let long = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || {
            let cost = path_cost(&db);
            (cost, Instant::now())
        })
    };
    // Well inside the long path: it holds the graph by now.
    std::thread::sleep(alone / 4);
    let shorts_started = Instant::now();
    for _ in 0..5 {
        assert_eq!(rows(db.run_lang_read(SCHEMA, SHORT).unwrap()), vec![json!({"n": 7})]);
    }
    let shorts_done = Instant::now();
    let (cost, long_done) = long.join().unwrap();
    assert_eq!(cost, json!((2 * (SIDE - 1)) as f64));
    // With one lock for every statement the short reads queue behind the
    // path: the first alone waits for the ~3/4 of it still to run.
    let shorts_took = shorts_done - shorts_started;
    assert!(
        shorts_took < alone / 2 && shorts_done < long_done,
        "the short reads waited for the long path: they took {shorts_took:?} beside a {alone:?} path"
    );
}

#[test]
fn a_write_during_long_reads_lands_whole_and_reads_see_all_or_none_of_it() {
    let db = Arc::new(grid(60));
    // One statement, two nodes: the engine's unit of atomicity.
    let pair = |db: &Zega, id: u64| {
        let sources = std::collections::HashMap::from([(
            "pair.json".to_string(),
            json!([{"id": id * 2}, {"id": id * 2 + 1}]).to_string(),
        )]);
        db.run_lang_with_sources(SCHEMA, "mutation json [\"pair.json\"] { Pair(id: $id) }", &sources)
            .unwrap();
    };
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..2 {
        let (db, stop) = (Arc::clone(&db), Arc::clone(&stop));
        readers.push(std::thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let pairs = rows(db.run_lang_read(SCHEMA, "{ Pair { id } }").unwrap());
                assert_eq!(pairs.len() % 2, 0, "a read saw half a write: {pairs:?}");
                reads += 1;
            }
            reads
        }));
    }
    for _ in 0..4 {
        let (db, stop) = (Arc::clone(&db), Arc::clone(&stop));
        readers.push(std::thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let path = db
                    .run_lang_read(SCHEMA, "{ Junction(n: 0) { road *path by &cost -> Junction(n: 3599) { n } } }")
                    .unwrap();
                assert_eq!(path["road"]["cost"], json!(118.0));
                reads += 1;
            }
            reads
        }));
    }
    for id in 0..20 {
        let started = Instant::now();
        pair(&db, id);
        // Not starved: a writer waits for the reads in progress, not for
        // every read that arrives after it.
        assert!(started.elapsed() < Duration::from_secs(30), "write {id} waited {:?}", started.elapsed());
        let both = db.run_lang_read(SCHEMA, &format!("{{ Pair(id: {}) {{ id }} }}", id * 2 + 1)).unwrap();
        assert_eq!(rows(both), vec![json!({"id": id * 2 + 1})], "an acknowledged write is not visible");
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let reads: usize = readers.into_iter().map(|reader| reader.join().unwrap()).sum();
    assert!(reads > 0);
    assert_eq!(rows(db.run_lang_read(SCHEMA, "{ Pair { id } }").unwrap()).len(), 40);
}

#[test]
fn zql_is_read_admits_reads_only() {
    let read = "{ Junction(n: 0) { n } }";
    let mutation = "mutation { Pair(id: 1) { id } }";
    let load = "mutation json [\"rows.json\"] { Pair(id: $id) }";
    assert!(zql_is_read(ZqlEntryPoint::Statement, read));
    assert!(!zql_is_read(ZqlEntryPoint::Statement, mutation));
    assert!(!zql_is_read(ZqlEntryPoint::Statement, load));
    assert!(!zql_is_read(ZqlEntryPoint::Statement, "{ Junction("));
    assert!(zql_is_read(ZqlEntryPoint::File, &format!("{SCHEMA}\nquery {read}\nquery {{ Pair {{ id }} }}")));
    assert!(!zql_is_read(ZqlEntryPoint::File, &format!("{SCHEMA}\nquery {read}\n{mutation}")));
    // A raw statement sent as a document, as `apply_zql` takes it.
    assert!(zql_is_read(ZqlEntryPoint::File, read));
    assert!(!zql_is_read(ZqlEntryPoint::File, mutation));
}
