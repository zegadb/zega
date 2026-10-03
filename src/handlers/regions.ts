import openapi from '../../openapi/openapi.json'
import type { Session } from '@/handlers/session'
import type { RegionChoice } from '@/questions'
import { NetworkFailure } from '@/utils/errors'
import { write } from '@/utils/output'

/**
 * The regions a project can be placed in. With credentials, the live list from
 * the API (the authority, with each region's city); without, or when the API
 * cannot be reached, the ids in the OpenAPI document this CLI was built from.
 */
export async function regionChoices(session: Session | null): Promise<RegionChoice[]> {
    if (session !== null) {
        try {
            const { json } = await session.client.get('/v1/regions')
            return json.regions.map((r) => ({ id: r.id, label: `${r.id}  ${r.city}, ${r.country}` }))
        } catch (error) {
            if (!(error instanceof NetworkFailure)) throw error
            await write('stderr', `zega: ${error.message}; checking the region against the built-in list\n`)
        }
    }
    return builtInRegions().map((id) => ({ id, label: id }))
}

function builtInRegions(): string[] {
    return openapi.components.schemas.Region.properties.id.enum
}
