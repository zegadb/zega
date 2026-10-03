import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, credentialsFile, fakeApi, regionsBody, remove, scratch, whoamiRoute } from './helpers'

/**
 * The wizard on a real terminal (a pty): it asks, takes typed answers, and the
 * hidden token prompt never echoes the token.
 */
const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')

async function onTerminal(args: string[], options: { home: string; cwd: string }) {
    let output = ''
    const child = Bun.spawn([process.execPath, ENTRY, ...args], {
        cwd: options.cwd,
        env: { PATH: process.env.PATH ?? '', HOME: options.home, XDG_CONFIG_HOME: path.join(options.home, '.config') },
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

const api = fakeApi({ ...whoamiRoute, 'GET /v1/regions': { body: regionsBody } })
const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => {
    api.stop()
    remove(home)
})

describe.skipIf(process.platform === 'win32')('on a terminal', () => {
    test('init asks for the name, then the region, and writes what was answered', async () => {
        const dir = scratch()
        try {
            const session = await onTerminal(['init', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
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
            const session = await onTerminal(['init', '--name', 'flagged', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
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
            const session = await onTerminal(['init', '--name', 'fn-app', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
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
            const session = await onTerminal(['init', '--name', 'new', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
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
            const session = await onTerminal(['login', '--api', api.url], { home: dir, cwd: dir })
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
})
