import inquirer from 'inquirer'

import { DEFAULT_TEMPLATE, TEMPLATES, validateTemplate, type Template } from '@/templates'

export { validateTemplate }

const ABOUT: Record<Template, string> = {
    function: 'function: one Workers-style function',
    nextjs: 'nextjs: a Next.js app',
    astro: 'astro: an Astro app',
}

/** Which starter: the function first, and chosen unless another is picked. */
export async function template(): Promise<Template> {
    const { result } = await inquirer.prompt<{ result: Template }>({
        type: 'select',
        name: 'result',
        message: 'Starter:',
        default: DEFAULT_TEMPLATE,
        choices: TEMPLATES.map((name) => ({ name: ABOUT[name], value: name })),
    })
    return result
}
