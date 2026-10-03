import type { Globals } from '@/handlers'
import { devCommand, isApp } from '@/handlers/app'
import * as projectConfig from '@/handlers/project_config'
import { prepare, selectTargets, type Target } from '@/handlers/deploy'
import { chooseGraph, DEFAULT_KEY_VAR, DEFAULT_URL_VAR, type GraphChoice, type GraphFlags } from '@/handlers/dev_graph'
import { DevFunction, load, type Env } from '@/handlers/dev_server'
import { readDevVars } from '@/handlers/dev_vars'
import { watchProject } from '@/handlers/dev_watch'
import { humanLine, type DevEvent } from '@/handlers/dev_events'
import { event as eventLine, relay, showLogo, write } from '@/utils'
import { logo } from '@/utils/banner'

export type DevOptions = Globals &
    GraphFlags & {
        /** The first function's port; the next one's is one more, in the order `zega.json` lists them. */
        port?: number
        minify?: boolean
        /** `--no-watch` makes it false. */
        watch?: boolean
        graphUrlVar?: string
        graphKeyVar?: string
    }

export const DEFAULT_PORT = 8787


/** What the function's `env` is right now, and which file or flag each name came from. */
function envFor(dir: string, target: Target, graph: GraphChoice, urlVar: string): { env: Env; files: string[]; overrides: boolean } {
    const { vars, files } = readDevVars(dir, target.id)
    const overrides = graph.kind !== 'none' && urlVar in vars
    if (graph.kind !== 'none') vars[urlVar] = graph.url
    return { env: vars, files, overrides }
}

/**
 * Serve each function `zega.json` lists (or only <id>) on localhost, built exactly as `zega deploy`
 * builds it, and rebuild when a file changes. Runs until interrupted.
 */
export async function dev(id: string | undefined, options: DevOptions): Promise<void> {
    const dir = process.cwd()
    const config = projectConfig.read(dir)
    if (isApp(config)) return await devApp(dir, config, options)
    const all = selectTargets(dir, undefined)
    const targets = id === undefined ? all : selectTargets(dir, id)
    const human = options.human === true
    /** An event: a JSON line on stdout, or with --human the line it always was, on stderr. */
    const say = (event: DevEvent): Promise<void> => write(human ? 'stderr' : 'stdout', human ? `${humanLine(event)}\n` : eventLine(event))
    const base = options.port ?? DEFAULT_PORT
    const urlVar = options.graphUrlVar ?? DEFAULT_URL_VAR
    const keyVar = options.graphKeyVar ?? DEFAULT_KEY_VAR
    const graph = await chooseGraph(options, options)

    const functions: DevFunction[] = []
    const stops: (() => void)[] = []
    const stopAll = (): void => {
        for (const stop of stops) stop()
        for (const fn of functions) fn.stop()
    }
    const lastCode = new Map<string, string>()
    const lastEnv = new Map<string, string>()
    let generation = 0

    /** Build, read the env and swap the new module in. False when nothing changed. */
    const refresh = async (fn: DevFunction): Promise<boolean> => {
        const [built] = await prepare([fn.target], { dir, minify: options.minify === true, human })
        const { env, overrides } = envFor(dir, fn.target, graph, urlVar)
        const envText = JSON.stringify(env)
        if (built!.code === lastCode.get(fn.target.id) && envText === lastEnv.get(fn.target.id)) return false
        const loaded = await load(fn.target, built!.code, env, ++generation)
        const before = new Set(fn.loaded?.warnings ?? [])
        fn.loaded = loaded
        fn.env = env
        lastCode.set(fn.target.id, built!.code)
        lastEnv.set(fn.target.id, envText)
        for (const warning of loaded.warnings) {
            if (!before.has(warning)) await say({ event: 'warning', function: fn.target.id, message: `${fn.target.entry}: ${warning}` })
        }
        if (overrides) {
            await say({ event: 'warning', function: fn.target.id, message: `${fn.target.id}: ${urlVar} from .dev.vars is replaced by the graph you chose` })
        }
        return true
    }

    try {
        for (const target of targets) {
            const fn = new DevFunction(target, base + all.findIndex((item) => item.id === target.id), (event) => void say(event))
            functions.push(fn)
            await refresh(fn)
        }
        for (const fn of functions) fn.start()
    } catch (error) {
        stopAll()
        throw error
    }

    // What is running, said once: the addresses, where the env came from, which graph.
    const described = functions.map((fn) => ({
        id: fn.target.id,
        entry: fn.target.entry,
        url: fn.url,
        env: Object.keys(fn.env).sort(),
        files: envFor(dir, fn.target, graph, urlVar).files,
    }))
    const graphLine =
        graph.kind === 'none'
            ? `none (--local-graph points ${urlVar} at a local zega-server, --cloud-graph <id> at a cloud graph)`
            : graph.kind === 'local'
              ? `LOCAL zega-server at ${graph.url} as ${urlVar}` +
                (graph.reachable ? '' : ' (nothing answers there yet: start it with `zega start`)')
              : `CLOUD graph ${graph.id} "${graph.name}" at ${graph.url} as ${urlVar}; its key must be ${keyVar} in .dev.vars (this command never sends your API token to a function or to the graph)`
    if (!human) {
        await write(
            'stdout',
            eventLine({
                event: 'ready',
                functions: described.map(({ files: _files, ...rest }) => rest),
                graph: graph.kind === 'none' ? null : { ...graph, urlVar },
                watching: options.watch !== false,
            }),
        )
    } else {
        const lines = ['zega dev: Bun serves your function; this is not the Workers runtime (see `zega dev --help`)']
        for (const item of described) {
            lines.push(`  ${item.id}  ${item.entry}  ${item.url}`)
            lines.push(
                `    env: ${item.env.length === 0 ? 'empty (no .dev.vars here)' : `${item.env.join(', ')}${item.files.length > 0 ? `  (${item.files.join(' + ')})` : ''}`}`,
            )
        }
        lines.push(`  graph: ${graphLine}`)
        lines.push(options.watch === false ? '  not watching for changes' : '  watching for changes; Ctrl-C to stop')
        // The mark on a terminal only: piped output stays the same plain lines.
        if (showLogo(options)) lines.unshift('', logo(), '')
        await write('stdout', lines.join('\n') + '\n')
    }

    if (graph.kind === 'cloud') {
        for (const item of described) {
            if (!item.env.includes(keyVar)) {
                await say({ event: 'warning', function: item.id, message: `${item.id}: no ${keyVar} in .dev.vars; the cloud graph will answer 401` })
            }
        }
    }

    // One rebuild at a time; a change that arrives during one asks for another.
    let running = false
    let again = false
    const rebuild = async (): Promise<void> => {
        if (running) {
            again = true
            return
        }
        running = true
        try {
            do {
                again = false
                for (const fn of functions) {
                    try {
                        if (await refresh(fn)) await say({ event: 'reloaded', function: fn.target.id, entry: fn.target.entry })
                    } catch (error) {
                        await say({ event: 'build_failed', function: fn.target.id, message: (error as Error).message })
                    }
                }
            } while (again)
        } finally {
            running = false
        }
    }
    if (options.watch !== false) stops.push(watchProject(dir, () => void rebuild()))

    await new Promise<void>((resolve) => {
        const leave = (): void => resolve()
        process.once('SIGINT', leave)
        process.once('SIGTERM', leave)
    })
    stopAll()
}

/**
 * `zega dev` in an app: the framework's own dev server (`next dev`, `astro dev`) with `.dev.vars` in its
 * environment (the framework does not read that file itself). Its output is passed on, and its exit code is ours.
 */
async function devApp(dir: string, config: Parameters<typeof devCommand>[1], options: DevOptions): Promise<void> {
    const human = options.human === true
    const urlVar = options.graphUrlVar ?? DEFAULT_URL_VAR
    const graph = await chooseGraph(options, options)
    const server = devCommand(dir, config, {
        port: options.port,
        graph: graph.kind === 'none' ? undefined : { urlVar, url: graph.url },
    })
    const shown = server.command.map((word, index) => (index === 0 ? word.split(/[\\/]/).pop()! : word))
    if (human) {
        await write(
            'stdout',
            `zega dev: ${config.framework} app, running \`${shown.join(' ')}\`\n` +
                `  env: ${server.names.length === 0 ? 'nothing added (no .dev.vars here)' : `${server.names.join(', ')}  (${server.files.join(' + ') || 'graph flag'})`}\n`,
        )
    } else {
        await write('stdout', eventLine({ event: 'starting', framework: config.framework, command: shown, env: server.names }))
    }
    const child = Bun.spawn(server.command, { cwd: dir, env: server.env, stdin: 'ignore', stdout: 'pipe', stderr: 'inherit' })
    const forward = (signal: NodeJS.Signals) => () => child.kill(signal)
    const signals = (['SIGINT', 'SIGTERM'] as const).map((signal) => [signal, forward(signal)] as const)
    for (const [signal, listener] of signals) process.on(signal, listener)
    const out = relay(human)
    try {
        const reader = child.stdout.getReader()
        for (;;) {
            const next = await reader.read()
            if (next.done) break
            await out.push(new TextDecoder().decode(next.value))
        }
        await out.end()
        const code = await child.exited
        const signal = child.signalCode
        process.exitCode = signal === null ? code : 128 + (signal === 'SIGINT' ? 2 : 15)
    } finally {
        for (const [signal, listener] of signals) process.off(signal, listener)
        if (child.exitCode === null) {
            child.kill('SIGTERM')
            await child.exited
        }
    }
}
