# Search intake spike (≤300 pages)

A bounded, standalone experiment for zegadb/zega#130. It uses Python's standard
library and changes no engine code.

## Run

From the repository root, with network access to Wikidata and Common Crawl and
the Workers AI proxy already listening on `127.0.0.1:8977`:

```sh
umask 022
mkdir -p .tmp
chmod 700 .tmp
export TMPDIR="$PWD/.tmp" CARGO_TARGET_DIR="$PWD/.target"
cargo build --locked -p zega-cli > .tmp/search-intake-build.log 2>&1
build_status=$?
cat .tmp/search-intake-build.log
if [ "$build_status" -ne 0 ]; then exit "$build_status"; fi
python3 experiments/search-intake/run.py --limit 300 > .tmp/search-intake-run.log 2>&1
run_status=$?
cat .tmp/search-intake-run.log
if [ "$run_status" -ne 0 ]; then exit "$run_status"; fi
```

The limit is validated to 1–300. The script queries the latest Common Crawl
collection's URL index for exact official-site hosts and requests individual
WARC byte ranges only. It never crawls sites. CDX lookups are spaced by at least
one second; record range fetches share the same limiter. It asks for at most 500
index rows per host, then selects up to 12 shallow URLs per host (and stops at
the global page limit). Responses and index metadata are cached under
`.tmp/search-intake/`.

The Wikimedia APIs resolve the fixed English seed labels to QIDs and retrieve
Wikidata labels, aliases, official websites (P856), entity types (P31), NHL
league membership (P118), and sport (P641) in one batched entity lookup. This
avoids WDQS because it was actively limited to one request per minute during
the spike. Page text is parsed with
Python's HTML parser, removing common boilerplate tags. Exact label/alias matches
are applied first; zero or multiple matches go through the local Workers AI
proxy at `http://127.0.0.1:8977/run/@cf/meta/llama-3.1-8b-instruct-fp8`.

The script starts the checked-out CLI on an ephemeral loopback port, writes
Entity and Page nodes via ZQL JSON loads in ≤1.8 MB chunks, then creates
`RELATED_TO`, `ABOUT`, and captured-page `LINKS_TO` edges. It runs ten fixed
queries, ranks by term frequency plus incoming captured graph links, evaluates
whether an official-domain page linked to the queried entity ranks in the top
three, and records three query → entity → related entity → page paths as JSON.
Each transparency path is executed as ZQL against the loaded graph before it is
reported. Load memory is the zega server process RSS sampled after load (`ps`,
KiB), not a peak allocator measurement.

`results.json` is the machine-readable report. It includes seeds found/missing,
CDX hits, fetched and parsed pages, linking method counts, per-stage timings,
zega load time and memory, graph counts, referee score, search results,
transparency paths, and observed failures. Page and entity input JSON plus the
schema and bounded load chunks are retained in this folder to make the graph
load reviewable. Raw Common Crawl records remain in `.tmp`. When the report run
reuses cached CDX/WARC data, direct request timings are zero; the report also
records the first-to-last cache file modification span as an acquisition-window
observation, not as summed network latency.
