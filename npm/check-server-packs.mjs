// The per-platform server packages, built from a fixture release the way
// publish-npm.yml builds them from the real one: the right names, an exact
// version, one executable per package with the platform's os/cpu, and a hard
// refusal for bytes that are not the ones the release manifest lists.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { PLATFORMS, packServer } from './pack-server.mjs';

const root = resolve(import.meta.dirname, '..');
const base = resolve(root, '.tmp');
await mkdir(base, { recursive: true });
const work = await mkdtemp(join(base, 'server-packs-'));

function artifactName(platform) {
  return execFileSync('python3', ['scripts/channels.py', 'artifact-name', platform.release], { cwd: root, encoding: 'utf8' }).trim();
}

async function fixture(directory) {
  await mkdir(directory, { recursive: true });
  const artifacts = {};
  for (const platform of PLATFORMS) {
    const bytes = Buffer.from(`not a real ${platform.id} executable`);
    await writeFile(join(directory, artifactName(platform)), bytes);
    artifacts[artifactName(platform)] = createHash('sha256').update(bytes).digest('hex');
  }
  await writeFile(join(directory, 'manifest.json'), JSON.stringify({ artifacts }));
}

try {
  const release = join(work, 'release');
  await fixture(release);

  const version = '1.2.3-canary.abcdef0';
  const packed = await packServer(release, version, join(work, 'out'));
  assert.deepEqual(packed.map(pack => pack.name), [
    '@zegadb/server-darwin-arm64', '@zegadb/server-darwin-x64', '@zegadb/server-linux-x64', '@zegadb/server-win32-x64',
  ]);
  for (const [index, platform] of PLATFORMS.entries()) {
    const tarball = join(work, 'out', packed[index].filename);
    const listing = execFileSync('tar', ['-tvzf', tarball], { encoding: 'utf8' }).split('\n').filter(Boolean);
    const binary = listing.find(line => line.endsWith(`package/bin/${platform.bin}`));
    assert.ok(binary, `${platform.id}: no bin/${platform.bin} in the tarball`);
    assert.match(binary, /^-rwxr-xr-x/, `${platform.id}: the executable lost its mode: ${binary}`);
    const pkg = JSON.parse(execFileSync('tar', ['-xzOf', tarball, 'package/package.json'], { encoding: 'utf8' }));
    assert.equal(pkg.name, packed[index].name);
    assert.equal(pkg.version, version);
    assert.deepEqual(pkg.os, [platform.os]);
    assert.deepEqual(pkg.cpu, [platform.cpu]);
    assert.equal(pkg.scripts, undefined, 'a platform package must not run install scripts');
    assert.equal(pkg.dependencies, undefined);
    const shipped = execFileSync('tar', ['-xzOf', tarball, `package/bin/${platform.bin}`]);
    assert.equal(shipped.toString(), `not a real ${platform.id} executable`);
  }
  assert.deepEqual(JSON.parse(await readFile(join(work, 'out', 'server-packs.json'), 'utf8')), packed);

  // Bytes that are not the manifest's bytes never reach a package.
  const tampered = join(work, 'tampered');
  await fixture(tampered);
  await writeFile(join(tampered, artifactName(PLATFORMS[2])), 'different bytes');
  await assert.rejects(packServer(tampered, version, join(work, 'out-tampered')), /does not match the release manifest checksum/);

  // An artifact the manifest does not list is refused too.
  const unlisted = join(work, 'unlisted');
  await fixture(unlisted);
  const manifest = JSON.parse(await readFile(join(unlisted, 'manifest.json'), 'utf8'));
  delete manifest.artifacts[artifactName(PLATFORMS[0])];
  await writeFile(join(unlisted, 'manifest.json'), JSON.stringify(manifest));
  await assert.rejects(packServer(unlisted, version, join(work, 'out-unlisted')), /is not in the release manifest/);

  await assert.rejects(packServer(release, '1.2.3-canary-abcdef0', join(work, 'out-bad-version')), /version must be/);
  console.log(`server packages: ${packed.map(pack => pack.name).join(', ')} at ${version}; tampered and unlisted binaries refused`);
} finally {
  await rm(work, { recursive: true, force: true });
}
