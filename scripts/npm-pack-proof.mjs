import assert from 'node:assert/strict'
import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const temp = fs.mkdtempSync(path.join(root, '.tmp', 'npm-pack-proof-'))

function run(command, args, options = {}) {
    const result = spawnSync(command, args, { cwd: root, encoding: 'utf8', ...options })
    if (result.status !== 0) {
        throw new Error(`${command} ${args.join(' ')} exited ${result.status}\n${result.stdout}\n${result.stderr}`)
    }
    return result.stdout
}

function filesUnder(dir, base = dir) {
    return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
        const file = path.join(dir, entry.name)
        return entry.isDirectory() ? filesUnder(file, base) : [path.relative(base, file).split(path.sep).join('/')]
    })
}

try {
    const packed = JSON.parse(run('npm', ['pack', '--json', '--pack-destination', temp]))[0]
    const archive = path.join(temp, packed.filename)
    const files = packed.files.map((file) => file.path)
    assert(files.includes('README.md'), 'README.md is missing from the tarball')
    assert(files.includes('LICENSE'), 'LICENSE is missing from the tarball')
    assert(files.includes('tsconfig.json'), 'Bun runtime path aliases need the package tsconfig')
    assert(!files.some((file) => /(^|\/)tests?(\/|$)|(^|\/)\.tmp(\/|$)/i.test(file)), 'tests or .tmp made it into the tarball')

    const sourceFiles = filesUnder(path.join(root, 'src')).map((file) => `src/${file}`)
    for (const file of sourceFiles) assert(files.includes(file), `${file} is read at runtime or ships with src/ but is absent`)
    assert(files.includes('src/types/openapi.json'), 'the region fallback OpenAPI data is absent')
    assert(files.some((file) => file.startsWith('src/explorer/page/')), 'the vendored explorer page is absent')

    const manifest = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))
    const platformIds = {
        'darwin-arm64': 'server-darwin-arm64',
        'darwin-x64': 'server-darwin-x64',
        'linux-x64': 'server-linux-x64',
        'win32-x64': 'server-win32-x64',
    }
    const id = `${process.platform}-${process.arch}`
    const serverPackage = platformIds[id]
    assert(serverPackage, `pack proof does not have a local server fixture for ${id}`)
    const name = `@zegadb/${serverPackage}`
    const fake = path.join(temp, 'fake-server')
    const fakeBinaryName = id.startsWith('win32-') ? 'zega-server.exe' : 'zega-server'
    fs.mkdirSync(path.join(fake, 'bin'), { recursive: true })
    fs.writeFileSync(path.join(fake, 'package.json'), JSON.stringify({ name, version: manifest.optionalDependencies[name], bin: { 'zega-server': `bin/${fakeBinaryName}` } }))
    fs.writeFileSync(path.join(fake, 'bin', fakeBinaryName), '#!/usr/bin/env bun\nconsole.log("fake zega-server help")\n')
    fs.chmodSync(path.join(fake, 'bin', fakeBinaryName), 0o755)

    const project = path.join(temp, 'project')
    fs.mkdirSync(project)
    const projectManifest = {
        private: true,
        dependencies: { zegadb: `file:${archive}` },
    }
    fs.writeFileSync(path.join(project, 'package.json'), JSON.stringify(projectManifest, null, 2))
    fs.writeFileSync(path.join(project, 'tsconfig.json'), JSON.stringify({ compilerOptions: {} }))
    run('bun', ['install', '--no-save', '--no-progress', '--no-optional'], { cwd: project })
    const installedFake = path.join(project, 'node_modules', ...name.split('/'))
    const fakeBinary = path.join(installedFake, 'bin', fakeBinaryName)
    fs.mkdirSync(path.dirname(fakeBinary), { recursive: true })
    fs.cpSync(fake, installedFake, { recursive: true })
    assert(fs.existsSync(fakeBinary), `local fake server is missing at ${fakeBinary}`)

    const cli = path.join(project, 'node_modules', '.bin', 'zega')
    const env = { ...process.env, PATH: `${path.dirname(cli)}${path.delimiter}${process.env.PATH ?? ''}` }
    const runInstalled = (args, cwd = project) => run('zega', args, { cwd, env })
    const help = runInstalled(['--help'])
    assert.match(help, /"name":\s*"zega"/)

    const starter = path.join(temp, 'starter')
    fs.mkdirSync(starter)
    const init = runInstalled(['init', '--template', 'function', '--name', 't', '--region', 'yyz'], starter)
    assert.match(init, /"template":\s*"function"/)
    const starterDir = path.join(starter, 't')
    const starterPackage = JSON.parse(fs.readFileSync(path.join(starterDir, 'package.json'), 'utf8'))
    assert.equal(starterPackage.devDependencies.zegadb, '0.1.0')
    assert.equal(starterPackage.dependencies['@zegadb/lib'], '0.2.0')

    const inPlace = path.join(temp, 'in-place')
    fs.mkdirSync(inPlace)
    runInstalled(['init', '--template', 'function', '--name', '.', '--region', 'yyz'], inPlace)
    assert(fs.existsSync(path.join(inPlace, 'package.json')), '--name . did not write in place')
    assert(!fs.existsSync(path.join(inPlace, path.basename(inPlace), 'package.json')), '--name . created a child folder')

    // Use a local package-manager fixture so --install is proved without a registry or network.
    const fakePm = path.join(temp, 'fake-pm')
    const installMarker = path.join(temp, 'install-cwd')
    fs.mkdirSync(fakePm)
    const fakeBun = path.join(fakePm, 'bun')
    fs.writeFileSync(fakeBun, `#!/bin/sh\npwd > ${JSON.stringify(installMarker)}\n`)
    fs.chmodSync(fakeBun, 0o755)
    const installRoot = path.join(temp, 'install')
    fs.mkdirSync(installRoot)
    const human = run('zega', ['init', '--human', '--template', 'function', '--name', 'installed', '--region', 'yyz', '--install'], {
        cwd: installRoot,
        env: { ...env, PATH: `${fakePm}${path.delimiter}${env.PATH}`, npm_config_user_agent: 'bun/1.3.14' },
    })
    const installDir = path.join(installRoot, 'installed')
    assert.equal(fs.readFileSync(installMarker, 'utf8').trim(), installDir, '--install did not run in the new project folder')
    assert.match(human, /Created installed\/ \(function starter\)/)
    assert(!/\bFiles\b/.test(human), '--human output contains a file list')
    assert(!/cloudflare|workers?\b|wrangler|opennext|workerd/i.test(human), '--human output names the provider')

    for (const template of ['nextjs', 'astro']) {
        const directory = path.join(temp, template)
        fs.mkdirSync(directory)
        runInstalled(['init', '--template', template, '--name', 't', '--region', 'yyz'], directory)
        const generated = JSON.parse(fs.readFileSync(path.join(directory, 't', 'package.json'), 'utf8'))
        assert.equal(generated.devDependencies.zegadb, '0.1.0', `${template} starter CLI version differs`)
        assert.equal(generated.dependencies['@zegadb/lib'], '0.2.0', `${template} starter lib version differs`)
    }

    const serverHelp = runInstalled(['start', '--help'])
    assert.match(serverHelp, /fake zega-server help/)
    process.stdout.write('npm pack proof passed: tarball allowlist, fresh Bun install, zega --help, init, and start --help\n')
} finally {
    fs.rmSync(temp, { recursive: true, force: true })
}
