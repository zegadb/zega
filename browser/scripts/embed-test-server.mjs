import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { resolve, sep } from 'node:path';

const root = resolve('dist-embed');
const server = createServer(async (request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  if (url.pathname === '/') {
    response.setHeader('content-type', 'text/html; charset=utf-8');
    response.end(`<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head><body><header id="dashboard-chrome"></header><main><div id="explorer"></div></main><script type="module">import {mountExplorer} from '/embed/embed.js';try{await mountExplorer(document.querySelector('#explorer'),{database:'/api',assetBase:'/embed',chrome:document.querySelector('#dashboard-chrome'),label:'Local explorer'});document.documentElement.dataset.ready='yes'}catch(error){document.documentElement.dataset.error=String(error)}</script></body></html>`);
    return;
  }
  if (url.pathname.startsWith('/api/')) {
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ ok: true, result: url.pathname.endsWith('/graph') ? { nodes: [], rels: [] } : { data: [] } }));
    return;
  }
  if (url.pathname.startsWith('/embed/')) {
    const file = resolve(root, url.pathname.slice('/embed/'.length));
    if (!file.startsWith(`${root}${sep}`) && file !== resolve(root, 'embed.js')) { response.writeHead(404).end(); return; }
    try {
      if (!(await stat(file)).isFile()) { response.writeHead(404).end(); return; }
      const extension = file.split('.').at(-1);
      const types = { js: 'text/javascript', css: 'text/css', html: 'text/html', wasm: 'application/wasm', ttf: 'font/ttf', json: 'application/json', geojson: 'application/geo+json', pbf: 'application/x-protobuf', png: 'image/png' };
      response.setHeader('content-type', types[extension] ?? 'application/octet-stream');
      response.end(await readFile(file));
    } catch { response.writeHead(404).end(); }
    return;
  }
  response.writeHead(404).end();
});
server.listen(8790, '127.0.0.1');
process.on('SIGTERM', () => server.close(() => process.exit(0)));
