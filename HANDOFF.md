# zega mcp: handoff (claude, 3 Oct, budget stop)

Branch `claude/mcp`. Delete this file before the PR is marked ready.

## Done
- `zega mcp [--groups] [--read-only] [--graph-key-file] [--storage-key-file] [--server]` and `zega mcp config --client ...` (src/core/mcp/mcp.ts, registered in src/program.ts under the Local group).
- `src/mcp/`: groups.ts, clients.ts (the snippet generator, imports nothing, copy it into the dashboard), tool.ts (answer wrapper: redaction + 100k cut), keys.ts (graph/storage key: given file, `.dev.vars`, else minted for the session with a manage token and revoked on exit; never under --read-only), local_servers.ts, server.ts (tool selection, stdio loop, stdout guard), tools/{docs,account,graphs,functions,storage,local}.ts.
- Support: `CloudClient.post/delete` + 201 answers (utils/cloud_request.ts), `capture()` sink in utils/output.ts (tools reuse the core commands without printing on stdout), handlers/graph_http.ts, handlers/docs.ts, vendored docs (src/docs, `bun run docs:update ../zega`, from zegadb/zega 418f361).
- tests/mcp.test.ts (16 pass): per-group tool lists, --read-only, read token, read tool vs fake API, token/key never in output, key minted + revoked, QUERY-only reads, deploy path containment, local group. Fixtures: tests/fixtures/fake-query-server.ts, tests/mcp_helpers.ts.
- `bunx tsc --noEmit` clean.

## Left
1. Run the whole `bun test` (not run yet; log to a file, check the exit code). The help test pins the group list (`local` is used, so it should pass) and lists every command.
2. Revert-prove the read-only test: in `selectTools` (src/mcp/server.ts) drop the `readOnly` condition, run `bun test tests/mcp.test.ts`, confirm the `--read-only` and read-token tests fail, restore; put the output in the PR body.
3. Tests not written yet: `mcp config` (each client; exactly `claude mcp add --scope project zega -- zega mcp --groups account,graphs`, with and without --read-only, a --token-file with spaces), a unit test that src/docs matches inventory.json/pages.ts (like tests/explorer_assets.test.ts), and the compiled executable answering `tools/list` (tests/executable.test.ts pattern; checks the SDK bundles in `bun build --compile`). Also `bun run build` (all four targets).
4. README: a short "zega mcp" section (per-client commands from `zega mcp config`, groups, --read-only, key handling: a manage token mints a `zega mcp <time>` graph key per session and revokes it on exit; a killed process leaves it, revoke in the dashboard), and add `mcp` to the commands list and output table.
5. Windsurf path (`~/.codeium/windsurf/mcp_config.json`) is from guides, its docs page now redirects to docs.devin.ai (names ~/.config/devin/mcp_config.json): say so in the PR.
6. Open PR body (sign `-claude`, no AI trailers in commits): what, why, evidence, the tool list, "closes" the issue number if one exists. No `zega query` CLI command exists; graph query logic lives in handlers/graph_http.ts + mcp/tools/graphs.ts, a follow-up could expose it as a command.

-claude
