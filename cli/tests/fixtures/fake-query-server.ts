#!/usr/bin/env bun
/**
 * A stand-in for `zega-server`, for the tests of `zega mcp`'s local group. Like the real one it has `QUERY /zql`
 * (reads only: a mutation is refused with 422 `not_a_read`), `POST /zql`, `GET /stats` and `GET /health`; it
 * writes every request to `<data>/requests.jsonl`, prints its address, and leaves `<data>/stopped` on SIGINT/SIGTERM.
 */
import fs from 'node:fs'
import path from 'node:path'

const args = process.argv.slice(2)
const flag = (name: string): string | undefined => {
    const at = args.indexOf(name)
    return at === -1 ? undefined : args[at + 1]
}
const data = flag('--data') ?? './zega-data'
fs.mkdirSync(data, { recursive: true })
const log = path.join(data, 'requests.jsonl')
let nodes = 0

const server = Bun.serve({
    hostname: '127.0.0.1',
    port: Number(flag('--port') ?? 9342),
    async fetch(request) {
        const url = new URL(request.url)
        const body = request.method === 'GET' ? '' : await request.text()
        fs.appendFileSync(log, JSON.stringify({ method: request.method, path: url.pathname, body }) + '\n')
        if (url.pathname === '/health') return Response.json({ ok: true })
        if (url.pathname === '/stats') return Response.json({ ok: true, result: { nodes, relationships: 0 } })
        if (url.pathname === '/zql' && (request.method === 'POST' || request.method === 'QUERY')) {
            const sent = JSON.parse(body) as { query: string }
            if (request.method === 'QUERY' && /\bmutation\b/.test(sent.query)) {
                return Response.json({ ok: false, error: 'QUERY /zql runs reads only', code: 'not_a_read' }, { status: 422 })
            }
            if (request.method === 'POST') nodes++
            return Response.json({ ok: true, result: { via: request.method, echoed: sent.query } })
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
