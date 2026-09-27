//! Identical benchmark runnable against origin/main and this branch.
use crate::{graph::Graph, Value, Zega};
use std::{collections::HashMap, time::Instant};
#[test]
#[ignore = "explicit APS 24 before/after live traversal measurement"]
fn aps24_live_traversal_100k() {
    let mut graph = Graph::new();
    let mut ids = Vec::new();
    for n in 0..100_000 {
        ids.push(graph.create_node(
            vec!["P".into()],
            HashMap::from([("n".into(), Value::Int(n))]),
        ));
    }
    for pair in ids.chunks_exact(2) {
        graph.create_relationship("peers".into(), pair[0], pair[1], HashMap::new());
    }
    let bytes = crate::wal::encode_snapshot(&graph).unwrap();
    let db = Zega::in_memory()
        .query_time_limit(std::time::Duration::from_secs(120))
        .build()
        .unwrap();
    db.restore_bytes(&bytes).unwrap();
    let query = "{ P limit 100000 { n peers -> P { n } } }";
    let schema = "type P { n: Int peers -> P[] }";
    for _ in 0..3 {
        std::hint::black_box(db.run_lang(schema, query).unwrap());
    }
    let before = db.lock_graph().unwrap().history.accesses();
    let mut samples = Vec::new();
    for _ in 0..15 {
        let start = Instant::now();
        let rows = db.run_lang(schema, query).unwrap();
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(rows.as_array().unwrap().len(), 100_000);
        samples.push(elapsed);
        std::hint::black_box(rows);
    }
    assert_eq!(db.lock_graph().unwrap().history.accesses(), before);
    samples.sort_by(f64::total_cmp);
    println!("APS24 live nodes=100000 edges=50000 samples=15 median_ms={:.3} min_ms={:.3} max_ms={:.3} history_accesses=0 profile={}", samples[7], samples[0], samples[14], if cfg!(debug_assertions) { "debug" } else { "release" });
}
