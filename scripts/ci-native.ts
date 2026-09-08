import { readFile } from 'node:fs/promises';
import { userInfo } from 'node:os';

// Hosted Linux runners live inside a .service. Give destructive fixture tests
// their own .scope so the real scanner cannot associate them with that service.
const args = [process.execPath, ...process.argv.slice(2)];
if (args.length < 2) throw new Error('Expected a Bun script or command');
let command = args;
if (process.platform === 'linux') {
  const cgroups = await readFile('/proc/self/cgroup', 'utf8');
  if (cgroups.split('\n').some(line => line.trim().endsWith('.service'))) {
    const user = userInfo();
    if (user.uid === 0) throw new Error('Native fixture tests must run as an ordinary user');
    const environment = ['PATH', 'HOME', 'CARGO_HOME', 'RUSTUP_HOME', 'LD_LIBRARY_PATH',
      'STRAYD_EXPECT_PLATFORM', 'STRAYD_TEST_IMAGE', 'CI', 'LANG', 'LC_ALL', 'TZ']
      .flatMap(key => process.env[key] === undefined ? [] : [`${key}=${process.env[key]}`]);
    command = ['sudo', '-n', 'systemd-run', '--scope', '--quiet', '--collect',
      `--unit=strayd-test-ci-${process.pid}`, '--property=RuntimeMaxSec=1200',
      `--uid=${user.uid}`, `--gid=${user.gid}`, 'env', ...environment, ...args];
  }
}
const child = Bun.spawn(command, { stdin: 'inherit', stdout: 'inherit', stderr: 'inherit' });
process.on('SIGINT', () => child.kill('SIGINT'));
process.on('SIGTERM', () => child.kill('SIGTERM'));
process.exit(await child.exited);
