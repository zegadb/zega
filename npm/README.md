# @zegadb/lib

An embeddable graph database with ZQL v2, and a small client for a running
zega. One package contains the JavaScript API, TypeScript declarations and
WebAssembly engine, plus [`@zegadb/lib/client`](#talking-to-a-running-zega),
which is `fetch` only and never loads the engine. It has no runtime npm
dependencies or install scripts. Requires an ES module environment and, on the
server, Node 22.14 or later.

```sh
npm install @zegadb/lib
```

```js
import { createDatabase } from '@zegadb/lib';

const db = await createDatabase();
try {
  const result = db.run(
    'type Person { name: String }',
    'mutation { Person(name: "Ada") { name } }',
  );
  console.log(JSON.parse(result)); // { name: 'Ada' }
} finally {
  db.free();
}
```

The same code works in Node and Vite. Initialization is asynchronous and shared;
each call creates an independent database. Queries are synchronous. In CommonJS,
use `await import('@zegadb/lib')`. For large workloads use a Web Worker or worker thread.

## Exports

- `@zegadb/lib`: `createDatabase(options?)`, `ZegaWasm`, and the `DatabaseOptions` and
  `InitInput` types. `ZegaWasm` is the generated class; initialize via
  `createDatabase()` before constructing it directly.
- `@zegadb/lib/client`: `connect()`, `zql` and the `ZegaError` classes; see
  [Talking to a running zega](#talking-to-a-running-zega). No wasm, no
  dependencies, no Node built-ins.
- `@zegadb/lib/wasm`: the unmodified wasm-bindgen API and declarations, including
  default async `init` and `initSync`, for callers managing initialization.
- `@zegadb/lib/zega_wasm_bg.wasm`: the binary asset for bundler loaders or custom hosting.

`db.run(schema, source)` executes ZQL v2 against a schema string.
`db.apply(source)` executes a complete `.zql` file containing schema, mutation
and query blocks. `db.check(schema, source)` returns JSON diagnostics.
Results from these methods are JSON strings; use `JSON.parse`.
The generated types also cover graph inspection, node/relationship deletion,
connections and base64 snapshot import/export. Errors from the bindings can be strings, so catch `unknown`.

The database is in memory in both environments. Persist explicitly using
`export_base64()` and `import_base64()`. Call `free()` when finished and do not
use a freed handle. Snapshot restore replaces the receiving database's state.

## Bundlers and WASM assets

The conditional exports select the browser loader for browser builds and the
filesystem loader for Node. Both load the same binary. The browser loader uses
`new URL('./wasm/zega_wasm_bg.wasm', import.meta.url)`, which Vite and Webpack
can emit as an asset. Serve it as `application/wasm`. Vite needs no WASM plugin.
Generated glue is explicitly retained in `sideEffects`; do not override that
metadata with blanket tree-shaking settings.

With Next (including Turbopack), call `createDatabase()` in a client component's effect or event
handler, not during rendering. A server import selects the Node loader. For a
bundler that does not emit `new URL` assets (including esbuild), use its asset
loader or copy the exported binary to your public assets:

```js
// esbuild: loader: { '.wasm': 'file' }, publicPath matching your asset server
import wasmURL from '@zegadb/lib/zega_wasm_bg.wasm';
import { createDatabase } from '@zegadb/lib';
const db = await createDatabase({ wasm: wasmURL });
```

`options.wasm` accepts a URL/string, Response, bytes, compiled WebAssembly.Module
or a promise of one. It controls the first initialization only; later calls use
the already loaded module. Node's default reads the packaged bytes directly
because Node fetch cannot read `file:` URLs. Importing the package itself does
not initialize WASM. This supports custom asset hosting and SSR imports.

## Talking to a running zega

`@zegadb/lib/client` calls a zega that is already running: a local
`zega-server start`, or a Zega Cloud graph. It is `fetch` only, with no
dependencies, no wasm and no Node built-ins, so it works unchanged in Workers
(a Zega Cloud function), Node 22+, Bun, Deno and browsers, and adds about 4 KB
(2 KB gzipped) to a bundle. Importing it never loads the engine.

In a Zega Cloud function, the graph's address and key arrive as variables
(`ZEGA_GRAPH_URL`, and the `zk_` key as `ZEGA_GRAPH_KEY`):

```ts
import { connect } from '@zegadb/lib/client'

export default {
  async fetch(req, env) {
    const zega = connect({ url: env.ZEGA_GRAPH_URL, key: env.ZEGA_GRAPH_KEY })
    const people = await zega.query<{ name: string }[]>('{ Person { name } }')
    return Response.json(people)
  },
}
```

The same in Node or Bun, against a local server (`zega-server start --token-file ./token`;
leave `key` out for one started without a token):

```js
import { connect, zql } from '@zegadb/lib/client'

const zega = connect({
  url: 'http://127.0.0.1:9342',
  key: process.env.ZEGA_GRAPH_KEY,
})
await zega.setSchema('type Person { name: String }')
await zega.mutate(zql`mutation { Person(name: ${'Ada'}) { name } }`)
console.log(await zega.query('{ Person { name } }')) // [ { name: 'Ada' } ]
```

`zega dev --local-graph` runs a function with `ZEGA_GRAPH_URL` pointing at a
local `zega-server` (default `http://127.0.0.1:9342`), and `--cloud-graph <id>`
at a cloud graph, so the function above runs unchanged against either.

### Calls

- `connect({ url, key?, schema?, fetch?, readMethod? })` sends nothing until the first call.
  `url` is a server's address or a graph's (`https://<graph>.zegadb.com`, or its
  own domain); `key` goes out as `Authorization: Bearer <key>`; `fetch` is your own
  (a service binding, a test double).
- `zega.query<T>(zql, options?)` runs a read and returns its result. `zega.mutate<T>(zql, options?)`
  runs a statement that writes (it takes any statement). `T` is what you say you
  will get; nothing checks the data against it.
- `options` is `{ schema?, document?, signal? }`: a schema for this call only, `document: true`
  to send a whole `.zql` file (its own `schema { }` block, then `query { }` and `mutation { }`
  blocks), and an `AbortSignal` (`AbortSignal.timeout(2000)` is a timeout).
- `zega.setSchema(schema)` stores the schema on the graph (`PUT /schema`). Do this when creating
  or changing the graph's schema, not on every request.
- `zega.schema()` reads the stored schema (`GET /schema`) when the application needs it.

**The schema.** A graph keeps its schema across restarts, so ordinary queries do not
need to carry it and `connect()` does not fetch it. Set it once with `setSchema()`;
`schema` on `connect()` or an individual call remains available for one-off and
older-server use. `Auth` is built in on every graph.

**Reads and writes.** The server tells them apart by method. `query()` sends the HTTP
`QUERY` method ([RFC 10008](https://www.rfc-editor.org/rfc/rfc10008.html)): safe,
and a server refuses anything that writes (`not_a_read`, nothing ran), so a read-only
function cannot be talked into a write. `mutate()` sends `POST`. A target that does
not know `QUERY` (405 or 501, or an older Zega Cloud router's 404) gets the read as
`POST` instead, once, and the client remembers; that read runs whatever it is given,
so for a target you know is old, say `readMethod: 'post'`, and `'query'` never falls back.

**Values.** ZQL has no query parameters, so there is nowhere to pass `vars`. Build a
statement with the `zql` tag, which writes each `${value}` as an escaped literal (a
string, finite number, bigint, boolean or `null`; anything else throws), never by joining
strings:

```ts
await zega.query(zql`{ Person(name: ${userInput}) { name } }`)
```

### Errors

Every error the client raises is a `ZegaError`, and none carries the key.

```ts
import { ZegaError, ZegaNetworkError } from '@zegadb/lib/client'

try {
  await zega.query('{ Person { nam } }')
} catch (error) {
  if (error instanceof ZegaNetworkError) {
    // no answer: error.code is 'network', 'aborted' or 'timeout'; error.cause is the original
  } else if (error instanceof ZegaError) {
    error.code     // 'query'
    error.status   // 422
    error.help     // 'did you mean `name`?'
    error.location // { source: 'query', line: 1, column: 12 }
    error.message  // the engine's whole text, with the source line and carets
  }
}
```

`code` is one of `config` (`connect()` refused its options), `network`, `aborted`, `timeout`,
`unauthorized` (401), `query` (the engine refused the ZQL: 400 or 422), `not_a_read`
(a write sent to `query()`), `rate_limited` (429; `retryAfter` is the seconds to wait), `server` (5xx),
`http` (any other refusal) and `bad_response` (a 2xx that is not a zega answer, so the url is probably
not a graph). `status` is the HTTP status, 0 when there was no answer. `serverCode` is the server's own
`code` when it sent one: `query_time_limit`, `invalid_key`, `missing_key`, `spending_cap_reached`,
`graph_not_ready`. The client never retries; `retryAfter` and the code tell you whether to.

## Scope

`@zegadb/lib` is the engine and, as a separate entry point, the client. Importing the
package itself does not import the client, and importing the client does not import the
engine, so a remote-only user downloads no wasm into a bundle (the wasm stays in the
tarball and is never reached). Not here yet: types generated from a schema, connection
pooling, subscriptions, and the `/graph` import and export routes. The repository's own explorer
remains in `browser/` and builds from Rust directly.

Source and issues: [zegadb/zega](https://github.com/zegadb/zega).
