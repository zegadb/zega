/**
 * Talking to a graph or a bucket at its own address, which is not the cloud API's: a graph's `hostUrl`
 * (`https://<graph-id>.zegadb.com`) answers `QUERY|POST /zql`, `GET /schema`, with a graph key (`zk_...`) as the bearer;
 * a bucket's address answers `GET /?prefix=` with a storage key (`zs_...`); a local `zega-server` answers the same
 * routes as a graph, on 127.0.0.1, with no key. The API token never goes here, and the key goes nowhere else.
 *
 * Every answer is `{ ok: true, result }` or `{ ok: false, error, code? }`.
 */
import { LocalFailure, NetworkFailure } from '@/utils/errors'

const TIMEOUT_MS = 60_000
const MAX_ANSWER_BYTES = 32 << 20

export type Target = {
    /** The address, without a path. */
    url: string
    /** The key to send, or none (a local server). */
    key?: string
}

export function isLoopback(hostname: string): boolean {
    return hostname === '127.0.0.1' || hostname === 'localhost' || hostname === '[::1]' || hostname === '::1'
}

/** A local server's address: http, on this machine. */
export function localUrl(given: string): string {
    let url: URL
    try {
        url = new URL(given)
    } catch {
        throw new LocalFailure(`"${given}" is not a URL (a local server is http://127.0.0.1:<port>)`, 'invalid_flag')
    }
    if (url.protocol !== 'http:' || !isLoopback(url.hostname)) {
        throw new LocalFailure(`a local server is http://127.0.0.1:<port> on this machine, not ${url.origin}; use the graph tools for a cloud graph`, 'invalid_flag')
    }
    return url.origin
}

/** The address the API gave for a graph or bucket: https (http only on this machine, as for `--api`), nothing but a host. */
export function cloudUrl(given: string): string {
    const url = new URL(given)
    const plainLocal = url.protocol === 'http:' && isLoopback(url.hostname)
    if ((url.protocol !== 'https:' && !plainLocal) || url.username !== '' || url.password !== '') {
        throw new LocalFailure(`refusing to send a key to ${url.origin}: a cloud address is https`)
    }
    return url.origin
}

/**
 * One request; the answer's `result` (with `bare`, the answer itself: a bucket's listing is not wrapped in `{ ok, result }`).
 * A refusal is a `LocalFailure` whose code is the server's own when it gave one.
 */
export async function call(target: Target, method: string, path: string, body?: unknown, bare = false): Promise<unknown> {
    const headers: Record<string, string> = { accept: 'application/json' }
    if (target.key !== undefined) headers.authorization = `Bearer ${target.key}`
    if (body !== undefined) headers['content-type'] = 'application/json'
    let response: Response
    let raw: string
    try {
        response = await fetch(target.url + path, {
            method,
            headers,
            body: body === undefined ? undefined : JSON.stringify(body),
            // Never follow a redirect with a key attached.
            redirect: 'manual',
            signal: AbortSignal.timeout(TIMEOUT_MS),
        })
        const bytes = new Uint8Array(await response.arrayBuffer())
        if (bytes.length > MAX_ANSWER_BYTES) throw new LocalFailure(`${new URL(target.url).host} sent an answer larger than ${MAX_ANSWER_BYTES} bytes`)
        raw = new TextDecoder().decode(bytes)
    } catch (error) {
        if (error instanceof LocalFailure) throw error
        const cause = error instanceof Error ? ((error as { cause?: unknown }).cause instanceof Error ? (error as { cause: Error }).cause.message : error.message) : String(error)
        throw new NetworkFailure(new URL(target.url).host, cause)
    }
    let json: { ok?: unknown; result?: unknown; error?: unknown; code?: unknown } | undefined
    try {
        json = JSON.parse(raw)
    } catch {
        json = undefined
    }
    if (response.ok && json !== undefined && (bare || json.ok === true)) return bare ? json : json.result
    const code = typeof json?.code === 'string' ? json.code : `http_${response.status}`
    const message = typeof json?.error === 'string' ? json.error : `${new URL(target.url).host} answered HTTP ${response.status}`
    if (response.status === 401 || response.status === 403) {
        throw new LocalFailure(`${message} (the key was refused by ${new URL(target.url).host})`, code)
    }
    if (response.status === 405 && method === 'QUERY') {
        throw new LocalFailure(`${new URL(target.url).host} does not take QUERY /zql: this zega-server is older than the read-only query route. Update it.`, 'query_unsupported')
    }
    throw new LocalFailure(message, code)
}
