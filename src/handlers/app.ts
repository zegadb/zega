/**
 * An app project (the `nextjs` and `astro` starters): what `zega build`, `zega dev` and `zega deploy` do with it.
 * The framework's own tools do the work; this runs them from the project's
 * `node_modules/.bin` as `npm run` would, and reads what to run and what it produces from `zega.json`.
 */
import fs from 'node:fs'
import path from 'node:path'

import { readDevVars } from '@/handlers/dev_vars'
import type { AppBuild, ProjectConfig } from '@/handlers/project_config'
import { LocalFailure } from '@/utils/errors'

export type AppProject = ProjectConfig & {
    kind: 'app'
    framework: NonNullable<ProjectConfig['framework']>
    build: AppBuild
    runtime: NonNullable<ProjectConfig['runtime']>
}

export function isApp(config: ProjectConfig | null): config is AppProject {
    return config !== null && config.kind === 'app'
}

/** A tool from the project's own `node_modules/.bin`, or an error saying to install. */
export function projectTool(dir: string, name: string): string {
    const bin = path.join(dir, 'node_modules', '.bin')
    const found = [name, `${name}.cmd`].map((file) => path.join(bin, file)).find((file) => fs.existsSync(file))
    if (found === undefined) {
        throw new LocalFailure(`${name} is not installed here: install the project's packages first (npm install, or bun install)`, 'not_installed')
    }
    return found
}

/** The environment a framework tool runs in: the shell's own, with the project's tools first on the PATH, and `extra`. */
export function toolEnv(dir: string, extra: Record<string, string> = {}): Record<string, string> {
    const bin = path.join(dir, 'node_modules', '.bin')
    const env: Record<string, string> = {}
    for (const [name, value] of Object.entries(process.env)) if (value !== undefined) env[name] = value
    env.PATH = `${bin}${path.delimiter}${env.PATH ?? ''}`
    return { ...env, ...extra }
}

export type Built = { kind: 'app'; framework: string; command: string[]; worker: string; assets: string; workerBytes: number }

/**
 * Run the app's build and check what it promised: the worker file and the assets directory. The tool's own output
 * goes to stderr (stdout is the result); with `--human` it is shown as it is.
 */
export async function buildApp(dir: string, config: AppProject, human: boolean): Promise<Built> {
    const [tool, ...args] = config.build.command as [string, ...string[]]
    const child = Bun.spawn([projectTool(dir, tool), ...args], {
        cwd: dir,
        env: toolEnv(dir, readDevVars(dir).vars),
        stdin: 'ignore',
        stdout: human ? 'inherit' : 2,
        stderr: 'inherit',
    })
    const code = await child.exited
    if (code !== 0) throw new LocalFailure(`the build failed: \`${config.build.command.join(' ')}\` exited with ${code}`, 'build_failed')
    const worker = path.resolve(dir, config.build.worker)
    const assets = path.resolve(dir, config.build.assets)
    if (!fs.existsSync(worker) || !fs.statSync(worker).isFile()) {
        throw new LocalFailure(`the build did not make its server entry: ${config.build.worker} is missing (zega.json says \`build.worker\`)`, 'build_output_missing')
    }
    if (!fs.existsSync(assets) || !fs.statSync(assets).isDirectory()) {
        throw new LocalFailure(`the build did not make the assets: ${config.build.assets} is missing (zega.json says \`build.assets\`)`, 'build_output_missing')
    }
    return {
        kind: 'app',
        framework: config.framework,
        command: config.build.command,
        worker: config.build.worker,
        assets: config.build.assets,
        workerBytes: fs.statSync(worker).size,
    }
}

/** What each framework's dev server is run as, and how it is given a port. */
const DEV_SERVER: Record<string, { tool: string; args: string[]; port: (n: number) => string[] }> = {
    nextjs: { tool: 'next', args: ['dev'], port: (n) => ['--port', String(n)] },
    // --ignore-lock keeps `astro dev` in the foreground, where an agent's environment would otherwise send it to the background.
    astro: { tool: 'astro', args: ['dev', '--ignore-lock'], port: (n) => ['--port', String(n)] },
}

export type DevCommand = { command: string[]; env: Record<string, string>; names: string[]; files: string[] }

/** The framework's dev server command, and the environment it gets: the shell's own plus `.env` (and a chosen graph's address). */
export function devCommand(dir: string, config: AppProject, options: { port?: number; graph?: { urlVar: string; url: string } }): DevCommand {
    const server = DEV_SERVER[config.framework]
    if (server === undefined) throw new LocalFailure(`there is no dev server for ${config.framework}`, 'app_unsupported')
    const { vars, files } = readDevVars(dir)
    if (options.graph !== undefined) vars[options.graph.urlVar] = options.graph.url
    return {
        command: [projectTool(dir, server.tool), ...server.args, ...(options.port === undefined ? [] : server.port(options.port))],
        env: toolEnv(dir, vars),
        names: Object.keys(vars).sort(),
        files,
    }
}
