import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { chromium } from 'playwright';
import { build as viteBuild, createServer, preview } from 'vite';
import { build as esbuild } from 'esbuild';
import { startFakeGraph } from './support/fake-graph.mjs';

const root = resolve(import.meta.dirname, '..');
process.chdir(root);
const work = resolve('.tmp/consumers');
const manifestPath = resolve('dist/package.json');

function run(command, args, cwd = root) {
  return execFileSync(command, args, { cwd, stdio: 'inherit' });
}

async function installConsumers() {
  const pack = JSON.parse(await readFile('artifacts/pack.json', 'utf8'));
  await rm(work, { recursive: true, force: true });
  for (const consumer of ['node', 'browser', 'worker']) {
    const cwd = resolve(work, consumer);
    await cp(`npm/consumers/${consumer}`, cwd, { recursive: true });
    run('npm', ['install', resolve('artifacts', pack.filename), '--ignore-scripts', '--no-save', '--no-package-lock', '--no-audit', '--no-fund'], cwd);
  }
}

async function checkBrowser(outDir, label, dev = false) {
  const server = dev ? await createServer({
    root: resolve(work, 'browser'), server: { host: '127.0.0.1', port: 0 },
  }) : await preview({
    configFile: false, root: resolve(work, 'browser'), base: '/consumer/',
    build: { outDir }, preview: { host: '127.0.0.1', port: 0 },
  });
  let browser;
  try {
    if (dev) await server.listen();
    browser = await chromium.launch({ executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH });
    const page = await browser.newPage();
    const errors = [];
    const assets = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.on('response', response => { if (new URL(response.url()).pathname.endsWith('.wasm')) assets.push(response); });
    await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/consumer/`);
    await page.waitForFunction(() => document.querySelector('#result').textContent !== 'Loading');
    const result = await page.locator('#result').textContent();
    assert.deepEqual(errors, []);
    assert.equal(result, '{"name":"Ada"}');
    assert.ok(assets.length > 0, 'Production browser must fetch a real WASM asset');
    for (const asset of assets) {
      assert.equal(asset.status(), 200);
      assert.match(asset.headers()['content-type'], /application\/wasm/);
    }
    console.log(`${label}: ${result} (WASM HTTP 200, application/wasm)`);
  } finally {
    await browser?.close();
    if (dev) await server.close();
    else await new Promise((done, reject) => server.httpServer.close(error => error ? reject(error) : done()));
  }
}

// What `@zegadb/lib/client` may cost a function. The sample below measures about
// 4.8 KB minified (the whole client is 4.4 KB, 2 KB gzipped); the engine's wasm is
// 3.3 MB. The limit is room for the client to grow, not for the engine to sneak in.
const FUNCTION_BUNDLE_LIMIT = 6 * 1024;

// A Zega Cloud function bundled the way `zega build` does, for a Workers
// runtime: nothing but the client may come with it, and it must still work.
async function checkWorker() {
  const cwd = resolve(work, 'worker');
  const bundle = await esbuild({
    absWorkingDir: cwd, entryPoints: ['worker.ts'], outdir: 'dist',
    bundle: true, minify: true, format: 'esm', target: 'es2022', write: false, metafile: true, logLevel: 'silent',
    // `neutral` resolves no Node built-ins: a `node:` import in the client would fail the build.
    platform: 'neutral', conditions: ['workerd', 'worker', 'browser'], mainFields: ['module', 'main'],
    // were the wasm imported, it would show up as an emitted file
    loader: { '.wasm': 'file' },
  });
  const inputs = Object.keys(bundle.metafile.inputs).sort();
  assert.deepEqual(inputs, ['node_modules/@zegadb/lib/client.js', 'worker.ts'], 'the function bundle pulled in more than the client');
  assert.equal(bundle.outputFiles.length, 1, 'the bundle emitted extra files (wasm?)');
  const [output] = bundle.outputFiles;
  assert.ok(output.contents.length <= FUNCTION_BUNDLE_LIMIT, `function bundle is ${output.contents.length} bytes, over ${FUNCTION_BUNDLE_LIMIT}`);
  for (const forbidden of [/WebAssembly/, /\.wasm/, /zega_wasm/, /ZegaWasm/, /node:/]) {
    assert.doesNotMatch(output.text, forbidden, `function bundle mentions ${forbidden}`);
  }
  await mkdir(resolve(cwd, 'dist'), { recursive: true });
  await writeFile(resolve(cwd, 'dist/worker.mjs'), output.text);

  // run it as the platform would: fetch(request, env), against a Zega Cloud stand-in
  const graph = await startFakeGraph();
  try {
    const worker = (await import(resolve(cwd, 'dist/worker.mjs'))).default;
    const env = { ZEGA_GRAPH_URL: graph.url, ZEGA_GRAPH_KEY: graph.key };
    const read = await worker.fetch(new Request('https://function.example/'), env);
    assert.equal(read.status, 200);
    assert.deepEqual(await read.json(), { echo: { query: '{ Person { name } }', schema: 'type Person { name: String }' } });
    const write = await worker.fetch(new Request('https://function.example/?name=O%22Brien', { method: 'POST' }), env);
    assert.deepEqual((await write.json()).echo.query, 'mutation { Person(name: "O\\"Brien") { name } }');
    const denied = await worker.fetch(new Request('https://function.example/'), { ...env, ZEGA_GRAPH_KEY: 'zk_' + '2'.repeat(32) });
    assert.deepEqual([denied.status, (await denied.json()).code], [401, 'unauthorized']);
    assert.deepEqual(graph.requests.map(r => r.method), ['QUERY', 'POST', 'QUERY']);
  } finally { await graph.close(); }
  console.log(`worker: ${output.contents.length} bytes minified (limit ${FUNCTION_BUNDLE_LIMIT}); inputs ${inputs.join(', ')}; no wasm, no node: imports`);
}

async function checkConsumers() {
  await installConsumers();
  run('node', ['index.mjs'], resolve(work, 'node'));
  await checkWorker();
  const tsc = resolve('node_modules/typescript/bin/tsc');
  const flags = ['--noEmit', '--strict', '--target', 'es2022', '--lib', 'es2022,dom,esnext.disposable'];
  run('node', [tsc, ...flags, '--module', 'NodeNext', '--moduleResolution', 'NodeNext', 'types.mts'], resolve(work, 'node'));
  await cp(resolve(work, 'node/types.mts'), resolve(work, 'browser/types.mts'));
  run('node', [tsc, ...flags, '--module', 'ESNext', '--moduleResolution', 'Bundler', 'types.mts'], resolve(work, 'browser'));

  await viteBuild({ root: resolve(work, 'browser') });
  await checkBrowser(resolve(work, 'browser/dist'), 'browser/vite');
  await checkBrowser(undefined, 'browser/vite-dev', true);

  // esbuild requires an explicit asset import/file loader; its new URL handling
  // differs from Vite/Webpack. Exercise the documented override against the tarball.
  const outDir = resolve(work, 'browser/dist-esbuild');
  await mkdir(outDir, { recursive: true });
  await writeFile(resolve(work, 'browser/esbuild-main.js'), `import wasmURL from '@zegadb/lib/zega_wasm_bg.wasm';\nimport { run } from './query.js';\nawait run({ wasm: wasmURL });\n`);
  await esbuild({
    absWorkingDir: resolve(work, 'browser'), entryPoints: ['esbuild-main.js'],
    bundle: true, minify: true, format: 'esm', platform: 'browser', target: 'es2022',
    loader: { '.wasm': 'file' }, outdir: outDir, publicPath: '/consumer/',
  });
  await writeFile(resolve(outDir, 'index.html'), '<!doctype html><pre id="result">Loading</pre><script type="module" src="/consumer/esbuild-main.js"></script>');
  await checkBrowser(outDir, 'browser/esbuild');
}

if (process.argv.includes('--break-exports')) {
  const original = await readFile(manifestPath, 'utf8');
  try {
    const broken = JSON.parse(original);
    broken.exports['.'] = './missing-entry.js';
    await writeFile(manifestPath, JSON.stringify(broken, null, 2) + '\n');
    run('node', ['npm/pack.mjs']);
    await installConsumers();
    const node = spawnSync('node', ['index.mjs'], { cwd: resolve(work, 'node'), encoding: 'utf8' });
    assert.notEqual(node.status, 0, 'Broken exports unexpectedly passed Node');
    assert.match(node.stderr, /ERR_MODULE_NOT_FOUND/);
    console.log(`broken exports/node: exit ${node.status}; ${node.stderr.split('\n').find(line => line.includes('Error ['))}`);
    await assert.rejects(viteBuild({ root: resolve(work, 'browser'), logLevel: 'silent' }), /[Ff]ailed to resolve (entry for package|import) "@zegadb\/lib"/);
    console.log('broken exports/browser: Vite failed to resolve import "@zegadb/lib"');
  } finally {
    await writeFile(manifestPath, original);
    run('node', ['npm/pack.mjs']);
    console.log('Restored original exports and repacked.');
  }
}
await checkConsumers();
