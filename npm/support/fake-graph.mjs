// A stand-in for a Zega Cloud graph's address (`https://<graph>.zegadb.com`),
// for the client's tests. It answers with the shapes the real router sends
// (zegadb/cloud src/router.ts, query-method.ts, keys.ts, rate-limit.ts, schema.ts):
//
//   QUERY /zql   reads only; a write is 422 `not_a_read`; a refused statement is 422
//   POST  /zql   the same statement, a refused one is 400
//   GET   /schema
//   401 `missing_key` / `invalid_key`, 429 + Retry-After, 402 `spending_cap_reached`
//
// What it does NOT do is run ZQL: a read or write answers with an echo of what
// the client sent, so a test can assert exactly that. The real engine is
// covered by the same tests against a real zega-server (check-client.mjs).
//
// The statement text steers it, so tests need no side channel:
//   NOPE     the engine refuses it (the real server's text for a misspelt field)
//   LIMITED  429 with Retry-After: 7
//   CAPPED   402 spending_cap_reached
//   DOWN     a proxy's HTML 502
//   NOTZEGA  200 with a body that is not a zega answer
//   SLOW     no answer for 5 s
//
// `queryMethod` is how old a target to imitate: 'supported' (a current router
// or zega-server), 'router-404' (a Zega Cloud router before QUERY: 404 not_found)
// or 'server-405' (a zega-server before QUERY: 405, and no /schema route at all).
import { createServer } from 'node:http';

export const KEY = 'zk_' + 'abcdefghijklmnopqrstuvwxyz234567';
export const SCHEMA = 'type Person { name: String }';
export const ENGINE_ERROR = 'execution error: error: Person has no field nam\n  query:1:12\n  { Person { nam } }\n             ^^^\n  help: did you mean `name`?';

const problem = (response, status, code, error, headers = {}) => {
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store', ...headers });
  response.end(JSON.stringify(code === undefined ? { ok: false, error } : { ok: false, error, code }));
};
const answer = (response, result) => {
  response.writeHead(200, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' });
  response.end(JSON.stringify({ ok: true, result }));
};

/** Whether the statement writes: the router's own scan, in miniature (`mutation` at the top level, outside strings). */
export function writes(source) {
  return /^\s*mutation\b/m.test(source.replace(/"(?:[^"\\]|\\.)*"/g, '""'));
}

export async function startFakeGraph({ queryMethod = 'supported', key = KEY } = {}) {
  const requests = [];
  let storedSchema = SCHEMA;
  const server = createServer((request, response) => {
    const chunks = [];
    request.on('data', chunk => chunks.push(chunk));
    request.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8');
      let body;
      try { body = JSON.parse(text); } catch { /* no body, or not JSON */ }
      const seen = { method: request.method, path: request.url, headers: request.headers, body };
      requests.push(seen);
      const bearer = /^Bearer (\S+)$/.exec(request.headers.authorization ?? '')?.[1];
      if (bearer === undefined) return problem(response, 401, 'missing_key', 'Send the graph API key as "Authorization: Bearer zk_…".');
      if (bearer !== key) return problem(response, 401, 'invalid_key', 'That is not a valid key for this graph.');

      if (request.url === '/schema' && request.method === 'GET') {
        if (queryMethod === 'server-405') { response.writeHead(404, { 'content-length': 0 }); return response.end(); }
        return answer(response, { schema: storedSchema, builtin: ['Auth'], updatedAt: '2026-10-03T00:00:00.000Z' });
      }
      if (request.url === '/schema' && request.method === 'PUT') {
        storedSchema = body.schema;
        return answer(response, { schema: storedSchema, builtin: ['Auth'], updatedAt: '2026-10-03T00:00:00.000Z' });
      }
      if (request.url !== '/zql') return problem(response, 404, 'not_found', `zega does not serve ${request.method} ${request.url}.`);
      const isQuery = request.method === 'QUERY';
      if (isQuery && queryMethod === 'router-404') return problem(response, 404, 'not_found', 'zega does not serve QUERY /zql.');
      if (isQuery && queryMethod === 'server-405') {
        response.writeHead(405, { allow: 'POST', 'content-length': 0 });
        return response.end();
      }
      if (!isQuery && request.method !== 'POST') return problem(response, 405, 'method_not_allowed', `${request.method} is not supported on zql.`);
      if (typeof body?.query !== 'string') return problem(response, 400, 'bad_content', 'The content is not a ZQL request.');

      const source = body.query;
      if (source.includes('LIMITED')) return problem(response, 429, 'rate_limited', 'Too many requests. Try again in 7 seconds.', { 'retry-after': '7' });
      if (source.includes('CAPPED')) return problem(response, 402, 'spending_cap_reached', 'This graph has reached its spending cap for October.');
      if (source.includes('DOWN')) {
        response.writeHead(502, { 'content-type': 'text/html' });
        return response.end('<html><body>502 Bad Gateway</body></html>');
      }
      if (source.includes('NOTZEGA')) { response.writeHead(200, { 'content-type': 'text/html' }); return response.end('<html>hello</html>'); }
      if (source.includes('SLOW')) { setTimeout(() => answer(response, null), 5000).unref(); return; }
      if (isQuery && writes(source)) {
        return problem(response, 422, 'not_a_read', 'QUERY /zql runs reads only. This is a mutation, or could not be told from one. Send mutations with POST /zql.');
      }
      if (source.includes('NOPE')) {
        // the machine says 400; the router turns it into 422 for QUERY (RFC 10008)
        return problem(response, isQuery ? 422 : 400, undefined, ENGINE_ERROR);
      }
      return answer(response, { echo: { query: source, schema: body.schema, document: body.document } });
    });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address();
  return {
    url: `http://127.0.0.1:${port}`,
    key,
    requests,
    async close() {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
    },
  };
}
