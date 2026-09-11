#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, delimiter, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const server = join(repository, 'server');
const windows = process.platform === 'win32';
// The pnpm launcher may bundle Node without exposing it on the caller's PATH.
const environment = { ...process.env, PATH: `${dirname(process.execPath)}${delimiter}${process.env.PATH || ''}` };

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: server, env: environment, stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status ?? result.signal}).`);
}

function git(args) {
  const result = spawnSync('git', args, { cwd: repository, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
  return result.status === 0 ? result.stdout.trim() : '';
}

function pnpm(args) {
  // Only fixed commands are passed to cmd.exe; user paths stay in cwd/env.
  if (windows) run('cmd.exe', ['/d', '/s', '/c', `pnpm ${args.join(' ')}`]);
  else run('pnpm', args);
}

function main() {
  const options = {};
  const args = process.argv.slice(2);
  while (args.length) {
    const name = args.shift();
    if (name === '--help' || name === '-h') {
      console.log('Usage: package-server-release.sh [--version v<semver>] [--output-dir DIR] [--client-key-file FILE]');
      console.log('Build the admin console and CGO-free Windows/Linux amd64 server packages; no upload or deployment.');
      return;
    }
    if (!['--version', '--output-dir', '--client-key-file'].includes(name) || !args.length) {
      throw new Error(`Unknown or incomplete option: ${name}`);
    }
    options[name] = args.shift();
  }
  const [major, minor] = process.versions.node.split('.').map(Number);
  if (major < 22 || (major === 22 && minor < 12)) throw new Error('Node.js 22.12 or newer is required.');
  const commit = git(['rev-parse', '--short=12', 'HEAD']);
  const version = (options['--version'] || process.env.VCLOGG2_BUILD_VERSION ||
    git(['describe', '--tags', '--exact-match', '--match', 'v[0-9]*', 'HEAD']) ||
    (commit ? `0.0.0-dev+g${commit}` : '0.0.0')).replace(/^v/, '');
  const number = '(0|[1-9][0-9]*)';
  const prerelease = '(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  if (!new RegExp(`^${number}\\.${number}\\.${number}(-${prerelease}(\\.${prerelease})*)?(\\+[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$`).test(version)) {
    throw new Error(`Invalid semantic version: ${version}`);
  }
  let keyId = process.env.VCLOGG_CLIENT_KEY_ID || '';
  let publicKey = process.env.VCLOGG_CLIENT_PUBLIC_KEY || '';
  if (options['--client-key-file']) {
    const descriptor = JSON.parse(readFileSync(resolve(options['--client-key-file']), 'utf8').replace(/^\uFEFF/, ''));
    keyId = descriptor.keyId || '';
    publicKey = descriptor.publicKey || '';
    if (!keyId || !publicKey) throw new Error('The public key descriptor requires keyId and publicKey.');
  }
  if (Boolean(keyId) !== Boolean(publicKey)) throw new Error('Client key ID and public key must be provided together.');
  // config.ClientPublicKeys expects unpadded base64url, not standard base64.
  if (keyId && (!/^[A-Za-z0-9._-]+$/.test(keyId) || !/^[A-Za-z0-9_-]{43}$/.test(publicKey) || Buffer.from(publicKey, 'base64url').toString('base64url') !== publicKey)) {
    throw new Error('Invalid client key ID or Ed25519 public key.');
  }
  let linkerFlags = '-s -w';
  if (keyId) linkerFlags += ` -X vclogg/server/internal/config.BundledClientKeyID=${keyId} -X vclogg/server/internal/config.BundledClientPublicKey=${publicKey}`;

  run('go', ['version']);
  run('tar', ['--version']);
  if (!windows) run('zip', ['-v'], { stdio: 'ignore' });
  pnpm(['install', '--frozen-lockfile']);
  pnpm(['run', 'typecheck']);
  pnpm(['run', 'build:admin']);

  const output = resolve(options['--output-dir'] || join(repository, 'dist', 'server'));
  mkdirSync(output, { recursive: true });
  const temporary = mkdtempSync(join(output, '.server-release-'));
  const packages = [];
  try {
    for (const goos of ['windows', 'linux']) {
      const name = `vclogg-server-${version}-${goos}-amd64`;
      const stage = join(temporary, name);
      mkdirSync(stage);
      const executable = join(stage, goos === 'windows' ? 'vclogg-server.exe' : 'vclogg-server');
      run('go', ['build', '-mod=readonly', '-trimpath', '-ldflags', linkerFlags, '-o', executable, './cmd/vclogg-server'], {
        env: { ...environment, GOOS: goos, GOARCH: 'amd64', CGO_ENABLED: '0' },
      });
      chmodSync(executable, 0o755);
      copyFileSync(join(server, 'config.example.yaml'), join(stage, 'config.example.yaml'));
      copyFileSync(join(server, 'README.md'), join(stage, 'README.md'));
      copyFileSync(join(repository, 'LICENSE'), join(stage, 'LICENSE'));
      writeFileSync(join(stage, 'VERSION'), `${version}\n`);
      const filename = `${name}.${goos === 'windows' ? 'zip' : 'tar.gz'}`;
      const archive = join(temporary, filename);
      if (goos === 'windows') {
        if (windows) {
          run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
            "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:VCLOGG_SERVER_STAGE -DestinationPath $env:VCLOGG_SERVER_ARCHIVE"], {
            env: { ...environment, VCLOGG_SERVER_STAGE: stage, VCLOGG_SERVER_ARCHIVE: archive },
          });
        } else run('zip', ['-q', '-r', archive, name], { cwd: temporary });
      } else run('tar', ['-czf', archive, '-C', temporary, name]);
      packages.push({ filename, archive });
    }
    const checksumName = `vclogg-server-${version}-SHA256SUMS.txt`;
    const checksums = packages.map(({ filename, archive }) => `${createHash('sha256').update(readFileSync(archive)).digest('hex')}  ${filename}`).join('\n') + '\n';
    writeFileSync(join(temporary, checksumName), checksums);
    // Publish only after every target has built and packaged successfully.
    for (const { filename, archive } of [...packages, { filename: checksumName, archive: join(temporary, checksumName) }]) {
      const destination = join(output, filename);
      if (existsSync(destination)) rmSync(destination);
      renameSync(archive, destination);
      console.log(destination);
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
