// What serves the explorer page against a native database in these tests.
//
// `zega-server explorer` used to; the explorer now belongs to the `zega` command (zegadb/cli:
// `zega explorer`), and this binary is the database only. So the test starts the database
// (`zega-server start`) and serves dist/ in front of it the way that command does: the page's files,
// `/explorer-config.json` saying the backend is native, and every other request passed on to the
// database. The page and the engine are the real ones; only the few lines of serving are a stand-in.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { extname, resolve, sep } from 'node:path';
import { createInterface } from 'node:readline';

const DIST = resolve('dist');
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.wasm': 'application/wasm', '.json': 'application/json', '.geojson': 'application/geo+json', '.ttf': 'font/ttf' };

export async function start(directory) {
  const database = spawn(resolve('../.target/debug/zega-server'), ['start', '--port', '0', '--data', directory, '--query-time-limit', '0'], { cwd: directory, stdio: ['ignore', 'pipe', 'pipe'] });
  let stderr = '';
  database.stderr.on('data', (data) => { stderr += data; });
  const upstream = await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => { database.kill(); reject(Error(`database startup timed out: ${stderr}`)); }, 15000);
    database.once('error', (error) => { clearTimeout(timeout); reject(error); });
    database.once('exit', (code) => { clearTimeout(timeout); reject(Error(`database exited ${code}: ${stderr}`)); });
    createInterface({ input: database.stdout }).once('line', (line) => { clearTimeout(timeout); resolve(line); });
  });

  const web = createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    if (request.method === 'GET' && url.pathname === '/explorer-config.json') {
      response.writeHead(200, { 'content-type': 'application/json' });
      return response.end(JSON.stringify({ backend: 'native' }));
    }
    if (request.method === 'GET') {
      const file = resolve(DIST, url.pathname === '/' ? 'index.html' : `.${decodeURIComponent(url.pathname)}`);
      if (file.startsWith(DIST + sep)) {
        try {
          const body = await readFile(file);
          response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'text/plain; charset=utf-8', 'cache-control': 'no-cache' });
          return response.end(body);
        } catch { /* not a file of the page: the database answers */ }
      }
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const headers = {};
    for (const name of ['accept', 'content-type']) if (request.headers[name]) headers[name] = request.headers[name];
    try {
      const answer = await fetch(upstream + url.pathname + url.search, { method: request.method, headers, body: ['GET', 'HEAD'].includes(request.method) ? undefined : Buffer.concat(chunks) });
      const body = Buffer.from(await answer.arrayBuffer());
      response.writeHead(answer.status, { 'content-type': answer.headers.get('content-type') ?? 'application/octet-stream' });
      response.end(body);
    } catch (error) {
      response.writeHead(502);
      response.end(String(error));
    }
  });
  await new Promise((resolve) => web.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${web.address().port}`;
  return {
    url,
    async stop() {
      web.close();
      if (database.exitCode !== null) return;
      const exit = new Promise((resolve) => database.once('exit', resolve));
      database.kill('SIGINT');
      await exit;
    },
  };
}
