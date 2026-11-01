import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

/**
 * Read a server's standard output until it has printed its address (`http://host:port` on a line of its own):
 * the lines before it, and the address. `{ ended }` (with all it printed) when the output ends first, 'timeout' after `waitMs`.
 * Text after the address on the same read is returned too, so a caller can pass it on.
 */
export async function readAddress(
    reader: { read(): Promise<{ done: boolean; value?: Uint8Array }> },
    waitMs: number,
): Promise<{ url: string; before: string; after: string } | { ended: string } | 'timeout'> {
    const decoder = new TextDecoder()
    const deadline = Date.now() + waitMs
    let seen = ''
    for (;;) {
        const next = await Promise.race([reader.read(), Bun.sleep(Math.max(deadline - Date.now(), 0)).then(() => 'timeout' as const)])
        if (next === 'timeout') return 'timeout'
        if (next.done) return { ended: seen }
        seen += decoder.decode(next.value)
        const lines = seen.split('\n')
        const at = lines.findIndex((line, index) => index < lines.length - 1 && /^https?:\/\/\S+$/.test(line.trim()))
        if (at !== -1) {
            return { url: lines[at]!.trim(), before: lines.slice(0, at).map((line) => line + '\n').join(''), after: lines.slice(at + 1).join('\n') }
        }
    }
}

export type LocalServer = {
    /** `http://127.0.0.1:<port>`, as the server printed it. */
    url: string
    /** Resolves with its exit code when it ends, however it ends. */
    exited: Promise<number>
    /** Ask it to stop the way its own Ctrl-C does (SIGINT: it shuts the database down cleanly) and wait; after `graceMs`, kill it. */
    stop(graceMs?: number): Promise<void>
}

/**
 * Run `zega-server start --data <dir>` on a port of its own (127.0.0.1, no token: the explorer is one
 * person's local database) and wait for the address it prints. This process started it, owns it,
 * and is the only thing that ever signals it. The query time limit is off, as in `zega-server
 * explorer`: an import of a big file is one statement.
 */
export async function startLocal(binary: string, data: string): Promise<LocalServer> {
    const child = Bun.spawn([binary, 'start', '--data', path.resolve(data), '--port', '0', '--query-time-limit', '0'], {
        stdin: 'ignore',
        stdout: 'pipe',
        stderr: 'pipe',
    })
    let stderr = ''
    const decoder = new TextDecoder()
    void (async () => {
        for await (const chunk of child.stderr) {
            const part = decoder.decode(chunk)
            stderr += part
            process.stderr.write(part)
        }
    })()

    const reader = child.stdout.getReader()
    const found = await readAddress(reader, 30_000)
    if (found === 'timeout') {
        child.kill()
        throw new LocalFailure('the database server did not print its address within 30 seconds')
    }
    if (typeof found === 'object' && 'ended' in found) {
        const code = await child.exited
        throw new LocalFailure(`the database server (zega-server start) exited with ${code} before it was ready${stderr.trim() === '' ? '' : `:\n${stderr.trim()}`}`)
    }
    const url = found.url
    // Keep reading what it prints so its pipe never fills.
    void (async () => {
        for (;;) if ((await reader.read()).done) return
    })()

    return {
        url,
        exited: child.exited,
        async stop(graceMs = 5000) {
            if (child.exitCode !== null) return
            child.kill('SIGINT')
            const ended = await Promise.race([child.exited.then(() => true), Bun.sleep(graceMs).then(() => false)])
            if (!ended) {
                child.kill('SIGKILL')
                await child.exited
            }
        },
    }
}
