#!/usr/bin/env node

'use strict';

const { existsSync } = require('node:fs');
const { join } = require('node:path');
const { spawnSync } = require('node:child_process');

const supportedBinaries = {
  'linux-x64': 'strayd-linux-x64',
  'linux-arm64': 'strayd-linux-arm64',
  'win32-x64': 'strayd-win32-x64.exe',
  'win32-arm64': 'strayd-win32-arm64.exe',
  'darwin-x64': 'strayd-darwin-x64',
  'darwin-arm64': 'strayd-darwin-arm64',
};
const platformKey = `${process.platform}-${process.arch}`;
const binaryName = supportedBinaries[platformKey];

if (!binaryName) {
  console.error(
    `strayd 暂不支持 ${platformKey}；当前包支持 Windows、Linux 与 macOS 的 x64/arm64。`,
  );
  process.exit(1);
}

const binaryPath = join(__dirname, '..', 'native', binaryName);
if (!existsSync(binaryPath)) {
  console.error(`strayd 原生程序缺失：${binaryPath}`);
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
  windowsHide: false,
});

if (result.error) {
  console.error(`strayd 启动失败：${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  console.error(`strayd 被信号 ${result.signal} 终止`);
  process.exit(1);
}
process.exit(result.status ?? 1);
