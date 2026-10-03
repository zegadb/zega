/**
 * Bundle a function's entry into the one ES module Zega Cloud takes
 * (`PUT /v1/functions/{function}/code`).
 *
 * Bun.build, target `browser` (the Workers runtime has no Node, no `process`, no
 * file system: browser resolution is the right default), format `esm`, resolving
 * with the conditions `workerd`, `worker` and `browser` so a package that ships a
 * Workers build gets it. npm dependencies are bundled in from the project's
 * `node_modules`. One file out, no code splitting: the API takes exactly one
 * module.
 *
 * No source map. Zega Cloud uploads the module as `worker.js` with nothing beside
 * it, the runtime ignores an inline map, and an inline map would count against
 * the 1,000,000-byte limit while changing no stack trace anyone can read.
 * Minifying is off unless asked for (`--minify`): readable exceptions in the
 * function's logs matter more than bytes until the size limit is near.
 *
 * Node built-ins are refused. Bun's browser target quietly substitutes a
 * polyfill for `node:crypto` (about 900 KB by itself, nearly the whole limit) and
 * for `node:buffer`, `path` and the rest, none of it written for the Workers
 * runtime; and Zega Cloud uploads functions without `nodejs_compat`, so the real
 * built-ins are not there either. A function that imports one would deploy and
 * then fail, or not fit. So the import, in the entry or in any dependency, is an
 * error that names it and the file that imports it.
 */
import fs from 'node:fs'
import { builtinModules } from 'node:module'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

/** The most bytes a function's module may have (the OpenAPI document: "at most 1,000,000 bytes"). */
export const MAX_FUNCTION_BYTES = 1_000_000

export type Bundle = { code: string; bytes: number }

const BUILTINS = new Set(builtinModules)

/** `node:crypto`, or a bare built-in name such as `path` (Bun resolves those to the built-in too). */
function isNodeBuiltin(specifier: string): boolean {
    return specifier.startsWith('node:') || BUILTINS.has(specifier)
}

export async function bundle(options: { dir: string; entry: string; minify: boolean }): Promise<Bundle> {
    const entry = path.resolve(options.dir, options.entry)
    if (!fs.existsSync(entry) || !fs.statSync(entry).isFile()) {
        throw new LocalFailure(`entry ${options.entry} is not a file (looked for ${entry})`)
    }
    const shown = (file: string): string => {
        const relative = path.relative(options.dir, file)
        return relative.startsWith('..') || path.isAbsolute(relative) ? file : relative.split(path.sep).join('/')
    }

    const builtins = new Map<string, string>()
    let result: Awaited<ReturnType<typeof Bun.build>>
    try {
        result = await Bun.build({
            entrypoints: [entry],
            target: 'browser',
            format: 'esm',
            conditions: ['workerd', 'worker', 'browser'],
            splitting: false,
            sourcemap: 'none',
            minify: options.minify,
            // `cloudflare:` modules are the runtime's own, not files to bundle.
            external: ['cloudflare:*'],
            // Libraries branch on it; the Workers runtime has no `process` to read it from.
            define: { 'process.env.NODE_ENV': '"production"' },
            throw: false,
            plugins: [
                {
                    name: 'zega-node-builtins',
                    setup(build) {
                        build.onResolve({ filter: /.*/ }, (args) => {
                            if (!isNodeBuiltin(args.path)) return undefined
                            const from = args.importer === '' || args.importer === undefined ? entry : args.importer
                            builtins.set(`${args.path} in ${shown(from)}`, args.path)
                            return { path: args.path, external: true }
                        })
                    },
                },
            ],
        })
    } catch (error) {
        throw new LocalFailure(`cannot bundle ${options.entry}: ${(error as Error).message}`)
    }

    if (builtins.size > 0) {
        throw new LocalFailure(
            `Node built-ins are not available in Zega Cloud functions yet: ${[...builtins.keys()].join(', ')}. ` +
                'Use what the Workers runtime has (fetch, Request, Response, URL, TextEncoder, crypto.subtle, crypto.randomUUID), ' +
                'or a package that does not import Node built-ins.',
        )
    }
    if (!result.success) {
        const problems = result.logs
            .filter((log) => log.level === 'error')
            .map((log) => {
                const where = log.position?.file ? ` (${shown(path.resolve(options.dir, log.position.file))}:${log.position.line})` : ''
                return `  ${log.message}${where}`
            })
        throw new LocalFailure(`cannot bundle ${options.entry}:\n${problems.join('\n') || '  the bundler failed without a message'}`)
    }
    const [output, ...rest] = result.outputs
    if (output === undefined || rest.length > 0) {
        throw new LocalFailure(
            `bundling ${options.entry} made ${result.outputs.length} files; a function is one module (do not import files that are not code, such as images)`,
        )
    }
    const code = await output.text()
    if (!/\bexport\s+default\b|\bas\s+default\b/.test(code)) {
        throw new LocalFailure(
            `${options.entry} has no default export: a function is \`export default { fetch(request, env) { ... } }\``,
        )
    }
    return { code, bytes: new TextEncoder().encode(code).length }
}
