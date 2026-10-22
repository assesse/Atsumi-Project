import { expect,it,vi } from "vitest";
import script from "../../../src-tauri/src/x_projection.js?raw";
const owner='<a data-testid="AppTabBar_Profile_Link" href="/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
const tabs='<div role="tablist"><a role="tab" aria-selected="true" href="/home">For you</a><a role="tab" href="/home">Following</a><a role="tab" href="/i/communities/123456789">작업 모임</a><a role="tab" href="/i/lists/998877">보통 목록</a></div>';
const post=(id="987654321",text="풍경")=>'<article data-testid="tweet"><div data-testid="User-Name"><a href="/creator">Creator</a><a role="link" href="/creator/status/'+id+'"><time datetime="2026-10-18T00:00:00Z"></time></a></div><div data-testid="tweetText">'+text+'</div><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Test?format=jpg&amp;name=orig"></div><button data-testid="reply">3</button><button data-testid="unlike">10</button></article>';
function run(html:string,req:Record<string,unknown>={},path="/home"){
  const doc=new DOMParser().parseFromString(owner+'<main data-testid="primaryColumn">'+html+'</main>',"text/html");
  const win={top:null as unknown,scrollBy:vi.fn()};win.top=win;
  return new Function("window","document","location","URL","return ("+script.replace("__ATSUMI_X_REQUEST__",JSON.stringify({nonce:"test",feed:"foryou",...req}))+");")(win,doc,{protocol:"https:",hostname:"x.com",origin:"https://x.com",pathname:path},URL);
}
it("projects app-owned data with image and actual account community names",()=>{
  expect(run(tabs+post())).toMatchObject({state:"ready",communities:[{id:"123456789",name:"작업 모임",path:"/i/communities/123456789"}],posts:[{id:"987654321",media:["https://pbs.twimg.com/media/Test?format=jpg&name=medium"],stats:{likes:"10",liked:true,canLike:true,canReply:true}}]});
});
it('opens only the native-bound rendered author profile route',()=>{
  expect(run(post(),{feed:'profile:creator',owner:'alice',target:'/creator',destination:'/creator'},'/creator').state).toBe('ready');
  expect(run(post(),{feed:'profile:creator',owner:'alice',target:'/creator',destination:'/creator'},'/other').state).not.toBe('ready');
  expect(run(tabs+post(),{feed:'profile:creator',owner:'alice'},'/home').state).toBe('unavailable');
});
it('distinguishes explicit replies and related headings, retaining uncertainty',()=>{
  const path='/creator/status/987654321';
  const html=post()+post('112233445','대화').replace('</article>','<div data-testid="replyingTo">Replying to @creator</div></article>')+post('112233446','불명')+'<h2>Discover more</h2>'+post('112233447','관련');
  expect(run(html,{feed:'post:987654321',owner:'alice',target:path,destination:path},path).posts.map((p:{relation:string})=>p.relation)).toEqual([null,'reply','unknown','related']);
});
it.each([
  '<div data-testid="replyingTo">Replying to @creator</div>',
  '<div>Replying to @creator</div>',
])('does not inherit a quoted reply marker as the enclosing post relationship: %s',marker=>{
  const path='/creator/status/987654321';
  const quote=post('223344556','인용된 답글').replace('<article data-testid="tweet">','<div role="link">').replace('</article>',marker+'</div>');
  const outer=post('112233445','답글이 아닌 인용 게시물').replace('</article>',quote+'</article>');
  const result=run(post()+outer,{feed:'post:987654321',owner:'alice',target:path,destination:path},path);
  expect(result.posts[1]).toMatchObject({id:'112233445',relation:'unknown',quote:{id:'223344556',relation:null}});
});
it('does not use ordinary authored text as a reply marker',()=>{
  const path='/creator/status/987654321';
  const result=run(post()+post('112233445','Replying to @creator is the text of this standalone post'),{feed:'post:987654321',owner:'alice',target:path,destination:path},path);
  expect(result.posts[1].relation).toBe('unknown');
});
it('retains a genuine own reply marker when a nested quote also exists',()=>{
  const path='/creator/status/987654321';
  const quote=post('223344556','인용 본문').replace('<article data-testid="tweet">','<div role="link">').replace('</article>','</div>');
  const reply=post('112233445','본인 답글').replace('</article>',quote+'<div data-testid="replyingTo">Replying to @creator</div></article>');
  expect(run(post()+reply,{feed:'post:987654321',owner:'alice',target:path,destination:path},path).posts[1].relation).toBe('reply');
});
it('does not let a heading inside a post or quote become a related-post section boundary',()=>{
  const path='/creator/status/987654321';
  const quote=post('223344556','인용 본문').replace('<article data-testid="tweet">','<div role="link">').replace('</article>','<h2>Discover more</h2></div>');
  const article=post('112233445','인용 게시물').replace('</article>',quote+'<h2>관련 게시물</h2></article>');
  const html=post()+article+post('112233446','분류할 근거가 없음')+'<h2>Discover more</h2>'+post('112233447','실제 관련 영역');
  expect(run(html,{feed:'post:987654321',owner:'alice',target:path,destination:path},path).posts.map((p:{relation:string})=>p.relation)).toEqual([null,'unknown','unknown','related']);
});
it("uses the accessible count instead of concatenated animated digits",()=>{
  const html=post().replace('<button data-testid="unlike">10</button>','<button data-testid="unlike" aria-label="8 Likes. Liked"><span>7</span><span>8</span></button>');
  expect(run(tabs+html).posts[0].stats.likes).toBe("8");
  expect(run(tabs+html.replace('8 Likes. Liked','1.2만 마음에 들어요')).posts[0].stats.likes).toBe("1.2만");
});
it("handles observed history routes with selected collection tabs",()=>{
  for(const [feed,path,label] of [["likes","/i/history/likes","마음에 들어요"],["bookmarks","/i/history/bookmarks","북마크"]]){
    const html='<h2>역사</h2><div role="tablist"><a role="tab" href="'+path+'" aria-selected="true">'+label+'</a></div>'+post();
    expect(run(html,{feed,owner:"alice",target:path},path).state).toBe("ready");
    expect(run(html,{feed,owner:"other",target:path},path).state).toBe("ownerChanged");
  }
});
it("does not treat enclosing role=link as a quote or lose its media",()=>{
  const html=post().replace('<div data-testid="User-Name">','<div role="link"><div data-testid="User-Name">').replace('</article>','</div></article>');
  expect(run(tabs+html).posts[0]).toMatchObject({id:"987654321",text:"풍경",media:[expect.stringContaining("/media/Test")]});
});
it("preserves repost context and a distinct quote without replacing identity",()=>{
  const quote=post("123451234","인용 풍경").replace('<article data-testid="tweet">','<div role="link">').replace('</article>','</div>');
  const html=post().replace('</article>','<span data-testid="socialContext">친구님이 재게시했습니다</span>'+quote+'</article>');
  expect(run(tabs+html).posts[0]).toMatchObject({id:"987654321",repost:"친구님이 재게시했습니다",quote:{id:"123451234",text:"인용 풍경"}});
});
it.each(['메인에 올림','Pinned','固定','Alice liked','친구님이 마음에 들어 합니다'])('does not misrepresent %s context as a repost',label=>{
  const result=run(tabs+post().replace('</article>','<span data-testid="socialContext">'+label+'</span></article>')).posts[0];
  expect(result.repost).toBeNull();expect(result).toMatchObject({id:'987654321',text:'풍경',handle:'creator'});
});
it.each(['메인에 올림','Pinned','고정','固定'])('preserves %s as a separate pinned fact',label=>{
  expect(run(tabs+post().replace('</article>','<span data-testid="socialContext">'+label+'</span></article>')).posts[0]).toMatchObject({pinned:true,repost:null});
});
it.each(['친구님이 재게시했습니다','친구님이 재게시함','친구님이 재게시','Alice reposted','You retweeted','作家さんがリポスト'])('preserves explicit own %s context as a repost',label=>{
  expect(run(tabs+post().replace('</article>','<span data-testid="socialContext">'+label+'</span></article>')).posts[0].repost).toBe(label);
});
it('does not inherit a nested quote repost context for the enclosing post',()=>{
  const quote=post('123451234','인용 본문').replace('<article data-testid="tweet">','<div role="link">').replace('</article>','<span data-testid="socialContext">Alice reposted</span></div>');
  expect(run(tabs+post().replace('</article>',quote+'</article>')).posts[0].repost).toBeNull();
});
it('does not inherit a nested quote pin or assert pin state for other social context',()=>{
  const quote=post('123451234','인용 본문').replace('<article data-testid="tweet">','<div role="link">').replace('</article>','<span data-testid="socialContext">Pinned</span></div>');
  expect(run(tabs+post().replace('</article>',quote+'</article>')).posts[0]).toMatchObject({pinned:false,repost:null});
  expect(run(tabs+post().replace('</article>','<span data-testid="socialContext">Alice liked</span></article>')).posts[0].pinned).toBe(false);
  expect(run(tabs+post().replace('</article>','<span data-testid="socialContext">Alice reposted</span></article>')).posts[0].pinned).toBe(false);
});
it("reads the main detail timestamp outside its user heading and includes replies",()=>{
  const main=post().replace('<a role="link" href="/creator/status/987654321"><time datetime="2026-10-18T00:00:00Z"></time></a>','').replace('</article>','<a href="/creator/status/987654321"><time datetime="2026-10-18T00:00:00Z"></time></a></article>');
  const path="/creator/status/987654321";
  expect(run(main+post("112233445","답글"),{feed:"post:987654321",destination:path,target:path,owner:"alice"},path).posts.map((p:{id:string})=>p.id)).toEqual(["987654321","112233445"]);
});
it("does not mislabel recommended communities, errors, or loading as an empty joined list",()=>{
  expect(run('<h2>Communities</h2>'+post(),{feed:"communities",owner:"alice",target:"/i/communities"},"/i/communities").state).toBe("unsupported");
  expect(run(tabs).state).toBe("pending");
});
it("does not read credentials, clone site markup/styles, or use private APIs",()=>{
  expect(script).not.toMatch(/document\.cookie|localStorage|sessionStorage|fetch\(|XMLHttpRequest|__INITIAL_STATE__|graphql|innerHTML|outerHTML|style\.cssText/);
});
it("binds observed linkless account tabs by exact label without inventing community IDs",()=>{
  const html='<div role="tablist"><div role="tab" aria-selected="true">추천</div><div role="tab">팔로잉</div><div role="tab">작업 모임</div></div>'+post();
  expect(run(html).communities).toEqual([{id:"tab-2",name:"작업 모임",path:"/home"}]);
  expect(run(html,{feed:"community:tab-2",owner:"alice",destination:"/home",binding:{id:"tab-2",name:"다른 모임",path:"/home"}}).state).toBe("unsupported");
  expect(run(html.replace('role="tab">작업','role="tab" aria-selected="true">작업'),{feed:"community:tab-2",owner:"alice",destination:"/home",binding:{id:"tab-2",name:"작업 모임",path:"/home"}}).state).toBe("ready");
});
it("keeps account-tab identity stable when a text emoji hydrates to an image",()=>{
  const tab=(label:string)=>'<div role="tablist"><div role="tab">추천</div><div role="tab">팔로잉</div><div role="tab" aria-selected="true">'+label+'</div></div>'+post();
  const binding={id:"tab-2",name:"작업 🔞 모임",path:"/home"};
  for(const label of ['작업 🔞 모임','작업 <img alt="🔞" src="https://abs.twimg.com/emoji.svg"> 모임']){
    const result=run(tab(label),{feed:"community:tab-2",owner:"alice",destination:"/home",binding});
    expect(result.state).toBe("ready");expect(result.communities).toEqual([binding]);
  }
  expect(run(tab('작업 <img alt="다른" src="https://abs.twimg.com/emoji.svg"> 모임'),{feed:"community:tab-2",owner:"alice",destination:"/home",binding}).state).toBe("unsupported");
});
it("never substitutes the main post ID for a timestamp-less reply",()=>{
  const reply=post("112233445","답글").replace('<a role="link" href="/creator/status/112233445"><time datetime="2026-10-18T00:00:00Z"></time></a>','');
  const path="/creator/status/987654321";
  expect(run(post()+reply,{feed:"post:987654321",owner:"alice",target:path,destination:path},path).posts).toHaveLength(1);
});
const detailPath='/creator/status/987654321';
function plainDetail(author='creator',permalink=detailPath,extra=''){
  return '<article data-testid="tweet"><div data-testid="User-Name"><a href="/'+author+'">Author</a></div>'+(permalink?'<a href="'+permalink+'"><span>Date</span></a>':'')+'<div data-testid="tweetText">Observed article</div>'+extra+'</article>';
}
function projectDetail(html:string){
  return run(html,{feed:'post:987654321',owner:'alice',target:detailPath,destination:detailPath},detailPath);
}
it('supports a plain-text main detail date with an exact own permalink and matching author',()=>{
  expect(projectDetail(plainDetail('CREATOR')).posts).toMatchObject([{id:'987654321',handle:'creator',text:'Observed article'}]);
});
it.each([
  ['virtualized main replaced by a timestamp-less reply',()=>plainDetail('creator','/creator/status/112233445')],
  ['missing permalink',()=>plainDetail('creator','')],
  ['wrong own author',()=>plainDetail('another')],
  ['external permalink',()=>plainDetail('creator','https://evil.test'+detailPath)],
  ['different origin alias',()=>plainDetail('creator','https://www.x.com'+detailPath)],
  ['credentialed permalink',()=>plainDetail('creator','https://user@x.com'+detailPath)],
  ['query permalink',()=>plainDetail('creator',detailPath+'?s=20')],
  ['quoted plain-date permalink',()=>plainDetail('creator','','<div role="link"><div data-testid="User-Name"><a href="/creator">Quoted</a></div><a href="'+detailPath+'"><span>Date</span></a></div>')],
  ['nested article permalink',()=>plainDetail('creator','',plainDetail())],
] as const)('does not invent the detail identity from %s',(_name,html)=>{
  expect(projectDetail(html()).posts).toEqual([]);
});
it('does not assign a later plain-date article the main identity',()=>{
  expect(projectDetail(post('112233445')+plainDetail()).posts.map((p:{id:string})=>p.id)).toEqual(['112233445']);
});
it('retains a timestamped first reply under its own identity after the main article virtualizes away',()=>{
  expect(projectDetail(post('112233445','Still a reply')).posts).toMatchObject([{id:'112233445',text:'Still a reply',relation:'unknown'}]);
});
it('keeps linkless quoted content without inventing a mutable post identity',()=>{
  const quote='<div role="link"><div data-testid="User-Name"><span>인용 작가</span><span>@quoted</span><time datetime="2026-10-18T00:00:00Z"></time></div><div data-testid="tweetText">인용 본문</div><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/Quote?format=jpg"></div></div>';
  const result=run(tabs+post().replace('</article>',quote+'</article>')).posts[0];
  expect(result.quote).toBeNull();expect(result.quotePreview).toMatchObject({handle:'quoted',name:'인용 작가',text:'인용 본문',media:[expect.stringContaining('/media/Quote')]});
  expect(result.media).toEqual(['https://pbs.twimg.com/media/Test?format=jpg&name=medium']);
  expect(result.quotePreview).not.toHaveProperty('id');
});
it('reveals ordinary per-post content only, never age/login gates or social controls',()=>{
  const click=vi.spyOn(HTMLElement.prototype,'click').mockImplementation(()=>{});
  try{
    const warning=(text:string)=>'<div data-testid="sensitiveMediaWarning">'+text+'<button>Show</button></div>';
    const html=post().replace('</article>',warning('Sensitive content')+warning('Sensitive content verify age 18')+warning('Sensitive content login')+'</article>');
    expect(run(tabs+html).state).toBe('pending');expect(click).toHaveBeenCalledTimes(1);
  }finally{click.mockRestore();}
});
