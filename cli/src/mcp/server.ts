/**
 * The MCP server: which tools it offers, and the loop that serves them on stdio.
 *
 * What is offered is decided once, at start, and only ever narrows: a tool is there when its group is on and, if it
 * changes anything, the server is not `--read-only` and (for a change in Zega Cloud) the token is `manage`. A tool
 * that is not offered is not in `tools/list` and cannot be called, whatever a client sends.
 *
 * stdout belongs to the protocol. The commands the tools reuse print through `capture` (see `utils/output.ts`); anything
 * else that tries to write on stdout (a stray `console.log`) is sent to stderr instead, so no byte of it can break the stream.
 */
import { Writable } from 'node:stream'

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js'

import pkg from '../../package.json'
import { GROUPS, type Group } from '@/mcp/groups'
import { answer, type Context, type Tool } from '@/mcp/tool'
import { TOOLS } from '@/mcp/tools'

export type Mode = { groups: readonly Group[]; readOnly: boolean }

/**
 * The tools a server with this mode and this token offers. A read token is `--read-only`, whatever was asked: it
 * offers only tools that read. Otherwise a tool that changes something is offered unless `--read-only`, and one that
 * changes Zega Cloud only to a manage token.
 */
export function selectTools(mode: Mode, scope: 'read' | 'manage' | null, tools: Record<Group, Tool[]> = TOOLS): Tool[] {
    const readOnly = mode.readOnly || scope === 'read'
    return GROUPS.filter((group) => mode.groups.includes(group))
        .flatMap((group) => tools[group])
        .filter((tool) => !tool.writes || (!readOnly && (!tool.manage || scope === 'manage')))
}

/** What the server tells a client about itself. */
export function instructions(mode: Mode, scope: 'read' | 'manage' | null): string {
    const lines = [
        'Zega: an embeddable graph database, and Zega Cloud, which hosts graphs, functions and storage. These tools use the same code as the `zega` command.',
        `Groups: ${mode.groups.join(', ')}.`,
        mode.readOnly
            ? 'This server is read-only: nothing it offers changes anything.'
            : scope === 'read'
              ? 'The API token is a read token: nothing it offers changes anything.'
              : 'Tools that change something say so in their description; look with the read tools first.',
        'Search the docs (zega_docs_search) before writing ZQL.',
    ]
    return lines.join('\n')
}

export function buildServer(tools: Tool[], mode: Mode, context: Context): McpServer {
    const server = new McpServer({ name: 'zega', version: pkg.version }, { instructions: instructions(mode, context.scope) })
    for (const tool of tools) {
        server.registerTool(
            tool.name,
            {
                title: tool.title,
                description: tool.description,
                inputSchema: tool.input,
                annotations: { readOnlyHint: !tool.writes, destructiveHint: tool.writes, openWorldHint: tool.group !== 'docs' },
            },
            async (args: Record<string, unknown>) => {
                const { text, error } = await answer(tool, args, context)
                return { content: [{ type: 'text' as const, text }], ...(error ? { isError: true } : {}) }
            },
        )
    }
    return server
}

/** Serve on stdin and stdout until the client goes away (stdin ends, or a stop signal). Then clean up what this session made. */
export async function serve(server: McpServer, context: Context): Promise<void> {
    const protocolOut = process.stdout.write.bind(process.stdout)
    const out = new Writable({
        write(chunk, encoding, done) {
            protocolOut(chunk, encoding, done)
        },
    })
    const toStderr = ((chunk: string | Uint8Array, ...rest: unknown[]) => process.stderr.write(chunk, ...(rest as []))) as typeof process.stdout.write
    process.stdout.write = toStderr
    console.log = console.info = console.debug = console.error

    const transport = new StdioServerTransport(process.stdin, out)
    let stop!: () => void
    const stopped = new Promise<void>((resolve) => (stop = resolve))
    server.server.onclose = stop
    process.stdin.once('end', stop)
    process.stdin.once('close', stop)
    const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const
    for (const signal of signals) process.once(signal, stop)
    await server.connect(transport)
    await stopped
    for (const signal of signals) process.off(signal, stop)
    await server.close().catch(() => undefined)
    await context.local.stopAll().catch(() => undefined)
    const failed = await context.keys.revokeAll()
    if (failed.length > 0) {
        process.stderr.write(`zega mcp: could not revoke ${failed.length} key(s) it made (${failed.join(', ')}); revoke them in the dashboard\n`)
    }
}
