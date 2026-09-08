import { mkdir, appendFile } from 'node:fs/promises';
await mkdir('.test-env', { recursive: true });
const enabled = process.env.STRAYD_SPECIAL_RUNNERS === 'enabled';
const report = {
  status: enabled ? 'scheduled' : 'not-requested',
  reason: enabled
    ? 'Three optional runner jobs provide systemd, WSL and desktop results.'
    : 'Optional systemd / WSL / desktop runners are not enabled; application mocks run in native CI.',
  application_coverage: 'native-fixture',
  dedicated_jobs: enabled ? 'scheduled' : 'not-requested',
};
await Bun.write('.test-env/special-availability.json', JSON.stringify(report, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) await appendFile(process.env.GITHUB_STEP_SUMMARY, `### Optional system environments\n\n${report.reason}\n`);
console.log(JSON.stringify(report));
