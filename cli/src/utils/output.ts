import { AsyncLocalStorage } from 'node:async_hooks'

import type { Reply } from '@/utils/cloud_request'
import { eventLine, formatJson } from '@/utils/pretty'
import type { Mode } from '@/utils/tty'

/** Where a command's output goes instead of the process's streams (see `capture`). */
type Sink = { stdout: string; stderr: string }
const sinks = new AsyncLocalStorage<Sink>()

/**
 * Run a command and collect what it would have printed. `zega mcp` speaks its protocol on stdout, so the commands it
 * reuses for its tools must not print there: this gives each call its own buffers (concurrent calls do not mix), and
 * what a command printed is what the tool answers. Nothing else in the CLI uses it.
 */
export async function capture(run: () => Promise<void>): Promise<Sink> {
    const sink: Sink = { stdout: '', stderr: '' }
    await sinks.run(sink, run)
    return sink
}

/** Write and wait until it is out, so a process that exits right after loses nothing on a pipe. */
export function write(stream: 'stdout' | 'stderr', text: string): Promise<void> {
    const sink = sinks.getStore()
    if (sink !== undefined) {
        sink[stream] += text
        return Promise.resolve()
    }
    return new Promise((resolve) => {
        process[stream].write(text, () => resolve())
    })
}

/** One event of a command that keeps running, as one compact JSON line. */
export const event = eventLine

/** A command's result on stdout: pretty and coloured on a terminal, one compact line piped or with `--nopretty`. */
export function result(globals: Mode, value: unknown): Promise<void> {
    return write('stdout', formatJson(value, { stream: 'stdout', nopretty: globals.nopretty }))
}

/** Print an API answer as JSON, or with `--human` the readable rendering of it. */
export async function show<T>(globals: Mode, reply: Reply<T>, human: (body: T) => string): Promise<void> {
    if (globals.human === true) {
        await write('stdout', human(reply.json))
        return
    }
    await result(globals, reply.json)
}

/**
 * What a child process prints on stdout, passed on: as it is with `--human`, otherwise one `output` event per line,
 * so that everything on stdout is JSON.
 */
export function relay(human: boolean) {
    let pending = ''
    return {
        async push(text: string): Promise<void> {
            if (human) return await write('stdout', text)
            pending += text
            const lines = pending.split('\n')
            pending = lines.pop() ?? ''
            for (const line of lines) await write('stdout', event({ event: 'output', line }))
        },
        async end(): Promise<void> {
            if (pending !== '') await write('stdout', event({ event: 'output', line: pending }))
            pending = ''
        },
    }
}
