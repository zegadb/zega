use serde_json::json;
use zega::{linked::*, Zega};
const SCHEMA: &str = "type Player { id: String name: String score: Int } unique { Player { id } }";
fn seed(db: &Zega) {
    db.run_lang(
        SCHEMA,
        r#"mutation { Player(id: "Q1" && name: "First" && score: 1) { id } }"#,
    )
    .unwrap();
}
fn edit(db: &Zega, n: i64) {
    db.run_lang(
        SCHEMA,
        &format!(r#"mutation {{ Player(id: "Q1") set score: {n} {{ score }} }}"#),
    )
    .unwrap();
}
#[test]
fn wal_checkpoint_restart_preserves_versions_history_and_leases() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    seed(&db);
    let first = db.graph_version().unwrap();
    edit(&db, 2);
    let version = db.graph_version().unwrap();
    db.subscribe(&SubscribeRequest {
        subscriber: "fan".into(),
        endpoint: None,
        ids: vec!["Q1".into()],
        lease_secs: 60,
    })
    .unwrap();
    drop(db);
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    assert_eq!(db.graph_version().unwrap(), version);
    assert_eq!(db.sync_node("Q1").unwrap().fields["score"], json!(2));
    db.checkpoint().unwrap();
    drop(db);
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    assert_eq!(db.graph_version().unwrap(), version);
    let history = db.changes_since(first).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(db.subscribers(&["Q1".into()]).unwrap()["Q1"].len(), 1);
    edit(&db, 3);
    assert!(db.graph_version().unwrap() > version);
    assert_eq!(db.sync_node("Q1").unwrap().id, "Q1");
}
#[test]
fn mirror_facts_are_read_only_and_diffs_survive_restart() {
    let source = Zega::in_memory().build().unwrap();
    seed(&source);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let fan = Zega::open(path).snapshot_every(0).build().unwrap();
    let reference = Reference::new("earth", "Q1").unwrap();
    let id = fan
        .link(&reference, &source.sync_node("Q1").unwrap())
        .unwrap();
    let err = fan
        .run_lang(
            SCHEMA,
            r#"mutation { Player(id: "Q1") set score: 7 { score } }"#,
        )
        .unwrap_err();
    assert!(err.to_string().contains("mirror facts are read-only"));
    assert!(fan
        .delete_node(id)
        .unwrap_err()
        .to_string()
        .contains("mirror facts are read-only"));
    let since = source.graph_version().unwrap();
    edit(&source, 8);
    let diff = coalesce("earth", &source.changes_since(since).unwrap());
    fan.apply_diff(&diff).unwrap();
    fan.apply_diff(&diff).unwrap();
    fan.checkpoint().unwrap();
    drop(fan);
    let fan = Zega::open(path).snapshot_every(0).build().unwrap();
    assert_eq!(
        fan.run_lang(SCHEMA, "{ Player { score } }").unwrap(),
        json!([{"score":8}])
    );
    source.delete_node(1).unwrap();
    fan.apply_diff(&coalesce(
        "earth",
        &source.changes_since(diff.graph_version).unwrap(),
    ))
    .unwrap();
    assert!(fan.mirrors("earth").unwrap()[0].1.source_gone);
    assert_eq!(fan.counts().unwrap().nodes, 1);
}
#[test]
fn rejected_identity_change_is_atomic_and_coalescing_keeps_final_fields() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    let since = db.graph_version().unwrap();
    assert!(db
        .run_lang(
            SCHEMA,
            r#"mutation { Player(id: "Q1") set id: "Q2" { id } }"#
        )
        .is_err());
    assert_eq!(db.graph_version().unwrap(), since);
    assert!(db.sync_node("Q2").is_err());
    for n in 2..=6 {
        edit(&db, n);
    }
    let diff = coalesce("earth", &db.changes_since(since).unwrap());
    assert_eq!(diff.changes.len(), 1);
    let Change::Upsert { fields, .. } = &diff.changes[0].change else {
        panic!()
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields["score"], json!(6));
}
#[test]
fn references_validate_and_round_trip_without_network() {
    let db = Zega::in_memory().build().unwrap();
    let schema = "type Bookmark { target: Reference }";
    db.run_lang(
        schema,
        r#"mutation { Bookmark(target: "zega://earth/Q2096") { target } }"#,
    )
    .unwrap();
    assert!(db
        .run_lang(
            schema,
            r#"mutation { Bookmark(target: "https://earth/Q2096") { target } }"#
        )
        .is_err());
    let mut bytes = Vec::new();
    db.export(&mut bytes).unwrap();
    let reopened = Zega::in_memory().build().unwrap();
    reopened.import(bytes.as_slice()).unwrap();
    assert_eq!(
        reopened
            .run_lang(schema, "{ Bookmark { target } }")
            .unwrap(),
        json!([{"target":"zega://earth/Q2096"}])
    );
}
#[test]
fn source_reload_reorders_slots_without_renumbering_external_ids_or_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    seed(&db);
    edit(&db, 8);
    db.subscribe(&SubscribeRequest {
        subscriber: "fan".into(),
        endpoint: None,
        ids: vec!["Q1".into()],
        lease_secs: 60,
    })
    .unwrap();
    let first = db.graph_version().unwrap();
    let before = db.sync_node("Q1").unwrap().version;
    let replacement = Zega::in_memory().build().unwrap();
    replacement
        .run_lang(
            SCHEMA,
            r#"mutation { Player(id: "Q2" && name: "Second" && score: 2) { id } }"#,
        )
        .unwrap();
    seed(&replacement);
    edit(&replacement, 9);
    let mut bytes = Vec::new();
    replacement.export(&mut bytes).unwrap();
    db.import(bytes.as_slice()).unwrap();
    assert!(db.graph_version().unwrap() > first);
    assert!(db.sync_node("Q1").unwrap().version > before);
    assert_eq!(db.sync_node("Q1").unwrap().fields["score"], json!(9));
    assert_eq!(db.subscribers(&["Q1".into()]).unwrap()["Q1"].len(), 1);
    drop(db);
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    assert_eq!(db.sync_node("Q1").unwrap().fields["score"], json!(9));
    assert_eq!(db.changes_since(first).unwrap().len(), 1);
    db.clear().unwrap();
    assert!(db.sync_node("Q1").unwrap().source_gone);
    db.checkpoint().unwrap();
    drop(db);
    let db = Zega::open(path).snapshot_every(0).build().unwrap();
    assert!(db.sync_node("Q1").unwrap().source_gone);
}
#[test]
fn a_tombstoned_identity_does_not_resolve_to_a_reused_slot_on_reload() {
    let db = Zega::in_memory().build().unwrap();
    seed(&db);
    db.prepare_linked().unwrap();
    let replacement = Zega::in_memory().build().unwrap();
    replacement
        .run_lang(
            "type City { id: String }",
            r#"mutation { City(id: "Q2") { id } }"#,
        )
        .unwrap();
    let mut bytes = Vec::new();
    replacement.export(&mut bytes).unwrap();
    db.import(bytes.as_slice()).unwrap();
    let gone = db.sync_node("Q1").unwrap();
    assert!(gone.source_gone);

    let restored = Zega::in_memory().build().unwrap();
    seed(&restored);
    bytes.clear();
    restored.export(&mut bytes).unwrap();
    db.import(bytes.as_slice()).unwrap();
    let node = db.sync_node("Q1").unwrap();
    assert!(!node.source_gone);
    assert!(node.version > gone.version);
    assert_eq!(node.fields["name"], json!("First"));
    assert!(db.sync_node("Q2").unwrap().source_gone);
}
#[test]
fn a_missing_partial_diff_requires_refetch_instead_of_silently_losing_fields() {
    let source = Zega::in_memory().build().unwrap();
    seed(&source);
    let fan = Zega::in_memory().build().unwrap();
    fan.link(
        &Reference::new("earth", "Q1").unwrap(),
        &source.sync_node("Q1").unwrap(),
    )
    .unwrap();
    source
        .run_lang(
            SCHEMA,
            r#"mutation { Player(id: "Q1") set name: "Changed" { name } }"#,
        )
        .unwrap();
    let after_lost = source.graph_version().unwrap();
    edit(&source, 3);
    let diff = coalesce("earth", &source.changes_since(after_lost).unwrap());
    assert!(fan
        .apply_diff(&diff)
        .unwrap_err()
        .to_string()
        .contains("version gap"));
    fan.link(
        &Reference::new("earth", "Q1").unwrap(),
        &source.sync_node("Q1").unwrap(),
    )
    .unwrap();
    fan.apply_diff(&diff).unwrap();
    assert_eq!(
        fan.run_lang(SCHEMA, "{ Player { name score } }").unwrap(),
        json!([{"name":"Changed","score":3}])
    );
}
#[test]
fn mirrors_preserve_point_vector_types_and_current_temporal_facts() {
    let source = Zega::in_memory().build().unwrap();
    let fan = Zega::in_memory().build().unwrap();
    let schema = "type Place { id: String at: Point embedding: Vector<2> score: <Int> }";
    source.run_lang(schema,r#"mutation at 2024-01-15 { Place(id: "Q1" && at: @point(53, -113) && embedding: @vector[1,0] && score: 30) { id } }"#).unwrap();
    let snapshot = source.sync_node("Q1").unwrap();
    assert_eq!(snapshot.types.len(), 2);
    fan.link(&Reference::new("earth", "Q1").unwrap(), &snapshot)
        .unwrap();
    let query = "{ Place @near(embedding, @vector[1,0], 1) { id @distance(at, @point(53,-113)) } }";
    assert_eq!(
        fan.run_lang(schema, query).unwrap(),
        source.run_lang(schema, query).unwrap()
    );
    let since = source.graph_version().unwrap();
    source
        .run_lang(
            schema,
            r#"mutation at 2024-01-01 { Place(id: "Q1") set score: 10 { score } }"#,
        )
        .unwrap();
    fan.apply_diff(&coalesce("earth", &source.changes_since(since).unwrap()))
        .unwrap();
    assert_eq!(
        fan.run_lang(schema, "{ Place { score } }").unwrap(),
        json!([{"score":30}])
    );
}
