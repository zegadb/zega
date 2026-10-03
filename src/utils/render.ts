/**
 * Human-readable output: aligned tables and detail views built from the API's
 * JSON (`--json` skips all of this and prints the API's own text).
 *
 * Every string that came from the API is cleaned before it reaches the
 * terminal: a project can be named anything, and a control character in a name
 * must not drive the terminal. The shapes come from the OpenAPI document
 * (`src/types/cloud-api.d.ts`), so a renamed field is a type error here.
 */
import type { components } from '@/types/cloud-api'
import type { Answer } from '@/utils/cloud_request'
import { LocalFailure, clean } from '@/utils/errors'
import { NONE, Table, detail } from '@/utils/table'

type Schemas = components['schemas']

/** A field's text: strings as they are, numbers and booleans written out, null or missing as `-`. */
function text(value: string | number | boolean | null | undefined): string {
    if (value === null || value === undefined) return NONE
    return typeof value === 'string' ? clean(value) : String(value)
}

function cents(value: number | null | undefined): string {
    if (value === null || value === undefined) return NONE
    const abs = Math.abs(value)
    return `${value < 0 ? '-' : ''}$${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, '0')}`
}

/** Decimal units, like the API's own gigabytes. */
export function bytes(count: number): string {
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let size = count
    let unit = 0
    while (size >= 1000 && unit < units.length - 1) {
        size /= 1000
        unit += 1
    }
    return unit === 0 ? `${count} B` : `${size.toFixed(1)} ${units[unit]}`
}

function quantity(value: number | null | undefined): string {
    if (value === null || value === undefined) return NONE
    return Number.isInteger(value) ? String(value) : value.toFixed(2)
}

/** A resource's project as `name (id)`. */
function project(value: { projectId?: string | null; projectName?: string | null }): string {
    const { projectId: id, projectName: name } = value
    if (typeof id === 'string' && typeof name === 'string') return `${clean(name)} (${clean(id)})`
    if (typeof id === 'string') return clean(id)
    return NONE
}

/** `body[key]` as a list; an answer that is not the API's is an error, not an empty list. */
function items<T, K extends keyof T>(body: T, key: K): NonNullable<T[K]> {
    const list = (body as Record<PropertyKey, unknown> | null)?.[key]
    if (!Array.isArray(list)) {
        throw new LocalFailure(`unexpected answer: no \`${String(key)}\` list in it (is --api a Zega Cloud address?)`)
    }
    return list as NonNullable<T[K]>
}

function one<T, K extends keyof T>(body: T, key: K): NonNullable<T[K]> {
    const value = (body as Record<PropertyKey, unknown> | null)?.[key]
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        throw new LocalFailure(`unexpected answer: no \`${String(key)}\` in it (is --api a Zega Cloud address?)`)
    }
    return value as NonNullable<T[K]>
}

export function whoami(body: Answer<'/v1/whoami'>, api: string | null): string {
    const token = one(body, 'token')
    const rows: [string, string][] = []
    if (api !== null) rows.push(['API', clean(api)])
    rows.push(
        ['Token', `${text(token.name)} (${text(token.prefix)}...)`],
        ['Scope', text(token.scope)],
        ['Project', token.projectId ? project(token) : 'all projects'],
        ['Created', text(token.createdAt)],
        ['Last used', text(token.lastUsedAt ?? 'never')],
        ['Expires', text(token.expiresAt ?? 'never')],
    )
    return detail(rows)
}

function graphTable(graphs: Schemas['Graph'][]): Table {
    const table = new Table([
        { header: 'ID' },
        { header: 'NAME' },
        { header: 'PROJECT' },
        { header: 'REGION' },
        { header: 'TIER' },
        { header: 'STATUS' },
    ])
    for (const graph of graphs) {
        table.row([text(graph.id), text(graph.name), project(graph), text(graph.region), text(graph.tier), text(graph.status)])
    }
    return table
}

export function projects(body: Answer<'/v1/projects'>): string {
    const list = items(body, 'projects')
    if (list.length === 0) return 'no projects\n'
    const table = new Table([{ header: 'ID' }, { header: 'NAME' }, { header: 'GRAPHS', right: true }, { header: 'CREATED' }])
    for (const p of list) {
        table.row([text(p.id), text(p.name), String(Array.isArray(p.graphs) ? p.graphs.length : 0), text(p.createdAt)])
    }
    return table.render()
}

export function projectOne(body: Answer<'/v1/projects/{project}'>): string {
    const p = one(body, 'project')
    let out = detail([
        ['ID', text(p.id)],
        ['Name', text(p.name)],
        ['Created', text(p.createdAt)],
    ])
    const graphs = items(p, 'graphs')
    out += graphs.length === 0 ? '\nno graphs\n' : '\n' + graphTable(graphs).render()
    return out
}

export function graphs(body: Answer<'/v1/graphs'>): string {
    const list = items(body, 'graphs')
    return list.length === 0 ? 'no graphs\n' : graphTable(list).render()
}

export function graphOne(body: Answer<'/v1/graphs/{graph}'>): string {
    const g = one(body, 'graph')
    const rows: [string, string][] = [
        ['ID', text(g.id)],
        ['Name', text(g.name)],
        ['Project', project(g)],
        ['Status', text(g.status)],
        ['Region', text(g.region)],
        ['Tier', `${text(g.tier)} (${text(g.memoryMb)} MB)`],
        ['Billing', `${text(g.billing)}, ${cents(g.priceCents)} base price`],
        ['Address', text(g.hostUrl)],
        ['Created', text(g.createdAt)],
    ]
    if (g.graceUntil) rows.push(['Grace until', text(g.graceUntil)])
    if (g.deleteAfter) rows.push(['Data deleted', text(g.deleteAfter)])
    return detail(rows)
}

export function buckets(body: Answer<'/v1/buckets'>): string {
    const list = items(body, 'buckets')
    if (list.length === 0) return 'no buckets\n'
    const table = new Table([
        { header: 'ID' },
        { header: 'NAME' },
        { header: 'PROJECT' },
        { header: 'STORED', right: true },
        { header: 'CREATED' },
    ])
    for (const b of list) {
        table.row([text(b.id), text(b.name), project(b), typeof b.bytesStored === 'number' ? bytes(b.bytesStored) : NONE, text(b.createdAt)])
    }
    return table.render()
}

export function bucketOne(body: Answer<'/v1/buckets/{bucket}'>): string {
    const b = one(body, 'bucket')
    return detail([
        ['ID', text(b.id)],
        ['Name', text(b.name)],
        ['Project', project(b)],
        ['Address', text(b.url)],
        ['Stored', typeof b.bytesStored === 'number' ? bytes(b.bytesStored) : NONE],
        ['Cap', `${cents(b.capCents)} of overage a month (changed in the dashboard)`],
        ['Created', text(b.createdAt)],
    ])
}

function deployed(f: Schemas['Function']): string {
    return f.deployed === true ? `v${text(f.version)}` : 'no'
}

export function functions(body: Answer<'/v1/functions'>): string {
    const list = items(body, 'functions')
    if (list.length === 0) return 'no functions\n'
    const table = new Table([
        { header: 'ID' },
        { header: 'NAME' },
        { header: 'PROJECT' },
        { header: 'DEPLOYED' },
        { header: 'LOGS' },
        { header: 'ADDRESS' },
    ])
    for (const f of list) {
        table.row([text(f.id), text(f.name), project(f), deployed(f), text(f.logs), text(f.url)])
    }
    return table.render()
}

export function functionOne(body: Answer<'/v1/functions/{function}'>): string {
    const f = one(body, 'function')
    let out = detail([
        ['ID', text(f.id)],
        ['Name', text(f.name)],
        ['Project', project(f)],
        ['Address', text(f.url)],
        ['Deployed', deployed(f) === 'no' ? 'no' : `${deployed(f)} at ${text(f.deployedAt)}`],
        ['Logs', text(f.logs)],
        ['Cap', `${cents(f.capCents)} of overage a month (changed in the dashboard)`],
        ['Limits', f.limits ? `${text(f.limits.cpuMs)} ms CPU, ${text(f.limits.subRequests)} subrequests per call` : NONE],
        ['Created', text(f.createdAt)],
        ['Updated', text(f.updatedAt)],
    ])
    const vars = Object.entries(f.vars ?? {})
    if (vars.length > 0) {
        out += '\nVariables\n'
        for (const [name, value] of vars) out += `  ${clean(name)}=${clean(String(value))}\n`
    }
    const secrets = f.secrets ?? []
    if (secrets.length > 0) {
        out += '\nSecrets (names only; values are not readable)\n'
        for (const name of secrets) out += `  ${clean(String(name))}\n`
    }
    return out
}

export function deployResult(body: Answer<'/v1/functions/{function}/code', 'put'>): string {
    const f = one(body, 'function')
    return detail([
        ['Deployed', `${text(f.id)} (${text(f.name)}) as v${text(f.version)}`],
        ['Address', text(f.url)],
    ])
}

export function regions(body: Answer<'/v1/regions'>): string {
    const table = new Table([{ header: 'ID' }, { header: 'CITY' }, { header: 'COUNTRY' }, { header: 'CONTINENT' }])
    for (const r of items(body, 'regions')) table.row([text(r.id), text(r.city), text(r.country), text(r.continent)])
    return table.render()
}

export function usage(body: Answer<'/v1/usage'>): string {
    const list = items(body, 'graphs')
    let out = detail([
        ['Month', text(body.month)],
        ['To date', cents(body.toDateCents)],
        ['Projected', cents(body.projectedCents)],
        ['Worst case', cents(body.worstCaseCents)],
    ])
    if (list.length === 0) return out + '\nno active graphs\n'
    const table = new Table([
        { header: 'GRAPH' },
        { header: 'QUERIES', right: true },
        { header: 'WRITES', right: true },
        { header: 'STORAGE GB', right: true },
        { header: 'EGRESS GB', right: true },
        { header: 'NODES', right: true },
        { header: 'TO DATE', right: true },
        { header: 'PROJECTED', right: true },
        { header: 'STATE' },
    ])
    for (const g of list) {
        table.row([
            text(g.graph),
            quantity(g.usage?.queries),
            quantity(g.usage?.writes),
            quantity(g.usage?.storageGb),
            quantity(g.usage?.egressGb),
            `${text(g.nodes?.count)}/${text(g.nodes?.included)}`,
            cents(g.toDate?.totalCents),
            cents(g.projected?.totalCents),
            g.paused === true ? 'paused at cap' : 'ok',
        ])
    }
    return `${out}\n${table.render()}`
}

export function logs(body: Answer<'/v1/functions/{function}/logs'>): string {
    const entries = items(body, 'entries')
    let out = `${text(body.function)}: logs ${text(body.logs)}, ${text(body.from)} to ${text(body.to)} (kept ${text(body.retentionDays)} days)\n`
    if (entries.length === 0) out += 'no requests in this window\n'
    for (const entry of entries) {
        out += `${text(entry.time)}  ${text(entry.method)} ${text(entry.path)}  ${text(entry.status)}  ${
            typeof entry.durationMs === 'number' ? quantity(entry.durationMs) : NONE
        } ms  ${text(entry.outcome)}\n`
        if (typeof entry.exception === 'string') out += `    ! ${clean(entry.exception)}\n`
        for (const line of entry.logs ?? []) {
            const [first = '', ...more] = String(line.message ?? '').split('\n')
            out += `    ${text(line.level)}  ${clean(first)}\n`
            for (const rest of more) out += `        ${clean(rest)}\n`
        }
        if (entry.logsTruncated === true) out += '    (more console lines were written than are kept)\n'
    }
    if (typeof body.cursor === 'string') out += `more: pass --cursor ${clean(body.cursor)} for the next page\n`
    return out
}
