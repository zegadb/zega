import inquirer from 'inquirer'

import { oneToken } from '@/handlers/credentials'

/** The reason a typed token is not one, or null. Shared with every other way a token arrives. */
export function validateToken(value: string): string | null {
    return oneToken(value) === null ? 'a token is one nonempty word, with no spaces or line breaks' : null
}

/** A hidden prompt: the token is never echoed. */
export async function token(): Promise<string> {
    const { result } = await inquirer.prompt<{ result: string }>({
        type: 'password',
        name: 'result',
        message: 'API token (made at the dashboard, input is hidden):',
        mask: '*',
        validate: (value: string) => validateToken(value) ?? true,
    })
    return result.trim()
}
