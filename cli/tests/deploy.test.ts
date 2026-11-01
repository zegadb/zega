import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import {
    FIXTURES,
    FUNCTION_ID,
    TOKEN,
    callBundle,
    deployedFunction,
    fakeApi,
    remove,
    run,
    scratch,
    type Canned,
} from './helpers'

const CODE_PATH = `/v1/functions/${FUNCTION_ID}/code`
const ok: Canned = { body: { function: deployedFunction } }

const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => remove(home))

const hello = path.join(FIXTURES, 'hello')
const builtins = path.join(FIXTURES, 'builtins')

async function deployIn(cwd: string, args: string[], route: Canned = ok) {
    const api = fakeApi({ [`PUT ${CODE_PATH}`]: route })
    try {
        const result = await run([...args, '--api', api.url, '--token-file', tokenFile], { home, cwd })
        return { result, seen: api.seen }
    } finally {
        api.stop()
    }
}

describe('zega function deploy', () => {
    test('uploads one ES module: PUT, the path, the manage token, application/javascript, the code as the body', async () => {
        const { result, seen } = await deployIn(hello, ['function', 'deploy', FUNCTION_ID, 'src/index.ts', '--human'])
        expect(result.code).toBe(0)
        expect(seen).toHaveLength(1)
        const request = seen[0]!
        expect(request).toMatchObject({
            method: 'PUT',
            pathname: CODE_PATH,
            authorization: `Bearer ${TOKEN}`,
            contentType: 'application/javascript',
        })

        // One module: exports a default fetch handler, carries the dependency's code and the local files', and
        // nothing a Workers runtime cannot load.
        const code = request.body
        expect(code).toMatch(/\bas default\b|\bexport default\b/)
        expect(code).toContain('useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict') // nanoid
        expect(code).toContain('hello from workerd') // the `workerd` condition picked the Workers build
        expect(code).not.toContain('node:os')
        expect(code).not.toContain('process.env')
        expect(code).not.toMatch(/^import /m)
        expect(code).not.toContain('sourceMappingURL')

        // And it runs: loaded as a module, its handler answers.
        const answer = await callBundle(code, 'https://f.example/abc')
        expect(answer).toMatch(/^HELLO FROM WORKERD \/ABC [A-Za-z0-9_-]{12}$/)

        expect(result.stderr).toMatch(/bundled src\/index\.ts: [\d,]+ bytes \(limit 1,000,000\)/)
        expect(result.stdout).toContain(`Deployed  ${FUNCTION_ID} (hello) as v4`)
        expect(result.stdout).toContain(`Address   https://${FUNCTION_ID}.zegadb.com`)
        expect(result.stdout + result.stderr).not.toContain(TOKEN)
    })

    test('by default it prints the API answer unchanged and nothing else: the bundle size line is --human', async () => {
        const { result } = await deployIn(hello, ['function', 'deploy', FUNCTION_ID, 'src/index.ts'])
        expect(result.code).toBe(0)
        expect(result.stdout).toBe(JSON.stringify(ok.body) + '\n')
        expect(result.stderr).toBe('')
    })

    test('--minify makes a smaller module that still runs', async () => {
        const plain = await deployIn(hello, ['function', 'deploy', FUNCTION_ID, 'src/index.ts'])
        const small = await deployIn(hello, ['function', 'deploy', FUNCTION_ID, 'src/index.ts', '--minify'])
        expect(small.result.code).toBe(0)
        expect(small.seen[0]!.body.length).toBeLessThan(plain.seen[0]!.body.length)
        expect(await callBundle(small.seen[0]!.body, 'https://f.example/x')).toMatch(/^HELLO FROM WORKERD \/X /)
    })

    test('the entry is found in zega.json when it is not given', async () => {
        const dir = scratch()
        try {
            fs.cpSync(hello, dir, { recursive: true })
            fs.writeFileSync(
                path.join(dir, 'zega.json'),
                JSON.stringify({ version: 1, name: 'x', region: 'yyz', functions: [{ id: FUNCTION_ID, entry: 'src/index.ts' }] }),
            )
            const { result, seen } = await deployIn(dir, ['function', 'deploy', FUNCTION_ID])
            expect(result.code).toBe(0)
            expect(seen).toHaveLength(1)
        } finally {
            remove(dir)
        }
    })

    test('no entry anywhere: fails naming what to pass, asks nothing, sends nothing', async () => {
        const dir = scratch()
        try {
            const { result, seen } = await deployIn(dir, ['function', 'deploy', FUNCTION_ID])
            expect(result.code).toBe(1)
            expect(result.stderr).toContain(`no entry for ${FUNCTION_ID}`)
            expect(result.stderr).toContain('zega function deploy ' + FUNCTION_ID + ' <entry>')
            expect(seen).toHaveLength(0)
        } finally {
            remove(dir)
        }
    })

    test('an entry that is not there, a bundle error, and a module with no default export', async () => {
        const dir = scratch()
        try {
            const missing = await deployIn(dir, ['function', 'deploy', FUNCTION_ID, 'nope.ts'])
            expect(missing.result.code).toBe(1)
            expect(missing.result.stderr).toContain('entry nope.ts is not a file')

            fs.writeFileSync(path.join(dir, 'bad.ts'), "import { nothing } from './not-here'\nexport default { fetch() { return nothing } }\n")
            const bad = await deployIn(dir, ['function', 'deploy', FUNCTION_ID, 'bad.ts'])
            expect(bad.result.code).toBe(1)
            expect(bad.result.stderr).toContain('cannot bundle bad.ts')
            expect(bad.result.stderr).toContain('(bad.ts:1)')

            fs.writeFileSync(path.join(dir, 'named.ts'), 'export const fetch = () => new Response("x")\n')
            const named = await deployIn(dir, ['function', 'deploy', FUNCTION_ID, 'named.ts'])
            expect(named.result.code).toBe(1)
            expect(named.result.stderr).toContain('named.ts has no default export')
            expect(bad.seen.length + named.seen.length + missing.seen.length).toBe(0)
        } finally {
            remove(dir)
        }
    })

    test('a bundle over the API limit is refused before anything is sent, with its size', async () => {
        const dir = scratch()
        try {
            fs.writeFileSync(
                path.join(dir, 'big.ts'),
                `const blob = ${JSON.stringify('x'.repeat(1_100_000))}\nexport default { fetch() { return new Response(blob) } }\n`,
            )
            const { result, seen } = await deployIn(dir, ['function', 'deploy', FUNCTION_ID, 'big.ts', '--human'])
            expect(result.code).toBe(1)
            expect(seen).toHaveLength(0)
            expect(result.stderr).toMatch(/bundled big\.ts: 1,1\d\d,\d{3} bytes \(limit 1,000,000\)/)
            expect(result.stderr).toMatch(/the bundle for big\.ts is 1,1\d\d,\d{3} bytes; a function is at most 1,000,000 bytes\. Try --minify/)
        } finally {
            remove(dir)
        }
    })
})

describe('Node built-ins', () => {
    const message = 'Node built-ins are not available in Zega Cloud functions yet'

    test('node:crypto in the entry is refused, naming it and the file; nothing is sent, no token needed', async () => {
        const api = fakeApi({})
        try {
            // No token at all: the refusal comes before the CLI asks for one.
            const result = await run(['function', 'deploy', FUNCTION_ID, 'src/crypto.ts', '--api', api.url], { home, cwd: builtins })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain(`${message}: node:crypto in src/crypto.ts`)
            expect(result.stdout).toBe('')
            expect(api.seen).toHaveLength(0)
        } finally {
            api.stop()
        }
    })

    test('a bare built-in name is one too', async () => {
        const { result, seen } = await deployIn(builtins, ['function', 'deploy', FUNCTION_ID, 'src/bare.ts'])
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`${message}: path in src/bare.ts`)
        expect(seen).toHaveLength(0)
    })

    test('one inside a dependency is found, naming the dependency file', async () => {
        const { result, seen } = await deployIn(builtins, ['function', 'deploy', FUNCTION_ID, 'src/dependency.ts'])
        expect(result.code).toBe(1)
        expect(result.stderr).toContain(`${message}: node:buffer in node_modules/legacy/index.js`)
        expect(seen).toHaveLength(0)
    })

    test('a dependency whose Node build imports a built-in is fine when it has a Workers build', async () => {
        // edge-greeting's default build imports node:os, its `workerd` build does not: the conditions choose.
        const { result } = await deployIn(hello, ['function', 'deploy', FUNCTION_ID, 'src/index.ts'])
        expect(result.code).toBe(0)
    })

    test('cloudflare: modules stay imports, they are the runtime\'s own', async () => {
        const dir = scratch()
        try {
            fs.writeFileSync(
                path.join(dir, 'cf.ts'),
                "import { env } from 'cloudflare:workers'\nexport default { fetch() { return new Response(String(env)) } }\n",
            )
            const { result, seen } = await deployIn(dir, ['function', 'deploy', FUNCTION_ID, 'cf.ts'])
            expect(result.code).toBe(0)
            expect(seen[0]!.body).toContain('from "cloudflare:workers"')
        } finally {
            remove(dir)
        }
    })
})

describe('what the API says', () => {
    // --human: these are the words people read (errors.test.ts has the JSON).
    const args = ['function', 'deploy', FUNCTION_ID, 'src/index.ts', '--human']

    test('403 scope_required: a read token cannot deploy, and the CLI says to make a manage token', async () => {
        const { result } = await deployIn(
            hello,
            args,
            { status: 403, body: { ok: false, code: 'scope_required', error: 'This token can only read.' } },
        )
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('(code: scope_required, HTTP 403)')
        expect(result.stderr).toContain('This needs a manage token. Make one at https://dashboard.zega.dev/tokens')
        expect(result.stdout).toBe('')
    })

    test('413 and 400 are relayed in the API\'s own words', async () => {
        const tooLarge = await deployIn(hello, args, {
            status: 413,
            body: { ok: false, code: 'code_too_large', error: 'A function is one module of at most 1000000 bytes. Bundle it first.' },
        })
        expect(tooLarge.result.code).toBe(1)
        expect(tooLarge.result.stderr).toContain('A function is one module of at most 1000000 bytes. Bundle it first. (code: code_too_large, HTTP 413)')

        const rejected = await deployIn(hello, args, {
            status: 400,
            body: { ok: false, code: 'code_rejected', error: 'Uncaught SyntaxError: nope at worker.js:1:1' },
        })
        expect(rejected.result.code).toBe(1)
        expect(rejected.result.stderr).toContain('Uncaught SyntaxError: nope at worker.js:1:1 (code: code_rejected, HTTP 400)')
    })

    test('404 function_not_found and 429 are relayed too; a 429 is not retried', async () => {
        const missing = await deployIn(hello, args, {
            status: 404,
            body: { ok: false, code: 'function_not_found', error: 'No such function of this account.' },
        })
        expect(missing.result.stderr).toContain('code: function_not_found')

        const limited = await deployIn(hello, args, {
            status: 429,
            body: { ok: false, code: 'rate_limited', error: 'Slow down.', retryAfterSeconds: 12 },
        })
        expect(limited.result.stderr).toContain('Wait 12 seconds')
        expect(limited.seen).toHaveLength(1)
    })

    test('not logged in: says how to log in, after the bundle is made', async () => {
        const result = await run([...args], { home, cwd: hello })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('not logged in: run `zega login`')
    })
})

describe('zega deploy', () => {
    const second = 'f0cccccccccccccccc'
    const project = (dir: string, functions: unknown) =>
        fs.writeFileSync(path.join(dir, 'zega.json'), JSON.stringify({ version: 1, name: 'x', region: 'yyz', functions }))

    function twoFunctions(): string {
        const dir = scratch()
        fs.cpSync(hello, dir, { recursive: true })
        fs.writeFileSync(path.join(dir, 'src', 'other.ts'), "export default { fetch() { return new Response('other') } }\n")
        project(dir, [
            { id: FUNCTION_ID, entry: 'src/index.ts' },
            { id: second, entry: 'src/other.ts' },
        ])
        return dir
    }

    async function deployAll(dir: string, args: string[]) {
        const api = fakeApi({
            [`PUT ${CODE_PATH}`]: ok,
            [`PUT /v1/functions/${second}/code`]: { body: { function: { ...deployedFunction, id: second, name: 'other' } } },
        })
        try {
            const result = await run(['deploy', ...args, '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            return { result, seen: api.seen }
        } finally {
            api.stop()
        }
    }

    test('deploys every function zega.json lists, each from its own entry', async () => {
        const dir = twoFunctions()
        try {
            const { result, seen } = await deployAll(dir, ['--human'])
            expect(result.code).toBe(0)
            expect(seen.map((r) => `${r.method} ${r.pathname}`)).toEqual([`PUT ${CODE_PATH}`, `PUT /v1/functions/${second}/code`])
            expect(await callBundle(seen[1]!.body, 'https://f.example/')).toBe('other')
            expect(result.stdout).toContain(`${FUNCTION_ID} (hello)`)
            expect(result.stdout).toContain(`${second} (other)`)
        } finally {
            remove(dir)
        }
    })

    test('with an id, only that function', async () => {
        const dir = twoFunctions()
        try {
            const { result, seen } = await deployAll(dir, [second])
            expect(result.code).toBe(0)
            expect(seen.map((r) => r.pathname)).toEqual([`/v1/functions/${second}/code`])
        } finally {
            remove(dir)
        }
    })

    test('one that cannot be bundled stops the whole deploy before any upload', async () => {
        const dir = twoFunctions()
        try {
            project(dir, [
                { id: FUNCTION_ID, entry: 'src/index.ts' },
                { id: second, entry: 'src/missing.ts' },
            ])
            const { result, seen } = await deployAll(dir, [])
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('entry src/missing.ts is not a file')
            expect(seen).toHaveLength(0)
        } finally {
            remove(dir)
        }
    })

    test('by default one API answer per function, unchanged', async () => {
        const dir = twoFunctions()
        try {
            const { result } = await deployAll(dir, [])
            const lines = result.stdout.trim().split('\n')
            expect(lines).toHaveLength(2)
            expect(JSON.parse(lines[0]!).function.id).toBe(FUNCTION_ID)
            expect(JSON.parse(lines[1]!).function.id).toBe(second)
        } finally {
            remove(dir)
        }
    })

    test('no zega.json, no functions, an id it does not list, a broken file: each says what to do', async () => {
        const dir = scratch()
        try {
            const none = await deployAll(dir, [])
            expect(none.result.code).toBe(1)
            expect(none.result.stderr).toContain('no zega.json here')
            expect(none.result.stderr).toContain('zega function deploy <id> <entry>')

            project(dir, undefined)
            const empty = await deployAll(dir, [])
            expect(empty.result.stderr).toContain('lists no functions')

            project(dir, [{ id: FUNCTION_ID, entry: 'src/index.ts' }])
            const unknown = await deployAll(dir, [second])
            expect(unknown.result.stderr).toContain(`${second} is not in zega.json; it lists ${FUNCTION_ID}`)

            project(dir, [{ id: 'nope', entry: 'x.ts' }])
            const broken = await deployAll(dir, [])
            expect(broken.result.stderr).toContain('functions[0].id must be a function id')
            expect(none.seen.length + empty.seen.length + unknown.seen.length + broken.seen.length).toBe(0)
        } finally {
            remove(dir)
        }
    })
})
