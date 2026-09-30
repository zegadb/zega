// zega-scale: the travel-graph scale bench of zegadb/zega#143. Where do the
// wasm (zega.earth) and server paths stop being acceptable, at 1k to 1M
// travel-shaped nodes?
//
//   zega-scale gen <nodes> <dir>
//   zega-scale server <dir> --data <persist-dir>
//
// `gen` writes a deterministic (seeded by the node count) synthetic travel
// graph in the shape zega.earth's samples use (zegadb/rixse-intake output): a
// `.zql` file whose `mutation csv` blocks name shards of at most 2 MB
// (zegadb/zega#127: the engine refuses an import source past 2,000,000
// bytes), plus the fixed query set (`queries.json`) both paths run, so the
// browser page and this binary measure identical statements.
//
// Shape: `Place { name zid kind at description }` with a ~200-character
// description, `Source` nodes like the intake's, and ~4 relationships per
// place: `locatedIn` (a tree of container places), `mentionedIn` (to a
// Source) and two `near` (to places in the same geographic cluster).
//
// `server` is the native path: it loads the graph the way the wasm host does
// — `zql_load_locations` on the .zql, the shard texts read in, one
// `apply_zql_with_sources` — but into a persistent data directory, the way
// zega-server holds a graph. It reports load time, heap (a counting global
// allocator, as zega-mem does for #128) and RSS, on-disk size after a
// checkpoint, a restart's open time, and p50/p95/p99 over 20 runs of each
// query in queries.json. One process prints one JSON line.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use serde_json::{json, Value as Json};
use zega::Zega;

// ---------------------------------------------------------------- allocator
// The same counting allocator zega-mem installs: "live bytes" is exact heap.

static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            grew(layout.size() as i64);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            grew(layout.size() as i64);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            let delta = new_size as i64 - layout.size() as i64;
            let now = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        new_ptr
    }
}

fn grew(size: i64) {
    let now = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn live() -> i64 {
    LIVE.load(Ordering::Relaxed)
}

fn reset_peak() {
    PEAK.store(live(), Ordering::Relaxed);
}

fn peak() -> i64 {
    PEAK.load(Ordering::Relaxed)
}

/// This process's RSS now, in bytes (`ps`, which reports KiB).
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

/// Peak RSS over the process so far (`getrusage`; `ru_maxrss` is bytes on macOS).
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

// ---------------------------------------------------------------- rng

/// splitmix64, as zega-mem: the same graph for the same node count, everywhere.
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
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

// ---------------------------------------------------------------- the shape

/// Stay under zega#127's 2,000,000-byte import ceiling with margin.
const SHARD_LIMIT: usize = 1_900_000;
/// Places are spread over this many geographic clusters; `near` links stay
/// inside a place's own cluster, so the relationship means what it says.
const CLUSTERS: u64 = 200;
/// One Source per this many nodes, like the intake's entity-to-source ratio.
const SOURCE_EVERY: u64 = 20;
const QUERY_RUNS: usize = 20;

const KINDS: [&str; 12] = [
    "park", "cafe", "museum", "hotel", "landmark", "restaurant", "beach", "market", "garden", "bridge", "gallery",
    "viewpoint",
];

const ADJ: [&str; 24] = [
    "Blue", "Golden", "Silent", "North", "Lake", "Mount", "Cape", "Port", "Fort", "San", "Rio", "Grand", "Old",
    "New", "Little", "Great", "Red", "White", "Green", "Long", "Fair", "East", "West", "South",
];

const NOUN: [&str; 24] = [
    "Harbour", "Falls", "Ridge", "Meadow", "Square", "Gardens", "Point", "Bay", "Hill", "Springs", "Crossing",
    "Terrace", "Cove", "Market", "Palace", "Tower", "Bridge", "Park", "Beach", "Cliff", "Valley", "Shore",
    "Gate", "Promenade",
];

const WORDS: [&str; 32] = [
    "visitors", "wander", "cobbled", "streets", "overlooking", "harbour", "morning", "market", "stalls", "local",
    "guides", "gather", "sunset", "terrace", "museum", "trails", "ferry", "crossing", "historic", "quarter",
    "cafes", "gardens", "viewpoint", "cliffs", "old", "town", "square", "festival", "season", "travelers",
    "kitchen", "shoreline",
];

fn place_name(rng: &mut Rng, i: u64) -> String {
    format!(
        "{} {} {}",
        ADJ[rng.below(ADJ.len() as u64) as usize],
        NOUN[rng.below(NOUN.len() as u64) as usize],
        i
    )
}

/// About 200 characters of travel-blurb text: letters, spaces and full stops
/// only, so a cell never needs CSV quoting.
fn description(rng: &mut Rng) -> String {
    let mut text = String::with_capacity(210);
    while text.len() < 195 {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(WORDS[rng.below(WORDS.len() as u64) as usize]);
        if rng.below(7) == 0 {
            text.push('.');
        }
    }
    text.truncate(200);
    while text.ends_with('.') {
        text.pop();
    }
    text.push('.');
    text
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// CSV rows into shards of at most SHARD_LIMIT bytes, header per shard.
/// Returns the shard file names.
fn shards(dir: &std::path::Path, stem: &str, header: &str, rows: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    let mut body = String::with_capacity(SHARD_LIMIT / 4);
    let flush = |names: &mut Vec<String>, body: &mut String| {
        if body.is_empty() {
            return;
        }
        let name = format!("{stem}-{:03}.csv", names.len() + 1);
        let mut text = String::with_capacity(header.len() + 1 + body.len());
        text.push_str(header);
        text.push('\n');
        text.push_str(body);
        std::fs::write(dir.join(&name), text).expect("write shard");
        names.push(name);
        body.clear();
    };
    for row in rows {
        if header.len() + 1 + body.len() + row.len() + 1 > SHARD_LIMIT {
            flush(&mut names, &mut body);
        }
        body.push_str(row);
        body.push('\n');
    }
    flush(&mut names, &mut body);
    names
}

/// The generated graph's counts, shared by `gen`'s output and the doc.
struct Counts {
    places: u64,
    containers: u64,
    sources: u64,
    rels: u64,
}

/// Every row of the graph, deterministic for `n`, plus each place's name
/// (drawn from the same rng stream, so queries can name real places). Kept
/// out of the engine: generation time is not load time.
#[allow(clippy::type_complexity)] // one tuple, one row pass: splitting it is noise
fn rows(n: u64) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<String>, Counts) {
    let mut rng = Rng(0x0143_5ca1_e000_0000 ^ n);
    let sources_n = (n / SOURCE_EVERY).max(1);
    let places_n = n - sources_n;
    let containers = (places_n / 50).max(1);

    let centers: Vec<(f64, f64)> = (0..CLUSTERS)
        .map(|_| (rng.unit() * 130.0 - 60.0, rng.unit() * 358.0 - 179.0))
        .collect();

    let zid = |i: u64| format!("p{i:07}");
    let mut names = Vec::with_capacity(places_n as usize);
    let mut places = Vec::with_capacity(places_n as usize);
    let mut located_in = Vec::with_capacity(places_n as usize);
    let mut mentioned_in = Vec::with_capacity(places_n as usize);
    let mut near = Vec::with_capacity((places_n * 2) as usize);
    for i in 0..places_n {
        let container = i < containers;
        let kind = if container { "city" } else { KINDS[(i % 12) as usize] };
        let (clat, clon) = centers[(i % CLUSTERS) as usize];
        let lat = clat + (rng.unit() - 0.5);
        let lon = clon + (rng.unit() - 0.5);
        let name = place_name(&mut rng, i);
        let row = format!(
            "{},{},{},{},{:.6},{:.6}",
            csv_field(&name),
            zid(i),
            kind,
            csv_field(&description(&mut rng)),
            lat,
            lon
        );
        names.push(name);
        places.push(row);
        if container {
            if i > 0 {
                located_in.push(format!("{},{}", zid(i), zid(i / 8)));
            }
        } else {
            located_in.push(format!("{},{}", zid(i), zid(rng.below(containers))));
        }
        mentioned_in.push(format!("{},s{:07}", zid(i), rng.below(sources_n)));
        for _ in 0..2 {
            // Another place in the same geographic cluster: places with the
            // same index mod CLUSTERS share a centre.
            let c = i % CLUSTERS;
            let count = (places_n - 1 - c) / CLUSTERS + 1;
            if count < 2 {
                continue;
            }
            let to = ((i / CLUSTERS + rng.below(count - 1) + 1) % count) * CLUSTERS + c;
            near.push(format!("{},{}", zid(i), zid(to)));
        }
    }
    let mut sources = Vec::with_capacity(sources_n as usize);
    for s in 0..sources_n {
        let kind = ["osm", "wikidata", "guide"][(s % 3) as usize];
        sources.push(format!(
            "s{s:07},{kind},ref-{s},https://example.org/{kind}/{s},2026-09-30T00:00:00Z"
        ));
    }
    let rels = (located_in.len() + mentioned_in.len() + near.len()) as u64;
    let counts = Counts {
        places: places_n,
        containers,
        sources: sources_n,
        rels,
    };
    (places, sources, located_in, mentioned_in, near, names, counts)
}

// ---------------------------------------------------------------- gen

fn gen(args: &[String]) {
    let n: u64 = args
        .first()
        .and_then(|s| s.replace('_', "").parse().ok())
        .unwrap_or_else(|| usage());
    let dir = std::path::PathBuf::from(args.get(1).unwrap_or_else(|| usage()));
    std::fs::create_dir_all(&dir).expect("data dir");

    let (places, sources, located_in, mentioned_in, near, names, counts) = rows(n);
    let place_shards = shards(&dir, "places", "name,zid,kind,description,lat,lon", &places);
    let source_shards = shards(&dir, "sources", "zid,kind,ref,url,fetchedAt", &sources);
    let located_shards = shards(&dir, "edges-locatedIn", "from,to", &located_in);
    let mentioned_shards = shards(&dir, "edges-mentionedIn", "from,to", &mentioned_in);
    let near_shards = shards(&dir, "edges-near", "from,to", &near);

    let mut zql = String::new();
    zql.push_str(
        "// Synthetic travel graph for the scale bench (zegadb/zega#143), in the\n\
         // shape of zega.earth's intake samples: every entity has a zid and a\n\
         // Source. Generated by `zega-scale gen`; deterministic per node count.\n\
         schema {\n  type Place {\n    name: String\n    zid: String\n    kind: String\n    \
         at: Point from (lat, lon)\n    description: String\n    locatedIn: LOCATED_IN -> Place[]\n    \
         mentionedIn: MENTIONED_IN -> Source[]\n    near: NEAR -> Place[]\n  }\n  type Source {\n    \
         zid: String\n    kind: String\n    ref: String\n    url: String<url>\n    fetchedAt: String\n    \
         mentions: MENTIONED_IN <- Place[]\n  }\n}\n\nunique {\n  Place { zid }\n  Source { zid }\n}\n\n",
    );
    let mut blocks = String::new();
    for name in &place_shards {
        blocks.push_str(&format!(
            "mutation csv [\"./{name}\"] {{\n  Place(name: $name && zid: $zid && kind: $kind && description: $description) {{ name at }}\n}}\n\n"
        ));
    }
    for name in &source_shards {
        blocks.push_str(&format!(
            "mutation csv [\"./{name}\"] {{\n  Source(zid: $zid && kind: $kind && ref: $ref && url: $url && fetchedAt: $fetchedAt)\n}}\n\n"
        ));
    }
    for (field, target, list) in [
        ("locatedIn", "Place", &located_shards),
        ("mentionedIn", "Source", &mentioned_shards),
        ("near", "Place", &near_shards),
    ] {
        for name in list {
            blocks.push_str(&format!(
                "mutation csv [\"./{name}\"] {{\n  Place(zid: $from) {{ {field} -> link {target}(zid: $to) }}\n}}\n\n"
            ));
        }
    }
    zql.push_str(blocks.trim_end());
    zql.push('\n');
    std::fs::write(dir.join("travel.zql"), &zql).expect("write travel.zql");

    // The fixed query set both paths run: name search (an unindexed scan,
    // since zids carry the unique index, as in the intake samples), one-hop
    // neighbours (indexed by zid), and places-near (nearest ten to a cluster
    // centre, the explorer's "Nearest to the Calgary Tower" shape).
    let mut rng = Rng(0x0143_9e9e_0000_0000 ^ n);
    let mut search = Vec::with_capacity(QUERY_RUNS);
    let mut neighbours = Vec::with_capacity(QUERY_RUNS);
    let mut near_q = Vec::with_capacity(QUERY_RUNS);
    for _ in 0..QUERY_RUNS {
        let i = rng.below(counts.places);
        let name = &names[i as usize];
        search.push(format!("{{ Place(name: \"{name}\") {{ name kind at }} }}"));
        neighbours.push(format!(
            "{{ Place(zid: \"p{i:07}\") {{ name locatedIn -> Place {{ name }} near -> Place {{ name }} mentionedIn -> Source {{ ref }} }} }}"
        ));
        let c = rng.below(CLUSTERS);
        let (lat, lon) = cluster_center(n, c);
        near_q.push(format!(
            "{{ Place order by @distance(at, @point({lat:.6}, {lon:.6})) limit 10 {{ name @distance(at, @point({lat:.6}, {lon:.6})) }} }}"
        ));
    }
    let queries = json!({ "search": search, "neighbours": neighbours, "near": near_q });
    std::fs::write(dir.join("queries.json"), queries.to_string()).expect("write queries.json");

    let shard_bytes: u64 = std::fs::read_dir(&dir)
        .expect("read dir")
        .map(|e| e.expect("entry").metadata().expect("meta").len())
        .sum();
    let meta = json!({
        "nodes": n,
        "places": counts.places,
        "containers": counts.containers,
        "sources": counts.sources,
        "rels": counts.rels,
        "shards": place_shards.len() + source_shards.len() + located_shards.len() + mentioned_shards.len() + near_shards.len(),
        "data_bytes": shard_bytes,
    });
    std::fs::write(dir.join("meta.json"), meta.to_string()).expect("write meta.json");
    println!("{meta}");
}

/// Cluster c's centre, regenerated from the same stream `rows` draws them
/// from (the first CLUSTERS pairs of unit draws).
fn cluster_center(n: u64, c: u64) -> (f64, f64) {
    let mut rng = Rng(0x0143_5ca1_e000_0000 ^ n);
    let mut center = (0.0, 0.0);
    for i in 0..CLUSTERS {
        center = (rng.unit() * 130.0 - 60.0, rng.unit() * 358.0 - 179.0);
        if i == c {
            break;
        }
    }
    center
}

// ---------------------------------------------------------------- server

fn timed(samples: &mut [f64]) -> Json {
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() as f64 - 1.0) * p).round() as usize];
    json!({ "runs": samples.len(), "p50_ms": at(0.50), "p95_ms": at(0.95), "p99_ms": at(0.99) })
}

fn dir_bytes(dir: &std::path::Path) -> u64 {
    let mut total = 0;
    for entry in walkdir(dir) {
        total += entry.metadata().expect("meta").len();
    }
    total
}

fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn run_queries(zega: &Zega, schema: &str, queries: &Json) -> Json {
    let mut out = serde_json::Map::new();
    for name in ["search", "neighbours", "near"] {
        let texts: Vec<&str> = queries[name]
            .as_array()
            .expect("query list")
            .iter()
            .map(|q| q.as_str().expect("query text"))
            .collect();
        let warm = zega.run_lang(schema, texts[0]).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!warm.is_null(), "{name}: warm run matched nothing");
        let mut samples = Vec::with_capacity(texts.len());
        for text in &texts {
            let t = Instant::now();
            let out_text = zega.run_lang(schema, text).unwrap_or_else(|e| panic!("{name}: {e}"));
            samples.push(t.elapsed().as_secs_f64() * 1e3);
            std::hint::black_box(out_text);
        }
        out.insert(name.to_string(), timed(&mut samples));
    }
    Json::Object(out)
}

fn server(args: &[String]) {
    let dir = std::path::PathBuf::from(args.first().unwrap_or_else(|| usage()));
    let data = args
        .windows(2)
        .find(|w| w[0] == "--data")
        .map(|w| std::path::PathBuf::from(&w[1]))
        .unwrap_or_else(|| usage());

    let source = std::fs::read_to_string(dir.join("travel.zql")).expect("travel.zql");
    let meta: Json = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).expect("meta.json")).expect("meta");
    let queries: Json =
        serde_json::from_str(&std::fs::read_to_string(dir.join("queries.json")).expect("queries.json")).expect("queries");

    let base = live();
    let base_rss = rss();

    // The wasm host's load sequence, natively: locations from the .zql, the
    // shard texts read in, one apply with the source map.
    let locations = zega::zql_load_locations(zega::ZqlEntryPoint::File, &source).expect("load locations");
    let mut sources: HashMap<String, String> = HashMap::with_capacity(locations.len());
    let mut shard_bytes = 0u64;
    for location in &locations {
        let text = std::fs::read_to_string(dir.join(location.trim_start_matches("./"))).expect("shard");
        shard_bytes += text.len() as u64;
        sources.insert(location.clone(), text);
    }

    let zega = Zega::open(data.to_str().expect("data dir utf8")).build().expect("open");
    let before_load = live();
    reset_peak();
    let started = Instant::now();
    zega.apply_zql_with_sources(&source, &sources).expect("apply");
    let load_ms = started.elapsed().as_secs_f64() * 1e3;
    let load_peak_bytes = peak() - before_load;
    drop(sources);
    let resting_heap = live() - base;

    // The first statement with the schema declares the unique indexes. Like
    // the explorer's `run(source, zql)`, statements carry the whole .zql
    // document as their schema argument, so both paths parse the same text.
    let started = Instant::now();
    zega.run_lang(&source, "{ Place(zid: \"p0000000\") { zid } }").expect("probe");
    let index_ms = started.elapsed().as_secs_f64() * 1e3;
    let settled_heap = live() - base;
    let settled_rss = rss().map(|r| r.saturating_sub(base_rss.unwrap_or(0)));

    let query_json = run_queries(&zega, &source, &queries);

    // Persist: checkpoint so the on-disk size is the steady state's, not a
    // WAL holding every write since the beginning of the load.
    let checkpoint = zega.checkpoint().expect("checkpoint");
    drop(zega);
    let on_disk_bytes = dir_bytes(&data);

    // A restart: open the data directory in this same fresh state it would
    // have after a server restart (the graph file replay, not the WAL).
    let restart_base = live();
    let started = Instant::now();
    let reopened = Zega::open(data.to_str().expect("data dir utf8")).build().expect("reopen");
    let reopen_ms = started.elapsed().as_secs_f64() * 1e3;
    reopened.run_lang(&source, "{ Place(zid: \"p0000000\") { zid } }").expect("reopen probe");
    let reopen_heap = live() - restart_base;
    drop(reopened);

    let out = json!({
        "path": "server",
        "nodes": meta["nodes"],
        "places": meta["places"],
        "sources": meta["sources"],
        "rels": meta["rels"],
        "shard_bytes": shard_bytes,
        "load_ms": load_ms,
        "load_peak_bytes": load_peak_bytes,
        "resting_heap_bytes": resting_heap,
        "settled_heap_bytes": settled_heap,
        "rss_bytes": settled_rss,
        "peak_rss_bytes": peak_rss(),
        "index_ms": index_ms,
        "on_disk_bytes": on_disk_bytes,
        "checkpoint_graph_bytes": checkpoint.map(|c| c.graph_bytes),
        "reopen_ms": reopen_ms,
        "reopen_heap_bytes": reopen_heap,
        "queries": query_json,
    });
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    writeln!(lock, "{out}").unwrap();
}

fn usage() -> ! {
    eprintln!("usage: zega-scale gen <nodes> <dir>\n       zega-scale server <dir> --data <persist-dir>");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => gen(&args[1..]),
        Some("server") => server(&args[1..]),
        _ => usage(),
    }
}
