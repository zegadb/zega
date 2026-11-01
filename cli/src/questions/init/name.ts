import inquirer from 'inquirer'

/** The reason a project name is not acceptable, or null. A project name is a display label of 1 to 64 characters. */
export function validateName(value: string): string | null {
    const length = Array.from(value.trim()).length
    if (length < 1 || length > 64) return 'a project name is 1 to 64 characters'
    if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return 'a project name has no control characters'
    return null
}

export async function name(suggestion: string): Promise<string> {
    const { result } = await inquirer.prompt<{ result: string }>({
        name: 'result',
        message: 'Project name:',
        default: suggestion,
        validate: (value: string) => validateName(value) ?? true,
    })
    return result.trim()
}
