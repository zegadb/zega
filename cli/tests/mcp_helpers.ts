import fs from 'node:fs'
import path from 'node:path'

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'

import { TOKEN, scratch } from './helpers'

export const FAKE_QUERY_SERVER = path.resolve(import.meta.dir, 'fixtures', 'fake-query-server.ts')
const ENTRY = path.resolve(import.meta.dir, '..', 'src', 'zega.ts')

export type Connected = {
    client: Client
    stderr: () => string
    close: () => Promise<void>
}

/**
 * Start `zega mcp` as a subprocess and speak to it with the SDK's own client over stdio, as an agent does. The
 * environment is a PATH, a HOME and a config directory of its own, so nothing of the machine's `zega login` reaches it.
 * `command` runs the compiled executable instead of the TypeScript.
 */
export async function connectMcp(
    args: string[],
    options: { home: string; cwd?: string; api: string; token?: string; command?: string[] },
): Promise<Connected> {
    const tokenFile = path.join(options.home, 'token')
    fs.writeFileSync(tokenFile, (options.token ?? TOKEN) + '\n', { mode: 0o600 })
    const [command, ...front] = options.command ?? [process.execPath, ENTRY]
    const transport = new StdioClientTransport({
        command: command!,
        args: [...front, 'mcp', ...args, '--api', options.api, '--token-file', tokenFile],
        cwd: options.cwd ?? options.home,
        env: { PATH: process.env.PATH ?? '', HOME: options.home, XDG_CONFIG_HOME: path.join(options.home, '.config') },
        stderr: 'pipe',
    })
    let stderr = ''
    const client = new Client({ name: 'zega-test', version: '0' })
    await client.connect(transport)
    transport.stderr?.on('data', (chunk: Buffer) => (stderr += chunk.toString()))
    return { client, stderr: () => stderr, close: () => client.close() }
}

export async function toolNames(connected: Connected): Promise<string[]> {
    return (await connected.client.listTools()).tools.map((tool) => tool.name).sort()
}

/** The text a tool answered, and whether it was an error. */
export async function callTool(connected: Connected, name: string, args: Record<string, unknown> = {}): Promise<{ text: string; isError: boolean }> {
    const result = (await connected.client.callTool({ name, arguments: args })) as { content: { type: string; text?: string }[]; isError?: boolean }
    return { text: result.content.map((part) => part.text ?? '').join(''), isError: result.isError === true }
}

export const newHome = (): string => {
    const dir = scratch()
    fs.mkdirSync(path.join(dir, 'home'))
    return dir
}
