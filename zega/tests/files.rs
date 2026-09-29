//! `String<blake3>` and `@image` over blake3 (APS 34, docs/files.md). The
//! shape of these tests mirrors tests/node_shapes.rs, which pins `String<url>`.
use serde_json::json;
use std::collections::HashMap;
use zega::Zega;

const SCHEMA: &str = "type File { name: String mediaType: String size: Int hash: String<blake3> width?: Int height?: Int duration?: Float source?: String<url> licence: String author?: String credit?: String fetchedAt?: String } display { graph { File(@shape: document, @image: &hash) } }";

/// blake3 of "hello\n", as `b3sum` prints it.
const HASH: &str = "8e4c7c1b99dbfd50e7a95185fead5ee1448fa904a2fdd778eaf5f2dbfd629a99";

fn photo() -> (Vec<u8>, String) {
    let bytes = b"\x89PNG not really a png".to_vec();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    (bytes, hash)
}

#[test]
fn blake3_of_test_vectors_matches_the_crate() {
    // The refinement and the bundle agree on what a blake3 hash is: 64
    // lowercase hex of the bytes' digest.
    assert_eq!(blake3::hash(b"hello\n").to_hex().as_str(), HASH);
}

#[test]
fn blake3_writes_updates_and_string_operations() {
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "a.png" && mediaType: "image/png" && size: 3 && hash: "{HASH}" && licence: "CC0") }}"#)).unwrap();
    assert_eq!(
        db.run_lang(SCHEMA, r#"query { File(hash startsExact "8e4c") { name hash source } }"#).unwrap(),
        json!([{"name":"a.png","hash":HASH,"source":null}])
    );
    let (_, other) = photo();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "a.png") set hash: "{other}" {{ hash }} }}"#)).unwrap();
    assert_eq!(
        db.run_lang(SCHEMA, r#"query { File(name: "a.png") { hash } }"#).unwrap(),
        json!({"hash":other})
    );
}

#[test]
fn invalid_blake3_writes_and_updates_are_atomic() {
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "a.png" && mediaType: "image/png" && size: 3 && hash: "{HASH}" && licence: "CC0") }}"#)).unwrap();
    let before = db.graph_json().unwrap();
    let bad: Vec<String> = vec![
        // Too short, too long, uppercase, not hex, a bare filename, a url.
        format!("\"{}\"", &HASH[..63]),
        format!("\"{HASH}0\""),
        format!("\"{}\"", HASH.to_uppercase()),
        format!("\"{}zz\"", &HASH[..62]),
        "\"photo.png\"".into(),
        format!("\"https://example.com/{HASH}\""),
        "42".into(),
        "true".into(),
        "null".into(),
    ];
    for value in bad {
        for source in [
            format!("mutation {{ File(name: \"b.png\" && mediaType: \"image/png\" && size: 1 && hash: {value} && licence: \"CC0\") }}"),
            format!("mutation {{ File(name: \"a.png\") set hash: {value} {{ hash }} }}"),
        ] {
            let error = db.run_lang(SCHEMA, &source).unwrap_err().to_string();
            assert!(error.contains("File.hash must be String<blake3>"), "{error}");
            assert!(error.contains("64 lowercase hex"), "{error}");
            assert_eq!(db.graph_json().unwrap(), before);
        }
    }
}

#[test]
fn blake3_import_validation_rejects_entire_batch() {
    let db = Zega::in_memory().build().unwrap();
    for (format, rows) in [
        ("json", &format!(r#"[{{"name":"a.png","hash":"{HASH}"}},{{"name":"b.png","hash":"broken"}}]"#)),
        ("csv", &format!("name,hash\na.png,{HASH}\nb.png,broken\n")),
    ] {
        let schema = "type File { name: String hash: String<blake3> }";
        let source = format!("mutation {format} [\"rows.{format}\"] {{ File(name: $name && hash: $hash) }}");
        let sources = HashMap::from([(format!("rows.{format}"), rows.as_str().into())]);
        let error = db.run_lang_with_sources(schema, &source, &sources).unwrap_err().to_string();
        assert!(error.contains("File.hash must be String<blake3>"), "{error}");
        assert_eq!(db.graph_json().unwrap()["nodes"], json!([]));
    }
}

#[test]
fn blake3_type_grammar_and_edge_properties() {
    let db = Zega::in_memory().build().unwrap();
    for ty in ["Float<blake3>", "Int<blake3>", "String<blake2>", "Bool<blake3>"] {
        assert!(db.schema(&format!("type T {{ hash: {ty} }}")).is_err());
    }
    let schema = "type Scan { name: String from -> Scan[] { origin: String<blake3> } }";
    db.run_lang(schema, &format!(r#"mutation {{ Scan(name: "A") {{ from -> Scan(name: "B") {{ &origin: "{HASH}" }} }} }}"#)).unwrap();
    assert!(db
        .run_lang(schema, r#"mutation { Scan(name: "C") { from -> Scan(name: "D") { &origin: "not-a-hash" } } }"#)
        .is_err());
    assert_eq!(
        db.run_lang(schema, r#"query { Scan(name: "A") { from -> Scan { &origin } } }"#).unwrap(),
        json!({"from":[{"origin":HASH}]})
    );
}

#[test]
fn image_accepts_blake3_file_and_url_and_rejects_other_types() {
    let db = Zega::in_memory().build().unwrap();
    for ty in ["String<url>", "String<file>", "String<blake3>"] {
        let schema = format!("type Doc {{ image: {ty} }} display {{ graph {{ Doc(@image: &image) }} }}");
        db.schema(&schema).unwrap_or_else(|error| panic!("{ty}: {error}"));
    }
    for ty in ["String", "String<iso2>", "Int", "Float", "Bool", "Point"] {
        let schema = format!("type Doc {{ image: {ty} }} display {{ graph {{ Doc(@image: &image) }} }}");
        let error = db.schema(&schema).unwrap_err().to_string();
        assert!(
            error.contains("@image needs Doc.image to be String<url>, String<file> or String<blake3>"),
            "{ty}: {error}"
        );
    }
}
