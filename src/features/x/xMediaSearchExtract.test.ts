import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_media_search_extract.js?raw';
type Metrics={styles:number;rects:number;queries:number};
function extract(html:string, options:{provider?:string;url?:string;verified?:boolean;probe?:boolean;ready?:string;metrics?:Metrics;now?:()=>number}={}) {
  const doc=new DOMParser().parseFromString(html,'text/html');
  Object.defineProperty(doc,'readyState',{value:options.ready??'complete'});
  vi.spyOn(Element.prototype,'getBoundingClientRect').mockImplementation(function(this:Element){if(options.metrics)options.metrics.rects++;const top=this.hasAttribute('data-offscreen')?1000:0,width=this.hasAttribute('data-zero-rect')?0:600;return {width,height:40,top,left:0,bottom:top+40,right:width,x:0,y:top,toJSON(){}};});
  const query=doc.querySelectorAll.bind(doc);vi.spyOn(doc,'querySelectorAll').mockImplementation((selector:string)=>{if(options.metrics)options.metrics.queries++;return query(selector);});
  const location=new URL(options.url??'https://www.google.com/search?fixture=1');
  return new Function('document','location','Element','getComputedStyle','innerWidth','innerHeight','URL','performance',`return (${script.replace('__ATSUMI_MEDIA_SEARCH__',JSON.stringify({nonce:'fixture',provider:options.provider??'lens',probe:options.probe===true,lensVerified:options.verified===true,saucenaoVerified:options.provider==='saucenao'&&options.verified===true}))})`)(doc,location,Element,(node:HTMLElement)=>{if(options.metrics)options.metrics.styles++;return {display:node.style.display||'block',visibility:node.style.visibility||'visible',opacity:node.style.opacity||'1'};},1100,900,URL,{now:options.now??(()=>performance.now())}) as {nonce:string;state:string;candidates:Array<{title:string;creator:string|null;similarity:number|null;links:Array<{label:string;url:string}>}>;probe?:{title:string;body:string;outline:unknown[];textBlocks:unknown[];limited:boolean;evidenceLimited:boolean}};
}
afterEach(()=>vi.restoreAllMocks());
it('does not claim unobserved provider layouts as complete',()=>{
  const result=extract('<main><a href="https://example.org/art"><h3>Landscape source</h3></a></main>');
  expect(result).toEqual({nonce:'fixture',state:'unknown',candidates:[]});
});
it('reads only bounded visible semantic candidates after an evidence gate',()=>{
  const result=extract('<main><a href="https://example.org/art"><h3>Landscape source</h3></a><a hidden href="https://example.org/private"><h3>Hidden</h3></a><a href="javascript:alert(1)"><h3>Bad</h3></a></main>',{verified:true});
  expect(result.state).toBe('complete');expect(result.candidates).toEqual([{title:'Landscape source',creator:null,similarity:null,links:[{label:'example.org',url:'https://example.org/art'}]}]);
});
it.each([
  ['<title>Just a moment...</title><h1>Checking your browser</h1>','challenge'],
  ['<p>Our systems have detected unusual traffic</p>','challenge'],
  ['<h1>Before you continue to Google</h1>','consent'],
  ['<p>Daily search limit exceeded</p>','quota'],
])('recognizes terminal public provider states without results: %s',(html,state)=>expect(extract(html).state).toBe(state));
it('does not treat a bootstrap/hidden error document as an empty match',()=>{
  expect(extract('<div hidden>Search result</div>',{ready:'loading'}).state).toBe('pending');
  expect(extract('<p>Unknown page layout</p>').state).toBe('unknown');
});
it('never accepts a different origin or attaches diagnostic content in production',()=>{
  expect(extract('<main>private</main>',{url:'https://example.org/search',probe:true}).probe).toBeUndefined();
  expect(extract('<main>visible</main>').probe).toBeUndefined();
});
it('bounds probe output and removes Google query tokens from link evidence',()=>{
  const result=extract('<main><a href="https://www.google.com/search?secret=fixture">Search</a><script>secret runtime state</script></main>',{probe:true});
  const serialized=JSON.stringify(result.probe);expect(serialized).toContain('https://www.google.com/search');expect(serialized).not.toContain('secret=fixture');
  expect(serialized).not.toContain('secret runtime state');
});
it('caps and deduplicates candidate links without inventing creator or similarity',()=>{
  const html='<main>'+Array.from({length:20},(_,i)=>`<a href="https://example.org/${i}"><h3>Result ${i}</h3></a>`).join('')+'</main>';
  const result=extract(html,{verified:true});expect(result.candidates).toHaveLength(10);expect(result.candidates.every(v=>v.creator===null&&v.similarity===null)).toBe(true);
});
it('caps titles by UTF-16 units without splitting an emoji surrogate pair',()=>{
  const result=extract('<main><a href="https://example.org/art"><h3>'+('가'.repeat(511))+'😀'+'</h3></a></main>',{verified:true});
  const candidate=result.candidates[0];
  expect(candidate).toBeDefined();
  if(!candidate)throw new Error('Missing fixture candidate');
  expect(candidate.title.length).toBeLessThanOrEqual(512);
  expect(candidate.title).toBe('가'.repeat(511));
});
it('projects the actually observed center_col heading links and keeps opaque Google goto URLs unchanged',()=>{
  const url='https://www.google.com/goto?url=OPAQUE_fixture%3D%3D&source=images';
  const html=`<div id="center_col" role="main"><section><h3>AI overview without a source link</h3><p>Not an artist assertion.</p></section><a href="${url}"><div><h3>Observed visual match</h3></div></a></div>`;
  const result=extract(html,{verified:true});
  expect(result.state).toBe('complete');expect(result.candidates).toEqual([{title:'Observed visual match',creator:null,similarity:null,links:[{label:'www.google.com',url}]}]);
});
it('only lets the explicit synthetic probe inspect the parser while the production evidence gate is false',()=>{
  const html='<main><a href="https://www.google.com/goto?url=OPAQUE"><h3>Observed title</h3></a></main>';
  expect(extract(html).state).toBe('unknown');expect(extract(html,{probe:true}).state).toBe('complete');
  expect(extract(html,{provider:'saucenao',url:'https://saucenao.com/search.php',probe:true}).state).toBe('unknown');
});
it.each(['https://www.google.com/search?q=other','https://accounts.google.com/login','https://www.google.com/goto','https://www.google.com/goto?url=','https://www.google.com/goto?url=one&url=two','https://lens.google.com/goto?url=opaque'])('does not turn unrelated provider navigation into a result: %s',url=>{
  expect(extract(`<main><a href="${url}"><h3>Navigation</h3></a></main>`,{verified:true}).candidates).toEqual([]);
});
it('rejects hidden, inert, non-rendered, outside-main and ambiguous-main candidates',()=>{
  const items=['hidden','inert','aria-hidden="true"','style="opacity:0"','style="display:none"','style="visibility:hidden"'];
  for(const attr of items){expect(extract(`<main><div ${attr}><a href="https://example.org/hidden"><h3>Hidden</h3></a></div></main>`,{verified:true}).candidates).toEqual([]);}
  expect(extract('<main><a data-zero-rect href="https://example.org/zero"><h3>Non-rendered link</h3></a></main>',{verified:true}).candidates).toEqual([]);
  expect(extract('<main>Visible main</main><a href="https://example.org/out"><h3>Outside</h3></a>',{verified:true}).candidates).toEqual([]);
  expect(extract('<main><a href="https://example.org/one"><h3>One</h3></a></main><div role="main">Another main</div>',{verified:true}).state).toBe('unknown');
});
it('reads already-rendered below-fold ordinary results without adopting the observed AI overview citations',()=>{
  // Root's actual Lens probe1792619176702 shows AI overview + a.alLuqe
  // citations without h3; the ordinary main/result region was below viewport.
  // This fixture records that relation, not an invented SauceNAO or AI adapter.
  const html='<div role="heading">AI 개요</div><section><p>파란색 사각형과 주황색 원입니다.</p>'
    +'<a class="alLuqe" href="https://www.google.com/goto?url=AI-citation">Démarche FLE</a></section>'
    +'<div id="center_col" role="main" data-offscreen><section><a class="alLuqe" href="https://www.google.com/goto?url=another-citation">Clip Studio TIPS</a></section>'
    +'<a data-offscreen href="https://www.google.com/goto?url=observed-opaque"><h3 data-offscreen>Rendered ordinary result</h3></a></div>';
  const result=extract(html,{probe:true,now:()=>0});expect(result.state).toBe('complete');
  expect(result.candidates).toEqual([{title:'Rendered ordinary result',creator:null,similarity:null,links:[{label:'www.google.com',url:'https://www.google.com/goto?url=observed-opaque'}]}]);
  const heading=result.probe?.outline.find(item=>(item as {tag?:string}).tag==='h3');
  expect(heading).toMatchObject({text:'Rendered ordinary result',context:{main:{tag:'div',id:'center_col',role:'main'},anchor:{url:'https://www.google.com/goto',rendered:true}}});
  expect(JSON.stringify(result.probe)).not.toContain('observed-opaque');expect(result.probe?.limited).toBe(false);
});
it('does not infer matches from a rendered AI overview or below-fold unrecognized layout alone',()=>{
  const html='<main data-offscreen><div role="heading">AI 개요</div><section><a class="alLuqe" href="https://www.google.com/goto?url=citation">A cited site</a><p>AI response</p></section></main>';
  const result=extract(html,{probe:true});expect(result.state).toBe('unknown');expect(result.candidates).toEqual([]);expect(result.probe?.body).toContain('AI response');
});
it('adds bounded probe-only rendered structural evidence without assuming provider classes',()=>{
  const html='<div class="observed-wrapper"><div class="observed-item"><div class="observed-number">52.56%</div><span class="observed-title">Displayed title</span><div class="observed-source"><a href="https://example.org/source">Source</a></div></div></div><div hidden class="private-wrapper"><span>Hidden title</span></div>';
  const result=extract(html,{provider:'saucenao',url:'https://saucenao.com/search.php',probe:true,now:()=>0});
  expect(result.state).toBe('unknown');expect(result.candidates).toEqual([]);
  expect(result.probe?.textBlocks).toContainEqual(expect.objectContaining({tag:'div',class:'observed-number',text:'52.56%',ancestors:expect.arrayContaining([expect.objectContaining({class:'observed-item'})])}));
  expect(result.probe?.textBlocks).toContainEqual(expect.objectContaining({tag:'span',class:'observed-title',text:'Displayed title'}));
  const source=result.probe?.outline.find(item=>(item as {tag?:string}).tag==='a');
  expect(source).toMatchObject({ancestors:[{tag:'div',class:'observed-source'},{tag:'div',class:'observed-item'},{tag:'div',class:'observed-wrapper'},{tag:'body'}]});
  expect(JSON.stringify(result.probe)).not.toContain('private-wrapper');expect(JSON.stringify(result.probe)).not.toContain('Hidden title');
  expect(extract(html,{provider:'saucenao',url:'https://saucenao.com/search.php'}).probe).toBeUndefined();
});
it('caps structural probe extras independently and retains later semantic headings',()=>{
  const html='<main>'+Array.from({length:100},(_,i)=>`<div class="shape-${i}"><span>Label ${i}</span></div>`).join('')+'<a href="https://example.org/last"><h3>Still retained</h3></a></main>';
  const result=extract(html,{probe:true,now:()=>0});
  expect(result.probe?.textBlocks).toHaveLength(64);expect(result.probe?.outline).toContainEqual(expect.objectContaining({tag:'h3',text:'Still retained'}));
  expect(result.probe?.textBlocks.every(item=>(item as {ancestors:unknown[]}).ancestors.length<=4)).toBe(true);
  expect(result.state).toBe('complete');
});
it('does not call CSS/geometry or enumerate the DOM at all on an unrecognized origin',()=>{
  const metrics={styles:0,rects:0,queries:0};expect(extract('<main>Not a provider</main>',{url:'https://example.org/',probe:true,metrics}).state).toBe('unknown');expect(metrics).toEqual({styles:0,rects:0,queries:0});
});
it('uses one bounded projection for overlapping probe subtrees instead of rescanning each outline node',()=>{
  const metrics={styles:0,rects:0,queries:0},html='<main>'+Array.from({length:200},(_,i)=>`<section><article><a href="https://example.org/${i}"><h3>Title ${i}</h3><p>Visible description ${i}</p></a></article></section>`).join('')+'</main>';
  const result=extract(html,{probe:true,metrics,now:()=>0});expect(result.state).toBe('complete');expect(result.probe?.limited).toBe(false);expect(result.probe?.outline.length).toBeLessThanOrEqual(250);
  expect(metrics.styles).toBeLessThan(1300);expect(metrics.rects).toBe(metrics.styles);expect(metrics.queries).toBe(0);
});
it('stops visiting wide sibling lists at the global node bound and never reports partial work as empty or complete',()=>{
  const metrics={styles:0,rects:0,queries:0};
  const html='<main><a href="https://example.org/first"><h3>First</h3></a>'+('<span></span>'.repeat(12000))+'</main>';
  const result=extract(html,{verified:true,probe:true,metrics,now:()=>0});expect(result.state).toBe('unknown');expect(result.candidates).toEqual([]);expect(result.probe?.limited).toBe(true);expect(metrics.styles).toBeLessThanOrEqual(6000);expect(metrics.queries).toBe(0);
});
it('bounds depth, text volume and elapsed work independently without claiming an empty result',()=>{
  const deep=extract('<main>'+('<div>'.repeat(150))+'Visible'+('</div>'.repeat(150))+'</main>',{probe:true,now:()=>0});expect(deep.state).toBe('unknown');expect(deep.probe?.limited).toBe(true);
  const large=extract('<main><p>'+('가'.repeat(100000))+'</p></main>',{probe:true,now:()=>0});expect(large.state).toBe('unknown');expect(large.probe?.body.length).toBeLessThanOrEqual(24000);expect(large.probe?.limited).toBe(true);
  let clock=0;const expired=extract('<main><a href="https://example.org/art"><h3>Source</h3></a></main>',{probe:true,now:()=>clock+=100});expect(expired.state).toBe('unknown');expect(expired.candidates).toEqual([]);expect(expired.probe?.limited).toBe(true);
});
it('skips hidden subtrees without traversing all of their children',()=>{
  const metrics={styles:0,rects:0,queries:0};const result=extract('<div hidden>'+('<span>private</span>'.repeat(12000))+'</div><main><a href="https://example.org/a"><h3>Visible</h3></a></main>',{verified:true,probe:true,metrics,now:()=>0});
  expect(result.state).toBe('complete');expect(result.probe?.limited).toBe(false);expect(JSON.stringify(result)).not.toContain('private');expect(metrics.styles).toBeLessThan(10);
});
it('bounds UTF-8 probe evidence independently of the native reply cap',()=>{
  const html='<main>'+Array.from({length:250},(_,i)=>`<a id="${'가'.repeat(80)}" class="${'가'.repeat(240)}" href="https://example.org/${i}?q=${'x'.repeat(1900)}"><h3>Match ${i}</h3></a>`).join('')+'</main>';
  const result=extract(html,{probe:true,now:()=>0});expect(new TextEncoder().encode(JSON.stringify(result)).byteLength).toBeLessThan(512*1024);expect(result.probe?.evidenceLimited).toBe(true);expect(result.state).toBe('complete');
});

// Minimal reconstruction from native probe1792619714359's actual rendered
// classes, ancestry, title/score and source/creator link labels, not live HTML.
function sauceCard(options:{title?:string;score?:string;columns?:string;extra?:string;attrs?:string}={}) {
  return `<div class="result" ${options.attrs??''}><table class="resulttable"><tbody><tr><td class="resulttableimage"><div class="resultimage"><a class="linkify" href="https://saucenao.com/search.php?url=thumbnail">Thumbnail repeat</a></div></td><td class="resulttablecontent"><div class="resultmatchinfo"><div class="resultsimilarityinfo">${options.score??'52.56%'}</div></div><div class="resultcontent"><div class="resulttitle">${options.title??'【ＧＩＦ】エレベーター争奪戦'}</div><div class="resultcontentcolumn">${options.columns??'Seiga ID: <a class="linkify" href="https://seiga.nicovideo.jp/seiga/im3596411">3596411</a><span><a href="https://saucenao.com/info.php?id=3596411">Info</a></span> Member: <a class="linkify" href="https://seiga.nicovideo.jp/user/illust/11493614">noverun</a>'}</div>${options.extra??''}</div></td></tr></tbody></table></div>`;
}
const saucePage=(cards:string)=>`<div id="mainarea"><div id="middle">${cards}</div></div>`;
const sauceOptions={provider:'saucenao',url:'https://saucenao.com/search.php',verified:true,now:()=>0};
it('projects two observed SauceNAO cards without mixing their source and creator metadata',()=>{
  const html=saucePage(sauceCard()+sauceCard({title:'Coffee table and gadgets',score:'52.25%',columns:'ArtStation Project: <a class="linkify" href="https://www.artstation.com/artwork/9el0Eq">9el0Eq</a> Author: <a class="linkify" href="https://www.artstation.com/hanyang1">Han Yang</a>'})+'<div class="result" id="result-hidden-notification">Low similarity results have been hidden. Click here to display them...</div>');
  const result=extract(html,sauceOptions);expect(result.state).toBe('complete');
  expect(result.candidates).toEqual([
    {title:'【ＧＩＦ】エレベーター争奪戦',creator:'noverun',similarity:52.56,links:[{label:'3596411',url:'https://seiga.nicovideo.jp/seiga/im3596411'},{label:'noverun',url:'https://seiga.nicovideo.jp/user/illust/11493614'}]},
    {title:'Coffee table and gadgets',creator:'Han Yang',similarity:52.25,links:[{label:'9el0Eq',url:'https://www.artstation.com/artwork/9el0Eq'},{label:'Han Yang',url:'https://www.artstation.com/hanyang1'}]},
  ]);
});
it('keeps SauceNAO production gated while allowing the explicitly approved synthetic probe',()=>{
  const html=saucePage(sauceCard());expect(extract(html,{...sauceOptions,verified:false}).state).toBe('unknown');
  expect(extract(html,{...sauceOptions,verified:false,probe:true}).state).toBe('complete');
});
it.each(['hidden','inert','style="display:none"','aria-hidden="true"','style="opacity:0"','data-zero-rect','id="result-hidden-notification"'])('does not expose hidden or notification SauceNAO result cards: %s',attrs=>{
  const result=extract('<h1>Search results</h1>'+saucePage(sauceCard({attrs})),sauceOptions);expect(result.state).toBe('unknown');expect(result.candidates).toEqual([]);
});
it.each(['-1%','100.01%','NaN%','52.56% guessed','52,56%',''])('rejects unsupported SauceNAO similarity values: %s',score=>{
  expect(extract(saucePage(sauceCard({score})),sauceOptions).candidates).toEqual([]);
});
it('preserves truthful zero and full similarity without classifying them as verified identity',()=>{
  const result=extract(saucePage(sauceCard({score:'0%',columns:'Source: <a class="linkify" href="https://example.org/zero">Zero</a>'})+sauceCard({score:'100%',columns:'Source: <a class="linkify" href="https://example.org/full">Full</a>'})),sauceOptions);
  expect(result.candidates.map(v=>[v.similarity,v.creator])).toEqual([[0,null],[100,null]]);
});
it('never invents a creator from an unlabeled, hidden or conflicting profile link',()=>{
  const base='<a class="linkify" href="https://example.org/source">Artwork</a>';
  for(const columns of [base+' By <a class="linkify" href="https://example.org/person">Name</a>',base+'<span hidden>Author: <a class="linkify" href="https://example.org/hidden">Private</a></span>',base+' Author: <a class="linkify" href="https://example.org/one">One</a> Member: <a class="linkify" href="https://example.org/two">Two</a>']){
    expect(extract(saucePage(sauceCard({columns})),sauceOptions).candidates[0]?.creator).toBeNull();
  }
});
it('rejects provider actions and unsafe URLs while bounding unique actual source links',()=>{
  const columns='<a class="linkify" href="https://saucenao.com/info.php?id=1">Info</a><a class="linkify" href="javascript:alert(1)">Unsafe</a><a class="linkify" href="https://name:secret@example.org/">Credentials</a>'+Array.from({length:6},(_,i)=>`<a class="linkify" href="https://example.org/${i}">Source ${i}</a>`).join('');
  const result=extract(saucePage(sauceCard({columns})),sauceOptions);expect(result.candidates[0]?.links).toEqual([0,1,2].map(i=>({label:`Source ${i}`,url:`https://example.org/${i}`})));
  expect(extract(saucePage(sauceCard({columns:'<a class="linkify" href="https://saucenao.com/search.php?url=repeat">Repeat</a>'})),sauceOptions).state).toBe('unknown');
});
it('rejects ambiguous nested, duplicate or outside-container SauceNAO card structures',()=>{
  expect(extract(sauceCard(),sauceOptions).candidates).toEqual([]);
  expect(extract(saucePage(sauceCard())+saucePage(sauceCard()),sauceOptions).state).toBe('unknown');
  expect(extract(saucePage(sauceCard({extra:sauceCard()})),sauceOptions).candidates).toEqual([]);
  expect(extract(saucePage(sauceCard({extra:'<div class="resulttitle">Other title</div>'})),sauceOptions).candidates).toEqual([]);
});
it('does not borrow missing metadata from a neighbouring incomplete SauceNAO card',()=>{
  const html=saucePage(sauceCard().replace('class="resulttitle"','class="unrecognized"')+sauceCard().replace('class="resultsimilarityinfo"','class="unrecognized"'));
  expect(extract(html,sauceOptions).state).toBe('unknown');
});
it('caps SauceNAO candidates and globally incomplete projections remain unknown',()=>{
  const cards=Array.from({length:12},(_,i)=>sauceCard({columns:`Source: <a class="linkify" href="https://example.org/${i}">Item ${i}</a>`})).join('');
  const metrics={styles:0,rects:0,queries:0};expect(extract(saucePage(cards),{...sauceOptions,metrics}).candidates).toHaveLength(10);expect(metrics.queries).toBe(0);
  expect(extract(saucePage(sauceCard())+'<div>'+('<span></span>'.repeat(7000))+'</div>',sauceOptions).state).toBe('unknown');
});
