/**
 * Refresh the API types: fetch the Zega Cloud OpenAPI document (production by
 * default, or the URL given: staging is https://cloud.zega.world/openapi.json),
 * save it as openapi/openapi.json and regenerate src/types/cloud-api.d.ts.
 * Commit both files. `bun run api:types` regenerates the types from the saved
 * document alone.
 */
import fs from 'node:fs'
import path from 'node:path'

const ROOT = path.resolve(import.meta.dir, '..')
const url = process.argv[2] ?? 'https://cloud.zega.dev/openapi.json'

const response = await fetch(url)
if (!response.ok) {
    console.error(`${url} answered HTTP ${response.status}`)
    process.exit(1)
}
const text = await response.text()
const document = JSON.parse(text) as { openapi?: string; info?: { title?: string } }
if (document.openapi === undefined || document.info?.title !== 'Zega Cloud API') {
    console.error(`${url} is not the Zega Cloud API's OpenAPI document`)
    process.exit(1)
}
fs.writeFileSync(path.join(ROOT, 'openapi', 'openapi.json'), text.endsWith('\n') ? text : text + '\n')
const generate = Bun.spawnSync(['bun', 'run', 'api:types'], { cwd: ROOT, stdout: 'inherit', stderr: 'inherit' })
process.exit(generate.exitCode)
