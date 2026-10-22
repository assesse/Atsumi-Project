// Invoked only by an explicit app action, on a native-bound post document.
(() => {
  const request=__ATSUMI_X_ACTION__;
  const out=(state,extra={})=>({nonce:request.nonce,state,...extra});
  if(window!==window.top||location.protocol!=='https:'||!['x.com','www.x.com'].includes(location.hostname)||location.pathname!==(request.documentPath||request.path))return out('rejected');
  if(!['like','bookmark'].includes(request.action)||typeof request.expected!=='boolean'||![undefined,'start','probe','confirm'].includes(request.phase))return out('rejected');
  const origin=location.origin||'https://'+location.hostname;
  const pathOf=raw=>{
    if(typeof raw!=='string'||!raw.trim())return null;
    try{const u=new URL(raw,origin);return u.origin===origin&&!u.port&&!u.username&&!u.password&&!u.search&&!u.hash?u.pathname:null;}catch{return null;}
  };
  const profile=/^\/([A-Za-z0-9_]{1,15})\/?$/.exec(pathOf(document.querySelector('[data-testid="AppTabBar_Profile_Link"]')?.getAttribute('href'))||'')?.[1]?.toLowerCase();
  if(!profile||!document.querySelector('[data-testid="SideNav_AccountSwitcher_Button"]'))return out('pendingOwner');
  if(profile!==request.owner)return out('rejected');
  const quotesOf=a=>[...a.querySelectorAll('[role="link"]')].filter(n=>n.tagName!=='A'&&!n.contains(a.querySelector('[data-testid="User-Name"]'))&&n.querySelector('[data-testid="User-Name"]'));
  const articles=[...document.querySelectorAll('[data-testid="primaryColumn"] article[data-testid="tweet"]')].filter(a=>!a.parentElement?.closest('article[data-testid="tweet"]'));
  const exactLink=(a,requireTime)=>[...a.querySelectorAll('a[href]')].some(link=>link.closest('article[data-testid="tweet"]')===a&&pathOf(link.getAttribute('href'))===request.path&&(!requireTime||link.querySelector('time'))&&!quotesOf(a).some(q=>q.contains(link)));
  let article=articles.find(a=>exactLink(a,true));
  if(!article&&location.pathname===request.path){
    // A detail date can be plain text. Neither its URL nor the first visible
    // reply proves identity: require this first article's own author/permalink.
    const first=articles[0],handle=/^\/([A-Za-z0-9_]{1,15})\/status\/[1-9]\d{4,24}$/.exec(request.path)?.[1]?.toLowerCase();
    const user=first?.querySelector('[data-testid="User-Name"]');
    // Without an outer author, a quote's first User-Name is not own evidence.
    // Ambiguous plain-date role-link wrappers fail closed; timed rows keep
    // their established own-permalink path above.
    const ambiguousAuthor=user&&[...first.querySelectorAll('[role="link"]')].some(n=>n.tagName!=='A'&&n.contains(user));
    const matchesAuthor=handle&&user&&!ambiguousAuthor&&user.closest('article[data-testid="tweet"]')===first&&[...user.querySelectorAll('a[href]')].some(link=>pathOf(link.getAttribute('href'))?.toLowerCase()==='/'+handle);
    if(first&&matchesAuthor&&exactLink(first,false))article=first;
  }
  if(!article)return out('pending');
  const quotes=quotesOf(article);
  const ownNode=n=>n.closest('article')===article&&!quotes.some(q=>q.contains(n));
  const controls=id=>[...article.querySelectorAll('[data-testid="'+id+'"]')].filter(ownNode);
  const own=id=>controls(id)[0];
  const on={like:'unlike',bookmark:'removeBookmark'}[request.action];
  const off={like:'like',bookmark:'bookmark'}[request.action];
  const enabled=own(on),disabled=own(off);
  if(!enabled&&!disabled)return out('unavailable');
  if(request.phase==='confirm'){
    // This branch never clicks or scrolls. Confirmation is not a feed page:
    // a successful mutation may outlive the target's viewport placement.
    const target=/^\/([A-Za-z0-9_]{1,15})\/status\/([1-9]\d{4,24})$/.exec(request.path);
    const users=[...article.querySelectorAll('[data-testid="User-Name"]')].filter(ownNode);
    const user=users.length===1?users[0]:null;
    if(!target||!user||![...user.querySelectorAll('a[href]')].some(link=>pathOf(link.getAttribute('href'))?.toLowerCase()==='/'+target[1].toLowerCase()))return out('identityMissing');
    const observed=[...controls(on),...controls(off)];
    if(observed.length!==1)return out('ambiguous');
    const button=observed[0];
    // A broad own-content wrapper may contain the author and action together.
    // An author found only in a quoted wrapper cannot attest an outer action.
    if([...article.querySelectorAll('[role="link"]')].some(n=>n.tagName!=='A'&&n.contains(user)&&!n.contains(button)))return out('identityMissing');
    if(button.disabled||button.getAttribute('aria-disabled')==='true')return out('disabled');
    const label=button.getAttribute('aria-label')?.trim();
    const number=label?.match(/^(\d[\d,.]*(?:\s*[kKmMbB천만억])?)(?=\s|$)/)?.[1];
    const count=(number||(button.textContent||'').trim()).slice(0,32);
    return out('observed',{observation:{postId:target[2],handle:target[1],owner:profile,path:request.path,active:button.getAttribute('data-testid')===on,count}});
  }
  if(!!enabled!==request.expected)return out('stateChanged');
  const button=request.expected?enabled:disabled;if(!button)return out('unavailable');
  if(button.disabled||button.getAttribute('aria-disabled')==='true')return out('disabled');
  if(request.phase==='probe'){
    const rect=article.getBoundingClientRect();
    return out(window.innerHeight&&rect.height>0&&(rect.bottom<0||rect.top>window.innerHeight)?'offscreen':'ready');
  }
  button.click();return out('sent');
})()
