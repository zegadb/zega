# Linked graphs: wire contract v1 (shared by the engine lane and the mailbox Worker lane)

IDs: `zega://<graph>/<id>`. earth ids are Wikidata QIDs (for example Q2096). Versions are u64 from the APS 24 write log, and increase per node.

## Diff (JSON; compression is applied by the transport)
{ "source": "earth", "graph_version": 1234,
  "changes": [ { "id": "Q2096", "version": 42, "kind": "node", "op": "upsert",
                 "fields": { "name": "Edmonton" },
                 "rels": { "add": [ { "type": "locatedIn", "to": "Q1951" } ], "remove": [] } },
               { "id": "Q9999", "version": 7, "kind": "node", "op": "delete" } ] }
- Changed fields only. The subscriber applies a change only if its version is newer than the mirror's (idempotent).

## Source endpoints (zega-server)
- POST /sync/subscribe  { subscriber, endpoint?, ids:[...], lease_secs } → { mailbox_token, graph_version }
- POST /sync/check      { items: [[id, version], ...] } (≤1000) → { stale: [id, ...] }
- GET  /sync/node/:id?hops=1 → node + one-hop stubs (used by link)
- The subscriber's push receiver: POST <endpoint>/zega/sync/push  body = Diff  → 204

## Mailbox Worker (Cloudflare Worker + KV, binding MAILBOX)
- Key: `mb:<subscriber>:<graph_version zero-padded to 20 digits>`, so keys sort oldest-first. Value: Diff. expirationTtl = 30 days.
- PUT    /mb/:subscriber/:graph_version   auth: `Authorization: Bearer <SOURCE_SECRET>` (the source only)
- GET    /mb/:subscriber?cursor=&limit=100 → { items: [{ key, diff }], cursor }   auth: mailbox token
- DELETE /mb/:subscriber/:graph_version   auth: mailbox token
- mailbox token = base64url(HMAC-SHA256(SOURCE_SECRET, subscriber)). The Worker verifies it; the source mints it on /sync/subscribe.
- Secrets live only in Worker secrets and the source's environment. Never in git.
