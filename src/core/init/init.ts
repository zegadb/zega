import path from 'node:path'

import { openSessionIfAny, projectConfig, regionChoices, type Globals } from '@/handlers'
import * as questions from '@/questions'
import { LocalFailure } from '@/utils/errors'
import { answer, canPrompt, result, welcome, write } from '@/utils'

export type InitOptions = Globals & {
    name?: string
    region?: string
    force?: boolean
    function?: string
    entry?: string
}

/**
 * Write `zega.json` for a project in the current directory. It creates nothing
 * in Zega Cloud: creating a project or a graph costs money and is done in the
 * dashboard, signed in.
 */
export async function init(options: InitOptions): Promise<void> {
    const dir = process.cwd()
    const prompt = canPrompt(options)
    if (prompt) await welcome('init', 'start a zega project here')

    // Nothing is asked without --human on a terminal: name every flag that is missing at once.
    if (!prompt) {
        const missing = [
            options.name === undefined ? '--name' : null,
            options.region === undefined ? '--region' : null,
            projectConfig.exists(dir) && options.force !== true ? '--force' : null,
        ].filter((flag) => flag !== null)
        if (missing.length > 1) {
            throw new LocalFailure(
                `missing ${missing.join(', ')}. Nothing is asked unless --human is given on a terminal: pass them as flags.`,
                'missing_flag',
            )
        }
    }

    if (projectConfig.exists(dir) && options.force !== true) {
        const file = projectConfig.CONFIG_FILE
        const replace = await answer({
            flag: '--force',
            given: undefined,
            hint: `${file} exists already; --force replaces it`,
            prompt,
            validate: () => null,
            ask: () => questions.overwrite(file),
        })
        if (!replace) throw new LocalFailure(`${file} exists already: left as it is`)
    }

    const name = await answer({
        flag: '--name',
        given: options.name,
        hint: 'the project name, 1 to 64 characters',
        prompt,
        validate: questions.validateName,
        ask: () => questions.name(path.basename(dir)),
    })

    const choices = await regionChoices(await openSessionIfAny(options), options.human === true)
    const region = await answer({
        flag: '--region',
        given: options.region,
        hint: `a region id (${choices.map((choice) => choice.id).join(', ')})`,
        prompt,
        validate: (value) => questions.validateRegion(value, choices),
        ask: () => questions.region(choices),
    })

    // Optional: a function to deploy. Asked on a terminal, never required, never asked without one.
    const functionId =
        options.function ?? (prompt && options.entry === undefined ? (await questions.functionId()) || undefined : undefined)
    if (functionId === undefined && options.entry !== undefined) {
        throw new LocalFailure('--entry needs --function: the id of the function it is the source of', 'missing_flag')
    }
    let entry: string | undefined
    if (functionId !== undefined) {
        const badId = questions.validateFunctionId(functionId)
        if (badId !== null) throw new LocalFailure(`--function: ${badId}`, 'invalid_flag')
        if (options.entry !== undefined) {
            const badEntry = questions.validateEntry(options.entry)
            if (badEntry !== null) throw new LocalFailure(`--entry: ${badEntry}`, 'invalid_flag')
            entry = options.entry
        } else {
            entry = prompt ? await questions.entry() : questions.DEFAULT_ENTRY
        }
    }

    const config: projectConfig.ProjectConfig = {
        version: 1,
        name: name.trim(),
        region,
        ...(functionId === undefined || entry === undefined ? {} : { functions: [{ id: functionId, entry }] }),
    }
    const file = projectConfig.write(dir, config)
    if (options.human !== true) {
        await result(options, { file, config })
        return
    }
    const deploys = (config.functions ?? []).map((item) => `Deploys  ${item.id} from ${item.entry}\n`).join('')
    await write('stdout', `wrote ${file}\nName    ${config.name}\nRegion  ${config.region}\n${deploys}`)
}
