import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, credentialsFile, fakeApi, remove, run, scratch, whoamiRoute } from './helpers'

const home = scratch()
afterAll(() => remove(home))

function secrets(...outputs: string[]) {
    for (const output of outputs) expect(output).not.toContain(TOKEN)
}

describe('zega login', () => {
    const api = fakeApi({ ...whoamiRoute })
    afterAll(() => api.stop())

    test('checks the token with whoami, stores it privately, and never prints it', async () => {
        const result = await run(['login', '--api', api.url], { home, stdin: TOKEN + '\n' })
        expect(result.code).toBe(0)

        // The API was asked who this token is, with that token and no other credential.
        expect(api.seen).toHaveLength(1)
        expect(api.seen[0]).toMatchObject({ method: 'GET', pathname: '/v1/whoami', authorization: `Bearer ${TOKEN}` })

        // The token is on disk, in a file only its owner can read, in a directory only its owner can enter.
        const file = credentialsFile(home)
        expect(JSON.parse(fs.readFileSync(file, 'utf8'))).toEqual({ api: api.url, store: 'file', token: TOKEN })
        if (process.platform !== 'win32') {
            expect(fs.statSync(file).mode & 0o777).toBe(0o600)
            expect(fs.statSync(path.dirname(file)).mode & 0o777).toBe(0o700)
        }

        // The person is told who they are and where it went, not the secret.
        expect(result.stdout).toContain(`logged in to ${api.url}`)
        expect(result.stdout).toContain('ci-bot')
        expect(result.stdout).toContain(file)
        secrets(result.stdout, result.stderr)
    })

    test('a later command uses the stored token and its host', async () => {
        const result = await run(['whoami'], { home })
        expect(result.code).toBe(0)
        expect(api.seen.at(-1)).toMatchObject({ pathname: '/v1/whoami', authorization: `Bearer ${TOKEN}` })
        expect(result.stdout).toContain('Scope')
        secrets(result.stdout, result.stderr)
    })

    test('a stored token is never sent to another host', async () => {
        const before = api.seen.length
        const result = await run(['whoami', '--api', 'https://cloud.zega.world'], { home })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`you are logged in to ${api.url}`)
        expect(result.stderr).toContain('zega login --api https://cloud.zega.world')
        expect(api.seen).toHaveLength(before)
    })

    test('logout removes it, and the next command says to log in', async () => {
        const out = await run(['logout'], { home })
        expect(out.code).toBe(0)
        expect(fs.existsSync(credentialsFile(home))).toBe(false)
        expect(out.stdout).toContain('still valid until you revoke it')

        const after = await run(['whoami'], { home })
        expect(after.code).toBe(1)
        expect(after.stderr).toContain('not logged in: run `zega login`')
        expect(after.stderr).toContain('--token-file')

        const again = await run(['logout'], { home })
        expect(again.code).toBe(0)
        expect(again.stdout).toContain('not logged in')
    })
})

describe('zega login with a token the API refuses', () => {
    const rejecting = fakeApi({
        'GET /v1/whoami': {
            status: 401,
            body: { ok: false, code: 'invalid_token', error: 'This is not an API token.' },
        },
    })
    afterAll(() => rejecting.stop())
    const good = fakeApi({ ...whoamiRoute })
    afterAll(() => good.stop())
    const dir = scratch()
    afterAll(() => remove(dir))

    test('stores nothing, and does not replace a good login', async () => {
        const first = await run(['login', '--api', good.url], { home: dir, stdin: TOKEN })
        expect(first.code).toBe(0)
        const before = fs.readFileSync(credentialsFile(dir), 'utf8')

        const bad = 'zc_mistyped_token_value'
        const second = await run(['login', '--api', rejecting.url], { home: dir, stdin: bad })
        expect(second.code).toBe(1)
        expect(second.stderr).toContain('This is not an API token.')
        expect(second.stderr).toContain('code: invalid_token')
        expect(second.stderr).toContain('HTTP 401')
        expect(second.stderr).toContain('https://dashboard.zega.dev/tokens')
        expect(second.stderr).toContain('zega login')
        expect(fs.readFileSync(credentialsFile(dir), 'utf8')).toBe(before)
        expect(second.stdout + second.stderr).not.toContain(bad)
    })

    test('with nothing stored before, nothing is written at all', async () => {
        const empty = scratch()
        try {
            const result = await run(['login', '--api', rejecting.url], { home: empty, stdin: TOKEN })
            expect(result.code).toBe(1)
            expect(fs.existsSync(path.join(empty, '.config'))).toBe(false)
            secrets(result.stdout, result.stderr)
        } finally {
            remove(empty)
        }
    })
})

describe('zega login reading the token', () => {
    const api = fakeApi({ ...whoamiRoute })
    afterAll(() => api.stop())
    const dir = scratch()
    afterAll(() => remove(dir))

    test('--token-file: the token comes from the file, with a final newline', async () => {
        const file = path.join(dir, 'token.txt')
        fs.writeFileSync(file, TOKEN + '\n', { mode: 0o600 })
        const result = await run(['login', '--api', api.url, '--token-file', file], { home: dir })
        expect(result.code).toBe(0)
        expect(api.seen.at(-1)?.authorization).toBe(`Bearer ${TOKEN}`)
        expect(JSON.parse(fs.readFileSync(credentialsFile(dir), 'utf8')).token).toBe(TOKEN)
        secrets(result.stdout, result.stderr)
    })

    test('--token-file that is missing or holds no single token fails and names the file', async () => {
        const missing = await run(['login', '--api', api.url, '--token-file', path.join(dir, 'nope')], { home: dir })
        expect(missing.code).toBe(1)
        expect(missing.stderr).toContain('cannot read token file')

        const two = path.join(dir, 'two.txt')
        fs.writeFileSync(two, 'zc_one zc_two\n')
        const result = await run(['login', '--api', api.url, '--token-file', two], { home: dir })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`token file ${two} must contain one nonempty token`)
        expect(result.stderr).not.toContain('zc_one')
    })

    test('no terminal and no token: fails at once, naming the flag, without waiting for a prompt', async () => {
        const before = api.seen.length
        const started = Date.now()
        const result = await run(['login', '--api', api.url], { home: dir, stdin: null })
        expect(Date.now() - started).toBeLessThan(5000)
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('missing --token-file')
        expect(result.stderr).toContain('no terminal')
        expect(result.stdout).toBe('')
        expect(api.seen).toHaveLength(before)
    })

    test('a token piped on stdin must be one word', async () => {
        const result = await run(['login', '--api', api.url], { home: dir, stdin: 'zc_one zc_two' })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('one nonempty word')
        expect(result.stderr).not.toContain('zc_one')
    })

    test('--json prints the whoami answer unchanged', async () => {
        const result = await run(['login', '--api', api.url, '--json'], { home: dir, stdin: TOKEN })
        expect(result.code).toBe(0)
        expect(JSON.parse(result.stdout)).toEqual(JSON.parse(JSON.stringify(whoamiRoute['GET /v1/whoami'].body)))
        secrets(result.stdout, result.stderr)
    })

    test('--api must be https unless it is local', async () => {
        const result = await run(['login', '--api', 'http://cloud.zega.dev'], { home: dir, stdin: TOKEN })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('only sent over https')
        secrets(result.stdout, result.stderr)
    })

    test('--store with something else is refused before anything is read', async () => {
        const result = await run(['login', '--api', api.url, '--store', 'cloud'], { home: dir, stdin: TOKEN })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('`file` (the default) or `keychain`')
    })
})
