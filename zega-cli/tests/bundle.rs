//! `zega bundle` subcommands (APS 34, docs/files.md), end to end through the
//! binary the user runs.
use std::path::Path;
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_zega");
const SCHEMA: &str = "type File { name: String mediaType: String size: Int hash: String<blake3> path?: String<file> width?: Int height?: Int duration?: Float source?: String<url> licence: String author?: String credit?: String fetchedAt?: String } display { graph { File(@shape: document, @image: &hash) } }";

fn zega(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn ok(dir: &Path, args: &[&str]) -> Output {
    let output = zega(dir, args);
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
    output
}

#[test]
fn bundle_new_add_verify_pack_unpack_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let bundle = dir.join("photos.zga");
    let bundle_arg = bundle.to_str().unwrap();

    ok(dir, &["bundle", "new", bundle_arg]);
    assert!(bundle.join("graph.graph").is_file());
    assert!(bundle.join("assets").is_dir());
    // A second `new` on the same path refuses.
    let again = zega(dir, &["bundle", "new", bundle_arg]);
    assert!(!again.status.success());

    // Write a photo and a graph that references its hash.
    std::fs::write(dir.join("photo.png"), b"fake png bytes").unwrap();
    let add = ok(dir, &["bundle", "add", bundle_arg, "photo.png"]);
    let hash = String::from_utf8(add.stdout).unwrap().trim().to_string();
    assert_eq!(hash.len(), 64, "{hash}");
    assert!(bundle.join("assets").join(&hash).is_file());

    // `zega export` the graph into the bundle, with the File schema.
    let data = dir.join("db");
    let schema_file = dir.join("files.zql");
    std::fs::write(&schema_file, format!("schema {{ {SCHEMA} }}")).unwrap();
    let db = zega::Zega::open(data.to_str().unwrap()).build().unwrap();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "photo.png" && mediaType: "image/png" && size: 14 && hash: "{hash}" && licence: "CC0") }}"#)).unwrap();
    drop(db);
    ok(dir, &[
        "export",
        bundle.join("graph.graph").to_str().unwrap(),
        "--data",
        data.to_str().unwrap(),
        "--schema",
        schema_file.to_str().unwrap(),
    ]);

    ok(dir, &["bundle", "verify", bundle_arg]);
    let pack = ok(dir, &["bundle", "pack", bundle_arg]);
    assert!(pack.status.success());
    let zgz = dir.join("photos.zga.zgz");
    assert!(zgz.is_file(), "default .zgz name is <dir>.zgz");

    let restored = dir.join("restored.zga");
    ok(dir, &["bundle", "unpack", zgz.to_str().unwrap(), restored.to_str().unwrap()]);
    assert_eq!(
        std::fs::read(bundle.join("graph.graph")).unwrap(),
        std::fs::read(restored.join("graph.graph")).unwrap()
    );
    assert_eq!(
        std::fs::read(bundle.join("assets").join(&hash)).unwrap(),
        std::fs::read(restored.join("assets").join(&hash)).unwrap()
    );
    // Unpack refuses to clobber an existing directory.
    let clobber = zega(dir, &["bundle", "unpack", zgz.to_str().unwrap(), restored.to_str().unwrap()]);
    assert!(!clobber.status.success());
}

#[test]
fn bundle_verify_reports_a_tampered_asset() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let bundle = dir.join("t.zga");
    let bundle_arg = bundle.to_str().unwrap();
    ok(dir, &["bundle", "new", bundle_arg]);
    std::fs::write(dir.join("a.bin"), b"some bytes").unwrap();
    let add = ok(dir, &["bundle", "add", bundle_arg, "a.bin"]);
    let hash = String::from_utf8(add.stdout).unwrap().trim().to_string();
    ok(dir, &["bundle", "verify", bundle_arg]);
    std::fs::write(bundle.join("assets").join(&hash), b"other bytes").unwrap();
    let output = zega(dir, &["bundle", "verify", bundle_arg]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("corrupt asset"), "{stderr}");
}

#[test]
fn bundle_pack_is_deterministic_from_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let bundle = dir.join("d.zga");
    let bundle_arg = bundle.to_str().unwrap();
    ok(dir, &["bundle", "new", bundle_arg]);
    std::fs::write(dir.join("a.bin"), b"deterministic").unwrap();
    ok(dir, &["bundle", "add", bundle_arg, "a.bin"]);
    ok(dir, &["bundle", "pack", bundle_arg, "one.zgz"]);
    ok(dir, &["bundle", "pack", bundle_arg, "two.zgz"]);
    assert_eq!(std::fs::read(dir.join("one.zgz")).unwrap(), std::fs::read(dir.join("two.zgz")).unwrap());
}

#[test]
fn pack_include_local_copies_the_bytes_into_assets() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let bundle = dir.join("desk.zga");
    let bundle_arg = bundle.to_str().unwrap();
    ok(dir, &["bundle", "new", bundle_arg]);

    // A photo that exists only on this computer: hash it, then take the asset
    // back out — the graph will name its hash and its local path, and nothing
    // sits in assets/.
    let photo = dir.join("desktop-photo.png");
    std::fs::write(&photo, b"desktop photo bytes").unwrap();
    let add = ok(dir, &["bundle", "add", bundle_arg, photo.to_str().unwrap()]);
    let hash = String::from_utf8(add.stdout).unwrap().trim().to_string();
    std::fs::remove_file(bundle.join("assets").join(&hash)).unwrap();

    let url = format!("file://{}", photo.display());
    let data = dir.join("db");
    let schema_file = dir.join("files.zql");
    std::fs::write(&schema_file, format!("schema {{ {SCHEMA} }}")).unwrap();
    let db = zega::Zega::open(data.to_str().unwrap()).build().unwrap();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "desktop-photo.png" && mediaType: "image/png" && size: 19 && hash: "{hash}" && path: "{url}" && licence: "CC0") }}"#)).unwrap();
    drop(db);
    ok(dir, &[
        "export",
        bundle.join("graph.graph").to_str().unwrap(),
        "--data",
        data.to_str().unwrap(),
        "--schema",
        schema_file.to_str().unwrap(),
    ]);

    // Verify counts the local path; a default pack refuses to ship it.
    let verify = ok(dir, &["bundle", "verify", bundle_arg]);
    assert!(String::from_utf8_lossy(&verify.stderr).contains("1 at local paths"), "{}", String::from_utf8_lossy(&verify.stderr));
    let refused = zega(dir, &["bundle", "pack", bundle_arg]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--include-local"), "{}", String::from_utf8_lossy(&refused.stderr));
    assert!(!bundle.join("assets").join(&hash).exists());

    // --include-local copies the bytes into assets/ first; then it packs.
    ok(dir, &["bundle", "pack", bundle_arg, "--include-local"]);
    assert_eq!(std::fs::read(bundle.join("assets").join(&hash)).unwrap(), b"desktop photo bytes");
    ok(dir, &["bundle", "verify", bundle_arg]);

    // The packed graph carries no file:// value.
    let restored = dir.join("restored.zga");
    ok(dir, &["bundle", "unpack", dir.join("desk.zga.zgz").to_str().unwrap(), restored.to_str().unwrap()]);
    let graph = std::fs::read(restored.join("graph.graph")).unwrap();
    assert!(!graph.windows(b"file://".len()).any(|w| w == b"file://"), "a .zgz never ships a local path");
    assert_eq!(std::fs::read(restored.join("assets").join(&hash)).unwrap(), b"desktop photo bytes");
}
