import { openSession, type Globals } from '@/handlers'
import inquirer from 'inquirer'
import { isApp } from '@/handlers/app'
import { deployApp } from '@/handlers/app_deploy'
import { prepare, selectTargets, upload } from '@/handlers/deploy'
import * as projectConfig from '@/handlers/project_config'
import { LocalFailure } from '@/utils/errors'

export type DeployOptions = Globals & { minify?: boolean; project?: string }

/**
 * Deploy the functions `zega.json` lists, or only the one whose id is given:
 * bundle each entry into one ES module, then upload it. Needs a manage token.
 */
export async function deploy(id: string | undefined, options: DeployOptions): Promise<void> {
    const dir = process.cwd()
    const initial = projectConfig.read(dir)
    if (isApp(initial)) return await deployApp(dir, initial, await openSession(options), id, options)
    const targets = selectTargets(dir, id, ', or deploy one function with `zega function deploy <id> <entry>`')
    const prepared = await prepare(targets, { dir, minify: options.minify === true, human: options.human === true })
    const config = initial!
    const pending = prepared.filter((item) => item.cloudId === undefined)
    let projectId = options.project ?? config.projectId
    let session: Awaited<ReturnType<typeof openSession>> | undefined
    if (pending.length > 0) {
        if (projectId === undefined && options.human === true && process.stdin.isTTY) {
            session = await openSession(options)
            const projects = (await session.client.get('/v1/projects')).json.projects
            if (projects.length === 0) throw new LocalFailure('no Cloud projects are available; create one in the dashboard first', 'project_not_found')
            const answer = await inquirer.prompt<{ project: string }>({
                type: 'select',
                name: 'project',
                message: 'Cloud project:',
                choices: projects.map((project) => ({ name: project.name, value: project.id })),
            })
            projectId = answer.project
        }
        if (projectId === undefined) throw new LocalFailure('this function has no Cloud id yet: pass --project <project id>', 'missing_flag')
        config.projectId = projectId
    }
    session ??= await openSession(options)
    if (pending.length > 0) {
        for (const target of pending) {
            const response = await session.client.postJson('/v1/projects/{project}/functions', {
                path: { project: projectId! },
                body: { name: target.id },
            }) as { json: { function?: { id?: unknown } } }
            const functionId = response.json.function?.id
            if (typeof functionId !== 'string' || !projectConfig.isFunctionId(functionId)) throw new LocalFailure('Cloud returned a function without a valid id')
            const entry = config.functions?.find((item) => item.name === target.id && item.entry === target.entry)
            if (entry === undefined) throw new LocalFailure(`could not find local function ${target.id} in zega.json`)
            entry.id = functionId
            delete entry.name
            projectConfig.write(dir, config)
            target.cloudId = functionId
        }
    }
    await upload(session, prepared, options)
}
