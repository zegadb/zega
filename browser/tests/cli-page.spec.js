// The page the `zega` command serves (`npm run build:cli` makes dist-cli/): the explorer without
// the website, on a local database or a cloud graph.
//
// dist-cli/ is served from disk at the test server's address, with the two things the command
// itself answers: /explorer-config.json (ui: cli) and the database's routes (a small stand-in for
// `zega-server start`: GET /graph, DELETE /graph). Everything else is the real page, the real
// wasm parser and the real editors, so what is asserted is what a person sees.
import { test, expect } from './offline.js';
import { readFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';

const DIST = resolve('dist-cli');
const TYPES = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.geojson': 'application/json', '.ttf': 'font/ttf' };
const ORIGIN = 'http://127.0.0.1:8787';
const GRAPH = 'u7bgg2k9q4xmh3ne5t';
const KEY = 'zk_abcdefghijklmnopqrstuvwxyz234567';

/**
 * Serve the CLI page. `local` is the stored graph the stand-in database answers with; `seen` records
 * every request that is not a file of the page, so a test can say what the page sent, and where.
 */
async function serve(page, { nodes = [], signedIn = true } = {}) {
  const seen = [];
  const local = { nodes, rels: [] };
  await page.route(`${ORIGIN}/**`, async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const json = (body, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    if (url.pathname === '/explorer-config.json') {
      return json({
        backend: 'native', ui: 'cli',
        local: { label: 'local server · zega-data', detail: '/home/someone/zega-data' },
        remote: { base: '/_zega/remote', graphs: '/_zega/graphs' },
      });
    }
    if (url.pathname === '/_zega/graphs') {
      seen.push({ method: request.method(), path: url.pathname });
      return signedIn ? json({ ok: true, graphs: [{ id: GRAPH, name: 'Atlas hockey', project: 'Atlas' }] }) : json({ ok: false, error: 'not signed in' }, 401);
    }
    if (url.pathname.startsWith('/_zega/remote/')) {
      seen.push({ method: request.method(), path: url.pathname, authorization: request.headers().authorization ?? null });
      if (url.pathname === `/_zega/remote/${GRAPH}/graph`) {
        return json({ ok: true, result: { nodes: [{ id: 1, labels: ['Player'], name: 'Ada Remote' }], rels: [] } });
      }
      return json({ ok: false, error: 'not found' }, 404);
    }
    if (url.pathname === '/graph') {
      seen.push({ method: request.method(), path: url.pathname });
      if (request.method() === 'DELETE') { local.nodes = []; return json({ ok: true, result: null }); }
      return json({ ok: true, result: local });
    }
    const file = resolve(DIST, url.pathname === '/' ? 'index.html' : `.${decodeURIComponent(url.pathname)}`);
    if (!file.startsWith(DIST + sep)) return route.abort();
    try {
      return route.fulfill({ status: 200, contentType: TYPES[extname(file)] ?? 'application/octet-stream', body: await readFile(file) });
    } catch {
      seen.push({ method: request.method(), path: url.pathname, missing: true });
      return route.fulfill({ status: 404, body: '' });
    }
  });
  return seen;
}

async function open(page, options) {
  const seen = await serve(page, options);
  await page.goto(`${ORIGIN}/`);
  await expect(page.locator('#query .monaco-editor')).toBeVisible({ timeout: 45000 });
  return seen;
}

const paneText = (page, id) => page.evaluate((id) => window.monaco.editor.getEditors().find((e) => e.getDomNode()?.closest(id)).getValue(), id);

test('has none of the website: no links out, no sample buttons, no examples bar, no reset', async ({ page }) => {
  const seen = await open(page);
  await expect(page.locator('a')).toHaveCount(0);
  for (const id of ['flights', 'tickets', 'cities', 'westeros', 'calgary', 'seed']) await expect(page.locator(`#btn-${id}`)).toHaveCount(0);
  await expect(page.locator('header#topbar')).not.toContainText(/zega\.dev|back to|Flights|Westeros|Calgary|Cities|Tickets/);
  await expect(page.locator('#tour')).toBeHidden();
  expect(await page.content()).not.toContain('zega.dev');
  expect(seen.filter((r) => r.path.startsWith('/samples'))).toEqual([]);
  // The bar is the connection, the way to switch it, and the page's own controls.
  await expect(page.locator('.conn')).toContainText('local server · zega-data');
  await expect(page.locator('#btn-remote')).toBeVisible();
  await expect(page.locator('#btn-remote')).toHaveText('Connect to remote graph');
});

test('an empty database says so, shows how to load data, and opens on empty panes (not the hockey example)', async ({ page }) => {
  await open(page);
  await expect(page.locator('.empty-state h3')).toHaveText('This database is empty.');
  await expect(page.locator('.empty-state')).toContainText('Import');
  await expect(page.locator('.empty-state pre')).toContainText('type Note { text: String }');
  await expect(page.locator('.empty-state pre')).toContainText('mutation { Note(text: "hello") }');
  expect(await paneText(page, '#schema')).toBe('');
  expect(await paneText(page, '#query')).toBe('');
  expect(await page.content()).not.toContain('Connor McDavid');
});

test('the example the empty state shows is valid ZQL: the engine accepts it and returns the node', async ({ page }) => {
  await open(page);
  // The stand-in keeps no engine, so the page's own wasm parser is what is asked: it must accept both texts.
  const accepted = await page.evaluate(async () => {
    const { default: init, ZegaWasm } = await import('/pkg/zega_wasm.js');
    await init();
    const db = new ZegaWasm();
    const schema = document.querySelector('.empty-state pre').textContent.split('\n');
    const type = schema[1];
    const mutation = schema[4];
    db.run(type, mutation);
    return JSON.parse(db.run(type, '{ Note { text } }'));
  });
  expect(accepted).toEqual([{ text: 'hello' }]);
});

test('stored data with no schema written down says how many nodes, and points at the schema pane', async ({ page }) => {
  await open(page, { nodes: [{ id: 1, labels: ['Note'], text: 'hello' }, { id: 2, labels: ['Note'], text: 'again' }] });
  await expect(page.locator('.empty-state h3')).toHaveText('This database stores 2 nodes and 0 relationships.');
  await expect(page.locator('.empty-state')).toContainText('schema pane');
  await expect(page.locator('#raw-count')).toContainText('2 nodes');
});

test('connects to a cloud graph by its name through the command, never to api.zega.dev, and disconnects back', async ({ page }) => {
  const seen = await open(page);
  const external = [];
  page.on('request', (request) => { if (!request.url().startsWith(ORIGIN) && !request.url().startsWith('data:')) external.push(request.url()); });
  await page.locator('#btn-remote').click();
  await expect(page.locator('#remote-dialog')).toBeVisible();
  await expect(page.locator('.remote-note')).toContainText('The zega command passes your requests on to the graph');
  await expect(page.locator('#remote-dialog')).not.toContainText('api.zega.dev');
  // The account's graphs are offered by name.
  await expect.poll(() => page.locator('#remote-graphs option').evaluateAll((o) => o.map((x) => [x.value, x.label]))).toEqual([[GRAPH, 'Atlas hockey (Atlas)']]);
  await page.locator('#remote-id').fill(GRAPH);
  await page.locator('#remote-key').fill(KEY);
  await page.locator('#remote-connect').click();
  await expect(page.locator('.conn')).toContainText('cloud graph · Atlas hockey');
  await expect(page.locator('#btn-remote')).toHaveText('Disconnect');
  await expect(page.locator('#btn-push-schema')).toBeVisible();
  // The page asked the command, with the key; nothing went to the cloud from the browser, and the key is nowhere it is kept.
  const remote = seen.filter((r) => r.path.startsWith('/_zega/remote/'));
  expect(remote[0]).toEqual({ method: 'GET', path: `/_zega/remote/${GRAPH}/graph`, authorization: `Bearer ${KEY}` });
  expect(external.filter((url) => !url.includes('cdn.jsdelivr.net'))).toEqual([]);
  const stored = await page.evaluate(() => JSON.stringify([{ ...localStorage }, { ...sessionStorage }, document.cookie, location.href]));
  expect(stored).not.toContain(KEY);

  await page.locator('#btn-remote').click();
  await expect(page.locator('.conn')).toContainText('local server · zega-data');
  await expect(page.locator('#btn-remote')).toHaveText('Connect to remote graph');
});

test('without a signed-in command the graph is typed by its id, and still connects', async ({ page }) => {
  await open(page, { signedIn: false });
  await page.locator('#btn-remote').click();
  await expect(page.locator('#remote-graphs option')).toHaveCount(0);
  await page.locator('#remote-id').fill(GRAPH);
  await page.locator('#remote-key').fill(KEY);
  await page.locator('#remote-connect').click();
  await expect(page.locator('.conn')).toContainText(`cloud graph · ${GRAPH}`);
});

test('clear asks before it deletes the local database, and deletes nothing when declined', async ({ page }) => {
  const seen = await open(page, { nodes: [{ id: 1, labels: ['Note'], text: 'hello' }] });
  const asked = [];
  page.on('dialog', (dialog) => { asked.push(dialog.message()); dialog.dismiss(); });
  await page.locator('#btn-clear').click();
  await expect.poll(() => asked.length).toBe(1);
  expect(asked[0]).toBe('Delete every node and relationship in the local database? This cannot be undone.');
  expect(seen.filter((r) => r.method === 'DELETE')).toEqual([]);

  page.removeAllListeners('dialog');
  page.on('dialog', (dialog) => dialog.accept());
  await page.locator('#btn-clear').click();
  await expect.poll(() => seen.filter((r) => r.method === 'DELETE').length).toBe(1);
  await expect(page.locator('.empty-state h3')).toHaveText('This database is empty.');
});

test('the standalone site is unchanged: it still has its links, samples and examples', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('a[href="https://zega.dev"]').first()).toBeVisible();
  for (const id of ['flights', 'tickets', 'cities', 'westeros', 'calgary', 'seed']) await expect(page.locator(`#btn-${id}`)).toBeVisible();
  await expect(page.locator('#btn-remote')).toBeVisible();
  await expect(page.locator('.conn')).toContainText('local · wasm');
});
