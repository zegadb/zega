# Scale results: wasm (zega.earth) and server paths, 1k–1M travel nodes

zegadb/zega#143. Measured 2026-09-30 with `scripts/scale-bench.sh` (see
`zega-bench/README.md`; raw JSON under `.tmp/scale-143/`, gitignored), on the
fleet iMac: **iMac20,2, Intel Core i9-10910 (10C/20T, 3.60 GHz), 128 GB RAM,
macOS 26.7, x86_64**. Engine at 07a47f4 (`origin/main`).

The browser path was run against **two wasm builds, side by side**:

- **earth — the build zega.earth actually serves today**: zegadb/earth@312c3f6
  `vendor/explorer/browser/pkg/` (`zega_wasm_bg.wasm` git blob `0beb6a0`,
  sha256 `e20647dd149ee99cb5c3113783083f122dcf1b8bd8ae1a518aeae418f97ca032`;
  engine ef0982c-era — the blob appears in this repo's history between
  d03388f and dc8200b). Benched from a separate copy via
  `scripts/scale-browser.mjs --pkg`; the repo's `browser/pkg` was not touched.
- **main — the repo's committed `browser/pkg`**: git blob `8aefe13`, sha256
  `82b0d00b45552bf51ec4588faab598f5d77b9a4c274bfef589857fd97f3a6715`,
  vendored in 61217bb from engine cef8edd8 — 61 engine commits ahead of
  earth's build, including APS 24 typed field history and temporal queries.

An earlier revision of this document claimed the bench ran "the same bytes
zega.earth serves today"; that was wrong — it ran only the **main** build.
The verdict below is stated on the **earth** build.

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
page-open to the first query's result, **including the wasm download and
instantiation** (the "engine" column is that page-open → engine-ready part
on its own). Phone = CDP CPU throttling 4×.

### earth — the build zega.earth serves (blob `0beb6a0`)

| nodes | gz download | desktop: engine / download / load / first usable | phone 4×: engine / download / load / first usable | JS heap | wasm memory |
|---:|---:|---|---|---:|---:|
| 1k | 0.07 MB | 42 / 21 / 68 / **137 ms** | 103 / 67 / 261 / **451 ms** | 2 MB | 6.6 MB |
| 10k | 0.72 MB | 29 / 37 / 411 / **484 ms** | 105 / 146 / 1.8 s / **2.1 s** | 11 MB | 55 MB |
| 100k | 7.0 MB | 30 / 246 / 4.4 s / **4.7 s** | 108 / 855 / 19.2 s / **20.2 s** | 62 MB | 376 MB |
| 1M | 72.2 MB | 30 / 2.2 s / 50.1 s / **52.6 s** | 102 / 8.7 s / 222.6 s / **232.4 s** ❌ | 608 MB | 1.6 GB |

Query p95 (20 runs), desktop / phone:

| nodes | search | neighbours | near |
|---:|---|---|---|
| 1k | 0.3 / 1.7 ms | 0.1 / 0.7 ms | 0.3 / 1.4 ms |
| 10k | 1.8 / 9.8 ms | 0.1 / 0.8 ms | 0.4 / 1.6 ms |
| 100k | 21.3 / 93.4 ms | 0.2 / 0.7 ms | 0.8 / 3.7 ms |
| 1M | 210 / 1011 ms | 0.2 / 0.9 ms | 3.0 / 23.3 ms |

### main — committed `browser/pkg` (blob `8aefe13`, engine cef8edd8)

| nodes | gz download | desktop: engine / download / load / first usable | phone 4×: engine / download / load / first usable | JS heap | wasm memory |
|---:|---:|---|---|---:|---:|
| 1k | 0.07 MB | 65 / 29 / 87 / **190 ms** | 158 / 95 / 508 / **814 ms** | 2 MB | 9.1 MB |
| 10k | 0.72 MB | 45 / 49 / 789 / **892 ms** | 156 / 223 / 3.2 s / **3.7 s** | 11 MB | 85 MB |
| 100k | 7.0 MB | 45 / 309 / 7.4 s / **7.8 s** | 155 / 1.2 s / 37.4 s / **39.0 s** | 62 MB | 589 MB |
| 1M | 72.2 MB | 46 / 3.7 s / 89.4 s / **93.4 s** ❌ | 125 / 9.9 s / 346.6 s / **357.7 s** ❌ | 608 MB | 3.6 GB |

Query p95 (20 runs), desktop / phone:

| nodes | search | neighbours | near |
|---:|---|---|---|
| 1k | 0.4 / 13.9 ms | 0.2 / 11.7 ms | 0.3 / 4.4 ms |
| 10k | 2.8 / 12.6 ms | 0.2 / 15.1 ms | 0.4 / 1.6 ms |
| 100k | 28.4 / 214.5 ms | 0.2 / 9.8 ms | 0.8 / 12.5 ms |
| 1M | 237 / 989 ms | 1.2 / 5.1 ms | 5.1 / 18.4 ms |

(main's 1k/10k phone p95s carry visible machine noise; the first-usable and
load columns are the verdict-relevant ones.)

**Where it stops being acceptable** (first usable query in under 2 s on the
phone proxy), on the build zega.earth serves: **between 1k and 10k nodes** —
1k passes at 0.45 s, 10k fails at 2.1 s. On desktop the same budget holds to
somewhere between 10k and 100k (0.48 s at 10k, 4.7 s at 100k). The verdict
stands on earth's build.

The engine gap shows in the ceiling, not the verdict. Earth's build loads 1M
on desktop in 50 s (under the 60 s stop rule) at 1.6 GB of wasm memory —
unusable, but with headroom. Main's build needs 89 s for the same load
(❌ over the stop rule) and reaches 3.6–3.7 GB, just under the 4 GB wasm32
ceiling: the 61 engine commits earth has not yet deployed roughly double the
memory and load time at 1M, so the hard memory wall arrives with the next
engine deploy to zega.earth even though it is not what today's visitors hit.

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
at ~5k.** On the phone proxy, the build zega.earth serves today already blows
the 2 s first-usable budget at 10k (2.1 s), and the intake's next steps (a
city, then a country at 50–100k) land squarely in the 4.7–20 s
first-usable range even on desktop. 100k in the browser is 20 s-to-first-query
on a phone and ~380 MB of wasm memory on today's build (39 s and ~590 MB on
the engine main now has); 1M is unusable on either build and stops fitting at
all once main's engine ships. The server path answers the same queries at
100k in p95 ≤ 16 ms, so the browser loading only what it shows is comfortable
well past the country step.
