// Clean-room data adapter. Reads bounded rendered facts, never X application
// internals, cookies, tokens or private API routes. No site markup is returned.
(() => {
  const request = __ATSUMI_X_REQUEST__;
  let shape=null;
  const out = (state, extra={}) => ({nonce:request.nonce,state,...extra,...(request.diagnostics?{shape}: {})});
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname))return out('unavailable');
  const postId=request.feed.startsWith('post:')?request.feed.slice(5):null;
  const profileHandle=request.feed.startsWith('profile:')?request.feed.slice(8):null;
  const communityId=request.feed.startsWith('community:')?request.feed.slice(10):null;
  const homeTab=communityId&&/^tab-\d{1,2}$/.test(communityId);
  if(!['foryou','following','communities','likes','bookmarks'].includes(request.feed)&&!homeTab&&!(/^[1-9]\d{4,24}$/.test(postId||communityId||''))&&!(/^[A-Za-z0-9_]{1,15}$/.test(profileHandle||'')))return out('unavailable');
  if(location.pathname==='/login'||location.pathname.startsWith('/i/flow/'))return out('signedOut');
  const pathOf=raw=>{if(typeof raw!=='string'||!raw.trim())return null;try{const u=new URL(raw,location.origin||'https://'+location.hostname);return u.protocol==='https:'&&['x.com','www.x.com'].includes(u.hostname)&&!u.port&&!u.username&&!u.password&&!u.search&&!u.hash?u.pathname:null;}catch{return null;}};
  const sameOriginPathOf=raw=>{const path=pathOf(raw);try{return path&&new URL(raw,location.origin).origin===location.origin?path:null;}catch{return null;}};
  const profile=document.querySelector('a[data-testid="AppTabBar_Profile_Link"]');
  const account=document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]');
  const owner=/^\/([A-Za-z0-9_]{1,15})\/?$/.exec(pathOf(profile?.getAttribute('href'))||'')?.[1]?.toLowerCase();
  if(!owner||!account)return out(document.querySelector('a[href="/login"],a[href="/i/flow/login"]')?'signedOut':'pending');
  if(request.owner&&request.owner!==owner)return out('ownerChanged');
  const primary=document.querySelector('[data-testid="primaryColumn"]');
  if(!primary)return out('pending');
  // X hydrates text emoji into img[alt]. Read the same visible label on both
  // sides of that transition, without dropping identity-bearing characters.
  const labelOf=root=>{
    let visited=0;
    const read=node=>{
      if(++visited>256)return '';
      if(node.nodeType===3)return node.textContent||'';
      if(node.nodeType!==1)return '';
      if(node.tagName==='IMG')return node.getAttribute('alt')||'';
      return [...node.childNodes].map(read).join('');
    };
    const value=read(root);
    return visited>256?'':value.replace(/\s+/g,' ').trim().normalize('NFC');
  };
  const tabs=[...primary.querySelectorAll('[role="tablist"] [role="tab"]')];
  if(request.diagnostics){
    const address=raw=>{try{const u=new URL(raw);return {host:u.hostname,kind:u.pathname.split('/')[1],extension:u.pathname.split('.').pop()?.slice(-8),query:[...u.searchParams.keys()]};}catch{return null;}};
    shape={path:location.pathname,tabs:tabs.slice(0,12).map(t=>({tag:t.tagName,href:t.getAttribute('href'),childHref:t.querySelector('a[href]')?.getAttribute('href'),selected:t.getAttribute('aria-selected'),testid:t.getAttribute('data-testid')})),articles:[...primary.querySelectorAll('article[data-testid="tweet"]')].slice(0,4).map(a=>({images:[...a.querySelectorAll('img')].slice(0,8).map(n=>({src:address(n.src),current:address(n.currentSrc),testid:n.closest('[data-testid]')?.getAttribute('data-testid')})),videos:a.querySelectorAll('video,[data-testid="videoPlayer"]').length,buttons:[...a.querySelectorAll('button,[role="button"]')].slice(0,12).map(n=>({testid:n.getAttribute('data-testid'),label:(n.getAttribute('aria-label')||'').slice(0,80)}))}))};
  }
  const communities=[];
  // Only exact community links in the account's home tabs qualify; ordinary
  // lists and suggested community cards are not reported as memberships.
  if(location.pathname==='/home')for(const [index,tab] of tabs.entries()){
    const path=pathOf(tab.getAttribute('href')||tab.querySelector('a[href]')?.getAttribute('href'));
    const id=/^\/i\/communities\/([1-9]\d{4,24})$/.exec(path||'')?.[1];
    const name=labelOf(tab);
    if(id&&name&&!communities.some(c=>c.id===id))communities.push({id,name:name.slice(0,100),path});
    else if(!path&&index>=2&&index<80&&name&&name.length<=100&&!/^(for you|following|추천|팔로잉|おすすめ|フォロー中)$/i.test(name))communities.push({id:'tab-'+index,name,path:'/home'});
  }
  const paths=postId||communityId||profileHandle?[request.destination].filter(Boolean):request.feed==='likes'?['/i/history/likes','/'+owner+'/likes']:request.feed==='bookmarks'?['/i/bookmarks','/i/history/bookmarks','/i/history']:request.feed==='communities'?['/i/communities','/'+owner+'/communities']:['/home'];
  if(!paths.includes(location.pathname)){
    if(request.target)return out('pending');
    if(location.pathname!=='/home'||request.advance||!paths.length)return out('unavailable');
    const observed=[...document.querySelectorAll('nav a[href]')].map(a=>pathOf(a.getAttribute('href'))).find(path=>paths.includes(path));
    return out('navigate',{owner,path:observed||paths[0],communities});
  }
  const context={owner,path:location.pathname,communities};
  if(!['foryou','following'].includes(request.feed)&&(!request.owner||!request.target&&!homeTab))return out('unavailable');
  const headings=[...primary.querySelectorAll('h1,h2,[role="heading"]')].map(n=>n.textContent.trim());
  if(request.feed==='foryou'||request.feed==='following'){
    const wanted=tabs.find(t=>(request.feed==='following'?/^(following|팔로잉|フォロー中)$/i:/^(for you|추천|おすすめ)$/i).test(t.textContent.trim()));
    if(!wanted)return out('unsupported',context);
    if(wanted.getAttribute('aria-selected')!=='true'){
      if(request.advance)return out('unsupported',context);
      wanted.click();return out('pending',context);
    }
  }else if(homeTab){
    const binding=request.binding;
    const wanted=tabs[Number(communityId.slice(4))];
    if(!binding||binding.id!==communityId||!wanted||labelOf(wanted)!==binding.name)return out('unsupported',context);
    if(wanted.getAttribute('aria-selected')!=='true'){if(request.advance)return out('unsupported',context);wanted.click();return out('pending',context);}
  }else if(request.feed==='likes'||request.feed==='bookmarks'){
    const label=request.feed==='likes'?/^(likes|마음에 들어요|마음함|좋아요|いいね)$/i:/^(bookmarks|all bookmarks|북마크|모든 북마크|ブックマーク)$/i;
    const wanted=tabs.find(t=>paths.includes(pathOf(t.getAttribute('href')||t.querySelector('a[href]')?.getAttribute('href')))||label.test(t.textContent.trim()));
    if(wanted&&wanted.getAttribute('aria-selected')!=='true'){if(request.advance)return out('unsupported',context);wanted.click();return out('pending',context);}
    if(!wanted&&!headings.some(h=>label.test(h)))return out('unsupported',context);
  }else if(request.feed==='communities'){
    const joined=primary.querySelector('[aria-label="Your communities"],[aria-label="내 커뮤니티"]')
      ||[...primary.querySelectorAll('section,[role="region"]')].find(s=>/^(your communities|내 커뮤니티|가입한 커뮤니티)$/i.test(s.querySelector('h1,h2,[role="heading"]')?.textContent?.trim()||''));
    if(!joined)return out('unsupported',context);
    for(const link of [...joined.querySelectorAll('a[href]')].slice(0,80)){
      const path=pathOf(link.getAttribute('href')),id=/^\/i\/communities\/([1-9]\d{4,24})$/.exec(path||'')?.[1];
      const name=link.textContent.trim().replace(/\s+/g,' ');
      if(id&&name&&!communities.some(c=>c.id===id))communities.push({id,name:name.slice(0,100),path});
    }
    return out('empty',{...context,communities,posts:[]});
  }
  const articles=[...primary.querySelectorAll('article[data-testid="tweet"]')].filter(a=>!a.parentElement?.closest('article[data-testid="tweet"]')).slice(0,60);
  const relatedHeadings=postId?[...primary.querySelectorAll('h1,h2,h3,[role="heading"]')].filter(n=>!n.closest('article[data-testid="tweet"]')&&/^(discover more|related posts|more posts|관련 게시물|더 알아보기|더 찾아보기|추천 게시물|もっと見つける)$/i.test(labelOf(n))):[];
  const precedes=(first,second)=>!!(first.compareDocumentPosition(second)&4);
  const relationOf=(root,id)=>{
    if(!postId||id===postId)return null;
    if(relatedHeadings.some(heading=>precedes(heading,root)))return 'related';
    // Use a rendered reply marker or an explicitly labelled conversation region.
    // When X does not expose either, keep it unclassified rather than inventing
    // a parent/child relationship from the text or author identity.
    const quotes=quotesFor(root);
    // Relationship metadata belongs to this article, not the quoted author's
    // article or user-written prose. Exclude ancestors that merely aggregate
    // a quote/body's text as well as nodes inside them.
    const ownMetadata=n=>n.closest('article[data-testid="tweet"]')===root&&!n.closest('[data-testid="tweetText"]')&&!n.querySelector('[data-testid="tweetText"]')&&!quotes.some(q=>q.contains(n)||n.contains(q));
    const marker=[...root.querySelectorAll('[data-testid="replyingTo"],[data-testid="replyingToText"]')].find(ownMetadata)||[...root.querySelectorAll('div')].find(n=>ownMetadata(n)&&n.children.length<4&&(/^(replying to(?:\s|$)|답글 대상|返信先)/i.test(n.textContent?.trim()||'')||/님에게 보내는 답글$/.test(n.textContent?.trim()||'')));
    const region=root.closest('[aria-label]')?.getAttribute('aria-label')||'';
    if(marker||/^(timeline: conversation|타임라인: 대화|タイムライン: 会話)$/i.test(region))return 'reply';
    return 'unknown';
  };
  if(request.advance){window.scrollBy({top:Math.max(300,(window.innerHeight||900)-250),behavior:'instant'});return out('advanced',context);}
  let revealing=false;
  for(const article of articles)for(const warning of [...article.querySelectorAll('[data-testid="contentWarning"],[data-testid="sensitiveMediaWarning"]')].slice(0,4)){
    const text=warning.textContent||'';
    if(!/sensitive|민감|センシティブ/i.test(text)||/age|verify|18|연령|나이|인증|subscribe|구독|login|sign in|로그인/i.test(text))continue;
    const button=[...warning.querySelectorAll('button,[role="button"]')].find(b=>/^(show|view|표시|보기|표시하기|表示)$/i.test(b.textContent.trim()));
    if(button&&button.getAttribute('data-atsumi-x-revealed')!=='true'){
      button.setAttribute('data-atsumi-x-revealed','true');button.click();revealing=true;
    }
  }
  if(revealing)return out('pending',context);
  const local=(node,root)=>node.closest('article')===root.closest('article');
  const quotesFor=root=>[...root.querySelectorAll('[role="link"]')].filter(n=>n.tagName!=='A'&&!n.contains(root.querySelector('[data-testid="User-Name"]'))&&n.querySelector('[data-testid="User-Name"]')&&n.querySelector('time')&&n.querySelector('[data-testid="tweetText"],[data-testid="tweetPhoto"]'));
  const raster=(raw,avatar=false)=>{
    try{const u=new URL(raw);if(u.protocol!=='https:'||u.hostname!=='pbs.twimg.com'||u.port||u.username||u.password||u.hash)return null;
      if(avatar)return !u.search&&/^\/profile_images\/[1-9]\d{0,24}\/[A-Za-z0-9_-]+\.(jpg|jpeg|png|webp)$/.test(u.pathname)?u.href:null;
      if(!/^\/(?:media\/[A-Za-z0-9_-]+(?:\.(?:jpg|jpeg|png|webp))?|ext_tw_video_thumb\/[1-9]\d{4,24}\/pu\/img\/[A-Za-z0-9_-]+\.(?:jpg|jpeg|png|webp)|amplify_video_thumb\/[1-9]\d{4,24}\/img\/[A-Za-z0-9_-]+\.(?:jpg|jpeg|png|webp)|tweet_video_thumb\/[A-Za-z0-9_-]+\.(?:jpg|jpeg|png|webp))$/.test(u.pathname))return null;
      if([...u.searchParams].some(([k,v])=>!['format','name'].includes(k)||k==='format'&&!['jpg','jpeg','png','webp'].includes(v)))return null;
      u.searchParams.set('name','medium');return u.href;
    }catch{return null;}
  };
  const previewQuote=root=>{
    const user=root.querySelector('[data-testid="User-Name"]');
    const handle=[...user?.querySelectorAll('span')||[]].map(n=>n.textContent.trim()).find(t=>/^@[A-Za-z0-9_]{1,15}$/.test(t))?.slice(1);
    if(!handle)return null;
    const name=[...user.querySelectorAll('span')].map(n=>n.textContent.trim()).find(t=>t&&!t.startsWith('@')&&t!=='·')||handle;
    const text=root.querySelector('[data-testid="tweetText"]')?.textContent||'';
    const media=[...root.querySelectorAll('[data-testid="tweetPhoto"] img,[data-testid="videoPlayer"] img,video[poster]')].map(n=>raster(n.getAttribute('poster')||n.currentSrc||n.src)).filter(Boolean).filter((v,i,a)=>a.indexOf(v)===i).slice(0,4);
    const avatar=[...root.querySelectorAll('[data-testid^="UserAvatar-Container"] img')].map(n=>raster(n.currentSrc||n.src,true)).find(Boolean)||null;
    return {handle,name:name.slice(0,120),text:text.slice(0,12000),time:(user.querySelector('time')?.dateTime||'').slice(0,40),media,avatar,video:!!root.querySelector('video,[data-testid="videoPlayer"]')};
  };
  const extract=(root,depth=0)=>{
    const candidates=depth?[]:quotesFor(root);
    const own=n=>local(n,root)&&!candidates.some(q=>q.contains(n));
    const user=[...root.querySelectorAll('[data-testid="User-Name"]')].find(own);
    if(!user)return null;
    let time=[...root.querySelectorAll('[data-testid="User-Name"] time')].find(own);
    let path=pathOf(time?.closest('a')?.getAttribute('href'));
    let match=/^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(path||'');
    if(!match&&postId&&!depth){
      time=[...root.querySelectorAll('time')].find(own);
      path=pathOf(time?.closest('a')?.getAttribute('href'));
      match=/^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(path||'');
      if(!match&&root===articles[0]){
        // The first rendered article may be a virtualized reply. A plain-text
        // detail date is usable only with the exact own permalink and author.
        const target=/^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(location.pathname);
        const identityQuotes=[...root.querySelectorAll('[role="link"]')].filter(n=>n.tagName!=='A'&&!n.contains(user)&&n.querySelector('[data-testid="User-Name"]'));
        const identityOwn=n=>own(n)&&!identityQuotes.some(q=>q.contains(n));
        const author=target&&[...user.querySelectorAll('a[href]')].some(link=>identityOwn(link)&&sameOriginPathOf(link.getAttribute('href'))?.toLowerCase()==='/'+target[1].toLowerCase());
        const permalink=[...root.querySelectorAll('a[href]')].some(link=>identityOwn(link)&&sameOriginPathOf(link.getAttribute('href'))===location.pathname);
        if(target?.[2]===postId&&author&&permalink)match=target;
      }
    }
    if(!match)return null;
    const name=[...user.querySelectorAll('a[href]')].find(a=>pathOf(a.getAttribute('href'))?.toLowerCase()==='/'+match[1].toLowerCase())?.textContent.trim()||match[1];
    const text=[...root.querySelectorAll('[data-testid="tweetText"]')].find(own)?.textContent||'';
    const liveMedia=[...root.querySelectorAll('[data-testid="tweetPhoto"] img,[data-testid="videoPlayer"] img,video[poster]')].filter(own).map(n=>raster(n.getAttribute('poster')||n.currentSrc||n.src)).filter(Boolean);
    const retained=depth?[]:window.__atsumiXRenderedMedia?.get(owner,match[2])||[];
    const media=[...liveMedia,...retained.slice(0,4).map(u=>raster(u)).filter(Boolean)].filter((v,i,a)=>a.indexOf(v)===i).slice(0,4);
    const avatar=[...root.querySelectorAll('[data-testid^="UserAvatar-Container"] img')].filter(own).map(n=>raster(n.currentSrc||n.src,true)).find(Boolean)||null;
    const control=id=>[...root.querySelectorAll('[data-testid="'+id+'"]')].find(own);
    const count=id=>{
      const node=control(id);if(!node)return '';
      // X animates old and new digits together. Its accessible label is the
      // canonical displayed count; concatenating both text nodes invents 78/87.
      const label=node.getAttribute('aria-label')?.trim();
      const number=label?.match(/^(\d[\d,.]*(?:\s*[kKmMbB천만억])?)(?=\s|$)/)?.[1];
      if(number)return number.slice(0,32);
      return (node.textContent||'').trim().slice(0,32);
    };
    const stats={replies:count('reply'),reposts:count('unretweet')||count('retweet'),likes:count('unlike')||count('like'),bookmarks:count('removeBookmark')||count('bookmark'),views:[...root.querySelectorAll('a[href$="/analytics"]')].find(own)?.textContent?.trim().slice(0,32)||'',liked:!!control('unlike'),reposted:!!control('unretweet'),bookmarked:!!control('removeBookmark'),canReply:!!control('reply'),canLike:!!(control('like')||control('unlike')),canRepost:!!(control('retweet')||control('unretweet')),canBookmark:!!(control('bookmark')||control('removeBookmark'))};
    const social=depth?null:[...root.querySelectorAll('[data-testid="socialContext"]')].find(own);
    const socialLabel=social?labelOf(social).slice(0,160):'';
    const pinned=/^(메인에 올림|고정|고정됨|pinned|pinned post|pinned tweet|固定|固定されたポスト)$/i.test(socialLabel);
    const isRepost=/^(?:.+\s+)?(?:reposted|retweeted)$/i.test(socialLabel)||/^(?:.+(?:님이|님|이|가)\s*)?재게시(?:했습니다|함)?$/.test(socialLabel)||/^.+が(?:リポスト|リツイート)(?:しました)?$/.test(socialLabel);
    const repost=isRepost?socialLabel:null;
    const quote=depth?null:candidates.map(q=>extract(q,1)).find(Boolean)||null;
    const quotePreview=depth||quote?null:candidates.map(previewQuote).find(Boolean)||null;
    return {id:match[2],handle:match[1],name:name.slice(0,120),text:text.slice(0,12000),time:(time?.dateTime||'').slice(0,40),media,avatar,video:[...root.querySelectorAll('[data-testid="videoPlayer"],video')].some(own),gif:media.some(u=>u.includes('/tweet_video_thumb/')),relation:depth?null:relationOf(root,match[2]),truncated:text.length>12000||!!control('tweet-text-show-more-link'),pinned,repost,quote,quotePreview,stats};
  };
  const posts=[],seen=new Set();
  for(const article of articles){
    // X attaches media only once its card enters the rendered viewport. Never
    // publish an offscreen text-only placeholder as a complete post.
    const rect=article.getBoundingClientRect();
    if(window.innerHeight&&rect.height>0&&(rect.bottom<0||rect.top>window.innerHeight+100))continue;
    const post=extract(article);if(post&&!seen.has(post.id)){seen.add(post.id);posts.push(post);}
  }
  if(!posts.length&&!articles.length){
    const empty=primary.querySelector('[data-testid="emptyState"]')?.textContent||'';
    if(/hasn.t liked any|haven.t liked|no likes yet|save posts for later|haven.t added any.*bookmark|no bookmarks|아직.*마음에|마음에.*게시물이 없|아직.*북마크|나중에.*게시물을 저장/i.test(empty))return out('empty',{...context,posts:[]});
  }
  return out(posts.length?'ready':'pending',{...context,posts});
})()
