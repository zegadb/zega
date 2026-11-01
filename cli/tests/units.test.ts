import { describe, expect, test } from 'bun:test'

import { oneToken } from '@/handlers/credentials'
import { validateName } from '@/questions/init/name'
import { validateRegion } from '@/questions/init/region'
import { normalizeApi } from '@/utils/api_address'
import { fillPath } from '@/utils/cloud_request'
import { Table, detail } from '@/utils/table'

describe('units', () => {
    test('a token is one word', () => {
        expect(oneToken('zc_abc\n')).toBe('zc_abc')
        expect(oneToken('  zc_abc  \r\n')).toBe('zc_abc')
        expect(oneToken('')).toBeNull()
        expect(oneToken('\n')).toBeNull()
        expect(oneToken('zc_abc zc_def')).toBeNull()
        expect(oneToken('zc_abc\nzc_def')).toBeNull()
    })

    test('API addresses', () => {
        expect(normalizeApi('https://cloud.zega.world/')).toBe('https://cloud.zega.world')
        expect(normalizeApi('http://127.0.0.1:8787')).toBe('http://127.0.0.1:8787')
        expect(normalizeApi('http://cloud.zega.localhost:8787')).toBe('http://cloud.zega.localhost:8787')
        for (const bad of [
            'http://cloud.zega.dev',
            'ftp://cloud.zega.dev',
            'https://user:pw@cloud.zega.dev',
            'https://cloud.zega.dev/?x=1',
            'https://cloud.zega.dev/v1',
            'cloud.zega.dev',
        ]) {
            expect(() => normalizeApi(bad)).toThrow()
        }
    })

    test('path templates are percent-encoded per segment', () => {
        expect(fillPath('/v1/graphs/{graph}/keys/{key}', { graph: 'a/b', key: 'k 1' })).toBe('/v1/graphs/a%2Fb/keys/k%201')
        expect(() => fillPath('/v1/graphs/{graph}', {})).toThrow()
    })

    test('names and regions share one validator between flags and prompts', () => {
        expect(validateName('ok')).toBeNull()
        expect(validateName('')).not.toBeNull()
        expect(validateName('a\u0007b')).not.toBeNull()
        expect(validateRegion('yyz', [{ id: 'yyz', label: 'yyz' }])).toBeNull()
        expect(validateRegion('moon', [{ id: 'yyz', label: 'yyz' }])).toContain('yyz')
    })

    test('a table aligns columns, right-aligns numbers and leaves no trailing space', () => {
        const table = new Table([{ header: 'NAME' }, { header: 'N', right: true }])
        table.row(['alpha', '5'])
        table.row(['b', '120'])
        expect(table.render()).toBe('NAME     N\nalpha    5\nb      120\n')
        expect(detail([['A', '1'], ['Long', '2']])).toBe('A     1\nLong  2\n')
    })
})
