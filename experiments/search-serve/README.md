# Search serve experiment (phase 2)

Serves ranked results plus Search Transparency paths from the intake graph of
experiments/search-intake (the same ~279 pages; no new crawling). Node.js
standard library only; changes no engine code. Issue: zegadb/zega#134. Design:
[APS 29](https://github.com/zegadb/aps/issues/29) — relationships are shown,
scores never are; the web-ranking steps appear only as a vague
"ranked by relevance" step.

## Schema

`schema.zql` models the search graph: `Site`, `Entity` (Wikidata `qid` and P31
`kind`), and `Page`. `officialSiteOf` is path-aware: the relationship carries a
`&path_prefix` property from P856 (for example `www.nhl.com/oilers/`, not the
host `www.nhl.com`, which every NHL team shares). `mentions` links a page to
every entity whose label or alias appears in its title or text, `linksTo`
connects captured pages, and `memberOf` / `sport` hold the Wikidata league/team
relations (`relations.json`, fetched once from P118/P641 and committed).

## Ranking

All ranking lives in `lib/rank.mjs`; zega is only asked for graph facts over
ZQL. The whole model is the `WEIGHTS` table:

| weight | value | meaning |
|---|---|---|
| `bm25` | 1.0 | BM25 (k1=1.2, b=0.75) over title + body, title tokens ×3 |
| `officialSource` | 8.0 | page URL sits under the linked entity's official path prefix |
| `mention` | 3.0 | page has a MENTIONS edge to the linked entity |
| `proximity` | 2.0 | × 0.5^(hops−1), BFS hops from the entity over MENTIONS + LINKS_TO |

Query → entity linking matches labels and aliases case-insensitively at word
boundaries (either direction, so "leafs" finds "Maple Leafs"), plus derived
acronyms of multi-word labels ("nhl" finds "National Hockey League", whose
Wikidata alias list is empty in the intake data), and falls back to
whole-query Damerau-Levenshtein with distance ≤ 1 (≤ 2 for terms longer
than 8 characters). The score for a page is BM25 plus the best entity signal
across the linked entities. The intake spike's exact-entity-match boost
(+1000 when the query string equaled a label, PR #131) is gone;
`test/rank.test.mjs` asserts scores are invariant under query-string
decoration and that the weight table contains no boost key.

## Serve

```sh
node experiments/search-serve/serve.mjs --port 0
```

starts a local zega on an ephemeral port, loads the graph (JSON sources stay
under 2 MB each; the page chunk is ~0.4 MB), and serves
`GET /search?q=` on the printed loopback address:

```json
{
  "results": [{ "url": "…", "title": "…", "snippet": "…", "site": "…" }],
  "transparency": { "<url>": [ {"type": "entity", …}, {"edge": "mentions"}, {"type": "page", …}, {"step": "ranked by relevance"} ] }
}
```

Each transparency path is the real graph path from the query's linked entity
to the page, fetched from zega per query: a `mentions` edge, an
`official site <prefix>` hop (only when the page URL really sits under the
relationship's `&path_prefix`), or a `mentions` → `links to` chain found by
BFS over the fetched subgraph. A page with no graph relationship to the linked
entity shows only `{"step": "ranked by relevance"}`.

## Evaluate

```sh
node experiments/search-serve/eval.mjs
```

runs the 25 queries in `queries.json` (7 official-site, 10 informational,
8 typo/alias; each with its expected top-3 URL prefixes recorded) against a
freshly loaded graph and writes `eval-results.json` with precision@1 and @3
per class. Results are in the PR body.

## Test

```sh
node --test experiments/search-serve/test/*.test.mjs
```

covers entity linking (labels, aliases, typo tolerance), path-aware
official-source scoring, BM25 title weighting, the absence of the spike boost,
and transparency path extraction (including the shared-host trap: a page on
`www.nhl.com/flames/` must never get the Oilers' official-site path). Each
test was revert-proven: breaking the component it covers makes it fail.
