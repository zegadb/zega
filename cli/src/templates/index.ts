/**
 * The starters `zega init` writes: three small projects vendored here as text, so that initialising
 * never runs another generator, never prompts and never touches the network.
 *
 *   function  one Workers-style function (the default)
 *   nextjs    a Next.js app, set up for the Workers runtime with OpenNext
 *   astro     an Astro app, set up for the Workers runtime with the Cloudflare adapter
 *
 * Each returns its files, the `zega.json` keys that are its own and what to do next. The app starters
 * carry in `zega.json` what `zega build` runs and what it produces (so a deploy can read it), and the
 * runtime flags a worker needs, instead of a wrangler file to keep.
 */
import pkg from '../../package.json'
import type { ProjectConfig } from '@/handlers/project_config'
import { astro } from '@/templates/astro'
import { fn } from '@/templates/function'
import { nextjs } from '@/templates/nextjs'

export const TEMPLATES = ['function', 'nextjs', 'astro'] as const
export type Template = (typeof TEMPLATES)[number]
export const DEFAULT_TEMPLATE: Template = 'function'

export function isTemplate(value: string): value is Template {
    return (TEMPLATES as readonly string[]).includes(value)
}

/** The reason a template name is not one, or null. */
export function validateTemplate(value: string): string | null {
    return isTemplate(value) ? null : `${JSON.stringify(value)} is not a template; the templates are ${TEMPLATES.join(', ')}`
}

export type StarterInput = {
    /** The project's display name (zega.json's `name`). */
    name: string
    /** The function's cloud id, when it is known (a function starter only). */
    functionId?: string
    /** The function's source file, relative to the project (a function starter only). */
    entry: string
}

export type Starter = {
    /** Path (relative, with `/`) to content. Never `zega.json`: that is written from `config`. */
    files: Record<string, string>
    /** The keys of `zega.json` beyond `version`, `name` and `region`. */
    config: Omit<ProjectConfig, 'version' | 'name' | 'region'>
    /** What to do next, in order, for a person or an agent. */
    next: string[]
}

export function starter(template: Template, input: StarterInput): Starter {
    const context = { ...input, packageName: packageName(input.name), cli: pkg.version }
    if (template === 'nextjs') return nextjs(context)
    if (template === 'astro') return astro(context)
    return fn(context)
}

/** A project name as an npm package name: lowercase letters, digits, `-`, `_` and `.`, not starting with one of the last three. */
export function packageName(name: string): string {
    const slug = name
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9._-]+/g, '-')
        .replace(/^[-_.]+|[-_.]+$/g, '')
        .slice(0, 214)
    return slug === '' ? 'zega-app' : slug
}

export type Context = StarterInput & { packageName: string; cli: string }
