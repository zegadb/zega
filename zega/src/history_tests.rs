//! APS 24 acceptance tests through the public ZQL entry point.
use crate::Zega;
use serde_json::json;
const SCHEMA: &str = "type Team { name: String points: <Int> }";
fn season() -> Zega {
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(
        SCHEMA,
        r#"mutation at 2024-01-01 { Team(name: "Oilers" && points: 10) { name } }"#,
    )
    .unwrap();
    for (date, points) in [("2024-01-15", 30), ("2024-01-08", 20), ("2024-01-22", 15)] {
        db.run_lang(
            SCHEMA,
            &format!(
                r#"mutation at {date} {{ Team(name = "Oilers") set points: {points} {{ name }} }}"#
            ),
        )
        .unwrap();
    }
    db
}
#[test]
fn aps24_live_never_accesses_history() {
    let db = season();
    let bytes = db.snapshot_bytes().unwrap();
    db.restore_bytes(&bytes).unwrap();
    let before = db.lock_graph().unwrap().history.accesses();
    assert_eq!(before, 0);
    assert_eq!(
        db.run_lang(SCHEMA, r#"{ Team(name = "Oilers") { points } }"#)
            .unwrap(),
        json!({"points":15})
    );
    assert_eq!(db.lock_graph().unwrap().history.accesses(), before);
}
#[test]
fn aps24_as_of_backfills_and_filters() {
    let db = season();
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team(points = 20) { points } } as of 2024-01-10")
            .unwrap(),
        json!({"points":20})
    );
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team limit 10 { points } } as of 2023-12-31")
            .unwrap(),
        json!([{"points":null}])
    );
}
#[test]
fn aps24_week_series() {
    let db = season();
    let result = db
        .run_lang(
            SCHEMA,
            r#"{ Team(name = "Oilers") { <points> } } from 2024-01-01 to 2024-01-22 by week"#,
        )
        .unwrap();
    assert_eq!(
        result,
        json!({"points":[
            {"time":"2024-01-01T00:00","value":10},
            {"time":"2024-01-08T00:00","value":20},
            {"time":"2024-01-15T00:00","value":30},
            {"time":"2024-01-22T00:00","value":15}
        ]})
    );
}
#[test]
fn aps24_first_last_ever_always() {
    let db = season();
    assert_eq!(db.run_lang(SCHEMA, "{ Team limit 10 { @firstTime(points >= 20) @lastTime(points >= 20) never: @firstTime(points > 99) } }").unwrap(), json!([{"firstTime":"2024-01-08T00:00","lastTime":"2024-01-15T00:00","never":null}]));
    assert_eq!(db.run_lang(SCHEMA, "{ Team(ever points >= 30 && always points >= 10 && @firstTime(points >= 20) < 2024-01-09) { points } }").unwrap(), json!([{"points":15}]));
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team(always points >= 20) { points } }")
            .unwrap(),
        json!([])
    );
}
#[test]
fn aps24_snapshot_round_trip_lazy_and_old() {
    let db = season();
    let bytes = db.snapshot_bytes().unwrap();
    let mut graph = crate::graph::Graph::new();
    crate::wal::restore_bytes(&mut graph, &bytes).unwrap();
    assert_eq!(graph.history.accesses(), 0);
    let histories = graph.history.get().unwrap();
    assert_eq!(
        histories
            .values()
            .next()
            .unwrap()
            .at(crate::history::date("2024-01-10").unwrap()),
        Some(&crate::Value::Int(20))
    );
    let empty = crate::graph::Graph::new();
    let old = crate::wal::encode_snapshot(&empty).unwrap();
    crate::wal::restore_bytes(&mut graph, &old).unwrap();
    assert!(!graph.history.has_data());
}

#[test]
fn aps24_schema_and_formatter() {
    let db = Zega::in_memory().build().unwrap();
    db.schema("type T { a: <Int> b: <Int[]> c: <Int>[] }")
        .unwrap();
    for schema in [
        "type T { rs -> <T[]> }",
        "type T { born: Date appears at born }",
        "type T { died: Date ends at died }",
    ] {
        assert!(db
            .schema(schema)
            .unwrap_err()
            .to_string()
            .contains("not yet: APS 24 phase 2"));
    }
    assert!(
        db.schema_diff("type Team { name: String points: Int }", SCHEMA)
            .unwrap()
            .ok
    );
    let db = season();
    assert!(
        !db.schema_diff(SCHEMA, "type Team { name: String points: Int }")
            .unwrap()
            .ok
    );
    for query in [
        "{ Team limit 10 { points } } as of 2024-01-10T12:30",
        "{ Team limit 10 { <points> } } from 2024-01-01 to 2024-01-22 by week",
        "{ Team(@firstTime(points > 10) < 2024-01-09) { @lastTime(points > 10) } }",
    ] {
        let formatted = crate::lang::fmt::format_zql(query).unwrap();
        assert_eq!(
            db.run_lang(SCHEMA, query).unwrap(),
            db.run_lang(SCHEMA, &formatted).unwrap(),
            "{formatted}"
        );
    }
}

#[test]
fn aps24_graph_round_trip_and_legacy() {
    let original = season();
    let mut bytes = Vec::new();
    original.export(&mut bytes).unwrap();
    let db = Zega::in_memory().build().unwrap();
    db.import(&bytes[..]).unwrap();
    assert_eq!(db.lock_graph().unwrap().history.accesses(), 0);
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team(points = 20) { points } } as of 2024-01-10")
            .unwrap(),
        json!({"points":20})
    );
    let empty = Zega::in_memory().build().unwrap();
    bytes.clear();
    empty.export(&mut bytes).unwrap();
    db.import(&bytes[..]).unwrap();
    assert!(!db.lock_graph().unwrap().history.has_data());
}

#[test]
fn aps24_wal_backfills_and_plain_now() {
    let dir = tempfile::tempdir().unwrap();
    let db = Zega::open(dir.path().to_str().unwrap())
        .snapshot_every(0)
        .wal_flush_every_write()
        .build()
        .unwrap();
    db.run_lang(
        SCHEMA,
        r#"mutation at 2024-01-15 { Team(name: "Oilers" && points: 30) { name } }"#,
    )
    .unwrap();
    db.run_lang(
        SCHEMA,
        r#"mutation at 2024-01-01 { Team(name = "Oilers") set points: 10 { name } }"#,
    )
    .unwrap();
    drop(db);
    let db = Zega::open(dir.path().to_str().unwrap())
        .snapshot_every(0)
        .build()
        .unwrap();
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team(points = 10) { points } } as of 2024-01-10")
            .unwrap(),
        json!({"points":10})
    );
    assert_eq!(
        db.run_lang(SCHEMA, "{ Team limit 1 { points } }").unwrap(),
        json!([{"points":30}])
    );
    let before = crate::history::now();
    db.run_lang(
        SCHEMA,
        r#"mutation { Team(name = "Oilers") set points: 40 { points } }"#,
    )
    .unwrap();
    let graph = db.lock_graph().unwrap();
    let history = graph.history.get().unwrap().values().next().unwrap();
    assert!(history.last >= before && history.last <= crate::history::now());
    assert_eq!(history.changes.last().unwrap().1, crate::Value::Int(40));
}

#[test]
#[ignore = "explicit APS 24 storage and latency measurement"]
fn aps24_measurements() {
    use crate::{graph::Graph, Value};
    use std::{collections::HashMap, time::Instant};
    let mut graph = Graph::new();
    let start = crate::history::date("2024-01-01").unwrap();
    for team in 0..32 {
        let id = graph.create_node(vec!["Team".into()], HashMap::new());
        for stat in 0..5 {
            let field = format!("stat{stat}");
            for game in 0..82 {
                let value = Value::Int(game * (stat + 1) + team);
                graph
                    .record_history(
                        id,
                        start + game * 86400,
                        &HashMap::from([(field.clone(), value)]),
                        std::slice::from_ref(&field),
                    )
                    .unwrap();
            }
        }
    }
    let bytes = graph.history.bytes().unwrap().unwrap().len();
    println!(
        "APS24 HIST: changes=13120 bytes={bytes} bytes_per_change={:.4}",
        bytes as f64 / 13120.0
    );
    let db = Zega::in_memory().build().unwrap();
    {
        let mut graph = db.lock_graph().unwrap();
        for n in 0..100_000 {
            let props = HashMap::from([("points".into(), Value::Int(n))]);
            let id = graph.create_node(vec!["Team".into()], props.clone());
            graph
                .record_history(id, start, &props, &["points".into()])
                .unwrap();
        }
    }
    for (label, query) in [
        ("live", "{ Team limit 100000 { points } }"),
        ("as_of", "{ Team limit 100000 { points } } as of 2024-01-02"),
    ] {
        db.run_lang(SCHEMA, query).unwrap();
        let mut samples = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            let result = db.run_lang(SCHEMA, query).unwrap();
            assert_eq!(result.as_array().unwrap().len(), 100_000);
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(result);
        }
        samples.sort_by(f64::total_cmp);
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        println!("APS24 latency: nodes=100000 rows=100000 mode={label} median_ms={:.3} samples=7 profile={profile}", samples[3]);
    }
}

#[test]
fn aps24_lists_and_discovery_use_historical_values() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type T { key: Int name: <String> lineup: <Int[]> splits: <Int>[] }";
    db.run_lang(schema, r#"mutation at 2024-01-01 { T(key: 1 && name: "Before" && lineup: [1, 2] && splits: [3, 4]) { key } }"#).unwrap();
    db.run_lang(schema, r#"mutation at 2024-01-15 { T(key = 1) set name: "After", lineup: [5, 6], splits: [7, 8] { key } }"#).unwrap();
    assert_eq!(
        db.run_lang(schema, "{ T(key = 1) { lineup splits } } as of 2024-01-10")
            .unwrap(),
        json!({"lineup":[1,2],"splits":[3,4]})
    );
    let result = db.run_lang(schema, r#"{ T limit 10 { name } } as of 2024-01-10 then { findExact { "Before" in { name } } }"#).unwrap();
    assert_eq!(result["stages"][1]["nodes"][0]["props"]["name"], "Before");
    assert_eq!(result["stages"][1]["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn aps24_dates_and_temporal_recursion_are_bounded() {
    let db = season();
    assert_eq!(
        db.run_lang(
            SCHEMA,
            "{ Team(@firstTime(points >= 20) = 2024-01-08) { points } }"
        )
        .unwrap(),
        json!([{"points":15}])
    );
    for date in [
        "2024-02-30",
        "2023-02-29",
        "2024-01-01T24:00",
        "2024-01-01T01:60",
        "2024-01-01T00:00Z",
        "2024-01-01T00:00:60",
    ] {
        assert!(db
            .run_lang(
                SCHEMA,
                &format!("{{ Team limit 10 {{ points }} }} as of {date}")
            )
            .is_err());
    }
    let at = crate::history::date("2024-01-01T01:02:03").unwrap();
    assert_eq!(crate::history::format_date(at), "2024-01-01T01:02:03");
    let now = crate::history::now();
    assert_eq!(
        crate::history::date(&crate::history::format_date(now)).unwrap(),
        now
    );
    let test = format!("{{ Team({}points > 1) {{ points }} }}", "ever ".repeat(200));
    let error = db.run_lang(SCHEMA, &test).unwrap_err().to_string();
    assert!(error.contains("nested too deeply"), "{error}");
}

#[test]
fn aps24_old_snapshot_metadata_can_end_in_hist() {
    let mut graph = crate::graph::Graph::new();
    graph.set_state(
        Default::default(),
        Default::default(),
        crate::graph_file::Carried {
            meta: std::collections::BTreeMap::from([("title".into(), "HIST".into())]),
            ..Default::default()
        },
    );
    let bytes = crate::wal::encode_snapshot(&graph).unwrap();
    assert!(bytes.ends_with(b"HIST"));
    let mut restored = crate::graph::Graph::new();
    crate::wal::restore_bytes(&mut restored, &bytes).unwrap();
    assert_eq!(restored.carried().meta["title"], "HIST");
    assert!(!restored.history.has_data());
}

#[test]
fn aps24_live_indexes_cannot_hide_historical_matches() {
    let db = season();
    let schema = format!("{SCHEMA} index {{ range Team {{ points }} }}");
    assert_eq!(
        db.run_lang(&schema, "{ Team(points = 20) { points } } as of 2024-01-10")
            .unwrap(),
        json!({"points":20})
    );
    assert_eq!(
        db.run_lang(&schema, "{ Team(points = 15) { points } }")
            .unwrap(),
        json!({"points":15})
    );
}
