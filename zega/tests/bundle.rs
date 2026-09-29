//! `.zga` bundles and deterministic `.zgz` packing (APS 34, docs/files.md).
//! Every test here fails if its piece of the feature is removed.
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use zega::bundle::{Bundle, GRAPH_FILE};
use zega::graph_file::ExportOptions;
use zega::Zega;

const SCHEMA: &str = "type File { name: String mediaType: String size: Int hash: String<blake3> width?: Int height?: Int duration?: Float source?: String<url> licence: String author?: String credit?: String fetchedAt?: String } display { graph { File(@shape: document, @image: &hash) } }";

fn photo(name: &str) -> (Vec<u8>, String) {
    let bytes = format!("fake image bytes of {name}").into_bytes();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    (bytes, hash)
}

/// Write a graph with one File node per `(name, hash)` into the bundle.
fn write_graph(bundle: &Path, files: &[(&str, &str)]) {
    let db = Zega::in_memory().build().unwrap();
    for (name, hash) in files {
        db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "{name}" && mediaType: "image/png" && size: 3 && hash: "{hash}" && licence: "CC0") }}"#)).unwrap();
    }
    let options = ExportOptions { schema: Some(SCHEMA.to_string()), ..Default::default() };
    let mut out = fs::File::create(bundle.join(GRAPH_FILE)).unwrap();
    db.export_with(&mut out, &options).unwrap();
}

fn sample_bundle(dir: &Path) -> (Bundle, Vec<(Vec<u8>, String)>) {
    let bundle = Bundle::create(dir).unwrap();
    let mut assets = Vec::new();
    for name in ["a.png", "b.png"] {
        let (bytes, hash) = photo(name);
        let source = dir.with_file_name(format!("{name}.src"));
        fs::write(&source, &bytes).unwrap();
        // The same bytes twice deduplicate to one asset.
        assert_eq!(bundle.put_asset(&source).unwrap(), hash);
        assert_eq!(bundle.put_asset(&source).unwrap(), hash);
        fs::remove_file(&source).unwrap();
        assets.push((bytes, hash));
    }
    let files: Vec<(&str, &str)> = ["a.png", "b.png"]
        .iter()
        .zip(&assets)
        .map(|(name, (_, hash))| (*name, hash.as_str()))
        .collect();
    write_graph(dir, &files);
    (bundle, assets)
}

#[test]
fn pack_twice_is_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let (bundle, assets) = sample_bundle(&temp.path().join("pack.zga"));
    let mut first = Vec::new();
    bundle.pack(&mut first).unwrap();
    // The same bundle, packed again later: mtimes on disk must not leak into
    // the archive. Set them to now, well past the filesystem's clock tick.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3);
    fs::File::open(temp.path().join("pack.zga").join(GRAPH_FILE))
        .unwrap()
        .set_modified(later)
        .unwrap();
    for (_, hash) in &assets {
        fs::File::open(bundle.asset_path(hash).unwrap())
            .unwrap()
            .set_modified(later)
            .unwrap();
    }
    let mut second = Vec::new();
    bundle.pack(&mut second).unwrap();
    assert_eq!(first, second, "packing the same bundle must give the same bytes");
    assert_eq!(first[..2], [0x1f, 0x8b], "a .zgz is gzip");
}

#[test]
fn tampered_asset_fails_verify() {
    let temp = tempfile::tempdir().unwrap();
    let (bundle, assets) = sample_bundle(&temp.path().join("tampered.zga"));
    bundle.verify().unwrap();
    let path = bundle.asset_path(&assets[0].1).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes.push(b'X');
    fs::write(&path, bytes).unwrap();
    let error = bundle.verify().unwrap_err().to_string();
    assert!(error.contains("corrupt asset"), "{error}");
    assert!(error.contains(&assets[0].1), "{error}");
}

#[test]
fn dangling_hash_fails_verify() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("dangling.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (_, hash) = photo("missing.png");
    write_graph(&dir, &[("missing.png", &hash)]);
    let error = bundle.verify().unwrap_err().to_string();
    assert!(error.contains("dangling asset reference"), "{error}");
    assert!(error.contains(&hash), "{error}");
}

#[test]
fn stray_file_outside_the_layout_fails_verify() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("stray.zga");
    let (bundle, _) = sample_bundle(&dir);
    bundle.verify().unwrap();
    fs::write(dir.join("notes.txt"), "not part of a bundle").unwrap();
    let error = bundle.verify().unwrap_err().to_string();
    assert!(error.contains("outside the .zga layout"), "{error}");
    fs::remove_file(dir.join("notes.txt")).unwrap();
    fs::create_dir(dir.join("assets/nested")).unwrap();
    let error = bundle.verify().unwrap_err().to_string();
    assert!(error.contains("assets are stored flat"), "{error}");
}

/// A valid packed bundle plus one hostile entry, as .zgz bytes.
fn hostile_archive(dir: &Path, add: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let bundle = Bundle::create(dir).unwrap();
    let mut good = Vec::new();
    bundle.pack(&mut good).unwrap();
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(&good[..]).read_to_end(&mut plain).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    let mut archive = tar::Archive::new(&plain[..]);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let mut header = entry.header().clone();
        let path = entry.path().unwrap().into_owned();
        if header.entry_type() == tar::EntryType::Directory {
            tar.append_data(&mut header, &path, std::io::empty()).unwrap();
        } else {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            tar.append_data(&mut header, &path, &bytes[..]).unwrap();
        }
    }
    add(&mut tar);
    let plain = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&plain).unwrap();
    gz.finish().unwrap()
}

#[test]
fn traversal_absolute_symlink_and_device_entries_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Box<dyn FnOnce(&mut tar::Builder<Vec<u8>>)>)> = vec![
        ("../escape.txt", Box::new(file_entry("../escape.txt"))),
        ("/absolute.txt", Box::new(file_entry("/absolute.txt"))),
        ("assets/../../escape.txt", Box::new(file_entry("assets/../../escape.txt"))),
        (
            "assets/link",
            Box::new(|tar| {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_link_name("/etc/passwd").unwrap();
                tar.append_data(&mut header, "assets/link", std::io::empty()).unwrap();
            }),
        ),
        (
            "assets/null",
            Box::new(|tar| {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Char);
                header.set_size(0);
                header.set_device_major(1).unwrap();
                header.set_device_minor(3).unwrap();
                tar.append_data(&mut header, "assets/null", std::io::empty()).unwrap();
            }),
        ),
    ];
    for (i, (entry, add)) in cases.into_iter().enumerate() {
        let archive = hostile_archive(&temp.path().join(format!("good-{i}.zga")), add);
        let target = temp.path().join(format!("out-{i}.zga"));
        let error = Bundle::unpack(&archive[..], &target).unwrap_err().to_string();
        assert!(error.contains("unsafe .zgz entry refused"), "{entry}: {error}");
        // Nothing is left behind, and nothing escaped the target directory.
        assert!(!target.exists(), "{entry}");
        assert!(!temp.path().join("escape.txt").exists(), "{entry}");
    }
}

fn file_entry(name: &'static str) -> impl FnOnce(&mut tar::Builder<Vec<u8>>) {
    move |tar| {
        let bytes: &[u8] = b"pwned";
        // Raw header bytes: tar's own append_data refuses `..` and absolute
        // paths, which is exactly what this entry must smuggle in.
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(bytes.len() as u64);
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        tar.append(&header, bytes).unwrap();
    }
}

#[test]
fn round_trip_keeps_the_graph_and_assets() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("round.zga");
    let (bundle, assets) = sample_bundle(&dir);
    let mut zgz = Vec::new();
    bundle.pack(&mut zgz).unwrap();
    let restored_dir = temp.path().join("restored.zga");
    let restored = Bundle::unpack(&zgz[..], &restored_dir).unwrap();
    // The same graph bytes and the same asset bytes.
    assert_eq!(fs::read(dir.join(GRAPH_FILE)).unwrap(), fs::read(restored_dir.join(GRAPH_FILE)).unwrap());
    for (bytes, hash) in &assets {
        assert_eq!(&restored.read_asset(hash).unwrap(), bytes);
    }
    // And the unpacked graph answers queries.
    let db = Zega::in_memory().build().unwrap();
    db.import(fs::File::open(restored_dir.join(GRAPH_FILE)).unwrap()).unwrap();
    let result = db
        .run_lang(SCHEMA, r#"query { File(name: "a.png") { name hash } }"#)
        .unwrap();
    assert_eq!(result, serde_json::json!({"name":"a.png","hash":assets[0].1}));
}

#[test]
fn image_resolves_through_a_blake3_field() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("image.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (bytes, hash) = photo("photo.png");
    let source = temp.path().join("photo.png");
    fs::write(&source, &bytes).unwrap();
    assert_eq!(bundle.put_asset(&source).unwrap(), hash);
    write_graph(&dir, &[("photo.png", &hash)]);
    // @image names the blake3 field; the schema carries that into the file.
    let db = Zega::in_memory().build().unwrap();
    db.import(fs::File::open(dir.join(GRAPH_FILE)).unwrap()).unwrap();
    let display = db.schema(SCHEMA).unwrap();
    let image = display.display.views[0].nodes["File"].image.as_deref();
    assert_eq!(image, Some("hash"));
    // The field's value names this bundle's asset, byte for byte.
    let node = db.run_lang(SCHEMA, r#"query { File(name: "photo.png") { hash } }"#).unwrap();
    assert_eq!(node, serde_json::json!({"hash":hash}));
    assert_eq!(bundle.read_asset(&hash).unwrap(), bytes);
    bundle.verify().unwrap();
}

#[test]
fn remote_manifest_counts_as_a_reference() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("remote.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (_, hash) = photo("far.png");
    write_graph(&dir, &[("far.png", &hash)]);
    // Dangling while no local asset and no remote entry exist.
    assert!(bundle.verify().is_err());
    let manifest = zega::bundle::manifest_json(
        &std::collections::BTreeMap::from([(hash.clone(), "https://static.example.com/far.png".to_string())]),
    )
    .unwrap();
    fs::write(dir.join("assets/zega.json"), manifest).unwrap();
    let report = bundle.verify().unwrap();
    assert_eq!(report.remote, 1);
    assert_eq!(report.references, 1);
    // A credential-bearing or relative URL is refused.
    fs::write(
        dir.join("assets/zega.json"),
        format!(r#"{{"remote": {{"{hash}": "https://user:pass@example.com/far.png"}}}}"#),
    )
    .unwrap();
    assert!(bundle.verify().is_err());
}
