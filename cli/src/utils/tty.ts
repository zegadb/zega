/**
 * What a command may print and ask. The default is JSON and no questions: the CLI is for agents
 * first. `--human` is the one switch to readable output, and the terminal only matters inside it:
 * questions need a terminal on both ends, the logo (and colour, which chalk leaves to the terminal)
 * needs one on stdout. Nothing here, or anywhere, picks JSON or text from whether a terminal is attached.
 */
export type Mode = {
    human?: boolean
    /** `--nopretty`: compact JSON even on a terminal. */
    nopretty?: boolean
}

/** `--human`: tables, banners, readable errors and, on a terminal, questions. */
export function isHuman(globals: Mode): boolean {
    return globals.human === true
}

/**
 * Whether a command may ask questions: only with `--human`, with a terminal on both ends. A command that is
 * missing an answer otherwise fails at once, naming the flag that carries it.
 */
export function canPrompt(globals: Mode): boolean {
    return isHuman(globals) && process.stdin.isTTY === true && process.stdout.isTTY === true
}

/** Whether to draw the zega mark: `--human` on a terminal. */
export function showLogo(globals: Mode): boolean {
    return isHuman(globals) && process.stdout.isTTY === true
}
