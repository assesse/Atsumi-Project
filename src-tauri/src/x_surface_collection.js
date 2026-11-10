// Optional collection presentation over the same official document.
// No private API, credential reads, action replay or media URL rewriting.
(() => {
  const config = __ATSUMI_X_COLLECTION__;
  if (window !== window.top || location.protocol !== 'https:' || !['x.com','www.x.com'].includes(location.hostname)
    || !['likes','bookmarks'].includes(config.feed) || window.__atsumiXCollection) return;
  const LIMIT = 1200, BATCH = 60;
  const records = new Map(), order = [], mediaOrder = [], mediaKeys = new Set(), nodes = new Map(), scrolls = {posts:0,media:0};
  const hiddenScrollbars = new Set();
  let cardWidth = 220;
  let mode = 'original', allowed = false, root = null, grid = null, status = null, more = null;
  let primary = null, originalScroll = [], sourceScroll = [], loading = null, timer = 0, pageLimit = BATCH;
  let failure = '', fingerprint = '', observer = null, scheduled = 0, noMediaPages = 0, fillTimer = 0, autoPaused = false;
  const limits = {posts:BATCH,media:BATCH};
  const own = (node, article) => node.closest('article[data-testid="tweet"]') === article
    && !node.closest('[data-testid="quoteTweet"],[data-testid="quotedTweet"],[role="link"]:not(a)');
  const canonical = raw => {
    try { const url = new URL(raw, location.origin); return url.protocol === 'https:' && ['x.com','www.x.com'].includes(url.hostname)
      && !url.username && !url.password && !url.port ? url.pathname : ''; } catch { return ''; }
  };
  const mediaUrl = raw => {
    if (typeof raw !== 'string' || raw.length > 2048) return null;
    try { const url = new URL(raw); return url.protocol === 'https:' && url.hostname === 'pbs.twimg.com'
      && !url.username && !url.password && !url.port
      && /^\/(?:media|ext_tw_video_thumb|amplify_video_thumb|tweet_video_thumb)\/[A-Za-z0-9_./-]+$/.test(url.pathname) ? raw : null; } catch { return null; }
  };
  const route = () => {
    const path = location.pathname.replace(/\/$/, '');
    return config.feed === 'likes' ? ["/"+config.owner+"/likes",'/i/history/likes'].includes(path.toLowerCase())
      : ['/i/bookmarks','/i/history','/i/history/bookmarks'].includes(path);
  };
  const usable = () => allowed && mode !== 'original' && route() && !document.hidden;
  function read(article) {
    if (article.closest('[hidden],[inert],[aria-hidden="true"],[data-atsumi-x-ad-hidden]')
      || window.__atsumiXAds?.blocked(article)) return null;
    const user = [...article.querySelectorAll('[data-testid="User-Name"]')].find(node => own(node,article));
    if (!user) return null;
    const links = [...user.querySelectorAll('time')].filter(node=>own(node,article))
      .map(node=>({node,path:canonical(node.closest('a')?.getAttribute('href'))}))
      .filter(value=>/^\/[A-Za-z0-9_]{1,15}\/status\/[1-9]\d{4,24}$/.test(value.path));
    const paths = new Set(links.map(value=>value.path));
    if (paths.size !== 1) return null;
    const path = links[0].path, parts = path.split('/'), id = parts[3], handle = parts[1];
    const nameNode = [...user.querySelectorAll('a[href]')].find(node=>canonical(node.getAttribute('href')).toLowerCase()==='/'+handle.toLowerCase());
    const textNode = [...article.querySelectorAll('[data-testid="tweetText"]')].find(node=>own(node,article));
    const text = (textNode?.textContent||'').trim().slice(0,6000);
    const media = [], seen = new Set();
    for (const image of [...article.querySelectorAll('[data-testid="tweetPhoto"] img,video[poster],[data-testid="videoPlayer"] img')].slice(0,20)) {
      if (!own(image,article)) continue;
      const src = mediaUrl(image.tagName==='VIDEO'?image.getAttribute('poster'):image.currentSrc||image.getAttribute('src'));
      if (!src || media.length >= 4) continue;
      const mediaKey=new URL(src).pathname;
      if(seen.has(mediaKey))continue;seen.add(mediaKey);
      const video = image.tagName==='VIDEO'||!!image.closest('[data-testid="videoPlayer"]');
      const observed = canonical(image.closest('a')?.getAttribute('href'));
      const target = observed.startsWith(path+'/') && /\/(?:photo|video)\/[1-4]$/.test(observed)
        ? observed : path+'/'+(video?'video':'photo')+'/'+(media.length+1);
      media.push({src,video,path:target});
    }
    const social = [...article.querySelectorAll('[data-testid="socialContext"]')].find(node=>own(node,article));
    return {id,path,handle,name:(nameNode?.textContent||handle).trim().slice(0,100),text,
      time:links[0].node.getAttribute('datetime')||'',media,repost:(social?.textContent||'').trim().slice(0,100)};
  }
  const mediaKey = media => new URL(media.src).pathname;
  function retainMedia(previous, incoming) {
    // A recycled article may briefly omit media or render it in several passes.
    // Retain observed identities; a resolution/DOM change is not a new card.
    const result = [...(previous || [])], known = new Set(result.map(mediaKey));
    for (const media of incoming) if (!known.has(mediaKey(media)) && result.length < 4) {
      known.add(mediaKey(media)); result.push(media);
    }
    return result;
  }
  function collect() {
    if (!usable()) return false;
    const host = document.querySelector('[data-testid="primaryColumn"]');
    if (!host) return false;
    if(primary!==host){primary?.removeAttribute('data-atsumi-x-collection-source');primary=host;observer?.disconnect();observer=null;}
    const incoming = [...host.querySelectorAll('article[data-testid="tweet"]')].slice(0,160).map(read).filter(Boolean);
    let changed = false;
    for(let i=0;i<incoming.length;i++){
      const post=incoming[i],previous=records.get(post.id);
      if(!previous&&records.size>=LIMIT)continue;
      // Once shown, a card's place is immutable until this collection resets.
      // X recycles/reorders source nodes while paging; those are not insert intents.
      if(!previous)order.push(post.id);
      post.media=retainMedia(previous?.media,post.media);
      if(previous){post.text=post.text||previous.text;post.time=post.time||previous.time;}
      for(const media of post.media){
        const key=post.id+':'+mediaKey(media);
        if(!mediaKeys.has(key)){mediaKeys.add(key);mediaOrder.push({key,id:post.id,identity:mediaKey(media)});}
      }
      if(JSON.stringify(previous)!==JSON.stringify(post)){records.set(post.id,post);changed=true;}
    }
    primary.setAttribute('data-atsumi-x-collection-source','');
    syncSourceScrollbars();
    return changed;
  }
  function elementsToScroll(){
    const result=[],page=document.scrollingElement;
    if(page)result.push(page);
    for(let node=primary?.parentElement,depth=0;node&&depth<32;node=node.parentElement,depth++){
      if(node===page)continue;
      if(/auto|scroll|overlay/.test(getComputedStyle(node).overflowY)&&node.scrollHeight>node.clientHeight)result.push(node);
    }
    return result;
  }
  function syncSourceScrollbars() {
    const current=new Set(elementsToScroll());
    for(const node of hiddenScrollbars)if(!current.has(node)){
      node.removeAttribute('data-atsumi-x-collection-scroll-source');hiddenScrollbars.delete(node);
    }
    for(const node of current){node.setAttribute('data-atsumi-x-collection-scroll-source','');hiddenScrollbars.add(node);}
  }
  const positions = () => elementsToScroll().map(node=>({node,top:node.scrollTop,left:node.scrollLeft}));
  const restore = saved => { for(const value of saved)if(value.node.isConnected){value.node.scrollTop=value.top;value.node.scrollLeft=value.left;} };
  function stopLoading(){ if(timer)clearTimeout(timer);timer=0;loading=null; }
  function hide() {
    if(root){if(!root.hidden&&mode!=='original')scrolls[mode]=root.scrollTop;root.hidden=true;}
    primary?.removeAttribute('data-atsumi-x-collection-source');
    for(const node of hiddenScrollbars)node.removeAttribute('data-atsumi-x-collection-scroll-source');
    hiddenScrollbars.clear();
    observer?.disconnect();observer=null;
    if(scheduled)clearTimeout(scheduled);scheduled=0;
    if(fillTimer)clearTimeout(fillTimer);fillTimer=0;
    stopLoading();
  }
  const el=(tag,className,text)=>{const node=document.createElement(tag);if(className)node.className=className;if(text)node.textContent=text;return node;};
  function install() {
    if(root?.isConnected)return;
    const style=el('style');style.textContent=__ATSUMI_X_COLLECTION_CSS__;
    document.head.append(style);
    root=el('section','atsumi-x-collection');root.setAttribute('aria-label','X 카드 목록');root.tabIndex=0;
    root.style.setProperty('--atsumi-x-card-width',cardWidth+'px');
    grid=el('div','atsumi-x-collection-grid');grid.setAttribute('role','list');
    status=el('p','atsumi-x-collection-status');status.setAttribute('role','status');
    more=el('button','atsumi-x-collection-more','더 불러오기');more.type='button';
    more.addEventListener('click',event=>{if(!event.isTrusted)return;event.preventDefault();event.stopPropagation();noMediaPages=0;autoPaused=false;requestMore();});
    root.append(grid,status,more);
    root.addEventListener('scroll',()=>{
      if(!usable())return;
      scrolls[mode]=root.scrollTop;
      if(root.scrollHeight-root.scrollTop-root.clientHeight<500&&!loading&&!failure&&!autoPaused)requestMore();
    },{passive:true});
    document.body.append(root);
    window.addEventListener?.('resize',queueFill,{passive:true});
  }
  function cards() {
    if(mode==='media')return mediaOrder.map(entry=>{
      const post=records.get(entry.id),index=post.media.findIndex(media=>mediaKey(media)===entry.identity);
      return {key:entry.key,post,media:post.media[index],count:post.media.length,index};
    });
    return order.map(id=>{const post=records.get(id);return {key:id,post,media:post.media[0]||null,count:post.media.length,index:0};});
  }
  function card(value) {
    const {post,media,count,index}=value;
    const item=el('div','atsumi-x-collection-item');item.setAttribute('role','listitem');item.dataset.cardKey=value.key;
    const button=el('button','atsumi-x-collection-card');button.type='button';
    button.dataset.postId=post.id;
    const label=[post.name,post.text|| (media?.video?'동영상':media?'이미지':'게시물')].join(' · ');
    button.setAttribute('aria-label',label.slice(0,500));button.title=label;
    if(media){
      const image=el('img');image.src=media.src;image.alt='';image.loading='lazy';image.decoding='async';
      image.addEventListener('error',()=>{image.hidden=true;button.classList.add('has-image-error');},{once:true});
      button.append(image);
      if(media.video){const kind=el('span','atsumi-x-collection-kind','▶');kind.setAttribute('aria-label','동영상');button.append(kind);}
      if(count>1){const badge=el('span','atsumi-x-collection-count',mode==='media'?(index+1)+'/'+count:String(count));badge.setAttribute('aria-label',mode==='media'?(index+1)+' / '+count+' 미디어':count+'개 미디어');button.append(badge);}
    }else{
      button.classList.add('is-text');
      button.append(el('span','atsumi-x-collection-text',post.text||'게시물'));
    }
    const caption=el('span','atsumi-x-collection-caption');
    caption.append(el('strong','',post.text.split(/\r?\n/).find(line=>line.trim())|| (media?.video?'동영상':media?'이미지':'게시물')));
    caption.append(el('span','atsumi-x-collection-author',post.name+' · @'+post.handle));
    const metadata=el('span','atsumi-x-collection-meta'),date=new Date(post.time);
    if(Number.isFinite(date.getTime())){const time=el('time','',date.toLocaleDateString('ko-KR'));time.dateTime=post.time;metadata.append(time);}
    if(post.repost){const repost=el('span','atsumi-x-collection-repost','↻');repost.title=post.repost;repost.setAttribute('aria-label',post.repost);metadata.append(repost);}
    caption.append(metadata);
    button.append(caption);item.append(button);
    button.addEventListener('click',event=>{
      if(!event.isTrusted||!usable()||event.altKey||event.shiftKey)return;
      const path=mode==='media'&&media?media.path:post.path;
      window.__atsumiXSurface?.collectionOpen(event,path);
    });
    return item;
  }
  function render() {
    if(!root||!usable())return;
    const values=cards(),shown=values.slice(0,pageLimit),key=JSON.stringify(shown);
    // Reuse nodes and their decoded images through native DOM churn/paging.
    if(key!==fingerprint){
      const keep=new Set();
      for(const value of shown){
        keep.add(value.key);const signature=JSON.stringify(value);
        let cached=nodes.get(value.key);
        if(!cached||cached.signature!==signature){
          const next={signature,node:card(value)};cached?.node.replaceWith(next.node);cached=next;nodes.set(value.key,cached);
        }
      }
      for(const [key,value] of nodes)if(!keep.has(key)){value.node.remove();nodes.delete(key);}
      let cursor=grid.firstChild;
      for(const value of shown){const node=nodes.get(value.key).node;if(node!==cursor)grid.insertBefore(node,cursor);cursor=node.nextSibling;}
      fingerprint=key;
    }
    const reached=records.size>=LIMIT;
    const empty=!!primary?.querySelector('[data-testid="emptyState"]');
    const message=loading?'불러오는 중…':failure||(reached?'기본 보기에서 계속 볼 수 있습니다.':!values.length?(empty?'표시할 항목이 없습니다.':mode==='media'&&records.size?'불러온 게시물에 미디어가 없습니다.':'게시물을 불러오고 있습니다.'):'');
    if(status.textContent!==message)status.textContent=message;
    status.hidden=!message;more.disabled=!!loading;more.hidden=reached||empty&&values.length===0;
    const moreText=failure?'다시 시도':'더 불러오기';if(more.textContent!==moreText)more.textContent=moreText;
    root.setAttribute('aria-busy',String(!!loading));
    queueFill();
  }
  function queueFill(){
    if(fillTimer||!usable()||loading||failure||autoPaused||!root||root.clientHeight<=0
      ||primary?.querySelector('[data-testid="emptyState"]')||records.size>=LIMIT)return;
    if(root.scrollHeight-root.scrollTop-root.clientHeight<Math.min(300,root.clientHeight/2))fillTimer=setTimeout(()=>{fillTimer=0;requestMore();},180);
  }
  function receivedPage(){
    if(!loading)return;
    const increased=cards().length>loading.visibleCount;
    stopLoading();failure='';autoPaused=false;noMediaPages=increased?0:noMediaPages+1;
  }
  function observe(){
    if(observer||!primary)return;
    observer=new MutationObserver(()=>{if(!scheduled&&usable())scheduled=setTimeout(()=>{
      scheduled=0;const changed=collect();if(changed){if(loading&&records.size>loading.count)receivedPage();render();}
    },180);});
    observer.observe(primary,{childList:true,subtree:true,characterData:true,attributes:true,attributeFilter:['src','srcset','poster']});
  }
  function tick(){
    timer=0;if(!loading||!usable()){stopLoading();return;}
    const changed=collect();
    if(records.size>loading.count){receivedPage();render();return;}
    if(performance.now()-loading.at>12000){
      // Silence can be the native list's end or a delayed response, not proof of failure.
      // Keep manual loading available without polling an unchanged tail forever.
      stopLoading();autoPaused=true;render();return;
    }
    // Advance only while the user has reached the card list's end. Incremental
    // native scrolling keeps the site's virtualizer and cursor authoritative.
    const sources=elementsToScroll();let advanced=false;
    for(const node of sources){
      const next=Math.min(node.scrollHeight-node.clientHeight,node.scrollTop+Math.max(200,node.clientHeight*.8));
      if(next>node.scrollTop+1){node.scrollTop=next;advanced=true;}
    }
    if(changed)render();
    timer=setTimeout(tick,advanced?500:900);
  }
  function requestMore(){
    if(!usable()||loading||autoPaused||records.size>=LIMIT)return;
    const available=cards();
    if(pageLimit<available.length){pageLimit+=BATCH;limits[mode]=pageLimit;render();return;}
    if(noMediaPages>=3){failure='미디어를 더 불러오려면 다시 시도해 주세요.';render();return;}
    failure='';loading={count:records.size,visibleCount:available.length,at:performance.now()};render();timer=setTimeout(tick,0);
  }
  window.__atsumiXCollection=Object.freeze({
    active:usable,
    mode(next,width){
      if(!['original','posts','media'].includes(next))return;
      if(Number.isInteger(width)&&width>=160&&width<=360&&width!==cardWidth){
        const anchor=root&&!root.hidden?[...grid.children].find(node=>node.getBoundingClientRect().bottom>root.getBoundingClientRect().top+12):null;
        const before=anchor?.getBoundingClientRect().top;
        cardWidth=width;
        root?.style.setProperty('--atsumi-x-card-width',cardWidth+'px');
        if(anchor&&before!==undefined){root.scrollTop+=anchor.getBoundingClientRect().top-before;scrolls[mode]=root.scrollTop;}
        queueFill();
      }
      if(next===mode)return;
      const previous=mode;
      if(root&&previous!=='original')scrolls[previous]=root.scrollTop;
      if(previous==='original'&&next!=='original'){primary=document.querySelector('[data-testid="primaryColumn"]');originalScroll=positions();if(sourceScroll.length)restore(sourceScroll);window.__atsumiXSurface?.pause();}
      if(next==='original'){sourceScroll=positions();hide();restore(originalScroll);}
      if(root&&next!=='original')root.hidden=true;
      stopLoading();mode=next;pageLimit=limits[next]||BATCH;fingerprint='';failure='';noMediaPages=0;autoPaused=false;
      this.update(allowed);
    },
    update(ready,reset=false){
      allowed=ready===true;
      if(reset){autoPaused=false;records.clear();order.length=0;mediaOrder.length=0;mediaKeys.clear();nodes.clear();fingerprint='';scrolls.posts=0;scrolls.media=0;limits.posts=BATCH;limits.media=BATCH;pageLimit=BATCH;sourceScroll=[];originalScroll=[];if(grid)grid.replaceChildren();}
      if(!usable()){hide();return;}
      install();const returning=root.hidden;collect();root.hidden=false;observe();render();
      if(returning)root.scrollTop=scrolls[mode]||0;
    }
  });
})();
