import fs from 'node:fs'
import path from 'node:path'

import { FIXTURES, scratch } from './helpers'

const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')

export const FN = 'f0bbbbbbbbbbbbbbbb'
export const SECOND = 'f0cccccccccccccccc'

/** A port nothing listens on right now. */
export function freePort(): number {
    const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response('') })
    const port = server.port as number
    server.stop(true)
    return port
}

/** Write zega.json listing these functions. */
export function project(dir: string, functions: { id: string; entry: string }[]): void {
    fs.writeFileSync(
        path.join(dir, 'zega.json'),
        JSON.stringify({ version: 1, name: 'demo', region: 'yyz', functions }, null, 2) + '\n',
    )
}

/** A project directory: the `hello` fixture (a real dependency, a local import) listing it as `FN`. */
export function helloProject(): string {
    const dir = scratch()
    fs.cpSync(path.join(FIXTURES, 'hello'), dir, { recursive: true })
    project(dir, [{ id: FN, entry: 'src/index.ts' }])
    return dir
}

/** A project of small functions written here: name to source. */
export function writeProject(sources: Record<string, string>, functions: { id: string; entry: string }[]): string {
    const dir = scratch()
    for (const [name, text] of Object.entries(sources)) {
        fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true })
        fs.writeFileSync(path.join(dir, name), text)
    }
    project(dir, functions)
    return dir
}

export type Dev = {
    /** Everything `zega dev` printed on stdout so far, and on stderr. */
    stdout: () => string
    stderr: () => string
    /** Resolves when stderr or stdout contains the text. */
    until: (text: string, where?: 'stdout' | 'stderr') => Promise<void>
    stop: () => Promise<void>
    exited: Promise<number>
}

/**
 * Start `zega dev` as a subprocess in `cwd` and wait until it has printed its address summary (or
 * failed). The environment is a PATH, a HOME of its own and whatever `env` adds: a stray secret
 * in `env` is how a test shows the shell does not reach the function.
 */
export async function startDev(
    args: string[],
    options: { cwd: string; home: string; env?: Record<string, string>; command?: string[] },
): Promise<Dev> {
    const command = options.command ?? [process.execPath, ENTRY]
    const child = Bun.spawn([...command, 'dev', ...args], {
        cwd: options.cwd,
        env: {
            PATH: process.env.PATH ?? '',
            HOME: options.home,
            XDG_CONFIG_HOME: path.join(options.home, '.config'),
            ...options.env,
        },
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
    const until = async (text: string, where: 'stdout' | 'stderr' = 'stdout') => {
        const deadline = Date.now() + 20_000
        while (Date.now() < deadline) {
            if ((where === 'stdout' ? out : err).includes(text)) return
            if (child.exitCode !== null) break
            await Bun.sleep(25)
        }
        throw new Error(`zega dev never printed ${JSON.stringify(text)} on ${where}.\nstdout: ${out}\nstderr: ${err}`)
    }
    const dev: Dev = {
        stdout: () => out,
        stderr: () => err,
        until,
        exited: child.exited,
        stop: async () => {
            if (child.exitCode === null) child.kill('SIGINT')
            await child.exited
        },
    }
    return dev
}

/** Fetch until the body satisfies the check (a rebuild is asynchronous), or fail with the last body. */
export async function eventually(url: string, check: (body: string) => boolean): Promise<string> {
    let last = ''
    const deadline = Date.now() + 15_000
    while (Date.now() < deadline) {
        try {
            last = await (await fetch(url)).text()
            if (check(last)) return last
        } catch {
            // not up yet
        }
        await Bun.sleep(50)
    }
    throw new Error(`${url} never answered as expected; last body: ${last}`)
}
