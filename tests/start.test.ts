import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import { FAKE_SERVER, recorded, startCli } from './cli_helpers'
import { remove, run, scratch } from './helpers'
import { runningBanner, runningJson } from '../src/core/start/start'
import { LOGO, logo } from '../src/utils/banner'
import { findInstalled, flagValue, resolveServer, searchRoots, serverPackage, takeStartFlags } from '../src/handlers/server_binary'

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
  --wait) trap 'echo got-term; exit 7' TERM; echo http://127.0.0.1:9342; echo ready; while :; do sleep 0.05; done ;;
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

    test('--server, --explorer, --explorer-port and --json are ours and nothing else is', () => {
        const none = { server: undefined, explorer: false, explorerPort: undefined, json: false }
        expect(takeStartFlags(['--data', './d', '--server', '/x/zs', '--port', '0'])).toEqual({ ...none, server: '/x/zs', rest: ['--data', './d', '--port', '0'] })
        expect(takeStartFlags(['--server=/y', '--token-file', 't'])).toEqual({ ...none, server: '/y', rest: ['--token-file', 't'] })
        expect(takeStartFlags(['--explorer', '--explorer-port', '9500', '--json', '--host', '0.0.0.0'])).toEqual({ ...none, explorer: true, explorerPort: 9500, json: true, rest: ['--host', '0.0.0.0'] })
        expect(takeStartFlags(['--explorer-port=0'])).toEqual({ ...none, explorerPort: 0, rest: [] })
        expect(takeStartFlags(['--', '--server', 'z', '--explorer'])).toEqual({ ...none, rest: ['--', '--server', 'z', '--explorer'] })
        expect(() => takeStartFlags(['--server'])).toThrow('--server needs a value')
        expect(() => takeStartFlags(['--explorer-port', 'x'])).toThrow('--explorer-port is a whole number from 0 to 65535')
        expect(() => takeStartFlags(['--explorer-port=70000'])).toThrow('from 0 to 65535')
        expect(() => resolveServer({ override: path.join(home, 'no-such-server') })).toThrow('no such file')
    })

    test('the server\'s own flags are found in its arguments, the last one winning', () => {
        expect(flagValue(['--data', './a', '--port', '1', '--data=./b'], '--data')).toBe('./b')
        expect(flagValue(['--port', '1'], '--data')).toBeUndefined()
        expect(flagValue(['--', '--data', 'x'], '--data')).toBeUndefined()
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

const strip = (text: string) => text.replace(/\u001b\[[0-9;]*m/g, '')

describe('what zega start says is running', () => {
    test('the mark is two greens in true colour: body #55FF55, shaded edge #00AA00; plain without colour', () => {
        const coloured = logo(true)
        expect(coloured).toContain('\u001b[38;2;85;255;85m██████') // body
        expect(coloured).toContain('\u001b[38;2;0;170;0m▐') // edge
        expect(logo(false)).toBe(LOGO.map((row) => '  ' + row).join('\n'))
    })

    test('the banner: the mark, the tagline, a rule, the server, and the explorer stopped with the command that starts it (no more)', () => {
        const banner = strip(runningBanner({ server: 'http://127.0.0.1:9342', explorer: null }, false))
        expect(banner).toContain('▐██████▐██████▐██████▐██████') // the mark's top row
        expect(banner).toContain('▐██████▐██████▐██████▐██ ▐██') // and its bottom row
        expect(banner).toContain('An embeddable graph database.')
        expect(banner).toContain('─'.repeat(20))
        const lines = banner.split('\n')
        const local = lines.findIndex((line) => line.includes('➜  Local:'))
        expect(lines[local]).toBe('  ➜  Local:     http://127.0.0.1:9342')
        expect(lines[local + 1]).toBe('  ➜  Explorer:  (stopped)')
        expect(lines[local + 2]).toBe('       zega start --explorer')
        expect(banner).not.toContain('both')
    })

    test('with the explorer running both lines have their address and there is no hint', () => {
        const banner = strip(runningBanner({ server: 'http://127.0.0.1:9342', explorer: 'http://127.0.0.1:9343' }, false))
        expect(banner).toContain('  ➜  Local:     http://127.0.0.1:9342')
        expect(banner).toContain('  ➜  Explorer:  http://127.0.0.1:9343')
        expect(banner).not.toContain('(stopped)')
        expect(banner).not.toContain('zega start --explorer')
    })

    test('the colour is the terminal\'s to decide: none when asked for none, the hint dim when it has colour', () => {
        expect(runningBanner({ server: 'http://x', explorer: null }, false)).not.toContain('\u001b[')
        expect(runningBanner({ server: 'http://x', explorer: null }, true)).toContain('\u001b[2mzega start --explorer')
    })

    test('the JSON line has each thing\'s address and status', () => {
        expect(JSON.parse(runningJson({ server: 'http://127.0.0.1:9342', explorer: null }))).toEqual({
            server: { url: 'http://127.0.0.1:9342', status: 'running' },
            explorer: { url: null, status: 'stopped' },
        })
        expect(JSON.parse(runningJson({ server: 'http://a', explorer: 'http://b' })).explorer).toEqual({ url: 'http://b', status: 'running' })
    })
})

describe.skipIf(!posix)('zega start, running', () => {
    const dir = path.join(home, 'running')
    fs.mkdirSync(dir, { recursive: true })
    const base = (data: string, extra: string[] = []) => ['start', '--server', FAKE_SERVER, '--data', path.join(dir, data), '--port', '0', ...extra]
    const started: ReturnType<typeof startCli>[] = []
    afterAll(async () => {
        for (const cli of started) {
            cli.signal('SIGTERM')
            await cli.exited
        }
    })
    const go = (args: string[]) => {
        const cli = startCli(args, { cwd: dir, home })
        started.push(cli)
        return cli
    }

    test('with no terminal it prints one JSON line (not the banner) and the explorer is stopped', async () => {
        const cli = go(base('plain'))
        await cli.until('\n')
        const line = JSON.parse(cli.stdout().split('\n')[0]!)
        expect(line.server.status).toBe('running')
        expect(line.server.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        expect(line.explorer).toEqual({ url: null, status: 'stopped' })
        expect(cli.stdout()).not.toContain('➜')
        // The server is the real thing behind that address, started with the arguments it was given.
        expect((await (await fetch(`${line.server.url}/health`)).json())).toEqual({ ok: true })
        expect(recorded(path.join(dir, 'plain')).argv).toEqual(['start', '--data', path.join(dir, 'plain'), '--port', '0'])
        cli.signal('SIGTERM')
        expect(await cli.exited).toBe(0)
    })

    test('on a terminal it prints the banner; --json on a terminal prints the JSON line instead', async () => {
        const onTerminal = async (args: string[]) => {
            let output = ''
            const child = Bun.spawn([process.execPath, ENTRY, ...args], {
                cwd: dir,
                env: { PATH: process.env.PATH ?? '', HOME: home, TERM: 'xterm' },
                terminal: { cols: 100, rows: 30, data: (_t, data) => void (output += new TextDecoder().decode(data)) },
            })
            const deadline = Date.now() + 20_000
            while (!/zega start --explorer|"status":"stopped"/.test(output) && Date.now() < deadline) await Bun.sleep(25)
            child.kill('SIGTERM')
            await child.exited
            return strip(output)
        }
        const banner = await onTerminal(base('tty'))
        expect(banner).toMatch(/➜  Local:     http:\/\/127\.0\.0\.1:\d+\r?\n/)
        expect(banner).toMatch(/➜  Explorer:  \(stopped\)\r?\n {7}zega start --explorer\r?\n/)
        expect(banner).not.toContain('"status"')
        const json = await onTerminal(base('tty-json', ['--json']))
        expect(json).toContain('"explorer":{"url":null,"status":"stopped"}')
        expect(json).not.toContain('➜')
    })

    test('--explorer serves the explorer on the same database: both addresses are printed, and the page\'s requests reach the server', async () => {
        const cli = go(base('both', ['--explorer', '--explorer-port', '0']))
        await cli.until('\n')
        const line = JSON.parse(cli.stdout().split('\n')[0]!)
        expect(line.explorer.status).toBe('running')
        expect(line.explorer.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/)
        expect(line.explorer.url).not.toBe(line.server.url)
        const page = await (await fetch(line.explorer.url)).text()
        expect(page).toContain('<title>zega</title>')
        const config = (await (await fetch(`${line.explorer.url}/explorer-config.json`)).json()) as { local: { label: string; detail: string } }
        expect(config.local).toEqual({ label: 'local server · both', detail: path.join(dir, 'both') })
        // One server, not two: the explorer's database requests land on the server this command started.
        expect(await (await fetch(`${line.explorer.url}/graph`)).json()).toEqual({ ok: true, result: { nodes: [], rels: [] } })
        expect(recorded(path.join(dir, 'both')).requests.map((r) => `${r.method} ${r.target}`)).toEqual(['GET /graph'])
        // --explorer and --explorer-port are not the server's: it never saw them.
        expect(recorded(path.join(dir, 'both')).argv).toEqual(['start', '--data', path.join(dir, 'both'), '--port', '0'])
        cli.signal('SIGTERM')
        expect(await cli.exited).toBe(0)
        expect(recorded(path.join(dir, 'both')).stopped).toBe(true)
        await expect(fetch(line.explorer.url)).rejects.toThrow()
    })

    test('a server that needs a token still works through the explorer: the page never has the token, the explorer adds it', async () => {
        const tokenFile = path.join(dir, 'server-token')
        fs.writeFileSync(tokenFile, 'server-secret-token\n', { mode: 0o600 })
        const cli = go(base('guarded', ['--token-file', tokenFile, '--explorer', '--explorer-port', '0']))
        await cli.until('\n')
        const line = JSON.parse(cli.stdout().split('\n')[0]!)
        expect((await fetch(`${line.server.url}/graph`)).status).toBe(401)
        expect(await (await fetch(`${line.explorer.url}/graph`)).json()).toEqual({ ok: true, result: { nodes: [], rels: [] } })
        expect(cli.stdout() + cli.stderr()).not.toContain('server-secret-token')
        // The page sent no token; the server saw one.
        const seen = recorded(path.join(dir, 'guarded')).requests
        expect(seen.at(-1)).toMatchObject({ target: '/graph', authorization: 'Bearer server-secret-token' })
    })

    test('an explorer port that is taken is named, the server is stopped again, and the exit is 1', async () => {
        const taken = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('') })
        try {
            const cli = go(base('busy', ['--explorer', '--explorer-port', String(taken.port)]))
            expect(await cli.exited).toBe(1)
            expect(cli.stderr()).toContain(`the explorer's port ${taken.port} is already in use`)
            expect(recorded(path.join(dir, 'busy')).stopped).toBe(true)
        } finally {
            taken.stop(true)
        }
    })

    test('a server that refuses to start passes its words and its exit code through; nothing is announced', async () => {
        const cli = go(base('locked', ['--explorer']))
        expect(await cli.exited).toBe(1)
        expect(cli.stderr()).toContain('data directory is locked')
        expect(cli.stdout()).toBe('')
    })
})

describe('the mark in the help', () => {
    test('piped help has no mark (scripts and agents read plain text); `zega` alone prints the help, exit 0', async () => {
        const home = scratch()
        const help = await run(['--help'], { home })
        expect(help.code).toBe(0)
        expect(help.stdout).not.toContain('▐██')
        expect(help.stdout).toContain('Usage: zega')
        const bare = await run([], { home })
        remove(home)
        expect(bare.code).toBe(0)
        expect(bare.stdout).toContain('Usage: zega')
    })
})
