// Read only the author and first line belonging to the current preview route.
(() => {
  const config = __ATSUMI_X_PRESENTATION__;
  if (window !== window.top || location.protocol !== 'https:' || !['x.com','www.x.com'].includes(location.hostname) || config.feed !== 'preview') return;
  let last = '', lastProbe = '';
  const probe = phase => {
    if(config.diagnostics!==true || phase===lastProbe)return;
    lastProbe=phase;
    window.chrome?.webview?.postMessage('ATSUMI_X_SURFACE_V1:https://x.com/__atsumi_surface__/'+config.nonce+'?kind=presentationProbe&value='+phase);
  };
  const pathOf = raw => { try {const u=new URL(raw,location.origin);return u.origin===location.origin&&!u.search&&!u.hash?u.pathname:null;}catch{return null;} };
  const shown = node => node && !node.closest('[hidden],[aria-hidden="true"],[inert]');
  const text = (node,limit) => [...(node?.innerText || node?.textContent || '').split(/\r?\n/).map(s=>s.trim()).filter(Boolean)][0]?.replace(/[\u0000-\u001f\u007f]/g,'').slice(0,limit) || '';
  const displayName = node => {
    if(!shown(node))return '';
    const copy=node.cloneNode(true);
    for(const hidden of copy.querySelectorAll('svg,[hidden],[aria-hidden="true"],[inert]'))hidden.remove();
    for(const image of copy.querySelectorAll('img[alt]'))image.replaceWith(document.createTextNode(image.getAttribute('alt')||''));
    return text(copy,120);
  };
  window.__atsumiXPresentation = Object.freeze({
    update(ready) {
      if (!ready) {last='';return;}
      const path=location.pathname,post=/^\/(?:[A-Za-z0-9_]{1,15}|i\/web)\/status\/([1-9]\d{4,24})(?:\/(?:photo|video)\/[1-4])?$/.exec(path);
      let user=null,article=null,handle='',body=null,avatar=null;
      if (post) {
        const articles=[...document.querySelectorAll('article[data-testid="tweet"]')];
        if (articles.length>160) {probe("limit");return;}
        for (const candidate of articles) {
          if (!shown(candidate)||candidate.parentElement?.closest('article'))continue;
          const users=[...candidate.querySelectorAll('[data-testid="User-Name"]')].filter(n=>n.closest('article')===candidate&&!n.closest('[data-testid="quoteTweet"]'));
          // Expanded posts put their timestamp below the body, not in User-Name.
          const permalink=[...candidate.querySelectorAll('a[href]')].find(a=>a.closest('article')===candidate&&!a.closest('[data-testid="quoteTweet"],[role="link"]:not(a)')&&(a.querySelector('time')||/\/history$/.test(pathOf(a.getAttribute('href'))||''))&&pathOf(a.getAttribute('href'))?.match(/\/status\/([1-9]\d{4,24})(?:\/history)?$/)?.[1]===post[1]);
          const own=users.find(n=>!n.closest('[data-testid="quoteTweet"],[role="link"]:not(a)'));
          if(permalink&&own){if(article){probe("ambiguous");return;}article=candidate;user=own;}
        }
        if(!article||!user){probe(articles.length?"unmatched":"empty");return;}
        const author=[...user.querySelectorAll('a[href]')].find(a=>/^\/[A-Za-z0-9_]{1,15}$/.test(pathOf(a.getAttribute('href'))||''));
        handle=author&&pathOf(author.getAttribute('href'))?.slice(1)||'';
        if(!handle){probe("author");return;}
        body=[...article.querySelectorAll('[data-testid="tweetText"]')].find(n=>n.closest('article')===article&&!n.closest('[data-testid="quoteTweet"],[role="link"]'));
        const ownPortrait=node=>node.closest('article')===article&&!node.closest('[data-testid="quoteTweet"],[hidden],[inert]');
        const avatarContainer=[...article.querySelectorAll('[data-testid^="UserAvatar-Container-"]')].find(node=>ownPortrait(node)&&node.getAttribute('data-testid').toLowerCase()==='useravatar-container-'+handle.toLowerCase());
        // A decorative aria-hidden portrait is still the exact displayed avatar.
        avatar=avatarContainer?.querySelector('img')||[...article.querySelectorAll('a[href] img')].find(img=>ownPortrait(img)&&
          ['/'+handle.toLowerCase(),'/'+handle.toLowerCase()+'/photo'].includes(pathOf(img.closest('a').getAttribute('href'))?.toLowerCase()));
      } else {
        const match=/^\/([A-Za-z0-9_]{1,15})$/.exec(path);if(!match)return;handle=match[1];
        user=document.querySelector('[data-testid="primaryColumn"] [data-testid="UserName"]');
        if(!shown(user)){probe("profile");return;}
        body=document.querySelector('[data-testid="UserDescription"]');
        avatar=document.querySelector('[data-testid="primaryColumn"] a[href="/'+handle+'/photo"] img');
      }
      // X's author link has no stable dir attribute. Anchor the name to the
      // exact author's public profile, excluding quote/reply and badge text.
      const nameNodes=post?[...user.querySelectorAll('a[href]')].filter(n=>pathOf(n.getAttribute('href'))?.toLowerCase()==='/'+handle.toLowerCase()):
        [...user.querySelectorAll('[dir="auto"],span')].filter(n=>!n.querySelector('span,[dir="auto"]'));
      const name=nameNodes.map(displayName).find(t=>t&&!t.startsWith('@'))||'';
      if(!name){probe("name");return;}
      const src=avatar?.currentSrc||avatar?.getAttribute('src')||'';
      const portrait=/^https:\/\/pbs\.twimg\.com\/profile_images\/[0-9]+\/[A-Za-z0-9_.-]+\.(?:jpg|jpeg|png|webp)$/.test(src)?src:null;
      const value=JSON.stringify({path,name,firstLine:text(body,200)||(post?'':'프로필'),avatar:portrait});
      const packet='https://x.com/__atsumi_surface__/'+config.nonce+'?kind=previewPresentation&value='+encodeURIComponent(value);
      if(packet.length>4096){probe("envelope");return;}if(value===last)return;
      probe('send');
      last=value;
      window.chrome?.webview?.postMessage('ATSUMI_X_SURFACE_V1:'+packet);
    }
  });
})();
