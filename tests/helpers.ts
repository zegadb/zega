import fs from 'node:fs'
import path from 'node:path'

import type { components } from '@/types/cloud-api'

const ROOT = path.resolve(import.meta.dir, '..')
const ENTRY = path.join(ROOT, 'src', 'zega.ts')

/** A fresh directory under the repo's own .tmp (never the system's temp dir). */
export function scratch(): string {
    const base = path.join(ROOT, '.tmp')
    fs.mkdirSync(base, { recursive: true, mode: 0o700 })
    return fs.mkdtempSync(path.join(base, 'case-'))
}

export function remove(dir: string): void {
    fs.rmSync(dir, { recursive: true, force: true })
}

export type Seen = {
    method: string
    /** Path and query, as sent. */
    target: string
    pathname: string
    authorization: string | null
    contentType: string | null
    body: string
}

export type Canned = { status?: number; body: unknown; headers?: Record<string, string> }

/**
 * A fake Zega Cloud API on 127.0.0.1 (port chosen by the OS) that records every
 * request and answers from `routes` (`"GET /v1/projects"` to a canned answer).
 */
export function fakeApi(routes: Record<string, Canned>) {
    const seen: Seen[] = []
    const server = Bun.serve({
        hostname: '127.0.0.1',
        port: 0,
        async fetch(request) {
            const url = new URL(request.url)
            seen.push({
                method: request.method,
                target: url.pathname + url.search,
                pathname: url.pathname,
                authorization: request.headers.get('authorization'),
                contentType: request.headers.get('content-type'),
                body: await request.text(),
            })
            const canned = routes[`${request.method} ${url.pathname}`]
            if (canned === undefined) {
                return Response.json({ ok: false, code: 'not_found', error: 'No such route.' }, { status: 404 })
            }
            const body = typeof canned.body === 'string' ? canned.body : JSON.stringify(canned.body)
            return new Response(body, {
                status: canned.status ?? 200,
                headers: { 'content-type': 'application/json', ...canned.headers },
            })
        },
    })
    return {
        seen,
        url: `http://127.0.0.1:${server.port}`,
        host: `127.0.0.1:${server.port}`,
        stop: () => server.stop(true),
    }
}

export type Run = { code: number; stdout: string; stderr: string }

/**
 * Run the CLI as a subprocess, as an agent would: stdin is not a terminal.
 * `stdin` is piped in; null is /dev/null. The environment holds only a PATH and
 * the home that keeps the stored token inside the test's own directory.
 */
export async function run(
    args: string[],
    options: { home: string; cwd?: string; stdin?: string | null },
): Promise<Run> {
    const process_ = Bun.spawn([process.execPath, ENTRY, ...args], {
        cwd: options.cwd ?? options.home,
        env: {
            PATH: process.env.PATH ?? '',
            HOME: options.home,
            XDG_CONFIG_HOME: path.join(options.home, '.config'),
        },
        stdin: options.stdin === null || options.stdin === undefined ? 'ignore' : new Blob([options.stdin]),
        stdout: 'pipe',
        stderr: 'pipe',
    })
    const [stdout, stderr, code] = await Promise.all([
        new Response(process_.stdout).text(),
        new Response(process_.stderr).text(),
        process_.exited,
    ])
    return { code, stdout, stderr }
}

/** The error a failed command printed on stderr: one JSON object on one line. Fails the test if it is anything else. */
export function errorOf(result: { stderr: string }): { code: string; message: string; hint: string | null } {
    const lines = result.stderr.trim().split('\n')
    if (lines.length !== 1) throw new Error(`stderr is not one line: ${JSON.stringify(result.stderr)}`)
    const { error } = JSON.parse(lines[0]!) as { error: { code: string; message: string; hint: string | null } }
    return error
}

export function credentialsFile(home: string): string {
    return path.join(home, '.config', 'zega', 'cloud.json')
}

type Schemas = components['schemas']

export const TOKEN = 'zc_test_0123456789abcdef0123456789abcdef'

export const tokenInfo = {
    id: 'tok1',
    name: 'ci-bot',
    prefix: 'zc_test_0123',
    scope: 'read',
    projectId: null,
    projectName: null,
    createdAt: '2026-10-01T10:00:00.000Z',
    lastUsedAt: null,
    expiresAt: '2026-12-01T00:00:00.000Z',
} satisfies Schemas['TokenInfo']

export const account = { login: 'ada', name: 'Ada Lovelace' } satisfies Schemas['AccountInfo']

export const whoamiRoute = { 'GET /v1/whoami': { body: { account, token: tokenInfo } } }

export const graph = {
    id: 'g0aaaaaaaaaaaaaaaa',
    name: 'main graph',
    projectId: 'p1',
    projectName: 'Atlas',
    region: 'yyz',
    tier: '512mb',
    memoryMb: 512,
    priceCents: 1200,
    billing: 'monthly',
    status: 'active',
    url: '/g/g0aaaaaaaaaaaaaaaa',
    hostUrl: 'https://g0aaaaaaaaaaaaaaaa.zegadb.com',
    graceUntil: null,
    deleteAfter: null,
    createdAt: '2026-09-20T12:00:00.000Z',
} satisfies Schemas['Graph']

export const project = {
    id: 'p1',
    name: 'Atlas',
    createdAt: '2026-09-20T12:00:00.000Z',
    graphs: [graph],
} satisfies Schemas['Project']

export const regionsBody = {
    regions: [
        { id: 'yyz', city: 'Toronto', country: 'Canada', continent: 'North America' },
        { id: 'ams', city: 'Amsterdam', country: 'Netherlands', continent: 'Europe' },
    ],
} satisfies { regions: Schemas['Region'][] }

export const FIXTURES = path.join(ROOT, 'tests', 'fixtures')

export const FUNCTION_ID = 'f0bbbbbbbbbbbbbbbb'

export const deployedFunction = {
    id: FUNCTION_ID,
    name: 'hello',
    projectId: 'p1',
    projectName: 'Atlas',
    url: `https://${FUNCTION_ID}.zegadb.com`,
    capCents: 300,
    deployed: true,
    version: 4,
    etag: 'abc',
    deployedAt: '2026-10-03T09:00:00.000Z',
    vars: {},
    secrets: [],
    limits: { cpuMs: 50, subRequests: 50 },
    logs: 'off',
    logsStoppedAt: null,
    createdAt: '2026-09-22T08:00:00.000Z',
    updatedAt: '2026-10-03T09:00:00.000Z',
} satisfies Schemas['Function']

/** Load a bundle the way a runtime would: as an ES module, and call its default fetch handler. */
export async function callBundle(code: string, url: string): Promise<string> {
    const module = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`)
    const response = await module.default.fetch(new Request(url))
    return await response.text()
}
