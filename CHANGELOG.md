# Changelog

Changes that affect code using the `zega` crate, newest first. File formats
(WAL, snapshot, `.graph`) have their own versions; see
[docs/graph-format.md](docs/graph-format.md).

## Unreleased

- **`zega cloud`: a command line for Zega Cloud's public management API**
  (`https://cloud.zega.dev`). `login`, `logout` and `whoami` for an API token
  made in the dashboard; `projects`, `graphs`, `buckets`, `functions`,
  `usage`, `regions` and `function logs` to read (a table, or `--json` for the
  API's own JSON); `function deploy`, `function var set|unset` and
  `function secret set|unset` to change functions. Global flags `--api` and
  `--token-file`. No effect on the `zega` crate. README, "Manage Zega Cloud".

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
