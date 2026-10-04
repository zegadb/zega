import { execFileSync } from 'node:child_process';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
process.chdir(root);
const tsc = resolve('node_modules/typescript/bin/tsc');
const fixture = 'npm/consumers/client-types.mts';
const flags = ['--noEmit', '--strict', '--target', 'es2022', '--lib', 'es2022,dom,esnext.disposable'];

for (const [module, moduleResolution] of [['NodeNext', 'NodeNext'], ['ESNext', 'Bundler']]) {
  execFileSync('node', [tsc, ...flags, '--module', module, '--moduleResolution', moduleResolution, fixture], { stdio: 'inherit' });
  console.log(`client declarations (${module}/${moduleResolution}): passed`);
}
