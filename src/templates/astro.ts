import { AUTH_LINE, DEV_VARS_EXAMPLE, GRAPH_KEY_VAR, GRAPH_URL_VAR, RUNTIME, gitignore, json, text, zegaDependencies } from '@/templates/shared'
import { ASTRO, ASTRO_CLOUDFLARE, TYPESCRIPT, WRANGLER } from '@/templates/versions'
import type { Context, Starter } from '@/templates'

/** What `zega build` and `bun run build` run. */
export const ASTRO_BUILD = ['astro', 'build']

/** An Astro app on the Workers runtime (the Cloudflare adapter). */
export function astro(context: Context): Starter {
    const base = zegaDependencies(context.cli)
    return {
        files: {
            'package.json': json({
                name: context.packageName,
                private: true,
                type: 'module',
                scripts: { dev: 'zega dev', build: ASTRO_BUILD.join(' ') },
                dependencies: { ...base.dependencies, astro: ASTRO, '@astrojs/cloudflare': ASTRO_CLOUDFLARE },
                devDependencies: { ...base.devDependencies, typescript: TYPESCRIPT, wrangler: WRANGLER },
            }),
            'astro.config.mjs': text(`
import cloudflare from '@astrojs/cloudflare'
import { defineConfig } from 'astro/config'

// Server-rendered, with the images left as they are: Zega Cloud gives an edge function no bindings.
export default defineConfig({ output: 'server', adapter: cloudflare({ imageService: 'passthrough' }) })
`),
            'tsconfig.json': json({
                extends: 'astro/tsconfigs/strict',
                include: ['.astro/types.d.ts', '**/*'],
                exclude: ['dist'],
            }),
            'src/pages/index.astro': text(`
---
import { env } from 'cloudflare:workers'
import { connect } from '@zegadb/lib/client'

// Edge function deployed to Zega Cloud. Render on every request on the server, so the graph key never reaches the browser.
export const prerender = false

const graph = connect({ url: env.${GRAPH_URL_VAR}, key: env.${GRAPH_KEY_VAR} })
// Replace this with a type your graph has.
const people = await graph.query('{ Person { name } }')
---

<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>${context.name.replace(/[<>&]/g, '')}</title>
  </head>
  <body>
    <h1>${context.name.replace(/[<>{}&]/g, '')}</h1>
    <pre>{JSON.stringify(people, null, 2)}</pre>
  </body>
</html>
`),
            '.dev.vars.example': DEV_VARS_EXAMPLE,
            '.gitignore': gitignore('.astro', '.wrangler'),
            'README.md': text(`
# ${context.name}

An Astro app for [Zega Cloud](https://zega.dev). \`src/pages/index.astro\` reads from a graph on the server with \`@zegadb/lib\`.

\`\`\`
bun install
cp .dev.vars.example .dev.vars   # then fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}
bun run dev                      # astro dev, with .dev.vars in its environment
bun run build                    # the server entry and its static assets
\`\`\`

What \`zega build\` runs, what it produces and the runtime flags are in \`zega.json\`. Run \`zega deploy\` to build and deploy the app to a matching Zega Cloud project.

${AUTH_LINE}
`),
        },
        config: {
            kind: 'app',
            framework: 'astro',
            build: { command: ASTRO_BUILD, worker: 'dist/server/entry.mjs', assets: 'dist/client' },
            runtime: RUNTIME,
        },
        next: [
            'bun install',
            `copy .dev.vars.example to .dev.vars and fill in ${GRAPH_URL_VAR} and ${GRAPH_KEY_VAR}`,
            'bun run dev',
            'zega deploy',
        ],
    }
}
