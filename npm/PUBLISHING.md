# Releasing @zegadb/lib

The embeddable graph database library is published as **@zegadb/lib**, a
scoped package in the `@zegadb` npm organization. The unscoped name **zegadb**
belongs to the separate developer CLI (its own repository), so this package
does not use it, and the name zega is unavailable (npm's similarity filter
matched egg). Only a `zegadb@0.0.0` placeholder was ever published from here;
nothing depended on it. The repository and Rust crate names remain zega. The
native server executable is named `zega-server`; it is not an npm package.

## Build once, promote the tested payload

This follows [RFD 68](https://github.com/dekaruntime/rfd/issues/68) and dsc's
`tag-canary.yml`, `release.yml`, and `promote.yml`.

| Channel | Git / R2 prefix | npm version | npm dist-tag |
| --- | --- | --- | --- |
| canary | `v<V>-canary-<sha7>` | `<V>-canary.<sha7>` | `canary` |
| stable | `v<V>` | `<V>` | `latest` |

`Cargo.toml` keeps a plain stable workspace version. Every main push runs
`tag-canary.yml`: it refuses an already-promoted version or existing canary
tag, otherwise pushes the tag and explicitly dispatches `release.yml` against
it. GitHub's built-in token does not trigger workflows from its own tag pushes.
A user-pushed canary tag also triggers the build; a plain stable tag never does.

Release builds the `zega-server` executable on Linux x64, macOS arm64/x64, and Windows x64. It
starts each resulting binary and executes an authenticated HTTP query. The
npm branch's builder compiles WASM once, stamps @zegadb/lib's canary version, packs,
and runs installed-tarball consumers in Node, Chromium, Vite, esbuild and Next.
The same WASM files and tested npm tarball, plus native binaries, are stored in
both `zega-releases/<CANARY>/` and `zega-wasm/<CANARY>/`. Each has a checksum
inventory in `manifest.json` and `release.json`. Readback is verified before
`canary/` and `canary.json` advance. Existing canary prefixes are immutable.

Sami promotes with **Actions → Promote → Run workflow → Branch: main → canary:
full tested tag → Run workflow**. Promotion resolves the tag's commit, refuses
an existing stable tag, and reads the canary's own `release.json` from R2.
Its commit must match the tag. Both bucket copies and every payload checksum
are checked before any writes. It copies objects to `v<V>/`, rewrites only
`version`, `tag`, `channel`, and `promoted_from` in both metadata files, verifies
readback, creates the stable tag at the canary commit, then advances `latest/`
and `latest.json` in both buckets. No compilation or dependency installation
occurs in the promotion job.

**R2 payload artifacts are byte-identical between canary and stable.** The two
JSON bookkeeping files intentionally differ. Even R2's `package.tgz` remains
the original canary archive, with unchanged checksum. **npm tarballs differ by
their package.json version string.** publish-npm repacks that R2 archive locally
for a stable npm publish, preserving every other file byte and package field. It never uploads
the repacked archive over the original R2 payload or pretends the tarballs are
byte-identical. Tar/gzip container encoding can also change during repacking.

If a run stops after copying objects, before tagging, rerun the same canary;
readback gates still apply. If the stable tag was pushed but pointer updates
failed, do not delete or move the tag: Ava/Sami must inspect the recorded commit
and hashes and recover the pointers. Promotion deliberately refuses an existing
stable tag. A canary upload interrupted partway leaves a prefix that is refused
on rerun; inspect and remove only that incomplete prefix before retrying. npm
publishing failures can be retried using GitHub's **Re-run failed jobs**; never
rebuild or replace an already-published npm version.

## Environments and compilation

All PR jobs declare `public-ci`. All publishing and tagging jobs declare
`release`. Compilation uses the matrix's sccache bucket and a job-level
`RUSTC_WRAPPER: sccache`, including wasm-pack installation. The cache setup
fails closed on absent credentials or inaccessible bucket. Release compilation
also uses the limited public-ci cache identity; full R2 keys are used only by
copy/upload jobs. No PR job references full release credentials.

| Environment | Secret names |
| --- | --- |
| release | `R2_ACCOUNT_ID`, `R2_SCCACHE_ACCESS_KEY_ID`, `R2_SCCACHE_SECRET_ACCESS_KEY` (the all-buckets token) |
| public-ci | `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` (the sccache-only token) |

The endpoint is always composed from the account ID:
`https://${{ secrets.R2_ACCOUNT_ID }}.r2.cloudflarestorage.com`.
Cache buckets: `zega-sccache-{linux-x64,darwin-arm64,darwin-x64,windows-x64}`.
There is no stored npm credential or crates.io publisher.

## What is published to npm, and from where

One workflow publishes everything: `.github/workflows/publish-npm.yml`
(workflow name `publish-npm`; the filename is fixed because npm trusted
publishing keys on it, as with create-deka-app's `publish-runtime.yml`). It
runs in the GitHub environment **release** with `id-token: write` (OIDC) and
no npm token secret. It never compiles: it downloads a release that Release
(canary) or Promote already stored in `zega-releases/<tag>/`, checks every file
against that release's `manifest.json`, packs, and publishes.

| Package | What it is |
| --- | --- |
| `@zegadb/lib` | The embeddable engine (WASM + JS API) and, as `@zegadb/lib/client`, the fetch-only client for a running zega (no wasm, no dependencies); built by `build.mjs`. |
| `@zegadb/server-darwin-arm64`, `-darwin-x64`, `-linux-x64`, `-win32-x64` | The `zega-server` executable, one binary per package, `os`/`cpu` restricted, no install scripts, no dependencies (`pack-server.mjs`). |

The server packages exist for the future `zegadb` CLI package (its own
repository, owned by Sami): it lists them as `optionalDependencies` at the
exact same version, and npm installs the one that matches the machine. All five
packages always have the same version: `<V>-canary.<sha7>` under dist-tag
`canary`, `<V>` under `latest`.

**Dispatching it** (Actions -> publish-npm -> Run workflow -> Branch: main ->
tag): a canary tag `v<V>-canary-<sha7>` after Release finished for it, or a
stable `v<V>` after Promote finished. Nothing dispatches it automatically yet;
once trust works, a later PR can add the dispatch to `release.yml`. The run
packs and verifies all five tarballs before the first publish, publishes the
four server packages and then `@zegadb/lib`, and skips any package@version npm
already has, so re-running after a partial failure resumes. A stable publish
repacks R2's canary-built `package.tgz` with only the version changed
(`scripts/channels.py repack`).

## Sami's npm clicks, before the first publish

Trusted publishing can only be attached to a package that already exists, so
every package that was never published needs a one-time **manual placeholder
publish** first (create-deka-app and every `@dekaruntime/*` package started the
same way: a `0.0.0` published by hand, then trusted publishing for everything
after). Placeholders: a directory with a `package.json` that has only
`name` and `version: "0.0.0"`, then `npm publish --access public` signed in
with 2FA. A scoped package is private unless `--access public` is given on that
first publish. Never put a token in this repository or its secrets.

| Package | Placeholder needed? |
| --- | --- |
| `@zegadb/lib` | yes, never published |
| `@zegadb/server-darwin-arm64`, `@zegadb/server-darwin-x64`, `@zegadb/server-linux-x64`, `@zegadb/server-win32-x64` | yes, never published |
| `zegadb` (the CLI, other repository) | no, `0.0.0` is already there |

Then, for each of the five packages in this repository, on npmjs.com ->
**Packages** -> the package -> **Settings** -> **Trusted publishing** ->
**GitHub Actions**: **Organization or user: zegadb**, **Repository: zega**,
**Workflow filename: publish-npm.yml**, **Environment name: release**. Allow
direct **npm publish**, save, complete 2FA. One workflow file and one
environment for all five: there is no second connection (the old design had
`release.yml` and `promote.yml`; their disabled npm jobs are gone). Then
**Settings -> Publishing access -> Require two-factor authentication and
disallow tokens** on each. The `@zegadb` organization already exists.

Then dispatch publish-npm with a canary tag, test `npm install
@zegadb/lib@canary`, and after Promote dispatch it with the stable tag and
confirm `npm install @zegadb/lib@latest`, provenance, and versions.

The field names and npm requirements were checked
against [npm's trusted-publisher documentation](https://docs.npmjs.com/trusted-publishers/).
Saving trust settings does not verify an OIDC publish. This change performs no
npm/crates.io publication, live canary tagging, stable promotion or R2 writes.

## Reproduce local checks

Use Node >=22.14 (CI 24), Rust 1.96.0, wasm-pack 0.15.0 and sccache. On this Mac,
keep the globally configured Rust wrapper and use a private build directory:

```sh
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
export npm_config_cache="$PWD/.tmp/npm-cache"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
python3 -m unittest -v scripts.test_channels
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features
npm ci --ignore-scripts --no-audit --no-fund
npm run build
node npm/stamp-version.mjs "0.1.0-canary.$(git rev-parse HEAD | cut -c1-7)"
npm run pack:package
# macOS: use installed Chrome; CI installs Playwright's Chromium.
export PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
npm test
npm run test:exports
```

Keep zega-wasm's standalone workspace: its committed lockfile and path
dependencies isolate browser builds from the native server workspace.
The release workflow runs builds on PR events without publishing. The
Tag canary and Promote proof jobs exercise their production scripts against
isolated local Git/R2 fixtures and print both refusal messages and the integrity
failure. They are not live-bucket or live-tag tests.
