// Preparation only. The preview server never downloads assets.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const sourceRoot = path.join(root, '.runtime/chzzk-original-player');
const destination = path.join(root, '.runtime/chzzk-offline-surface-inputs');
const hash = value => createHash('sha256').update(value).digest('hex');
const index = await readFile(path.join(sourceRoot, 'index-C4sif-4p.js'), 'utf8');
if (hash(index) !== '40a9c54387bb40ed0136147ba5626647f138777def7cecc574b7a41536d982fc') throw new Error('Source pin mismatch');
const css = await readFile(path.join(sourceRoot, 'index-2Gtwn4qD.css'), 'utf8');
const base = 'https://ssl.pstatic.net/static/nng/glive/resource/p/static/js/';
const scripts = [...index.matchAll(/from"(\.\/[^"/]+\.js)"/g)].map(match => new URL(match[1], base).href)
  .filter(url => /\/(?:vendor-DQYi7KBW|date-vendor-DhkCJ9xZ)\.js$/.test(url));
const fonts = [...css.matchAll(/url\(([^)]+)\)/g)].map(match => match[1].replace(/["']/g, ''))
  .filter(url => url.includes('/SD/woff2/SDNemony2dBasicBd/') && url.endsWith('.woff2'));
const profile = index.match(/fl=`(https:\/\/ssl\.pstatic\.net\/[^`]+)`/)?.[1];
if (!profile || scripts.length !== 2 || fonts.length !== 121) throw new Error('Original dependency boundaries changed');
const urls = [...new Set([...scripts, ...fonts, profile])];
await mkdir(destination, { recursive: true });
const previous = await readFile(path.join(destination, 'manifest.json'), 'utf8').then(JSON.parse).catch(() => ({ assets: [] }));
const assets = [], queue = [...urls];
let totalBytes = 0;
async function worker() {
  while (queue.length) {
    const url = queue.shift(), parsed = new URL(url);
    if (parsed.protocol !== 'https:' || parsed.hostname !== 'ssl.pstatic.net' || !parsed.pathname.startsWith('/static/nng/')) throw new Error('Unexpected source host');
    const file = path.basename(parsed.pathname), pin = previous.assets.find(asset => asset.sourceUrl === url);
    let bytes = pin && await readFile(path.join(destination, file)).catch(() => null);
    if (!bytes || hash(bytes) !== pin.sha256) {
      const response = await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(25000) });
      if (!response.ok || Number(response.headers.get('content-length')) > 16000000) throw new Error(`Asset unavailable: ${file}`);
      bytes = Buffer.from(await response.arrayBuffer());
      if (!bytes.length || bytes.length > 16000000) throw new Error('Asset size limit exceeded');
      await writeFile(path.join(destination, file), bytes);
    }
    totalBytes += bytes.length;
    if (totalBytes > 32000000) throw new Error('Bundle size limit exceeded');
    assets.push({ file, sourceUrl: url, sha256: hash(bytes), bytes: bytes.length });
  }
}
await Promise.all(Array.from({ length: 4 }, worker));
assets.sort((a, b) => a.file.localeCompare(b.file));
await writeFile(path.join(destination, 'manifest.json'), JSON.stringify({ format: 1, source: 'Dependencies referenced by pinned CHZZK 1.16.2 distribution', assets }, null, 2));
console.log(JSON.stringify({ prepared: assets.length, totalBytes, destination, runtimeNetwork: false }));
