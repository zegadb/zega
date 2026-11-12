#!/usr/bin/env bash
# Bulk loading at millions of nodes (zegadb/zega#127, #128).
#
#   scripts/bulk-bench.sh <nodes> <relationships> <work-dir> [bin-dir]
#
# Generates the graph (`zega-bulk gen`), then for each storage (sqlite, file)
# loads it with `zega-server import graph.jsonl` and reopens the data
# directory in a fresh process (`zega-bulk reopen`), whose RSS after one query
# is the graph's resting size. Also imports the same graph as a `.graph` file,
# and, with ZQL=1, loads it the old way: one ZQL statement per 2 MB chunk
# (`zega-bulk zql`). Wall time and peak RSS come from the OS (`/usr/bin/time`),
# never a profiler. bin-dir holds zega-server and zega-bulk (default: the
# release build). Everything is written under work-dir; delete it after.
set -euo pipefail
nodes="$1"
rels="$2"
work="$3"
root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${4:-}"
if [ -z "$bin" ]; then
  cargo build --locked --release -p zega-cli -p zega-bench --manifest-path "$root/Cargo.toml" >&2
  bin="${CARGO_TARGET_DIR:-$root/target}/release"
fi
mkdir -p "$work"
data="$work/data"

# `measure <label> <command…>`: one line, label, wall seconds, peak RSS MiB.
measure() {
  local label="$1"
  shift
  local log="$work/$label.time"
  if [ "$(uname)" = Darwin ]; then
    /usr/bin/time -l "$@" > "$work/$label.out" 2> "$log"
    local wall rss
    wall=$(awk '/ real /{print $1}' "$log")
    rss=$(awk '/maximum resident set size/{printf "%.0f", $1/1048576}' "$log")
  else
    /usr/bin/time -v "$@" > "$work/$label.out" 2> "$log"
    wall=$(awk -F': ' '/Elapsed \(wall clock\)/{split($2,t,":"); s=0; for(i=1;i<=length(t);i++) s=s*60+t[i]; print s}' "$log")
    rss=$(awk -F': ' '/Maximum resident set size/{printf "%.0f", $2/1024}' "$log")
  fi
  printf '%-28s wall %8.1f s   peak RSS %7s MiB\n' "$label" "$wall" "$rss"
}

[ -f "$data/graph.jsonl" ] || "$bin/zega-bulk" gen "$nodes" "$rels" "$data" >&2
for storage in sqlite file; do
  rm -rf "$work/db-$storage"
  measure "jsonl-import-$storage" "$bin/zega-server" import "$data/graph.jsonl" \
    --data "$work/db-$storage" --schema "$data/schema.zql" --storage "$storage"
  measure "jsonl-reopen-$storage" "$bin/zega-bulk" reopen --data "$work/db-$storage"
  grep -o '"resting_rss_bytes":[0-9]*' "$work/jsonl-reopen-$storage.out" | awk -F: '{printf "%-28s resting RSS %.0f MiB\n", "  after reopen", $2/1048576}'
done
"$bin/zega-server" export "$work/graph.graph" --data "$work/db-sqlite"
rm -rf "$work/db-graph"
measure "graph-import-sqlite" "$bin/zega-server" import "$work/graph.graph" --data "$work/db-graph"
measure "graph-reopen-sqlite" "$bin/zega-bulk" reopen --data "$work/db-graph"
if [ "${ZQL:-0}" = 1 ]; then
  rm -rf "$work/db-zql"
  measure "zql-2mb-chunks-sqlite" "$bin/zega-bulk" zql "$data" --data "$work/db-zql" --storage sqlite
  measure "zql-reopen-sqlite" "$bin/zega-bulk" reopen --data "$work/db-zql"
fi
