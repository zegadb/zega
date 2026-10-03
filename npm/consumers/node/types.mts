import { createDatabase, type DatabaseOptions, type ZegaWasm } from '@zegadb/lib';
import init, { initSync, type InitInput } from '@zegadb/lib/wasm';

const options: DatabaseOptions = {};
const db: ZegaWasm = await createDatabase(options);
const result: string = db.run('type Person { name: String }', '{ Person { name } }');
db.free();
const input: InitInput = new Uint8Array();
void [init, initSync, input, result];

import { connect, zql, ZegaError, ZegaNetworkError, type CallOptions, type Client } from '@zegadb/lib/client';

// The client's types, checked the way a consumer would meet them (never run).
export async function clientTypes(): Promise<void> {
  const zega: Client = connect({ url: 'http://127.0.0.1:9342', key: 'zk_example', schema: 'type Person { name: String }', readMethod: 'auto' });
  const options: CallOptions = { signal: AbortSignal.timeout(1000), document: false };
  const people: { name: string }[] = await zega.query<{ name: string }[]>(zql`{ Person(name: ${'Ada'}) { name } }`, options);
  const created: unknown = await zega.mutate('mutation { Person(name: "Ada") { name } }');
  const stored: { schema: string; updatedAt: string | null } = await zega.schema();
  try {
    void [people, created, stored];
  } catch (error) {
    if (error instanceof ZegaNetworkError) {
      const code: 'network' | 'aborted' | 'timeout' = error.code;
      const status: 0 = error.status;
      void [code, status];
    } else if (error instanceof ZegaError) {
      const help: string | undefined = error.help;
      const line: number | undefined = error.location?.line;
      void [error.code === 'not_a_read', error.serverCode, error.retryAfter, help, line];
    }
  }
  // @ts-expect-error ZQL is a string
  void zega.query(5);
  // @ts-expect-error a url is required
  void connect({});
  // @ts-expect-error an object has no ZQL literal
  void zql`${{}}`;
  // @ts-expect-error readMethod is auto, query or post
  void connect({ url: 'http://x', readMethod: 'get' });
}
