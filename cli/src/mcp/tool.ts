/**
 * What a tool is, and what every tool's answer goes through.
 *
 * A tool is a name, a group, whether it changes anything, an input shape and a function. The server registers it only
 * when its group is on and (for one that changes something) the server is not `--read-only` and the token is `manage`.
 * Whatever the function returns or throws is answered here: as the JSON the CLI itself prints, with every secret the
 * server knows of replaced, and cut off when it is too long to be useful in a conversation.
 */
import type { ZodRawShape } from 'zod'

import type { Globals, Session } from '@/handlers'
import type { Group } from '@/mcp/groups'
import type { KeyRing } from '@/mcp/keys'
import type { LocalServers } from '@/mcp/local_servers'
import { capture } from '@/utils'
import { isZegaError, report } from '@/utils/errors'

export type Context = {
    /** What every command takes: `--api` and `--token-file` as the server was started with. */
    globals: Globals
    /** The token's scope; null when no cloud group is on and so no token was read. */
    scope: 'read' | 'manage' | null
    session: () => Promise<Session>
    keys: KeyRing
    local: LocalServers
    /** The project: the directory the server was started in. Paths a tool is given are inside it. */
    cwd: string
    /** `--server <path>`: another zega-server build, for the local group. */
    serverPath?: string
}

export type Tool = {
    name: string
    group: Group
    /** Whether it changes anything (a cloud resource, a graph, a file or a process). Such a tool is never offered under `--read-only`. */
    writes: boolean
    /** Whether it needs the token's `manage` scope (it changes something in Zega Cloud). */
    manage: boolean
    title: string
    description: string
    input: ZodRawShape
    run: (args: Record<string, unknown>, context: Context) => Promise<string>
}

/** The most characters one answer holds: more is more than a conversation can use. */
export const MAX_ANSWER_CHARS = 100_000

/** The commands' own options: JSON, compact. */
export function commandGlobals(globals: Globals): Globals {
    return { ...globals, human: false, nopretty: true }
}

/** What a CLI command prints on stdout when run for a tool (its stderr is for people, and not part of the answer). */
export async function printed(run: () => Promise<void>): Promise<string> {
    const { stdout } = await capture(run)
    return stdout.trim()
}

/** A value as the text a tool answers. */
export const json = (value: unknown): string => JSON.stringify(value)

export type Answer = { text: string; error: boolean }

/** Replace the secrets: each exact string given, and anything shaped like an API token (`zc_`). */
export function redact(text: string, secrets: Iterable<string>): string {
    let out = text
    for (const secret of secrets) if (secret.length >= 8) out = out.split(secret).join('[redacted]')
    return out.replace(/\bzc_[A-Za-z0-9_]{12,}/g, '[redacted]')
}

export async function answer(tool: Tool, args: Record<string, unknown>, context: Context): Promise<Answer> {
    let text: string
    let error = false
    try {
        text = await tool.run(args, context)
    } catch (caught) {
        error = true
        if (isZegaError(caught)) {
            const { code, message, hint } = report(caught, { api: context.globals.api ?? null, tokenFile: context.globals.tokenFile ?? null })
            text = json({ error: { code, message, hint } })
        } else {
            text = json({ error: { code: 'error', message: caught instanceof Error ? caught.message : String(caught), hint: null } })
        }
    }
    text = redact(text, context.keys.secrets())
    if (text.length > MAX_ANSWER_CHARS) {
        text = `${text.slice(0, MAX_ANSWER_CHARS)}\n[cut at ${MAX_ANSWER_CHARS} characters of ${text.length}: ask for less (a limit, a narrower query)]`
    }
    return { text, error }
}
