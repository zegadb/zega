# Changelog

Changes that affect code using the `zega` crate, newest first. File formats
(WAL, snapshot, `.graph`) have their own versions; see
[docs/graph-format.md](docs/graph-format.md).

## Unreleased

- **`zega-server explorer` is removed.** The explorer is part of the `zega` command
  (zegadb/cli: `zega explorer`, `zega start --explorer`), which bundles this server
  and serves the page in front of `zega-server start`. This executable no longer embeds the
  explorer's files, so it is smaller and `zega-cli` has no build script. The page's
  source stays in `browser/`; `npm run build:cli` builds the page for that command.
  The data directory is held by `zega-server start`, as before.
- **The executable is renamed `zega` -> `zega-server`, and the npm library
  `zegadb` -> `@zegadb/lib`.** The `zega` command is going to a new developer
  command line (its own repository), so two programs would otherwise share the
  name. Same flags and subcommands (`zega-server start|explorer|export|import|
  fmt|bundle|schema-diff|cloud`); release artifacts are `zega-server-<platform>`
  (was `zega-<platform>`); the Fly image runs `/usr/local/bin/zega-server`. The
  headless linked-graph host that was also called `zega-server` is now
  `zega-linked-host`. The embeddable library is `@zegadb/lib` on npm (the
  `zegadb` placeholder was never a release); the executable ships as
  `@zegadb/server-<os>-<cpu>` packages. `zega cloud` stays here for now as
  `zega-server cloud`; it moves to the new command line later. No effect on the
  `zega` crate.

- **`zega-server cloud`: a command line for Zega Cloud's public management API**
  (`https://cloud.zega.dev`). `login`, `logout` and `whoami` for an API token
  made in the dashboard; `projects`, `graphs`, `buckets`, `functions`,
  `usage`, `regions` and `function logs` to read (a table, or `--json` for the
  API's own JSON); `function deploy`, `function var set|unset` and
  `function secret set|unset` to change functions. Global flags `--api` and
  `--token-file`. No effect on the `zega` crate. README, "Manage Zega Cloud".

- **`zega-server cloud` manages: every `manage` route of the API is a command.**
  `project`, `graph`, `bucket` and `function` take `rename` and `delete`;
  `bucket create` and `function create` make one in a project; `graph key`
  and `bucket key` (`list`, `create`, `revoke`), `graph domain` (`list`, `add`,
  `remove`), `graph monitoring` (`show`, `set --keep-query-text on|off`),
  `function logging <id> on|off` and `function code <id> [--out <file>]`.
  A delete shows what it deletes and needs the id typed at a terminal, or
  `--yes`; without either it refuses before sending anything. A created key's
  secret is printed once. Money controls and tokens stay dashboard-only. No
  effect on the `zega` crate. README, "Manage Zega Cloud".

- **Read-only entry points: `Zega::run_lang_read` and `Zega::apply_zql_read`,
  and `ZegaError::NotARead`.** They run what `run_lang` and `apply_zql` run
  but refuse a `mutation` or a load (anywhere in a document) before anything
  executes: no write, no WAL entry. `zega-server` serves them as the HTTP
  `QUERY /zql` method (RFC 10008; README, "`QUERY /zql`"). `ZegaError` gained a
  variant, so an exhaustive `match` on it needs an arm for `NotARead`.

- **`zega::Value` payloads are boxed** (zegadb/zega#100). A stored property
  is 24 bytes instead of 56, so the variants that set the size moved behind
  a pointer:

  | variant | was | now |
  |---|---|---|
  | `Value::String` | `String` | `Box<str>` |
  | `Value::List` | `Vec<Value>` | `Box<[Value]>` |
  | `Value::Map` | `HashMap<String, Value>` | `Box<HashMap<String, Value>>` |
  | `Value::Vector` | `Vector` | `Box<Vector>` |

  Every variant, its discriminant and its meaning are unchanged, and
  `Box<T>` serializes exactly as `T`, so WAL, snapshot and `.graph` bytes
  are the same and files written before open as they did. Code that builds
  values should prefer `Value::from(..)` (for `&str`, `String`, `i64`,
  `bool`) or `.into()` (`Value::List(vec![..].into())`); code that matches
  on `Value::String(s)` gets a `&Box<str>`, which derefs to `&str`.
- **Only `unique` fields are indexed by value** (zegadb/zega#100). The graph
  used to index every property of every node by value; now each
  `unique { Type { field } }` of the running schema has its own index,
  built from the stored nodes when a statement first declares it and kept
  current by every write. Results are unchanged.
