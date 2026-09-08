import net from 'node:net';
import process from 'node:process';

if (process.env.STRAYD_FIXTURE_TITLE) process.title = process.env.STRAYD_FIXTURE_TITLE;
const server = net.createServer((socket) => {
  // A discovery probe may close before consuming the response. Its reset must
  // not take down the listening fixture or change subsequent scan results.
  socket.on('error', () => socket.destroy());
  socket.end('strayd-fixture\n');
});
server.listen(0, '127.0.0.1', () => {
  console.log(JSON.stringify({ event: 'ready', pid: process.pid, ports: [server.address().port], children: [] }));
});
process.stdin.resume();
process.stdin.on('end', () => server.close(() => process.exit(0)));
const watchdog = setTimeout(() => process.exit(0), 120_000);
watchdog.unref?.();
