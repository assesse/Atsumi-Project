// Extracts hash-pinned CHZZK presentation expressions without executing its app.
// This helper writes nothing; the acquisition tool owns staging and verification.
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const require = createRequire(import.meta.url);
const parserPath = require.resolve('rolldown/parseAst', {
  paths: [path.dirname(require.resolve('vite'))],
});
const { parseAst } = await import(pathToFileURL(parserPath).href);
const PIN = '40a9c54387bb40ed0136147ba5626647f138777def7cecc574b7a41536d982fc';
const LOCALE_PIN = '8e76cb2eed30e8c14f1401fab339909ae574ec7dac53999d6f6006e68d294f08';
const digest = (value) => createHash('sha256').update(value).digest('hex');
const invariant = (condition, message) => { if (!condition) throw new Error(message); };

function walk(node, visit) {
  if (!node || typeof node !== 'object') return;
  if (node.type) visit(node);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) value.forEach((child) => walk(child, visit));
    else if (value && typeof value === 'object') walk(value, visit);
  }
}

/** Reads the pinned flat locale object as AST data, never by evaluating its JS. */
export function extractOriginalChatTranslations(localeSource) {
  invariant(typeof localeSource === 'string', 'Expected UTF-8-decoded original locale source');
  invariant(digest(localeSource) === LOCALE_PIN, 'Original CHZZK locale source pin mismatch');
  const ast = parseAst(localeSource);
  const objects = ast.body.flatMap((statement) => statement.type === 'VariableDeclaration'
    ? statement.declarations.filter((node) => node.init?.type === 'ObjectExpression').map((node) => node.init) : []);
  invariant(objects.length === 1, 'Expected one original flat locale object');
  const translations = Object.create(null);
  for (const property of objects[0].properties) {
    invariant(property.type === 'Property' && !property.computed && property.kind === 'init' && !property.method,
      'Unexpected executable or computed original locale property');
    const key = property.key.type === 'Identifier' ? property.key.name : property.key.value;
    const value = property.value.type === 'Literal' && typeof property.value.value === 'string'
      ? property.value.value
      : property.value.type === 'TemplateLiteral' && property.value.expressions.length === 0
        && property.value.quasis.length === 1 ? property.value.quasis[0].value.cooked : undefined;
    invariant(typeof key === 'string' && typeof value === 'string', 'Expected original flat string locale entry');
    invariant(!Object.hasOwn(translations, key), `Duplicate original locale key: ${key}`);
    translations[key] = value;
  }
  return Object.freeze(translations);
}

/** Returns source strings and provenance only; never writes files or starts CHZZK. */
export function extractOriginalChatPresentation({ indexSource, sourceSha256 = PIN, translations = {} }) {
  invariant(typeof indexSource === 'string', 'Expected UTF-8-decoded original index source');
  invariant(sourceSha256 === PIN && digest(indexSource) === PIN, 'Original CHZZK chat source pin mismatch');
  const ast = parseAst(indexSource);
  const declarations = new Map(ast.body.flatMap((statement) => statement.type === 'VariableDeclaration'
    ? statement.declarations.filter((node) => node.id.type === 'Identifier').map((node) => [node.id.name, node]) : []));
  const components = [];
  const substitutions = [];
  const get = (name) => {
    const node = declarations.get(name);
    invariant(node?.init, `Missing original declaration: ${name}`);
    return node;
  };
  const original = (name) => {
    const node = get(name);
    const expression = indexSource.slice(node.init.start, node.init.end);
    components.push({ name, start: node.start, end: node.end, expressionStart: node.init.start,
      expressionEnd: node.init.end, sha256: digest(expression), transformation: 'verbatim expression' });
    return `const ${name}=${expression};`;
  };
  const source = (node) => indexSource.slice(node.start, node.end);
  const isElement = (node) => node?.type === 'CallExpression' && source(node.callee) === 'K.createElement';
  const hasClass = (node, className) => isElement(node) && node.arguments[1]?.type === 'ObjectExpression'
    && node.arguments[1].properties.some((property) => property.key?.name === 'className' && source(property.value) === className);
  const find = (parent, predicate, label) => {
    const found = [];
    walk(parent, (node) => { if (predicate(node)) found.push(node); });
    invariant(found.length === 1, `Expected one original ${label}, found ${found.length}`);
    return found[0];
  };
  const replace = (root, changes) => {
    const sorted = [...changes].sort((a, b) => a.node.start - b.node.start);
    let cursor = root.start, result = '';
    for (const { node, replacement, reason } of sorted) {
      invariant(node.start >= cursor && node.end <= root.end, `Overlapping chat substitution: ${reason}`);
      result += indexSource.slice(cursor, node.start) + replacement;
      substitutions.push({ start: node.start, end: node.end, originalSha256: digest(source(node)), replacement, reason });
      cursor = node.end;
    }
    return result + indexSource.slice(cursor, root.end);
  };

  const jq = get('JQ').init;
  const shell = find(jq, (node) => hasClass(node, 'qQ.container'), 'VOD chat shell');
  const header = find(shell, (node) => hasClass(node, 'qQ.header'), 'chat header');
  const heading = find(header, (node) => hasClass(node, 'qQ.header_title'), 'chat heading');
  const close = find(header, (node) => hasClass(node, 'qQ.close_button'), 'chat close button');
  const list = find(shell, (node) => hasClass(node, 'qQ.list'), 'chat list');
  const floating = find(shell, (node) => hasClass(node, 'qQ.floating'), 'floating controls');
  const content = find(shell, (node) => hasClass(node, 'qQ.content'), 'chat content');
  invariant(list.arguments.length === 4 && list.arguments[3].type === 'ConditionalExpression', 'Original chat loader/list boundary changed');
  invariant(content.arguments.length === 6, 'Original chat popup boundary changed');
  const shellExpression = replace(shell, [
    { node: heading.arguments[2], replacement: 'title', reason: 'local replay title slot' },
    { node: close, replacement: `headerControls,onClose?${source(close)}:null`, reason: 'local header controls and optional close action' },
    { node: list.arguments[1], replacement: '{className:qQ.list,ref:te,onScroll,style}', reason: 'local virtual-list ref, scrolling and layout' },
    { node: list.arguments[3], replacement: 'children', reason: 'local replay rows replace online loading, timing and donation dispatcher' },
    { node: floating.arguments[2], replacement: 'floatingContent', reason: 'local floating controls slot' },
    { node: content.arguments[4], replacement: 'footer', reason: 'local status/footer slot outside list replaces online subscription gift popup' },
    { node: content.arguments[5], replacement: 'null', reason: 'online donation toast omitted' },
  ]);

  // Select the first original normal/text row wrapper, not a donation renderer.
  const rowCalls = [];
  walk(list, (node) => {
    if (isElement(node) && node.arguments[2]?.type === 'CallExpression'
      && source(node.arguments[2].arguments?.[0] ?? { start: 0, end: 0 }) === 'sC') rowCalls.push(node);
  });
  invariant(rowCalls.length === 2, 'Original normal/text row wrapper boundary changed');
  const row = rowCalls[0];
  const rowProps = row.arguments[2].arguments[1];
  invariant(rowProps.type === 'ObjectExpression', 'Original message props changed');
  const rowExpression = replace(row, [{ node: rowProps,
    replacement: source(rowProps).slice(0, -1) + ',chatScale}', reason: 'optional original popup-chat scaling prop' }]);
  components.push({ name: 'JQ.shell', start: shell.start, end: shell.end, sha256: digest(source(shell)), transformation: 'recorded local slots' });
  components.push({ name: 'JQ.textRow', start: row.start, end: row.end, sha256: digest(source(row)), transformation: 'recorded scaling prop' });

  const names = ['Nt', 'TD', 'gS', 'Hc', 'il', 'rl', 'tl', 'al', 'l_', 'u_', 'd_', 'f_', 'h_', 'v_', 'Hu', 'nv',
    'rv', 'iv', 'av', 'Ay', 'jy', 'Iy', 'aC', 'oC', 'sC', 'qQ'];
  const originals = names.map(original).join('\n');
  const localeKeys = new Set();
  for (const component of components) {
    const text = indexSource.slice(component.expressionStart ?? component.start, component.expressionEnd ?? component.end);
    for (const match of text.matchAll(/\$t\(`([^`]+)`/g)) localeKeys.add(match[1]);
  }
  const locale = Object.fromEntries([...localeKeys].sort().filter((key) => typeof translations[key] === 'string')
    .map((key) => [key, translations[key]]));
  const moduleSource = `// Generated from pinned CHZZK public presentation expressions. Do not hand edit.
// No CHZZK application entry, socket, account API, storage cache or online popup is imported.
export function createOriginalChatPresentation(React, ReactDOM = {}) {
  const K=React;
  const ReadOnlyContext=K.createContext({theme:'dark',badgeAssets:[],nicknameColors:[]});
  const noop=()=>{};
  const denied=()=>{throw new Error('Online CHZZK action unavailable in local replay');};
  const disabledComponent=()=>null;
  const locale=${JSON.stringify(locale)};
  const $t=(key,values={})=>{const value=locale[key]??key;return value.replace(/{{\\s*([^}, ]+)[^}]*}}/g,(match,name)=>Object.hasOwn(values,name)?String(values[name]):match);};
  const classes=(...values)=>values.flatMap(value=>!value?[]:typeof value==='string'||typeof value==='number'?[String(value)]:Array.isArray(value)?[classes(...value)]:Object.keys(value).filter(key=>value[key])).filter(Boolean).join(' ');
  const q={default:classes};
  const J={isNil:value=>value==null,isEmpty:value=>value==null||typeof value==='string'||Array.isArray(value)?!value?.length:typeof value==='object'?!Object.keys(value).length:true,find:(values,match)=>(values??[]).find(value=>Object.entries(match).every(([key,wanted])=>value?.[key]===wanted))};
  const c_={createPortal:(element,target)=>ReactDOM.createPortal?ReactDOM.createPortal(element,target):element};
  const Ut='channel',Gt='chatChannel',Zn='selectedBadges',el='badgeEditor',Mr='nicknameColors',jr='selectedColor',Qc='badgeAssets';
  const bn='profilePopup',Vn='emoji',Pn='emojiOwnedPopup',Hn='emojiPurchasePopup',js='viewer';
  const G=token=>{const context=K.useContext(ReadOnlyContext);if(token===Mr)return context.nicknameColors??[];if(token===Qc)return context.badgeAssets??[];if(token===jr||token===Gt)return '';if(token===js)return {beforeLoaded:false,loggedIn:false,userIdHash:''};if(token===el||token===bn)return false;if(token===Zn)return [];return undefined;};
  const U=token=>{const value=G(token),context=K.useContext(ReadOnlyContext);return [value,token===bn?value=>{if(value)context.onNicknameClick?.();}:noop];};
  const H=()=>noop;
  const w=()=>({theme:K.useContext(ReadOnlyContext).theme??'dark'});
  const We=()=>({channelId:''});
  const Ne=noop,Oy=()=>false,ky=()=>false;
  const Xg=denied,Xb=denied,Wa=noop,Y9=noop,ys=denied,Ge=()=>false;
  const bC=disabledComponent,__=disabledComponent,$_ =disabledComponent,kt=disabledComponent,m_=disabledComponent,p_=disabledComponent,ee=disabledComponent;
  const Et={},Ot={},Tt={},wt={};
  ${originals}
  function ChatRow({chatMessage,onNicknameClick,listRef,chatScale,theme='dark',badgeAssets=[],nicknameColors=[],isCleanBotWorking=true}) {
    if(!chatMessage||typeof chatMessage.key!=='string'||!chatMessage.key)return null;
    const e=chatMessage,l=isCleanBotWorking,M=undefined,N=noop,te=listRef,ae=noop;
    return K.createElement(ReadOnlyContext.Provider,{value:{theme,badgeAssets,nicknameColors,onNicknameClick}},${rowExpression});
  }
  function ChatShell({children,title='채팅',headerControls,onClose,listRef,listBottomRef,onScroll,style,floatingContent,footer}) {
    const re=onClose,te=listRef,ne=listBottomRef;
    return ${shellExpression};
  }
  return {ChatRow,ChatShell,MenuIcon:gS};
}
`;
  // Parse, rather than execute, the generated presentation module.
  parseAst(moduleSource);
  return {
    moduleSource,
    declarationSource: `import type * as React from 'react';
export type OriginalChatMessage = { key: string; user?: string; time?: number; type?: number; status: 'NORMAL' | 'BLIND' | 'CBOTBLIND'; content: React.ReactNode; extras?: unknown; profile: Record<string, any> | null; displayBadgeList: Array<Record<string, any>>; displayNicknameColor: { light: string | null; dark: string | null } };
export type ChatRowProps = { chatMessage: OriginalChatMessage; onNicknameClick?: () => void; listRef?: React.RefObject<HTMLDivElement | null>; chatScale?: number; theme?: 'dark' | 'light'; badgeAssets?: Array<Record<string, any>>; nicknameColors?: Array<Record<string, any>>; isCleanBotWorking?: boolean };
export type ChatShellProps = { children?: React.ReactNode; title?: React.ReactNode; headerControls?: React.ReactNode; onClose?: () => void; listRef?: React.Ref<HTMLDivElement>; listBottomRef?: React.Ref<HTMLDivElement>; onScroll?: React.UIEventHandler<HTMLDivElement>; style?: React.CSSProperties; floatingContent?: React.ReactNode; footer?: React.ReactNode };
export declare function createOriginalChatPresentation(react: typeof React, reactDOM?: { createPortal?: (children: React.ReactNode, container: Element | DocumentFragment) => React.ReactPortal }): { ChatRow: React.ComponentType<ChatRowProps>; ChatShell: React.ComponentType<ChatShellProps>; MenuIcon: React.ComponentType<React.SVGProps<SVGSVGElement> & { title?: string; titleId?: string }> };
`,
    manifest: {
      version: 1, sourceFile: 'index-C4sif-4p.js', sourceSha256: PIN, offsets: 'zero-based UTF-16 code units, end exclusive',
      components, substitutions, translations: locale,
      boundaries: {
        original: 'verbatim React message/nickname/badge/name/tooltip expressions; original VOD shell with recorded slots',
        local: 'React, ReactDOM portal, classnames, locale interpolation, read-only atom/theme/router adapters and local nickname callback',
        disabled: ['online chat connection', 'account access', 'badge editor', 'profile API popup', 'emoji purchase', 'donation and gift UI', 'online badge cache loader'],
        input: 'Caller supplies sanitized original-shaped profile/content and locally owned image URLs; av recomputes display badges from profile via rv',
      },
      cssFamilies: ['189hq', 'w9pvh', '1mc5x', '1iatj', '1nwpy'],
      moduleSha256: digest(moduleSource),
    },
  };
}
