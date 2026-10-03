import fs from 'node:fs'
import path from 'node:path'

import pkg from '../../../package.json'
import type { Globals } from '@/handlers'
import { readAddress } from '@/handlers/explorer_local'
import { DEFAULT_PORT, serveDatabaseExplorer, type Serving } from '@/handlers/explorer_run'
import { flagValue, resolveServer, takeStartFlags } from '@/handlers/server_binary'
import { LocalFailure } from '@/utils/errors'
import { startBanner } from '@/utils/banner'
import { json, showLogo, write } from '@/utils'

/** The signals worth forwarding: a supervisor's stop, and a hang-up. */
const FORWARDED = ['SIGTERM', 'SIGHUP'] as const
/** The numbers shells report for a child that died of one (128 plus the signal). */
const SIGNAL_NUMBER: Record<string, number> = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGTERM: 15 }

/** The tagline under the mark (the one zegadb/zega's README opens with). */
const TAGLINE = 'An embeddable graph database.'

/** What the start-up says is running: the server, and the explorer or not. */
export type Running = { server: string; explorer: string | null }

/** The `ready` event, one JSON line: each thing's address and whether it is running. */
export function runningJson(running: Running): string {
    return json({
        event: 'ready',
        server: { url: running.server, status: 'running' },
        explorer: running.explorer === null ? { url: null, status: 'stopped' } : { url: running.explorer, status: 'running' },
    })
}

export function runningBanner(running: Running, color?: boolean, mark?: boolean): string {
    return startBanner({
        name: 'zega',
        version: pkg.version,
        tagline: TAGLINE,
        lines: [
            { label: 'Local', url: running.server },
            { label: 'Explorer', url: running.explorer, hint: 'zega start --explorer' },
        ],
        color,
        mark,
    })
}

/**
 * What the server prints on stdout, passed on: as it is with `--human`, otherwise one `output` event per line,
 * so that everything on stdout is JSON.
 */
function relay(human: boolean) {
    let pending = ''
    return {
        async push(text: string): Promise<void> {
            if (human) return await write('stdout', text)
            pending += text
            const lines = pending.split('\n')
            pending = lines.pop() ?? ''
            for (const line of lines) await write('stdout', json({ event: 'output', line }))
        },
        async end(): Promise<void> {
            if (pending !== '') await write('stdout', json({ event: 'output', line: pending }))
            pending = ''
        },
    }
}

/**
 * `zega start [args]` is `zega-server start [args]`, the server that ships with this CLI: the arguments go through
 * untouched, stderr is the server's, and so is the exit code. It says what is running as one JSON `ready` line,
 * or with `--human` in a banner. With `--explorer` it also serves the explorer page, on the database this
 * server holds. Returns the code to exit with.
 */
export async function start(rawArgs: string[], globals: Globals): Promise<number> {
    const flags = takeStartFlags(rawArgs)
    const binary = resolveServer({ override: flags.server })
    const human = flags.human || globals.human === true
    const out = relay(human)
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
            await out.push(found.ended)
            await out.end()
            return await child.exited
        }
        await out.push(found.before)

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
        await write('stdout', human ? runningBanner(running, undefined, showLogo({ human })) : runningJson(running))
        if (found.after !== '') await out.push(found.after)
        // What else it prints goes through as it comes.
        void (async () => {
            for (;;) {
                const next = await reader.read()
                if (next.done) return await out.end()
                await out.push(new TextDecoder().decode(next.value))
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
