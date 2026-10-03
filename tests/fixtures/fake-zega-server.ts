#!/usr/bin/env bun
/**
 * A stand-in for `zega-server`, for the tests of what runs it. It takes the arguments of
 * `zega-server start`, writes them to `<data>/argv.json`, serves the database routes the explorer
 * page uses (GET /health, GET and DELETE /graph, POST /zql, which it records), prints its address as the
 * real one does, and, on SIGINT or SIGTERM, leaves `<data>/stopped` (naming the signal) and exits 0.
 *   <data> named "locked":  prints the real one's complaint on stderr and exits 1
 *   --token-file <file>:    every route needs `Authorization: Bearer <that token>`
 */
import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
const flag = (name: string): string | undefined => {
    const at = args.indexOf(name)
    return at === -1 ? undefined : args[at + 1]
}
const data = flag('--data') ?? './zega-data'

if (args[0] !== 'start') {
    console.error(`fake zega-server: only \`start\` is imitated, got ${args[0]}`)
    process.exit(2)
}
if (path.basename(data) === 'locked') {
    console.error('Error: data directory is locked by another zega process')
    process.exit(1)
}
fs.mkdirSync(data, { recursive: true })
fs.writeFileSync(path.join(data, 'argv.json'), JSON.stringify(args))
const tokenFile = flag('--token-file')
const token = tokenFile === undefined ? null : fs.readFileSync(tokenFile, 'utf8').trim()
const stored = path.join(data, 'graph.json')
const log = path.join(data, 'requests.jsonl')

const server = Bun.serve({
    hostname: '127.0.0.1',
    port: Number(flag('--port') ?? 9342),
    async fetch(request) {
        const url = new URL(request.url)
        const body = request.method === 'GET' || request.method === 'HEAD' ? '' : await request.text()
        fs.appendFileSync(
            log,
            JSON.stringify({
                method: request.method,
                target: url.pathname + url.search,
                contentType: request.headers.get('content-type'),
                authorization: request.headers.get('authorization'),
                origin: request.headers.get('origin'),
                cookie: request.headers.get('cookie'),
                body,
            }) + '\n',
        )
        if (token !== null && request.headers.get('authorization') !== `Bearer ${token}`) {
            return Response.json({ ok: false, error: 'missing or wrong token' }, { status: 401 })
        }
        const read = () => (fs.existsSync(stored) ? JSON.parse(fs.readFileSync(stored, 'utf8')) : { nodes: [], rels: [] })
        if (url.pathname === '/health') return Response.json({ ok: true })
        if (url.pathname === '/graph' && request.method === 'GET') return Response.json({ ok: true, result: read() })
        if (url.pathname === '/graph' && request.method === 'DELETE') {
            fs.rmSync(stored, { force: true })
            return Response.json({ ok: true, result: null })
        }
        if (url.pathname === '/zql' && request.method === 'POST') {
            const sent = JSON.parse(body) as { query: string }
            return Response.json({ ok: true, result: { echoed: sent.query } }, { headers: { 'x-from': 'fake-zega-server' } })
        }
        return Response.json({ ok: false, error: 'not found' }, { status: 404 })
    },
})
console.log(`http://127.0.0.1:${server.port}`)
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
    process.on(signal, () => {
        fs.writeFileSync(path.join(data, 'stopped'), `${signal}\n`)
        server.stop(true)
        process.exit(0)
    })
}
