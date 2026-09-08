import { test, expect } from 'bun:test';
import { join, resolve } from 'node:path';
import { existsSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
const image = process.env.STRAYD_TEST_IMAGE;
if (!image) throw new Error('Container contracts require STRAYD_TEST_IMAGE=<prepared test image>');
const root = resolve(import.meta.dir, '..');
for (const interrupt of [false, true]) {
  test(`${interrupt ? 'SIGINT' : 'timeout'} destroys the owned container without treating its PIDs as host PIDs`, async () => {
    const id = `container-contract-${randomUUID()}`;
    const directory = join(root, '.test-env/runs', id);
    const child = Bun.spawn(['bun', 'scripts/test-env.ts', 'run', '--profile', 'linux-container', '--suite', 'tui', '--image', image, '--run', id,
      '--timeout-ms', interrupt ? '180000' : '1000'], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
    if (interrupt) {
      const deadline = Date.now() + 10000;
      while (Date.now() < deadline) {
        const ledger = join(directory, 'tui/resources.json');
        if (existsSync(ledger) && (await Bun.file(ledger).json()).length >= 3) break;
        await Bun.sleep(30);
      }
      expect(existsSync(join(directory, 'container.json'))).toBe(true);
      child.kill('SIGINT');
    }
    expect(await child.exited).not.toBe(0);
    const report = await Bun.file(join(directory, 'report.json')).json();
    expect(report.status).toBe(interrupt ? 'cancelled' : 'fail');
    expect(report.cleanup).toBe('pass');
    const containers: string[] = await Bun.file(join(directory, 'container.json')).json();
    expect(containers.length).toBe(1);
    for (const cid of containers) {
      const inspect = Bun.spawn(['docker', 'inspect', cid], { stdout: 'ignore', stderr: 'ignore' });
      expect(await inspect.exited).not.toBe(0);
    }
    expect(existsSync(join(directory, 'tui/resources.json'))).toBe(false);
    expect((await Bun.file(join(directory, 'tui/container-resources.json')).json()).length).toBeGreaterThan(0);
    expect((await readdir(join(directory, 'tui'))).filter(file => file.startsWith('scratch-'))).toEqual([]);
  }, 25000);
}
