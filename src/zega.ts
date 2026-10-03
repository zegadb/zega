#!/usr/bin/env bun

import { Command, InvalidArgumentError } from 'commander'

import pkg from '../package.json'
import * as core from '@/core'
import type { Globals } from '@/handlers'
import { describe, isZegaError, type ZegaError } from '@/utils/errors'
import { write } from '@/utils'

const program = new Command()

program
    .name('zega')
    .version(pkg.version)
    .description('The Zega developer CLI: manage Zega Cloud from a terminal, a script or an agent.')
    .option('--api <url>', 'the API address (default https://cloud.zega.dev; staging https://cloud.zega.world); `login` stores it with the token')
    .option('--token-file <path>', 'use the token in this file instead of the stored one (CI, agents); one nonempty token')
    .option('--json', "print the API's JSON unchanged instead of a table; never ask questions")
    .showHelpAfterError()

/** The options every command takes, from wherever on the command line they were written. */
const globals = (): Globals => program.opts<Globals>()

/** Run a command: a failure prints on stderr and the exit code is 1, never anything else. */
function action<A extends unknown[]>(run: (...args: A) => Promise<void>) {
    return async (...args: A): Promise<void> => {
        try {
            await run(...args)
        } catch (error) {
            const known: ZegaError | null = isZegaError(error) ? error : null
            const options = globals()
            if (known === null) {
                await write('stderr', `zega: ${error instanceof Error ? error.message : String(error)}\n`)
            } else {
                await write(
                    'stderr',
                    describe(known, { api: options.api ?? null, tokenFile: options.tokenFile ?? null }),
                )
            }
            process.exitCode = 1
        }
    }
}

program
    .command('login')
    .description(
        'store an API token (made at https://dashboard.zega.dev/tokens): read from stdin, --token-file or a hidden prompt, never from an argument; checked with the API before it is saved',
    )
    .option('--store <where>', '`file` (default: mode 0600 in your config directory) or `keychain` (the OS keychain)')
    .action(action(async (options: { store?: string }) => core.login({ ...globals(), ...options })))

program
    .command('logout')
    .description('remove the stored token (it stays valid until you revoke it in the dashboard)')
    .action(action(async () => core.logout(globals())))

program
    .command('whoami')
    .description('show which token this is: name, scope, project restriction, expiry')
    .action(action(async () => core.whoami(globals())))

program
    .command('init')
    .description('write zega.json for a project here (creates nothing in Zega Cloud)')
    .option('--name <name>', 'the project name, 1 to 64 characters')
    .option('--region <id>', 'a region id (see `zega regions`)')
    .option('--force', 'replace zega.json if it exists')
    .action(action(async (options: { name?: string; region?: string; force?: boolean }) => core.init({ ...globals(), ...options })))

program
    .command('projects')
    .description('list projects, or show one with its graphs')
    .argument('[id]', 'a project id')
    .action(action(async (id: string | undefined) => core.projects(id, globals())))

program
    .command('graphs')
    .description('list graphs, or show one')
    .argument('[id]', 'a graph id')
    .action(action(async (id: string | undefined) => core.graphs(id, globals())))

program
    .command('buckets')
    .description('list storage buckets, or show one')
    .argument('[id]', 'a bucket id')
    .action(action(async (id: string | undefined) => core.buckets(id, globals())))

program
    .command('functions')
    .description('list functions, or show one (its variables and secret names too)')
    .argument('[id]', 'a function id (f0 and 16 characters)')
    .action(action(async (id: string | undefined) => core.functions(id, globals())))

program
    .command('usage')
    .description("this month's usage and bill per graph")
    .action(action(async () => core.usage(globals())))

program
    .command('regions')
    .description('the regions a graph can run in')
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

const fn = program.command('function').description("a function's logs")
fn.command('logs')
    .description("a page of a function's requests, newest first, with their console lines")
    .argument('<id>', 'a function id')
    .option('--from <time>', 'start of the window: an ISO time or epoch milliseconds (default: 24 hours before --to)')
    .option('--to <time>', 'end of the window (default: now)')
    .option('--limit <n>', 'requests in the page, 1 to 200 (default 50)', integer('--limit', 1, 200))
    .option('--status <code>', 'only requests that answered this HTTP status', integer('--status', 100, 599))
    .option('--search <text>', 'search the method, path, error and every console line')
    .option('--cursor <cursor>', 'the cursor a previous page printed')
    .action(
        action(async (id: string, options: Omit<core.LogsOptions, keyof Globals>) =>
            core.functionLogs(id, { ...globals(), ...options }),
        ),
    )

await program.parseAsync()
// A prompt can leave stdin open: leave on purpose, once everything is written.
process.exit(process.exitCode ?? 0)
