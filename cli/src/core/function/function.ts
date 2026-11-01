import { openSession, projectConfig, type Globals } from '@/handlers'
import { prepare, upload } from '@/handlers/deploy'
import { LocalFailure } from '@/utils/errors'
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

export type FunctionDeployOptions = Globals & { minify?: boolean }

/**
 * Bundle one function's entry and upload it. The entry is the argument, or the
 * one `zega.json` lists for this id. Needs a manage token.
 */
export async function functionDeploy(
    id: string,
    entry: string | undefined,
    options: FunctionDeployOptions,
): Promise<void> {
    const dir = process.cwd()
    const found = entry ?? projectConfig.read(dir)?.functions?.find((item) => item.id === id)?.entry
    if (found === undefined) {
        throw new LocalFailure(
            `no entry for ${id}: pass it (\`zega function deploy ${id} <entry>\`) or list it in ${projectConfig.CONFIG_FILE}`,
        )
    }
    const prepared = await prepare([{ id, entry: found }], { dir, minify: options.minify === true, human: options.human === true })
    await upload(await openSession(options), prepared, options)
}
