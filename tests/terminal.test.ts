import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, credentialsFile, fakeApi, graph, regionsBody, remove, scratch, whoamiRoute, whoamiShown } from './helpers'

/**
 * A real terminal (a pty). With `--human` the wizard asks, takes typed answers, and the
 * hidden token prompt never echoes the token. Without it nothing is asked and everything is
 * JSON, terminal or not: no output is chosen by whether one is attached.
 */
const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')

async function onTerminal(args: string[], options: { home: string; cwd: string; env?: Record<string, string> }) {
    let output = ''
    const child = Bun.spawn([process.execPath, ENTRY, ...args], {
        cwd: options.cwd,
        env: { PATH: process.env.PATH ?? '', HOME: options.home, XDG_CONFIG_HOME: path.join(options.home, '.config'), ...options.env },
        terminal: {
            cols: 100,
            rows: 30,
            data(_terminal, data) {
                output += new TextDecoder().decode(data)
            },
        },
    })
    return {
        get output() {
            return output
        },
        async waitFor(text: string) {
            const deadline = Date.now() + 15_000
            while (!output.includes(text)) {
                if (Date.now() > deadline) throw new Error(`timed out waiting for ${JSON.stringify(text)}; saw ${JSON.stringify(output)}`)
                await Bun.sleep(25)
            }
        },
        type(text: string) {
            child.terminal?.write(text)
        },
        exited: child.exited,
    }
}

const api = fakeApi({ ...whoamiRoute, 'GET /v1/regions': { body: regionsBody }, 'GET /v1/graphs': { body: { graphs: [graph] } } })
const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => {
    api.stop()
    remove(home)
})

describe.skipIf(process.platform === 'win32')('on a terminal', () => {
    test('init asks which starter first, with the function selected and listed first; Enter takes it, and a down arrow moves to nextjs', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['init', '--human', '--name', 'picked', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            await session.waitFor('Starter:')
            await session.waitFor('astro: an Astro app')
            // The function is the first choice, and the one the cursor is on.
            expect(session.output.indexOf('function: one Workers-style function')).toBeLessThan(session.output.indexOf('nextjs: a Next.js app'))
            session.type('\u001b[B') // down: nextjs
            session.type('\r')
            expect(await session.exited).toBe(0)
            const config = JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))
            expect(config).toMatchObject({ name: 'picked', kind: 'app', framework: 'nextjs' })
            expect(fs.existsSync(path.join(dir, 'app', 'page.tsx'))).toBe(true)

            const plain = scratch()
            try {
                const again = await onTerminal(['init', '--human', '--name', 'plain', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], { home, cwd: plain })
                await again.waitFor('Starter:')
                again.type('\r') // the function, as it is preselected
                await again.waitFor('Function id to deploy')
                again.type('\r')
                expect(await again.exited).toBe(0)
                expect(fs.existsSync(path.join(plain, 'src', 'index.ts'))).toBe(true)
                expect(JSON.parse(fs.readFileSync(path.join(plain, 'package.json'), 'utf8')).dependencies['@zegadb/lib']).toBeDefined()
            } finally {
                remove(plain)
            }
        } finally {
            remove(dir)
        }
    })

    test('init asks for the name, then the region, and writes what was answered', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['init', '--human', '--template', 'function', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            await session.waitFor('Project name:')
            session.type('wizard-app\r')
            await session.waitFor('Region:')
            // The choices carry the API's city names.
            await session.waitFor('ams  Amsterdam, Netherlands')
            session.type('\u001b[B') // down: the second choice
            session.type('\r')
            // The function is optional: an empty answer is none.
            await session.waitFor('Function id to deploy')
            session.type('\r')
            expect(await session.exited).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'wizard-app',
                region: 'ams',
            })
        } finally {
            remove(dir)
        }
    })

    test('a flag answers its question, so only the other is asked', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['init', '--human', '--template', 'function', '--name', 'flagged', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            await session.waitFor('Region:')
            expect(session.output).not.toContain('Project name:')
            session.type('\r')
            await session.waitFor('Function id to deploy')
            session.type('\r')
            expect(await session.exited).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).name).toBe('flagged')
        } finally {
            remove(dir)
        }
    })

    test('init can tie a function to its entry file, asking for the entry with a default', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['init', '--human', '--template', 'function', '--name', 'fn-app', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            await session.waitFor('Function id to deploy')
            session.type('f0bbbbbbbbbbbbbbbb\r')
            await session.waitFor('Entry file to bundle:')
            session.type('\r') // the default
            expect(await session.exited).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'fn-app',
                region: 'yyz',
                functions: [{ id: 'f0bbbbbbbbbbbbbbbb', entry: 'src/index.ts' }],
            })
        } finally {
            remove(dir)
        }
    })

    test('an existing zega.json is asked about, and a no leaves it alone', async () => {
        const dir = scratch()
        try {
            fs.writeFileSync(path.join(dir, 'zega.json'), '{"version":1,"name":"old","region":"yyz"}\n')
            const session = await onTerminal(['init', '--human', '--template', 'function', '--name', 'new', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            await session.waitFor('zega.json exists. Replace it?')
            session.type('n\r')
            expect(await session.exited).toBe(1)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).name).toBe('old')
        } finally {
            remove(dir)
        }
    })

    test('login asks for the token with the input hidden, and stores it', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['login', '--human', '--api', api.url], { home: dir, cwd: dir })
            await session.waitFor('input is hidden):')
            session.type(TOKEN)
            session.type('\r')
            expect(await session.exited).toBe(0)
            expect(session.output).not.toContain(TOKEN)
            expect(JSON.parse(fs.readFileSync(credentialsFile(dir), 'utf8')).token).toBe(TOKEN)
        } finally {
            remove(dir)
        }
    })

    test('the default on a terminal is JSON: whoami, regions and graphs print the API answer, with no table and no colour', async () => {
        for (const [args, route] of [
            [['whoami'], 'GET /v1/whoami'],
            [['regions'], 'GET /v1/regions'],
            [['graphs'], 'GET /v1/graphs'],
        ] as const) {
            const session = await onTerminal([...args, '--api', api.url, '--token-file', tokenFile], { home, cwd: home })
            expect(await session.exited).toBe(0)
            // On a terminal the JSON is pretty-printed (a pty turns \n into \r\n); it is the API's answer all the same.
            const answers: Record<string, unknown> = {
                'GET /v1/whoami': whoamiShown, // the API's answer without the token's prefix
                'GET /v1/regions': regionsBody,
                'GET /v1/graphs': { graphs: [graph] },
            }
            expect(session.output.replace(/\r\n/g, '\n')).toBe(JSON.stringify(answers[route], null, 2) + '\n')
            expect(session.output).not.toContain('\u001b')
        }
    })

    test('--human on a terminal is the table', async () => {
        const session = await onTerminal(['regions', '--human', '--api', api.url, '--token-file', tokenFile], { home, cwd: home })
        expect(await session.exited).toBe(0)
        expect(session.output).toContain('ID   CITY       COUNTRY      CONTINENT')
        expect(() => JSON.parse(session.output)).toThrow()
    })

    test('without --human nothing is asked, even on a terminal: init and login fail at once, as JSON on stderr', async () => {
        const dir = scratch()
        try {
            const init = await onTerminal(['init', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(await init.exited).toBe(1)
            // The pty carries stderr too: it is one JSON object naming the flags.
            const initError = JSON.parse(init.output.trim()).error
            expect(initError).toMatchObject({ code: 'missing_flag' })
            expect(initError.message).toContain('missing --name, --region')
            expect(init.output).not.toContain('Project name:')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)

            const login = await onTerminal(['login', '--api', api.url], { home: dir, cwd: dir })
            expect(await login.exited).toBe(1)
            expect(login.output).not.toContain('input is hidden')
            const loginError = JSON.parse(login.output).error
            expect(loginError).toMatchObject({ code: 'missing_flag' })
            expect(loginError.message).toContain('missing --token-file')
            expect(fs.existsSync(credentialsFile(dir))).toBe(false)
        } finally {
            remove(dir)
        }
    })

    test('errors on a terminal are JSON too, and --human makes them text', async () => {
        const dir = scratch()
        try {
            const json = await onTerminal(['whoami'], { home: dir, cwd: dir })
            expect(await json.exited).toBe(1)
            expect(JSON.parse(json.output.trim()).error.code).toBe('not_logged_in')
            const text = await onTerminal(['whoami', '--human'], { home: dir, cwd: dir })
            expect(await text.exited).toBe(1)
            expect(text.output).toStartWith('zega: not logged in: run `zega login`')
        } finally {
            remove(dir)
        }
    })

    test('help is JSON on a terminal as well, with the mark above it and colour in it; --human changes nothing', async () => {
        const plain = await onTerminal(['--help'], { home, cwd: home, env: { TERM: 'xterm-256color' } })
        expect(await plain.exited).toBe(0)
        expect(plain.output).toContain('\u001b[36m"name"') // keys, strings and punctuation each have a colour
        const text = plain.output.replace(/\u001b\[[0-9;]*m/g, '').replace(/\r\n/g, '\n')
        expect(text).toContain('▐██████▐██████▐██████▐██████')
        expect(JSON.parse(text.slice(text.indexOf('{'))).name).toBe('zega')
        const human = await onTerminal(['--help', '--human'], { home, cwd: home, env: { TERM: 'xterm-256color' } })
        expect(await human.exited).toBe(0)
        expect(human.output).toBe(plain.output)
    })
})
