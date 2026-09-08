import { createHash, randomUUID } from 'node:crypto';
import { mkdir, readdir, readFile, rm, chmod, rename } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { arch, platform, release, userInfo } from 'node:os';
import { parseArgs } from 'node:util';
import { prepareConpty } from './test-conpty';

const repo = resolve(import.meta.dir, '..');
const runs = join(repo, '.test-env/runs');
const suffix = process.platform === 'win32' ? '.exe' : '';
const runner = join(repo, 'target/debug/strayd-test-runner' + suffix);
const fixture = join(repo, 'target/debug/fixture-process' + suffix);
const strayd = join(repo, 'target/debug/strayd' + suffix);
const suites = ['smoke', 'native', 'cli', 'lifecycle', 'faults', 'tui', 'runner-interrupt', 'runtime-smoke', 'runtimes', 'replay', 'package', 'stability', 'systemd', 'permissions', 'desktop', 'wsl', 'app-mock', 'tunnel'];
const profiles = ['native', 'linux-container', 'replay', 'systemd-vm', 'permissions-container', 'wsl', 'desktop'];
const { values, positionals } = parseArgs({ args: process.argv.slice(2), allowPositionals: true, options: {
  profile: { type: 'string', default: 'native' }, suite: { type: 'string', default: 'smoke' },
  scenario: { type: 'string' }, artifact: { type: 'string' }, run: { type: 'string' },
  'timeout-ms': { type: 'string', default: '180000' }, 'no-build': { type: 'boolean' },
  'image': { type: 'string' },
} });
const action = positionals[0] ?? 'run';
if (action === 'list') { console.log(JSON.stringify({ profiles, suites }, null, 2)); process.exit(0); }
if (!['run', 'cleanup'].includes(action)) throw new Error('Use run, list or cleanup');
const id = values.run ?? `${new Date().toISOString().replace(/[:.]/g, '-')}-${randomUUID().slice(0, 8)}`;
if (!/^[a-zA-Z0-9_-]+$/.test(id)) throw new Error('Invalid run ID');
const directory = join(runs, id);
const timeout = Number(values['timeout-ms']);
if (!Number.isFinite(timeout) || timeout < 100 || timeout > 3_600_000) throw new Error('timeout-ms must be 100..3600000');
let cancelled = false;
const active = new Set<ReturnType<typeof Bun.spawn>>();
const onSignal = () => { cancelled = true; for (const child of active) child.kill(); };
process.on('SIGINT', onSignal); process.on('SIGTERM', onSignal);
let sequence = 0;
async function command(args: string[], opts: { cwd?: string; limit?: number; cleanup?: boolean } = {}) {
  if (cancelled && !opts.cleanup) throw new Error('Run cancelled');
  const log = join(directory, `${action === 'cleanup' ? 'cleanup-' : ''}${String(sequence++).padStart(3, '0')}-${args[0].split(/[\\/]/).at(-1)}.log`);
  const child = Bun.spawn(args, { cwd: opts.cwd ?? repo, env: { ...process.env, STRAYD_TEST_BUN: process.execPath }, stdin: 'ignore', stdout: Bun.file(log), stderr: Bun.file(log + '.stderr') });
  active.add(child);
  let expired = false;
  const timer = setTimeout(() => { expired = true; child.kill(); }, opts.limit ?? timeout);
  const killer = setTimeout(() => { child.kill(9); }, (opts.limit ?? timeout) + 2000);
  // Signals also get a bounded grace period, even when the command's normal timeout is long.
  const cancellation = setInterval(() => { if (cancelled && !opts.cleanup) child.kill(9); }, 2000);
  const code = await child.exited;
  clearTimeout(timer); clearTimeout(killer); clearInterval(cancellation); active.delete(child);
  const output = await Bun.file(log).text() + await Bun.file(log + '.stderr').text();
  if (cancelled && !opts.cleanup) throw new Error('Run cancelled');
  if (expired) throw new Error(`Command timed out: ${args[0]} (log ${log})`);
  return { code, output };
}
async function checked(args: string[], opts = {}) {
  const result = await command(args, opts);
  if (result.code !== 0) throw new Error(`${args[0]} exited ${result.code}: ${result.output.slice(-3000)}`);
  return result.output.trim();
}
async function cleanup() {
  const errors: string[] = [];
  if (existsSync(join(directory, 'container.json'))) {
    const containers: string[] = await Bun.file(join(directory, 'container.json')).json();
    for (const cid of containers) {
      if (!/^[a-f0-9]{64}$/.test(cid)) { errors.push('Invalid owned container ID'); continue; }
      const result = await command(['docker', 'rm', '-f', cid], { cleanup: true, limit: 15000 });
      if (result.code !== 0 && !result.output.includes('No such container')) errors.push(result.output);
    }
  }
  const outputs = [directory, ...(await readdir(directory, { withFileTypes: true })).filter(entry => entry.isDirectory()).map(entry => join(directory, entry.name))
    .filter(output => ['resources.json', 'units.json', 'container-namespace.json'].some(file => existsSync(join(output, file))))];
  for (const output of outputs) {
    if (existsSync(join(output, 'container-namespace.json'))) {
      if (existsSync(join(output, 'resources.json'))) await rename(join(output, 'resources.json'), join(output, 'container-resources.json'));
      continue;
    }
    if (existsSync(join(output, 'resources.json'))) {
      const result = await command([runner, '--cleanup', '--output', output, '--repository', repo], { cleanup: true, limit: 15000 });
      if (result.code !== 0) errors.push(result.output);
    }
    // Remove scratch data only after independently verified process cleanup.
    if (!errors.length) for (const item of await readdir(output)) {
      if (item.startsWith('scratch-')) await rm(join(output, item), { recursive: true, force: true });
    }
  }
  if (!errors.length && existsSync(join(directory, 'installed package'))) await rm(join(directory, 'installed package'), { recursive: true, force: true });
  if (errors.length) throw new Error(`Cleanup failed: ${errors.join('; ')}`);
}
if (action === 'cleanup') {
  if (!values.run || !existsSync(directory)) throw new Error('cleanup requires an existing --run ID');
  await cleanup(); console.log(`Cleaned ${id}`); process.exit(0);
}
if (existsSync(directory)) throw new Error('Run ID already exists; use a new ID or cleanup');
await mkdir(directory, { recursive: true });
const report: any = { schema_version: 1, run_id: id, profile: values.profile, requested_suite: values.suite,
  started_at: new Date().toISOString(), host: { os: platform(), arch: arch(), kernel: release(), uid: userInfo().uid },
  versions: {}, suites: [], status: 'pending', cleanup: 'pending' };
let exitCode = 1;
try {
  if (!profiles.includes(values.profile!)) throw new Error(`Unknown profile ${values.profile}`);
  const special: Record<string, string> = { 'systemd-vm': 'systemd', 'permissions-container': 'permissions', wsl: 'wsl', desktop: 'desktop' };
  const selected = [...new Set(values.profile === 'replay' ? ['replay'] : [...values.suite!.split(','), ...(special[values.profile!] ? [special[values.profile!]] : [])])];
  if (selected.some(suite => !suites.includes(suite))) throw new Error(`Unknown suite ${values.suite}`);
  if (values.profile!.endsWith('container') && selected.includes('package')) { report.status = 'blocked'; throw new Error('Package installation runs on native runners; use --profile native --suite package --artifact <tgz>'); }
  report.commit = await checked(['git', 'rev-parse', 'HEAD']);
  report.dirty = Boolean(await checked(['git', 'status', '--porcelain']));
  for (const program of ['cargo', 'rustc', 'bun', 'node']) {
    if (Bun.which(program)) report.versions[program] = await checked([program, '--version']);
  }
  if (values.profile!.endsWith('container')) {
    if (!Bun.which('docker')) { report.status = 'blocked'; throw new Error('Docker is required for the selected profile'); }
    let image = values.image;
    if (!image) {
      image = `strayd-test:${id}`;
      await checked(['docker', 'build', '-f', 'tests/environments/linux/Dockerfile', '-t', image, '.'], { limit: 1200000 });
    }
    report.image = JSON.parse(await checked(['docker', 'image', 'inspect', image]))[0].Id;
    for (const suite of selected) {
      const output = join(directory, suite); await mkdir(output, { recursive: true });
      await Bun.write(join(output, 'container-namespace.json'), JSON.stringify({ profile: values.profile }));
      if (process.platform !== 'win32') await chmod(output, 0o777);
      const cid = await checked(['docker', 'create', '--init', '--network', 'none', '--cap-drop', 'ALL',
        ...(values.profile === 'permissions-container' ? ['--user', 'root', '--env', 'STRAYD_TEST_ISOLATED=1', '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--cap-add', 'KILL'] : []),
        '--label', `strayd.test.run=${id}`, '--mount', `type=bind,src=${output},dst=/evidence`, image, '--suite', suite,
        ...(values.scenario ? ['--scenario', values.scenario] : [])]);
      const ledger = join(directory, 'container.json');
      const owned = existsSync(ledger) ? await Bun.file(ledger).json() : [];
      await Bun.write(ledger, JSON.stringify([...owned, cid]));
      await checked(['docker', 'cp', `${cid}:/app/dependency-versions.txt`, join(output, 'dependency-versions.txt')]);
      const result = await command(['docker', 'start', '-a', cid]);
      const state = JSON.parse(await checked(['docker', 'inspect', '--format', '{{json .State}}', cid]));
      const suiteReport = existsSync(join(output, 'report.json')) ? await Bun.file(join(output, 'report.json')).json() : { status: 'fail', reason: result.output.slice(-3000) };
      suiteReport.container_exit_code = state.ExitCode;
      report.suites.push(suiteReport);
      // Container PIDs belong to a different namespace; never feed its ledger to host cleanup.
      if (existsSync(join(output, 'resources.json'))) await rename(join(output, 'resources.json'), join(output, 'container-resources.json'));
      if (result.code !== 0 || state.Running || state.ExitCode !== 0 || suiteReport.status === 'fail') report.suite_error = true;
    }
  } else {
    const required = values.profile === 'systemd-vm' ? 'linux' : values.profile === 'wsl' ? 'linux' : values.profile === 'desktop' ? 'darwin' : undefined;
    if (required && process.platform !== required) { report.status = 'blocked'; throw new Error(`Profile requires a ${required} native runner`); }
    if (!values['no-build'] && !Bun.which('cargo')) { report.status = 'blocked'; throw new Error('Cargo is required to build native test artifacts'); }
    if (values['no-build'] && [runner, fixture, strayd].some(file => !existsSync(file))) { report.status = 'blocked'; throw new Error('Native artifacts are missing; run without --no-build'); }
    if (!values['no-build']) await checked(['cargo', 'build', '--locked', '-p', 'port-deck-cli', '-p', 'strayd-test-support'], { limit: 600000 });
    if (process.platform === 'win32' && selected.some(suite => ['tui', 'runner-interrupt'].includes(suite))) {
      report.conpty = await prepareConpty(runner, directory, checked);
    }
    report.artifact_sha256 = createHash('sha256').update(await readFile(strayd)).digest('hex');
    let wrapper: string | undefined;
    if (selected.includes('package')) {
      if (!values.artifact || !existsSync(resolve(values.artifact))) { report.status = 'blocked'; throw new Error('package requires --artifact <actual tgz>'); }
      const artifact = resolve(values.artifact);
      report.package_sha256 = createHash('sha256').update(await readFile(artifact)).digest('hex');
      const installation = join(directory, 'installed package'); await mkdir(installation);
      await Bun.write(join(installation, 'package.json'), '{"name":"strayd-install-test","private":true}');
      await checked(['bun', 'add', '--ignore-scripts', artifact], { cwd: installation });
      wrapper = join(installation, 'node_modules/strayd/bin/strayd.cjs');
      const nodePlatform = await checked(['node', '-p', 'process.platform + "-" + process.arch']);
      if (!/^(linux|darwin|win32)-(x64|arm64)$/.test(nodePlatform)) { report.status = 'blocked'; throw new Error(`Unsupported Node platform ${nodePlatform}`); }
      report.package_node_platform = nodePlatform;
      if (process.env.STRAYD_EXPECT_PLATFORM && nodePlatform !== process.env.STRAYD_EXPECT_PLATFORM) throw new Error(`Package runner expected ${process.env.STRAYD_EXPECT_PLATFORM}, actual Node is ${nodePlatform}`);
      const native = join(dirname(dirname(wrapper)), 'native');
      const files = await readdir(native);
      report.package_native_files = files;
      if (files.some(file => file.includes('fixture') || file.includes('runner'))) throw new Error('Test support leaked into package');
      const binary = join(native, `strayd-${nodePlatform}${nodePlatform.startsWith('win32-') ? '.exe' : ''}`);
      // Verify the installed wrapper's real missing-file path, then restore before journeys.
      await rename(binary, binary + '.held');
      try {
        const missing = await command(['node', wrapper, '--version']);
        if (missing.code !== 1 || !missing.output.includes('原生程序缺失')) throw new Error('Missing native binary contract failed');
        report.package_missing_binary = 'pass';
      } finally { await rename(binary + '.held', binary); }
    }
    for (const suite of selected) {
      const output = join(directory, suite);
      const args = [runner, '--suite', suite, '--output', output, '--repository', repo, '--fixture', fixture, '--strayd', strayd];
      if (wrapper && ['package', 'tui'].includes(suite)) args.push('--node-wrapper', wrapper);
      if (values.scenario) args.push('--scenario', values.scenario);
      const result = await command(args);
      if (existsSync(join(output, 'report.json'))) report.suites.push(await Bun.file(join(output, 'report.json')).json());
      if (result.code !== 0) report.suite_error = true;
    }
  }
  if (report.suite_error) {
    const cases = report.suites.flatMap((suite: any) => suite.cases ?? []);
    for (const item of cases.filter((item: any) => ['fail', 'blocked'].includes(item.status))) {
      console.error(`${item.id}: ${item.status}: ${String(item.reason ?? 'No reason recorded').slice(0, 6000)}`);
    }
    const failed = report.suites.some((suite: any) => !suite.cases?.length || suite.cleanup !== 'pass') || cases.some((item: any) => item.status === 'fail');
    report.status = cases.some((item: any) => item.status === 'blocked') && !failed ? 'blocked' : 'fail';
    throw new Error('One or more required suites did not pass; inspect per-case reports');
  }
  report.status = 'pass'; exitCode = 0;
} catch (error) {
  report.reason = String(error); if (cancelled) report.status = 'cancelled'; else if (report.status !== 'blocked') report.status = 'fail';
  console.error(report.reason);
} finally {
  try { await cleanup(); report.cleanup = 'pass'; }
  catch (error) { report.cleanup = String(error); report.status = 'fail'; exitCode = 1; }
  if (cancelled && report.cleanup === 'pass') { report.status = 'cancelled'; exitCode = 130; }
  report.finished_at = new Date().toISOString();
  await Bun.write(join(directory, 'report.json'), JSON.stringify(report, null, 2));
  console.log(`Report: ${join(directory, 'report.json')} (${report.status}; cleanup ${report.cleanup})`);
}
process.exit(exitCode);
