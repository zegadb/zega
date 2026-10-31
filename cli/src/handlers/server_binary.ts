/**
 * Finding the `zega-server` this CLI ships with.
 *
 * The database server is built once, in zegadb/zega, and published as one npm package per
 * platform, `@zegadb/server-<os>-<cpu>`, holding `bin/zega-server` (`bin/zega-server.exe` on
 * Windows) and nothing else: no install script, no dependency. `zegadb` (this CLI) lists them as
 * optionalDependencies at one exact version, so npm installs the package that matches the
 * machine, as create-deka-app does for the deka binary. Everything here finds that file.
 *
 * A compiled `zega` (`bun build --compile`) first uses the `zega-server` beside its own
 * executable: a release puts both in `~/.zega/bin` (zega.dev/install.sh), built from one commit.
 *
 * It is never the `zega-server` on PATH: a different version found by accident is the failure
 * this avoids. The only other way in is an explicit `--server <path>`, for development.
 */
import fs from 'node:fs'
import path from 'node:path'

import { SERVER_VERSION } from '@/server-version'
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

/** Whether this is a compiled `zega`: its own code lives in a virtual file system ("/$bunfs", "B:/~BUN"), not a place. */
function compiled(): boolean {
    return /^(?:\/\$bunfs|[A-Za-z]:[\\/]~BUN)/.test(import.meta.dir)
}

/** The `zega-server` beside a compiled `zega`'s executable `execPath`, or null (not compiled, or not there). */
export function besideExecutable(execPath: string, platform: NodeJS.Platform, isCompiled: boolean = compiled()): string | null {
    if (!isCompiled) return null
    const candidate = path.join(path.dirname(execPath), platform === 'win32' ? 'zega-server.exe' : 'zega-server')
    return fs.existsSync(candidate) && fs.statSync(candidate).isFile() ? candidate : null
}

/**
 * Where to look, in order: from the directory this CLI's code is in (an npm install puts the package
 * beside it, a checkout has it in its own node_modules), then from the directory of the running
 * executable (a compiled `zega` placed in a project's `node_modules/.bin`, say). In each, every
 * `node_modules` from that directory up to the filesystem root.
 */
export function searchRoots(): string[] {
    const roots: string[] = []
    if (!compiled()) roots.push(import.meta.dir)
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

/** `beside`: the server next to the executable (default: looked up); `null` skips that step. */
export type Resolve = { override?: string; platform?: NodeJS.Platform; arch?: string; roots?: string[]; beside?: string | null }

/** The `zega-server` executable to run: `--server <path>` when given, else the one beside a compiled `zega`, else the platform package's. */
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
    const beside = options.beside !== undefined ? options.beside : besideExecutable(process.execPath, platform)
    if (beside !== null) return beside
    const pkg = serverPackage(platform, arch)
    if (pkg === null) {
        throw new LocalFailure(
            `there is no zega-server for ${platform}-${arch}: zegadb/zega publishes one for ${PLATFORMS.map((p) => p.id).join(', ')}`,
        )
    }
    const found = findInstalled(pkg, options.roots ?? searchRoots())
    if (found === null) {
        throw new LocalFailure(
            `the zega-server for ${platform}-${arch} is not installed: it is the npm package ${pkg.name}@${SERVER_VERSION}, which the zegadb package depends on. ` +
                'Reinstall zegadb (optional dependencies must not be skipped: no --omit=optional, no --no-optional), or point at a build with --server <path>.',
        )
    }
    return found
}

export type StartFlags = {
    /** `--server <path>`: another zega-server build (development). */
    server: string | undefined
    /** `--explorer`: serve the explorer as well. */
    explorer: boolean
    /** `--explorer-port <n>`: its port (default 9343). */
    explorerPort: number | undefined
    /** `--human`: say what is running as the banner instead of one JSON line. */
    human: boolean
    /** Everything else, for `zega-server start`, in the order it was typed. */
    rest: string[]
}

/**
 * What `zega start` takes for itself out of the arguments meant for `zega-server start`: `--server`, `--explorer`,
 * `--explorer-port` and `--human` (the server has none of them). Everything else, and everything after `--`, is the
 * server's, untouched.
 */
export function takeStartFlags(args: string[]): StartFlags {
    const flags: StartFlags = { server: undefined, explorer: false, explorerPort: undefined, human: false, rest: [] }
    const value = (name: string, at: number): string => {
        const given = args[at + 1]
        if (given === undefined) throw new LocalFailure(`${name} needs a value`)
        return given
    }
    for (let i = 0; i < args.length; i++) {
        const arg = args[i]!
        if (arg === '--') {
            flags.rest.push(...args.slice(i))
            break
        }
        if (arg === '--server') {
            flags.server = value('--server', i)
            i++
        } else if (arg.startsWith('--server=')) {
            flags.server = arg.slice('--server='.length)
        } else if (arg === '--explorer') {
            flags.explorer = true
        } else if (arg === '--human') {
            flags.human = true
        } else if (arg === '--explorer-port' || arg.startsWith('--explorer-port=')) {
            const given = arg === '--explorer-port' ? value('--explorer-port', i) : arg.slice('--explorer-port='.length)
            if (arg === '--explorer-port') i++
            if (!/^\d+$/.test(given) || Number(given) > 65535) throw new LocalFailure('--explorer-port is a whole number from 0 to 65535')
            flags.explorerPort = Number(given)
        } else {
            flags.rest.push(arg)
        }
    }
    return flags
}

/** The value of a `--name value` or `--name=value` flag of the server's arguments (the last one wins, as the server's own parser has it). */
export function flagValue(args: string[], name: string): string | undefined {
    let found: string | undefined
    for (let i = 0; i < args.length; i++) {
        const arg = args[i]!
        if (arg === '--') break
        if (arg === name) found = args[i + 1]
        else if (arg.startsWith(`${name}=`)) found = arg.slice(name.length + 1)
    }
    return found
}
