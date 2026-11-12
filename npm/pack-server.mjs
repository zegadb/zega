// The npm packages of a release (APS 13: npm is zega's channel; APS 9: they
// are copies of the release):
// - @zegadb/server-<os>-<cpu>: that platform's `zega-server` and `zega` (the
//   CLI) executables, no install scripts, no dependencies.
// - zegadb: a Node launcher for `zega`, listing the platform packages as
//   optionalDependencies at exactly this version; npm installs the one whose
//   `os`/`cpu` match. Packed last, since it depends on them.
// Nothing here compiles: the executables are the ones the release workflow
// built and stored in R2, and every byte is checked against the release's own
// manifest.json before it goes into a package.
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
  { id: 'darwin-arm64', os: 'darwin', cpu: 'arm64', release: 'darwin-arm64', bin: 'zega-server', cli: 'zega' },
  { id: 'darwin-x64', os: 'darwin', cpu: 'x64', release: 'darwin-x64', bin: 'zega-server', cli: 'zega' },
  { id: 'linux-x64', os: 'linux', cpu: 'x64', release: 'linux-x64', bin: 'zega-server', cli: 'zega' },
  // win32-x64 is paused with the release's windows-x64 leg (scripts/channels.py NATIVE_PLATFORMS):
  // { id: 'win32-x64', os: 'win32', cpu: 'x64', release: 'windows-x64', bin: 'zega-server.exe', cli: 'zega.exe' },
];

const root = resolve(import.meta.dirname, '..');

function artifactName(platform, command = 'artifact-name') {
  return execFileSync('python3', ['scripts/channels.py', command, platform.release], { cwd: root, encoding: 'utf8' }).trim();
}

/** The release's bytes of `artifact`, refused unless the manifest lists them with that sha256. */
async function releaseBytes(releaseDir, manifest, artifact) {
  const bytes = await readFile(resolve(releaseDir, artifact));
  const expected = manifest.artifacts?.[artifact];
  assert.ok(expected, `${artifact} is not in the release manifest`);
  assert.equal(sha256(bytes), expected, `${artifact} does not match the release manifest checksum`);
  return bytes;
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
    const name = packageName(platform);
    const directory = resolve(outDir, 'staging', platform.id);
    await mkdir(resolve(directory, 'bin'), { recursive: true });
    for (const [file, artifact] of [[platform.bin, artifactName(platform)], [platform.cli, artifactName(platform, 'cli-artifact-name')]]) {
      const target = resolve(directory, 'bin', file);
      await writeFile(target, await releaseBytes(releaseDir, manifest, artifact));
      await chmod(target, 0o755);
    }
    await writeFile(resolve(directory, 'LICENSE'), license);
    await writeFile(resolve(directory, 'README.md'), `# ${name}\n\nThe \`zega\` and \`zega-server\` executables for ${platform.os} ${platform.cpu}. They are installed for you by the \`zegadb\` package; install that instead.\n\nSource: https://github.com/zegadb/zega\n`);
    await writeFile(resolve(directory, 'package.json'), JSON.stringify({
      name,
      version,
      description: `The zega (CLI) and zega-server (Zega database server) executables for ${platform.os} ${platform.cpu}.`,
      os: [platform.os],
      cpu: [platform.cpu],
      files: [`bin/${platform.bin}`, `bin/${platform.cli}`],
      license: 'Apache-2.0',
      repository: { type: 'git', url: 'git+https://github.com/zegadb/zega.git', directory: 'npm' },
      homepage: 'https://github.com/zegadb/zega#readme',
      bugs: 'https://github.com/zegadb/zega/issues',
      publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' },
    }, null, 2) + '\n');

    const [pack] = JSON.parse(execFileSync('npm', ['pack', directory, '--json', '--ignore-scripts', '--pack-destination', outDir], { encoding: 'utf8' }));
    assert.deepEqual(pack.files.map(file => file.path).sort(), ['LICENSE', 'README.md', `bin/${platform.bin}`, `bin/${platform.cli}`, 'package.json'].sort(), `Unexpected tarball contents for ${name}`);
    packed.push({ name, version, filename: pack.filename });
  }
  packed.push(await packLauncher(version, outDir, license));
  await rm(resolve(outDir, 'staging'), { recursive: true, force: true });
  await writeFile(resolve(outDir, 'server-packs.json'), JSON.stringify(packed, null, 2) + '\n');
  return packed;
}

/** `zegadb`: npm/zegadb/zega.js with this list of platforms, and the platform packages at `version`. */
async function packLauncher(version, outDir, license) {
  const directory = resolve(outDir, 'staging', 'zegadb');
  await mkdir(resolve(directory, 'bin'), { recursive: true });
  const platforms = Object.fromEntries(PLATFORMS.map(platform => [`${platform.os}-${platform.cpu}`, platform.id]));
  const template = await readFile(resolve(root, 'npm', 'zegadb', 'zega.js'), 'utf8');
  assert.equal(template.split('__PLATFORMS__').length, 2, 'npm/zegadb/zega.js must name __PLATFORMS__ once');
  await writeFile(resolve(directory, 'bin', 'zega.js'), template.replace('__PLATFORMS__', JSON.stringify(platforms)));
  await chmod(resolve(directory, 'bin', 'zega.js'), 0o755);
  await writeFile(resolve(directory, 'LICENSE'), license);
  await copyFile(resolve(root, 'npm', 'zegadb', 'README.md'), resolve(directory, 'README.md'));
  await writeFile(resolve(directory, 'package.json'), JSON.stringify({
    name: 'zegadb',
    version,
    description: 'zega: run Zega locally, build and deploy with Zega Cloud.',
    bin: { zega: 'bin/zega.js' },
    files: ['bin/zega.js'],
    engines: { node: '>=18' },
    optionalDependencies: Object.fromEntries(PLATFORMS.map(platform => [packageName(platform), version])),
    license: 'Apache-2.0',
    repository: { type: 'git', url: 'git+https://github.com/zegadb/zega.git', directory: 'cli' },
    homepage: 'https://zega.dev',
    bugs: 'https://github.com/zegadb/zega/issues',
    publishConfig: { access: 'public', registry: 'https://registry.npmjs.org/' },
  }, null, 2) + '\n');
  const [pack] = JSON.parse(execFileSync('npm', ['pack', directory, '--json', '--ignore-scripts', '--pack-destination', outDir], { encoding: 'utf8' }));
  assert.deepEqual(pack.files.map(file => file.path).sort(), ['LICENSE', 'README.md', 'bin/zega.js', 'package.json'], 'Unexpected tarball contents for zegadb');
  return { name: 'zegadb', version, filename: pack.filename };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const [releaseDir, version, outDir] = process.argv.slice(2);
  assert.ok(releaseDir && version && outDir, 'usage: node npm/pack-server.mjs <release-dir> <npm-version> <out-dir>');
  for (const pack of await packServer(releaseDir, version, outDir)) console.log(`${pack.name}@${pack.version}: ${pack.filename}`);
}
