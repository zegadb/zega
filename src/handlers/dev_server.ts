/**
 * Serving a function the way Zega Cloud calls it: `export default { fetch(request, env, ctx) }`,
 * with `env` the function's variables and secrets and nothing else (see dev_vars.ts).
 *
 * The bundle (made by the same `prepare` as `zega deploy`) is loaded as an ES module in this
 * process. One Bun.serve per function owns the port for the whole run; a rebuild only swaps
 * the module its handler calls, so the port never changes and a request is never answered by
 * half of a build. `cloudflare:workers` is not a file: it is provided here, with `env` (the
 * function's own, found through AsyncLocalStorage, since every function runs in this one
 * process) and `waitUntil`.
 */
import { AsyncLocalStorage } from 'node:async_hooks'

import type { Target } from '@/handlers/deploy'
import type { Emit } from '@/handlers/dev_events'
import { moduleWarnings, sourceWarnings } from '@/handlers/workers_compat'
import { LocalFailure } from '@/utils/errors'

export type Env = Record<string, string>

type ExecutionContext = {
    waitUntil(promise: Promise<unknown>): void
    passThroughOnException(): void
    props: Record<string, never>
}

type Scope = { env: Env; ctx: ExecutionContext }
const scope = new AsyncLocalStorage<Scope>()

const noContext = (): ExecutionContext => ({ waitUntil() {}, passThroughOnException() {}, props: {} })

let provided = false

/** Make `import … from 'cloudflare:workers'` resolve, once per process. */
function provideCloudflareWorkers(): void {
    if (provided) return
    provided = true
    const current = (): Scope => {
        const found = scope.getStore()
        if (found === undefined) throw new Error('`env` from cloudflare:workers is only readable while a function is loading or handling a request in zega dev')
        return found
    }
    const env = new Proxy({} as Env, {
        get: (_, name) => (typeof name === 'string' ? current().env[name] : undefined),
        has: (_, name) => typeof name === 'string' && name in current().env,
        ownKeys: () => Reflect.ownKeys(current().env),
        getOwnPropertyDescriptor: (_, name) => Reflect.getOwnPropertyDescriptor(current().env, name),
    })
    Bun.plugin({
        name: 'zega-dev-cloudflare-workers',
        setup(build) {
            build.module('cloudflare:workers', () => ({
                loader: 'object',
                exports: { env, waitUntil: (promise: Promise<unknown>) => current().ctx.waitUntil(promise) },
            }))
        },
    })
}

type Handler = (request: Request, env: Env, ctx: ExecutionContext) => Response | Promise<Response>
export type Loaded = { fetch: Handler; warnings: string[]; bytes: number }

/**
 * Load a bundle as a module and find its fetch handler. Each load is its own module (the
 * comment makes the text unique), so two functions, or two builds of one, never share state.
 */
export async function load(target: Target, code: string, env: Env, generation: number): Promise<Loaded> {
    provideCloudflareWorkers()
    const unique = `${code}\n//# zega-dev ${target.id} ${generation}\n`
    let module: Record<string, unknown>
    try {
        module = await scope.run({ env, ctx: noContext() }, () =>
            import(`data:text/javascript;base64,${Buffer.from(unique).toString('base64')}`),
        )
    } catch (error) {
        throw new LocalFailure(`cannot load ${target.entry}: ${(error as Error).message}`)
    }
    const entry = module.default as { fetch?: unknown } | undefined
    if (entry === undefined || entry === null || typeof entry.fetch !== 'function') {
        throw new LocalFailure(`${target.entry}: the default export has no fetch(request, env, ctx) function`)
    }
    const fetch = entry.fetch.bind(entry) as Handler
    return { fetch, warnings: [...sourceWarnings(code), ...moduleWarnings(module)], bytes: code.length }
}

/** One function on one port. The module behind it can be replaced while it serves. */
export class DevFunction {
    loaded: Loaded | null = null
    env: Env = {}
    private server: ReturnType<typeof Bun.serve> | null = null

    constructor(
        readonly target: Target,
        readonly port: number,
        private readonly emit: Emit,
    ) {}

    get url(): string {
        return `http://127.0.0.1:${this.port}`
    }

    start(): void {
        try {
            this.server = Bun.serve({ hostname: '127.0.0.1', port: this.port, fetch: (request) => this.handle(request) })
        } catch (error) {
            const why = (error as NodeJS.ErrnoException).code === 'EADDRINUSE' ? 'is already in use' : `cannot be used: ${(error as Error).message}`
            throw new LocalFailure(`port ${this.port} ${why} (for ${this.target.id}); pick another with --port`)
        }
    }

    stop(): void {
        this.server?.stop(true)
        this.server = null
    }

    private async handle(request: Request): Promise<Response> {
        const started = performance.now()
        const path = new URL(request.url).pathname
        const loaded = this.loaded
        let response: Response
        if (loaded === null) {
            response = new Response('zega dev: the function is not loaded', { status: 503 })
        } else {
            const waiting: Promise<unknown>[] = []
            const ctx: ExecutionContext = {
                waitUntil: (promise) => {
                    waiting.push(
                        Promise.resolve(promise).catch((error: unknown) => this.emit({ event: 'wait_until_rejected', function: this.target.id, message: errorText(error) })),
                    )
                },
                passThroughOnException() {},
                props: {},
            }
            try {
                const result = await scope.run({ env: this.env, ctx }, () => loaded.fetch(request, this.env, ctx))
                if (!(result instanceof Response)) {
                    throw new TypeError(`fetch returned ${result === null ? 'null' : typeof result}, not a Response`)
                }
                response = result
            } catch (error) {
                this.emit({ event: 'threw', function: this.target.id, message: errorText(error) })
                response = new Response('Internal Server Error', { status: 500 })
            }
        }
        this.emit({
            event: 'request',
            function: this.target.id,
            method: request.method,
            path,
            status: response.status,
            ms: Math.round(performance.now() - started),
        })
        return response
    }
}

function errorText(error: unknown): string {
    return error instanceof Error ? (error.stack ?? error.message) : String(error)
}
