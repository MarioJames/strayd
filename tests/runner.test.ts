import { test, expect } from 'bun:test';
import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
const repo = resolve(import.meta.dir, '..');
function run(args: string[]) {
  const id = `contract-${randomUUID()}`;
  const child = Bun.spawn(['bun', 'scripts/test-env.ts', 'run', '--no-build', '--run', id, ...args], { cwd: repo, stdout: 'pipe', stderr: 'pipe' });
  return { id, child, directory: join(repo, '.test-env/runs', id) };
}
async function finished(task: ReturnType<typeof run>) {
  const code = await task.child.exited;
  return { code, report: await Bun.file(join(task.directory, 'report.json')).json() };
}
async function noOwnedProcesses(directory: string, suite: string) {
  const output = join(directory, suite);
  const ledger = await Bun.file(join(output, 'resources.json')).json();
  expect(ledger.length).toBeGreaterThan(0);
  for (const identity of ledger) {
    let live = true;
    try { process.kill(identity.pid, 0); } catch { live = false; }
    if (live && process.platform === 'linux') {
      const stat = await Bun.file(`/proc/${identity.pid}/stat`).text().catch(() => '');
      live = Boolean(stat) && !stat.slice(stat.lastIndexOf(')') + 2).startsWith('Z');
    }
    expect(live).toBe(false);
  }
  expect((await readdir(output)).filter(file => file.startsWith('scratch-'))).toEqual([]);
}
test('a timeout fails the run and releases real PTY and fixture processes', async () => {
  const task = run(['--suite', 'tui', '--timeout-ms', '700']);
  const result = await finished(task);
  expect(result.code).not.toBe(0); expect(result.report.status).toBe('fail');
  expect(result.report.reason).toContain('timed out'); expect(result.report.cleanup).toBe('pass');
  await noOwnedProcesses(task.directory, 'tui');
}, 20000);
test('terminal Ctrl+C cancels a live runner and cleans its registered resources', async () => {
  const task = run(['--suite', 'runner-interrupt']);
  const result = await finished(task);
  expect(result.code).toBe(0); expect(result.report.cleanup).toBe('pass');
  expect(result.report.suites[0].cases[0].status).toBe('pass');
  expect(result.report.suites[0].cases[0].observations[0].status).toBe('cancelled');
  await noOwnedProcesses(task.directory, 'runner-interrupt');
}, 45000);
test('a missing native platform is blocked and never replaced by replay', async () => {
  const profile = process.platform === 'darwin' ? 'wsl' : 'desktop';
  const task = run(['--profile', profile, '--suite', profile]);
  const result = await finished(task);
  expect(result.code).not.toBe(0); expect(result.report.status).toBe('blocked'); expect(result.report.suites).toEqual([]);
  expect(result.report.reason).toContain('native runner'); expect(result.report.cleanup).toBe('pass');
});
test('fixture startup, readiness and assertion failures are observable and cleanly scoped', async () => {
  const task = run(['--suite', 'faults']);
  const result = await finished(task);
  expect(result.code).toBe(0); expect(result.report.cleanup).toBe('pass');
  expect(result.report.suites[0].cases.map((item: any) => item.status)).toEqual(['pass', 'pass', 'pass']);
  await noOwnedProcesses(task.directory, 'faults');
});
