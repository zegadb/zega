import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { remove, run, scratch } from './helpers'
import { findInstalled, resolveServer, searchRoots, serverPackage, takeServerFlag } from '../src/handlers/server_binary'

const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')
const posix = process.platform !== 'win32'

const home = scratch()
afterAll(() => remove(home))

/** A stand-in for zega-server: prints what it was given, one argument per line, and acts on a few of them. */
function fakeServer(dir: string, name = 'zega-server'): string {
    const file = path.join(dir, name)
    fs.mkdirSync(dir, { recursive: true })
    fs.writeFileSync(
        file,
        `#!/bin/sh
echo "fake-server $#"
for a in "$@"; do echo "arg:$a"; done
for a in "$@"; do case "$a" in
  --exit=*) echo "complaint on stderr" >&2; exit "\${a#--exit=}" ;;
  --wait) trap 'echo got-term; exit 7' TERM; echo ready; while :; do sleep 0.05; done ;;
  --die-by-term) kill -TERM $$ ;;
esac; done
`,
        { mode: 0o755 },
    )
    return file
}

/** A directory laid out like an npm install of zegadb with these platform packages. */
function install(root: string, ids: string[]): Record<string, string> {
    const bins: Record<string, string> = {}
    for (const id of ids) {
        const bin = id.startsWith('win32') ? 'zega-server.exe' : 'zega-server'
        bins[id] = path.join(root, 'node_modules', '@zegadb', `server-${id}`, 'bin', bin)
        fs.mkdirSync(path.dirname(bins[id]!), { recursive: true })
        fs.writeFileSync(bins[id]!, `${id}\n`, { mode: 0o755 })
    }
    return bins
}

describe('which zega-server', () => {
    test('each platform has its own package, and the right binary name', () => {
        expect(serverPackage('darwin', 'arm64')).toEqual({ name: '@zegadb/server-darwin-arm64', bin: 'zega-server' })
        expect(serverPackage('darwin', 'x64')).toEqual({ name: '@zegadb/server-darwin-x64', bin: 'zega-server' })
        expect(serverPackage('linux', 'x64')).toEqual({ name: '@zegadb/server-linux-x64', bin: 'zega-server' })
        expect(serverPackage('win32', 'x64')).toEqual({ name: '@zegadb/server-win32-x64', bin: 'zega-server.exe' })
        expect(serverPackage('linux', 'arm64')).toBeNull()
        expect(serverPackage('freebsd', 'x64')).toBeNull()
    })

    test('the package for THIS platform is the one chosen, whatever else is installed', () => {
        const root = path.join(home, 'resolve')
        const bins = install(root, ['darwin-arm64', 'darwin-x64', 'linux-x64', 'win32-x64'])
        for (const [platform, arch, id] of [
            ['darwin', 'arm64', 'darwin-arm64'],
            ['darwin', 'x64', 'darwin-x64'],
            ['linux', 'x64', 'linux-x64'],
            ['win32', 'x64', 'win32-x64'],
        ] as const) {
            expect(resolveServer({ platform, arch, roots: [root] })).toBe(bins[id]!)
        }
    })

    test('found from a nested directory by walking up, from the first root that has it', () => {
        const outer = path.join(home, 'walk')
        const bins = install(outer, ['linux-x64'])
        const deep = path.join(outer, 'node_modules', 'zegadb', 'src', 'handlers')
        fs.mkdirSync(deep, { recursive: true })
        const pkg = serverPackage('linux', 'x64')!
        expect(findInstalled(pkg, [deep])).toBe(bins['linux-x64']!)
        const other = path.join(home, 'walk-other')
        const otherBins = install(other, ['linux-x64'])
        expect(findInstalled(pkg, [other, outer])).toBe(otherBins['linux-x64']!)
        expect(findInstalled(pkg, [path.join(home, 'nothing-here')])).toBeNull()
    })

    test('nothing on PATH is ever used; a missing package names the package, the platform and what to do', () => {
        const bin = path.join(home, 'pathbin')
        fakeServer(bin)
        const saved = process.env.PATH
        process.env.PATH = `${bin}:${saved}`
        try {
            expect(() => resolveServer({ platform: 'linux', arch: 'x64', roots: [path.join(home, 'empty')] })).toThrow(
                /the zega-server for linux-x64 is not installed: it is the npm package @zegadb\/server-linux-x64.*no --omit=optional.*--server <path>/,
            )
        } finally {
            process.env.PATH = saved
        }
        expect(() => resolveServer({ platform: 'linux', arch: 'arm64', roots: [] })).toThrow(
            'there is no zega-server for linux-arm64: zegadb/zega publishes one for darwin-arm64, darwin-x64, linux-x64, win32-x64',
        )
    })

    test('the search starts at the CLI code and at the running executable, and skips a compiled executable\'s virtual files', () => {
        const roots = searchRoots()
        expect(roots.at(-1)).toBe(path.dirname(process.execPath))
        expect(roots.every((root) => !root.includes('$bunfs'))).toBe(true)
    })

    test('--server is ours and nothing else is', () => {
        expect(takeServerFlag(['--data', './d', '--server', '/x/zs', '--port', '0'])).toEqual({ override: '/x/zs', rest: ['--data', './d', '--port', '0'] })
        expect(takeServerFlag(['--server=/y', '--token-file', 't'])).toEqual({ override: '/y', rest: ['--token-file', 't'] })
        expect(takeServerFlag(['--', '--server', 'z'])).toEqual({ override: undefined, rest: ['--', '--server', 'z'] })
        expect(() => takeServerFlag(['--server'])).toThrow('--server needs a path')
        expect(() => resolveServer({ override: path.join(home, 'no-such-server') })).toThrow('no such file')
    })
})

describe.skipIf(!posix)('zega start', () => {
    const server = fakeServer(path.join(home, 'fake'))

    test('runs zega-server start with the arguments untouched, on the same terminal', async () => {
        // `--token-file` is a global option of zega too: the server's own must reach the server.
        const result = await run(['start', '--data', './d', '--port', '0', '--token-file', './tok', '--host', '0.0.0.0', '--server', server], { home })
        expect(result.code).toBe(0)
        expect(result.stdout.split('\n')).toEqual([
            'fake-server 9',
            'arg:start',
            'arg:--data',
            'arg:./d',
            'arg:--port',
            'arg:0',
            'arg:--token-file',
            'arg:./tok',
            'arg:--host',
            'arg:0.0.0.0',
            '',
        ])
    })

    test('--help is the server\'s, not zega\'s', async () => {
        const result = await run(['start', '--server', server, '--help'], { home })
        expect(result.stdout).toContain('arg:start\narg:--help')
    })

    test('the exit code and stderr are the server\'s', async () => {
        const result = await run(['start', '--server', server, '--exit=3'], { home })
        expect(result.code).toBe(3)
        expect(result.stderr).toContain('complaint on stderr')
        expect(result.stderr).not.toContain('zega:')
    })

    test('a server that dies of a signal exits 128 plus the signal', async () => {
        const result = await run(['start', '--server', server, '--die-by-term'], { home })
        expect(result.code).toBe(143)
    })

    test('a SIGTERM to zega reaches the server, and its exit code is zega\'s', async () => {
        const child = Bun.spawn([process.execPath, ENTRY, 'start', '--server', server, '--wait'], {
            cwd: home,
            env: { PATH: process.env.PATH ?? '', HOME: home },
            stdin: 'ignore',
            stdout: 'pipe',
            stderr: 'pipe',
        })
        const reader = child.stdout.getReader()
        let out = ''
        const decoder = new TextDecoder()
        while (!out.includes('ready')) {
            const { value, done } = await reader.read()
            if (done) break
            out += decoder.decode(value)
        }
        expect(out).toContain('ready')
        child.kill('SIGTERM')
        let rest = ''
        for (;;) {
            const { value, done } = await reader.read()
            if (done) break
            rest += decoder.decode(value)
        }
        expect(await child.exited).toBe(7)
        expect(rest).toContain('got-term')
    })

    // (Where the real package is installed beside this checkout, there is nothing to fail.)
    const real = serverPackage(process.platform, process.arch)
    const installed = real !== null && findInstalled(real, searchRoots()) !== null
    test.skipIf(installed)('with no bundled server installed it fails nonzero and says which package; no PATH lookup', async () => {
        const result = await run(['start', '--port', '0'], { home })
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('is not installed: it is the npm package @zegadb/server-')
        expect(result.stdout).toBe('')
    })
})
