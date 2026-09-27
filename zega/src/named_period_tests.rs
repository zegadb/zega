use crate::Zega;
use serde_json::{json, Value};

const SCHEMA: &str = r#"
type Season {
  year: Int starts: Date ends: Date games: Int
  period from starts to ends named by year
}
type Era { name: String starts: Date ends: Date period from starts to ends named by name }
type Team { name: String points: <Int> calendar season -> Season calendar era -> Era }
"#;
fn run(db: &Zega, query: &str) -> Value {
    db.run_lang(SCHEMA, query).unwrap()
}
fn seed() -> Zega {
    let db = Zega::in_memory().build().unwrap();
    // Deliberately insert out of chronological order.
    for (year, starts, ends, games) in [
        (2026, "2026-10-01", "2027-06-20", 82),
        (2024, "2024-10-02", "2025-06-15", 82),
        (2020, "2021-01-13", "2021-07-07", 56),
        (2025, "2025-10-03", "2026-06-16", 82),
        (2004, "2004-10-01", "2005-06-01", 0),
    ] {
        run(&db, &format!("mutation {{ Season {{ year: {year} starts: {starts} ends: {ends} games: {games} }} }}"));
    }
    run(
        &db,
        r#"mutation { Era { name: "Original Six" starts: 1942-01-01 ends: 1967-06-01 } }"#,
    );
    run(
        &db,
        r#"mutation at 2020-01-01 { Team(name: "Oilers" && points: 0) { name } }"#,
    );
    for (date, points) in [
        ("2021-01-13", 50),
        ("2021-07-07", 56),
        ("2021-07-08", 0),
        ("2024-10-02", 24),
        ("2025-10-03", 25),
        ("2026-01-01", 10),
        ("2026-06-16", 55),
        ("2026-10-01", 60),
        ("2027-01-01", 70),
        ("2027-06-20", 80),
        ("2027-06-21", 0),
    ] {
        run(&db, &format!("mutation at {date} {{ Team(name = \"Oilers\") set points: {points} {{ points }} }}"));
    }
    db
}
#[test]
fn named_periods_three_year_forms_and_cross_new_year() {
    let db = seed();
    for name in ["26", "2026", "26-27"] {
        assert_eq!(
            run(
                &db,
                &format!(
                    "{{ Team(always points >= 60 during season {name}) limit 10 {{ name }} }}"
                )
            ),
            json!([{"name":"Oilers"}])
        );
        assert_eq!(
            run(
                &db,
                &format!("{{ Team(ever points = 70 during season {name}) limit 10 {{ name }} }}")
            ),
            json!([{"name":"Oilers"}])
        );
        assert_eq!(
            run(
                &db,
                &format!("{{ Team(name = \"Oilers\") {{ points }} }} as of end of season {name}")
            ),
            json!({"points":80})
        );
    }
}
#[test]
fn named_periods_covid_uses_actual_node_dates() {
    let db = seed();
    assert_eq!(
        run(
            &db,
            "{ Team(always points >= 50 during season 20) limit 10 { name } }"
        ),
        json!([{"name":"Oilers"}])
    );
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { points } } as of start of season 20"
        ),
        json!({"points":50})
    );
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { points } } as of end of season 20"
        ),
        json!({"points":56})
    );
}
#[test]
fn named_periods_buckets_follow_nodes_and_endpoints() {
    let db = seed();
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { <points> } } from season 24 to season 26 by season"
        ),
        json!({"points":[
            {"time":"2024-10-02T00:00","value":24}, {"time":"2025-10-03T00:00","value":25}, {"time":"2026-10-01T00:00","value":60}
        ]})
    );
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { points } } as of end of season 25"
        ),
        json!({"points":55})
    );
}
#[test]
fn named_periods_bare_year_is_calendar_year() {
    let db = seed();
    assert_eq!(
        run(&db, "{ Team(name = \"Oilers\") { points } } as of 2026"),
        json!({"points":10})
    );
    assert_eq!(
        run(
            &db,
            "{ Team(always points >= 60 during 2026) limit 10 { name } }"
        ),
        json!([])
    );
    assert_eq!(
        run(
            &db,
            "{ Team(ever points = 70 during 2026) limit 10 { name } }"
        ),
        json!([])
    );
}
#[test]
fn named_periods_current_graph_mutation_and_unknown_names() {
    let db = seed();
    let q = "{ Team(name = \"Oilers\") { points } } as of start of season 27";
    let error = db.run_lang(SCHEMA, q).unwrap_err().to_string();
    assert!(error.contains("no season 2027 in Season"), "{error}");
    run(
        &db,
        "mutation { Season { year: 2027 starts: 2027-06-20 ends: 2028-01-01 games: 82 } }",
    );
    assert_eq!(run(&db, q), json!({"points":80}));
    run(
        &db,
        "mutation { Season(year = 2027) set starts: 2027-06-21 { year } }",
    );
    assert_eq!(run(&db, q), json!({"points":0}));
    for name in ["26-28", "26-25"] {
        let error = db
            .run_lang(
                SCHEMA,
                &format!("{{ Team limit 10 {{ name }} }} during season {name}"),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("consecutive"), "{error}");
    }
}
#[test]
fn named_periods_other_names_lost_season_and_empty_graph_errors() {
    let db = seed();
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { points } } as of end of era \"Original Six\""
        ),
        json!({"points":null})
    );
    assert_eq!(
        run(
            &db,
            "{ Team(name = \"Oilers\") { points } } as of end of season 04"
        ),
        json!({"points":null})
    );
    let db = Zega::in_memory().build().unwrap();
    for query in [
        "{ Team limit 10 { name } } during season 04",
        "{ Team(always points > 0 during season 04) limit 10 { name } }",
    ] {
        let error = db.run_lang(SCHEMA, query).unwrap_err().to_string();
        assert!(error.contains("no season 2004 in Season"), "{error}");
    }
    let error = db
        .run_lang(
            "type Team { name: String }",
            "{ Team limit 10 { name } } during season 26",
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("calendar season -> PeriodType"), "{error}");
    let error = db
        .run_lang(
            "type Season { year: Int } type Team { name: String calendar season -> Season }",
            "{ Team limit 10 { name } } during season 26",
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("without a period") && error.contains("period from"),
        "{error}"
    );
}
#[test]
fn named_periods_schema_validation_and_diff() {
    use crate::lang::parse_schema;
    use crate::schema_diff::{diff_schemas, Severity};
    for schema in [
        "type Season { year: Int ends: Date period from missing to ends named by year }",
        "type Season { year: Int starts: Int ends: Date period from starts to ends named by year }",
        "type Season { starts: Date ends: Date period from starts to ends named by missing }",
    ] {
        assert!(parse_schema(schema).is_err(), "{schema}");
    }
    let old = parse_schema("type Season { year: Int starts: Date ends: Date games: Int } type Era { name: String starts: Date ends: Date } type Team { name: String points: <Int> }").unwrap();
    let new = parse_schema(SCHEMA).unwrap();
    let graph = crate::graph::Graph::new();
    let added = diff_schemas(&old, &new, &graph);
    assert!(!added.changes.is_empty());
    assert!(added.changes.iter().all(|c| c.severity == Severity::Safe));
    let renamed = parse_schema(&SCHEMA.replace("calendar season", "calendar playoffs")).unwrap();
    assert!(diff_schemas(&new, &renamed, &graph)
        .changes
        .iter()
        .any(|c| c.severity == Severity::Warn));
    assert!(diff_schemas(&new, &old, &graph)
        .changes
        .iter()
        .any(|c| c.severity == Severity::Warn));
}
#[test]
fn named_periods_amendment_examples_parse_and_format() {
    use crate::lang::{parse_query, parse_schema};
    parse_schema(SCHEMA).unwrap();
    for query in [
        "mutation { Season { year: 2020 starts: 2021-01-13 ends: 2021-07-07 games: 56 } }",
        "{ Team(always points >= 50 during season 26) limit 100 { name } }",
        "{ Team(always points >= 50 during season 23) limit 100 { name } }",
        "{ Team limit 100 { name } } during season 26",
        "{ Team limit 100 { name } } during season 2026",
        "{ Team limit 100 { name } } during season 26-27",
        "{ Team limit 100 { name } } during era \"Original Six\"",
        "{ Team limit 100 { <points> } } from season 24 to season 26 by season",
        "{ Team limit 100 { name } } as of start of season 26",
        "{ Team limit 100 { name } } as of end of season 25",
    ] {
        parse_query(query).unwrap_or_else(|e| panic!("{query}: {e}"));
        let formatted = crate::lang::fmt::format_zql(query).unwrap();
        parse_query(&formatted).unwrap_or_else(|e| panic!("{formatted}: {e}"));
        assert_eq!(crate::lang::fmt::format_zql(&formatted).unwrap(), formatted);
    }
    let formatted = crate::lang::fmt::format_zql(SCHEMA).unwrap();
    parse_schema(&formatted).unwrap();
    assert_eq!(crate::lang::fmt::format_zql(&formatted).unwrap(), formatted);
}

#[test]
fn named_periods_traversal_changes_and_time_comparisons() {
    let db = seed();
    let schema = SCHEMA.replace(
        "points: <Int> calendar",
        "players -> <Person[]> points: <Int> calendar",
    ) + " type Person { name: String calendar season -> Season }";
    for query in [
        r#"mutation at 2026-10-01 { Team(name = "Oilers") set points: 60 { players -> Person(name: "Pat") { name } } }"#,
        r#"mutation at 2027-02-01 { Team(name = "Oilers") { players -> unlink Person(name = "Pat") { name } } }"#,
    ] {
        db.run_lang(&schema, query).unwrap();
    }
    assert_eq!(
        db.run_lang(
            &schema,
            r#"{ Team(name = "Oilers") { players -> Person during season 26 { name } } }"#
        )
        .unwrap(),
        json!({"players":[{"name":"Pat"}]})
    );
    assert_eq!(db.run_lang(&schema, r#"{ Team(name = "Oilers") { players -> Person changes from start of season 26 to end of season 26 { name } } }"#).unwrap(), json!({"players":{"joined":[],"left":[{"name":"Pat"}],"changed":[]}}));
    assert_eq!(
        run(
            &db,
            "{ Team(@firstTime(points = 70) > start of season 26) limit 10 { name } }"
        ),
        json!([{"name":"Oilers"}])
    );
}

#[test]
fn named_periods_nhl_games_keep_announcement_history() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type Season { year: Int starts: Date ends: Date games: <Int> period from starts to ends named by year }";
    db.run_lang(
        schema,
        "mutation at 2025-01-01 { Season { year: 2026 starts: 2026-10-01 ends: 2027-06-30 games: 82 } }",
    )
    .unwrap();
    db.run_lang(
        schema,
        "mutation at 2025-07-01 { Season(year: 2026) set games: 84 }",
    )
    .unwrap();

    assert_eq!(
        db.run_lang(schema, "{ Season(year = 2026) { games } } as of 2025-06-30")
            .unwrap(),
        json!({"games": 82})
    );
    assert_eq!(
        db.run_lang(schema, "{ Season(year = 2026) { games } }")
            .unwrap(),
        json!({"games": 84})
    );
    assert_eq!(
        db.run_lang(schema, "{ Season(year = 2026) { @firstTime(games = 84) } }")
            .unwrap(),
        json!({"firstTime": "2025-07-01T00:00"})
    );
}
