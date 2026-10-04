/**
 * Where Bun is not the Workers runtime. `zega dev` serves a function with Bun, which
 * has most of the web platform a Workers function uses (fetch, Request, Response,
 * URL, streams, crypto.subtle, TextEncoder, HTMLRewriter) and none of workerd's own
 * extras. A function that reaches for one of those would work in the cloud and fail
 * here, or the other way round; these checks find the ones that can be found and say
 * so in one line each, rather than pretending. They only warn: nothing is stopped.
 */

/** Source patterns: what the bundled code mentions that Bun does not provide. */
const SOURCE_CHECKS: { pattern: RegExp; warning: string }[] = [
    { pattern: /\b(?:request|req)\.cf\b/, warning: '`request.cf` is not set by zega dev (it is undefined here)' },
    { pattern: /\bcaches\s*\.\s*(?:default|open)\b/, warning: 'the Cache API (`caches`) does not exist in zega dev' },
    { pattern: /\bWebSocketPair\b/, warning: '`WebSocketPair` does not exist in zega dev (no WebSocket upgrade responses)' },
    { pattern: /\bDigestStream\b|\btimingSafeEqual\b/, warning: 'crypto.DigestStream and crypto.subtle.timingSafeEqual are Workers-only and missing in zega dev' },
    { pattern: /\bpassThroughOnException\b/, warning: '`ctx.passThroughOnException()` has no origin to fall through to in zega dev; an error answers 500' },
    { pattern: /\bprocess\s*\.\s*env\b/, warning: '`process.env` exists here and not in Zega Cloud: read `env.NAME` (and note it holds your shell, not your .env)' },
    { pattern: /\bfrom\s*["']cloudflare:(?!workers["'])|\bimport\s*\(\s*["']cloudflare:(?!workers["'])/, warning: 'a `cloudflare:` module other than `cloudflare:workers` is imported; zega dev provides only `env` and `waitUntil` from that one' },
]

/** One line per thing the bundled code uses that Bun cannot imitate. */
export function sourceWarnings(code: string): string[] {
    return SOURCE_CHECKS.filter((check) => check.pattern.test(code)).map((check) => check.warning)
}

/** Handlers a Workers module may export that Zega Cloud never calls (a function is only invoked over HTTP) and zega dev never calls either. */
const OTHER_HANDLERS = ['scheduled', 'queue', 'email', 'tail', 'trace', 'tailStream'] as const

/** One line per thing about the loaded module's shape that does not carry over. */
export function moduleWarnings(module: Record<string, unknown>): string[] {
    const warnings: string[] = []
    const entry = module.default as Record<string, unknown> | undefined
    const others = OTHER_HANDLERS.filter((name) => typeof entry?.[name] === 'function')
    if (others.length > 0) {
        warnings.push(`defines ${others.join(', ')}: a function is only invoked over HTTP, so zega dev never calls ${others.length === 1 ? 'it' : 'them'}`)
    }
    const extra = Object.keys(module).filter((name) => name !== 'default')
    if (extra.length > 0) {
        warnings.push(`exports ${extra.join(', ')} besides the default: zega dev uses only the default export's fetch (Durable Objects and entrypoint classes are not provided)`)
    }
    return warnings
}
