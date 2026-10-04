/**
 * The project file `zega init` writes: `zega.json` in the project's directory.
 * `zega deploy` and, later, the local server read it.
 *
 *     {
 *       "version": 1,
 *       "name": "my-app",
 *       "region": "yyz",
 *       "functions": [
 *         { "id": "f0abcdefghjkmnpqrs", "entry": "src/index.ts" }
 *       ]
 *     }
 *
 * `version` is the file's own format. `name` is the project's display label
 * (1 to 64 characters, as in Zega Cloud). `region` is a Zega Cloud region id,
 * where the project's graph will live. `functions` is optional: each item ties
 * a cloud function (made in the dashboard or with the API) to the source file
 * `zega deploy` bundles for it, `entry`, a path relative to this file. The file
 * holds no secret: it is safe to commit, and it does not say a cloud project
 * exists. Keys are only ever added to version 1; `version` changes only if the
 * meaning of an existing key does.
 *
 * An app (the `nextjs` and `astro` starters of `zega init`) has no `functions`. It says what it is, how it
 * is built and where the build puts its output, and the runtime flags the worker needs, so that
 * `zega build` (and, later, a deploy) read them from here instead of from a wrangler file:
 *
 *     {
 *       "kind": "app",
 *       "framework": "nextjs",
 *       "build": { "command": ["opennextjs-cloudflare", "build", "..."], "worker": ".open-next/worker.js", "assets": ".open-next/assets" },
 *       "runtime": { "compatibilityDate": "2026-09-01", "compatibilityFlags": ["nodejs_compat"] }
 *     }
 *
 * `build.command` is run in this directory with `node_modules/.bin` first on the PATH; `worker` and `assets`
 * are paths relative to this file.
 */
import fs from 'node:fs'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

export const CONFIG_FILE = 'zega.json'

export type FunctionEntry = { id: string; entry: string }

export const FRAMEWORKS = ['nextjs', 'astro'] as const
export type Framework = (typeof FRAMEWORKS)[number]

export type AppBuild = { command: string[]; worker: string; assets: string }
export type AppRuntime = { compatibilityDate: string; compatibilityFlags: string[] }

export type ProjectConfig = {
    version: 1
    name: string
    region: string
    /** `function` (the default when absent) or `app`. */
    kind?: 'function' | 'app'
    framework?: Framework
    build?: AppBuild
    runtime?: AppRuntime
    functions?: FunctionEntry[]
}

/** A function id: `f0` and 16 characters (the API's own rule, checked here only to catch a typo early). */
export function isFunctionId(value: string): boolean {
    return /^f0[A-Za-z0-9]{16}$/.test(value)
}

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

/** Read and check the file. Null when there is none. */
export function read(dir: string): ProjectConfig | null {
    const file = path.resolve(configPath(dir))
    let text: string
    try {
        text = fs.readFileSync(file, 'utf8')
    } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null
        throw new LocalFailure(`cannot read ${file}: ${(error as Error).message}`)
    }
    const broken = (why: string) => new LocalFailure(`${file} is not a valid zega.json: ${why}`)
    let value: unknown
    try {
        value = JSON.parse(text)
    } catch (error) {
        throw broken((error as Error).message)
    }
    if (typeof value !== 'object' || value === null || Array.isArray(value)) throw broken('it must be a JSON object')
    const object = value as Record<string, unknown>
    if (object.version !== 1) throw broken('`version` must be 1 (this zega understands version 1)')
    if (typeof object.name !== 'string' || object.name === '') throw broken('`name` must be a nonempty string')
    if (typeof object.region !== 'string' || object.region === '') throw broken('`region` must be a nonempty string')
    const config: ProjectConfig = { version: 1, name: object.name, region: object.region }
    if (object.kind !== undefined) {
        if (object.kind !== 'function' && object.kind !== 'app') throw broken('`kind` must be "function" or "app"')
        config.kind = object.kind
    }
    if (object.kind === 'app') {
        if (typeof object.framework !== 'string' || !(FRAMEWORKS as readonly string[]).includes(object.framework)) {
            throw broken(`\`framework\` must be one of ${FRAMEWORKS.join(', ')}`)
        }
        config.framework = object.framework as Framework
        const build = object.build as Record<string, unknown> | null | undefined
        const command = build?.command
        if (
            typeof build !== 'object' ||
            build === null ||
            !Array.isArray(command) ||
            command.length === 0 ||
            !command.every((word) => typeof word === 'string' && word !== '') ||
            typeof build.worker !== 'string' ||
            build.worker === '' ||
            typeof build.assets !== 'string' ||
            build.assets === ''
        ) {
            throw broken('`build` must have a `command` (a list of words), a `worker` and an `assets` path')
        }
        config.build = { command: command as string[], worker: build.worker, assets: build.assets }
        const runtime = object.runtime as Record<string, unknown> | null | undefined
        if (
            typeof runtime !== 'object' ||
            runtime === null ||
            typeof runtime.compatibilityDate !== 'string' ||
            !Array.isArray(runtime.compatibilityFlags) ||
            !runtime.compatibilityFlags.every((flag) => typeof flag === 'string')
        ) {
            throw broken('`runtime` must have a `compatibilityDate` and a list of `compatibilityFlags`')
        }
        config.runtime = { compatibilityDate: runtime.compatibilityDate, compatibilityFlags: runtime.compatibilityFlags as string[] }
    }
    if (object.functions !== undefined) {
        if (!Array.isArray(object.functions)) throw broken('`functions` must be a list')
        config.functions = object.functions.map((item: unknown, index): FunctionEntry => {
            const at = `functions[${index}]`
            const entry = item as Record<string, unknown> | null
            if (typeof entry !== 'object' || entry === null) throw broken(`${at} must be an object with an \`id\` and an \`entry\``)
            if (typeof entry.id !== 'string' || !isFunctionId(entry.id)) {
                throw broken(`${at}.id must be a function id: "f0" and 16 characters`)
            }
            if (typeof entry.entry !== 'string' || entry.entry === '') {
                throw broken(`${at}.entry must be the path of the source file to bundle`)
            }
            return { id: entry.id, entry: entry.entry }
        })
        const ids = config.functions.map((f) => f.id)
        const twice = ids.find((id, index) => ids.indexOf(id) !== index)
        if (twice !== undefined) throw broken(`function ${twice} is listed twice`)
    }
    return config
}
