import {expect,it,vi} from "vitest";
import script from "../../../src-tauri/src/x_action.js?raw";
import projection from "../../../src-tauri/src/x_projection.js?raw";
function prepare(extra="",request:Record<string,unknown>={},locationPath='/creator/status/123456789'){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn"><article data-testid="tweet"><div role="link"><div data-testid="User-Name"><a href="/creator/status/123456789"><time></time></a></div><button data-testid="like">4</button>'+extra+'</div></article></main>',"text/html");
  const click=vi.spyOn(doc.querySelector('[data-testid="like"]') as HTMLElement,"click");
  const win={top:null as unknown};win.top=win;
  const run=()=>new Function("window","document","location","return ("+script.replace("__ATSUMI_X_ACTION__",JSON.stringify({nonce:"n",owner:"alice",path:"/creator/status/123456789",action:"like",phase:"start",expected:false,...request}))+");")(win,doc,{protocol:"https:",hostname:"x.com",pathname:locationPath});
  return {doc,click,run};
}
it("clicks only the requested post action despite a broad role=link wrapper",()=>{const {click,run}=prepare();expect(run().state).toBe("sent");expect(click).toHaveBeenCalledTimes(1);});
it("rejects stale action state and mismatched accounts before any mutation",()=>{
  for(const r of [{owner:"other"},{expected:true},{action:"delete"},{action:"repost"},{action:"reply"}]){const {click,run}=prepare("",r);expect(run().state).not.toBe("sent");expect(click).not.toHaveBeenCalled();}
});
it("has no reply composer or repost submission path even if stale clients request one",()=>{
  expect(script).not.toMatch(/tweetTextarea|tweetButton|retweetConfirm|execCommand|insertText/);
});
it("does not inspect auth secrets or invoke private endpoints",()=>{expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|graphql|innerHTML|outerHTML/);});
it('probes a rendered action without clicking or changing it',()=>{const {click,run}=prepare('',{phase:'probe'});expect(run().state).toBe('ready');expect(click).not.toHaveBeenCalled();});
it('only dispatches once after a separate read-only feed probe',()=>{
  const request={phase:'probe',documentPath:'/home'};
  const {click,run}=prepare('',request,'/home');
  expect(run().state).toBe('ready');expect(click).not.toHaveBeenCalled();
  request.phase='start';expect(run().state).toBe('sent');expect(click).toHaveBeenCalledTimes(1);
});
it('rejects a changed owner between probe and dispatch',()=>{
  const request={phase:'probe',documentPath:'/home'};
  const {doc,click,run}=prepare('',request,'/home');
  expect(run().state).toBe('ready');
  doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');
  request.phase='start';expect(run().state).toBe('rejected');expect(click).not.toHaveBeenCalled();
});
it('rejects a reader document mismatch before any mutation',()=>{const {click,run}=prepare('',{documentPath:'/home'});expect(run().state).toBe('rejected');expect(click).not.toHaveBeenCalled();});
it('does not act on the enclosing post when only its quote matches',()=>{
  const {doc,click,run}=prepare();doc.querySelector('a')!.setAttribute('href','/alice');
  const root=doc.querySelector('article')!;root.querySelector('a[href="/creator/status/123456789"]')!.setAttribute('href','/creator/status/999999999');
  root.insertAdjacentHTML('beforeend','<div role="link"><div data-testid="User-Name"><a href="/creator/status/123456789"><time></time></a></div><button data-testid="like">1</button></div>');
  expect(run().state).toBe('pending');expect(click).not.toHaveBeenCalled();
});

const postPath='/creator/status/123456789';
function detailArticle(author='creator',permalink=postPath,extra='',timed=false){
  return '<article data-testid="tweet"><div data-testid="User-Name"><a href="/'+author+'">Author</a></div>'+(permalink?'<a href="'+permalink+'">'+(timed?'<time></time>':'<span>Date</span>')+'</a>':'')+'<button data-testid="like">4</button>'+extra+'</article>';
}
function detailFixture(body:string,documentPath=postPath){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn">'+body+'</main>','text/html');
  const click=vi.fn();doc.querySelectorAll('button[data-testid="like"],button[data-testid="unlike"],button[data-testid="bookmark"],button[data-testid="removeBookmark"]').forEach(button=>button.addEventListener('click',click));
  const win={top:null as unknown,innerHeight:900};win.top=win;
  const run=(request:Record<string,unknown>={})=>new Function('window','document','location','return ('+script.replace('__ATSUMI_X_ACTION__',JSON.stringify({nonce:'n',owner:'alice',path:postPath,documentPath,action:'like',phase:'probe',expected:false,...request}))+');')(win,doc,{protocol:'https:',hostname:'x.com',origin:'https://x.com',pathname:documentPath});
  return {doc,run,click};
}
it.each([postPath,'https://x.com'+postPath])('accepts only proved plain-date detail identity for a read-only action probe: %s',(permalink)=>{
  const {run,click}=detailFixture(detailArticle('CREATOR',permalink));
  expect(run().state).toBe('ready');expect(click).not.toHaveBeenCalled();
  expect(run({phase:'start'}).state).toBe('sent');expect(click).toHaveBeenCalledTimes(1);
});
it.each([
  ['missing permalink',()=>detailArticle('creator','')],
  ['wrong own author',()=>detailArticle('another')],
  ['foreign permalink',()=>detailArticle('creator','https://evil.test'+postPath)],
  ['different-origin permalink',()=>detailArticle('creator','https://www.x.com'+postPath)],
  ['credential-bearing permalink',()=>detailArticle('creator','https://user@x.com'+postPath)],
  ['port-bearing permalink',()=>detailArticle('creator','https://x.com:444'+postPath)],
  ['query-bearing permalink',()=>detailArticle('creator',postPath+'?other=1')],
  ['fragment-bearing permalink',()=>detailArticle('creator',postPath+'#other')],
  ['later plain-date article',()=>detailArticle('another','/another/status/987654321')+detailArticle()],
  ['quoted plain-date permalink',()=>detailArticle('creator','','<div role="link"><div data-testid="User-Name"><a href="/creator">Quoted</a></div><a href="'+postPath+'"><span>Date</span></a><button data-testid="like">1</button></div>')],
  ['quote-only author without an outer author',()=>'<article data-testid="tweet"><div role="link"><div data-testid="User-Name"><a href="/creator">Quoted</a></div><a href="'+postPath+'"><span>Date</span></a></div><button data-testid="like">4</button></article>'],
  ['ambiguous plain-date role-link author',()=>'<article data-testid="tweet"><div role="link"><div data-testid="User-Name"><a href="/creator">Unproved</a></div><a href="'+postPath+'"><span>Date</span></a><button data-testid="like">4</button></div></article>'],
  ['nested timed article',()=>detailArticle('creator','/creator/status/987654321',detailArticle('creator',postPath,'',true))],
] as const)('does not authorize an action through %s',(_name,body)=>{
  const {run,click}=detailFixture(body());
  expect(run().state).toBe('pending');expect(run({phase:'start'}).state).toBe('pending');expect(click).not.toHaveBeenCalled();
});
it('does not use the plain-date detail fallback in a feed document',()=>{
  const {run,click}=detailFixture(detailArticle(),'/home');
  expect(run().state).toBe('pending');expect(click).not.toHaveBeenCalled();
});
it('excludes quote-owned controls even when the quote date is plain text',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'<div role="link"><div data-testid="User-Name"><a href="/quoted">Quoted</a></div><a href="/quoted/status/987654321">Date</a><button data-testid="like">1</button></div>',true));
  doc.querySelector('article > button')!.remove();
  expect(run().state).toBe('unavailable');expect(run({phase:'start'}).state).toBe('unavailable');expect(click).not.toHaveBeenCalled();
});
it.each(['disabled','aria-disabled'])('waits for an enabled own action control: %s',(attribute)=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  const button=doc.querySelector('button[data-testid="like"]')!;
  button.setAttribute(attribute,'true');expect(run().state).toBe('disabled');expect(run({phase:'start'}).state).toBe('disabled');expect(click).not.toHaveBeenCalled();
  button.removeAttribute(attribute);expect(run().state).toBe('ready');expect(click).not.toHaveBeenCalled();
});
it('does not mistake an unhydrated control for an expected-state change',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  doc.querySelector('article > button')!.remove();
  expect(run({expected:true}).state).toBe('unavailable');expect(click).not.toHaveBeenCalled();
});
it('requires the own account controls to hydrate before a read-only ready result',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.remove();
  expect(run().state).toBe('pendingOwner');expect(click).not.toHaveBeenCalled();
});
it('reports an offscreen probe separately without scrolling or clicking',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  const article=doc.querySelector('article')!;article.getBoundingClientRect=()=>({top:901,bottom:1001,height:100}) as DOMRect;
  article.scrollIntoView=vi.fn();expect(run().state).toBe('offscreen');expect(article.scrollIntoView).not.toHaveBeenCalled();expect(click).not.toHaveBeenCalled();
});
it('does not dispatch an unknown phase or a state changed after the probe',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  expect(run().state).toBe('ready');expect(run({phase:'prepare'}).state).toBe('rejected');expect(click).not.toHaveBeenCalled();
  doc.querySelector('[data-testid="like"]')!.setAttribute('data-testid','unlike');
  expect(run({phase:'start'}).state).toBe('stateChanged');expect(click).not.toHaveBeenCalled();
});
it('probes and dispatches bookmark controls using the same exact-post guard',()=>{
  const {doc,run,click}=detailFixture(detailArticle());doc.querySelector('[data-testid="like"]')!.setAttribute('data-testid','bookmark');
  expect(run({action:'bookmark'}).state).toBe('ready');expect(click).not.toHaveBeenCalled();
  expect(run({action:'bookmark',phase:'start'}).state).toBe('sent');expect(click).toHaveBeenCalledTimes(1);
});
it('confirms one dispatched action after the exact article moves outside the feed projection viewport',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  const article=doc.querySelector('article')!,button=doc.querySelector('button[data-testid="like"]')!;
  let top=0;
  article.getBoundingClientRect=()=>({top,bottom:top+100,height:100}) as DOMRect;
  article.scrollIntoView=vi.fn();
  button.addEventListener('click',()=>{button.setAttribute('data-testid','unlike');button.setAttribute('aria-label','5 Likes. Liked');top=-101;});
  expect(run().state).toBe('ready');
  expect(run({phase:'start'}).state).toBe('sent');
  expect(click).toHaveBeenCalledTimes(1);
  const win={top:null as unknown,innerHeight:900,scrollBy:vi.fn()};win.top=win;
  const projected=new Function('window','document','location','return ('+projection.replace('__ATSUMI_X_REQUEST__',JSON.stringify({nonce:'projection',feed:'post:123456789',owner:'alice',target:postPath,destination:postPath}))+');')(win,doc,{protocol:'https:',hostname:'x.com',origin:'https://x.com',pathname:postPath});
  expect(projected.posts).toEqual([]);
  expect(run({phase:'confirm'})).toEqual({nonce:'n',state:'observed',observation:{postId:'123456789',handle:'creator',owner:'alice',path:postPath,active:true,count:'5'}});
  expect(click).toHaveBeenCalledTimes(1);expect(win.scrollBy).not.toHaveBeenCalled();expect(article.scrollIntoView).not.toHaveBeenCalled();
});
it.each([
  ['like','like',false],['like','unlike',true],
  ['bookmark','bookmark',false],['bookmark','removeBookmark',true],
] as const)('observes %s/%s without dispatching even when it is unchanged', (action,testid,active)=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  doc.querySelector('article > button')!.setAttribute('data-testid',testid);
  expect(run({phase:'confirm',action,expected:active})).toMatchObject({nonce:'n',state:'observed',observation:{postId:'123456789',handle:'creator',owner:'alice',path:postPath,active,count:'4'}});
  expect(click).not.toHaveBeenCalled();
});
it('does not confirm a quoted control or confuse missing controls with removal',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><a href="/quoted">Quoted</a></div><a href="/quoted/status/987654321"><time></time></a><button data-testid="unlike">99</button></div>';
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,quote,true));
  doc.querySelector('article > button')!.remove();
  expect(run({phase:'confirm',expected:true}).state).toBe('unavailable');expect(click).not.toHaveBeenCalled();
});
it('does not confirm ambiguous duplicate own controls',()=>{
  const {run,click}=detailFixture(detailArticle('creator',postPath,'<button data-testid="unlike">5</button>',true));
  expect(run({phase:'confirm'}).state).toBe('ambiguous');expect(click).not.toHaveBeenCalled();
});
it('keeps a broad timed wrapper but requires its own author and own control to agree',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  const article=doc.querySelector('article')!,wrapper=doc.createElement('div');wrapper.setAttribute('role','link');
  while(article.firstChild)wrapper.append(article.firstChild);article.append(wrapper);
  expect(run({phase:'confirm'}).state).toBe('observed');expect(click).not.toHaveBeenCalled();
  doc.querySelector('[data-testid="User-Name"] a')!.setAttribute('href','/another');
  expect(run({phase:'confirm'}).state).toBe('identityMissing');expect(click).not.toHaveBeenCalled();
});
it('rejects wrong owner, native document, exact target and disabled controls in the read-only phase',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'',true));
  expect(run({phase:'confirm',owner:'bob'}).state).toBe('rejected');
  expect(run({phase:'confirm',documentPath:'/home'}).state).toBe('rejected');
  expect(run({phase:'confirm',path:'/creator/status/987654321'}).state).toBe('pending');
  doc.querySelector('article > button')!.setAttribute('aria-disabled','true');
  expect(run({phase:'confirm'}).state).toBe('disabled');expect(click).not.toHaveBeenCalled();
});
it('returns only the bounded own requested count and no unrelated state',()=>{
  const {doc,run,click}=detailFixture(detailArticle('creator',postPath,'<button data-testid="removeBookmark">99</button>',true));
  const button=doc.querySelector('article > button')!;
  button.textContent='78';button.setAttribute('aria-label','8 Likes. Liked');
  const observed=run({phase:'confirm'});
  expect(observed.observation).toEqual({postId:'123456789',handle:'creator',owner:'alice',path:postPath,active:false,count:'8'});
  button.removeAttribute('aria-label');button.textContent='9'.repeat(40);
  expect(run({phase:'confirm'}).observation.count).toHaveLength(32);expect(click).not.toHaveBeenCalled();
});
