import { registry, repository } from './release-npm';

// A real token exchange checks the configured trust without uploading a version.
// npm publish --dry-run can succeed with no credentials, so it is not this check.
export async function checkNpmOidc(env: NodeJS.ProcessEnv = process.env, request: (url: string | URL, options?: RequestInit) => Promise<Response> = fetch) {
  if (env.GITHUB_ACTIONS !== 'true' || env.GITHUB_REPOSITORY !== repository || env.GITHUB_REF !== 'refs/heads/main'
      || env.GITHUB_ACTOR !== 'MarioJames' || env.GITHUB_TRIGGERING_ACTOR !== 'MarioJames'
      || env.GITHUB_EVENT_NAME !== 'workflow_dispatch'
      || env.GITHUB_WORKFLOW_REF !== `${repository}/.github/workflows/build-npm.yml@refs/heads/main`) {
    throw new Error('OIDC verification must run in the upstream workflow on main');
  }
  if (!env.ACTIONS_ID_TOKEN_REQUEST_URL || !env.ACTIONS_ID_TOKEN_REQUEST_TOKEN) throw new Error('Missing GitHub OIDC permission: id-token: write');
  const url = new URL(env.ACTIONS_ID_TOKEN_REQUEST_URL);
  url.searchParams.set('audience', 'npm:registry.npmjs.org');
  const identity = await request(url, { headers: { Authorization: `Bearer ${env.ACTIONS_ID_TOKEN_REQUEST_TOKEN}` }, signal: AbortSignal.timeout(20000) });
  if (!identity.ok) throw new Error(`GitHub OIDC request failed: HTTP ${identity.status}`);
  const body = await identity.json() as { value?: string };
  if (!body.value) throw new Error('GitHub returned no OIDC identity');
  const exchange = await request(`${registry}-/npm/v1/oidc/token/exchange/package/strayd`, {
    method: 'POST', headers: { Authorization: `Bearer ${body.value}` }, signal: AbortSignal.timeout(20000),
  });
  if (!exchange.ok) throw new Error(`npm rejected the trusted publisher: HTTP ${exchange.status}; check repository, workflow, environment and allowed actions in npm settings`);
  const credential = await exchange.json() as { token?: string };
  if (!credential.token) throw new Error('npm returned no publishing credential');
  // Do not log or persist either token, including API response bodies on errors.
  console.log('GitHub OIDC → npm strayd token exchange verified; no version was published');
}

if (import.meta.main) await checkNpmOidc();
