//! The bulk path (zegadb/zega#127, #128): a JSON-lines graph of any size,
//! loaded all or nothing with `Zega::import_json_lines`, stored on disk as
//! one imported `.graph` file and one WAL entry, on both storages.

use std::collections::BTreeMap;

use serde_json::{json, Value as Json};
use zega::graph_file::json_lines::JsonLinesOptions;
use zega::graph_file::{self, Error};
use zega::{Storage, Zega, ZegaError};

const SCHEMA: &str = "schema {
  type Item {
    qid: String
    name?: String
    pop?: Int
    aliases?: String[]
    at?: Point
    links: LINK -> Item[]
  }
}

unique {
  Item { qid }
}
";

/// Three items, two links (one with a property), one id left to the loader.
const LINES: &str = r#"{"type":"node","id":10,"labels":["Item"],"props":{"qid":"Q10","name":"Ottawa","pop":1017449,"aliases":["Bytown"],"at":{"lat":45.4215,"lon":-75.6972}}}
{"type":"node","id":20,"labels":["Item"],"props":{"qid":"Q20","name":"Toronto","pop":2794356,"at":{"lat":43.6532,"lon":-79.3832}}}

{"type":"node","labels":["Item","City"],"props":{"qid":"Q21","name":"Montreal"}}
{"type":"rel","kind":"LINK","from":10,"to":20,"props":{"weight":3}}
{"type":"rel","id":7,"kind":"LINK","from":20,"to":21}
"#;

fn options() -> JsonLinesOptions {
    JsonLinesOptions {
        schema: Some(SCHEMA.to_string()),
        meta: BTreeMap::from([("source".to_string(), "test".to_string())]),
    }
}

fn rows(result: Json) -> Vec<Json> {
    match result {
        Json::Array(rows) => rows,
        Json::Null => Vec::new(),
        row => vec![row],
    }
}

fn export(zega: &Zega) -> Vec<u8> {
    let mut bytes = Vec::new();
    zega.export(&mut bytes).unwrap();
    bytes
}

fn json_lines_error(result: Result<graph_file::ImportSummary, ZegaError>) -> (u64, String) {
    match result {
        Err(ZegaError::GraphFile(Error::JsonLines { line, reason })) => (line, reason),
        other => panic!("expected a JSON-lines error, got {other:?}"),
    }
}

fn storages() -> Vec<Storage> {
    let mut storages = vec![Storage::File];
    #[cfg(feature = "sqlite")]
    storages.push(Storage::Sqlite);
    storages
}

fn open(path: &std::path::Path, storage: Storage) -> Zega {
    Zega::open(path.to_str().unwrap()).storage(storage).build().unwrap()
}

/// Every file in the data directory's `graphs/`.
fn imported_files(path: &std::path::Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(path.join("graphs"))
        .map(|dir| dir.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

#[test]
fn a_json_lines_graph_loads_its_ids_labels_properties_and_relationships() {
    let zega = Zega::in_memory().build().unwrap();
    let summary = zega.import_json_lines(LINES.as_bytes(), &options()).unwrap();
    assert_eq!((summary.nodes, summary.relationships), (3, 2));
    assert_eq!(summary.uniques, [("Item".to_string(), "qid".to_string())]);
    assert_eq!(summary.meta.get("source").map(String::as_str), Some("test"));

    let ottawa = rows(zega.run_lang(SCHEMA, r#"{ Item(qid: "Q10") { @id name pop aliases links -> Item { @id name } } }"#).unwrap());
    assert_eq!(ottawa.len(), 1, "{ottawa:?}");
    assert_eq!(ottawa[0]["id"], json!(10));
    assert_eq!(ottawa[0]["pop"], json!(1017449));
    // `String[]` in the schema makes the array a list, as `mutation json` does.
    assert_eq!(ottawa[0]["aliases"], json!(["Bytown"]));
    assert_eq!(ottawa[0]["links"][0]["name"], json!("Toronto"));
    // The node without an id took the next one after the highest given.
    let montreal = rows(zega.run_lang(SCHEMA, r#"{ Item(qid: "Q21") { @id name } }"#).unwrap());
    assert_eq!(montreal[0]["id"], json!(21));
    let toronto = rows(zega.run_lang(SCHEMA, r#"{ Item(qid: "Q20") { links -> Item { name } } }"#).unwrap());
    assert_eq!(toronto[0]["links"][0]["name"], json!("Montreal"));
    // `Point` fields are points: nearest to Toronto's coordinates first.
    let near = rows(
        zega.run_lang(SCHEMA, "{ Item order by @distance(at, @point(43.6, -79.4)) limit 1 { name } }").unwrap(),
    );
    assert_eq!(near[0]["name"], json!("Toronto"));
    // The unique index is live: a mutation can't add a second Q10.
    assert!(zega.run_lang(SCHEMA, r#"mutation { Item(qid: "Q10" && name: "again") }"#).is_err());

    // The graph is an ordinary imported graph: its `.graph` file loads back
    // to the same bytes, with the schema and metadata it was loaded with.
    let bytes = export(&zega);
    let again = Zega::in_memory().build().unwrap();
    let reimported = again.import(&bytes[..]).unwrap();
    assert_eq!(export(&again), bytes);
    assert_eq!(reimported.content_sha256, summary.content_sha256);
    assert_eq!(reimported.schema.as_deref(), Some(SCHEMA));
}

#[test]
fn on_disk_a_load_is_one_imported_file_that_survives_reopen() {
    for storage in storages() {
        let dir = tempfile::tempdir().unwrap();
        let (summary, bytes) = {
            let zega = open(dir.path(), storage);
            let summary = zega.import_json_lines(LINES.as_bytes(), &options()).unwrap();
            (summary, export(&zega))
        };
        // The graph is stored as the `.graph` file named by its digest: the
        // same bytes an export writes.
        let files = imported_files(dir.path());
        assert_eq!(files.len(), 1, "{storage:?}: {files:?}");
        let stored = std::fs::read(dir.path().join("graphs").join(&files[0])).unwrap();
        assert_eq!(stored, bytes, "{storage:?}");
        let zega = open(dir.path(), storage);
        assert_eq!(export(&zega), bytes, "{storage:?}: reopen changed the graph");
        let again = Zega::in_memory().build().unwrap();
        assert_eq!(again.import(&stored[..]).unwrap().content_sha256, summary.content_sha256);

        // Writes after the load replay on top of it.
        zega.run_lang(SCHEMA, r#"mutation { Item(qid: "Q99" && name: "Halifax") }"#).unwrap();
        drop(zega);
        let zega = open(dir.path(), storage);
        let count = rows(zega.run_lang(SCHEMA, "{ Item { qid } }").unwrap()).len();
        assert_eq!(count, 4, "{storage:?}");
    }
}

/// Each input is refused at the line named, and a database that already
/// held a graph holds exactly that graph afterwards, in memory and on disk,
/// with no file left behind.
#[test]
fn a_bad_line_anywhere_changes_nothing() {
    let good = LINES.trim_end();
    let cases: Vec<(String, u64, &str)> = vec![
        (format!("{good}\n{{\"type\":\"rel\",\"kind\":\"LINK\",\"from\":10,\"to\":404}}"), 7, "node 404, which no earlier line defines"),
        (format!("{good}\n{{\"type\":\"node\",\"id\":20,\"labels\":[\"Item\"]}}"), 7, "node 20 is already defined"),
        (format!("{good}\n{{\"type\":\"rel\",\"id\":7,\"kind\":\"LINK\",\"from\":10,\"to\":10}}"), 7, "relationship 7 is already defined"),
        (format!("{good}\n{{\"type\":\"node\",\"labels\":[\"Item\"],\"props\":{{\"qid\":\"Q20\"}}}}"), 7, "which node 20 already has"),
        (format!("{good}\n{{\"type\":\"node\",\"labels\":[\"Item\"],\"props\":{{\"aliases\":\"one\"}}}}"), 7, "list field needs an array"),
        (format!("{good}\n{{\"type\":\"node\",\"labels\":[\"Item\",\"Item\"]}}"), 7, "the label \"Item\" twice"),
        (format!("{good}\n{{\"type\":\"node\",\"colour\":\"red\"}}"), 7, "unknown field `colour`"),
        (format!("{good}\n{{\"type\":\"edge\"}}"), 7, "unknown variant `edge`"),
        (format!("{good}\n{{\"type\":\"rel\",\"kind\":\"LINK\",\"from\":10}}"), 7, "needs to"),
        (format!("{good}\n{{\"type\":\"node\",\"kind\":\"LINK\"}}"), 7, "a node has no kind"),
        (format!("{good}\n{{\"type\":\"node\""), 7, "EOF"),
        (format!("{{\"type\":\"node\"}}\n{}", "x".repeat(graph_file::json_lines::MAX_LINE_BYTES + 1)), 2, "longer than 16 MiB"),
    ];
    for storage in storages() {
        let dir = tempfile::tempdir().unwrap();
        let zega = open(dir.path(), storage);
        zega.run_lang(SCHEMA, r#"mutation { Item(qid: "Q1" && name: "before") }"#).unwrap();
        let before = export(&zega);
        let files = imported_files(dir.path());
        for (input, line, reason) in &cases {
            let (at, why) = json_lines_error(zega.import_json_lines(input.as_bytes(), &options()));
            assert_eq!((at, why.contains(reason)), (*line, true), "{storage:?}: {why} (line {at})");
            assert_eq!(export(&zega), before, "{storage:?}: {reason}");
            assert_eq!(imported_files(dir.path()), files, "{storage:?}: {reason}");
        }
        drop(zega);
        assert_eq!(export(&open(dir.path(), storage)), before, "{storage:?}: reopen");
    }
}

#[test]
fn without_a_schema_values_load_as_untyped_json_does() {
    let zega = Zega::in_memory().build().unwrap();
    let lines = r#"{"type":"node","labels":["T"],"props":{"n":1,"x":1.5,"s":"a","b":true,"none":null,"at":{"lat":1,"lon":2}}}"#;
    let summary = zega.import_json_lines(lines.as_bytes(), &JsonLinesOptions::default()).unwrap();
    assert_eq!((summary.nodes, summary.schema), (1, None));
    let schema = "schema { type T { n: Int x: Float s: String b: Boolean at: Point } }";
    let row = rows(zega.run_lang(schema, "{ T { n x s b at } }").unwrap());
    assert_eq!(row[0]["n"], json!(1));
    assert_eq!(row[0]["x"], json!(1.5));
    assert_eq!(row[0]["s"], json!("a"));
    assert_eq!(row[0]["b"], json!(true));
    assert_eq!(row[0]["at"]["lat"], json!(1.0));
    // A list of strings has no untyped meaning (an array is a vector):
    // it needs a `T[]` field in a schema, exactly as in `mutation json`.
    let (line, reason) = json_lines_error(
        zega.import_json_lines(r#"{"type":"node","props":{"tags":["a"]}}"#.as_bytes(), &JsonLinesOptions::default()),
    );
    assert_eq!(line, 1);
    assert!(reason.starts_with("property \"tags\""), "{reason}");
}

#[test]
fn an_empty_input_loads_an_empty_graph_and_a_bad_schema_is_refused() {
    let zega = Zega::in_memory().build().unwrap();
    zega.run_lang(SCHEMA, r#"mutation { Item(qid: "Q1") }"#).unwrap();
    let summary = zega.import_json_lines(&b""[..], &JsonLinesOptions::default()).unwrap();
    assert_eq!((summary.nodes, summary.relationships), (0, 0));
    assert!(zega.is_empty().unwrap());

    let bad = JsonLinesOptions { schema: Some("schema { type".into()), ..Default::default() };
    let (line, reason) = json_lines_error(zega.import_json_lines(LINES.as_bytes(), &bad));
    assert_eq!(line, 0);
    assert!(reason.starts_with("the schema does not parse"), "{reason}");
}
