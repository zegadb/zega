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

The limit is validated to 1–300. The script reads the official website (P856)
for each Wikidata seed and queries Common Crawl by URL prefix. Legacy NHL
subdomains are normalized to their current team paths on `www.nhl.com`; for
example, `http://oilers.nhl.com` maps to `www.nhl.com/oilers/`, queried as
`url=www.nhl.com/oilers/*&matchType=prefix`. If the index rejects or times out
on the trailing-star form, it retries the slash-terminated prefix with the same
`matchType=prefix`. Each prefix contributes at most eight WARC records. The
script requests individual byte ranges only and never follows page links to
fetch more pages. CDX and WARC data are cached under `.tmp/search-intake/`.

The Wikimedia APIs resolve the fixed English seed labels to QIDs and retrieve
Wikidata labels, aliases, official websites (P856), entity types (P31), NHL
league membership (P118), and sport (P641) in one batched entity lookup. Page
responses are decoded from their WARC gzip member, embedded HTTP headers,
transfer framing, content encoding, and declared charset before HTML parsing.
Official-site path ownership is assigned to the longest matching P856 prefix;
exact label/alias matches and the local Workers AI proxy handle other pages.

The script starts the checked-out CLI on an ephemeral loopback port, writes
Entity and Page nodes via ZQL JSON loads in ≤1.8 MB chunks, then creates
`RELATED_TO`, `ABOUT`, and captured-page `LINKS_TO` edges. It runs ten fixed
queries. The spike ranker combines term frequency and incoming links with an
explicit entity match boost for exact entity-name queries. The referee checks
whether a page from the entity's official path is linked to that entity and
appears in the top three. Each of three transparency paths is executed as ZQL
against the loaded graph before being reported. Load memory is the zega server
process RSS sampled after load (`ps`, KiB), not a peak allocator measurement.

## Round 2 results

- Seeds: 38/38 resolved; 37 official-site URL prefixes queried. The index returned 5,110 candidate rows. A cap of eight records per prefix produced 279 WARC range records: 32 NHL team paths × 8 (256), six NHL root pages, eight PWHL, eight AHL, one IIHF, and zero for Hockey Canada. Thirty-six prefixes returned pages; Hockey Canada returned 404/no captures. `records_by_site_prefix` in `results.json` gives the complete per-prefix count.
- Parsing: 279/279 records parsed (100%), compared with 18/23 (78.3%) in round 1. The five earlier skips were caused by stripping the WARC/HTTP separator twice: the parser received the body without its HTTP headers, then split the HTML body as if it contained another header block. Gzip decompression succeeded; content encoding, chunked transfer, and charset were not the cause. The corrected parser preserves HTTP headers and handles those encodings and framing.
- Linking: 155 exact label/alias, 116 official-site-prefix, eight Workers AI decisions, zero unlinked. The final audit replay reused all 279 cached classification decisions.
- Referee: 7/7 (100%) in round 2 versus 0/7 in round 1. The round 1 misses were acquisition/path selection for all six clubs and ranking for the NHL root page: three P856 values still named legacy team subdomains that returned 404; three current `www.nhl.com/<team>/` URLs were lost in the broad host query's shallow-page selection, and generic root pages were sometimes linked to the wrong entity; the NHL root was captured at rank 6. Round 2 maps pages to the longest official path and adds an explicit entity-name ranking boost, which put every referee page at rank 1. This measures the spike's own attribution and scoring rules, not the production engine. The engine would need path-aware official-source associations and entity-aware query ranking, then a benchmark against representative queries without this spike-specific boost.
- Transparency paths, all executed and verified in the loaded graph:
  - Query “Edmonton Oilers” → Oilers entity → `https://www.nhl.com/oilers/info/oilersplus-faq`
  - Query “Calgary Flames” → Flames entity → `https://www.nhl.com/flames/fans/club-red`
  - Query “Toronto Maple Leafs” → Maple Leafs entity → `https://www.nhl.com/mapleleafs/roster`
- Measurement pass: 114.724 s total (Wikidata 1.401 s; Common Crawl 98.051 s, including 50.679 s CDX and 17.221 s range fetch; parse/extract 3.282 s; classification 0.636 s; Zega load 14.220 s; search 0.320 s). That pass downloaded 32 records and reused 247 WARC cache records; earlier iterations had already populated much of the cache. The final artifact refresh took 42.350 s and reused all 279 records, so its direct CDX and range-fetch timings were zero.
- What broke: the wildcard prefix form intermittently returned 400/404/502/504; slash-terminated prefix fallback recovered the team and PWHL sites. The PWHL 504 succeeded on one retry. Hockey Canada remained unavailable. Two early round-2 report attempts hit site-count bookkeeping errors after capture; both were corrected before the final report. One unrelated NHL homepage variant included `ref=blog.mathspace.co` tracking text and appeared in results. Two initial round-2 report passes also exposed site-count bookkeeping errors after capture; those were fixed before the final report.

`results.json` is the machine-readable report. It includes the per-prefix counts,
parse rate, timing details, referee causes/ranks, all ten result sets, transparency
paths, and failures. `entities.json`, `pages.json`, `schema.zql`, and bounded load
chunks make the graph input reviewable. Every JSON load is below 2 MB; the largest
is `load-001.json` at about 1.04 MB. Raw Common Crawl records remain in `.tmp`.
