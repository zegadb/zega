/**
 * Where `zega login` keeps the token, and how a token is read.
 *
 * One JSON file, `cloud.json`, in the user's config directory:
 * `$XDG_CONFIG_HOME/zega` (default `~/.config/zega`) on Linux and macOS,
 * `%APPDATA%\zega` on Windows. It holds the API host the token belongs to, so a
 * later command talks to the host the token was made for, and either
 *
 *   - the token itself (`store: "file"`, the default): mode 0600 in a 0700
 *     directory on Unix, the profile directory's own ACL on Windows; or
 *   - nothing secret (`store: "keychain"`, `zega login --store keychain`): the
 *     token is in the OS keychain (Bun.secrets: macOS Keychain, libsecret on
 *     Linux, Windows Credential Manager) under service `zega-cli`, account =
 *     the API host.
 *
 * Reading the home or config directory is how the OS says where a user's files
 * live, not product behavior: nothing here changes what a command does.
 */
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

export type Store = 'file' | 'keychain'

export type Stored = { api: string; token: string; store: Store }

const KEYCHAIN_SERVICE = 'zega-cli'

export function configDir(): string {
    if (process.platform === 'win32') {
        const appdata = process.env.APPDATA
        if (appdata === undefined || appdata === '') {
            throw new LocalFailure('cannot find your config directory: %APPDATA% is not set')
        }
        return path.join(appdata, 'zega')
    }
    const xdg = process.env.XDG_CONFIG_HOME
    if (xdg !== undefined && path.isAbsolute(xdg)) return path.join(xdg, 'zega')
    const home = os.homedir()
    if (!path.isAbsolute(home)) throw new LocalFailure('cannot find your config directory: HOME is not set')
    return path.join(home, '.config', 'zega')
}

export function credentialsPath(): string {
    return path.join(configDir(), 'cloud.json')
}

type StoredRecord = { api: string; store: Store; token: string | null }

function readRecord(): StoredRecord | null {
    const file = credentialsPath()
    let text: string
    try {
        text = fs.readFileSync(file, 'utf8')
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null
        throw new LocalFailure(`cannot read ${file}: ${(error as Error).message}`)
    }
    const broken = (why: string) =>
        new LocalFailure(`${file} is not a zega credentials file (${why}); run \`zega login\` to write it again`)
    let value: unknown
    try {
        value = JSON.parse(text)
    } catch (error) {
        throw broken((error as Error).message)
    }
    const field = (name: string): string | null => {
        const v = typeof value === 'object' && value !== null ? (value as Record<string, unknown>)[name] : undefined
        return typeof v === 'string' && v !== '' ? v : null
    }
    const api = field('api')
    if (api === null) throw broken('it needs an `api`')
    const store = field('store') ?? 'file'
    if (store === 'keychain') return { api, store, token: null }
    if (store !== 'file') throw broken(`unknown store ${JSON.stringify(store)}`)
    const token = field('token')
    if (token === null) throw broken('it needs a `token`')
    return { api, store, token }
}

export async function load(): Promise<Stored | null> {
    const record = readRecord()
    if (record === null) return null
    if (record.token !== null) return { api: record.api, token: record.token, store: 'file' }
    const token = await keychain(() => Bun.secrets.get({ service: KEYCHAIN_SERVICE, name: record.api }))
    if (token === null) {
        throw new LocalFailure(
            `the token for ${record.api} is not in the OS keychain any more; run \`zega login\` again`,
        )
    }
    return { api: record.api, token, store: 'keychain' }
}

/**
 * Store the token. The credentials file is never readable by anyone else, not
 * even for an instant: it is created with its final mode, then renamed into
 * place, so a crash leaves the old file or the new one. Nothing says where it
 * went: that is not for the screen.
 */
export async function save(api: string, token: string, store: Store): Promise<void> {
    const file = credentialsPath()
    const previous = readRecordQuietly()
    let body: Record<string, string>
    if (store === 'keychain') {
        await keychain(() => Bun.secrets.set({ service: KEYCHAIN_SERVICE, name: api, value: token }))
        body = { api, store: 'keychain' }
    } else {
        body = { api, store: 'file', token }
    }
    const dir = path.dirname(file)
    try {
        fs.mkdirSync(dir, { recursive: true, mode: 0o700 })
        const partial = `${file}.${process.pid}.partial`
        try {
            fs.writeFileSync(partial, JSON.stringify(body, null, 2) + '\n', { mode: 0o600, flag: 'wx' })
            fs.renameSync(partial, file)
        } catch (error) {
            fs.rmSync(partial, { force: true })
            throw error
        }
    } catch (error) {
        throw new LocalFailure(`cannot write ${file}: ${(error as Error).message}`)
    }
    // A keychain entry the new login replaced would otherwise stay behind.
    if (previous !== null && previous.store === 'keychain' && !(store === 'keychain' && previous.api === api)) {
        await keychain(() => Bun.secrets.delete({ service: KEYCHAIN_SERVICE, name: previous.api })).catch(() => false)
    }
}

function readRecordQuietly(): StoredRecord | null {
    try {
        return readRecord()
    } catch {
        return null
    }
}

/** Remove the stored token. Returns what was removed, or null when there was nothing. */
export async function remove(): Promise<string | null> {
    const file = credentialsPath()
    const record = readRecord()
    if (record === null) return null
    if (record.store === 'keychain') {
        await keychain(() => Bun.secrets.delete({ service: KEYCHAIN_SERVICE, name: record.api }))
    }
    fs.rmSync(file, { force: true })
    return record.store === 'keychain' ? `the OS keychain entry for ${record.api} and ${file}` : file
}

async function keychain<T>(action: () => Promise<T>): Promise<T> {
    try {
        return await action()
    } catch (error) {
        throw new LocalFailure(
            `the OS keychain is not available here (${(error as Error).message}); use the default file store: \`zega login\` without --store`,
        )
    }
}

/** One nonempty token: no spaces or line breaks inside, a final newline is fine. */
export function oneToken(text: string): string | null {
    const token = text.trim()
    return token !== '' && !/\s/.test(token) ? token : null
}

/** A token from `--token-file`. */
export function readTokenFile(file: string): string {
    let text: string
    try {
        text = fs.readFileSync(file, 'utf8')
    } catch (error) {
        throw new LocalFailure(`cannot read token file ${file}: ${(error as Error).message}`)
    }
    const token = oneToken(text)
    if (token === null) throw new LocalFailure(`token file ${file} must contain one nonempty token`)
    return token
}

/** Everything on stdin, for a token that is piped in. */
export async function readStdin(): Promise<string> {
    return await Bun.stdin.text()
}
