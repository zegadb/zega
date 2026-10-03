import pkg from '../../package.json'
import type { paths } from '@/types/cloud-api'
import { normalizeApi } from '@/utils/api_address'
import { ApiFailure, LocalFailure, NetworkFailure } from '@/utils/errors'

/** The longest answer the client reads: a page of 200 log entries is far below this. */
const MAX_ANSWER_BYTES = 32 << 20
const TIMEOUT_MS = 60_000

/** Every path of the OpenAPI document that has a GET. */
export type GetPath = {
    [P in keyof paths]: paths[P] extends { get: object } ? P : never
}[keyof paths]

type GetOperation<P extends GetPath> = paths[P] extends { get: infer O } ? O : never

/** What a 200 answers, typed from the OpenAPI document. */
export type Answer<P extends GetPath> = GetOperation<P> extends {
    responses: { 200: { content: { 'application/json': infer Body } } }
}
    ? Body
    : never

type OperationParameters<P extends GetPath> = GetOperation<P> extends { parameters: infer Q } ? Q : never

export type PathParams<P extends GetPath> =
    OperationParameters<P> extends { path?: infer X } ? Exclude<X, undefined> : never
export type QueryParams<P extends GetPath> =
    OperationParameters<P> extends { query?: infer X } ? Exclude<X, undefined> : never

/** A 2xx answer: the text as sent (for `--json`) and its parse. */
export type Reply<T> = { raw: string; json: T }

export class CloudClient {
    readonly base: string
    private readonly token: string

    constructor(api: string, token: string) {
        this.base = normalizeApi(api)
        this.token = token
    }

    /** `host` or `host:port`, as a person would write it. */
    get host(): string {
        return new URL(this.base).host
    }

    async get<P extends GetPath>(
        path: P,
        args: { path?: PathParams<P>; query?: QueryParams<P> } = {},
    ): Promise<Reply<Answer<P>>> {
        const filled = fillPath(path, (args.path ?? {}) as Record<string, string>)
        const url = new URL(this.base + filled)
        for (const [name, value] of Object.entries((args.query ?? {}) as Record<string, unknown>)) {
            if (value !== undefined && value !== null) url.searchParams.append(name, String(value))
        }
        let response: Response
        let raw: string
        try {
            response = await fetch(url, {
                method: 'GET',
                headers: {
                    authorization: `Bearer ${this.token}`,
                    accept: 'application/json',
                    'user-agent': `zega-cli/${pkg.version}`,
                },
                // An API that redirects is not this API; never follow a redirect with a bearer token attached.
                redirect: 'manual',
                signal: AbortSignal.timeout(TIMEOUT_MS),
            })
            const bytes = new Uint8Array(await response.arrayBuffer())
            if (bytes.length > MAX_ANSWER_BYTES) {
                throw new LocalFailure(`${this.host} sent an answer larger than ${MAX_ANSWER_BYTES} bytes`)
            }
            raw = new TextDecoder().decode(bytes)
        } catch (error) {
            if (error instanceof LocalFailure) throw error
            throw new NetworkFailure(this.host, detail(error))
        }
        let json: unknown
        try {
            json = JSON.parse(raw)
        } catch {
            json = undefined
        }
        if (response.status >= 200 && response.status < 300) {
            if (json === undefined) {
                throw new LocalFailure(`${this.host} answered HTTP ${response.status} with something that is not JSON`)
            }
            return { raw, json: json as Answer<P> }
        }
        throw apiFailure(response.status, json, response.headers.get('retry-after'), this.host)
    }
}

/** `/v1/graphs/{graph}` with `{ graph: "a/b" }` gives `/v1/graphs/a%2Fb`: an id typed with a `/` stays one segment. */
export function fillPath(template: string, params: Record<string, string>): string {
    return template.replace(/\{([^}]+)\}/g, (_, name: string) => {
        const value = params[name]
        if (value === undefined) throw new LocalFailure(`internal error: no value for {${name}} in ${template}`)
        return encodeURIComponent(value)
    })
}

function detail(error: unknown): string {
    if (error instanceof Error) {
        const cause = (error as { cause?: unknown }).cause
        if (cause instanceof Error && cause.message !== '') return cause.message
        return error.message
    }
    return String(error)
}

function apiFailure(status: number, body: unknown, retryHeader: string | null, host: string): ApiFailure {
    const field = (name: string): unknown =>
        typeof body === 'object' && body !== null ? (body as Record<string, unknown>)[name] : undefined
    const code = field('code')
    const message = field('error')
    const bodyRetry = field('retryAfterSeconds')
    const headerRetry = retryHeader !== null && /^\d+$/.test(retryHeader.trim()) ? Number(retryHeader.trim()) : null
    const effect = field('effect')
    return new ApiFailure(
        status,
        // Not the API's shape (a proxy's page, a redirect): say what we know.
        typeof code === 'string' ? code : `http_${status}`,
        typeof message === 'string' ? message : `${host} answered HTTP ${status} without an error message`,
        typeof bodyRetry === 'number' && Number.isInteger(bodyRetry) && bodyRetry >= 0 ? bodyRetry : headerRetry,
        typeof effect === 'string' ? effect : null,
    )
}
