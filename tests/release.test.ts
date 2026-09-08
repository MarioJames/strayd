import { expect, test } from 'bun:test';
import { alreadyPublished, assertCiPublisher, nativeFiles, repository, validateArchive, validateVersions } from '../scripts/release-npm';
import { checkNpmOidc } from '../scripts/check-npm-oidc';

test('a release requires matching stable versions and an exact tag', () => {
  expect(validateVersions(['1.2.3', '1.2.3', '1.2.3', '1.2.3'], 'refs/tags/v1.2.3')).toBe('1.2.3');
  expect(() => validateVersions(['1.2.3', '1.2.4'])).toThrow('must match');
  for (const version of ['01.2.3', '1.2.3-beta.1', 'v1.2.3']) expect(() => validateVersions([version])).toThrow('stable');
  expect(() => validateVersions(['1.2.3'], 'refs/tags/v1.2.4')).toThrow('Tag');
});

const ci: NodeJS.ProcessEnv = {
  GITHUB_ACTIONS: 'true', GITHUB_REPOSITORY: repository, GITHUB_REF: 'refs/tags/v1.2.3',
  GITHUB_ACTOR: 'MarioJames', GITHUB_TRIGGERING_ACTOR: 'MarioJames',
  GITHUB_WORKFLOW_REF: `${repository}/.github/workflows/build-npm.yml@refs/tags/v1.2.3`,
  GITHUB_EVENT_NAME: 'push', ACTIONS_ID_TOKEN_REQUEST_URL: 'https://example.invalid/token', ACTIONS_ID_TOKEN_REQUEST_TOKEN: 'test-request-token',
};
test('forks, branches, mismatched workflows and static tokens cannot publish', () => {
  expect(() => assertCiPublisher(ci, '1.2.3')).not.toThrow();
  for (const change of [{ GITHUB_REPOSITORY: 'someone/strayd' }, { GITHUB_REF: 'refs/heads/main' },
    { GITHUB_EVENT_NAME: 'pull_request' }, { GITHUB_WORKFLOW_REF: 'other.yml' },
    { ACTIONS_ID_TOKEN_REQUEST_TOKEN: '' }, { NODE_AUTH_TOKEN: 'static-token' },
    { GITHUB_ACTOR: 'collaborator' }, { GITHUB_TRIGGERING_ACTOR: 'collaborator' }]) {
    expect(() => assertCiPublisher({ ...ci, ...change }, '1.2.3')).toThrow();
  }
});

test('a release archive contains the named package and every supported native executable', () => {
  const manifest = { name: 'strayd', version: '1.2.3', repository: { url: `git+https://github.com/${repository}.git` } };
  const files = nativeFiles.map(file => `package/native/${file}`);
  expect(() => validateArchive(manifest, files, '1.2.3')).not.toThrow();
  expect(() => validateArchive(manifest, files.slice(1), '1.2.3')).toThrow('six');
  expect(() => validateArchive(manifest, [...files, 'package/native/fixture-process'], '1.2.3')).toThrow('six');
  expect(() => validateArchive({ ...manifest, version: '1.2.4' }, files, '1.2.3')).toThrow('Expected');
  expect(() => validateArchive({ ...manifest, name: 'other' }, files, '1.2.3')).toThrow('Expected');
});

test('a rerun only skips an already published version when its bytes match', () => {
  expect(alreadyPublished(null, 'sha512-exact')).toBe(false);
  expect(alreadyPublished({ dist: { integrity: 'sha512-exact' } }, 'sha512-exact')).toBe(true);
  expect(() => alreadyPublished({ dist: { integrity: 'sha512-other' } }, 'sha512-exact')).toThrow('different bytes');
  expect(() => alreadyPublished({}, 'sha512-exact')).toThrow('different bytes');
});

test('OIDC verification requires a real identity and a package token; denial fails closed', async () => {
  const env = { ...ci, GITHUB_REF: 'refs/heads/main', GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_WORKFLOW_REF: `${repository}/.github/workflows/build-npm.yml@refs/heads/main` };
  const calls: { url: string; method?: string; authorization?: string }[] = [];
  const request = (async (url: string | URL | Request, options?: RequestInit) => {
    calls.push({ url: String(url), method: options?.method, authorization: new Headers(options?.headers).get('Authorization') ?? undefined });
    return Response.json(calls.length === 1 ? { value: 'test-identity' } : { token: 'test-short-lived-credential' });
  });
  await checkNpmOidc(env, request);
  expect(new URL(calls[0].url).searchParams.get('audience')).toBe('npm:registry.npmjs.org');
  expect(calls[1]).toEqual({ url: 'https://registry.npmjs.org/-/npm/v1/oidc/token/exchange/package/strayd', method: 'POST', authorization: 'Bearer test-identity' });
  await expect(checkNpmOidc(env, async () => new Response('', { status: 403 }))).rejects.toThrow('HTTP 403');
  await expect(checkNpmOidc(env, async () => Response.json({}))).rejects.toThrow('no OIDC identity');
  let step = 0;
  await expect(checkNpmOidc(env, async () => Response.json(++step === 1 ? { value: 'identity' } : {}))).rejects.toThrow('no publishing credential');
  step = 0;
  await expect(checkNpmOidc(env, async () => ++step === 1 ? Response.json({ value: 'identity' }) : new Response('', { status: 403 }))).rejects.toThrow('npm rejected');
  await expect(checkNpmOidc(ci, request)).rejects.toThrow('on main');
});
