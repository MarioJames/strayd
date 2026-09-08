import { mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { strict as assert } from 'node:assert';
const url = process.env.APP_URL;
if (!url) throw new Error('APP_URL must be supplied by the acceptance environment');
const evidence = resolve(process.env.BROWSER_EVIDENCE ?? '.test-env/browser-evidence');
await mkdir(evidence, { recursive: true });
const session = process.env.AGENT_BROWSER_SESSION ?? 'strayd-journey';
const binary = Bun.which('agent-browser') ?? resolve('node_modules/.bin/agent-browser');
async function browser(...args: string[]) {
  const child = Bun.spawn([binary, '--session', session, ...args, '--json'], { stdout: 'pipe', stderr: 'pipe' });
  const output = await new Response(child.stdout).text();
  const error = await new Response(child.stderr).text();
  if (await child.exited !== 0) throw new Error(`agent-browser ${args[0]}: ${error} ${output}`);
  const response = JSON.parse(output); if (!response.success) throw new Error(JSON.stringify(response)); return response.data;
}
const evaluate = async (script: string) => (await browser('eval', script)).result;
async function until(script: string, label: string) {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) { if (await evaluate(script)) return; await Bun.sleep(80); }
  throw new Error(`Browser timed out: ${label}; screen: ${await evaluate('window.straydTest.screen()')}`);
}
const results: any[] = [];
await browser('set', 'viewport', '1400', '1000');
for (const language of ['en', 'zh-cn']) {
  await browser('open', url);
  await browser('select', '#language', language);
  await until('document.querySelector("#status").textContent === "已连接"', 'socket ready');
  await browser('click', '#start');
  const started = language === 'en' ? 'Started' : '创建时间';
  const uptime = language === 'en' ? 'Uptime' : '运行时长';
  const copy = language === 'en' ? 'COPY COMMAND' : '复制命令';
  await until(`window.straydTest.screen().includes(${JSON.stringify(started)})`, 'first frame');
  const fixture = await evaluate('window.straydTest.fixture');
  assert.equal(fixture.length, 2);
  const initialTime = await evaluate(String.raw`window.straydTest.screen().split('\n').find(line=>line.includes(${JSON.stringify(uptime)}))`);
  await until(String.raw`window.straydTest.screen().split('\n').find(line=>line.includes(${JSON.stringify(uptime)})) !== ${JSON.stringify(initialTime)}`, 'uptime advance');
  await browser('press', 'End');
  await until(`window.straydTest.screen().includes('PID ${fixture[1].pid}')`, 'last resource');
  await browser('click', '[data-cols="100"]');
  for (let count = 0; count < 20 && !await evaluate(`window.straydTest.screen().includes(${JSON.stringify(copy)})`); count++) {
    await browser('press', 'PageDown'); await Bun.sleep(100);
  }
  await until(`window.straydTest.screen().includes(${JSON.stringify(copy)})`, 'scrolled copy button');
  const position = await evaluate(String.raw`(()=>{const t=window.straydTest.term;const r=document.querySelector('.xterm-screen').getBoundingClientRect();const row=window.straydTest.screen().split('\n').findIndex(line=>line.includes(${JSON.stringify(copy)}));return {x:r.x+r.width*0.75,y:r.y+(row+0.5)*r.height/t.rows}})()`);
  await browser('mouse', 'move', String(Math.round(position.x)), String(Math.round(position.y)));
  await browser('mouse', 'down'); await browser('mouse', 'up');
  await until(`document.querySelector('#clipboard').textContent === ${JSON.stringify(fixture[1].command)}`, 'full copied command');
  await browser('screenshot', join(evidence, `${language}-100x18-copy.png`));
  await browser('press', 'ArrowUp');
  await until(`window.straydTest.screen().includes('PID ${fixture[0].pid}') && window.straydTest.screen().includes(${JSON.stringify(started)})`, 'selection scroll reset');
  await browser('click', '[data-cols="80"]');
  await browser('press', 's');
  const confirmation = language === 'en' ? 'CONFIRM STOP' : '确认关闭';
  await until(`window.straydTest.screen().includes(${JSON.stringify(confirmation)})`, 'stop confirmation');
  await browser('screenshot', join(evidence, `${language}-80x24-confirm.png`));
  await browser('press', 'Escape');
  await until(`!window.straydTest.screen().includes(${JSON.stringify(confirmation)})`, 'cancel stop');
  await browser('click', '[data-cols="132"]');
  await browser('press', 'h');
  const hide = language === 'en' ? 'CREATE HIDE RULE' : '创建隐藏规则';
  await until(`window.straydTest.screen().includes(${JSON.stringify(hide)})`, 'hide dialog');
  await browser('press', 'Escape');
  await until(`!window.straydTest.screen().includes(${JSON.stringify(hide)})`, 'cancel hide');
  await browser('screenshot', join(evidence, `${language}-132x40.png`));
  await browser('press', 'q');
  await until('window.straydTest.exited === 0', 'clean terminal exit');
  const errors = await browser('errors');
  const consoleLog = await browser('console');
  await Bun.write(join(evidence, `${language}-console.json`), JSON.stringify(consoleLog, null, 2));
  await Bun.write(join(evidence, `${language}-errors.json`), JSON.stringify(errors, null, 2));
  assert.equal(errors.errors?.length ?? 0, 0, 'browser errors');
  results.push({ language, status: 'pass', sizes: [[132, 40], [100, 18], [80, 24]], copied_full_command: true, cancelled_stop: true, exited: 0 });
}
await Bun.write(join(evidence, 'journey.json'), JSON.stringify({ APP_URL: url, source: 'native-fixture', results }, null, 2));
console.log(`Browser journey passed: ${evidence}`);
