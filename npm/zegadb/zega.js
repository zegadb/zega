#!/usr/bin/env node
// `zega` from npm: runs the `zega` executable in the platform package npm
// installed beside this one (@zegadb/server-<os>-<cpu>, which also holds the
// zega-server it starts). Node is enough; no Bun, no download, no install script.
// npm/pack-server.mjs fills in the platforms when it packs zegadb.
'use strict';
const { spawnSync } = require('node:child_process');
const path = require('node:path');

const PLATFORMS = __PLATFORMS__;
const id = PLATFORMS[`${process.platform}-${process.arch}`];
if (id === undefined) {
  console.error(`zega: there is no zega for ${process.platform}-${process.arch}; it ships for ${Object.keys(PLATFORMS).join(', ')}`);
  process.exit(1);
}
let executable;
try {
  executable = path.join(path.dirname(require.resolve(`@zegadb/server-${id}/package.json`)), 'bin', 'zega');
} catch {
  console.error(`zega: @zegadb/server-${id} is not installed. It is an optional dependency of zegadb: reinstall without --omit=optional or --no-optional.`);
  process.exit(1);
}
const run = spawnSync(executable, process.argv.slice(2), { stdio: 'inherit' });
if (run.error) {
  console.error(`zega: could not run ${executable}: ${run.error.message}`);
  process.exit(1);
}
if (run.signal) process.kill(process.pid, run.signal);
process.exit(run.status ?? 1);
