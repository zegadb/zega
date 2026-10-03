import { projectConfig, openSession, type Globals } from '@/handlers'
import { prepare, upload } from '@/handlers/deploy'
import { LocalFailure } from '@/utils/errors'

export type DeployOptions = Globals & { minify?: boolean }

/**
 * Deploy the functions `zega.json` lists, or only the one whose id is given:
 * bundle each entry into one ES module, then upload it. Needs a manage token.
 */
export async function deploy(id: string | undefined, options: DeployOptions): Promise<void> {
    const dir = process.cwd()
    const config = projectConfig.read(dir)
    if (config === null) {
        throw new LocalFailure(
            `no ${projectConfig.CONFIG_FILE} here: run \`zega init --function <id>\` to make one, or deploy one function with \`zega function deploy <id> <entry>\``,
        )
    }
    const listed = config.functions ?? []
    if (listed.length === 0) {
        throw new LocalFailure(
            `${projectConfig.CONFIG_FILE} lists no functions: add { "id": "<function id>", "entry": "src/index.ts" } under "functions", or run \`zega function deploy <id> <entry>\``,
        )
    }
    const targets = id === undefined ? listed : listed.filter((item) => item.id === id)
    if (targets.length === 0) {
        throw new LocalFailure(`${id} is not in ${projectConfig.CONFIG_FILE}; it lists ${listed.map((item) => item.id).join(', ')}`)
    }
    const prepared = await prepare(targets, { dir, minify: options.minify === true })
    await upload(await openSession(options), prepared, options)
}
