import openapi from '../types/openapi.json'
import type { Session } from '@/handlers/session'
import type { RegionChoice } from '@/questions'
import { NetworkFailure } from '@/utils/errors'
import { write } from '@/utils/output'

/**
 * The regions a project can be placed in. With credentials, the live list from
 * the API (the authority, with each region's city); without, or when the API
 * cannot be reached, the ids in the OpenAPI document this CLI was built from.
 */
export async function regionChoices(session: Session | null, human: boolean): Promise<RegionChoice[]> {
    if (session !== null) {
        try {
            const { json } = await session.client.get('/v1/regions')
            return json.regions.map((r) => ({ id: r.id, label: `${r.city} (${r.id})` }))
        } catch (error) {
            if (!(error instanceof NetworkFailure)) throw error
            if (human) await write('stderr', `zega: ${error.message}; checking the region against the built-in list\n`)
        }
    }
    return builtInRegions().map((id) => ({ id, label: `${CITY[id] ?? id} (${id})` }))
}

const CITY: Record<string, string> = { dfw: 'Dallas', ewr: 'Newark', iad: 'Washington', lax: 'Los Angeles', ord: 'Chicago', sjc: 'San Jose', yyz: 'Toronto', ams: 'Amsterdam', arn: 'Stockholm', cdg: 'Paris', fra: 'Frankfurt', lhr: 'London', nrt: 'Tokyo', sin: 'Singapore', syd: 'Sydney', gru: 'São Paulo' }

function builtInRegions(): string[] {
    return openapi.components.schemas.Region.properties.id.enum
}
