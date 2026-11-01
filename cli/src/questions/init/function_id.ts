import inquirer from 'inquirer'

import { isFunctionId } from '@/handlers/project_config'

/** The reason a function id is not one, or null. */
export function validateFunctionId(value: string): string | null {
    return isFunctionId(value) ? null : 'a function id is "f0" and 16 characters (see `zega functions`)'
}

/** Optional: an empty answer means this project deploys no function yet. */
export async function functionId(): Promise<string> {
    const { result } = await inquirer.prompt<{ result: string }>({
        name: 'result',
        message: 'Function id to deploy from this project (empty for none):',
        validate: (value: string) => (value.trim() === '' ? true : (validateFunctionId(value.trim()) ?? true)),
    })
    return result.trim()
}
