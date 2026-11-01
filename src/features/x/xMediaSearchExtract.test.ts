import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_media_search_extract.js?raw';
type Metrics={styles:number;rects:number;queries:number};
function extract(html:string, options:{provider?:string;url?:string;verified?:boolean;probe?:boolean;ready?:string;metrics?:Metrics;now?:()=>number}={}) {
  const doc=new DOMParser().parseFromString(html,'text/html');
  Object.defineProperty(doc,'readyState',{value:options.ready??'complete'});
  vi.spyOn(Element.prototype,'getBoundingClientRect').mockImplementation(function(this:Element){if(options.metrics)options.metrics.rects++;const top=this.hasAttribute('data-offscreen')?1000:0,width=this.hasAttribute('data-zero-rect')?0:Number(this.getAttribute('data-width')||600),height=Number(this.getAttribute('data-height')||40);return {width,height,top,left:0,bottom:top+height,right:width,x:0,y:top,toJSON(){}};});
  const query=doc.querySelectorAll.bind(doc);vi.spyOn(doc,'querySelectorAll').mockImplementation((selector:string)=>{if(options.metrics)options.metrics.queries++;return query(selector);});
  const location=new URL(options.url??'https://www.google.com/search?fixture=1');
  return new Function('document','location','Element','getComputedStyle','innerWidth','innerHeight','URL','performance',`return (${script.replace('__ATSUMI_MEDIA_SEARCH__',JSON.stringify({nonce:'fixture',provider:options.provider??'lens',probe:options.probe===true,lensVerified:options.verified===true,saucenaoVerified:options.provider==='saucenao'&&options.verified===true}))})`)(doc,location,Element,(node:HTMLElement)=>{if(options.metrics)options.metrics.styles++;return {display:node.style.display||'block',visibility:node.style.visibility||'visible',opacity:node.style.opacity||'1'};},1100,900,URL,{now:options.now??(()=>performance.now())}) as {nonce:string;state:string;candidates:Array<{title:string;creator:string|null;similarity:number|null;thumbnailSource?:string;sourceMetadata?:Array<{label:string;value:string}>;links:Array<{label:string;url:string}>}>;probe?:{title:string;body:string;outline:unknown[];textBlocks:unknown[];limited:boolean;evidenceLimited:boolean}};
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
it('identifies the observed SauceNAO 521 outage without treating generic failure text as no matches',()=>{
  const html='<title>saucenao.com | 521: Web server is down</title><h1>Web server is down Error code 521</h1><p>The web server is not returning a connection.</p>';
  expect(extract(html,{provider:'saucenao',url:'https://saucenao.com/search.php',verified:true})).toMatchObject({state:'unavailable',candidates:[]});
  expect(extract(html,{verified:true}).state).toBe('unknown');
  expect(extract('<h1>Web server is down Error code 521</h1>',{provider:'saucenao',url:'https://saucenao.com/search.php',verified:true}).state).toBe('unknown');
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
const lensVisualTitle=(url:string,title:string)=>`<div class="kb0PBd"><a class="LBcIee" href="${url}"><div class="N54PNb"><div class="kb0PBd"><img data-width="14" data-height="14" src="data:image/png;base64,AQID"><div class="lTYpse"><div role="heading" class="HePszc">${title}</div></div></div></div></a></div>`;
const lensVisualPhoto=(source:string)=>`<div class="kb0PBd pAEXpf"><div class="R8rd3e"><div class="DeNS1c" role="button"><div class="pb66sc"><img data-width="205" data-height="161" src="${source}"></div></div></div></div>`;
it('preserves the observed semantic visual-card heading before lower H3 web results',()=>{
  // Actual neutral Lens probe1792619668276: a.LBcIee contains the visual title
  // as DIV[role=heading], nested under N54PNb/kb0PBd/lTYpse, not an H3.
  const url='https://www.google.com/goto?url=visual-neutral',source='data:image/jpeg;base64,/9j/2Q==';
  const html=`<div id="center_col" role="main"><div class="N54PNb">${lensVisualPhoto(source)}${lensVisualTitle(url,'Blue square and orange circle')}</div><a href="https://example.org/article"><h3>Lower text article</h3></a></div>`;
  const result=extract(html,{verified:true});
  expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(2);
  expect(result.candidates[0]).toMatchObject({title:'Blue square and orange circle',thumbnailSource:source,creator:null,similarity:null,links:[{url}]});
  expect(result.candidates[1]?.title).toBe('Lower text article');
});
it('binds each visual photo to its same outer card and never borrows a neighbor or favicon',()=>{
  const first='data:image/jpeg;base64,/9j/2Q==',second='data:image/png;base64,BAUG';
  const html=`<main><div class="N54PNb">${lensVisualPhoto(first)}${lensVisualTitle('https://example.org/first','First')}</div><div class="N54PNb">${lensVisualPhoto(second)}${lensVisualTitle('https://example.org/second','Second')}</div><div class="N54PNb">${lensVisualTitle('https://example.org/no-photo','No photo')}</div></main>`;
  const items=extract(html,{verified:true}).candidates;
  expect(items.map(item=>item.thumbnailSource)).toEqual([first,second,undefined]);
});
it('keeps actual zero-rect visual wrappers without crossing the nearest outer card',()=>{
  const source='data:image/jpeg;base64,/9j/2Q==';
  const photo=lensVisualPhoto(source).replace('<div class="pb66sc">','<div class="T3Fozb" data-zero-rect><div class="pb66sc">')+'</div>';
  const result=extract(`<main><div class="N54PNb">${photo}${lensVisualTitle('https://example.org/work','Observed wrapped image')}</div></main>`,{verified:true});
  expect(result.candidates[0]?.thumbnailSource).toBe(source);
});
it('omits a visual thumbnail when its same owner has multiple photos or title anchors',()=>{
  const source='data:image/jpeg;base64,/9j/2Q==',title=lensVisualTitle('https://example.org/first','First');
  const multiplePhotos=`<main><div class="N54PNb">${lensVisualPhoto(source).repeat(2)}${title}</div></main>`;
  expect(extract(multiplePhotos,{verified:true}).candidates[0]?.thumbnailSource).toBeUndefined();
  const multipleTitles=`<main><div class="N54PNb">${lensVisualPhoto(source)}${title}${lensVisualTitle('https://example.org/second','Second')}</div></main>`;
  expect(extract(multipleTitles,{verified:true}).candidates.map(item=>item.thumbnailSource)).toEqual([undefined,undefined]);
});
it('rejects hidden, tiny, query, AI and incorrectly nested photos for visual cards',()=>{
  const source='data:image/jpeg;base64,/9j/2Q==',title=lensVisualTitle('https://example.org/first','First');
  for(const photo of [lensVisualPhoto(source).replace('<img ','<img hidden '),lensVisualPhoto(source).replace('data-height="161"','data-height="14"'),`<img data-width="500" data-height="300" src="${source}">`,lensVisualPhoto(source).replace('class="pb66sc"','class="uNDRH"'),`<div class="N54PNb">${lensVisualPhoto(source)}</div>`]){
    expect(extract(`<main><div class="N54PNb">${photo}${title}</div></main>`,{verified:true}).candidates[0]?.thumbnailSource).toBeUndefined();
  }
});
it('does not adopt unlinked, hidden or outside-main semantic headings as visual matches',()=>{
  const html='<main><div role="heading">AI overview</div><a href="https://example.org/hidden"><div hidden role="heading">Hidden match</div></a><a href="https://example.org/citation">Citation without a heading</a></main><a href="https://example.org/outside"><div role="heading">Outside main</div></a>';
  expect(extract(html,{verified:true}).candidates).toEqual([]);
});
it('does not promote generic topic headings or ambiguous visual-card titles to results',()=>{
  expect(extract('<main><a href="https://example.org/topic"><div role="heading">Browse related topic</div></a></main>',{verified:true}).candidates).toEqual([]);
  const title='<div class="N54PNb"><div class="kb0PBd"><div class="lTYpse"><div class="HePszc" role="heading">';
  const html=`<main><a class="LBcIee" href="https://example.org/ambiguous">${title}First title</div></div></div></div>${title}Different title</div></div></div></div></a></main>`;
  expect(extract(html,{verified:true}).candidates).toEqual([]);
  expect(extract(html.replace(`${title}Different title</div></div></div></div>`,'<h3>Different title</h3>'),{verified:true}).candidates).toEqual([]);
});
it('keeps neutral image diagnostics bounded without recording thumbnail pixels or query tokens',()=>{
  const html='<main>'+Array.from({length:40},(_,i)=>`<img src="https://encrypted-tbn0.gstatic.com/images?q=private-query-${i}">`).join('')+'<img src="data:image/jpeg;base64,/9j/2Q=="></main>';
  const evidence=extract(html,{probe:true,now:()=>0}).probe as unknown as {images:unknown[]};
  expect(evidence.images).toHaveLength(32);expect(JSON.stringify(evidence)).not.toContain('private-query-');
  const data=extract('<main><img src="data:image/jpeg;base64,/9j/2Q=="></main>',{probe:true}).probe;
  expect(JSON.stringify(data)).toContain('image/jpeg');expect(JSON.stringify(data)).not.toContain('/9j/2Q==');
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
const lowSauceNotice='<div class="result" id="result-hidden-notification">Low similarity results have been hidden. Click here to display them...</div>';
it('recognizes only the rendered source-hidden low-similarity disposition without reading or expanding hidden cards',()=>{
  const html=saucePage(lowSauceNotice+sauceCard({attrs:'style="display:none"',title:'Hidden private candidate'}));
  const result=extract(html,sauceOptions);expect(result).toEqual({nonce:'fixture',state:'lowSimilarityHidden',candidates:[]});
  expect(JSON.stringify(result)).not.toContain('Hidden private');expect(extract(html,{...sauceOptions,verified:false}).state).toBe('unknown');
});
it.each(['outside-layout','duplicate','hidden','wrong-text','invalid-visible-card','incomplete'])('does not turn an unproved %s low-similarity notice into a completed search',reason=>{
  let notice=lowSauceNotice,cards='';
  if(reason==='duplicate')notice+=lowSauceNotice;
  if(reason==='hidden')notice=notice.replace('class="result"','class="result" hidden');
  if(reason==='wrong-text')notice=notice.replace('Low similarity results have been hidden.','Unrecognized result message.');
  if(reason==='invalid-visible-card')cards=sauceCard({score:'invalid'});
  const html=reason==='outside-layout'?notice:saucePage(notice+cards);
  expect(extract(html,{...sauceOptions,ready:reason==='incomplete'?'loading':'complete'}).state).not.toBe('lowSimilarityHidden');
});
it.each([['Our systems have detected unusual traffic','challenge'],['Daily search limit exceeded','quota']])('keeps %s ahead of a low-only source notice', (notice,state)=>{
  expect(extract(saucePage(lowSauceNotice)+`<p>${notice}</p>`,sauceOptions)).toMatchObject({state,candidates:[]});
});
// SYNTHETIC: the reported Creator/Source/Material field names are reproduced on
// the previously observed card hierarchy. This is not captured DOM/live proof.
const metadataSource='<strong>Source:</strong> <a class="linkify" href="https://example.org/source">example.org</a>';
const metadataCreator='<strong>Creator:</strong> Fixture Artist';
function metadataCard(columns:string,title?:string) {
  const card=sauceCard({title:title??'',score:'91.25%',columns});
  return title===undefined?card.replace('<div class="resulttitle"></div>',''):card;
}
// Shape captured in the neutral NASA URL's actual 2026-10-24 response.
// Destinations are neutral fixtures; no source pages are loaded by these tests.
const sourceIcons='<div class="resultmiscinfo"><a href="https://example.org/post/1"><img src="images/static/siteicons/danbooru.ico" alt=""></a><br><a href="https://example.net/post/2"><img src="images/static/siteicons/gelbooru.ico" alt=""></a></div>';
const iconCard=(icons=sourceIcons)=>metadataCard('<strong>Source: </strong>Wikipedia<br>').replace('<div class="resultmatchinfo">','<div class="resultmatchinfo">'+icons);
it('retains exact provider thumbnail ownership and visible metadata without promoting material to title',()=>{
  const source='https://img1.saucenao.com/res/pixiv/fixture.jpg?auth=neutral-fixture';
  const card=metadataCard(`${metadataCreator}<br>${metadataSource}<br><strong>Material:</strong> original`).replace('Thumbnail repeat',`<img src="${source}">`);
  const candidate=extract(saucePage(card),sauceOptions).candidates[0];
  expect(candidate).toMatchObject({title:'',creator:'Fixture Artist',similarity:91.25,thumbnailSource:source,sourceMetadata:[
    {label:'Creator',value:'Fixture Artist'},{label:'Source',value:'example.org'},{label:'Material',value:'original'},
  ]});
});
it('retains the actual ArtStation CDN path shown in the original result window',()=>{
  // Root's actual native result window showed img3 /artstation/numeric/numeric_numeric.jpg.
  // Neutral IDs/tokens stand in for that structure; no signed live query is stored.
  const source='https://img3.saucenao.com/artstation/533/12345_67890.jpg?auth=neutral-fixture&exp=2000000000';
  const result=extract(saucePage(sauceCard().replace('Thumbnail repeat',`<img src="${source}">`)),sauceOptions);
  expect(result.candidates[0]?.thumbnailSource).toBe(source);
  for(const path of ['/artstation/a/123_456.jpg','/artstation/533/123_456.svg','/artstation/533/123_456/other.jpg','/artstation/533/123_456_789.jpg']){
    const card=sauceCard().replace('Thumbnail repeat',`<img src="https://img3.saucenao.com${path}">`);
    expect(extract(saucePage(card),sauceOptions).candidates[0]?.thumbnailSource).toBeUndefined();
  }
});
it.each(['hidden','style="display:none"','aria-hidden="true"'])('omits hidden thumbnail and metadata fields: %s',attr=>{
  const card=sauceCard({columns:`${metadataSource}<br><span ${attr}>Material: hidden</span>`}).replace('Thumbnail repeat',`<img ${attr} src="https://img1.saucenao.com/res/pixiv/fixture.jpg">`);
  const result=extract(saucePage(card),sauceOptions);
  expect(result.candidates[0]?.thumbnailSource).toBeUndefined();
  expect(JSON.stringify(result)).not.toContain('hidden');
});
it.each(['https://evil.test/a.jpg','http://img1.saucenao.com/res/a.jpg','https://img1.saucenao.com/profile/a.jpg','https://user@img1.saucenao.com/res/a.jpg','https://img1.saucenao.com:444/res/a.jpg','data:image/svg+xml;base64,AAAA'])('rejects unowned thumbnail destinations: %s',src=>{
  expect(extract(saucePage(sauceCard().replace('Thumbnail repeat',`<img src="${src}">`)),sauceOptions).candidates[0]?.thumbnailSource).toBeUndefined();
});
it('rejects ambiguous or unrelated card images without dropping an otherwise valid candidate',()=>{
  const image='<img src="https://img1.saucenao.com/res/pixiv/fixture.jpg">';
  const ambiguous=sauceCard().replace('Thumbnail repeat',image+image);
  const candidate=extract(saucePage(ambiguous),sauceOptions).candidates[0];
  expect(candidate?.title).toBeTruthy();expect(candidate?.thumbnailSource).toBeUndefined();
  expect(extract(saucePage(sauceCard({extra:image})),sauceOptions).candidates[0]?.thumbnailSource).toBeUndefined();
});
it('keeps a Lens thumbnail only in the same validated heading link and never guesses creator',()=>{
  const source='https://encrypted-tbn0.gstatic.com/images?q=neutral-fixture';
  const result=extract(`<main><img src="${source}"><a href="https://example.org/art"><img src="${source}"><h3>Geometry</h3></a></main>`,{verified:true});
  expect(result.candidates[0]).toMatchObject({title:'Geometry',creator:null,similarity:null,thumbnailSource:source});
  expect(extract(`<main><img src="${source}"><a href="https://example.org/art"><h3>Geometry</h3></a></main>`,{verified:true}).candidates[0]?.thumbnailSource).toBeUndefined();
});
it('bounds already-rendered Lens data thumbnails and excludes SVG or oversized payloads',()=>{
  const html=(src:string)=>`<main><a href="https://example.org/art"><img src="${src}"><h3>Geometry</h3></a></main>`;
  expect(extract(html('data:image/jpeg;base64,/9j/2Q=='),{verified:true}).candidates[0]?.thumbnailSource).toBe('data:image/jpeg;base64,/9j/2Q==');
  for(const src of ['data:image/svg+xml;base64,AAAA','data:image/jpeg;base64,'+'A'.repeat(128*1024)])expect(extract(html(src),{verified:true}).candidates[0]?.thumbnailSource).toBeUndefined();
});
it('retains actual titleless source-icon links without inventing artist or title',()=>{
  const result=extract(saucePage(iconCard()),sauceOptions);
  expect(result).toMatchObject({state:'complete',candidates:[{title:'',creator:null,similarity:91.25,links:[
    {label:'example.org',url:'https://example.org/post/1'},
    {label:'example.net',url:'https://example.net/post/2'},
  ]}]});
});
it.each([
  sourceIcons.replace('class="resultmiscinfo"','class="unrecognized"'),
  sourceIcons.replace('class="resultmiscinfo"','class="resultmiscinfo" hidden'),
  sourceIcons.replaceAll('<img','<img hidden'),
  sourceIcons.replaceAll('images/static/siteicons/','unrecognized/'),
  sourceIcons.replaceAll('href="https://','href="javascript:'),
  sourceIcons.replaceAll('https://example.org/post/1','https://saucenao.com/search.php').replaceAll('https://example.net/post/2','https://saucenao.com/info.php'),
  sourceIcons+sourceIcons,
])('does not promote ambiguous or hidden misc links into a titleless result',icons=>{
  expect(extract(saucePage(iconCard(icons)),sauceOptions).candidates).toEqual([]);
});
it('keeps titleless source icons within their own validated score/card',()=>{
  expect(extract(saucePage(iconCard().replace('class="resultsimilarityinfo"','class="unknown"')),sauceOptions).candidates).toEqual([]);
  const outside=metadataCard('<strong>Source:</strong> Wikipedia<br>')+sourceIcons;
  expect(extract(saucePage(outside),sauceOptions).candidates).toEqual([]);
  expect(extract(saucePage(iconCard()),{...sauceOptions,verified:false}).candidates).toEqual([]);
});
it.each([
  ['plain',metadataCreator],
  ['linked','<strong>Creator:</strong> <a class="linkify" href="https://example.org/artist">Fixture Artist</a>'],
])('reads an explicit %s Creator without deriving a work title',(variant,creator)=>{
  const columns=`${creator}<br>${metadataSource}<br><strong>Material:</strong> original`;
  for(const title of [undefined,'','Neutral geometry']){
    const result=extract(saucePage(metadataCard(columns,title)),sauceOptions);
    expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(1);
    expect(result.candidates[0]).toMatchObject({title:title??'',creator:'Fixture Artist',similarity:91.25});
    expect(result.candidates[0]?.links).toContainEqual({label:'example.org',url:'https://example.org/source'});
    if(!title)expect(result.candidates[0]?.links[0]).toEqual({label:'example.org',url:'https://example.org/source'});
    if(variant==='linked')expect(result.candidates[0]?.links).toContainEqual({label:'Fixture Artist',url:'https://example.org/artist'});
  }
});
it.each([
  ['source alone',metadataSource],
  ['material instead of creator',`${metadataSource}<br><strong>Material:</strong> original`],
  ['unlabelled link',`${metadataCreator}<br><a class="linkify" href="https://example.org/source">example.org</a>`],
  ['hidden creator',`<span hidden>${metadataCreator}</span><br>${metadataSource}`],
  ['empty creator',`<strong>Creator:</strong><br>${metadataSource}`],
  ['oversized creator',`<strong>Creator:</strong> ${'a'.repeat(257)}<br>${metadataSource}`],
  ['duplicate creator',`${metadataCreator}<br>${metadataCreator}<br>${metadataSource}`],
  ['duplicate creator on one line',`${metadataCreator} ${metadataCreator}<br>${metadataSource}`],
  ['conflicting creator',`${metadataCreator}<br><strong>Creator:</strong> Other Artist<br>${metadataSource}`],
  ['conflicting legacy author',`${metadataCreator}<br>Author: <a class="linkify" href="https://example.org/other">Other Artist</a><br>${metadataSource}`],
  ['duplicate source',`${metadataCreator}<br>${metadataSource}<br>${metadataSource}`],
  ['two source links',`${metadataCreator}<br>${metadataSource} <a class="linkify" href="https://example.org/other">Other</a>`],
  ['unsafe source',`${metadataCreator}<br><strong>Source:</strong> <a class="linkify" href="javascript:alert(1)">example.org</a>`],
  ['provider action',`${metadataCreator}<br><strong>Source:</strong> <a class="linkify" href="https://saucenao.com/info.php">Info</a>`],
  ['creator link with extra text',`<strong>Creator:</strong> Fixture <a class="linkify" href="https://example.org/artist">Artist</a><br>${metadataSource}`],
  ['unsafe creator link',`<strong>Creator:</strong> <a class="linkify" href="javascript:alert(1)">Fixture Artist</a><br>${metadataSource}`],
  ['HTML newlines are not field separators',`${metadataCreator}\n${metadataSource}`],
  ['hidden BR is not a field separator',`${metadataCreator}<br hidden>${metadataSource}`],
])('does not infer titleless candidates from insufficient or ambiguous metadata: %s',(_reason,columns)=>{
  expect(extract(saucePage(metadataCard(columns)),sauceOptions).candidates).toEqual([]);
});
it('retains only explicit metadata links in a titleless card',()=>{
  const columns=`<a class="linkify" href="https://example.org/unrelated">Unlabelled link</a><br>${metadataCreator}<br>${metadataSource}`;
  const result=extract(saucePage(metadataCard(columns)),sauceOptions);
  expect(result.state).toBe('complete');
  expect(result.candidates[0]?.links).toEqual([{label:'example.org',url:'https://example.org/source'}]);
});
it('recognizes a normal zero-width BR while keeping metadata projection bounded',()=>{
  const card=metadataCard(`${metadataCreator}<br data-zero-rect>${metadataSource}`);
  expect(extract(saucePage(card),sauceOptions).state).toBe('complete');
  const excessive=metadataCard(`${metadataCreator}${'<br>'.repeat(33)}${metadataSource}`);
  expect(extract(saucePage(excessive),sauceOptions).candidates).toEqual([]);
});
it('does not guess a titled result creator from duplicated or conflicting explicit fields',()=>{
  for(const other of ['Fixture Artist','Other Artist']){
    const result=extract(saucePage(metadataCard(`${metadataCreator}<br><strong>Creator:</strong> ${other}<br>${metadataSource}`,'Neutral geometry')),sauceOptions);
    expect(result.state).toBe('complete');expect(result.candidates[0]?.creator).toBeNull();
  }
});
it('keeps titleless metadata cards isolated and does not replace missing required structure',()=>{
  const creatorOnly=metadataCard(metadataCreator),sourceOnly=metadataCard(metadataSource);
  expect(extract(saucePage(creatorOnly+sourceOnly),sauceOptions).candidates).toEqual([]);
  const complete=metadataCard(`${metadataCreator}<br>${metadataSource}`);
  expect(extract(saucePage(complete.replace('class="resultsimilarityinfo"','class="unsupported"')),sauceOptions).candidates).toEqual([]);
  expect(extract(saucePage(complete),{...sauceOptions,verified:false}).candidates).toEqual([]);
});
it('keeps metadata source text separate from actual service notices',()=>{
  const card=metadataCard(`<strong>Creator:</strong> Daily search limit exceeded<br>${metadataSource}`);
  expect(extract(saucePage(card),sauceOptions).state).toBe('complete');
  const result=extract('<p role="alert">Daily search limit exceeded</p>'+saucePage(card),sauceOptions);
  expect(result.state).toBe('quota');expect(result.candidates).toEqual([]);
});
const servicePhrases=[
  ['Our systems detected unusual traffic','challenge'],
  ['Verify you are human','challenge'],
  ['Checking your browser','challenge'],
  ['Before you continue to Google','consent'],
  ['Daily search limit exceeded','quota'],
] as const;
it.each(servicePhrases)('does not mistake a verified Lens result title or linked description for a service notice: %s',(phrase)=>{
  for(const content of [`<h3>${phrase}</h3>`,`<h3>Source article</h3><p>${phrase}</p>`]){
    const result=extract(`<main><a href="https://example.org/source">${content}</a></main>`,{verified:true,now:()=>0});
    expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(1);
  }
});
it.each(servicePhrases)('does not mistake a verified SauceNAO result title or metadata for a service notice: %s',(phrase)=>{
  const result=extract(saucePage(sauceCard({title:phrase,columns:`Source: <a class="linkify" href="https://example.org/source">Artwork</a><span>${phrase}</span>`})),sauceOptions);
  expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(1);
});
it.each(servicePhrases)('still recognizes a service notice outside otherwise valid results: %s',(phrase,state)=>{
  const result=extract(`<p role="alert">${phrase}</p><main><a href="https://example.org/source"><h3>Source article</h3></a></main>`,{verified:true,now:()=>0});
  expect(result.state).toBe(state);expect(result.candidates).toEqual([]);
  const sauce=extract(`<p role="alert">${phrase}</p>`+saucePage(sauceCard()),sauceOptions);
  expect(sauce.state).toBe(state);expect(sauce.candidates).toEqual([]);
});
it.each([
  ['<title>Just a moment...</title>','https://www.google.com/search'],
  ['','https://www.google.com/sorry/index'],
  ['<iframe src="https://www.google.com/recaptcha/api2/anchor"></iframe>','https://www.google.com/search'],
  ['<div id="cf-chl-widget">Check</div>','https://www.google.com/search'],
])('never lets result markup override an actual challenge signal: %s',(signal,url)=>{
  const result=extract(signal+'<main><a href="https://example.org/source"><h3>Source article</h3></a></main>',{verified:true,url,now:()=>0});
  expect(result.state).toBe('challenge');expect(result.candidates).toEqual([]);
});
it('keeps unverified and incomplete result-shaped documents fail-closed',()=>{
  const html='<main><a href="https://example.org/source"><h3>Verify you are human</h3></a></main>';
  expect(extract(html,{now:()=>0}).state).toBe('challenge');
  const result=extract(html+'<div>'+('<span></span>'.repeat(7000))+'</div>',{verified:true,now:()=>0});
  expect(result.state).toBe('challenge');expect(result.candidates).toEqual([]);
});
it('keeps bounded duplicate and later result text separate from service notices',()=>{
  const links=Array.from({length:14},(_,i)=>`<a href="https://example.org/${i===13?0:i}"><h3>${i>=10?'Daily search limit exceeded':`Source ${i}`}</h3></a>`).join('');
  const result=extract(`<main>${links}</main>`,{verified:true,now:()=>0});
  expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(10);
});
it('recognizes result-content ranges beyond128headings without increasing output caps or hiding later notices',()=>{
  const links=Array.from({length:200},(_,i)=>`<a href="https://example.org/${i}"><h3>${i>=128?'Daily search limit exceeded':`Source ${i}`}</h3></a>`).join('');
  const html=`<main>${links}</main>`;
  const result=extract(html,{verified:true,probe:true,now:()=>0});
  expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(10);
  expect(result.probe?.outline.length).toBeLessThanOrEqual(250);expect(result.probe?.limited).toBe(false);
  const notice=extract(html+'<p role="alert">Verify you are human</p>',{verified:true,now:()=>0});
  expect(notice.state).toBe('challenge');expect(notice.candidates).toEqual([]);
});
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
