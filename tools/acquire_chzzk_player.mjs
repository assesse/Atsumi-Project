// Acquires unmodified public assets and mechanically renders a pinned original
// CHZZK VOD slot expression. Does not execute the CHZZK application or use auth.
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import { extractOriginalChatPresentation, extractOriginalChatTranslations } from './extract_chzzk_chat.mjs';

const project = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cache = path.join(project, '.runtime/chzzk-original-player');
const output = path.join(project, 'public/original-player');
const generatedOutput = path.join(project, 'src/features/streaming/generated');
const require = createRequire(import.meta.url);
const postcss = require(require.resolve('postcss', { paths: [path.dirname(require.resolve('vite'))] }));
const base = 'https://ssl.pstatic.net/static/nng/glive/resource/p/static/';
const pins = [
  ['js/player-vendor-BYg0wCyN.js', '3add7ef97995bc4e7052e03c84c18e9fe34d5d6212e4672c6f5f76412c9147bc', true],
  ['js/common-vendor-pcHQV1G2.js', '2801491c851815a10c8f5afcbbbd448fbd861e6e3b690adcd3cb8f1cda1ccea0', true],
  ['js/rolldown-runtime-CJJwijRH.js', '0c0bf430afd3b71beff576c47908b01139baba43f826f93f587baaccb6e3a378', true],
  ['css/player-vendor-Ct4cRQDi.css', '949d15281ab5bf7daf8727043fae36bf5239826bbd5c179477232819102423f3', true],
  ['js/index-C4sif-4p.js', '40a9c54387bb40ed0136147ba5626647f138777def7cecc574b7a41536d982fc', false],
  ['css/index-2Gtwn4qD.css', '6b341bb079e0ffe182caaf5437783a1f2a05ffdd27e4224b1a6a6a43b1ec78f6', false],
  ['css/vendor-DVDNLy6N.css', 'f1e7912437519b86ba1473246bc0e1dbeaea0715a5263e531bdd35f00c625ee9', false],
  ['js/strings-ko_kr-CYa8lLdQ.js', '8e76cb2eed30e8c14f1401fab339909ae574ec7dac53999d6f6006e68d294f08', false],
  ['https://ssl.pstatic.net/static/nng/glive/image/icon_official_mark.png', '9e744d6406720d53a658f0e9923193f16c2e836095a443be9ada996274045af4', false],
  ['https://ssl.pstatic.net/static/nng/resource/font/SD/woff2/SDNemony2dBasicBd/SDNemony2dBasicBd115.woff2', 'ce5e3d1932c103057869cee02c8ffcaaddb93a2e322162537bbfcaace7b3efb0', false],
  ['https://ssl.pstatic.net/static/nng/resource/font/SD/woff/SDNemony2dBasicBd/SDNemony2dBasicBd115.woff', '8ec165c7f62ec7ea77b6e45199f3bdb4d3c506f0eb826f36dc97ba4b19bbbf2c', false],
];
const check = process.argv.includes('--check');
const offline = check || process.argv.includes('--offline');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const invariant = (condition, message) => { if (!condition) throw Error(message); };
const assets = new Map();
const manifest = { version: 1, source: 'CHZZK public PC player 1.16.2', authentication: 'none', redistributionPermission: 'not_assumed', assets: [], derived: {} };
await mkdir(cache, { recursive: true });
if (!check) await mkdir(output, { recursive: true });
for (const [relative, digest, stage] of pins) {
  const name = path.basename(relative), url = relative.startsWith('https://') ? relative : base + relative;
  let bytes = await readFile(path.join(cache, name)).catch(() => null);
  if (!bytes || hash(bytes) !== digest) {
    invariant(!offline, `Missing/mismatched pinned asset: ${name}`);
    const response = await fetch(url, { redirect: 'error', credentials: 'omit', signal: AbortSignal.timeout(40000) });
    invariant(response.ok, `Public asset status ${response.status}: ${name}`);
    invariant(Number(response.headers.get('content-length') || 0) <= 16 * 1024 * 1024, 'Asset too large');
    const chunks = []; let length = 0;
    for await (const chunk of response.body) {
      length += chunk.length; invariant(length <= 16 * 1024 * 1024, 'Asset too large'); chunks.push(chunk);
    }
    bytes = Buffer.concat(chunks);
    invariant(hash(bytes) === digest, `Upstream asset changed; review pins/extraction before updating: ${name}`);
    await writeFile(path.join(cache, name), bytes);
    await writeFile(path.join(cache, `${name}.provenance.json`), JSON.stringify({ sourceUrl: url, sha256: digest, bytes: bytes.length, retrievedAtUtc: new Date().toISOString(), authentication: 'none', redistributionPermission: 'not_assumed' }, null, 2));
  }
  invariant(bytes.length <= 16 * 1024 * 1024, 'Asset too large');
  assets.set(name, bytes);
  manifest.assets.push({ file: name, sourceUrl: url, sha256: digest, bytes: bytes.length, stagedUnchanged: stage });
  if (stage) await emit(name, bytes);
}

async function emit(name, data) {
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data);
  if (check) invariant(hash(await readFile(path.join(output, name))) === hash(bytes), `Generated output differs: ${name}`);
  else await writeFile(path.join(output, name), bytes);
}

async function emitGenerated(name, data) {
  const bytes = Buffer.isBuffer(data) ? data : Buffer.from(data);
  if (check) invariant(hash(await readFile(path.join(generatedOutput, name))) === hash(bytes), `Generated chat output differs: ${name}`);
  else { await mkdir(generatedOutput, { recursive: true }); await writeFile(path.join(generatedOutput, name), bytes); }
}

// Parenthesis scanner is used only on the exact hash-pinned element expression.
// Strings in this pinned expression have no template interpolations; fail closed
// if its syntax changes instead of evaluating a different part of the app.
function endOfCall(source, start) {
  let depth = 0, quote = null, escaped = false;
  for (let i = source.indexOf('(', start); i < source.length; i++) {
    const c = source[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (c === '\\') escaped = true;
      else if (c === quote) quote = null;
      else invariant(!(quote === '`' && c === '$' && source[i + 1] === '{'), 'Unexpected template interpolation');
    } else if (['"', "'", '`'].includes(c)) quote = c;
    else if (c === '(') depth++;
    else if (c === ')' && --depth === 0) return i + 1;
  }
  throw Error('Unbalanced original template');
}
const index = assets.get('index-C4sif-4p.js').toString('utf8');
const locale = assets.get('strings-ko_kr-CYa8lLdQ.js').toString('utf8');
const templateAnchor = 'K.createElement(`div`,{id:`player_layout`,className:(0,q.default)(`chzzk_player`,`type_vod`,`has_shortcut`,{pip_mode:S,is_large:b||x,has_chapter_title:ee,ai_caption:ne,';
const start = index.indexOf(templateAnchor);
invariant(start > 3_000_000 && index.indexOf(templateAnchor, start + 1) < 0, 'Original VOD template anchor changed');
const end = endOfCall(index, start);
const expression = index.slice(start, end);
invariant(expression.length < 8000 && expression.includes('`pzp-pc-pip-button`') && expression.includes('`pzp-pc-viewmode-button`'), 'Original slot contract changed');
const svgRanges = { kY: [2654015, 2655346], AY: [2655347, 2656005] };
const createElement = (type, props, ...children) => {
  if (typeof type === 'function') return type(props || {});
  invariant(typeof type === 'string' && /^[a-z][a-z0-9-]*$/.test(type), 'Unexpected element type');
  return { type, props: props || {}, children };
};
const translations = {};
for (const key of ['streamer_shop_tooltip', 'clip_tooltip_enabled', 'radio_mode']) {
  const full = `player.vod_player.${key}`, marker = `"${full}":\``;
  const at = locale.indexOf(marker);
  invariant(at >= 0, `Missing pinned translation: ${full}`);
  const valueStart = at + marker.length, valueEnd = locale.indexOf('`', valueStart);
  const value = locale.slice(valueStart, valueEnd);
  invariant(!/[\\${}]/.test(value) && value.length < 80, 'Unexpected translated value');
  translations[full] = value;
}
const context = vm.createContext({ K: Object.freeze({ createElement }), q: Object.freeze({ default: (...items) => items.flatMap(item => typeof item === 'string' ? [item] : Object.keys(item).filter(key => item[key])).join(' ') }), S: false, b: false, x: false, ee: false, ne: false, g: Object.freeze({ language: 'ko' }), h: null, o: null, s: null, $t: key => { invariant(key in translations, 'Unexpected translation'); return translations[key]; } }, { codeGeneration: { strings: false, wasm: false } });
for (const [name, range] of Object.entries(svgRanges)) {
  invariant(index.slice(range[0], range[0] + 3) === `${name}=`, 'Original SVG anchor changed');
  context[name] = vm.runInContext(`(${index.slice(range[0] + 3, range[1])})`, context, { timeout: 1000 });
}
const tree = vm.runInContext(`(${expression})`, context, { timeout: 1000 });
const escape = value => String(value).replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const attribute = name => ({ className: 'class', strokeWidth: 'stroke-width', strokeLinecap: 'stroke-linecap', strokeLinejoin: 'stroke-linejoin', fillRule: 'fill-rule', clipRule: 'clip-rule' }[name] || name);
function html(node) {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node !== 'object') return escape(node);
  const attrs = Object.entries(node.props).flatMap(([name, value]) => {
    if (name === 'ref' || name === 'key' || value == null || value === false) return [];
    invariant(!/^on/i.test(name) && !['src', 'href', 'srcdoc'].includes(name), 'Unexpected active template attribute');
    if (name === 'style' && typeof value === 'object') value = Object.entries(value).map(([property, item]) => `${property.replace(/[A-Z]/g, c => '-' + c.toLowerCase())}:${item}`).join(';');
    return [` ${attribute(name)}="${escape(value)}"`];
  }).join('');
  return `<${node.type}${attrs}>${node.children.map(html).join('')}</${node.type}>`;
}
const template = html(tree);
invariant(template.includes('pzp-pc-pip-button') && template.includes('pzp-pc-viewmode-button') && template.includes('<svg'), 'Original components missing');
manifest.derived.template = { sourceFile: 'index-C4sif-4p.js', utf16Range: [start, end], sourceExpressionSha256: hash(expression), svgRanges, translations, context: 'VOD; Korean; no adjacent videos/chapter/AI caption/live PiP state; original shop/clip/radio markup retained', htmlSha256: hash(template), bytes: Buffer.byteLength(template) };
await emit('chzzk-slot-template.html', template + '\n');
await emit('chzzk-slot-template.js', `// Generated mechanically from the pinned original CHZZK VOD expression.\nexport default ${JSON.stringify(template)};\n`);
if (!check) await writeFile(path.join(cache, 'chzzk-slot-template.html'), template + '\n');

// Retain exact original rule/declaration bytes; remove whole unrelated rules,
// remote @imports and font-faces, not reimplement colors/geometry/icons.
const retained = [];
const cssSources = ['vendor-DVDNLy6N.css', 'index-2Gtwn4qD.css'];
const keepRule = node => /(?:pzp-|chzzk_player|#player_layout|webplayer-internal-core-shadow)/.test(node.selector) || node.source.start.offset < 1450 || ((/(?:^|[\s,])(?:html|:root|\.theme_dark|\.theme_light)(?:$|[\s,.#[:])/.test(node.selector)) && node.nodes.some(child => child.type === 'decl' && child.prop.startsWith('--')));
function extractStyles(sourceFile) {
const css = assets.get(sourceFile).toString('utf8');
const parsed = postcss.parse(css);
const animationNames = new Set();
parsed.walkRules(node => { if (keepRule(node)) node.walkDecls(/^animation/, decl => decl.value.split(/[\s,]+/).forEach(name => animationNames.add(name))); });
function extract(node) {
  if (node.type === 'rule' && keepRule(node)) {
    // PostCSS offsets are end-exclusive. Adding one copies the next selector's
    // first character (including '[') and can make otherwise valid CSS invalid.
    const range = [node.source.start.offset, node.source.end.offset];
    retained.push({ sourceFile, selector: node.selector, utf16Range: range });
    return css.slice(...range);
  }
  if (node.type === 'atrule' && /^(?:-webkit-)?keyframes$/.test(node.name) && animationNames.has(node.params)) return css.slice(node.source.start.offset, node.source.end.offset);
  if (node.type === 'atrule' && node.nodes && /^(?:media|supports|container|layer)$/.test(node.name)) {
    const children = node.nodes.map(extract).filter(Boolean).join('');
    if (children) return css.slice(node.source.start.offset, css.indexOf('{', node.source.start.offset) + 1) + children + '}';
  }
  return '';
}
return parsed.nodes.map(extract).filter(Boolean).join('\n') + '\n';
}
const theme = cssSources.map(extractStyles).join('\n');
invariant(retained.length > 80 && theme.includes('container:player/inline-size') && theme.includes('html.theme_dark'), 'Original theme extraction incomplete');
// Hash comparison alone cannot detect a reproducible slicing mistake. Parse the
// emitted stylesheet and require every selected rule to end at its own brace.
postcss.parse(theme);
for (const entry of retained) {
  const exact = assets.get(entry.sourceFile).toString('utf8').slice(...entry.utf16Range);
  invariant(exact.endsWith('}'), `CSS source range includes a following token: ${entry.selector}`);
  invariant(postcss.parse(exact).nodes.length === 1, `CSS source range is not one complete rule: ${entry.selector}`);
}
manifest.derived.theme = { sourceFiles: cssSources, sha256: hash(theme), bytes: Buffer.byteLength(theme), retainedRules: retained, transforms: 'Whole-rule selection only; original selectors/declarations preserved; enclosing media/supports/container rules preserved; remote @import and unrelated rules omitted.' };
await emit('chzzk-theme.css', theme);
if (!check) await writeFile(path.join(cache, 'chzzk-theme.css'), theme);

// The chat surface is mounted in a ShadowRoot by the app. Only document-root
// selectors change; all selected official declarations and component selectors
// remain original. No remote fonts/imports or viewport-sized document rules are
// included in this pane stylesheet.
const chatFamilies = ['189hq', 'w9pvh', '1mc5x', '1iatj', '1nwpy'];
const chatRules = [];
const chatSelector = selector => selector
  .replace(/\bhtml\.theme_dark\b/g, ':host(.theme_dark)')
  .replace(/\bhtml\.theme_light\b/g, ':host(.theme_light)')
  .replace(/:root\b/g, ':host')
  .replace(/\bhtml\b/g, ':host')
  .replace(/\bbody\b/g, '.original-chat-root');
function extractChatStyles(sourceFile) {
  const css = assets.get(sourceFile).toString('utf8');
  const parsed = postcss.parse(css);
  function selected(node) {
    if (chatFamilies.some(family => node.selector.includes(`_${family}_`))) return true;
    if (/(?:^|[\s,])(?:html|:root|\.theme_dark|\.theme_light)(?:$|[\s,.#[:])/.test(node.selector) && node.nodes.some(child => child.type === 'decl' && child.prop.startsWith('--'))) return true;
    if (sourceFile !== 'index-2Gtwn4qD.css' || node.source.start.offset >= 2647 || node.nodes.some(child => child.type === 'decl' && child.prop === 'min-height')) return false;
    return node.selector === '.blind' || !/[.#]/.test(node.selector);
  }
  function extract(node) {
    if (node.type === 'rule' && selected(node)) {
      const range = [node.source.start.offset, node.source.end.offset];
      const exact = css.slice(...range), selector = chatSelector(node.selector);
      invariant(exact.endsWith('}') && exact.startsWith(node.selector), 'Unexpected chat CSS source boundary');
      chatRules.push({ sourceFile, selector: node.selector, outputSelector: selector, utf16Range: range, sourceSha256: hash(exact) });
      return selector + exact.slice(node.selector.length);
    }
    if (node.type === 'atrule' && node.nodes && /^(?:media|supports|container|layer)$/.test(node.name)) {
      const children = node.nodes.map(extract).filter(Boolean).join('');
      if (children) return css.slice(node.source.start.offset, css.indexOf('{', node.source.start.offset) + 1) + children + '}';
    }
    return '';
  }
  return parsed.nodes.map(extract).filter(Boolean).join('\n') + '\n';
}
const verifiedIcon = assets.get('icon_official_mark.png');
invariant(verifiedIcon.length <= 65536 && verifiedIcon.subarray(0, 8).toString('hex') === '89504e470d0a1a0a', 'Invalid pinned official verified icon');
const verifiedIconUrl = 'https://ssl.pstatic.net/static/nng/glive/image/icon_official_mark.png';
const chatCss = cssSources.map(extractChatStyles).join('\n').replaceAll(verifiedIconUrl, `data:image/png;base64,${verifiedIcon.toString('base64')}`);
const parsedChat = postcss.parse(chatCss), chatVariables = new Set(), chatVariableUses = new Set();
parsedChat.walkDecls(node => {
  if (node.prop.startsWith('--')) chatVariables.add(node.prop);
  for (const match of node.value.matchAll(/var\((--[A-Za-z0-9_-]+)/g)) chatVariableUses.add(match[1]);
});
invariant([...chatVariableUses].every(name => chatVariables.has(name)), `Chat CSS has missing theme variables: ${[...chatVariableUses].filter(name => !chatVariables.has(name)).join(', ')}`);
invariant(!/@import|@font-face|url\(\s*["']?https?:/i.test(chatCss), 'Unexpected external resource in offline chat CSS');
for (const family of chatFamilies) invariant(chatRules.some(rule => rule.selector.includes(`_${family}_`)), `Missing official chat CSS family: ${family}`);
manifest.derived.chatStyles = { sourceFiles: cssSources, sha256: hash(chatCss), bytes: Buffer.byteLength(chatCss), cssModuleFamilies: chatFamilies, retainedRules: chatRules, inlineAssets: [{ sourceUrl: verifiedIconUrl, sha256: hash(verifiedIcon), bytes: verifiedIcon.length, mimeType: 'image/png' }], transforms: 'Exact original declarations except pinned official verified-icon URL replaced with the same PNG bytes as a data URL; :root/html -> :host, html.theme_dark/light -> :host(.theme_dark/.theme_light), body -> .original-chat-root. Omit document min-height:100vh, unrelated rules, remote imports and font-face. Host supplies theme_dark class and a bounded pane root.' };
await emitGenerated('originalChat.css', chatCss);
const chatPresentation = extractOriginalChatPresentation({ indexSource: index, sourceSha256: hash(assets.get('index-C4sif-4p.js')), translations: extractOriginalChatTranslations(locale) });
await emitGenerated('originalChatPresentation.js', chatPresentation.moduleSource);
await emitGenerated('originalChatPresentation.d.ts', chatPresentation.declarationSource);
manifest.derived.chatPresentation = chatPresentation.manifest;

// Chromium does not reliably register @font-face from a ShadowRoot stylesheet.
// Emit the exact official heading face separately for a regular app CSS import;
// this file registers a font only and has no document selectors. The original
// subset 115 contains both title characters, so the other 119 subsets are not
// acquired. Preserve WOFF2 and the original WOFF fallback without re-subsetting.
const fontSource = assets.get('index-2Gtwn4qD.css').toString('utf8');
const fontFaces = [];
postcss.parse(fontSource).walkAtRules('font-face', node => {
  if (node.nodes.some(declaration => declaration.prop === 'font-family' && declaration.value === 'Sandoll Nemony2') && node.nodes.some(declaration => declaration.prop === 'src' && declaration.value.includes('/SDNemony2dBasicBd115.woff2'))) fontFaces.push(node);
});
invariant(fontFaces.length === 1, 'Original title font-face boundary changed');
const fontFace = fontFaces[0];
invariant(fontFace.source.start.offset === 99567 && fontFace.source.end.offset === 100632, 'Original title font-face range changed');
const unicodeRange = fontFace.nodes.find(declaration => declaration.prop === 'unicode-range').value;
for (const codePoint of ['U+CC44', 'U+D305']) invariant(unicodeRange.split(',').includes(codePoint), `Original title font subset no longer covers ${codePoint}`);
let fontCss = fontSource.slice(fontFace.source.start.offset, fontFace.source.end.offset);
const fontAssets = []; let fontBytes = 0;
for (const [format, signature] of [['woff2', 'wOF2'], ['woff', 'wOFF']]) {
  const name = `SDNemony2dBasicBd115.${format}`;
  const bytes = assets.get(name);
  invariant(bytes.subarray(0, 4).toString('ascii') === signature, `Invalid pinned ${format} font`);
  // These files exceed Vite's 4KiB inline threshold: the app's unchanged
  // font-src 'self' loads emitted local assets, never data: or remote fonts.
  invariant(bytes.length > 4096, 'Title font must remain a separate local build asset');
  fontBytes += bytes.length;
  const sourceUrl = `https://ssl.pstatic.net/static/nng/resource/font/SD/${format}/SDNemony2dBasicBd/${name}`;
  invariant(fontCss.includes(sourceUrl), `Original title font src changed: ${format}`);
  fontCss = fontCss.replace(sourceUrl, `./${name}`);
  await emitGenerated(name, bytes);
  fontAssets.push({ sourceUrl, localFile: name, sha256: hash(bytes), bytes: bytes.length, mimeType: `font/${format}` });
}
invariant(fontBytes <= 200000, 'Original title font exceeds the bounded acquisition budget');
fontCss += '\n';
const parsedFont = postcss.parse(fontCss);
invariant(parsedFont.nodes.length === 1 && parsedFont.nodes[0].type === 'atrule' && parsedFont.nodes[0].name === 'font-face', 'Title font output contains non-font selectors');
invariant(!/@import|https?:|file:|data:/i.test(fontCss), 'Non-local URL in offline title font');
manifest.derived.chatFonts = { sourceFile: 'index-2Gtwn4qD.css', utf16Range: [fontFace.source.start.offset, fontFace.source.end.offset], originalSha256: hash(fontSource.slice(fontFace.source.start.offset, fontFace.source.end.offset)), sha256: hash(fontCss), bytes: Buffer.byteLength(fontCss), unicodeRange, requiredCharacters: ['U+CC44', 'U+D305'], localAssets: fontAssets, totalFontBytes: fontBytes, transforms: 'Exact original @font-face and unicode-range; only the WOFF2/WOFF source URLs become relative local file URLs with original font bytes preserved. Separate global font registration; no document selectors, data-font CSP relaxation or font re-subsetting.' };
await emitGenerated('originalChatFonts.css', fontCss);
await emitGenerated('originalChat.source-manifest.json', JSON.stringify({ version: 1, sources: manifest.assets, presentation: manifest.derived.chatPresentation, styles: manifest.derived.chatStyles, fonts: manifest.derived.chatFonts }, null, 2) + '\n');
await emit('source-manifest.json', JSON.stringify(manifest, null, 2) + '\n');
console.log(JSON.stringify({ ok: true, check, output, pinnedAssets: pins.length, templateBytes: Buffer.byteLength(template), themeBytes: Buffer.byteLength(theme), retainedRules: retained.length, chatCssBytes: Buffer.byteLength(chatCss), chatRules: chatRules.length, chatFontBytes: fontBytes }));
