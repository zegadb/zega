// The browser path of the zegadb/zega#143 scale bench: the wasm engine in
// headless Chromium, loading each generated travel sample exactly as
// zega.earth loads one (browser/scale/bench.js), on a desktop profile and
// with 4× CDP CPU throttling as a phone proxy.
//
//   node scripts/scale-browser.mjs <dataRoot> <out.json> [sizes...] [--pkg=<dir>]
//
// <dataRoot> holds <nodes>/ dirs from `zega-scale gen`. --pkg serves the wasm
// package from <dir> instead of the repo's browser/pkg (to bench the build
// zega.earth actually serves, without touching browser/pkg). Writes one JSON
// document: per size the gzipped download size, then per profile the bench
// page's timings, memory and query p50/p95/p99. A size that throws, crashes
// the page, or loads in over 60 s is recorded as failed and stops the run:
// bigger sizes would only fail harder.

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(path.join(root, 'browser', 'package.json'));
const { chromium } = require('@playwright/test');

const [dataRoot, outFile, ...rest] = process.argv.slice(2);
if (!dataRoot || !outFile) {
  console.error('usage: node scripts/scale-browser.mjs <dataRoot> <out.json> [sizes...] [--pkg=<dir>]');
  process.exit(2);
}
const pkgArg = rest.find((arg) => arg.startsWith('--pkg='));
const pkgDir = pkgArg ? path.resolve(pkgArg.slice('--pkg='.length)) : null;
const sizes = (rest.filter((arg) => !arg.startsWith('--')).length ? rest.filter((arg) => !arg.startsWith('--')) : ['1000', '10000', '100000', '1000000']).map(Number);

const MIME = {
  '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm',
  '.json': 'application/json', '.csv': 'text/csv', '.zql': 'text/plain',
};

const server = http.createServer((req, res) => {
  const url = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  // /browser/* comes from the repo (the page and the wasm package), /data/*
  // from the generated samples. With --pkg, /browser/pkg/* comes from the
  // given directory instead (the build zega.earth serves).
  const base = url.startsWith('/data/') ? dataRoot
    : pkgDir && url.startsWith('/browser/pkg/') ? pkgDir
    : url.startsWith('/browser/') ? path.join(root, 'browser') : null;
  const rel = url.startsWith('/data/') ? url.slice('/data'.length)
    : pkgDir && url.startsWith('/browser/pkg/') ? url.slice('/browser/pkg'.length)
    : url.slice('/browser'.length);
  const file = base && path.normalize(path.join(base, rel));
  if (!file || !file.startsWith(base) || !fs.existsSync(file) || !fs.statSync(file).isFile()) {
    res.writeHead(404).end('not found');
    return;
  }
  res.writeHead(200, { 'content-type': MIME[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});

const LOAD_LIMIT_MS = 60_000;

async function benchProfile(browser, port, nodes, throttle) {
  const context = await browser.newContext();
  const page = await context.newPage();
  let crashed = false;
  page.on('crash', () => { crashed = true; });
  if (throttle > 1) {
    const cdp = await context.newCDPSession(page);
    await cdp.send('Emulation.setCPUThrottlingRate', { rate: throttle });
  }
  try {
    await page.goto(`http://127.0.0.1:${port}/browser/scale/index.html`);
    const cap = new Promise((_, reject) => setTimeout(() => reject(new Error('over the 600 s cap')), 600_000));
    const result = await Promise.race([page.evaluate((n) => window.runScaleBench(n), nodes), cap]);
    if (crashed) return { failed: 'page crashed' };
    if (result.load_ms > LOAD_LIMIT_MS) return { ...result, failed: `load ${Math.round(result.load_ms)} ms > ${LOAD_LIMIT_MS} ms` };
    return result;
  } catch (error) {
    return { failed: crashed ? 'page crashed (out of memory)' : String(error?.message || error).split('\n')[0] };
  } finally {
    await context.close();
  }
}

const gzipSize = (nodes) => {
  const dir = path.join(dataRoot, String(nodes));
  return fs.readdirSync(dir)
    .filter((name) => name.endsWith('.csv') || name.endsWith('.zql'))
    .reduce((total, name) => total + zlib.gzipSync(fs.readFileSync(path.join(dir, name))).length, 0);
};

await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const port = server.address().port;
const browser = await chromium.launch({ headless: true, args: ['--enable-precise-memory-info'] });

const results = { path: 'browser', machine: process.env.SCALE_MACHINE || 'unknown', pkg: pkgDir || path.join(root, 'browser', 'pkg'), sizes: {} };
try {
  for (const nodes of sizes) {
    if (!fs.existsSync(path.join(dataRoot, String(nodes), 'travel.zql'))) {
      console.error(`no generated data for ${nodes}; run zega-scale gen first`);
      break;
    }
    const entry = { gz_bytes: gzipSize(nodes) };
    for (const [profile, throttle] of [['desktop', 1], ['phone', 4]]) {
      process.stderr.write(`browser ${nodes} ${profile}...\n`);
      entry[profile] = await benchProfile(browser, port, nodes, throttle);
    }
    results.sizes[nodes] = entry;
    if (entry.desktop.failed) {
      results.stopped_at = nodes;
      break; // Bigger sizes would only fail harder.
    }
  }
} finally {
  await browser.close();
  // Chromium's keep-alive connections survive server.close() and hold the
  // event loop; drop them so the process exits instead of hanging.
  server.closeAllConnections();
  server.close();
}
fs.writeFileSync(outFile, JSON.stringify(results, null, 2));
console.log(`wrote ${outFile}`);
process.exit(0);
