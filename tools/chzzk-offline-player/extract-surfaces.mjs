// Presentation expressions from the pinned distribution, not a redraw.
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { extractOriginalChatPresentation } from '../extract_chzzk_chat.mjs';

const require = createRequire(import.meta.url);
const { parseAst } = await import(pathToFileURL(require.resolve('rolldown/parseAst', { paths: [path.dirname(require.resolve('vite'))] })).href);
export function validateBrowserModule(source) { parseAst(source); }
const digest = value => createHash('sha256').update(value).digest('hex');
const check = (value, text) => { if (!value) throw new Error(text); };
const walk = (node, visit) => {
  if (!node || typeof node !== 'object') return;
  if (node.type) visit(node);
  for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(child => walk(child, visit)); else if (value && typeof value === 'object') walk(value, visit);
};

export function extractSurfaces(index, translations) {
  check(digest(index) === '40a9c54387bb40ed0136147ba5626647f138777def7cecc574b7a41536d982fc', 'UI source pin changed');
  const ast = parseAst(index), declarations = new Map(ast.body.flatMap(statement => statement.type === 'VariableDeclaration' ? statement.declarations.map(node => [node.id.name, node]) : []));
  const records = [], substitutions = [];
  const source = node => index.slice(node.start, node.end);
  const declaration = name => { const node = declarations.get(name)?.init; check(node, `Missing original ${name}`); return node; };
  const record = (name, node, changes = []) => {
    records.push({ name, utf16Range: [node.start, node.end], sha256: digest(source(node)), changes });
    return source(node);
  };
  const original = name => `const ${name}=${record(name, declaration(name))};`;
  const find = (root, predicate, name) => {
    const nodes = []; walk(root, node => { if (predicate(node)) nodes.push(node); });
    check(nodes.length === 1, `${name}: expected one boundary, found ${nodes.length}`); return nodes[0];
  };
  const isElement = node => node.type === 'CallExpression' && source(node.callee) === 'K.createElement';
  const hasClass = (node, token) => isElement(node) && node.arguments[1]?.type === 'ObjectExpression' && node.arguments[1].properties.some(prop => prop.key?.name === 'className' && source(prop.value).includes(token));
  const element = (name, token) => find(declaration(name), node => hasClass(node, token), `${name}.${token}`);
  const finalReturn = name => {
    const node = declaration(name), fn = node.type === 'CallExpression' ? node.arguments[0] : node;
    const ret = fn.body.body?.filter(node => node.type === 'ReturnStatement').at(-1);
    check(ret?.argument, `Missing ${name} presentation return`);
    // Discard comma-prefixed app effects; preserve the final JSX expression.
    return ret.argument.type === 'SequenceExpression' ? ret.argument.expressions.at(-1) : ret.argument;
  };
  const replace = (node, changes) => {
    let cursor = node.start, result = '';
    for (const change of changes.sort((a, b) => a.node.start - b.node.start)) {
      check(change.node.start >= cursor && change.node.end <= node.end, 'Overlapping presentation slot');
      result += index.slice(cursor, change.node.start) + change.value;
      substitutions.push({ utf16Range: [change.node.start, change.node.end], originalSha256: digest(source(change.node)), replacement: change.value, reason: change.reason });
      cursor = change.node.end;
    }
    return result + index.slice(cursor, node.end);
  };

  const chat = extractOriginalChatPresentation({ indexSource: index, translations });
  const exportText = 'return {ChatRow,ChatShell,MenuIcon:gS};';
  check(chat.moduleSource.includes(exportText), 'Original message adapter export boundary changed');
  const messages = chat.moduleSource.replace(exportText, 'return {ChatRow,ChatShell,MenuIcon:gS,Name:al,TextMessage:sC,Context:ReadOnlyContext};');

  const broadcastNode = finalReturn('DJ');
  record('DJ.return', broadcastNode, ['Replay binds saved metadata and omits live-only badges/counts; original elements and styling retained']);
  const broadcastChanges = [];
  walk(broadcastNode, node => {
    if (isElement(node) && source(node.arguments[0]) === 'Vc') broadcastChanges.push({ node, value: `(replay?null:${source(node)})`, reason: 'A saved recording is not LIVE' });
    if (node.type === 'MemberExpression' && source(node) === 'EJ.is_live') broadcastChanges.push({ node, value: '(!replay&&EJ.is_live)', reason: 'Do not label recorded profile as live' });
    if (hasClass(node, 'EJ.channel')) broadcastChanges.push({ node, value: `((!replay||channelName)&&${source(node)})`, reason: 'Do not invent an unavailable recording channel name' });
    if (isElement(node) && source(node.arguments[0]) === 'O') broadcastChanges.push({ node, value: `(replay?recordedAtLabel:${source(node)})`, reason: 'Recorded date instead of a fictional live uptime' });
  });
  const broadcast = replace(broadcastNode, broadcastChanges);
  const headerNode = finalReturn('DS'), headerMenu = element('DS', 'ES.menu');
  record('DS.return', headerNode, ['Optional local replay menu uses the original header menu slot']);
  const header = replace(headerNode, [{ node: { start: headerMenu.arguments[2].start, end: headerMenu.arguments.at(-1).end }, value: `controls??K.createElement(K.Fragment,null,${headerMenu.arguments.slice(2).map(source).join(',')})`, reason: 'Local replay menu slot, preserving original header geometry' }]);
  const menuNode = find(declaration('DS'), node => node.type === 'VariableDeclarator' && node.id.name === 'ge', 'header menu factory').init;
  const menu = record('DS.menu', menuNode);
  const input = record('Uv.return', finalReturn('Uv'));
  const menuRow = record('yx.return', finalReturn('yx'), ['Share-plugin loading effect is not executed; original menu return is retained']);
  const aside = element('Nk', 'yD.container'), area = element('Nk', 'yD.area');
  const list = element('vD', 'JE.container'), wrapper = element('vD', 'JE.wrapper'), bottom = element('vD', 'JE.list_bottom');
  const rowNodes = [];
  walk(declaration('vD'), node => { if (isElement(node) && source(node.arguments[0]) === 'sC' && node.arguments[1]?.type === 'ObjectExpression') rowNodes.push(node); });
  check(rowNodes.length === 2, 'Live text row boundaries changed');
  const messageNode = rowNodes[1];
  const row = find(declaration('vD'), node => isElement(node) && node.arguments.includes(messageNode), 'live text row');
  const rowExpression = record('vD.textRow', row);
  const listExpr = replace(list, [{ node: { start: list.arguments[2].start, end: list.arguments.at(-1).end }, value: `K.createElement(${source(wrapper.arguments[0])},{...${source(wrapper.arguments[1])},onScroll},${source(bottom)},children)`, reason: 'Local rows and optional scroll callback; no online list lifecycle' }]);
  record('vD.list', list, ['Local rows slot only; original list, reverse ordering and row geometry']);
  const shellExpr = replace(aside, [{ node: { start: aside.arguments[2].start, end: aside.arguments.at(-1).end }, value: `header,list,floatingContent,K.createElement(${source(area.arguments[0])},{...${source(area.arguments[1])},'data-replay-search-area':replay?'':undefined},input)`, reason: 'Local header/list slots; original input in LIVE, user-requested local search replaces input/tools in replay' }]);
  record('Nk.aside', aside, ['Local presentation slots']);
  const names = ['EJ','Vc','zc','Rc','Bc','ES','TS','pS','CS','wS','vS','yS','_S','Ct','kt','Et','Ot','Dt','wt','Tt','vx','bx','Oc','Cv','dv','uv','fv','pv','hv','mv','JE','yD','TJ'];
  const module = `// Generated from original CHZZK PC 1.16.2. See manifest for exact source ranges.
import { createOriginalChatPresentation } from './original-messages.js';
import { createChatSearchIndex, searchChatMessages, createReplaySearch } from './replay-search.js';
export function createSurfaces(K, ReactDOM, Se, fixture, notify) {
  const {Name:al,MenuIcon:gS,TextMessage:sC,Context}=createOriginalChatPresentation(K,ReactDOM);
  const noop=()=>{};
  const q={default:(...values)=>values.flatMap(value=>!value?[]:typeof value==='string'?[value]:Array.isArray(value)?value:Object.keys(value).filter(key=>value[key])).join(' ')};
  const locale=${JSON.stringify(translations)};
  const $t=(key,values={})=>{if(!Object.hasOwn(locale,key))throw new Error('Missing original label '+key);return locale[key].replace(/{{\\s*([^}, ]+)[^}]*}}/g,(all,name)=>Object.hasOwn(values,name)?String(values[name]):all);};
  const J={isEmpty:value=>!value||!Object.keys(value).length};
  const V=()=>{throw new Error('Online navigation is not part of the offline fixture');};
  const vl=value=>value||fixture.profileImage,El=event=>{const image=event.currentTarget;if(image.getAttribute('src')!==fixture.profileImage)image.setAttribute('src',fixture.profileImage);};
  const Dl=value=>Number(value).toLocaleString('ko-KR');
  function xK(){return fixture.uptime;}
  function O({i18nKey,components}){const text=$t(i18nKey),parts=text.split(/(<uptime\\s*\\/>|<uptime><\\/uptime>)/);return K.createElement(K.Fragment,null,...parts.map((part,i)=>part.startsWith('<uptime')?K.cloneElement(components.uptime,{key:i}):part));}
  ${names.map(original).join('\n')}
  const ReplaySearch=createReplaySearch(K,Cv);
  function yx({link:e,shareLink:t,contents:n,clickHandler:r,showPopupHandler:i,type:a,value:o,depthLayer:s,isSelected:c,isOpen:l,toggleLayer:u}){const d=K.useRef(null);return ${menuRow};}
  function BroadcastInfo({wide=false,chatHidden=false,onChat=noop,replay=false,title=fixture.title,channelName=fixture.channelName,recordedAt=null,profileImage=fixture.profileImage}) {
    const recordedAtLabel=Number.isFinite(recordedAt)&&recordedAt>0?new Date(recordedAt).toLocaleString('ko-KR'):'';
    const a={channel:{channelImageUrl:profileImage,channelName,verifiedMark:false}},o={liveTitle:title,concurrentUserCount:replay?0:fixture.viewers};
    const s=wide,c=false,t='dark',e=true,r=chatHidden,i=false,x=false,p=null,d=null,f=null,u=null,n='',l={userIdHash:''},m=onChat;
    return ${broadcast};
  }
  function Header({onCollapse=noop,controls=null}) {
    const [_,v]=K.useState(false),[t,setClean]=K.useState(false),oe=K.useRef(null);
    const s=false,e=false,D=false,O=false,C=false,n=onCollapse,g=null,S=false,c=100,l=noop,u=noop;
    const ae=()=>v(value=>!value),r=()=>setClean(value=>!value),i=()=>notify('로컬 테스트에는 채팅 규칙 서버가 연결되지 않았습니다.'),a=()=>notify('팝업 채팅은 이 로컬 테스트에 연결하지 않았습니다.'),le=()=>false;
    const ge=${menu};
    K.useEffect(()=>{const close=event=>{if(!oe.current?.contains(event.target))v(false);};document.addEventListener('pointerdown',close);return()=>document.removeEventListener('pointerdown',close);},[]);
    const output=${header};
    return K.createElement(K.Fragment,null,K.Children.map(output.props.children,child=>K.isValidElement(child)?K.cloneElement(child,{'data-replay-header':true}):child));
  }
  function Input() {
    const b=false,t=false,P={},p={loggedIn:false},u=false,i=false,o=false,h='offline-channel',n=false,D=false,f='dark';
    const Te={donationActive:true},De={donationActive:true},Ee={donationActive:true},Oe={donationActive:true},Re=null,ze=false,Ge=false,ie=null;
    const oe={},ke=0,lt=()=>false,$e=()=>false,rt=()=>$t('live_chatting_input.placeholder_login_required');
    const Hv=false,x=noop,Qe=()=>notify('오프라인 외형 테스트입니다. 로그인·채팅 전송은 연결하지 않았습니다.'),ys=noop,it=()=>{Qe();return false;},r=noop,s=noop;
    const ut=Qe,st=Qe,Bh={clickDonationButton:noop},se='',ce='';
    return ${input};
  }
  function Row({message, listRef}) {
    const e=message,h=false,i=false,m=null,y='',b=noop,d=noop,Ve=listRef,s=noop,o=100;
    return K.createElement(Context.Provider,{value:{theme:'dark',badgeAssets:[],nicknameColors:[],onNicknameClick:()=>notify('로컬 샘플 이용자입니다. 실제 프로필 서버에는 연결하지 않습니다.')}},${rowExpression});
  }
  function List({children,listRef,onScroll}){const ot=false,Ve=listRef,He=K.useRef(null);return ${listExpr};}
  function Shell({header,list,input,replay=false,floatingContent=null}){const e=false,re=false,Li=false,Ri=false,te=true;return ${shellExpr};}
  function ReplayShell({children,listRef,onScroll,headerControls,onCollapse=null,footer,floatingContent}) {
    return K.createElement(Shell,{replay:true,header:K.createElement(Header,{onCollapse,controls:headerControls}),list:K.createElement(List,{listRef,onScroll},children),input:footer,floatingContent});
  }
  function Chat({onCollapse=noop,replay=false}) {
    const listRef=K.useRef(null);
    const [query,setQuery]=K.useState('');
    const index=K.useMemo(()=>createChatSearchIndex(fixture.messages),[fixture.messages]);
    const found=K.useMemo(()=>replay?searchChatMessages(index,query):fixture.messages,[index,replay,query]);
    const rows=found.map(message=>K.createElement(Row,{key:message.key,message,listRef})).reverse();
    K.useEffect(()=>{if(replay&&listRef.current)listRef.current.scrollTop=0;},[replay,query]);
    const contents=rows.length||!replay?rows:K.createElement('p',{className:'replay-search__empty'},fixture.messages.length?'검색 결과가 없습니다.':'저장된 채팅이 없습니다.');
    return K.createElement(Shell,{replay,header:K.createElement(Header,{onCollapse}),list:K.createElement(List,{listRef},contents),input:replay?K.createElement(ReplaySearch,{query,count:found.length,onQuery:setQuery}):K.createElement(Input)});
  }
  return {BroadcastInfo,Chat,ReplayShell,ReplaySearch};
}
`;
  parseAst(module);
  return { module, messages, manifest: { sourceFile: 'index-C4sif-4p.js', sourceSha256: digest(index), records, substitutions, messages: chat.manifest, moduleSha256: digest(module), messagesSha256: digest(messages), states: 'Synthetic local channel and text messages; original live header, controls, list and logged-out input. Replay only replaces input/tools with local body/nickname search; no online commerce, account, profile or message transport.' } };
}
