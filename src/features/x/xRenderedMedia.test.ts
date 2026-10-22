import {expect,it} from 'vitest';
import script from '../../../src-tauri/src/x_rendered_media.js?raw';
const defaultArticle='<article data-testid="tweet"><div data-testid="User-Name"><a href="/artist">Artist</a><a href="/artist/status/123456789"><time></time></a></div><div data-testid="videoPlayer"></div></article>';
function setup(articles=defaultArticle,path='/home'){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn">'+articles+'</main>','text/html');
  const win={top:null as unknown,addEventListener:()=>{}};win.top=win;
  new Function('window','document','location',script)(win,doc,{protocol:'https:',hostname:'x.com',origin:'https://x.com',pathname:path});
  return {doc,facts:(win as unknown as {__atsumiXRenderedMedia:{get:(owner:string,id:string)=>string[]}}).__atsumiXRenderedMedia};
}
it('retains a transient rendered video poster only for its exact owner and post',async()=>{
  const {doc,facts}=setup(),player=doc.querySelector('[data-testid="videoPlayer"]')!;
  const video=doc.createElement('video');video.poster='https://pbs.twimg.com/amplify_video_thumb/987654321/img/Poster.jpg';
  player.append(video);await Promise.resolve();video.remove();await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([video.poster]);
  expect(facts.get('other','123456789')).toEqual([]);expect(facts.get('alice','987654321')).toEqual([]);
  doc.querySelector('[data-testid="AppTabBar_Profile_Link"]')!.setAttribute('href','/bob');await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([]);expect(facts.get('bob','123456789')).toEqual([]);
});
it('does not observe site private state or network responses',()=>{
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql|innerHTML|outerHTML/);
});
it('retains an observed poster when X clears its attribute in the same batch',async()=>{
  const {doc,facts}=setup(),player=doc.querySelector('[data-testid="videoPlayer"]')!;
  const video=doc.createElement('video');player.append(video);await Promise.resolve();
  const url='https://pbs.twimg.com/amplify_video_thumb/987654321/img/Transient.jpg';
  video.setAttribute('poster',url);video.removeAttribute('poster');await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([url]);
});
it('never retains a detached quoted video poster as the enclosing post',async()=>{
  const {doc,facts}=setup(),article=doc.querySelector('article')!,player=doc.querySelector('[data-testid="videoPlayer"]')!;
  const own=doc.createElement('video');own.poster='https://pbs.twimg.com/amplify_video_thumb/111111111/img/Own.jpg';player.append(own);
  const quote=doc.createElement('div');quote.setAttribute('role','link');
  quote.innerHTML='<div data-testid="User-Name"><a href="/quoted/status/999999999"><time></time></a></div><div data-testid="videoPlayer"><video poster="https://pbs.twimg.com/amplify_video_thumb/222222222/img/Quote.jpg"></video></div>';
  article.append(quote);await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([own.poster]);
  quote.querySelector('video')!.remove();await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([own.poster]);
});
it('rejects detached quote posters even when the quote username disappears in the same batch',async()=>{
  const {doc,facts}=setup(),article=doc.querySelector('article')!;
  const quote=doc.createElement('div');quote.setAttribute('role','link');
  quote.innerHTML='<div data-testid="User-Name"><a href="/quoted/status/999999999"><time></time></a></div><div data-testid="videoPlayer"><video poster="https://pbs.twimg.com/amplify_video_thumb/222222222/img/Quote.jpg"></video></div>';
  article.append(quote);await Promise.resolve();
  quote.replaceChildren();await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([]);
});
it('does not admit a quoted poster through attributeOldValue',async()=>{
  const {doc,facts}=setup(),article=doc.querySelector('article')!;
  const quote=doc.createElement('div');quote.setAttribute('role','link');
  quote.innerHTML='<div data-testid="User-Name"><a href="/quoted/status/999999999"><time></time></a></div><div data-testid="videoPlayer"><video></video></div>';
  article.append(quote);await Promise.resolve();
  const video=quote.querySelector('video')!;video.poster='https://pbs.twimg.com/amplify_video_thumb/222222222/img/Quote.jpg';video.removeAttribute('poster');await Promise.resolve();
  expect(facts.get('alice','123456789')).toEqual([]);
});
const detailPath='/artist/status/123456789';
const observedPoster='https://pbs.twimg.com/amplify_video_thumb/987654321/img/Observed.jpg';
function plainDetail(author='artist',permalink=detailPath,extra=''){
  return '<article data-testid="tweet"><div data-testid="User-Name"><a href="/'+author+'">Author</a></div>'+(permalink?'<a href="'+permalink+'"><span>Date</span></a>':'')+extra+'<div data-testid="videoPlayer"><video poster="'+observedPoster+'"></video></div></article>';
}
it('retains a plain-date main poster only with its exact own permalink and author',()=>{
  const {facts}=setup(plainDetail('ARTIST'),detailPath);
  expect(facts.get('alice','123456789')).toEqual([observedPoster]);
});
it.each([
  ['virtualized main replaced by a timestamp-less reply',()=>plainDetail('artist','/artist/status/112233445')],
  ['missing permalink',()=>plainDetail('artist','')],
  ['wrong own author',()=>plainDetail('another')],
  ['external permalink',()=>plainDetail('artist','https://evil.test'+detailPath)],
  ['different origin alias',()=>plainDetail('artist','https://www.x.com'+detailPath)],
  ['credentialed permalink',()=>plainDetail('artist','https://user@x.com'+detailPath)],
  ['query permalink',()=>plainDetail('artist',detailPath+'?s=20')],
  ['unusable reply timestamp',()=>plainDetail('artist','').replace('Author</a>','Author</a><a href="/artist/status/112233445?s=20"><time></time></a>')],
  ['cross-origin reply timestamp',()=>plainDetail('artist','').replace('Author</a>','Author</a><a href="https://evil.test/artist/status/112233445"><time></time></a>')],
  ['quoted plain-date permalink',()=>plainDetail('artist','','<div role="link"><div data-testid="User-Name"><a href="/artist">Quoted</a></div><a href="'+detailPath+'"><span>Date</span></a></div>')],
  ['nested article permalink',()=>plainDetail('artist','',plainDetail())],
  ['later plain-date article',()=>defaultArticle.replaceAll('123456789','112233445')+plainDetail()],
] as const)('does not contaminate main poster scope from %s',(_name,html)=>{
  const {facts}=setup(html(),detailPath);
  expect(facts.get('alice','123456789')).toEqual([]);
});
it('keeps a timestamped first reply poster under its own ID',()=>{
  const reply=defaultArticle.replaceAll('123456789','112233445').replace('<div data-testid="videoPlayer"></div>','<div data-testid="videoPlayer"><video poster="'+observedPoster+'"></video></div>');
  const {facts}=setup(reply,detailPath);
  expect(facts.get('alice','123456789')).toEqual([]);
  expect(facts.get('alice','112233445')).toEqual([observedPoster]);
});
