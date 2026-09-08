import { createHash } from 'node:crypto';
import { copyFile, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

const version = '1.24.260710001';
const packageHash = '175640566a3b59c4b132070ee96c2c77e5ab7edd2e92732a5eb3610bbf63d90e';
const url = `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${version}/microsoft.windows.console.conpty.${version}.nupkg`;
const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const expectedFiles = {
  x64: { 'conpty.dll': '39fba2713e2495117b1591ae8c32a3b904bea7aa66069cf7815e2844c76d75d8',
    'OpenConsole.exe': 'b7fd936c2668b87b9ecf7b3366dc6568afc1c6f981874cba3e955a1c35cf8160' },
  arm64: { 'conpty.dll': 'db3d173640b172bafd42d5b541b638a9aeec1c7d0e40dd636bf02822a32c912c',
    'OpenConsole.exe': 'ed7622fd0d3bedc9ab9f122f5e58edf0def9e7999224f52dd395ba9f54edbe09' },
};

export async function prepareConpty(runner: string, output: string, checked: (args: string[]) => Promise<string>) {
  // Match the Rust executable, including when Bun itself runs under emulation.
  const pe = await readFile(runner);
  const header = pe.readUInt32LE(0x3c);
  if (pe.toString('ascii', header, header + 4) !== 'PE\0\0') throw new Error('Test runner is not a PE executable');
  const machine = pe.readUInt16LE(header + 4);
  const arch = machine === 0x8664 ? 'x64' : machine === 0xaa64 ? 'arm64' : null;
  if (!arch) throw new Error(`Unsupported ConPTY test architecture: ${machine}`);
  const expected = expectedFiles[arch];
  const cached = await Promise.all(Object.entries(expected).map(async ([name, digest]) => {
    try { return hash(await readFile(join(dirname(runner), name))) === digest; }
    catch { return false; }
  }));
  if (cached.every(Boolean)) return { version, arch, package_sha256: packageHash, files: expected };
  const scratch = await mkdtemp(join(output, 'scratch-conpty-'));
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(30_000) });
    if (!response.ok) throw new Error(`ConPTY dependency download failed: HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (hash(bytes) !== packageHash) throw new Error('ConPTY dependency integrity mismatch');
    const archive = join(scratch, 'conpty.zip');
    await writeFile(archive, bytes);
    const script = join(scratch, 'extract.ps1');
    await writeFile(script, 'param([string]$archive, [string]$destination)\n$ErrorActionPreference = "Stop"\nExpand-Archive -LiteralPath $archive -DestinationPath $destination -Force\n');
    const extracted = join(scratch, 'extracted');
    await checked(['powershell.exe', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, archive, extracted]);
    const sources = {
      'conpty.dll': join(extracted, `runtimes/win-${arch}/native/conpty.dll`),
      'OpenConsole.exe': join(extracted, `build/native/runtimes/${arch}/OpenConsole.exe`),
    };
    const files: Record<string, string> = {};
    for (const [name, source] of Object.entries(sources)) {
      const destination = join(dirname(runner), name);
      await copyFile(source, destination);
      files[name] = hash(await readFile(destination));
      if (files[name] !== expected[name as keyof typeof expected]) throw new Error(`ConPTY file integrity mismatch: ${name}`);
    }
    return { version, arch, package_sha256: packageHash, files };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
