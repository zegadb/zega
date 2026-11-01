import { randomUUID } from 'node:crypto'

export default {
    fetch() {
        return new Response(randomUUID())
    },
}
