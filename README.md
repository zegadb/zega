# zega

The Zega developer CLI. It manages [Zega Cloud](https://zega.dev) from a terminal, a script or an agent: who you are, your projects, graphs, buckets, functions and usage, and the project file (`zega.json`) that later commands read.

Built for agents first. There is no browser login: a person makes an API token in the dashboard, the CLI stores it, and every command can run with no terminal at all.

The npm package is `zegadb`; the command is `zega`. (The Rust database binary is `zega-server`; the embeddable library is `@zegadb/lib`.)

## install

Not published yet. Build it:

```
bun install
bun run build        # dist/zega-darwin-arm64, -darwin-x64, -linux-x64, -windows-x64.exe
```

or run it from source: `bun run start -- <command>`.

## log in

Make an API token at https://dashboard.zega.dev/tokens (a `read` token reads everything; a `manage` token can also create, rename, delete and deploy). Then:

```
zega login                      # a hidden prompt on a terminal
echo "$TOKEN" | zega login      # or pipe it
zega login --token-file ./token # or read it from a file
```

The token is never an argument (arguments show in process listings) and never an environment variable. It is checked with `GET /v1/whoami` before it is saved, so a mistyped token never replaces a good one.

Where it is kept: by default in `cloud.json` in your config directory (`$XDG_CONFIG_HOME/zega`, default `~/.config/zega`; `%APPDATA%\zega` on Windows), mode 0600 in a 0700 directory, together with the API host it belongs to. A stored token is only ever sent to that host. `zega login --store keychain` keeps the token in the OS keychain instead (macOS Keychain, libsecret, Windows Credential Manager); the file then holds only the host.

For CI and agents, skip login: pass `--token-file <path>` to any command.

`--api <url>` picks the API (default `https://cloud.zega.dev`; staging `https://cloud.zega.world`). Plain http is accepted only for localhost.

`zega logout` removes the stored token. The token itself stays valid until you revoke it in the dashboard.

## commands

```
zega login [--store file|keychain]   store an API token
zega logout                          remove the stored token
zega whoami                          which token this is: name, scope, project, expiry
zega init                            write zega.json for a project here
zega start [args]                    run the zega-server that ships with this CLI (zega-server start)
zega explorer [graph]                the explorer for a local database; with a graph id, its console address
zega deploy [id]                     bundle and upload the functions zega.json lists
zega function deploy <id> [entry]    bundle and upload one function
zega projects [id]                   list projects, or show one with its graphs
zega graphs [id]                     list graphs, or show one
zega buckets [id]                    list storage buckets, or show one
zega functions [id]                  list functions, or show one
zega function logs <id>              a page of a function's requests and console lines
zega usage                           this month's usage and bill per graph
zega regions                         the regions a graph can run in
```

Global options: `--api <url>`, `--token-file <path>`, `--json`.

### deploy

`zega deploy` bundles each function `zega.json` lists into one ES module and uploads it (`PUT /v1/functions/{function}/code`, a manage token). `zega deploy <id>` deploys one; `zega function deploy <id> [entry]` takes the entry as an argument and needs no `zega.json`. `--minify` shrinks the bundle.

- The bundle is made with Bun.build (inside the compiled executable too): target `browser`, ES module, the conditions `workerd`, `worker` and `browser`, npm dependencies bundled from `node_modules`, `process.env.NODE_ENV` set to `"production"` (Bun's default is development), no source map (Zega Cloud takes one file and the runtime ignores an inline map; it would only count against the limit), `cloudflare:` imports left as they are.
- The size is printed on stderr before the upload. A bundle over the API's limit of 1,000,000 bytes is refused with its size, before anything is sent. All entries are bundled before the first upload, so a function that cannot be bundled stops the deploy while nothing has changed.
- **Node built-ins are not available in Zega Cloud functions yet** (no `nodejs_compat`). An import of `node:crypto`, `node:buffer`, a bare `path` and the rest, in your code or in a dependency, is an error naming the import and the file. (Bun's browser target would otherwise swap in a polyfill: `node:crypto` alone is about 925 KB.)
- With `--json` stdout is the API's answer unchanged, one per function; the size line is on stderr.

`zega function logs <id>` takes `--from`, `--to`, `--limit` (1 to 200), `--status`, `--search` and `--cursor`.

### start

`zega start [args]` runs `zega-server start [args]`, the database server that ships with this CLI: the arguments are passed through untouched (`zega start --data ./data --port 9342`, `--host 0.0.0.0 --token-file ./token`; `--help` is the server's own), stderr is the server's, and the exit code is the server's (128 plus the signal if it was killed by one). SIGTERM and SIGHUP sent to `zega` reach the server; so does SIGINT unless a terminal is attached, where Ctrl-C already reaches both. What is `zega`'s own, taken out of the arguments: `--explorer`, `--explorer-port <n>`, `--json` and `--server <path>`.

On a terminal it says what is running in a banner (the mark, a tagline, a rule, then a line each); without a terminal, or with `--json`, it prints one JSON line with the same facts:

```
  ➜  Local:     http://127.0.0.1:9342
  ➜  Explorer:  (stopped)
       zega start --explorer
```

```
{"server":{"url":"http://127.0.0.1:9342","status":"running"},"explorer":{"url":null,"status":"stopped"}}
```

`zega start --explorer` starts the server and serves the explorer (below) on the same database, on 9343 (`--explorer-port <n>`; 0 chooses a free one); both lines then have their addresses. If the server needs a token (`--token-file`), the explorer adds it to the requests it passes on: the page never has it.

Which server: the binary of the npm package `@zegadb/server-<os>-<cpu>` (`darwin-arm64`, `darwin-x64`, `linux-x64`, `win32-x64`; one file, `bin/zega-server`), which `zegadb` lists as optionalDependencies at the exact version of the zega release this CLI is tested against, as create-deka-app does for deka. It is looked for from where the CLI's code is, then from where the running executable is, in each one's `node_modules` and every `node_modules` above. **It is never the `zega-server` on `PATH`.** If the package for this machine is not installed, `zega start` exits 1 naming the package and the platform (reinstall `zegadb` without `--omit=optional`). `--server <path>` runs another build, for development.

### explorer

```
zega explorer [--port 9343] [--data ./zega-data]
zega explorer <graph>
```

`zega explorer` runs the bundled `zega-server start` on the data directory (on a port of its own, 127.0.0.1) and serves the explorer page on 127.0.0.1:9343, then prints the address and opens nothing (a browser is yours to open). Ctrl-C stops both. It exits 1 if the database cannot start (a data directory another process holds, say; the server's own words are printed) or the port is taken. `--json` prints `{ "url", "data", "server" }`.

The page is zegadb/zega's explorer in its CLI form: the same source as the zega.dev playground, built without the website's links, sample graphs and examples, so it opens on your own database and has nothing else in it. The bar says what it is connected to (`local server · zega-data`, or `cloud graph · <name>`) and has one button to switch between the two. An empty database says so and shows how to load data (Import, or a ZQL mutation); `clear` asks before it deletes anything. Writes go to the real engine in `zega-server`, so the database is persistent and the same one `zega start` serves.

**A cloud graph** is connected from the same page: **Connect to remote graph**, then the graph and its key (`zk_...`, made in the dashboard). If you are logged in the graphs of your account are offered by name. The page's requests go to `zega`, which passes them on to the graph's own address with the key the page sent; the cloud only ever sees a server, so its CORS rule does not apply, and `zega` keeps nothing (no key, no body, no log of either). Your API token is used only to list your graphs and find their addresses, and never goes to a graph.

`zega explorer <graph>` prints instead the address of that cloud graph's console in the dashboard (`https://dashboard.zega.dev/graphs/<id>/console`; the project-first layout in zegadb/dashboard#44 redirects it), after asking the API that the graph exists. `--json` prints `{ "url" }`. It needs a token (`zega login` or `--token-file`).

Only 127.0.0.1 is served. A page of another website must not be able to drive your database through your machine, so a request must carry this server's own `Host` (against DNS rebinding), and one that can change anything must have no `Origin` or its own.

The page is vendored in `src/explorer/` (built files and `inventory.json`: the zegadb/zega commit and the SHA-256 of each file; `bun test` fails if they disagree). It is embedded in the compiled executables. To update it: commit the change in a zegadb/zega checkout, then `bun run explorer:update ../zega` here (it builds `browser/` with `npm run build:cli`, refuses a dirty checkout, and rewrites `src/explorer/`). `zega explorer --assets <dir>` serves a build of the page from a directory instead (a `browser/dist-cli` you are working on), and `--server <path>` runs another zega-server.

Two things the page still takes from the network, as the playground does: the editor (Monaco, from cdn.jsdelivr.net) and map tiles.

### for agents and scripts

- `--json` prints the API's JSON unchanged (one document on stdout). Without it you get an aligned table.
- Every question the wizard asks is also a flag. With a terminal and a missing answer, `zega init` asks. With no terminal (an agent, CI, a pipe) or with `--json` it never asks: it fails at once, naming the missing flags.
- Every failure exits 1 with the API's own sentence and code on stderr (`zega: <sentence> (code: <code>, HTTP <status>)`), then what to do about it: how to log in on a 401, a manage token on `scope_required`, the API's own text on `session_required`, the wait time on 429 and 503 (the CLI never retries by itself), the host it could not reach on a network failure.
- What costs money, and the tokens themselves, are dashboard-only: the API answers 403 `session_required` and the CLI says so. `zega init` therefore creates nothing in the cloud.

### zega init

```
zega init                                   # asks for the name and the region
zega init --name my-app --region yyz        # no questions
zega init --name my-app --region yyz --force
```

writes `zega.json` in the current directory:

```json
{
  "version": 1,
  "name": "my-app",
  "region": "yyz"
}
```

`version` is the file's own format, `name` the project's display label (1 to 64 characters), `region` a Zega Cloud region id (`zega regions`). It holds no secret.

To deploy functions it lists them, each with its cloud id and the source file to bundle (relative to `zega.json`):

```json
{
  "version": 1,
  "name": "my-app",
  "region": "yyz",
  "functions": [{ "id": "f0abcdefghjkmnpqrs", "entry": "src/index.ts" }]
}
```

`zega init --function <id> [--entry <path>]` writes that (the entry defaults to `src/index.ts`; on a terminal init asks for both, and an empty function id means none). `init` never creates the function: that is done in the dashboard.

## develop

Bun 1.3.14 is pinned (`packageManager`, `engines`, CI).

```
bun install
bun run typecheck
bun test
bun run build
```

The code keeps the shape it was seeded with: `src/core/<command>/<command>.ts` is a command, `src/questions/<command>/<question>.ts` is one wizard question (a flag has the same answer; both pass the same validator), `src/handlers/` does the work, `src/utils/` is shared.

The API client is typed from the OpenAPI document: `openapi/openapi.json` is a saved copy of https://cloud.zega.dev/openapi.json and `src/types/cloud-api.d.ts` is generated from it with `openapi-typescript`. Both are committed; CI regenerates the types and fails if they differ. To follow a change in the API: `bun run api:update` (or `bun run api:update https://cloud.zega.world/openapi.json` for staging), then commit both files.

The tests run the CLI as a subprocess against a fake API on `127.0.0.1` that records every request, on a real terminal (a pty) for the wizard, and as the compiled executable.
