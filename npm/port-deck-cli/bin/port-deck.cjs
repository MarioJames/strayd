#!/usr/bin/env node

'use strict';

const { existsSync } = require('node:fs');
const { join } = require('node:path');
const { spawnSync } = require('node:child_process');

const supportedBinaries = {
  'linux-x64': 'port-deck-linux-x64',
  'win32-x64': 'port-deck-win32-x64.exe',
};
const platformKey = `${process.platform}-${process.arch}`;
const binaryName = supportedBinaries[platformKey];

if (!binaryName) {
  console.error(
    `port-deck-cli 暂不支持 ${platformKey}；当前包支持 Windows x64 与 WSL/Linux x64。`,
  );
  process.exit(1);
}

const binaryPath = join(__dirname, '..', 'native', binaryName);
if (!existsSync(binaryPath)) {
  console.error(`port-deck-cli 原生程序缺失：${binaryPath}`);
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), {
  stdio: 'inherit',
  windowsHide: false,
});

if (result.error) {
  console.error(`port-deck-cli 启动失败：${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  console.error(`port-deck-cli 被信号 ${result.signal} 终止`);
  process.exit(1);
}
process.exit(result.status ?? 1);
