//! `.zga` bundles (APS 34, docs/files.md): a graph together with its files.
//!
//! A bundle is a directory:
//!
//! ```text
//! name.zga/
//!   graph.graph        the standard .graph file (docs/graph-format.md)
//!   assets/<blake3>    the bytes, content-addressed; the name is the hash
//!   assets/zega.json   optional: `{"remote": {"<blake3>": "<url>"}}`, where
//!                      assets not present locally live. Never credentials.
//! ```
//!
//! - [`Bundle::put_asset`] writes atomically (a temp file, then a rename) and
//!   deduplicates by hash.
//! - [`Bundle::resolve`] applies the APS 34 amendment's order: a File's local
//!   `String<file>` path first (only while its bytes still hash to `hash` — a
//!   mismatch means the file changed and is stale, never resolved), then
//!   `assets/<hash>`, then the remote in `assets/zega.json`.
//! - [`Bundle::verify`] checks the layout, every asset's hash, and that each
//!   `String<blake3>` value in the graph names a matching local path, a local
//!   asset or a remote entry; stale local paths are reported.
//! - [`Bundle::pack`] writes a `.zgz`: a gzipped tar, deterministic — entries
//!   sorted by name, mtime 0, uid and gid 0, modes 644/755 — so the same
//!   bundle always packs to the same bytes. A `.zgz` never ships a local
//!   path: `String<file>` values are stripped from the packed graph, and with
//!   `include_local` the bytes behind them are copied into `assets/` first.
//!   Serve it as a file, never with `Content-Encoding: gzip`.
//! - [`Bundle::unpack`] refuses path traversal, absolute paths, symlinks and
//!   device files, and runs [`Bundle::verify`] before it reports success.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::graph_file::{self, ExportOptions};
use crate::lang::{self, Field};
use crate::value::Value;

/// The graph inside a bundle.
pub const GRAPH_FILE: &str = "graph.graph";
/// The assets directory inside a bundle.
pub const ASSETS_DIR: &str = "assets";
/// The optional manifest of remote asset locations.
pub const MANIFEST_FILE: &str = "zega.json";

static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Why a bundle operation failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} already exists; a bundle is created in a new directory")]
    AlreadyExists(PathBuf),
    #[error("{0} is not a .zga bundle: it has no graph.graph and assets/ directory")]
    NotBundle(PathBuf),
    #[error("invalid .zga bundle: {0}")]
    Layout(String),
    #[error("corrupt asset: {name} hashes to {actual}; the file's bytes do not match its name")]
    HashMismatch { name: String, actual: String },
    #[error("dangling asset reference: the graph refers to {hash}, but it is not in assets/ and assets/zega.json has no remote entry for it")]
    Dangling { hash: String },
    #[error("local-only reference: the graph refers to {hash}, whose bytes only exist at a local path; a .zgz never ships local paths — pack with --include-local to copy the bytes into assets/ first")]
    LocalOnly { hash: String },
    #[error("invalid assets/zega.json: {0}")]
    Manifest(String),
    #[error("unsafe .zgz entry refused: {0}")]
    Unsafe(String),
    #[error(".graph error: {0}")]
    Graph(#[from] graph_file::Error),
    #[error("bundle i/o error: {0}")]
    Io(#[from] io::Error),
}

/// What a successful [`Bundle::verify`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Assets present locally under `assets/`.
    pub assets: u64,
    /// Their total size in bytes.
    pub bytes: u64,
    /// Remote entries in `assets/zega.json`.
    pub remote: u64,
    /// `String<blake3>` references found in the graph.
    pub references: u64,
    /// References satisfied by a local `String<file>` path whose bytes still
    /// hash to the reference.
    pub local: u64,
    /// Local paths whose bytes no longer hash to the reference they sit with:
    /// the file changed on disk. Re-index; the bytes are never served.
    pub stale: Vec<String>,
}

/// Where a file's bytes resolve to (APS 34 amendment): the local path, then
/// the bundle's asset, then the remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A readable local file whose bytes hash to the value.
    Local(PathBuf),
    /// The bundle's asset, `assets/<hash>`.
    Asset(PathBuf),
    /// The remote URL in `assets/zega.json`.
    Remote(String),
    /// The local file's bytes no longer hash to the value: it changed on
    /// disk and is never resolved. Re-index it.
    Stale(PathBuf),
    /// Not present anywhere.
    Missing,
}

/// An open `.zga` bundle directory.
#[derive(Debug)]
pub struct Bundle {
    root: PathBuf,
}

impl Bundle {
    /// Create an empty bundle at `path`: the directory, `assets/` and an
    /// empty `graph.graph`. `path` must not exist yet.
    pub fn create(path: &Path) -> Result<Bundle, Error> {
        if path.exists() {
            return Err(Error::AlreadyExists(path.to_path_buf()));
        }
        fs::create_dir_all(path.join(ASSETS_DIR))?;
        let bundle = Bundle { root: path.to_path_buf() };
        // An empty graph, exported with the standard writer, so the file is a
        // valid .graph from the start.
        let db = crate::Zega::in_memory()
            .build()
            .map_err(|error| Error::Layout(format!("cannot write an empty graph: {error}")))?;
        let mut file = fs::File::create(path.join(GRAPH_FILE))?;
        db.export(&mut file)
            .map_err(|error| Error::Layout(format!("cannot write an empty graph: {error}")))?;
        file.sync_all()?;
        Ok(bundle)
    }

    /// Open an existing bundle directory.
    pub fn open(path: &Path) -> Result<Bundle, Error> {
        let bundle = Bundle { root: path.to_path_buf() };
        if !path.join(GRAPH_FILE).is_file() || !path.join(ASSETS_DIR).is_dir() {
            return Err(Error::NotBundle(path.to_path_buf()));
        }
        Ok(bundle)
    }

    /// The bundle's directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The local path of the asset with this blake3 hash, if it is present.
    pub fn asset_path(&self, hash: &str) -> Option<PathBuf> {
        let path = self.root.join(ASSETS_DIR).join(hash);
        (lang::valid_blake3(hash) && path.is_file()).then_some(path)
    }

    /// Read the asset with this blake3 hash. The bytes are re-hashed before
    /// they are returned, so a corrupt file never reaches the caller.
    pub fn read_asset(&self, hash: &str) -> Result<Vec<u8>, Error> {
        let Some(path) = self.asset_path(hash) else {
            return Err(Error::Dangling { hash: hash.to_string() });
        };
        let mut file = fs::File::open(path)?;
        let (actual, bytes) = hash_reader(&mut file)?;
        if actual != hash {
            return Err(Error::HashMismatch { name: hash.to_string(), actual });
        }
        Ok(bytes)
    }

    /// Add `source`'s bytes to `assets/` under their blake3 hash; returns the
    /// hash. Atomic: the bytes land in a temp file first and are renamed into
    /// place, and an asset already present is not rewritten.
    pub fn put_asset(&self, source: &Path) -> Result<String, Error> {
        let mut input = fs::File::open(source)?;
        self.put_reader(&mut input)
    }

    /// Like [`Bundle::put_asset`], from a reader.
    pub fn put_reader(&self, input: &mut impl Read) -> Result<String, Error> {
        let staging = self.root.join(ASSETS_DIR).join(format!(
            ".upload-{}-{}",
            std::process::id(),
            UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let result = self.stage_asset(input, &staging);
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    }

    /// Resolve a File's bytes by its blake3 `hash`, in the APS 34 amendment's
    /// order: the local `String<file>` `path` first — but only while its bytes
    /// still hash to `hash` (a mismatch means the file changed: it is stale,
    /// re-index it, and never resolve to it) — then `assets/<hash>`, then the
    /// remote in `assets/zega.json`. Engines that cannot read the local disk
    /// pass no `path` and fall through to the asset or the remote.
    pub fn resolve(&self, hash: &str, path: Option<&str>) -> Result<Resolution, Error> {
        if !lang::valid_blake3(hash) {
            return Err(Error::Layout(format!("{hash:?} is not a blake3 hash: {}", lang::BLAKE3_HELP)));
        }
        if let Some(value) = path {
            let local = file_url_path(value)?;
            if let Ok(bytes) = fs::read(&local) {
                if blake3::hash(&bytes).to_hex().as_str() == hash {
                    return Ok(Resolution::Local(local));
                }
                // The file changed on disk. An asset or remote still serves
                // the right bytes by hash; the changed file's do not.
                return Ok(self.off_local(hash)?.unwrap_or(Resolution::Stale(local)));
            }
        }
        Ok(self.off_local(hash)?.unwrap_or(Resolution::Missing))
    }

    /// Resolution past the local path: the asset, then the remote.
    fn off_local(&self, hash: &str) -> Result<Option<Resolution>, Error> {
        if let Some(asset) = self.asset_path(hash) {
            return Ok(Some(Resolution::Asset(asset)));
        }
        if let Some(url) = self.remote_manifest()?.get(hash) {
            return Ok(Some(Resolution::Remote(url.clone())));
        }
        Ok(None)
    }

    fn stage_asset(&self, input: &mut impl Read, staging: &Path) -> Result<String, Error> {
        let mut hasher = blake3::Hasher::new();
        let mut out = fs::File::create(staging)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = input.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            out.write_all(&buffer[..read])?;
        }
        out.sync_all()?;
        drop(out);
        let hash = hasher.finalize().to_hex().to_string();
        let target = self.root.join(ASSETS_DIR).join(&hash);
        // Deduplicated: the same bytes are already stored under this name.
        if target.exists() {
            fs::remove_file(staging)?;
        } else {
            fs::rename(staging, &target)?;
        }
        Ok(hash)
    }

    /// Check the bundle: the layout, every asset's hash, and every
    /// `String<blake3>` reference in the graph against local paths, local
    /// assets and the remote manifest.
    pub fn verify(&self) -> Result<VerifyReport, Error> {
        self.verify_scanned(&self.scan()?)
    }

    fn verify_scanned(&self, scanned: &Scanned) -> Result<VerifyReport, Error> {
        let remote = self.remote_manifest()?;
        self.check_layout()?;
        let mut report = VerifyReport { remote: remote.len() as u64, ..Default::default() };
        for entry in fs::read_dir(self.root.join(ASSETS_DIR))? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == MANIFEST_FILE {
                continue;
            }
            if !lang::valid_blake3(&name) {
                return Err(Error::Layout(format!(
                    "{ASSETS_DIR}/{name} is not an asset: asset names are 64 lowercase hex characters"
                )));
            }
            let (actual, bytes) = hash_reader(&mut fs::File::open(entry.path())?)?;
            if actual != name {
                return Err(Error::HashMismatch { name, actual });
            }
            report.assets += 1;
            report.bytes += bytes.len() as u64;
        }
        self.check_references(scanned, &remote, &mut report)?;
        Ok(report)
    }

    /// Nothing outside `graph.graph` and `assets/`, and only files inside.
    fn check_layout(&self) -> Result<(), Error> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let ok = match name.as_str() {
                GRAPH_FILE => entry.file_type()?.is_file(),
                ASSETS_DIR => entry.file_type()?.is_dir(),
                _ => false,
            };
            if !ok {
                return Err(Error::Layout(format!(
                    "{name} is outside the .zga layout: a bundle holds graph.graph and assets/ only"
                )));
            }
        }
        for entry in fs::read_dir(self.root.join(ASSETS_DIR))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(Error::Layout(format!(
                    "{ASSETS_DIR}/{} is not a file: assets are stored flat",
                    entry.file_name().to_string_lossy()
                )));
            }
        }
        Ok(())
    }

    /// The `remote` map of `assets/zega.json`: blake3 hash to URL, both
    /// checked. Absent file, absent key or `null` all mean no remotes.
    fn remote_manifest(&self) -> Result<HashMap<String, String>, Error> {
        let path = self.root.join(ASSETS_DIR).join(MANIFEST_FILE);
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let text = fs::read_to_string(&path)
            .map_err(|error| Error::Manifest(format!("cannot read {MANIFEST_FILE}: {error}")))?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|error| Error::Manifest(format!("not JSON: {error}")))?;
        let empty = serde_json::Map::new();
        let entries = json
            .get("remote")
            .and_then(|remote| if remote.is_null() { None } else { remote.as_object() })
            .ok_or_else(|| {
                Error::Manifest(r#"`remote` must be an object of `"<blake3>": "<url>"`, e.g. {"remote": {"af13…62": "https://…/photo.jpg"}}"#.to_string())
            })
            .unwrap_or(&empty);
        let mut remote = HashMap::new();
        for (hash, url) in entries {
            if !lang::valid_blake3(hash) {
                return Err(Error::Manifest(format!("remote key {hash:?} is not a blake3 hash")));
            }
            let Some(url) = url.as_str().filter(|url| lang::valid_url(url)) else {
                return Err(Error::Manifest(format!(
                    "remote URL for {hash} is not an absolute http(s) URL; {}",
                    lang::URL_HELP
                )));
            };
            remote.insert(hash.clone(), url.to_string());
        }
        Ok(remote)
    }

    /// The graph's file facts: every `String<blake3>` reference and every
    /// `String<file>` path, per node and relationship. With a schema in the
    /// graph file, exactly the declared fields are read; without one, every
    /// string shaped like a blake3 hash is a reference and every string
    /// shaped like a file URL is a path. Verify and pack share this scan, so
    /// both read the same graph the same way.
    fn scan(&self) -> Result<Scanned, Error> {
        let file = fs::File::open(self.root.join(GRAPH_FILE))?;
        let (graph, summary) = graph_file::read(BufReader::new(file))?;
        let mut subjects = Vec::new();
        if let Some(source) = &summary.schema {
            let schema = lang::parse_schema(source)
                .map_err(|error| Error::Layout(format!("the graph's schema does not parse: {error}")))?;
            let mut node_fields: HashMap<&str, (Vec<&str>, Vec<&str>)> = HashMap::new();
            let mut edge_fields: HashMap<&str, (Vec<&str>, Vec<&str>)> = HashMap::new();
            for type_def in &schema.types {
                for field in &type_def.fields {
                    match field {
                        Field::Prop { name, ty, .. } => {
                            let entry = node_fields.entry(type_def.name.as_str()).or_default();
                            match ty.as_str() {
                                "String<blake3>" => entry.0.push(name),
                                "String<file>" => entry.1.push(name),
                                _ => {}
                            }
                        }
                        Field::Edge { rel, props, .. } => {
                            for prop in props {
                                let entry = edge_fields.entry(rel.as_str()).or_default();
                                match prop.ty.as_str() {
                                    "String<blake3>" => entry.0.push(&prop.name),
                                    "String<file>" => entry.1.push(&prop.name),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            for node in graph.nodes() {
                let Some(label) = node.first_label() else { continue };
                let Some((hashes, files)) = node_fields.get(label) else { continue };
                subjects.push(Subject::node(node.id, declared(node.props(), hashes, files)));
            }
            for rel in graph.relationships() {
                let Some((hashes, files)) = edge_fields.get(rel.kind) else { continue };
                subjects.push(Subject::relationship(rel.id, declared(rel.props(), hashes, files)));
            }
        } else {
            for node in graph.nodes() {
                subjects.push(Subject::node(node.id, shape_guessed(node.props())));
            }
            for rel in graph.relationships() {
                subjects.push(Subject::relationship(rel.id, shape_guessed(rel.props())));
            }
        }
        subjects.retain(|subject| !subject.hashes.is_empty() || !subject.paths.is_empty());
        Ok(Scanned { graph, created_by: summary.created_by, subjects })
    }

    /// Every `String<blake3>` reference in the graph names a matching local
    /// path, a local asset or a remote entry. A local path whose bytes no
    /// longer hash to any of its node's hashes is stale: reported, never
    /// counted.
    fn check_references(
        &self,
        scanned: &Scanned,
        remote: &HashMap<String, String>,
        report: &mut VerifyReport,
    ) -> Result<(), Error> {
        for subject in &scanned.subjects {
            for (_, hash) in &subject.hashes {
                if !lang::valid_blake3(hash) {
                    return Err(Error::Layout(format!(
                        "a String<blake3> field holds {hash:?}, which is not a blake3 hash"
                    )));
                }
            }
            for (_, url) in &subject.paths {
                if !lang::valid_file(url) {
                    return Err(Error::Layout(format!(
                        "a String<file> field holds {url:?}, which is not a local file URL: {}",
                        lang::FILE_HELP
                    )));
                }
            }
            // A path counts when its bytes hash to one of the subject's
            // references; otherwise it changed on disk and is stale.
            let mut local = vec![false; subject.hashes.len()];
            if !subject.hashes.is_empty() {
                for (_, url) in &subject.paths {
                    let path = file_url_path(url)?;
                    let Ok(bytes) = fs::read(&path) else { continue };
                    let actual = blake3::hash(&bytes).to_hex().to_string();
                    match subject.hashes.iter().position(|(_, hash)| hash == &actual) {
                        Some(at) => local[at] = true,
                        None if !report.stale.contains(url) => report.stale.push(url.clone()),
                        None => {}
                    }
                }
            }
            for (at, (_, hash)) in subject.hashes.iter().enumerate() {
                report.references += 1;
                if local[at] {
                    report.local += 1;
                } else if self.asset_path(hash).is_none() && !remote.contains_key(hash) {
                    return Err(Error::Dangling { hash: hash.clone() });
                }
            }
        }
        Ok(())
    }

    /// Pack the bundle to `out` as a deterministic `.zgz`: a verified bundle,
    /// entries sorted by name, mtime 0, uid and gid 0, modes 644 for files
    /// and 755 for directories, gzip with mtime 0. The same bundle always
    /// packs to the same bytes.
    ///
    /// A `.zgz` never ships local paths (APS 34 amendment): `String<file>`
    /// values are stripped from the packed graph — they reveal usernames and
    /// folder layout — so a reference whose bytes exist only at a local path
    /// is refused; [`Bundle::pack_with`] with `include_local` copies those
    /// bytes into `assets/` first.
    pub fn pack(&self, out: impl Write) -> Result<(), Error> {
        self.pack_with(out, false)
    }

    /// [`Bundle::pack`] with `include_local`: copy the bytes behind each
    /// local `String<file>` path into `assets/` before packing, so the `.zgz`
    /// carries them. Either way the packed graph holds no `file://` value.
    pub fn pack_with(&self, out: impl Write, include_local: bool) -> Result<(), Error> {
        let scanned = self.scan()?;
        if include_local {
            self.copy_local_assets(&scanned)?;
        }
        self.verify_scanned(&scanned)?;
        // Verify passed, so every reference is present somewhere — but a
        // local path does not ship. Refuse a reference only a path satisfies.
        let remote = self.remote_manifest()?;
        for subject in &scanned.subjects {
            for (_, hash) in &subject.hashes {
                if self.asset_path(hash).is_none() && !remote.contains_key(hash) {
                    return Err(Error::LocalOnly { hash: hash.clone() });
                }
            }
        }
        let graph = stripped_graph(scanned)?;
        // A fixed gzip header: no mtime, no original filename.
        let gz = flate2::GzBuilder::new()
            .mtime(0)
            .write(out, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for entry in self.entries()? {
            let Entry { name, dir } = entry;
            let mut header = tar::Header::new_gnu();
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            if dir {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                tar.append_data(&mut header, format!("{name}/"), io::empty())?;
            } else {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                // The packed graph.graph is the stripped copy when the graph
                // held local paths; every other file ships its disk bytes.
                let stripped = graph.as_ref().filter(|_| name == GRAPH_FILE);
                header.set_size(stripped.map_or(self.root.join(&name).metadata()?.len(), |bytes| bytes.len() as u64));
                match stripped {
                    Some(bytes) => tar.append_data(&mut header, &name, bytes.as_slice())?,
                    None => tar.append_data(&mut header, &name, fs::File::open(self.root.join(&name))?)?,
                }
            }
        }
        let gz = tar.into_inner()?;
        gz.finish()?;
        Ok(())
    }

    /// Copy the bytes behind each local `String<file>` path into `assets/`
    /// for references that have no asset yet. A path whose bytes no longer
    /// hash to its reference is stale: nothing is copied for it, and verify
    /// fails the pack if nothing else serves the reference.
    fn copy_local_assets(&self, scanned: &Scanned) -> Result<(), Error> {
        for subject in &scanned.subjects {
            for (_, hash) in &subject.hashes {
                if self.asset_path(hash).is_some() {
                    continue;
                }
                for (_, url) in &subject.paths {
                    let Ok(bytes) = fs::read(file_url_path(url)?) else { continue };
                    if blake3::hash(&bytes).to_hex().as_str() == hash {
                        self.put_reader(&mut bytes.as_slice())?;
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// The archive's entries, sorted by name: one directory entry for
    /// `assets`, then every file. Sorted so packing is deterministic.
    fn entries(&self) -> Result<Vec<Entry>, Error> {
        let mut entries = vec![Entry { name: ASSETS_DIR.to_string(), dir: true }];
        entries.push(Entry { name: GRAPH_FILE.to_string(), dir: false });
        for entry in fs::read_dir(self.root.join(ASSETS_DIR))? {
            entries.push(Entry {
                name: format!("{ASSETS_DIR}/{}", entry?.file_name().to_string_lossy()),
                dir: false,
            });
        }
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        Ok(entries)
    }

    /// Unpack a `.zgz` into `dir`, which must not exist, and verify the
    /// bundle before reporting success. Entries that traverse (`..`), are
    /// absolute, or are symlinks, links or device files are refused; on any
    /// failure the partial directory is removed.
    pub fn unpack(input: impl Read, dir: &Path) -> Result<Bundle, Error> {
        if dir.exists() {
            return Err(Error::AlreadyExists(dir.to_path_buf()));
        }
        fs::create_dir_all(dir)?;
        let result = unpack_entries(input, dir).and_then(|()| {
            let bundle = Bundle::open(dir)?;
            bundle.verify()?;
            Ok(bundle)
        });
        if result.is_err() {
            let _ = fs::remove_dir_all(dir);
        }
        result
    }
}

struct Entry {
    name: String,
    dir: bool,
}

/// A bundle's graph, read once, with its file facts collected. Shared by
/// verify and pack so both read the same graph the same way.
struct Scanned {
    graph: crate::graph::Graph,
    created_by: String,
    subjects: Vec<Subject>,
}

/// One node's or relationship's file facts: its `String<blake3>` references
/// and its `String<file>` paths, as (field, value).
struct Subject {
    id: u64,
    node: bool,
    hashes: Vec<(String, String)>,
    paths: Vec<(String, String)>,
}

impl Subject {
    fn node(id: u64, facts: Facts) -> Subject {
        Subject { id, node: true, hashes: facts.0, paths: facts.1 }
    }

    fn relationship(id: u64, facts: Facts) -> Subject {
        Subject { id, node: false, hashes: facts.0, paths: facts.1 }
    }
}

/// A subject's `String<blake3>` and `String<file>` values.
type Facts = (Vec<(String, String)>, Vec<(String, String)>);

/// The values of the declared `String<blake3>` and `String<file>` fields
/// among `props`.
fn declared<'a>(
    props: impl Iterator<Item = (&'a str, &'a Value)>,
    hashes: &[&str],
    files: &[&str],
) -> Facts {
    let mut facts = Facts::default();
    for (name, value) in props {
        let Value::String(text) = value else { continue };
        if hashes.contains(&name) {
            facts.0.push((name.to_string(), text.to_string()));
        } else if files.contains(&name) {
            facts.1.push((name.to_string(), text.to_string()));
        }
    }
    facts
}

/// Without a schema, the shape of a string decides: a blake3 hash is a
/// reference, a file URL is a path.
fn shape_guessed<'a>(props: impl Iterator<Item = (&'a str, &'a Value)>) -> Facts {
    let mut facts = Facts::default();
    for (name, value) in props {
        let Value::String(text) = value else { continue };
        if lang::valid_blake3(text) {
            facts.0.push((name.to_string(), text.to_string()));
        } else if lang::valid_file(text) {
            facts.1.push((name.to_string(), text.to_string()));
        }
    }
    facts
}

/// The graph with every `String<file>` prop removed, re-encoded as a `.graph`
/// file — or None when the graph holds no local paths and packs verbatim.
/// The bundle on disk keeps its paths; only the packed copy loses them. The
/// undo history and the linked change log mention the stripped paths too:
/// the packed copy drops the history and scrubs the log's field values.
fn stripped_graph(scanned: Scanned) -> Result<Option<Vec<u8>>, Error> {
    if scanned.subjects.iter().all(|subject| subject.paths.is_empty()) {
        return Ok(None);
    }
    let mut graph = scanned.graph;
    let mut stripped: HashSet<&str> = HashSet::new();
    for subject in &scanned.subjects {
        if subject.paths.is_empty() {
            continue;
        }
        let fields: HashSet<&str> = subject.paths.iter().map(|(field, _)| field.as_str()).collect();
        stripped.extend(fields.iter().copied());
        if subject.node {
            let Some(node) = graph.get_node(subject.id) else { continue };
            let labels: Vec<String> = node.labels().map(str::to_string).collect();
            let props = keep_without(node.props(), &fields);
            graph.restore_node(subject.id, labels, props);
        } else {
            let Some(rel) = graph.get_relationship(subject.id) else { continue };
            let props = keep_without(rel.props(), &fields);
            graph.restore_relationship(subject.id, rel.kind.to_string(), rel.from, rel.to, props);
        }
    }
    // The undo history mentions the stripped paths; the packed copy ships
    // none of it.
    graph.history = Default::default();
    // The linked change log carries past field values; scrub the stripped
    // names out of every recorded upsert.
    for record in &mut graph.linked.history {
        for change in &mut record.changes {
            if let crate::linked::Change::Upsert { fields, .. } = &mut change.change {
                fields.retain(|name, _| !stripped.contains(name.as_str()));
            }
        }
    }
    let mut bytes = Vec::new();
    graph_file::write(&graph, &ExportOptions::default(), &scanned.created_by, &mut bytes)?;
    Ok(Some(bytes))
}

fn keep_without<'a>(
    props: impl Iterator<Item = (&'a str, &'a Value)>,
    fields: &HashSet<&str>,
) -> HashMap<String, Value> {
    props
        .filter(|(name, _)| !fields.contains(name))
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect()
}

/// The local path a `String<file>` value names on this computer.
fn file_url_path(value: &str) -> Result<PathBuf, Error> {
    if !lang::valid_file(value) {
        return Err(Error::Layout(format!("{value:?} is not a String<file>: {}", lang::FILE_HELP)));
    }
    url::Url::parse(value)
        .map_err(|error| Error::Layout(format!("{value:?}: {error}")))?
        .to_file_path()
        .map_err(|()| Error::Layout(format!("{value:?} does not name a path on this computer")))
}

/// Hash a reader's bytes, keeping them for the caller.
fn hash_reader(input: &mut impl Read) -> Result<(String, Vec<u8>), Error> {
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    Ok((blake3::hash(&bytes).to_hex().to_string(), bytes))
}

fn unpack_entries(input: impl Read, dir: &Path) -> Result<(), Error> {
    let gz = flate2::read::GzDecoder::new(input);
    let mut archive = tar::Archive::new(gz);
    let mut seen = HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let safe = safe_path(&path).ok_or_else(|| Error::Unsafe(format!("{} is not a safe path", path.display())))?;
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                fs::create_dir_all(dir.join(&safe))?;
            }
            tar::EntryType::Regular => {
                let target = dir.join(&safe);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                // One name once: a second entry with the same path would
                // overwrite the first, and duplicate entries are never valid.
                if !seen.insert(safe.clone()) {
                    return Err(Error::Unsafe(format!("{} appears twice in the archive", safe.display())));
                }
                let mut out = fs::OpenOptions::new().write(true).create_new(true).open(target)?;
                io::copy(&mut entry, &mut out)?;
            }
            other => {
                return Err(Error::Unsafe(format!(
                    "{} is a {other:?}; a .zgz holds files and directories only",
                    safe.display()
                )));
            }
        }
    }
    Ok(())
}

/// A path is safe when it is relative and every component is a normal name:
/// no `..`, no root, no prefix, no empty middle components.
fn safe_path(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => out.push(name),
            // tar paths may carry a leading `./`; it changes nothing.
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// The `remote` map of a bundle's manifest, for `zega.json` writers. Kept
/// here so the CLI and intakes build the one shape verify accepts.
pub fn manifest_json(remote: &BTreeMap<String, String>) -> Result<String, Error> {
    for (hash, url) in remote {
        if !lang::valid_blake3(hash) {
            return Err(Error::Manifest(format!("remote key {hash:?} is not a blake3 hash")));
        }
        if !lang::valid_url(url) {
            return Err(Error::Manifest(format!("remote URL for {hash} is not an absolute http(s) URL")));
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({ "remote": remote }))
        .map_err(|error| Error::Manifest(format!("cannot write {MANIFEST_FILE}: {error}")))
}
