import { DEV_VARS_EXAMPLE, GRAPH_KEY_VAR, GRAPH_URL_VAR, gitignore, json, text, zegaDependencies } from '@/templates/shared'
import { TYPESCRIPT } from '@/templates/versions'
import type { Context, Starter } from '@/templates'

/** One Workers-style function that reads from a graph with the client library. */
export function fn(context: Context): Starter {
    const listed = context.functionId !== undefined
    return {
        files: {
            'package.json': json({
                name: context.packageName,
                private: true,
                type: 'module',
                scripts: { dev: 'zega dev', build: 'zega build', deploy: 'zega deploy' },
                ...zegaDependencies(context.cli),
                devDependencies: { ...zegaDependencies(context.cli).devDependencies, typescript: TYPESCRIPT },
            }),
            [context.entry]: text(`
import { connect } from '@zegadb/lib/client'

type Env = { ${GRAPH_URL_VAR}: string; ${GRAPH_KEY_VAR}: string }

export default {
    async fetch(_request: Request, env: Env): Promise<Response> {
        const graph = connect({ url: env.${GRAPH_URL_VAR}, key: env.${GRAPH_KEY_VAR} })
        // Replace this with a type your graph has.
        const people = await graph.query('{ Person { name } }')
        return Response.json(people)
    },
}
`),
            'tsconfig.json': json({
                compilerOptions: {
                    target: 'ES2022',
                    module: 'ESNext',
                    moduleResolution: 'Bundler',
                    lib: ['ES2022', 'DOM'],
                    types: [],
                    strict: true,
                    noEmit: true,
                    skipLibCheck: true,
                },
                include: ['**/*.ts'],
                exclude: ['node_modules', 'dist'],
            }),
            '.dev.vars.example': DEV_VARS_EXAMPLE,
            '.gitignore': gitignore(),
            'README.md': text(`
# ${context.name}

A [Zega Cloud](https://zega.dev) function that reads from a graph with \`@zegadb/lib\`.

\`\`\`
npm install
cp .dev.vars.example .dev.vars   # then fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}
npm run dev                      # serves ${context.entry} on http://127.0.0.1:8787
npm run build                    # bundles it into dist/
npm run deploy                   # uploads it (needs a manage token: zega login)
\`\`\`
${listed ? '' : `
The function is made in the dashboard. Then list it in \`zega.json\`: add \`"functions": [{ "id": "<function id>", "entry": "${context.entry}" }]\`.
`}`),
        },
        config: listed ? { functions: [{ id: context.functionId!, entry: context.entry }] } : {},
        next: [
            'npm install',
            `copy .dev.vars.example to .dev.vars and fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}`,
            ...(listed
                ? []
                : [`make the function in the dashboard, then add { "id": "<function id>", "entry": "${context.entry}" } under "functions" in zega.json`]),
            'npm run dev',
        ],
    }
}
