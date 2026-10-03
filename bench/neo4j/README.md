# Benchmark: zega vs Neo4j

Idle RAM and throughput, head to head, on the same generated graph. The
numbers live in [RESULTS.md](RESULTS.md); raw CSVs in
[results/](results/); the method, machine and caveats are written down
there because these numbers get quoted in public.

## What's here

- `src/` — one Rust client (`bench`) with five subcommands:
  - `gen <nodes> <dir>` — the deterministic graph generator (splitmix64
    seed; persons `u%07d`, products `s%06d`, symmetric `KNOWS` friendships,
    `PURCHASED` edges; ~4.5 relationships per node). Emits CSV shards, the
    zega `schema.zql`, `params.json` (the seeded parameter list both
    engines replay) and `meta.json`.
  - `load-zega --dataset <dir> --data <data-dir>` — in-process load into a
    zega data directory (the same calls `zega-scale` makes), checkpointed.
  - `load-neo4j --dataset <dir> --url <host:port> --password <pw> [--clear]`
    — unique constraints (the enforced-index match for zega's `unique`),
    then 10k-row `UNWIND` batches over Bolt, indexes awaited ONLINE.
  - `parity --zega-url … --neo4j-url …` — runs every query kind on both
    engines over the same sampled params and diffs the canonical answers.
    Nothing is timed until this passes 6/6.
  - `run --engine zega|neo4j|neo4j-http --query … --conc N --seconds S` —
    one measured cell; prints a `RESULT` CSV line (qps + p50/p95/p99,
    client-observed).
- `run.sh` — the whole matrix (idle RAM empty/loaded, loads, parity,
  throughput at concurrency 1/8/32, default + tuned Neo4j, Docker + native).
- `results/` — raw CSVs + `machine.json`.

## The queries

Same questions, each engine's own language, equivalent semantics (verified
by `parity` before any timing):

| query | zega (ZQL) | Neo4j (Cypher) |
|---|---|---|
| lookup | `Person(id: …)` (unique index) | `MATCH (p:Person {id:$id})` |
| onehop | `knows -> Person` | `MATCH …-[:KNOWS]->(f)` |
| twohop | `knows *2..2 -> Person` (shortest-distance band) | `[:KNOWS*2]` minus 1-hop nodes and self |
| filtered | 2-hop walk to indexed anchor + `has purchases(category = …) limit 50` | same, `LIMIT 50` |
| path | `knows *path -> Person(id: …)` (BFS) | `shortestPath((a)-[:KNOWS*]-(b))` |
| write | one-node `mutation` | one-node `CREATE` |

## Rerun

```sh
# from the repo root, with the workspace built:
cargo build --release -p zega-cli          # provides .target/release/zega-server
(cd bench/neo4j && cargo build --release)  # provides the client
docker pull neo4j:5.26-community

# smoke it at a tiny size first (minutes), then the real thing (hours):
(cd bench/neo4j && CELL_SECONDS=3 ./run.sh 2000)
(cd bench/neo4j && ./run.sh 10000 100000 1000000)
```

Native (non-Docker) Neo4j runs need a JRE 21 (`JAVA_HOME` or Homebrew
`openjdk@21`); without one they are skipped. `SKIP_NATIVE=1` / `SKIP_TUNED=1`
trim the matrix. Everything listens on 127.0.0.1; the only containers
touched are `kimi-neo4j-bench*`.
