import { readdir } from 'node:fs/promises';
import { join } from 'node:path';
const directory = process.argv[2] ?? 'dist/npm';
const packages = (await readdir(directory)).filter(file => file.endsWith('.tgz'));
if (packages.length !== 1) throw new Error('Expected exactly one package artifact');
const child = Bun.spawn(['bun', 'scripts/test-env.ts', 'run', '--suite', 'package,tui', '--artifact', join(directory, packages[0])], { stdout: 'inherit', stderr: 'inherit' });
process.exit(await child.exited);
