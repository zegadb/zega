/**
 * The project file `zega init` writes: `zega.json` in the project's directory.
 * Later commands (deploy, the local server) read it.
 *
 *     {
 *       "version": 1,
 *       "name": "my-app",
 *       "region": "yyz"
 *     }
 *
 * `version` is the file's own format. `name` is the project's display label
 * (1 to 64 characters, as in Zega Cloud). `region` is a Zega Cloud region id,
 * where the project's graph will live. The file holds no secret and no cloud
 * resource id: it is safe to commit, and it does not say a cloud project exists.
 */
import fs from 'node:fs'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

export const CONFIG_FILE = 'zega.json'

export type ProjectConfig = { version: 1; name: string; region: string }

export function configPath(dir: string): string {
    return path.join(dir, CONFIG_FILE)
}

export function exists(dir: string): boolean {
    return fs.existsSync(configPath(dir))
}

/** Write the file (replacing one) and return its absolute path. */
export function write(dir: string, config: ProjectConfig): string {
    const file = path.resolve(configPath(dir))
    try {
        fs.writeFileSync(file, JSON.stringify(config, null, 2) + '\n')
    } catch (error) {
        throw new LocalFailure(`cannot write ${file}: ${(error as Error).message}`)
    }
    return file
}
