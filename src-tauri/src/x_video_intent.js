// App playback intent (explicit play or user-requested viewport autoplay).
// Reads only the bound post's rendered media; no private state/API or guessing.
(() => {
  const request=__ATSUMI_X_VIDEO__,out=(state,extra={})=>({nonce:request.nonce,state,...extra});
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname)||location.pathname!==(request.documentPath||request.path))return out('rejected');
  const owner=document.querySelector('[data-testid="AppTabBar_Profile_Link"]')?.getAttribute('href')?.replace(/^\/|\/$/g,'').toLowerCase();
  if(owner!==request.owner||!document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]'))return out('rejected');
  const quotesOf=a=>[...a.querySelectorAll('[role="link"]')].filter(n=>n.tagName!=='A'&&!n.contains(a.querySelector('[data-testid="User-Name"]'))&&n.querySelector('[data-testid="User-Name"]'));
  const pathOf=raw=>{
    try{const u=new URL(raw,'https://x.com');return u.protocol==='https:'&&['x.com','www.x.com'].includes(u.hostname)&&!u.port&&!u.username&&!u.password&&!u.search&&!u.hash?u.pathname:null;}catch{return null;}
  };
  const articles=[...document.querySelectorAll('[data-testid="primaryColumn"] article[data-testid="tweet"]')].filter(a=>!a.parentElement?.closest('article[data-testid="tweet"]'));
  const exactLink=(a,requireTime)=>[...a.querySelectorAll('a[href]')].some(link=>pathOf(link.getAttribute('href'))===request.path&&(!requireTime||link.querySelector('time'))&&!quotesOf(a).some(q=>q.contains(link)));
  let article=articles.find(a=>exactLink(a,true));
  if(!article&&location.pathname===request.path){
    // Detail dates may be plain text, but a pathname alone cannot identify a
    // virtualized first reply. Still require the exact own permalink and author.
    const first=articles[0],handle=/^\/([A-Za-z0-9_]{1,15})\/status\/[1-9]\d{4,24}$/.exec(request.path)?.[1]?.toLowerCase();
    const user=first?.querySelector('[data-testid="User-Name"]');
    const matchesAuthor=handle&&user&&[...user.querySelectorAll('a[href]')].some(link=>pathOf(link.getAttribute('href'))?.toLowerCase()==='/'+handle);
    if(first&&matchesAuthor&&exactLink(first,false))article=first;
  }
  if(!article)return out('postMissing');
  const quotes=quotesOf(article);
  const player=[...article.querySelectorAll('[data-testid="videoPlayer"]')].find(n=>!quotes.some(q=>q.contains(n)));
  if(!player)return out('playerMissing');
  const video=player.querySelector('video');
  let intentSent=false;
  if(request.inspect!==true){
    article.scrollIntoView({block:'center',behavior:'instant'});
    if(video){
      video.muted=true;intentSent=true;
      try{void Promise.resolve(video.play()).catch(()=>{});}catch{}
    }else{
      const button=[...player.querySelectorAll('button,[role="button"]')].find(b=>!b.disabled&&b.getAttribute('aria-disabled')!=='true'&&/^(play|play video|재생|플레이하기|동영상 재생|再生)$/i.test(b.getAttribute('aria-label')||b.textContent.trim()));
      if(button){intentSent=true;button.click();}
    }
  }
  const urls=values=>values.filter(v=>typeof v==='string'&&v.length<2048&&v.startsWith('https://')).slice(0,4);
  return out('observed',{intentSent,posters:urls([video?.poster,...[...player.querySelectorAll('img')].map(n=>n.currentSrc||n.src)]),
    sources:urls([video?.currentSrc,video?.getAttribute('src'),...[...player.querySelectorAll('video source')].map(n=>n.src)]),
    video:!!video,ready:video?.readyState??null,error:video?.error?.code??null});
})()
