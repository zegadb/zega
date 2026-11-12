# ZQL data loading

```zql
schema {
  type Player {
    name: String
    salary: Int
  }
}

mutation csv ["./players.csv"] {
  Player(name: $Name && salary: $Salary) { name salary }
}

mutation json ["https://example.com/players.json"] {
  Player(name: $Name && salary: $Salary) { name salary }
}
```

JSON accepts an object or an array of objects. CSV uses a header row and the
Rust `csv` crate, including quoted commas, escaped quotes and multiline cells.
Records must have the same number of fields as the header; headers must be
nonempty and unique. CSV cells retain ZQL's existing scalar inference (null,
boolean, integer, decimal, otherwise string). JSON retains its original types.
Use `$"Full Name"` for a column containing spaces. Normal mutation rules,
including required fields, unique constraints and relationship linking, apply.

Native `Zega::run_lang` and `Zega::apply_zql` read files relative to the **process
working directory**, for both in-memory and disk databases. Absolute paths are
also accepted. The existing prohibition on `..` path components remains. The
HTTP client is blocking `ureq` with rustls, requires no Tokio runtime, ignores
proxy environment variables, has a 20-second request timeout and follows at
most five redirects. Imports are limited to 2,000,000 UTF-8 bytes per source;
reads stop at that limit instead of buffering an unbounded response. The
limit bounds what one statement holds (an HTTP request body included) and
keeps it all or nothing. A bigger dataset is not split by hand: it takes the
[bulk path](#bulk-loading), which has no size limit.

The `http` Cargo feature is enabled by default. `default-features = false`
disables native HTTP while retaining local-file and supplied-source imports.
HTTP dependencies and native I/O are excluded on wasm32, even under
`--all-features`; `zega-wasm` explicitly disables default features.

Public HTTP(S) addresses are accepted by default. Private/loopback hosts and
resolved private IPs are rejected, including redirect targets. Trusted native
callers can explicitly build with `.allow_private_imports(true)`. This allows
local fixture servers without changing the default conformance policy. URL
credentials and non-HTTP schemes are rejected. This is not a filesystem sandbox:
callers authorized to execute ZQL may read files accessible to the process.

Fetching and parsing happen before taking the graph lock. Rows insert under
that lock through the same mutation and WAL operations as handwritten ZQL.
A malformed source is rejected before insertion. A load is one statement, and
every statement is all-or-nothing: if any row fails a constraint, or the WAL
refuses the write, no row of the load is kept, in memory or in the WAL.

## Bulk loading

A whole graph of any size (millions of nodes, a multi-GB file) loads in one
step from **JSON lines**, one node or relationship per line:

```sh
zega-server import graph.jsonl --data ./zega-data --schema schema.zql --meta source=wikidata
```

```json
{"type":"node","id":42,"labels":["Item"],"props":{"qid":"Q42","name":"Douglas Adams","at":{"lat":51.5,"lon":-0.1}}}
{"type":"node","labels":["Item"],"props":{"qid":"Q5","name":"human"}}
{"type":"rel","kind":"P31","from":42,"to":43,"props":{"rank":"normal"}}
```

- `"type"` is `"node"` or `"rel"`. A node has `labels` (its first label is
  its ZQL type) and an optional `id`; without one it gets the next id after
  the highest so far. A relationship has a `kind`, `from` and `to` (node
  ids), and an optional `id`. Both have optional `props`. Any other field is
  refused, and so is an id used twice.
- **Nodes come before the relationships that use them**: a relationship whose
  `from` or `to` no earlier line defines is refused. An intake writes every
  node, then every relationship.
- Property values convert exactly as in `mutation json` (the same function):
  with `--schema`, a field the node's type declares as `T[]`, a vector or a
  `Reference` takes that type; otherwise strings, numbers, booleans and null
  are themselves, an object is a `Point` (`{"lat","lon"}`) and an array a
  vector, so a list of strings needs a `T[]` field in the schema.
- The schema's `unique` fields are checked across the whole load (a second
  node with the same `Item.qid` is refused), its indexes are built, and the
  graph carries the schema and the `--meta` metadata as an imported `.graph`
  file does. Other schema rules (required fields, field types) are not
  checked: the loader stores what the lines say.
- Blank lines are skipped; a line may be up to 16 MiB. `-` reads stdin.
  `zega-server import` tells a JSON-lines file from a `.graph` file by its
  first byte; `--schema` and `--meta` are for JSON lines.

**All or nothing, like a `.graph` import** ([graph-format.md](graph-format.md#zegas-implementation)).
Every line is read and checked into a new graph while the old one keeps
serving; a bad line is reported by its number (`invalid JSON-lines graph:
… (line 1812)`) and nothing changes. A disk database (file or SQLite
storage) then writes the new graph as a `.graph` file in `graphs/` in one
streaming pass and commits it with one WAL entry: a crash before that entry
leaves the old graph, a restart after it reads the file. Like an import, it
replaces the whole graph (`--replace` for a database that holds one).

**Memory is the graph being built plus one line** (#128). Nothing else grows
with the input: no line is kept once read, and the `.graph` file is written
from the graph with fixed buffers. A load into an empty database peaks close
to the graph's resting size, where loading the same graph as 2 MB ZQL
statements peaks at ~2x (measured in zegadb/zega#127's PR with
`zega-bulk`, `zega-bench/README.md`). Replacing a graph holds the old one
until the new one is installed, so that peak is the two graphs together.

The crate path is `Zega::import_json_lines(reader, &JsonLinesOptions { schema,
meta })`, returning the same `ImportSummary` as `Zega::import`; its
`content_sha256` is the stored `.graph` file's content id. `PUT /graph` and
ZQL loads keep their size caps: the bulk path is for the CLI and the crate.

## Browser hosts

Wasm performs no file I/O or blocking HTTP. Hosts call `zql_load_locations` to
get validated locations, fetch raw text asynchronously (or use a file picker),
and call `run_lang_with_sources` / `apply_zql_with_sources` with a map from each
literal ZQL location to its raw UTF-8 text. Missing entries are errors; there is
no network fallback. `parse_import` exposes the same Rust parser for previews.
The corresponding wasm methods serialize the source map as a JSON envelope;
that envelope does not parse or convert data rows in JavaScript.

The explorer uses JS `fetch` / `File.text` for transport and Rust for both
preview parsing and insertion. Its Import UI maps column names to a ZQL load
template and sends the original text to the engine. JS renders the Rust preview
and constructs the template, but does not parse JSON/CSV rows, coerce cells,
generate one literal mutation per row, or silently cap imports at 150 rows.
Imports now have actual ZQL semantics: one graph per row, including repeated
values; use explicit ZQL `link` to connect to an existing unique node.

## Before this change (c14fbd3)

| Consumer | Local file reader | Remote fetcher | JSON parser | CSV parser |
| --- | --- | --- | --- | --- |
| Native library, disk | Rust std::fs, process cwd | Rust reqwest::blocking | serde_json + json_rows | handwritten Rust parser |
| Native library, memory | Same as disk | Same as disk | Same as disk | Same as disk |
| zega-server | Not exposed by the server API of that revision | Not exposed | Only HTTP request envelopes | No load API |
| zega-wasm | No disk access; relative locations are browser URLs | Synchronous XHR via web-sys | Rust serde_json | handwritten Rust parser |
| Explorer ZQL editor | Relative browser URLs via wasm | wasm XHR | Rust engine | Rust engine |
| Explorer Import dialog | JS File.text/drop/clipboard | JS fetch | JS JSON.parse/jsonTable | JS parseCsv |

The library already inserted imports through the WAL. The explorer Import
dialog previously generated literal mutations from JS-parsed rows. The CLI PR
replaces the old server API with ZQL so native server callers use this loader.
