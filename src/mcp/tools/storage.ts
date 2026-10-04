import { z } from 'zod'

import * as core from '@/core'
import { call, cloudUrl } from '@/handlers/graph_http'
import { commandGlobals, json, printed } from '@/mcp/tool'
import { read, tool } from '@/mcp/tools/define'
import { LocalFailure } from '@/utils/errors'

export const storage = [
    tool({
        ...read,
        name: 'zega_buckets',
        group: 'storage',
        title: 'Storage buckets',
        description: 'List the account\'s storage buckets, or with an id show one: its address, stored bytes and spending cap.',
        input: { id: z.string().optional().describe('A bucket id (b0 and 16 characters). Leave out to list every bucket.') },
        run: async ({ id }, { globals }) => await printed(() => core.buckets(id, commandGlobals(globals))),
    }),
    tool({
        ...read,
        name: 'zega_bucket_objects',
        group: 'storage',
        title: 'Objects in a bucket',
        description:
            'List the objects in a bucket (key, size, ...), a page at a time; pass `nextCursor` from the answer as `cursor` for the next page. A listing counts on the bucket\'s uploads, listings and deletes line. A bucket is opened with a storage key (zs_...), not the API token: this server uses --storage-key-file, else (a manage token, not --read-only) it makes one for this session and revokes it when it stops.',
        input: {
            bucket: z.string().describe('A bucket id.'),
            prefix: z.string().optional().describe('Only keys that start with this.'),
            limit: z.number().int().min(1).max(1000).optional().describe('How many (default 100).'),
            cursor: z.string().optional().describe('The next page.'),
        },
        run: async ({ bucket, prefix, limit, cursor }, context) => {
            const { client } = await context.session()
            const found = (await client.get('/v1/buckets/{bucket}', { path: { bucket } })).json.bucket
            if (found.url === '') throw new LocalFailure(`bucket ${bucket} has no address`)
            const target = { url: cloudUrl(found.url), key: await context.keys.storageKey(bucket) }
            const query = new URLSearchParams()
            query.set('prefix', prefix ?? '')
            if (limit !== undefined) query.set('limit', String(limit))
            if (cursor !== undefined) query.set('cursor', cursor)
            return json(await call(target, 'GET', `/?${query.toString()}`, undefined, true))
        },
    }),
]
