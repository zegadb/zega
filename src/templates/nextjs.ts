import { AUTH_LINE, DEV_VARS_EXAMPLE, GRAPH_KEY_VAR, GRAPH_URL_VAR, RUNTIME, gitignore, json, text, zegaDependencies } from '@/templates/shared'
import { NEXT, OPENNEXT_CLOUDFLARE, REACT, TYPESCRIPT, TYPES_NODE, TYPES_REACT, WRANGLER } from '@/templates/versions'
import type { Context, Starter } from '@/templates'

/** What `zega build` and `bun run build` run: the OpenNext build, which runs `next build` itself (open-next.config.ts says so). */
export const NEXT_BUILD = ['opennextjs-cloudflare', 'build', '--skipWranglerConfigCheck']

/** A Next.js app on the Workers runtime (OpenNext). */
export function nextjs(context: Context): Starter {
    const base = zegaDependencies(context.cli)
    return {
        files: {
            'package.json': json({
                name: context.packageName,
                private: true,
                type: 'module',
                scripts: { dev: 'zega dev', build: NEXT_BUILD.join(' '), deploy: 'zega deploy' },
                dependencies: { ...base.dependencies, next: NEXT, react: REACT, 'react-dom': REACT },
                devDependencies: {
                    ...base.devDependencies,
                    '@opennextjs/cloudflare': OPENNEXT_CLOUDFLARE,
                    '@types/node': TYPES_NODE,
                    '@types/react': TYPES_REACT,
                    '@types/react-dom': TYPES_REACT,
                    typescript: TYPESCRIPT,
                    wrangler: WRANGLER,
                },
            }),
            'next.config.ts': text(`
import type { NextConfig } from 'next'

const config: NextConfig = {}

export default config
`),
            'open-next.config.ts': text(`
import { defineCloudflareConfig } from '@opennextjs/cloudflare'

export default {
    ...defineCloudflareConfig(),
    // \`bun run build\` is the OpenNext build, so the Next.js build it runs is named here.
    buildCommand: 'next build',
}
`),
            'tsconfig.json': json({
                compilerOptions: {
                    target: 'ES2022',
                    lib: ['dom', 'dom.iterable', 'esnext'],
                    allowJs: false,
                    skipLibCheck: true,
                    strict: true,
                    noEmit: true,
                    esModuleInterop: true,
                    module: 'esnext',
                    moduleResolution: 'bundler',
                    resolveJsonModule: true,
                    isolatedModules: true,
                    jsx: 'react-jsx',
                    incremental: true,
                    plugins: [{ name: 'next' }],
                },
                include: ['next-env.d.ts', '**/*.ts', '**/*.tsx', '.next/types/**/*.ts', '.next/dev/types/**/*.ts'],
                exclude: ['node_modules'],
            }),
            'app/layout.tsx': text(`
export const metadata = { title: '${context.name.replace(/['\\]/g, '')}' }

export default function Layout({ children }: { children: React.ReactNode }) {
    return (
        <html lang="en">
            <body>{children}</body>
        </html>
    )
}
`),
            'app/page.tsx': text(`
import { connect } from '@zegadb/lib/client'

// Read on every request, on the server: the graph's key never reaches the browser.
export const dynamic = 'force-dynamic'

export default async function Page() {
    const graph = connect({ url: process.env.${GRAPH_URL_VAR}!, key: process.env.${GRAPH_KEY_VAR}! })
    // Replace this with a type your graph has.
    const people = await graph.query('{ Person { name } }')
    return (
        <main>
            <h1>${context.name.replace(/[<>{}&]/g, '')}</h1>
            <pre>{JSON.stringify(people, null, 2)}</pre>
        </main>
    )
}
`),
            '.env.example': DEV_VARS_EXAMPLE,
            '.gitignore': gitignore('.next', '.open-next', 'next-env.d.ts'),
            'README.md': text(`
# ${context.name}

A Next.js app for [Zega Cloud](https://zega.dev), set up for the Workers runtime with OpenNext. \`app/page.tsx\` reads from a graph on the server with \`@zegadb/lib\`.

\`\`\`
bun install
cp .env.example .env   # then fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}
bun run dev                      # next dev, with .env in its environment
bun run build                    # the worker (.open-next/worker.js) and its static assets (.open-next/assets)
\`\`\`

What \`zega build\` runs, what it produces and the runtime flags are in \`zega.json\`. Run \`zega deploy\` to build and deploy the app to a matching Cloud project.

${AUTH_LINE}
`),
        },
        config: {
            kind: 'app',
            framework: 'nextjs',
            build: { command: NEXT_BUILD, worker: '.open-next/worker.js', assets: '.open-next/assets' },
            runtime: RUNTIME,
        },
        next: [
            'bun install',
            `copy .env.example to .env and fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}`,
            'bun run dev',
            'zega deploy',
        ],
    }
}
