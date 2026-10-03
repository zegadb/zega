import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'

import { TOKEN, fakeApi, remove, run, scratch, type Canned } from './helpers'

const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => remove(home))

const fail = (status: number, code: string, error: string, extra: object = {}, headers: Record<string, string> = {}): Canned => ({
    status,
    headers,
    body: { ok: false, code, error, ...extra },
})

async function against(route: Canned, args: string[] = ['usage']) {
    const api = fakeApi({ 'GET /v1/usage': route })
    try {
        const result = await run([...args, '--api', api.url, '--token-file', tokenFile], { home })
        return { result, api }
    } finally {
        api.stop()
    }
}

describe('what a failure says', () => {
    test('401: the API sentence and code, then how to get a token and what to do with it', async () => {
        const { result } = await against(fail(401, 'token_expired', "The token's expiry has passed."))
        expect(result.code).toBe(1)
        expect(result.stdout).toBe('')
        expect(result.stderr).toContain("zega: The token's expiry has passed. (code: token_expired, HTTP 401)")
        expect(result.stderr).toContain('Make a token at https://dashboard.zega.dev/tokens')
        // Given --token-file, the fix is the file, not `zega login`.
        expect(result.stderr).toContain(`put a new token in ${tokenFile}`)
        expect(result.stderr + result.stdout).not.toContain(TOKEN)
    })

    test('401 with a stored token says to run `zega login`', async () => {
        const api = fakeApi({
            'GET /v1/whoami': { body: { token: { name: 'x', prefix: 'zc_', scope: 'read' } } },
            'GET /v1/usage': fail(401, 'token_revoked', 'The token was revoked in the dashboard.'),
        })
        const dir = scratch()
        try {
            expect((await run(['login', '--api', api.url], { home: dir, stdin: TOKEN })).code).toBe(0)
            const result = await run(['usage'], { home: dir })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('code: token_revoked')
            expect(result.stderr).toContain('then run `zega login`.')
        } finally {
            api.stop()
            remove(dir)
        }
    })

    test('401 wrong_credential: a graph or storage key is not an API token', async () => {
        const { result } = await against(fail(401, 'wrong_credential', 'That is a graph key.'))
        expect(result.stderr).toContain('That is a graph or storage key, not an API token.')
    })

    test('403 scope_required: needs a manage token', async () => {
        const { result } = await against(fail(403, 'scope_required', 'This token can only read.'))
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('(code: scope_required, HTTP 403)')
        expect(result.stderr).toContain('This needs a manage token. Make one at https://dashboard.zega.dev/tokens')
    })

    test('403 session_required: the API\'s own text is relayed; the dashboard is named only if the API did not', async () => {
        const sentence = 'Spending caps are set in the dashboard, signed in. A token cannot do it.'
        const { result } = await against(fail(403, 'session_required', sentence))
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`zega: ${sentence} (code: session_required, HTTP 403)`)
        // No second, redundant instruction when the API said it already.
        expect(result.stderr.split('\n').filter((line) => line !== '')).toHaveLength(1)

        const silent = await against(fail(403, 'session_required', 'Not available to a token.'))
        expect(silent.result.stderr).toContain('Not available to a token.')
        expect(silent.result.stderr).toContain('This is done in the dashboard (https://dashboard.zega.dev), signed in; an API token cannot do it.')
    })

    test('429: the wait time from retryAfterSeconds, and exactly one request (no automatic retry)', async () => {
        const { result, api } = await against(
            fail(429, 'rate_limited', 'Too many requests.', { retryAfterSeconds: 37 }, { 'retry-after': '37' }),
        )
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('(code: rate_limited, HTTP 429)')
        expect(result.stderr).toContain('Wait 37 seconds, then run the command again.')
        expect(api.seen).toHaveLength(1)
    })

    test('503: the wait time, and whether the change took effect', async () => {
        const { result, api } = await against(
            fail(503, 'busy_try_again', 'Zega Cloud is busy.', { retryAfterSeconds: 1, effect: 'unknown' }),
        )
        expect(result.stderr).toContain('Wait 1 second, then run the command again.')
        expect(result.stderr).toContain('It is not known whether the change took effect')
        expect(api.seen).toHaveLength(1)
    })

    test('429 without a body: the Retry-After header is used', async () => {
        const { result } = await against({ status: 429, body: 'slow down', headers: { 'retry-after': '9' } })
        expect(result.stderr).toContain('(code: http_429, HTTP 429)')
        expect(result.stderr).toContain('Wait 9 seconds')
    })

    test('a network failure names the host it could not reach', async () => {
        // A port nothing listens on: bind one, learn its number, close it.
        const port = await new Promise<number>((resolve) => {
            const server = net.createServer().listen(0, '127.0.0.1', () => {
                const { port } = server.address() as net.AddressInfo
                server.close(() => resolve(port))
            })
        })
        const result = await run(['usage', '--api', `http://127.0.0.1:${port}`, '--token-file', tokenFile], { home })
        expect(result.code).toBe(1)
        expect(result.stdout).toBe('')
        expect(result.stderr).toContain(`could not reach 127.0.0.1:${port}`)
        expect(result.stderr).toContain('Check the address given to --api.')
    })

    test('a redirect is not followed, so the token never leaves for another host', async () => {
        const elsewhere = fakeApi({ 'GET /v1/usage': { body: {} } })
        const api = fakeApi({
            'GET /v1/usage': { status: 302, body: '', headers: { location: `${elsewhere.url}/v1/usage` } },
        })
        try {
            const result = await run(['usage', '--api', api.url, '--token-file', tokenFile], { home })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('HTTP 302')
            expect(elsewhere.seen).toHaveLength(0)
        } finally {
            api.stop()
            elsewhere.stop()
        }
    })

    test('a 200 that is not JSON is a failure that names the host', async () => {
        const api = fakeApi({ 'GET /v1/usage': { body: '<html>captive portal</html>' } })
        try {
            const result = await run(['usage', '--api', api.url, '--token-file', tokenFile], { home })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain(`${api.host} answered HTTP 200 with something that is not JSON`)
        } finally {
            api.stop()
        }
    })
})
