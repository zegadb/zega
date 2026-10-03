# Handoff: `zega init` starters (branch claude/init-templates)

Delete this file before merging.

## Done
- `zega init --template function|nextjs|astro` (default function; `--human` on a TTY asks, function first). JSON result: `template`, `file`, `config`, `files`, `next`. Never overwrites without `--force` (names the files in the way).
- Starters are vendored text in `src/templates/` (versions pinned in `versions.ts`; `@zegadb/lib` 0.2.0 = guess of the version that will carry `/client`, check against zegadb/zega when published). Function starter, Next.js (OpenNext) and Astro (`@astrojs/cloudflare`, `imageService: 'passthrough'`).
- `zega.json` for apps: `kind: "app"`, `framework`, `build {command[], worker, assets}`, `runtime {compatibilityDate, compatibilityFlags}`; validated in `src/handlers/project_config.ts`.
- `zega build` in an app runs `build.command` from `node_modules/.bin`, checks worker/assets, prints JSON (`src/handlers/app.ts`). `zega dev` in an app runs `next dev` / `astro dev --ignore-lock` with `.dev.vars` in the env (`devApp` in `src/core/dev/dev.ts`). `zega deploy` in an app fails `app_deploy_unsupported`.
- README documents all of it. Tests in `tests/templates.test.ts` (+ updated init/terminal tests); stand-in client in `tests/fixtures/zega-lib-stand-in/` because `@zegadb/lib/client` was not on npm or in the lib-client worktree when this was written (its API assumed from the brief: `connect({url,key}).query(zql)`).

## Left (verify first: last run was 13 pass, 2 fail)
1. One failing assertion was fixed in the last edit (not re-run): the `next` text check in "the function is the default...". Re-run `bun test tests/templates.test.ts`.
2. One "(unnamed) beforeEach/afterEach hook timed out (22 s)" in the templates run: find which (likely the network describe: `bun install` + `zega dev` for nextjs/astro, 300 s timeout each) and fix. Set `SKIP_NETWORK_TESTS=1` to skip those two tests.
3. Run full `bun run typecheck` and `bun test` (the compiled-executable `zega start` test timed out once under machine load, passed before).
4. Check `@zegadb/lib/client` real API (`query`/`mutate` signature, result shape) when zegadb/zega `claude/lib-client` lands, and align `src/templates/*` and the stand-in; set the pinned version in `src/templates/versions.ts`.
5. App deploy follow-up (not built here): OpenNext output is multi-file (`.open-next/worker.js` imports `./cloudflare`, `./server-functions`...) plus assets and needs `nodejs_compat`; Astro 7 output is `dist/server/entry.mjs` + chunks + `dist/client`, and its adapter adds KV `SESSION` and ASSETS bindings in `dist/server/wrangler.json`. Zega Cloud functions today take one module <= 1 MB, no assets, no bindings, no nodejs_compat: the cloud API and `zega deploy` need multi-module + assets + compat flags (being built separately). `zega.json` already carries what deploy needs.
6. `zega.json` needs a function id for `zega dev`/`deploy` of the function starter; init lists it only with `--function <id>` (no fake placeholder id).
7. Open the PR as ready once green; list app deploys as the open follow-up; sign the body `-claude`.

## Notes
- Astro 7 auto-backgrounds `astro dev` when an agent env is detected (AI_AGENT etc.); `--ignore-lock` keeps it in the foreground.
- Next 16 writes AGENTS.md/CLAUDE.md on first `next dev`.
