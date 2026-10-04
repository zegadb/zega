import fs from 'node:fs'
import path from 'node:path'
import inquirer from 'inquirer'

import { openSessionIfAny, projectConfig, regionChoices, type Globals } from '@/handlers'
import * as questions from '@/questions'
import { DEFAULT_TEMPLATE, TEMPLATES, starter, type Template } from '@/templates'
import { LocalFailure } from '@/utils/errors'
import { answer, canPrompt, result, welcome, write } from '@/utils'

export type InitOptions = Globals & {
    name?: string
    region?: string
    force?: boolean
    here?: boolean
    install?: boolean
    function?: string
    entry?: string
    template?: string
}

export function packageManager(userAgent = process.env.npm_config_user_agent ?? ''): string {
    const agent = userAgent.split(/[ /]/)[0]
    if (agent === 'bun') return 'bun'
    if (agent === 'npm') return 'npm'
    if (agent === 'pnpm') return 'pnpm'
    if (agent === 'yarn') return 'yarn'
    return 'bun'
}

export async function installDependencies(dir: string, pm: string, spawn = Bun.spawn): Promise<void> {
    const child = spawn([pm, 'install'], { cwd: dir, stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' })
    const code = await child.exited
    if (code !== 0) throw new LocalFailure(`dependency installation failed: ${pm} install exited with ${code}`, 'install_failed')
}

/**
 * Write a starter project and its `zega.json` in the current directory: a function (the default), a Next.js
 * app or an Astro app (`--template`). It creates nothing in Zega Cloud: creating a project or a graph costs
 * money and is done in the dashboard, signed in. It never overwrites a file without `--force`.
 */
export async function init(options: InitOptions): Promise<void> {
    const cwd = process.cwd()
    let dir = cwd
    const prompt = canPrompt(options)
    if (prompt) await welcome('init', 'start a zega project here')

    // Which starter is known before anything else: it decides which files are in the way.
    const chosen = await answer({
        flag: '--template',
        given: options.template ?? (prompt ? undefined : DEFAULT_TEMPLATE),
        hint: `one of ${TEMPLATES.join(', ')}`,
        prompt,
        validate: questions.validateTemplate,
        ask: questions.template,
    })
    const template = chosen as Template
    if (template !== 'function' && (options.function !== undefined || options.entry !== undefined)) {
        throw new LocalFailure(`--function and --entry are for the function starter, not ${template}`, 'invalid_flag')
    }
    const entryFlag = options.entry
    if (entryFlag !== undefined) {
        const badEntry = questions.validateEntry(entryFlag)
        if (badEntry !== null) throw new LocalFailure(`--entry: ${badEntry}`, 'invalid_flag')
    }

    // The files this starter writes, to see before asking anything what is already there.
    const planned = starter(template, { name: 'x', entry: entryFlag ?? questions.DEFAULT_ENTRY })
    const writes = [projectConfig.CONFIG_FILE, ...Object.keys(planned.files)]
    let inTheWay: string[] = []

    // Nothing is asked without --human on a terminal: name every flag that is missing at once.
    if (!prompt) {
        const missing = [
            options.name === undefined ? '--name' : null,
            inTheWay.length > 0 && options.force !== true ? '--force' : null,
        ].filter((flag) => flag !== null)
        if (missing.length > 1) {
            throw new LocalFailure(
                `missing ${missing.join(', ')}. Nothing is asked unless --human is given on a terminal: pass them as flags.`,
                'missing_flag',
            )
        }
    }

    const name = await answer({
        flag: '--name',
        given: options.name,
        hint: 'the project name, 1 to 64 characters',
        prompt,
        validate: questions.validateName,
        ask: () => questions.name(path.basename(dir)),
    })

    const session = await openSessionIfAny(options)
    const choices = await regionChoices(session, options.human === true)
    if (session === null) choices.sort((a, b) => Number(b.id === 'yyz') - Number(a.id === 'yyz'))
    const defaultRegion = session !== null ? choices[0]!.id : choices.some((choice) => choice.id === 'yyz') ? 'yyz' : choices[0]!.id
    const region = options.region ?? (prompt ? await questions.region(choices) : defaultRegion)
    const badRegion = questions.validateRegion(region, choices)
    if (badRegion !== null) throw new LocalFailure(`--region: ${badRegion}`, 'invalid_flag')

    // An existing Cloud function can still be supplied explicitly; new starters use their local name until deploy.
    let functionId: string | undefined
    let entry = entryFlag ?? questions.DEFAULT_ENTRY
    if (template === 'function') {
        functionId = options.function
        if (functionId === undefined && options.entry !== undefined) {
            throw new LocalFailure('--entry needs --function: the id of the function it is the source of', 'missing_flag')
        }
        if (functionId !== undefined) {
            const badId = questions.validateFunctionId(functionId)
            if (badId !== null) throw new LocalFailure(`--function: ${badId}`, 'invalid_flag')
            if (entryFlag === undefined && prompt) entry = await questions.entry()
        }
    }

    const projectName = name.trim() === '.' ? path.basename(cwd) : name.trim()
    const inPlace = options.here === true || name.trim() === '.'
    if (!inPlace) {
        if (projectName.includes('/') || projectName.includes('\\') || projectName === '..') throw new LocalFailure('--name must be a folder name, or use --here', 'invalid_flag')
        dir = path.resolve(cwd, projectName)
        if (!dir.startsWith(cwd + path.sep)) throw new LocalFailure('project folder must be inside the current directory', 'invalid_flag')
        if (fs.existsSync(dir) && fs.readdirSync(dir).length > 0 && options.force !== true) throw new LocalFailure(`${projectName}/ is not empty; use --force to write into it`, 'already_exists')
        fs.mkdirSync(dir, { recursive: true })
    }
    inTheWay = writes.filter((file) => fs.existsSync(path.join(dir, file)))
    if (!prompt && inTheWay.length > 0 && options.force !== true) {
        throw new LocalFailure(`${inTheWay.join(', ')} already exists in the target; pass --force`, 'missing_flag')
    }
    if (inTheWay.length > 0 && options.force !== true) {
        const replace = await answer({ flag: '--force', given: undefined, hint: `${inTheWay.join(', ')} already exists; --force replaces it`, prompt, validate: () => null, ask: () => questions.overwrite(inTheWay) })
        if (!replace) throw new LocalFailure(`${inTheWay.join(', ')} already exists: left as they are`)
    }
    const made = starter(template, { name: projectName, functionId, entry })
    const config: projectConfig.ProjectConfig = { version: 1, name: projectName, region, ...made.config }

    const pm = packageManager()
    const install = options.install ?? (prompt ? await askInstall() : false)
    const written = writeFiles(dir, made.files)
    const file = projectConfig.write(dir, config)
    written.unshift(projectConfig.CONFIG_FILE)
    if (install) {
        await installDependencies(dir, pm)
    }
    if (options.human !== true) {
        const next = made.next.map((step) => step.replaceAll('bun', pm).replace('zega deploy', `${pm} run deploy`))
        await result(options, { template, file, config, files: written, next: [...(inPlace ? [] : [`cd ${path.basename(dir)}`]), ...(install ? next.slice(1) : next)] })
        return
    }
    const deploys = (config.functions ?? []).map((item) => `Function  ${item.id ?? item.name} from ${item.entry}\n`).join('')
    const next = made.next.map((step) => step.replaceAll('bun', pm).replace('zega deploy', `${pm} run deploy`))
    await write(
        'stdout',
        `Created ${path.basename(dir)}/ (${template === 'nextjs' ? 'Next.js' : template === 'astro' ? 'Astro' : 'function'} starter)\nName      ${config.name}\nRegion    ${config.region}\n${deploys}` +
            `\nNext\n${[...(inPlace ? [] : [`cd ${path.basename(dir)}`]), ...(install ? next.slice(1) : next)].map((step, index) => `  ${index + 1}. ${step}\n`).join('')}`,
    )
}

async function askInstall(): Promise<boolean> {
    const { result } = await inquirer.prompt<{ result: boolean }>({ type: 'confirm', name: 'result', message: 'Install dependencies now?', default: true })
    return result
}

/** Write each file (making its directories); the relative paths written, in order. Nothing outside `dir`. */
function writeFiles(dir: string, files: Record<string, string>): string[] {
    const written: string[] = []
    for (const [relative, content] of Object.entries(files)) {
        const target = path.resolve(dir, relative)
        if (path.relative(dir, target).startsWith('..')) throw new LocalFailure(`${relative} is outside this directory`, 'invalid_flag')
        try {
            fs.mkdirSync(path.dirname(target), { recursive: true })
            fs.writeFileSync(target, content)
        } catch (error) {
            throw new LocalFailure(`cannot write ${target}: ${(error as Error).message}`)
        }
        written.push(relative)
    }
    return written
}
