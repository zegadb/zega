import { openSession, type Globals } from '@/handlers'
import { deployUnsupported, isApp } from '@/handlers/app'
import { prepare, selectTargets, upload } from '@/handlers/deploy'
import * as projectConfig from '@/handlers/project_config'

export type DeployOptions = Globals & { minify?: boolean }

/**
 * Deploy the functions `zega.json` lists, or only the one whose id is given:
 * bundle each entry into one ES module, then upload it. Needs a manage token.
 */
export async function deploy(id: string | undefined, options: DeployOptions): Promise<void> {
    const dir = process.cwd()
    const config = projectConfig.read(dir)
    if (isApp(config)) throw deployUnsupported(config)
    const targets = selectTargets(dir, id, ', or deploy one function with `zega function deploy <id> <entry>`')
    const prepared = await prepare(targets, { dir, minify: options.minify === true, human: options.human === true })
    await upload(await openSession(options), prepared, options)
}
