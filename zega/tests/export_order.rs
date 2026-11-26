//! A graph exports to the same bytes however its in-memory maps happen to be ordered (#216).
//!
//! The sync metadata lives in `HashMap`s. Export sorts their keys through `serde_json::Value`, which is sorted only
//! while serde_json's `preserve_order` feature is off; an embedder that enables it (Cargo unifies features) must still
//! get the same bytes. CI runs this file a second time with `--features serde_json/preserve_order`, the build where
//! it can fail.

use std::collections::HashMap;

use zega::Zega;

const PEOPLE: &str = "type Person { name: String age: Int }";

fn export(zega: &Zega) -> Vec<u8> {
    let mut bytes = Vec::new();
    zega.export(&mut bytes).unwrap();
    bytes
}

#[test]
fn export_import_export_gives_the_same_bytes() {
    let zega = Zega::in_memory().build().unwrap();
    for batch in 0..3 {
        let rows: Vec<String> = (0..1000).map(|i| format!(r#"{{"Name":"person {batch}-{i}","Age":{}}}"#, i % 90)).collect();
        let sources = HashMap::from([("./p.json".to_string(), format!("[{}]", rows.join(",")))]);
        zega.run_lang_with_sources(PEOPLE, r#"mutation json ["./p.json"] { Person(name: $Name && age: $Age) { name } }"#, &sources)
            .unwrap();
    }
    let first = export(&zega);
    assert!(first.windows(4).any(|w| w == b"SYNC"), "the file carries sync metadata, the section whose maps are unordered");

    // Each import fills fresh maps, each with its own hash seed.
    let mut previous = first.clone();
    for round in 0..3 {
        let copy = Zega::in_memory().build().unwrap();
        copy.import(&previous[..]).unwrap();
        let again = export(&copy);
        let at = first.iter().zip(&again).position(|(a, b)| a != b);
        assert!(again == first, "round {round}: export after import differs from the original (first differing byte: {at:?})");
        previous = again;
    }
}
