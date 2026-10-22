import { afterEach, expect, it, vi } from "vitest";
import script from "../../../src-tauri/src/x_feed.js?raw";

const owner='<a data-testid="AppTabBar_Profile_Link" href="/my_account"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
const tabs='<div role="tablist"><button role="tab" aria-selected="true">For you</button><button role="tab" aria-selected="false">Following</button></div>';
const article=(id='123456789',text='산 풍경',extra='')=>`<article data-testid="tweet"><div data-testid="User-Name"><a href="https://x.com/creator">작가</a><a href="https://x.com/creator/status/${id}"><time datetime="2026-10-18T01:00:00Z">오늘</time></a></div><div data-testid="tweetText">${text}</div><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Fixture?format=jpg&name=orig"></div>${extra}</article>`;
function run(html:string,request:Record<string,unknown>={},path='/home',hostname='x.com',framed=false){
  const doc=new DOMParser().parseFromString(`<div data-testid="primaryColumn">${html}</div>`,'text/html');
  const win={top:null as unknown,scrollBy:vi.fn()};win.top=framed?{}:win;
  return {doc,win,result:new Function('window','document','location','URL',`return (\n${script.replace('__ATSUMI_X_REQUEST__',JSON.stringify({nonce:'fixture',feed:'foryou',...request}))}\n);`)(win,doc,{hostname,pathname:path,protocol:'https:'},URL)};
}
afterEach(()=>vi.restoreAllMocks());
it('reads only rendered posts and validates identity, media and quote boundaries',()=>{
  const {result}=run(owner+tabs+article('123456789','산 풍경','<div role="link"><div data-testid="tweetText">인용문</div><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Quote"></div></div>'));
  expect(result).toMatchObject({state:'ready',owner:'my_account',posts:[{id:'123456789',handle:'creator',name:'작가',text:'산 풍경',media:['https://pbs.twimg.com/media/Fixture?format=jpg&name=medium']}]});
  expect(JSON.stringify(result)).not.toContain('인용문');
});
it('does not infer ownership from posts or recommended profiles',()=>{
  expect(run(tabs+article()).result.state).toBe('pending');
  expect(run(owner+tabs+article(),{},'/i/flow/login').result.state).toBe('signedOut');
  expect(run(owner+tabs+article(),{},'/home','x.com.evil.test').result.state).toBe('unavailable');
  expect(run(owner+tabs+article(),{},'/home','x.com',true).result.state).toBe('unavailable');
});
it('allows semantic link anchors but still excludes linked quote cards',()=>{
  const html=article('123456789','own text','<div role="link"><div data-testid="tweetText">quote text</div></div>').replaceAll('<a href=','<a role="link" href=');
  expect(run(owner+tabs+html).result.posts).toMatchObject([{id:'123456789',text:'own text'}]);
});
it('selects only a recognized official read-only feed tab',()=>{
  const {doc,result}=run(owner+tabs,{feed:'following'});expect(result.state).toBe('pending');
  expect(doc.querySelectorAll('[role=tab]')).toHaveLength(2);
  expect(run(owner+tabs.replace('For you','Unknown'),{}).result.state).toBe('unsupported');
  expect(run(owner+tabs,{feed:'following',advance:true}).result.state).toBe('unsupported');
});
it('rejects foreign image hosts, wrong post IDs and quoted-only identities',()=>{
  const {result}=run(owner+tabs+article().replace('https://pbs.twimg.com/','https://pbs.twimg.com.evil.test/'));
  expect(result.posts[0].media).toEqual([]);
  expect(run(owner+tabs+article('bad')).result.state).toBe('pending');
  expect(run(owner+tabs+'<article data-testid="tweet"><div role="link">'+article()+'</div></article>').result.posts).toEqual([]);
});
it('bounds text/media and deduplicates visible IDs without claiming end',()=>{
  const {result}=run(owner+tabs+article('123456789','a'.repeat(6500))+article());
  expect(result.posts).toHaveLength(1);expect(result.posts[0].text).toHaveLength(6000);expect(result.posts[0].truncated).toBe(true);
  expect(run(owner+tabs).result.state).toBe('pending');expect(result.exhausted).toBeUndefined();
});
it('never reads credentials or runs arbitrary requests',()=>{expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql/);});
it('accepts an exact own-likes tab link without depending on its translated wording',()=>{
  const request={feed:'likes',owner:'my_account',target:'/my_account/likes'};
  const liked='<div role="tablist"><a role="tab" href="/my_account/likes" aria-selected="true">いいね</a></div>';
  expect(run(owner+liked+article(),request,'/my_account/likes').result.state).toBe('ready');
  expect(run(owner+liked.replace('/my_account/likes','https://evil.test/my_account/likes')+article(),request,'/my_account/likes').result.state).toBe('unsupported');
});
it('reads only safe top-level post avatars, never quote avatars or arbitrary image hosts',()=>{
  const avatar='<div data-testid="UserAvatar-Container-creator"><img src="https://pbs.twimg.com/profile_images/123456789/avatar_normal.jpg"></div>';
  expect(run(owner+tabs+article('123456789','text',avatar)).result.posts[0].avatar).toBe('https://pbs.twimg.com/profile_images/123456789/avatar_normal.jpg');
  expect(run(owner+tabs+article('123456789','text','<div role="link">'+avatar+'</div>')).result.posts[0].avatar).toBeNull();
  expect(run(owner+tabs+article('123456789','text',avatar.replace('pbs.twimg.com','evil.test'))).result.posts[0].avatar).toBeNull();
});
it('accepts the documented no-tab member timeline only with official sidebar and tagged posts',()=>{
  const request={feed:'communities',owner:'my_account',target:'/i/communities'};
  const sidebar='<nav><a href="/i/communities">Communities</a></nav>';
  const tagged=article('123456789','text','<a href="/i/communities/123456789">내 모임</a>');
  expect(run(owner+sidebar+'<h2>Communities</h2>'+tagged,request,'/i/communities').result.state).toBe('ready');
  expect(run(owner+sidebar+'<h2>Communities</h2>'+tabs+tagged,request,'/i/communities').result.state).toBe('unsupported');
  expect(run(owner+'<h2>Communities</h2>'+tagged,request,'/i/communities').result.state).toBe('unsupported');
});
it('bootstraps only closed collection routes from the authenticated home owner',()=>{
  expect(run(owner+tabs,{feed:'likes'}).result).toMatchObject({state:'navigate',owner:'my_account',path:'/my_account/likes'});
  expect(run(owner+tabs,{feed:'bookmarks'}).result).toMatchObject({state:'navigate',path:'/i/bookmarks'});
  expect(run(owner+tabs,{feed:'communities'}).result).toMatchObject({state:'navigate',path:'/i/communities'});
  expect(run(owner+tabs,{feed:'likes',owner:'other'}).result.state).toBe('ownerChanged');
  expect(run(owner+tabs,{feed:'messages'}).result.state).toBe('unavailable');
});
it('requires exact authenticated own-likes route and selected Likes tab',()=>{
  const html=owner+'<div role="tablist"><button role="tab" aria-selected="true">Likes</button></div>'+article();
  const request={feed:'likes',owner:'my_account',target:'/my_account/likes'};
  expect(run(html,request,'/my_account/likes').result).toMatchObject({state:'ready',path:'/my_account/likes',posts:[{id:'123456789'}]});
  expect(run(html,{feed:'likes'},'/my_account/likes').result.state).toBe('unavailable');
  expect(run(html,{...request,target:'/another/likes'},'/another/likes').result.state).not.toBe('ready');
  expect(run(html.replace('Likes','Posts'),request,'/my_account/likes').result.state).toBe('unsupported');
});
it('reads bookmarks only under a proven root bookmark heading',()=>{
  const request={feed:'bookmarks',owner:'my_account',target:'/i/bookmarks'};
  expect(run(owner+'<h2>All Bookmarks</h2>'+article(),request,'/i/bookmarks').result.state).toBe('ready');
  expect(run(owner+'<h2>Explore</h2>'+article(),request,'/i/bookmarks').result.state).toBe('unsupported');
  expect(run(owner+'<h2>All Bookmarks</h2>'+article(),{...request,owner:'other'},'/i/bookmarks').result.state).toBe('ownerChanged');
});
it('never represents a community discovery feed as the joined-community timeline',()=>{
  const request={feed:'communities',owner:'my_account',target:'/i/communities'};
  expect(run(owner+'<h2>Communities</h2>'+tabs+article(),request,'/i/communities').result.state).toBe('unsupported');
  const joined='<div role="tablist"><button role="tab" aria-selected="true">Your communities</button></div>';
  expect(run(owner+joined+article(),request,'/i/communities').result.state).toBe('ready');
  expect(run(owner+'<h2>내 커뮤니티</h2>'+article(),request,'/i/communities').result.state).toBe('ready');
});
it('distinguishes explicit empty collections from loading, errors and unrecognized layouts',()=>{
  const request={feed:'bookmarks',owner:'my_account',target:'/i/bookmarks'};
  expect(run(owner+'<h2>Bookmarks</h2><div data-testid="emptyState">Save posts for later</div>',request,'/i/bookmarks').result).toMatchObject({state:'empty',posts:[]});
  expect(run(owner+'<h2>Bookmarks</h2>',request,'/i/bookmarks').result.state).toBe('pending');
  expect(run(owner+'<h2>Bookmarks</h2><div data-testid="emptyState">Something went wrong</div>',request,'/i/bookmarks').result.state).toBe('pending');
});
