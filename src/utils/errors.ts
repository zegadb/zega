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

/** What a failed command prints on stderr: the API's sentence and code first, then what to do about it. */
export function describe(error: ZegaError, context: ErrorContext): string {
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
