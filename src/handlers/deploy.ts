import { bundle, MAX_FUNCTION_BYTES, type Bundle } from '@/handlers/bundle'
import * as projectConfig from '@/handlers/project_config'
import type { Session } from '@/handlers/session'
import type { Globals } from '@/handlers/session'
import * as render from '@/utils/render'
import { LocalFailure } from '@/utils/errors'
import { show, write } from '@/utils/output'

export type Target = { id: string; entry: string }

export type Prepared = Target & Bundle

/**
 * The functions `zega.json` lists, or only the one whose id is given: what `deploy`,
 * `build` and `dev` each work on. `instead` says what to do when there is no
 * `zega.json` to read.
 */
export function selectTargets(dir: string, id: string | undefined, instead = ''): Target[] {
    const config = projectConfig.read(dir)
    if (config === null) {
        throw new LocalFailure(
            `no ${projectConfig.CONFIG_FILE} here: run \`zega init --function <id>\` to make one${instead}`,
        )
    }
    const listed = config.functions ?? []
    if (listed.length === 0) {
        throw new LocalFailure(
            `${projectConfig.CONFIG_FILE} lists no functions: add { "id": "<function id>", "entry": "src/index.ts" } under "functions"${instead}`,
        )
    }
    const targets = id === undefined ? listed : listed.filter((item) => item.id === id)
    if (targets.length === 0) {
        throw new LocalFailure(`${id} is not in ${projectConfig.CONFIG_FILE}; it lists ${listed.map((item) => item.id).join(', ')}`)
    }
    return targets
}

/**
 * Bundle each target, say how big each is, and refuse one over the API's limit,
 * before anything is uploaded: a function that cannot be bundled stops the whole
 * deploy while nothing has changed in the cloud.
 */
export async function prepare(targets: Target[], options: { dir: string; minify: boolean }): Promise<Prepared[]> {
    const prepared: Prepared[] = []
    for (const target of targets) {
        const built = await bundle({ dir: options.dir, entry: target.entry, minify: options.minify })
        await write(
            'stderr',
            `bundled ${target.entry}: ${built.bytes.toLocaleString('en-US')} bytes (limit ${MAX_FUNCTION_BYTES.toLocaleString('en-US')})\n`,
        )
        if (built.bytes > MAX_FUNCTION_BYTES) {
            throw new LocalFailure(
                `the bundle for ${target.entry} is ${built.bytes.toLocaleString('en-US')} bytes; a function is at most ${MAX_FUNCTION_BYTES.toLocaleString('en-US')} bytes. ` +
                    (options.minify ? 'Remove dependencies or split the function.' : 'Try --minify, remove dependencies, or split the function.'),
            )
        }
        prepared.push({ ...target, ...built })
    }
    return prepared
}

/** Upload each prepared bundle with `PUT /v1/functions/{function}/code` (a manage token), one after the other. */
export async function upload(session: Session, prepared: Prepared[], globals: Globals): Promise<void> {
    for (const item of prepared) {
        const reply = await session.client.put('/v1/functions/{function}/code', {
            path: { function: item.id },
            body: item.code,
            contentType: 'application/javascript',
        })
        await show(globals, reply, render.deployResult)
    }
}
