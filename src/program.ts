import { Command, InvalidArgumentError } from 'commander'

import pkg from '../package.json'
import * as core from '@/core'
import type { Globals } from '@/handlers'
import { describe, usageError } from '@/utils/errors'
import { write } from '@/utils'
import { logo } from '@/utils/banner'
import { helpJson } from '@/utils/help'

export const program = new Command()

program
    .name('zega')
    .version(pkg.version)
    .description('build, run and deploy with Zega')
    .option('--api <url>', 'use another API (e.g. staging)')
    .option('--token-file <path>', 'read the token from a file (CI, agents)')
    .option('--human', 'readable output for people')
    // A command-line mistake is JSON on stderr like any other error (text with --human).
    .configureOutput({ outputError: (text, out) => out(usageError(text, process.argv.slice(2).includes('--human'))) })
    // The help is JSON too, written from the command definitions below, never by hand.
    .configureHelp({ formatHelp: (cmd, helper) => helpJson(cmd, helper), showGlobalOptions: true })
    // `zega <cmd> --help` covers it; a lone "Commands:" group with only `help` is noise.
    .helpCommand(false)
    // The mark above the help, on a terminal only: piped help is plain JSON for scripts and agents.
    .addHelpText('beforeAll', () => (process.stdout.isTTY ? `\n${logo()}\n` : ''))

/** The options every command takes, from wherever on the command line they were written. */
const globals = (): Globals => program.opts<Globals>()

/** Run a command: a failure prints on stderr (JSON, or text with `--human`) and the exit code is 1, never anything else. */
function action<A extends unknown[]>(run: (...args: A) => Promise<void>) {
    return async (...args: A): Promise<void> => {
        try {
            await run(...args)
        } catch (error) {
            const options = globals()
            await write(
                'stderr',
                describe(error, { api: options.api ?? null, tokenFile: options.tokenFile ?? null }, options.human === true),
            )
            process.exitCode = 1
        }
    }
}

program
    .command('login')
    .description('save an API token')
    .helpGroup('Account:')
    .option('--store <where>', '`file` (default) or `keychain`')
    .action(action(async (options: { store?: string }) => core.login({ ...globals(), ...options })))

program
    .command('logout')
    .description('forget the saved token')
    .helpGroup('Account:')
    .action(action(async () => core.logout(globals())))

program
    .command('whoami')
    .description("show who you're logged in as")
    .helpGroup('Account:')
    .action(action(async () => core.whoami(globals())))

program
    .command('init')
    .description('create zega.json here')
    .helpGroup('Project:')
    .option('--name <name>', 'project name')
    .option('--region <id>', 'region (see `zega regions`)')
    .option('--force', 'overwrite zega.json')
    .option('--function <id>', 'a function id to deploy')
    .option('--entry <path>', 'its source file (default src/index.ts)')
    .action(action(async (options: { name?: string; region?: string; force?: boolean; function?: string; entry?: string }) => core.init({ ...globals(), ...options })))

program
    .command('deploy')
    .description('deploy your functions')
    .helpGroup('Project:')
    .argument('[id]', 'only this function')
    .option('--minify', 'minify the bundle')
    .action(action(async (id: string | undefined, options: { minify?: boolean }) => core.deploy(id, { ...globals(), ...options })))

program
    .command('start')
    .description('run a local zega server')
    .helpGroup('Local:')
    .allowUnknownOption()
    .allowExcessArguments()
    .helpOption(false)
    .argument('[args...]', 'server options; add --explorer to start the explorer too')
    .action(
        // commander would take the server's own --token-file for the global one: the arguments are read raw.
        action(async () => {
            process.exitCode = await core.start(process.argv.slice(process.argv.indexOf('start') + 1), globals())
        }),
    )

program
    .command('explorer')
    .description('open the explorer for a local database')
    .helpGroup('Local:')
    .argument('[graph]', 'a cloud graph: print its console link')
    .option('--port <n>', `port (default ${core.EXPLORER_PORT})`, integer('--port', 0, 65535))
    .option('--data <dir>', 'database folder (default ./zega-data)')
    .option('--server <path>', 'use this zega-server build')
    .option('--assets <dir>', 'use this explorer build')
    .action(
        action(async (graph: string | undefined, options: Omit<core.ExplorerOptions, keyof Globals>) =>
            core.explorer(graph, { ...globals(), ...options }),
        ),
    )

program
    .command('build')
    .description('bundle your functions into dist/')
    .helpGroup('Local:')
    .argument('[id]', 'only this function')
    .option('--minify', 'minify the bundle')
    .option('--out <dir>', 'output folder (default dist)')
    .action(action(async (id: string | undefined, options: { minify?: boolean; out?: string }) => core.build(id, { ...globals(), ...options })))

program
    .command('dev')
    .description('run your functions locally')
    .helpGroup('Local:')
    .argument('[id]', 'only this function')
    .option('--port <n>', `first port (default ${core.DEFAULT_PORT})`, integer('--port', 1, 65535))
    .option('--local-graph [url]', 'use your local graph (`zega start`)')
    .option('--cloud-graph <id>', 'use this cloud graph')
    .option('--graph-url-var <name>', 'graph URL variable (default ZEGA_GRAPH_URL)')
    .option('--graph-key-var <name>', 'graph key variable (default ZEGA_GRAPH_KEY)')
    .option('--minify', 'minify the bundle')
    .option('--no-watch', 'don\'t reload on changes')
    .action(action(async (id: string | undefined, options: Omit<core.DevOptions, keyof Globals>) => core.dev(id, { ...globals(), ...options })))

program
    .command('projects')
    .description('list projects')
    .helpGroup('Cloud:')
    .argument('[id]', 'a project id')
    .action(action(async (id: string | undefined) => core.projects(id, globals())))

program
    .command('graphs')
    .description('list graphs')
    .helpGroup('Cloud:')
    .argument('[id]', 'a graph id')
    .action(action(async (id: string | undefined) => core.graphs(id, globals())))

program
    .command('buckets')
    .description('list storage buckets')
    .helpGroup('Cloud:')
    .argument('[id]', 'a bucket id')
    .action(action(async (id: string | undefined) => core.buckets(id, globals())))

program
    .command('functions')
    .description('list functions')
    .helpGroup('Cloud:')
    .argument('[id]', 'a function id')
    .action(action(async (id: string | undefined) => core.functions(id, globals())))

program
    .command('usage')
    .description("this month's usage")
    .helpGroup('Cloud:')
    .action(action(async () => core.usage(globals())))

program
    .command('regions')
    .description('list regions')
    .helpGroup('Cloud:')
    .action(action(async () => core.regions(globals())))

/** A flag that is a whole number in a range. */
function integer(name: string, min: number, max: number) {
    return (value: string): number => {
        const number = Number(value)
        if (!/^\d+$/.test(value) || number < min || number > max) {
            throw new InvalidArgumentError(`${name} is a whole number from ${min} to ${max}`)
        }
        return number
    }
}

const fn = program.command('function').description("deploy one function, or read its logs").helpGroup('Project:').helpCommand(false)
fn.command('deploy')
    .description('deploy one function')
    .argument('<id>', 'a function id')
    .argument('[entry]', 'source file (default: from zega.json)')
    .option('--minify', 'minify the bundle')
    .action(
        action(async (id: string, entry: string | undefined, options: { minify?: boolean }) =>
            core.functionDeploy(id, entry, { ...globals(), ...options }),
        ),
    )
fn.command('logs')
    .description("show a function's recent requests and logs")
    .argument('<id>', 'a function id')
    .option('--from <time>', 'from (ISO time; default 24h ago)')
    .option('--to <time>', 'to (default now)')
    .option('--limit <n>', 'how many (1-200, default 50)', integer('--limit', 1, 200))
    .option('--status <code>', 'only this HTTP status', integer('--status', 100, 599))
    .option('--search <text>', 'search requests and logs')
    .option('--cursor <cursor>', 'next page (from the last output)')
    .action(
        action(async (id: string, options: Omit<core.LogsOptions, keyof Globals>) =>
            core.functionLogs(id, { ...globals(), ...options }),
        ),
    )

// Help lists the groups in this order (a stable sort keeps each group's own order).
const GROUPS = ['Local:', 'Project:', 'Cloud:', 'Account:']
;(program.commands as Command[]).sort((a: Command, b: Command) => GROUPS.indexOf(a.helpGroup()) - GROUPS.indexOf(b.helpGroup()))

