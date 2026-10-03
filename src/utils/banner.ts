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
    /** Draw the zega mark (default: yes). */
    mark?: boolean
}

const WIDTH = 44
const INDENT = '  '

/** The zega mark: five rows of blocks, `█` the body and `▐` the shaded left edge (the look of TAAG's "Cfh 3" in green). */
export const LOGO = [
    '▐██████▐██████▐██████▐██████',
    '   ▐██ ▐██    ▐██    ▐██ ▐██',
    '  ▐██  ▐████  ▐██ ▐██▐██████',
    ' ▐██   ▐██    ▐██  ▐█▐██ ▐██',
    '▐██████▐██████▐██████▐██ ▐██',
]

/** The mark's two greens in true colour (VGA bright green and green); 16-colour terminals get greenBright and green. */
const LOGO_MAIN = '#55FF55'
const LOGO_SHADE = '#00AA00'

/** The mark, coloured for this terminal (or plain), indented, without a trailing newline. */
export function logo(color?: boolean): string {
    const paint = color === undefined ? chalk : new Chalk({ level: color ? 3 : 0 })
    const main = paint.level >= 3 ? paint.hex(LOGO_MAIN) : paint.greenBright
    const shade = paint.level >= 3 ? paint.hex(LOGO_SHADE) : paint.green
    return LOGO.map((row) => INDENT + row.replace(/█+|▐+/g, (run) => (run.startsWith('█') ? main(run) : shade(run)))).join('\n')
}


export function startBanner(options: BannerOptions): string {
    const paint = options.color === undefined ? chalk : new Chalk({ level: options.color ? 1 : 0 })
    const labelWidth = Math.max(...options.lines.map((line) => line.label.length)) + 1
    const out: string[] = ['']
    if (options.mark !== false) out.push(logo(options.color), '')
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
