import { WRANGLER, ZEGA_LIB } from '@/templates/versions'

export const GRAPH_URL_VAR = 'ZEGA_GRAPH_URL'
export const GRAPH_KEY_VAR = 'ZEGA_GRAPH_KEY'

/** Text with a trailing newline. */
export const text = (value: string): string => value.replace(/^\n/, '').replace(/\s+$/, '') + '\n'

export const json = (value: unknown): string => JSON.stringify(value, null, 2) + '\n'

/** The dependencies every starter has: the client library, and the CLI that runs the scripts. */
export const zegaDependencies = (cli: string) => ({
    dependencies: { '@zegadb/lib': ZEGA_LIB },
    devDependencies: { zegadb: cli },
})

export const wrangler = { wrangler: WRANGLER }

/** Names only: a real value never goes in a file that is committed. */
export const DEV_VARS_EXAMPLE = text(`
# Copy this file to .env (it is git-ignored) and fill in the values.
# ${GRAPH_URL_VAR}: the graph's address. ${GRAPH_KEY_VAR}: a graph key (zk_...), made in the dashboard.
${GRAPH_URL_VAR}=
${GRAPH_KEY_VAR}=
`)

export const gitignore = (...more: string[]): string => text(['node_modules', '.env', 'dist', ...more].join('\n'))

/** What a worker on the Workers runtime needs, recorded in `zega.json` rather than in a wrangler file to keep. */
export const RUNTIME = {
    compatibilityDate: '2026-09-01',
    // Cloud adds global_fetch_strictly_public itself when it compiles an app.
    compatibilityFlags: ['nodejs_compat'],
}

/** One line in each app's README: where the auth package will go. It does not exist yet. */
export const AUTH_LINE = 'Auth: a zega auth package will go here once it exists (nothing is set up for it yet).'
