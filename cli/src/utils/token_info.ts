import type { Answer, Reply } from '@/utils/cloud_request'

type Whoami = Answer<'/v1/whoami'>

/**
 * The API's `whoami` answer without the token's `prefix` (the first characters of the secret): no part of
 * a token is ever shown, in JSON or with `--human`. Every command that prints a `token` object goes through
 * here, so a field the API adds later is the only thing that can still reach the output, never the secret's own text.
 */
export function withoutPrefix(reply: Reply<Whoami>): Reply<Whoami> {
    const { token, ...rest } = reply.json
    // Not the API's shape: leave it to be reported where it is read, not made up here.
    if (typeof token !== 'object' || token === null) return reply
    const { prefix: _prefix, ...kept } = token
    const json = { ...rest, token: kept } as unknown as Whoami
    return { json }
}
