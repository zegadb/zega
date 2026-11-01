import { LocalFailure } from '@/utils/errors'

/**
 * Validate `--api` or a stored host: an http(s) origin, plain http only for a
 * local name (a token must not cross the network in the clear), no
 * credentials, query or fragment in it. Returns it without a trailing `/`.
 */
export function normalizeApi(raw: string): string {
    let url: URL
    try {
        url = new URL(raw.trim())
    } catch {
        throw new LocalFailure(`--api ${JSON.stringify(raw)} is not a URL`)
    }
    const host = url.hostname
    const local =
        host === 'localhost' ||
        host.endsWith('.localhost') ||
        /^127(\.\d{1,3}){3}$/.test(host) ||
        host === '[::1]'
    if (url.protocol === 'http:' && !local) {
        throw new LocalFailure(
            `--api ${JSON.stringify(raw)}: a token is only sent over https (plain http is allowed for localhost)`,
        )
    }
    if (url.protocol !== 'https:' && url.protocol !== 'http:') {
        throw new LocalFailure(
            `--api ${JSON.stringify(raw)}: the scheme must be https, not ${url.protocol.replace(/:$/, '')}`,
        )
    }
    if (url.username !== '' || url.password !== '' || url.search !== '' || url.hash !== '') {
        throw new LocalFailure(
            `--api ${JSON.stringify(raw)} must be an address only: no user, password, query or fragment`,
        )
    }
    if (url.pathname !== '/' && url.pathname !== '') {
        throw new LocalFailure(`--api ${JSON.stringify(raw)} must be an address only: no path`)
    }
    return url.origin
}
