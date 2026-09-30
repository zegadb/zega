# zega-bench

Benchmarks for the zega engine. Each binary is separate so it can install its
own measurement machinery (a counting global allocator) without sharing it.

## zega-bench

zega (embedded, ZQL) side by side with Neo4j (client/server, Cypher over
Bolt): single-node writes, then indexed lookups, with throughput and p50/p99
latency. Memory (RSS) is captured by the surrounding driver script.

## zega-mem — bytes per node (#100, #128)

Heap (exact, via the counting allocator) and RSS per node and per
relationship for several graph shapes, with query timings that must not
regress while memory comes down:

    scripts/mem-bench.sh results.jsonl
    .target/release/zega-mem table results.jsonl

One `run` is one process and one JSON line; `snapshot`/`reopen` measure a
restart opening a data directory in a fresh process.

## zega-scale — travel-graph scale bench (#143)

Where the two paths zega.earth could use stop being acceptable, on synthetic
travel-shaped graphs (1k–1M nodes: `Place { name zid kind at description }`,
`Source` nodes, ~4 relationships per place). One command runs both paths:

    scripts/scale-bench.sh [size ...]     # default: 1000 10000 100000 1000000

- `zega-scale gen <nodes> <dir>` writes a deterministic sample in the shape
  zega.earth loads: a `.zql` file whose `mutation csv` blocks name shards of
  at most 2 MB (#127), plus `queries.json`, the fixed query set (name search,
  one-hop neighbours, places-near) both paths run.
- **Server** (`zega-scale server <dir> --data <persist-dir>`): the native
  engine loading the sample into a persistent data directory. Reports load
  time, heap and RSS (#128's method), on-disk size after a checkpoint,
  restart time, and query p50/p95/p99.
- **Browser** (`scripts/scale-browser.mjs`, driven by the script): the wasm
  engine in headless Chromium loading the same sample exactly as zega.earth
  does (`load_locations` + fetch + `apply_with_sources`), on a desktop
  profile and with 4× CDP CPU throttling as a phone proxy. Reports gzipped
  download size, download/load time, JS heap and wasm memory, and query
  p50/p95/p99.

Generated data and results live under `.tmp/scale-143/` (gitignored); the
latest results and the recommendation are in `docs/scale-results.md`.
