// The per-platform packages for the `zega-server` executable:
// @zegadb/server-<os>-<cpu>, one binary each, no install scripts, no
// dependencies. The future `zegadb` CLI package lists them as
// optionalDependencies at exactly this version; npm installs the one whose
// `os`/`cpu` match. Nothing here compiles: the binaries are the ones the
// release workflow built and stored in R2, and every byte is checked against
// the release's own manifest.json before it goes into a package.
//
//   node npm/pack-server.mjs <release-dir> <npm-version> <out-dir>
//
// <release-dir> is a downloaded zega-releases/<tag>/ directory. Writes
// <out-dir>/<package>-<version>.tgz for each platform and
// <out-dir>/server-packs.json (package name, version, file name, in the order
// they are published).
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { chmod, copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

// `id`, `os` and `cpu` are npm's vocabulary (process.platform / process.arch):
// Windows is `win32` here. `release` is the release workflow's platform name
// (`windows`), and the artifact file name comes from the one function that
// makes it, scripts/channels.py native_artifact(), not from a copy of it here.
export const PLATFORMS = [
  { id: 'darwin-arm64', os: 'darwin', cpu: 'arm64', release: 'darwin-arm64', bin: 'zega-server' },
  { id: 'darwin-x64', os: 'darwin', cpu: 'x64', release: 'darwin-x64', bin: 'zega-server' },
  { id: 'linux-x64', os: 'linux', cpu: 'x64', release: 'linux-x64', bin: 'zega-server' },
  { id: 'win32-x64', os: 'win32', cpu: 'x64', release: 'windows-x64', bin: 'zega-server.exe' },
];

const root = resolve(import.meta.dirname, '..');

function artifactName(platform) {
  return execFileSync('python3', ['scripts/channels.py', 'artifact-name', platform.release], { cwd: root, encoding: 'utf8' }).trim();
}

export function packageName(platform) {
  return `@zegadb/server-${platform.id}`;
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

export async function packServer(releaseDir, version, outDir) {
  assert.match(version, /^\d+\.\d+\.\d+(-canary\.[0-9a-f]{7})?$/, 'version must be X.Y.Z or X.Y.Z-canary.<sha7>');
  const manifest = JSON.parse(await readFile(resolve(releaseDir, 'manifest.json'), 'utf8'));
  const license = await readFile(resolve(root, 'LICENSE'));
  await rm(outDir, { recursive: true, force: true });
  await mkdir(outDir, { recursive: true });
  const packed = [];
  for (const platform of PLATFORMS) {
    const artifact = artifactName(platform);
    const source = resolve(releaseDir, artifact);
    const bytes = await readFile(source);
    const expected = manifest.artifacts?.[artifact];
    assert.ok(expected, `${artifact} is not in the release manifest`);
    assert.equal(sha256(bytes), expected, `${artifact} does not match the release manifest checksum`);

    const name = packageName(platform);
    const directory = resolve(outDir, 'staging', platform.id);
    await mkdir(resolve(directory, 'bin'), { recursive: true });
    const target = resolve(directory, 'bin', platform.bin);
    await copyFile(source, target);
    await chmod(target, 0o755);
    await writeFile(resolve(directory, 'LICENSE'), license);
    await writeFile(resolve(directory, 'README.md'), `# ${name}\n\nThe \`zega-server\` executable for ${platform.os} ${platform.cpu}. It is installed for you by the \`zegadb\` command line package; install that instead.\n\nSource: https://github.com/zegadb/zega\n`);
    await writeFile(resolve(directory, 'package.json'), JSON.stringify({
      name,
      version,
      description: `The zega-server executable (Zega graph database server) for ${platform.os} ${platform.cpu}.`,
      os: [platform.os],
      cpu: [platform.cpu],
      files: [`bin/${platform.bin}`],
      license: 'Apache-2.0',
      repository: { type: 'git', url: 'git+https://github.com/zegadb/zega.git', directory: 'npm' },
      homepage: 'https://github.com/zegadb/zega#readme',
      bugs: 'https://github.com/zegadb/zega/issues',
      publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' },
    }, null, 2) + '\n');

    const [pack] = JSON.parse(execFileSync('npm', ['pack', directory, '--json', '--ignore-scripts', '--pack-destination', outDir], { encoding: 'utf8' }));
    assert.deepEqual(pack.files.map(file => file.path).sort(), ['LICENSE', 'README.md', `bin/${platform.bin}`, 'package.json'].sort(), `Unexpected tarball contents for ${name}`);
    packed.push({ name, version, filename: pack.filename });
  }
  await rm(resolve(outDir, 'staging'), { recursive: true, force: true });
  await writeFile(resolve(outDir, 'server-packs.json'), JSON.stringify(packed, null, 2) + '\n');
  return packed;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [releaseDir, version, outDir] = process.argv.slice(2);
  assert.ok(releaseDir && version && outDir, 'usage: node npm/pack-server.mjs <release-dir> <npm-version> <out-dir>');
  for (const pack of await packServer(releaseDir, version, outDir)) console.log(`${pack.name}@${pack.version}: ${pack.filename}`);
}
