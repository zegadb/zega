import inquirer from 'inquirer'

/** Whether to replace what is there: one file by name, several by count. */
export async function overwrite(files: string[]): Promise<boolean> {
    const { result } = await inquirer.prompt<{ result: boolean }>({
        type: 'confirm',
        name: 'result',
        message: files.length === 1 ? `${files[0]} exists. Replace it?` : `${files.join(', ')} exist. Replace them?`,
        default: false,
    })
    return result
}
