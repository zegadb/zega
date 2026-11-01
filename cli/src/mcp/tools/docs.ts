import { z } from 'zod'

import { PAGES } from '@/docs/pages'
import { pageNames, search, sectionsOf, titleOf } from '@/handlers/docs'
import { LocalFailure } from '@/utils/errors'
import { json } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'

export const docs = [
    tool({
        ...read,
        name: 'zega_docs_search',
        group: 'docs',
        title: 'Search the Zega docs',
        description:
            'Search the ZQL and Zega documentation that ships with this CLI (schema, queries, mutations, relationships, time, vectors, files, errors, the .graph format). Returns the best sections with their page and heading; read a whole page with zega_docs_read. Offline: it is the documentation of the version of zega this CLI was built with.',
        input: {
            query: z.string().min(1).describe('Words to look for, e.g. "mutation connect relationship".'),
            limit: z.number().int().min(1).max(20).optional().describe('How many sections (default 5).'),
        },
        run: async ({ query, limit }) => json({ results: search(query, limit ?? 5) }),
    }),
    tool({
        ...read,
        name: 'zega_docs_read',
        group: 'docs',
        title: 'Read a Zega docs page',
        description: 'The text of one documentation page, or with no page the list of pages and their titles. A page name is what zega_docs_search returns (`query`, `schema`, `mutation`, ...).',
        input: { page: z.string().optional().describe('A page name. Leave out to list the pages.') },
        run: async ({ page }) => {
            if (page === undefined) return json({ pages: pageNames().map((name) => ({ page: name, title: titleOf(name) })) })
            const name = page.replace(/\.md$/, '')
            const text = Object.hasOwn(PAGES, name) ? PAGES[name] : undefined
            if (text === undefined) throw new LocalFailure(`no page "${page}": the pages are ${pageNames().join(', ')}`, 'not_found')
            return json({ page: name, title: titleOf(name, text), sections: sectionsOf(name, text).map((section) => section.heading), text })
        },
    }),
]
