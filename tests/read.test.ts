import { afterAll, describe, expect, test } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'

import type { components } from '@/types/cloud-api'
import { TOKEN, fakeApi, graph, project, regionsBody, remove, run, scratch, whoamiRoute, whoamiShown } from './helpers'

type Schemas = components['schemas']

const bucket = {
    id: 'b1',
    name: 'uploads',
    url: 'https://b1.zegadb.com',
    projectId: 'p1',
    projectName: 'Atlas',
    bytesStored: 1_234_567,
    capCents: 500,
    createdAt: '2026-09-21T08:00:00.000Z',
} satisfies Schemas['Bucket']

const fn = {
    id: 'f0bbbbbbbbbbbbbbbb',
    name: 'hello',
    projectId: 'p1',
    projectName: 'Atlas',
    url: 'https://f0bbbbbbbbbbbbbbbb.zegadb.com',
    capCents: 300,
    deployed: true,
    version: 3,
    etag: 'abc',
    deployedAt: '2026-10-01T09:00:00.000Z',
    vars: { GREETING: 'hi' },
    secrets: ['API_KEY'],
    limits: { cpuMs: 50, subRequests: 50 },
    logs: 'on',
    logsStoppedAt: null,
    createdAt: '2026-09-22T08:00:00.000Z',
    updatedAt: '2026-10-01T09:00:00.000Z',
} satisfies Schemas['Function']

const bill = { lines: [], totalCents: 450, worstCaseCents: 900, capReached: false, upfrontCents: 0 }

const usageBody = {
    month: '2026-10',
    toDateCents: 450,
    projectedCents: 1300,
    worstCaseCents: 2000,
    graphs: [
        {
            graph: graph.id,
            month: '2026-10',
            usage: { queries: 12000, writes: 340, storageGb: 0.25, egressGb: 1.5, days: [] },
            included: {},
            overage: { egressGb: { extra: 0, cents: 0 } },
            nodes: { count: 800, included: 1000, countedAt: null },
            capCents: 500,
            paused: false,
            writesPaused: false,
            toDate: bill,
            projected: { ...bill, totalCents: 1300 },
            projectedUsage: { queries: 0, writes: 0, storageGb: 0, egressGb: 0 },
        },
    ],
}

const logsBody = {
    function: fn.id,
    logs: 'on',
    from: '2026-10-02T00:00:00.000Z',
    to: '2026-10-03T00:00:00.000Z',
    retentionDays: 7,
    entries: [
        {
            id: 'r1',
            time: '2026-10-02T12:00:00.000Z',
            method: 'GET',
            path: '/hello',
            status: 500,
            durationMs: 12,
            cpuMs: 3,
            outcome: 'exception',
            exception: 'TypeError: boom',
            logs: [{ time: '2026-10-02T12:00:00.001Z', level: 'error', message: 'first line\nsecond line' }],
            logsTruncated: false,
        },
    ],
    cursor: 'next-page',
}

const routes = {
    ...whoamiRoute,
    'GET /v1/projects': { body: { projects: [project, { ...project, id: 'p2', name: 'Two', graphs: [] }] } },
    'GET /v1/projects/p1': { body: { project } },
    'GET /v1/graphs': { body: { graphs: [graph] } },
    'GET /v1/graphs/g0aaaaaaaaaaaaaaaa': { body: { graph } },
    'GET /v1/buckets': { body: { buckets: [bucket] } },
    'GET /v1/buckets/b1': { body: { bucket } },
    'GET /v1/functions': { body: { functions: [fn] } },
    'GET /v1/functions/f0bbbbbbbbbbbbbbbb': { body: { function: fn } },
    'GET /v1/usage': { body: usageBody },
    'GET /v1/regions': { body: regionsBody },
    'GET /v1/functions/f0bbbbbbbbbbbbbbbb/logs': { body: logsBody },
}

const api = fakeApi(routes)
const home = scratch()
const tokenFile = path.join(home, 'token')
fs.writeFileSync(tokenFile, TOKEN + '\n', { mode: 0o600 })
afterAll(() => {
    api.stop()
    remove(home)
})

const cli = (args: string[]) => run([...args, '--api', api.url, '--token-file', tokenFile], { home })
/** The same with `--human`: the tables people read. */
const human = (args: string[]) => cli([...args, '--human'])

describe('read commands', () => {
    test('--human: an aligned table, with the token from --token-file and nothing stored', async () => {
        const result = await human(['projects'])
        expect(result.code).toBe(0)
        expect(result.stdout).toBe(
            [
                'ID  NAME   GRAPHS  CREATED',
                'p1  Atlas       1  2026-09-20T12:00:00.000Z',
                'p2  Two         0  2026-09-20T12:00:00.000Z',
                '',
            ].join('\n'),
        )
        expect(api.seen.at(-1)).toMatchObject({ method: 'GET', target: '/v1/projects', authorization: `Bearer ${TOKEN}` })
        expect(fs.existsSync(path.join(home, '.config'))).toBe(false)
    })

    test('by default the API answer is printed byte for byte, with no table', async () => {
        const result = await cli(['projects'])
        expect(result.code).toBe(0)
        expect(result.stdout).toBe(JSON.stringify(routes['GET /v1/projects'].body) + '\n')
    })

    test('projects <id> shows the project and its graphs', async () => {
        const result = await human(['projects', 'p1'])
        expect(result.stdout).toContain('Name     Atlas')
        expect(result.stdout).toContain('g0aaaaaaaaaaaaaaaa  main graph  Atlas (p1)  yyz     512mb  active')
        expect(api.seen.at(-1)?.target).toBe('/v1/projects/p1')
    })

    test('graphs, graphs <id>, buckets, buckets <id>, functions, functions <id>', async () => {
        const graphs = await human(['graphs'])
        expect(graphs.stdout.split('\n')[0]).toBe('ID                  NAME        PROJECT     REGION  TIER   STATUS')
        const graphOne = await human(['graphs', graph.id])
        expect(graphOne.stdout).toContain('Tier     512mb (512 MB)')
        expect(graphOne.stdout).toContain('Billing  monthly, $12.00 base price')
        expect(graphOne.stdout).toContain('Address  https://g0aaaaaaaaaaaaaaaa.zegadb.com')

        const buckets = await human(['buckets'])
        expect(buckets.stdout).toContain('1.2 MB')
        const bucketOne = await human(['buckets', 'b1'])
        expect(bucketOne.stdout).toContain('Cap      $5.00 of overage a month (changed in the dashboard)')

        const functions = await human(['functions'])
        expect(functions.stdout).toContain('v3')
        const functionOne = await human(['functions', fn.id])
        expect(functionOne.stdout).toContain('GREETING=hi')
        expect(functionOne.stdout).toContain('Secrets (names only; values are not readable)')
        expect(functionOne.stdout).toContain('  API_KEY')
        expect(functionOne.stdout).toContain('50 ms CPU, 50 subrequests per call')
    })

    test('usage and regions', async () => {
        const usage = await human(['usage'])
        expect(usage.stdout).toContain('To date     $4.50')
        expect(usage.stdout).toContain('Worst case  $20.00')
        expect(usage.stdout).toContain('800/1000')
        const regions = await human(['regions'])
        expect(regions.stdout).toBe(
            ['ID   CITY       COUNTRY      CONTINENT', 'yyz  Toronto    Canada       North America', 'ams  Amsterdam  Netherlands  Europe', ''].join('\n'),
        )
    })

    test('every read command prints the API text unchanged by default', async () => {
        const commands: [string[], keyof typeof routes][] = [
            [['whoami'], 'GET /v1/whoami'],
            [['graphs'], 'GET /v1/graphs'],
            [['graphs', graph.id], 'GET /v1/graphs/g0aaaaaaaaaaaaaaaa'],
            [['buckets'], 'GET /v1/buckets'],
            [['functions', fn.id], 'GET /v1/functions/f0bbbbbbbbbbbbbbbb'],
            [['usage'], 'GET /v1/usage'],
            [['regions'], 'GET /v1/regions'],
            [['function', 'logs', fn.id], 'GET /v1/functions/f0bbbbbbbbbbbbbbbb/logs'],
        ]
        for (const [args, route] of commands) {
            const result = await cli(args)
            expect(result.code).toBe(0)
            // whoami is the API's answer without the token's prefix; everything else is byte for byte.
            const shown = route === 'GET /v1/whoami' ? whoamiShown : routes[route].body
            expect(result.stdout).toBe(JSON.stringify(shown) + '\n')
        }
    })

    test('function logs: the filters are sent as the API names them, entries are printed with their console lines', async () => {
        const result = await human([
            'function', 'logs', fn.id,
            '--limit', '20', '--status', '500', '--search', 'boom', '--from', '2026-10-02T00:00:00Z', '--cursor', 'abc',
        ])
        expect(result.code).toBe(0)
        const url = new URL('http://x' + api.seen.at(-1)?.target)
        expect(url.pathname).toBe(`/v1/functions/${fn.id}/logs`)
        expect(Object.fromEntries(url.searchParams)).toEqual({
            limit: '20', status: '500', q: 'boom', from: '2026-10-02T00:00:00Z', cursor: 'abc',
        })
        expect(result.stdout).toContain('GET /hello  500  12 ms  exception')
        expect(result.stdout).toContain('! TypeError: boom')
        expect(result.stdout).toContain('    error  first line\n        second line')
        expect(result.stdout).toContain('more: pass --cursor next-page for the next page')
    })

    test('function logs: a limit outside 1 to 200 is refused before any request', async () => {
        const before = api.seen.length
        const result = await cli(['function', 'logs', fn.id, '--limit', '500'])
        expect(result.code).toBe(1)
        expect(result.stderr).toContain('--limit is a whole number from 1 to 200')
        expect(api.seen).toHaveLength(before)
    })

    test('an id typed with a slash stays one path segment', async () => {
        await cli(['projects', 'a/b'])
        expect(api.seen.at(-1)?.pathname).toBe('/v1/projects/a%2Fb')
    })

    test('control characters in an API string never reach the terminal', async () => {
        const nasty = fakeApi({ 'GET /v1/projects': { body: { projects: [{ ...project, name: 'evil\u001b[31mname', graphs: [] }] } } })
        try {
            const result = await run(['projects', '--human', '--api', nasty.url, '--token-file', tokenFile], { home })
            expect(result.stdout).not.toContain('\u001b')
            expect(result.stdout).toContain('evil?[31mname')
        } finally {
            nasty.stop()
        }
    })

    test('an answer that is not the API\'s is an error, not an empty list', async () => {
        const wrong = fakeApi({ 'GET /v1/projects': { body: { hello: 'world' } } })
        try {
            // The default prints the answer as it came; it is the readable view that needs the list.
            const result = await run(['projects', '--human', '--api', wrong.url, '--token-file', tokenFile], { home })
            expect(result.code).toBe(1)
            expect(result.stderr).toContain('no `projects` list')
        } finally {
            wrong.stop()
        }
    })
})
