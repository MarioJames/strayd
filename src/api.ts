import { invoke } from '@tauri-apps/api/core';
import type { ScanSnapshot, ServiceProcess, TerminateRequest } from './types';

const mockServices: ServiceProcess[] = [
  {
    id: 'wsl:Debian:4182:90110',
    origin: 'wsl',
    distribution: 'Debian',
    pid: 4182,
    parentPid: 4129,
    ports: [3000],
    hosts: ['0.0.0.0'],
    processName: 'next-server',
    command: 'node node_modules/next/dist/bin/next dev --turbo',
    cwd: '/home/mocha/workspaces/storefront',
    projectName: 'storefront',
    runtime: 'nextJs',
    isDevServer: true,
    startToken: '90110',
  },
  {
    id: 'wsl:Debian:6391:92518',
    origin: 'wsl',
    distribution: 'Debian',
    pid: 6391,
    parentPid: 6368,
    ports: [5173, 24678],
    hosts: ['127.0.0.1', '127.0.0.1'],
    processName: 'node',
    command: 'node node_modules/vite/bin/vite.js --host 127.0.0.1',
    cwd: '/home/mocha/workspaces/design-system',
    projectName: 'design-system',
    runtime: 'vite',
    isDevServer: true,
    startToken: '92518',
  },
  {
    id: 'windows:12880:4729382',
    origin: 'windows',
    distribution: null,
    pid: 12880,
    parentPid: 8604,
    ports: [6006],
    hosts: ['0.0.0.0'],
    processName: 'node.exe',
    command: 'node.exe storybook dev -p 6006',
    cwd: 'C:\\Projects\\component-lab',
    projectName: 'component-lab',
    runtime: 'storybook',
    isDevServer: true,
    startToken: '4729382',
  },
  {
    id: 'wsl:Ubuntu:772:8100',
    origin: 'wsl',
    distribution: 'Ubuntu',
    pid: 772,
    parentPid: 1,
    ports: [5432],
    hosts: ['127.0.0.1'],
    processName: 'postgres',
    command: '/usr/lib/postgresql/17/bin/postgres -D /var/lib/postgresql/data',
    cwd: '/var/lib/postgresql/17/main',
    projectName: 'main',
    runtime: 'other',
    isDevServer: false,
    startToken: '8100',
  },
];

const isTauri = () => '__TAURI_INTERNALS__' in window;

export async function scanServices(): Promise<ScanSnapshot> {
  if (!isTauri()) {
    await new Promise((resolve) => window.setTimeout(resolve, 260));
    return { services: mockServices, warnings: [], scannedAt: Date.now() };
  }

  return invoke<ScanSnapshot>('scan_services');
}
export async function terminateService(request: TerminateRequest): Promise<void> {
  if (!isTauri()) {
    await new Promise((resolve) => window.setTimeout(resolve, 450));
    return;
  }

  await invoke('terminate_service', { request });
}

export async function openService(port: number): Promise<void> {
  if (!isTauri()) {
    window.open(`http://localhost:${port}`, '_blank', 'noopener,noreferrer');
    return;
  }

  await invoke('open_service', { port });
}
