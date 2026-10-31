import inquirer from 'inquirer'

export const DEFAULT_ENTRY = 'src/index.ts'

/** The reason an entry path is not acceptable, or null. */
export function validateEntry(value: string): string | null {
    return value.trim() === '' ? 'the entry is the path of the source file to bundle' : null
}

export async function entry(): Promise<string> {
    const { result } = await inquirer.prompt<{ result: string }>({
        name: 'result',
        message: 'Entry file to bundle:',
        default: DEFAULT_ENTRY,
        validate: (value: string) => validateEntry(value) ?? true,
    })
    return result.trim()
}
