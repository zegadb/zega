# zega browser

A four-pane workbench for the v2 schema language. The engine is compiled to
WebAssembly (`zega-wasm`), so there is no server to start. The graph lives
in the page and persists across reloads via `localStorage`.

Schema is the top-left pane. A read or a `mutation` is the top-right pane.
The bottom-left pane is the force-directed graph of everything stored. The
bottom-right pane is the JSON that came back.

## Run it

The prebuilt wasm package is committed, so this works with any static file
server:

```bash
cd browser
python3 -m http.server 8080
# open http://localhost:8080
```

Run executes the query pane. Ctrl/Cmd+Enter does the same. Seed library
writes the Le Guin example. Clear drops the saved database and reloads.
Writes are saved to `localStorage`.

The graph view's physics is [d3-force](https://github.com/d3/d3-force)
v3.0.0 (ISC license, © Observable), vendored as ESM in `vendor/` so the page
stays fully self-hosted — no CDN, no bundler.

## Rebuild the wasm package

Only needed after changing `zega-wasm` (or the engine crates):

```bash
cargo install wasm-pack   # once
cd zega-wasm
wasm-pack build --target web --out-dir ../browser/pkg
```

`zega-wasm` is its own workspace (see its `Cargo.toml`) so the wasm build
never tries to compile the server crates for `wasm32`.

## Remote graphs

**Connect to remote graph**, next to the `local · wasm` indicator, opens a Zega
Cloud graph by id and API key (`zk_…`). The explorer then uses the same HTTP
backend as the native CLI mode (`backend.js`), pointed at
`https://api.zega.dev/g/<id>` with `Authorization: Bearer <key>`; the header
shows `connected to <id>`, and **Disconnect** returns to the local graph.

- The key is held in memory only, in the `RemoteDatabase`'s private field:
  never in localStorage, sessionStorage, IndexedDB, a cookie, the URL or a log.
  A reload or Disconnect forgets it. Requests omit credentials.
- The cloud router allows exactly `https://explorer.zega.dev` (CORS, no
  credentials), so this works from the deployed explorer, not from `wrangler
  dev` or the CLI; the CLI's native mode shows no button.
- ZQL sources are fetched by the browser and sent with the query, as in wasm
  mode: the graph's machine cannot read this computer's files.
- Every call to a remote graph is metered, so nothing runs on typing: queries
  run only on Run or Cmd/Ctrl+Enter, connecting reads the graph once, and the
  `GET /graph` snapshot is refreshed only after something that may write.
- zega-server keeps no schema: every query carries one. Zega Cloud stores the
  text last pushed (`GET|PUT /g/<id>/schema`, zegadb/cloud#15). Connecting
  opens the schema pane on it (the local panes wait in memory and come back
  on Disconnect); a graph with none says so. **Push schema** stores the
  schema pane, and applies a ZQL document's mutation blocks, only after a
  confirmation that names the graph. Typed `mutation { … }` in the query pane
  run on Run, and the view shows the new nodes.
- Run never applies the schema pane to a remote graph, and sample buttons and
  reset are hidden while connected (they clear the graph first); clear asks
  before deleting anything remote.
- `tests/remote.spec.js` runs this against a local HTTPS stand-in for
  api.zega.dev (Chromium's resolver maps the host; `openssl` makes the
  certificate) and inspects every storage API, the URL and the DOM for the key
  after connect, Disconnect and reload.

The explorer sends no Content-Security-Policy today, so nothing blocks the
connection; the same spec fails if a policy is added without api.zega.dev in
`connect-src`.

## Native mode and the `zega` command

The page has three modes, and `/explorer-config.json` says which. No such file: the
standalone site, a wasm/localStorage database in the browser. `{ "backend": "native" }`:
the page talks to a database's own HTTP routes (`/zql`, `/graph`, ...) on its own
origin. `{ "backend": "native", "ui": "cli", ... }`: the `zega` command's page (the
[zegadb/cli](https://github.com/zegadb/cli) package, `zega explorer` and
`zega start --explorer`), which serves it in front of a `zega-server start` it runs.
The explorer subcommand of `zega-server` itself is gone: this binary is the database only.

`npm run build:cli` makes `dist-cli/`, the page for that command: the same source as the
site without its links, sample buttons, examples and demo components (the elements marked
`data-site-only` in `index.html` are removed, and the build fails if a link to the site is
left). It opens on the user's own database or a cloud graph, with an empty state, and
reaches a cloud graph through the command (`remote.base`: its requests are passed on, so the
cloud router's CORS rule does not apply). zegadb/cli vendors it with `bun run
explorer:update`. `tests/cli-page.spec.js` runs it.

After `cargo build --locked -p zega-cli` from the workspace with
`CARGO_TARGET_DIR=.target`, run `npm run test:cli` here for the real Chromium
native-backend import/reload/restart test. `cli-tests/harness.mjs` stands in for the
command's serving: it starts `zega-server start` and serves `dist/` in front of it. `assets.json` is the shared static
bundle inventory used by both `npm run build` and the CLI's build script; a
normal Cargo build needs no Node installation. The build validates the vendored
wasm hashes before embedding the bundle.
