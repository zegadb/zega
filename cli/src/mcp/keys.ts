/**
 * The keys a graph or a bucket is opened with, which are not the API token.
 *
 * A graph answers queries only to a graph key (`zk_...`), a bucket only to a storage key (`zs_...`); the API token
 * (`zc_...`) is refused at both. The API can make a key (`POST /v1/graphs/{graph}/keys`, a manage token; the secret
 * is in that one answer and nowhere else), so this finds one in this order:
 *
 *   1. one the person gave: `--graph-key-file` / `--storage-key-file`, or for a graph `ZEGA_GRAPH_KEY` in `.env`
 *      (the file `zega dev` reads; skipped when its `ZEGA_GRAPH_URL` names another graph);
 *   2. one made for this session, when the token is `manage` and the server is not `--read-only` (making a key changes
 *      the account). It is named `zega mcp <time>`, lives in memory only and is revoked when the server stops.
 *
 * With neither there is no key, and the tool says what to do about it. A key is never in any answer: `secrets()` is
 * what the answers are scrubbed of.
 */
import fs from 'node:fs'
import path from 'node:path'

import type { Session } from '@/handlers'
import { parseDotenv, VARS_FILE } from '@/handlers/dev_vars'
import { LocalFailure } from '@/utils/errors'

const GRAPH_KEY = /^zk_[a-z2-7]{32}$/
const STORAGE_KEY = /^zs_[a-z2-7]{32}$/

type Made = { id: string; secret: string; revoke: (session: Session) => Promise<unknown> }

export type KeyOptions = {
    session: () => Promise<Session>
    /** The project directory, where `.env` is. */
    cwd: string
    /** Whether a key may be made: a manage token, and not `--read-only`. */
    mayMake: boolean
    /** Why not, when `mayMake` is false (for the message). */
    whyNot: string
    graphKeyFile?: string
    storageKeyFile?: string
    /** Secrets that are not keys (the API token) and must not appear in an answer either. */
    others?: string[]
}

export class KeyRing {
    private readonly cache = new Map<string, Promise<string>>()
    private readonly made: Made[] = []
    private readonly known = new Set<string>()

    constructor(private readonly options: KeyOptions) {
        for (const other of options.others ?? []) this.known.add(other)
    }

    /** Everything an answer must be scrubbed of. */
    secrets(): string[] {
        return [...this.known]
    }

    async graphKey(graph: string): Promise<string> {
        return await this.find(`graph:${graph}`, () => this.provided('graph', graph) ?? this.make('graph', graph))
    }

    async storageKey(bucket: string): Promise<string> {
        return await this.find(`bucket:${bucket}`, () => this.provided('bucket', bucket) ?? this.make('bucket', bucket))
    }

    private async find(id: string, get: () => string | Promise<string>): Promise<string> {
        const cached = this.cache.get(id)
        if (cached !== undefined) return await cached
        const found = Promise.resolve(get())
        this.cache.set(id, found)
        try {
            const key = await found
            this.known.add(key)
            return key
        } catch (error) {
            this.cache.delete(id)
            throw error
        }
    }

    /** A key the person gave, or null. */
    private provided(kind: 'graph' | 'bucket', id: string): string | null {
        const { graphKeyFile, storageKeyFile, cwd } = this.options
        const file = kind === 'graph' ? graphKeyFile : storageKeyFile
        const pattern = kind === 'graph' ? GRAPH_KEY : STORAGE_KEY
        const flag = kind === 'graph' ? '--graph-key-file' : '--storage-key-file'
        if (file !== undefined) {
            let text: string
            try {
                text = fs.readFileSync(file, 'utf8').trim()
            } catch (error) {
                throw new LocalFailure(`cannot read ${flag} ${file}: ${(error as Error).message}`)
            }
            if (!pattern.test(text)) throw new LocalFailure(`${flag} ${file} must hold one ${kind} key (${kind === 'graph' ? 'zk_' : 'zs_'} and 32 characters)`)
            return text
        }
        if (kind === 'graph') {
            const dotenv = path.join(cwd, VARS_FILE)
            if (fs.existsSync(dotenv)) {
                const vars = parseDotenv(fs.readFileSync(dotenv, 'utf8'), dotenv)
                const key = vars.ZEGA_GRAPH_KEY
                const named = /^https:\/\/([^./]+)\.zegadb\.com\/?$/.exec(vars.ZEGA_GRAPH_URL ?? '')?.[1]
                if (key !== undefined && pattern.test(key) && (named === undefined || named === id)) return key
            }
        }
        return null
    }

    private async make(kind: 'graph' | 'bucket', id: string): Promise<string> {
        const flag = kind === 'graph' ? 'a graph key: pass --graph-key-file <file> or put ZEGA_GRAPH_KEY in .env' : 'a storage key: pass --storage-key-file <file>'
        if (!this.options.mayMake) {
            throw new LocalFailure(
                `no key for ${kind} ${id}: ${this.options.whyNot}, so none is made. Give ${flag} (a key is made in the dashboard).`,
                'key_unavailable',
            )
        }
        const session = await this.options.session()
        const label = { name: `zega mcp ${new Date().toISOString()}` }
        const created =
            kind === 'graph'
                ? await session.client.post('/v1/graphs/{graph}/keys', { path: { graph: id }, json: label })
                : await session.client.post('/v1/buckets/{bucket}/keys', { path: { bucket: id }, json: label })
        const { secret, id: keyId } = created.json.key
        // Known before anything else can print it.
        this.known.add(secret)
        this.made.push({
            id: keyId,
            secret,
            revoke: (s) =>
                kind === 'graph'
                    ? s.client.delete('/v1/graphs/{graph}/keys/{key}', { path: { graph: id, key: keyId } })
                    : s.client.delete('/v1/buckets/{bucket}/keys/{key}', { path: { bucket: id, key: keyId } }),
        })
        return secret
    }

    /** Revoke the keys this session made. Failures are returned, not thrown: a stopping server has nowhere to report. */
    async revokeAll(): Promise<string[]> {
        const failed: string[] = []
        const session = this.made.length === 0 ? null : await this.options.session().catch(() => null)
        for (const key of this.made.splice(0)) {
            try {
                if (session === null) throw new Error('no session')
                await key.revoke(session)
            } catch {
                failed.push(key.id)
            }
        }
        return failed
    }
}
