// zega-bulk: loading a multi-million-node graph (zegadb/zega#127, #128).
//
//   zega-bulk gen <nodes> <relationships> <dir>
//   zega-bulk zql <dir> --data <data-dir> [--storage file|sqlite]
//   zega-bulk reopen --data <data-dir>
//
// `gen` writes one synthetic graph two ways, deterministic per size:
// `graph.jsonl` for the bulk path (`zega-server import graph.jsonl`,
// docs/graph-format.md#json-lines) and, for the path that existed before
// it, ZQL `mutation json` loads of at most 2 MB each (#127's cap) named by
// `load.zql`. `schema.zql` is the schema both use.
//
// `zql` loads the 2 MB chunks one statement at a time into a disk database,
// the way a user splitting files by hand does, and `reopen` opens a data
// directory and runs one query with the schema (which declares the unique
// index), so its RSS is the graph's resting size. Each prints one JSON line
// with its wall time, RSS now and peak RSS (getrusage). Peak RSS of
// `zega-server import` itself is taken with `/usr/bin/time -l` by the
// caller (scripts/bulk-bench.sh, which runs the whole comparison). No
// profiler.

use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::json;
use zega::{Storage, Zega};

/// Each ZQL chunk stays under #127's 2,000,000-byte cap.
const CHUNK_LIMIT: usize = 1_900_000;

/// The schema both paths load with: `Item { qid name kind pop }`, a unique
/// `qid` (the Wikidata-style key a relationship load links by), and `links`.
const SCHEMA: &str = "schema {\n  type Item {\n    qid: String\n    name: String\n    kind: String\n    pop: Int\n    links: LINK -> Item[]\n  }\n}\n\nunique {\n  Item { qid }\n}\n";

const KINDS: [&str; 8] = ["city", "person", "river", "lake", "company", "school", "park", "event"];

/// splitmix64, as zega-mem and zega-scale: the same graph everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[cfg(unix)]
fn rss() -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok()?;
    Some(kib * 1024)
}

#[cfg(not(unix))]
fn rss() -> Option<u64> {
    None
}

/// Peak RSS of this process (`getrusage`; `ru_maxrss` is bytes on macOS).
#[cfg(unix)]
fn peak_rss() -> Option<u64> {
    // SAFETY: getrusage writes into the zeroed struct we own.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return None;
    }
    let max = usage.ru_maxrss as u64;
    Some(if cfg!(target_os = "macos") { max } else { max * 1024 })
}

#[cfg(not(unix))]
fn peak_rss() -> Option<u64> {
    None
}

/// Writes JSON arrays of rows into files of at most [`CHUNK_LIMIT`] bytes.
struct Chunks<'a> {
    dir: &'a Path,
    stem: &'static str,
    names: Vec<String>,
    body: String,
}

impl Chunks<'_> {
    fn push(&mut self, row: &str) {
        if !self.body.is_empty() && self.body.len() + row.len() + 2 > CHUNK_LIMIT {
            self.flush();
        }
        self.body.push(if self.body.is_empty() { '[' } else { ',' });
        self.body.push_str(row);
    }
    fn flush(&mut self) {
        if self.body.is_empty() {
            return;
        }
        self.body.push(']');
        let name = format!("{}-{:05}.json", self.stem, self.names.len() + 1);
        std::fs::write(self.dir.join(&name), &self.body).expect("write chunk");
        self.names.push(name);
        self.body.clear();
    }
}

fn gen(args: &[String]) {
    let count = |at: usize| -> u64 {
        args.get(at)
            .and_then(|s| s.replace('_', "").parse().ok())
            .unwrap_or_else(|| usage())
    };
    let (nodes, rels) = (count(0), count(1));
    let dir = PathBuf::from(args.get(2).unwrap_or_else(|| usage()));
    std::fs::create_dir_all(dir.join("zql")).expect("create dir");
    std::fs::write(dir.join("schema.zql"), SCHEMA).expect("write schema");

    let mut rng = Rng(0x0127_0128_b011_0000 ^ nodes ^ (rels << 32));
    let mut jsonl = BufWriter::new(std::fs::File::create(dir.join("graph.jsonl")).expect("graph.jsonl"));
    let mut node_chunks = Chunks { dir: &dir.join("zql"), stem: "nodes", names: Vec::new(), body: String::new() };
    for i in 1..=nodes {
        let kind = KINDS[(i % KINDS.len() as u64) as usize];
        let name = format!("{kind} {:x}", rng.next() >> 24);
        let pop = rng.below(10_000_000) as i64;
        let props = json!({ "qid": format!("Q{i}"), "name": name, "kind": kind, "pop": pop });
        writeln!(jsonl, "{}", json!({ "type": "node", "id": i, "labels": ["Item"], "props": props })).expect("write");
        node_chunks.push(&props.to_string());
    }
    node_chunks.flush();
    let mut rel_chunks = Chunks { dir: &dir.join("zql"), stem: "rels", names: Vec::new(), body: String::new() };
    for _ in 0..rels {
        let from = rng.below(nodes) + 1;
        let to = rng.below(nodes) + 1;
        writeln!(jsonl, "{}", json!({ "type": "rel", "kind": "LINK", "from": from, "to": to })).expect("write");
        rel_chunks.push(&json!({ "from": format!("Q{from}"), "to": format!("Q{to}") }).to_string());
    }
    rel_chunks.flush();
    jsonl.flush().expect("flush");

    let mut zql = String::from(SCHEMA);
    for name in &node_chunks.names {
        zql.push_str(&format!(
            "\nmutation json [\"./zql/{name}\"] {{\n  Item(qid: $qid && name: $name && kind: $kind && pop: $pop)\n}}\n"
        ));
    }
    for name in &rel_chunks.names {
        zql.push_str(&format!(
            "\nmutation json [\"./zql/{name}\"] {{\n  Item(qid: $from) {{ links -> link Item(qid: $to) }}\n}}\n"
        ));
    }
    std::fs::write(dir.join("load.zql"), zql).expect("write load.zql");
    let jsonl_bytes = std::fs::metadata(dir.join("graph.jsonl")).expect("meta").len();
    println!(
        "{}",
        json!({
            "nodes": nodes,
            "relationships": rels,
            "jsonl_bytes": jsonl_bytes,
            "zql_chunks": node_chunks.names.len() + rel_chunks.names.len(),
        })
    );
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].as_str())
}

fn storage(args: &[String]) -> Storage {
    match flag(args, "--storage") {
        Some("file") => Storage::File,
        Some("sqlite") | None => Storage::Sqlite,
        Some(_) => usage(),
    }
}

/// The path before the bulk loader: every 2 MB chunk is one ZQL statement
/// (all or nothing, through the WAL), run one after another.
fn zql(args: &[String]) {
    let dir = PathBuf::from(args.first().unwrap_or_else(|| usage()));
    let data = flag(args, "--data").unwrap_or_else(|| usage());
    let load = std::fs::read_to_string(dir.join("load.zql")).expect("load.zql");
    let statements: Vec<&str> = load.split("\nmutation json ").skip(1).collect();
    let zega = Zega::open(data).storage(storage(args)).build().expect("open");
    let started = Instant::now();
    for (at, statement) in statements.iter().enumerate() {
        let location = statement
            .split('"')
            .nth(1)
            .expect("a quoted location");
        let text = std::fs::read_to_string(dir.join(location)).expect("chunk");
        let source = format!("{SCHEMA}\nmutation json {statement}");
        let sources = HashMap::from([(location.to_string(), text)]);
        zega.apply_zql_with_sources(&source, &sources)
            .unwrap_or_else(|error| panic!("chunk {at} ({location}): {error}"));
    }
    let load_ms = started.elapsed().as_secs_f64() * 1e3;
    let counts = zega.counts().expect("counts");
    println!(
        "{}",
        json!({
            "path": "zql-2mb-chunks",
            "statements": statements.len(),
            "nodes": counts.nodes,
            "relationships": counts.relationships,
            "load_ms": load_ms,
            "rss_after_load_bytes": rss(),
            "peak_rss_bytes": peak_rss(),
        })
    );
}

fn reopen(args: &[String]) {
    let data = flag(args, "--data").unwrap_or_else(|| usage());
    let started = Instant::now();
    let zega = Zega::open(data).storage(Storage::of(Path::new(data)).unwrap_or(Storage::Sqlite)).build().expect("open");
    let open_ms = started.elapsed().as_secs_f64() * 1e3;
    // One query with the schema declares the unique index, as the first
    // request after a restart would.
    let probe = zega.run_lang(SCHEMA, "{ Item(qid: \"Q1\") { qid name links -> Item { qid } } }").expect("probe");
    let counts = zega.counts().expect("counts");
    println!(
        "{}",
        json!({
            "path": "reopen",
            "nodes": counts.nodes,
            "relationships": counts.relationships,
            "open_ms": open_ms,
            "resting_rss_bytes": rss(),
            "peak_rss_bytes": peak_rss(),
            "probe": probe,
        })
    );
}

fn usage() -> ! {
    eprintln!(
        "usage: zega-bulk gen <nodes> <relationships> <dir>\n       zega-bulk zql <dir> --data <data-dir> [--storage file|sqlite]\n       zega-bulk reopen --data <data-dir>"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => gen(&args[1..]),
        Some("zql") => zql(&args[1..]),
        Some("reopen") => reopen(&args[1..]),
        _ => usage(),
    }
}
