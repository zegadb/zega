import { afterAll, describe, expect, test } from 'bun:test'

import type { Command } from 'commander'

import { program } from '../src/program'
import { helpObject } from '../src/utils/help'
import { paint } from '../src/utils/pretty'
import { errorOf, remove, run, scratch } from './helpers'

const home = scratch()
afterAll(() => remove(home))

/** Every command of the CLI, each with the words that reach it (`['function', 'logs']`), found in commander, not in a list of ours. */
function everyCommand(parent: Command = program, prefix: string[] = []): { path: string[]; command: Command }[] {
    return parent.commands.flatMap((command) => [{ path: [...prefix, command.name()], command }, ...everyCommand(command, [...prefix, command.name()])])
}

const group = (command: Command) => command.helpGroup().replace(/:$/, '').toLowerCase()

describe('the help is JSON, made from the commands themselves', () => {
    test('`zega`, `zega --help` and `zega --human --help` are the same JSON that JSON.parse takes, with no mark when piped', async () => {
        const [bare, long, human] = await Promise.all([run([], { home }), run(['--help'], { home }), run(['--help', '--human'], { home })])
        for (const result of [bare, long, human]) {
            expect(result.code).toBe(0)
            expect(result.stderr).toBe('')
            expect(result.stdout).not.toContain('▐')
            expect(result.stdout).not.toContain('\u001b')
            expect(result.stdout.startsWith('{\n  "name": "zega",\n')).toBe(true) // pretty-printed, 2 spaces
            expect(result.stdout.endsWith('}\n')).toBe(true)
        }
        expect(bare.stdout).toBe(long.stdout)
        expect(human.stdout).toBe(long.stdout)
        const help = JSON.parse(long.stdout)
        expect(help).toMatchObject({ name: 'zega', description: 'build, run and deploy with Zega' })
        expect(Object.keys(help)).toEqual(['name', 'version', 'description', 'usage', 'commands', 'options'])
    })

    test('it lists every command commander has, under its group, with its own description; a new command cannot be missed', async () => {
        const help = JSON.parse((await run(['--help'], { home })).stdout) as {
            commands: Record<string, Record<string, string>>
            options: Record<string, string>
        }
        const top = program.commands
        expect(top.length).toBeGreaterThan(10)
        for (const command of top) {
            expect(help.commands[group(command)]?.[command.name()]).toBe(command.description())
        }
        // Nothing is listed that is not a command, and the groups are the ones the code names.
        const listed = Object.values(help.commands).flatMap((names) => Object.keys(names))
        expect(listed.sort()).toEqual(top.map((command) => command.name()).sort())
        expect(Object.keys(help.commands)).toEqual(['local', 'project', 'cloud', 'account'])
        // The global options, with --human in them.
        expect(help.options['--human']).toBe('readable output for people')
        for (const option of program.options) expect(help.options[option.flags]).toBeDefined()
    })

    test('a command\'s help has its description, usage, arguments and every option it registered', async () => {
        for (const { path, command } of everyCommand()) {
            const expected = helpObject(command, command.createHelp()) as {
                command: string
                description: string
                usage: string
                arguments?: Record<string, string>
                options: Record<string, string>
            }
            // `zega start` hands --help to the server, so for it the object is read from the code; the rest are run.
            const printed =
                path[0] === 'start'
                    ? expected
                    : (JSON.parse((await run([...path, '--help'], { home })).stdout) as typeof expected)
            expect(printed).toEqual(expected)
            expect(printed.command).toBe(command.name())
            expect(printed.usage).toStartWith(`zega ${path.join(' ')}`)
            for (const option of command.options) expect(printed.options[option.flags]).toBeDefined()
            for (const argument of command.registeredArguments) expect(printed.arguments?.[argument.name() + (argument.variadic ? '...' : '')]).toBeDefined()
        }
}, 30_000)

    test('dev lists its flags with their words; function lists its subcommands; the global options are in every command\'s help', async () => {
        const dev = JSON.parse((await run(['dev', '--help'], { home })).stdout)
        expect(dev.options['--port <n>']).toBe('first port (default 8787)')
        expect(dev.options['--no-watch']).toBe("don't reload on changes")
        expect(dev.arguments).toEqual({ id: 'only this function' })
        const fn = JSON.parse((await run(['function', '--help'], { home })).stdout)
        expect(fn.commands).toEqual({ deploy: 'deploy one function', logs: "show a function's recent requests and logs" })
        for (const help of [dev, fn]) expect(Object.keys(help.globalOptions)).toEqual(['--api <url>', '--token-file <path>', '--human', '--nopretty'])
    })

    test('an unknown command or flag is the JSON error, pointing at the help; --human says it as text', async () => {
        const unknown = await run(['frobnicate'], { home })
        expect(unknown.code).toBe(1)
        expect(unknown.stdout).toBe('')
        expect(errorOf(unknown)).toMatchObject({ code: 'usage', hint: 'zega --help' })
        expect(errorOf(unknown).message).toContain("unknown command 'frobnicate'")
        const flag = await run(['whoami', '--bogus'], { home })
        expect(flag.code).toBe(1)
        expect(errorOf(flag)).toMatchObject({ code: 'usage', message: "unknown option '--bogus'", hint: 'zega --help' })
        const text = await run(['whoami', '--bogus', '--human'], { home })
        expect(text.stderr).toBe("error: unknown option '--bogus'\nhint: zega --help\n")
    })

    test('the colour keeps what JSON.parse needs: stripped of it, it is the same text', () => {
        const text = JSON.stringify(helpObject(program, program.createHelp()), null, 2)
        expect(paint(text).replace(/\u001b\[[0-9;]*m/g, '')).toBe(text)
    })
})
