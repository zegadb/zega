import type { Reply } from '@/utils/cloud_request'
import type { Mode } from '@/utils/tty'

/** Write and wait until it is out, so a process that exits right after loses nothing on a pipe. */
export function write(stream: 'stdout' | 'stderr', text: string): Promise<void> {
    return new Promise((resolve) => {
        process[stream].write(text, () => resolve())
    })
}

/** One JSON value on a line of its own: a result, or an event of a command that keeps running. */
export function json(value: unknown): string {
    return JSON.stringify(value) + '\n'
}

/** Print an API answer: its own text, unchanged, or with `--human` the readable rendering of it. */
export async function show<T>(globals: Mode, reply: Reply<T>, human: (body: T) => string): Promise<void> {
    if (globals.human === true) {
        await write('stdout', human(reply.json))
        return
    }
    await write('stdout', reply.raw.endsWith('\n') ? reply.raw : reply.raw + '\n')
}
