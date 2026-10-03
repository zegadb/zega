import type { Reply } from '@/utils/cloud_request'
import { eventLine, formatJson } from '@/utils/pretty'
import type { Mode } from '@/utils/tty'

/** Write and wait until it is out, so a process that exits right after loses nothing on a pipe. */
export function write(stream: 'stdout' | 'stderr', text: string): Promise<void> {
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
