import { chmodSync, copyFileSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const nativeDirectory = resolve(repositoryRoot, 'npm/strayd/native');
const supportedNames: Record<string, string> = {
  'linux-x64': 'strayd-linux-x64',
  'linux-arm64': 'strayd-linux-arm64',
  'win32-x64': 'strayd-win32-x64.exe',
  'win32-arm64': 'strayd-win32-arm64.exe',
  'darwin-x64': 'strayd-darwin-x64',
  'darwin-arm64': 'strayd-darwin-arm64',
};
const platformKey = `${process.platform}-${process.arch}`;
const binaryName = supportedNames[platformKey];
if (!binaryName) {
  throw new Error(`本机构建暂不支持 ${platformKey}`);
}

const source = resolve(
  repositoryRoot,
  'target/release',
  process.platform === 'win32' ? 'strayd.exe' : 'strayd',
);
if (!existsSync(source)) throw new Error(`缺少 CLI 构建产物：${source}`);

rmSync(nativeDirectory, { recursive: true, force: true });
mkdirSync(nativeDirectory, { recursive: true });
mkdirSync(resolve(repositoryRoot, 'dist/npm'), { recursive: true });
const destination = resolve(nativeDirectory, binaryName);
copyFileSync(source, destination);
if (process.platform !== 'win32') chmodSync(destination, 0o755);

console.log(`已装箱 ${platformKey} 原生 CLI：${destination}`);
