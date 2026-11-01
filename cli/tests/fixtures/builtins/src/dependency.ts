import { size } from 'legacy'

export default {
    fetch() {
        return new Response(String(size('x')))
    },
}
