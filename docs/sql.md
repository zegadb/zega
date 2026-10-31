# SQL beside the graph, and deliveries

APS 54 (SQL and the graph in one database) and APS 52 (deliveries).

## SQLite storage

```sh
zega-server start --data ./zega-data
```

SQLite is the default: a new data directory keeps its log as rows of `zega.db` (Turso's Rust SQLite). `--storage file` creates one with the older `wal.bin` log instead. A directory keeps the storage it was created with: later starts, `export` and `import` need no flag, and asking for the other storage is refused, so existing `wal.bin` databases keep working as they are. In Rust the crate's default is still the file log; ask for SQLite with `Zega::open(path).storage(Storage::Sqlite)` (feature `sqlite`).

To back up a SQLite-storage database, copy **`zega.db` and `graphs/`** together. The log's first entry names a checkpoint file in `graphs/`.

## SQL

Your own tables live in the same `zega.db`, and commit together with graph writes.

```sh
curl -X POST localhost:9342/sql -H 'Content-Type: application/json' \
  -d '{"sql": "CREATE TABLE orders (id INTEGER PRIMARY KEY, paid INTEGER)"}'

curl -X POST localhost:9342/transaction -H 'Content-Type: application/json' -d '{"steps": [
  {"sql": "INSERT INTO orders (id, paid) VALUES (?1, 1)", "params": [7]},
  {"schema": "type Order { id: Int paid: Bool }", "zql": "mutation { Order(id: 7 && paid: true) { id } }"}
]}'
```

- **`POST /sql`** `{sql, params}` runs one statement in its own transaction. It returns `{"ok": true, "result": {columns, rows, changes}}`. `changes` counts INSERT, UPDATE and DELETE rows, and is 0 for anything else.
- **`POST /transaction`** `{steps: [...]}` runs SQL steps `{sql, params}` and ZQL steps `{schema?, zql}` all-or-nothing, in one commit. Later steps see earlier ones. A failure names its `step`, and nothing from any step is kept.
- **Statements allowed:** SELECT, INSERT, UPDATE, DELETE, CREATE/DROP TABLE and INDEX, and ALTER TABLE, one at a time. Triggers, views, PRAGMA, ATTACH, VACUUM and BEGIN/COMMIT are refused; use `/transaction` for several statements.
- **`zega_*` names are Zega's.** A statement other than SELECT that mentions an identifier starting with `zega_` is refused.
- **Limits:** a result has at most 100 000 rows or 64 MiB (add a `LIMIT`). The server's `--query-time-limit` applies.
- **Values:** `null`, numbers, strings, `true`/`false` (stored as 1/0). Integers beyond ±2^53 are written `{"int": "…"}`, non-finite reals `{"real": "Infinity"}`, and blobs `{"base64": "…"}`, both in requests and in results.
- **Rust:** `zega.sql(sql, &params)` and `zega.transaction(|tx| { tx.sql(..)?; tx.zql(schema, zql)?; tx.document(src) })`. `transaction` with only ZQL steps works on either storage.

| code | status | meaning |
|---|---|---|
| `constraint` | 409 | a UNIQUE, CHECK, NOT NULL or FOREIGN KEY constraint, checked at the statement or at commit |
| `reserved_table` | 400 | the statement writes, or names, a `zega_*` table |
| `sql_error` | 400 | not valid here, or failed while running |
| `too_large` | 400 | the result passed the limit |
| `query_time_limit` | 400 | ran past `--query-time-limit` |
| `sql_unavailable` | 422 | the database uses file storage |

## Deliveries

A source (for example pg_zega's journal) sends each change once per identity:

```sh
curl -X POST localhost:9342/deliveries -H 'Content-Type: application/json' -d '{
  "source_id": "pg:7362810001:16384", "txid": "981",
  "document": "schema { type P { name: String } }\nmutation { P(name: \"a\") { name } }"}'
```

- The document is a `.zql` document (a schema, then its blocks), applied as one transaction on either storage.
- An optional `"schema"` is stored as the graph's schema first, in the same transaction (as `PUT /schema`, refused the same way when the data cannot satisfy it). If the document then fails, the old schema stays.
- An optional `"flush"` is deleted before that: `{"all": true}` for every node and relationship, or `{"labels": [...], "relationships": [...]}` for the nodes with those labels (with their relationships) and the relationships of those types. Receipts are kept. pg_zega's `zega.import` in journal mode sends one, so a type can be emptied, given a schema its old nodes could not satisfy, and loaded again as one delivery.
- Its receipt `{source_id, txid, payload_sha256, result}` is written in the **same** log entry as the document's writes, so the two are durable together or not at all.
- Receipts are graph state: they are replayed, carried by checkpoints and exports (the `.graph` file's `RCPT` section), and copied wherever the log goes.
- `"forget_earlier": true` says the source will never send an earlier delivery again: Zega keeps this delivery's receipt and drops the source's others, in the same log entry. Without it, receipts are kept forever. pg_zega's worker sends it on every delivery, so a graph keeps one receipt per database. It is not part of `payload_sha256`.

| answer | meaning | the source should |
|---|---|---|
| 200 | applied now, or already applied with this document (the original receipt) | move on |
| 409 `delivery_conflict` | this identity was delivered with a different flush, schema or document; nothing written | stop and alert |
| 422 `invalid_identity` | `source_id`/`txid` must be 1–128 bytes of `A-Z a-z 0-9 - _ . :` | fix the sender |
| 400 | the engine refused the document; nothing written | fix the document |
| 503 `outcome_unknown` | the write may or may not be on disk | retry the identical request |

In Rust: `zega.deliver(source_id, txid, &Payload { flush, schema, document, forget_earlier })` (`Payload::document(document)` for a document alone; `zega::receipts::{Payload, Flush}`) and `zega.receipt(source_id, txid)`.
