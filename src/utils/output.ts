import type { Reply } from '@/utils/cloud_request'

/** Write and wait until it is out, so a process that exits right after loses nothing on a pipe. */
export function write(stream: 'stdout' | 'stderr', text: string): Promise<void> {
    return new Promise((resolve) => {
        process[stream].write(text, () => resolve())
    })
}

/** Print an API answer: its own text with `--json`, else the human rendering of it. */
export async function show<T>(globals: { json?: boolean }, reply: Reply<T>, human: (body: T) => string): Promise<void> {
    if (globals.json === true) {
        await write('stdout', reply.raw.endsWith('\n') ? reply.raw : reply.raw + '\n')
        return
    }
    await write('stdout', human(reply.json))
}
