import fs from 'node:fs'
import path from 'node:path'

import { z } from 'zod'

import * as core from '@/core'
import * as projectConfig from '@/handlers/project_config'
import { commandGlobals, json, printed } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'
import { LocalFailure } from '@/utils/errors'

/** The path of an entry file, which must be inside the project: a function is bundled from the project's own files. */
function insideProject(entry: string, cwd: string): string {
    const root = fs.realpathSync(cwd)
    const resolved = path.resolve(root, entry)
    const real = fs.existsSync(resolved) ? fs.realpathSync(resolved) : resolved
    const relative = path.relative(root, real)
    if (relative === '' || relative.startsWith('..') || path.isAbsolute(relative)) {
        throw new LocalFailure(`${entry} is outside the project (${cwd}): a function is deployed from files inside it`, 'outside_project')
    }
    return entry
}

export const functions = [
    tool({
        ...read,
        name: 'zega_functions',
        group: 'functions',
        title: 'Functions',
        description: 'List the account\'s functions, or with an id show one: its address, version, variables, secret names (never values), limits and logging.',
        input: { id: z.string().optional().describe('A function id (f0 and 16 characters). Leave out to list every function.') },
        run: async ({ id }, { globals }) => await printed(() => core.functions(id, commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_function_code',
        group: 'functions',
        title: 'A function\'s deployed code',
        description: 'The module that is deployed for a function (null before its first deploy).',
        input: { id: z.string().describe('A function id.') },
        run: async ({ id }, { session }) => json((await (await session()).client.get('/v1/functions/{function}/code', { path: { function: id } })).json),
    }),
    tool({
        ...read,
        name: 'zega_function_logs',
        group: 'functions',
        title: 'A function\'s logs',
        description: 'A page of a function\'s requests, newest first, with their console lines. The function must have logs on. Pass `cursor` from the last answer for the next page.',
        input: {
            id: z.string().describe('A function id.'),
            from: z.string().optional().describe('From this ISO time (default 24 hours ago).'),
            to: z.string().optional().describe('Until this time (default now).'),
            limit: z.number().int().min(1).max(200).optional().describe('How many requests (default 50).'),
            status: z.number().int().min(100).max(599).optional().describe('Only this HTTP status.'),
            search: z.string().optional().describe('Search the requests and log lines.'),
            cursor: z.string().optional().describe('The next page, from the last answer.'),
        },
        run: async ({ id, ...rest }, { globals }) => await printed(() => core.functionLogs(id, { ...commandGlobals(globals), ...rest })),
    }),
    tool({
        writes: true,
        manage: true,
        name: 'zega_function_deploy',
        group: 'functions',
        title: 'Deploy a function from a file',
        description:
            'Bundle a function\'s entry file (TypeScript or JavaScript, with its npm dependencies) into one ES module and upload it: it is live at once and replaces the deployed version. `entry` is a path inside the project; leave it out to use the one zega.json lists for this id. Same as `zega function deploy`.',
        input: {
            id: z.string().describe('A function id (made in the dashboard).'),
            entry: z.string().optional().describe('Source file, relative to the project (default: from zega.json).'),
            minify: z.boolean().optional().describe('Minify the bundle.'),
        },
        run: async ({ id, entry, minify }, { globals, cwd }) => {
            const listed = entry ?? projectConfig.read(cwd)?.functions?.find((item) => item.id === id)?.entry
            if (listed !== undefined) insideProject(listed, cwd)
            return await printed(() => core.functionDeploy(id, entry, { ...commandGlobals(globals), minify }))
        },
    }),
]
