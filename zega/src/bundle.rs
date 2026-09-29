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
//! - [`Bundle::verify`] checks the layout, every asset's hash, and that each
//!   `String<blake3>` value in the graph names a local asset or a remote
//!   entry.
//! - [`Bundle::pack`] writes a `.zgz`: a gzipped tar, deterministic — entries
//!   sorted by name, mtime 0, uid and gid 0, modes 644/755 — so the same
//!   bundle always packs to the same bytes. Serve it as a file, never with
//!   `Content-Encoding: gzip`.
//! - [`Bundle::unpack`] refuses path traversal, absolute paths, symlinks and
//!   device files, and runs [`Bundle::verify`] before it reports success.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::graph_file;
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
    /// `String<blake3>` reference in the graph against local assets and the
    /// remote manifest.
    pub fn verify(&self) -> Result<VerifyReport, Error> {
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
        report.references = self.check_references(&remote)?;
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

    /// Every `String<blake3>` value in the graph names a local asset or a
    /// remote entry. With a schema in the graph file, exactly the declared
    /// fields are checked; without one, every string shaped like a blake3
    /// hash is treated as a reference.
    fn check_references(&self, remote: &HashMap<String, String>) -> Result<u64, Error> {
        let file = fs::File::open(self.root.join(GRAPH_FILE))?;
        let (graph, summary) = graph_file::read(BufReader::new(file))?;
        let mut checked = 0;
        let mut resolve = |hash: &str| -> Result<(), Error> {
            checked += 1;
            if !lang::valid_blake3(hash) {
                return Err(Error::Layout(format!(
                    "a String<blake3> field holds {hash:?}, which is not a blake3 hash"
                )));
            }
            if self.asset_path(hash).is_none() && !remote.contains_key(hash) {
                return Err(Error::Dangling { hash: hash.to_string() });
            }
            Ok(())
        };
        if let Some(source) = &summary.schema {
            let schema = lang::parse_schema(source)
                .map_err(|error| Error::Layout(format!("the graph's schema does not parse: {error}")))?;
            let mut node_fields: HashMap<&str, Vec<&str>> = HashMap::new();
            let mut edge_fields: HashMap<&str, Vec<&str>> = HashMap::new();
            for type_def in &schema.types {
                for field in &type_def.fields {
                    match field {
                        Field::Prop { name, ty, .. } if ty == "String<blake3>" => {
                            node_fields.entry(type_def.name.as_str()).or_default().push(name);
                        }
                        Field::Edge { rel, props, .. } => {
                            for prop in props {
                                if prop.ty == "String<blake3>" {
                                    edge_fields.entry(rel.as_str()).or_default().push(&prop.name);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            for node in graph.nodes() {
                let Some(label) = node.first_label() else { continue };
                let Some(fields) = node_fields.get(label) else { continue };
                for field in fields {
                    if let Some(Value::String(hash)) = node.prop(field) {
                        resolve(hash)?;
                    }
                }
            }
            for rel in graph.relationships() {
                let Some(fields) = edge_fields.get(rel.kind) else { continue };
                for field in fields {
                    if let Some(Value::String(hash)) = rel.prop(field) {
                        resolve(hash)?;
                    }
                }
            }
        } else {
            for node in graph.nodes() {
                for (_, value) in node.props() {
                    if let Value::String(hash) = value {
                        if lang::valid_blake3(hash) {
                            resolve(hash)?;
                        }
                    }
                }
            }
            for rel in graph.relationships() {
                for (_, value) in rel.props() {
                    if let Value::String(hash) = value {
                        if lang::valid_blake3(hash) {
                            resolve(hash)?;
                        }
                    }
                }
            }
        }
        Ok(checked)
    }

    /// Pack the bundle to `out` as a deterministic `.zgz`: a verified bundle,
    /// entries sorted by name, mtime 0, uid and gid 0, modes 644 for files
    /// and 755 for directories, gzip with mtime 0. The same bundle always
    /// packs to the same bytes.
    pub fn pack(&self, out: impl Write) -> Result<(), Error> {
        self.verify()?;
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
                let path = self.root.join(&name);
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                header.set_size(path.metadata()?.len());
                tar.append_data(&mut header, &name, fs::File::open(path)?)?;
            }
        }
        let gz = tar.into_inner()?;
        gz.finish()?;
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
