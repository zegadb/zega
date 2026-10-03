import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, account, credentialsFile, fakeApi, remove, run, scratch, tokenInfo, whoamiRoute, whoamiShown } from './helpers'

const home = scratch()
afterAll(() => remove(home))

function secrets(...outputs: string[]) {
    for (const output of outputs) expect(output).not.toContain(TOKEN)
}

describe('zega login', () => {
    const api = fakeApi({ ...whoamiRoute })
    afterAll(() => api.stop())

    test('checks the token with whoami, stores it privately, and never prints it', async () => {
        const result = await run(['login', '--human', '--api', api.url], { home, stdin: TOKEN + '\n' })
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

        // The person is told who they are, not the secret and not where it went.
        expect(result.stdout).toBe(
            [
                'Logged in as Ada Lovelace (@ada)',
                '',
                'Token    ci-bot',
                'Scope    read',
                'Project  all projects',
                'Expires  Dec 1, 2026',
                '',
            ].join('\n'),
        )
        secrets(result.stdout, result.stderr)
        expect(result.stdout).not.toContain('zc_')
        expect(result.stdout).not.toContain(file)
        expect(result.stdout).not.toContain(home)
        expect(result.stderr).toBe('')
    })

    test('a later command uses the stored token and its host', async () => {
        const result = await run(['whoami', '--human'], { home })
        expect(result.code).toBe(0)
        expect(api.seen.at(-1)).toMatchObject({ pathname: '/v1/whoami', authorization: `Bearer ${TOKEN}` })
        expect(result.stdout).toBe(
            [
                'Logged in as Ada Lovelace (@ada)',
                '',
                `API      ${api.url}`,
                'Token    ci-bot',
                'Scope    read',
                'Project  all projects',
                'Expires  Dec 1, 2026',
                '',
            ].join('\n'),
        )
        secrets(result.stdout, result.stderr)
        expect(result.stdout).not.toContain('zc_')
        expect(result.stdout).not.toContain(credentialsFile(home))
        expect(result.stdout).not.toMatch(/Created|Last used/)
    })

    test('a stored token is never sent to another host', async () => {
        const before = api.seen.length
        const result = await run(['whoami', '--human', '--api', 'https://cloud.zega.world'], { home })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`you are logged in to ${api.url}`)
        expect(result.stderr).toContain('zega login --api https://cloud.zega.world')
        expect(api.seen).toHaveLength(before)
    })

    test('logout and the logged-out state are JSON by default', async () => {
        const dir = scratch()
        try {
            expect((await run(['login', '--api', api.url], { home: dir, stdin: TOKEN })).code).toBe(0)
            const out = await run(['logout'], { home: dir })
            expect(out.code).toBe(0)
            expect(JSON.parse(out.stdout)).toEqual({ loggedOut: true })
            expect(fs.existsSync(credentialsFile(dir))).toBe(false)
            const again = await run(['logout'], { home: dir })
            expect(JSON.parse(again.stdout)).toEqual({ loggedOut: false })
        } finally {
            remove(dir)
        }
    })

    test('logout removes it, and the next command says to log in', async () => {
        const out = await run(['logout', '--human'], { home })
        expect(out.code).toBe(0)
        expect(fs.existsSync(credentialsFile(home))).toBe(false)
        expect(out.stdout).toContain('still valid until you revoke it')

        const after = await run(['whoami', '--human'], { home })
        expect(after.code).toBe(1)
        expect(after.stderr).toContain('not logged in: run `zega login`')
        expect(after.stderr).toContain('--token-file')

        const again = await run(['logout', '--human'], { home })
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
        const second = await run(['login', '--human', '--api', rejecting.url], { home: dir, stdin: bad })
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
        expect(JSON.parse(result.stderr).error).toMatchObject({ code: 'missing_flag' })
        expect(result.stderr).toContain('missing --token-file')
        expect(result.stderr).toContain('piped on stdin or read from a file')
        expect(result.stdout).toBe('')
        expect(api.seen).toHaveLength(before)
        // --human does not change that without a terminal: there is nobody to ask.
        const human = await run(['login', '--human', '--api', api.url], { home: dir, stdin: null })
        expect(human.code).toBe(1)
        expect(human.stderr).toStartWith('zega: missing --token-file')
        expect(api.seen).toHaveLength(before)
    })

    test('a token piped on stdin must be one word', async () => {
        const result = await run(['login', '--api', api.url], { home: dir, stdin: 'zc_one zc_two' })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('one nonempty word')
        expect(result.stderr).not.toContain('zc_one')
    })

    test('by default login prints the whoami answer, without the token prefix', async () => {
        const result = await run(['login', '--api', api.url], { home: dir, stdin: TOKEN })
        expect(result.code).toBe(0)
        expect(JSON.parse(result.stdout)).toEqual(JSON.parse(JSON.stringify(whoamiShown)))
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

describe('who the output says you are', () => {
    const dir = scratch()
    afterAll(() => remove(dir))
    const withBody = (body: unknown) => fakeApi({ 'GET /v1/whoami': { body } })

    test('an account with no full name is just its login', async () => {
        const api = withBody({ account: { login: 'ada', name: null }, token: tokenInfo })
        try {
            const login = await run(['login', '--human', '--api', api.url], { home: dir, stdin: TOKEN })
            expect(login.code).toBe(0)
            expect(login.stdout.split('\n')[0]).toBe('Logged in as @ada')
            const whoami = await run(['whoami', '--human'], { home: dir })
            expect(whoami.stdout.split('\n')[0]).toBe('Logged in as @ada')
        } finally {
            api.stop()
        }
    })

    test('a token for one project, and one that never expires, say so; dates are readable, not ISO', async () => {
        const api = withBody({
            account,
            token: { ...tokenInfo, scope: 'manage', projectId: 'p1', projectName: 'Atlas', expiresAt: null, lastUsedAt: '2026-10-03T12:00:00.000Z' },
        })
        try {
            const result = await run(['login', '--human', '--api', api.url], { home: dir, stdin: TOKEN })
            expect(result.code).toBe(0)
            expect(result.stdout).toContain('Scope    manage')
            expect(result.stdout).toContain('Project  Atlas (p1)')
            expect(result.stdout).toContain('Expires  never')
            expect(result.stdout).not.toMatch(/\d{4}-\d{2}-\d{2}T/)
            expect(result.stdout).not.toContain('Oct 3, 2026') // "last used" is not shown at all
        } finally {
            api.stop()
        }
    })

    test('an API that does not say who the account is yet: the token name, and nothing fails', async () => {
        const api = withBody({ token: tokenInfo })
        try {
            const login = await run(['login', '--human', '--api', api.url], { home: dir, stdin: TOKEN })
            expect(login.code).toBe(0)
            expect(login.stdout).toBe(['Logged in', '', 'Token    ci-bot', 'Scope    read', 'Project  all projects', 'Expires  Dec 1, 2026', ''].join('\n'))
            const whoami = await run(['whoami', '--human'], { home: dir })
            expect(whoami.code).toBe(0)
            expect(whoami.stdout).toBe([`API      ${api.url}`, 'Token    ci-bot', 'Scope    read', 'Project  all projects', 'Expires  Dec 1, 2026', ''].join('\n'))
            for (const out of [login.stdout, whoami.stdout]) expect(out).not.toContain('zc_')
        } finally {
            api.stop()
        }
    })

    test('control characters in a name never reach the terminal', async () => {
        const api = withBody({ account: { login: 'ada', name: 'Ada\u001b[31m Lovelace' }, token: tokenInfo })
        try {
            const result = await run(['login', '--human', '--api', api.url], { home: dir, stdin: TOKEN })
            expect(result.stdout).not.toContain('\u001b')
        } finally {
            api.stop()
        }
    })

    test('by default whoami is the API answer without the token prefix, account included', async () => {
        const api = fakeApi({ ...whoamiRoute })
        try {
            const tokenFile = path.join(dir, 'token')
            fs.writeFileSync(tokenFile, TOKEN, { mode: 0o600 })
            const result = await run(['whoami', '--api', api.url, '--token-file', tokenFile], { home: dir })
            expect(result.code).toBe(0)
            expect(result.stdout).toBe(JSON.stringify(whoamiShown) + '\n')
            expect(JSON.parse(result.stdout).account).toEqual(account)
        } finally {
            api.stop()
        }
    })
})

describe('no part of a token is ever shown', () => {
    const dir = scratch()
    afterAll(() => remove(dir))
    const tokenFile = path.join(dir, 'token')
    fs.writeFileSync(tokenFile, TOKEN, { mode: 0o600 })

    /** The API sends the token's prefix (`zc_test_0123`); nothing the CLI prints may carry it. */
    test('whoami and login print no `prefix` key and no `zc_`, by default and with --human', async () => {
        const api = fakeApi({ ...whoamiRoute })
        try {
            // The API does send it: the test is only meaningful if so.
            expect(JSON.stringify(whoamiRoute['GET /v1/whoami'].body)).toContain('"prefix":"zc_test_0123"')
            const runs = [
                await run(['whoami', '--api', api.url, '--token-file', tokenFile], { home: dir }),
                await run(['whoami', '--human', '--api', api.url, '--token-file', tokenFile], { home: dir }),
                await run(['login', '--api', api.url], { home: dir, stdin: TOKEN }),
                await run(['login', '--human', '--api', api.url], { home: dir, stdin: TOKEN }),
            ]
            for (const result of runs) {
                expect(result.code).toBe(0)
                expect(result.stdout + result.stderr).not.toContain('zc_')
                expect(result.stdout + result.stderr).not.toContain('prefix')
            }
            // The JSON is still the whole answer, minus that one field.
            const json = JSON.parse(runs[0]!.stdout)
            expect(json).toEqual(whoamiShown)
            expect('prefix' in json.token).toBe(false)
            expect(json.token.name).toBe('ci-bot')
        } finally {
            api.stop()
        }
    })
})
