import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, fakeApi, regionsBody, remove, run, scratch } from './helpers'

const api = fakeApi({ 'GET /v1/regions': { body: regionsBody } })
const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => {
    api.stop()
    remove(home)
})

function project(): string {
    return scratch()
}

describe('zega init', () => {
    test('with flags: writes zega.json and creates nothing in the cloud', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--name', 'my-app', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            expect(result.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'my-app',
                region: 'yyz',
            })
            expect(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).toBe(
                '{\n  "version": 1,\n  "name": "my-app",\n  "region": "yyz"\n}\n',
            )
            expect(result.stdout).toContain(`wrote ${fs.realpathSync(dir)}${path.sep}zega.json`)
            // The only thing it asked the API was the list of regions: nothing was created.
            expect(api.seen.at(-1)).toMatchObject({ method: 'GET', pathname: '/v1/regions', authorization: `Bearer ${TOKEN}` })
            expect(api.seen.every((request) => request.method === 'GET')).toBe(true)
        } finally {
            remove(dir)
        }
    })

    test('without a terminal and without flags: fails at once naming every missing flag, writes nothing', async () => {
        const dir = project()
        try {
            const started = Date.now()
            const result = await run(['init'], { home, cwd: dir, stdin: null })
            expect(Date.now() - started).toBeLessThan(5000)
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('missing --name, --region')
            expect(result.stderr).toContain('no terminal')
            expect(result.stdout).toBe('')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)

            const nameOnly = await run(['init', '--region', 'yyz'], { home, cwd: dir, stdin: null })
            expect(nameOnly.code).toBe(1)
            expect(nameOnly.stderr).toContain('missing --name')
            expect(nameOnly.stderr).not.toContain('--region')
        } finally {
            remove(dir)
        }
    })

    test('--json never asks either', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--json'], { home, cwd: dir, stdin: null })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('missing --name, --region')
            const done = await run(['init', '--json', '--name', 'a', '--region', 'yyz'], { home, cwd: dir, stdin: null })
            expect(done.code).toBe(0)
            const printed = JSON.parse(done.stdout)
            expect(printed.config).toEqual({ version: 1, name: 'a', region: 'yyz' })
            expect(path.basename(printed.file)).toBe('zega.json')
        } finally {
            remove(dir)
        }
    })

    test('a region the API does not list is refused, listing the ones it does', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--name', 'x', '--region', 'moon', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('--region: "moon" is not a region; the regions are yyz, ams')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)
        } finally {
            remove(dir)
        }
    })

    test('without credentials the region is checked against the list built into the CLI', async () => {
        const dir = project()
        try {
            const ok = await run(['init', '--name', 'x', '--region', 'sjc'], { home, cwd: dir })
            expect(ok.code).toBe(0)
            const bad = await run(['init', '--name', 'x', '--region', 'moon', '--force'], { home, cwd: dir })
            expect(bad.code).toBe(1)
            expect(bad.stderr).toContain('"moon" is not a region')
            expect(bad.stderr).toContain('dfw')
        } finally {
            remove(dir)
        }
    })

    test('a name is 1 to 64 characters', async () => {
        const dir = project()
        try {
            const long = await run(['init', '--name', 'x'.repeat(65), '--region', 'yyz'], { home, cwd: dir })
            expect(long.code).toBe(1)
            expect(long.stderr).toContain('--name: a project name is 1 to 64 characters')
            const blank = await run(['init', '--name', '   ', '--region', 'yyz'], { home, cwd: dir })
            expect(blank.code).toBe(1)
        } finally {
            remove(dir)
        }
    })

    test('an existing zega.json is kept unless --force, and --force replaces it', async () => {
        const dir = project()
        try {
            const first = await run(['init', '--name', 'one', '--region', 'yyz'], { home, cwd: dir })
            expect(first.code).toBe(0)
            const again = await run(['init', '--name', 'two', '--region', 'ams'], { home, cwd: dir, stdin: null })
            expect(again.code).toBe(1)
            expect(again.stderr).toContain('exists already')
            expect(again.stderr).toContain('--force')
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).name).toBe('one')

            const forced = await run(['init', '--name', 'two', '--region', 'ams', '--force'], { home, cwd: dir })
            expect(forced.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'two',
                region: 'ams',
            })
        } finally {
            remove(dir)
        }
    })

    test('--function ties a function to its entry (src/index.ts unless --entry); nothing is asked without a terminal', async () => {
        const dir = project()
        try {
            const first = await run(['init', '--name', 'a', '--region', 'yyz', '--function', 'f0bbbbbbbbbbbbbbbb'], { home, cwd: dir, stdin: null })
            expect(first.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'a',
                region: 'yyz',
                functions: [{ id: 'f0bbbbbbbbbbbbbbbb', entry: 'src/index.ts' }],
            })
            expect(first.stdout).toContain('Deploys  f0bbbbbbbbbbbbbbbb from src/index.ts')

            const second = await run(
                ['init', '--name', 'a', '--region', 'yyz', '--function', 'f0bbbbbbbbbbbbbbbb', '--entry', 'worker/main.ts', '--force'],
                { home, cwd: dir, stdin: null },
            )
            expect(second.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).functions).toEqual([
                { id: 'f0bbbbbbbbbbbbbbbb', entry: 'worker/main.ts' },
            ])
        } finally {
            remove(dir)
        }
    })

    test('a function id that is not one, and --entry without --function, are refused and write nothing', async () => {
        const dir = project()
        try {
            const bad = await run(['init', '--name', 'a', '--region', 'yyz', '--function', 'hello'], { home, cwd: dir, stdin: null })
            expect(bad.code).toBe(1)
            expect(bad.stderr).toContain('--function: a function id is "f0" and 16 characters')
            const orphan = await run(['init', '--name', 'a', '--region', 'yyz', '--entry', 'x.ts'], { home, cwd: dir, stdin: null })
            expect(orphan.code).toBe(1)
            expect(orphan.stderr).toContain('--entry needs --function')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)
        } finally {
            remove(dir)
        }
    })
})
