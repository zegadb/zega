import { resolveServer, takeServerFlag } from '@/handlers/server_binary'

/** The signals worth forwarding: a supervisor's stop, and a hang-up. */
const FORWARDED = ['SIGTERM', 'SIGHUP'] as const
/** The numbers shells report for a child that died of one (128 plus the signal). */
const SIGNAL_NUMBER: Record<string, number> = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGTERM: 15 }

/**
 * `zega start [args]` is `zega-server start [args]`, the server that ships with this CLI: the
 * arguments go through untouched, the terminal is the server's (it prints its own address), and
 * the exit code and signals are the server's too. Returns the code to exit with.
 */
export async function start(rawArgs: string[]): Promise<number> {
    const { override, rest } = takeServerFlag(rawArgs)
    const binary = resolveServer({ override })
    const child = Bun.spawn([binary, 'start', ...rest], { stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' })

    const forward = (signal: NodeJS.Signals) => () => child.kill(signal)
    const signals: NodeJS.Signals[] = [...FORWARDED]
    // At a terminal, Ctrl-C reaches the server directly (same process group); anywhere else somebody
    // signalled this process only, and the server has to be told.
    if (!process.stdin.isTTY) signals.push('SIGINT')
    const listeners = signals.map((signal) => [signal, forward(signal)] as const)
    for (const [signal, listener] of listeners) process.on(signal, listener)
    try {
        const code = await child.exited
        const signal = child.signalCode
        return signal === null ? code : 128 + (SIGNAL_NUMBER[signal] ?? 0)
    } finally {
        for (const [signal, listener] of listeners) process.off(signal, listener)
    }
}
