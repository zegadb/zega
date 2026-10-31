import { afterAll, describe, expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

import { errorOf, fakeApi, remove, run, scratch } from './helpers'

const home = scratch()
const tokenFile = path.join(home, 'token')
const projectId = 'p0project123'
const functionId = 'f0abcdefghjklmnopq'
const routesBase = `/v1/functions/${functionId}/app`
fs.writeFileSync(tokenFile, 'zc_test_0123456789abcdef0123456789abcdef\n', { mode: 0o600 })
afterAll(() => remove(home))

function assetHash(content: string, extension: string): string {
    return createHash('sha256').update(Buffer.from(Buffer.from(content).toString('base64') + extension.toLowerCase())).digest('hex').slice(0, 32)
}

function appProject(appId?: string): string {
    const dir = scratch()
    fs.mkdirSync(path.join(dir, 'node_modules', '.bin'), { recursive: true })
    fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ type: 'commonjs' }))
    const build = `#!/usr/bin/env node
const fs = require('node:fs')
const path = require('node:path')
const root = process.cwd()
const worker = path.join(root, 'build/worker')
const assets = path.join(root, 'build/assets')
fs.mkdirSync(path.join(worker, 'chunks'), { recursive: true })
fs.mkdirSync(assets, { recursive: true })
fs.writeFileSync(path.join(worker, 'worker.js'), 'export default { fetch() { return new Response("ok") } }')
fs.writeFileSync(path.join(worker, 'chunks/extra.mjs'), 'export const value = 1')
fs.writeFileSync(path.join(worker, 'module.wasm'), 'wasm bytes')
fs.writeFileSync(path.join(assets, 'index.html'), 'unchanged asset')
fs.writeFileSync(path.join(assets, 'changed.js'), 'changed asset')
fs.writeFileSync(path.join(assets, 'ignored-debug.txt'), 'ignore this asset')
fs.writeFileSync(path.join(assets, '.assetsignore'), 'ignored-*.txt\\n')
`
    fs.writeFileSync(path.join(dir, 'node_modules', '.bin', 'fake-build'), build, { mode: 0o755 })
    fs.writeFileSync(
        path.join(dir, 'zega.json'),
        JSON.stringify({
            version: 1,
            name: 'Deploy App',
            region: 'yyz',
            kind: 'app',
            framework: 'nextjs',
            build: { command: ['fake-build'], worker: 'build/worker/worker.js', assets: 'build/assets' },
            runtime: { compatibilityDate: '2026-09-01', compatibilityFlags: ['nodejs_compat'] },
            ...(appId === undefined ? {} : { appId }),
        }),
    )
    return dir
}

describe('zega deploy app', () => {
    test('builds modules and assets, creates and remembers the app, and uploads only API-requested hashes', async () => {
        const dir = appProject()
        const changedHash = assetHash('changed asset', 'js')
        const api = fakeApi({
            'GET /v1/projects': { body: { projects: [{ id: projectId, name: 'Deploy App', createdAt: '2026-10-03T00:00:00Z', graphs: [] }] } },
            [`POST /v1/projects/${projectId}/functions`]: { status: 201, body: { function: { id: functionId } } },
            [`POST ${routesBase}/assets`]: {
                body: { session: 'session1', buckets: [[changedHash]], remaining: 1, complete: false },
            },
            [`PUT ${routesBase}/assets/session1`]: { body: { session: 'session1', uploaded: 1, skipped: 0, remaining: 0, complete: true } },
            [`PUT ${routesBase}`]: {
                body: { function: { id: functionId, name: 'Deploy App', version: 1, url: `https://${functionId}.zegadb.com` } },
            },
        })
        try {
            const deployed = await run(['deploy', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(deployed.code).toBe(0)
            const events = deployed.stdout.trim().split('\n').map((line) => JSON.parse(line))
            expect(events.map(({ event }) => event)).toEqual([
                'building',
                'built',
                'function_created',
                'assets_manifest',
                'assets_uploaded',
                'deploying',
                'deployed',
            ])
            expect(events[1]).toMatchObject({ mainModule: 'worker.js', modules: 3, assets: 2 })
            expect(events[2]).toMatchObject({ projectId, function: functionId })
            expect(api.seen.map(({ method, pathname }) => `${method} ${pathname}`)).toEqual([
                'GET /v1/projects',
                `POST /v1/projects/${projectId}/functions`,
                `POST ${routesBase}/assets`,
                `PUT ${routesBase}/assets/session1`,
                `PUT ${routesBase}`,
            ])
            for (const request of api.seen) expect(request.authorization).toBe('Bearer zc_test_0123456789abcdef0123456789abcdef')

            const manifest = JSON.parse(api.seen[2]!.body).manifest as Record<string, { hash: string; size: number }>
            expect(Object.keys(manifest).sort()).toEqual(['/changed.js', '/index.html'])
            expect(manifest['/changed.js']).toEqual({ hash: changedHash, size: Buffer.byteLength('changed asset') })
            const batch = api.seen[3]!.body
            expect(batch).toContain(`name="${changedHash}"`)
            expect(batch).toContain('changed asset')
            expect(batch).not.toContain('unchanged asset')
            expect(batch).not.toContain('ignore this asset')
            expect(api.seen[4]!.body).toContain('"mainModule":"worker.js"')
            expect(api.seen[4]!.body).toContain('application/javascript+module')
            expect(api.seen[4]!.body).toContain('application/wasm')
            expect(api.seen[4]!.body).toContain('"runWorkerFirst":true')

            const config = JSON.parse(fs.readFileSync(path.join(dir, 'zega.json'), 'utf8'))
            expect(config).toMatchObject({ projectId, appId: functionId })
        } finally {
            api.stop()
            remove(dir)
        }
    })

    test('reports named 413 limits from the cloud API', async () => {
        const dir = appProject(functionId)
        const api = fakeApi({
            [`POST ${routesBase}/assets`]: {
                status: 413,
                body: { ok: false, code: 'account_asset_limit', error: 'An account has exceeded its 1 GiB static asset limit.' },
            },
        })
        try {
            const deployed = await run(['deploy', '--api', api.url, '--token-file', tokenFile], { home, cwd: dir })
            expect(deployed.code).toBe(1)
            expect(errorOf(deployed)).toMatchObject({ code: 'account_asset_limit' })
            expect(errorOf(deployed).message).toContain('1 GiB static asset limit')
            expect(api.seen).toHaveLength(1)
        } finally {
            api.stop()
            remove(dir)
        }
    })
})
