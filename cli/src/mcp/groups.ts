import { LocalFailure } from '@/utils/errors'

/** The feature groups of `zega mcp`, to keep the number of tools an agent sees small. */
export const GROUPS = ['docs', 'account', 'graphs', 'functions', 'storage', 'local'] as const
export type Group = (typeof GROUPS)[number]

/** Every group except `storage`, which is off until asked for. */
export const DEFAULT_GROUPS: readonly Group[] = GROUPS.filter((group) => group !== 'storage')

/** The groups that talk to Zega Cloud with the API token (the others need no login). */
export const CLOUD_GROUPS: readonly Group[] = ['account', 'graphs', 'functions', 'storage']

/** `--groups docs,graphs` (or `all`): in the order the groups are listed above, each once. */
export function parseGroups(text: string): Group[] {
    const asked = text.split(',').map((part) => part.trim().toLowerCase()).filter((part) => part !== '')
    if (asked.length === 0) throw new LocalFailure(`--groups needs a list: ${GROUPS.join(', ')} (or all)`, 'invalid_flag')
    if (asked.includes('all')) return [...GROUPS]
    const unknown = asked.find((part) => !(GROUPS as readonly string[]).includes(part))
    if (unknown !== undefined) {
        throw new LocalFailure(`--groups: "${unknown}" is not a group; the groups are ${GROUPS.join(', ')} (or all)`, 'invalid_flag')
    }
    return GROUPS.filter((group) => asked.includes(group))
}
