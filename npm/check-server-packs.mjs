// The npm packages, built from a fixture release the way publish-npm.yml builds
// them from the real one: the right names, an exact version, each platform's
// `zega-server` and `zega` with its os/cpu, a hard refusal for bytes that are
// not the ones the release manifest lists, and a `zegadb` launcher that Node
// alone runs.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { PLATFORMS, packServer } from './pack-server.mjs';

const root = resolve(import.meta.dirname, '..');
const base = resolve(root, '.tmp');
await mkdir(base, { recursive: true });
const work = await mkdtemp(join(base, 'server-packs-'));

function artifactName(platform, command = 'artifact-name') {
  return execFileSync('python3', ['scripts/channels.py', command, platform.release], { cwd: root, encoding: 'utf8' }).trim();
}

async function fixture(directory) {
  await mkdir(directory, { recursive: true });
  const artifacts = {};
  for (const platform of PLATFORMS) {
    for (const [command, what] of [['artifact-name', 'server'], ['cli-artifact-name', 'cli']]) {
      const bytes = Buffer.from(`not a real ${platform.id} ${what} executable`);
      await writeFile(join(directory, artifactName(platform, command)), bytes);
      artifacts[artifactName(platform, command)] = createHash('sha256').update(bytes).digest('hex');
    }
  }
  await writeFile(join(directory, 'manifest.json'), JSON.stringify({ artifacts }));
}

try {
  const release = join(work, 'release');
  await fixture(release);

  const version = '1.2.3-canary.abcdef0';
  const packed = await packServer(release, version, join(work, 'out'));
  // zegadb last: it depends on the platform packages, and they publish in this order.
  assert.deepEqual(packed.map(pack => pack.name), [
    '@zegadb/server-darwin-arm64', '@zegadb/server-darwin-x64', '@zegadb/server-linux-x64', 'zegadb',
  ]);
  for (const [index, platform] of PLATFORMS.entries()) {
    const tarball = join(work, 'out', packed[index].filename);
    const listing = execFileSync('tar', ['-tvzf', tarball], { encoding: 'utf8' }).split('\n').filter(Boolean);
    for (const file of [platform.bin, platform.cli]) {
      const binary = listing.find(line => line.endsWith(`package/bin/${file}`));
      assert.ok(binary, `${platform.id}: no bin/${file} in the tarball`);
      assert.match(binary, /^-rwxr-xr-x/, `${platform.id}: the executable lost its mode: ${binary}`);
    }
    const pkg = JSON.parse(execFileSync('tar', ['-xzOf', tarball, 'package/package.json'], { encoding: 'utf8' }));
    assert.equal(pkg.name, packed[index].name);
    assert.equal(pkg.version, version);
    assert.deepEqual(pkg.os, [platform.os]);
    assert.deepEqual(pkg.cpu, [platform.cpu]);
    assert.equal(pkg.scripts, undefined, 'a platform package must not run install scripts');
    assert.equal(pkg.dependencies, undefined);
    assert.equal(execFileSync('tar', ['-xzOf', tarball, `package/bin/${platform.bin}`]).toString(), `not a real ${platform.id} server executable`);
    assert.equal(execFileSync('tar', ['-xzOf', tarball, `package/bin/${platform.cli}`]).toString(), `not a real ${platform.id} cli executable`);
  }

  // zegadb: a launcher with no install script, depending on every platform package at exactly this version.
  const launcher = join(work, 'out', packed.at(-1).filename);
  const pkg = JSON.parse(execFileSync('tar', ['-xzOf', launcher, 'package/package.json'], { encoding: 'utf8' }));
  assert.equal(pkg.name, 'zegadb');
  assert.equal(pkg.version, version);
  assert.deepEqual(pkg.bin, { zega: 'bin/zega.js' });
  assert.equal(pkg.scripts, undefined, 'zegadb must not run install scripts');
  assert.deepEqual(pkg.optionalDependencies, Object.fromEntries(PLATFORMS.map(platform => [`@zegadb/server-${platform.id}`, version])));
  // Installed as npm installs it, Node alone runs the platform's `zega` with the arguments and its exit code.
  const host = PLATFORMS.find(platform => platform.os === process.platform && platform.cpu === process.arch);
  if (host && process.platform !== 'win32') {
    const project = join(work, 'project');
    const modules = join(project, 'node_modules');
    await mkdir(join(modules, 'zegadb'), { recursive: true });
    execFileSync('tar', ['-xzf', launcher, '-C', join(modules, 'zegadb'), '--strip-components', '1']);
    const platformDir = join(modules, '@zegadb', `server-${host.id}`);
    await mkdir(join(platformDir, 'bin'), { recursive: true });
    await writeFile(join(platformDir, 'package.json'), JSON.stringify({ name: `@zegadb/server-${host.id}`, version }));
    await writeFile(join(platformDir, 'bin', host.cli), '#!/bin/sh\necho "zega got $*"\nexit 7\n', { mode: 0o755 });
    const ran = spawnSync(process.execPath, [join(modules, 'zegadb', 'bin', 'zega.js'), 'start', '--data', 'd b'], { cwd: project, encoding: 'utf8' });
    assert.equal(ran.stdout, 'zega got start --data d b\n', ran.stderr);
    assert.equal(ran.status, 7);
    // Without the platform package it says how to fix it.
    await rm(platformDir, { recursive: true });
    const missing = spawnSync(process.execPath, [join(modules, 'zegadb', 'bin', 'zega.js')], { cwd: project, encoding: 'utf8' });
    assert.equal(missing.status, 1);
    assert.match(missing.stderr, /is not installed\. It is an optional dependency of zegadb/);
  }
  assert.deepEqual(JSON.parse(await readFile(join(work, 'out', 'server-packs.json'), 'utf8')), packed);

  // Bytes that are not the manifest's bytes never reach a package.
  const tampered = join(work, 'tampered');
  await fixture(tampered);
  await writeFile(join(tampered, artifactName(PLATFORMS[2])), 'different bytes');
  await assert.rejects(packServer(tampered, version, join(work, 'out-tampered')), /does not match the release manifest checksum/);
  const tamperedCli = join(work, 'tampered-cli');
  await fixture(tamperedCli);
  await writeFile(join(tamperedCli, artifactName(PLATFORMS[0], 'cli-artifact-name')), 'different bytes');
  await assert.rejects(packServer(tamperedCli, version, join(work, 'out-tampered-cli')), /zega-darwin-arm64 does not match the release manifest checksum/);

  // An artifact the manifest does not list is refused too.
  const unlisted = join(work, 'unlisted');
  await fixture(unlisted);
  const manifest = JSON.parse(await readFile(join(unlisted, 'manifest.json'), 'utf8'));
  delete manifest.artifacts[artifactName(PLATFORMS[0])];
  await writeFile(join(unlisted, 'manifest.json'), JSON.stringify(manifest));
  await assert.rejects(packServer(unlisted, version, join(work, 'out-unlisted')), /is not in the release manifest/);

  await assert.rejects(packServer(release, '1.2.3-canary-abcdef0', join(work, 'out-bad-version')), /version must be/);
  console.log(`npm packages: ${packed.map(pack => pack.name).join(', ')} at ${version}; the launcher runs zega; tampered and unlisted binaries refused`);
} finally {
  await rm(work, { recursive: true, force: true });
}
