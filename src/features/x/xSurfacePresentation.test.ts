import {afterEach,expect,it,vi} from 'vitest';
import script from '../../../src-tauri/src/x_surface_presentation.js?raw';
const article=(id:string,name='작가',body='First\nSecond')=>'<article data-testid="tweet"><a href="/artist"><img src="https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"></a><div data-testid="User-Name"><a href="/artist"><span dir="auto">'+name+'</span></a><a href="/artist/status/'+id+'"><time>date</time></a></div><div data-testid="tweetText">'+body+'</div></article>';
function setup(html=article('123456'),path='/artist/status/123456'){
 const doc=new DOMParser().parseFromString(html,'text/html'),post=vi.fn(),win={top:null as unknown,chrome:{webview:{postMessage:post}},__atsumiXPresentation:undefined as undefined|{update:(ready:boolean)=>void}};win.top=win;
 const loc={protocol:'https:',hostname:'x.com',origin:'https://x.com',pathname:path};
 new Function('window','document','location',script.replace('__ATSUMI_X_PRESENTATION__','{"feed":"preview","nonce":"test"}'))(win,doc,loc);
 return{doc,post,loc,update:(ready=true)=>win.__atsumiXPresentation!.update(ready),value:()=>JSON.parse(new URL(post.mock.calls.at(-1)![0].split('ATSUMI_X_SURFACE_V1:')[1]).searchParams.get('value')!)};
}
afterEach(()=>vi.restoreAllMocks());
it('takes the exact root author/portrait and first line, not a quote or reply',()=>{
 const a=setup(article('123456').replace('</article>','<div data-testid="quoteTweet">'+article('654321','Quoted')+'</div></article>')+article('444444','Reply'));
 a.update();expect(a.value()).toEqual({path:'/artist/status/123456',name:'작가',firstLine:'First',avatar:'https://pbs.twimg.com/profile_images/123/avatar_normal.jpg'});
 a.update();expect(a.post).toHaveBeenCalledOnce();
});
it('does not announce unrelated, duplicate, hidden or unready content',()=>{
 for(const html of [article('555555'),article('123456')+article('123456'),'<div hidden>'+article('123456')+'</div>']){
 const a=setup(html);a.update();expect(a.post).not.toHaveBeenCalled();}
 const a=setup();a.update(false);expect(a.post).not.toHaveBeenCalled();
});
it('never exposes a foreign portrait or invents a larger image URL',()=>{
 const a=setup(article('123456').replace('https://pbs.twimg.com/','https://evil.example/'));a.update();expect(a.value().avatar).toBeNull();
});
it('reads a profile heading separately from its first timeline post',()=>{
 const a=setup('<div data-testid="primaryColumn"><div data-testid="UserName"><span dir="auto">Profile name</span></div><a href="/artist/photo"><img src="https://pbs.twimg.com/profile_images/123/profile.jpg"></a><div data-testid="UserDescription">Biography</div>'+article('555555','Someone else')+'</div>','/artist');
 a.update();expect(a.value().name).toBe('Profile name');expect(a.value().firstLine).toBe('Biography');
});


it('reads an expanded post timestamp below the text rather than requiring it inside the author heading',()=>{
 const html=article('123456').replace('<a href="/artist/status/123456"><time>date</time></a>','').replace('</article>','<a href="/artist/status/123456"><time>date</time></a></article>');
 const a=setup(html);a.update();expect(a.value().name).toBe('작가');expect(a.value().firstLine).toBe('First');
});


it('reads names without dir attributes and keeps emoji but excludes verification icons',()=>{
 const a=setup(article('123456','<svg aria-label="Verified">badge</svg><span>Name</span><img alt="🐽">').replaceAll(' dir="auto"',''));
 a.update();expect(a.value().name).toBe('Name🐽');
});

it('uses the exact own decorative avatar container, not quoted avatars',()=>{
 const a=setup(article('123456').replace('<a href="/artist"><img src="https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"></a>','<div data-testid="UserAvatar-Container-artist" aria-hidden="true"><img src="https://pbs.twimg.com/profile_images/123/avatar_normal.jpg"></div>'));
 a.update();expect(a.value().avatar).toBe('https://pbs.twimg.com/profile_images/123/avatar_normal.jpg');
});

it('recognizes the exact edited-post history timestamp without reading a quote or a different post',()=>{
 const html=article('123456').replace('<a href="/artist/status/123456"><time>date</time></a>','<a href="/artist/status/123456/history">Last edited 7:26 PM</a>');
 const a=setup(html);a.update();expect(a.value().name).toBe('작가');expect(a.value().firstLine).toBe('First');
 for(const bad of [html.replace('/123456/history','/654321/history'),article('555555').replace('</article>','<div data-testid="quoteTweet">'+html+'</div></article>')]){
 const b=setup(bad);b.update();expect(b.post).not.toHaveBeenCalled();}
});
