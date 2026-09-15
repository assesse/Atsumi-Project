// Prepare locally built release assets; never changes Git or publishes a release.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { createHash, createPublicKey, verify } from 'node:crypto';

const root = process.cwd();
const config = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const version = config.version;
assert.match(version, /^\d+\.\d+\.\d+$/);
assert.equal(JSON.parse(fs.readFileSync('package.json', 'utf8')).version, version);
assert.equal(fs.readFileSync('src-tauri/Cargo.toml', 'utf8').match(/^version = "([^"]+)"$/m)?.[1], version);
const source = path.join(root, `src-tauri/target/release/bundle/nsis/Atsumi_${version}_x64-setup.exe`);
const bytes = fs.readFileSync(source);
assert.ok(bytes.length > 1_000_000);
const signature = fs.readFileSync(source + '.sig', 'utf8').trim();
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
assert.ok(verify(null, digest, key, primarySignature), 'Installer signature verification failed');
assert.ok(verify(null, Buffer.concat([primarySignature, Buffer.from(lines[2].slice('trusted comment: '.length))]), key, Buffer.from(lines[3], 'base64')), 'Trusted comment signature verification failed');
const tamperedDigest = Buffer.from(digest);
tamperedDigest[0] ^= 1;
assert.equal(verify(null, tamperedDigest, key, primarySignature), false);
const output = path.join(root, `.runtime/release-v${version}`);
fs.mkdirSync(output, { recursive: true });
const asset = path.join(output, 'Atsumi-Setup.exe');
if (fs.existsSync(asset)) assert.ok(fs.readFileSync(asset).equals(bytes), 'Existing staged installer differs; do not overwrite a published artifact');
else fs.copyFileSync(source, asset);
const platform = { signature, url: `https://github.com/assesse/Atsumi-Project/releases/download/v${version}/Atsumi-Setup.exe` };
const metadata = {
  version,
  notes: fs.readFileSync(`docs/releases/v${version}.md`, 'utf8').trim(),
  pub_date: new Date().toISOString(),
  platforms: { 'windows-x86_64': platform, 'windows-x86_64-nsis': platform },
};
fs.writeFileSync(path.join(output, 'latest.json'), JSON.stringify(metadata, null, 2) + '\n');
const result = { version, signatureVerified: true, tamperingRejected: true, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), assets: ['Atsumi-Setup.exe', 'latest.json'] };
fs.writeFileSync(path.join(output, 'verification.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result));
