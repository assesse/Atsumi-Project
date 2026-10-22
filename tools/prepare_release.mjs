// Prepare locally built release assets; never changes Git or publishes a release.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';
import { spawnSync } from 'node:child_process';

const root = process.cwd();
assert.equal(process.argv.length, 3, 'Usage: node tools/prepare_release.mjs <successful-build-receipt.json>');
const buildReceipt = path.resolve(process.argv[2]);
const config = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const version = config.version;
assert.match(version, /^\d+\.\d+\.\d+$/);
assert.equal(JSON.parse(fs.readFileSync('package.json', 'utf8')).version, version);
assert.equal(fs.readFileSync('src-tauri/Cargo.toml', 'utf8').match(/^version = "([^"]+)"$/m)?.[1], version);
const portableSource = path.join(root, `.runtime/portable-v${version}/Atsumi-Portable.zip`);
const receiptCheck = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(root, 'tools/build_receipt.ps1'), '-VerifyReceipt', buildReceipt, '-PortableArchive', portableSource], { encoding: 'utf8', windowsHide: true, timeout: 300_000, maxBuffer: 64 * 1024 });
assert.equal(receiptCheck.status, 0, `Build receipt verification failed: ${receiptCheck.error?.message ?? receiptCheck.stderr?.trim() ?? 'unknown failure'}`);
const buildProof = JSON.parse(receiptCheck.stdout);
assert.equal(buildProof.version, version);
const source = path.join(buildProof.stage, 'Atsumi-Setup.exe');
function verifyBuildBuffer(name, buffer) {
  const proof = buildProof.outputs.files.find(file => file.path === name);
  assert.ok(proof, 'Missing build output proof');
  assert.equal(buffer.length, proof.length, 'Build output changed after receipt verification');
  assert.equal(createHash('sha256').update(buffer).digest('hex'), proof.sha256, 'Build output changed after receipt verification');
}
// Reject runtime-inclusive/renamed/partial archives before staging any artifact.
// This pure verifier performs no download, extraction, tool execution or cleanup.
const portableCheck = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(root, 'tools/package_manifest.ps1'), '-VerifyPortableArchive', portableSource, '-ExpectedVersion', version], { encoding: 'utf8', windowsHide: true, timeout: 120_000, maxBuffer: 64 * 1024 });
assert.equal(portableCheck.status, 0, `Portable package verification failed: ${portableCheck.error?.message ?? portableCheck.stderr?.trim() ?? 'unknown failure'}`);
const portableProof = JSON.parse(portableCheck.stdout);
assert.equal(portableProof.version, version);
assert.equal(portableProof.updaterCompatible, true);
const bytes = fs.readFileSync(source);
verifyBuildBuffer('Atsumi-Setup.exe', bytes);
assert.ok(bytes.length > 1_000_000);
const signatureBytes = fs.readFileSync(source + '.sig');
verifyBuildBuffer('Atsumi-Setup.exe.sig', signatureBytes);
const signature = signatureBytes.toString('utf8').trim();
function verifyArtifact(bytes, signature) {
const lines = Buffer.from(signature, 'base64').toString('utf8').trim().split(/\r?\n/);
assert.equal(lines.length, 4);
assert.ok(lines[2].startsWith('trusted comment: '));
const publicLines = Buffer.from(config.plugins.updater.pubkey, 'base64').toString('utf8').trim().split(/\r?\n/);
const publicRecord = Buffer.from(publicLines[1], 'base64');
const signatureRecord = Buffer.from(lines[1], 'base64');
assert.equal(publicRecord.length, 42);
assert.equal(signatureRecord.length, 74);
assert.equal(signatureRecord.subarray(0, 2).toString(), 'ED');
assert.deepEqual(publicRecord.subarray(2, 10), signatureRecord.subarray(2, 10));
const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), publicRecord.subarray(10)]), format: 'der', type: 'spki' });
const primarySignature = signatureRecord.subarray(10);
const digest = createHash('blake2b512').update(bytes).digest();
assert.ok(verify(null, digest, key, primarySignature), 'Artifact signature verification failed');
assert.ok(verify(null, Buffer.concat([primarySignature, Buffer.from(lines[2].slice('trusted comment: '.length))]), key, Buffer.from(lines[3], 'base64')), 'Trusted comment signature verification failed');
const tamperedDigest = Buffer.from(digest);
tamperedDigest[0] ^= 1;
assert.equal(verify(null, tamperedDigest, key, primarySignature), false);
}
verifyArtifact(bytes, signature);
const platform = { signature, url: `https://github.com/assesse/Atsumi-Project/releases/download/v${version}/Atsumi-Setup.exe` };
const portableBytes = fs.readFileSync(portableSource);
assert.equal(portableBytes.length, portableProof.bytes, 'Portable changed after package verification');
assert.equal(createHash('sha256').update(portableBytes).digest('hex'), portableProof.sha256, 'Portable changed after package verification');
const portableSignature = fs.readFileSync(portableSource + '.sig', 'utf8').trim();
verifyArtifact(portableBytes, portableSignature);
// Recheck whole inputs after all reads/signature checks, before any staged output.
const finalReceiptCheck = spawnSync('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(root, 'tools/build_receipt.ps1'), '-VerifyReceipt', buildReceipt, '-PortableArchive', portableSource], { encoding: 'utf8', windowsHide: true, timeout: 300_000, maxBuffer: 64 * 1024 });
assert.equal(finalReceiptCheck.status, 0, 'Build inputs changed before release staging');
assert.equal(JSON.parse(finalReceiptCheck.stdout).receiptSha256, buildProof.receiptSha256);
const receiptBytes = fs.readFileSync(buildReceipt);
assert.equal(receiptBytes.length, buildProof.receiptBytes);
assert.equal(createHash('sha256').update(receiptBytes).digest('hex'), buildProof.receiptSha256);
const output = path.join(root, `.runtime/release-v${version}`);
fs.mkdirSync(output, { recursive: true });
const asset = path.join(output, 'Atsumi-Setup.exe');
if (fs.existsSync(asset)) assert.ok(fs.readFileSync(asset).equals(bytes), 'Existing staged installer differs; do not overwrite a published artifact');
else fs.writeFileSync(asset, bytes, { flag: 'wx' });
const portableAsset = path.join(output, 'Atsumi-Portable.zip');
if (fs.existsSync(portableAsset)) assert.ok(fs.readFileSync(portableAsset).equals(portableBytes), 'Existing portable artifact differs');
else fs.writeFileSync(portableAsset, portableBytes, { flag: 'wx' });
const portablePlatform = { signature: portableSignature, url: `https://github.com/assesse/Atsumi-Project/releases/download/v${version}/Atsumi-Portable.zip` };
const metadata = {
  version,
  notes: fs.readFileSync(`docs/releases/v${version}.md`, 'utf8').trim(),
  pub_date: new Date().toISOString(),
  platforms: { 'windows-x86_64': platform, 'windows-x86_64-nsis': platform, 'windows-x86_64-portable': portablePlatform },
};
fs.writeFileSync(path.join(output, 'latest.json'), JSON.stringify(metadata, null, 2) + '\n');
const receiptAsset = path.join(output, 'build-receipt.json');
if (fs.existsSync(receiptAsset)) assert.ok(fs.readFileSync(receiptAsset).equals(receiptBytes), 'Existing build receipt differs');
else fs.writeFileSync(receiptAsset, receiptBytes, { flag: 'wx' });
const result = { version, buildId: buildProof.buildId, buildReceiptSha256: buildProof.receiptSha256, signatureVerified: true, tamperingRejected: true, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), portable: { bytes: portableBytes.length, sha256: createHash('sha256').update(portableBytes).digest('hex'), signatureVerified: true }, assets: ['Atsumi-Setup.exe', 'Atsumi-Portable.zip', 'latest.json', 'build-receipt.json'] };
fs.writeFileSync(path.join(output, 'verification.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result));
