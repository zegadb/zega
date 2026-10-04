/**
 * What `zega dev` gives a function as `env`: the same thing Zega Cloud gives it.
 *
 * A deployed function has no bindings (zegadb/cloud: "we will not expose any binds"):
 * its `env` is a flat object of strings, its variables (plain text, set with
 * `PUT /v1/functions/{function}/vars/{name}`) and its secrets (set apart, never
 * readable again). It reaches a graph or a bucket over HTTPS, with the address in
 * a variable and the key in a secret. Locally both come from one dotenv file, the
 * convention Workers tooling uses:
 *
 *     .env               every function in the project
 *     .env.<function id> that function only, over the shared file
 *
 * in the directory of `zega.json`, never from the shell's environment and never from
 * a stored token. The rules a name and a value must keep are the cloud's own
 * (src/functions/api.ts there), so what runs here is something you can set there.
 */
import fs from 'node:fs'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

export const VARS_FILE = '.env'

/** What a variable or secret may be called in code (zegadb/cloud). */
const SETTING_NAME = /^[A-Za-z_][A-Za-z0-9_]{0,63}$/
/** A value's most characters (Cloudflare's 5 KB). */
export const MAX_VALUE_CHARS = 5_120
/** Variables and secrets together, per function. */
export const MAX_SETTINGS = 64

/** Parse dotenv text: `KEY=value`, `export KEY=value`, `#` comments, single or double quotes. No expansion, no multiline values. */
export function parseDotenv(text: string, file: string): Record<string, string> {
    const values: Record<string, string> = {}
    const lines = text.split(/\r?\n/)
    lines.forEach((raw, index) => {
        const at = `${file}:${index + 1}`
        const line = raw.trim()
        if (line === '' || line.startsWith('#')) return
        const match = /^(?:export\s+)?([^=\s]+)\s*=\s*(.*)$/.exec(line)
        if (match === null) throw new LocalFailure(`${at}: expected NAME=value`)
        const [, name, rest] = match as unknown as [string, string, string]
        if (!SETTING_NAME.test(name)) {
            throw new LocalFailure(`${at}: "${name}" is not a name Zega Cloud accepts (a letter or _ then letters, digits and _, up to 64)`)
        }
        values[name] = value(rest, at)
    })
    return values
}

function value(rest: string, at: string): string {
    const quote = rest[0]
    if (quote === '"' || quote === "'") {
        let out = ''
        for (let i = 1; i < rest.length; i++) {
            const c = rest[i]!
            if (c === quote) {
                const after = rest.slice(i + 1).trim()
                if (after !== '' && !after.startsWith('#')) throw new LocalFailure(`${at}: text after the closing quote`)
                return out
            }
            if (quote === '"' && c === '\\' && i + 1 < rest.length) {
                const next = rest[++i]!
                out += next === 'n' ? '\n' : next === 'r' ? '\r' : next === 't' ? '\t' : next
            } else {
                out += c
            }
        }
        throw new LocalFailure(`${at}: the quote is never closed (a value is on one line)`)
    }
    return rest.replace(/\s+#.*$/, '').trim()
}

export type DevVars = { vars: Record<string, string>; files: string[] }

/** The values for one function: `.env`, then `.env.<id>` over it (an app has no function: `.env` alone). */
export function readDevVars(dir: string, functionId?: string): DevVars {
    const vars: Record<string, string> = {}
    const files: string[] = []
    for (const name of functionId === undefined ? [VARS_FILE] : [VARS_FILE, `${VARS_FILE}.${functionId}`]) {
        const file = path.join(dir, name)
        let text: string
        try {
            text = fs.readFileSync(file, 'utf8')
        } catch (error) {
            if ((error as NodeJS.ErrnoException).code === 'ENOENT') continue
            throw new LocalFailure(`cannot read ${file}: ${(error as Error).message}`)
        }
        Object.assign(vars, parseDotenv(text, name))
        files.push(name)
    }
    for (const [name, text] of Object.entries(vars)) {
        if (text.length > MAX_VALUE_CHARS) {
            throw new LocalFailure(`${name} is ${text.length.toLocaleString('en-US')} characters; a variable or secret is at most ${MAX_VALUE_CHARS.toLocaleString('en-US')}`)
        }
    }
    if (Object.keys(vars).length > MAX_SETTINGS) {
        throw new LocalFailure(`${Object.keys(vars).length} values for ${functionId}; a function has at most ${MAX_SETTINGS} variables and secrets together`)
    }
    return { vars, files }
}
