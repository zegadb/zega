import { createHash } from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

import type { Globals, Session } from '@/handlers'
import { buildApp, type AppProject } from '@/handlers/app'
import type { ProjectConfig } from '@/handlers/project_config'
import { isFunctionId, write as writeConfig } from '@/handlers/project_config'
import { ApiFailure, LocalFailure } from '@/utils/errors'
import { event, write } from '@/utils'

const LIMITS = {
    moduleBytes: 20 * 1024 * 1024,
    modules: 128,
    assetFiles: 20_000,
    assetFileBytes: 10 * 1024 * 1024,
    assetBytes: 200 * 1024 * 1024,
    batchFiles: 200,
    batchBytes: 16 * 1024 * 1024,
} as const

type FileData = { path: string; bytes: Buffer; type: string }
type AssetData = FileData & { hash: string }
type AppFiles = {
    mainModule: string
    modules: FileData[]
    moduleBytes: number
    manifest: Record<string, { hash: string; size: number }>
    assetsByHash: Map<string, AssetData>
    assetBytes: number
}
type AppSession = { session: string; buckets: string[][]; remaining: number; complete: boolean }
type AppReply = { function: { id: string; name: string; version: number; url: string } }

function line(globals: Globals, name: string, data: Record<string, unknown>, human: string): Promise<void> {
    return write('stdout', globals.human ? `${human}\n` : event({ event: name, ...data }))
}

function inside(dir: string, relative: string): string {
    const absolute = path.resolve(dir, relative)
    const fromRoot = path.relative(path.resolve(dir), absolute)
    if (fromRoot === '..' || fromRoot.startsWith(`..${path.sep}`) || path.isAbsolute(fromRoot)) {
        throw new LocalFailure(`build output ${relative} is outside the project directory`)
    }
    return absolute
}

function walk(root: string): string[] {
    const files: string[] = []
    const visit = (dir: string): void => {
        for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
            const absolute = path.join(dir, entry.name)
            if (entry.isSymbolicLink()) throw new LocalFailure(`build output contains a symbolic link: ${absolute}`)
            if (entry.isDirectory()) visit(absolute)
            else if (entry.isFile()) files.push(absolute)
        }
    }
    visit(root)
    return files.sort()
}

function mime(file: string): string {
    switch (path.extname(file).toLowerCase()) {
        case '.js':
        case '.mjs':
            return 'application/javascript+module'
        case '.wasm':
            return 'application/wasm'
        case '.txt':
            return 'text/plain'
        default:
            return 'application/octet-stream'
    }
}

function ignoredByAssetsFile(root: string): (candidate: string) => boolean {
    const ignoreFile = path.join(root, '.assetsignore')
    if (!fs.existsSync(ignoreFile)) return () => false
    const patterns = fs
        .readFileSync(ignoreFile, 'utf8')
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line !== '' && !line.startsWith('#'))
        .map((pattern) => {
            const normalized = pattern.replace(/^\//, '').replace(/\/$/, '')
            const escaped = normalized
                .replace(/[.+^${}()|[\]\\]/g, '\\$&')
                .replace(/\*\*/g, '\u0000')
                .replace(/\*/g, '[^/]*')
                .replace(/\?/g, '[^/]')
                .replace(/\u0000/g, '.*')
            return new RegExp(`^(?:${escaped})(?:/.*)?$`)
        })
    return (candidate) => candidate === '.assetsignore' || patterns.some((pattern) => pattern.test(candidate))
}

function collect(dir: string, config: AppProject): AppFiles {
    const workerFile = inside(dir, config.build.worker)
    const workerRoot = path.dirname(workerFile)
    if (!fs.existsSync(workerFile) || !fs.statSync(workerFile).isFile()) {
        throw new LocalFailure(`the build did not make the worker: ${config.build.worker} is missing`, 'build_output_missing')
    }
    const assetRoot = inside(dir, config.build.assets)
    if (!fs.existsSync(assetRoot) || !fs.statSync(assetRoot).isDirectory()) {
        throw new LocalFailure(`the build did not make the assets: ${config.build.assets} is missing`, 'build_output_missing')
    }

    const assets = new Map<string, AssetData>()
    const manifest: AppFiles['manifest'] = {}
    let assetBytes = 0
    const isIgnored = ignoredByAssetsFile(assetRoot)
    for (const absolute of walk(assetRoot)) {
        const relative = path.relative(assetRoot, absolute).split(path.sep).join('/')
        if (isIgnored(relative)) continue
        const bytes = fs.readFileSync(absolute)
        const extension = path.extname(relative).slice(1).toLowerCase()
        const hash = createHash('sha256').update(Buffer.from(bytes.toString('base64') + extension)).digest('hex').slice(0, 32)
        if (bytes.byteLength > LIMITS.assetFileBytes) {
            throw new LocalFailure(`asset_size_limit: ${relative} is ${bytes.byteLength} bytes; the limit is ${LIMITS.assetFileBytes} bytes`)
        }
        assetBytes += bytes.byteLength
        manifest[`/${relative}`] = { hash, size: bytes.byteLength }
        assets.set(hash, { path: relative, bytes, hash, type: mime(relative) })
    }
    if (Object.keys(manifest).length > LIMITS.assetFiles) {
        throw new LocalFailure(`asset_count_limit: ${Object.keys(manifest).length} files; the limit is ${LIMITS.assetFiles}`)
    }
    if (assetBytes > LIMITS.assetBytes) {
        throw new LocalFailure(`assets_total_limit: ${assetBytes} bytes; the per-app limit is ${LIMITS.assetBytes} bytes`)
    }

    const assetPrefix = path.relative(workerRoot, assetRoot).split(path.sep).join('/')
    const moduleFiles = walk(workerRoot).filter((file) => {
        const relative = path.relative(workerRoot, file).split(path.sep).join('/')
        return assetPrefix === '' || !(relative === assetPrefix || relative.startsWith(`${assetPrefix}/`))
    })
    const modules = moduleFiles.map((file) => ({
        path: path.relative(workerRoot, file).split(path.sep).join('/'),
        bytes: fs.readFileSync(file),
        type: mime(file),
    }))
    const moduleBytes = modules.reduce((total, module) => total + module.bytes.byteLength, 0)
    if (modules.length > LIMITS.modules) {
        throw new LocalFailure(`modules_count_limit: ${modules.length} files; the limit is ${LIMITS.modules}`)
    }
    if (moduleBytes > LIMITS.moduleBytes) {
        throw new LocalFailure(`modules_too_large: ${moduleBytes} bytes; the limit is ${LIMITS.moduleBytes} bytes`)
    }
    const mainModule = path.relative(workerRoot, workerFile).split(path.sep).join('/')
    if (!modules.some((module) => module.path === mainModule)) throw new LocalFailure(`main module ${mainModule} is missing`)
    return { mainModule, modules, moduleBytes, manifest, assetsByHash: assets, assetBytes }
}

async function call<T>(globals: Globals, action: () => Promise<T>): Promise<T> {
    for (;;) {
        try {
            return await action()
        } catch (error) {
            if (!(error instanceof ApiFailure) || error.status !== 429 || error.retryAfter === null) throw error
            await line(globals, 'rate_limited', { retryAfter: error.retryAfter }, `Rate limited; retrying in ${error.retryAfter}s`)
            await Bun.sleep(error.retryAfter * 1000)
        }
    }
}

async function targetFunction(
    dir: string,
    config: ProjectConfig & { kind: 'app' },
    session: Session,
    requestedId: string | undefined,
    globals: Globals,
): Promise<string> {
    if (requestedId !== undefined) {
        if (!isFunctionId(requestedId)) throw new LocalFailure(`not a function id: ${requestedId}`)
        return requestedId
    }
    if (config.appId !== undefined) return config.appId

    let projectId = config.projectId
    if (projectId === undefined) {
        const projects = (await call(globals, () => session.client.get('/v1/projects'))).json.projects
        const matches = projects.filter((project) => project.name === config.name)
        if (matches.length === 0) {
            throw new LocalFailure(
                `no Cloud project named ${JSON.stringify(config.name)}; create it in the dashboard or set "projectId" in ${path.basename(path.join(dir, 'zega.json'))}`,
                'project_not_found',
            )
        }
        if (matches.length > 1) {
            throw new LocalFailure(`more than one Cloud project is named ${JSON.stringify(config.name)}; set "projectId" in zega.json`, 'project_ambiguous')
        }
        projectId = matches[0]!.id
    }

    const reply = await call(globals, () =>
        session.client.postJson('/v1/projects/{project}/functions', {
            path: { project: projectId! },
            body: { name: config.name, kind: 'app' },
        }),
    )
    const created = (reply.json as { function?: { id?: unknown } }).function?.id
    if (typeof created !== 'string' || !isFunctionId(created)) throw new LocalFailure('Cloud returned a function without a valid id')
    const next = { ...config, projectId, appId: created }
    writeConfig(dir, next)
    await line(globals, 'function_created', { projectId, function: created }, `Created app ${created} in project ${projectId}`)
    return created
}

export async function deployApp(
    dir: string,
    config: AppProject,
    session: Session,
    requestedId: string | undefined,
    globals: Globals,
): Promise<void> {
    await line(globals, 'building', { framework: config.framework, command: config.build.command }, `Building ${config.framework} app`)
    await buildApp(dir, config, globals.human === true)
    const files = collect(dir, config)
    await line(
        globals,
        'built',
        { framework: config.framework, mainModule: files.mainModule, modules: files.modules.length, moduleBytes: files.moduleBytes, assets: Object.keys(files.manifest).length, assetBytes: files.assetBytes },
        `Built ${files.modules.length} worker modules and ${Object.keys(files.manifest).length} assets`,
    )

    const functionId = await targetFunction(dir, config, session, requestedId, globals)
    const opened = (await call(globals, () =>
        session.client.postJson('/v1/functions/{function}/app/assets', {
            path: { function: functionId },
            body: { manifest: files.manifest },
        }),
    )).json as AppSession
    await line(
        globals,
        'assets_manifest',
        { function: functionId, files: Object.keys(files.manifest).length, changed: opened.buckets.reduce((sum, batch) => sum + batch.length, 0), batches: opened.buckets.length },
        `Prepared ${Object.keys(files.manifest).length} assets; ${opened.buckets.reduce((sum, batch) => sum + batch.length, 0)} need upload`,
    )

    let uploaded = 0
    for (let index = 0; index < opened.buckets.length; index++) {
        const hashes = opened.buckets[index]!
        const form = new FormData()
        let bytes = 0
        for (const hash of hashes) {
            const asset = files.assetsByHash.get(hash)
            if (asset === undefined) throw new LocalFailure(`Cloud requested unknown asset hash ${hash}`)
            bytes += asset.bytes.byteLength
            form.append(hash, new Blob([asset.bytes], { type: asset.type }))
        }
        if (hashes.length > LIMITS.batchFiles || bytes > LIMITS.batchBytes) {
            throw new LocalFailure(`Cloud returned an upload batch above its ${LIMITS.batchFiles}-file / ${LIMITS.batchBytes}-byte limit`)
        }
        await call(globals, () =>
            session.client.putForm('/v1/functions/{function}/app/assets/{session}', {
                path: { function: functionId, session: opened.session },
                body: form,
            }),
        )
        uploaded += hashes.length
        await line(
            globals,
            'assets_uploaded',
            { function: functionId, batch: index + 1, batches: opened.buckets.length, files: uploaded, bytes },
            `Uploaded asset batch ${index + 1}/${opened.buckets.length} (${hashes.length} files)`,
        )
    }

    const form = new FormData()
    const metadata = {
        mainModule: files.mainModule,
        session: opened.session,
        config: {
            htmlHandling: 'auto-trailing-slash',
            notFoundHandling: 'none',
            runWorkerFirst: true,
            compatibilityDate: config.runtime.compatibilityDate,
            compatibilityFlags: config.runtime.compatibilityFlags,
        },
    }
    form.append('metadata', JSON.stringify(metadata))
    for (const module of files.modules) form.append(module.path, new Blob([module.bytes], { type: module.type }))
    await line(globals, 'deploying', { function: functionId, modules: files.modules.length, moduleBytes: files.moduleBytes }, `Deploying ${functionId}`)
    const deployed = (await call(globals, () =>
        session.client.putForm('/v1/functions/{function}/app', { path: { function: functionId }, body: form }),
    )).json as AppReply
    const item = deployed.function
    await line(
        globals,
        'deployed',
        { function: item, modules: files.modules.length, assets: Object.keys(files.manifest).length },
        `Deployed  ${item.id} (${item.name}) as v${item.version}\nAddress   ${item.url}`,
    )
}
