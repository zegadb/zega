import { nanoid } from 'nanoid'
import { greeting } from 'edge-greeting'

import { shout } from './shout'

export default {
    async fetch(request: Request): Promise<Response> {
        const id = nanoid(12)
        return new Response(shout(`${greeting()} ${new URL(request.url).pathname} ${id}`))
    },
}
