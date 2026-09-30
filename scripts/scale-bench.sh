#!/usr/bin/env bash
# The zegadb/zega#143 scale bench, both paths, one command:
#
#   scripts/scale-bench.sh [size ...]      default: 1000 10000 100000 1000000
#
# Generates the travel-shaped samples under .tmp/scale-143/<size>/
# (gitignored), then runs:
#   server  — the native engine loading each sample into a persistent data
#             directory (zega-scale server): load time, heap and RSS, on-disk
#             size, restart time, query p50/p95/p99 → server-results.jsonl
#   browser — the wasm engine in headless Chromium loading the same sample
#             exactly as zega.earth does, desktop and 4× CPU throttled
#             (scripts/scale-browser.mjs) → browser-results.json
# Existing generated samples are reused; delete .tmp/scale-143/<size> to
# regenerate. SCALE_MACHINE overrides the machine label in the output.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/.tmp/scale-143"
bin="${CARGO_TARGET_DIR:-$root/target}/release/zega-scale"
sizes=("$@")
[ ${#sizes[@]} -eq 0 ] && sizes=(1000 10000 100000 1000000)

cargo build --locked --release -p zega-bench --bin zega-scale --manifest-path "$root/Cargo.toml"
mkdir -p "$out"
for n in "${sizes[@]}"; do
  if [ ! -f "$out/$n/meta.json" ]; then
    echo "gen $n" >&2
    "$bin" gen "$n" "$out/$n" > /dev/null
  fi
done

: > "$out/server-results.jsonl"
for n in "${sizes[@]}"; do
  data="$out/server-data/$n"
  rm -rf "$data"
  echo "server $n" >&2
  "$bin" server "$out/$n" --data "$data" >> "$out/server-results.jsonl"
done

export SCALE_MACHINE="${SCALE_MACHINE:-$(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m)}"
node "$root/scripts/scale-browser.mjs" "$out" "$out/browser-results.json" "${sizes[@]}"
echo "server:  $out/server-results.jsonl" >&2
echo "browser: $out/browser-results.json" >&2
