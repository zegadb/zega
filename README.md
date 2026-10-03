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
zega projects [id]                   list projects, or show one with its graphs
zega graphs [id]                     list graphs, or show one
zega buckets [id]                    list storage buckets, or show one
zega functions [id]                  list functions, or show one
zega function logs <id>              a page of a function's requests and console lines
zega usage                           this month's usage and bill per graph
zega regions                         the regions a graph can run in
```

Global options: `--api <url>`, `--token-file <path>`, `--json`.

`zega function logs <id>` takes `--from`, `--to`, `--limit` (1 to 200), `--status`, `--search` and `--cursor`.

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

`version` is the file's own format, `name` the project's display label (1 to 64 characters), `region` a Zega Cloud region id (`zega regions`). It holds no secret and no cloud id.

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
