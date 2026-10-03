/**
 * The help, as pretty-printed JSON in the shape of a package.json. It is made from commander's own
 * definitions (the commands, arguments and options registered in `program.ts`), so a command added
 * there is in the help and nothing is written twice. `zega` and `zega --help` describe the whole CLI
 * (the commands grouped as the code groups them); `zega <command> --help` describes one command.
 */
import chalk from 'chalk'
import type { Command, Help } from 'commander'

type Described = Record<string, string>

/** `Local:` becomes `local`. */
function groupKey(heading: string): string {
    return heading.replace(/:$/, '').trim().toLowerCase() || 'other'
}

function optionsOf(helper: Help, options: ReturnType<Help['visibleOptions']>): Described {
    return Object.fromEntries(options.map((option) => [option.flags, helper.optionDescription(option)]))
}

/** The help of this command, as the object that is printed. */
export function helpObject(cmd: Command, helper: Help): Record<string, unknown> {
    const commands = helper.visibleCommands(cmd)
    const options = optionsOf(helper, helper.visibleOptions(cmd))
    if (cmd.parent === null) {
        const groups: Record<string, Described> = {}
        for (const command of commands) {
            const key = groupKey(command.helpGroup())
            ;(groups[key] ??= {})[command.name()] = helper.subcommandDescription(command)
        }
        return {
            name: cmd.name(),
            version: cmd.version(),
            description: cmd.description(),
            usage: helper.commandUsage(cmd),
            commands: groups,
            options,
        }
    }
    const out: Record<string, unknown> = {
        command: cmd.name(),
        description: helper.commandDescription(cmd),
        usage: helper.commandUsage(cmd),
    }
    const args = helper.visibleArguments(cmd)
    if (args.length > 0) {
        out.arguments = Object.fromEntries(args.map((arg) => [arg.name() + (arg.variadic ? '...' : ''), helper.argumentDescription(arg)]))
    }
    if (commands.length > 0) {
        out.commands = Object.fromEntries(commands.map((command) => [command.name(), helper.subcommandDescription(command)]))
    }
    out.options = options
    const globals = helper.visibleGlobalOptions(cmd).filter((option) => !option.long?.match(/^--(help|version)$/))
    if (globals.length > 0) out.globalOptions = optionsOf(helper, globals)
    return out
}

/**
 * Colour as an editor shows a package.json: keys, strings and punctuation each in a muted colour of
 * their own. Only the pretty-printed text this file makes goes through it (strings and braces).
 */
export function paint(text: string): string {
    return text.replace(/("(?:[^"\\]|\\.)*")(\s*:)?|[{}[\],:]/g, (token, string: string | undefined, colon: string | undefined) => {
        if (string === undefined) return chalk.dim(token)
        return colon === undefined ? chalk.green(string) : chalk.cyan(string) + chalk.dim(colon)
    })
}

/** What `--help` prints: 2-space pretty JSON, coloured on a terminal and plain (what `JSON.parse` takes) anywhere else. */
export function helpJson(cmd: Command, helper: Help): string {
    const text = JSON.stringify(helpObject(cmd, helper), null, 2)
    return (process.stdout.isTTY === true ? paint(text) : text) + '\n'
}
