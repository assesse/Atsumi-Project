#!/usr/bin/env node
/** Generate the original SOOP LIVE chat presentation for Shadow DOM use only.
 * No remote code is executed. Mutable upstream URLs must match these source pins.
 * Usage: node tools/soop-official-chat-assets.mjs [--source-dir DIR] [--check]
 * DIR must contain official-chat.css and official-live-webplayer.css snapshots.
 */
import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const outputDir = path.join(root, 'src/features/soop/generated');
const sources = {
  chat: {
    url: 'https://res.sooplive.com/css/global/chat/chat.css',
    snapshot: 'official-chat.css',
    sha256: '3922faf09623f891d977dce38133fd4a4d74dbd83a9c15a505cf0ab9d6371ae9',
  },
  webplayer: {
    url: 'https://res.sooplive.com/css/global/soop_webplayer/webplayer.css',
    snapshot: 'official-live-webplayer.css',
    sha256: '3d21cb2f640353e68bd64707dff0e67d03a0cc1066092a8ccf64dd027f3064d0',
  },
};
const digest = value => crypto.createHash('sha256').update(value).digest('hex');
const assert = (condition, message) => { if (!condition) throw new Error(message); };
const args = process.argv.slice(2);
let sourceDir = null;
let check = false;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--check') check = true;
  else if (args[i] === '--source-dir' && args[i + 1]) sourceDir = path.resolve(args[++i]);
  else throw new Error(`Unknown or incomplete argument: ${args[i]}`);
}

async function readSource(source) {
  let bytes;
  if (sourceDir) bytes = await fs.readFile(path.join(sourceDir, source.snapshot));
  else {
    const response = await fetch(source.url, { signal: AbortSignal.timeout(30_000) });
    assert(response.ok, `Cannot read ${source.url}: HTTP ${response.status}`);
    bytes = Buffer.from(await response.arrayBuffer());
  }
  assert(digest(bytes) === source.sha256, `Source changed: ${source.url}. Review a new official reference before updating this pin.`);
  return bytes.toString('utf8');
}

// The pinned sources contain quoted, percent-encoded SVG URLs, not CSS escapes.
// Matching the whole quoted URL also preserves the SVG's nested url(%23id).
const urlPattern = /url\((?:"([^"]*)"|'([^']*)'|([^)]*))\)/g;
const urls = css => [...css.matchAll(urlPattern)].map(m => m[1] ?? m[2] ?? m[3]);
function transform(css) {
  assert(!/@import\b/i.test(css), 'Unexpected CSS import');
  const originalData = urls(css).filter(url => url.startsWith('data:'));
  assert(originalData.every(url => url.startsWith('data:image/svg+xml,')), 'Unexpected data URL type');
  assert(originalData.every(url => !/<script|<foreignObject|<image|href\s*=\s*['"]https?:/i.test(decodeURIComponent(url))), 'Unexpected active or remote SVG content');
  const removedFontFaces = (css.match(/@font-face\s*\{[^{}]*\}/g) ?? []).length;
  css = css.replace(/@font-face\s*\{[^{}]*\}/g, '');
  const removedExternalUrls = [];
  css = css.replace(urlPattern, (whole, quoted, single, unquoted) => {
    const url = quoted ?? single ?? unquoted;
    if (url.startsWith('data:')) return whole;
    removedExternalUrls.push(url);
    return 'none';
  });
  // Only selector/prelude text changes; original declaration values/SVGs survive.
  let rootSelectors = 0;
  let darkBodySelectors = 0;
  css = css.replace(/([^{}]+)\{/g, (_whole, selector) => `${selector
    .replace(/:root\b/g, () => { rootSelectors++; return ':host'; })
    .replace(/body\.thema_dark\b/g, () => { darkBodySelectors++; return '.thema_dark'; })}{`);
  css = css.replace(/\/\*[#@]\s*sourceMappingURL=[\s\S]*?\*\//g, '');
  assert(urls(css).every(url => url.startsWith('data:image/svg+xml,')), 'A remote CSS URL survived');
  assert(JSON.stringify(urls(css).filter(url => url.startsWith('data:'))) === JSON.stringify(originalData), 'Original SVG bytes changed');
  assert(!/@font-face\b|@import\b|sourceMappingURL/i.test(css), 'Unexpected external dependency survived');
  return {
    css: css.trim(),
    changes: { removedFontFaces, removedExternalUrls: [...new Set(removedExternalUrls)], rootSelectors, darkBodySelectors, preservedInlineSvgUrls: originalData.length },
  };
}

const [chatSource, webplayerSource] = await Promise.all([readSource(sources.chat), readSource(sources.webplayer)]);
// Standalone role badge rules originate in the LIVE webplayer's common CSS.
// Other player/gift/payment/layout styles are intentionally not selected.
const badgeRules = [...webplayerSource.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
  .filter(match => /^\[class\^=grade-badge-\]/.test(match[1]) || match[1].startsWith(':root:not([lang=ko-KR]) [class^=grade-badge-]'));
assert(badgeRules.length === 11, 'Official badge extraction coverage changed');
const chat = transform(chatSource);
const badges = transform(badgeRules.map(match => match[0]).join('\n'));
assert(chat.changes.preservedInlineSvgUrls === 253, 'Official chat SVG coverage changed');
const banner = '/* Generated by tools/soop-official-chat-assets.mjs. SHADOW DOM ONLY.\n * Original SOOP LIVE styles; source hashes and offline transforms in source-manifest.json.\n * Import as text (?raw), never as a global stylesheet. */\n';
const outputs = {
  'official-chat.css': `${banner}${chat.css}\n`,
  'official-badges.css': `${banner}${badges.css}\n`,
};
const manifest = {
  schemaVersion: 1,
  reference: 'SOOP official LIVE chat, observed 2026-11-17 KST',
  shadowDomOnly: true,
  generatedBy: 'tools/soop-official-chat-assets.mjs',
  sources,
  font: 'Existing local /fonts/pretendard/PretendardVariable.woff2; parent adapter supplies font-family. Teko donation font is excluded.',
  transformations: { chat: chat.changes, badges: badges.changes },
  badgeSelectors: badgeRules.map(match => match[1]),
  excluded: ['All official scripts and account/payment handlers', 'All remote CSS/image/font requests', 'All webplayer CSS except the 11 original core grade-badge rules', 'CSS source-map requests'],
  outputs: Object.fromEntries(Object.entries(outputs).map(([name, content]) => [name, { bytes: Buffer.byteLength(content), sha256: digest(content) }])),
};
outputs['source-manifest.json'] = `${JSON.stringify(manifest, null, 2)}\n`;
if (check) {
  for (const [name, content] of Object.entries(outputs)) {
    assert(await fs.readFile(path.join(outputDir, name), 'utf8') === content, `Generated output differs: ${name}`);
  }
  process.stdout.write('SOOP official chat assets: pinned inputs, original SVGs, offline output and deterministic regeneration verified.\n');
} else {
  await fs.mkdir(outputDir, { recursive: true });
  for (const [name, content] of Object.entries(outputs)) await fs.writeFile(path.join(outputDir, name), content, 'utf8');
  process.stdout.write(`Generated ${Object.keys(outputs).join(', ')}\n`);
}
