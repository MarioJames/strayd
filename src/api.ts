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
    resourceKind: 'development',
    canTerminate: true,
    managerUnit: null,
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
    resourceKind: 'development',
    canTerminate: true,
    managerUnit: null,
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
    resourceKind: 'development',
    canTerminate: true,
    managerUnit: null,
    startToken: '4729382',
  },
  {
    id: 'wsl:Debian:730:44000',
    origin: 'wsl',
    distribution: 'Debian',
    pid: 730,
    parentPid: 1,
    ports: [],
    hosts: [],
    processName: 'ssh',
    command: '/usr/bin/ssh -NT -R 0.0.0.0:33:127.0.0.1:22 aliyun',
    cwd: '/root',
    projectName: null,
    runtime: 'sshTunnel',
    resourceKind: 'tunnel',
    canTerminate: true,
    managerUnit: 'aliyunhost-reverse-tunnel.service',
    startToken: '44000',
  },
  {
    id: 'wsl:Debian:9032:97112',
    origin: 'wsl',
    distribution: 'Debian',
    pid: 9032,
    parentPid: 8988,
    ports: [],
    hosts: [],
    processName: 'cloudflared',
    command: 'cloudflared tunnel --url http://127.0.0.1:3000',
    cwd: '/home/mocha/workspaces/storefront',
    projectName: 'storefront',
    runtime: 'cloudflared',
    resourceKind: 'tunnel',
    canTerminate: true,
    managerUnit: null,
    startToken: '97112',
  },
  {
    id: 'wsl:Debian:612:5012',
    origin: 'wsl',
    distribution: 'Debian',
    pid: 612,
    parentPid: 1,
    ports: [22],
    hosts: ['0.0.0.0'],
    processName: 'sshd',
    command: '/usr/sbin/sshd -D',
    cwd: '/',
    projectName: null,
    runtime: 'sshd',
    resourceKind: 'system',
    canTerminate: false,
    managerUnit: 'ssh.service',
    startToken: '5012',
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
