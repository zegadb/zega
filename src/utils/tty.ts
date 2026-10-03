/**
 * Whether a command may ask questions. It may only with a terminal on both
 * ends and not in `--json` mode: an agent, a CI job or a pipe is never asked
 * anything. A command that is missing an answer then fails at once, naming the
 * flag that carries it.
 */
export function canPrompt(globals: { json?: boolean }): boolean {
    return globals.json !== true && process.stdin.isTTY === true && process.stdout.isTTY === true
}
