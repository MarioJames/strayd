import { mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
const root = resolve(import.meta.dir, '../..');
const directory = join(root, '.test-env/runs', `browser-${randomUUID()}`);
await mkdir(directory, { recursive: true });
const helper = Bun.spawn([join(root, 'target/debug/strayd-test-runner'), '--suite', 'browser-setup', '--output', directory,
  '--repository', root, '--fixture', join(root, 'target/debug/fixture-process'), '--strayd', join(root, 'target/debug/strayd')],
  { stdin: 'pipe', stdout: Bun.file(join(directory, 'fixture.log')), stderr: 'inherit' });
const ready = Bun.file(join(directory, 'browser.json'));
const deadline = Date.now() + 10000;
while (!existsSync(join(directory, 'browser.json'))) {
  if (helper.exitCode !== null || Date.now() > deadline) { helper.kill(); throw new Error('Browser fixtures did not become ready'); }
  await Bun.sleep(30);
}
const metadata = await ready.json();
const sessions = new Map<string, { child: ReturnType<typeof Bun.spawn>; terminal: Bun.Terminal; bytes: string }>();
const token = randomUUID();
const server = Bun.serve<{ key: string }>({
  hostname: '127.0.0.1', port: 0,
  async fetch(request, server) {
    const url = new URL(request.url);
    if (url.pathname === '/favicon.ico') return new Response(null, { status: 204 });
    if (url.pathname === '/xterm.js') return new Response(Bun.file(join(root, 'node_modules/@xterm/xterm/lib/xterm.js')));
    if (url.pathname === '/xterm.css') return new Response(Bun.file(join(root, 'node_modules/@xterm/xterm/css/xterm.css')));
    if (url.pathname === '/socket' && url.searchParams.get('token') === token) {
      if (server.upgrade(request, { data: { key: randomUUID() } })) return;
      return new Response('Upgrade required', { status: 400 });
    }
    if (url.pathname !== '/') return new Response('Not found', { status: 404 });
    return new Response((await Bun.file(join(import.meta.dir, 'terminal.html')).text()).replace('__TOKEN__', token), { headers: { 'Content-Type': 'text/html; charset=utf-8' } });
  },
  websocket: {
    open(socket) { socket.send(JSON.stringify({ type: 'fixture', services: metadata.services })); },
    async message(socket, raw) {
      const value = JSON.parse(String(raw));
      const session = sessions.get(socket.data.key);
      if (value.type === 'start' && !session && ['en', 'zh-cn'].includes(value.language)) {
        const scan = Bun.spawn([...metadata.cli, '--no-config', 'list', '--json'], { stdout: 'pipe', stderr: 'pipe' });
        const snapshot = JSON.parse(await new Response(scan.stdout).text());
        if (await scan.exited !== 0) throw new Error('Could not isolate browser fixtures');
        const own = metadata.services.map((service: any) => service.pid);
        const hidden = snapshot.groups.flatMap((group: any) => group.services).filter((service: any) => !own.includes(service.pid)).map((service: any) => service.id);
        await Bun.write(metadata.config, 'version = 1\n[[display.hide]]\nids = ' + JSON.stringify(hidden) + '\n');
        const terminal = new Bun.Terminal({ cols: 132, rows: 40, data(_terminal, data) { socket.send(JSON.stringify({ type: 'output', data: Buffer.from(data).toString('base64') })); } });
        const child = Bun.spawn([...metadata.cli, '--config', metadata.config, '--language', value.language, 'tui', '--refresh', '0'],
          { terminal, env: { ...process.env, TERM: 'xterm-256color', COLORTERM: 'truecolor' } });
        helper.stdin.write(JSON.stringify({ register_pid: child.pid }) + '\n');
        sessions.set(socket.data.key, { terminal, child, bytes: '' });
        child.exited.then(code => socket.send(JSON.stringify({ type: 'exit', code })));
      } else if (session && value.type === 'input' && typeof value.data === 'string') session.terminal.write(value.data);
      else if (session && value.type === 'resize' && [[132, 40], [100, 18], [80, 24]].some(([cols, rows]) => value.cols === cols && value.rows === rows)) session.terminal.resize(value.cols, value.rows);
    },
    close(socket) { const session = sessions.get(socket.data.key); if (session) { session.child.kill(); session.terminal.close(); sessions.delete(socket.data.key); } },
  },
});
let shuttingDown = false;
async function close() {
  if (shuttingDown) return; shuttingDown = true;
  for (const { child, terminal } of sessions.values()) { child.kill(); terminal.close(); await child.exited; }
  sessions.clear(); server.stop(true); helper.stdin.end();
  const kill = setTimeout(() => helper.kill(9), 5000); await helper.exited; clearTimeout(kill);
  const cleanup = Bun.spawn([join(root, 'target/debug/strayd-test-runner'), '--cleanup', '--output', directory, '--repository', root], { stdout: 'inherit', stderr: 'inherit' });
  process.exit(await cleanup.exited);
}
process.on('SIGINT', close); process.on('SIGTERM', close);
setTimeout(close, 300000).unref();
console.log(`APP_URL=http://127.0.0.1:${server.port}`);
console.log(`EVIDENCE_DIR=${directory}`);
