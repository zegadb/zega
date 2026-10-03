import path from 'node:path'

import { startLocal, type LocalServer } from '@/handlers/explorer_local'
import { LocalFailure } from '@/utils/errors'

/** The default address of `zega start`. */
export const DEFAULT_LOCAL_URL = 'http://127.0.0.1:9342'

type Running = { data: string; server: LocalServer }

/**
 * The `zega-server` processes this MCP session started (the `local` group's `zega_local_start`), by data directory.
 * This server started them and is the only thing that ever signals them; they are stopped when it stops.
 */
export class LocalServers {
    private readonly running = new Map<string, Running>()

    /** Start one on the data directory (one per directory), on a port the system chooses. */
    async start(binary: string, data: string): Promise<{ url: string; data: string; alreadyRunning: boolean }> {
        const dir = path.resolve(data)
        const existing = this.running.get(dir)
        if (existing !== undefined) return { url: existing.server.url, data: dir, alreadyRunning: true }
        const server = await startLocal(binary, dir)
        this.running.set(dir, { data: dir, server })
        void server.exited.then(() => {
            if (this.running.get(dir)?.server === server) this.running.delete(dir)
        })
        return { url: server.url, data: dir, alreadyRunning: false }
    }

    list(): { url: string; data: string }[] {
        return [...this.running.values()].map(({ data, server }) => ({ url: server.url, data }))
    }

    /** Stop the one on this data directory (or the only one). Only a server this session started can be stopped. */
    async stop(data: string | undefined): Promise<{ stopped: string; data: string }> {
        let entry: Running | undefined
        if (data !== undefined) entry = this.running.get(path.resolve(data))
        else if (this.running.size === 1) entry = [...this.running.values()][0]
        else if (this.running.size > 1) throw new LocalFailure(`more than one local server is running (${this.list().map((s) => s.data).join(', ')}): say which with data`)
        if (entry === undefined) throw new LocalFailure('no local server started by this session is running there (a server somebody else started is not this tool\'s to stop)')
        await entry.server.stop()
        this.running.delete(entry.data)
        return { stopped: entry.server.url, data: entry.data }
    }

    /** The address a query goes to when none is given: the most recent server this session started, else `zega start`'s default. */
    defaultUrl(): string {
        const all = this.list()
        return all.length === 0 ? DEFAULT_LOCAL_URL : all[all.length - 1]!.url
    }

    async stopAll(): Promise<void> {
        await Promise.all([...this.running.values()].map(({ server }) => server.stop()))
        this.running.clear()
    }
}
