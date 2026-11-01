import path from 'node:path'

import { openSession, credentials, type Globals } from '@/handlers'
import { CLIENTS, connect, type Client, type Connect } from '@/mcp/clients'
import { CLOUD_GROUPS, DEFAULT_GROUPS, parseGroups } from '@/mcp/groups'
import { KeyRing } from '@/mcp/keys'
import { LocalServers } from '@/mcp/local_servers'
import { buildServer, selectTools, serve } from '@/mcp/server'
import type { Context } from '@/mcp/tool'
import { LocalFailure } from '@/utils/errors'
import { result, write } from '@/utils'

export type McpOptions = Globals & {
    groups?: string
    readOnly?: boolean
    graphKeyFile?: string
    storageKeyFile?: string
    /** Another zega-server build, for the local group (development). */
    server?: string
}

/**
 * `zega mcp`: an MCP server on stdio for agents. It uses the stored token (or `--token-file`) like any command; its
 * tools are the commands' own code. Returns when the client goes away.
 */
export async function mcp(options: McpOptions): Promise<void> {
    const groups = options.groups === undefined ? [...DEFAULT_GROUPS] : parseGroups(options.groups)
    const readOnly = options.readOnly === true
    const cwd = process.cwd()

    let scope: 'read' | 'manage' | null = null
    const secrets: string[] = []
    if (groups.some((group) => CLOUD_GROUPS.includes(group))) {
        // Fail at the start, with the usual error, rather than on every call: the token's scope decides what is offered.
        const { client } = await openSession(options)
        scope = (await client.get('/v1/whoami')).json.token.scope
        const token = options.tokenFile !== undefined ? credentials.readTokenFile(options.tokenFile) : (await credentials.load())?.token
        if (token !== undefined) secrets.push(token)
    }
    const mayMake = !readOnly && scope === 'manage'
    const session = () => openSession(options)
    const keys = new KeyRing({
        session,
        cwd,
        mayMake,
        whyNot: readOnly ? '--read-only does not create keys' : 'the API token is a read token and making a key needs a manage token',
        graphKeyFile: options.graphKeyFile,
        storageKeyFile: options.storageKeyFile,
        others: secrets,
    })
    const context: Context = { globals: options, scope, session, keys, local: new LocalServers(), cwd, serverPath: options.server }
    const mode = { groups, readOnly }
    await serve(buildServer(selectTools(mode, scope), mode, context), context)
}

export type ConfigOptions = Globals & {
    client?: string
    name?: string
    groups?: string
    readOnly?: boolean
    command?: string
}

/** `zega mcp config --client <client>`: the command or file entry that adds `zega mcp` to that agent. Prints; changes nothing. */
export async function mcpConfig(options: ConfigOptions): Promise<void> {
    if (options.client === undefined) {
        throw new LocalFailure(`--client is needed: ${CLIENTS.join(', ')}`, 'missing_flag')
    }
    if (!(CLIENTS as readonly string[]).includes(options.client)) {
        throw new LocalFailure(`--client: "${options.client}" is not one of ${CLIENTS.join(', ')}`, 'invalid_flag')
    }
    const groups = options.groups === undefined ? undefined : parseGroups(options.groups)
    const command = options.command === undefined ? undefined : options.command.split(/\s+/).filter((word) => word !== '')
    if (command !== undefined && command.length === 0) throw new LocalFailure('--command needs a command, e.g. zega or "npx zegadb"', 'invalid_flag')
    const done = connect({
        client: options.client as Client,
        name: options.name,
        groups,
        readOnly: options.readOnly,
        tokenFile: options.tokenFile === undefined ? undefined : path.resolve(options.tokenFile),
        command,
    })
    if (options.human === true) return await write('stdout', humanConnect(done))
    await result(options, done)
}

/** The snippets as text for a person: each with its heading and where it goes. */
function humanConnect(done: Connect): string {
    return (
        done.snippets
            .map((snippet) => `${snippet.label}  (${snippet.where})\n\n${snippet.text.replace(/^/gm, '  ')}\n`)
            .join('\n')
    )
}
