/**
 * How JSON is written, in one place: the only place that looks at whether a stream is a terminal to choose
 * the shape. On a terminal a value is pretty-printed (2 spaces) and coloured as an editor shows a package.json;
 * piped it is one compact line. `--nopretty` forces the compact line on a terminal too. A stream of events
 * (`event`) is always one compact line each, so that a reader that goes line by line never breaks.
 */
import chalk from 'chalk'

/** Colour: keys, strings, numbers and punctuation each in a muted colour of their own. Only JSON text goes through it. */
export function paint(text: string): string {
    const token = /("(?:[^"\\]|\\.)*")(\s*:)?|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null|[{}[\],:]/g
    return text.replace(token, (found: string, string: string | undefined, colon: string | undefined) => {
        if (string !== undefined) return colon === undefined ? chalk.green(string) : chalk.cyan(string) + chalk.dim(colon)
        return /^[{}[\],:]$/.test(found) ? chalk.dim(found) : chalk.yellow(found)
    })
}

export type FormatOptions = {
    /** Where it will be written. */
    stream: 'stdout' | 'stderr'
    /** `--nopretty`: one compact line, uncoloured, on a terminal too. */
    nopretty?: boolean
    /** Pretty even when piped (the help): colour still follows the terminal. */
    always?: boolean
}

/** A value as the text to print, newline included. */
export function formatJson(value: unknown, options: FormatOptions): string {
    const terminal = process[options.stream].isTTY === true && options.nopretty !== true
    if (!terminal && options.always !== true) return JSON.stringify(value) + '\n'
    const text = JSON.stringify(value, null, 2)
    return (terminal ? paint(text) : text) + '\n'
}

/** One event of a command that keeps running: always one compact line. */
export function eventLine(value: unknown): string {
    return JSON.stringify(value) + '\n'
}
