/**
 * Finding the `zega-server` this CLI ships with.
 *
 * The database server is built once, in zegadb/zega, and published as one npm package per
 * platform, `@zegadb/server-<os>-<cpu>`, holding `bin/zega-server` (`bin/zega-server.exe` on
 * Windows) and nothing else: no install script, no dependency. `zegadb` (this CLI) lists them as
 * optionalDependencies at one exact version, so npm installs the package that matches the
 * machine, as create-deka-app does for the deka binary. Everything here finds that file.
 *
 * It is never the `zega-server` on PATH: a different version found by accident is the failure
 * this avoids. The only other way in is an explicit `--server <path>`, for development.
 */
import fs from 'node:fs'
import path from 'node:path'

import { LocalFailure } from '@/utils/errors'

/** npm's own names for the platforms zegadb/zega publishes (`process.platform`, `process.arch`). */
const PLATFORMS: { os: NodeJS.Platform; cpu: string; id: string; bin: string }[] = [
    { os: 'darwin', cpu: 'arm64', id: 'darwin-arm64', bin: 'zega-server' },
    { os: 'darwin', cpu: 'x64', id: 'darwin-x64', bin: 'zega-server' },
    { os: 'linux', cpu: 'x64', id: 'linux-x64', bin: 'zega-server' },
    { os: 'win32', cpu: 'x64', id: 'win32-x64', bin: 'zega-server.exe' },
]

export type ServerPackage = { name: string; bin: string }

/** The package that holds the server for this platform, or null when zegadb/zega publishes none. */
export function serverPackage(platform: NodeJS.Platform, arch: string): ServerPackage | null {
    const found = PLATFORMS.find((p) => p.os === platform && p.cpu === arch)
    return found === undefined ? null : { name: `@zegadb/server-${found.id}`, bin: found.bin }
}

/**
 * Where to look, in order: from the directory this CLI's code is in (an npm install puts the package
 * beside it, a checkout has it in its own node_modules), then from the directory of the running
 * executable (a compiled `zega` placed in a project's `node_modules/.bin`, say). In each, every
 * `node_modules` from that directory up to the filesystem root.
 */
export function searchRoots(): string[] {
    const roots: string[] = []
    // A compiled executable's own code lives in a virtual file system ("/$bunfs", "B:/~BUN"): not a place.
    if (!/^(?:\/\$bunfs|[A-Za-z]:[\\/]~BUN)/.test(import.meta.dir)) roots.push(import.meta.dir)
    roots.push(path.dirname(process.execPath))
    return roots
}

/** The installed binary of `pkg`, or null. */
export function findInstalled(pkg: ServerPackage, roots: string[]): string | null {
    for (const root of roots) {
        let dir = path.resolve(root)
        for (;;) {
            const candidate = path.join(dir, 'node_modules', ...pkg.name.split('/'), 'bin', pkg.bin)
            if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate
            const parent = path.dirname(dir)
            if (parent === dir) break
            dir = parent
        }
    }
    return null
}

export type Resolve = { override?: string; platform?: NodeJS.Platform; arch?: string; roots?: string[] }

/** The `zega-server` executable to run: `--server <path>` when given, else the platform package's. */
export function resolveServer(options: Resolve = {}): string {
    if (options.override !== undefined) {
        const file = path.resolve(options.override)
        if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
            throw new LocalFailure(`--server ${options.override}: no such file (looked for ${file})`)
        }
        return file
    }
    const platform = options.platform ?? process.platform
    const arch = options.arch ?? process.arch
    const pkg = serverPackage(platform, arch)
    if (pkg === null) {
        throw new LocalFailure(
            `there is no zega-server for ${platform}-${arch}: zegadb/zega publishes one for ${PLATFORMS.map((p) => p.id).join(', ')}`,
        )
    }
    const found = findInstalled(pkg, options.roots ?? searchRoots())
    if (found === null) {
        throw new LocalFailure(
            `the zega-server for ${platform}-${arch} is not installed: it is the npm package ${pkg.name}, which the zegadb package depends on. ` +
                'Reinstall zegadb (optional dependencies must not be skipped: no --omit=optional, no --no-optional), or point at a build with --server <path>.',
        )
    }
    return found
}

/** `--server <path>` or `--server=<path>` taken out of the arguments meant for zega-server (it has no such flag of its own). */
export function takeServerFlag(args: string[]): { override: string | undefined; rest: string[] } {
    const rest: string[] = []
    let override: string | undefined
    for (let i = 0; i < args.length; i++) {
        const arg = args[i]!
        if (arg === '--') {
            rest.push(...args.slice(i))
            break
        }
        if (arg === '--server') {
            const value = args[i + 1]
            if (value === undefined) throw new LocalFailure('--server needs a path')
            override = value
            i++
        } else if (arg.startsWith('--server=')) {
            override = arg.slice('--server='.length)
        } else {
            rest.push(arg)
        }
    }
    return { override, rest }
}
