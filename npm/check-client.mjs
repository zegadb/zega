// @zegadb/lib/client against a Zega Cloud stand-in, and, with --server, against
// a real zega-server.
//
//   node npm/check-client.mjs                             the stand-in only (part of `npm test`)
//   node npm/check-client.mjs --server .target/debug/zega-server
//                                                         also a real server: it is started, and
//                                                         failing to start it fails the run
//
// It tests npm/src/client.js, the file the package ships. The packed tarball's
// own client is exercised by check-consumers.mjs.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { after, before, describe, test } from 'node:test';
import { ZegaError, ZegaNetworkError, connect, zql } from './src/client.js';
import { ENGINE_ERROR, KEY, SCHEMA, startFakeGraph } from './support/fake-graph.mjs';

const root = resolve(import.meta.dirname, '..');
const flag = process.argv.indexOf('--server');
const serverBinary = flag === -1 ? undefined : process.argv[flag + 1];
if (flag !== -1 && !serverBinary) throw new Error('--server needs the path of a zega-server executable');

async function rejection(promise) {
  try { await promise; } catch (error) { return error; }
  assert.fail('expected the call to fail');
}

describe('client against a Zega Cloud stand-in', () => {
  let graph;
  const zega = options => connect({ url: graph.url, key: graph.key, schema: SCHEMA, ...options });
  const sent = () => graph.requests.map(({ method, path }) => `${method} ${path}`);
  before(async () => { graph = await startFakeGraph(); });
  after(() => graph.close());

  test('a read is QUERY /zql with the key as a bearer, and the result comes back unwrapped', async () => {
    graph.requests.length = 0;
    const result = await zega().query('{ Person { name } }');
    assert.deepEqual(result, { echo: { query: '{ Person { name } }', schema: SCHEMA } });
    const [request] = graph.requests;
    assert.equal(request.method, 'QUERY');
    assert.equal(request.path, '/zql');
    assert.equal(request.headers.authorization, `Bearer ${KEY}`);
    assert.match(request.headers['content-type'], /^application\/json/);
    assert.deepEqual(request.body, { query: '{ Person { name } }', schema: SCHEMA });
  });

  test('a write is POST /zql', async () => {
    graph.requests.length = 0;
    await zega().mutate('mutation { Person(name: "Ada") { name } }');
    assert.deepEqual(sent(), ['POST /zql']);
  });

  test("a call's schema replaces the connection's, and a document is flagged as one", async () => {
    const other = 'type Team { name: String }';
    const { echo } = await zega().query('{ Team { name } }', { schema: other });
    assert.equal(echo.schema, other);
    const doc = await connect({ url: graph.url, key: graph.key }).query('schema { type A { b: Int } }\nquery { A { b } }', { document: true });
    assert.equal(doc.echo.document, true);
    assert.equal(doc.echo.schema, undefined);
  });

  test('without a key no Authorization header is sent', async () => {
    graph.requests.length = 0;
    const error = await rejection(connect({ url: graph.url }).query('{ Person { name } }'));
    assert.equal(graph.requests[0].headers.authorization, undefined);
    assert.equal(error.code, 'unauthorized');
    assert.equal(error.serverCode, 'missing_key');
  });

  test('a write sent to query() is refused as not_a_read, not run', async () => {
    const error = await rejection(zega().query('mutation { Person(name: "Ada") { name } }'));
    assert.ok(error instanceof ZegaError && !(error instanceof ZegaNetworkError));
    assert.equal(error.code, 'not_a_read');
    assert.equal(error.status, 422);
    assert.equal(error.serverCode, 'not_a_read');
  });

  test('a ZQL error keeps the engine text, its help and its position, on QUERY (422) and POST (400)', async () => {
    for (const [call, status] of [['query', 422], ['mutate', 400]]) {
      const error = await rejection(zega()[call]('{ Person { NOPE } }'));
      assert.equal(error.code, 'query');
      assert.equal(error.status, status);
      assert.equal(error.message, ENGINE_ERROR);
      assert.equal(error.help, 'did you mean `name`?');
      assert.deepEqual(error.location, { source: 'query', line: 1, column: 12 });
    }
  });

  test('a wrong key is unauthorized with the router code, and the key is nowhere in the error', async () => {
    const wrong = 'zk_' + '7'.repeat(32);
    const error = await rejection(connect({ url: graph.url, key: wrong }).query('{ a }'));
    assert.equal(error.code, 'unauthorized');
    assert.equal(error.status, 401);
    assert.equal(error.serverCode, 'invalid_key');
    for (const text of [error.message, error.stack, JSON.stringify(error, Object.getOwnPropertyNames(error)), String(error.cause)]) {
      assert.ok(!text.includes(wrong));
    }
  });

  test('429 carries retryAfter, 402 the server code, a proxy page is a server error, a non-zega 200 is bad_response', async () => {
    const limited = await rejection(zega().query('{ LIMITED }'));
    assert.deepEqual([limited.code, limited.status, limited.retryAfter], ['rate_limited', 429, 7]);
    const capped = await rejection(zega().mutate('{ CAPPED }'));
    assert.deepEqual([capped.code, capped.status, capped.serverCode], ['http', 402, 'spending_cap_reached']);
    const down = await rejection(zega().query('{ DOWN }'));
    assert.deepEqual([down.code, down.status], ['server', 502]);
    assert.match(down.message, /^HTTP 502.*Bad Gateway/);
    const notZega = await rejection(zega().query('{ NOTZEGA }'));
    assert.deepEqual([notZega.code, notZega.status], ['bad_response', 200]);
  });

  test('an unreachable server is a ZegaNetworkError, not a query error', async () => {
    const closed = await startFakeGraph();
    const { url } = closed;
    await closed.close();
    const error = await rejection(connect({ url, key: KEY }).query('{ a }'));
    assert.ok(error instanceof ZegaNetworkError && error instanceof ZegaError);
    assert.deepEqual([error.code, error.status], ['network', 0]);
    assert.ok(error.cause);
    assert.ok(error.message.includes(new URL(url).origin));
  });

  test('a signal cancels the call: abort() is aborted, AbortSignal.timeout() is timeout, and neither retries', async () => {
    graph.requests.length = 0;
    const started = Date.now();
    const timedOut = await rejection(zega().query('{ SLOW }', { signal: AbortSignal.timeout(80) }));
    assert.deepEqual([timedOut.constructor, timedOut.code, timedOut.status], [ZegaNetworkError, 'timeout', 0]);
    const controller = new AbortController();
    const pending = rejection(zega().mutate('{ SLOW }', { signal: controller.signal }));
    setTimeout(() => controller.abort(), 50);
    const aborted = await pending;
    assert.deepEqual([aborted.constructor, aborted.code], [ZegaNetworkError, 'aborted']);
    assert.ok(Date.now() - started < 3000, 'the calls waited for the 5 s answer');
    assert.deepEqual(sent(), ['QUERY /zql', 'POST /zql']);
    graph.requests.length = 0;
    const early = await rejection(zega().query('{ a }', { signal: AbortSignal.abort() }));
    assert.equal(early.code, 'aborted');
    assert.deepEqual(sent(), []);
  });

  test('the fetch option is the only thing used to reach the server', async () => {
    const real = globalThis.fetch;
    globalThis.fetch = () => { throw new Error('global fetch used'); };
    try {
      const calls = [];
      const zegaFetch = connect({
        url: 'https://abc.zegadb.com/', key: KEY, schema: SCHEMA,
        fetch: async (input, init) => {
          calls.push([input, init.method]);
          return new Response(JSON.stringify({ ok: true, result: [1] }), { headers: { 'content-type': 'application/json' } });
        },
      });
      assert.equal(zegaFetch.url, 'https://abc.zegadb.com');
      assert.deepEqual(await zegaFetch.query('{ a }'), [1]);
      assert.deepEqual(calls, [['https://abc.zegadb.com/zql', 'QUERY']]);
    } finally { globalThis.fetch = real; }
  });

  test('a url with a path (api.zega.dev/g/<id>) keeps it', async () => {
    const calls = [];
    const via = connect({ url: 'https://api.zega.dev/g/my-graph', fetch: async input => { calls.push(input); return Response.json({ ok: true, result: null }); } });
    assert.equal(await via.mutate('{ a }'), null);
    assert.deepEqual(calls, ['https://api.zega.dev/g/my-graph/zql']);
  });

  test('connect() refuses what it cannot use, before any request, without echoing the key', () => {
    const key = 'zk_secret value';
    for (const options of [undefined, {}, { url: '' }, { url: 'not a url' }, { url: 'ftp://x' }, { url: 'https://x/?k=1' }, { url: 'https://u:p@x' },
      { url: 'https://x', key }, { url: 'https://x', schema: 5 }, { url: 'https://x', fetch: 1 }, { url: 'https://x', readMethod: 'GET' }]) {
      assert.throws(() => connect(options), error => error instanceof ZegaError && error.code === 'config' && !error.message.includes(key), JSON.stringify(options));
    }
  });

  describe('a target without the QUERY method', () => {
    for (const queryMethod of ['router-404', 'server-405']) {
      test(`${queryMethod}: query() falls back to POST once and remembers`, async () => {
        const old = await startFakeGraph({ queryMethod });
        try {
          const zegaOld = connect({ url: old.url, key: KEY, schema: SCHEMA });
          assert.deepEqual((await zegaOld.query('{ a }')).echo.query, '{ a }');
          assert.deepEqual((await zegaOld.query('{ b }')).echo.query, '{ b }');
          assert.deepEqual(old.requests.map(r => r.method), ['QUERY', 'POST', 'POST']);
          // readMethod pins the choice: 'query' reports the refusal, 'post' never asks
          const pinned = await rejection(connect({ url: old.url, key: KEY, readMethod: 'query' }).query('{ a }'));
          assert.deepEqual([pinned.code, pinned.status], ['http', queryMethod === 'server-405' ? 405 : 404]);
          old.requests.length = 0;
          await connect({ url: old.url, key: KEY, readMethod: 'post' }).query('{ a }');
          assert.deepEqual(old.requests.map(r => r.method), ['POST']);
        } finally { await old.close(); }
      });
    }

    test('a refusal that is not "no QUERY" is never retried as POST', async () => {
      graph.requests.length = 0;
      await rejection(zega().query('{ Person { NOPE } }'));
      await rejection(connect({ url: graph.url, key: 'zk_' + '2'.repeat(32) }).query('{ a }'));
      assert.deepEqual(sent(), ['QUERY /zql', 'QUERY /zql']);
    });
  });

  test('schema() reads the graph\'s stored schema; a server with no /schema route says what to do instead', async () => {
    assert.deepEqual(await zega().schema(), { schema: SCHEMA, updatedAt: '2026-10-03T00:00:00.000Z' });
    const local = await startFakeGraph({ queryMethod: 'server-405' });
    try {
      const error = await rejection(connect({ url: local.url, key: KEY }).schema());
      assert.deepEqual([error.code, error.status], ['http', 404]);
      assert.match(error.message, /keeps no schema; pass `schema` to connect\(\)/);
    } finally { await local.close(); }
  });
});

describe('zql tagged template', () => {
  test('writes strings, numbers, bigints, booleans and null as literals', () => {
    assert.equal(zql`Person(name: ${'Ada'} && age: ${37} && ratio: ${-1.5e-7} && big: ${12n} && on: ${true} && x: ${null})`,
      'Person(name: "Ada" && age: 37 && ratio: -1.5e-7 && big: 12 && on: true && x: null)');
    assert.equal(zql`{ Person }`, '{ Person }');
  });

  test('escapes what ends a string, so a value cannot become ZQL', () => {
    assert.equal(zql`${'a"b\\c\nd\te'}`, '"a\\"b\\\\c\\nd\\te"');
    const hostile = '"}) } mutation { Person(name: "pwned';
    assert.equal(zql`{ Person(name: ${hostile}) { name } }`.match(/(?<!\\)"/g).length, 2);
  });

  test('refuses what has no literal form', () => {
    for (const value of [undefined, NaN, Infinity, {}, [], () => 1, Symbol('s')]) assert.throws(() => zql`${value}`, TypeError);
  });
});

if (serverBinary) {
  describe(`client against a real zega-server (${serverBinary})`, () => {
    const TOKEN = 'zk_' + 'real-server-token';
    let scratch;
    const running = [];

    /** Starts `zega-server start`, resolves to its address once it prints it. Port 0: the OS picks one. */
    async function start(...args) {
      const child = spawn(resolve(root, serverBinary), ['start', '--data', join(scratch, `data-${running.length}`), '--port', '0', ...args], { stdio: ['ignore', 'pipe', 'pipe'] });
      running.push(child);
      let output = '';
      const url = await new Promise((done, fail) => {
        child.once('error', fail);
        child.once('exit', code => fail(new Error(`zega-server exited with ${code} before it printed its address: ${output}`)));
        child.stdout.on('data', chunk => {
          output += chunk;
          const found = /http:\/\/127\.0\.0\.1:\d+/.exec(output);
          if (found) done(found[0]);
        });
        child.stderr.on('data', chunk => { output += chunk; });
      });
      child.removeAllListeners('exit');
      return { url, child };
    }

    before(async () => {
      await mkdir(join(root, '.tmp'), { recursive: true });
      scratch = await mkdtemp(join(root, '.tmp', 'client-'));
      await writeFile(join(scratch, 'token'), `${TOKEN}\n`);
    });
    after(() => { for (const child of running) child.kill(); });

    test('write, read, escaped values, a ZQL error with its help and position, a write refused by QUERY, bad auth', async () => {
      const { url } = await start('--token-file', join(scratch, 'token'));
      const zega = connect({ url, key: TOKEN, schema: SCHEMA });

      assert.deepEqual(await zega.mutate('mutation { Person(name: "Ada") { name } }'), { name: 'Ada' });
      assert.deepEqual(await zega.query('{ Person { name } }'), [{ name: 'Ada' }]);

      const awkward = ['O"Brien', 'back\\slash', 'two\nlines', 'tab\there', 'mutation { Person(name: "x") }', 'ünï — 日本語 😀'];
      for (const name of awkward) assert.deepEqual(await zega.mutate(zql`mutation { Person(name: ${name}) { name } }`), { name });
      const names = (await zega.query('{ Person { name } }')).map(person => person.name);
      assert.deepEqual(names, ['Ada', ...awkward]);

      const typo = await rejection(zega.query('{ Person { nam } }'));
      assert.deepEqual([typo.code, typo.status, typo.help, typo.location], ['query', 422, 'did you mean `name`?', { source: 'query', line: 1, column: 12 }]);
      assert.match(typo.message, /^execution error: error: Person has no field nam\n/);
      const typoPost = await rejection(zega.mutate('{ Person { nam } }'));
      assert.deepEqual([typoPost.code, typoPost.status, typoPost.help], ['query', 400, 'did you mean `name`?']);

      // QUERY really is what read: a POST would have run this and added Bo
      const refused = await rejection(zega.query('mutation { Person(name: "Bo") { name } }'));
      assert.deepEqual([refused.code, refused.status, refused.serverCode], ['not_a_read', 422, 'not_a_read']);
      assert.equal((await zega.query('{ Person { name } }')).length, names.length);
      // a connection pinned to POST sends reads as POST, and so lets the write through
      const viaPost = connect({ url, key: TOKEN, schema: SCHEMA, readMethod: 'post' });
      assert.deepEqual(await viaPost.query('mutation { Person(name: "Bo") { name } }'), { name: 'Bo' });
      assert.equal((await zega.query('{ Person { name } }')).length, names.length + 1);

      for (const key of [undefined, 'zk_wrong']) {
        const denied = await rejection(connect({ url, key, schema: SCHEMA }).query('{ Person { name } }'));
        assert.deepEqual([denied.code, denied.status, denied.message], ['unauthorized', 401, 'unauthorized']);
        const deniedWrite = await rejection(connect({ url, key, schema: SCHEMA }).mutate('mutation { Person(name: "no") { name } }'));
        assert.equal(deniedWrite.code, 'unauthorized');
      }

      const noSchema = await rejection(connect({ url, key: TOKEN }).query('{ Person { name } }'));
      assert.deepEqual([noSchema.code, noSchema.help], ['query', 'start with `type Name { }`']);
      const rows = await connect({ url, key: TOKEN }).query('schema { type Person { name: String } }\nquery { Person { name } }', { document: true });
      assert.equal(rows.length, names.length + 1);

      const stored = await rejection(zega.schema());
      assert.deepEqual([stored.code, stored.status], ['http', 404]);

      const aborted = await rejection(zega.query('{ Person { name } }', { signal: AbortSignal.abort() }));
      assert.deepEqual([aborted.constructor, aborted.code], [ZegaNetworkError, 'aborted']);
    });

    test('a server started without a token takes a client without a key', async () => {
      const { url } = await start();
      const zega = connect({ url, schema: SCHEMA });
      await zega.mutate('mutation { Person(name: "Ada") { name } }');
      assert.deepEqual(await zega.query('{ Person { name } }'), [{ name: 'Ada' }]);
    });

    test('a server that has stopped is a network error', async () => {
      const { url, child } = await start();
      const zega = connect({ url, schema: SCHEMA });
      await zega.query('{ Person { name } }').catch(() => {});
      await new Promise(done => { child.once('exit', done); child.kill(); });
      const error = await rejection(zega.query('{ Person { name } }'));
      assert.deepEqual([error.constructor, error.code], [ZegaNetworkError, 'network']);
    });
  });
}
