import { describe, expect, it } from 'vitest';
import script from '../../../src-tauri/src/x_archive_capture.js?raw';
import actionsScript from '../../../src-tauri/src/x_surface_actions.js?raw';

const post = (id = '123', body = 'Body') => `<article data-testid="tweet"><div data-testid="User-Name">Author <a href="https://x.com/author/status/${id}"><time datetime="2026-10-20T01:00:00Z">now</time></a></div><p data-testid="tweetText">${body}</p><div role="group" aria-label="2 replies, 3 likes"></div></article>`;
function capture(html: string, resources: string[] = [], options: { path?: string; origin?: string; photoOwner?: unknown; offscreen?: boolean; diagnostics?: boolean } = {}) {
  const document = new DOMParser().parseFromString(html, 'text/html');
  const pathname = options.path ?? '/author/status/123', origin = options.origin ?? 'https://x.com';
  const location = { href: origin + pathname + '?ref=ordinary', origin, pathname };
  for (const element of document.querySelectorAll('*')) element.getBoundingClientRect = () => {
    const left=element.hasAttribute('data-invalid-rect')?Number.NaN:element.hasAttribute('data-left')?Number(element.getAttribute('data-left')):element.hasAttribute('data-shifted')?30:10, width=element.hasAttribute('data-zero-size')?0:600;
    return { left, right:left+width, top:options.offscreen?2000:10, bottom:options.offscreen?2400:410, width, height:400, x:left, y:10, toJSON:()=>({}) };
  };
  const window = { innerWidth: 1000, innerHeight: 800, __atsumiXSurface: { archivePhotoOwner: () => options.photoOwner } };
  const testScript = options.diagnostics ? script.replace('const DIAGNOSTICS = false;', 'const DIAGNOSTICS = true;') : script;
  return new Function('document', 'location', 'performance', 'Element', 'getComputedStyle', 'window', `return (\n${testScript}\n)`)(document, location, { getEntriesByType: () => resources.map(name => ({ name })) }, Element, (n: Element) => ({ display: (n as HTMLElement).style.display, visibility: (n as HTMLElement).style.visibility, opacity: (n as HTMLElement).style.opacity, backgroundImage:(n as HTMLElement).style.backgroundImage,position:(n as HTMLElement).style.position,overflow:(n as HTMLElement).style.overflow,overflowX:(n as HTMLElement).style.overflowX,overflowY:(n as HTMLElement).style.overflowY,transform:(n as HTMLElement).style.transform,translate:(n as HTMLElement).style.translate,rotate:(n as HTMLElement).style.rotate,scale:(n as HTMLElement).style.scale,perspective:(n as HTMLElement).style.perspective,filter:(n as HTMLElement).style.filter,backdropFilter:(n as HTMLElement).style.backdropFilter,contain:(n as HTMLElement).style.contain,willChange:(n as HTMLElement).style.willChange,containerType:(n as HTMLElement).style.containerType,contentVisibility:(n as HTMLElement).style.contentVisibility,transformStyle:(n as HTMLElement).style.transformStyle }), window);
}
describe('read-only X point-in-time capture', () => {
  const conversation = (content:string) => `<main data-testid="primaryColumn"><section aria-label="Timeline: Conversation">${content}</section></main>`;
  it('freezes the exact detail root and already-loaded conversation replies, not ancestors, quotes or related posts', () => {
    const quote='<div data-testid="quoteTweet" role="link"><div data-testid="User-Name">Quoted <a href="https://x.com/other/status/777"><time>then</time></a></div><p data-testid="tweetText">Quote</p></div>';
    const html=conversation(post('111','Ancestor')+post().replace('</article>',quote+'</article>')+post('456','Loaded reply')+'<h2>Discover more</h2>'+post('789','Related'))+`<aside>${post('999','Background')}</aside>`;
    const rows=capture(html).records;
    expect(rows.map((row:{id:string;presentation:{relation:string;conversationId?:string;order:number}})=>[row.id,row.presentation.relation,row.presentation.conversationId,row.presentation.order])).toEqual([
      ['111','unknown',undefined,0],['123','root','123',1],['456','reply','123',2],['789','unknown',undefined,3],['999','unknown',undefined,4],
    ]);
    expect(rows.find((row:{id:string})=>row.id==='777')).toBeUndefined();
    expect(rows[1].presentation.quote).toMatchObject({id:'777',handle:'other',body:'Quote'});
  });
  it('requires own explicit target metadata when the conversation label is absent', () => {
    const marker='<div data-testid="replyingTo">Replying to <a href="https://x.com/author/status/123">@author</a></div>';
    const reply=post('456').replace('</article>',marker+'</article>');
    const unrelated=post('789').replace('</article>','<div data-testid="replyingTo">Replying to <a href="https://x.com/elsewhere">@elsewhere</a></div></article>');
    const rows=capture(`<main>${post()}${reply}${unrelated}${post('999','Replying to @author')}</main>`).records;
    expect(rows.map((row:{presentation:{relation:string}})=>row.presentation.relation)).toEqual(['root','reply','unknown','unknown']);
    // Identical markers in a separate rendered main section are not this thread.
    expect(capture(`<main>${post()}</main><aside>${reply}</aside>`).records[1].presentation.relation).toBe('unknown');
  });
  it('does not infer the exact thread from an author-only reply marker outside a labelled conversation', () => {
    const sameAuthor=post('456').replace('</article>','<div data-testid="replyingTo">Replying to <a href="https://x.com/author">@author</a></div></article>');
    const otherThread=post('789').replace('</article>','<div data-testid="replyingTo">Replying to <a href="https://x.com/author/status/888">@author</a></div></article>');
    const rows=capture(`<main>${post()}${sameAuthor}${otherThread}</main>`).records;
    expect(rows.map((row:{presentation:{relation:string}})=>row.presentation.relation)).toEqual(['root','unknown','unknown']);
  });
  it('does not use quote or body text and their ancestors as reply proof', () => {
    const falseMarker='<div><p data-testid="tweetText"><span data-testid="replyingTo">Replying to <a href="https://x.com/author/status/123">@author</a></span></p></div>';
    const quoteMarker='<div data-testid="quoteTweet"><div data-testid="User-Name">Quoted</div><div data-testid="replyingTo">Replying to <a href="https://x.com/author/status/123">@author</a></div><p data-testid="tweetText">Quoted body</p></div>';
    const rows=capture(`<main>${post()}${post('456').replace('</article>',falseMarker+'</article>')}${post('789').replace('</article>',quoteMarker+'</article>')}</main>`).records;
    expect(rows.map((row:{presentation:{relation:string}})=>row.presentation.relation)).toEqual(['root','unknown','unknown']);
  });
  it.each(['타임라인: 대화','タイムライン: 会話'])('uses the explicitly labelled current conversation %s without inferring a direct parent', label => {
    const rows=capture(conversation(post()+post('456')).replace('Timeline: Conversation',label)).records;
    expect(rows[1].presentation).toMatchObject({relation:'reply',conversationId:'123',order:1});
    expect(rows[1].presentation).not.toHaveProperty('parentId');
  });
  it('treats any new non-article heading as a conservative boundary, but not the preceding detail heading', () => {
    const rows=capture(conversation('<h1>Post</h1>'+post()+post('456')+'<div role="heading">Unfamiliar section title</div>'+post('789'))).records;
    expect(rows.map((row:{presentation:{relation:string}})=>row.presentation.relation)).toEqual(['root','reply','unknown']);
  });
  it('never classifies feed neighbours or replies without the current root as a conversation', () => {
    const html=conversation(post()+post('456'));
    expect(capture(html,[],{path:'/home'}).records.every((row:{presentation:{relation:string}})=>row.presentation.relation==='unknown')).toBe(true);
    expect(capture(conversation(post('456'))).records[0].presentation).toMatchObject({relation:'unknown'});
    expect(capture(conversation(post('456'))).records[0].presentation).not.toHaveProperty('conversationId');
  });
  it('captures observed author name, badge and individual displayed counts without inventing missing counts', () => {
    const author='<div data-testid="User-Name"><a href="https://x.com/author">Displayed Name</a><a href="https://x.com/author">@author</a><svg data-testid="icon-verified"></svg><a href="https://x.com/author/status/123"><time datetime="2026-10-20T01:00:00Z">now</time></a></div>';
    const html=post().replace(/<div data-testid="User-Name">.*?<\/div>/,author).replace('</article>',
      '<button data-testid="reply" aria-label="4 replies">34</button><button data-testid="unretweet" aria-label="456 reposts">old456</button><button data-testid="unlike" aria-label="3.5천 마음에 들어요">3.43.5</button><button data-testid="bookmark"><span>1.4천</span></button><a href="https://x.com/author/status/123/analytics" aria-label="85.5만 조회수">85.5만</a></article>');
    expect(capture(html).records[0].presentation).toMatchObject({handle:'author',displayName:'Displayed Name',verified:true,counts:{replies:'4',reposts:'456',likes:'3.5천',bookmarks:'1.4천',views:'85.5만'}});
    expect(capture(post()).records[0].presentation).toMatchObject({displayName:'',verified:null,counts:{replies:'',reposts:'',likes:'',bookmarks:'',views:''}});
  });
  it('keeps quote identity and an actually captured quote avatar separately, never a guessed URL', () => {
    const quote='<div data-testid="quoteTweet" role="link"><div data-testid="User-Name"><a href="https://x.com/other">Quoted name</a><svg data-testid="icon-verified"></svg><a href="https://x.com/other/status/999"><time datetime="2026-10-19T01:00:00Z">then</time></a></div><p data-testid="tweetText">Quoted body</p><div data-testid="UserAvatar-Container-other"><img src="https://pbs.twimg.com/profile_images/123/quote.jpg"></div></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.quote).toEqual({id:'999',url:'https://x.com/other/status/999',handle:'other',displayName:'Quoted name',verified:true,body:'Quoted body',timestamp:'2026-10-19T01:00:00Z',timestampLabel:'then',avatarUrl:'https://pbs.twimg.com/profile_images/123/quote.jpg'});
    expect(row.media).toContainEqual({kind:'avatar',url:'https://pbs.twimg.com/profile_images/123/quote.jpg',alt:''});
  });
  it.each(['🍫','👩🏽‍💻','🇰🇷','1️⃣'])('preserves only actually rendered emoji %s in an author display name', emoji => {
    const user=`<div data-testid="User-Name"><a href="https://x.com/author"><span><img alt="${emoji}" src="https://abs.twimg.com/emoji.svg"></span></a><a href="https://x.com/author">@author</a><a href="https://x.com/author/status/123"><time>now</time></a></div>`;
    const row=capture(post().replace(/<div data-testid="User-Name">.*?<\/div>/,user)).records[0];
    expect(row.presentation.displayName).toBe(emoji);
    expect(row.media).toEqual([]);
  });
  it('ignores hidden emoji, arbitrary image alt, descriptions, excessive emoji sequences and hidden badges', () => {
    const user='<div data-testid="User-Name"><a href="https://x.com/author">Name <img alt="portrait"><img hidden alt="🍫"><img style="opacity:0" alt="👩🏽‍💻"><img alt="123🍫"><img alt="🍫 description"><img alt="'+ '🍫'.repeat(17) +'"></a><svg hidden data-testid="icon-verified"></svg><a href="https://x.com/author/status/123"><time>now</time></a></div>';
    expect(capture(post().replace(/<div data-testid="User-Name">.*?<\/div>/,user)).records[0].presentation).toMatchObject({displayName:'Name',verified:null});
  });
  // Synthetic source-like markup: live UIA proved the displayed fields, not
  // this exact DOM. Actual new capture remains the integration acceptance.
  it('preserves a linkless quote display name, handle, badge, avatar and time without inventing canonical authority', () => {
    const quote='<div role="link"><div data-testid="User-Name"><span>asa</span><svg data-testid="icon-verified"></svg><span>@C4SSUAL</span><time datetime="2026-10-19T01:00:00Z">10월 19일</time></div><div data-testid="UserAvatar-Container-C4SSUAL"><img src="https://pbs.twimg.com/profile_images/123/asa.jpg"></div><p data-testid="tweetText">Observed quoted text</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quote.jpg"></div></div>';
    const video='<div data-testid="tweetPhoto"><div data-testid="videoPlayer"><video src="https://video.twimg.com/ext_tw_video/111/pu/vid/observed.mp4" poster="https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg"></video></div></div>';
    const row=capture(post().replace('</article>',`<div>${video}${quote}</div></article>`)).records[0];
    expect(row.presentation).not.toHaveProperty('quote');
    expect(row.presentation.quotePresentation).toEqual({handle:'C4SSUAL',displayName:'asa',verified:true,body:'Observed quoted text',timestamp:'2026-10-19T01:00:00Z',timestampLabel:'10월 19일',avatarUrl:'https://pbs.twimg.com/profile_images/123/asa.jpg',mediaUrls:['https://pbs.twimg.com/media/quote.jpg']});
    expect(row.presentation.quotePresentation).not.toHaveProperty('id');
    expect(row.presentation.quotePresentation).not.toHaveProperty('url');
    expect(row.presentation.mediaUrls).toEqual(['https://video.twimg.com/ext_tw_video/111/pu/vid/observed.mp4','https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg']);
    expect(row.presentation.mediaBeforeQuote).toBe(true);
    expect(row.media).toContainEqual({kind:'avatar',url:'https://pbs.twimg.com/profile_images/123/asa.jpg',alt:''});
    expect(row.quotes[0]).toContain('Observed quoted text');
    expect(capture(post().replace('</article>',quote+'</article>')).records).toHaveLength(1);
  });
  it('does not infer a display-only handle from ambiguous metadata or quote-body mentions', () => {
    const quote='<div data-testid="quotedTweet"><div data-testid="User-Name"><span>Someone</span><span>@alice</span><span>@bob</span><span hidden>@hidden</span></div><p data-testid="tweetText">@mentioned</p></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.quotePresentation).toMatchObject({handle:'',displayName:'Someone',body:'@mentioned'});
    expect(row.presentation.quotePresentation).not.toHaveProperty('timestampLabel');
  });
  it('preserves an emoji-only linkless quote name rather than mistaking the whole user block for its handle', () => {
    const quote='<div role="link"><div data-testid="User-Name"><span><img alt="🍫"></span><span>@quoted</span><span>·</span><time>10월 19일</time></div><p data-testid="tweetText">Quote</p></div>';
    expect(capture(post().replace('</article>',quote+'</article>')).records[0].presentation.quotePresentation).toMatchObject({displayName:'🍫',handle:'quoted',timestampLabel:'10월 19일'});
    const textName=quote.replace('<span><img alt="🍫"></span>','<span>asa</span>');
    expect(capture(post().replace('</article>',textName+'</article>')).records[0].presentation.quotePresentation.displayName).toBe('asa');
    const genuinePunctuationName=quote.replace('<span><img alt="🍫"></span>','<span>·</span>');
    expect(capture(post().replace('</article>',genuinePunctuationName+'</article>')).records[0].presentation.quotePresentation.displayName).toBe('·');
  });
  it('keeps canonical and noncanonical quote presentations mutually exclusive', () => {
    const quote='<div data-testid="quotedTweet"><div data-testid="User-Name"><a href="https://x.com/other">Name<img alt="🍫"></a><a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Quote</p></div>';
    const view=capture(post().replace('</article>',quote+'</article>')).records[0].presentation;
    expect(view.quote).toMatchObject({id:'999',handle:'other',displayName:'Name🍫'});
    expect(view).not.toHaveProperty('quotePresentation');
  });
  it('records opposite media ordering only when all own media is after the quote and omits mixed/absent ordering', () => {
    const quote='<div role="link"><div data-testid="User-Name"><span>Name</span><span>@other</span></div><p data-testid="tweetText">Quote</p></div>';
    const media='<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/own.jpg"></div>';
    expect(capture(post().replace('</article>',quote+media+'</article>')).records[0].presentation.mediaBeforeQuote).toBe(false);
    expect(capture(post().replace('</article>',media+quote+media+'</article>')).records[0].presentation).not.toHaveProperty('mediaBeforeQuote');
    expect(capture(post().replace('</article>',quote+'</article>')).records[0].presentation).not.toHaveProperty('mediaBeforeQuote');
    expect(capture(post().replace('</article>',media+'</article>')).records[0].presentation).not.toHaveProperty('mediaBeforeQuote');
  });
  it('does not preserve hidden quote presentation or use its hidden author/avatar as observed metadata', () => {
    const quote='<div hidden data-testid="quotedTweet"><div data-testid="User-Name"><span>Name</span><span>@other</span></div><p data-testid="tweetText">Quote</p><div data-testid="UserAvatar-Container-other"><img src="https://pbs.twimg.com/profile_images/123/hidden.jpg"></div></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation).not.toHaveProperty('quotePresentation');
    expect(row.media).not.toContainEqual(expect.objectContaining({kind:'avatar'}));
  });
  it.each([true,false])('preserves one visible unwrapped observed quote avatar, canonical=%s', canonical => {
    const quote=`<div role="link"><div data-testid="User-Name"><span>zeya</span><span>@zeya</span>${canonical?'<a href="https://x.com/zeya/status/999"><time>then</time></a>':''}</div><div><img src="https://pbs.twimg.com/profile_images/456/zeya.jpg" alt="zeya"></div><p data-testid="tweetText">Quote <img alt="✨" src="https://abs.twimg.com/emoji.svg"></p></div>`;
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    const view=canonical?row.presentation.quote:row.presentation.quotePresentation;
    expect(view.avatarUrl).toBe('https://pbs.twimg.com/profile_images/456/zeya.jpg');
    expect(view.body).toBe('Quote ✨');
    expect(row.quotes[0]).toContain('Quote ✨');
    expect(row.media).toEqual([{kind:'avatar',url:'https://pbs.twimg.com/profile_images/456/zeya.jpg',alt:'zeya'}]);
    expect(row.presentation).not.toHaveProperty('avatarUrl');
    if (!canonical) expect(view).not.toHaveProperty('url');
  });
  it.each([
    ['two different profiles','<img src="https://pbs.twimg.com/profile_images/123/a.jpg"><img src="https://pbs.twimg.com/profile_images/456/b.jpg">'],
    ['two occurrences of one profile','<img src="https://pbs.twimg.com/profile_images/123/a.jpg"><img src="https://pbs.twimg.com/profile_images/123/a.jpg">'],
    ['foreign host','<img src="https://foreign.test/profile_images/123/a.jpg" alt="✨">'],
    ['host suffix spoof','<img src="https://pbs.twimg.com.evil.test/profile_images/123/a.jpg">'],
    ['hidden profile','<span style="display:none"><img src="https://pbs.twimg.com/profile_images/123/a.jpg"></span>'],
    ['ordinary photo','<img src="https://pbs.twimg.com/media/a.jpg">'],
    ['profile inside tweet body','<p data-testid="tweetText"><img src="https://pbs.twimg.com/profile_images/123/a.jpg"></p>'],
    ['profile inside media','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/profile_images/123/a.jpg"></div>'],
    ['too many image candidates',Array.from({length:65},()=>'<img alt="✨">').join('')+'<img src="https://pbs.twimg.com/profile_images/123/a.jpg">'],
  ])('does not guess a fallback quote avatar from %s', (_name, images) => {
    const quote=`<div role="link"><div data-testid="User-Name"><span>Name</span><span>@other</span></div><p data-testid="tweetText">Quote</p>${images}</div>`;
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.quotePresentation).not.toHaveProperty('avatarUrl');
    expect(row.media.filter((entry:{kind:string})=>entry.kind==='avatar')).toHaveLength(0);
  });
  it('does not borrow the outer post avatar or a nested quote avatar for a linkless quote', () => {
    const avatar='<div data-testid="UserAvatar-Container-author"><img src="https://pbs.twimg.com/profile_images/123/outer.jpg"></div>';
    const nested='<div data-testid="quotedTweet"><div data-testid="User-Name"><span>Nested</span></div><p data-testid="tweetText">Nested body</p><img src="https://pbs.twimg.com/profile_images/456/nested.jpg"></div>';
    const quote=`<div role="link"><div data-testid="User-Name"><span>Name</span></div><p data-testid="tweetText">Quote</p>${nested}</div>`;
    const row=capture(post().replace('</article>',avatar+quote+'</article>')).records[0];
    expect(row.presentation.avatarUrl).toBe('https://pbs.twimg.com/profile_images/123/outer.jpg');
    expect(row.presentation.quotePresentation).not.toHaveProperty('avatarUrl');
  });
  const wrapperQuoteUrl='https://pbs.twimg.com/profile_images/123/wrapped.jpg';
  const wrapperQuote = (known:boolean, painted:boolean, extraAuthor='') => `<div role="link"><div data-testid="User-Name"><span>Quoted</span><span>@quoted</span>${extraAuthor}<a href="https://x.com/quoted/status/999"><time>then</time></a></div>${known?'<div data-testid="UserAvatar-Container-quoted">':''}<div${painted?` style="background-image:url(${wrapperQuoteUrl})"`:''}><img${painted?' style="opacity:0"':''} src="${wrapperQuoteUrl}"></div>${known?'</div>':''}<p data-testid="tweetText">Quote body</p></div>`;
  // Synthetic regression: UIA proves the displayed quote, not these exact
  // wrappers. The active pre-fix script loses the avatar in all twelve cases.
  it.each([
    ['data-testid="quoteTweet"',true,false],['data-testid="quoteTweet"',false,false],
    ['data-testid="quoteTweet"',true,true],['data-testid="quoteTweet"',false,true],
    ['data-testid="quotedTweet"',true,false],['data-testid="quotedTweet"',false,false],
    ['data-testid="quotedTweet"',true,true],['data-testid="quotedTweet"',false,true],
    ['role="link"',true,false],['role="link"',false,false],
    ['role="link"',true,true],['role="link"',false,true],
  ] as const)('keeps one card avatar through redundant wrapper %s known=%s painted=%s', (wrapper,known,painted) => {
    const quote=`<div ${wrapper}>${wrapperQuote(known,painted)}</div>`;
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.quote).toMatchObject({id:'999',handle:'quoted',body:'Quote body',avatarUrl:wrapperQuoteUrl});
    expect(row.media.filter((item:{kind:string})=>item.kind==='avatar')).toEqual([{kind:'avatar',url:wrapperQuoteUrl,alt:''}]);
  });
  it('admits a bounded redundant wrapper chain without changing body, author, media or canonical identity', () => {
    const inner=wrapperQuote(false,false).replace('</p>','</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quote.jpg"></div>');
    const quote=`<div data-testid="quoteTweet"><div role="link"><div role="link">${inner}</div></div></div>`;
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.body).toBe('Body');
    expect(row.presentation.quote).toMatchObject({id:'999',url:'https://x.com/quoted/status/999',handle:'quoted',body:'Quote body',avatarUrl:wrapperQuoteUrl,mediaUrls:['https://pbs.twimg.com/media/quote.jpg']});
    expect(row.presentation.mediaUrls).toEqual([]);
    expect(row.quotes).toHaveLength(1);
  });
  it('preserves a linkless photo-only card avatar through identical wrappers without inventing a quote ID', () => {
    const inner=wrapperQuote(false,false)
      .replace('<a href="https://x.com/quoted/status/999"><time>then</time></a>','<time>then</time>')
      .replace('<p data-testid="tweetText">Quote body</p>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quote.jpg"></div>');
    const row=capture(post().replace('</article>',`<div data-testid="quoteTweet">${inner}</div></article>`)).records[0];
    expect(row.presentation).not.toHaveProperty('quote');
    expect(row.presentation.quotePresentation).toMatchObject({handle:'quoted',body:'',avatarUrl:wrapperQuoteUrl,mediaUrls:['https://pbs.twimg.com/media/quote.jpg']});
    expect(row.presentation.quotePresentation).not.toHaveProperty('id');
    expect(row.presentation.quotePresentation).not.toHaveProperty('url');
  });
  it.each([
    ['extra author','<div data-testid="User-Name"><span>Outer</span><span>@outer</span></div>','', ''],
    ['text-equal separate author','<div data-testid="User-Name"><span>Quoted</span><span>@quoted</span></div>','', ''],
    ['extra body','','<p data-testid="tweetText">Outer body</p>', ''],
    ['text-equal separate body','','<p data-testid="tweetText">Quote body</p>', ''],
    ['extra photo','','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/outer.jpg"></div>', ''],
    ['extra video','','<video src="https://video.twimg.com/ext_tw_video/321/pu/vid/outer.mp4"></video>', ''],
    ['hidden additional author','<div hidden data-testid="User-Name"><span>@outer</span></div>','', ''],
    ['ambiguous handles','','','<span>@other</span>'],
    ['true nested quote','<div data-testid="User-Name"><span>Outer</span><span>@outer</span></div><p data-testid="tweetText">Outer body</p>','', ''],
  ])('does not treat %s as a redundant same-card wrapper', (_case,before,after,extraAuthor) => {
    for(const known of [true,false]) {
      const quote=`<div data-testid="quoteTweet">${before}${wrapperQuote(known,false,extraAuthor)}${after}</div>`;
      const row=capture(post().replace('</article>',quote+'</article>')).records[0];
      expect(row.presentation.quote).not.toHaveProperty('avatarUrl');
      expect(row.media).not.toContainEqual(expect.objectContaining({url:wrapperQuoteUrl}));
    }
  });
  it('rejects conflicting wrapper permalinks and a wrapper without an observed unambiguous author handle', () => {
    const conflicting=`<div role="link" href="https://x.com/other/status/888">${wrapperQuote(false,false)}</div>`;
    const noHandle=`<div data-testid="quoteTweet">${wrapperQuote(false,false).replace('<span>@quoted</span>','')}</div>`;
    for(const quote of [conflicting,noHandle]) {
      const row=capture(post().replace('</article>',quote+'</article>')).records[0];
      expect(row.presentation.quote).not.toHaveProperty('avatarUrl');
    }
  });
  it('does not exempt hidden inner wrappers, separate nested cards or excessive content from avatar scope', () => {
    const hidden=wrapperQuote(false,false).replace('<div role="link">','<div role="link" hidden>');
    const nested=wrapperQuote(false,false).replace('</p>','</p><div data-testid="quotedTweet"><div data-testid="User-Name"><span>@nested</span></div><p data-testid="tweetText">Nested</p></div>');
    const excessive=wrapperQuote(false,false).replace('</p>','</p>'+ '<div data-testid="tweetPhoto"></div>'.repeat(33));
    for(const inner of [hidden,nested,excessive]) {
      const row=capture(post().replace('</article>',`<div data-testid="quoteTweet">${inner}</div></article>`)).records[0];
      const quote=row.presentation.quote || row.presentation.quotePresentation;
      expect(quote).not.toHaveProperty('avatarUrl');
    }
  });
  it('fails closed on excessive wrapper or link candidates', () => {
    const wrapped='<div role="link">'.repeat(33)+wrapperQuote(false,false)+'</div>'.repeat(33);
    const linked=wrapperQuote(false,false).replace('</p>','</p>'+ '<a href="https://x.com/quoted">profile</a>'.repeat(65));
    for(const inner of [wrapped,linked]) {
      const row=capture(post().replace('</article>',`<div data-testid="quoteTweet">${inner}</div></article>`)).records[0];
      expect(row.presentation.quote).not.toHaveProperty('avatarUrl');
    }
  });
  const quotePaintUrl='https://pbs.twimg.com/profile_images/456/zeya.jpg';
  const paintedQuote = (image:string, known:boolean, canonical=false) => `<div role="link"><div data-testid="User-Name"><span>zeya</span><span>@zeya</span>${canonical?'<a href="https://x.com/zeya/status/999"><time>then</time></a>':''}</div>${known?'<div data-testid="UserAvatar-Container-zeya">':''}${image}${known?'</div>':''}<p data-testid="tweetText">Observed quote ✨</p></div>`;
  // Source-like fixtures only: actual source DOM/paint remains a live
  // recapture acceptance step, not something these fixtures can establish.
  it.each([
    [false,false,'parent'],[false,true,'parent'],[true,false,'parent'],[true,true,'parent'],
    [false,false,'sibling'],[false,true,'sibling'],[true,false,'sibling'],[true,true,'sibling'],
  ] as const)('retains an exact painted profile image: known=%s canonical=%s paint=%s', (known,canonical,paint) => {
    const image=paint==='parent'
      ? `<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}" alt="zeya"></div>`
      : `<div><div style="background-image:url(${quotePaintUrl})"></div><img style="opacity:0" src="${quotePaintUrl}" alt="zeya"></div>`;
    const row=capture(post().replace('</article>',paintedQuote(image,known,canonical)+'</article>')).records[0];
    const view=canonical?row.presentation.quote:row.presentation.quotePresentation;
    expect(view.avatarUrl).toBe(quotePaintUrl);
    expect(row.media).toEqual([{kind:'avatar',url:quotePaintUrl,alt:'zeya'}]);
    expect(view.body).toBe('Observed quote ✨');
    expect(row.presentation).not.toHaveProperty('avatarUrl');
    if (!canonical) expect(view).not.toHaveProperty('id');
  });
  it.each([
    ['no paint',`<div><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['foreign resource','<div style="background-image:url(https://foreign.test/profile_images/456/zeya.jpg)"><img style="opacity:0" src="https://foreign.test/profile_images/456/zeya.jpg"></div>'],
    ['foreign paint',`<div style="background-image:url(https://foreign.test/paint.jpg)"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['other profile paint',`<div style="background-image:url(https://pbs.twimg.com/profile_images/456/other.jpg)"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['layered paint',`<div style="background-image:url(${quotePaintUrl}),url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['hidden ancestor',`<div hidden><div style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div></div>`],
    ['inert ancestor',`<div inert style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['aria-hidden ancestor',`<div aria-hidden="true" style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['transparent ancestor',`<div style="opacity:0;background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['hidden image',`<div style="background-image:url(${quotePaintUrl})"><img hidden style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['display-none image',`<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0;display:none" src="${quotePaintUrl}"></div>`],
    ['visibility-hidden image',`<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0;visibility:hidden" src="${quotePaintUrl}"></div>`],
    ['displaced parent',`<div data-shifted style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['displaced sibling',`<div><div data-shifted style="background-image:url(${quotePaintUrl})"></div><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['zero-size paint',`<div data-zero-size style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['non-finite paint',`<div data-invalid-rect style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['non-finite sizing image',`<div style="background-image:url(${quotePaintUrl})"><img data-invalid-rect style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['non-immediate paint',`<div style="background-image:url(${quotePaintUrl})"><div><img style="opacity:0" src="${quotePaintUrl}"></div></div>`],
    ['two painting siblings',`<div><div style="background-image:url(${quotePaintUrl})"></div><div style="background-image:url(${quotePaintUrl})"></div><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['painting sibling with unrelated text',`<div><div style="background-image:url(${quotePaintUrl})">unrelated</div><img style="opacity:0" src="${quotePaintUrl}"></div>`],
    ['two painted candidates',`<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"><img style="opacity:0" src="${quotePaintUrl}"></div>`],
  ])('rejects %s for both known and fallback quote avatar paint', (_case,image) => {
    for (const known of [true,false]) {
      const row=capture(post().replace('</article>',paintedQuote(image,known)+'</article>')).records[0];
      expect(row.presentation.quotePresentation).not.toHaveProperty('avatarUrl');
      expect(row.media.filter((entry:{kind:string})=>entry.kind==='avatar')).toHaveLength(0);
    }
  });
  it.each([false,true])('preserves an already loaded offscreen quote avatar with exact paint, known=%s', known => {
    const image=`<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`;
    const html=post().replace('</article>',paintedQuote(image,known)+'</article>');
    const row=capture(html,[],{offscreen:true}).records[0];
    expect(row.presentation.quotePresentation.avatarUrl).toBe(quotePaintUrl);
    expect(row.media).toEqual([{kind:'avatar',url:quotePaintUrl,alt:''}]);
    expect(capture(html,[],{offscreen:true,diagnostics:true}).records).toEqual([row]);
  });
  it('does not relax exact paint, hidden ancestors or genuine nested-card exclusion for loaded offscreen quotes', () => {
    const image=`<div style="background-image:url(${quotePaintUrl})"><img style="opacity:0" src="${quotePaintUrl}"></div>`;
    for(const known of [true,false]){
      for(const invalid of [
        image.replace('<div style','<div data-shifted style'),
        image.replace('<div style','<div data-zero-size style'),
        image.replace('<div style','<div data-invalid-rect style'),
        `<div hidden>${image}</div>`, `<div aria-hidden="true">${image}</div>`,
        `<div style="opacity:0">${image}</div>`,
        image.replace(`url(${quotePaintUrl})`,'url(https://pbs.twimg.com/profile_images/456/other.jpg)'),
      ]) {
        const row=capture(post().replace('</article>',paintedQuote(invalid,known)+'</article>'),[],{offscreen:true}).records[0];
        expect(row.presentation.quotePresentation).not.toHaveProperty('avatarUrl');
      }
      const nested=paintedQuote(image,known);
      const outer=`<div data-testid="quotedTweet"><div data-testid="User-Name">Outer</div><p data-testid="tweetText">Outer quote</p>${nested}</div>`;
      expect(capture(post().replace('</article>',outer+'</article>'),[],{offscreen:true}).records[0].presentation.quotePresentation).not.toHaveProperty('avatarUrl');
    }
  });
  it('uses only the known quote avatar when another already loaded profile image belongs to the quote body', () => {
    const image=`<div><div style="background-image:url(${quotePaintUrl})"></div><img style="opacity:0" src="${quotePaintUrl}"></div>`;
    const quote=paintedQuote(image,true).replace('Observed quote ✨',`Observed quote <span style="background-image:url(https://pbs.twimg.com/profile_images/777/inline.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/profile_images/777/inline.jpg"></span>`);
    const row=capture(post().replace('</article>',quote+'</article>'),[],{offscreen:true}).records[0];
    expect(row.presentation.quotePresentation.avatarUrl).toBe(quotePaintUrl);
    expect(row.media.filter((entry:{kind:string})=>entry.kind==='avatar')).toEqual([{kind:'avatar',url:quotePaintUrl,alt:''}]);
  });
  it('preserves body and quote whitespace around rendered emoji, explicit line breaks and safe text branches', () => {
    const body='<span>first  </span><img alt="✨"><span> second\n  third </span><img alt="👩🏽‍💻"><br><span>last</span>';
    const quote=`<div role="link"><div data-testid="User-Name"><span>Name</span><span>@other</span></div><div data-testid="tweetText">${body}</div></div>`;
    const row=capture(post('123',body).replace('</article>',quote+'</article>')).records[0];
    expect(row.body).toBe('first  ✨ second\n  third 👩🏽‍💻\nlast');
    expect(row.presentation.quotePresentation.body).toBe(row.body);
    expect(row.truncated).toBe(false);
    expect(capture(post('123','<span>first  \n</span><span>  second</span>')).records[0].body).toBe('first  \n  second');
  });
  it('never inserts hidden, scripted, vector, image-description or oversized emoji-alt text into body/quote', () => {
    const body='Before<span hidden>private<img alt="✨"></span><span style="opacity:0">hidden</span><script>unsafe()</script><style>bad{}</style><svg><text>vector</text></svg><img alt="portrait"><img alt="✨ description"><img alt="'+ '✨'.repeat(33) +'"><img alt="1✨"><img alt="🇰🇷"> After';
    const quote=`<div role="link"><div data-testid="User-Name">Name</div><p data-testid="tweetText">${body}</p></div>`;
    const row=capture(post('123',body).replace('</article>',quote+'</article>')).records[0];
    expect(row.body).toBe('Before🇰🇷 After');
    expect(row.presentation.quotePresentation.body).toBe('Before🇰🇷 After');
    expect(row.quotes[0]).not.toMatch(/private|hidden|unsafe|bad\{|vector|portrait|description/);
  });
  it('retains text length limits and marks bounded content traversal as truncated rather than complete', () => {
    const long=capture(post('123','x'.repeat(23999)+'<img alt="✨">tail')).records[0];
    expect(long.body).toHaveLength(24000);expect(long.body.endsWith('✨')).toBe(true);expect(long.truncated).toBe(true);
    const deep='<span>'.repeat(66)+'<img alt="✨">'+'</span>'.repeat(66);
    expect(capture(post('123',deep)).records[0].truncated).toBe(true);
    const broad='<span>x</span>'.repeat(4097);
    expect(capture(post('123',broad)).records[0].truncated).toBe(true);
    const quote='<div role="link"><div data-testid="User-Name">Name</div><p data-testid="tweetText">'+'x'.repeat(11999)+'<img alt="✨">tail</p></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.quotePresentation.body).toHaveLength(12000);expect(row.truncated).toBe(true);
  });
  it('recognizes quotedTweet and a unique exact observed quote card permalink without a time node', () => {
    const quote='<div data-testid="quotedTweet" role="link"><a href="https://x.com/other/status/999">Quoted post</a><div data-testid="User-Name"><a href="https://x.com/other">Quoted name</a></div><p data-testid="tweetText">Quoted body</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quote.jpg"></div></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.body).toBe('Body');expect(row.id).toBe('123');
    expect(row.presentation.quote).toMatchObject({id:'999',url:'https://x.com/other/status/999',body:'Quoted body',timestamp:'',mediaUrls:['https://pbs.twimg.com/media/quote.jpg']});
    expect(row.quotes[0]).toContain('Quoted body');
  });
  it('does not fabricate quote identity from linkless cards or user-written body permalinks', () => {
    const quote='<div data-testid="quotedTweet"><div data-testid="User-Name">Someone</div><p data-testid="tweetText">User text <a href="https://x.com/author/status/999">a mentioned post</a></p></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation).not.toHaveProperty('quote');
    expect(row.quotes[0]).toContain('User text');expect(row.body).toBe('Body');
  });
  it.each([
    ['linkless', ''],
    ['ambiguous', '<a href="https://x.com/other/status/998">One post</a><a href="https://x.com/other/status/999">Another post</a>'],
  ])('preserves %s quote media without misattributing it to the root or inventing a quote identity', (_kind, links) => {
    const quote=`<div data-testid="quotedTweet">${links}<div data-testid="User-Name">Someone</div><p data-testid="tweetText">Unidentified quote</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/unidentified.jpg" alt="Observed quote image"></div></div>`;
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.media).toEqual([{kind:'image',url:'https://pbs.twimg.com/media/unidentified.jpg',alt:'Observed quote image'}]);
    expect(row.presentation.mediaUrls).toEqual([]);
    expect(row.presentation).not.toHaveProperty('quote');
    expect(row.quotes[0]).toContain('Unidentified quote');
    expect(row.body).toBe('Body');
  });
  it('attributes only quote-observed media resources, including an observed matching video playlist', () => {
    const quote='<div data-testid="quoteTweet"><div data-testid="User-Name"><a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Video quote</p><video poster="https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg" src="blob:https://x.com/local"></video></div>';
    const html=post().replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/parent.jpg"></div>'+quote+'</article>');
    const row=capture(html,['https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8']).records[0];
    expect(row.presentation.quote.mediaUrls).toEqual(['https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8','https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg']);
    expect(row.presentation.quote.mediaUrls).not.toContain('https://pbs.twimg.com/media/parent.jpg');
    expect(row.presentation.mediaUrls).toEqual(['https://pbs.twimg.com/media/parent.jpg']);
  });
  it('preserves a shared image and self-quote avatar in both visual positions while storing each resource only once', () => {
    const avatar='<div data-testid="UserAvatar-Container-author"><img src="https://pbs.twimg.com/profile_images/123/shared.jpg"></div>';
    const photo='<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/shared.jpg" alt="Same picture"></div>';
    const quote=`<div data-testid="quotedTweet"><div data-testid="User-Name"><a href="https://x.com/author/status/999"><time>then</time></a></div><p data-testid="tweetText">Self quote</p>${avatar}${photo}</div>`;
    const row=capture(post().replace('</article>',avatar+photo+quote+'</article>')).records[0];
    expect(row.presentation.mediaUrls).toEqual(['https://pbs.twimg.com/media/shared.jpg']);
    expect(row.presentation.quote.mediaUrls).toEqual(row.presentation.mediaUrls);
    expect(row.presentation.avatarUrl).toBe('https://pbs.twimg.com/profile_images/123/shared.jpg');
    expect(row.presentation.quote.avatarUrl).toBe(row.presentation.avatarUrl);
    expect(row.media).toHaveLength(2);
  });
  it('records an explicit empty own-media set when all displayed media belongs to the quote', () => {
    const quote='<div data-testid="quoteTweet"><div data-testid="User-Name"><a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Photo quote</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quoted.jpg"></div></div>';
    const row=capture(post().replace('</article>',quote+'</article>')).records[0];
    expect(row.presentation.mediaUrls).toEqual([]);
    expect(row.presentation.quote.mediaUrls).toEqual(['https://pbs.twimg.com/media/quoted.jpg']);
    expect(row.presentation).not.toHaveProperty('avatarUrl');
  });
  it('keeps observed own video and poster occurrences separate from unrelated quote images', () => {
    const video='<video poster="https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg" src="blob:https://x.com/local"></video>';
    const quote='<div data-testid="quoteTweet"><div data-testid="User-Name"><a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Photo quote</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quoted.jpg"></div></div>';
    const row=capture(post().replace('</article>',video+quote+'</article>'),['https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8']).records[0];
    expect(row.presentation.mediaUrls).toEqual(['https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8','https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg']);
    expect(row.presentation.quote.mediaUrls).toEqual(['https://pbs.twimg.com/media/quoted.jpg']);
  });
  it('freezes each observed compact timestamp label without recalculating or borrowing quote time', () => {
    const quote='<div data-testid="quoteTweet"><div data-testid="User-Name"><a href="https://x.com/other/status/999"><time datetime="2026-10-19T01:00:00Z">8시간</time></a></div><p data-testid="tweetText">Quote</p></div>';
    const root=post().replace('>now</time>','>오후 1:00 · 2026년 10월 20일</time>').replace('</article>',quote+'</article>');
    const reply=post('456').replace('>now</time>','>44분</time>');
    const rows=capture(conversation(root+reply)).records;
    expect(rows[0].presentation.timestampLabel).toBe('오후 1:00 · 2026년 10월 20일');
    expect(rows[0].presentation.quote.timestampLabel).toBe('8시간');
    expect(rows[1].presentation.timestampLabel).toBe('44분');
    expect(rows[0].timestamp).toBe('2026-10-20T01:00:00Z');
  });
  it('omits absent or hidden time labels and bounds observed source labels', () => {
    expect(capture(post().replace('>now</time>','></time>')).records[0].presentation).not.toHaveProperty('timestampLabel');
    expect(capture(post().replace('<time datetime=','<time hidden datetime=')).records[0].presentation).not.toHaveProperty('timestampLabel');
    expect(capture(post().replace('>now</time>',`>${'가'.repeat(100)}</time>`)).records[0].presentation.timestampLabel).toBe('가'.repeat(80));
    const quote='<div data-testid="quotedTweet" role="link"><a href="https://x.com/other/status/999">Quote</a><div data-testid="User-Name">Other</div><p data-testid="tweetText">8시간</p></div>';
    expect(capture(post().replace('</article>',quote+'</article>')).records[0].presentation.quote).not.toHaveProperty('timestampLabel');
  });
  it('does not extract nested quote articles as independent thread records', () => {
    const row=capture(conversation(post().replace('</article>',`<div data-testid="quoteTweet">${post('456','Nested quote')}</div></article>`))).records;
    expect(row.map((record:{id:string})=>record.id)).toEqual(['123']);
  });
  it('keeps loaded root and comment identity, displayed counts, order and exact original media URL', () => {
    const source = post().replace('</article>', '<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/example?format=jpg&amp;name=large" alt="Scene"></div></article>') + post('456', 'Comment');
    const result = capture(source);
    expect(result.sourceUrl).toBe('https://x.com/author/status/123');
    expect(result.coverage).toBe('loaded_dom_only');
    expect(result.records.map((r: { id: string }) => r.id)).toEqual(['123', '456']);
    expect(result.records[0]).toMatchObject({ body: 'Body', counts: '2 replies, 3 likes', media: [{ kind: 'image', url: 'https://pbs.twimg.com/media/example?format=jpg&name=large', alt: 'Scene' }] });
  });
  it('captures quote text and media without replacing the outer permalink/body', () => {
    const quote = '<div data-testid="quoteTweet" role="link"><div data-testid="User-Name">Quoted <a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Quoted text</p><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/quote.jpg"></div></div>';
    const result = capture(post().replace('</article>', `${quote}</article>`));
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({ id: '123', body: 'Body', author: 'Author now' });
    expect(result.records[0].quotes[0]).toContain('Quoted text');
    expect(result.records[0].media[0].url).toBe('https://pbs.twimg.com/media/quote.jpg');
  });
  it('handles the actual link-root quote layout without quoteTweet test id', () => {
    const quote = '<div role="link"><div data-testid="User-Name">Quoted <a href="https://x.com/other/status/999"><time>then</time></a></div><p data-testid="tweetText">Nested quote</p></div>';
    const result = capture(post().replace('</article>', `${quote}</article>`));
    expect(result.records[0].body).toBe('Body');
    expect(result.records[0].quotes[0]).toContain('Nested quote');
  });
  it('only associates observed video URLs in the DOM poster scope', () => {
    const result = capture(post().replace('</article>', '<video poster="https://pbs.twimg.com/ext_tw_video_thumb/111/pu/img/poster.jpg" src="blob:https://x.com/local"></video></article>'), ['https://video.twimg.com/ext_tw_video/222/pu/vid/other.mp4', 'https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8']);
    expect(result.records[0].media.find((m: { kind: string }) => m.kind === 'video').url).toBe('https://video.twimg.com/ext_tw_video/111/pu/pl/root.m3u8');
    const unavailable = capture(post().replace('</article>', '<video src="blob:https://x.com/local"></video></article>'));
    expect(unavailable.records[0].media[0].kind).toBe('unavailable_video');
  });
  it('does not mistake a native tweetPhoto video container for an unavailable still image', () => {
    const video = '<div data-testid="tweetPhoto"><div data-testid="videoPlayer"><video poster="https://pbs.twimg.com/amplify_video_thumb/111/img/poster.jpg" src="blob:https://x.com/local"></video></div></div>';
    const result = capture(post().replace('</article>', `${video}</article>`), ['https://video.twimg.com/amplify_video/111/pl/root.m3u8']);
    expect(result.records[0].media.map((m: { kind: string }) => m.kind)).toEqual(['video', 'poster']);
    const mixed = capture(post().replace('</article>', `${video}<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/photo.jpg"></div><div data-testid="tweetPhoto"></div></article>`));
    expect(mixed.records[0].media.filter((m: { kind: string }) => m.kind === 'image')).toHaveLength(1);
    expect(mixed.records[0].media.filter((m: { kind: string }) => m.kind === 'unavailable_image')).toHaveLength(1);
  });
  it('does not read hidden rows or silently claim truncated text complete', () => {
    const result = capture(`<div style="display:none">${post('789')}</div>${post('123', 'x'.repeat(24001))}`);
    expect(result.records).toHaveLength(1);
    expect(result.records[0].body).toHaveLength(24000);
    expect(result.records[0].truncated).toBe(true);
  });
  it('has no network, mutation, private runtime, cookie or source HTML access', () => {
    expect(script).not.toMatch(/fetch\(|XMLHttpRequest|document\.cookie|localStorage|sessionStorage|__react|innerHTML|outerHTML|\.click\(/);
    const result = capture(post('123', '&lt;img src=x onerror=alert(1)&gt;'));
    expect(result.records[0].body).toBe('<img src=x onerror=alert(1)>');
  });
  it('proves the displayed account without reading cookies or storage', () => {
    const identity='<a data-testid="AppTabBar_Profile_Link" href="https://x.com/Alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
    expect(capture(identity+post()).owner).toBe('alice');
    expect(capture(post()).owner).toBe('');
    expect(capture(identity+'<a data-testid="AppTabBar_Profile_Link" href="https://x.com/Bob"></a>'+post()).owner).toBe('');
  });
  it('captures a unique visible full-page photo with same-document owner proof without inventing body or replies', () => {
    const result = capture('<img src="https://pbs.twimg.com/media/actual?format=png&amp;name=large" alt="Actual photo">', [], { path: '/author/status/123/photo/1', photoOwner: 'alice' });
    expect(result.owner).toBe('alice');
    expect(result.sourceUrl).toBe('https://x.com/author/status/123/photo/1');
    expect(result.records).toEqual([{ id: '123', url: 'https://x.com/author/status/123', author: '', body: '', timestamp: '', counts: '', context: '', quotes: [], media: [{ kind: 'image', url: 'https://pbs.twimg.com/media/actual?format=png&name=large', alt: 'Actual photo' }], truncated: false, photoOnly: true }]);
  });
  const carousel=(ordinal=1)=>{
    const left=[ordinal===1?10:-595,ordinal===2?10:620];
    const image=(name:string,index:number)=>`<li><div data-left="${left[index]}" style="background-image:url(https://pbs.twimg.com/media/${name}.jpg)"><img data-left="${left[index]}" style="opacity:0" src="https://pbs.twimg.com/media/${name}.jpg" alt="${name}"></div></li>`;
    return `<span>슬라이드 ${ordinal}/2 - 회전 슬라이드</span><div aria-roledescription="carousel"><ul style="overflow-x:hidden">${image('first',0)}${image('second',1)}</ul></div>`;
  };
  it('keeps the current-carousel predicate byte-identical between controls and capture',()=>{
    const select=(value:string,end:string)=>value.slice(value.indexOf('  function resolveCurrentCarouselPhoto('),value.indexOf(end,value.indexOf('  function resolveCurrentCarouselPhoto('))).trim();
    expect(select(script,'  // X photos can remove')).toBe(select(actionsScript,'  function currentPhoto('));
  });
  it.each([1,2])('captures exactly the proved current source slide %s without its clipped neighbour',ordinal=>{
    const result=capture(carousel(ordinal),[],{path:`/author/status/123/photo/${ordinal}`,photoOwner:'alice'});
    expect(result.owner).toBe('alice');expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({id:'123',photoOnly:true,media:[{
      kind:'image',url:`https://pbs.twimg.com/media/${ordinal===1?'first':'second'}.jpg`,alt:ordinal===1?'first':'second',
    }]});
    expect(result.records[0].media).toHaveLength(1);
    expect(Object.keys(result.records[0].media[0]).sort()).toEqual(['alt','kind','url']);
  });
  it.each([1,2])('captures the exact slide %s in an observed four-item viewport-fixed carousel',ordinal=>{
    const html=carousel(ordinal).replace(`슬라이드 ${ordinal}/2`,`슬라이드 ${ordinal}/4`)
      .replace('overflow-x:hidden','position:fixed;overflow-x:hidden').replace('</ul>','<li></li><li></li></ul>');
    const row=capture(html,[],{path:`/author/status/123/photo/${ordinal}`,photoOwner:'alice'}).records[0];
    expect(row.media).toEqual([{kind:'image',url:`https://pbs.twimg.com/media/${ordinal===1?'first':'second'}.jpg`,alt:ordinal===1?'first':'second'}]);
  });
  it.each([
    'transform:translateX(1px)','translate:1px','rotate:1deg','scale:1','perspective:100px','filter:blur(0px)',
    'backdrop-filter:blur(0px)','contain:paint','will-change:transform','container-type:inline-size',
    'content-visibility:auto','transform-style:preserve-3d',
  ])('does not infer viewport clipping through an uncertain fixed containing block %s',style=>{
    const html=`<div style="${style}">${carousel().replace('overflow-x:hidden','position:fixed;overflow-x:hidden')}</div>`;
    expect(capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
  });
  it('does not falsely clip a viewport-fixed neighbouring photo using an outside overflow ancestor',()=>{
    const html=`<div style="overflow-x:hidden">${carousel().replace('overflow-x:hidden','position:fixed;overflow-x:visible')}</div>`;
    expect(capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
  });
  it.each([
    ['source counter missing',(html:string)=>html.replace('<span>슬라이드 1/2 - 회전 슬라이드</span>','')],
    ['source counter conflicts',(html:string)=>html.replace('슬라이드 1/2','슬라이드 2/2')],
    ['source total conflicts',(html:string)=>html.replace('슬라이드 1/2','슬라이드 1/3')],
    ['wrong item position',(html:string)=>html.replace('<li>','<li aria-posinset="2">')],
    ['wrong selected item',(html:string)=>html.replace('<li>','<li aria-selected="false">')],
    ['hidden selected item',(html:string)=>html.replace('<li>','<li aria-hidden="true">')],
    ['unknown clipping',(html:string)=>html.replace('overflow-x:hidden','overflow-x:visible')],
    ['fixed escaping image',(html:string)=>html.replace('style="opacity:0"','style="opacity:0;position:fixed"')],
    ['absolute escaping image',(html:string)=>html.replace('style="opacity:0"','style="opacity:0;position:absolute"')],
    ['multiple lists',(html:string)=>html.replace('</ul>','</ul><ul></ul>')],
    ['nested carousel',(html:string)=>`<div role="carousel">${html}</div>`],
  ])('does not capture an ambiguous or contradictory carousel: %s',(_label,change)=>{
    expect(capture(change(carousel()),[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
  });
  it('rejects an old single candidate during a route/counter transition and preserves owner/security checks',()=>{
    const onlyOld=carousel().replace(/<li><div data-left="620"[\s\S]*?<\/li>/,'');
    expect(capture(onlyOld,[],{path:'/author/status/123/photo/2',photoOwner:'alice'}).records).toEqual([]);
    const options={path:'/author/status/123/photo/1',photoOwner:'alice'};
    expect(capture(carousel(),[],{...options,photoOwner:undefined}).records).toEqual([]);
    expect(capture(carousel()+'<input type="password">',[],options).records).toEqual([]);
    expect(capture(carousel()+'<a data-testid="AppTabBar_Profile_Link" href="https://x.com/bob"></a>',[],options).records).toEqual([]);
  });
  it('admits absolute sizing images whose positioned parents remain inside the clipping list',()=>{
    const html=carousel().replaceAll('style="opacity:0"','style="opacity:0;position:absolute"')
      .replaceAll('style="background-image:','style="position:relative;background-image:');
    expect(capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records[0].media)
      .toEqual([{kind:'image',url:'https://pbs.twimg.com/media/first.jpg',alt:'first'}]);
  });
  it('does not borrow a URL-deduplicated image occurrence to prove another current slide',()=>{
    const first=carousel().replaceAll('/second.jpg','/first.jpg');
    expect(capture(first,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
    const second=carousel(2).replaceAll('/second.jpg','/first.jpg');
    expect(capture(second,[],{path:'/author/status/123/photo/2',photoOwner:'alice'}).records[0].media)
      .toEqual([{kind:'image',url:'https://pbs.twimg.com/media/first.jpg',alt:'second'}]);
  });
  it('rejects an over-limit image DOM instead of deciding uniqueness from a truncated prefix', () => {
    const first = '<img src="https://pbs.twimg.com/media/first.jpg">'.repeat(128);
    const options = { path: '/author/status/123/photo/1', photoOwner: 'alice' };
    expect(capture(first, [], options).records).toHaveLength(1);
    expect(capture(first+'<img src="https://pbs.twimg.com/media/second.jpg">', [], options).records).toEqual([]);
  });
  it.each([
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg" alt="Displayed"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg" alt="Displayed"></div>',
  ])('captures a transparent sizing image only when its exact resource is visibly painted in its immediate container: %s', html => {
    const result=capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'});
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({photoOnly:true,media:[{kind:'image',url:'https://pbs.twimg.com/media/actual.jpg',alt:'Displayed'}]});
  });
  it.each([
    '<div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/other.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg),url(https://pbs.twimg.com/media/other.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="opacity:0;background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div hidden style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div inert style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div aria-hidden="true" style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img hidden style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0;display:none" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0;visibility:hidden" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div data-shifted style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div data-zero-size style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div></div>',
    '<div><div data-shifted style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="opacity:0;background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)">unrelated text</div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><span></span></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><div style="background-image:url(https://pbs.twimg.com/media/other.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div hidden style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div><div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"></div><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg">'+ '<span></span>'.repeat(7)+'</div>',
  ])('rejects unproven, hidden, layered, displaced or nonlocal background paint: %s', html => {
    expect(capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
  });
  it('never lets visible background proof bypass account, route, ambiguous-modal, security or uniqueness gates', () => {
    const html='<div style="background-image:url(https://pbs.twimg.com/media/actual.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/actual.jpg"></div>';
    const options={path:'/author/status/123/photo/1',photoOwner:'alice'};
    expect(capture(html,[],{...options,photoOwner:undefined}).records).toEqual([]);
    expect(capture(html,[],{...options,path:'/author/status/123'}).records).toEqual([]);
    expect(capture(html,[],{...options,offscreen:true}).records).toEqual([]);
    expect(capture(`<div role="dialog">${html}</div><div role="dialog">Another layer</div>`,[],options).records).toEqual([]);
    expect(capture(html+'<input type="password">',[],options).records).toEqual([]);
    expect(capture(html+'<img src="https://pbs.twimg.com/media/other.jpg">',[],options).records).toEqual([]);
  });
  it.each([
    { photoOwner: undefined }, { photoOwner: '' }, { photoOwner: 'invalid/value' },
    { photoOwner: 'alice', path: '/author/status/123' }, { photoOwner: 'alice', path: '/author/status/123/photo/5' },
    { photoOwner: 'alice', path: '/i/web/status/123/photo/1' }, { photoOwner: 'alice', origin: 'https://evil.test' },
    { photoOwner: 'alice', offscreen: true },
  ])('fails closed without canonical photo route, valid proof and visible image: %j', options => {
    const result = capture('<img src="https://pbs.twimg.com/media/actual.jpg">', [], { path: '/author/status/123/photo/1', ...options });
    expect(result.records).toEqual([]);
  });
  it.each([
    '<div hidden><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div inert><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div aria-hidden="true"><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="display:none"><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div style="opacity:0"><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<img src="https://pbs.twimg.com/profile_images/actual.jpg">',
    '<img src="https://pbs.twimg.com/media/one.jpg"><img src="https://pbs.twimg.com/media/two.jpg">',
    '<img src="https://pbs.twimg.com/media/actual.jpg"><input type="password">',
    '<img src="https://pbs.twimg.com/media/actual.jpg"><a href="/login">Login</a>',
    '<img src="https://pbs.twimg.com/media/actual.jpg"><a data-testid="AppTabBar_Profile_Link" href="https://x.com/bob"></a>',
    '<div role="dialog" hidden><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
    '<div aria-modal="true" style="display:none"><img src="https://pbs.twimg.com/media/actual.jpg"></div>',
  ])('does not synthesize a photo snapshot for hidden, ambiguous or security-gated DOM: %s', html => {
    expect(capture(html, [], { path: '/author/status/123/photo/1', photoOwner: 'alice' }).records).toEqual([]);
  });
  it('keeps an ordinary post/article snapshot distinct from the photo-only fallback', () => {
    const result = capture(post().replace('</article>', '<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/actual.jpg"></div></article>'), [], { path: '/author/status/123/photo/1', photoOwner: 'alice' });
    expect(result.records[0]).toMatchObject({ body: 'Body', author: 'Author now', truncated: false });
    expect(result.records[0].photoOnly).not.toBe(true);
  });
  it('keeps article-based photo dialogs on the ordinary post capture path', () => {
    const result = capture(`<div role="dialog">${post().replace('</article>', '<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/actual.jpg"></div></article>')}</div>`, [], { path: '/author/status/123/photo/1', photoOwner: 'alice' });
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({ id: '123', body: 'Body', author: 'Author now' });
    expect(result.records[0].photoOnly).not.toBe(true);
  });
  it.each(['role="dialog"','aria-modal="true"'])('captures only the independently displayed modal photo without background articles: %s', attribute => {
    const background=post('999','Background post').replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/background.jpg"></div></article>');
    const result=capture(background+`<div ${attribute}><img src="https://pbs.twimg.com/media/current.jpg" alt="Current photo"></div>`,[],{path:'/author/status/123/photo/1',photoOwner:'alice'});
    expect(result.owner).toBe('alice');expect(result.records).toHaveLength(1);
    expect(result.records[0]).toEqual({id:'123',url:'https://x.com/author/status/123',author:'',body:'',timestamp:'',counts:'',context:'',quotes:[],media:[{kind:'image',url:'https://pbs.twimg.com/media/current.jpg',alt:'Current photo'}],truncated:false,photoOnly:true});
    expect(JSON.stringify(result.records)).not.toMatch(/Background|background\.jpg|999/);
  });
  it('keeps only the exact matching full post in a photo dialog when its media already proves the independently shown image',()=>{
    const matching=post().replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/current.jpg"></div><div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/second.jpg"></div></article>');
    const result=capture(post('999','Background')+`<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg">${matching}${post('456','Unrelated reply')}</div>`,[],{path:'/author/status/123/photo/1',photoOwner:'alice'});
    expect(result.records).toHaveLength(1);
    expect(result.records[0]).toMatchObject({id:'123',body:'Body',author:'Author now'});
    expect(result.records[0].photoOnly).not.toBe(true);expect(result.records[0].media).toHaveLength(2);
  });
  it.each([
    post().replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/not-current.jpg"></div></article>'),
    post('456','Another post').replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/current.jpg"></div></article>'),
    post().replace('https://x.com/author/status/123','https://x.com/someone/status/123').replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/current.jpg"></div></article>'),
  ])('does not join a photo with an article whose resource or exact permalink differs', article=>{
    const result=capture(`<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg">${article}</div>`,[],{path:'/author/status/123/photo/1',photoOwner:'alice'});
    expect(result.records).toHaveLength(1);expect(result.records[0]).toMatchObject({id:'123',body:'',author:'',photoOnly:true,media:[{kind:'image',url:'https://pbs.twimg.com/media/current.jpg'}]});
  });
  it('uses the single visible inner dialog and ignores its backdrop articles and hidden older dialogs',()=>{
    const html=`<div role="dialog" hidden><img src="https://pbs.twimg.com/media/old.jpg"></div><div role="dialog">${post('999','Outer background')}<section aria-modal="true"><img src="https://pbs.twimg.com/media/current.jpg"></section></div>`;
    const result=capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'});
    expect(result.records).toHaveLength(1);expect(result.records[0]).toMatchObject({photoOnly:true,media:[{url:'https://pbs.twimg.com/media/current.jpg'}]});
  });
  it('supports the exact same-size visible background paint inside the admitted modal',()=>{
    const html='<div role="dialog"><div style="background-image:url(https://pbs.twimg.com/media/current.jpg)"><img style="opacity:0" src="https://pbs.twimg.com/media/current.jpg" alt="Current"></div></div>';
    expect(capture(html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records[0]).toMatchObject({photoOnly:true,media:[{url:'https://pbs.twimg.com/media/current.jpg',alt:'Current'}]});
  });
  it.each([
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"></div><div role="dialog">Unrelated top layer</div>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"><img src="https://pbs.twimg.com/media/other.jpg"></div>',
    '<div role="dialog"><img style="opacity:0" src="https://pbs.twimg.com/media/current.jpg"></div>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"><input type="password"></div>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"><a href="/login">Login</a></div>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"></div><a data-testid="AppTabBar_Profile_Link" href="https://x.com/bob"></a>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"><div role="dialog">Unknown inner layer</div></div>',
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg"></div>'+('<div role="dialog" hidden></div>').repeat(8),
    '<div role="dialog"><img src="https://pbs.twimg.com/media/current.jpg">'+('<img src="https://pbs.twimg.com/media/current.jpg">').repeat(128)+'</div>',
  ])('does not fall back to background articles for an unproven or ambiguous photo modal: %s',html=>{
    expect(capture(post('999','Background')+html,[],{path:'/author/status/123/photo/1',photoOwner:'alice'}).records).toEqual([]);
  });
  it('requires existing same-document owner proof even when the modal contains a valid ordinary article',()=>{
    const identity='<a data-testid="AppTabBar_Profile_Link" href="https://x.com/alice"></a><button data-testid="SideNav_AccountSwitcher_Button"></button>';
    const html=`<div role="dialog">${post().replace('</article>','<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/current.jpg"></div></article>')}</div>`;
    expect(capture(identity+html,[],{path:'/author/status/123/photo/1'}).records).toEqual([]);
    expect(capture(identity+html,[],{path:'/author/status/123/photo/1',photoOwner:'bob'}).records).toEqual([]);
  });
  const quoteDiagnosticFields = [
    'detailRoot','quoteFound','quoteVisible','authorUnambiguous','conflictingTargets','selected','admitted','mediaFull',
    'quoteRoots','nestedRoots','aliases','authorNodes','contentNodes','links','images','known','knownInScope',
    'profiles','notProfile','bodyOrMedia','nested','visible','painted','hidden','ariaHidden','ancestorHidden','opacityZero',
    'paintOwnStyle','paintParent','paintRect','paintLayers','paintShape','paintMatch','candidates',
    'selectedParentDiv','selectedAncestorComplete','selectedAncestorLimit','selectedParentChildren','selectedAncestors',
    'selectedAncestorAriaHidden','selectedAncestorHidden','selectedAncestorInert','selectedAncestorDisplayNone',
    'selectedAncestorVisibilityHidden','selectedAncestorOpacityZero'
  ].sort();
  const diagnosticQuote = (image:string,known=true) => `<div role="link"><div data-testid="User-Name"><span>Private author</span><span>@private</span></div><p data-testid="tweetText">Private body</p>${known?'<div data-testid="UserAvatar-Container-private">':''}${image}${known?'</div>':''}</div>`;
  it('emits only closed bounded quote counts/booleans in debug captures and never changes captured records', () => {
    const html=post().replace('</article>',diagnosticQuote(`<img src="${wrapperQuoteUrl}">`)+'</article>');
    const plain=capture(html), debug=capture(html,[],{diagnostics:true});
    expect(plain).not.toHaveProperty('quoteDiagnostic');
    expect(debug.records).toEqual(plain.records);
    expect(Object.keys(debug.quoteDiagnostic).sort()).toEqual(quoteDiagnosticFields);
    expect(debug.quoteDiagnostic).toMatchObject({detailRoot:true,quoteFound:true,quoteVisible:true,authorUnambiguous:true,images:1,known:1,knownInScope:1,profiles:1,visible:1,candidates:1,selected:true,admitted:true});
    for(const value of Object.values(debug.quoteDiagnostic))expect(typeof value==='boolean'||typeof value==='number'&&Number.isInteger(value)&&value>=0&&value<=65).toBe(true);
    expect(JSON.stringify(debug.quoteDiagnostic)).not.toMatch(/private|Private|https|profile_images|wrapped|cookie|token|<|>/);
  });
  it('diagnoses only the first feed record or exact canonical detail root, never a neighbouring quote', () => {
    const first=post('111').replace('</article>',diagnosticQuote(`<img src="${wrapperQuoteUrl}">`)+'</article>');
    const target=post('123').replace('</article>',diagnosticQuote('<img src="https://foreign.test/avatar.jpg">')+'</article>');
    expect(capture(first+target,[],{path:'/i/history',diagnostics:true}).quoteDiagnostic).toMatchObject({detailRoot:false,profiles:1,admitted:true});
    expect(capture(first+target,[],{path:'/author/status/123',diagnostics:true}).quoteDiagnostic).toMatchObject({detailRoot:true,profiles:0,notProfile:1,admitted:false});
    expect(capture(post('111')+target,[],{path:'/i/history',diagnostics:true}).quoteDiagnostic).toMatchObject({detailRoot:false,quoteFound:false,images:0});
    expect(capture(first,[],{path:'/author/status/999',diagnostics:true})).not.toHaveProperty('quoteDiagnostic');
  });
  it.each([
    ['own aria-hidden',`<div style="background-image:url(${wrapperQuoteUrl})"><img aria-hidden="true" style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{ariaHidden:1,paintOwnStyle:1}],
    ['own hidden',`<div><img hidden style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{hidden:1,paintOwnStyle:1}],
    ['ancestor hidden',`<div aria-hidden="true"><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{ancestorHidden:1,paintParent:1}],
    ['non-div parent',`<span><img style="opacity:0" src="${wrapperQuoteUrl}"></span>`,{paintParent:1}],
    ['shifted geometry',`<div data-shifted style="background-image:url(${wrapperQuoteUrl})"><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{paintRect:1}],
    ['no paint layer',`<div><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{paintLayers:1}],
    ['paint sibling shape',`<div><div style="background-image:url(${wrapperQuoteUrl})">unrelated</div><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{paintShape:1}],
    ['paint URL mismatch',`<div style="background-image:url(https://pbs.twimg.com/profile_images/456/other.jpg)"><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{paintMatch:1}],
    ['valid paint',`<div style="background-image:url(${wrapperQuoteUrl})"><img style="opacity:0" src="${wrapperQuoteUrl}"></div>`,{painted:1,admitted:true}],
  ])('distinguishes the fixed quote paint stage %s without exposing the resource', (_case,image,expected) => {
    const html=post().replace('</article>',diagnosticQuote(image as string)+'</article>');
    const debug=capture(html,[],{diagnostics:true});
    expect(debug.quoteDiagnostic).toMatchObject({profiles:1,visible:0,opacityZero:1,...expected});
    expect(debug.records).toEqual(capture(html).records);
    expect(Object.keys(debug.quoteDiagnostic).sort()).toEqual(quoteDiagnosticFields);
  });
  it('counts scope exclusions, duplicate wrappers and known-invalid fallback blocking without changing admission', () => {
    const inner=diagnosticQuote(`<img src="${wrapperQuoteUrl}">`);
    const alias=`<div data-testid="quoteTweet">${inner}</div>`;
    const nested=`<div data-testid="quoteTweet"><div data-testid="User-Name"><span>@outer</span></div><p data-testid="tweetText">Outer body</p>${inner}</div>`;
    const body=diagnosticQuote(`<p data-testid="tweetText"><img src="${wrapperQuoteUrl}"></p>`,false);
    const blocked=diagnosticQuote('<img src="https://foreign.test/avatar.jpg">').replace('</p>',`</p><img src="${wrapperQuoteUrl}">`);
    for(const [quote,expected] of [
      [alias,{quoteRoots:2,nestedRoots:1,aliases:1,nested:0,admitted:true}],
      [nested,{quoteRoots:2,nestedRoots:1,aliases:0,nested:1,candidates:0,admitted:false}],
      [body,{profiles:1,bodyOrMedia:1,candidates:0,admitted:false}],
      [blocked,{known:1,knownInScope:1,notProfile:1,profiles:1,candidates:1,selected:true,admitted:false}],
    ] as const) {
      const html=post().replace('</article>',quote+'</article>');
      expect(capture(html,[],{diagnostics:true}).quoteDiagnostic).toMatchObject(expected);
      expect(capture(html,[],{diagnostics:true}).records).toEqual(capture(html).records);
    }
  });
  it.each([
    ['aria-hidden="true"','selectedAncestorAriaHidden'],
    ['hidden','selectedAncestorHidden'],
    ['inert','selectedAncestorInert'],
    ['style="display:none"','selectedAncestorDisplayNone'],
    ['style="visibility:hidden"','selectedAncestorVisibilityHidden'],
    ['style="visibility:collapse"','selectedAncestorVisibilityHidden'],
    ['style="opacity:0"','selectedAncestorOpacityZero'],
  ])('diagnoses the selected quote avatar ancestor %s without changing its rejection', (attribute,field) => {
    const image=`<div ${attribute}><div style="background-image:url(${wrapperQuoteUrl})"><img style="opacity:0" src="${wrapperQuoteUrl}"></div></div>`;
    const html=post().replace('</article>',diagnosticQuote(image)+'</article>');
    const debug=capture(html,[],{diagnostics:true});
    expect(debug.records).toEqual(capture(html).records);
    expect(debug.quoteDiagnostic).toMatchObject({
      selected:true,admitted:false,ancestorHidden:1,paintParent:1,selectedParentDiv:true,selectedParentChildren:1,
      selectedAncestors:4,selectedAncestorComplete:true,selectedAncestorLimit:false,
      selectedAncestorAriaHidden:0,selectedAncestorHidden:0,selectedAncestorInert:0,
      selectedAncestorDisplayNone:0,selectedAncestorVisibilityHidden:0,selectedAncestorOpacityZero:0,[field]:1,
    });
    expect(debug.records[0].presentation.quotePresentation).not.toHaveProperty('avatarUrl');
    expect(JSON.stringify(debug.quoteDiagnostic)).not.toMatch(/private|Private|https|profile_images|wrapped|cookie|token|<|>/);
  });
  it('keeps selected-ancestor reasons inside the quote and distinguishes a non-DIV paint parent', () => {
    const image=`<span aria-hidden="true"><div hidden inert style="display:none;visibility:hidden;opacity:0"><span><img style="opacity:0" src="${wrapperQuoteUrl}"></span></div></span>`;
    const html=post().replace('</article>',diagnosticQuote(image)+'</article>')+
      '<div aria-hidden="true"><img src="https://foreign.test/unrelated.jpg"></div>';
    const debug=capture(html,[],{diagnostics:true});
    expect(debug.records).toEqual(capture(html).records);
    expect(debug.quoteDiagnostic).toMatchObject({
      selected:true,admitted:false,selectedParentDiv:false,selectedParentChildren:1,selectedAncestors:5,
      selectedAncestorComplete:true,selectedAncestorLimit:false,selectedAncestorAriaHidden:1,selectedAncestorHidden:1,
      selectedAncestorInert:1,selectedAncestorDisplayNone:1,selectedAncestorVisibilityHidden:1,selectedAncestorOpacityZero:1,
    });
  });
  it('bounds selected-ancestor scans without confusing the image own opacity with its ancestors', () => {
    const image=`<img style="opacity:0" src="${wrapperQuoteUrl}">`;
    const shallow=post().replace('</article>',diagnosticQuote(`<div style="background-image:url(${wrapperQuoteUrl})">${image}</div>`)+'</article>');
    const debug=capture(shallow,[],{diagnostics:true});
    expect(debug.records).toEqual(capture(shallow).records);
    expect(debug.quoteDiagnostic).toMatchObject({
      selected:true,admitted:true,opacityZero:1,selectedAncestors:3,selectedAncestorComplete:true,
      selectedAncestorLimit:false,selectedAncestorOpacityZero:0,
    });
    const deep=post().replace('</article>',diagnosticQuote('<div>'.repeat(40)+image+'</div>'.repeat(40))+'</article>');
    const bounded=capture(deep,[],{diagnostics:true});
    expect(bounded.records).toEqual(capture(deep).records);
    expect(bounded.quoteDiagnostic).toMatchObject({
      selected:true,selectedAncestors:32,selectedAncestorComplete:false,selectedAncestorLimit:true,selectedAncestorOpacityZero:0,
    });
  });
  it('does not inspect an arbitrary ancestor when the quote avatar is ambiguous', () => {
    const html=post().replace('</article>',diagnosticQuote(`<img src="${wrapperQuoteUrl}"><img src="${wrapperQuoteUrl}">`)+'</article>');
    const debug=capture(html,[],{diagnostics:true});
    expect(debug.records).toEqual(capture(html).records);
    expect(debug.quoteDiagnostic).toMatchObject({
      selected:false,admitted:false,selectedParentDiv:false,selectedParentChildren:0,selectedAncestors:0,
      selectedAncestorComplete:false,selectedAncestorLimit:false,
    });
  });
  it('bounds quote diagnostic scans and distinguishes candidate ambiguity, no image and media cap', () => {
    const many=diagnosticQuote(`<img src="${wrapperQuoteUrl}">`.repeat(100),false);
    const excessive=capture(post().replace('</article>',many+'</article>'),[],{diagnostics:true}).quoteDiagnostic;
    expect(excessive).toMatchObject({images:65,profiles:64,candidates:64,selected:false,admitted:false});
    const noImage=diagnosticQuote('<div style="background-image:url(https://pbs.twimg.com/profile_images/123/only-background.jpg)"></div>',false);
    expect(capture(post().replace('</article>',noImage+'</article>'),[],{diagnostics:true}).quoteDiagnostic).toMatchObject({images:0,profiles:0,selected:false,admitted:false});
    const media=Array.from({length:16},(_,i)=>`<div data-testid="tweetPhoto"><img src="https://pbs.twimg.com/media/own${i}.jpg"></div>`).join('');
    const capped=post().replace('</article>',media+diagnosticQuote(`<img src="${wrapperQuoteUrl}">`)+'</article>');
    const result=capture(capped,[],{diagnostics:true});
    expect(result.quoteDiagnostic).toMatchObject({profiles:1,candidates:1,selected:true,admitted:false,mediaFull:true});
    expect(result.records).toEqual(capture(capped).records);
  });
  it('only emits bounded non-content capture diagnostics for explicitly enabled empty captures', () => {
    const html='<img style="opacity:0" src="https://pbs.twimg.com/media/secret.jpg" alt="private text">';
    const options={path:'/author/status/123/photo/1',photoOwner:'alice'};
    expect(capture(html,[],options).diagnostic).toBeUndefined();
    const debug=capture(html,[],{...options,diagnostics:true});
    expect(debug.records).toEqual([]);
    expect(debug.diagnostic).toEqual({photoRoute:true,articles:0,dialogs:0,images:1,imagesOverLimit:false,permittedImages:1,cssVisibleImages:0,inViewportImages:0,uniqueCandidates:0,opacityZeroImages:1,zeroSizeImages:0});
    expect(JSON.stringify(debug.diagnostic)).not.toMatch(/alice|author|secret|private|https/);
    const visible='<img src="https://pbs.twimg.com/media/actual.jpg">';
    expect(capture(visible,[],{...options,diagnostics:true}).diagnostic).toBeUndefined();
    expect(capture(visible,[],{...options,diagnostics:true,offscreen:true}).diagnostic).toMatchObject({permittedImages:1,cssVisibleImages:1,inViewportImages:0,uniqueCandidates:0});
    expect(capture(visible+'<img src="https://pbs.twimg.com/media/other.jpg">',[],{...options,diagnostics:true}).diagnostic).toMatchObject({permittedImages:2,cssVisibleImages:2,inViewportImages:2,uniqueCandidates:2});
    expect(capture(visible.repeat(130),[],{...options,diagnostics:true}).diagnostic).toMatchObject({images:129,imagesOverLimit:true,permittedImages:128});
    expect(capture(`<div hidden>${post()}</div>${visible}`,[],{...options,diagnostics:true}).diagnostic).toMatchObject({articles:1,dialogs:0,permittedImages:1});
  });
});
