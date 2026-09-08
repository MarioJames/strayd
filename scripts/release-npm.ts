import { createHash } from 'node:crypto';
import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { parseArgs } from 'node:util';

export const repository = 'MarioJames/strayd';
export const registry = 'https://registry.npmjs.org/';
export const nativeFiles = [
  'strayd-darwin-arm64', 'strayd-darwin-x64', 'strayd-linux-arm64',
  'strayd-linux-x64', 'strayd-win32-arm64.exe', 'strayd-win32-x64.exe',
];
const root = resolve(import.meta.dir, '..');

export function validateVersions(versions: string[], ref?: string) {
  const version = versions[0];
  if (!version || !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error('Release requires a stable X.Y.Z version');
  if (versions.some(item => item !== version)) throw new Error('Workspace, npm, Cargo manifest and lockfile versions must match');
  if (ref?.startsWith('refs/tags/') && ref !== `refs/tags/v${version}`) throw new Error(`Tag must be v${version}`);
  return version;
}

export function assertCiPublisher(env: NodeJS.ProcessEnv, version: string) {
  const ref = `refs/tags/v${version}`;
  if (env.GITHUB_ACTIONS !== 'true' || env.GITHUB_REPOSITORY !== repository || env.GITHUB_REF !== ref
      || env.GITHUB_ACTOR !== 'MarioJames' || env.GITHUB_TRIGGERING_ACTOR !== 'MarioJames'
      || !['push', 'workflow_dispatch'].includes(env.GITHUB_EVENT_NAME ?? '')
      || env.GITHUB_WORKFLOW_REF !== `${repository}/.github/workflows/build-npm.yml@${ref}`) {
    throw new Error('Automatic publishing requires the upstream release workflow on its matching version tag');
  }
  if (!env.ACTIONS_ID_TOKEN_REQUEST_URL || !env.ACTIONS_ID_TOKEN_REQUEST_TOKEN) throw new Error('Missing GitHub OIDC permission: id-token: write');
  if (env.NODE_AUTH_TOKEN || env.NPM_TOKEN || env.NPM_CONFIG_TOKEN || env.npm_config_token) throw new Error('CI publishing uses OIDC; remove static npm token environment variables');
}

export function validateArchive(manifest: { name?: string; version?: string; private?: boolean; repository?: { url?: string } }, entries: string[], version: string) {
  if (manifest.name !== 'strayd' || manifest.version !== version || manifest.private) throw new Error(`Expected public strayd@${version}`);
  if (manifest.repository?.url !== `git+https://github.com/${repository}.git`) throw new Error('Package repository does not match the trusted publisher');
  const binaries = entries.filter(entry => entry.startsWith('package/native/') && !entry.endsWith('/')).sort();
  const expected = nativeFiles.map(name => `package/native/${name}`).sort();
  if (JSON.stringify(binaries) !== JSON.stringify(expected)) throw new Error('Release archive must contain exactly the six supported native binaries');
}

export function alreadyPublished(remote: { dist?: { integrity?: string } } | null, integrity: string) {
  if (remote === null) return false;
  if (remote.dist?.integrity !== integrity) throw new Error('This version already exists with different bytes; bump the version instead of overwriting it');
  return true;
}

async function command(args: string[]) {
  const child = Bun.spawn(args, { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  if (code !== 0) throw new Error(`${args[0]} exited ${code}: ${stderr}`);
  return stdout;
}

async function releaseVersion() {
  const workspace = await Bun.file(join(root, 'package.json')).json();
  const npm = await Bun.file(join(root, 'npm/strayd/package.json')).json();
  const cargo = Bun.TOML.parse(await Bun.file(join(root, 'crates/port-deck-cli/Cargo.toml')).text()) as { package: { version: string } };
  const lock = Bun.TOML.parse(await Bun.file(join(root, 'Cargo.lock')).text()) as { package: { name: string; version: string }[] };
  const locked = lock.package.find(item => item.name === 'port-deck-cli');
  return validateVersions([workspace.version, npm.version, cargo.package.version, locked?.version ?? ''], process.env.GITHUB_REF);
}

async function publishedVersion(version: string) {
  const response = await fetch(`${registry}strayd/${version}`, { signal: AbortSignal.timeout(20000) });
  if (response.status === 404) return null;
  if (!response.ok) throw new Error(`Cannot inspect npm version: HTTP ${response.status}`);
  return await response.json() as { dist?: { integrity?: string } };
}

async function main() {
  const { values, positionals } = parseArgs({ args: process.argv.slice(2), allowPositionals: true, options: {
    validate: { type: 'boolean' }, check: { type: 'boolean' }, verify: { type: 'boolean' }, publish: { type: 'boolean' },
  } });
  if (Object.values(values).filter(Boolean).length !== 1) throw new Error('Choose --validate, --check, --verify or --publish');
  const version = await releaseVersion();
  if (values.validate) { console.log(`Validated release metadata: strayd@${version}`); return; }
  if (values.publish) assertCiPublisher(process.env, version);
  const directory = resolve(positionals[0] ?? join(root, 'dist/npm'));
  const archives = (await readdir(directory)).filter(name => name.endsWith('.tgz'));
  if (archives.length !== 1) throw new Error('Expected exactly one .tgz release artifact');
  const artifact = join(directory, archives[0]);
  const manifest = JSON.parse(await command(['tar', '-xOf', artifact, 'package/package.json']));
  const entries = (await command(['tar', '-tzf', artifact])).trim().split(/\r?\n/);
  validateArchive(manifest, entries, version);
  const integrity = 'sha512-' + createHash('sha512').update(new Uint8Array(await Bun.file(artifact).arrayBuffer())).digest('base64');
  if (values.check) { console.log(`Validated six-platform archive: strayd@${version}`); return; }
  if (alreadyPublished(await publishedVersion(version), integrity)) {
    console.log(`Verified strayd@${version}: npm already contains this exact archive`);
    return;
  }
  if (values.verify) throw new Error(`strayd@${version} is not published`);
  const child = Bun.spawn(['npm', 'publish', artifact, '--access=public', '--tag=latest', '--ignore-scripts', '--provenance', `--registry=${registry}`], {
    cwd: root, stdin: 'ignore', stdout: 'inherit', stderr: 'inherit',
  });
  if (await child.exited !== 0) throw new Error('npm publish failed; inspect the npm error above');
  for (let attempt = 0; attempt < 6; attempt++) {
    if (alreadyPublished(await publishedVersion(version), integrity)) {
      console.log(`Published and verified strayd@${version}`);
      return;
    }
    await Bun.sleep(2000);
  }
  throw new Error('npm accepted the publish but the registry version is not visible yet');
}

if (import.meta.main) await main();
