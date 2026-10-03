// @zegadb/lib/client: talk to a running zega over HTTP.
//
// `fetch` only. No dependencies, no wasm, no Node built-ins, so it bundles to a
// few KB and runs on Workers, Node, Bun, Deno and in browsers. Nothing here may
// import the engine: check-consumers.mjs bundles a sample function and fails if
// the wasm or its glue shows up.

/** An answer the server gave, but not the one wanted. `code` says which kind; client.d.ts lists them. */
export class ZegaError extends Error {
  /**
   * @param {string} code
   * @param {string} message
   * @param {{ status?: number, serverCode?: string, help?: string, location?: object, retryAfter?: number, cause?: unknown }} [details]
   */
  constructor(code, message, details = {}) {
    super(message, details.cause === undefined ? undefined : { cause: details.cause });
    this.name = 'ZegaError';
    this.code = code;
    this.status = details.status ?? 0;
    if (details.serverCode !== undefined) this.serverCode = details.serverCode;
    if (details.help !== undefined) this.help = details.help;
    if (details.location !== undefined) this.location = details.location;
    if (details.retryAfter !== undefined) this.retryAfter = details.retryAfter;
  }
}

/** The request never produced an answer: the network failed, or the caller's signal fired. */
export class ZegaNetworkError extends ZegaError {
  constructor(code, message, cause) {
    super(code, message, { cause });
    this.name = 'ZegaNetworkError';
  }
}

const ESCAPES = { '\\': '\\\\', '"': '\\"', '\n': '\\n', '\t': '\\t' };

function literal(value) {
  switch (typeof value) {
    case 'string': return `"${value.replace(/[\\"\n\t]/g, c => ESCAPES[c])}"`;
    case 'number':
      if (Number.isFinite(value)) return String(value);
      break;
    case 'bigint':
    case 'boolean': return String(value);
    case 'object':
      if (value === null) return 'null';
  }
  throw new TypeError(`zql: cannot write ${typeof value === 'number' ? value : `a ${typeof value}`} into ZQL; use a string, finite number, bigint, boolean or null`);
}

/**
 * Tagged template that writes values into ZQL as literals, escaped. ZQL has no
 * query parameters, so this is how user input goes into a statement safely.
 */
export function zql(strings, ...values) {
  let out = strings[0];
  for (let i = 0; i < values.length; i++) out += literal(values[i]) + strings[i + 1];
  return out;
}

function config(message) {
  return new ZegaError('config', `zega connect(): ${message}`);
}

/** `help:` and `query:2:17` lines of an engine error, which keeps the whole text in `message`. */
function engineDetails(message) {
  const help = /^\s*help: (.+)$/m.exec(message)?.[1];
  const at = /^\s+([a-z]+):(\d+):(\d+)\s*$/m.exec(message);
  return { help, location: at ? { source: at[1], line: Number(at[2]), column: Number(at[3]) } : undefined };
}

function codeFor(status, serverCode) {
  if (status === 401 || status === 403) return 'unauthorized';
  if (serverCode === 'not_a_read') return 'not_a_read';
  if (status === 429) return 'rate_limited';
  if (status === 400 || status === 422) return 'query';
  if (status >= 500) return 'server';
  return 'http';
}

function unreachable(error, signal, origin) {
  if (signal?.aborted) {
    const timedOut = signal.reason?.name === 'TimeoutError';
    return new ZegaNetworkError(timedOut ? 'timeout' : 'aborted', timedOut ? 'The request timed out.' : 'The request was aborted.', error);
  }
  const why = error instanceof Error ? `: ${error.message}` : '';
  return new ZegaNetworkError('network', `Could not reach ${origin}${why}`, error);
}

/**
 * Connect to a zega-server (`zega-server start`) or a Zega Cloud graph.
 * Nothing is sent until the first call.
 */
export function connect(options) {
  if (options === null || typeof options !== 'object') throw config('takes { url, key?, schema?, fetch? }.');
  const { url, key, schema: defaultSchema, fetch: customFetch, readMethod = 'auto' } = options;
  if (typeof url !== 'string' || url.trim() === '') {
    throw config('needs a url: the address of a zega-server or a Zega Cloud graph (a function is given it as ZEGA_GRAPH_URL).');
  }
  let parsed;
  try { parsed = new URL(url); } catch { throw config('the url is not a valid URL.'); }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') throw config('the url must be http: or https:.');
  if (parsed.search || parsed.hash || parsed.username || parsed.password) throw config('the url must not carry a query string, fragment or credentials; the key goes in `key`.');
  if (key !== undefined && key !== null && key !== '' && !(typeof key === 'string' && /^[\x21-\x7e]+$/.test(key))) throw config('the key must be a string of printable ASCII.');
  if (customFetch !== undefined && typeof customFetch !== 'function') throw config('`fetch` must be a function.');
  if (readMethod !== 'auto' && readMethod !== 'query' && readMethod !== 'post') throw config("`readMethod` is 'auto', 'query' or 'post'.");
  if (defaultSchema !== undefined && typeof defaultSchema !== 'string') throw config('`schema` is ZQL schema text.');

  const base = url.replace(/\/+$/, '');
  const origin = parsed.origin;
  const send = customFetch ?? ((input, init) => globalThis.fetch(input, init));
  let queryRefused = readMethod === 'post';

  async function call(method, path, body, signal) {
    const headers = { accept: 'application/json' };
    if (body !== undefined) headers['content-type'] = 'application/json';
    if (key) headers.authorization = `Bearer ${key}`;
    let response;
    let text;
    try {
      response = await send(base + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), signal });
      text = await response.text();
    } catch (error) {
      throw unreachable(error, signal, origin);
    }
    let payload;
    try { payload = JSON.parse(text); } catch { /* not JSON: a proxy's page, or an empty 404 */ }
    const answer = payload !== null && typeof payload === 'object' && !Array.isArray(payload) ? payload : undefined;
    const status = response.status;
    if (response.ok && answer?.ok === true && 'result' in answer) return answer.result;
    if (response.ok && answer?.ok !== false) {
      throw new ZegaError('bad_response', `The server answered ${status} but not with { ok: true, result }. Is the url a zega graph?`, { status });
    }
    const serverCode = typeof answer?.code === 'string' ? answer.code : undefined;
    const message = typeof answer?.error === 'string' ? answer.error
      : `HTTP ${status}${response.statusText ? ` ${response.statusText}` : ''}${text.trim() && !answer ? `: ${text.trim().slice(0, 200)}` : ''}`;
    const retryAfter = Number(response.headers?.get?.('retry-after') ?? answer?.retryAfterSeconds);
    throw new ZegaError(codeFor(status, serverCode), message, {
      status,
      serverCode,
      retryAfter: Number.isFinite(retryAfter) ? retryAfter : undefined,
      ...engineDetails(message),
    });
  }

  function zqlBody(source, callOptions) {
    if (typeof source !== 'string') throw new TypeError('zega: the ZQL must be a string');
    const schema = callOptions?.schema ?? defaultSchema;
    const body = { query: source };
    if (schema !== undefined) body.schema = schema;
    if (callOptions?.document) body.document = true;
    return body;
  }

  /** A server that does not know the QUERY method: zega-server before it, or a Zega Cloud router before it. */
  const lacksQuery = error => error.status === 405 || error.status === 501 || (error.status === 404 && error.serverCode === 'not_found');

  return {
    url: base,

    mutate(source, callOptions) {
      return call('POST', '/zql', zqlBody(source, callOptions), callOptions?.signal);
    },

    async query(source, callOptions) {
      const body = zqlBody(source, callOptions);
      if (!queryRefused) {
        try {
          return await call('QUERY', '/zql', body, callOptions?.signal);
        } catch (error) {
          if (!(error instanceof ZegaError) || !lacksQuery(error)) throw error;
          if (readMethod === 'query') throw error;
          queryRefused = true;
        }
      }
      return call('POST', '/zql', body, callOptions?.signal);
    },

    async schema(callOptions) {
      try {
        return await call('GET', '/schema', undefined, callOptions?.signal);
      } catch (error) {
        if (error instanceof ZegaError && error.status === 404) {
          error.message += ' (a local zega-server keeps no schema; pass `schema` to connect() instead)';
        }
        throw error;
      }
    },
  };
}
