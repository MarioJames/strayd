// CI entry. Local Agent acceptance uses browser-harness prepare and supplies its APP_URL.
import { resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
const session = `strayd-ci-${randomUUID()}`;
let url = process.env.APP_URL;
let server: ReturnType<typeof Bun.spawn> | undefined;
let journey: ReturnType<typeof Bun.spawn> | undefined;
let interrupted = false;
const cancel = () => { interrupted = true; journey?.kill(); server?.kill(); };
process.on('SIGINT', cancel); process.on('SIGTERM', cancel);
try {
  if (!url) {
    const started = Bun.spawn(['bun', 'tests/browser/server.ts'], { stdout: 'pipe', stderr: 'inherit' });
    server = started;
    const reader = started.stdout.getReader(); const decoder = new TextDecoder(); let text = '';
    const timer = setTimeout(() => server?.kill(), 15000);
    try {
      while (!url) {
        const chunk = await reader.read(); if (chunk.done) throw new Error('Browser server exited before readiness');
        text += decoder.decode(chunk.value); url = text.match(/APP_URL=(http:\/\/[^\s]+)/)?.[1];
      }
    } finally { clearTimeout(timer); reader.releaseLock(); }
  }
  journey = Bun.spawn(['bun', 'tests/browser/journey.ts'], { stdout: 'inherit', stderr: 'inherit', env: { ...process.env, APP_URL: url, AGENT_BROWSER_SESSION: session } });
  const code = await journey.exited;
  process.exitCode = interrupted ? 130 : code;
} finally {
  const close = Bun.spawn([Bun.which('agent-browser') ?? resolve('node_modules/.bin/agent-browser'), '--session', session, 'close'], { stdout: 'inherit', stderr: 'inherit' });
  await close.exited;
  if (server) { server.kill(); await server.exited; }
}
