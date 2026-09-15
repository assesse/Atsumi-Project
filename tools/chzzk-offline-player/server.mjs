import http from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { createReadStream } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

export function framePolicy(origin) {
  return `default-src 'none'; script-src ${origin} 'unsafe-eval'; style-src ${origin} 'unsafe-inline'; media-src ${origin}; img-src ${origin} data:; font-src ${origin}; connect-src 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors ${origin}; sandbox allow-scripts`;
}
export function byteRange(value, size) {
  if (!value) return { start: 0, end: size - 1, status: 200 };
  const match = /^bytes=(\d*)-(\d*)$/.exec(value);
  if (!match || !match[1] && !match[2]) return null;
  let start, end;
  if (!match[1]) { const suffix = Number(match[2]); if (!Number.isSafeInteger(suffix) || suffix < 1) return null; start = Math.max(0, size - suffix); end = size - 1; }
  else { start = Number(match[1]); end = match[2] ? Math.min(size - 1, Number(match[2])) : size - 1; }
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || start < 0 || start >= size || end < start) return null;
  return { start, end: Math.min(end, start + 1024 * 1024 - 1), status: 206 };
}
export async function startServer(port = 18764, bundleRoot = null) {
  let proof, templates, outputRoot;
  if (bundleRoot) {
    outputRoot = bundleRoot;
    proof = JSON.parse(await readFile(path.join(outputRoot, 'manifest.json'), 'utf8'));
    templates = {};
    for (const name of ['live', 'vod', 'liveBadge']) {
      templates[name] = await readFile(path.join(outputRoot, `${name}.html`), 'utf8');
      if (createHash('sha256').update(templates[name]).digest('hex') !== proof.expressions[name].htmlSha256) throw new Error('Original template integrity check failed');
    }
    for (const original of [...proof.originalFiles, ...(proof.resourceFiles || []), ...(proof.derivedFiles || [])]) {
      if (!/^(?:assets\/)?[A-Za-z0-9_-]+\.(?:js|css|png|woff2)$/.test(original.file)) throw new Error('Unexpected asset path');
      if (createHash('sha256').update(await readFile(path.join(outputRoot, original.file))).digest('hex') !== original.sha256) throw new Error('Original asset integrity check failed');
    }
  } else ({ proof, templates, outputRoot } = await (await import('./build.mjs')).prepareDemo());
  const files = new Set(['index.html', 'demo.css', 'demo.js', 'frame.js', 'frame.css', 'player-adaptations.css', 'chat.js', 'isolation.js', 'surface-data.js', 'replay-search.js', 'replay-search.css', 'manifest.json', 'sample.mp4', ...proof.originalFiles.map(asset => asset.file), ...(proof.resourceFiles || []).map(asset => asset.file), ...(proof.derivedFiles || []).map(asset => asset.file)]);
  const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json; charset=utf-8', '.mp4': 'video/mp4', '.png': 'image/png', '.woff2': 'font/woff2' };
  const server = http.createServer(async (request, response) => {
    try {
      const origin = `http://127.0.0.1:${server.address().port}`;
      if (request.headers.host !== `127.0.0.1:${server.address().port}` || !['GET', 'HEAD'].includes(request.method)) { response.writeHead(403).end(); return; }
      const url = new URL(request.url, origin);
      if (url.origin !== origin) { response.writeHead(403).end(); return; }
      response.setHeader('Cache-Control', 'no-store');
      response.setHeader('X-Content-Type-Options', 'nosniff');
      response.setHeader('Referrer-Policy', 'no-referrer');
      response.setHeader('Access-Control-Allow-Origin', 'null');
      response.setHeader('Content-Security-Policy', `default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'`);
      if (url.pathname === '/frame.html' || url.pathname === '/chat.html') {
        const chat = url.pathname === '/chat.html';
        const mode = url.searchParams.get('mode') === 'vod' ? 'vod' : 'live';
        const replayChat = chat && mode === 'vod';
        const body = Buffer.from(`<!doctype html><html lang="ko" class="theme_dark"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>CHZZK 원본 ${chat ? (replayChat ? '녹화본 채팅 검색' : '라이브 채팅') : mode.toUpperCase() + ' 플레이어'}</title><link rel="stylesheet" href="/assets/vendor-DVDNLy6N.css"><link rel="stylesheet" href="/assets/player-vendor-Ct4cRQDi.css"><link rel="stylesheet" href="/local-index.css"><link rel="stylesheet" href="/frame.css">${!chat ? '<link rel="stylesheet" href="/player-adaptations.css">' : ''}${replayChat ? '<link rel="stylesheet" href="/replay-search.css">' : ''}</head><body>${chat ? '<div id="chat-root"></div>' : templates[mode] + '<template id="live-badge-template">' + templates.liveBadge + '</template>'}<script type="module" src="/${chat ? 'chat' : 'frame'}.js"></script></body></html>`);
        response.setHeader('Content-Security-Policy', framePolicy(origin));
        response.writeHead(200, { 'Content-Type': types['.html'], 'Content-Length': body.length });
        response.end(request.method === 'HEAD' ? undefined : body); return;
      }
      const key = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
      if (!files.has(key)) { response.writeHead(404).end(); return; }
      const file = path.join(outputRoot, key), info = await stat(file), range = byteRange(request.headers.range, info.size);
      if (!range) { response.writeHead(416, { 'Content-Range': `bytes */${info.size}` }).end(); return; }
      if (range.status === 206) response.setHeader('Content-Range', `bytes ${range.start}-${range.end}/${info.size}`);
      response.writeHead(range.status, { 'Content-Type': types[path.extname(file)], 'Accept-Ranges': 'bytes', 'Content-Length': range.end - range.start + 1 });
      if (request.method === 'HEAD') response.end();
      else createReadStream(file, { start: range.start, end: range.end }).on('error', () => response.destroy()).pipe(response);
    } catch { if (!response.headersSent) response.writeHead(500); response.end(); }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  return server;
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const directory = path.dirname(fileURLToPath(import.meta.url));
  const bundleRoot = await stat(path.join(directory, 'manifest.json')).then(() => directory).catch(() => null);
  const server = await startServer(18764, bundleRoot);
  console.log(JSON.stringify({ ready: true, url: `http://127.0.0.1:${server.address().port}`, standalone: Boolean(bundleRoot), externalNetwork: false }));
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => server.close(() => process.exit(0)));
}
