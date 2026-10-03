import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FIXTURES, FUNCTION_ID, TOKEN, callBundle, deployedFunction, fakeApi, remove, run, scratch } from './helpers'
import { helloProject, project } from './dev_helpers'

const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => remove(home))

const CODE_PATH = `/v1/functions/${FUNCTION_ID}/code`

/** The hello fixture, listed as FUNCTION_ID (the same id as the deploy tests'). */
const hello = helloProject

describe('zega build', () => {
    test('writes exactly the module zega deploy uploads, and sends nothing anywhere', async () => {
        const dir = hello()
        const api = fakeApi({ [`PUT ${CODE_PATH}`]: { body: { function: deployedFunction } } })
        try {
            const built = await run(['build'], { home, cwd: dir })
            expect(built.code).toBe(0)
            const file = path.join(dir, 'dist', `${FUNCTION_ID}.js`)
            const onDisk = fs.readFileSync(file, 'utf8')
            expect(built.stdout).toContain(`Built  ${FUNCTION_ID}  src/index.ts -> dist/${FUNCTION_ID}.js (`)
            expect(built.stderr).toMatch(/bundled src\/index\.ts: [\d,]+ bytes/)
            // It needed no token: HOME is empty and nothing was passed.
            expect(fs.existsSync(path.join(home, '.config'))).toBe(false)

            const deployed = await run(['deploy', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(deployed.code).toBe(0)
            expect(api.seen).toHaveLength(1)
            // The same bytes: the file `build` wrote is the body `deploy` sent.
            expect(onDisk).toBe(api.seen[0]!.body)
            expect(await callBundle(onDisk, 'https://f.example/abc')).toMatch(/^HELLO FROM WORKERD \/ABC [A-Za-z0-9_-]{12}$/)
        } finally {
            api.stop()
            remove(dir)
        }
    })

    test('--minify matches deploy --minify; --out chooses the directory; --json lists what was written', async () => {
        const dir = hello()
        const api = fakeApi({ [`PUT ${CODE_PATH}`]: { body: { function: deployedFunction } } })
        try {
            const built = await run(['build', '--minify', '--out', 'out/js', '--json'], { home, cwd: dir })
            expect(built.code).toBe(0)
            const files = JSON.parse(built.stdout) as { id: string; entry: string; file: string; bytes: number }[]
            expect(files).toEqual([
                {
                    id: FUNCTION_ID,
                    entry: 'src/index.ts',
                    file: `out/js/${FUNCTION_ID}.js`,
                    bytes: Buffer.byteLength(fs.readFileSync(path.join(dir, files[0]!.file), 'utf8')),
                },
            ])
            expect(fs.existsSync(path.join(dir, 'dist'))).toBe(false)
            await run(['deploy', '--minify', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(fs.readFileSync(path.join(dir, files[0]!.file), 'utf8')).toBe(api.seen[0]!.body)
        } finally {
            api.stop()
            remove(dir)
        }
    })

    test('builds only the id it is given, and says what is wrong when it cannot', async () => {
        const dir = hello()
        try {
            const unknown = await run(['build', 'f0zzzzzzzzzzzzzzzz'], { home, cwd: dir })
            expect(unknown.code).toBe(1)
            expect(unknown.stderr).toContain(`f0zzzzzzzzzzzzzzzz is not in zega.json; it lists ${FUNCTION_ID}`)

            const none = await run(['build'], { home, cwd: home })
            expect(none.code).toBe(1)
            expect(none.stderr).toContain('no zega.json here')
            expect(none.stderr).not.toContain('zega function deploy')
        } finally {
            remove(dir)
        }
    })

    test('refuses what deploy refuses (Node built-ins), writing nothing', async () => {
        const dir = scratch()
        fs.cpSync(path.join(FIXTURES, 'builtins'), dir, { recursive: true })
        project(dir, [{ id: FUNCTION_ID, entry: 'src/crypto.ts' }])
        try {
            const built = await run(['build'], { home, cwd: dir })
            expect(built.code).toBe(1)
            expect(built.stderr).toContain('Node built-ins are not available in Zega Cloud functions yet')
            expect(fs.existsSync(path.join(dir, 'dist'))).toBe(false)
        } finally {
            remove(dir)
        }
    })
})
