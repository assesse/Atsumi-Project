// Public DOM facts can be transient when the non-playing transport rejects a
// video stream. Retain only observed posters, never site state or credentials.
(() => {
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname))return;
  const facts=new Map();let owner=null;
  const pathOf=raw=>{if(typeof raw!=='string'||!raw.trim())return null;try{const u=new URL(raw,location.origin);return u.origin===location.origin&&!u.username&&!u.password&&!u.search&&!u.hash?u.pathname:null;}catch{return null;}};
  const poster=raw=>{try{const u=new URL(raw);return u.protocol==='https:'&&u.hostname==='pbs.twimg.com'&&!u.port&&!u.username&&!u.password&&!u.hash&&u.href.length<2048&&/^\/(?:ext_tw_video_thumb|amplify_video_thumb|tweet_video_thumb)\//.test(u.pathname)?u.href:null;}catch{return null;}};
  const collect=records=>{
    const current=/^\/([A-Za-z0-9_]{1,15})\/?$/.exec(pathOf(document.querySelector('[data-testid="AppTabBar_Profile_Link"]')?.getAttribute('href'))||'')?.[1]?.toLowerCase();
    if(!current||!document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]')){facts.clear();owner=null;return;}
    if(owner!==current){facts.clear();owner=current;}
    const articles=[...document.querySelectorAll('[data-testid="primaryColumn"] article[data-testid="tweet"]')].filter(a=>!a.parentElement?.closest('article[data-testid="tweet"]')).slice(0,60);
    for(const article of articles){
      const username=article.querySelector('[data-testid="User-Name"]');
      const quotes=[...article.querySelectorAll('[role="link"]')].filter(n=>n.tagName!=='A'&&!n.contains(username)&&n.querySelector('[data-testid="User-Name"]'));
      const own=n=>n.closest('article[data-testid="tweet"]')===article&&!quotes.some(q=>q.contains(n));
      if(!username||!own(username))continue;
      const time=[...username.querySelectorAll('time')].find(own);
      let path=pathOf(time?.closest('a')?.getAttribute('href'));
      if(!path&&article===articles[0]){
        // Location plus position alone cannot authorize a virtualized reply's
        // poster for the main post. Plain dates need exact own rendered proof.
        const target=/^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(location.pathname);
        const author=target&&[...username.querySelectorAll('a[href]')].some(link=>own(link)&&pathOf(link.getAttribute('href'))?.toLowerCase()==='/'+target[1].toLowerCase());
        const permalink=[...article.querySelectorAll('a[href]')].some(link=>own(link)&&pathOf(link.getAttribute('href'))===location.pathname);
        if(target&&author&&permalink)path=location.pathname;
      }
      const id=/^\/[A-Za-z0-9_]{1,15}\/status\/([1-9]\d{4,24})$/.exec(path||'')?.[1];
      if(!id||!username)continue;
      // Removed nodes no longer have their old ancestors. Validate the mutation
      // target before admitting them, and fail closed for a foreign link even
      // if its quoted username was removed in the same mutation batch.
      const foreignLink=node=>{
        for(let ancestor=node;ancestor&&ancestor!==article;ancestor=ancestor.parentElement){
          if(ancestor.getAttribute('role')==='link'&&ancestor.tagName!=='A'&&!ancestor.contains(username))return true;
        }
        return false;
      };
      const roots=[article];
      for(const record of records||[]){
        if(record.target instanceof Element&&record.target.closest('article')===article&&!foreignLink(record.target)){
          roots.push(...[...record.addedNodes,...record.removedNodes].filter(n=>n instanceof Element));
        }
      }
      const urls=[];
      for(const record of records||[]){
        const node=record.target;
        if(record.type!=='attributes'||!['poster','src'].includes(record.attributeName)||!(node instanceof Element)||node.closest('article')!==article)continue;
        if(foreignLink(node))continue;
        const url=poster(record.oldValue);if(url&&!urls.includes(url))urls.push(url);
      }
      for(const root of roots){
        const nodes=[...(root.matches('video[poster],img')?[root]:[]),...root.querySelectorAll('video[poster],[data-testid="videoPlayer"] img')];
        for(const node of nodes){
          // A quoted video's poster must never authorize the enclosing post.
          if(foreignLink(node))continue;
          const url=poster(node.getAttribute('poster')||node.currentSrc||node.getAttribute('src'));
          if(url&&!urls.includes(url))urls.push(url);
        }
      }
      if(urls.length)facts.set(id,{owner,at:Date.now(),media:urls.slice(0,4)});
    }
    for(const [id,fact] of facts)if(Date.now()-fact.at>300000)facts.delete(id);
    while(facts.size>200)facts.delete(facts.keys().next().value);
  };
  Object.defineProperty(window,'__atsumiXRenderedMedia',{value:Object.freeze({get:(wanted,id)=>{
    const fact=facts.get(id);return wanted===owner&&fact?.owner===wanted&&Date.now()-fact.at<300000?[...fact.media]:[];
  }}),configurable:false,writable:false});
  const observer=new MutationObserver(collect);
  observer.observe(document,{subtree:true,childList:true,attributes:true,attributeOldValue:true,attributeFilter:['poster','src','href']});
  window.addEventListener('pagehide',()=>{observer.disconnect();facts.clear();owner=null;},{once:true});
  collect([]);
})()
