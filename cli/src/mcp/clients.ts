/**
 * How to add `zega mcp` to an agent: the one place that knows each client's configuration format, so `zega mcp config`
 * and the dashboard's "Connect your agent" panel print the same thing. This file imports nothing: copy it as it is.
 *
 * What each client takes (checked against its documentation, 3 Oct 2026):
 *
 *   claude-code  `claude mcp add --scope project <name> -- <command> [args]`; the project file `.mcp.json` is
 *                `{ "mcpServers": { <name>: { "type": "stdio", "command", "args" } } }` (an entry with no `type` is read as stdio too).
 *   codex        `codex mcp add <name> -- <command> [args]`; `~/.codex/config.toml` (or `.codex/config.toml` in a trusted
 *                project) has a `[mcp_servers.<name>]` table with `command` and `args`.
 *   cursor       `.cursor/mcp.json` (project) or `~/.cursor/mcp.json`: `{ "mcpServers": { <name>: { "command", "args" } } }`.
 *   vscode       `.vscode/mcp.json`: `{ "servers": { <name>: { "type": "stdio", "command", "args" } } }`; or
 *                `code --add-mcp '{"name":<name>,"command":...,"args":[...]}'`.
 *   windsurf     `~/.codeium/windsurf/mcp_config.json`: `{ "mcpServers": { <name>: { "command", "args" } } }`. (Windsurf's own
 *                documentation page now redirects to Devin's, which names `~/.config/devin/mcp_config.json`; this path is from
 *                Windsurf's long-standing layout and its guides. Recheck it when the page settles.)
 *   zed          `settings.json` (`zed: open settings file`): `{ "context_servers": { <name>: { "command", "args", "env" } } }`.
 *
 * No snippet ever holds a token: the server reads the one `zega login` stored, or the file `--token-file` names.
 */

export const CLIENTS = ['claude-code', 'codex', 'cursor', 'vscode', 'windsurf', 'zed'] as const
export type Client = (typeof CLIENTS)[number]

export type ConnectOptions = {
    client: Client
    /** The name the server has in the client (default `zega`). */
    name?: string
    /** `--read-only`: only the tools that read. */
    readOnly?: boolean
    /** `--groups`: which tool groups to serve; left out, the server's default. */
    groups?: string[]
    /** `--token-file`: where the server reads the token (default: the one `zega login` stored). */
    tokenFile?: string
    /** The command that starts zega: one word (`zega`) or several (`npx`, `zegadb`). Default `zega`. */
    command?: string[]
}

export type Snippet = {
    /** `command`: run it in a shell. `json` and `toml`: add it to the file named in `where`. */
    kind: 'command' | 'json' | 'toml'
    /** What it is, for a heading. */
    label: string
    /** The file to put it in (a JSON or TOML snippet), or where to run the command. */
    where: string
    text: string
}

export type Connect = { client: Client; name: string; command: string[]; snippets: Snippet[] }

/** The arguments of the server: `mcp`, then what the options ask for. */
export function serverArgs(options: Pick<ConnectOptions, 'readOnly' | 'groups' | 'tokenFile'>): string[] {
    const args = ['mcp']
    if (options.groups !== undefined && options.groups.length > 0) args.push('--groups', options.groups.join(','))
    if (options.readOnly === true) args.push('--read-only')
    if (options.tokenFile !== undefined) args.push('--token-file', options.tokenFile)
    return args
}

/** A word as a POSIX shell takes it literally. */
export function shellWord(word: string): string {
    return /^[A-Za-z0-9_@%+=:,./-]+$/.test(word) ? word : `'${word.replace(/'/g, `'\\''`)}'`
}

const json = (value: unknown): string => JSON.stringify(value, null, 2)
const toml = (value: string): string => JSON.stringify(value)

export function connect(options: ConnectOptions): Connect {
    const name = options.name ?? 'zega'
    const start = options.command ?? ['zega']
    const [program = 'zega', ...front] = start
    const args = [...front, ...serverArgs(options)]
    const line = [program, ...args].map(shellWord).join(' ')
    const stdio = { type: 'stdio', command: program, args }
    const plain = { command: program, args }
    let snippets: Snippet[]
    switch (options.client) {
        case 'claude-code':
            snippets = [
                { kind: 'command', label: 'Add it to this project (shared in .mcp.json)', where: 'a terminal, in the project', text: `claude mcp add --scope project ${shellWord(name)} -- ${line}` },
                { kind: 'command', label: 'Or only for you, in every project', where: 'a terminal', text: `claude mcp add --scope user ${shellWord(name)} -- ${line}` },
                { kind: 'json', label: 'Or write the project file yourself', where: '.mcp.json', text: json({ mcpServers: { [name]: stdio } }) },
            ]
            break
        case 'codex':
            snippets = [
                { kind: 'command', label: 'Add it to Codex', where: 'a terminal', text: `codex mcp add ${shellWord(name)} -- ${line}` },
                {
                    kind: 'toml',
                    label: 'Or write the config yourself',
                    where: '~/.codex/config.toml (or .codex/config.toml in a trusted project)',
                    text: `[mcp_servers.${/^[A-Za-z0-9_-]+$/.test(name) ? name : toml(name)}]\ncommand = ${toml(program)}\nargs = [${args.map(toml).join(', ')}]`,
                },
            ]
            break
        case 'cursor':
            snippets = [{ kind: 'json', label: 'Add it to Cursor', where: '.cursor/mcp.json in the project, or ~/.cursor/mcp.json for every project', text: json({ mcpServers: { [name]: plain } }) }]
            break
        case 'vscode':
            snippets = [
                { kind: 'json', label: 'Add it to this workspace', where: '.vscode/mcp.json', text: json({ servers: { [name]: stdio } }) },
                { kind: 'command', label: 'Or add it to your user profile', where: 'a terminal (POSIX shell quoting; in PowerShell write the JSON in double quotes)', text: `code --add-mcp ${shellWord(JSON.stringify({ name, command: program, args }))}` },
            ]
            break
        case 'windsurf':
            snippets = [{ kind: 'json', label: 'Add it to Windsurf', where: '~/.codeium/windsurf/mcp_config.json (%USERPROFILE%\\.codeium\\windsurf\\mcp_config.json on Windows)', text: json({ mcpServers: { [name]: plain } }) }]
            break
        case 'zed':
            snippets = [{ kind: 'json', label: 'Add it to Zed', where: 'settings.json (run `zed: open settings file`)', text: json({ context_servers: { [name]: { ...plain, env: {} } } }) }]
            break
    }
    return { client: options.client, name, command: [program, ...args], snippets }
}
