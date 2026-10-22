import {expect,it,vi} from "vitest";
import script from "../../../src-tauri/src/x_video_intent.js?raw";
const path="/artist/status/123456789";
function run(label:string,owner="alice",locationPath=path){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn"><article data-testid="tweet"><a href="'+path+'"><time></time></a><div data-testid="videoPlayer"><button aria-label="'+label+'"></button></div></article></main>',"text/html");
  const click=vi.fn();doc.querySelector('button[aria-label]')!.addEventListener('click',click);
  doc.querySelector('article')!.scrollIntoView=vi.fn();
  const win={top:null as unknown};win.top=win;
  new Function('window','document','location',script.replace('__ATSUMI_X_VIDEO__',JSON.stringify({path,owner})))(win,doc,{protocol:'https:',hostname:'x.com',pathname:locationPath});
  return click;
}
it("recognizes the actual localized player control without touching social buttons",()=>{
  expect(run("플레이하기")).toHaveBeenCalledTimes(1);expect(run("동영상 재생")).toHaveBeenCalledTimes(1);expect(run("재게시")).not.toHaveBeenCalled();
});
it("does not start playback for another account or page",()=>{
  expect(run("플레이하기","bob")).not.toHaveBeenCalled();expect(run("플레이하기","alice","/home")).not.toHaveBeenCalled();
});
function hydrationFixture(){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn"><article data-testid="tweet"><a href="'+path+'"><time></time></a><div data-testid="videoPlayer"></div></article></main>',"text/html");
  doc.querySelector('article')!.scrollIntoView=vi.fn();
  const win={top:null as unknown};win.top=win;
  const inspect=(intentSent=false)=>new Function('window','document','location','return ('+script.replace('__ATSUMI_X_VIDEO__',JSON.stringify({nonce:'n',path,owner:'alice',inspect:intentSent}))+');')(win,doc,{protocol:'https:',hostname:'x.com',pathname:path});
  return {doc,player:doc.querySelector('[data-testid="videoPlayer"]')!,inspect};
}
it('keeps waiting for hydration until one actual play intent is delivered',()=>{
  const {doc,player,inspect}=hydrationFixture();
  let sent=false;
  const pending=inspect(sent);sent ||= pending.intentSent===true;
  expect(pending.state).toBe('observed');expect(sent).toBe(false);
  const button=doc.createElement('button');button.setAttribute('aria-label','Play');
  const click=vi.fn();button.addEventListener('click',click);player.append(button);
  const delivered=inspect(sent);sent ||= delivered.intentSent===true;
  expect(sent).toBe(true);expect(click).toHaveBeenCalledTimes(1);
  expect(inspect(sent).intentSent).toBe(false);expect(click).toHaveBeenCalledTimes(1);
});
it('does not count a disabled player control as a delivered intent',()=>{
  const {doc,player,inspect}=hydrationFixture();
  const button=doc.createElement('button');button.setAttribute('aria-label','재생');button.disabled=true;
  const click=vi.fn();button.addEventListener('click',click);player.append(button);
  expect(inspect().intentSent).toBe(false);expect(click).not.toHaveBeenCalled();
  button.disabled=false;expect(inspect().intentSent).toBe(true);expect(click).toHaveBeenCalledTimes(1);
});
it('delivers play only once when a video element appears after the first inspection',()=>{
  const {doc,player,inspect}=hydrationFixture();
  expect(inspect().intentSent).toBe(false);
  const video=doc.createElement('video');const play=vi.spyOn(video,'play').mockResolvedValue();player.append(video);
  const delivered=inspect();expect(delivered.intentSent).toBe(true);expect(play).toHaveBeenCalledTimes(1);expect(video.muted).toBe(true);
  inspect(delivered.intentSent);expect(play).toHaveBeenCalledTimes(1);
});
function detailFixture(body:string,documentPath=path){
  const doc=new DOMParser().parseFromString('<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button><main data-testid="primaryColumn">'+body+'</main>',"text/html");
  const click=vi.fn();
  doc.querySelectorAll('article').forEach(article=>{article.scrollIntoView=vi.fn();});
  doc.querySelectorAll('[data-testid="videoPlayer"] button').forEach(button=>button.addEventListener('click',click));
  const win={top:null as unknown};win.top=win;
  const result=new Function('window','document','location','return ('+script.replace('__ATSUMI_X_VIDEO__',JSON.stringify({nonce:'n',path,documentPath,owner:'alice'}))+');')(win,doc,{protocol:'https:',hostname:'x.com',pathname:documentPath});
  return {result,click};
}
function detailArticle(author='artist',permalink=path,extra=''){
  return '<article data-testid="tweet"><div data-testid="User-Name"><a href="/'+author+'">Author</a></div>'+(permalink?'<a href="'+permalink+'"><span>Date</span></a>':'')+extra+'<div data-testid="videoPlayer"><button aria-label="Play"></button></div></article>';
}
it('accepts a plain-text detail date only with the exact own permalink and author',()=>{
  const {result,click}=detailFixture(detailArticle('ARTIST'));
  expect(result.state).toBe('observed');expect(result.intentSent).toBe(true);expect(click).toHaveBeenCalledTimes(1);
});
it.each([
  ['missing permalink',()=>detailArticle('artist','')],
  ['wrong own author',()=>detailArticle('another')],
  ['same-path external permalink',()=>detailArticle('artist','https://evil.test'+path)],
  ['later plain-date article',()=>detailArticle('another','/another/status/987654321')+detailArticle()],
  ['quoted plain-date permalink',()=>detailArticle('artist','','<div role="link"><div data-testid="User-Name"><a href="/artist">Quoted</a></div><a href="'+path+'"><span>Date</span></a></div>')],
] as const)('rejects a detail fallback with %s',(_name,body)=>{
  const {result,click}=detailFixture(body());
  expect(result.state).toBe('postMissing');expect(click).not.toHaveBeenCalled();
});
it('does not use the plain-date detail fallback on a different rendered path',()=>{
  const {result,click}=detailFixture(detailArticle(),'/home');
  expect(result.state).toBe('postMissing');expect(click).not.toHaveBeenCalled();
});
