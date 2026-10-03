# HANDOFF: a graph keeps its own schema (+ built-in `Auth`)

Branch `claude/stored-schema` (from `origin/main` 7e868cc). Stopped on a budget
stop before any code was written: this file is the design and the plan. Delete
it in the PR that finishes the work.

## Goal

`zega.query('{ Person { name } }')` with no schema. Today every `POST`/`QUERY
/zql` must carry `schema`, or the engine fails with "schema has no types"
(`zega/src/lang/mod.rs`, `parse_schema_at`). Client: zegadb/zega#157, branch
`claude/lib-client` (`@zegadb/lib/client`).

## What already exists (read these first)

- `Graph.carried: graph_file::Carried { schema: Option<String>, uniques, indexes, meta }`
  (`zega/src/graph/mod.rs`, `zega/src/graph_file/mod.rs`). It is already
  persisted in the snapshot encoding (`wal/mod.rs` ~1081/1229), in checkpoints
  (they are `.graph` files) and in exports/imports (`.graph` Schema section).
  **Use it as the stored schema**; no new file format is needed.
- What it lacks: a WAL entry that sets it (today only an import or `clear()`
  changes it), so a set must survive a restart before the next checkpoint.
- `Zega::schema_diff(old, new)` / `schema_diff::diff_schemas` classify type
  changes against real data (`Blocks` when data would be lost). Ignores
  `unique`/`index`/`display`.
- Server routes: `zega-server/src/routes.rs`; handlers in `handlers.rs`.
  `/schema/diff` already exists. The CLI (`zega-cli`, binary `zega-server`)
  mounts the same router; its tests are `zega-cli/tests/cli.rs`.

## API chosen

- `PUT /schema` body `{"schema": "<text>"}` (or `Content-Type: application/zql`
  raw text). Explicit, idempotent, separately metered. Not "a schema block in
  a `.zql` apply sets it": existing callers send a schema with every document,
  and making each apply rewrite the stored schema would be an implicit change.
- `GET /schema` -> `{"ok":true,"result":{"schema":"<full text incl. Auth>","builtin":["Auth"]}}`.
- `POST /zql` / `QUERY /zql` with `schema` empty or absent -> the stored
  schema. `QUERY` with `application/zql` raw text and no `schema {}` block ->
  the stored schema too (parse with `parse_statement`, not `parse_zql`).
- Both present: the request schema must match the stored one (same types by
  `diff_schemas` with zero changes, same uniques and indexes), else 400/422
  `{"code":"schema_mismatch"}` naming the first difference. **Decision to
  confirm:** this applies only once a schema was set with `PUT /schema`. A
  graph whose `carried.schema` came only from an import keeps today's
  behaviour (request schema wins) so the explorer, which sends its editor's
  schema on every call (`browser/backend.js` `execute`), does not break. Track
  "set explicitly" in `Carried` (bincode/snapshot field appended last) and in
  the `.graph` manifest meta (e.g. `zega.schema = "stored"`), or simply apply
  the rule to any graph with a carried schema and update the explorer to
  `GET /schema`. Recommend the first.
- Refusals for `PUT /schema`: parse errors (rendered like today); any
  `Severity::Blocks` from `diff_schemas(stored_or_empty, new)`; and, new,
  labels present in the graph that the new schema has no type for (a first
  `PUT` diffs against an empty schema, so the diff alone would allow hiding
  data). Nothing is ever dropped; the graph is untouched on refusal.
- Meter: `PUT /schema` = write class, `GET /schema` = read class.

## Built-in `Auth` (APS 48, zegadb/aps#48, section "In the project's graph")

APS 48 now fixes the type (provisional until APS 48 is committed):

```zql
type Auth {
  uid: String            // "a_" + 18 random characters; the token "sub"
  email?: String         // lower case; absent for anonymous users
  emailVerified: Bool
  isAnonymous: Bool
  createdAt: Int         // epoch seconds
  name?: String
  avatarUrl?: String
}
unique { Auth { uid email } }
```

- One constant in the engine (e.g. `zega/src/builtin.rs`: `AUTH_SCHEMA`), the
  only place that text lives. No secrets in it (APS 48 keeps credentials,
  sessions, tokens in the Auth D1 store, never in the graph).
- Every stored schema = builtin + developer text. A new graph's `GET /schema`
  returns the Auth type alone; a query without schema on a new graph then says
  "unknown type Person", not "schema has no types".
- **Reserved** (APS 48): developer text that declares `type Auth` or lists
  `Auth` in `unique` is refused, code `reserved_type`. The developer may
  `extend type Auth { posts: MEMBER <- Post[] }` with relationships only, no
  fields (the one parser addition APS 48 asks for). `Post.author -> Auth`
  works without declaring Auth.
- Note: APS 48 says "the gate adds the Auth block ... so the engine does not
  change". With the schema stored in the engine, composing it in the engine
  (one constant) is simpler and keeps `GET /schema` honest; raise this on
  aps#48 when the PR opens.
- Per-request schemas (today's form) may also use `Auth` undeclared (APS 48),
  so compose the builtin into those too; a request text that already has no
  reference to Auth behaves exactly as today.

### How to compose (the tricky part)

The parser rejects `-> Auth` when `Auth` is not declared, so text
concatenation before parsing breaks error positions. Plan:

1. In `lang/mod.rs`, give `parse_schema_at` an optional list of built-in
   `TypeDef`s (parsed once from `AUTH_SCHEMA`) that it adds before the
   relationship-target check, and teach it `extend type Auth { ... }` (edges
   only; a prop field is an error). Errors keep the developer's spans.
2. Refuse `type Auth` / `unique { Auth ... }` in developer text (`reserved_type`).
3. Store the developer text as written in `carried.schema`; build the
   effective schema (types + builtin, uniques + builtin uniques) at use time.
   `carried.uniques`/`indexes` hold the effective declarations so exports and
   `sync_indexes` see `Auth.uid`/`Auth.email`.
4. Exports: `GET /schema` and `.graph` exports should carry text another
   engine can parse; emit the composed text (builtin block first, marked
   `// built in`) or keep developer text and rely on step 1 everywhere. Pick
   one and test import-then-export byte equality (`graph_file/tests.rs`).

## Engine work, in order

1. `wal::Operation::SetSchema { source: String, uniques, indexes }` appended
   **last** (bincode discriminants are wire ids). Replay sets `carried`.
2. `Zega::set_schema(text) -> Result<SchemaDiffReport>`: compose, diff
   against stored, orphan-label check, then one WAL append + `set_carried`
   under the graph lock; `sync_indexes`/`sync_uniques` to the new declarations.
3. `Zega::stored_schema() -> Result<String>` (effective text).
4. `run_lang*`/`apply_zql*`: empty schema -> stored; both -> mismatch rule.
   Same for `vector_view` and `connect_schema` (both take a schema today).
5. `clear()` (DELETE /graph) currently drops `carried`. Decide: keep the
   stored schema (Supabase-like) and drop only data. Recommend keep.

## Tests owed (AGENTS.md section 3: revert-prove the persistence test)

- engine: set, reopen (WAL replay), checkpoint + reopen, export/import carry
  it; incompatible change refused with data intact; first PUT that hides a
  labelled node refused; new graph has Auth; `type Auth` refused; changing
  Auth refused; `Post.author -> Auth` works; `extend type Auth` edges only.
- server (`zega-server/tests/server.rs`): query without schema after PUT,
  both forms (POST JSON, QUERY JSON, QUERY application/zql), mismatch error,
  GET /schema includes Auth.
- client (#157 branch `claude/lib-client`, merge commit, no force): `schema`
  optional in `connect()`/`query()`/`mutate()`, `client.schema()`,
  `client.setSchema(text)`, README example without schema, a real-server test
  with no schema.
- zegadb/cloud: check the router passes `GET`/`PUT /g/:id/schema` to the
  machine; if not, a separate small PR there (open, do not merge). Meter PUT
  as write, GET as read.
- Gates: `cargo check --locked --workspace --all-targets`, `cargo clippy
  --locked --workspace --all-targets --all-features` (no new warnings),
  `cargo test -p zega -p zega-server -p zega-cli`, npm checks, the testsuite
  if CI runs it.

## Status

- Done: reading and design (this file). No code yet.
- Next concrete step: engine step 1 (`Operation::SetSchema` + replay) with its
  restart test, then step 2.
