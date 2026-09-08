import { mkdir, appendFile } from 'node:fs/promises';
await mkdir('.test-env', { recursive: true });
const enabled = process.env.STRAYD_SPECIAL_RUNNERS === 'enabled';
const report = { status: enabled ? 'scheduled' : 'blocked', reason: enabled ? 'Four dedicated runner jobs provide the actual systemd, WSL, desktop and versioned application results.' : 'Dedicated systemd / WSL / desktop / real-application runners are not connected.', real_app_coverage: 'requires-real-app-job', dedicated_jobs: enabled ? 'scheduled' : 'unavailable' };
await Bun.write('.test-env/special-availability.json', JSON.stringify(report, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, `### Special environment coverage\n\n**Not fully verified.** ${report.reason}\n`);
console.log(JSON.stringify(report));
// Missing required infrastructure must not make the extended workflow green.
process.exit(enabled ? 0 : 1);
