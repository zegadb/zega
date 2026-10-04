import fs from 'node:fs'
import path from 'node:path'

import type { Globals } from '@/handlers'
import { buildApp, isApp } from '@/handlers/app'
import { prepare, selectTargets } from '@/handlers/deploy'
import * as projectConfig from '@/handlers/project_config'
import { LocalFailure } from '@/utils/errors'
import { result, write } from '@/utils'

export type BuildOptions = Globals & { minify?: boolean; out?: string }

/** The directory `zega build` writes to, relative to `zega.json`. */
export const DEFAULT_OUT = 'dist'

/**
 * Bundle the functions `zega.json` lists, or only the one whose id is given, into
 * `<out>/<function id>.js`: exactly the module `zega deploy` would upload (it is the
 * same `prepare`, so the same bundler, the same refusals and the same size limit),
 * and nothing is sent anywhere. Needs no token. In an app (`"kind": "app"` in zega.json) it runs the app's own
 * build instead and reports the worker and the assets it made.
 */
export async function build(id: string | undefined, options: BuildOptions): Promise<void> {
    const dir = process.cwd()
    const config = projectConfig.read(dir)
    if (isApp(config)) return await app(dir, config, options)
    const targets = selectTargets(dir, id)
    const prepared = await prepare(targets, { dir, minify: options.minify === true, human: options.human === true })
    const outDir = path.resolve(dir, options.out ?? DEFAULT_OUT)
    try {
        fs.mkdirSync(outDir, { recursive: true })
    } catch (error) {
        throw new LocalFailure(`cannot make ${outDir}: ${(error as Error).message}`)
    }
    const built = prepared.map((item) => {
        const file = path.join(outDir, `${item.id}.js`)
        try {
            fs.writeFileSync(file, item.code)
        } catch (error) {
            throw new LocalFailure(`cannot write ${file}: ${(error as Error).message}`)
        }
        const relative = path.relative(dir, file).split(path.sep).join('/')
        return { id: item.id, entry: item.entry, file: relative.startsWith('..') ? file : relative, bytes: item.bytes }
    })
    if (options.human !== true) {
        await result(options, built)
        return
    }
    await write(
        'stdout',
        built.map((item) => `Built  ${item.id}  ${item.entry} -> ${item.file} (${item.bytes.toLocaleString('en-US')} bytes)\n`).join(''),
    )
}

/** An app's build: the framework's own, then what it made. */
async function app(dir: string, config: Parameters<typeof buildApp>[1], options: BuildOptions): Promise<void> {
    const human = options.human === true
    const built = await buildApp(dir, config, human)
    if (!human) return await result(options, built)
    await write(
        'stdout',
        `Built  ${built.framework} app: server entry ${built.worker} (${built.workerBytes.toLocaleString('en-US')} bytes), assets ${built.assets}\n`,
    )
}
