import inquirer from 'inquirer'

export async function overwrite(file: string): Promise<boolean> {
    const { result } = await inquirer.prompt<{ result: boolean }>({
        type: 'confirm',
        name: 'result',
        message: `${file} exists. Replace it?`,
        default: false,
    })
    return result
}
