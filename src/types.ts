export type RuntimeKind =
  | 'nextJs'
  | 'vite'
  | 'nuxt'
  | 'astro'
  | 'svelteKit'
  | 'remix'
  | 'angular'
  | 'storybook'
  | 'webpack'
  | 'parcel'
  | 'rspack'
  | 'node'
  | 'bun'
  | 'deno'
  | 'other';

export type ProcessOrigin = 'windows' | 'wsl';

export interface ServiceProcess {
  id: string;
  origin: ProcessOrigin;
  distribution: string | null;
  pid: number;
  parentPid: number;
  ports: number[];
  hosts: string[];
  processName: string;
  command: string;
  cwd: string | null;
  projectName: string | null;
  runtime: RuntimeKind;
  isDevServer: boolean;
  startToken: string;
}
export interface ScanSnapshot {
  services: ServiceProcess[];
  warnings: string[];
  scannedAt: number;
}

export interface TerminateRequest {
  origin: ProcessOrigin;
  distribution: string | null;
  pid: number;
  startToken: string;
}
