# Linked graphs (APS 39)

This implements [APS 39](https://github.com/zegadb/aps/pull/40) and the storage extension authorized in [zega#138](https://github.com/zegadb/zega/issues/138). The engine and mailbox Worker share [wire contract v1](linked-graphs-contract.md).

A reference is `zega://graph/id`. A ZQL `Reference` property validates that spelling and uses the existing string encoding in the WAL and `.graph` format. Construct references in Rust with `zega::linked::Reference`. Reference values never perform network I/O when evaluated.

```zql
schema {
  type Bookmark { target: Reference }
}

mutation {
  Bookmark(target: "zega://earth/Z2096") { target }
}
```

## Identity and durability

A node's string `id` property is its graph-wide external identity; earth allocates ZIDs (`Z` + a number, never reused or renumbered), and source identifiers such as Wikidata QIDs are kept as external-id properties on the entity, not as identity. The engine rejects duplicate identities and attempts to change them. Nodes without a string `id` receive an opaque generated identity, persisted with the graph. Applications that reload independent datasets must supply explicit stable IDs.

External identity is independent of the engine's numeric node slot. A sync host activates source tracking with `Zega::prepare_linked`; fetching a sync node or registering a subscription activates it too. Ordinary, unlinked import/export keeps its existing byte-for-byte behavior. Source `.graph` reloads reconcile facts by external ID, retain leases and history, and advance versions instead of adopting the incoming file's versions. A source reload cannot change the type of an existing external ID. Deleted IDs retain tombstone versions. Replacing a graph containing mirrors is rejected; restore it by reopening its data directory, or import its exported bundle into an empty database.

Each acknowledged mutating statement appends its fact operations and derived change record in one CRC-protected WAL statement. `graph_version` advances per statement; each affected node's version advances independently. The record contains changed fields and relationships, not a second copy of the graph. Failed statements publish neither versions nor history. `Zega::changes_since(V)` reads these retained records using a version cursor.

APS 24 HIST remains the valid-time store for temporal fields and relationships. As authorized in #138, linked-graph write history extends the existing WAL/checkpoint path; it does not reinterpret valid time as a write sequence. The optional `SYNC` checkpoint section stores identities, versions, retained records, mirror provenance, and engine-owned subscription tables. Imports use an atomic `ReplaceLinkedGraph` WAL record to keep source versions monotonic across reloads. Checkpoints retain at least 30 days of records; expired leases are removed there. Older engines reject the new records/section rather than dropping them.

## Source, subscriber, and host configuration

The headless `zega-server <config.json>` host binds to loopback for use behind a gateway. The existing `zega start` host remains available for other server uses. Configure a source as follows; paths and URLs are examples:

```json
{
  "data": "./earth-data",
  "listen": "127.0.0.1:9342",
  "linked": { "graph": "earth", "mailbox": "https://mailbox.example.invalid" }
}
```

The deployment supplies `SOURCE_SECRET` to the source host and Worker. It is not part of the config file or graph. Source subscriptions mint base64url HMAC-SHA256 mailbox tokens exactly as the shared contract specifies. Push requests carry the same subscriber token as bearer authentication. Tokens are kept in host memory and renewed with subscriptions on wake; they are not written to `.graph` exports or logs.

A subscriber configuration identifies the source and its own reachable push endpoint:

```json
{
  "data": "./fan-data",
  "listen": "127.0.0.1:9343",
  "linked": {
    "graph": "fan",
    "subscriber": "fan",
    "endpoint": "http://127.0.0.1:9343",
    "mailbox": "https://mailbox.example.invalid",
    "sources": { "earth": "http://127.0.0.1:9342" }
  }
}
```

The mailbox URL is optional. A mailbox subscriber identity has one source: the shared key format has no source component, so separate sources require separate mailbox identities. These endpoints cover public source graphs; private-source authentication is outside APS 39. The host does not follow redirects or accept URL credentials in configured URLs.

- `POST /sync/link` with `{"reference":"zega://earth/Z2096"}` explicitly fetches `GET /sync/node/Z2096?hops=1`, installs the snapshot, and leases its subscription. The engine API is `Zega::link(reference, snapshot)`; the server owns HTTP.
- `GET /sync/node/:id?hops=1` returns `NodeSnapshot`: `id`, `version`, `labels`, `fields`, optional native value `types` (preserving points/vectors instead of degrading them to maps/lists), outgoing `rels`, target `stubs`, and `source_gone`. Stubs carry identity and labels, not recursively fetched facts.
- `POST /sync/subscribe`, `POST /sync/check`, and `POST /zega/sync/push` use the shared contract unchanged. Check and subscription batches are limited to 1,000 IDs. Leases are 1 second through 30 days.
- `POST /sync/repair` explicitly wakes the subscriber: renew, drain, check versions, and refetch stale IDs. Maintenance repeats daily, independently of queries.

Local mirrors carry source, external ID, version, stub status, source-gone status, and the IDs of source-owned edges. `Zega::mirrors(source)` and `GET /sync/mirrors?source=earth` return this local provenance without network I/O. Mirror facts and source edges reject ordinary mutation and deletion. Local edges remain writable. A source deletion marks the mirror source-gone and retains it and its local edges.

Live delivery wakes from a constant-work post-commit notification. The worker reads committed history outside the write path, coalesces five-second windows, and takes shared lease snapshots. It iterates subscribers after releasing the engine lock. Each subscriber receives only its referenced nodes; duplicate fields collapse to their final values and relationship additions/removals collapse to final membership. Push JSON is gzip-compressed; the Worker receives contract JSON. A push has a 300 ms timeout. Failure goes once to the configured mailbox, without an outbox or retries; a lost/expired event is repaired by the version check.

On wake, mailbox pages are applied oldest-first, durably, before deletion. Live pushes receive 503 until wake completes. Duplicate/older node versions are ignored. A gap in per-node versions triggers a fresh one-hop snapshot before applying partial fields, including when coalescing spans several edits. This prevents an out-of-order partial diff from silently hiding an earlier missed field. Newly added relationships with unknown targets likewise fetch one-hop stub metadata. Compound field updates refetch native value shape information because plain JSON cannot distinguish a numeric list from a vector. If the source is unavailable at startup, local queries still open; live sync stays gated until an explicit wake or daily maintenance succeeds. Because tokens are renewed from the source, mailbox recovery after a process restart requires source availability; offline queries do not.

CDN change segments and P2P are outside this implementation. Source corrections and private-source access control remain outside APS 39.

## Reproducible proof

```sh
export TMPDIR="$PWD/.tmp" CARGO_TARGET_DIR="$PWD/.target"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
cargo test --locked -p zega --test linked_graphs
cargo test --locked -p zega-server --test linked_proof -- --test-threads=1 --nocapture
python3 scripts/prove-linked-graphs.py
```

The harness launches separate EARTH and FAN OS child processes hosting the production server routes and sync runtime. Its child entry point injects a public test fixture key; it never reads or sets deployment secrets. The fixture reuses the 38 landed hockey entities in `experiments/search-intake/entities.json` and adds 12 hand-written hockey cities. Every entity gets a ZID (`Z1`–`Z50`, allocated by earth in fixture order) as its identity, with its Wikidata QID kept as a `wikidata` external-id property. FAN links 20 nodes and adds its own edge. The fake Worker enforces the contract's source and mailbox authentication, sorted keys, pagination, idempotent delete, and 30-day expiry.

Scenarios a–g are individual automated tests. For b–e, the harness counts the actual bytes read in both directions through TCP proxies between source, subscriber, and mailbox. These are HTTP stream bytes, including HTTP headers and compressed bodies, excluding TCP/IP framing and local administrative writes/assertion queries. The c assertion forbids snapshot refetch during recovery, so a broken mailbox replay cannot pass through the repair path. The e assertion checks one HTTP diff, one node change, and only the final changed field.

Scenario g compares two separate EARTH processes with identical one-node graphs, zero versus 10,000 persisted active leases, and the real asynchronous worker running. It times the engine call inside the server process, including the server mutex, query execution, WAL durability, and contention with sync. It alternates paired 50-write blocks, discards the warmup block, and compares medians of block means (500 measured writes per condition). Registration is fixture setup outside timing. No delivery endpoint/mailbox is configured for these leases, isolating subscription-registry overhead. HTTP round-trip latency is deliberately excluded from this engine measurement.

The revert script temporarily disables push application, mailbox application, and coalescing one at a time, asserts the corresponding test fails, and restores the exact original files in `finally`. It does not rewrite Git history. Logs stay in `.tmp/revert-{b,c,e}.log`.

Needs Ava: deploy the mailbox Worker + set SOURCE_SECRET
