/**
 * What `ZegaError.code` says went wrong.
 *
 * - `config`: `connect()` was given something unusable. Thrown before any request.
 * - `network`: no answer at all (DNS, refused connection, TLS, reset).
 * - `aborted` / `timeout`: the call's `signal` fired (`timeout` when it was `AbortSignal.timeout()`).
 * - `unauthorized`: 401 or 403. The key is missing, wrong, or for another graph.
 * - `query`: the engine refused the ZQL (400 or 422): syntax, schema, a failed
 *   constraint, or the time limit. `message` is the engine's own text, with
 *   `help` and `location` read out of it; `serverCode` is `query_time_limit` for the time limit.
 * - `not_a_read`: `query()` was given something that writes. Nothing ran. Use `mutate()`.
 * - `rate_limited`: 429 (Zega Cloud). `retryAfter` is the seconds to wait.
 * - `server`: 5xx, including a Zega Cloud graph that is not reachable (`graph_unreachable`).
 * - `http`: any other refusal. Zega Cloud's own are in `serverCode`:
 *   `spending_cap_reached` (402), `graph_not_ready` (409), `body_too_large` (413) and so on.
 * - `bad_response`: a 2xx that is not a zega answer; the url is probably not a graph.
 */
export type ZegaErrorCode =
  | 'config' | 'network' | 'aborted' | 'timeout'
  | 'unauthorized' | 'query' | 'not_a_read' | 'rate_limited' | 'server' | 'http' | 'bad_response';

/** Where the engine says an error is: `schema` or `query` (or the source's own name), a 1-based line and column. */
export interface ZegaErrorLocation {
  source: string;
  line: number;
  column: number;
}

/** The server answered with an error, or the call could not be made. The key is never in `message`. */
export class ZegaError extends Error {
  readonly name: string;
  readonly code: ZegaErrorCode;
  /** The HTTP status; 0 when there was no answer (`config`, `network`, `aborted`, `timeout`). */
  readonly status: number;
  /** The server's own `code` (`query_time_limit`, `invalid_key`, `spending_cap_reached`, ...), when it sent one. */
  readonly serverCode?: string;
  /** The engine's `help:` line, if the error has one: "did you mean `name`?". */
  readonly help?: string;
  /** The engine's `query:1:12` position, if the error has one. */
  readonly location?: ZegaErrorLocation;
  /** Seconds to wait before trying again, on `rate_limited`. */
  readonly retryAfter?: number;
  constructor(
    code: ZegaErrorCode,
    message: string,
    details?: { status?: number; serverCode?: string; help?: string; location?: ZegaErrorLocation; retryAfter?: number; cause?: unknown },
  );
}

/**
 * No answer came: the network failed or the caller's signal fired. Distinct from
 * a query that was refused. `code` is `network`, `aborted` or `timeout`; `cause` is the original error.
 */
export class ZegaNetworkError extends ZegaError {
  readonly code: 'network' | 'aborted' | 'timeout';
  readonly status: 0;
  constructor(code: 'network' | 'aborted' | 'timeout', message: string, cause?: unknown);
}

export interface ConnectOptions {
  /**
   * A zega-server's address (`http://127.0.0.1:9342`) or a Zega Cloud graph's
   * (`https://<graph>.zegadb.com`, or its custom domain). In a function this is `env.ZEGA_GRAPH_URL`.
   */
  url: string;
  /** The bearer token: a `zk_` graph key, or the `--token-file` token of a zega-server. In a function, `env.ZEGA_GRAPH_KEY`. */
  key?: string;
  /**
   * Optional ZQL schema text sent with every statement. Prefer `setSchema()` to
   * store it on the server; a call's own `schema` replaces this one.
   */
  schema?: string;
  /** Your own `fetch`: a service binding, a test double, a polyfill. Defaults to the global one. */
  fetch?: (input: string, init?: RequestInit) => Promise<Response>;
  /**
   * How `query()` reaches the server. `auto` (the default) sends the HTTP QUERY
   * method (RFC 10008), and for a server that does not know it (405, 501) falls
   * back to POST and remembers. `query` never falls back; `post` never tries QUERY.
   */
  readMethod?: 'auto' | 'query' | 'post';
}

export interface CallOptions {
  /** ZQL schema text for this call, instead of the one given to `connect()`. */
  schema?: string;
  /** Treat the source as a whole `.zql` document (a `schema { }` block, then `query { }` or `mutation { }` blocks). */
  document?: boolean;
  /** Cancels the call. Use `AbortSignal.timeout(ms)` for a timeout. */
  signal?: AbortSignal;
}

export interface StoredSchema {
  schema: string;
  /** Built-in types available to the graph, including Auth. */
  builtin: string[];
  /** ISO time of the last push, or null before the first (Zega Cloud only). */
  updatedAt: string | null;
}

export interface Client {
  /** The address calls go to, without a trailing slash. */
  readonly url: string;
  /**
   * Run a read and return its result. Sent as `QUERY /zql`, which the server
   * refuses (`not_a_read`) if it would write. On a server that has no QUERY
   * method it is sent as `POST /zql` and runs whatever it is given.
   * `T` is not checked against the data: it is what you say you will get.
   */
  query<T = unknown>(zql: string, options?: CallOptions): Promise<T>;
  /** Run a statement that writes (or any statement) as `POST /zql` and return its result. */
  mutate<T = unknown>(zql: string, options?: CallOptions): Promise<T>;
  /** Read the graph's stored schema (`GET /schema`). */
  schema(options?: { signal?: AbortSignal }): Promise<StoredSchema>;
  /** Store a schema on the graph (`PUT /schema`) for later schema-less queries. */
  setSchema(schema: string, options?: { signal?: AbortSignal }): Promise<StoredSchema>;
}

/** Connect to a zega-server or a Zega Cloud graph. Sends nothing until the first call. */
export function connect(options: ConnectOptions): Client;

/**
 * Writes values into ZQL as escaped literals: `zql\`Person(name: ${name})\``.
 * ZQL has no query parameters; this is how user input goes into a statement
 * without being able to change it. Takes strings, finite numbers, bigints,
 * booleans and null; anything else throws a TypeError.
 */
export function zql(strings: TemplateStringsArray, ...values: (string | number | bigint | boolean | null)[]): string;
