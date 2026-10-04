import { z } from 'zod'

import * as core from '@/core'
import { commandGlobals, printed } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'

export const account = [
    tool({
        ...read,
        name: 'zega_whoami',
        group: 'account',
        title: 'Who the token is',
        description: 'Which Zega Cloud account and API token this server uses: the account, the token\'s name, its scope (read or manage), the project it is limited to and when it expires. Never shows the token itself.',
        input: {},
        run: async (_args, { globals }) => await printed(() => core.whoami(commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_projects',
        group: 'account',
        title: 'Projects',
        description: 'List the account\'s projects, or with an id show one project with its graphs.',
        input: { id: z.string().optional().describe('A project id. Leave out to list every project.') },
        run: async ({ id }, { globals }) => await printed(() => core.projects(id, commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_usage',
        group: 'account',
        title: 'Usage and bill',
        description: 'This month\'s usage and bill, per graph, for the account (or the project the token is limited to).',
        input: {},
        run: async (_args, { globals }) => await printed(() => core.usage(commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_regions',
        group: 'account',
        title: 'Regions',
        description: 'The regions a graph can run in (ids such as yyz or ams, with the city).',
        input: {},
        run: async (_args, { globals }) => await printed(() => core.regions(commandGlobals(globals))),
    }),
]
