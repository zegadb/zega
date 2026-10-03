import fs from 'node:fs'
import path from 'node:path'

import pkg from '../../../package.json'
import type { Globals } from '@/handlers'
import { readAddress } from '@/handlers/explorer_local'
import { DEFAULT_PORT, serveDatabaseExplorer, type Serving } from '@/handlers/explorer_run'
import { flagValue, resolveServer, takeStartFlags } from '@/handlers/server_binary'
import { LocalFailure } from '@/utils/errors'
import { startBanner } from '@/utils/banner'
import { write } from '@/utils'

/** The signals worth forwarding: a supervisor's stop, and a hang-up. */
const FORWARDED = ['SIGTERM', 'SIGHUP'] as const
/** The numbers shells report for a child that died of one (128 plus the signal). */
const SIGNAL_NUMBER: Record<string, number> = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGTERM: 15 }

/** The tagline under the mark (the one zegadb/zega's README opens with). */
const TAGLINE = 'An embeddable graph database.'

/** What the start-up says is running: the server, and the explorer or not. */
export type Running = { server: string; explorer: string | null }

/** One JSON line for `--json` and for no terminal: each thing's address and whether it is running. */
export function runningJson(running: Running): string {
    return (
        JSON.stringify({
            server: { url: running.server, status: 'running' },
            explorer: running.explorer === null ? { url: null, status: 'stopped' } : { url: running.explorer, status: 'running' },
        }) + '\n'
    )
}

export function runningBanner(running: Running, color?: boolean): string {
    return startBanner({
        name: 'zega',
        version: pkg.version,
        tagline: TAGLINE,
        lines: [
            { label: 'Local', url: running.server },
            { label: 'Explorer', url: running.explorer, hint: 'zega start --explorer' },
        ],
        color,
    })
}

/**
 * `zega start [args]` is `zega-server start [args]`, the server that ships with this CLI: the arguments go through
 * untouched, stderr is the server's, and so is the exit code. It says what is running, in a banner on a terminal
 * and as one JSON line anywhere else. With `--explorer` it also serves the explorer page, on the database this
 * server holds. Returns the code to exit with.
 */
export async function start(rawArgs: string[], globals: Globals): Promise<number> {
    const flags = takeStartFlags(rawArgs)
    const binary = resolveServer({ override: flags.server })
    const child = Bun.spawn([binary, 'start', ...flags.rest], { stdin: 'inherit', stdout: 'pipe', stderr: 'inherit' })

    const signals: NodeJS.Signals[] = [...FORWARDED]
    // At a terminal, Ctrl-C reaches the server directly (same process group); anywhere else somebody
    // signalled this process only, and the server has to be told.
    if (!process.stdin.isTTY) signals.push('SIGINT')
    const listeners = signals.map((signal) => [signal, () => child.kill(signal)] as const)
    for (const [signal, listener] of listeners) process.on(signal, listener)
    let explorer: Serving | null = null
    try {
        const reader = child.stdout.getReader()
        const found = await readAddress(reader, 30_000)
        if (found === 'timeout') {
            child.kill()
            throw new LocalFailure('the database server did not print its address within 30 seconds')
        }
        if (typeof found === 'object' && 'ended' in found) {
            // It ended before it was ready (a --help, a refusal): what it printed is passed on and its exit code is ours.
            await write('stdout', found.ended)
            return await child.exited
        }
        await write('stdout', found.before)

        if (flags.explorer) {
            const data = path.resolve(flagValue(flags.rest, '--data') ?? './zega-data')
            const tokenFile = flagValue(flags.rest, '--token-file')
            explorer = await serveDatabaseExplorer({
                port: flags.explorerPort ?? DEFAULT_PORT,
                upstream: found.url,
                token: tokenFile === undefined ? undefined : fs.readFileSync(tokenFile, 'utf8').trim(),
                data,
                globals,
            }).catch(async (error) => {
                child.kill('SIGTERM')
                await child.exited
                throw error
            })
        }

        const running: Running = { server: found.url, explorer: explorer?.url ?? null }
        const json = flags.json || globals.json === true || !process.stdout.isTTY
        await write('stdout', json ? runningJson(running) : runningBanner(running))
        if (found.after !== '') await write('stdout', found.after)
        // What else it prints goes through as it comes.
        void (async () => {
            for (;;) {
                const next = await reader.read()
                if (next.done) return
                await write('stdout', new TextDecoder().decode(next.value))
            }
        })()

        const code = await child.exited
        const signal = child.signalCode
        return signal === null ? code : 128 + (SIGNAL_NUMBER[signal] ?? 0)
    } finally {
        for (const [signal, listener] of listeners) process.off(signal, listener)
        await explorer?.stop()
        if (child.exitCode === null) {
            child.kill('SIGTERM')
            await child.exited
        }
    }
}
