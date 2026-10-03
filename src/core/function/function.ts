import { openSession, type Globals } from '@/handlers'
import * as render from '@/utils/render'
import { show } from '@/utils'

export type LogsOptions = Globals & {
    from?: string
    to?: string
    limit?: number
    status?: number
    search?: string
    cursor?: string
}

/** A page of a function's requests, newest first, with their console lines. */
export async function functionLogs(id: string, options: LogsOptions): Promise<void> {
    const { client } = await openSession(options)
    const reply = await client.get('/v1/functions/{function}/logs', {
        path: { function: id },
        query: {
            from: options.from,
            to: options.to,
            limit: options.limit,
            status: options.status,
            q: options.search,
            cursor: options.cursor,
        },
    })
    await show(options, reply, render.logs)
}
