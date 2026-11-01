import path from 'path'

export default {
    fetch() {
        return new Response(path.join('a', 'b'))
    },
}
