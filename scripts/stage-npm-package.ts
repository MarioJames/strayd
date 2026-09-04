import { chmodSync, copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const nativeDirectory = resolve(repositoryRoot, 'npm/port-deck-cli/native');
const artifacts = [
  {
    source: resolve(repositoryRoot, 'target/port-deck-cli-linux/release/port-deck'),
    destination: resolve(nativeDirectory, 'port-deck-linux-x64'),
    executable: true,
  },
  {
    source: resolve(
      repositoryRoot,
      'target/port-deck-cli-windows/x86_64-pc-windows-msvc/release/port-deck.exe',
    ),
    destination: resolve(nativeDirectory, 'port-deck-win32-x64.exe'),
    executable: false,
  },
];

for (const artifact of artifacts) {
  if (!existsSync(artifact.source)) {
    throw new Error(`缺少 CLI 构建产物：${artifact.source}`);
  }
}

mkdirSync(nativeDirectory, { recursive: true });
mkdirSync(resolve(repositoryRoot, 'dist/npm'), { recursive: true });
for (const artifact of artifacts) {
  copyFileSync(artifact.source, artifact.destination);
  if (artifact.executable) chmodSync(artifact.destination, 0o755);
}

console.log(`已装箱 ${artifacts.length} 个原生 CLI：${nativeDirectory}`);
