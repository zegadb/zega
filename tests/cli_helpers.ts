import fs from 'node:fs'
import path from 'node:path'

import { scratch } from './helpers'

const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')
export const FAKE_SERVER = path.resolve(import.meta.dir, 'fixtures', 'fake-zega-server.ts')
export const FIXTURE_ASSETS = path.resolve(import.meta.dir, 'fixtures', 'explorer-assets')

export type Cli = {
    stdout: () => string
    stderr: () => string
    /** Resolves when the text has been printed on the stream; rejects, with everything printed, if the process ends or 20 s pass first. */
    until: (text: string, where?: 'stdout' | 'stderr') => Promise<void>
    signal: (signal: NodeJS.Signals) => void
    exited: Promise<number>
    pid: number
}

/**
 * Start the CLI as a subprocess and keep what it prints. The environment is a PATH, a HOME and a
 * config directory of its own and whatever `env` adds, so nothing of the machine's own
 * `zega login` can reach a test.
 */
export function startCli(args: string[], options: { cwd: string; home: string; env?: Record<string, string>; command?: string[] }): Cli {
    const command = options.command ?? [process.execPath, ENTRY]
    const child = Bun.spawn([...command, ...args], {
        cwd: options.cwd,
        env: { PATH: process.env.PATH ?? '', HOME: options.home, XDG_CONFIG_HOME: path.join(options.home, '.config'), ...options.env },
        stdin: 'ignore',
        stdout: 'pipe',
        stderr: 'pipe',
    })
    let out = ''
    let err = ''
    const pump = async (stream: ReadableStream<Uint8Array>, add: (text: string) => void) => {
        const decoder = new TextDecoder()
        for await (const chunk of stream) add(decoder.decode(chunk))
    }
    void pump(child.stdout, (t) => (out += t))
    void pump(child.stderr, (t) => (err += t))
    return {
        stdout: () => out,
        stderr: () => err,
        pid: child.pid,
        exited: child.exited,
        signal: (signal) => child.kill(signal),
        until: async (text, where = 'stdout') => {
            const deadline = Date.now() + 20_000
            while (Date.now() < deadline) {
                if ((where === 'stdout' ? out : err).includes(text)) return
                if (child.exitCode !== null) break
                await Bun.sleep(25)
            }
            throw new Error(`never printed ${JSON.stringify(text)} on ${where}.\nstdout: ${out}\nstderr: ${err}`)
        },
    }
}

/** A directory of the test's own to put a database and files in. */
export function workdir(): string {
    const dir = scratch()
    fs.mkdirSync(path.join(dir, 'home'))
    return dir
}

/** What the fake zega-server recorded for a data directory. */
export function recorded(data: string): { argv: string[]; requests: Record<string, string | null>[]; stopped: boolean } {
    const read = (name: string) => path.join(data, name)
    return {
        argv: JSON.parse(fs.readFileSync(read('argv.json'), 'utf8')),
        requests: fs.existsSync(read('requests.jsonl'))
            ? fs
                  .readFileSync(read('requests.jsonl'), 'utf8')
                  .trim()
                  .split('\n')
                  .filter((line) => line !== '')
                  .map((line) => JSON.parse(line))
            : [],
        stopped: fs.existsSync(read('stopped')),
    }
}
