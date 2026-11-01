import { afterAll, describe, expect, test } from 'bun:test'
import chalk from 'chalk'
import fs from 'node:fs'
import path from 'node:path'

import { eventLine, formatJson, paint } from '../src/utils/pretty'
import { TOKEN, fakeApi, remove, run, scratch, whoamiRoute, whoamiShown } from './helpers'

/**
 * The shape of JSON output: pretty and coloured on a terminal, one compact line piped or with
 * `--nopretty`, and one compact line per event always.
 */
const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')
const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
const api = fakeApi({ ...whoamiRoute })
afterAll(() => {
    api.stop()
    remove(home)
})

/** Run on a pseudo-terminal until it exits; what was printed (stdout and stderr share a pty), `\r\n` as `\n`. */
async function onTerminal(args: string[], env: Record<string, string> = {}, stopAfter?: string): Promise<{ output: string; code: number }> {
    let output = ''
    const child = Bun.spawn([process.execPath, ENTRY, ...args], {
        cwd: home,
        env: { PATH: process.env.PATH ?? '', HOME: home, XDG_CONFIG_HOME: path.join(home, '.config'), ...env },
        terminal: { cols: 100, rows: 30, data: (_t, data) => void (output += new TextDecoder().decode(data)) },
    })
    if (stopAfter !== undefined) {
        // A command that keeps running: stop it once it has said what is wanted.
        const deadline = Date.now() + 15_000
        while (!output.includes(stopAfter) && Date.now() < deadline) await Bun.sleep(25)
        child.kill('SIGTERM')
    }
    const code = await child.exited
    return { output: output.replace(/\r\n/g, '\n'), code }
}

const strip = (text: string) => text.replace(/\u001b\[[0-9;]*m/g, '')
const call = ['--api', api.url, '--token-file', tokenFile]
const COLOUR = { TERM: 'xterm-256color' }

describe('the formatter', () => {
    test('not a terminal: one compact line; `always` is pretty with 2 spaces; an event is one line whatever it holds', () => {
        const value = { a: 1, b: { c: [true, null, 'x'] } }
        // `bun test` is not a terminal, so this is the piped case.
        expect(formatJson(value, { stream: 'stdout' })).toBe('{"a":1,"b":{"c":[true,null,"x"]}}\n')
        expect(formatJson(value, { stream: 'stdout', always: true })).toBe(JSON.stringify(value, null, 2) + '\n')
        expect(eventLine(value)).toBe('{"a":1,"b":{"c":[true,null,"x"]}}\n')
    })

    test('the colour is only decoration: without it the text is the same, and every kind of value has its own', () => {
        const text = JSON.stringify({ name: 'zega', n: 3, ok: true, none: null, list: ['a'] }, null, 2)
        const level = chalk.level
        chalk.level = 1 // this process has no terminal, so chalk would print none
        try {
            expect(strip(paint(text))).toBe(text)
            const coloured = paint(text)
            for (const code of ['\u001b[36m"name"', '\u001b[32m"zega"', '\u001b[33m3', '\u001b[33mtrue', '\u001b[33mnull', '\u001b[2m{']) expect(coloured).toContain(code)
        } finally {
            chalk.level = level
        }
    })
})

describe('result JSON', () => {
    test('piped: one line that parses', async () => {
        const result = await run(['whoami', ...call], { home })
        expect(result.code).toBe(0)
        expect(result.stdout).toBe(JSON.stringify(whoamiShown) + '\n')
        expect(result.stdout.trim().split('\n')).toHaveLength(1)
        expect(result.stdout).not.toContain('\u001b')
    })

    test('piped errors are one line too', async () => {
        const result = await run(['whoami', '--api', api.url], { home: scratch() })
        expect(result.code).toBe(1)
        expect(result.stderr.trim().split('\n')).toHaveLength(1)
        expect(JSON.parse(result.stderr).error.code).toBe('not_logged_in')
    })

    test('on a terminal: indented 2 spaces, and the same value', async () => {
        const { output, code } = await onTerminal(['whoami', ...call])
        expect(code).toBe(0)
        expect(output).toBe(JSON.stringify(whoamiShown, null, 2) + '\n')
        expect(output.split('\n').length).toBeGreaterThan(5)
        expect(output).toContain('\n  "account": {\n    "login": "ada"')
    })

    test('on a terminal with colour: keys, strings and punctuation are coloured, as in the help', async () => {
        const { output } = await onTerminal(['whoami', ...call], COLOUR)
        expect(output).toContain('\u001b[36m"account"') // a key
        expect(output).toContain('\u001b[32m"Ada Lovelace"') // a string
        expect(JSON.parse(strip(output))).toEqual(whoamiShown)
    })

    test('--nopretty on a terminal: one compact line, no colour', async () => {
        const { output, code } = await onTerminal(['whoami', '--nopretty', ...call], COLOUR)
        expect(code).toBe(0)
        expect(output).toBe(JSON.stringify(whoamiShown) + '\n')
        expect(output).not.toContain('\u001b')
    })

    test('errors on a terminal are pretty (stderr is the terminal); --nopretty makes one line', async () => {
        const dir = scratch()
        try {
            const args = ['whoami', '--api', api.url]
            const env = { HOME: dir, XDG_CONFIG_HOME: path.join(dir, '.config') }
            const pretty = await onTerminal(args, env)
            expect(pretty.code).toBe(1)
            expect(pretty.output).toContain('\n  "error": {\n    "code": "not_logged_in"')
            expect(JSON.parse(pretty.output).error.code).toBe('not_logged_in')
            const compact = await onTerminal([...args, '--nopretty'], env)
            expect(compact.output.trim().split('\n')).toHaveLength(1)
            expect(JSON.parse(compact.output).error.code).toBe('not_logged_in')
            // A mistake on the command line is the same.
            const usage = await onTerminal(['whoami', '--bogus', '--nopretty'])
            expect(usage.output.trim().split('\n')).toHaveLength(1)
            expect(JSON.parse(usage.output).error.code).toBe('usage')
        } finally {
            remove(dir)
        }
    })

    test('--human is not affected, and the help stays pretty JSON piped and with --nopretty', async () => {
        const human = await onTerminal(['whoami', '--human', ...call])
        expect(human.output).toStartWith('Logged in as Ada Lovelace (@ada)')
        const help = await run(['--help', '--nopretty'], { home })
        expect(help.stdout.startsWith('{\n  "name": "zega"')).toBe(true)
        expect(JSON.parse(help.stdout).options['--nopretty']).toBe('compact JSON')
    })
})

describe.skipIf(process.platform === 'win32')('streaming events stay one line on a terminal', () => {
    test('zega start: the ready event and the server\'s lines are one compact line each', async () => {
        const fake = path.join(home, 'fake-server')
        fs.writeFileSync(fake, '#!/bin/sh\necho http://127.0.0.1:9342\necho hello\nsleep 30\n', { mode: 0o755 })
        const { output } = await onTerminal(['start', '--server', fake, '--data', path.join(home, 'd')], COLOUR, '"event":"output"')
        const lines = strip(output).trim().split('\n')
        expect(lines.map((line) => JSON.parse(line).event)).toEqual(['ready', 'output'])
        expect(output).not.toContain('\u001b')
        expect(JSON.parse(lines[0]!)).toEqual({
            event: 'ready',
            server: { url: 'http://127.0.0.1:9342', status: 'running' },
            explorer: { url: null, status: 'stopped' },
        })
    })

    test('zega dev: ready, request and reload events are single lines', async () => {
        const dir = scratch()
        try {
            fs.writeFileSync(path.join(dir, 'f.ts'), "export default { fetch: () => new Response('one') }\n")
            fs.writeFileSync(path.join(dir, 'zega.json'), JSON.stringify({ version: 1, name: 'x', region: 'yyz', functions: [{ id: 'f0bbbbbbbbbbbbbbbb', entry: 'f.ts' }] }))
            const port = 18000 + Math.floor(Math.random() * 2000)
            let output = ''
            const child = Bun.spawn([process.execPath, ENTRY, 'dev', '--port', String(port)], {
                cwd: dir,
                env: { PATH: process.env.PATH ?? '', HOME: home, TERM: 'xterm-256color' },
                terminal: { cols: 100, rows: 30, data: (_t, data) => void (output += new TextDecoder().decode(data)) },
            })
            const until = async (text: string) => {
                const deadline = Date.now() + 15_000
                while (!output.includes(text) && Date.now() < deadline) await Bun.sleep(25)
            }
            await until('"event":"ready"')
            expect(await (await fetch(`http://127.0.0.1:${port}/x`)).text()).toBe('one')
            await until('"event":"request"')
            child.kill('SIGINT')
            await child.exited
            const lines = output.replace(/\r\n/g, '\n').trim().split('\n')
            expect(lines.length).toBeGreaterThanOrEqual(2)
            const events = lines.map((line) => JSON.parse(line))
            expect(events[0].event).toBe('ready')
            expect(events.some((e) => e.event === 'request')).toBe(true)
            expect(output).not.toContain('\u001b')
        } finally {
            remove(dir)
        }
    })
})
