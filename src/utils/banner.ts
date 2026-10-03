/**
 * The start-up banner `zega start` prints on a terminal: the zega mark, a tagline, a rule, and one
 * line per thing that is running (or is not, with the command that would start it).
 *
 * This is the only place the banner is made. It has the shape of `startBanner` from `@zegadb/stdio`
 * (zegadb/stdio, being revived; not published yet): one call with the product's name, version and
 * tagline, and the lines to list. When that package is published, this file becomes a re-export of
 * it, and nothing that calls `startBanner` changes.
 */
import chalk, { Chalk } from 'chalk'

export type BannerLine = {
    /** `Local`, `Explorer`: what the line is. */
    label: string
    /** The address, or null when it is not running. */
    url: string | null
    /** Shown dim under a line that is not running: the command that starts it. */
    hint?: string
}

export type BannerOptions = {
    name: string
    version: string
    tagline: string
    lines: BannerLine[]
    /** Colour and the dim hint (default: whatever chalk finds for the terminal). */
    color?: boolean
}

const LOGO = [
    ' _____ _____  ____    _    ',
    '|__  /| ____|/ ___|  / \\   ',
    '  / / |  _| | |  _  / _ \\  ',
    ' / /_ | |___| |_| |/ ___ \\ ',
    '/____||_____|\\____/_/   \\_\\',
]

const WIDTH = 44
const INDENT = '  '

export function startBanner(options: BannerOptions): string {
    const paint = options.color === undefined ? chalk : new Chalk({ level: options.color ? 1 : 0 })
    const labelWidth = Math.max(...options.lines.map((line) => line.label.length)) + 1
    const out: string[] = ['']
    for (const row of LOGO) out.push(INDENT + paint.green(row))
    out.push('')
    out.push(`${INDENT}${options.tagline}  ${paint.dim(`${options.name} v${options.version}`)}`)
    out.push(INDENT + paint.dim('─'.repeat(WIDTH)))
    out.push('')
    for (const line of options.lines) {
        const label = `${line.label}:`.padEnd(labelWidth + 1)
        out.push(`${INDENT}${paint.green('➜')}  ${paint.bold(label)} ${line.url === null ? '(stopped)' : paint.cyan(line.url)}`)
        if (line.url === null && line.hint !== undefined) out.push(`${INDENT}     ${paint.dim(line.hint)}`)
    }
    out.push('')
    return out.join('\n') + '\n'
}
