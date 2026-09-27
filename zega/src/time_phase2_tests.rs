//! APS 24 phase 2 acceptance through the production parser, executor and WAL.
use crate::Zega;
use serde_json::json;
const ROSTER: &str = r#"
type Team { name: String points: <Int> players -> <Person[]> }
type Person { name: String born: Date died?: Date team: players <- <Team[]> children -> Person[] appears at born ends at died }
"#;
fn run(db: &Zega, text: &str) -> serde_json::Value {
    db.run_lang(ROSTER, text).unwrap()
}
fn seed(db: &Zega) {
    run(
        db,
        r#"mutation at 2023-10-10 { Team(name: "Oilers" && points: 10) { players -> Person(name: "Pat" && born: "1990-01-01") { name } } }"#,
    );
    run(
        db,
        r#"mutation at 2023-10-10 { Team(name: "Flames" && points: 0) { name } }"#,
    );
    run(
        db,
        r#"mutation at 2024-02-01 { Team(name = "Oilers") { players -> unlink Person(name = "Pat") { name } } }"#,
    );
    run(
        db,
        r#"mutation at 2024-02-01 { Team(name = "Flames") { players -> link Person(name = "Pat") { name } } }"#,
    );
}
#[test]
fn aps24_phase2_live_and_roster() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    let bytes = db.snapshot_bytes().unwrap();
    db.restore_bytes(&bytes).unwrap();
    let before = db.lock_graph().unwrap().history.accesses();
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person { name } } }"#
        ),
        json!({"players":[]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Flames") { players -> Person { name } } }"#
        ),
        json!({"players":[{"name":"Pat"}]})
    );
    assert_eq!(db.lock_graph().unwrap().history.accesses(), before);
    assert_eq!(before, 0);
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person { name } } } as of 2024-01-15"#
        ),
        json!({"players":[{"name":"Pat"}]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(has team(name = "Oilers")) limit 100 { name } } as of 2024-01-15"#
        ),
        json!([{"name":"Pat"}])
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person { name } } } as of 2024-02-01"#
        ),
        json!({"players":[]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Flames") { players -> Person { name } } } as of 2024-01-15"#
        ),
        json!({"players":[]})
    );
}
#[test]
fn aps24_phase2_family_tree_and_first_time() {
    let db = Zega::in_memory().build().unwrap();
    run(
        &db,
        r#"mutation { Person(name: "Parent" && born: "1800-01-01" && died: "1880-01-01") { children -> Person(name: "Child" && born: "1830-01-01") { children -> Person(name: "Grandchild" && born: "1860-01-01") { name } } } }"#,
    );
    assert_eq!(
        run(&db, r#"{ Person limit 100 { name } } as of 1820-01-01"#),
        json!([{"name":"Parent"}])
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Parent") { children -> Person { name children -> Person { name } } } } as of 1840-01-01"#
        ),
        json!({"children":[{"name":"Child","children":[]}]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Parent") { name } } as of 1880-01-01"#
        ),
        json!(null)
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Parent") { @firstTime(has children) } }"#
        ),
        json!({"firstTime":"1830-01-01T00:00"})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Grandchild") { @firstTime(has children) } }"#
        ),
        json!({"firstTime":null})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(@firstTime(has children) < 1850) limit 100000 { name } }"#
        ),
        json!([{"name":"Parent"}])
    );
}
#[test]
fn aps24_phase2_windows_changes_readd_and_tests() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Pat") { @lastTime(has team(name = "Oilers")) } }"#
        ),
        json!({"lastTime":"2023-10-10T00:00"})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person during 2023-10-10 to 2024-04-18 { name } } }"#
        ),
        json!({"players":[{"name":"Pat"}]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person changes from 2024-01-01 to 2024-03-08 { name } } }"#
        ),
        json!({"players":{"joined":[],"left":[{"name":"Pat"}],"changed":[]}})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(name = "Pat") { @firstTime(has team(name = "Oilers")) never: @firstTime(has team(name = "Jets")) } }"#
        ),
        json!({"firstTime":"2023-10-10T00:00","never":null})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(ever has team(name = "Oilers")) limit 100 { name } }"#
        ),
        json!([{"name":"Pat"}])
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(always has team(name = "Oilers") during 2023-11-01 to 2024-01-01) limit 100 { name } }"#
        ),
        json!([{"name":"Pat"}])
    );
    assert_eq!(
        run(
            &db,
            r#"{ Person(always has team(name = "Oilers") during 2023-11-01 to 2024-03-01) limit 100 { name } }"#
        ),
        json!([])
    );
    run(
        &db,
        r#"mutation at 2024-03-01 { Team(name = "Oilers") { players -> link Person(name = "Pat") { name } } }"#,
    );
    run(
        &db,
        r#"mutation at 2024-03-01 { Team(name = "Oilers") set points: 30 { points } }"#,
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person changes from 2024-01-01 to 2024-03-08 { name } } }"#
        ),
        json!({"players":{"joined":[],"left":[],"changed":[]}})
    );
    let delta = run(
        &db,
        r#"{ Team(name = "Oilers") { points } } changes from 2024-01-01 to 2024-03-08"#,
    );
    assert_eq!(delta["changed"][0]["from"], json!({"points":10}));
    assert_eq!(delta["changed"][0]["to"], json!({"points":30}));
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person during 2024-02-10 to 2024-02-20 { name } } }"#
        ),
        json!({"players":[]})
    );
}
#[test]
fn aps24_phase2_hist_wal_and_schema_diff() {
    let dir = tempfile::tempdir().unwrap();
    let db = Zega::open(dir.path().to_str().unwrap())
        .snapshot_every(0)
        .wal_flush_every_write()
        .build()
        .unwrap();
    seed(&db);
    let without = ROSTER
        .replace("<Person[]>", "Person[]")
        .replace("<Team[]>", "Team[]");
    assert!(!db.schema_diff(ROSTER, &without).unwrap().ok);
    assert!(db.schema_diff(&without, ROSTER).unwrap().ok);
    assert!(
        db.schema_diff(
            "type Person { born: Date }",
            "type Person { born: Date appears at born }"
        )
        .unwrap()
        .ok
    );
    let query = r#"{ Team(name = "Oilers") { players -> Person { name } } } as of 2024-01-01"#;
    let expected = run(&db, query);
    let snapshot = db.snapshot_bytes().unwrap();
    let mut file = Vec::new();
    db.export(&mut file).unwrap();
    drop(db);
    let replayed = Zega::open(dir.path().to_str().unwrap())
        .snapshot_every(0)
        .build()
        .unwrap();
    assert_eq!(run(&replayed, query), expected);
    assert_eq!(
        run(
            &replayed,
            r#"{ Team(name = "Oilers") { players -> Person { name } } }"#
        ),
        json!({"players":[]})
    );
    for file_format in [false, true] {
        let restored = Zega::in_memory().build().unwrap();
        if file_format {
            restored.import(&file[..]).unwrap();
        } else {
            restored.restore_bytes(&snapshot).unwrap();
        }
        assert_eq!(restored.lock_graph().unwrap().history.accesses(), 0);
        assert_eq!(run(&restored, query), expected);
        assert_eq!(
            run(
                &restored,
                r#"{ Person limit 100 { name } } as of 1800-01-01"#
            ),
            json!([])
        );
    }
}
#[test]
fn aps24_phase2_exact_decision_examples_parse() {
    // Kept verbatim from APS 24, including year and season literals.
    crate::lang::parse_schema(
        r#"type Team {
  name: String
  points: <Int>
  lineup: <Int[]>
  splits: <Int>[]
  players -> <Person[]>
}
type Person {
  name: String
  born: Date
  children -> Person[]
  appears at born
}"#,
    )
    .unwrap();
    for query in [
        r#"{ Team limit 100 { name points wins } } as of 2024-01-15"#,
        r#"{ Team(name = "Oilers") { name <points> } } from 2023-10-10 to 2024-04-18 by week"#,
        r#"{ Team(name = "Oilers") { players -> Person during 2023-10-10 to 2024-04-18 { name } } }"#,
        r#"{ Team(ever points > 100) limit 100 { name } }"#,
        r#"{ Team(always points >= 50 during season 23) limit 100 { name } }"#,
        r#"{ Team(name = "Oilers") { players -> Person changes from 2024-01-01 to 2024-03-08 { name } } }"#,
        r#"{ Team limit 100 { name @firstTime(points >= 100) } }"#,
        r#"{ Person(@firstTime(has team(name = "Oilers")) > 2023-10-01) limit 1000 { name } }"#,
        r#"{ Person(@firstTime(has children) < 1850) limit 100000 { name } }"#,
        r#"{ Person(name = "Chris Evans") { name @firstTime(has acted_in(has genres(name = "superhero"))) } }"#,
        r#"{ Account limit 100000 { name @firstTime(plan = "pro") @lastTime(plan = "pro") } }"#,
        r#"{ Team(ever points > 100 && @firstTime(points > 100) < 2024-01-01) limit 100 { name <points> } } from 2023-10-10 to 2024-04-18 by week"#,
    ] {
        crate::lang::parse_statement(query).unwrap_or_else(|e| panic!("{query}: {e}"));
        let formatted =
            crate::lang::fmt::format_zql(query).unwrap_or_else(|e| panic!("{query}: {e}"));
        crate::lang::parse_statement(&formatted).unwrap();
    }
}

#[test]
fn aps24_phase2_differential_relationships_and_lifetimes() {
    // The reference is just independent (source, target, start, end) tuples.
    // It knows nothing about HIST, adjacency, indexes, or the executor.
    let db = Zega::in_memory().build().unwrap();
    let schema =
        "type P { n: Int born: Date gone?: Date peers -> <P[]> appears at born ends at gone }";
    let date = |day: u64| format!("2024-01-{:02}", day + 1);
    for n in 0..8 {
        db.run_lang(
            schema,
            &format!(
                "mutation {{ P(n: {n} && born: \"{}\" && gone: \"{}\") {{ n }} }}",
                date(n),
                date(24 + n % 4)
            ),
        )
        .unwrap();
    }
    let mut intervals: Vec<(u64, u64, u64, Option<u64>)> = Vec::new();
    let mut rng = 2402u64;
    for step in 0..96 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let from = (rng >> 32) % 8;
        let to = (from + 1 + ((rng >> 40) % 7)) % 8;
        let day = 8 + step / 6;
        let active = intervals
            .iter()
            .position(|(a, b, _, end)| *a == from && *b == to && end.is_none());
        let verb = if let Some(index) = active {
            intervals[index].3 = Some(day);
            "unlink"
        } else {
            intervals.push((from, to, day, None));
            "link"
        };
        db.run_lang(
            schema,
            &format!(
                "mutation at {} {{ P(n = {from}) {{ peers -> {verb} P(n = {to}) {{ n }} }} }}",
                date(day)
            ),
        )
        .unwrap();
        for at in [0, 7, 10, 15, 22, 28] {
            for source in 0..8 {
                let present = source <= at && at < 24 + source % 4;
                let mut expected: Vec<_> = intervals
                    .iter()
                    .filter(|(a, b, start, end)| {
                        *a == source
                            && *start <= at
                            && end.is_none_or(|end| at < end)
                            && *b <= at
                            && at < 24 + *b % 4
                    })
                    .map(|(_, b, _, _)| *b)
                    .collect();
                expected.sort_unstable();
                expected.dedup();
                let actual = db
                    .run_lang(
                        schema,
                        &format!(
                            "{{ P(n = {source}) {{ peers -> P order by n {{ n }} }} }} as of {}",
                            date(at)
                        ),
                    )
                    .unwrap();
                let expected = if present {
                    json!({"peers":expected.into_iter().map(|n| json!({"n":n})).collect::<Vec<_>>()})
                } else {
                    json!(null)
                };
                assert_eq!(actual, expected, "step={step}, source={source}, at={at}");
            }
        }
        if step % 16 == 0 {
            let bytes = db.snapshot_bytes().unwrap();
            db.restore_bytes(&bytes).unwrap();
            let mut file = Vec::new();
            db.export(&mut file).unwrap();
            db.import(&file[..]).unwrap();
        }
    }
}

#[test]
fn aps24_phase2_nested_relationship_test_and_edges() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type Person { name: String acted_in -> <Film[]> { role: String } } type Film { name: String genres -> <Genre[]> } type Genre { name: String }";
    db.run_lang(schema, r#"mutation at 2011-07-22 { Person(name: "Chris Evans") { acted_in -> Film(name: "Captain America") { &role: "Steve" genres -> Genre(name: "superhero") { name } } } }"#).unwrap();
    db.run_lang(schema, r#"mutation at 2020-01-01 { Person(name = "Chris Evans") { acted_in -> unlink Film(name = "Captain America") { name } } }"#).unwrap();
    assert_eq!(db.run_lang(schema, r#"{ Person(name = "Chris Evans") { name @firstTime(has acted_in(has genres(name = "superhero"))) } }"#).unwrap(), json!({"name":"Chris Evans","firstTime":"2011-07-22T00:00"}));
    assert_eq!(db.run_lang(schema, r#"{ Person(name = "Chris Evans") { acted_in -> Film { name &role } } } as of 2012-01-01"#).unwrap(), json!({"acted_in":[{"name":"Captain America","role":"Steve"}]}));
}

#[test]
#[ignore = "explicit NHL relationship storage measurement"]
fn aps24_phase2_relationship_bytes() {
    use crate::{
        graph::Graph,
        journal::atomically,
        wal::{Wal, WalError},
    };
    use std::collections::HashMap;
    let schema =
        crate::lang::parse_schema("type T { players -> <P[]> } type P { name: String }").unwrap();
    let mut graph = Graph::new();
    let teams: Vec<_> = (0..32)
        .map(|_| graph.create_node(vec!["T".into()], HashMap::new()))
        .collect();
    let players: Vec<_> = (0..700)
        .map(|_| graph.create_node(vec!["P".into()], HashMap::new()))
        .collect();
    let wal = Wal::in_memory();
    let start = crate::history::date("2023-10-10").unwrap();
    let mut edges = Vec::new();
    for (i, player) in players.iter().enumerate() {
        let id = atomically::<_, WalError>(&mut graph, &wal, |g, j| {
            j.configure_time(&schema, Some(start));
            Ok(j.create_relationship(g, "players".into(), teams[i % 32], *player, HashMap::new()))
        })
        .unwrap();
        edges.push(id);
    }
    let initial = graph.history.bytes().unwrap().unwrap().len();
    for (i, player) in players.iter().enumerate().take(150) {
        atomically::<_, WalError>(&mut graph, &wal, |g, j| {
            j.configure_time(&schema, Some(start + (i as i64 + 1) * 86400));
            j.delete_relationship(g, edges[i]);
            j.create_relationship(
                g,
                "players".into(),
                teams[(i + 1) % 32],
                *player,
                HashMap::new(),
            );
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(graph.relationship_count(), 700);
    let bytes = graph.history.bytes().unwrap().unwrap().len();
    println!("APS24 NHL teams=32 players=700 trades=150 relationship_changes=300 initial_hist_bytes={initial} final_hist_bytes={bytes} incremental_bytes={} bytes_per_relationship_change={:.3} bytes_per_trade={:.3}", bytes-initial, (bytes-initial) as f64 / 300., (bytes-initial) as f64 / 150.);
}

#[test]
fn aps24_phase2_reads_real_phase1_files() {
    // Generated by origin/main (phase 1), not by the codec under test.
    for graph_file in [false, true] {
        let db = Zega::in_memory().build().unwrap();
        if graph_file {
            db.import(&include_bytes!("../tests/fixtures/time-phase1/history.graph")[..])
                .unwrap();
        } else {
            db.restore_bytes(include_bytes!("../tests/fixtures/time-phase1/snapshot.bin"))
                .unwrap();
        }
        assert_eq!(db.lock_graph().unwrap().history.accesses(), 0);
        assert_eq!(
            db.run_lang(
                "type T { n: <Int> }",
                "{ T limit 10 { n } } as of 2024-01-15"
            )
            .unwrap(),
            json!([{"n":10}])
        );
    }
}

#[test]
fn aps24_phase2_backdated_end_preserves_later_membership() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    run(
        &db,
        r#"mutation at 2024-03-01 { Team(name = "Oilers") { players -> link Person(name = "Pat") { name } } }"#,
    );
    run(
        &db,
        r#"mutation at 2024-01-15 { Team(name = "Oilers") { players -> unlink Person(name = "Pat") { name } } }"#,
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person { name } } } as of 2024-01-20"#
        ),
        json!({"players":[]})
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { players -> Person { name } } }"#
        ),
        json!({"players":[{"name":"Pat"}]})
    );
}

#[test]
fn aps24_phase2_lifetime_added_to_existing_dates() {
    let db = Zega::in_memory().build().unwrap();
    let old = "type P { born: Date n: <Int> }";
    db.run_lang(
        old,
        r#"mutation at 1800-01-01 { P(born: "1900-01-01" && n: 1) { n } }"#,
    )
    .unwrap();
    let new = "type P { born: Date n: <Int> appears at born }";
    assert_eq!(
        db.run_lang(new, "{ P limit 10 { n } } as of 1850-01-01")
            .unwrap(),
        json!([])
    );
    assert_eq!(
        db.run_lang(new, "{ P limit 10 { @firstTime(n > 0) } }")
            .unwrap(),
        json!([{"firstTime":"1900-01-01T00:00"}])
    );
}

#[test]
fn aps24_phase2_zero_width_window_and_unresolved_season() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { points } } during 2023-10-10 to 2023-10-10"#
        ),
        json!([{"points":10}])
    );
    assert_eq!(
        run(
            &db,
            r#"{ Team(name = "Oilers") { points } } changes from 2023-10-10 to 2023-10-10"#
        ),
        json!({"joined":[],"left":[],"changed":[]})
    );
    assert!(db
        .run_lang(
            ROSTER,
            "{ Team(always points >= 50 during season 23) limit 100 { name } }"
        )
        .unwrap_err()
        .to_string()
        .contains("has no calendar season"));
}

#[test]
fn aps24_phase2_always_uses_the_nodes_own_lifetime() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type P { name: String born: Date peers -> <P[]> appears at born }";
    db.run_lang(
        schema,
        r#"mutation at 1900-01-01 { P(name: "Old" && born: "1900-01-01") { name } }"#,
    )
    .unwrap();
    db.run_lang(schema, r#"mutation at 2024-01-01 { P(name: "New" && born: "2024-01-01") { peers -> P(name: "Peer" && born: "2024-01-01") { name } } }"#).unwrap();
    assert_eq!(
        db.run_lang(schema, "{ P(always has peers) limit 10 { name } }")
            .unwrap(),
        json!([{"name":"New"}])
    );
}

#[test]
fn aps24_phase2_hist_property_encoding_is_deterministic() {
    let fields = (0..32)
        .map(|i| format!("f{i}: Int"))
        .collect::<Vec<_>>()
        .join(" ");
    let values = (0..32)
        .map(|i| format!("&f{i}: {i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let schema = format!("type A {{ rs -> <B[]> {{ {fields} }} }} type B {{ n: Int }}");
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(
        &schema,
        &format!("mutation at 2024-01-01 {{ A {{ rs -> B(n: 1) {{ {values} }} }} }}"),
    )
    .unwrap();
    let expected = db.lock_graph().unwrap().history.bytes().unwrap();
    for _ in 0..4 {
        let snapshot = db.snapshot_bytes().unwrap();
        db.restore_bytes(&snapshot).unwrap();
        let mut graph = db.lock_graph().unwrap();
        // Force re-encoding after lazy decoding, rather than returning the
        // original encoded bytes. Map hash seeds must not change file identity.
        graph.history.get_mut().unwrap();
        assert_eq!(graph.history.bytes().unwrap(), expected);
    }
}

#[test]
fn aps24_phase2_window_hops_are_taken_at_one_instant() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type P { n: Int next -> <P[]> }";
    db.run_lang(
        schema,
        "mutation at 2024-01-01 { P(n: 1) { next -> P(n: 2) { next -> P(n: 3) { n } } } }",
    )
    .unwrap();
    db.run_lang(
        schema,
        "mutation at 2024-02-01 { P(n = 2) { next -> unlink P(n = 3) { n } } }",
    )
    .unwrap();
    assert_eq!(
        db.run_lang(
            schema,
            "{ P(n = 1) { next *2..2 -> P during 2024-01-01 to 2024-03-01 { n @hops } } }"
        )
        .unwrap(),
        json!({"next":[{"n":3,"hops":2}]})
    );
    assert_eq!(
        db.run_lang(
            schema,
            "{ P(n = 1) { next *2..2 -> P during 2024-02-01 to 2024-03-01 { n } } }"
        )
        .unwrap(),
        json!({"next":[]})
    );
}
