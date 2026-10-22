// @vitest-environment node
import workflow from '../../../.github/workflows/windows-release.yml?raw';
import { expect, it } from 'vitest';
import config from '../../../src-tauri/tauri.conf.json';
import portable from '../../../tools/prepare_portable.ps1?raw';
import media from '../../../tools/prepare_media_tools.ps1?raw';
import platform from '../../../tools/prepare_platform_tools.ps1?raw';
import release from '../../../tools/prepare_release.mjs?raw';
import build from '../../../tools/build_release.ps1?raw';

// Keep Node-only test imports outside the application's DOM TypeScript globals.
const childName = 'node:child_process', urlName = 'node:url', processName = 'node:process';
const { spawnSync } = await import(childName) as { spawnSync(file: string, args: string[], options: { encoding: 'utf8'; windowsHide: boolean; timeout: number; maxBuffer: number }): { error?: Error; stderr: string; stdout: string; status: number | null } };
const { fileURLToPath } = await import(urlName) as { fileURLToPath(url: URL): string };
const { platform: hostPlatform } = await import(processName) as { platform: string };

it('prepares pinned platform resources before the first clean CI installer build', () => {
  const prepare = workflow.indexOf('run: ./tools/prepare_platform_tools.ps1');
  const build = workflow.indexOf('./tools/build_release.ps1');
  expect(prepare).toBeGreaterThan(0);
  expect(build).toBeGreaterThan(prepare);
  expect(config.bundle.resources['../.runtime/platform-tools/']).toBe('media-tools/platform-tools/');
  // Direct frontend builds stay local and never download tooling implicitly.
  expect(config.build.beforeBuildCommand).toContain('run_frontend.ps1 build');
});

it('requires a successful build receipt before portable or release staging', () => {
  expect(build).toContain('Invoke-AtsumiBuildReceipt');
  expect(build).toContain('CARGO_TARGET_DIR');
  expect(build).toContain('--target x86_64-pc-windows-msvc --bundles nsis -- --locked');
  expect(portable).toContain('[Parameter(Mandatory=$true)][string]$BuildReceipt');
  expect(portable).toContain('Assert-AtsumiBuildReceipt $projectRoot $BuildReceipt');
  expect(portable).not.toContain("'src-tauri/target/release/atsumi.exe'");
  expect(release).toContain("'-VerifyReceipt'");
  expect(release).toContain('verifyBuildBuffer');
  expect(release.indexOf('const finalReceiptCheck')).toBeLessThan(release.indexOf('fs.writeFileSync(asset'));
  expect(workflow).toContain('prepare_portable.ps1 -BuildReceipt $receipt');
  expect(workflow).toContain('prepare_release.mjs $receipt');
});

it('uses full positive manifests, immutable reused packages and non-repairing copy admission', () => {
  expect(media).toContain('Get-AtsumiArchiveManifest $archive $packageName');
  expect(media).toContain('Assert-AtsumiPackageTree $packageRoot $manifest');
  expect(platform).toContain('Assert-AtsumiPackageTree $platformPackage $manifest');
  expect(platform).toContain('licenses/yt-dlp-THIRD_PARTY_LICENSES.txt');
  expect(portable).toContain('-VerifyOnly -ReturnManifest');
  expect(portable).toContain('Get-AtsumiPackageDirectoryPath');
  expect(portable).toContain('Copy-AtsumiPackageTree $source $destination $approved[$source].manifest');
  expect(portable).toContain('New-AtsumiPackageArchive $payload $archive $manifest');
  expect(portable).not.toMatch(/Copy-Item[^\r\n]*-Recurse/);
  expect(portable).not.toContain('Compress-Archive');
});

it('rejects incompatible portable artifact metadata before release staging', () => {
  expect(portable).toContain('Get-AtsumiPortableArtifact');
  expect(portable).toContain('updaterCompatible=$artifact.updaterCompatible');
  expect(release).toContain("'-VerifyPortableArchive'");
  expect(release.indexOf('const portableCheck')).toBeLessThan(release.indexOf('fs.writeFileSync(asset'));
  expect(release).toContain('assert.equal(portableCheck.status, 0');
  expect(release).toContain("digest('hex'), portableProof.sha256");
  expect(release).toContain("fs.writeFileSync(portableAsset, portableBytes, { flag: 'wx' })");
});

it.skipIf(hostPlatform !== 'win32')('validates synthetic manifest/copy/ZIP and runtime separation on release Windows PowerShell', () => {
  const script = fileURLToPath(new URL('../../../tools/test_package_manifest.ps1', import.meta.url));
  const result = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script], {
    encoding: 'utf8', windowsHide: true, timeout: 60_000, maxBuffer: 256 * 1024,
  });
  expect(result.error?.message ?? result.stderr.trim()).toBe('');
  expect(result.status).toBe(0);
  const report = JSON.parse(result.stdout.trim()) as { passed: number; syntheticOnly: boolean; downloads: number; toolsExecuted: number; deletedFiles: number; checks: string[] };
  expect(report.passed).toBeGreaterThanOrEqual(50);
  expect(report.syntheticOnly).toBe(true);
  expect([report.downloads, report.toolsExecuted, report.deletedFiles]).toEqual([0, 0, 0]);
  expect(report.checks).toContain('held ancestor prevents rename during copy');
  expect(report.checks).toContain('hardlinked regular source copy rejected before destination creation');
  expect(report.checks).toContain('renamed runtime ZIP rejected even with misleading sidecar');
}, 70_000);

it.skipIf(hostPlatform !== 'win32')('validates successful-build provenance using only tiny synthetic fixtures', () => {
  const script = fileURLToPath(new URL('../../../tools/test_build_receipt.ps1', import.meta.url));
  const result = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script], {
    encoding: 'utf8', windowsHide: true, timeout: 60_000, maxBuffer: 256 * 1024,
  });
  expect(result.error?.message ?? result.stderr.trim()).toBe('');
  expect(result.status).toBe(0);
  const report = JSON.parse(result.stdout.trim()) as { passed: number; syntheticOnly: boolean; realBuilds: number; downloads: number; toolsExecuted: number; signaturesCreated: number; realPackages: number; deletedFiles: number; checks: string[] };
  expect(report.passed).toBeGreaterThanOrEqual(34);
  expect(report.syntheticOnly).toBe(true);
  expect([report.realBuilds, report.downloads, report.toolsExecuted, report.signaturesCreated, report.realPackages, report.deletedFiles]).toEqual([0, 0, 0, 0, 0, 0]);
  expect(report.checks).toContain('generic proof still rejects Cargo hardlink pair');
  expect(report.checks).toContain('whole manifest mismatch during build prevents receipt');
  expect(report.checks).toContain('portable from different build receipt rejected');
}, 70_000);
