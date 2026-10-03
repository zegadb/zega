import inquirer from 'inquirer'

export type RegionChoice = { id: string; label: string }

/** The reason a region is not one of the choices, or null. */
export function validateRegion(value: string, choices: RegionChoice[]): string | null {
    if (choices.some((choice) => choice.id === value)) return null
    return `${JSON.stringify(value)} is not a region; the regions are ${choices.map((choice) => choice.id).join(', ')}`
}

export async function region(choices: RegionChoice[]): Promise<string> {
    const { result } = await inquirer.prompt<{ result: string }>({
        type: 'select',
        name: 'result',
        message: 'Region:',
        choices: choices.map((choice) => ({ name: choice.label, value: choice.id })),
    })
    return result
}
