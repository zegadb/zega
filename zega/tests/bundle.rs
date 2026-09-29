//! `.zga` bundles and deterministic `.zgz` packing (APS 34, docs/files.md).
//! Every test here fails if its piece of the feature is removed.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zega::bundle::{Bundle, Resolution, UnpackLimits, GRAPH_FILE};
use zega::graph_file::ExportOptions;
use zega::Zega;

const SCHEMA: &str = "type File { name: String mediaType: String size: Int hash: String<blake3> path?: String<file> width?: Int height?: Int duration?: Float source?: String<url> licence: String author?: String credit?: String fetchedAt?: String } display { graph { File(@shape: document, @image: &hash) } }";

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

/// Write a graph with one File node that carries both a `hash` and the local
/// `path` its bytes live at (APS 34 amendment); returns the `file://` URL.
fn write_graph_with_path(bundle: &Path, name: &str, hash: &str, path: &Path) -> String {
    let url = url::Url::from_file_path(path).unwrap().to_string();
    let db = Zega::in_memory().build().unwrap();
    db.run_lang(SCHEMA, &format!(r#"mutation {{ File(name: "{name}" && mediaType: "image/png" && size: 3 && hash: "{hash}" && path: "{url}" && licence: "CC0") }}"#)).unwrap();
    let options = ExportOptions { schema: Some(SCHEMA.to_string()), ..Default::default() };
    let mut out = fs::File::create(bundle.join(GRAPH_FILE)).unwrap();
    db.export_with(&mut out, &options).unwrap();
    url
}

/// The .zgz's gunzipped bytes.
fn ungzip(zgz: &[u8]) -> Vec<u8> {
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(zgz).read_to_end(&mut plain).unwrap();
    plain
}

fn contains_file_url(bytes: &[u8]) -> bool {
    bytes.windows(b"file://".len()).any(|window| window == b"file://")
}

#[test]
fn pack_twice_is_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let (bundle, assets) = sample_bundle(&temp.path().join("pack.zga"));
    let mut first = Vec::new();
    bundle.pack(&mut first).unwrap();
    // The same bundle, packed again later: mtimes on disk must not leak into
    // the archive. Set them to now, well past the filesystem's clock tick.
    // Windows: `set_modified` (SetFileTime) needs a write handle — a read-only
    // `File::open` fails with ERROR_ACCESS_DENIED (CI run 36528181297), so
    // open for writing and drop the handle before packing again.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3);
    let set_modified = |path: &Path| {
        fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(later).unwrap();
    };
    set_modified(&temp.path().join("pack.zga").join(GRAPH_FILE));
    for (_, hash) in &assets {
        set_modified(&bundle.asset_path(hash).unwrap());
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

type Hostile = Box<dyn FnOnce(&mut tar::Builder<Vec<u8>>)>;

#[test]
fn traversal_absolute_symlink_and_device_entries_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let cases: Vec<(&str, Hostile)> = vec![
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

#[test]
fn zgz_never_contains_a_file_url() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("local.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (bytes, hash) = photo("scan.png");
    let source = temp.path().join("scan.png");
    fs::write(&source, &bytes).unwrap();
    assert_eq!(bundle.put_asset(&source).unwrap(), hash);
    let url = write_graph_with_path(&dir, "scan.png", &hash, &source);
    assert!(url.starts_with("file://"));
    // The bundle on disk keeps the local path; a hash counts as present when
    // its local path exists and matches.
    assert!(contains_file_url(&fs::read(dir.join(GRAPH_FILE)).unwrap()));
    let report = bundle.verify().unwrap();
    assert_eq!(report.references, 1);
    // The .zgz strips it: no file:// value anywhere in the archive.
    let mut zgz = Vec::new();
    bundle.pack(&mut zgz).unwrap();
    assert!(!contains_file_url(&ungzip(&zgz)), "a .zgz never ships a local path");
    // Unpacked, the node keeps its hash and has no path to query.
    let restored_dir = temp.path().join("unpacked.zga");
    let restored = Bundle::unpack(&zgz[..], &restored_dir).unwrap();
    assert_eq!(restored.read_asset(&hash).unwrap(), bytes);
    let db = Zega::in_memory().build().unwrap();
    db.import(fs::File::open(restored_dir.join(GRAPH_FILE)).unwrap()).unwrap();
    let node = db.run_lang(SCHEMA, r#"query { File(name: "scan.png") { hash path } }"#).unwrap();
    assert_eq!(node, serde_json::json!({"hash": hash, "path": null}));
}

#[test]
fn a_changed_local_file_is_stale_and_never_resolved() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("stale.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (bytes, hash) = photo("photo.png");
    let source = temp.path().join("photo.png");
    fs::write(&source, &bytes).unwrap();
    let url = write_graph_with_path(&dir, "photo.png", &hash, &source);
    // While the bytes match, the local path resolves and counts in verify.
    assert_eq!(bundle.resolve(&hash, Some(&url)).unwrap(), Resolution::Local(source.clone()));
    let report = bundle.verify().unwrap();
    assert_eq!(report.local, 1);
    assert!(report.stale.is_empty());
    // The file changes on disk: its bytes no longer hash to the reference.
    fs::write(&source, b"edited bytes").unwrap();
    // Never resolved to the changed bytes — without an asset it is stale…
    assert_eq!(bundle.resolve(&hash, Some(&url)).unwrap(), Resolution::Stale(source.clone()));
    let error = bundle.verify().unwrap_err().to_string();
    assert!(error.contains("dangling asset reference"), "{error}");
    // …and with the right bytes in assets/, the asset serves, not the file.
    bundle.put_reader(&mut &bytes[..]).unwrap();
    let asset = bundle.asset_path(&hash).unwrap();
    assert_eq!(bundle.resolve(&hash, Some(&url)).unwrap(), Resolution::Asset(asset));
    let report = bundle.verify().unwrap();
    assert_eq!(report.local, 0);
    assert_eq!(report.stale, vec![url]);
}

#[test]
fn include_local_copies_the_bytes_into_assets() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("inc.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let (bytes, hash) = photo("local.png");
    let source = temp.path().join("local.png");
    fs::write(&source, &bytes).unwrap();
    let url = write_graph_with_path(&dir, "local.png", &hash, &source);
    // No asset: the bytes exist only at the local path, which verify counts.
    let report = bundle.verify().unwrap();
    assert_eq!(report.local, 1);
    assert_eq!(report.references, 1);
    // A default pack refuses: the .zgz could not serve the reference.
    let error = bundle.pack(&mut Vec::new()).unwrap_err().to_string();
    assert!(error.contains("--include-local"), "{error}");
    assert!(bundle.asset_path(&hash).is_none());
    // --include-local copies the bytes into assets/ first; then it packs.
    let mut zgz = Vec::new();
    bundle.pack_with(&mut zgz, true).unwrap();
    assert_eq!(bundle.read_asset(&hash).unwrap(), bytes);
    bundle.verify().unwrap();
    assert_eq!(bundle.resolve(&hash, Some(&url)).unwrap(), Resolution::Local(source));
    assert!(!contains_file_url(&ungzip(&zgz)), "a .zgz never ships a local path");
}

/// A bundle with one `size`-byte zero-filled asset and a graph referencing
/// it, packed to a .zgz. Zeros compress to almost nothing, so the archive is
/// tiny next to what it expands to.
fn packed_zeros(temp: &Path, size: usize) -> (PathBuf, Vec<u8>, String) {
    let dir = temp.join("zeros.zga");
    let bundle = Bundle::create(&dir).unwrap();
    let bytes = vec![0u8; size];
    let hash = bundle.put_reader(&mut &bytes[..]).unwrap();
    write_graph(&dir, &[("zeros.bin", &hash)]);
    let mut zgz = Vec::new();
    bundle.pack(&mut zgz).unwrap();
    (dir, zgz, hash)
}

#[test]
fn unpack_refuses_a_size_bomb_and_leaves_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let (_, zgz, _) = packed_zeros(temp.path(), 256 * 1024);
    assert!(zgz.len() < 8 * 1024, "the bomb is tiny: {} bytes", zgz.len());
    let limits = |total: u64, entry: u64, entries: u64| UnpackLimits {
        max_total_bytes: total,
        max_entry_bytes: entry,
        max_entries: entries,
    };
    for (i, (caps, message)) in [
        // Total decompressed bytes, then one entry's bytes, then the count.
        (limits(64 * 1024, u64::MAX, u64::MAX), "past the total limit"),
        (limits(u64::MAX, 64 * 1024, u64::MAX), "past the per-entry limit"),
        (limits(u64::MAX, u64::MAX, 1), "more than 1 entries"),
    ]
    .iter()
    .enumerate()
    {
        let target = temp.path().join(format!("bomb-{i}.zga"));
        let error = Bundle::unpack_with(&zgz[..], &target, caps).unwrap_err().to_string();
        assert!(error.contains(".zgz expands past the unpack limit"), "{error}");
        assert!(error.contains(message), "{error}");
        // Refused, and the partial output is gone.
        assert!(!target.exists(), "{error}");
    }
    // The caps are the only refusal: raised explicitly, the same bytes unpack.
    let target = temp.path().join("raised.zga");
    let restored = Bundle::unpack_with(&zgz[..], &target, &limits(512 * 1024, 512 * 1024, 100)).unwrap();
    restored.verify().unwrap();
}

#[test]
fn unpack_strips_a_tar_czf_wrapper_directory() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("photos.zga");
    let (bundle, assets) = sample_bundle(&dir);
    let mut zgz = Vec::new();
    bundle.pack(&mut zgz).unwrap();
    // Repacked the way the APS 34 wire command `tar -czf X.zgz X.zga` does
    // it: one wrapper directory entry, then every entry beneath it.
    let wrapped = tar_wrapped(&zgz, "photos.zga");
    let restored_dir = temp.path().join("restored.zga");
    let restored = Bundle::unpack(&wrapped[..], &restored_dir).unwrap();
    // The wrapper is gone: the bundle sits at the top, byte for byte.
    assert!(!restored_dir.join("photos.zga").exists());
    assert_eq!(fs::read(dir.join(GRAPH_FILE)).unwrap(), fs::read(restored_dir.join(GRAPH_FILE)).unwrap());
    for (bytes, hash) in &assets {
        assert_eq!(&restored.read_asset(hash).unwrap(), bytes);
    }
    restored.verify().unwrap();
}

#[test]
fn unpack_of_a_foreign_archive_names_what_was_found() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("good.zga");
    let (bundle, _) = sample_bundle(&dir);
    let mut zgz = Vec::new();
    bundle.pack(&mut zgz).unwrap();
    // A wrapper that does not end in .zga, and no wrapper at all: today's
    // error, but it names the entries it found.
    for (i, (archive, found)) in [
        (tar_wrapped(&zgz, "photos"), "photos/"),
        (one_file_archive("readme.txt"), "readme.txt"),
    ]
    .iter()
    .enumerate()
    {
        let target = temp.path().join(format!("foreign-{i}.zga"));
        let error = Bundle::unpack(&archive[..], &target).unwrap_err().to_string();
        assert!(error.contains("is not a .zga bundle"), "{error}");
        assert!(error.contains(found), "{error}");
        assert!(!target.exists(), "{error}");
    }
}

/// `archive` rewritten as `tar -czf X.zgz X.zga` would write it: a `dir/`
/// entry, then every original entry under `dir/`. macOS tar also adds
/// AppleDouble `._*` metadata companions, so one of each is included.
fn tar_wrapped(zgz: &[u8], dir: &str) -> Vec<u8> {
    let mut plain = Vec::new();
    flate2::read::GzDecoder::new(zgz).read_to_end(&mut plain).unwrap();
    let mut tar = tar::Builder::new(Vec::new());
    let mut wrapper = tar::Header::new_gnu();
    wrapper.set_entry_type(tar::EntryType::Directory);
    wrapper.set_mode(0o755);
    wrapper.set_size(0);
    wrapper.set_cksum();
    tar.append_data(&mut wrapper, format!("{dir}/"), std::io::empty()).unwrap();
    apple_double(&mut tar, format!("._{dir}"));
    let mut archive = tar::Archive::new(&plain[..]);
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let mut header = entry.header().clone();
        let name = entry.path().unwrap().display().to_string().trim_end_matches('/').to_string();
        if header.entry_type() == tar::EntryType::Directory {
            tar.append_data(&mut header, format!("{dir}/{name}/"), std::io::empty()).unwrap();
        } else {
            let companion = match name.rsplit_once('/') {
                Some((parent, base)) => format!("{dir}/{parent}/._{base}"),
                None => format!("{dir}/._{name}"),
            };
            apple_double(&mut tar, companion);
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            tar.append_data(&mut header, format!("{dir}/{name}"), &bytes[..]).unwrap();
        }
    }
    gzip_tar(tar)
}

/// A macOS AppleDouble metadata entry, as `tar -czf` writes it.
fn apple_double(tar: &mut tar::Builder<Vec<u8>>, name: String) {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(3);
    header.set_cksum();
    tar.append_data(&mut header, name, &b"._x"[..]).unwrap();
}

/// A .zgz holding one small file and nothing a bundle has.
fn one_file_archive(name: &str) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(5);
    header.set_cksum();
    tar.append_data(&mut header, name, &b"hello"[..]).unwrap();
    gzip_tar(tar)
}

fn gzip_tar(tar: tar::Builder<Vec<u8>>) -> Vec<u8> {
    let plain = tar.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&plain).unwrap();
    gz.finish().unwrap()
}
