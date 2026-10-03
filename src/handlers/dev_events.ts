/**
 * What `zega dev` reports while it runs. Each is one event: a JSON line on stdout by default, and with
 * `--human` the line it always was, on stderr. `humanLine` is the only place the text is made.
 */
export type DevEvent =
    | { event: 'warning'; function: string; message: string }
    | { event: 'reloaded'; function: string; entry: string }
    /** A rebuild failed; the previous build is still what is served. */
    | { event: 'build_failed'; function: string; message: string }
    | { event: 'request'; function: string; method: string; path: string; status: number; ms: number }
    /** The function threw while it answered a request (the answer was a 500). */
    | { event: 'threw'; function: string; message: string }
    | { event: 'wait_until_rejected'; function: string; message: string }

export type Emit = (event: DevEvent) => void

export function humanLine(event: DevEvent): string {
    switch (event.event) {
        case 'warning':
            return `warning: ${event.message}`
        case 'reloaded':
            return `reloaded ${event.function} (${event.entry})`
        case 'build_failed':
            return `${event.function}: ${event.message}\n  still serving the previous build`
        case 'request':
            return `${event.function} ${event.method} ${event.path} ${event.status} ${event.ms}ms`
        case 'threw':
            return `${event.function} threw: ${event.message}`
        case 'wait_until_rejected':
            return `${event.function} waitUntil rejected: ${event.message}`
    }
}
