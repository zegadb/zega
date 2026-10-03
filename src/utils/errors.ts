import { DASHBOARD } from '@/utils/constants'

/** Why a command failed. Every one of these ends the process with exit code 1. */
export type ZegaError = ApiFailure | NetworkFailure | LocalFailure

/** The API answered with an error status. */
export class ApiFailure extends Error {
    readonly kind = 'api'
    constructor(
        readonly status: number,
        readonly code: string,
        message: string,
        readonly retryAfter: number | null,
        readonly effect: string | null,
    ) {
        super(message)
    }
}

/** No answer arrived: DNS, connect, TLS or timeout. */
export class NetworkFailure extends Error {
    readonly kind = 'network'
    constructor(readonly host: string, readonly detail: string) {
        super(`could not reach ${host}: ${detail}`)
    }
}

/** Something on this machine: no stored token, a missing file, a missing flag. */
export class LocalFailure extends Error {
    readonly kind = 'local'
    constructor(
        message: string,
        /** What a script can switch on: `local` unless a more exact one is known. */
        readonly code: string = 'local',
    ) {
        super(message)
    }
}

export function isZegaError(error: unknown): error is ZegaError {
    return error instanceof ApiFailure || error instanceof NetworkFailure || error instanceof LocalFailure
}

/** A string for a terminal: control characters become `?`. A project can be named anything. */
export function clean(text: string): string {
    return Array.from(text, (c) => {
        const code = c.codePointAt(0) ?? 0
        return code < 0x20 || (code >= 0x7f && code <= 0x9f) ? '?' : c
    }).join('')
}

export type ErrorContext = {
    /** `--api` as typed, when given. */
    api: string | null
    /** `--token-file` as typed, when given. */
    tokenFile: string | null
}

/** The code, message and hint of a failure: the one source of both the JSON and the readable text. */
export type Report = { code: string; message: string; hint: string | null }

/** What a failed command says, in the words the API (or this machine) gave and what to do about it. */
export function report(error: ZegaError, context: ErrorContext): Report {
    if (error instanceof LocalFailure) return { code: error.code, message: error.message, hint: null }
    if (error instanceof NetworkFailure) {
        return {
            code: 'network',
            message: `could not reach ${error.host}: ${error.detail}`,
            hint: context.api !== null ? 'Check the address given to --api.' : null,
        }
    }
    const hint = guidance(error, context.tokenFile).join(' ')
    return { code: error.code, message: error.message, hint: hint === '' ? null : hint }
}

/** What a failed command prints on stderr: `{"error":{"code","message","hint"}}` on one line, or with `--human` text. */
export function describe(error: unknown, context: ErrorContext, human: boolean): string {
    if (!isZegaError(error)) {
        const message = error instanceof Error ? error.message : String(error)
        return human ? `zega: ${clean(message)}\n` : JSON.stringify({ error: { code: 'error', message, hint: null } }) + '\n'
    }
    const { code, message, hint } = report(error, context)
    if (!human) return JSON.stringify({ error: { code, message, hint } }) + '\n'
    if (error instanceof LocalFailure) return `zega: ${error.message}\n`
    if (error instanceof NetworkFailure) {
        let text = `zega: could not reach ${error.host}: ${clean(error.detail)}\n`
        if (context.api !== null) text += 'Check the address given to --api.\n'
        return text
    }
    let text = `zega: ${clean(error.message)} (code: ${clean(error.code)}, HTTP ${error.status})\n`
    for (const line of guidance(error, context.tokenFile)) text += `${line}\n`
    return text
}

/** A command-line mistake (an unknown option, a bad value), as commander words it. */
export function usageError(message: string, human: boolean): string {
    const text = message.replace(/^error: /, '').trim()
    return human
        ? `error: ${text}\nhint: zega --help\n`
        : JSON.stringify({ error: { code: 'usage', message: text, hint: 'zega --help' } }) + '\n'
}

export function guidance(error: ApiFailure, tokenFile: string | null): string[] {
    const lines: string[] = []
    const newToken = tokenFile !== null ? `put a new token in ${tokenFile}` : 'run `zega login`'
    if (error.status === 401 && error.code === 'wrong_credential') {
        lines.push(
            `That is a graph or storage key, not an API token. Make an API token at ${DASHBOARD}/tokens, then ${newToken}.`,
        )
    } else if (error.status === 401) {
        lines.push(`Make a token at ${DASHBOARD}/tokens, then ${newToken}.`)
    } else if (error.code === 'scope_required') {
        lines.push(`This needs a manage token. Make one at ${DASHBOARD}/tokens, then ${newToken}.`)
    } else if (error.code === 'session_required') {
        // The API's own sentence says where it is done; add it only when it does not.
        if (!error.message.includes('dashboard')) {
            lines.push(`This is done in the dashboard (${DASHBOARD}), signed in; an API token cannot do it.`)
        }
    } else if (error.code === 'project_restricted') {
        lines.push(
            'This token is restricted to one project and this call is account-wide. Use a token without a project restriction.',
        )
    }
    if (error.status === 429 || error.status === 503) {
        lines.push(
            error.retryAfter === null
                ? 'Wait a little, then run the command again.'
                : `Wait ${error.retryAfter} ${error.retryAfter === 1 ? 'second' : 'seconds'}, then run the command again.`,
        )
    }
    if (error.effect === 'nothing') lines.push('The change did not take effect.')
    if (error.effect === 'partly') {
        lines.push(
            'The change took effect in part: look at what you changed (`zega projects|graphs|buckets|functions <id>`) before you repeat it.',
        )
    }
    if (error.effect === 'unknown') {
        lines.push(
            'It is not known whether the change took effect: look at what you changed (`zega projects|graphs|buckets|functions <id>`); repeating it is safe.',
        )
    }
    return lines
}
