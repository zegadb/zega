// A Zega Cloud function: a Workers-style module that talks to its graph.
// check-consumers.mjs bundles this against the installed tarball, the way
// `zega build` does, and fails if the engine or the wasm comes with it.
import { connect, zql, ZegaError } from '@zegadb/lib/client';

interface Env {
  ZEGA_GRAPH_URL: string;
  ZEGA_GRAPH_KEY: string;
}

const schema = 'type Person { name: String }';

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const zega = connect({ url: env.ZEGA_GRAPH_URL, key: env.ZEGA_GRAPH_KEY, schema });
    const name = new URL(request.url).searchParams.get('name');
    try {
      if (request.method === 'POST' && name) {
        return Response.json(await zega.mutate(zql`mutation { Person(name: ${name}) { name } }`));
      }
      const people = await zega.query<{ name: string }[]>('{ Person { name } }', { signal: AbortSignal.timeout(5000) });
      return Response.json(people);
    } catch (error) {
      if (error instanceof ZegaError) return Response.json({ code: error.code, message: error.message }, { status: error.status || 502 });
      throw error;
    }
  },
};
