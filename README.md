# zega

The Zega developer CLI. It manages [Zega Cloud](https://zega.dev) from a terminal, a script or an agent: who you are, your projects, graphs, buckets, functions and usage, and the project file (`zega.json`) that later commands read.

**zega prints JSON; add `--human` for people.** Every command prints JSON on stdout (one value per result, one line per event for the ones that keep running), asks nothing, and prints no colour, logo or table, on a terminal too. `--human` gives the readable form: tables, the logo and banners, colour, readable errors and questions. The help is JSON as well. What each command prints is under [output](#output).

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
echo "$TOKEN" | zega login      # pipe it
zega login --token-file ./token # or read it from a file
zega login --human              # or, on a terminal, a hidden prompt
```

Without `--human` and without one of the first two, `zega login` fails at once and says how to give the token. It then says who you are, and nothing about the token beyond its name and what it may do (never any part of the secret, and not where it is kept). This is `--human`; by default it is the API's answer as JSON (`{ "account", "token" }`):

```
$ zega login --human
Logged in as Ada Lovelace (@ada)

Token    ci-bot
Scope    manage
Project  all projects
Expires  Dec 1, 2026

$ zega whoami --human
Logged in as Ada Lovelace (@ada)

API      https://cloud.zega.dev
Token    ci-bot
Scope    manage
Project  all projects
Expires  never
```

An account with no full name shows as `Logged in as @ada`. The JSON is the API's answer (it also has when the token was made and last used), without the token's `prefix`: no part of a token is ever shown.

The token is never an argument (arguments show in process listings) and never an environment variable. It is checked with `GET /v1/whoami` before it is saved, so a mistyped token never replaces a good one.

Where it is kept: by default in `cloud.json` in your config directory (`$XDG_CONFIG_HOME/zega`, default `~/.config/zega`; `%APPDATA%\zega` on Windows), mode 0600 in a 0700 directory, together with the API host it belongs to. A stored token is only ever sent to that host. `zega login --store keychain` keeps the token in the OS keychain instead (macOS Keychain, libsecret, Windows Credential Manager); the file then holds only the host.

For CI and agents, skip login: pass `--token-file <path>` to any command.

`--api <url>` picks the API (default `https://cloud.zega.dev`; staging `https://cloud.zega.world`). Plain http is accepted only for localhost.

`zega logout` removes the stored token. The token itself stays valid until you revoke it in the dashboard.

## commands

```
zega login [--store file|keychain]   store an API token
zega logout                          remove the stored token
zega whoami                          who you are, and the token: name, scope, project, expiry
zega init                            write zega.json for a project here
zega start [args]                    run the zega-server that ships with this CLI (zega-server start)
zega explorer [graph]                the explorer for a local database; with a graph id, its console address

zega build [id]                      bundle the functions zega.json lists into dist/<id>.js (no upload, no token)
zega dev [id]                        serve them on localhost with Bun, rebuilt when a file changes
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

Global options: `--api <url>`, `--token-file <path>`, `--human`.

### deploy

`zega deploy` bundles each function `zega.json` lists into one ES module and uploads it (`PUT /v1/functions/{function}/code`, a manage token). `zega deploy <id>` deploys one; `zega function deploy <id> [entry]` takes the entry as an argument and needs no `zega.json`. `--minify` shrinks the bundle.

- The bundle is made with Bun.build (inside the compiled executable too): target `browser`, ES module, the conditions `workerd`, `worker` and `browser`, npm dependencies bundled from `node_modules`, `process.env.NODE_ENV` set to `"production"` (Bun's default is development), no source map (Zega Cloud takes one file and the runtime ignores an inline map; it would only count against the limit), `cloudflare:` imports left as they are.
- The size is printed on stderr before the upload. A bundle over the API's limit of 1,000,000 bytes is refused with its size, before anything is sent. All entries are bundled before the first upload, so a function that cannot be bundled stops the deploy while nothing has changed.
- **Node built-ins are not available in Zega Cloud functions yet** (no `nodejs_compat`). An import of `node:crypto`, `node:buffer`, a bare `path` and the rest, in your code or in a dependency, is an error naming the import and the file. (Bun's browser target would otherwise swap in a polyfill: `node:crypto` alone is about 925 KB.)
- Stdout is the API's answer unchanged, one JSON line per function. With `--human` it is a readable line each, and the size of each bundle is on stderr before the upload.

`zega function logs <id>` takes `--from`, `--to`, `--limit` (1 to 200), `--status`, `--search` and `--cursor`.

### start

`zega start [args]` runs `zega-server start [args]`, the database server that ships with this CLI: the arguments are passed through untouched (`zega start --data ./data --port 9342`, `--host 0.0.0.0 --token-file ./token`; `--help` is the server's own), stderr is the server's, and the exit code is the server's (128 plus the signal if it was killed by one). SIGTERM and SIGHUP sent to `zega` reach the server; so does SIGINT unless a terminal is attached, where Ctrl-C already reaches both. What is `zega`'s own, taken out of the arguments: `--explorer`, `--explorer-port <n>`, `--human` and `--server <path>`.

It says what is running as one JSON line, an event; whatever else the server prints on stdout is passed on one `{"event":"output","line":...}` per line, so that stdout is all JSON:

```
{"event":"ready","server":{"url":"http://127.0.0.1:9342","status":"running"},"explorer":{"url":null,"status":"stopped"}}
```

With `--human` it says it in a banner (a tagline, a rule, a line each, and on a terminal the mark above), and the server's own lines are passed on as they are:

```
  ➜  Local:     http://127.0.0.1:9342
  ➜  Explorer:  (stopped)
       zega start --explorer
```

`zega start --explorer` starts the server and serves the explorer (below) on the same database, on 9343 (`--explorer-port <n>`; 0 chooses a free one); both lines then have their addresses. If the server needs a token (`--token-file`), the explorer adds it to the requests it passes on: the page never has it.

Which server: the binary of the npm package `@zegadb/server-<os>-<cpu>` (`darwin-arm64`, `darwin-x64`, `linux-x64`, `win32-x64`; one file, `bin/zega-server`), which `zegadb` lists as optionalDependencies at the exact version of the zega release this CLI is tested against, as create-deka-app does for deka. It is looked for from where the CLI's code is, then from where the running executable is, in each one's `node_modules` and every `node_modules` above. **It is never the `zega-server` on `PATH`.** If the package for this machine is not installed, `zega start` exits 1 naming the package and the platform (reinstall `zegadb` without `--omit=optional`). `--server <path>` runs another build, for development.

### explorer

```
zega explorer [--port 9343] [--data ./zega-data]
zega explorer <graph>
```

`zega explorer` runs the bundled `zega-server start` on the data directory (on a port of its own, 127.0.0.1) and serves the explorer page on 127.0.0.1:9343, then prints the address and opens nothing (a browser is yours to open). Ctrl-C stops both. It exits 1 if the database cannot start (a data directory another process holds, say; the server's own words are printed) or the port is taken. By default it prints one event, `{"event":"ready","url","data","server"}`; with `--human` the address alone on a line, and what it serves on stderr.

The page is zegadb/zega's explorer in its CLI form: the same source as the zega.dev playground, built without the website's links, sample graphs and examples, so it opens on your own database and has nothing else in it. The bar says what it is connected to (`local server · zega-data`, or `cloud graph · <name>`) and has one button to switch between the two. An empty database says so and shows how to load data (Import, or a ZQL mutation); `clear` asks before it deletes anything. Writes go to the real engine in `zega-server`, so the database is persistent and the same one `zega start` serves.

**A cloud graph** is connected from the same page: **Connect to remote graph**, then the graph and its key (`zk_...`, made in the dashboard). If you are logged in the graphs of your account are offered by name. The page's requests go to `zega`, which passes them on to the graph's own address with the key the page sent; the cloud only ever sees a server, so its CORS rule does not apply, and `zega` keeps nothing (no key, no body, no log of either). Your API token is used only to list your graphs and find their addresses, and never goes to a graph.

`zega explorer <graph>` prints instead the address of that cloud graph's console in the dashboard (`https://dashboard.zega.dev/graphs/<id>/console`; the project-first layout in zegadb/dashboard#44 redirects it), after asking the API that the graph exists. It prints `{ "url" }` (with `--human`, the address alone). It needs a token (`zega login` or `--token-file`).

Only 127.0.0.1 is served. A page of another website must not be able to drive your database through your machine, so a request must carry this server's own `Host` (against DNS rebinding), and one that can change anything must have no `Origin` or its own.

The page is vendored in `src/explorer/` (built files and `inventory.json`: the zegadb/zega commit and the SHA-256 of each file; `bun test` fails if they disagree). It is embedded in the compiled executables. To update it: commit the change in a zegadb/zega checkout, then `bun run explorer:update ../zega` here (it builds `browser/` with `npm run build:cli`, refuses a dirty checkout, and rewrites `src/explorer/`). `zega explorer --assets <dir>` serves a build of the page from a directory instead (a `browser/dist-cli` you are working on), and `--server <path>` runs another zega-server.

Two things the page still takes from the network, as the playground does: the editor (Monaco, from cdn.jsdelivr.net) and map tiles.

### build

`zega build [id]` writes each function's bundle to `dist/<function id>.js` (`--out <dir>` changes the directory, `--minify` shrinks it). It is `zega deploy` without the upload: the same bundling step (one function, `prepare` in `src/handlers/deploy.ts`), so the file is byte for byte what `deploy` would send, the Node built-in refusal and the 1,000,000-byte limit are the same, and it needs no token and no network. It prints `[{ "id", "entry", "file", "bytes" }]`; `--human` prints what follows.

```
$ zega build --human
bundled src/index.ts: 540 bytes (limit 1,000,000)
Built  f0bbbbbbbbbbbbbbbb  src/index.ts -> dist/f0bbbbbbbbbbbbbbbb.js (540 bytes)
```

### dev

`zega dev [id]` runs the functions `zega.json` lists on localhost, with Bun, built by the same bundling step as `deploy`. A function is `export default { fetch(request, env, ctx) }` here as in the cloud.

```
$ zega dev --human
zega dev: Bun serves your function; this is not the Workers runtime (see `zega dev --help`)
  f0bbbbbbbbbbbbbbbb  src/index.ts  http://127.0.0.1:8787
    env: GREETING, SECRET  (.dev.vars)
  graph: none (--local-graph points ZEGA_GRAPH_URL at a local zega-server, --cloud-graph <id> at a cloud graph)
  watching for changes; Ctrl-C to stop
```

- **Ports** are stable: the first function in `zega.json` is on 8787 (`--port <n>` moves the first), the next on one more, and so on. `zega dev <id>` serves one function on the port it has when all are served. A port that is taken is an error naming it, never a silent move. Only `127.0.0.1` is bound.
- **Reload.** Any file saved under the project (not `node_modules`, `dist`, `.git`, `.tmp`) rebuilds. The port stays; the new build replaces the old one between requests. A build that fails (a syntax error, a Node built-in, a bundle over the limit) prints why and the previous build keeps serving. `--no-watch` serves once.
- **Env and secrets.** A deployed function has no bindings: its `env` is its variables and secrets, flat strings, and nothing else. `zega dev` gives it the same shape from a dotenv file next to `zega.json`: `.dev.vars` for every function, and `.dev.vars.<function id>` over it for one. Names and sizes follow the cloud's rules (a letter or `_` then letters, digits and `_`, up to 64 characters; a value up to 5,120; 64 values), so what runs here can be set there. The shell's environment and your stored API token are never passed to a function; names are printed at start, values never. Add `.dev.vars*` to `.gitignore`.
- **Which graph.** A function reaches a graph over HTTPS: the address in a variable, a graph key (`zk_...`) in a secret. `--local-graph [url]` sets that variable to a `zega-server start` on this machine (default `http://127.0.0.1:9342`; it says if nothing answers there). `--cloud-graph <id>` sets it to that cloud graph's address, asked of the API with your stored token (or `--token-file`); the key is a secret like any other and comes from `.dev.vars`, and the start-up says it is the CLOUD graph and warns when the key is missing. The variable is `ZEGA_GRAPH_URL` and the key `ZEGA_GRAPH_KEY`; `--graph-url-var` and `--graph-key-var` use the names your function reads. The two flags are alternatives, and with neither the function gets no graph address.
- By default every line on stdout is a JSON event: `ready` at start (`functions` with their addresses and env names, the `graph`, `watching`), then `reloaded`, `build_failed` (the previous build keeps serving), `request`, `threw`, `wait_until_rejected` and `warning`; stderr stays empty. With `--human` the start is the text above on stdout and the rest are lines on stderr.

**What Bun is not.** `zega dev` is not the Workers runtime, and says so. Where it can tell, it prints a one-line warning when a function uses something it cannot imitate; the rest is listed here:

- `request.cf` is undefined; there are no `cf-*` request headers; the request URL is `http://127.0.0.1:<port>/...`, not the function's address.
- No Cache API (`caches`), no `WebSocketPair` or WebSocket upgrade responses, no `crypto.DigestStream`, no `crypto.subtle.timingSafeEqual`.
- `cloudflare:workers` provides `env` and `waitUntil` only; other `cloudflare:` modules, Durable Object and entrypoint classes and any export besides the default are not provided. (Zega Cloud uploads a function with no bindings, so there is nothing to bind them to.)
- `ctx.passThroughOnException()` does nothing: there is no origin to fall through to, an error answers 500. `scheduled`, `queue`, `email` and `tail` handlers are never called (a function is only invoked over HTTP).
- Bun has globals and modules Workers has not: `process` (so `process.env` is your shell here and absent in the cloud), `Buffer`, `Bun`, `require`; `navigator.userAgent` says Bun. Node built-in imports are refused at build as in `deploy`, but the globals are not removed.
- Limits are not enforced: the function's CPU time and subrequest limits (`zega functions <id>`), memory, request and response sizes. Workers freezes `Date.now()` while code runs; Bun does not. Outbound `fetch` can reach `localhost`, which is how `--local-graph` works and which a deployed function cannot do.
- `console` output goes to your terminal, not to the function's cloud logs.

### output

**Default: JSON.** A command prints one JSON value on stdout, and nothing else (no colour, no logo, no table), whether or not a terminal is attached. A command that keeps running (`start`, `dev`, `explorer`) prints one JSON line per event, each with an `event` name. `--human` is the only switch to the readable form: no output is chosen by what stdout is.

| command | stdout |
|---|---|
| `login`, `whoami`, `projects`, `graphs`, `buckets`, `functions`, `usage`, `regions`, `function logs` | the API's answer, unchanged (except that a `token` object has no `prefix`; `projects` is `{ "projects": [...] }`, `whoami` is `{ "account", "token" }`, and so on; the shapes are in `openapi/openapi.json`) |
| `deploy`, `function deploy` | the API's answer, one line per function: `{ "function": { "id", "name", "url", "version", ... } }` |
| `logout` | `{ "loggedOut": true }` (`false` when nothing was stored) |
| `init` | `{ "file", "config": { "version", "name", "region", "functions"? } }` |
| `build` | `[{ "id", "entry", "file", "bytes" }]` |
| `explorer <graph>` | `{ "url" }` |
| `explorer` | `{ "event": "ready", "url", "data", "server" }`, then nothing until it stops |
| `start` | `{ "event": "ready", "server": { "url", "status" }, "explorer": { "url", "status" } }`, then `{ "event": "output", "line" }` for what the server prints |
| `dev` | `{ "event": "ready", "functions": [{ "id", "entry", "url", "env" }], "graph", "watching" }`, then `reloaded`, `build_failed`, `request`, `threw`, `wait_until_rejected`, `warning` events |

**Errors** are one JSON object on stderr and a nonzero exit code: `{"error":{"code","message","hint"}}`. For a failure of the API `code` is its code and `message` its sentence; `hint` is what to do about it (how to log in on a 401, a manage token on `scope_required`, the wait time on 429 and 503: the CLI never retries by itself; whether the change took effect), or `null`. A network failure has the code `network`; a mistake on the command line, `usage` (hint `zega --help`); one on this machine its own (`not_logged_in`, `missing_flag`, `invalid_flag`, otherwise `local`). With `--human` it is the sentence people read: `zega: <sentence> (code: <code>, HTTP <status>)` and the hint lines under it.

**Nothing is asked.** A question is a flag too. Without `--human`, a missing value is an error naming the flag (`zega init` names every missing flag at once; `zega login` says to pipe the token or pass `--token-file`). With `--human` on a terminal, `zega init` and `zega login` ask. The terminal matters only there: for the questions, and for the logo and colour (which `--human` shows on a terminal).

**Help is JSON** too, pretty-printed like a `package.json` and made from the command definitions, so it cannot go stale: `zega --help` lists the commands by group and the global options, `zega <command> --help` has the usage, arguments and options (`zega start --help` is the server's). On a terminal it is coloured and has the logo above it; piped it is plain JSON.

What costs money, and the tokens themselves, are dashboard-only: the API answers 403 `session_required` and the CLI says so. `zega init` therefore creates nothing in the cloud.

### zega init

```
zega init --human                           # on a terminal: asks for the name and the region
zega init --name my-app --region yyz        # no questions (the default)
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

`zega init --function <id> [--entry <path>]` writes that (the entry defaults to `src/index.ts`; with `--human` on a terminal init asks for both, and an empty function id means none). `init` never creates the function: that is done in the dashboard.

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

The tests run the CLI as a subprocess against a fake API on `127.0.0.1` that records every request, on a real terminal (a pty) for the questions and for the JSON a terminal still gets, and as the compiled executable.
