import { afterEach, expect, it, vi } from 'vitest';
import script from '../../../src-tauri/src/x_media_search_extract.js?raw';

type Projection = {state:string; lens?:{mode:string;exactUrl:string|null;visualUrl:string|null;allUrl:string|null};candidates:Array<{title:string;thumbnailSource?:string;creator:null;similarity:null;links:Array<{url:string}>}>};
const pixels='data:image/jpeg;base64,/9j/2Q==';
const nav=(exact=true)=>`<nav><a ${exact?'href="https://www.google.com/search?fixture=all"':'aria-current="page"'}>전체</a><a ${exact?'role="link" aria-disabled="true"':'href="https://www.google.com/search?fixture=exact"'}>정확히 일치하는 결과</a><a href="https://www.google.com/search?fixture=visual">시각적으로 일치하는 항목</a></nav>`;
const card=(index:number,extra='')=>`<a href="https://www.google.com/goto?url=neutral-${index}"><img data-width="96" data-height="128" src="${pixels}"><span>Neutral source ${index}</span><span>1일 전 · 675x1,200</span><img data-width="14" data-height="14" src="${pixels}"><span>X</span>${extra}</a>`;
function read(html:string,ready='complete'):Projection{
  const doc=new DOMParser().parseFromString(html,'text/html');
  Object.defineProperty(doc,'readyState',{value:ready});
  vi.spyOn(Element.prototype,'getBoundingClientRect').mockImplementation(function(this:Element){
    const width=Number(this.getAttribute('data-width')||600),height=Number(this.getAttribute('data-height')||30),top=this.hasAttribute('data-offscreen')?2000:0;
    return{width,height,top,left:0,bottom:top+height,right:width,x:0,y:top,toJSON(){}};
  });
  const config={nonce:'fixture',provider:'lens',lensVerified:true,preferExact:true};
  return new Function('document','location','Element','getComputedStyle','URL','performance',`return (${script.replace('__ATSUMI_MEDIA_SEARCH__',JSON.stringify(config))})`)(doc,new URL('https://www.google.com/search?fixture=1'),Element,(node:HTMLElement)=>({display:node.style.display||'block',visibility:node.style.visibility||'visible',opacity:node.style.opacity||'1'}),URL,{now:()=>0});
}
afterEach(()=>vi.restoreAllMocks());
it('reads all seven heading-free exact cards in source order including existing below-fold cards',()=>{
  const result=read(nav()+`<main>${Array.from({length:7},(_,i)=>`<div data-offscreen>${card(i)}</div>`).join('')}</main>`);
  expect(result.lens?.mode).toBe('exact');expect(result.state).toBe('complete');expect(result.candidates).toHaveLength(7);
  result.candidates.forEach((item,i)=>{expect(item.title).toContain(`Neutral source ${i}`);expect(item.links[0]?.url).toBe(`https://www.google.com/goto?url=neutral-${i}`);expect(item.thumbnailSource).toBe(pixels);expect(item.creator).toBeNull();expect(item.similarity).toBeNull();});
});
it('does not borrow neighboring photos, promote AI headings or count hidden/duplicate/icon-only links',()=>{
  const result=read(nav()+`<main>${card(1)}${card(1)}<div hidden>${card(2)}</div>${card(3,`<img data-width="96" data-height="128" src="${pixels}">`)}<a href="https://example.org/icon"><img data-width="14" data-height="14" src="${pixels}">Icon 675x1,200</a><a href="https://example.org/ai"><h3>AI citation</h3></a><div><img data-width="96" data-height="128" src="${pixels}"><a href="https://example.org/neighbor">Unowned 675x1,200</a></div></main>`);
  expect(result.candidates).toHaveLength(1);expect(result.candidates[0]?.title).toContain('Neutral source 1');
});
it('reports observed navigation without inventing an exact URL and refuses ambiguous active modes',()=>{
  const result=read(nav(false)+'<main>Waiting for selection</main>');
  expect(result.lens).toMatchObject({mode:'all',exactUrl:'https://www.google.com/search?fixture=exact',visualUrl:'https://www.google.com/search?fixture=visual'});
  const ambiguous=read(nav()+'<nav><a aria-current="page">전체</a></nav><main>No exact matches found</main>');
  expect(ambiguous.lens?.mode).toBe('unknown');expect(ambiguous.state).toBe('unknown');
});
it.each(['div','span'])('accepts a selected accessible %s link without assuming an anchor tag',tag=>{
  const navigation=nav().replace('<a role="link" aria-disabled="true">정확히 일치하는 결과</a>',`<${tag} role="link" aria-disabled="true"><span>정확히 일치하는 결과</span></${tag}>`);
  const result=read(navigation+`<main>${card(1)}</main>`);
  expect(result.lens?.mode).toBe('exact');expect(result.candidates).toHaveLength(1);
});
it.each(['No exact matches found','일치하는 검색결과가 없습니다.','일치검색 결과를 찾을 수 없음'])('requires an explicit empty notice in the selected exact mode: %s',message=>{
  expect(read(nav()+`<main><p>${message}</p></main>`).state).toBe('empty');
  expect(read(nav()+`<main><a href="https://example.org/unparsed">Unrecognized source result</a><p>${message}</p></main>`).state).toBe('unknown');
});
it('reads the actually observed exact-zero center_col notice without admitting unknown result layouts',()=>{
  const notice='<span>일치검색 결과를 찾을 수 없음</span>';
  expect(read(nav()+`<div id="center_col"><div>${notice}</div></div>`).state).toBe('empty');
  expect(read(nav()+`<div id="center_col"><a href="https://example.org/unknown">Unparsed source</a>${notice}</div>`).state).toBe('unknown');
  expect(read(nav()+`<div id="center_col">${notice}</div><div id="center_col">${notice}</div>`).state).toBe('unknown');
  expect(read(nav()+`<div id="center_col" hidden>${notice}</div>`).state).toBe('unknown');
  expect(read(nav()+`<div id="center_col">${card(1)}</div>`).state).toBe('unknown');
});
it.each([
  'https://www.google.com/url?q=https%3A%2F%2Fexample.org%2Fsource',
  'https://www.google.com/url?url=https%3A%2F%2Fexample.org%2Fsource',
  'https://www.google.com/url?opaque=retained-result',
  'https://www.google.com/goto?opaque=retained-result',
  'https://www.google.com/goto?url=one&url=two',
])('does not declare exact zero while an unparsed retained redirect remains: %s',url=>{
  const unknown=`<a href="${url}">Unrecognized source result</a><p>일치검색 결과를 찾을 수 없음</p>`;
  for(const body of [`<main>${unknown}</main>`,`<div id="center_col">${unknown}</div>`]){
    const result=read(nav()+body);
    expect(result.lens?.mode).toBe('exact');expect(result.candidates).toEqual([]);expect(result.state).toBe('unknown');
  }
});
it('keeps a genuine scoped zero independent of ordinary Google controls and hidden or outside-scope redirects',()=>{
  const wrapper='<a href="https://www.google.com/url?q=https%3A%2F%2Fexample.org%2Fsource">Unparsed source</a>';
  const controls='<a href="https://www.google.com/search?q=neutral">Search</a><a href="https://www.google.com/preferences?hl=ko">Settings</a>';
  for(const body of [
    `<main>${controls}<div hidden>${wrapper}</div><p>일치검색 결과를 찾을 수 없음</p></main><footer>${wrapper}</footer>`,
    `<div id="center_col">${controls}<div hidden>${wrapper}</div><p>일치검색 결과를 찾을 수 없음</p></div><footer>${wrapper}</footer>`,
  ])expect(read(nav()+body).state).toBe('empty');
  const populated=read(nav()+`<main>${card(1)}${wrapper}<p>일치검색 결과를 찾을 수 없음</p></main>`);
  expect(populated.state).toBe('complete');expect(populated.candidates).toHaveLength(1);
});
it('does not borrow zero-result notices outside the actual results scope',()=>{
  for(const notice of ['No exact matches found','일치검색 결과를 찾을 수 없음']){
    expect(read(nav()+`<main>Unknown layout</main><footer>${notice}</footer>`).state).toBe('unknown');
    expect(read(nav()+`<div id="center_col">Unknown layout</div><footer>${notice}</footer>`).state).toBe('unknown');
  }
});
it('does not turn loading, unknown layouts, security challenges or a missing exact tab into empty',()=>{
  expect(read(nav()+'<main>Unknown result layout</main>').state).toBe('unknown');
  expect(read(nav()+'<main>No exact matches found</main>','loading').state).toBe('pending');
  expect(read(nav()+'<main>Our systems have detected unusual traffic</main>').state).toBe('challenge');
  expect(read('<main>No exact matches found</main>').state).toBe('unknown');
});
