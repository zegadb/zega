import path from 'node:path'

import { openSessionIfAny, projectConfig, regionChoices, type Globals } from '@/handlers'
import * as questions from '@/questions'
import { LocalFailure } from '@/utils/errors'
import { answer, canPrompt, welcome, write } from '@/utils'

export type InitOptions = Globals & { name?: string; region?: string; force?: boolean }

/**
 * Write `zega.json` for a project in the current directory. It creates nothing
 * in Zega Cloud: creating a project or a graph costs money and is done in the
 * dashboard, signed in.
 */
export async function init(options: InitOptions): Promise<void> {
    const dir = process.cwd()
    const prompt = canPrompt(options)
    if (prompt) await welcome('init', 'start a zega project here')

    // Without a terminal nothing is asked: name every flag that is missing at once.
    if (!prompt) {
        const missing = [
            options.name === undefined ? '--name' : null,
            options.region === undefined ? '--region' : null,
            projectConfig.exists(dir) && options.force !== true ? '--force' : null,
        ].filter((flag) => flag !== null)
        if (missing.length > 1) {
            throw new LocalFailure(
                `missing ${missing.join(', ')}. There is no terminal to ask on, so every answer must be a flag.`,
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

    const choices = await regionChoices(await openSessionIfAny(options))
    const region = await answer({
        flag: '--region',
        given: options.region,
        hint: `a region id (${choices.map((choice) => choice.id).join(', ')})`,
        prompt,
        validate: (value) => questions.validateRegion(value, choices),
        ask: () => questions.region(choices),
    })

    const config: projectConfig.ProjectConfig = { version: 1, name: name.trim(), region }
    const file = projectConfig.write(dir, config)
    if (options.json === true) {
        await write('stdout', JSON.stringify({ file, config }, null, 2) + '\n')
        return
    }
    await write('stdout', `wrote ${file}\nName    ${config.name}\nRegion  ${config.region}\n`)
}
