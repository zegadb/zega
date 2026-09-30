# Scale results: wasm (zega.earth) and server paths, 1k–1M travel nodes

zegadb/zega#143. Measured 2026-09-30 with `scripts/scale-bench.sh` (see
`zega-bench/README.md`; raw JSON under `.tmp/scale-143/`, gitignored), on the
fleet iMac: **iMac20,2, Intel Core i9-10910 (10C/20T, 3.60 GHz), 128 GB RAM,
macOS 26.7, x86_64**. Engine at 07a47f4 (`origin/main`); the browser path ran
the committed `browser/pkg` wasm build (vendored in 61217bb from engine
cef8edd8) — the same bytes zega.earth serves today.

The graphs are synthetic and travel-shaped (deterministic per node count):
`Place { name zid kind at description }` with a ~200-character description,
`Source` nodes (1 per 20 nodes), ~4 relationships per place (`locatedIn`,
`mentionedIn`, two `near`), delivered as a `.zql` plus CSV shards of ≤ 1.9 MB
(#127's 2 MB import ceiling). Both paths run the same 20 statements per query
from `queries.json`: **search** (`Place(name: …)`, an unindexed scan — the
zid carries the unique index, as in the intake samples), **neighbours**
(one hop out via all three relationships, by indexed zid), **near**
(`order by @distance … limit 10`, the explorer's "Nearest to" shape).

## Browser: wasm engine in headless Chromium (how zega.earth loads today)

The page loads the sample exactly as zega.earth's `loadSample` does
(`load_locations` → fetch each CSV → `apply_with_sources`). "First usable" is
page-open to the first query's result. Phone = CDP CPU throttling 4×.

| nodes | gz download | desktop: download / load / first usable | phone 4×: download / load / first usable | JS heap | wasm memory |
|---:|---:|---|---|---:|---:|
| 1k | 0.07 MB | 21 ms / 84 ms / **113 ms** | 71 ms / 337 ms / **427 ms** | 2 MB | 9.5 MB |
| 10k | 0.72 MB | 40 ms / 541 ms / **590 ms** | 150 ms / 2.4 s / **2.6 s** | 11 MB | 85 MB |
| 100k | 7.0 MB | 225 ms / 5.6 s / **5.8 s** | 878 ms / 25.8 s / **26.8 s** | 62 MB | 589 MB |
| 1M | 72.2 MB | 2.2 s / 73.2 s / **75.7 s** ❌ | 9.2 s / 361.6 s / **371.8 s** ❌ | 608 MB | 3.7 GB |

Query p95 (20 runs), desktop / phone:

| nodes | search | neighbours | near |
|---:|---|---|---|
| 1k | 0.3 / 1.7 ms | 0.2 / 0.7 ms | 0.3 / 1.7 ms |
| 10k | 1.6 / 9.3 ms | 0.2 / 0.7 ms | 0.3 / 1.4 ms |
| 100k | 20.2 / 95.7 ms | 0.2 / 0.9 ms | 0.7 / 3.3 ms |
| 1M | 232 / 1064 ms | 1.2 / 5.1 ms | 4.6 / 23.7 ms |

**Where it stops being acceptable** (first usable query in under 2 s on the
phone proxy): **between 1k and 10k nodes** — 1k passes at 0.43 s, 10k fails at
2.6 s. On desktop the same budget holds to somewhere between 10k and 100k
(0.59 s at 10k, 5.8 s at 100k). The hard wall is 1M: the load alone takes 73 s
on desktop (over the 60 s stop rule), 362 s on the phone proxy, and wasm
memory reaches 3.7 GB, just under the 4 GB wasm32 ceiling — there is no
headroom left at all.

## Server: native engine, persistent data directory

The same sample loaded the same way (`zql_load_locations` +
`apply_zql_with_sources`) into an on-disk database, the way zega-server holds
a graph. Heap is exact (counting allocator, #128's method); on-disk size is
after a checkpoint; "reopen" is a restart opening the data directory.

| nodes | load | resting heap | RSS | peak RSS | on disk | reopen |
|---:|---:|---:|---:|---:|---:|---:|
| 1k | 0.24 s | 3.4 MB | 20 MB | 32 MB | 1.2 MB | 22 ms |
| 10k | 1.2 s | 36 MB | 156 MB | 241 MB | 12 MB | 233 ms |
| 100k | 16.1 s | 349 MB | 0.99 GB | 1.86 GB | 125 MB | 2.4 s |
| 1M | 219 s | 3.6 GB | 6.7 GB | 15.9 GB | 1.28 GB | 25.4 s |

Query p95 (20 runs; p50 in parentheses):

| nodes | search | neighbours | near |
|---:|---|---|---|
| 1k | 0.24 ms (0.14) | 0.05 ms (0.05) | 0.14 ms (0.08) |
| 10k | 1.4 ms (1.2) | 0.06 ms (0.05) | 0.10 ms (0.08) |
| 100k | 16.3 ms (15.4) | 0.07 ms (0.05) | 0.44 ms (0.26) |
| 1M | **201.9 ms** (161.6) | 0.12 ms (0.07) | 3.3 ms (2.2) |

**Where it stops being acceptable** (p95 under 50 ms): every query passes
through 100k. At 1M, only **search** crosses the line (202 ms) — it is an
unindexed full scan and costs ~0.2 µs per node; neighbours (0.12 ms) and
near (3.3 ms) are flat. That is an indexing gap, not an engine wall: with an
index on `Place.name` (or routing name search through one), the server path
holds at 1M too. Load peak memory at 1M is 2.4× resting (15.9 GB vs 6.7 GB
RSS), consistent with #128's ~2× finding, and the restart replays the 1.28 GB
graph file in 25 s.

## Recommendation

**zega.earth must move to server-side queries at 10k nodes — plan the move
at ~5k.** On the phone proxy the whole-graph load already blows the 2 s
first-usable budget at 10k (2.6 s), and the intake's next steps (a city,
then a country at 50–100k) land squarely in the 5.8–27 s range even on
desktop. 100k in the browser is 26.8 s-to-first-query on a phone and half a
gigabyte of wasm memory; 1M does not fit at all. The server path answers the
same queries at 100k in p95 ≤ 16 ms, so the browser loading only what it
shows is comfortable well past the country step.
