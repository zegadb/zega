# Handoff: `zega deploy` for apps (CLI side)

Branch `claude/app-deploy`. NOT STARTED beyond this file (budget stop). The cloud API is done: zegadb/cloud PR #53 (branch `claude/app-deploys`; read its README section "Apps" and `docs/openapi.json`).

## Next concrete steps
1. Read `claude/init-templates` (zega.json fields: `kind: "app"`, `framework`, build output paths; the branch had no commits when I looked) and `src/handlers/project_config.ts` (`ProjectConfig` has only `functions`; add `kind`, `framework`, `build` command, worker dir/main module, assets dir).
2. `zega deploy` in a project whose zega.json says `kind: app`: run the framework's Cloudflare build (OpenNext: `.open-next/worker.js` + `.open-next/assets`; Astro: `dist/_worker.js/` + `dist/`, honour `.assetsignore`), or reuse `zega build` output (`src/core/build`).
3. Collect worker modules (all files under the worker dir; type by extension: .js/.mjs `application/javascript+module`, .wasm `application/wasm`, .txt `text/plain`, else `application/octet-stream`; names are relative paths) and the assets dir (manifest path -> {hash, size}; hash = first 32 hex of sha256(base64(content) + extension without dot)).
4. Upload via the new API: create the app function if zega.json has no id (`POST /v1/projects/:p/functions {name, kind:"app"}`), `POST /v1/functions/:id/app/assets {manifest}`, one `PUT .../app/assets/:session` per returned bucket (multipart, part name = hash; split a bucket only above 200 files / 16 MiB; honour 429 `Retry-After`), then `PUT .../app` (multipart `metadata` {mainModule, session, config: {htmlHandling, notFoundHandling, runWorkerFirst, compatibilityDate, compatibilityFlags: ["nodejs_compat"] for both frameworks}} + modules). Code-only redeploy: `keepAssets: true`.
5. Output JSON by default (`--human` for people), progress as one JSON line per event (session opened, bucket n/m uploaded, deployed). Surface 413 limit errors verbatim (they name the limit).
6. Tests with a fake API (see `tests/fixtures/fake-zega-server.ts` style); run `bun run typecheck` and `bun test`, log to a file, check the exit code.
7. Do not touch `/Volumes/Projects/claude/zega-cli-repo` (Sami's `zega` is linked to it).

-claude
