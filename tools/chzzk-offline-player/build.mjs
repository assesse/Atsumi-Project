// Uses only ALREADY LOCAL, hash-pinned original assets. No network or app import.
import { readFile, writeFile, mkdir, copyFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';
import { extractOriginalChatTranslations } from '../extract_chzzk_chat.mjs';
import { extractSurfaces } from './extract-surfaces.mjs';

export const toolsRoot = path.dirname(fileURLToPath(import.meta.url));
export const projectRoot = path.resolve(toolsRoot, '../..');
export const outputRoot = path.join(projectRoot, '.runtime/chzzk-offline-player');
const cacheRoot = path.join(projectRoot, '.runtime/chzzk-original-player');
export const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const invariant = (value, message) => { if (!value) throw new Error(message); };
export function callRange(source, anchor) {
  const start = source.indexOf(anchor);
  invariant(start >= 0 && source.indexOf(anchor, start + 1) < 0, 'Original expression anchor changed');
  let depth = 0, quote = '', escaped = false;
  for (let i = source.indexOf('(', start); i < source.length; i++) {
    const ch = source[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (ch === '\\') escaped = true;
      else if (ch === quote) quote = '';
      else invariant(!(quote === '`' && ch === '$' && source[i + 1] === '{'), 'Unexpected interpolation');
    } else if (['"', "'", '`'].includes(ch)) quote = ch;
    else if (ch === '(') depth++;
    else if (ch === ')' && --depth === 0) return [start, i + 1];
  }
  throw new Error('Incomplete original expression');
}
const escape = text => String(text).replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
const attrs = { className: 'class', strokeWidth: 'stroke-width', strokeLinecap: 'stroke-linecap', strokeLinejoin: 'stroke-linejoin', fillRule: 'fill-rule', clipRule: 'clip-rule' };
export function render(node) {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node !== 'object') return escape(node);
  const attributes = Object.entries(node.props).flatMap(([key, value]) => {
    // Online closures are not executed. Original presentation is preserved.
    if (key === 'ref' || key === 'key' || /^on[A-Z]/.test(key) || value == null || value === false) return [];
    invariant(!['src', 'href', 'srcdoc', 'dangerouslySetInnerHTML'].includes(key), 'Unexpected active markup');
    if (key === 'style') value = Object.entries(value).map(([name, item]) => `${name.replace(/[A-Z]/g, ch => '-' + ch.toLowerCase())}:${item}`).join(';');
    if (value === true) return [` ${attrs[key] || key}`];
    return [` ${attrs[key] || key}="${escape(value)}"`];
  }).join('');
  return `<${node.type}${attributes}>${node.children.map(render).join('')}</${node.type}>`;
}
export async function prepareDemo() {
  const manifest = JSON.parse(await readFile(path.join(projectRoot, 'public/original-player/source-manifest.json'), 'utf8'));
  const names = ['player-vendor-BYg0wCyN.js', 'common-vendor-pcHQV1G2.js', 'rolldown-runtime-CJJwijRH.js', 'player-vendor-Ct4cRQDi.css', 'vendor-DVDNLy6N.css', 'index-2Gtwn4qD.css', 'index-C4sif-4p.js', 'strings-ko_kr-CYa8lLdQ.js'];
  const originals = new Map();
  for (const name of names) {
    const pin = manifest.assets.find(asset => asset.file === name);
    invariant(pin, `Missing source pin: ${name}`);
    const bytes = await readFile(path.join(cacheRoot, name));
    invariant(digest(bytes) === pin.sha256, `Original asset hash mismatch: ${name}`);
    originals.set(name, bytes);
  }
  const index = originals.get('index-C4sif-4p.js').toString('utf8');
  const translations = extractOriginalChatTranslations(originals.get('strings-ko_kr-CYa8lLdQ.js').toString('utf8'));
  const surfaces = extractSurfaces(index, translations);
  const inputsRoot = path.join(projectRoot, '.runtime/chzzk-offline-surface-inputs');
  const inputs = JSON.parse(await readFile(path.join(inputsRoot, 'manifest.json'), 'utf8'));
  const createElement = (type, props, ...children) => {
    if (typeof type === 'function') return type(props || {});
    invariant(typeof type === 'string' && /^[a-z][a-z0-9-]*$/.test(type), 'Unexpected original component');
    return { type, props: props || {}, children };
  };
  const context = vm.createContext({
    K: Object.freeze({ createElement }),
    q: Object.freeze({ default: (...items) => items.flatMap(item => typeof item === 'string' ? [item] : Object.keys(item).filter(key => item[key])).join(' ') }),
    $t: key => { invariant(Object.hasOwn(translations, key), `Missing original label: ${key}`); return translations[key]; },
    S: false, b: false, x: false, ee: false, ne: false, g: false,
    m: { language: 'ko' }, e: { timeMachineState: 'NO_DVR' }, SX: { AVAILABLE: 'DVR', NOT_AVAILABLE_LIVE: 'NO_DVR' },
    le: false, F: false, nt: false, et: false, D: false, _e: false, P: false, Fe: false, B: false, f: null, he: true,
    h: null, o: null, s: null,
  }, { codeGeneration: { strings: false, wasm: false } });
  const svgRanges = { kY: [2654015, 2655346], AY: [2655347, 2656005] };
  for (const [name, range] of Object.entries(svgRanges)) {
    invariant(index.slice(range[0], range[0] + 3) === `${name}=`, 'Original SVG anchor changed');
    context[name] = vm.runInContext(`(${index.slice(range[0] + 3, range[1])})`, context, { timeout: 1000 });
  }
  const templates = {}, expressions = {};
  const anchors = {
    live: 'K.createElement(`div`,{id:`live_player_layout`,className:(0,q.default)(`chzzk_player`,`type_live`,`has_shortcut`,{pip_mode:g,',
    vod: 'K.createElement(`div`,{id:`player_layout`,className:(0,q.default)(`chzzk_player`,`type_vod`,`has_shortcut`,{pip_mode:S,',
    liveBadge: 'K.createElement(`button`,{disabled:e.timeMachineState===SX.NOT_AVAILABLE_LIVE,onClick:',
  };
  for (const [name, anchor] of Object.entries(anchors)) {
    const range = callRange(index, anchor), expression = index.slice(...range);
    context.g = name === 'vod' ? { language: 'ko' } : false;
    templates[name] = render(vm.runInContext(`(${expression})`, context, { timeout: 1000 }));
    expressions[name] = { file: 'index-C4sif-4p.js', utf16Range: range, sha256: digest(expression), htmlSha256: digest(templates[name]) };
  }
  await mkdir(path.join(outputRoot, 'assets'), { recursive: true });
  const shipped = names.filter(name => !['index-C4sif-4p.js', 'strings-ko_kr-CYa8lLdQ.js'].includes(name));
  for (const name of shipped) await writeFile(path.join(outputRoot, 'assets', name), originals.get(name));
  const runtimeFiles = ['index.html', 'demo.css', 'demo.js', 'frame.js', 'frame.css', 'player-adaptations.css', 'chat.js', 'isolation.js', 'surface-data.js', 'replay-search.js', 'replay-search.css', 'server.mjs', 'start.ps1', 'start.cmd', 'README.md'];
  for (const name of runtimeFiles) await copyFile(path.join(toolsRoot, name), path.join(outputRoot, name));
  const resourceFiles = [];
  for (const asset of inputs.assets) {
    invariant(/^[A-Za-z0-9_-]+\.(?:js|png|woff2)$/.test(asset.file), 'Unexpected local surface asset name');
    const bytes = await readFile(path.join(inputsRoot, asset.file));
    invariant(digest(bytes) === asset.sha256, `Surface asset hash mismatch: ${asset.file}`);
    await writeFile(path.join(outputRoot, 'assets', asset.file), bytes);
    resourceFiles.push({ ...asset, file: `assets/${asset.file}`, unchanged: true });
  }
  const badgePin = manifest.assets.find(asset => asset.file === 'icon_official_mark.png');
  const badge = await readFile(path.join(cacheRoot, badgePin.file));
  invariant(digest(badge) === badgePin.sha256, 'Badge asset pin changed');
  await writeFile(path.join(outputRoot, 'assets', badgePin.file), badge);
  resourceFiles.push({ ...badgePin, file: `assets/${badgePin.file}`, unchanged: true });
  const assetMap = new Map(resourceFiles.map(asset => [asset.sourceUrl, `/${asset.file}`]));
  const cssOriginal = originals.get('index-2Gtwn4qD.css').toString('utf8');
  const cssMappings = [];
  const localCss = cssOriginal.replace(/@import\s+"https:\/\/cdn\.jsdelivr\.net[^;]+;/g, text => {
    cssMappings.push({ original: text, replacement: '', reason: 'Unused full-site font import; player/chat retain their original system and local Sandoll fonts' });
    return '';
  }).replace(/url\(([^)]+)\)/g, (whole, source) => {
    const url = source.replace(/["']/g, ''), target = assetMap.get(url);
    if (!target) return whole;
    cssMappings.push({ original: url, replacement: target, reason: 'Local URL only; original asset bytes and all CSS presentation declarations unchanged' });
    return `url(${target})`;
  });
  await writeFile(path.join(outputRoot, 'local-index.css'), localCss);
  await writeFile(path.join(outputRoot, 'original-surfaces.js'), surfaces.module);
  await writeFile(path.join(outputRoot, 'original-messages.js'), surfaces.messages);
  for (const [mode, template] of Object.entries(templates)) await writeFile(path.join(outputRoot, `${mode}.html`), template);
  await copyFile(path.join(cacheRoot, 'synthetic.mp4'), path.join(outputRoot, 'sample.mp4'));
  const proof = {
    format: 2, playerVersion: '1.16.2', source: 'Locally preserved CHZZK public PC distribution',
    runtimeNetwork: 'Loopback files only; remote resources and API traffic denied by CSP before SDK import',
    originalFiles: shipped.map(name => ({ file: `assets/${name}`, sha256: digest(originals.get(name)), unchanged: true })),
    resourceFiles,
    derivedFiles: [{ file: 'original-surfaces.js', sha256: digest(surfaces.module) }, { file: 'original-messages.js', sha256: digest(surfaces.messages) }, { file: 'local-index.css', sha256: digest(localCss) }],
    surfaces: surfaces.manifest,
    cssMappings,
    expressions, svgRanges,
    scope: 'LIVE/VOD controls; original broadcast-info portal and chat shell/header/menu/rows. LIVE retains logged-out input; VOD replaces the input/tool row with local body/nickname search. Synthetic fixture data only; no online account, commerce, chat transport or recording.',
    hostChanges: 'Document/iframe geometry, local media/fixture binding, original LIVE no-ad/no-poster state and LIVE/header portals. Original CSS only has recorded local asset URL mappings and unused full-site font import omission. User-requested adaptations are separate: replay-only footer/search, detailed original live header in both widths, and hiding only the floating mute notice (not the volume controls or muted state).',
    missingOnlineDependencies: '121 original Sandoll WOFF2 files, original default profile and badge are local. Unused full-site resources remain blocked. Login, chat/profile transport and commerce actions are deliberately disconnected.',
    redistributionPermission: 'not_assumed',
  };
  await writeFile(path.join(outputRoot, 'manifest.json'), JSON.stringify(proof, null, 2));
  return { outputRoot, proof, templates };
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const result = await prepareDemo();
  console.log(JSON.stringify({ ok: true, outputRoot, originalFiles: result.proof.originalFiles.length, modes: Object.keys(result.templates), networkUsed: false }));
}
