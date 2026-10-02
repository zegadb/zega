//! `run_lang_read` and `apply_zql_read` run reads and refuse everything that
//! writes, before it runs: the guard behind the HTTP `QUERY` method.
use serde_json::json;
use zega::{Zega, ZegaError};

const SCHEMA: &str = "type Person { name: String age?: Int }";
const DOCUMENT_SCHEMA: &str = "schema { type Person { name: String age?: Int } }";

fn db_with_ada() -> Zega {
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(SCHEMA, r#"mutation { Person(name: "Ada" && age: 37) }"#).unwrap();
    db
}

fn people(db: &Zega) -> serde_json::Value {
    db.run_lang(SCHEMA, "{ Person { name age } }").unwrap()
}

fn assert_not_a_read(result: Result<serde_json::Value, ZegaError>) {
    match result {
        Err(ZegaError::NotARead(_)) => {}
        other => panic!("expected NotARead, got {other:?}"),
    }
}

#[test]
fn a_read_answers_exactly_as_run_lang_does() {
    let db = db_with_ada();
    for query in ["{ Person { name age } }", "query { Person(name: \"Ada\") { age } }", "// a comment\n{ Person { name } }"] {
        assert_eq!(db.run_lang_read(SCHEMA, query).unwrap(), db.run_lang(SCHEMA, query).unwrap(), "{query}");
    }
    assert_eq!(db.run_lang_read(SCHEMA, "{ Person { name } }").unwrap(), json!([{"name": "Ada"}]));
}

#[test]
fn a_mutation_is_refused_and_the_graph_is_unchanged() {
    let db = db_with_ada();
    let before = db.graph_json().unwrap();
    for statement in [
        r#"mutation { Person(name: "Grace" && age: 85) }"#,
        r#"// a leading comment
           mutation { Person(name: "Grace" && age: 85) }"#,
        r#"mutation { Person(name: "Ada") set age: 38 { age } }"#,
        r#"mutation { delete Person(name: "Ada") }"#,
        r#"mutation at 2020-01-01 { Person(name: "Old") }"#,
    ] {
        assert_not_a_read(db.run_lang_read(SCHEMA, statement));
        assert_eq!(db.graph_json().unwrap(), before, "{statement}");
    }
    assert_eq!(people(&db), json!([{"name": "Ada", "age": 37}]));
}

#[test]
fn a_load_is_refused_before_it_reads_anything() {
    let db = db_with_ada();
    let before = db.graph_json().unwrap();
    // The file does not exist: a load that ran would fail with "cannot read", not NotARead.
    assert_not_a_read(db.run_lang_read(SCHEMA, r#"mutation json ["./nowhere.json"] { Person(name: $name) }"#));
    assert_not_a_read(db.run_lang_read(SCHEMA, r#"mutation csv ["./nowhere.csv"] { Person(name: $name) }"#));
    assert_eq!(db.graph_json().unwrap(), before);
}

#[test]
fn a_statement_that_does_not_parse_is_an_ordinary_error_not_a_refusal() {
    let db = db_with_ada();
    assert!(matches!(db.run_lang_read(SCHEMA, "MATCH this is not ZQL"), Err(ZegaError::Execution(_))));
}

#[test]
fn a_document_of_reads_runs_and_a_document_with_one_mutation_is_refused_whole() {
    let db = db_with_ada();
    let before = db.graph_json().unwrap();
    let read = format!("{DOCUMENT_SCHEMA}\nquery {{ Person {{ name }} }}");
    assert_eq!(db.apply_zql_read(&read).unwrap(), json!([{"name": "Ada"}]));
    assert_eq!(db.apply_zql_read(&read).unwrap(), db.apply_zql(&read).unwrap());

    // The mutation comes last: the refusal comes before the first block runs.
    let mixed = format!("{DOCUMENT_SCHEMA}\nquery {{ Person {{ name }} }}\nmutation {{ Person(name: \"Grace\") }}");
    assert_not_a_read(db.apply_zql_read(&mixed));
    let loading = format!("{DOCUMENT_SCHEMA}\nmutation json [\"./nowhere.json\"] {{ Person(name: $name) }}");
    assert_not_a_read(db.apply_zql_read(&loading));
    assert_eq!(db.graph_json().unwrap(), before);
}
