import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { TOKEN, errorOf, fakeApi, regionsBody, remove, run, scratch } from './helpers'
import { installDependencies, packageManager } from '@/core/init/init'

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
    test('creates a named folder and refuses a nonempty target', async () => {
        const dir = project()
        try {
            const created = await run(['init', '--name', 'new-app', '--region', 'yyz'], { home, cwd: dir })
            expect(created.code).toBe(0)
            expect(fs.existsSync(path.join(dir, 'new-app', 'zega.json'))).toBe(true)
            fs.mkdirSync(path.join(dir, 'occupied'))
            fs.writeFileSync(path.join(dir, 'occupied', 'sentinel'), 'keep')
            const refused = await run(['init', '--name', 'occupied', '--region', 'yyz'], { home, cwd: dir })
            expect(refused.code).toBe(1)
            expect(refused.stderr).toContain('not empty')
            expect(fs.existsSync(path.join(dir, 'occupied', 'zega.json'))).toBe(false)
        } finally { remove(dir) }
    })

    test('detects the package manager from its user agent', () => {
        expect(packageManager('bun/1.2.0')).toBe('bun')
        expect(packageManager('npm/10.0.0 node/v22')).toBe('npm')
        expect(packageManager('pnpm/9.0.0 npm/? node/v22')).toBe('pnpm')
        expect(packageManager('yarn/4.0.0 npm/? node/v22')).toBe('yarn')
        expect(packageManager('')).toBe('bun')
    })

    test('runs the package manager install in the project directory', async () => {
        const calls: unknown[][] = []
        const fakeSpawn = ((command: string[], options: { cwd: string }) => {
            calls.push([command, options.cwd])
            return { exited: Promise.resolve(0) }
        }) as typeof Bun.spawn
        await installDependencies('/projects/demo', 'pnpm', fakeSpawn)
        expect(calls).toEqual([[['pnpm', 'install'], '/projects/demo']])
    })
    test('with flags: writes zega.json and creates nothing in the cloud', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--here', '--human', '--name', 'my-app', '--region', 'yyz', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            expect(result.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'my-app',
                region: 'yyz',
                functions: [{ name: 'my-app', entry: 'src/index.ts' }],
            })
            expect(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).toBe(
                '{\n  "version": 1,\n  "name": "my-app",\n  "region": "yyz",\n  "functions": [\n    {\n      "name": "my-app",\n      "entry": "src/index.ts"\n    }\n  ]\n}\n',
            )
            expect(result.stdout).toContain('Created ')
            expect(result.stdout).not.toContain('Files ')
            expect(api.seen).toHaveLength(1)
            expect(api.seen[0]?.pathname).toBe('/v1/regions')
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
            expect(JSON.parse(result.stderr).error).toMatchObject({ code: 'missing_flag' })
            expect(result.stderr).toContain('missing --name')
            expect(result.stderr).toContain('Nothing is asked unless --human is given on a terminal')
            expect(result.stdout).toBe('')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)

            const nameOnly = await run(['init', '--here', '--region', 'yyz'], { home, cwd: dir, stdin: null })
            expect(nameOnly.code).toBe(1)
            expect(nameOnly.stderr).toContain('missing --name')
            expect(nameOnly.stderr).not.toContain('--region')
        } finally {
            remove(dir)
        }
    })

    test('by default it prints JSON and never asks; --human without a terminal has no one to ask either', async () => {
        const dir = project()
        try {
            const result = await run(['init'], { home, cwd: dir, stdin: null })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('missing --name')
            const human = await run(['init', '--here', '--human'], { home, cwd: dir, stdin: null })
            expect(human.code).toBe(1)
            expect(human.stderr).toStartWith('zega: missing --name')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)
            const done = await run(['init', '--here', '--name', 'a', '--region', 'yyz'], { home, cwd: dir, stdin: null })
            expect(done.code).toBe(0)
            const printed = JSON.parse(done.stdout)
            expect(printed.config).toMatchObject({ version: 1, name: 'a', region: 'yyz', functions: [{ name: 'a', entry: 'src/index.ts' }] })
            expect(path.basename(printed.file)).toBe('zega.json')
        } finally {
            remove(dir)
        }
    })

    test('without --region, init uses yyz and reports it in JSON', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--here', '--name', 'no-region'], { home, cwd: dir })
            expect(result.code).toBe(0)
            expect(JSON.parse(result.stdout).config.region).toBe('yyz')
        } finally { remove(dir) }
    })

    test('a region the API does not list is refused, listing the ones it does', async () => {
        const dir = project()
        try {
            const result = await run(['init', '--here', '--name', 'x', '--region', 'moon', '--api', api.url, '--token-file', tokenFile], {
                home,
                cwd: dir,
            })
            expect(result.code).toBe(1)
            expect(errorOf(result)).toMatchObject({ code: 'invalid_flag' })
            expect(errorOf(result).message).toContain('--region: "moon" is not a region')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)
        } finally {
            remove(dir)
        }
    })

    test('without credentials the region is checked against the list built into the CLI', async () => {
        const dir = project()
        try {
            const ok = await run(['init', '--here', '--name', 'x', '--region', 'sjc'], { home, cwd: dir })
            expect(ok.code).toBe(0)
            const bad = await run(['init', '--here', '--name', 'x', '--region', 'moon', '--force'], { home, cwd: dir })
            expect(bad.code).toBe(1)
            expect(errorOf(bad).message).toContain('"moon" is not a region')
            expect(errorOf(bad).message).toContain('dfw')
        } finally {
            remove(dir)
        }
    })

    test('a name is 1 to 64 characters', async () => {
        const dir = project()
        try {
            const long = await run(['init', '--here', '--name', 'x'.repeat(65), '--region', 'yyz'], { home, cwd: dir })
            expect(long.code).toBe(1)
            expect(long.stderr).toContain('--name: a project name is 1 to 64 characters')
            const blank = await run(['init', '--here', '--name', '   ', '--region', 'yyz'], { home, cwd: dir })
            expect(blank.code).toBe(1)
        } finally {
            remove(dir)
        }
    })

    test('an existing zega.json is kept unless --force, and --force replaces it', async () => {
        const dir = project()
        try {
            const first = await run(['init', '--here', '--name', 'one', '--region', 'yyz'], { home, cwd: dir })
            expect(first.code).toBe(0)
            const again = await run(['init', '--here', '--name', 'two', '--region', 'ams'], { home, cwd: dir, stdin: null })
            expect(again.code).toBe(1)
            // zega.json is in the way, and so are the starter's own files.
            expect(again.stderr).toContain('zega.json, package.json')
            expect(again.stderr).toContain('already exists')
            expect(again.stderr).toContain('--force')
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8')).name).toBe('one')

            const forced = await run(['init', '--here', '--name', 'two', '--region', 'ams', '--force'], { home, cwd: dir })
            expect(forced.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'two',
                region: 'ams',
                functions: [{ name: 'two', entry: 'src/index.ts' }],
            })
        } finally {
            remove(dir)
        }
    })

    test('--function ties a function to its entry (src/index.ts unless --entry); nothing is asked without a terminal', async () => {
        const dir = project()
        try {
            const first = await run(['init', '--here', '--human', '--name', 'a', '--region', 'yyz', '--function', 'f0bbbbbbbbbbbbbbbb'], { home, cwd: dir, stdin: null })
            expect(first.code).toBe(0)
            expect(JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))).toEqual({
                version: 1,
                name: 'a',
                region: 'yyz',
                functions: [{ id: 'f0bbbbbbbbbbbbbbbb', entry: 'src/index.ts' }],
            })
            expect(first.stdout).toContain('Function  f0bbbbbbbbbbbbbbbb from src/index.ts')

            const second = await run(
                ['init', '--here', '--name', 'a', '--region', 'yyz', '--function', 'f0bbbbbbbbbbbbbbbb', '--entry', 'worker/main.ts', '--force'],
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
            const bad = await run(['init', '--here', '--name', 'a', '--region', 'yyz', '--function', 'hello'], { home, cwd: dir, stdin: null })
            expect(bad.code).toBe(1)
            expect(errorOf(bad).message).toContain('--function: a function id is "f0" and 16 characters')
            const orphan = await run(['init', '--here', '--name', 'a', '--region', 'yyz', '--entry', 'x.ts'], { home, cwd: dir, stdin: null })
            expect(orphan.code).toBe(1)
            expect(orphan.stderr).toContain('--entry needs --function')
            expect(fs.existsSync(path.join(dir, 'zega.json'))).toBe(false)
        } finally {
            remove(dir)
        }
    })
})
